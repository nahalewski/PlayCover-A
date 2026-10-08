/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Apple LP64 Objective-C metadata decoding, before class realization.
//!
//! Call only on mapped, rebased and bound guest memory. Addresses remain guest
//! addresses: this module never casts them to host pointers or executes IMPs.
//! Layout reference: apple-oss-distributions/objc4/runtime/objc-runtime-new.h.
//! This is not a Foundation implementation or a replacement for libobjc init.

const MAX_METHODS: usize = 4096;
const MAX_STRING: usize = 4096;
const MAX_STRINGS: usize = 1024 * 1024;

/// Verified shared-cache VM interval and objc_opt_t v16 selector base. The
/// caller obtains this from the original libobjc optimization metadata.
#[derive(Clone, Copy, Debug)]
pub(super) struct CacheSelectorContext {
    pub start: u64,
    pub end: u64,
    pub base: u64,
}
impl CacheSelectorContext {
    pub(super) fn validate(self) -> Result<(), String> {
        if self.start >= self.end || self.base < self.start || self.base >= self.end {
            return Err("invalid cached Objective-C selector base context".into());
        }
        Ok(())
    }
}

#[path = "a64_objc_dispatch.rs"]
mod dispatch;
pub(super) use dispatch::{Invocation, MessagePlan, Registration, Registry, SelectorFixup};

#[path = "a64_objc_image.rs"]
mod image;
pub(super) use image::{image_slots, ImageSlots};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Method {
    pub selector_address: u64,
    pub selector: String,
    pub types: String,
    pub implementation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Class {
    pub address: u64,
    pub isa: u64,
    pub superclass: u64,
    pub name: String,
    pub flags: u32,
    pub instance_start: u32,
    pub instance_size: u32,
    pub methods: Vec<Method>,
}

fn add(address: u64, offset: u64) -> Result<u64, String> {
    address
        .checked_add(offset)
        .ok_or_else(|| "Objective-C metadata address overflow".into())
}

fn relative(address: u64, offset: i32) -> Result<u64, String> {
    if offset >= 0 {
        add(address, offset as u64)
    } else {
        address
            .checked_sub(-(offset as i64) as u64)
            .ok_or_else(|| "Objective-C relative pointer underflow".into())
    }
}

struct Reader<F> {
    read: F,
    strings: usize,
    string_cache: std::collections::BTreeMap<u64, String>,
}

impl<F: FnMut(u64, usize) -> Result<Vec<u8>, String>> Reader<F> {
    fn new(read: F) -> Self {
        Self {
            read,
            strings: 0,
            string_cache: Default::default(),
        }
    }
    fn bytes(&mut self, address: u64, size: usize) -> Result<Vec<u8>, String> {
        add(address, size as u64)?;
        let bytes = (self.read)(address, size)?;
        if bytes.len() != size {
            return Err("short Objective-C guest read".into());
        }
        Ok(bytes)
    }
    fn u64(&mut self, address: u64) -> Result<u64, String> {
        Ok(u64::from_le_bytes(
            self.bytes(address, 8)?.try_into().unwrap(),
        ))
    }
    fn string(&mut self, address: u64) -> Result<String, String> {
        if address == 0 {
            return Err("null Objective-C metadata string".into());
        }
        if let Some(text) = self.string_cache.get(&address) {
            return Ok(text.clone());
        }
        if self.string_cache.len() >= 65536 {
            return Err("Objective-C unique metadata string budget exceeded".into());
        }
        let mut bytes = Vec::new();
        for i in 0..MAX_STRING {
            let b = self.bytes(add(address, i as u64)?, 1)?[0];
            self.strings = self
                .strings
                .checked_add(1)
                .ok_or("Objective-C string budget overflow")?;
            if self.strings > MAX_STRINGS {
                return Err("Objective-C metadata string budget exceeded".into());
            }
            if b == 0 {
                let text = String::from_utf8(bytes)
                    .map_err(|_| "invalid UTF-8 Objective-C metadata string".to_string())?;
                self.string_cache.insert(address, text.clone());
                return Ok(text);
            }
            bytes.push(b);
        }
        Err("unterminated or oversized Objective-C metadata string".into())
    }
    fn methods(&mut self, address: u64) -> Result<Vec<Method>, String> {
        self.methods_with_cache(address, None)
    }
    fn methods_with_cache(
        &mut self,
        address: u64,
        cache: Option<CacheSelectorContext>,
    ) -> Result<Vec<Method>, String> {
        if let Some(cache) = cache {
            cache.validate()?;
        }
        if address == 0 {
            return Ok(Vec::new());
        }
        if address & 7 != 0 {
            return Err("unsupported tagged or unaligned Objective-C method list".into());
        }
        let header = self.bytes(address, 8)?;
        let format = u32::from_le_bytes(header[..4].try_into().unwrap());
        let count = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
        let small = format & 0x8000_0000 != 0;
        let direct = format & 0x4000_0000 != 0;
        if format & 0x3fff_0000 != 0 || (!small && direct) {
            return Err("unsupported Objective-C method list flags".into());
        }
        let stride = (format & 0x0000_fffc) as usize;
        if stride != if small { 12 } else { 24 } {
            return Err("invalid Objective-C method entry size".into());
        }
        if count > MAX_METHODS {
            return Err("Objective-C method count limit exceeded".into());
        }
        let base = add(address, 8)?;
        let entries = self.bytes(base, count * stride)?;
        let mut methods = Vec::with_capacity(count);
        for i in 0..count {
            let entry_address = add(base, (i * stride) as u64)?;
            let e = &entries[i * stride..(i + 1) * stride];
            let (selector, types, implementation) = if small {
                let pointer = |j: usize| -> Result<u64, String> {
                    relative(
                        add(entry_address, (j * 4) as u64)?,
                        i32::from_le_bytes(e[j * 4..j * 4 + 4].try_into().unwrap()),
                    )
                };
                let selector = if direct {
                    if let Some(cache) = cache.filter(|c| address >= c.start && address < c.end) {
                        // dyld optimized direct selectors use offsets from the
                        // shared selector base, not from this method field.
                        add(
                            cache.base,
                            u32::from_le_bytes(e[..4].try_into().unwrap()) as u64,
                        )?
                    } else {
                        pointer(0)?
                    }
                } else {
                    pointer(0)?
                };
                (
                    if direct {
                        selector
                    } else {
                        self.u64(selector)?
                    },
                    pointer(1)?,
                    pointer(2)?,
                )
            } else {
                let pointer =
                    |j: usize| u64::from_le_bytes(e[j * 8..j * 8 + 8].try_into().unwrap());
                (pointer(0), pointer(1), pointer(2))
            };
            if implementation == 0 || implementation & 3 != 0 {
                return Err("null or unaligned Objective-C ARM64 implementation".into());
            }
            let selector_address = selector;
            let selector = self.string(selector)?;
            let types = self.string(types)?;
            if selector.is_empty() || types.is_empty() {
                return Err(format!("empty Objective-C method metadata at list {address:#x}, entry {entry_address:#x}, format {format:#x}: selector={selector:?}, types={types:?}"));
            }
            methods.push(Method {
                selector_address,
                selector,
                types,
                implementation,
            });
        }
        Ok(methods)
    }
}

/// Decode one unrealized Apple ARM64 class, including metaclasses when requested.
/// The caller must resolve superclass/isa identities and validate executable IMP
/// mappings before dispatch. Null superclass is meaningful only for RO_ROOT.
pub(super) fn read_class(
    address: u64,
    read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
) -> Result<Class, String> {
    read_class_with_cache(address, None, read)
}
pub(super) fn read_class_with_cache(
    address: u64,
    cache: Option<CacheSelectorContext>,
    read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
) -> Result<Class, String> {
    read_class_from_reader(address, cache, &mut Reader::new(read))
}
fn read_class_from_reader<F: FnMut(u64, usize) -> Result<Vec<u8>, String>>(
    address: u64,
    cache: Option<CacheSelectorContext>,
    r: &mut Reader<F>,
) -> Result<Class, String> {
    if address == 0 || address & 7 != 0 {
        return Err("invalid Objective-C class address".into());
    }
    let class = r.bytes(address, 40)?;
    let word = |i: usize| u64::from_le_bytes(class[i * 8..i * 8 + 8].try_into().unwrap());
    let isa = word(0);
    let superclass = word(1);
    if isa == 0 || isa & 7 != 0 || superclass & 7 != 0 {
        return Err("unbound or unaligned Objective-C class identity".into());
    }
    let data = word(4);
    // Low three class-data bits are ABI flags. High runtime fast flags and
    // realized class_rw_t are deliberately not silently interpreted as class_ro.
    if data & 0xffff_0000_0000_0000 != 0 {
        return Err("unsupported Objective-C class data flags".into());
    }
    let ro_address = data & !7;
    if ro_address == 0 {
        return Err("null Objective-C class data".into());
    }
    let ro = r.bytes(ro_address, 72)?;
    let flags = u32::from_le_bytes(ro[..4].try_into().unwrap());
    if flags & 0xc000_0000 != 0 {
        return Err("realized Objective-C class data is unsupported".into());
    }
    if superclass == 0 && flags & 2 == 0 {
        return Err("unresolved Objective-C superclass".into());
    }
    let instance_start = u32::from_le_bytes(ro[4..8].try_into().unwrap());
    let instance_size = u32::from_le_bytes(ro[8..12].try_into().unwrap());
    if instance_start > instance_size || instance_size > 16 * 1024 * 1024 {
        return Err("invalid Objective-C instance size".into());
    }
    let name_address = u64::from_le_bytes(ro[24..32].try_into().unwrap());
    let methods_address = u64::from_le_bytes(ro[32..40].try_into().unwrap());
    let name = r.string(name_address)?;
    if name.is_empty() {
        return Err("empty Objective-C class name".into());
    }
    let methods = r.methods_with_cache(methods_address, cache)?;
    Ok(Class {
        address,
        isa,
        superclass,
        name,
        flags,
        instance_start,
        instance_size,
        methods,
    })
}

/// Decode a method list independently, useful for validating an image before
/// imported superclass identities have been bound. This does not realize it.
pub(super) fn read_methods(
    address: u64,
    read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
) -> Result<Vec<Method>, String> {
    Reader::new(read).methods(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_direct_selector_uses_verified_shared_base_not_entry_address() {
        // Actual NSAssertionHandler header is 0xc000000f: small, direct,
        // fixed-up. Selector offsets are cache-base-relative, while types and
        // IMP remain relative to their individual fields.
        let mut bytes = vec![0u8; 512];
        bytes[0..4].copy_from_slice(&0xc000000fu32.to_le_bytes());
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&16u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&116i32.to_le_bytes());
        bytes[16..20].copy_from_slice(&240i32.to_le_bytes());
        bytes[128..140].copy_from_slice(b"v16@0:8\0\0\0\0\0");
        bytes[272..278].copy_from_slice(b"value\0");
        let base = 0x1000u64;
        let read = |a: u64, n: usize| {
            bytes
                .get((a - base) as usize..(a - base) as usize + n)
                .map(|s| s.to_vec())
                .ok_or("unmapped".into())
        };
        let mut reader = Reader::new(read);
        let context = CacheSelectorContext {
            start: base,
            end: base + 512,
            base: base + 256,
        };
        let methods = reader.methods_with_cache(base, Some(context)).unwrap();
        assert_eq!(methods[0].selector, "value");
        assert_eq!(methods[0].selector_address, base + 272);
        assert_eq!(methods[0].types, "v16@0:8");
        assert_eq!(methods[0].implementation, base + 256);
        let mut reader = Reader::new(read);
        assert!(reader.methods(base).is_err());
        assert!(CacheSelectorContext {
            start: base,
            end: base + 512,
            base: base + 512
        }
        .validate()
        .is_err());
    }
    const BASE: u64 = 0x1_0000_0000;
    fn put32(b: &mut [u8], offset: usize, value: u32) {
        b[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(b: &mut [u8], offset: usize, value: u64) {
        b[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn read(b: &[u8], address: u64, size: usize) -> Result<Vec<u8>, String> {
        let offset = address.checked_sub(BASE).ok_or("unmapped")? as usize;
        b.get(offset..offset.checked_add(size).ok_or("overflow")?)
            .map(|x| x.to_vec())
            .ok_or_else(|| "unmapped".into())
    }
    fn fixture() -> Vec<u8> {
        let mut b = vec![0; 512];
        put64(&mut b, 0, BASE + 40); // isa
        put64(&mut b, 32, BASE + 80);
        put32(&mut b, 80, 2); // RO_ROOT
        put32(&mut b, 84, 8);
        put32(&mut b, 88, 24);
        put64(&mut b, 104, BASE + 160);
        put64(&mut b, 112, BASE + 192);
        b[160..168].copy_from_slice(b"Fixture\0");
        put32(&mut b, 192, 24);
        put32(&mut b, 196, 1);
        put64(&mut b, 200, BASE + 240);
        put64(&mut b, 208, BASE + 260);
        put64(&mut b, 216, BASE + 400);
        b[240..246].copy_from_slice(b"value\0");
        b[260..268].copy_from_slice(b"Q16@0:8\0");
        b
    }
    #[test]
    fn absolute_lp64_preserves_full_guest_addresses() {
        // Same 24-byte method stride and 40-byte class layout observed in the
        // RE4 image; all fixture names/code addresses are independently created.
        let b = fixture();
        let c = read_class(BASE, |a, n| read(&b, a, n)).unwrap();
        assert_eq!(c.name, "Fixture");
        assert_eq!(
            c.methods,
            vec![Method {
                selector_address: BASE + 240,
                selector: "value".into(),
                types: "Q16@0:8".into(),
                implementation: BASE + 400
            }]
        );
        assert_eq!(c.instance_size, 24);
    }
    #[test]
    fn relative_methods_use_each_fields_address_and_selector_indirection() {
        let mut b = fixture();
        put32(&mut b, 192, 0x8000_000c);
        put64(&mut b, 176, BASE + 240);
        put32(&mut b, 200, (-24i32) as u32); // name field -> SEL slot
        put32(&mut b, 204, 56); // types field -> 260
        put32(&mut b, 208, 192); // IMP field -> 400
        let c = read_class(BASE, |a, n| read(&b, a, n)).unwrap();
        assert_eq!(c.methods[0].selector, "value");
        assert_eq!(c.methods[0].implementation, BASE + 400);
        put32(&mut b, 192, 0xc000_000c);
        put32(&mut b, 200, 40); // direct selector string
        assert_eq!(
            read_class(BASE, |a, n| read(&b, a, n)).unwrap().methods[0].selector,
            "value"
        );
    }
    #[test]
    fn malformed_metadata_never_produces_a_partial_class() {
        for truncate in [0, 39, 151, 207, 267] {
            let b = fixture();
            assert!(read_class(BASE, |a, n| read(&b[..truncate], a, n)).is_err());
        }
        let mut b = fixture();
        put32(&mut b, 196, MAX_METHODS as u32 + 1);
        assert!(read_class(BASE, |a, n| read(&b, a, n))
            .unwrap_err()
            .contains("count limit"));
        put32(&mut b, 196, 1);
        put32(&mut b, 192, 16);
        assert!(read_class(BASE, |a, n| read(&b, a, n)).is_err());
        put32(&mut b, 192, 24);
        put32(&mut b, 80, 0); // missing imported superclass is not a root
        assert!(read_class(BASE, |a, n| read(&b, a, n))
            .unwrap_err()
            .contains("unresolved"));
        assert!(read_class(BASE, |_, n| Ok(vec![0; n.saturating_sub(1)]))
            .unwrap_err()
            .contains("short"));
    }
    #[test]
    fn string_and_pointer_limits_are_checked() {
        let mut r = Reader::new(|_: u64, n: usize| Ok(vec![b'a'; n]));
        assert!(r.string(BASE).unwrap_err().contains("oversized"));
        assert!(relative(1, -2).is_err());
        assert!(relative(u64::MAX, 1).is_err());
        assert_eq!(relative(0x8000_0000, i32::MIN).unwrap(), 0);
    }
    #[test]
    fn shared_metadata_strings_are_read_once_but_errors_are_not_cached() {
        let mut calls = 0;
        let mut reader = Reader::new(|address: u64, n: usize| {
            calls += 1;
            let data = b"value\0";
            data.get(address as usize..address as usize + n)
                .map(|b| b.to_vec())
                .ok_or("unmapped".into())
        });
        // Nonzero pointer with an overlapping immutable string suffix.
        assert_eq!(reader.string(1).unwrap(), "alue");
        let budget = reader.strings;
        assert_eq!(reader.string(1).unwrap(), "alue");
        assert_eq!(reader.strings, budget);
        drop(reader);
        assert_eq!(calls, 5);
    }
}

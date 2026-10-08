/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bounded discovery of Apple LP64 Objective-C registration sections.

use super::{add, Reader};
use std::collections::BTreeSet;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ImageSlots {
    pub class_list_slots: Vec<u64>,
    pub selector_ref_slots: Vec<u64>,
    pub class_ref_slots: Vec<u64>,
    pub non_lazy_class_slots: Vec<u64>,
    pub category_list_slots: Vec<u64>,
    pub non_lazy_category_slots: Vec<u64>,
    pub protocol_list_slots: Vec<u64>,
}

impl ImageSlots {
    /// Read class identities only after rebases/binds. Non-lazy classes are
    /// included, but this does not execute their +load methods. Categories and
    /// protocols remain separately visible and need real runtime registration.
    pub(crate) fn class_addresses(
        &self,
        read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
    ) -> Result<Vec<u64>, String> {
        let mut reader = Reader { read, strings: 0, string_cache: Default::default() };
        let mut addresses = Vec::new();
        let mut seen = BTreeSet::new();
        for &slot in self
            .class_list_slots
            .iter()
            .chain(&self.non_lazy_class_slots)
        {
            let address = reader.u64(slot)?;
            if address == 0 || address & 7 != 0 {
                return Err(format!(
                    "unresolved or unaligned Objective-C class at slot {slot:#x}"
                ));
            }
            if seen.insert(address) {
                addresses.push(address);
            }
        }
        Ok(addresses)
    }
}

fn bytes(file: &[u8], at: usize, size: usize) -> Result<&[u8], String> {
    file.get(
        at..at
            .checked_add(size)
            .ok_or("Objective-C Mach-O offset overflow")?,
    )
    .ok_or_else(|| "truncated Objective-C Mach-O metadata".into())
}
fn u32at(file: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(bytes(file, at, 4)?.try_into().unwrap()))
}
fn u64at(file: &[u8], at: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(bytes(file, at, 8)?.try_into().unwrap()))
}

/// `file` is the selected thin ARM64 image. Section slot addresses include the
/// image's mapping slide; pointer values must be read from post-fixup guest RAM.
pub(crate) fn image_slots(file: &[u8], slide: u64) -> Result<ImageSlots, String> {
    if u32at(file, 0)? != 0xfeed_facf || u32at(file, 4)? != 0x0100_000c {
        return Err("Objective-C scanner requires a thin little-endian ARM64 Mach-O".into());
    }
    bytes(file, 0, 32)?;
    let count = u32at(file, 16)? as usize;
    let command_bytes = u32at(file, 20)? as usize;
    if count > 4096 || command_bytes > 8 * 1024 * 1024 {
        return Err("Objective-C Mach-O command limit exceeded".into());
    }
    bytes(file, 32, command_bytes)?;
    let end = 32 + command_bytes;
    let mut command = 32;
    let mut slots = ImageSlots::default();
    for _ in 0..count {
        if command > end || end - command < 8 {
            return Err("truncated Objective-C load command".into());
        }
        let kind = u32at(file, command)?;
        let size = u32at(file, command + 4)? as usize;
        if size < 8 || size & 7 != 0 || size > end - command {
            return Err("invalid Objective-C load command size".into());
        }
        if kind == 0x19 {
            if size < 72 {
                return Err("truncated Objective-C segment command".into());
            }
            let sections = u32at(file, command + 64)? as usize;
            if sections > (size - 72) / 80 {
                return Err("truncated Objective-C section headers".into());
            }
            let vmaddr = u64at(file, command + 24)?;
            let vmsize = u64at(file, command + 32)?;
            let fileoff = u64at(file, command + 40)?;
            let filesize = u64at(file, command + 48)?;
            let vmend = add(vmaddr, vmsize)?;
            if filesize > vmsize || add(fileoff, filesize)? > file.len() as u64 {
                return Err("invalid Objective-C segment file range".into());
            }
            for index in 0..sections {
                let section = command + 72 + index * 80;
                let name = bytes(file, section, 16)?;
                let name = name.split(|&b| b == 0).next().unwrap();
                let (destination, cap): (&mut Vec<u64>, usize) = match name {
                    b"__objc_classlist" => (&mut slots.class_list_slots, 4096),
                    b"__objc_nlclslist" => (&mut slots.non_lazy_class_slots, 4096),
                    b"__objc_selrefs" => (&mut slots.selector_ref_slots, 65536),
                    b"__objc_classrefs" => (&mut slots.class_ref_slots, 65536),
                    b"__objc_catlist" => (&mut slots.category_list_slots, 4096),
                    b"__objc_nlcatlist" => (&mut slots.non_lazy_category_slots, 4096),
                    b"__objc_protolist" => (&mut slots.protocol_list_slots, 4096),
                    _ => continue,
                };
                if bytes(file, section + 16, 16)? != bytes(file, command + 8, 16)? {
                    return Err("Objective-C section segment name mismatch".into());
                }
                let address = u64at(file, section + 32)?;
                let length = u64at(file, section + 40)?;
                let offset = u32at(file, section + 48)? as u64;
                let section_type = u32at(file, section + 64)? & 0xff;
                // Apple emits selrefs as S_LITERAL_POINTERS and older protocol
                // lists as S_COALESCED; both are file-backed pointer arrays.
                let pointer_type = section_type == 0
                    || (name == b"__objc_selrefs" && section_type == 5)
                    || (name == b"__objc_protolist" && section_type == 11);
                if !pointer_type || address & 7 != 0 || length & 7 != 0 {
                    return Err("invalid Objective-C pointer section type/alignment".into());
                }
                let relative = address
                    .checked_sub(vmaddr)
                    .ok_or("Objective-C section precedes segment")?;
                if add(address, length)? > vmend
                    || add(relative, length)? > filesize
                    || (length != 0 && offset != add(fileoff, relative)?)
                {
                    return Err("Objective-C section outside file-backed segment".into());
                }
                let entries = usize::try_from(length / 8)
                    .map_err(|_| "Objective-C section count overflow")?;
                if entries > cap.saturating_sub(destination.len()) {
                    return Err("Objective-C section entry limit exceeded".into());
                }
                let mapped = add(address, slide)?;
                add(mapped, length)?;
                for index in 0..entries {
                    destination.push(add(mapped, index as u64 * 8)?);
                }
            }
        }
        command += size;
    }
    if command != end {
        return Err("Objective-C command count/size mismatch".into());
    }
    Ok(slots)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn put32(b: &mut [u8], at: usize, value: u32) {
        b[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(b: &mut [u8], at: usize, value: u64) {
        b[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut b = vec![0; 512];
        put32(&mut b, 0, 0xfeed_facf);
        put32(&mut b, 4, 0x0100_000c);
        put32(&mut b, 16, 1);
        put32(&mut b, 20, 232);
        put32(&mut b, 32, 0x19);
        put32(&mut b, 36, 232);
        b[40..46].copy_from_slice(b"__DATA");
        put64(&mut b, 56, 0x1_0000_0000);
        put64(&mut b, 64, 512);
        put64(&mut b, 80, 512);
        put32(&mut b, 96, 2);
        for (section, name, address, length) in [
            (104, b"__objc_classlist".as_slice(), 320, 16),
            (184, b"__objc_selrefs".as_slice(), 336, 8),
        ] {
            b[section..section + name.len()].copy_from_slice(name);
            b[section + 16..section + 22].copy_from_slice(b"__DATA");
            put64(&mut b, section + 32, 0x1_0000_0000 + address);
            put64(&mut b, section + 40, length);
            put32(&mut b, section + 48, address as u32);
        }
        b
    }
    #[test]
    fn discovery_preserves_slide_and_deduplicates_postfixup_classes() {
        let b = fixture();
        let s = image_slots(&b, 0x10000).unwrap();
        assert_eq!(s.class_list_slots, vec![0x1_0001_0140, 0x1_0001_0148]);
        assert_eq!(s.selector_ref_slots, vec![0x1_0001_0150]);
        let addresses = s
            .class_addresses(|_, n| {
                assert_eq!(n, 8);
                Ok(0x2_0000_0000u64.to_le_bytes().to_vec())
            })
            .unwrap();
        assert_eq!(addresses, vec![0x2_0000_0000]);
        assert!(s.class_addresses(|_, _| Ok(vec![0; 8])).is_err());
    }
    #[test]
    fn invalid_commands_sections_and_overflow_are_rejected() {
        for length in [0, 31, 103, 263] {
            assert!(image_slots(&fixture()[..length], 0).is_err());
        }
        let mut b = fixture();
        put32(&mut b, 36, 240);
        assert!(image_slots(&b, 0).is_err());
        let mut b = fixture();
        put64(&mut b, 104 + 40, 24);
        put64(&mut b, 104 + 32, 0x1_0000_01f8);
        assert!(image_slots(&b, 0).is_err());
        let mut b = fixture();
        put32(&mut b, 104 + 64, 1);
        assert!(image_slots(&b, 0).is_err());
        let mut b = fixture();
        put32(&mut b, 104 + 48, 0);
        assert!(image_slots(&b, 0).is_err());
        assert!(image_slots(&fixture(), u64::MAX).is_err());
    }

    #[test]
    fn actual_apple_selector_and_protocol_pointer_section_types() {
        let mut b = fixture();
        put32(&mut b, 184 + 64, 0x1000_0005);
        assert_eq!(image_slots(&b, 0).unwrap().selector_ref_slots.len(), 1);
        b[184..200].fill(0);
        b[184..200].copy_from_slice(b"__objc_protolist");
        put32(&mut b, 184 + 64, 0x1000_000b);
        assert_eq!(image_slots(&b, 0).unwrap().protocol_list_slots.len(), 1);
        put32(&mut b, 104 + 64, 5);
        assert!(image_slots(&b, 0).is_err());
    }
}

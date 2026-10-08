/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Register original mapped images after binding, while guest execution stops.
//! No class addresses or Apple initialization flags are synthesized. Selector
//! writes are returned as a transaction plan, not applied to protected memory.
use super::objc_execution::Initialization;
use super::objc_metadata::{image_slots, CacheSelectorContext, Registry, SelectorFixup};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct MappedImage<'a> {
    pub identity: &'a str,
    /// Selected thin ARM64 image bytes, not an unsliced FAT container.
    pub file: &'a [u8],
    pub slide: u64,
    /// Only slots proven weak by the actual binding metadata may remain nil.
    pub weak_class_ref_slots: &'a BTreeSet<u64>,
}
#[derive(Debug)]
pub(super) struct RegisteredImage {
    pub identity: String,
    pub classes: Vec<u64>,
    pub class_references: Vec<(u64, u64)>,
    pub selector_ref_count: usize,
    /// Protocol metadata remains visible; conformance/introspection is not
    /// implemented by a successful method/class registration.
    pub protocol_count: usize,
    /// Registration never executes non-lazy +load or issues a load receipt.
    pub pending_load_classes: Vec<u64>,
}
pub(super) struct RegisteredImages {
    pub registry: Registry,
    pub initialization: Initialization,
    pub images: Vec<RegisteredImage>,
    pub class_owners: BTreeMap<u64, String>,
    pub selector_fixups: Vec<SelectorFixup>,
}
impl RegisteredImages {
    /// Exact mapped-image ownership for a bundle load receipt. This does not
    /// execute image initializers, +load or lazily triggered +initialize.
    pub(super) fn validate_image_class(&self, identity: &str, class: u64) -> Result<(), String> {
        let image = self
            .images
            .iter()
            .find(|image| image.identity == identity)
            .ok_or("bundle executable is not an Objective-C registered image")?;
        if !image.classes.contains(&class)
            || self.class_owners.get(&class).map(String::as_str) != Some(identity)
        {
            return Err(
                "principal class does not belong to the exact registered bundle executable".into(),
            );
        }
        Ok(())
    }

    pub(super) fn principal_class(&self, identity: &str, name: &str) -> Result<u64, String> {
        if name.is_empty() || name.len() > 4096 || name.contains('\0') {
            return Err("invalid bundle principal class name".into());
        }
        let class = self
            .registry
            .lookup_class(name)
            .ok_or("bundle principal class is not registered")?;
        self.validate_image_class(identity, class)?;
        Ok(class)
    }
}

fn pointer(
    slot: u64,
    read: &mut impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
) -> Result<u64, String> {
    let bytes = read(slot, 8)?;
    if bytes.len() != 8 {
        return Err(format!("short Objective-C pointer read at {slot:#x}"));
    }
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

/// Cached regions describe mapped cache VM intervals [start,end), never a
/// guessed slide/base. `executable` validates physical RX mappings, while the
/// execution bridge independently enforces which guest IMPs may actually run.
/// Reader sees rebased/bound guest memory; file offsets are never guest pointers.
pub(super) fn register_images(
    images: &[MappedImage<'_>],
    cached_regions: &[(u64, u64)],
    read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
    executable: impl FnMut(u64, usize) -> Result<(), String>,
) -> Result<RegisteredImages, String> {
    register_images_with_cache(images, cached_regions, None, read, executable)
}
pub(super) fn register_images_with_cache(
    images: &[MappedImage<'_>],
    cached_regions: &[(u64, u64)],
    cache: Option<CacheSelectorContext>,
    mut read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
    mut executable: impl FnMut(u64, usize) -> Result<(), String>,
) -> Result<RegisteredImages, String> {
    if let Some(cache) = cache {
        cache.validate()?;
    }
    if images.is_empty() || images.len() > 256 || cached_regions.len() > 4096 {
        return Err("Objective-C image registration count outside bounds".into());
    }
    for &(start, end) in cached_regions {
        if start >= end {
            return Err("invalid cached Objective-C VM interval".into());
        }
    }
    let mut records = Vec::new();
    let mut class_owners = BTreeMap::new();
    let mut all_classes = BTreeSet::new();
    let mut selectors = BTreeSet::new();
    let mut identities = BTreeSet::new();
    let mut total_slots = 0usize;
    for image in images {
        if image.identity.is_empty()
            || image.identity.len() > 4096
            || !identities.insert(image.identity)
        {
            return Err("invalid or duplicate Objective-C mapped-image identity".into());
        }
        let slots =
            image_slots(image.file, image.slide).map_err(|e| format!("{}: {e}", image.identity))?;
        for count in [
            slots.class_list_slots.len(),
            slots.non_lazy_class_slots.len(),
            slots.class_ref_slots.len(),
            slots.selector_ref_slots.len(),
            slots.protocol_list_slots.len(),
        ] {
            total_slots = total_slots
                .checked_add(count)
                .ok_or("Objective-C registration slot count overflow")?;
        }
        if total_slots > 131072 {
            return Err("combined Objective-C registration slot budget exceeded".into());
        }
        if !slots.category_list_slots.is_empty() || !slots.non_lazy_category_slots.is_empty() {
            return Err(format!(
                "{}: Objective-C category attachment/method replacement is unsupported",
                image.identity
            ));
        }
        let classes = slots
            .class_addresses(&mut read)
            .map_err(|e| format!("{}: {e}", image.identity))?;
        for &class in &classes {
            if class_owners
                .insert(class, image.identity.to_string())
                .is_some()
            {
                return Err(format!(
                    "Objective-C class {class:#x} is declared by multiple mapped images"
                ));
            }
            all_classes.insert(class);
        }
        let mut references = Vec::new();
        for &slot in &slots.class_ref_slots {
            let address =
                pointer(slot, &mut read).map_err(|e| format!("{}: {e}", image.identity))?;
            if address == 0 {
                if !image.weak_class_ref_slots.contains(&slot) {
                    return Err(format!(
                        "{}: unresolved required Objective-C class reference at {slot:#x}",
                        image.identity
                    ));
                }
            } else {
                all_classes.insert(address);
            }
            references.push((slot, address));
        }
        for &slot in &slots.selector_ref_slots {
            selectors.insert(slot);
        }
        if all_classes.len() > 4096 || selectors.len() > 65536 {
            return Err("combined Objective-C mapped-image class/selector limit exceeded".into());
        }
        let mut pending_load_classes = Vec::new();
        for &slot in &slots.non_lazy_class_slots {
            pending_load_classes.push(pointer(slot, &mut read)?);
        }
        records.push(RegisteredImage {
            identity: image.identity.to_string(),
            classes,
            class_references: references,
            selector_ref_count: slots.selector_ref_slots.len(),
            protocol_count: slots.protocol_list_slots.len(),
            pending_load_classes,
        });
    }
    // Registry already traverses actual isa/superclass closure, enforces its
    // graph limits, validates every IMP and checks root topology. Decode once
    // through its shared immutable string reader instead of duplicating that
    // traversal here with separate caches.
    let registered = Registry::register_with_cache(
        &all_classes.into_iter().collect::<Vec<_>>(),
        &selectors.into_iter().collect::<Vec<_>>(),
        cache,
        &mut read,
        &mut executable,
    )
    .map_err(|error| {
        let address = error
            .strip_prefix("Objective-C class ")
            .and_then(|rest| rest.split_once(':').map(|parts| parts.0))
            .and_then(|text| text.strip_prefix("0x"))
            .and_then(|text| u64::from_str_radix(text, 16).ok());
        let origin = if address.is_some_and(|address| {
            cached_regions
                .iter()
                .any(|&(start, end)| address >= start && address < end)
        }) {
            "cached framework"
        } else {
            "mapped image"
        };
        format!("unresolved/unsupported {origin} Objective-C registration: {error}")
    })?;
    let initialization = Initialization::new(registered.registry.classes().cloned())?;
    Ok(RegisteredImages {
        registry: registered.registry,
        initialization,
        images: records,
        class_owners,
        selector_fixups: registered.selector_fixups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn put64(b: &mut [u8], at: usize, v: u64) {
        b[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    fn put32(b: &mut [u8], at: usize, v: u32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut b = vec![0; 4096];
        put32(&mut b, 0, 0xfeedfacf);
        put32(&mut b, 4, 0x0100000c);
        put32(&mut b, 16, 1);
        put32(&mut b, 20, 232);
        put32(&mut b, 32, 0x19);
        put32(&mut b, 36, 232);
        b[40..46].copy_from_slice(b"__DATA");
        put64(&mut b, 56, 0x10000);
        put64(&mut b, 64, 4096);
        put64(&mut b, 80, 4096);
        put32(&mut b, 88, 3);
        put32(&mut b, 92, 3);
        put32(&mut b, 96, 2);
        for (at, name, addr, offset, flags) in [
            (104, b"__objc_classlist".as_slice(), 0x10200, 512, 0),
            (184, b"__objc_selrefs".as_slice(), 0x10208, 520, 5),
        ] {
            b[at..at + name.len()].copy_from_slice(name);
            b[at + 16..at + 22].copy_from_slice(b"__DATA");
            put64(&mut b, at + 32, addr);
            put64(&mut b, at + 40, 8);
            put32(&mut b, at + 48, offset);
            put32(&mut b, at + 64, flags);
        }
        put64(&mut b, 512, 0x10400);
        put64(&mut b, 520, 0x10700);
        for (at, isa, superclass, ro) in [
            (0x400, 0x10440, 0, 0x10500),
            (0x440, 0x10440, 0x10400, 0x10580),
        ] {
            put64(&mut b, at, isa);
            put64(&mut b, at + 8, superclass);
            put64(&mut b, at + 32, ro);
        }
        for (ro, flags, start, size, methods) in [(0x500, 2, 0, 8, 0x10800), (0x580, 3, 40, 40, 0)]
        {
            put32(&mut b, ro, flags);
            put32(&mut b, ro + 4, start);
            put32(&mut b, ro + 8, size);
            put64(&mut b, ro + 24, 0x10600);
            put64(&mut b, ro + 32, methods);
        }
        b[0x600..0x605].copy_from_slice(b"Root\0");
        b[0x700..0x706].copy_from_slice(b"value\0");
        b[0x720..0x728].copy_from_slice(b"I16@0:8\0");
        put32(&mut b, 0x800, 24);
        put32(&mut b, 0x804, 1);
        put64(&mut b, 0x808, 0x10700);
        put64(&mut b, 0x810, 0x10720);
        put64(&mut b, 0x818, 0x20000);
        b
    }
    fn register_fixture(bytes: &[u8], cache: &[(u64, u64)]) -> Result<RegisteredImages, String> {
        let weak = BTreeSet::new();
        register_images(
            &[MappedImage {
                identity: "Payload/Test.app/Test",
                file: bytes,
                slide: 0,
                weak_class_ref_slots: &weak,
            }],
            cache,
            |address, n| {
                let offset = usize::try_from(
                    address
                        .checked_sub(0x10000)
                        .ok_or("unmapped fixture read")?,
                )
                .map_err(|_| "fixture offset overflow")?;
                bytes
                    .get(offset..offset.checked_add(n).ok_or("fixture read overflow")?)
                    .map(|b| b.to_vec())
                    .ok_or("unmapped fixture read".into())
            },
            |address, n| {
                if address == 0x20000 && n == 4 {
                    Ok(())
                } else {
                    Err("not fixture code".into())
                }
            },
        )
    }
    #[test]
    fn graph_class_headers_are_decoded_once_and_initialization_stays_pending() {
        let bytes = fixture();
        let weak = BTreeSet::new();
        let mut reads = BTreeMap::new();
        let registered = register_images(
            &[MappedImage {
                identity: "Payload/Test.app/Test",
                file: &bytes,
                slide: 0,
                weak_class_ref_slots: &weak,
            }],
            &[],
            |address, n| {
                if n == 40 && matches!(address, 0x10400 | 0x10440) {
                    *reads.entry(address).or_insert(0) += 1;
                }
                let offset = usize::try_from(
                    address
                        .checked_sub(0x10000)
                        .ok_or("unmapped fixture read")?,
                )
                .map_err(|_| "fixture offset overflow")?;
                bytes
                    .get(offset..offset.checked_add(n).ok_or("fixture read overflow")?)
                    .map(|data| data.to_vec())
                    .ok_or("unmapped fixture read".into())
            },
            |address, n| {
                if address == 0x20000 && n == 4 {
                    Ok(())
                } else {
                    Err("not fixture code".into())
                }
            },
        )
        .unwrap();
        assert_eq!(reads.get(&0x10400), Some(&1));
        assert_eq!(reads.get(&0x10440), Some(&1));
        assert!(!registered.initialization.is_initialized(0x10400));
    }
    #[test]
    fn mapped_classes_and_selectors_register_without_fake_initialization() {
        let b = fixture();
        let registered = register_fixture(&b, &[]).unwrap();
        assert_eq!(registered.registry.lookup_class("Root"), Some(0x10400));
        assert_eq!(registered.registry.selector_named("value"), Some(0x10700));
        assert!(!registered.initialization.is_initialized(0x10400));
        assert_eq!(registered.images[0].classes, vec![0x10400]);
        assert_eq!(registered.class_owners[&0x10400], "Payload/Test.app/Test");
        assert!(registered.selector_fixups.is_empty());
        assert_eq!(
            registered
                .principal_class("Payload/Test.app/Test", "Root")
                .unwrap(),
            0x10400
        );
        assert!(registered
            .principal_class("different/framework/executable", "Root")
            .is_err());
        assert!(registered
            .validate_image_class("Payload/Test.app/Test", 0x10440)
            .is_err());
        assert!(registered
            .principal_class("Payload/Test.app/Test", "Unavailable")
            .is_err());
    }
    #[test]
    fn unresolved_cached_superclass_reports_real_address_and_categories_reject() {
        let mut b = fixture();
        put32(&mut b, 0x500, 0);
        put32(&mut b, 0x504, 8);
        put64(&mut b, 0x408, 0x90000);
        let error = match register_fixture(&b, &[(0x90000, 0x91000)]) {
            Err(e) => e,
            Ok(_) => panic!("cached superclass must fail"),
        };
        assert!(error.contains("cached framework"));
        assert!(error.contains("0x90000"));
        let mut category = fixture();
        category[104..120].fill(0);
        category[104..119].copy_from_slice(b"__objc_catlist\0");
        let error = match register_fixture(&category, &[]) {
            Err(e) => e,
            Ok(_) => panic!("category attachment must fail"),
        };
        assert!(error.contains("category attachment"));
    }
    #[test]
    fn registration_rejects_empty_inputs_and_invalid_cache_bounds_before_reading() {
        let empty = BTreeSet::new();
        assert!(register_images(&[], &[], |_, _| panic!("must not read"), |_, _| Ok(())).is_err());
        let images = [MappedImage {
            identity: "image",
            file: &[],
            slide: 0,
            weak_class_ref_slots: &empty,
        }];
        assert!(register_images(
            &images,
            &[(10, 10)],
            |_, _| panic!("must not read"),
            |_, _| Ok(())
        )
        .is_err());
        assert!(register_images(
            &images,
            &[],
            |_, _| panic!("malformed file must reject before guest read"),
            |_, _| Ok(())
        )
        .is_err());
    }
}

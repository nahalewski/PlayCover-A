/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Post-binding registration and typed +load plans. No execution receipts are
//! issued here. Apple objc4 objc-loadmethod.mm calls direct IMPs with (Class,SEL)
//! superclass first; inherited +load is not dispatched as a subclass message.
use super::bundle_metadata::LoadMethod;
use super::objc_metadata::{CacheSelectorContext, Class};
use super::objc_registration::{register_images_with_cache, MappedImage, RegisteredImages};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub(super) struct ImageLoad {
    pub identity: String,
    pub method: LoadMethod,
}
pub(super) struct ImageLoads {
    pub registered: RegisteredImages,
    /// Global superclass-first order; do not reorder by image filename.
    pub ordered: Vec<ImageLoad>,
}

pub(super) fn prepare(
    images: &[MappedImage<'_>],
    cached_regions: &[(u64, u64)],
    read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
    executable: impl FnMut(u64, usize) -> Result<(), String>,
) -> Result<ImageLoads, String> {
    prepare_with_cache(images, cached_regions, None, read, executable)
}
pub(super) fn prepare_with_cache(
    images: &[MappedImage<'_>],
    cached_regions: &[(u64, u64)],
    cache: Option<CacheSelectorContext>,
    read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
    executable: impl FnMut(u64, usize) -> Result<(), String>,
) -> Result<ImageLoads, String> {
    let registered = register_images_with_cache(images, cached_regions, cache, read, executable)?;
    let classes: BTreeMap<_, _> = registered
        .registry
        .classes()
        .map(|c| (c.address, c))
        .collect();
    let candidates: Vec<_> = registered
        .images
        .iter()
        .flat_map(|i| i.pending_load_classes.iter().copied())
        .collect();
    let selector = registered.registry.selector_named("load");
    let ordered = plan(&classes, &registered.class_owners, &candidates, selector)?;
    Ok(ImageLoads {
        registered,
        ordered,
    })
}

fn plan(
    classes: &BTreeMap<u64, &Class>,
    owners: &BTreeMap<u64, String>,
    candidates: &[u64],
    selector: Option<u64>,
) -> Result<Vec<ImageLoad>, String> {
    if candidates.len() > 4096 || classes.len() > 4096 {
        return Err("Objective-C +load graph budget exceeded".into());
    }
    let mut ordered = Vec::new();
    let mut completed = BTreeSet::new();
    let mut unique = BTreeSet::new();
    for &candidate in candidates {
        if !unique.insert(candidate) || !owners.contains_key(&candidate) {
            return Err("duplicate or foreign non-lazy Objective-C class".into());
        }
        let mut chain = Vec::new();
        let mut visiting = BTreeSet::new();
        let mut cursor = candidate;
        while cursor != 0 && !completed.contains(&cursor) {
            if !visiting.insert(cursor) || chain.len() >= 4096 {
                return Err("Objective-C +load superclass cycle/budget exceeded".into());
            }
            let class = classes
                .get(&cursor)
                .ok_or("unregistered +load superclass")?;
            if class.flags & 1 != 0 {
                return Err("metaclass in non-lazy Objective-C class graph".into());
            }
            chain.push(cursor);
            cursor = class.superclass;
        }
        for address in chain.into_iter().rev() {
            let class = classes[&address];
            let meta = classes
                .get(&class.isa)
                .ok_or("unregistered +load metaclass")?;
            let mut methods = meta.methods.iter().filter(|m| m.selector == "load");
            if let Some(method) = methods.next() {
                if methods.next().is_some() || !matches!(method.types.as_str(), "v16@0:8" | "v@:") {
                    return Err("ambiguous or unsupported Objective-C +load ABI".into());
                }
                let identity = owners.get(&address).ok_or(
                    "external superclass +load requires genuine provider execution receipt",
                )?;
                let selector = selector
                    .filter(|&s| s != 0)
                    .ok_or("unregistered +load selector")?;
                ordered.push(ImageLoad {
                    identity: identity.clone(),
                    method: LoadMethod {
                        receiver: address,
                        selector,
                        imp: method.implementation,
                    },
                });
            }
            completed.insert(address);
        }
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::super::objc_metadata::Method;
    use super::*;
    fn class(address: u64, isa: u64, superclass: u64, meta: bool, load: bool) -> Class {
        Class {
            address,
            isa,
            superclass,
            name: format!("Class{address}"),
            flags: u32::from(meta),
            instance_start: 8,
            instance_size: 8,
            methods: if load {
                vec![Method {
                    selector_address: 99,
                    selector: "load".into(),
                    types: "v16@0:8".into(),
                    implementation: address + 0x1000,
                }]
            } else {
                vec![]
            },
        }
    }
    #[test]
    fn superclass_first_direct_load_once_and_no_inherited_subclass_call() {
        let values = [
            class(1, 11, 0, false, false),
            class(11, 11, 1, true, true),
            class(2, 12, 1, false, false),
            class(12, 11, 11, true, true),
            class(3, 13, 2, false, false),
            class(13, 11, 12, true, false),
        ];
        let classes = values.iter().map(|c| (c.address, c)).collect();
        let owners = [(1, "base".into()), (2, "unity".into()), (3, "unity".into())].into();
        let result = plan(&classes, &owners, &[3, 2, 1], Some(99)).unwrap();
        assert_eq!(
            result.iter().map(|r| r.method.receiver).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(result[0].identity, "base");
        assert_eq!(result[1].method.imp, 0x100c);
    }
    #[test]
    fn foreign_super_load_and_invalid_abi_are_explicit_errors() {
        let mut values = [class(1, 11, 0, false, false), class(11, 11, 1, true, true)];
        let owners = [(1, "unity".into())].into();
        let classes = values.iter().map(|c| (c.address, c)).collect();
        assert!(plan(&classes, &BTreeMap::new(), &[1], Some(99)).is_err());
        assert!(plan(&classes, &owners, &[1, 1], Some(99)).is_err());
        values[1].methods[0].types = "i16@0:8".into();
        let classes = values.iter().map(|c| (c.address, c)).collect();
        assert!(plan(&classes, &owners, &[1], Some(99)).is_err());
    }
}

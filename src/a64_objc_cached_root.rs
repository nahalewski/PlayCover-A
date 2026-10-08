/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Inspect the exact exported cached NSObject pair. This validates metadata
//! only: it grants no cached-code execution or runtime initialization receipt.
use super::objc_metadata::{read_class, Class};

pub(super) struct ExportEvidence<'a> {
    pub provider: &'a str,
    pub name: &'a str,
    pub address: u64,
    pub weak: bool,
}
#[derive(Debug)]
pub(super) struct CachedRootMetadata {
    pub class: Class,
    pub metaclass: Class,
}
fn evidence(value: &ExportEvidence<'_>, expected: &str) -> Result<(), String> {
    if value.provider != "/usr/lib/libobjc.A.dylib"
        || value.name != expected
        || value.weak
        || value.address == 0
        || value.address & 7 != 0
    {
        return Err("cached NSObject requires exact strong libobjc export evidence".into());
    }
    Ok(())
}
pub(super) fn inspect(
    class: ExportEvidence<'_>,
    metaclass: ExportEvidence<'_>,
    mut read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
    mut cache_readable: impl FnMut(u64, usize) -> Result<(), String>,
    mut physical_cache_rx: impl FnMut(u64, usize) -> Result<(), String>,
) -> Result<CachedRootMetadata, String> {
    evidence(&class, "_OBJC_CLASS_$_NSObject")?;
    evidence(&metaclass, "_OBJC_METACLASS_$_NSObject")?;
    if class.address == metaclass.address {
        return Err("cached NSObject class and metaclass identities coincide".into());
    }
    let mut bounded_read = |address, length| {
        cache_readable(address, length)?;
        read(address, length)
    };
    let class_data = read_class(class.address, &mut bounded_read).map_err(|e| {
        format!(
            "cached NSObject class metadata at {:#x}: {e}",
            class.address
        )
    })?;
    let meta_data = read_class(metaclass.address, &mut bounded_read).map_err(|e| {
        format!(
            "cached NSObject metaclass metadata at {:#x}: {e}",
            metaclass.address
        )
    })?;
    validate_pair(&class_data, &meta_data, &mut physical_cache_rx)?;
    Ok(CachedRootMetadata {
        class: class_data,
        metaclass: meta_data,
    })
}
fn validate_pair(
    class: &Class,
    meta: &Class,
    rx: &mut impl FnMut(u64, usize) -> Result<(), String>,
) -> Result<(), String> {
    if class.name != "NSObject"
        || meta.name != "NSObject"
        || class.flags & 3 != 2
        || meta.flags & 3 != 3
        || class.superclass != 0
        || class.isa != meta.address
        || meta.isa != meta.address
        || meta.superclass != class.address
    {
        return Err(
            "cached NSObject exported class/metaclass graph does not match root ABI".into(),
        );
    }
    for owner in [class, meta] {
        for method in &owner.methods {
            rx(method.implementation, 4).map_err(|e| {
                format!(
                    "cached NSObject metadata IMP {} is not physically cache RX: {e}",
                    method.selector
                )
            })?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::super::objc_metadata::Method;
    use super::*;
    fn pair() -> (Class, Class) {
        let class = Class {
            address: 8,
            isa: 16,
            superclass: 0,
            name: "NSObject".into(),
            flags: 2,
            instance_start: 0,
            instance_size: 8,
            methods: vec![Method {
                selector_address: 24,
                selector: "init".into(),
                types: "@16@0:8".into(),
                implementation: 0x1000,
            }],
        };
        let meta = Class {
            address: 16,
            isa: 16,
            superclass: 8,
            name: "NSObject".into(),
            flags: 3,
            instance_start: 40,
            instance_size: 40,
            methods: vec![],
        };
        (class, meta)
    }
    #[test]
    fn metadata_rx_validation_does_not_execute_or_initialize() {
        let (class, meta) = pair();
        let mut checked = vec![];
        validate_pair(&class, &meta, &mut |a, n| {
            checked.push((a, n));
            Ok(())
        })
        .unwrap();
        assert_eq!(checked, vec![(0x1000, 4)]);
        assert!(validate_pair(&class, &meta, &mut |_, _| Err("not RX".into())).is_err());
        let mut bad = meta;
        bad.superclass = 16;
        assert!(validate_pair(&class, &bad, &mut |_, _| panic!("graph rejects first")).is_err());
    }
    #[test]
    fn wrong_export_evidence_rejects_before_guest_reads() {
        let bad = ExportEvidence {
            provider: "Foundation",
            name: "_OBJC_CLASS_$_NSObject",
            address: 8,
            weak: false,
        };
        let meta = ExportEvidence {
            provider: "/usr/lib/libobjc.A.dylib",
            name: "_OBJC_METACLASS_$_NSObject",
            address: 16,
            weak: false,
        };
        assert!(inspect(
            bad,
            meta,
            |_, _| panic!("must not read"),
            |_, _| Ok(()),
            |_, _| Ok(())
        )
        .is_err());
    }
}

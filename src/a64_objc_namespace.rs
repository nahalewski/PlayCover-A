/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Emulator-owned, cold ObjC metadata for precisely implemented classes.
//! These are not cached Apple class tokens. Methods reference actual guest
//! service trampolines or owned RX routines. NSObject +initialize's empty body
//! matches objc4 NSObject.mm; alloc/init/dealloc perform genuine lifecycle.
use super::A64Cpu;
use std::collections::{BTreeMap, BTreeSet};
const DATA_SIZE: usize = 65536;
const CODE_SIZE: usize = 4096;

pub(super) struct MethodSpec<'a> {
    pub selector: &'a str,
    pub types: &'a str,
    pub implementation: u64,
}
pub(super) struct ClassSpec<'a> {
    pub name: &'a str,
    /// Parent must precede child; the single root is explicitly NSObject.
    pub parent: Option<&'a str>,
    pub instance_size: u32,
    pub instance_methods: Vec<MethodSpec<'a>>,
    pub class_methods: Vec<MethodSpec<'a>>,
}
pub(super) struct Namespace {
    pub class_addresses: Vec<u64>,
    classes: BTreeMap<String, (u64, u64)>,
    selectors: BTreeMap<String, u64>,
    code: u64,
    linked: bool,
}
struct Data {
    base: u64,
    bytes: Vec<u8>,
    strings: BTreeMap<String, u64>,
}
impl Data {
    fn reserve(&mut self, size: usize) -> Result<u64, String> {
        let start = self
            .bytes
            .len()
            .checked_add(7)
            .ok_or("ObjC namespace alignment overflow")?
            & !7;
        let end = start
            .checked_add(size)
            .ok_or("ObjC namespace size overflow")?;
        if end > DATA_SIZE {
            return Err("ObjC namespace metadata budget exceeded".into());
        }
        self.bytes.resize(end, 0);
        Ok(self.base + start as u64)
    }
    fn string(&mut self, s: &str) -> Result<u64, String> {
        if s.is_empty() || s.len() > 4096 || s.contains('\0') {
            return Err("invalid ObjC namespace string".into());
        }
        if let Some(&address) = self.strings.get(s) {
            return Ok(address);
        }
        let address = self.reserve(s.len() + 1)?;
        let at = (address - self.base) as usize;
        self.bytes[at..at + s.len()].copy_from_slice(s.as_bytes());
        self.strings.insert(s.into(), address);
        Ok(address)
    }
    fn u64(&mut self, address: u64, value: u64) {
        let at = (address - self.base) as usize;
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, address: u64, value: u32) {
        let at = (address - self.base) as usize;
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn methods(
        &mut self,
        methods: &[(String, String, u64)],
        selectors: &mut BTreeMap<String, u64>,
    ) -> Result<u64, String> {
        if methods.is_empty() {
            return Ok(0);
        }
        let address = self.reserve(8 + methods.len() * 24)?;
        self.u32(address, 24);
        self.u32(address + 4, methods.len() as u32);
        for (index, (selector, types, imp)) in methods.iter().enumerate() {
            let sel = self.string(selector)?;
            let ty = self.string(types)?;
            selectors.insert(selector.clone(), sel);
            let at = address + 8 + (index * 24) as u64;
            self.u64(at, sel);
            self.u64(at + 8, ty);
            self.u64(at + 16, *imp);
        }
        Ok(address)
    }
}
fn validate_imp(cpu: &A64Cpu, address: u64) -> Result<(), String> {
    if address == 0 || address & 3 != 0 {
        return Err("ObjC namespace method has invalid IMP".into());
    }
    for offset in 0..4 {
        if !cpu
            .mapped_permissions(
                address
                    .checked_add(offset)
                    .ok_or("ObjC IMP range overflow")?,
            )
            .is_some_and(|p| p & 4 != 0)
        {
            return Err("ObjC namespace IMP is not mapped executable code".into());
        }
    }
    Ok(())
}
impl Namespace {
    /// Startup's policy may authorize only this explicit emulator-owned RX
    /// interval in addition to original app code and registered trampolines.
    pub(super) fn code_range(&self) -> (u64, u64) {
        (self.code, self.code + CODE_SIZE as u64)
    }
    pub(super) fn class_address(&self, name: &str) -> Option<u64> {
        self.classes.get(name).map(|&(class, _)| class)
    }
    pub(super) fn metaclass_address(&self, name: &str) -> Option<u64> {
        self.classes.get(name).map(|&(_, meta)| meta)
    }
    pub(super) fn selector(&self, name: &str) -> Option<u64> {
        self.selectors.get(name).copied()
    }
    /// Only these exact, implemented emulator classes are substitutable. No
    /// wildcard/private class export or Foundation compatibility is claimed.
    pub(super) fn exports(&self) -> Result<BTreeMap<String, u64>, String> {
        self.require_linked()?;
        let mut result = BTreeMap::new();
        for (name, &(class, meta)) in &self.classes {
            result.insert(format!("_OBJC_CLASS_$_{name}"), class);
            result.insert(format!("_OBJC_METACLASS_$_{name}"), meta);
        }
        Ok(result)
    }
    pub(super) fn require_linked(&self) -> Result<(), String> {
        if self.linked {
            Ok(())
        } else {
            Err("owned NSObject allocation/disposal services are not linked".into())
        }
    }
    /// Link only known class_createInstance/object_dispose service IDs after
    /// Registry and ObjectRuntime construction, before any guest execution.
    pub(super) fn link_root_services(
        &mut self,
        cpu: &mut A64Cpu,
        create_instance: u64,
        dispose: u64,
    ) -> Result<(), String> {
        if self.linked {
            return Err("owned NSObject root services already linked".into());
        }
        validate_imp(cpu, create_instance)?;
        validate_imp(cpu, dispose)?;
        // Own RX code is immutable to guest writes. Host initialization writes
        // are bounded by our already mapped literal positions.
        cpu.try_write_bytes(self.code + 0x40, &create_instance.to_le_bytes())?;
        cpu.try_write_bytes(self.code + 0x80, &dispose.to_le_bytes())?;
        self.linked = true;
        Ok(())
    }

    pub(super) fn map(
        cpu: &mut A64Cpu,
        base: u64,
        specs: &[ClassSpec<'_>],
    ) -> Result<Self, String> {
        if base == 0 || base & 4095 != 0 || specs.is_empty() || specs.len() > 64 {
            return Err("invalid ObjC namespace placement/class count".into());
        }
        let code = base
            .checked_add(DATA_SIZE as u64)
            .ok_or("ObjC namespace range overflow")?;
        let end = code
            .checked_add(CODE_SIZE as u64)
            .ok_or("ObjC namespace range overflow")?;
        if end >> 63 != 0 {
            return Err("ObjC namespace address unsupported".into());
        }
        for address in base..end {
            if cpu.mapped_permissions(address).is_some() {
                return Err("ObjC namespace overlaps guest mapping".into());
            }
        }
        if specs[0].name != "NSObject" || specs[0].parent.is_some() || specs[0].instance_size != 8 {
            return Err("owned ObjC namespace requires plain NSObject root".into());
        }
        let mut data = Data {
            base,
            bytes: Vec::new(),
            strings: BTreeMap::new(),
        };
        let mut classes = BTreeMap::new();
        let mut sizes = BTreeMap::new();
        let mut selectors = BTreeMap::new();
        let mut addresses = Vec::new();
        let mut method_count = 0usize;
        for spec in specs {
            if spec.name.is_empty()
                || spec.name.len() > 255
                || !spec
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || classes.contains_key(spec.name)
            {
                return Err("invalid or duplicate owned ObjC class name".into());
            }
            let (parent, parent_meta, parent_size) = match spec.parent {
                Some(name) => {
                    let &(normal, meta) = classes
                        .get(name)
                        .ok_or("owned ObjC class parent is not yet defined")?;
                    (normal, meta, sizes[name])
                }
                None if classes.is_empty() => (0, 0, 0),
                None => return Err("owned ObjC namespace has multiple roots".into()),
            };
            if spec.instance_size < 8
                || spec.instance_size < parent_size
                || spec.instance_size > 1024 * 1024
            {
                return Err("owned ObjC class instance size invalid".into());
            }
            let mut instance = Vec::new();
            let mut class_methods = Vec::new();
            for (source, target) in [
                (&spec.instance_methods, &mut instance),
                (&spec.class_methods, &mut class_methods),
            ] {
                let mut seen = BTreeSet::new();
                for method in source {
                    if !seen.insert(method.selector) {
                        return Err("duplicate owned ObjC selector".into());
                    }
                    validate_imp(cpu, method.implementation)?;
                    data.string(method.selector)?;
                    data.string(method.types)?;
                    target.push((
                        method.selector.to_string(),
                        method.types.to_string(),
                        method.implementation,
                    ));
                    method_count += 1;
                }
            }
            if parent == 0 {
                for (selector, types, imp, is_class) in [
                    ("initialize", "v16@0:8", code, true),
                    ("alloc", "@16@0:8", code + 0x20, true),
                    ("init", "@16@0:8", code + 4, false),
                    ("dealloc", "v16@0:8", code + 0x60, false),
                ] {
                    let target = if is_class {
                        &mut class_methods
                    } else {
                        &mut instance
                    };
                    if target.iter().any(|(s, _, _)| s == selector) {
                        return Err("owned NSObject built-in lifecycle method overridden".into());
                    }
                    target.push((selector.into(), types.into(), imp));
                    method_count += 1;
                }
            }
            if method_count > 1024 {
                return Err("owned ObjC namespace method budget exceeded".into());
            }
            let normal = data.reserve(40)?;
            let meta = data.reserve(40)?;
            let ro = data.reserve(72)?;
            let meta_ro = data.reserve(72)?;
            let root_meta = classes.get("NSObject").map(|&(_, m)| m).unwrap_or(meta);
            data.u64(normal, meta);
            data.u64(normal + 8, parent);
            data.u64(normal + 32, ro);
            data.u64(meta, root_meta);
            data.u64(meta + 8, if parent == 0 { normal } else { parent_meta });
            data.u64(meta + 32, meta_ro);
            let name = data.string(spec.name)?;
            let methods = data.methods(&instance, &mut selectors)?;
            let meta_methods = data.methods(&class_methods, &mut selectors)?;
            for (ro, flags, start, size, list) in [
                (
                    ro,
                    if parent == 0 { 2 } else { 0 },
                    parent_size,
                    spec.instance_size,
                    methods,
                ),
                (
                    meta_ro,
                    if parent == 0 { 3 } else { 1 },
                    40,
                    40,
                    meta_methods,
                ),
            ] {
                data.u32(ro, flags);
                data.u32(ro + 4, start);
                data.u32(ro + 8, size);
                data.u64(ro + 24, name);
                data.u64(ro + 32, list);
            }
            classes.insert(spec.name.to_string(), (normal, meta));
            sizes.insert(spec.name.to_string(), spec.instance_size);
            addresses.push(normal);
        }
        // All metadata/IMP validation completes before the first mapping.
        cpu.map_zeroed(base, DATA_SIZE, 1)?;
        cpu.try_write_bytes(base, &data.bytes)?;
        cpu.map_zeroed(code, CODE_SIZE, 5)?;
        cpu.try_write_bytes(code, &0xd65f03c0u32.to_le_bytes())?;
        cpu.try_write_bytes(code + 4, &0xd65f03c0u32.to_le_bytes())?;
        let caller = [
            0xa9bf7bfdu32,
            0x910003fd,
            0xd2800001,
            0x580000b0,
            0xd63f0200,
            0xa8c17bfd,
            0xd65f03c0,
            0xd503201f,
        ];
        let bytes: Vec<_> = caller.iter().flat_map(|word| word.to_le_bytes()).collect();
        cpu.try_write_bytes(code + 0x20, &bytes)?;
        cpu.try_write_bytes(code + 0x60, &bytes)?;
        Ok(Self {
            class_addresses: addresses,
            classes,
            selectors,
            code,
            linked: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::objc_metadata::Registry;
    use super::*;
    #[test]
    fn owned_classes_have_real_root_meta_graph_and_require_exact_lifecycle_links() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x20000, 4096, 5).unwrap();
        cpu.try_write_bytes(0x20000, &0xd65f03c0u32.to_le_bytes())
            .unwrap();
        let mut namespace = Namespace::map(
            &mut cpu,
            0xa0000,
            &[
                ClassSpec {
                    name: "NSObject",
                    parent: None,
                    instance_size: 8,
                    instance_methods: vec![],
                    class_methods: vec![],
                },
                ClassSpec {
                    name: "NSBundle",
                    parent: Some("NSObject"),
                    instance_size: 8,
                    instance_methods: vec![],
                    class_methods: vec![MethodSpec {
                        selector: "mainBundle",
                        types: "@16@0:8",
                        implementation: 0x20000,
                    }],
                },
                ClassSpec {
                    name: "NSString",
                    parent: Some("NSObject"),
                    instance_size: 24,
                    instance_methods: vec![],
                    class_methods: vec![],
                },
            ],
        )
        .unwrap();
        assert!(namespace.exports().is_err());
        assert!(namespace
            .link_root_services(&mut cpu, 0x20000, 0x99999)
            .is_err());
        assert!(namespace.exports().is_err());
        namespace
            .link_root_services(&mut cpu, 0x20000, 0x20000)
            .unwrap();
        let registered = Registry::register(
            &namespace.class_addresses,
            &[],
            |a, n| {
                let mut b = vec![0; n];
                cpu.read_guest_into(a, &mut b)?;
                Ok(b)
            },
            |a, n| {
                if n == 4 && cpu.mapped_permissions(a).is_some_and(|p| p & 4 != 0) {
                    Ok(())
                } else {
                    Err("fixture executable mapping required".into())
                }
            },
        )
        .unwrap();
        assert_eq!(
            registered.registry.lookup_class("NSBundle"),
            namespace.class_address("NSBundle")
        );
        assert_eq!(
            registered.registry.selector_named("initialize"),
            namespace.selector("initialize")
        );
        assert_eq!(
            namespace.exports().unwrap()["_OBJC_CLASS_$_NSBundle"],
            namespace.class_address("NSBundle").unwrap()
        );
        assert_eq!(cpu.read_u64(namespace.code + 0x40), Some(0x20000));
        assert_eq!(cpu.mapped_permissions(namespace.code), Some(5));
        assert!(namespace
            .link_root_services(&mut cpu, 0x20000, 0x20000)
            .is_err());
    }
    #[test]
    fn namespace_rejects_unmapped_methods_and_undeclared_parent_before_mapping() {
        let mut cpu = A64Cpu::new_sparse();
        let root = ClassSpec {
            name: "NSObject",
            parent: None,
            instance_size: 8,
            instance_methods: vec![],
            class_methods: vec![MethodSpec {
                selector: "bad",
                types: "v16@0:8",
                implementation: 0xdead,
            }],
        };
        assert!(Namespace::map(&mut cpu, 0xa0000, &[root]).is_err());
        assert_eq!(cpu.mapped_bytes(), 0);
        let root = ClassSpec {
            name: "NSObject",
            parent: None,
            instance_size: 8,
            instance_methods: vec![],
            class_methods: vec![],
        };
        let child = ClassSpec {
            name: "NSBundle",
            parent: Some("Missing"),
            instance_size: 8,
            instance_methods: vec![],
            class_methods: vec![],
        };
        assert!(Namespace::map(&mut cpu, 0xa0000, &[root, child]).is_err());
        assert_eq!(cpu.mapped_bytes(), 0);
    }
}

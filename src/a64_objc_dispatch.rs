/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Registration and dispatch planning for already bound Apple LP64 metadata.
//! No IMP is executed and no Apple class initialization is fabricated here.

use super::{read_class_from_reader, CacheSelectorContext, Class, Reader};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_CLASSES: usize = 4096;
const MAX_SELECTORS: usize = 65536;
const MAX_METHODS_TOTAL: usize = 65536;
const MAX_TEXT_TOTAL: usize = 16 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SelectorFixup {
    pub slot: u64,
    pub original: u64,
    pub canonical: u64,
}

#[derive(Debug)]
pub(crate) struct Registration {
    pub registry: Registry,
    /// Commit these only after validating every destination as writable.
    /// The registry is a host-side plan until these guest SEL slots are fixed up.
    pub selector_fixups: Vec<SelectorFixup>,
}

#[derive(Debug)]
pub(crate) struct Registry {
    classes: BTreeMap<u64, Class>,
    class_names: BTreeMap<String, u64>,
    selectors: BTreeMap<String, u64>,
    selector_aliases: BTreeMap<u64, u64>,
    selector_names: BTreeMap<u64, String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Invocation {
    pub receiver: u64,
    pub selector: u64,
    pub implementation: u64,
    pub types: String,
    pub method_owner: u64,
    /// The dynamic normal (non-metaclass) receiver class, zero for a nil Super2
    /// receiver. This is independent of the lexical Super2 lookup context.
    /// +initialize remains a caller
    /// responsibility before this plan is executable. This module does not mark
    /// classes initialized merely because metadata was decoded.
    pub receiver_class: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MessagePlan {
    /// Ordinary objc_msgSend's actual nil ABI. The bridge must zero x0/x1 and
    /// execute the equivalent of MOVI d0..d3,#0; other argument registers and
    /// indirect-result storage are untouched. No non-nil lookup reaches this.
    Nil,
    /// Tail-call the guest IMP with x0=receiver and x1=canonical selector;
    /// preserve LR, SP, x8, remaining integer and SIMD arguments. Framework
    /// initialization, guest +initialize, ARC and forwarding are not supplied.
    Invoke(Invocation),
}

fn is_meta(class: &Class) -> bool {
    class.flags & 1 != 0
}
fn is_root(class: &Class) -> bool {
    class.flags & 2 != 0
}

impl Registry {
    pub(crate) fn selector_entries(&self) -> impl Iterator<Item = (&str, u64)> {
        self.selectors
            .iter()
            .map(|(name, &address)| (name.as_str(), address))
    }
    pub(crate) fn selector_named(&self, name: &str) -> Option<u64> {
        self.selectors.get(name).copied()
    }

    pub(crate) fn classes(&self) -> impl Iterator<Item = &Class> {
        self.classes.values()
    }
    /// Build a complete, bounded class graph and selector plan without guest
    /// writes. `class_addresses` identifies regular classes, not metaclasses;
    /// their metaclasses and superclasses are read recursively. An unbound or
    /// unavailable framework class is an error, never a substitute NSObject.
    /// `executable` must validate the full 4-byte ARM64 instruction at each IMP.
    pub(crate) fn register(
        class_addresses: &[u64],
        selector_ref_slots: &[u64],
        read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
        executable: impl FnMut(u64, usize) -> Result<(), String>,
    ) -> Result<Registration, String> {
        Self::register_with_cache(class_addresses, selector_ref_slots, None, read, executable)
    }
    pub(crate) fn register_with_cache(
        class_addresses: &[u64],
        selector_ref_slots: &[u64],
        cache: Option<CacheSelectorContext>,
        read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
        mut executable: impl FnMut(u64, usize) -> Result<(), String>,
    ) -> Result<Registration, String> {
        if class_addresses.len() > MAX_CLASSES || selector_ref_slots.len() > MAX_SELECTORS {
            return Err("Objective-C registration input limit exceeded".into());
        }
        let mut reader = Reader::new(read);
        let mut queued = BTreeSet::new();
        let mut queue = VecDeque::new();
        for &address in class_addresses {
            if address == 0 {
                return Err("unresolved Objective-C class list entry".into());
            }
            if queued.insert(address) {
                queue.push_back(address);
            }
        }
        let mut registry = Self {
            classes: BTreeMap::new(),
            class_names: BTreeMap::new(),
            selectors: BTreeMap::new(),
            selector_aliases: BTreeMap::new(),
            selector_names: BTreeMap::new(),
        };
        let mut text = 0usize;
        let mut method_count = 0usize;
        while let Some(address) = queue.pop_front() {
            let class = read_class_from_reader(address, cache, &mut reader)
                .map_err(|error| format!("Objective-C class {address:#x}: {error}"))?;
            method_count = method_count
                .checked_add(class.methods.len())
                .ok_or("Objective-C method budget overflow")?;
            text = text
                .checked_add(class.name.len())
                .ok_or("Objective-C text budget overflow")?;
            for method in &class.methods {
                text = text
                    .checked_add(method.selector.len())
                    .and_then(|n| n.checked_add(method.types.len()))
                    .ok_or("Objective-C text budget overflow")?;
            }
            if method_count > MAX_METHODS_TOTAL || text > MAX_TEXT_TOTAL {
                return Err("Objective-C registration metadata limit exceeded".into());
            }
            for method in &class.methods {
                executable(method.implementation, 4).map_err(|error| {
                    format!(
                        "non-executable Objective-C IMP for {}: {error}",
                        method.selector
                    )
                })?;
                registry.intern(&method.selector, method.selector_address)?;
            }
            if !is_meta(&class) {
                if registry
                    .class_names
                    .insert(class.name.clone(), address)
                    .is_some()
                {
                    return Err(format!("duplicate Objective-C class name {}", class.name));
                }
            }
            for dependency in [class.isa, class.superclass] {
                if dependency != 0 && queued.insert(dependency) {
                    if queued.len() > MAX_CLASSES {
                        return Err("Objective-C class graph limit exceeded".into());
                    }
                    queue.push_back(dependency);
                }
            }
            registry.classes.insert(address, class);
        }
        registry.validate_graph()?;
        for &address in class_addresses {
            if is_meta(&registry.classes[&address]) {
                return Err("metaclass in Objective-C regular class list".into());
            }
        }
        let mut fixups = Vec::new();
        let mut slots = BTreeSet::new();
        for &slot in selector_ref_slots {
            if slot & 7 != 0 || !slots.insert(slot) {
                return Err("unaligned or duplicate Objective-C selector slot".into());
            }
            let original = reader.u64(slot)?;
            let name = reader.string(original)?;
            let canonical = registry.intern(&name, original)?;
            if canonical != original {
                fixups.push(SelectorFixup {
                    slot,
                    original,
                    canonical,
                });
            }
        }
        Ok(Registration {
            registry,
            selector_fixups: fixups,
        })
    }

    fn intern(&mut self, name: &str, address: u64) -> Result<u64, String> {
        if address == 0 || name.is_empty() {
            return Err("invalid Objective-C selector identity".into());
        }
        let canonical = if let Some(&canonical) = self.selectors.get(name) {
            canonical
        } else {
            if self.selectors.len() >= MAX_SELECTORS {
                return Err("Objective-C selector limit exceeded".into());
            }
            self.selectors.insert(name.into(), address);
            self.selector_names.insert(address, name.into());
            address
        };
        if let Some(&old) = self.selector_aliases.get(&address) {
            if old != canonical {
                return Err("conflicting Objective-C selector address".into());
            }
        }
        self.selector_aliases.insert(address, canonical);
        Ok(canonical)
    }

    fn class(&self, address: u64) -> Result<&Class, String> {
        self.classes
            .get(&address)
            .ok_or_else(|| format!("unregistered Objective-C class/framework {address:#x}"))
    }

    fn validate_graph(&self) -> Result<(), String> {
        for class in self.classes.values() {
            let isa = self.class(class.isa)?;
            if !is_meta(isa) {
                return Err("Objective-C isa is not a metaclass".into());
            }
            if !is_meta(class) && isa.name != class.name {
                return Err("Objective-C class/metaclass name mismatch".into());
            }
            if is_meta(class) && !is_root(isa) {
                return Err("Objective-C metaclass isa is not a root metaclass".into());
            }
            if !is_meta(class) {
                if is_root(class) {
                    if class.superclass != 0
                        || isa.superclass != class.address
                        || isa.isa != isa.address
                    {
                        return Err("invalid Objective-C root class/metaclass graph".into());
                    }
                } else if isa.superclass != self.class(class.superclass)?.isa {
                    return Err("Objective-C class/metaclass superclass mismatch".into());
                }
            }
            if class.superclass != 0 {
                let superclass = self.class(class.superclass)?;
                if is_meta(class) != is_meta(superclass)
                    && !(is_meta(class)
                        && is_root(class)
                        && !is_meta(superclass)
                        && superclass.isa == class.address)
                {
                    return Err("Objective-C superclass/metaclass kind mismatch".into());
                }
            }
            let mut visited = BTreeSet::new();
            let mut cursor = class.address;
            while cursor != 0 {
                if !visited.insert(cursor) {
                    return Err("Objective-C superclass cycle".into());
                }
                cursor = self.class(cursor)?.superclass;
            }
        }
        Ok(())
    }

    pub(crate) fn lookup_class(&self, name: &str) -> Option<u64> {
        self.class_names.get(name).copied()
    }

    pub(crate) fn canonical_selector(&self, selector: u64) -> Result<u64, String> {
        self.selector_aliases
            .get(&selector)
            .copied()
            .ok_or_else(|| format!("unregistered Objective-C selector {selector:#x}"))
    }

    fn lookup(
        &self,
        start: u64,
        selector: u64,
        receiver: u64,
        receiver_class: u64,
        mut executable: impl FnMut(u64, usize) -> Result<(), String>,
    ) -> Result<MessagePlan, String> {
        let canonical = self.canonical_selector(selector)?;
        let name = &self.selector_names[&canonical];
        let mut cursor = start;
        for _ in 0..MAX_CLASSES {
            if cursor == 0 {
                break;
            }
            let class = self.class(cursor)?;
            if let Some(method) = class.methods.iter().find(|m| &m.selector == name) {
                executable(method.implementation, 4)?;
                return Ok(MessagePlan::Invoke(Invocation {
                    receiver,
                    selector: canonical,
                    implementation: method.implementation,
                    types: method.types.clone(),
                    method_owner: cursor,
                    receiver_class,
                }));
            }
            cursor = class.superclass;
        }
        Err(format!("Objective-C selector {name} has no method; dynamic resolution/forwarding is not implemented"))
    }

    /// Read only raw pointer isa. Encoded non-pointer isa and tagged instances
    /// require their own runtime ABI implementation and are rejected explicitly.
    pub(crate) fn plan_message(
        &self,
        receiver: u64,
        selector: u64,
        read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
        executable: impl FnMut(u64, usize) -> Result<(), String>,
    ) -> Result<MessagePlan, String> {
        if receiver == 0 {
            return Ok(MessagePlan::Nil);
        }
        let mut reader = Reader::new(read);
        let (isa, receiver_class) = self.receiver_identity(receiver, &mut reader)?;
        self.lookup(isa, selector, receiver, receiver_class, executable)
    }

    fn receiver_identity<F: FnMut(u64, usize) -> Result<Vec<u8>, String>>(
        &self,
        receiver: u64,
        reader: &mut Reader<F>,
    ) -> Result<(u64, u64), String> {
        if receiver == 0 || receiver & 7 != 0 || receiver >> 63 != 0 {
            return Err("unsupported tagged or unaligned Objective-C receiver".into());
        }
        let isa = reader.u64(receiver)?;
        if isa & 7 != 0 {
            return Err("unsupported encoded Objective-C isa".into());
        }
        let class = self.class(isa)?;
        let receiver_class = if is_meta(class) {
            let normal = self.class(receiver)?;
            if is_meta(normal) || normal.isa != isa {
                return Err("invalid Objective-C class receiver".into());
            }
            receiver
        } else {
            isa
        };
        Ok((isa, receiver_class))
    }

    /// objc_super contains {receiver,current_class}; Super2 begins at the
    /// supplied current class's superclass, not at current_class itself.
    /// Unlike ordinary msgSend, Apple's Super2 entry has no nil shortcut.
    pub(crate) fn plan_super2(
        &self,
        super_record: u64,
        selector: u64,
        read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
        executable: impl FnMut(u64, usize) -> Result<(), String>,
    ) -> Result<MessagePlan, String> {
        if super_record == 0 || super_record & 7 != 0 {
            return Err("invalid Objective-C super record".into());
        }
        let mut reader = Reader::new(read);
        let record = reader.bytes(super_record, 16)?;
        let receiver = u64::from_le_bytes(record[..8].try_into().unwrap());
        let current = u64::from_le_bytes(record[8..].try_into().unwrap());
        let class = self.class(current)?;
        let receiver_class = if receiver == 0 {
            0
        } else {
            self.receiver_identity(receiver, &mut reader)?.1
        };
        self.lookup(
            class.superclass,
            selector,
            receiver,
            receiver_class,
            executable,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: u64 = 0x1_0000_0000;
    fn put64(b: &mut [u8], at: usize, value: u64) {
        b[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn put32(b: &mut [u8], at: usize, value: u32) {
        b[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut b = vec![0; 8192];
        // Root/root metaclass, then Child/Child metaclass. The root metaclass's
        // superclass is the regular root class, exactly as Apple's ObjC2 ABI.
        for (at, isa, superclass, ro) in [
            (0, 40, None, 256),
            (40, 40, Some(0), 416),
            (80, 120, Some(0), 336),
            (120, 40, Some(40), 496),
        ] {
            put64(&mut b, at, BASE + isa);
            put64(&mut b, at + 8, superclass.map_or(0, |x| BASE + x));
            put64(&mut b, at + 32, BASE + ro);
        }
        for (ro, flags, start, size, name, methods) in [
            (256, 2, 8, 16, 600, 768),
            (336, 0, 16, 24, 608, 800),
            (416, 3, 40, 40, 600, 832),
            (496, 1, 40, 40, 608, 864),
        ] {
            put32(&mut b, ro, flags);
            put32(&mut b, ro + 4, start);
            put32(&mut b, ro + 8, size);
            put64(&mut b, ro + 24, BASE + name);
            put64(&mut b, ro + 32, BASE + methods);
        }
        b[600..605].copy_from_slice(b"Root\0");
        b[608..614].copy_from_slice(b"Child\0");
        b[1100..1106].copy_from_slice(b"value\0");
        b[1120..1126].copy_from_slice(b"value\0");
        b[1160..1168].copy_from_slice(b"I16@0:8\0");
        for (list, selector, imp) in [
            (768, 1100, 4096),
            (800, 1120, 4100),
            (832, 1100, 4104),
            (864, 1120, 4108),
        ] {
            put32(&mut b, list, 24);
            put32(&mut b, list + 4, 1);
            put64(&mut b, list + 8, BASE + selector);
            put64(&mut b, list + 16, BASE + 1160);
            put64(&mut b, list + 24, BASE + imp);
        }
        put64(&mut b, 1200, BASE + 1120);
        put64(&mut b, 1208, BASE + 1100);
        put64(&mut b, 1304, BASE + 80); // aligned object, raw isa
        put64(&mut b, 1320, BASE + 1304);
        put64(&mut b, 1328, BASE + 80);
        b
    }
    fn read(b: &[u8], address: u64, size: usize) -> Result<Vec<u8>, String> {
        let at: usize = address
            .checked_sub(BASE)
            .ok_or("unmapped")?
            .try_into()
            .map_err(|_| "overflow")?;
        b.get(at..at.checked_add(size).ok_or("overflow")?)
            .map(|b| b.to_vec())
            .ok_or_else(|| "unmapped".into())
    }
    fn executable(address: u64, size: usize) -> Result<(), String> {
        if size == 4 && (BASE + 4096..BASE + 4112).contains(&address) {
            Ok(())
        } else {
            Err("not code".into())
        }
    }
    fn registration(b: &[u8]) -> Result<Registration, String> {
        Registry::register(
            &[BASE, BASE + 80],
            &[BASE + 1200, BASE + 1208],
            |a, n| read(b, a, n),
            executable,
        )
    }
    fn invoke(plan: MessagePlan) -> Invocation {
        match plan {
            MessagePlan::Invoke(p) => p,
            MessagePlan::Nil => panic!("expected dispatch"),
        }
    }

    #[test]
    fn canonical_identity_and_instance_metaclass_dispatch() {
        let b = fixture();
        let Registration {
            registry,
            selector_fixups,
        } = registration(&b).unwrap();
        assert_eq!(registry.lookup_class("Child"), Some(BASE + 80));
        assert_eq!(registry.lookup_class("missing framework"), None);
        assert_eq!(
            selector_fixups,
            vec![SelectorFixup {
                slot: BASE + 1200,
                original: BASE + 1120,
                canonical: BASE + 1100
            }]
        );
        // Registration is atomic/read-only: caller has not committed this slot.
        assert_eq!(
            read(&b, BASE + 1200, 8).unwrap(),
            (BASE + 1120).to_le_bytes()
        );
        let instance = invoke(
            registry
                .plan_message(BASE + 1304, BASE + 1120, |a, n| read(&b, a, n), executable)
                .unwrap(),
        );
        assert_eq!(instance.implementation, BASE + 4100);
        assert_eq!(instance.selector, BASE + 1100);
        assert_eq!(instance.receiver_class, BASE + 80);
        let class = invoke(
            registry
                .plan_message(BASE + 80, BASE + 1100, |a, n| read(&b, a, n), executable)
                .unwrap(),
        );
        assert_eq!(class.implementation, BASE + 4108);
        assert_eq!(class.method_owner, BASE + 120);
    }
    #[test]
    fn super2_skips_current_override_and_does_not_shortcut_nil() {
        let mut b = fixture();
        let r = registration(&b).unwrap().registry;
        let p = invoke(
            r.plan_super2(BASE + 1320, BASE + 1120, |a, n| read(&b, a, n), executable)
                .unwrap(),
        );
        assert_eq!(p.receiver, BASE + 1304);
        assert_eq!(p.implementation, BASE + 4096);
        put64(&mut b, 1320, BASE + 80); // class message
        put64(&mut b, 1328, BASE + 120); // current metaclass
        assert_eq!(
            invoke(
                r.plan_super2(BASE + 1320, BASE + 1100, |a, n| read(&b, a, n), executable)
                    .unwrap()
            )
            .implementation,
            BASE + 4104
        );
        put64(&mut b, 1320, 0);
        assert_eq!(
            invoke(
                r.plan_super2(BASE + 1320, BASE + 1100, |a, n| read(&b, a, n), executable)
                    .unwrap()
            )
            .receiver,
            0
        );
        assert_eq!(
            r.plan_message(
                0,
                0,
                |_, _| panic!("nil must not read memory"),
                |_, _| panic!("nil must not validate IMP")
            )
            .unwrap(),
            MessagePlan::Nil
        );
    }

    #[test]
    fn super2_keeps_dynamic_receiver_class_separate_from_lexical_context() {
        let mut b = fixture();
        for (at, isa, superclass, ro) in [(160, 200, 80, 1504), (200, 40, 120, 1600)] {
            put64(&mut b, at, BASE + isa);
            put64(&mut b, at + 8, BASE + superclass);
            put64(&mut b, at + 32, BASE + ro);
        }
        for (ro, flags, start, size) in [(1504, 0, 24, 32), (1600, 1, 40, 40)] {
            put32(&mut b, ro, flags);
            put32(&mut b, ro + 4, start);
            put32(&mut b, ro + 8, size);
            put64(&mut b, ro + 24, BASE + 1700);
        }
        b[1700..1711].copy_from_slice(b"Grandchild\0");
        put64(&mut b, 1304, BASE + 160);
        let r = Registry::register(
            &[BASE, BASE + 80, BASE + 160],
            &[],
            |a, n| read(&b, a, n),
            executable,
        )
        .unwrap()
        .registry;
        let p = invoke(
            r.plan_super2(BASE + 1320, BASE + 1100, |a, n| read(&b, a, n), executable)
                .unwrap(),
        );
        assert_eq!(p.receiver_class, BASE + 160);
        assert_eq!(p.method_owner, BASE);
        assert_eq!(p.implementation, BASE + 4096);
        put64(&mut b, 1320, BASE + 160);
        put64(&mut b, 1328, BASE + 120);
        let p = invoke(
            r.plan_super2(BASE + 1320, BASE + 1100, |a, n| read(&b, a, n), executable)
                .unwrap(),
        );
        assert_eq!(p.receiver_class, BASE + 160);
        assert_eq!(p.method_owner, BASE + 40);
        assert_eq!(p.implementation, BASE + 4104);
    }
    #[test]
    fn inheritance_fallback_and_forwarding_are_distinct() {
        let mut b = fixture();
        put64(&mut b, 336 + 32, 0); // no Child instance methods
        let r = registration(&b).unwrap().registry;
        assert_eq!(
            invoke(
                r.plan_message(BASE + 1304, BASE + 1100, |a, n| read(&b, a, n), executable)
                    .unwrap()
            )
            .implementation,
            BASE + 4096
        );
        let mut b = fixture();
        put32(&mut b, 768 + 4, 0);
        put32(&mut b, 800 + 4, 0);
        let r = registration(&b).unwrap().registry;
        assert!(r
            .plan_message(BASE + 1304, BASE + 1100, |a, n| read(&b, a, n), executable)
            .unwrap_err()
            .contains("forwarding"));
    }
    #[test]
    fn unresolved_graph_and_non_executable_imp_reject_registration() {
        let mut b = fixture();
        put64(&mut b, 80 + 8, BASE + 9000);
        assert!(registration(&b).is_err());
        let mut b = fixture();
        put64(&mut b, 80 + 8, BASE + 80);
        assert!(registration(&b).is_err());
        let mut b = fixture();
        put64(&mut b, 120 + 8, BASE + 120);
        assert!(registration(&b).is_err());
        let mut b = fixture();
        put64(&mut b, 800 + 24, BASE + 4000);
        assert!(registration(&b).unwrap_err().contains("non-executable"));
        assert!(Registry::register(
            &[BASE],
            &[],
            |_, n| Ok(vec![0; n.saturating_sub(1)]),
            executable
        )
        .is_err());
    }
    #[test]
    fn runtime_pointer_abis_and_unregistered_selectors_are_not_guessed() {
        let mut b = fixture();
        let r = registration(&b).unwrap().registry;
        put64(&mut b, 1304, (BASE + 80) | 1);
        assert!(r
            .plan_message(BASE + 1304, BASE + 1100, |a, n| read(&b, a, n), executable)
            .unwrap_err()
            .contains("encoded"));
        assert!(r
            .plan_message(
                0x8000_0000_0000_0001,
                BASE + 1100,
                |a, n| read(&b, a, n),
                executable
            )
            .unwrap_err()
            .contains("tagged"));
        assert!(r
            .plan_super2(BASE + 8190, BASE + 1100, |a, n| read(&b, a, n), executable)
            .is_err());
        assert!(r.canonical_selector(BASE + 1176).is_err());
        assert!(r
            .plan_super2(
                BASE + 1320,
                BASE + 1100,
                |a, n| read(&b, a, n),
                |_, _| Err("no execute permission".into())
            )
            .is_err());
    }
}

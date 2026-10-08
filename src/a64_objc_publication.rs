/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Stage a real registry replacement. This never resets existing +initialize
//! state, runs constructors, or equates a mapped provider with a ready runtime.
use super::objc_execution_services::ObjectRuntime;
use super::objc_metadata::{CacheSelectorContext, Class, Registry, SelectorFixup};
use super::objc_registration::RegisteredImages;
use super::objc_slots::SlotMemory;
use std::collections::{BTreeMap, BTreeSet};
use std::{cell::RefCell, rc::Rc};

pub(super) struct PreparedPublication {
    active: Rc<Registry>,
    registry: Registry,
    fixups: Vec<SelectorFixup>,
    dependencies: Vec<String>,
    /// These classes require addition to the existing Initialization model as
    /// Uninitialized; existing states must be retained by the install hook.
    new_classes: Vec<Class>,
}

fn merge_class_identities<'a>(
    active: impl Iterator<Item = &'a Class>,
    added: impl Iterator<Item = &'a Class>,
) -> Result<(Vec<u64>, Vec<Class>), String> {
    let mut names = BTreeMap::new();
    let mut addresses = BTreeMap::new();
    let mut roots = BTreeSet::new();
    let mut new = Vec::new();
    for (is_new, class) in active.map(|c| (false, c)).chain(added.map(|c| (true, c))) {
        if let Some(previous) = addresses.get(&class.address) {
            if *previous != class {
                return Err("publication class metadata changed at an existing identity".into());
            }
            continue;
        }
        if addresses.len() >= 4096 {
            return Err("publication class graph budget exceeded".into());
        }
        if class.flags & 1 == 0 {
            if let Some(previous) = names.insert(class.name.clone(), class.address) {
                if previous != class.address {
                    return Err(format!(
                        "publication class-name collision {}: {previous:#x} and {:#x}",
                        class.name, class.address
                    ));
                }
            }
            roots.insert(class.address);
        }
        addresses.insert(class.address, class);
        if is_new {
            new.push(class.clone());
        }
    }
    Ok((roots.into_iter().collect(), new))
}

pub(super) fn prepare(
    active: &Rc<Registry>,
    candidate: &RegisteredImages,
    selector_ref_slots: &[u64],
    required_dependencies: &[String],
    cache: Option<CacheSelectorContext>,
    read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
    executable_metadata: impl FnMut(u64, usize) -> Result<(), String>,
) -> Result<PreparedPublication, String> {
    if required_dependencies.len() > 4096 {
        return Err("publication dependency budget exceeded".into());
    }
    let mut dependencies = BTreeSet::new();
    for path in required_dependencies {
        if !path.starts_with('/')
            || path.len() > 4096
            || path.contains('\0')
            || !dependencies.insert(path.clone())
        {
            return Err("invalid or duplicate exact publication dependency identity".into());
        }
    }
    let (classes, new_classes) =
        merge_class_identities(active.classes(), candidate.registry.classes())?;
    let registered = Registry::register_with_cache(
        &classes,
        selector_ref_slots,
        cache,
        read,
        executable_metadata,
    )?;
    // Rebuilding cannot silently change a live selector token. Additional
    // aliases require a separate exact slot transaction and explicit runtime
    // token migration; this implementation rejects that case.
    for (name, address) in active.selector_entries() {
        if registered.registry.selector_named(name) != Some(address) {
            return Err(format!(
                "publication would change active selector identity {}",
                name
            ));
        }
    }
    Ok(PreparedPublication {
        active: active.clone(),
        registry: registered.registry,
        fixups: registered.selector_fixups,
        dependencies: dependencies.into_iter().collect(),
        new_classes,
    })
}
impl PreparedPublication {
    /// Commit only while guest execution is stopped in the coordinator. Locks
    /// both shared runtime objects before guest writes; after the atomic slot
    /// transaction succeeds, replacement has no remaining fallible operation.
    /// Existing initialization states stay intact and all new classes start
    /// Uninitialized. Heap allocation policies are deliberately not broadened.
    pub(super) fn install(
        self,
        runtime: &Rc<RefCell<ObjectRuntime>>,
        mut dependency_ready: impl FnMut(&str) -> Result<(), String>,
        memory: &mut impl SlotMemory,
    ) -> Result<(), String> {
        for path in &self.dependencies {
            dependency_ready(path)?;
        }
        let mut runtime = runtime
            .try_borrow_mut()
            .map_err(|_| "reentrant Objective-C namespace publication")?;
        if !Rc::ptr_eq(&runtime.registry, &self.active) {
            return Err(
                "active Objective-C namespace changed after publication preparation".into(),
            );
        }
        let initialization = runtime.initialization.clone();
        let mut initialization = initialization
            .try_borrow_mut()
            .map_err(|_| "active Objective-C initialization borrow prevents publication")?;
        let extended = initialization.extended_with(self.new_classes)?;
        let lifetime = runtime.lifetime.clone();
        let mut lifetime = lifetime
            .try_borrow_mut()
            .map_err(|_| "active Objective-C ownership borrow prevents publication")?;
        let identities: Vec<u64> = self.registry.classes().map(|class| class.address).collect();
        let enrolled = lifetime.with_class_identities(&identities)?;
        commit_fixups(&self.fixups, memory)?;
        runtime.registry = Rc::new(self.registry);
        *initialization = extended;
        *lifetime = enrolled;
        Ok(())
    }
}

fn commit_fixups(fixups: &[SelectorFixup], memory: &mut impl SlotMemory) -> Result<(), String> {
    let mut slots = BTreeSet::new();
    for fixup in fixups {
        if fixup.slot & 7 != 0 || fixup.canonical == 0 || !slots.insert(fixup.slot) {
            return Err("invalid or duplicate publication selector slot".into());
        }
        memory.validate_write(fixup.slot, 8)?;
        let value = memory.read(fixup.slot, 8)?;
        if value.as_slice() != fixup.original.to_le_bytes() {
            return Err("publication selector slot changed after preparation".into());
        }
    }
    for (index, fixup) in fixups.iter().enumerate() {
        if let Err(error) = memory.write(fixup.slot, &fixup.canonical.to_le_bytes()) {
            let mut failed = Vec::new();
            for old in fixups[..=index].iter().rev() {
                if memory.write(old.slot, &old.original.to_le_bytes()).is_err() {
                    failed.push(old.slot);
                }
            }
            return Err(if failed.is_empty() {
                format!("publication selector transaction rolled back: {error}")
            } else {
                format!("publication selector rollback incomplete at {failed:x?}: {error}")
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{
        objc_execution::Initialization,
        objc_heap::GuestObjectHeap,
        objc_lifetime::Lifetime,
        objc_namespace::{ClassSpec, Namespace},
        A64Cpu,
    };
    use super::*;
    fn fixture() -> (A64Cpu, Rc<RefCell<ObjectRuntime>>, RegisteredImages, u64) {
        let mut cpu = A64Cpu::new_sparse();
        let namespace = Namespace::map(
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
                    name: "UnityFramework",
                    parent: Some("NSObject"),
                    instance_size: 8,
                    instance_methods: vec![],
                    class_methods: vec![],
                },
            ],
        )
        .unwrap();
        let root = namespace.class_address("NSObject").unwrap();
        let child = namespace.class_address("UnityFramework").unwrap();
        let registry = |roots: &[u64]| {
            Registry::register(
                roots,
                &[],
                |a, n| {
                    let mut bytes = vec![0; n];
                    cpu.read_guest_into(a, &mut bytes)?;
                    Ok(bytes)
                },
                |a, n| {
                    if n == 4 && cpu.mapped_permissions(a).is_some_and(|p| p & 4 != 0) {
                        Ok(())
                    } else {
                        Err("not RX".into())
                    }
                },
            )
            .unwrap()
            .registry
        };
        let active = Rc::new(registry(&[root]));
        let candidate_registry = registry(&[child]);
        let state = Rc::new(RefCell::new(
            Initialization::new(active.classes().cloned()).unwrap(),
        ));
        let candidate = RegisteredImages {
            initialization: Initialization::new(candidate_registry.classes().cloned()).unwrap(),
            registry: candidate_registry,
            images: vec![],
            class_owners: BTreeMap::new(),
            selector_fixups: vec![],
        };
        let heap = GuestObjectHeap::map(&mut cpu, 0x80000, 4096).unwrap();
        let runtime = Rc::new(RefCell::new(
            ObjectRuntime::new(
                active,
                state,
                Rc::new(RefCell::new(Lifetime::default())),
                heap,
                root,
            )
            .unwrap(),
        ));
        (cpu, runtime, candidate, child)
    }
    fn prepared(
        cpu: &A64Cpu,
        runtime: &Rc<RefCell<ObjectRuntime>>,
        candidate: &RegisteredImages,
        deps: &[String],
    ) -> PreparedPublication {
        prepare(
            &runtime.borrow().registry,
            candidate,
            &[],
            deps,
            None,
            |a, n| {
                let mut bytes = vec![0; n];
                cpu.read_guest_into(a, &mut bytes)?;
                Ok(bytes)
            },
            |a, n| {
                if n == 4 && cpu.mapped_permissions(a).is_some_and(|p| p & 4 != 0) {
                    Ok(())
                } else {
                    Err("not RX".into())
                }
            },
        )
        .unwrap()
    }
    #[test]
    fn real_runtime_install_rejects_missing_receipts_stale_registry_and_active_borrow() {
        let (mut cpu, runtime, candidate, child) = fixture();
        let active = runtime.borrow().registry.clone();
        let pending = prepared(
            &cpu,
            &runtime,
            &candidate,
            &["/usr/lib/libobjc.A.dylib".into()],
        );
        assert!(pending
            .install(
                &runtime,
                |_| Err("missing real initialization receipt".into()),
                &mut cpu
            )
            .is_err());
        assert!(Rc::ptr_eq(&runtime.borrow().registry, &active));
        assert!(!runtime
            .borrow()
            .registry
            .classes()
            .any(|c| c.address == child));
        let pending = prepared(&cpu, &runtime, &candidate, &[]);
        let initialization = runtime.borrow().initialization.clone();
        let guard = initialization.borrow_mut();
        assert!(pending
            .install(&runtime, |_| Ok(()), &mut cpu)
            .unwrap_err()
            .contains("initialization borrow"));
        drop(guard);
        let pending = prepared(&cpu, &runtime, &candidate, &[]);
        // A different actual registry snapshot makes the prepared plan stale.
        runtime.borrow_mut().registry = Rc::new(
            Registry::register(&[], &[], |_, _| unreachable!(), |_, _| Ok(()))
                .unwrap()
                .registry,
        );
        assert!(pending
            .install(&runtime, |_| Ok(()), &mut cpu)
            .unwrap_err()
            .contains("changed after"));
    }
    struct FailingMemory {
        bytes: [u8; 16],
        writes: usize,
    }
    impl SlotMemory for FailingMemory {
        fn read(&mut self, a: u64, n: usize) -> Result<Vec<u8>, String> {
            self.bytes
                .get(a as usize..a as usize + n)
                .map(|b| b.to_vec())
                .ok_or("unmapped".into())
        }
        fn validate_write(&self, a: u64, n: usize) -> Result<(), String> {
            if a.checked_add(n as u64).is_some_and(|end| end <= 16) {
                Ok(())
            } else {
                Err("unmapped".into())
            }
        }
        fn write(&mut self, a: u64, b: &[u8]) -> Result<(), String> {
            self.writes += 1;
            if self.writes == 2 {
                self.bytes[a as usize] = 255;
                return Err("partial backend write".into());
            }
            self.bytes[a as usize..a as usize + b.len()].copy_from_slice(b);
            Ok(())
        }
    }
    #[test]
    fn failed_selector_commit_restores_guest_bytes_and_live_runtime_then_valid_install_adds_uninitialized_class(
    ) {
        let (mut cpu, runtime, candidate, child) = fixture();
        let active = runtime.borrow().registry.clone();
        let mut pending = prepared(&cpu, &runtime, &candidate, &[]);
        pending.fixups = vec![
            SelectorFixup {
                slot: 0,
                original: 1,
                canonical: 10,
            },
            SelectorFixup {
                slot: 8,
                original: 2,
                canonical: 20,
            },
        ];
        let mut memory = FailingMemory {
            bytes: [0; 16],
            writes: 0,
        };
        memory.bytes[..8].copy_from_slice(&1u64.to_le_bytes());
        memory.bytes[8..].copy_from_slice(&2u64.to_le_bytes());
        let original = memory.bytes;
        let child_meta = candidate
            .registry
            .classes()
            .find(|c| c.address == child)
            .unwrap()
            .isa;
        assert!(pending
            .install(&runtime, |_| Ok(()), &mut memory)
            .unwrap_err()
            .contains("rolled back"));
        assert_eq!(memory.bytes, original);
        assert!(Rc::ptr_eq(&runtime.borrow().registry, &active));
        assert!(!runtime.borrow().lifetime.borrow().contains_identity(child));
        assert!(!runtime
            .borrow()
            .lifetime
            .borrow()
            .contains_identity(child_meta));
        assert!(!runtime
            .borrow()
            .registry
            .classes()
            .any(|c| c.address == child));
        prepared(&cpu, &runtime, &candidate, &[])
            .install(&runtime, |_| Ok(()), &mut cpu)
            .unwrap();
        assert_eq!(
            runtime.borrow().registry.lookup_class("UnityFramework"),
            Some(child)
        );
        assert!(!runtime
            .borrow()
            .initialization
            .borrow()
            .is_initialized(child));
        assert!(!Rc::ptr_eq(&runtime.borrow().registry, &active));
        let lifetime = runtime.borrow().lifetime.clone();
        assert_eq!(
            lifetime.borrow().plan_release(child).unwrap(),
            super::super::objc_lifetime::ReleasePlan::NoOp
        );
        assert_eq!(
            lifetime.borrow().plan_release(child_meta).unwrap(),
            super::super::objc_lifetime::ReleasePlan::NoOp
        );
    }
    #[test]
    fn class_ownership_collision_and_active_borrow_prevent_publication() {
        let (mut cpu, runtime, candidate, child) = fixture();
        let active = runtime.borrow().registry.clone();
        let lifetime = runtime.borrow().lifetime.clone();
        let pending = prepared(&cpu, &runtime, &candidate, &[]);
        let guard = lifetime.borrow();
        assert!(pending
            .install(&runtime, |_| Ok(()), &mut cpu)
            .unwrap_err()
            .contains("ownership borrow"));
        drop(guard);
        lifetime
            .borrow_mut()
            .register_object(child, child, 1)
            .unwrap();
        assert!(prepared(&cpu, &runtime, &candidate, &[])
            .install(&runtime, |_| Ok(()), &mut cpu)
            .unwrap_err()
            .contains("identity collision"));
        assert!(Rc::ptr_eq(&runtime.borrow().registry, &active));
        assert_eq!(
            lifetime.borrow().plan_release(child).unwrap(),
            super::super::objc_lifetime::ReleasePlan::Deallocate {
                object: child,
                class: child
            }
        );
    }
    fn class(address: u64, name: &str) -> Class {
        Class {
            address,
            isa: address + 8,
            superclass: 0,
            name: name.into(),
            flags: 2,
            instance_start: 0,
            instance_size: 8,
            methods: vec![],
        }
    }
    #[test]
    fn identical_identity_is_deduplicated_but_name_aliases_are_rejected() {
        let active = class(8, "NSObject");
        let copy = active.clone();
        let added = class(32, "UnityFramework");
        let (roots, new) =
            merge_class_identities([&active].into_iter(), [&copy, &added].into_iter()).unwrap();
        assert_eq!(roots, vec![8, 32]);
        assert_eq!(new, vec![added]);
        let alias = class(64, "NSObject");
        assert!(
            merge_class_identities([&active].into_iter(), [&alias].into_iter())
                .unwrap_err()
                .contains("collision")
        );
    }
    #[test]
    fn changed_metadata_at_live_address_is_rejected() {
        let active = class(8, "NSObject");
        let mut changed = active.clone();
        changed.instance_size = 16;
        assert!(merge_class_identities([&active].into_iter(), [&changed].into_iter()).is_err());
    }
}

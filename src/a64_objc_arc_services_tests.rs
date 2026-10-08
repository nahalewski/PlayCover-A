/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Regression coverage for objc_storeStrong and registered-class
//! lookup/introspection, executed through real guest service trampolines.
use super::bridge::{GuestBridge, GuestCall, ServiceId};
use super::objc_arc_services::install;
use super::objc_execution::Initialization;
use super::objc_execution_services::ObjectRuntime;
use super::objc_heap::GuestObjectHeap;
use super::objc_lifetime::{Lifetime, ReleasePlan};
use super::objc_lifetime_services;
use super::objc_metadata::Registry;
use super::objc_namespace::{ClassSpec, MethodSpec, Namespace};
use super::A64Cpu;
use std::cell::RefCell;
use std::rc::Rc;

const CODE: u64 = 0x20000;
const READ_ONLY: u64 = 0x30000;
const HEAP: u64 = 0x40000;
const BRIDGE: u64 = 0x50000;
const UNMAPPED: u64 = 0x90000;
const NAMESPACE: u64 = 0xa0000;
const OBJECT_A: u64 = HEAP;
const OBJECT_B: u64 = HEAP + 0x40;
const GADGET_OBJECT: u64 = HEAP + 0x80;
const SLOT: u64 = HEAP + 0x100;
const UNKNOWN: u64 = HEAP + 0x200;

struct Fixture {
    cpu: A64Cpu,
    bridge: GuestBridge,
    lifetime: Rc<RefCell<Lifetime>>,
    runtime: Rc<RefCell<ObjectRuntime>>,
    namespace: Namespace,
    services: Vec<(&'static str, ServiceId)>,
}

impl Fixture {
    fn new(custom_core: bool) -> Self {
        Self::with_gadget(custom_core, true)
    }
    fn with_gadget(custom_core: bool, include_gadget: bool) -> Self {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(CODE, 4096, 5).unwrap();
        cpu.try_write_bytes(CODE, &0xd65f03c0u32.to_le_bytes())
            .unwrap();
        cpu.map_zeroed(READ_ONLY, 4096, 1).unwrap();
        cpu.map_zeroed(HEAP, 4096, 3).unwrap();
        let widget_methods = if custom_core {
            vec![MethodSpec {
                selector: "class",
                types: "#16@0:8",
                implementation: CODE,
            }]
        } else {
            vec![]
        };
        let namespace = Namespace::map(
            &mut cpu,
            NAMESPACE,
            &[
                ClassSpec {
                    name: "NSObject",
                    parent: None,
                    instance_size: 8,
                    instance_methods: vec![],
                    class_methods: vec![],
                },
                ClassSpec {
                    name: "Widget",
                    parent: Some("NSObject"),
                    instance_size: 16,
                    instance_methods: widget_methods,
                    class_methods: vec![],
                },
                ClassSpec {
                    name: "Gadget",
                    parent: Some("NSObject"),
                    instance_size: 8,
                    instance_methods: vec![],
                    class_methods: vec![],
                },
            ],
        )
        .unwrap();
        let registry = Rc::new(
            Registry::register(
                &if include_gadget {
                    namespace.class_addresses.clone()
                } else {
                    vec![
                        namespace.class_address("NSObject").unwrap(),
                        namespace.class_address("Widget").unwrap(),
                    ]
                },
                &[],
                |address, length| {
                    let mut bytes = vec![0; length];
                    cpu.read_guest_into(address, &mut bytes)?;
                    Ok(bytes)
                },
                |address, length| {
                    if length == 4 && cpu.mapped_permissions(address).is_some_and(|p| p & 4 != 0) {
                        Ok(())
                    } else {
                        Err("fixture executable mapping required".into())
                    }
                },
            )
            .unwrap()
            .registry,
        );
        let lifetime = Rc::new(RefCell::new(Lifetime::default()));
        // As owned Foundation startup does: class objects are immortal.
        for class in registry.classes() {
            lifetime
                .borrow_mut()
                .register_immortal(class.address)
                .unwrap();
        }
        let mut bridge = GuestBridge::map(&mut cpu, BRIDGE).unwrap();
        let heap = GuestObjectHeap::map(&mut cpu, 0x100000, 4096).unwrap();
        let runtime = Rc::new(RefCell::new(
            ObjectRuntime::new(
                registry.clone(),
                Rc::new(RefCell::new(
                    Initialization::new(registry.classes().cloned()).unwrap(),
                )),
                lifetime.clone(),
                heap,
                namespace.class_address("NSObject").unwrap(),
            )
            .unwrap(),
        ));
        let mut services =
            objc_lifetime_services::install(&mut bridge, &mut cpu, lifetime.clone()).unwrap();
        let release = services
            .iter()
            .find(|(name, _)| *name == "_objc_release")
            .unwrap()
            .1
            .guest_address();
        services.extend(install(&mut bridge, &mut cpu, runtime.clone(), release).unwrap());
        let mut fixture = Self {
            cpu,
            bridge,
            lifetime,
            runtime,
            namespace,
            services,
        };
        let widget = fixture.class("Widget");
        let gadget = fixture.class("Gadget");
        for (object, class) in [
            (OBJECT_A, widget),
            (OBJECT_B, widget),
            (GADGET_OBJECT, gadget),
        ] {
            fixture.put(object, class);
        }
        fixture
    }
    fn class(&self, name: &str) -> u64 {
        self.namespace.class_address(name).unwrap()
    }
    fn own(&mut self, object: u64, references: u32) {
        let class = self.cpu.read_u64(object).unwrap();
        self.lifetime
            .borrow_mut()
            .register_object(object, class, references)
            .unwrap();
    }
    fn put(&mut self, address: u64, value: u64) {
        self.cpu
            .try_write_bytes(address, &value.to_le_bytes())
            .unwrap();
    }
    fn plan(&self, object: u64) -> ReleasePlan {
        self.lifetime.borrow().plan_release(object).unwrap()
    }
    fn call(&mut self, name: &str, arguments: &[u64]) -> Result<u64, String> {
        let entry = self
            .services
            .iter()
            .find(|(service, _)| *service == name)
            .ok_or("fixture service missing")?
            .1
            .guest_address();
        self.bridge
            .call(
                &mut self.cpu,
                &GuestCall {
                    entry,
                    integers: arguments.to_vec(),
                    ..Default::default()
                },
                10_000,
            )
            .map(|values| values.integers[0])
    }
}

fn final_reference(object: u64, class: u64) -> ReleasePlan {
    ReleasePlan::Deallocate { object, class }
}

#[test]
fn store_strong_retains_new_value_releases_old_value_and_stores_it() {
    let mut f = Fixture::new(false);
    let widget = f.class("Widget");
    f.own(OBJECT_A, 1);
    f.own(OBJECT_B, 2);
    f.put(SLOT, OBJECT_B);
    f.call("_objc_storeStrong", &[SLOT, OBJECT_A]).unwrap();
    assert_eq!(f.cpu.read_u64(SLOT), Some(OBJECT_A));
    // A gained exactly one reference; B lost exactly one, through the
    // registered _objc_release guest service.
    assert_eq!(f.plan(OBJECT_A), ReleasePlan::Decrement { remaining: 1 });
    assert_eq!(f.plan(OBJECT_B), final_reference(OBJECT_B, widget));
}

#[test]
fn same_pointer_store_strong_does_not_retain_release_or_free() {
    let mut f = Fixture::new(false);
    let widget = f.class("Widget");
    // At the maximum count any retain overflows and errors, so success proves
    // the early return; an unchanged count proves no release ran either.
    f.own(OBJECT_A, u32::MAX);
    f.put(SLOT, OBJECT_A);
    f.call("_objc_storeStrong", &[SLOT, OBJECT_A]).unwrap();
    assert_eq!(f.cpu.read_u64(SLOT), Some(OBJECT_A));
    assert!(f.lifetime.borrow().contains_identity(OBJECT_A));
    assert_eq!(
        f.plan(OBJECT_A),
        ReleasePlan::Decrement {
            remaining: u32::MAX - 1
        }
    );
    assert_eq!(f.cpu.read_u64(OBJECT_A), Some(widget));
    // A last-reference object is likewise never freed by a same-pointer store.
    f.own(OBJECT_B, 1);
    f.put(SLOT, OBJECT_B);
    f.call("_objc_storeStrong", &[SLOT, OBJECT_B]).unwrap();
    assert_eq!(f.plan(OBJECT_B), final_reference(OBJECT_B, widget));
}

#[test]
fn store_strong_nil_new_and_nil_old_values() {
    let mut f = Fixture::new(false);
    let widget = f.class("Widget");
    f.own(OBJECT_A, 1);
    // nil -> nil is a no-op.
    f.call("_objc_storeStrong", &[SLOT, 0]).unwrap();
    assert_eq!(f.cpu.read_u64(SLOT), Some(0));
    // nil old value: retain and store only.
    f.call("_objc_storeStrong", &[SLOT, OBJECT_A]).unwrap();
    assert_eq!(f.cpu.read_u64(SLOT), Some(OBJECT_A));
    assert_eq!(f.plan(OBJECT_A), ReleasePlan::Decrement { remaining: 1 });
    // nil new value: release only (ARC's local-variable release idiom).
    f.call("_objc_storeStrong", &[SLOT, 0]).unwrap();
    assert_eq!(f.cpu.read_u64(SLOT), Some(0));
    assert_eq!(f.plan(OBJECT_A), final_reference(OBJECT_A, widget));
    // Immortal registered class objects are accepted and stay immortal.
    f.call("_objc_storeStrong", &[SLOT, widget]).unwrap();
    assert_eq!(f.cpu.read_u64(SLOT), Some(widget));
    assert_eq!(f.plan(widget), ReleasePlan::NoOp);
}

#[test]
fn store_strong_final_release_is_routed_to_registered_release_service() {
    let mut f = Fixture::new(false);
    f.own(OBJECT_B, 1);
    f.put(SLOT, OBJECT_B);
    // The lifetime-only release service cannot run guest -dealloc; its genuine
    // error must surface instead of a fabricated free.
    let error = f.call("_objc_storeStrong", &[SLOT, 0]).unwrap_err();
    assert!(error.contains("guest -dealloc"), "{error}");
    assert!(f.lifetime.borrow().contains_identity(OBJECT_B));
}

#[test]
fn store_strong_rejects_bad_locations_without_ownership_change() {
    let mut f = Fixture::new(false);
    let widget = f.class("Widget");
    f.own(OBJECT_A, 1);
    for location in [0, SLOT + 4, UNMAPPED, 0x8000_0000_0000_0000] {
        assert!(f.call("_objc_storeStrong", &[location, OBJECT_A]).is_err());
    }
    // Readable but not writable: rejected before retain.
    assert!(f.call("_objc_storeStrong", &[READ_ONLY, OBJECT_A]).is_err());
    assert_eq!(f.cpu.read_u64(READ_ONLY), Some(0));
    assert_eq!(f.plan(OBJECT_A), final_reference(OBJECT_A, widget));
}

#[test]
fn store_strong_rejects_unknown_new_and_old_values_without_mutation() {
    let mut f = Fixture::new(false);
    let widget = f.class("Widget");
    f.own(OBJECT_A, 1);
    for unknown in [UNKNOWN, 0x8000_0000_0000_0001, OBJECT_A + 1] {
        assert!(f.call("_objc_storeStrong", &[SLOT, unknown]).is_err());
        assert_eq!(f.cpu.read_u64(SLOT), Some(0));
    }
    f.put(SLOT, UNKNOWN);
    assert!(f.call("_objc_storeStrong", &[SLOT, OBJECT_A]).is_err());
    assert_eq!(f.cpu.read_u64(SLOT), Some(UNKNOWN));
    assert_eq!(f.plan(OBJECT_A), final_reference(OBJECT_A, widget));
}

#[test]
fn look_up_class_and_get_class_return_registered_pointer_or_null() {
    let mut f = Fixture::new(false);
    let widget = f.class("Widget");
    let gadget = f.class("Gadget");
    f.cpu.try_write_bytes(HEAP + 0x800, b"Widget\0").unwrap();
    f.cpu.try_write_bytes(HEAP + 0x820, b"Missing\0").unwrap();
    f.cpu.try_write_bytes(HEAP + 0x840, b"\0").unwrap();
    // A name ending exactly at the end of its mapping.
    f.cpu.try_write_bytes(HEAP + 0xff9, b"Gadget\0").unwrap();
    for service in ["_objc_getClass", "_objc_lookUpClass"] {
        assert_eq!(f.call(service, &[HEAP + 0x800]).unwrap(), widget);
        assert_eq!(f.call(service, &[HEAP + 0xff9]).unwrap(), gadget);
        assert_eq!(f.call(service, &[HEAP + 0x820]).unwrap(), 0);
        assert_eq!(f.call(service, &[HEAP + 0x840]).unwrap(), 0);
        assert_eq!(f.call(service, &[0]).unwrap(), 0);
    }
    // Metaclasses are not looked up by name.
    assert_ne!(
        f.call("_objc_getClass", &[HEAP + 0x800]).unwrap(),
        f.namespace.metaclass_address("Widget").unwrap()
    );
}

#[test]
fn class_lookup_rejects_bad_guest_name_pointers() {
    let mut f = Fixture::new(false);
    // Unterminated name running into unmapped memory.
    f.cpu.try_write_bytes(HEAP + 0xff9, b"Gadgets").unwrap();
    for service in ["_objc_getClass", "_objc_lookUpClass"] {
        assert!(f.call(service, &[UNMAPPED]).is_err());
        assert!(f.call(service, &[HEAP + 0xff9]).is_err());
    }
}

#[test]
fn object_get_class_of_owned_instance_and_class_object() {
    let mut f = Fixture::new(false);
    let widget = f.class("Widget");
    let widget_meta = f.namespace.metaclass_address("Widget").unwrap();
    let root_meta = f.namespace.metaclass_address("NSObject").unwrap();
    f.own(OBJECT_A, 1);
    assert_eq!(f.call("_object_getClass", &[OBJECT_A]).unwrap(), widget);
    assert_eq!(f.call("_object_getClass", &[widget]).unwrap(), widget_meta);
    assert_eq!(
        f.call("_object_getClass", &[widget_meta]).unwrap(),
        root_meta
    );
    assert_eq!(f.call("_object_getClass", &[0]).unwrap(), 0);
    // Introspection does not change ownership.
    assert_eq!(f.plan(OBJECT_A), final_reference(OBJECT_A, widget));
}

#[test]
fn introspection_rejects_unknown_tagged_and_unaligned_receivers() {
    let mut f = Fixture::new(false);
    let root = f.class("NSObject");
    // OBJECT_B has a valid isa in memory but no Lifetime ownership.
    for receiver in [UNKNOWN, OBJECT_B, 0x8000_0000_0000_0010, OBJECT_A + 1] {
        assert!(f.call("_object_getClass", &[receiver]).is_err());
        assert!(f.call("_objc_opt_class", &[receiver]).is_err());
        assert!(f
            .call("_objc_opt_isKindOfClass", &[receiver, root])
            .is_err());
    }
}

#[test]
fn introspection_rejects_bad_guest_pointer_and_encoded_isa() {
    let mut f = Fixture::new(false);
    let widget = f.class("Widget");
    // Owned identity whose storage is not mapped guest memory.
    f.lifetime
        .borrow_mut()
        .register_object(UNMAPPED, widget, 1)
        .unwrap();
    assert!(f.call("_object_getClass", &[UNMAPPED]).is_err());
    // Owned identity with a non-pointer (encoded) isa.
    f.put(OBJECT_A, widget | 1);
    f.lifetime
        .borrow_mut()
        .register_object(OBJECT_A, widget, 1)
        .unwrap();
    assert!(f
        .call("_object_getClass", &[OBJECT_A])
        .unwrap_err()
        .contains("encoded"));
    // Owned identity whose isa is an unregistered pointer.
    f.put(OBJECT_A, UNKNOWN);
    assert!(f.call("_object_getClass", &[OBJECT_A]).is_err());
}

#[test]
fn opt_class_and_is_kind_of_class_follow_registered_hierarchy() {
    let mut f = Fixture::new(false);
    let root = f.class("NSObject");
    let widget = f.class("Widget");
    let gadget = f.class("Gadget");
    f.own(OBJECT_A, 1);
    assert_eq!(f.call("_objc_opt_class", &[OBJECT_A]).unwrap(), widget);
    assert_eq!(f.call("_objc_opt_class", &[widget]).unwrap(), widget);
    assert_eq!(f.call("_objc_opt_class", &[0]).unwrap(), 0);
    let kind = |f: &mut Fixture, object: u64, class: u64| -> u64 {
        f.call("_objc_opt_isKindOfClass", &[object, class]).unwrap()
    };
    assert_eq!(kind(&mut f, OBJECT_A, widget), 1);
    assert_eq!(kind(&mut f, OBJECT_A, root), 1);
    assert_eq!(kind(&mut f, OBJECT_A, gadget), 0);
    assert_eq!(kind(&mut f, OBJECT_A, 0), 0);
    assert_eq!(kind(&mut f, OBJECT_A, UNKNOWN), 0);
    // A class object walks its metaclass chain, which reaches the root class.
    assert_eq!(kind(&mut f, widget, root), 1);
    assert_eq!(kind(&mut f, widget, widget), 0);
    assert_eq!(kind(&mut f, 0, root), 0);
}

#[test]
fn opt_fast_paths_reject_custom_core_overrides() {
    let mut f = Fixture::new(true);
    let root = f.class("NSObject");
    let widget = f.class("Widget");
    let gadget = f.class("Gadget");
    f.own(OBJECT_A, 1);
    f.own(GADGET_OBJECT, 1);
    assert!(f
        .call("_objc_opt_class", &[OBJECT_A])
        .unwrap_err()
        .contains("core method"));
    assert!(f
        .call("_objc_opt_isKindOfClass", &[OBJECT_A, root])
        .is_err());
    // object_getClass never dispatches, and unaffected classes keep the
    // genuine fast path.
    assert_eq!(f.call("_object_getClass", &[OBJECT_A]).unwrap(), widget);
    assert_eq!(f.call("_objc_opt_class", &[GADGET_OBJECT]).unwrap(), gadget);
}

#[test]
fn install_rejects_non_executable_release_entry_and_duplicate_services() {
    let mut f = Fixture::new(false);
    for entry in [0, HEAP, CODE + 2, CODE] {
        assert!(install(&mut f.bridge, &mut f.cpu, f.runtime.clone(), entry).is_err());
    }
    // Already registered names are never shadowed by a second handler.
    assert!(install(&mut f.bridge, &mut f.cpu, f.runtime.clone(), CODE).is_err());
}

#[test]
fn class_services_follow_real_publication_and_enrolled_metaclass_identity() {
    use super::objc_registration::RegisteredImages;
    use std::collections::BTreeMap;
    let mut f = Fixture::with_gadget(false, false);
    let name = HEAP + 0x300;
    f.cpu.try_write_bytes(name, b"Gadget\0").unwrap();
    assert_eq!(f.call("_objc_getClass", &[name]).unwrap(), 0);
    let registered = Registry::register(
        &f.namespace.class_addresses,
        &[],
        |a, n| {
            let mut bytes = vec![0; n];
            f.cpu.read_guest_into(a, &mut bytes)?;
            Ok(bytes)
        },
        |_, _| Ok(()),
    )
    .unwrap();
    let candidate = RegisteredImages {
        initialization: Initialization::new(registered.registry.classes().cloned()).unwrap(),
        registry: registered.registry,
        images: vec![],
        class_owners: BTreeMap::new(),
        selector_fixups: vec![],
    };
    let pending = super::objc_publication::prepare(
        &f.runtime.borrow().registry,
        &candidate,
        &[],
        &[],
        None,
        |a, n| {
            let mut bytes = vec![0; n];
            f.cpu.read_guest_into(a, &mut bytes)?;
            Ok(bytes)
        },
        |_, _| Ok(()),
    )
    .unwrap();
    pending.install(&f.runtime, |_| Ok(()), &mut f.cpu).unwrap();
    let gadget = f.class("Gadget");
    let meta = f.namespace.metaclass_address("Gadget").unwrap();
    assert_eq!(f.call("_objc_getClass", &[name]).unwrap(), gadget);
    assert_eq!(f.call("_objc_lookUpClass", &[name]).unwrap(), gadget);
    assert_eq!(f.call("_object_getClass", &[gadget]).unwrap(), meta);
    assert_eq!(f.call("_objc_opt_class", &[gadget]).unwrap(), gadget);
    assert_eq!(
        f.call("_objc_opt_isKindOfClass", &[gadget, meta]).unwrap(),
        1
    );
    f.call("_objc_storeStrong", &[SLOT, gadget]).unwrap();
    assert_eq!(f.cpu.read_u64(SLOT), Some(gadget));
    assert_eq!(f.plan(gadget), ReleasePlan::NoOp);
    assert!(!f
        .runtime
        .borrow()
        .initialization
        .borrow()
        .is_initialized(gadget));
}

#[test]
fn final_strong_release_executes_guest_dealloc_nested_clear_and_real_disposal() {
    use super::bridge::ReturnValues;
    use super::objc_execution_services::{self, Selectors};
    use std::cell::Cell;
    let mut cpu = A64Cpu::new_sparse();
    cpu.map_zeroed(CODE, 4096, 5).unwrap();
    cpu.try_write_bytes(CODE, &0xd65f03c0u32.to_le_bytes())
        .unwrap();
    cpu.map_zeroed(HEAP, 4096, 3).unwrap();
    let mut namespace = Namespace::map(
        &mut cpu,
        NAMESPACE,
        &[
            ClassSpec {
                name: "NSObject",
                parent: None,
                instance_size: 8,
                instance_methods: vec![],
                class_methods: vec![],
            },
            ClassSpec {
                name: "Widget",
                parent: Some("NSObject"),
                instance_size: 8,
                instance_methods: vec![MethodSpec {
                    selector: "dealloc",
                    types: "v16@0:8",
                    implementation: CODE,
                }],
                class_methods: vec![],
            },
        ],
    )
    .unwrap();
    let registry = Rc::new(
        Registry::register(
            &namespace.class_addresses,
            &[],
            |a, n| {
                let mut bytes = vec![0; n];
                cpu.read_guest_into(a, &mut bytes)?;
                Ok(bytes)
            },
            |_, _| Ok(()),
        )
        .unwrap()
        .registry,
    );
    let lifetime = Rc::new(RefCell::new(Lifetime::default()));
    for class in registry.classes() {
        lifetime
            .borrow_mut()
            .register_immortal(class.address)
            .unwrap();
    }
    let runtime = Rc::new(RefCell::new(
        ObjectRuntime::new(
            registry.clone(),
            Rc::new(RefCell::new(
                Initialization::new(registry.classes().cloned()).unwrap(),
            )),
            lifetime.clone(),
            GuestObjectHeap::map(&mut cpu, 0x100000, 4096).unwrap(),
            namespace.class_address("NSObject").unwrap(),
        )
        .unwrap(),
    ));
    let mut bridge = GuestBridge::map(&mut cpu, BRIDGE).unwrap();
    let mut services =
        objc_lifetime_services::install_without_release(&mut bridge, &mut cpu, lifetime.clone())
            .unwrap();
    let (start, end) = namespace.code_range();
    services.extend(
        objc_execution_services::install(
            &mut cpu,
            &mut bridge,
            runtime.clone(),
            Selectors::from_registry(&registry).unwrap(),
            Rc::new(move |a, n| {
                if n == 4 && ((CODE..CODE + 4096).contains(&a) || (start..end).contains(&a)) {
                    Ok(())
                } else {
                    Err("outside fixture IMP".into())
                }
            }),
            UNMAPPED,
        )
        .unwrap(),
    );
    let find = |services: &Vec<(&'static str, ServiceId)>, name: &str| {
        services
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap()
            .1
            .guest_address()
    };
    let dispose = find(&services, "_object_dispose");
    namespace
        .link_root_services(&mut cpu, find(&services, "_class_createInstance"), dispose)
        .unwrap();
    services.extend(
        install(
            &mut bridge,
            &mut cpu,
            runtime.clone(),
            find(&services, "_objc_release"),
        )
        .unwrap(),
    );
    let strong = find(&services, "_objc_storeStrong");
    let expected_new = Rc::new(Cell::new(0u64));
    let seen = Rc::new(Cell::new(false));
    let wanted = expected_new.clone();
    let observed = seen.clone();
    let cleanup = bridge
        .register_service(&mut cpu, "fixture_dealloc_cleanup", move |frame| {
            let stored = frame.read(SLOT, 8)?;
            if u64::from_le_bytes(stored.try_into().unwrap()) != wanted.get() {
                return Err("strong slot not committed before dealloc".into());
            }
            observed.set(true);
            frame.request_guest_call(
                GuestCall {
                    entry: strong,
                    integers: vec![SLOT + 8, 0],
                    ..Default::default()
                },
                |r| r.map(|_| ()),
            )?;
            Ok(ReturnValues::integer(0))
        })
        .unwrap()
        .guest_address();
    // Real guest -dealloc calls cleanup, then the genuine object_dispose service.
    let words = [
        0xa9bf7bf3u32,
        0xaa0003f3,
        0x58000110,
        0xd63f0200,
        0xaa1303e0,
        0x580000f0,
        0xd63f0200,
        0xa8c17bf3,
        0xd65f03c0,
        0xd503201f,
    ];
    let mut code: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    code.extend_from_slice(&cleanup.to_le_bytes());
    code.extend_from_slice(&dispose.to_le_bytes());
    cpu.try_write_bytes(CODE, &code).unwrap();
    let mut invoke = |cpu: &mut A64Cpu, entry, args: Vec<u64>| {
        bridge
            .call(
                cpu,
                &GuestCall {
                    entry,
                    integers: args,
                    ..Default::default()
                },
                20_000,
            )
            .unwrap()
            .integers[0]
    };
    let alloc = find(&services, "_objc_alloc_init");
    let old = invoke(
        &mut cpu,
        alloc,
        vec![namespace.class_address("Widget").unwrap()],
    );
    let child = invoke(
        &mut cpu,
        alloc,
        vec![namespace.class_address("NSObject").unwrap()],
    );
    let new = invoke(
        &mut cpu,
        alloc,
        vec![namespace.class_address("NSObject").unwrap()],
    );
    expected_new.set(new);
    cpu.try_write_bytes(SLOT, &old.to_le_bytes()).unwrap();
    cpu.try_write_bytes(SLOT + 8, &child.to_le_bytes()).unwrap();
    cpu.set_vector(3, [0x1234, 0x5678]);
    let vector = cpu.vector(3);
    invoke(&mut cpu, strong, vec![SLOT, new]);
    assert!(seen.get());
    assert_eq!(cpu.read_u64(SLOT), Some(new));
    assert_eq!(cpu.read_u64(SLOT + 8), Some(0));
    assert!(!lifetime.borrow().contains_identity(old));
    assert!(!lifetime.borrow().contains_identity(child));
    assert_eq!(
        lifetime.borrow().plan_release(new).unwrap(),
        ReleasePlan::Decrement { remaining: 1 }
    );
    assert_eq!(cpu.read_u64(old), Some(0));
    assert_eq!(cpu.read_u64(child), Some(0));
    assert_eq!(cpu.vector(3), vector);
}

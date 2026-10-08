/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Explicit ObjC guest execution services. No framework classes are invented.
//! Registration needs a bound Registry, real superclass +initialize execution,
//! and an authorized implemented ordinary ownership root. Custom +alloc and
//! -init execute as guest methods; allocation never substitutes for them.
use super::bridge::{GuestBridge, ReturnValues, ServiceFrame, ServiceId};
use super::objc_dealloc::schedule_final_release;
use super::objc_execution::{dispatch_prepared, queue_initialization, Initialization};
use super::objc_heap::{GuestObjectHeap, PlainClass};
use super::objc_lifetime::{DeallocationTicket, Lifetime, ReleasePlan};
use super::objc_metadata::{MessagePlan, Registry};
use super::A64Cpu;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Canonical named selectors obtained from this exact Registry; callers must
/// not substitute arbitrary selector pointers with matching ABI signatures.
pub(super) struct Selectors {
    pub initialize: u64,
    pub alloc: u64,
    pub init: u64,
    pub dealloc: u64,
}
impl Selectors {
    pub(super) fn from_registry(registry: &Registry) -> Result<Self, String> {
        let named = |name| {
            registry
                .selector_named(name)
                .ok_or_else(|| format!("registered guest selector {name} is unavailable"))
        };
        Ok(Self {
            initialize: named("initialize")?,
            alloc: named("alloc")?,
            init: named("init")?,
            dealloc: named("dealloc")?,
        })
    }
}
pub(super) struct ObjectRuntime {
    pub registry: Rc<Registry>,
    pub initialization: Rc<RefCell<Initialization>>,
    pub lifetime: Rc<RefCell<Lifetime>>,
    heap: GuestObjectHeap,
    ordinary_root: u64,
    classes: BTreeMap<u64, PlainClass>,
    tickets: BTreeMap<u64, DeallocationTicket>,
}
impl ObjectRuntime {
    pub(super) fn allocate_plain(
        &mut self,
        frame: &mut ServiceFrame<'_>,
        class: u64,
        extra: usize,
    ) -> Result<u64, String> {
        if class == 0 {
            return Ok(0);
        }
        if !self
            .registry
            .classes()
            .any(|c| c.address == class && c.flags & 1 == 0)
        {
            return Err("guest allocator class is not registered".into());
        }
        if !self.classes.contains_key(&class) {
            let initialized = self
                .initialization
                .try_borrow()
                .map_err(|_| "reentrant initialization policy")?
                .is_initialized(class);
            let policy = PlainClass::read(class, initialized, self.ordinary_root, |a, n| {
                frame.read(a, n)
            })?;
            if self.classes.len() >= 4096 {
                return Err("guest allocation class policy limit".into());
            }
            self.classes.insert(class, policy);
        }
        let lifetime = self.lifetime.clone();
        let mut lifetime = lifetime
            .try_borrow_mut()
            .map_err(|_| "reentrant guest allocation ownership")?;
        let completed: Vec<_> = self
            .tickets
            .iter()
            .filter(|(object, _)| !lifetime.contains_identity(**object))
            .map(|(&object, &ticket)| (object, ticket))
            .collect();
        for (object, ticket) in completed {
            self.heap.reclaim(ticket, &lifetime)?;
            self.tickets.remove(&object);
        }
        self.heap
            .allocate(&self.classes[&class], extra, &mut lifetime, |a, b| {
                frame.write(a, b)
            })
    }
    pub(super) fn new(
        registry: Rc<Registry>,
        initialization: Rc<RefCell<Initialization>>,
        lifetime: Rc<RefCell<Lifetime>>,
        heap: GuestObjectHeap,
        ordinary_root: u64,
    ) -> Result<Self, String> {
        if ordinary_root == 0 || ordinary_root & 7 != 0 {
            return Err("invalid implemented ordinary Objective-C ownership root".into());
        }
        if !registry
            .classes()
            .any(|c| c.address == ordinary_root && c.flags & 3 == 2 && c.superclass == 0)
        {
            return Err("ordinary ownership root is not a registered normal root class".into());
        }
        Ok(Self {
            registry,
            initialization,
            lifetime,
            heap,
            ordinary_root,
            classes: BTreeMap::new(),
            tickets: BTreeMap::new(),
        })
    }
}

/// `executable` is a read-only validator for the mapped image graph. The bridge
/// independently verifies every actual executable target before tail dispatch.
/// The caller retains its initializer/main gate until the required frameworks,
/// ownership root and guest-call policy are genuinely supported.
pub(super) fn install(
    cpu: &mut A64Cpu,
    bridge: &mut GuestBridge,
    runtime: Rc<RefCell<ObjectRuntime>>,
    selectors: Selectors,
    executable: Rc<dyn Fn(u64, usize) -> Result<(), String>>,
    alloc_init_thunk: u64,
) -> Result<Vec<(&'static str, ServiceId)>, String> {
    install_inner(
        cpu,
        bridge,
        runtime,
        selectors,
        executable,
        alloc_init_thunk,
        false,
    )
}

/// Upgrade only the explicit ARC release service, retaining its guest address
/// and token so previously verified application import bindings stay valid.
pub(super) fn install_upgrading_release(
    cpu: &mut A64Cpu,
    bridge: &mut GuestBridge,
    runtime: Rc<RefCell<ObjectRuntime>>,
    selectors: Selectors,
    executable: Rc<dyn Fn(u64, usize) -> Result<(), String>>,
    alloc_init_thunk: u64,
) -> Result<Vec<(&'static str, ServiceId)>, String> {
    install_inner(
        cpu,
        bridge,
        runtime,
        selectors,
        executable,
        alloc_init_thunk,
        true,
    )
}

fn install_inner(
    cpu: &mut A64Cpu,
    bridge: &mut GuestBridge,
    runtime: Rc<RefCell<ObjectRuntime>>,
    selectors: Selectors,
    executable: Rc<dyn Fn(u64, usize) -> Result<(), String>>,
    alloc_init_thunk: u64,
    upgrade_release: bool,
) -> Result<Vec<(&'static str, ServiceId)>, String> {
    let registry = runtime.borrow().registry.clone();
    for (name, selector) in [
        ("initialize", selectors.initialize),
        ("alloc", selectors.alloc),
        ("init", selectors.init),
        ("dealloc", selectors.dealloc),
    ] {
        if registry.selector_named(name) != Some(selector)
            || registry.canonical_selector(selector)? != selector
        {
            return Err(format!("invalid canonical guest selector for {name}"));
        }
    }
    // Own generic thunk implements alloc followed by init, with actual returned
    // receiver and normal nil behavior. Validate placement before registering.
    if alloc_init_thunk == 0 || alloc_init_thunk & 4095 != 0 {
        return Err("ObjC alloc/init thunk placement invalid".into());
    }
    for address in alloc_init_thunk
        ..alloc_init_thunk
            .checked_add(4096)
            .ok_or("ObjC thunk range overflow")?
    {
        if cpu.mapped_permissions(address).is_some() {
            return Err("ObjC thunk overlaps mapped memory".into());
        }
    }
    let mut services = Vec::new();
    let initialize_selector = selectors.initialize;
    for (name, super2, forced_selector) in [
        ("_objc_msgSend", false, None),
        ("_objc_msgSendSuper2", true, None),
        ("_objc_alloc", false, Some(selectors.alloc)),
    ] {
        let runtime = runtime.clone();
        let executable = executable.clone();
        let service = bridge.register_service(cpu, name, move |frame| {
            let (registry, state) = {
                let r = runtime
                    .try_borrow()
                    .map_err(|_| "reentrant object runtime access")?;
                (r.registry.clone(), r.initialization.clone())
            };
            let receiver = frame.integer(0)?;
            let selector = match forced_selector {
                Some(s) => s,
                None => frame.integer(1)?,
            };
            let plan = if super2 {
                registry.plan_super2(
                    receiver,
                    selector,
                    |a, n| frame.read(a, n),
                    |a, n| executable(a, n),
                )?
            } else {
                registry.plan_message(
                    receiver,
                    selector,
                    |a, n| frame.read(a, n),
                    |a, n| executable(a, n),
                )?
            };
            let MessagePlan::Invoke(invocation) = plan else {
                return Ok(ReturnValues::integer(0));
            };
            let calls = state
                .try_borrow()
                .map_err(|_| "reentrant initialization planning")?
                .plan(
                    invocation.receiver_class,
                    &registry,
                    initialize_selector,
                    |a, n| frame.read(a, n),
                    |a, n| executable(a, n),
                )?;
            if super2 {
                queue_initialization(frame, state, calls)?;
                frame.request_tail_dispatch_receiver(
                    invocation.implementation,
                    invocation.selector,
                    invocation.receiver,
                )?;
            } else {
                dispatch_prepared(frame, state, &invocation, calls)?;
            }
            Ok(ReturnValues::integer(0)) // ignored by tail dispatch
        })?;
        services.push((name, service));
    }
    let alloc = services
        .iter()
        .find(|(n, _)| *n == "_objc_alloc")
        .unwrap()
        .1;
    let msgsend = services
        .iter()
        .find(|(n, _)| *n == "_objc_msgSend")
        .unwrap()
        .1;
    // stp fp,lr; mov fp,sp; ldr x16,alloc; blr x16;
    // ldr x1,initSEL; ldr x16,msgSend; blr x16; ldp fp,lr; ret; nop;
    // literals begin at byte40, signed PC-relative literal displacements.
    let words = [
        0xa9bf7bfdu32,
        0x910003fd,
        0x58000110,
        0xd63f0200,
        0x58000101,
        0x58000130,
        0xd63f0200,
        0xa8c17bfd,
        0xd65f03c0,
        0xd503201f,
    ];
    let mut code = Vec::new();
    for word in words {
        code.extend_from_slice(&word.to_le_bytes());
    }
    for value in [
        alloc.guest_address(),
        selectors.init,
        msgsend.guest_address(),
    ] {
        code.extend_from_slice(&value.to_le_bytes());
    }
    cpu.map_zeroed(alloc_init_thunk, 4096, 5)?;
    cpu.try_write_bytes(alloc_init_thunk, &code)?;
    // An executable own thunk is not an Apple framework implementation.
    let alloc_selector = selectors.alloc;
    services.push((
        "_objc_alloc_init",
        bridge.register_service(cpu, "_objc_alloc_init", move |frame| {
            frame.request_tail_dispatch(alloc_init_thunk, alloc_selector)?;
            Ok(ReturnValues::integer(0))
        })?,
    ));

    let allocation_runtime = runtime.clone();
    services.push((
        "_class_createInstance",
        bridge.register_service(cpu, "_class_createInstance", move |frame| {
            let class = frame.integer(0)?;
            if class == 0 {
                return Ok(ReturnValues::integer(0));
            }
            let extra = usize::try_from(frame.integer(1)?)
                .map_err(|_| "guest extra allocation bytes overflow")?;
            let mut r = allocation_runtime
                .try_borrow_mut()
                .map_err(|_| "reentrant guest allocator")?;
            let object = r.allocate_plain(frame, class, extra)?;
            Ok(ReturnValues::integer(object))
        })?,
    ));

    let dispose_runtime = runtime.clone();
    services.push((
        "_object_dispose",
        bridge.register_service(cpu, "_object_dispose", move |frame| {
            let object = frame.integer(0)?;
            if object == 0 {
                return Ok(ReturnValues::integer(0));
            }
            let mut r = dispose_runtime
                .try_borrow_mut()
                .map_err(|_| "reentrant guest disposal")?;
            let ticket = *r
                .tickets
                .get(&object)
                .ok_or("guest object_dispose requires active validated deallocation")?;
            let lifetime = r.lifetime.clone();
            let mut lifetime = lifetime
                .try_borrow_mut()
                .map_err(|_| "reentrant guest disposal ownership")?;
            r.heap
                .dispose(ticket, &mut lifetime, |a, b| frame.write(a, b))?;
            Ok(ReturnValues::integer(0))
        })?,
    ));

    let release_runtime = runtime.clone();
    let dealloc_selector = selectors.dealloc;
    let release_handler = move |frame: &mut ServiceFrame<'_>| {
        let object = frame.integer(0)?;
        let (registry, state, lifetime) = {
            let r = release_runtime
                .try_borrow()
                .map_err(|_| "reentrant guest release")?;
            (
                r.registry.clone(),
                r.initialization.clone(),
                r.lifetime.clone(),
            )
        };
        let plan = lifetime
            .try_borrow()
            .map_err(|_| "reentrant guest release ownership")?
            .plan_release(object)?;
        if !matches!(plan, ReleasePlan::Deallocate { .. }) {
            lifetime
                .try_borrow_mut()
                .map_err(|_| "reentrant guest release ownership")?
                .release(object)?;
            return Ok(ReturnValues::integer(0));
        }
        let MessagePlan::Invoke(invocation) = registry.plan_message(
            object,
            dealloc_selector,
            |a, n| frame.read(a, n),
            |a, n| executable(a, n),
        )?
        else {
            return Err("guest final release resolved nil dealloc".into());
        };
        let initialized = state
            .try_borrow()
            .map_err(|_| "reentrant guest release initialization")?
            .is_initialized(invocation.receiver_class);
        let mut r = release_runtime
            .try_borrow_mut()
            .map_err(|_| "reentrant guest release reservation")?;
        if r.tickets.len() >= 65536 || r.tickets.contains_key(&object) {
            return Err("guest deallocation reservation limit or duplicate".into());
        }
        let ticket =
            schedule_final_release(frame, lifetime, &invocation, dealloc_selector, initialized)?;
        r.tickets.insert(object, ticket);
        Ok(ReturnValues::integer(0))
    };
    let release = if upgrade_release {
        bridge.replace_registered_service(cpu, "_objc_release", release_handler)?
    } else {
        bridge.register_service(cpu, "_objc_release", release_handler)?
    };
    services.push(("_objc_release", release));
    Ok(services)
}

#[cfg(test)]
mod tests {
    use super::super::bridge::GuestCall;
    use super::*;
    fn put64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut b = vec![0; 4096];
        for (at, isa, superclass, ro) in [(0, 0x10040, 0, 0x10100), (64, 0x10040, 0x10000, 0x10180)]
        {
            put64(&mut b, at, isa);
            put64(&mut b, at + 8, superclass);
            put64(&mut b, at + 32, ro);
        }
        for (ro, flags, start, size, methods) in
            [(0x100, 2, 0, 8, 0x10300), (0x180, 3, 40, 40, 0x10400)]
        {
            put32(&mut b, ro, flags);
            put32(&mut b, ro + 4, start);
            put32(&mut b, ro + 8, size);
            put64(&mut b, ro + 24, 0x10280);
            put64(&mut b, ro + 32, methods);
        }
        b[0x280..0x285].copy_from_slice(b"Root\0");
        for (at, text) in [
            (0x500, b"init\0".as_slice()),
            (0x520, b"dealloc\0".as_slice()),
            (0x540, b"initialize\0".as_slice()),
            (0x560, b"alloc\0".as_slice()),
            (0x700, b"@16@0:8\0".as_slice()),
            (0x720, b"v16@0:8\0".as_slice()),
        ] {
            b[at..at + text.len()].copy_from_slice(text);
        }
        for (list, methods) in [
            (
                0x300,
                [(0x10500, 0x10700, 0x20080), (0x10520, 0x10720, 0x20100)],
            ),
            (
                0x400,
                [(0x10540, 0x10720, 0x20000), (0x10560, 0x10700, 0x20040)],
            ),
        ] {
            put32(&mut b, list, 24);
            put32(&mut b, list + 4, 2);
            for (index, (sel, types, imp)) in methods.into_iter().enumerate() {
                let at = list + 8 + index * 24;
                put64(&mut b, at, sel);
                put64(&mut b, at + 8, types);
                put64(&mut b, at + 16, imp);
            }
        }
        b
    }
    fn write_code(cpu: &mut A64Cpu, address: u64, words: &[u32], literal: Option<u64>) {
        let mut b = Vec::new();
        for word in words {
            b.extend_from_slice(&word.to_le_bytes());
        }
        if let Some(value) = literal {
            b.extend_from_slice(&value.to_le_bytes());
        }
        cpu.try_write_bytes(address, &b).unwrap();
    }
    #[test]
    fn real_guest_initialize_custom_alloc_init_and_dispose_execute_end_to_end() {
        exercise_execution(false);
    }
    #[test]
    fn release_upgrade_preserves_prebound_address_and_executes_real_dealloc() {
        exercise_execution(true);
    }
    fn exercise_execution(upgrade_release: bool) {
        let mut cpu = A64Cpu::new_sparse();
        let metadata = fixture();
        cpu.map_zeroed(0x10000, 4096, 1).unwrap();
        cpu.try_write_bytes(0x10000, &metadata).unwrap();
        cpu.map_zeroed(0x20000, 4096, 5).unwrap();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        let executable: Rc<dyn Fn(u64, usize) -> Result<(), String>> = Rc::new(|a, n| {
            if n == 4 && (0x20000..0x21000).contains(&a) {
                Ok(())
            } else {
                Err("outside fixture executable image".into())
            }
        });
        let registration = Registry::register(
            &[0x10000],
            &[],
            |a, n| {
                let offset =
                    usize::try_from(a.checked_sub(0x10000).ok_or("fixture address underflow")?)
                        .map_err(|_| "fixture address overflow")?;
                metadata
                    .get(offset..offset.checked_add(n).ok_or("fixture length overflow")?)
                    .map(|b| b.to_vec())
                    .ok_or("fixture metadata outside mapping".into())
            },
            |a, n| executable(a, n),
        )
        .unwrap();
        let registry = Rc::new(registration.registry);
        let state = Rc::new(RefCell::new(
            Initialization::new(registry.classes().cloned()).unwrap(),
        ));
        let lifetime = Rc::new(RefCell::new(Lifetime::default()));
        let heap = GuestObjectHeap::map(&mut cpu, 0x80000, 4096).unwrap();
        let runtime = Rc::new(RefCell::new(
            ObjectRuntime::new(
                registry.clone(),
                state.clone(),
                lifetime.clone(),
                heap,
                0x10000,
            )
            .unwrap(),
        ));
        let mut bridge = GuestBridge::map(&mut cpu, 0x50000).unwrap();
        let arc = if upgrade_release {
            super::super::objc_lifetime_services::install(&mut bridge, &mut cpu, lifetime.clone())
        } else {
            super::super::objc_lifetime_services::install_without_release(
                &mut bridge,
                &mut cpu,
                lifetime.clone(),
            )
        }
        .unwrap();
        assert_eq!(arc.len(), if upgrade_release { 10 } else { 9 });
        let installer = if upgrade_release {
            install_upgrading_release
        } else {
            install
        };
        let services = installer(
            &mut cpu,
            &mut bridge,
            runtime,
            Selectors::from_registry(&registry).unwrap(),
            executable,
            0x90000,
        )
        .unwrap();
        let service = |name| {
            services
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap()
                .1
                .guest_address()
        };
        if upgrade_release {
            assert_eq!(
                arc.iter()
                    .find(|(name, _)| *name == "_objc_release")
                    .unwrap()
                    .1
                    .guest_address(),
                service("_objc_release")
            );
        }
        // +initialize increments a real guest counter.
        write_code(
            &mut cpu,
            0x20000,
            &[
                0x580000c9, 0xf940012a, 0x9100054a, 0xf900012a, 0xd65f03c0, 0xd503201f,
            ],
            Some(0x40000),
        );
        // Custom +alloc and -dealloc actually call their proper services.
        let caller = [
            0xa9bf7bfd, 0x910003fd, 0xd2800001, 0x580000b0, 0xd63f0200, 0xa8c17bfd, 0xd65f03c0,
            0xd503201f,
        ];
        write_code(
            &mut cpu,
            0x20040,
            &caller,
            Some(service("_class_createInstance")),
        );
        write_code(&mut cpu, 0x20080, &[0xd65f03c0], None); // -init returns self
        write_code(&mut cpu, 0x20100, &caller, Some(service("_object_dispose")));
        let call = GuestCall {
            entry: service("_objc_alloc_init"),
            integers: vec![0x10000],
            ..GuestCall::default()
        };
        let object = bridge.call(&mut cpu, &call, 2000).unwrap().integers[0];
        assert_eq!(object, 0x80000);
        assert_eq!(cpu.read_u64(object), Some(0x10000));
        assert_eq!(cpu.read_u64(0x40000), Some(1));
        assert!(state.borrow().is_initialized(0x10000));
        let retain = arc
            .iter()
            .find(|(name, _)| *name == "_objc_retain")
            .unwrap()
            .1
            .guest_address();
        bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: retain,
                    integers: vec![object],
                    ..GuestCall::default()
                },
                1000,
            )
            .unwrap();
        bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: service("_objc_release"),
                    integers: vec![object],
                    ..GuestCall::default()
                },
                1000,
            )
            .unwrap();
        assert!(lifetime.borrow().contains_identity(object));
        assert_eq!(cpu.read_u64(object), Some(0x10000));
        bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: service("_objc_release"),
                    integers: vec![object],
                    ..GuestCall::default()
                },
                2000,
            )
            .unwrap();
        assert_eq!(cpu.read_u64(object), Some(0));
        assert!(!lifetime.borrow().contains_identity(object));
        let reused = bridge.call(&mut cpu, &call, 2000).unwrap().integers[0];
        assert_eq!(reused, object);
        assert_eq!(cpu.read_u64(0x40000), Some(1));
        assert_eq!(cpu.mapped_permissions(0x90000), Some(5));
    }
}

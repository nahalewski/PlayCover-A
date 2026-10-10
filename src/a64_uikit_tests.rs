/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Desktop tests for the ARM64 UIKit layer. They execute real guest ARM64
//! instructions (owned NSObject lifecycle, bridge trampolines, the owned
//! -dealloc thunks and a guest subclass override) through the emulator-owned
//! Objective-C services; no Apple binary or device is involved.
use super::super::{
    bridge::{GuestBridge, GuestCall},
    dyld_objc_callbacks::ObjcImage,
    objc_execution::Initialization,
    objc_execution_services::{self, ObjectRuntime, Selectors},
    objc_heap::GuestObjectHeap,
    objc_lifetime::Lifetime,
    objc_metadata::Registry,
    objc_namespace::{ClassSpec, Namespace},
    A64Cpu,
};
use super::{
    image::{self, ClassDef, External, MethodDef, StaticObject},
    state::{Device, Model, Orientation, Rect, Screen, Size},
    Links,
};
use std::{cell::RefCell, rc::Rc};

const IMAGE_BASE: u64 = 0x20_0000;
const GUEST_CODE: u64 = 0x40000;
const GUEST_MARKER: u64 = 0x41000;
const GUEST_CLASS: u64 = 0x50000;

fn model() -> Model {
    Model::new(
        Screen {
            portrait_points: Size { width: 375.0, height: 667.0 },
            scale: 2.0,
            maximum_frames_per_second: 60,
        },
        Device {
            model: "iPhone".into(),
            system_name: "iOS".into(),
            system_version: "16.7.16".into(),
            idiom: 0,
            orientation: Orientation::LandscapeRight,
        },
    )
    .unwrap()
}

fn root_namespace(cpu: &mut A64Cpu) -> Namespace {
    Namespace::map(
        cpu,
        0xa0000,
        &[ClassSpec {
            name: "NSObject",
            parent: None,
            instance_size: 8,
            instance_methods: vec![],
            class_methods: vec![],
        }],
    )
    .unwrap()
}

#[test]
fn synthetic_image_is_a_valid_objc_macho_and_registers_against_an_external_root() {
    let mut cpu = A64Cpu::new_sparse();
    cpu.map_zeroed(0x10000, 4096, 5).unwrap();
    cpu.try_write_bytes(0x10000, &0xd65f03c0u32.to_le_bytes()).unwrap();
    let root = root_namespace(&mut cpu);
    let external = External {
        root_class: root.class_address("NSObject").unwrap(),
        root_metaclass: root.metaclass_address("NSObject").unwrap(),
        empty_cache: 0,
    };
    let defs = [
        ClassDef {
            name: "UIResponder",
            parent: None,
            instance_size: 8,
            dispatcher: 0x10000,
            instance_methods: vec![MethodDef { selector: "nextResponder", types: "@16@0:8" }],
            class_methods: vec![],
            dealloc_cleanup: None,
        },
        ClassDef {
            name: "UIView",
            parent: Some("UIResponder"),
            instance_size: 8,
            dispatcher: 0x10000,
            instance_methods: vec![MethodDef { selector: "frame", types: "{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8" }],
            class_methods: vec![MethodDef { selector: "layerClass", types: "#16@0:8" }],
            dealloc_cleanup: Some(0x10000),
        },
    ];
    let built = image::build(
        IMAGE_BASE,
        super::INSTALL_NAME,
        &defs,
        &[StaticObject { class: "UIView", size: 16 }],
        &["loadView"],
        external,
    )
    .unwrap();
    built.map(&mut cpu).unwrap();
    let layout = &built.layout;
    // The dyld ObjC notification path accepts it as an ordinary ObjC image.
    let notified = ObjcImage::read(&cpu, layout.header, super::INSTALL_NAME.into(), false)
        .unwrap()
        .expect("synthetic image must carry __objc_imageinfo");
    assert!(notified.readonly_ranges().is_empty());
    // Strict LP64 metadata parsing: graph reaches the external root.
    let mut classes = vec![external.root_class];
    classes.extend(&layout.class_list);
    let registration = Registry::register(
        &classes,
        &layout.selector_refs.values().copied().collect::<Vec<_>>(),
        |address, length| {
            let mut bytes = vec![0; length];
            cpu.read_guest_into(address, &mut bytes)?;
            Ok(bytes)
        },
        |address, _| {
            if cpu.mapped_permissions(address).is_some_and(|p| p & 4 != 0) {
                Ok(())
            } else {
                Err("non-executable".into())
            }
        },
    )
    .unwrap();
    let registry = registration.registry;
    let view = layout.class("UIView").unwrap();
    assert_eq!(registry.lookup_class("UIView"), Some(view));
    assert_eq!(cpu.read_u64(view + 8), layout.class("UIResponder"));
    assert_eq!(cpu.read_u64(layout.metaclass("UIView").unwrap()), Some(external.root_metaclass));
    assert_eq!(cpu.read_u64(layout.class("UIResponder").unwrap() + 8), Some(external.root_class));
    assert!(registry.selector_named("dealloc").is_some());
    assert!(registry.selector_named("loadView").is_some());
    // Static object isa and exports.
    assert_eq!(cpu.read_u64(layout.static_object("UIView").unwrap()), Some(view));
    assert_eq!(layout.exports()["_OBJC_CLASS_$_UIView"], view);
    assert_eq!(layout.exports()["_OBJC_METACLASS_$_UIView"], layout.metaclass("UIView").unwrap());
    // __TEXT is RX, __DATA is RW and the image stays below the isa mask.
    assert_eq!(cpu.mapped_permissions(layout.text.0), Some(5));
    assert_eq!(cpu.mapped_permissions(layout.data.0), Some(3));
    assert!(layout.data.0 + layout.data.1 <= image::ISA_ADDRESS_LIMIT);
    assert_eq!(layout.thunks.len(), 1);
    assert!(layout.contains_code(layout.thunks[0].1, image::DEALLOC_THUNK_BYTES as u64));
}

#[test]
fn image_builder_rejects_bad_parents_selectors_and_placement() {
    let external = External { root_class: 0x1000, root_metaclass: 0x1028, empty_cache: 0 };
    let def = |name: &'static str, parent: Option<&'static str>, selector: &'static str| ClassDef {
        name,
        parent,
        instance_size: 8,
        dispatcher: 0x10000,
        instance_methods: vec![MethodDef { selector, types: "v16@0:8" }],
        class_methods: vec![],
        dealloc_cleanup: None,
    };
    let build = |defs: &[ClassDef], base: u64| image::build(base, super::INSTALL_NAME, defs, &[], &[], external);
    assert!(build(&[def("UIView", Some("UIResponder"), "frame")], IMAGE_BASE).is_err());
    assert!(build(&[def("UIView", None, "bad selector")], IMAGE_BASE).is_err());
    assert!(build(&[def("UIView", None, "a"), def("UIView", None, "b")], IMAGE_BASE).is_err());
    assert!(build(&[def("UIView", None, "frame")], IMAGE_BASE + 0x1000).is_err());
    assert!(build(&[def("UIView", None, "frame")], image::ISA_ADDRESS_LIMIT).is_err());
    let mut with_dealloc = def("UIView", None, "dealloc");
    with_dealloc.dealloc_cleanup = Some(0x10000);
    assert!(build(&[with_dealloc], IMAGE_BASE).is_err());
    assert!(build(&[def("UIView", None, "frame")], IMAGE_BASE).is_ok());
}

/// Write a guest subclass `GameViewController : UIViewController` whose
/// -viewDidLoad is real guest code storing 1 to GUEST_MARKER.
fn guest_subclass(cpu: &mut A64Cpu, parent: u64, parent_meta: u64, root_meta: u64) -> u64 {
    cpu.map_zeroed(GUEST_CODE, 4096, 5).unwrap();
    cpu.map_zeroed(GUEST_MARKER, 4096, 3).unwrap();
    cpu.map_zeroed(GUEST_CLASS, 4096, 3).unwrap();
    let mut code: Vec<u8> = [0x5800_0089u32, 0xd280_002a, 0xf900_012a, 0xd65f_03c0]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    code.extend_from_slice(&GUEST_MARKER.to_le_bytes());
    cpu.try_write_bytes(GUEST_CODE, &code).unwrap();
    let (class, meta, ro, meta_ro, list, name, sel, types) = (
        GUEST_CLASS,
        GUEST_CLASS + 0x40,
        GUEST_CLASS + 0x80,
        GUEST_CLASS + 0xd0,
        GUEST_CLASS + 0x120,
        GUEST_CLASS + 0x200,
        GUEST_CLASS + 0x240,
        GUEST_CLASS + 0x260,
    );
    let w64 = |cpu: &mut A64Cpu, a: u64, v: u64| cpu.write_bytes(a, &v.to_le_bytes());
    let w32 = |cpu: &mut A64Cpu, a: u64, v: u32| cpu.write_bytes(a, &v.to_le_bytes());
    w64(cpu, class, meta);
    w64(cpu, class + 8, parent);
    w64(cpu, class + 32, ro);
    w64(cpu, meta, root_meta);
    w64(cpu, meta + 8, parent_meta);
    w64(cpu, meta + 32, meta_ro);
    // Compiled against the real SDK: instance_start is larger than our
    // eight-byte UIViewController, so no ivar sliding is ever needed.
    for (ro, flags, start, size, methods) in [(ro, 0u32, 0x3f0u32, 0x400u32, list), (meta_ro, 1, 40, 40, 0)] {
        w32(cpu, ro, flags);
        w32(cpu, ro + 4, start);
        w32(cpu, ro + 8, size);
        w64(cpu, ro + 24, name);
        w64(cpu, ro + 32, methods);
    }
    w32(cpu, list, 24);
    w32(cpu, list + 4, 1);
    w64(cpu, list + 8, sel);
    w64(cpu, list + 16, types);
    w64(cpu, list + 24, GUEST_CODE);
    cpu.write_bytes(name, b"GameViewController\0");
    cpu.write_bytes(sel, b"viewDidLoad\0");
    cpu.write_bytes(types, b"v16@0:8\0");
    class
}

#[test]
fn guest_subclass_chains_into_owned_uikit_and_disposal_clears_host_state() {
    let mut cpu = A64Cpu::new_sparse();
    let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
    let lifetime = Rc::new(RefCell::new(Lifetime::default()));
    let arc = super::super::objc_lifetime_services::install_without_release(&mut bridge, &mut cpu, lifetime.clone()).unwrap();
    let mut root = root_namespace(&mut cpu);
    let external = External {
        root_class: root.class_address("NSObject").unwrap(),
        root_metaclass: root.metaclass_address("NSObject").unwrap(),
        empty_cache: 0,
    };
    let installed = super::install(&mut cpu, &mut bridge, IMAGE_BASE, external, model()).unwrap();
    let layout = installed.layout.clone();
    let kit = installed.state.clone();
    let game = guest_subclass(
        &mut cpu,
        layout.class("UIViewController").unwrap(),
        layout.metaclass("UIViewController").unwrap(),
        external.root_metaclass,
    );
    let mut classes = vec![external.root_class, game];
    classes.extend(&layout.class_list);
    let registration = Registry::register(
        &classes,
        &layout.selector_refs.values().copied().collect::<Vec<_>>(),
        |address, length| {
            let mut bytes = vec![0; length];
            cpu.read_guest_into(address, &mut bytes)?;
            Ok(bytes)
        },
        |address, _| {
            if cpu.mapped_permissions(address).is_some_and(|p| p & 4 != 0) {
                Ok(())
            } else {
                Err("non-executable".into())
            }
        },
    )
    .unwrap();
    // Apply the runtime's selector-reference fixups, as libobjc would.
    for fixup in &registration.selector_fixups {
        cpu.write_bytes(fixup.slot, &fixup.canonical.to_le_bytes());
    }
    let registry = Rc::new(registration.registry);
    let initialization = Rc::new(RefCell::new(Initialization::new(registry.classes().cloned()).unwrap()));
    let heap = GuestObjectHeap::map(&mut cpu, 0x80000, 65536).unwrap();
    let runtime = Rc::new(RefCell::new(
        ObjectRuntime::new(registry.clone(), initialization.clone(), lifetime.clone(), heap, external.root_class).unwrap(),
    ));
    let mut allowed = bridge.instruction_ranges();
    allowed.push(root.code_range());
    allowed.push((layout.text.0, layout.text.0 + layout.text.1));
    allowed.push((GUEST_CODE, GUEST_CODE + 4096));
    let executable = Rc::new(move |address: u64, length: usize| {
        if allowed
            .iter()
            .any(|&(start, end)| address >= start && address.checked_add(length as u64).is_some_and(|e| e <= end))
        {
            Ok(())
        } else {
            Err("test: outside owned executable ranges".into())
        }
    });
    let execution = objc_execution_services::install(
        &mut cpu,
        &mut bridge,
        runtime.clone(),
        Selectors::from_registry(&registry).unwrap(),
        executable,
        0x90000,
    )
    .unwrap();
    let service = |name: &str| execution.iter().find(|(n, _)| *n == name).unwrap().1.guest_address();
    root.link_root_services(&mut cpu, service("_class_createInstance"), service("_object_dispose")).unwrap();
    for class in registry.classes() {
        lifetime.borrow_mut().register_immortal(class.address).unwrap();
    }
    for (_, object) in &layout.static_objects {
        lifetime.borrow_mut().register_immortal(*object).unwrap();
    }
    kit.borrow_mut()
        .link(
            &mut cpu,
            Links {
                msg_send: service("_objc_msgSend"),
                msg_send_super2: service("_objc_msgSendSuper2"),
                retain: arc.iter().find(|(n, _)| *n == "_objc_retain").unwrap().1.guest_address(),
                release: service("_objc_release"),
                alloc_init: service("_objc_alloc_init"),
            },
        )
        .unwrap();
    let pool = arc.iter().find(|(n, _)| *n == "_objc_autoreleasePoolPush").unwrap().1.guest_address();
    let msg = service("_objc_msgSend");
    let sel = |name: &str| registry.selector_named(name).unwrap_or_else(|| panic!("selector {name}"));
    let _ = pool;
    fn invoke(bridge: &mut GuestBridge, cpu: &mut A64Cpu, entry: u64, integers: Vec<u64>) -> super::super::bridge::ReturnValues {
        bridge
            .call(cpu, &GuestCall { entry, integers, ..Default::default() }, 1_000_000)
            .unwrap()
    }
    macro_rules! send {
        ($integers:expr) => {
            invoke(&mut bridge, &mut cpu, msg, $integers)
        };
    }

    // Singletons and screen metrics.
    let screen_class = layout.class("UIScreen").unwrap();
    let screen = send!(vec![screen_class, sel("mainScreen")]).integers[0];
    assert_eq!(screen, layout.static_object("UIScreen").unwrap());
    let bounds = send!(vec![screen, sel("bounds")]);
    let rect: Vec<f64> = bounds.vectors.iter().map(|v| f64::from_bits(v[0])).collect();
    assert_eq!(rect, vec![0.0, 0.0, 667.0, 375.0]);
    let scale = send!(vec![screen, sel("scale")]);
    assert_eq!(f64::from_bits(scale.vectors[0][0]), 2.0);
    let app_class = layout.class("UIApplication").unwrap();
    let app = send!(vec![app_class, sel("sharedApplication")]).integers[0];
    assert_eq!(app, layout.static_object("UIApplication").unwrap());
    send!(vec![app, sel("setIdleTimerDisabled:"), 1]);
    assert_eq!(send!(vec![app, sel("isIdleTimerDisabled")]).integers[0], 1);
    let device = send!(vec![layout.class("UIDevice").unwrap(), sel("currentDevice")]).integers[0];
    assert_eq!(send!(vec![device, sel("userInterfaceIdiom")]).integers[0], 0);

    // Guest subclass: alloc/init via owned runtime; -view runs the default
    // -loadView (genuine guest objc_alloc_init of UIView) and the GUEST
    // -viewDidLoad override, then re-dispatches -view.
    let controller = bridge
        .call(&mut cpu, &GuestCall { entry: service("_objc_alloc_init"), integers: vec![game], ..Default::default() }, 200_000)
        .unwrap()
        .integers[0];
    assert_ne!(controller, 0);
    assert_eq!(cpu.read_u64(controller), Some(game));
    assert_eq!(cpu.read_u64(GUEST_MARKER), Some(0));
    let view = bridge
        .call(&mut cpu, &GuestCall { entry: msg, integers: vec![controller, sel("view")], ..Default::default() }, 400_000)
        .unwrap()
        .integers[0];
    assert_ne!(view, 0);
    assert_eq!(cpu.read_u64(view), layout.class("UIView"));
    assert_eq!(cpu.read_u64(GUEST_MARKER), Some(1), "guest -viewDidLoad override must run");
    let again = bridge
        .call(&mut cpu, &GuestCall { entry: msg, integers: vec![controller, sel("view")], ..Default::default() }, 200_000)
        .unwrap()
        .integers[0];
    assert_eq!(again, view);
    let frame = bridge
        .call(&mut cpu, &GuestCall { entry: msg, integers: vec![view, sel("frame")], ..Default::default() }, 200_000)
        .unwrap();
    assert_eq!(f64::from_bits(frame.vectors[2][0]), 667.0);

    // Window: init uses screen bounds; root view controller is retained.
    let window = bridge
        .call(&mut cpu, &GuestCall { entry: service("_objc_alloc_init"), integers: vec![layout.class("UIWindow").unwrap()], ..Default::default() }, 200_000)
        .unwrap()
        .integers[0];
    let call = |bridge: &mut GuestBridge, cpu: &mut A64Cpu, integers: Vec<u64>| {
        bridge.call(cpu, &GuestCall { entry: msg, integers, ..Default::default() }, 400_000).unwrap()
    };
    call(&mut bridge, &mut cpu, vec![window, sel("setRootViewController:"), controller]);
    call(&mut bridge, &mut cpu, vec![window, sel("makeKeyAndVisible")]);
    assert_eq!(call(&mut bridge, &mut cpu, vec![app, sel("keyWindow")]).integers[0], window);
    assert_eq!(call(&mut bridge, &mut cpu, vec![window, sel("rootViewController")]).integers[0], controller);
    let set_frame = GuestCall {
        entry: msg,
        integers: vec![view, sel("setFrame:")],
        vectors: [1.0f64, 2.0, 30.0, 40.0].iter().map(|v| [v.to_bits(), 0]).collect(),
        ..Default::default()
    };
    bridge.call(&mut cpu, &set_frame, 200_000).unwrap();
    let center = call(&mut bridge, &mut cpu, vec![view, sel("center")]);
    assert_eq!((f64::from_bits(center.vectors[0][0]), f64::from_bits(center.vectors[1][0])), (16.0, 22.0));
    call(&mut bridge, &mut cpu, vec![window, sel("addSubview:"), view]);
    assert_eq!(call(&mut bridge, &mut cpu, vec![view, sel("window")]).integers[0], window);
    {
        let k = kit.borrow();
        assert_eq!(k.model.view_count(), 2);
        assert_eq!(k.model.controller_count(), 1);
        assert_eq!(k.model.view(view).unwrap().frame, Rect::new(1.0, 2.0, 30.0, 40.0));
    }
    // The caller drops its own (+1 alloc) reference to the controller; the
    // -view result was +0. The window's -dealloc thunk then releases its
    // subview and root controller, whose thunk releases the last view owner.
    let release = service("_objc_release");
    bridge.call(&mut cpu, &GuestCall { entry: release, integers: vec![controller], ..Default::default() }, 200_000).unwrap();
    assert!(lifetime.borrow().contains_identity(controller));
    bridge.call(&mut cpu, &GuestCall { entry: release, integers: vec![window], ..Default::default() }, 1_000_000).unwrap();
    for object in [window, controller, view] {
        assert!(!lifetime.borrow().contains_identity(object), "{object:#x} must be disposed");
    }
    let k = kit.borrow();
    assert_eq!(k.model.view_count(), 0);
    assert_eq!(k.model.controller_count(), 0);
    assert_eq!(k.model.application.key_window, 0);
    assert!(k.first_use.iter().any(|s| s == "-[UIViewController loadView]"));
}

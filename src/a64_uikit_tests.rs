/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Desktop tests for the ARM64 UIKit layer. They execute real guest ARM64
//! instructions (owned NSObject lifecycle, index trampolines, the owned
//! thunks, UIApplicationMain's pump and guest subclass overrides) through the
//! emulator-owned Objective-C services. No Apple binary or device is used.
use super::super::{
    bridge::{GuestBridge, GuestCall, ReturnValues},
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
    app::{self, LaunchPlan},
    asm::Asm,
    image::{self, ClassDef, External, FunctionDef, ImageSpec, MethodDef, StaticObject},
    mgl::RecordingGles,
    state::{Device, Model, Orientation, Rect, Screen, Size},
    Links, UiKit,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

const IMAGE_BASE: u64 = 0x20_0000;
const GUEST_CODE: u64 = 0x40000;
const MARK: u64 = 0x41000;
const GUEST_META: u64 = 0x50000;

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

fn read_bytes(cpu: &A64Cpu) -> impl FnMut(u64, usize) -> Result<Vec<u8>, String> + '_ {
    move |address, length| {
        let mut bytes = vec![0; length];
        cpu.read_guest_into(address, &mut bytes)?;
        Ok(bytes)
    }
}
fn rx_only(cpu: &A64Cpu) -> impl FnMut(u64, usize) -> Result<(), String> + '_ {
    move |address, _| {
        if cpu.mapped_permissions(address).is_some_and(|p| p & 4 != 0) {
            Ok(())
        } else {
            Err("non-executable".into())
        }
    }
}

fn nop_generator(_: &image::Symbols<'_>) -> Asm {
    let mut a = Asm::default();
    a.ret();
    a
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
            provider: super::INSTALL_NAME,
            parent: None,
            instance_size: 8,
            instance_methods: vec![MethodDef { selector: "nextResponder", types: "@16@0:8", index: 1 }],
            class_methods: vec![],
            dealloc_cleanup: false,
        },
        ClassDef {
            name: "CALayer",
            provider: super::QUARTZCORE,
            parent: None,
            instance_size: 8,
            instance_methods: vec![],
            class_methods: vec![],
            dealloc_cleanup: false,
        },
        ClassDef {
            name: "UIView",
            provider: super::INSTALL_NAME,
            parent: Some("UIResponder"),
            instance_size: 8,
            instance_methods: vec![MethodDef { selector: "frame", types: "{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8", index: 2 }],
            class_methods: vec![MethodDef { selector: "layerClass", types: "#16@0:8", index: 3 }],
            dealloc_cleanup: true,
        },
    ];
    let functions = [FunctionDef { symbol: "_UIApplicationMain", provider: Some(super::INSTALL_NAME), generate: nop_generator }];
    let built = image::build(&ImageSpec {
        base: IMAGE_BASE,
        install_name: super::INSTALL_NAME,
        classes: &defs,
        statics: &[StaticObject { class: "UIView", size: 16 }],
        functions: &functions,
        send_selectors: &["loadView"],
        got: &["objc_msgSend"],
        entry_count: 4,
        cleanup_entry: Some(0),
        dispatcher: 0x10000,
        scratch_bytes: 256,
        external,
    })
    .unwrap();
    built.map(&mut cpu).unwrap();
    let layout = &built.layout;
    // The dyld ObjC notification path accepts it as an ordinary ObjC image.
    let notified = ObjcImage::read(&cpu, layout.header, super::INSTALL_NAME.into(), false)
        .unwrap()
        .expect("synthetic image must carry __objc_imageinfo");
    assert!(notified.readonly_ranges().is_empty());
    let mut classes = vec![external.root_class];
    classes.extend(&layout.class_list);
    let selrefs: Vec<u64> = layout.selector_refs.values().copied().collect();
    let registry = Registry::register(&classes, &selrefs, read_bytes(&cpu), rx_only(&cpu)).unwrap().registry;
    let view = layout.class("UIView").unwrap();
    assert_eq!(registry.lookup_class("UIView"), Some(view));
    assert_eq!(cpu.read_u64(view + 8), layout.class("UIResponder"));
    assert_eq!(cpu.read_u64(layout.metaclass("UIView").unwrap()), Some(external.root_metaclass));
    assert!(registry.selector_named("dealloc").is_some());
    // Every IMP is its own index trampoline: movz x17,#index; b hub.
    let frame_imp = layout.entry(2);
    assert_eq!(cpu.read_bytes(frame_imp, 4).unwrap(), (0xd2800000u32 | 2 << 5 | 17).to_le_bytes());
    let exports = layout.exports();
    assert!(exports.contains(&(super::QUARTZCORE.into(), "_OBJC_CLASS_$_CALayer".into(), layout.class("CALayer").unwrap())));
    assert!(exports.iter().any(|(p, s, _)| p == super::INSTALL_NAME && s == "_UIApplicationMain"));
    assert_eq!(cpu.read_u64(layout.static_object("UIView").unwrap()), Some(view));
    assert_eq!(cpu.mapped_permissions(layout.text.0), Some(5));
    assert_eq!(cpu.mapped_permissions(layout.data.0), Some(3));
    assert!(layout.got.contains_key("objc_msgSendSuper2"));
    assert!(layout.contains_code(layout.thunks[0].1, 64));
}

#[test]
fn image_builder_rejects_bad_parents_selectors_and_placement() {
    let external = External { root_class: 0x1000, root_metaclass: 0x1028, empty_cache: 0 };
    let def = |name: &'static str, parent: Option<&'static str>, selector: &'static str, index: u16| ClassDef {
        name,
        provider: super::INSTALL_NAME,
        parent,
        instance_size: 8,
        instance_methods: vec![MethodDef { selector, types: "v16@0:8", index }],
        class_methods: vec![],
        dealloc_cleanup: false,
    };
    let build = |defs: &[ClassDef], base: u64| {
        image::build(&ImageSpec {
            base,
            install_name: super::INSTALL_NAME,
            classes: defs,
            statics: &[],
            functions: &[],
            send_selectors: &[],
            got: &[],
            entry_count: 4,
            cleanup_entry: None,
            dispatcher: 0x10000,
            scratch_bytes: 0,
            external,
        })
    };
    assert!(build(&[def("UIView", Some("UIResponder"), "frame", 1)], IMAGE_BASE).is_err());
    assert!(build(&[def("UIView", None, "bad selector", 1)], IMAGE_BASE).is_err());
    assert!(build(&[def("UIView", None, "a", 1), def("UIView", None, "b", 2)], IMAGE_BASE).is_err());
    assert!(build(&[def("UIView", None, "frame", 4)], IMAGE_BASE).is_err(), "index out of range");
    assert!(build(&[def("UIView", None, "frame", 1)], IMAGE_BASE + 0x1000).is_err());
    assert!(build(&[def("UIView", None, "frame", 1)], image::ISA_ADDRESS_LIMIT).is_err());
    let mut cleanup = def("UIView", None, "frame", 1);
    cleanup.dealloc_cleanup = true;
    assert!(build(&[cleanup], IMAGE_BASE).is_err(), "cleanup thunk needs a cleanup entry");
    assert!(build(&[def("UIView", None, "frame", 1)], IMAGE_BASE).is_ok());
}

/// A guest class written into fixture memory, compiled-SDK style.
struct GuestClass {
    name: &'static str,
    parent: &'static str,
    /// (selector, types, code)
    instance: Vec<(&'static str, &'static str, Asm)>,
    class: Vec<(&'static str, &'static str, Asm)>,
}

struct Fixture {
    cpu: A64Cpu,
    bridge: GuestBridge,
    kit: Rc<RefCell<UiKit>>,
    layout: image::Layout,
    registry: Rc<Registry>,
    services: BTreeMap<&'static str, u64>,
    lifetime: Rc<RefCell<Lifetime>>,
    guests: BTreeMap<&'static str, u64>,
    gles: Rc<RefCell<Vec<String>>>,
}

impl Fixture {
    fn new(launch: Option<LaunchPlan>, guests: Vec<GuestClass>) -> Self {
        let mut cpu = A64Cpu::new_sparse();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        let lifetime = Rc::new(RefCell::new(Lifetime::default()));
        let lifetime_services =
            super::super::objc_lifetime_services::install_without_release(&mut bridge, &mut cpu, lifetime.clone()).unwrap();
        let mut root = root_namespace(&mut cpu);
        let external = External {
            root_class: root.class_address("NSObject").unwrap(),
            root_metaclass: root.metaclass_address("NSObject").unwrap(),
            empty_cache: 0,
        };
        let gles = Rc::new(RefCell::new(Vec::new()));
        let installed = super::install(
            &mut cpu,
            &mut bridge,
            IMAGE_BASE,
            external,
            model(),
            Box::new(RecordingGles::new(gles.clone())),
            launch,
        )
        .unwrap();
        let layout = installed.layout.clone();
        // Guest classes (code page + metadata), parents from the image.
        cpu.map_zeroed(GUEST_CODE, 0x1000, 5).unwrap();
        cpu.map_zeroed(MARK, 0x1000, 3).unwrap();
        cpu.map_zeroed(GUEST_META, 0x4000, 3).unwrap();
        let mut code_cursor = GUEST_CODE;
        let mut meta_cursor = GUEST_META;
        let mut guest_addresses = BTreeMap::new();
        for guest in guests {
            let mut alloc = |size: u64| {
                let at = meta_cursor;
                meta_cursor += (size + 15) & !15;
                at
            };
            let (class, meta, ro, meta_ro) = (alloc(40), alloc(40), alloc(72), alloc(72));
            let name = alloc(guest.name.len() as u64 + 1);
            cpu.write_bytes(name, guest.name.as_bytes());
            let mut list = |methods: &[(&str, &str, Asm)], cpu: &mut A64Cpu| -> u64 {
                if methods.is_empty() {
                    return 0;
                }
                let list = alloc(8 + 24 * methods.len() as u64);
                cpu.write_bytes(list, &24u32.to_le_bytes());
                cpu.write_bytes(list + 4, &(methods.len() as u32).to_le_bytes());
                for (i, (sel, types, code)) in methods.iter().enumerate() {
                    let s = alloc(sel.len() as u64 + 1);
                    cpu.write_bytes(s, sel.as_bytes());
                    let t = alloc(types.len() as u64 + 1);
                    cpu.write_bytes(t, types.as_bytes());
                    let bytes = code.finish();
                    let imp = code_cursor;
                    cpu.try_write_bytes(imp, &bytes).unwrap();
                    code_cursor += (bytes.len() as u64 + 15) & !15;
                    let at = list + 8 + 24 * i as u64;
                    for (o, v) in [(0, s), (8, t), (16, imp)] {
                        cpu.write_bytes(at + o, &v.to_le_bytes());
                    }
                }
                list
            };
            let instance_list = list(&guest.instance, &mut cpu);
            let class_list = list(&guest.class, &mut cpu);
            let (parent, parent_meta) = match guest_addresses.get(guest.parent) {
                Some(&(c, m)) => (c, m),
                None => (layout.class(guest.parent).unwrap(), layout.metaclass(guest.parent).unwrap()),
            };
            for (a, v) in [
                (class, meta),
                (class + 8, parent),
                (class + 32, ro),
                (meta, external.root_metaclass),
                (meta + 8, parent_meta),
                (meta + 32, meta_ro),
            ] {
                cpu.write_bytes(a, &v.to_le_bytes());
            }
            // Compiled against the real SDK: instance_start larger than our
            // eight-byte classes, so no ivar sliding is ever needed.
            for (ro, flags, start, size, methods) in
                [(ro, 0u32, 0x3f0u32, 0x400u32, instance_list), (meta_ro, 1, 40, 40, class_list)]
            {
                cpu.write_bytes(ro, &flags.to_le_bytes());
                cpu.write_bytes(ro + 4, &start.to_le_bytes());
                cpu.write_bytes(ro + 8, &size.to_le_bytes());
                cpu.write_bytes(ro + 24, &name.to_le_bytes());
                cpu.write_bytes(ro + 32, &methods.to_le_bytes());
            }
            guest_addresses.insert(guest.name, (class, meta));
        }
        let mut classes = vec![external.root_class];
        classes.extend(guest_addresses.values().map(|&(c, _)| c));
        classes.extend(&layout.class_list);
        let selrefs: Vec<u64> = layout.selector_refs.values().copied().collect();
        let registration = Registry::register(&classes, &selrefs, read_bytes(&cpu), rx_only(&cpu)).unwrap();
        // Apply the runtime's selector-reference fixups, as libobjc would.
        for fixup in &registration.selector_fixups {
            cpu.write_bytes(fixup.slot, &fixup.canonical.to_le_bytes());
        }
        let registry = Rc::new(registration.registry);
        let initialization = Rc::new(RefCell::new(Initialization::new(registry.classes().cloned()).unwrap()));
        let heap = GuestObjectHeap::map(&mut cpu, 0x80000, 0x10000).unwrap();
        let runtime = Rc::new(RefCell::new(
            ObjectRuntime::new(registry.clone(), initialization, lifetime.clone(), heap, external.root_class).unwrap(),
        ));
        let mut allowed = bridge.instruction_ranges();
        allowed.push(root.code_range());
        allowed.push((layout.text.0, layout.text.0 + layout.text.1));
        allowed.push((GUEST_CODE, GUEST_CODE + 0x1000));
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
        let mut services: BTreeMap<&'static str, u64> =
            execution.iter().map(|(n, id)| (*n, id.guest_address())).collect();
        let release = services["_objc_release"];
        let arc = super::super::objc_arc_services::install(&mut bridge, &mut cpu, runtime.clone(), release).unwrap();
        services.extend(arc.iter().map(|(n, id)| (*n, id.guest_address())));
        services.extend(lifetime_services.iter().map(|(n, id)| (*n, id.guest_address())));
        root.link_root_services(&mut cpu, services["_class_createInstance"], services["_object_dispose"]).unwrap();
        for class in registry.classes() {
            lifetime.borrow_mut().register_immortal(class.address).unwrap();
        }
        for (_, object) in &layout.static_objects {
            lifetime.borrow_mut().register_immortal(*object).unwrap();
        }
        installed
            .state
            .borrow_mut()
            .link(
                &mut cpu,
                Links {
                    msg_send: services["_objc_msgSend"],
                    msg_send_super2: services["_objc_msgSendSuper2"],
                    retain: services["_objc_retain"],
                    release: services["_objc_release"],
                    alloc_init: services["_objc_alloc_init"],
                    get_class: services["_objc_getClass"],
                },
            )
            .unwrap();
        Self {
            cpu,
            bridge,
            kit: installed.state,
            layout,
            registry,
            services,
            lifetime,
            guests: guest_addresses.into_iter().map(|(n, (c, _))| (n, c)).collect(),
            gles,
        }
    }
    fn call(&mut self, entry: u64, integers: Vec<u64>, vectors: Vec<f64>) -> ReturnValues {
        self.bridge
            .call(
                &mut self.cpu,
                &GuestCall { entry, integers, vectors: vectors.iter().map(|v| [v.to_bits(), 0]).collect(), ..Default::default() },
                1_000_000,
            )
            .unwrap()
    }
    fn sel(&self, name: &str) -> u64 {
        self.registry.selector_named(name).unwrap_or_else(|| panic!("selector {name}"))
    }
    fn send(&mut self, receiver: u64, selector: &str, args: &[u64]) -> ReturnValues {
        let mut integers = vec![receiver, self.sel(selector)];
        integers.extend_from_slice(args);
        self.call(self.services["_objc_msgSend"], integers, vec![])
    }
    fn send_f(&mut self, receiver: u64, selector: &str, args: &[u64], floats: &[f64]) -> ReturnValues {
        let mut integers = vec![receiver, self.sel(selector)];
        integers.extend_from_slice(args);
        self.call(self.services["_objc_msgSend"], integers, floats.to_vec())
    }
    fn alloc_init(&mut self, class: u64) -> u64 {
        self.call(self.services["_objc_alloc_init"], vec![class], vec![]).integers[0]
    }
    fn class(&self, name: &str) -> u64 {
        self.guests.get(name).copied().or_else(|| self.layout.class(name)).unwrap()
    }
    fn mark(&self, slot: u64) -> u64 {
        self.cpu.read_u64(MARK + 8 * slot).unwrap()
    }
}

fn f64s(values: &ReturnValues, count: usize) -> Vec<f64> {
    values.vectors[..count].iter().map(|v| f64::from_bits(v[0])).collect()
}

/// Guest method: counter at MARK += 1, stamp it into MARK[slot]; optionally
/// store x2 into MARK[store_x2]; return YES.
fn stamp(slot: u32, store_x2: Option<u32>) -> Asm {
    let mut a = Asm::default();
    a.ldr_literal(9, MARK).ldr(10, 9, 0).add_imm(10, 10, 1).str(10, 9, 0).str(10, 9, 8 * slot);
    if let Some(at) = store_x2 {
        a.str(2, 9, 8 * at);
    }
    a.movz(0, 1).ret();
    a
}
fn returns(value: u64) -> Asm {
    let mut a = Asm::default();
    a.ldr_literal(0, value).ret();
    a
}

#[test]
fn guest_subclass_chains_into_owned_uikit_and_disposal_clears_host_state() {
    let mut f = Fixture::new(
        None,
        vec![GuestClass {
            name: "GameViewController",
            parent: "UIViewController",
            instance: vec![("viewDidLoad", "v16@0:8", stamp(1, None))],
            class: vec![],
        }],
    );
    // Singletons and screen metrics.
    let screen = f.send(f.class("UIScreen"), "mainScreen", &[]).integers[0];
    assert_eq!(screen, f.layout.static_object("UIScreen").unwrap());
    assert_eq!(f64s(&f.send(screen, "bounds", &[]), 4), vec![0.0, 0.0, 667.0, 375.0]);
    assert_eq!(f64s(&f.send(screen, "scale", &[]), 1), vec![2.0]);
    let app = f.send(f.class("UIApplication"), "sharedApplication", &[]).integers[0];
    assert_eq!(app, f.layout.static_object("UIApplication").unwrap());
    f.send(app, "setIdleTimerDisabled:", &[1]);
    assert_eq!(f.send(app, "isIdleTimerDisabled", &[]).integers[0], 1);
    let device = f.send(f.class("UIDevice"), "currentDevice", &[]).integers[0];
    assert_eq!(f.send(device, "userInterfaceIdiom", &[]).integers[0], 0);

    // Guest subclass: -view runs the default -loadView (guest
    // objc_alloc_init of UIView, whose init creates a CALayer through guest
    // +layerClass code) and the GUEST -viewDidLoad override.
    let controller = f.alloc_init(f.class("GameViewController"));
    assert_eq!(f.cpu.read_u64(controller), Some(f.class("GameViewController")));
    let view = f.send(controller, "view", &[]).integers[0];
    assert_ne!(view, 0);
    assert_eq!(f.cpu.read_u64(view), Some(f.class("UIView")));
    assert_eq!(f.mark(1), 1, "guest -viewDidLoad override must run");
    assert_eq!(f.send(controller, "view", &[]).integers[0], view);
    assert_eq!(f64s(&f.send(view, "frame", &[]), 4), vec![0.0, 0.0, 667.0, 375.0]);
    let layer = f.send(view, "layer", &[]).integers[0];
    assert_ne!(layer, 0);
    assert_eq!(f.cpu.read_u64(layer), Some(f.class("CALayer")));
    assert_eq!(f64s(&f.send(layer, "contentsScale", &[]), 1), vec![2.0]);

    // Window: init uses screen bounds; root view controller is retained.
    let window = f.alloc_init(f.class("UIWindow"));
    f.send(window, "setRootViewController:", &[controller]);
    f.send(window, "makeKeyAndVisible", &[]);
    assert_eq!(f.send(app, "keyWindow", &[]).integers[0], window);
    assert_eq!(f.send(window, "rootViewController", &[]).integers[0], controller);
    f.send_f(view, "setFrame:", &[], &[1.0, 2.0, 30.0, 40.0]);
    assert_eq!(f64s(&f.send(view, "center", &[]), 2), vec![16.0, 22.0]);
    assert_eq!(f64s(&f.send(layer, "position", &[]), 2), vec![16.0, 22.0]);
    f.send(window, "addSubview:", &[view]);
    assert_eq!(f.send(view, "window", &[]).integers[0], window);
    {
        let k = f.kit.borrow();
        assert_eq!(k.model.view_count(), 2);
        assert_eq!(k.model.layer_count(), 2);
        assert_eq!(k.model.view(view).unwrap().frame, Rect::new(1.0, 2.0, 30.0, 40.0));
    }
    // Drop the caller's +1 controller; the window's -dealloc thunk releases
    // its subview, root controller and layer; the controller's releases its
    // view; each view's releases its layer.
    let release = f.services["_objc_release"];
    f.call(release, vec![controller], vec![]);
    assert!(f.lifetime.borrow().contains_identity(controller));
    f.call(release, vec![window], vec![]);
    for object in [window, controller, view, layer] {
        assert!(!f.lifetime.borrow().contains_identity(object), "{object:#x} must be disposed");
    }
    let k = f.kit.borrow();
    assert_eq!((k.model.view_count(), k.model.controller_count(), k.model.layer_count()), (0, 0, 0));
    assert_eq!(k.model.application.key_window, 0);
    assert!(k.first_use.iter().any(|s| s == "-[UIViewController loadView]"));
    assert!(k.first_use.iter().any(|s| s == "+[UIView layerClass]"));
}

#[test]
fn guest_layer_class_override_and_mglkit_stand_in_draw_and_present() {
    let mut f = Fixture::new(
        None,
        vec![
            GuestClass {
                name: "MetalBackedView",
                parent: "UIView",
                instance: vec![],
                // Filled in below once the MGLLayer address is known.
                class: vec![("layerClass", "#16@0:8", returns(0))],
            },
            GuestClass {
                name: "CoronaLikeView",
                parent: "MGLKView",
                instance: vec![("drawRect:", "v48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", stamp(2, None))],
                class: vec![],
            },
        ],
    );
    // Patch the guest +layerClass literal to return MGLLayer.
    let mgl_layer = f.class("MGLLayer");
    let meta = f.cpu.read_u64(f.class("MetalBackedView")).unwrap();
    let ro = f.cpu.read_u64(meta + 32).unwrap();
    let list = f.cpu.read_u64(ro + 32).unwrap();
    let imp = f.cpu.read_u64(list + 24).unwrap();
    f.cpu.try_write_bytes(imp + 8, &mgl_layer.to_le_bytes()).unwrap();
    let custom = f.alloc_init(f.class("MetalBackedView"));
    let layer = f.send(custom, "layer", &[]).integers[0];
    assert_eq!(f.cpu.read_u64(layer), Some(mgl_layer), "guest +layerClass override honoured");

    // MGLContext + MGLKView subclass (Corona's CoronaView shape).
    let context = f.call(f.services["_objc_alloc"], vec![f.class("MGLContext")], vec![]).integers[0];
    assert_eq!(f.send(context, "initWithAPI:", &[2]).integers[0], context);
    let view = f.call(f.services["_objc_alloc"], vec![f.class("CoronaLikeView")], vec![]).integers[0];
    let view = f.send_f(view, "initWithFrame:context:", &[context], &[0.0, 0.0, 667.0, 375.0]).integers[0];
    assert_eq!(f.send(view, "context", &[]).integers[0], context);
    let gl_layer = f.send(view, "glLayer", &[]).integers[0];
    assert_eq!(f.cpu.read_u64(gl_layer), Some(mgl_layer));
    f.send(view, "setDrawableDepthFormat:", &[24]);
    f.send_f(view, "setContentScaleFactor:", &[], &[3.0]);
    assert_eq!(f.send(view, "drawableWidth", &[]).integers[0], 2001);
    assert_eq!(f.send(view, "drawableHeight", &[]).integers[0], 1125);
    f.send(view, "display", &[]);
    assert_eq!(f.mark(2), 1, "guest -drawRect: override ran from -display");
    let log = f.gles.borrow().clone();
    assert_eq!(log, vec!["create 1 api=2", "current 1 2001x1125", "present 1 2001x1125"]);
    assert!(f.send(context, "API", &[]).integers[0] == 2);
}

#[test]
fn ui_application_main_runs_nib_launch_sequence_and_display_link_frames() {
    let plan = app::parse_nib(&app::tests::coromon_like_nib("TestDelegate")).unwrap();
    let mut f = Fixture::new(
        Some(plan),
        vec![
            GuestClass {
                name: "TestDelegate",
                parent: "UIResponder",
                instance: vec![
                    ("respondsToSelector:", "B24@0:8:16", returns(1)),
                    ("setWindow:", "v24@0:8@16", stamp(1, Some(20))),
                    ("application:willFinishLaunchingWithOptions:", "B32@0:8@16@24", stamp(2, Some(21))),
                    ("application:didFinishLaunchingWithOptions:", "B32@0:8@16@24", stamp(3, None)),
                    ("applicationDidBecomeActive:", "v24@0:8@16", stamp(4, None)),
                ],
                class: vec![],
            },
            GuestClass {
                name: "CoronaLikeView",
                parent: "MGLKView",
                instance: vec![("drawRect:", "v48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", stamp(5, None))],
                class: vec![],
            },
        ],
    );
    let main = f.layout.function("_UIApplicationMain").unwrap();
    // UIApplicationMain(argc, argv, nil, nil): no frame source yet, so the
    // pump stops after the launch sequence (an explicit, logged idle stop).
    assert_eq!(f.call(main, vec![1, 0, 0, 0], vec![]).integers[0], 0);
    let (delegate, window) = {
        let k = f.kit.borrow();
        (k.launcher.delegate(), k.launcher.window())
    };
    assert_eq!(f.cpu.read_u64(delegate), Some(f.class("TestDelegate")));
    assert_eq!(f.cpu.read_u64(window), Some(f.class("UIWindow")));
    // Order: setWindow:, willFinish, didFinish, didBecomeActive.
    assert_eq!((f.mark(1), f.mark(2), f.mark(3), f.mark(4)), (1, 2, 3, 4));
    assert_eq!(f.mark(20), window, "nib window outlet connected");
    assert_eq!(f.mark(21), f.layout.static_object("UIApplication").unwrap());
    let app = f.send(f.class("UIApplication"), "sharedApplication", &[]).integers[0];
    assert_eq!(f.send(app, "delegate", &[]).integers[0], delegate);
    assert_eq!(f.send(app, "keyWindow", &[]).integers[0], window);
    assert_eq!(f.send(app, "applicationState", &[]).integers[0], 0, "active");
    assert!(f.kit.borrow().launcher.log.iter().any(|l| l.contains("no frame source")));

    // What Corona's didFinishLaunching sets up: MGLKViewController + view.
    let controller = f.alloc_init(f.class("MGLKViewController"));
    let context = f.call(f.services["_objc_alloc"], vec![f.class("MGLContext")], vec![]).integers[0];
    f.send(context, "initWithAPI:", &[2]);
    let view = f.call(f.services["_objc_alloc"], vec![f.class("CoronaLikeView")], vec![]).integers[0];
    f.send_f(view, "initWithFrame:context:", &[context], &[0.0, 0.0, 667.0, 375.0]);
    f.send(controller, "setView:", &[view]);
    assert_eq!(f.send(controller, "glView", &[]).integers[0], view);
    f.send(window, "setRootViewController:", &[controller]);
    f.send(controller, "setPreferredFramesPerSecond:", &[60]);
    assert_eq!(f.send(controller, "isPaused", &[]).integers[0], 0);
    // A second UIApplicationMain entry continues the run loop for 3 frames.
    f.kit.borrow_mut().launcher.continue_frames(3);
    assert_eq!(f.call(main, vec![1, 0, 0, 0], vec![]).integers[0], 0);
    assert_eq!(f.mark(5), 3 + 4, "guest -drawRect: ran once per frame");
    assert_eq!(f.send(controller, "framesDisplayed", &[]).integers[0], 3);
    let presents = f.gles.borrow().iter().filter(|l| l.starts_with("present 1 1334x750")).count();
    assert_eq!(presents, 3);
    let since = f64s(&f.send(controller, "timeSinceLastUpdate", &[]), 1)[0];
    assert!((since - 1.0 / 60.0).abs() < 1e-9);
}

#[test]
fn table_budget_fits_single_service_and_entry_limit() {
    let (classes, methods) = super::table_counts();
    assert!(classes <= image::MAX_CLASSES);
    assert!(methods + 4 <= image::MAX_ENTRIES);
}

#[test]
fn bind_routes_only_owned_providers_and_reports_gaps() {
    let f = Fixture::new(None, vec![]);
    let routes = super::bind::Routes::from_layout(&f.layout);
    assert_eq!(routes.route(super::INSTALL_NAME, "_OBJC_CLASS_$_UIView"), f.layout.class("UIView"));
    assert_eq!(routes.route(super::METALANGLE, "_OBJC_CLASS_$_MGLKView"), f.layout.class("MGLKView"));
    assert_eq!(routes.route(super::INSTALL_NAME, "_UIApplicationMain"), f.layout.function("_UIApplicationMain"));
    // Provider identity matters: MGLKView is not a UIKit export.
    assert_eq!(routes.route(super::INSTALL_NAME, "_OBJC_CLASS_$_MGLKView"), None);
    let coverage = routes.coverage([
        (super::INSTALL_NAME, "_OBJC_CLASS_$_UIWindow", false),
        (super::QUARTZCORE, "_OBJC_CLASS_$_CADisplayLink", false),
        (super::INSTALL_NAME, "_UIAccessibilityIsVoiceOverRunning", true),
        ("/System/Library/Frameworks/Foundation.framework/Foundation", "_OBJC_CLASS_$_NSString", false),
    ]);
    assert_eq!(coverage.routed.len(), 1);
    assert_eq!(coverage.missing_required, vec![(super::QUARTZCORE.to_string(), "_OBJC_CLASS_$_CADisplayLink".to_string())]);
    assert_eq!(coverage.missing_weak.len(), 1);
}

/// Coromon's real import list (classic dyld-info binds) against the owned
/// exports: PLAYCOVER_COROMON_BINARY=/path/to/Payload/Coromon.app/Coromon.
/// Prints the remaining UIKit/QuartzCore/MetalANGLE gaps (milestone 2 list).
#[test]
#[ignore]
fn actual_coromon_imports_against_owned_exports() {
    let path = std::env::var("PLAYCOVER_COROMON_BINARY").expect("PLAYCOVER_COROMON_BINARY");
    let file = std::fs::read(path).unwrap();
    let bytes = super::super::thin_arm64_slice(&file).unwrap();
    let metadata = super::super::MachO64::parse_metadata(bytes).unwrap();
    let imports = match metadata.legacy_fixups {
        Some(streams) => super::super::legacy::imports(bytes, streams, &metadata.segments).unwrap(),
        None => super::super::fixups::imports(bytes).unwrap(),
    };
    let named: Vec<(String, String, bool)> = imports
        .iter()
        .filter(|i| i.library_ordinal > 0)
        .map(|i| (metadata.dependencies[i.library_ordinal as usize - 1].name.clone(), i.name.clone(), i.weak))
        .collect();
    let f = Fixture::new(None, vec![]);
    let routes = super::bind::Routes::from_layout(&f.layout);
    let coverage = routes.coverage(named.iter().map(|(p, s, w)| (p.as_str(), s.as_str(), *w)));
    echo!(
        "PLAYCOVER_UIKIT_COVERAGE routed={} missing_required={} missing_weak={}",
        coverage.routed.len(),
        coverage.missing_required.len(),
        coverage.missing_weak.len()
    );
    for (provider, symbol) in &coverage.routed {
        echo!("PLAYCOVER_UIKIT_ROUTED {provider} {symbol}");
    }
    for (provider, symbol) in &coverage.missing_required {
        echo!("PLAYCOVER_UIKIT_MISSING {provider} {symbol}");
    }
    for expected in [
        (super::INSTALL_NAME, "_UIApplicationMain"),
        (super::INSTALL_NAME, "_OBJC_CLASS_$_UIViewController"),
        (super::METALANGLE, "_OBJC_CLASS_$_MGLKViewController"),
        (super::METALANGLE, "_OBJC_CLASS_$_MGLContext"),
    ] {
        assert!(
            coverage.routed.contains(&(expected.0.to_string(), expected.1.to_string())),
            "{expected:?} must be routed"
        );
    }
}

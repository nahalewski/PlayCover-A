/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Actual NSBundle method trampolines for the emulator-owned namespace.
//! Only preloaded real bundle metadata is discoverable. Mapped Unity images
//! stay unloaded until the model receives genuine scheduler load receipts.
use super::{
    bridge::{GuestBridge, ReturnValues, ServiceFrame, ServiceId},
    bundle_metadata::{BundleId, Bundles},
    nsstring_services::Strings,
    objc_execution_services::ObjectRuntime,
    objc_namespace::Namespace,
    A64Cpu,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
struct Configuration {
    runtime: Rc<RefCell<ObjectRuntime>>,
    class: u64,
    strings: Rc<RefCell<Strings>>,
    selectors: BTreeMap<String, u64>,
}
type LoadCoordinator =
    Rc<dyn Fn(&mut super::bridge::ServiceFrame<'_>, BundleId) -> Result<(), String>>;
pub(super) struct BundleServices {
    pub model: Bundles,
    configuration: Option<Configuration>,
    objects: BTreeMap<BundleId, u64>,
    load_coordinator: Option<LoadCoordinator>,
}
pub(super) struct Methods {
    pub main: ServiceId,
    pub with_path: ServiceId,
    pub path: ServiceId,
    pub loaded: ServiceId,
    pub load: ServiceId,
    pub principal: ServiceId,
}
impl Methods {
    pub(super) fn class_methods(&self) -> Vec<super::objc_namespace::MethodSpec<'static>> {
        [
            ("mainBundle", "@16@0:8", self.main),
            ("bundleWithPath:", "@24@0:8@16", self.with_path),
        ]
        .into_iter()
        .map(|(selector, types, id)| super::objc_namespace::MethodSpec {
            selector,
            types,
            implementation: id.guest_address(),
        })
        .collect()
    }
    pub(super) fn instance_methods(&self) -> Vec<super::objc_namespace::MethodSpec<'static>> {
        [
            ("bundlePath", "@16@0:8", self.path),
            ("isLoaded", "B16@0:8", self.loaded),
            ("load", "B16@0:8", self.load),
            ("principalClass", "#16@0:8", self.principal),
        ]
        .into_iter()
        .map(|(selector, types, id)| super::objc_namespace::MethodSpec {
            selector,
            types,
            implementation: id.guest_address(),
        })
        .collect()
    }
}
impl BundleServices {
    pub(super) fn set_load_coordinator(
        &mut self,
        coordinator: LoadCoordinator,
    ) -> Result<(), String> {
        if self.load_coordinator.is_some() {
            return Err("bundle load coordinator already installed".into());
        }
        self.load_coordinator = Some(coordinator);
        Ok(())
    }
    pub(super) fn new(model: Bundles) -> Self {
        Self {
            model,
            load_coordinator: None,
            configuration: None,
            objects: BTreeMap::new(),
        }
    }
    pub(super) fn configure(
        &mut self,
        runtime: Rc<RefCell<ObjectRuntime>>,
        class: u64,
        strings: Rc<RefCell<Strings>>,
        namespace: &Namespace,
    ) -> Result<(), String> {
        if self.configuration.is_some() {
            return Err("NSBundle adapter already configured".into());
        }
        let r = runtime
            .try_borrow()
            .map_err(|_| "reentrant bundle configuration")?;
        if namespace.class_address("NSBundle") != Some(class)
            || r.registry.lookup_class("NSBundle") != Some(class)
            || !r
                .registry
                .classes()
                .any(|c| c.address == class && c.instance_size == 8 && c.flags & 1 == 0)
        {
            return Err(
                "NSBundle requires exact emulator-owned eight-byte registered class identity"
                    .into(),
            );
        }
        let selectors = [
            "mainBundle",
            "bundleWithPath:",
            "bundlePath",
            "isLoaded",
            "load",
            "principalClass",
        ]
        .into_iter()
        .map(|name| {
            r.registry
                .selector_named(name)
                .map(|sel| (name.into(), sel))
                .ok_or_else(|| format!("NSBundle selector absent: {name}"))
        })
        .collect::<Result<_, _>>()?;
        drop(r);
        self.configuration = Some(Configuration {
            runtime,
            class,
            strings,
            selectors,
        });
        Ok(())
    }
    fn configuration(&self) -> Result<&Configuration, String> {
        self.configuration
            .as_ref()
            .ok_or_else(|| "NSBundle emulator namespace not configured".into())
    }
    fn check_method(
        &self,
        frame: &mut ServiceFrame<'_>,
        name: &str,
        class_method: bool,
    ) -> Result<u64, String> {
        let config = self.configuration()?;
        let receiver = frame.integer(0)?;
        if config.selectors.get(name).copied() != Some(frame.integer(1)?) {
            return Err("NSBundle selector is not canonical".into());
        }
        let r = config
            .runtime
            .try_borrow()
            .map_err(|_| "reentrant bundle ownership")?;
        if !r
            .initialization
            .try_borrow()
            .map_err(|_| "reentrant bundle initialization")?
            .is_initialized(config.class)
        {
            return Err("NSBundle class +initialize has not genuinely executed".into());
        }
        if class_method {
            if receiver != config.class {
                return Err("NSBundle class receiver is foreign/cached".into());
            }
        } else {
            if !self.objects.values().any(|&value| value == receiver) {
                return Err("NSBundle instance is not an adapter-owned bundle".into());
            }
            r.lifetime
                .try_borrow()
                .map_err(|_| "reentrant bundle lifetime")?
                .check_object(receiver)?;
            if u64::from_le_bytes(frame.read(receiver, 8)?.try_into().unwrap()) != config.class {
                return Err("NSBundle instance isa was changed".into());
            }
        }
        Ok(receiver)
    }
    fn identity(&self, object: u64) -> Result<BundleId, String> {
        self.objects
            .iter()
            .find_map(|(&id, &value)| (value == object).then_some(id))
            .ok_or_else(|| "NSBundle object identity missing".into())
    }
    fn return_bundle(&mut self, frame: &mut ServiceFrame<'_>, id: BundleId) -> Result<u64, String> {
        self.model.metadata(id)?;
        let config = self.configuration()?;
        let runtime = config.runtime.clone();
        let class = config.class;
        let object = if let Some(&object) = self.objects.get(&id) {
            object
        } else {
            let object = runtime
                .try_borrow_mut()
                .map_err(|_| "reentrant bundle allocation")?
                .allocate_plain(frame, class, 0)?;
            self.objects.insert(id, object);
            object
        };
        // The model retains one singleton owner. Each non-owning method return
        // has an actual +1 autorelease receipt through the same lifetime pool.
        let lifetime = runtime
            .try_borrow()
            .map_err(|_| "reentrant bundle lifetime")?
            .lifetime
            .clone();
        lifetime
            .try_borrow_mut()
            .map_err(|_| "reentrant bundle return ownership")?
            .retain_autorelease(object)?;
        Ok(object)
    }
}
pub(super) fn install(
    bridge: &mut GuestBridge,
    cpu: &mut A64Cpu,
    state: Rc<RefCell<BundleServices>>,
) -> Result<Methods, String> {
    let main = state.clone();
    let with_path = state.clone();
    let path = state.clone();
    let loaded = state.clone();
    let load = state.clone();
    Ok(Methods{
        main:bridge.register_service(cpu,"_touchHLE_NSBundle_mainBundle",move|frame|{let mut s=main.try_borrow_mut().map_err(|_|"reentrant mainBundle")?;s.check_method(frame,"mainBundle",true)?;let id=s.model.main_bundle();Ok(ReturnValues::integer(s.return_bundle(frame,id)?))})?,
        with_path:bridge.register_service(cpu,"_touchHLE_NSBundle_bundleWithPath",move|frame|{let mut s=with_path.try_borrow_mut().map_err(|_|"reentrant bundleWithPath")?;s.check_method(frame,"bundleWithPath:",true)?;let object=frame.integer(2)?;let strings=s.configuration()?.strings.clone();let path=strings.try_borrow().map_err(|_|"reentrant bundle path string")?.read(frame,object)?;
            // Metadata was preloaded from exact real plists. Dynamic/foreign
            // paths cannot manufacture a bundle or silently read another file.
            let Some(id)=s.model.bundle_with_path(&path,|_|Ok(None))? else{return Ok(ReturnValues::integer(0))};Ok(ReturnValues::integer(s.return_bundle(frame,id)?))})?,
        path:bridge.register_service(cpu,"_touchHLE_NSBundle_bundlePath",move|frame|{let s=path.try_borrow().map_err(|_|"reentrant bundlePath")?;let object=s.check_method(frame,"bundlePath",false)?;let text=s.model.metadata(s.identity(object)?)?.path.clone();let strings=s.configuration()?.strings.clone();let result=strings.try_borrow_mut().map_err(|_|"reentrant bundlePath NSString")?.create_autoreleased(frame,&text)?;Ok(ReturnValues::integer(result))})?,
        loaded:bridge.register_service(cpu,"_touchHLE_NSBundle_isLoaded",move|frame|{let s=loaded.try_borrow().map_err(|_|"reentrant isLoaded")?;let object=s.check_method(frame,"isLoaded",false)?;Ok(ReturnValues::integer(s.model.is_loaded(s.identity(object)?)? as u64))})?,
        load:bridge.register_service(cpu,"_touchHLE_NSBundle_load",move|frame|{
            let (id, coordinator) = { let s=load.try_borrow().map_err(|_|"reentrant bundle load")?;
                let object=s.check_method(frame,"load",false)?;let id=s.identity(object)?;
                if s.model.is_loaded(id)? {return Ok(ReturnValues::integer(1));}
                echo!("[a64] actual NSBundle load request: {}; guest caller LR={:#x}",s.model.metadata(id)?.executable_path(),frame.caller_return_address());
                (id,s.load_coordinator.clone()) };
            let coordinator = coordinator.ok_or("NSBundle load requires genuine image/dependency/+load execution and registration receipts; mapping is insufficient")?;
            coordinator(frame,id)?;
            // Queued calls and their completion callbacks must succeed before
            // the bridge exposes this return value to the guest caller.
            Ok(ReturnValues::integer(1))
        })?,
        principal:bridge.register_service(cpu,"_touchHLE_NSBundle_principalClass",move|frame|{let s=state.try_borrow().map_err(|_|"reentrant principalClass")?;let object=s.check_method(frame,"principalClass",false)?;Ok(ReturnValues::integer(s.model.principal_class(s.identity(object)?)?.unwrap_or(0)))})?,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{
        bridge::GuestCall,
        bundle_metadata::Bundles,
        nsstring_services,
        objc_execution::Initialization,
        objc_execution_services::{self, Selectors},
        objc_heap::GuestObjectHeap,
        objc_lifetime::Lifetime,
        objc_metadata::Registry,
        objc_namespace::ClassSpec,
    };
    use super::*;
    fn plist(executable: &str, package: &str, principal: &str) -> Vec<u8> {
        format!("<plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>{executable}</string><key>CFBundlePackageType</key><string>{package}</string><key>NSPrincipalClass</key><string>{principal}</string></dict></plist>").into_bytes()
    }
    #[test]
    fn actual_guest_bundle_and_string_methods_use_real_namespace_heap_and_arc() {
        let mut cpu = A64Cpu::new_sparse();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        let lifetime = Rc::new(RefCell::new(Lifetime::default()));
        let arc = super::super::objc_lifetime_services::install_without_release(
            &mut bridge,
            &mut cpu,
            lifetime.clone(),
        )
        .unwrap();
        let strings = Rc::new(RefCell::new(Strings::default()));
        let string_methods =
            nsstring_services::install(&mut bridge, &mut cpu, strings.clone()).unwrap();
        let mut model =
            Bundles::from_main("/Terraria.app/Terraria", &plist("Terraria", "APPL", "App"))
                .unwrap();
        let unity = model
            .bundle_with_path("/Terraria.app/Frameworks/UnityFramework.framework", |_| {
                Ok(Some(plist("UnityFramework", "FMWK", "UnityFramework")))
            })
            .unwrap()
            .unwrap();
        let bundles = Rc::new(RefCell::new(BundleServices::new(model)));
        let bundle_methods = install(&mut bridge, &mut cpu, bundles.clone()).unwrap();
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
                    name: "NSString",
                    parent: Some("NSObject"),
                    instance_size: 24,
                    instance_methods: string_methods.instance_methods(),
                    class_methods: vec![],
                },
                ClassSpec {
                    name: "NSBundle",
                    parent: Some("NSObject"),
                    instance_size: 8,
                    instance_methods: bundle_methods.instance_methods(),
                    class_methods: bundle_methods.class_methods(),
                },
            ],
        )
        .unwrap();
        let registered = Registry::register(
            &namespace.class_addresses,
            &[],
            |address, length| {
                let mut bytes = vec![0; length];
                cpu.read_guest_into(address, &mut bytes)?;
                Ok(bytes)
            },
            |address, _| {
                if cpu.mapped_permissions(address).is_some_and(|p| p & 4 != 0) {
                    Ok(())
                } else {
                    Err("fixture non-RX method".into())
                }
            },
        )
        .unwrap();
        let registry = Rc::new(registered.registry);
        let initialization = Rc::new(RefCell::new(
            Initialization::new(registry.classes().cloned()).unwrap(),
        ));
        let heap = GuestObjectHeap::map(&mut cpu, 0x80000, 65536).unwrap();
        let runtime = Rc::new(RefCell::new(
            ObjectRuntime::new(
                registry.clone(),
                initialization.clone(),
                lifetime.clone(),
                heap,
                namespace.class_address("NSObject").unwrap(),
            )
            .unwrap(),
        ));
        let code = namespace.code_range();
        let executable = Rc::new(move |address: u64, length: usize| {
            if (address >= 0x20000
                && address
                    .checked_add(length as u64)
                    .is_some_and(|end| end <= 0x21000))
                || (address >= code.0
                    && address
                        .checked_add(length as u64)
                        .is_some_and(|end| end <= code.1))
            {
                Ok(())
            } else {
                Err("fixture non-owned executable".into())
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
        let service = |name: &str| {
            execution
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap()
                .1
                .guest_address()
        };
        namespace
            .link_root_services(
                &mut cpu,
                service("_class_createInstance"),
                service("_object_dispose"),
            )
            .unwrap();
        let string_class = namespace.class_address("NSString").unwrap();
        let bundle_class = namespace.class_address("NSBundle").unwrap();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        let suffix = b"/Frameworks/UnityFramework.framework";
        for (i, value) in [string_class, 0x7c8, 0x40100, suffix.len() as u64]
            .into_iter()
            .enumerate()
        {
            cpu.write_bytes(0x40000 + i as u64 * 8, &value.to_le_bytes());
        }
        cpu.write_bytes(0x40100, suffix);
        strings
            .borrow_mut()
            .configure(
                runtime.clone(),
                string_class,
                Some(nsstring_services::Constants {
                    class: string_class,
                    ranges: vec![(0x40000, 0x40020)],
                }),
                &namespace,
            )
            .unwrap();
        bundles
            .borrow_mut()
            .configure(runtime.clone(), bundle_class, strings.clone(), &namespace)
            .unwrap();
        let call = |bridge: &mut GuestBridge, cpu: &mut A64Cpu, entry, integers| {
            bridge
                .call(
                    cpu,
                    &GuestCall {
                        entry,
                        integers,
                        ..Default::default()
                    },
                    2000,
                )
                .unwrap()
                .integers[0]
        };
        let pool = arc
            .iter()
            .find(|(n, _)| *n == "_objc_autoreleasePoolPush")
            .unwrap()
            .1
            .guest_address();
        call(&mut bridge, &mut cpu, pool, vec![]);
        // Owned NSString initialization runs as a genuine inherited guest IMP
        // before a host method is allowed to allocate a wrapper instance.
        let empty = call(
            &mut bridge,
            &mut cpu,
            service("_objc_alloc_init"),
            vec![string_class],
        );
        assert!(initialization.borrow().is_initialized(string_class));
        let msg = service("_objc_msgSend");
        let main = call(
            &mut bridge,
            &mut cpu,
            msg,
            vec![bundle_class, namespace.selector("mainBundle").unwrap()],
        );
        assert_eq!(cpu.read_u64(main), Some(bundle_class));
        assert!(initialization.borrow().is_initialized(bundle_class));
        assert_eq!(
            call(
                &mut bridge,
                &mut cpu,
                msg,
                vec![bundle_class, namespace.selector("mainBundle").unwrap()]
            ),
            main
        );
        let path = call(
            &mut bridge,
            &mut cpu,
            msg,
            vec![main, namespace.selector("bundlePath").unwrap()],
        );
        assert_eq!(
            call(
                &mut bridge,
                &mut cpu,
                msg,
                vec![path, namespace.selector("length").unwrap()]
            ),
            13
        );
        let full = call(
            &mut bridge,
            &mut cpu,
            msg,
            vec![
                path,
                namespace.selector("stringByAppendingString:").unwrap(),
                0x40000,
            ],
        );
        let framework = call(
            &mut bridge,
            &mut cpu,
            msg,
            vec![
                bundle_class,
                namespace.selector("bundleWithPath:").unwrap(),
                full,
            ],
        );
        assert_ne!(framework, 0);
        assert_eq!(bundles.borrow().identity(framework).unwrap(), unity);
        assert_eq!(
            call(
                &mut bridge,
                &mut cpu,
                msg,
                vec![framework, namespace.selector("isLoaded").unwrap()]
            ),
            0
        );
        assert!(bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: msg,
                    integers: vec![framework, namespace.selector("load").unwrap()],
                    ..Default::default()
                },
                2000
            )
            .unwrap_err()
            .contains("receipts"));
        assert!(bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: msg,
                    integers: vec![framework, namespace.selector("principalClass").unwrap()],
                    ..Default::default()
                },
                2000
            )
            .is_err());
        // Object disposal must go through actual inherited -dealloc and the
        // same allocator. Reuse then writes a fresh generation payload.
        call(&mut bridge, &mut cpu, service("_objc_release"), vec![empty]);
        assert!(!lifetime.borrow().contains_identity(empty));
        let reused = call(
            &mut bridge,
            &mut cpu,
            service("_objc_alloc_init"),
            vec![string_class],
        );
        assert_eq!(reused, empty);
        assert_eq!(
            call(
                &mut bridge,
                &mut cpu,
                msg,
                vec![reused, namespace.selector("length").unwrap()]
            ),
            0
        );
    }
}

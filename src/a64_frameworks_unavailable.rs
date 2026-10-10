/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Owned "honestly unavailable" Objective-C classes for frameworks in
//! UIKitCore's dependency cone (StoreKit, GameKit, MessageUI, DeviceCheck,
//! LocalAuthentication). See ARM64_UIKIT_PLAN.md §4: they cannot load as
//! genuine cached code without dragging in Apple's UIKitCore.
//!
//! The classes live in their own synthetic objc2 image, built with the UIKit
//! layer's image builder (`uikit::image`). Each class carries its real
//! provider install name, so `Layout::exports()` yields
//! `(provider, _OBJC_CLASS_$_X, address)` triples for the same binder hookup
//! that UIKit's image will use. All methods reach ONE bridge service through
//! per-method index trampolines.
//!
//! Only answers that are true for a device with no Game Center account, no
//! payments, no mail/SMS accounts, no biometrics and no App Attest are
//! implemented. None of them needs a Foundation object:
//! - `+[SKPaymentQueue canMakePayments]` is NO.
//! - `+[GKLocalPlayer localPlayer].authenticated` is NO.
//! - `+[MF*ComposeViewController canSend*]` is NO.
//! - `+[DCDevice currentDevice].supported` is NO.
//! - `-[LAContext canEvaluatePolicy:error:]` is NO.
//!
//! Selectors whose real behavior needs an NSError, an NSArray or a completion
//! block are deliberately absent. Sending one fails as an unrecognized
//! selector instead of returning a fabricated success. Examples: GameKit
//! authentication handlers, product requests, the NSError out-parameter of
//! `canEvaluatePolicy:error:` and SKAdNetwork completion handlers.
use super::super::uikit::image::{
    self, BuiltImage, ClassDef, External, ImageSpec, Layout, MethodDef, StaticObject,
};
use crate::a64::bridge::{GuestBridge, ReturnValues, ServiceFrame};
use crate::a64::A64Cpu;
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

/// Internal name of the synthetic image (not an Apple path).
pub(super) const INSTALL_NAME: &str = "/usr/lib/touchHLE/libA64UnavailableFrameworks.dylib";
const STORE_KIT: &str = "/System/Library/Frameworks/StoreKit.framework/StoreKit";
const GAME_KIT: &str = "/System/Library/Frameworks/GameKit.framework/GameKit";
const MESSAGE_UI: &str = "/System/Library/Frameworks/MessageUI.framework/MessageUI";
const DEVICE_CHECK: &str = "/System/Library/Frameworks/DeviceCheck.framework/DeviceCheck";
const LOCAL_AUTHENTICATION: &str =
    "/System/Library/Frameworks/LocalAuthentication.framework/LocalAuthentication";

/// Host-side state: only what the implemented selectors need.
#[derive(Default)]
pub(in crate::a64) struct Unavailable {
    statics: Vec<(String, u64)>,
    /// SKPaymentTransactionObserver objects registered with the queue. No
    /// transaction will ever be delivered because payments are disabled.
    pub transaction_observers: BTreeSet<u64>,
    pub first_use: Vec<String>,
}
impl Unavailable {
    fn singleton(&self, class: &str) -> Result<u64, String> {
        self.statics
            .iter()
            .find(|(c, _)| c == class)
            .map(|&(_, a)| a)
            .ok_or_else(|| format!("{class} singleton missing"))
    }
}

type Handler = fn(&mut Unavailable, &mut ServiceFrame<'_>) -> Result<ReturnValues, String>;
struct Method {
    selector: &'static str,
    types: &'static str,
    class_method: bool,
    handler: Handler,
}
struct Table {
    name: &'static str,
    provider: &'static str,
    methods: &'static [Method],
}

fn no(_: &mut Unavailable, _: &mut ServiceFrame<'_>) -> Result<ReturnValues, String> {
    Ok(ReturnValues::integer(0))
}
fn nothing(_: &mut Unavailable, _: &mut ServiceFrame<'_>) -> Result<ReturnValues, String> {
    Ok(ReturnValues::integer(0))
}

const TABLES: &[Table] = &[
    Table {
        name: "SKPaymentQueue",
        provider: STORE_KIT,
        methods: &[
            Method { selector: "canMakePayments", types: "B16@0:8", class_method: true, handler: no },
            Method {
                selector: "defaultQueue",
                types: "@16@0:8",
                class_method: true,
                handler: |state, _| Ok(ReturnValues::integer(state.singleton("SKPaymentQueue")?)),
            },
            Method {
                selector: "addTransactionObserver:",
                types: "v24@0:8@16",
                class_method: false,
                handler: |state, frame| {
                    state.transaction_observers.insert(frame.integer(2)?);
                    Ok(ReturnValues::integer(0))
                },
            },
            Method {
                selector: "removeTransactionObserver:",
                types: "v24@0:8@16",
                class_method: false,
                handler: |state, frame| {
                    state.transaction_observers.remove(&frame.integer(2)?);
                    Ok(ReturnValues::integer(0))
                },
            },
        ],
    },
    // Apple documents that the review prompt may not be shown; with no App
    // Store account it never is.
    Table {
        name: "SKStoreReviewController",
        provider: STORE_KIT,
        methods: &[Method { selector: "requestReview", types: "v16@0:8", class_method: true, handler: nothing }],
    },
    // Legacy attribution registration has no result; nothing is sent.
    Table {
        name: "SKAdNetwork",
        provider: STORE_KIT,
        methods: &[Method {
            selector: "registerAppForAdNetworkAttribution",
            types: "v16@0:8",
            class_method: true,
            handler: nothing,
        }],
    },
    Table {
        name: "GKLocalPlayer",
        provider: GAME_KIT,
        methods: &[
            Method {
                selector: "localPlayer",
                types: "@16@0:8",
                class_method: true,
                handler: |state, _| Ok(ReturnValues::integer(state.singleton("GKLocalPlayer")?)),
            },
            Method { selector: "isAuthenticated", types: "B16@0:8", class_method: false, handler: no },
            Method { selector: "isUnderage", types: "B16@0:8", class_method: false, handler: no },
        ],
    },
    Table {
        name: "MFMailComposeViewController",
        provider: MESSAGE_UI,
        methods: &[Method { selector: "canSendMail", types: "B16@0:8", class_method: true, handler: no }],
    },
    Table {
        name: "MFMessageComposeViewController",
        provider: MESSAGE_UI,
        methods: &[Method { selector: "canSendText", types: "B16@0:8", class_method: true, handler: no }],
    },
    Table {
        name: "DCDevice",
        provider: DEVICE_CHECK,
        methods: &[
            Method {
                selector: "currentDevice",
                types: "@16@0:8",
                class_method: true,
                handler: |state, _| Ok(ReturnValues::integer(state.singleton("DCDevice")?)),
            },
            Method { selector: "isSupported", types: "B16@0:8", class_method: false, handler: no },
        ],
    },
    Table {
        name: "LAContext",
        provider: LOCAL_AUTHENTICATION,
        methods: &[Method {
            selector: "canEvaluatePolicy:error:",
            types: "B32@0:8q16^@24",
            class_method: false,
            handler: no,
        }],
    },
];

pub(in crate::a64) struct Installed {
    pub state: Rc<RefCell<Unavailable>>,
    pub layout: Layout,
}

/// Build and map the image at `base` (16 KiB aligned, below 64 GiB) and
/// register its single dispatcher service.
pub(in crate::a64) fn install(
    cpu: &mut A64Cpu,
    bridge: &mut GuestBridge,
    base: u64,
    external: External,
) -> Result<Installed, String> {
    let state = Rc::new(RefCell::new(Unavailable::default()));
    let mut entries: Vec<(&'static str, &'static Method)> = Vec::new();
    let mut defs = Vec::new();
    for table in TABLES {
        let mut instance_methods = Vec::new();
        let mut class_methods = Vec::new();
        for method in table.methods {
            let index = u16::try_from(entries.len()).map_err(|_| "entry overflow")?;
            entries.push((table.name, method));
            let def = MethodDef { selector: method.selector, types: method.types, index };
            if method.class_method {
                class_methods.push(def);
            } else {
                instance_methods.push(def);
            }
        }
        defs.push(ClassDef {
            name: table.name,
            provider: table.provider,
            parent: None,
            instance_size: 8,
            instance_methods,
            class_methods,
            dealloc_cleanup: false,
        });
    }
    let entry_count = u16::try_from(entries.len()).map_err(|_| "entry overflow")?;
    let dispatcher = {
        let state = state.clone();
        bridge.register_service(cpu, "_touchHLE_a64_frameworks_unavailable", move |frame| {
            let index = frame.dispatch_index() as usize;
            let &(class, method) = entries
                .get(index)
                .ok_or_else(|| format!("unavailable-framework dispatch index {index} invalid"))?;
            let mut state = state.try_borrow_mut().map_err(|_| "reentrant unavailable-framework call")?;
            if state.first_use.len() < 256 {
                let sign = if method.class_method { '+' } else { '-' };
                let name = format!("{sign}[{class} {}]", method.selector);
                if !state.first_use.contains(&name) {
                    log!("[a64] owned unavailable framework: {name}");
                    state.first_use.push(name);
                }
            }
            (method.handler)(&mut state, frame)
        })?
    };
    let statics = [
        StaticObject { class: "SKPaymentQueue", size: 16 },
        StaticObject { class: "GKLocalPlayer", size: 16 },
        StaticObject { class: "DCDevice", size: 16 },
    ];
    let built: BuiltImage = image::build(&ImageSpec {
        base,
        install_name: INSTALL_NAME,
        classes: &defs,
        statics: &statics,
        functions: &[],
        send_selectors: &[],
        got: &[],
        entry_count,
        cleanup_entry: None,
        dispatcher: dispatcher.guest_address(),
        scratch_bytes: 0,
        external,
    })?;
    built.map(cpu)?;
    let layout = built.layout.clone();
    state.borrow_mut().statics = layout.static_objects.clone();
    Ok(Installed { state, layout })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a64::bridge::GuestCall;
    use crate::a64::objc_execution::Initialization;
    use crate::a64::objc_execution_services::{self, ObjectRuntime, Selectors};
    use crate::a64::objc_heap::GuestObjectHeap;
    use crate::a64::objc_lifetime::Lifetime;
    use crate::a64::objc_metadata::Registry;
    use crate::a64::objc_namespace::{ClassSpec, Namespace};
    use std::collections::BTreeMap;

    const IMAGE_BASE: u64 = 0x40_0000;

    struct Fixture {
        cpu: A64Cpu,
        bridge: GuestBridge,
        registry: Rc<Registry>,
        services: BTreeMap<&'static str, u64>,
        installed: Installed,
    }
    impl Fixture {
        fn new() -> Self {
            let mut cpu = A64Cpu::new_sparse();
            let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
            let lifetime = Rc::new(RefCell::new(Lifetime::default()));
            let lifetime_services = crate::a64::objc_lifetime_services::install_without_release(
                &mut bridge,
                &mut cpu,
                lifetime.clone(),
            )
            .unwrap();
            let mut root = Namespace::map(
                &mut cpu,
                0xa0000,
                &[ClassSpec {
                    name: "NSObject",
                    parent: None,
                    instance_size: 8,
                    instance_methods: vec![],
                    class_methods: vec![],
                }],
            )
            .unwrap();
            let external = External {
                root_class: root.class_address("NSObject").unwrap(),
                root_metaclass: root.metaclass_address("NSObject").unwrap(),
                empty_cache: 0,
            };
            let installed = install(&mut cpu, &mut bridge, IMAGE_BASE, external).unwrap();
            let layout = installed.layout.clone();
            let mut classes = vec![external.root_class];
            classes.extend(&layout.class_list);
            let selrefs: Vec<u64> = layout.selector_refs.values().copied().collect();
            let registration = Registry::register(
                &classes,
                &selrefs,
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
            let registry = Rc::new(registration.registry);
            let initialization =
                Rc::new(RefCell::new(Initialization::new(registry.classes().cloned()).unwrap()));
            let heap = GuestObjectHeap::map(&mut cpu, 0x80000, 0x10000).unwrap();
            let runtime = Rc::new(RefCell::new(
                ObjectRuntime::new(registry.clone(), initialization, lifetime.clone(), heap, external.root_class)
                    .unwrap(),
            ));
            let mut allowed = bridge.instruction_ranges();
            allowed.push(root.code_range());
            allowed.push((layout.text.0, layout.text.0 + layout.text.1));
            let executable = Rc::new(move |address: u64, length: usize| {
                if allowed.iter().any(|&(start, end)| {
                    address >= start && address.checked_add(length as u64).is_some_and(|e| e <= end)
                }) {
                    Ok(())
                } else {
                    Err("test: outside owned executable ranges".into())
                }
            });
            let execution = objc_execution_services::install(
                &mut cpu,
                &mut bridge,
                runtime,
                Selectors::from_registry(&registry).unwrap(),
                executable,
                0x90000,
            )
            .unwrap();
            let mut services: BTreeMap<&'static str, u64> =
                execution.iter().map(|(n, id)| (*n, id.guest_address())).collect();
            services.extend(lifetime_services.iter().map(|(n, id)| (*n, id.guest_address())));
            root.link_root_services(&mut cpu, services["_class_createInstance"], services["_object_dispose"])
                .unwrap();
            for class in registry.classes() {
                lifetime.borrow_mut().register_immortal(class.address).unwrap();
            }
            for (_, object) in &layout.static_objects {
                lifetime.borrow_mut().register_immortal(*object).unwrap();
            }
            Self { cpu, bridge, registry, services, installed }
        }
        fn try_send(&mut self, receiver: u64, selector: &str, args: &[u64]) -> Result<u64, String> {
            let mut integers = vec![receiver, self.registry.selector_named(selector).unwrap_or(0)];
            integers.extend_from_slice(args);
            self.bridge
                .call(
                    &mut self.cpu,
                    &GuestCall { entry: self.services["_objc_msgSend"], integers, ..Default::default() },
                    1_000_000,
                )
                .map(|values| values.integers[0])
        }
        fn send(&mut self, receiver: u64, selector: &str, args: &[u64]) -> u64 {
            self.try_send(receiver, selector, args).unwrap_or_else(|e| panic!("{selector}: {e}"))
        }
        fn class(&self, name: &str) -> u64 {
            self.installed.layout.class(name).unwrap()
        }
    }

    #[test]
    fn exports_carry_real_provider_install_names() {
        let fixture = Fixture::new();
        let exports = fixture.installed.layout.exports();
        let has = |provider: &str, symbol: &str| {
            exports.iter().any(|(p, s, _)| p == provider && s == symbol)
        };
        assert!(has(STORE_KIT, "_OBJC_CLASS_$_SKPaymentQueue"));
        assert!(has(GAME_KIT, "_OBJC_CLASS_$_GKLocalPlayer"));
        assert!(has(MESSAGE_UI, "_OBJC_METACLASS_$_MFMailComposeViewController"));
        assert!(has(DEVICE_CHECK, "_OBJC_CLASS_$_DCDevice"));
        assert!(has(LOCAL_AUTHENTICATION, "_OBJC_CLASS_$_LAContext"));
        assert_eq!(fixture.registry.lookup_class("GKLocalPlayer"), Some(fixture.class("GKLocalPlayer")));
    }

    #[test]
    fn unavailable_capabilities_answer_honestly_through_the_owned_runtime() {
        let mut f = Fixture::new();
        let payment_queue = f.class("SKPaymentQueue");
        assert_eq!(f.send(payment_queue, "canMakePayments", &[]), 0);
        let queue = f.send(payment_queue, "defaultQueue", &[]);
        assert_eq!(f.send(payment_queue, "defaultQueue", &[]), queue);
        assert_eq!(f.cpu.read_u64(queue), Some(payment_queue));
        f.send(queue, "addTransactionObserver:", &[0x1234]);
        assert!(f.installed.state.borrow().transaction_observers.contains(&0x1234));
        f.send(queue, "removeTransactionObserver:", &[0x1234]);
        assert!(f.installed.state.borrow().transaction_observers.is_empty());

        let local_player_class = f.class("GKLocalPlayer");
        let player = f.send(local_player_class, "localPlayer", &[]);
        assert_ne!(player, 0);
        assert_eq!(f.send(player, "isAuthenticated", &[]), 0);

        let mail = f.class("MFMailComposeViewController");
        assert_eq!(f.send(mail, "canSendMail", &[]), 0);
        let text = f.class("MFMessageComposeViewController");
        assert_eq!(f.send(text, "canSendText", &[]), 0);

        let device_class = f.class("DCDevice");
        let device = f.send(device_class, "currentDevice", &[]);
        assert_eq!(f.send(device, "isSupported", &[]), 0);

        // LAContext instances come from the ordinary root alloc/init.
        let context_class = f.class("LAContext");
        let context = f
            .bridge
            .call(
                &mut f.cpu,
                &GuestCall { entry: f.services["_objc_alloc_init"], integers: vec![context_class], ..Default::default() },
                1_000_000,
            )
            .unwrap()
            .integers[0];
        assert_ne!(context, 0);
        assert_eq!(f.send(context, "canEvaluatePolicy:error:", &[2, 0]), 0);

        let review = f.class("SKStoreReviewController");
        f.send(review, "requestReview", &[]);
        assert!(f
            .installed
            .state
            .borrow()
            .first_use
            .contains(&"+[SKStoreReviewController requestReview]".to_string()));
    }

    #[test]
    fn selectors_needing_foundation_objects_are_not_faked() {
        let mut f = Fixture::new();
        let player = {
            let class = f.class("GKLocalPlayer");
            f.send(class, "localPlayer", &[])
        };
        // The registry does not even know these selectors: they are not
        // implemented, so the owned runtime refuses the message.
        for selector in ["setAuthenticateHandler:", "alias", "playerID"] {
            assert!(f.registry.selector_named(selector).is_none(), "{selector} must stay absent");
        }
        let queue = {
            let class = f.class("SKPaymentQueue");
            f.send(class, "defaultQueue", &[])
        };
        // A known selector on the wrong class is not answered either.
        assert!(f.try_send(queue, "isAuthenticated", &[]).is_err());
        assert!(f.try_send(player, "canMakePayments", &[]).is_err());
    }
}

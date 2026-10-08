/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Own precisely implemented Foundation classes and method services.
//! This does not initialize Apple cached classes or load Unity implicitly.
use super::{
    bridge::{GuestBridge, GuestCall, ServiceId},
    bundle_metadata::Bundles,
    bundle_services::{self, BundleServices},
    nsstring_services::{self, Constants, Strings},
    objc_execution::Initialization,
    objc_execution_services::{self, ObjectRuntime, Selectors},
    objc_heap::GuestObjectHeap,
    objc_lifetime::Lifetime,
    objc_metadata::Registry,
    objc_namespace::{ClassSpec, Namespace},
    A64Cpu,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
pub(super) const MAPPED_BYTES: u64 = 0x11000 + 0x40000 + 0x1000;
pub(super) const RESERVED_BYTES: u64 = 0x55000;
pub(super) struct Inputs {
    pub executable_path: String,
    pub main_plist: Vec<u8>,
    pub unity_plist: Vec<u8>,
    pub constants: Option<Constants>,
}
pub(super) struct Foundation {
    pub namespace: Namespace,
    pub runtime: Rc<RefCell<ObjectRuntime>>,
    pub bundles: Rc<RefCell<BundleServices>>,
    pub strings: Rc<RefCell<Strings>>,
    pub services: Vec<(&'static str, ServiceId)>,
}
impl Foundation {
    pub(super) fn exports(&self) -> Result<BTreeMap<String, u64>, String> {
        self.namespace.exports()
    }
    pub(super) fn owned_code(&self) -> (u64, u64) {
        self.namespace.code_range()
    }
}

/// Requires the caller's existing ARC bridge/lifetime; registered release is
/// deliberately upgraded in place so already bound ARC slots stay valid.
pub(super) fn install(
    cpu: &mut A64Cpu,
    bridge: &mut GuestBridge,
    lifetime: Rc<RefCell<Lifetime>>,
    base: u64,
    inputs: Inputs,
) -> Result<Foundation, String> {
    if base == 0 || base & 4095 != 0 || base.checked_add(RESERVED_BYTES).is_none() {
        return Err("owned Foundation scratch address invalid".into());
    }
    let mut model = Bundles::from_main(&inputs.executable_path, &inputs.main_plist)?;
    let main_path = model.metadata(model.main_bundle())?.path.clone();
    let unity_path = format!("{main_path}/Frameworks/UnityFramework.framework");
    let unity = model
        .bundle_with_path(&unity_path, |path| {
            if path == format!("{unity_path}/Info.plist") {
                Ok(Some(inputs.unity_plist.clone()))
            } else {
                Err("Foundation preload path mismatch".into())
            }
        })?
        .ok_or("actual Unity bundle metadata absent")?;
    let metadata = model.metadata(unity)?;
    if metadata.executable != "UnityFramework"
        || metadata.principal_class.as_deref() != Some("UnityFramework")
        || metadata.package_type != "FMWK"
    {
        return Err(
            "owned Foundation diagnostic requires actual UnityFramework bundle metadata".into(),
        );
    }
    let strings = Rc::new(RefCell::new(Strings::default()));
    let string_methods = nsstring_services::install(bridge, cpu, strings.clone())?;
    let bundles = Rc::new(RefCell::new(BundleServices::new(model)));
    let bundle_methods = bundle_services::install(bridge, cpu, bundles.clone())?;
    let mut namespace = Namespace::map(
        cpu,
        base,
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
    )?;
    let registered = Registry::register(
        &namespace.class_addresses,
        &[],
        |address, length| {
            let mut data = vec![0; length];
            cpu.read_guest_into(address, &mut data)?;
            Ok(data)
        },
        |address, length| validate_rx(cpu, address, length),
    )?;
    let registry = Rc::new(registered.registry);
    let initialization = Rc::new(RefCell::new(Initialization::new(
        registry.classes().cloned(),
    )?));
    let heap = GuestObjectHeap::map(cpu, base + 0x14000, 0x40000)?;
    let runtime = Rc::new(RefCell::new(ObjectRuntime::new(
        registry.clone(),
        initialization,
        lifetime.clone(),
        heap,
        namespace
            .class_address("NSObject")
            .ok_or("owned NSObject absent")?,
    )?));
    let mut allowed = bridge.instruction_ranges();
    allowed.push(namespace.code_range());
    // These addresses belong only to this bridge page. Runtime's independent
    // trap validation still requires an exact registered token/site.
    let bridge_page = allowed[0].0 & !4095;
    allowed.push((bridge_page, bridge_page + 4096));
    let executable = Rc::new(move |address: u64, length: usize| {
        if allowed.iter().any(|&(start, end)| {
            address >= start
                && address
                    .checked_add(length as u64)
                    .is_some_and(|next| next <= end)
        }) {
            Ok(())
        } else {
            Err("method is outside owned Foundation executable ranges".into())
        }
    });
    let mut services = objc_execution_services::install_upgrading_release(
        cpu,
        bridge,
        runtime.clone(),
        Selectors::from_registry(&registry)?,
        executable,
        base + 0x54000,
    )?;
    let release_entry = services
        .iter()
        .find(|(name, _)| *name == "_objc_release")
        .ok_or("owned Objective-C release service absent")?
        .1
        .guest_address();
    services.extend(super::objc_arc_services::install(
        bridge,
        cpu,
        runtime.clone(),
        release_entry,
    )?);
    let service = |name: &str| {
        services
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, id)| id.guest_address())
            .ok_or_else(|| format!("owned Objective-C service absent: {name}"))
    };
    namespace.link_root_services(
        cpu,
        service("_class_createInstance")?,
        service("_object_dispose")?,
    )?;
    let string_class = namespace
        .class_address("NSString")
        .ok_or("owned NSString absent")?;
    let bundle_class = namespace
        .class_address("NSBundle")
        .ok_or("owned NSBundle absent")?;
    strings
        .borrow_mut()
        .configure(runtime.clone(), string_class, inputs.constants, &namespace)?;
    bundles
        .borrow_mut()
        .configure(runtime.clone(), bundle_class, strings.clone(), &namespace)?;
    // Known generated class objects have genuine immortal class ownership.
    for class in registry.classes() {
        lifetime.borrow_mut().register_immortal(class.address)?;
    }
    // Only the implemented emulator-owned NSString is prepared here. Its
    // inherited NSObject +initialize, allocation, -init and -dealloc all run
    // as actual guest instructions; no cached/app initialization is claimed.
    let object = bridge
        .call(
            cpu,
            &GuestCall {
                entry: service("_objc_alloc_init")?,
                integers: vec![string_class],
                ..Default::default()
            },
            10_000,
        )?
        .integers[0];
    bridge.call(
        cpu,
        &GuestCall {
            entry: service("_objc_release")?,
            integers: vec![object],
            ..Default::default()
        },
        10_000,
    )?;
    if lifetime.borrow().contains_identity(object) {
        return Err("owned NSString preparation lacks genuine disposal receipt".into());
    }
    Ok(Foundation {
        namespace,
        runtime,
        bundles,
        strings,
        services,
    })
}
fn validate_rx(cpu: &A64Cpu, address: u64, length: usize) -> Result<(), String> {
    if length > 4096 {
        return Err("Foundation method RX validation length invalid".into());
    }
    for offset in 0..length {
        if !cpu
            .mapped_permissions(
                address
                    .checked_add(offset as u64)
                    .ok_or("method RX overflow")?,
            )
            .is_some_and(|p| p & 4 != 0)
        {
            return Err("Foundation method target is not mapped executable".into());
        }
    }
    Ok(())
}

/// Discover only original, file-backed CF constant slots in the selected
/// main Mach-O. The class identity is separately resolved from the genuine
/// CoreFoundation export; this routine never invents it from a class name.
pub(super) fn constants_for_main(file: &[u8], slide: u64, class: u64) -> Result<Constants, String> {
    let bytes = super::thin_arm64_slice(file)?;
    let metadata = super::MachO64::parse_metadata(bytes)?;
    let u32_at = |offset: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset.checked_add(4).ok_or("constant command overflow")?)
                .ok_or("constant command truncated")?
                .try_into()
                .unwrap(),
        ))
    };
    let u64_at = |offset: usize| -> Result<u64, String> {
        Ok(u64::from_le_bytes(
            bytes
                .get(offset..offset.checked_add(8).ok_or("constant command overflow")?)
                .ok_or("constant command truncated")?
                .try_into()
                .unwrap(),
        ))
    };
    let mut offset = 32usize;
    let mut ranges = Vec::new();
    for _ in 0..u32_at(16)? {
        let command = u32_at(offset)?;
        let size = u32_at(offset + 4)? as usize;
        if command == 0x19 {
            let count = u32_at(offset + 64)? as usize;
            for index in 0..count {
                let section = offset
                    .checked_add(72)
                    .and_then(|o| index.checked_mul(80).and_then(|n| o.checked_add(n)))
                    .ok_or("constant section overflow")?;
                let name = bytes
                    .get(section..section + 16)
                    .ok_or("constant section truncated")?;
                if name.split(|&b| b == 0).next() != Some(b"__cfstring".as_slice()) {
                    continue;
                }
                let address = u64_at(section + 32)?;
                let length = u64_at(section + 40)?;
                let flags = u32_at(section + 64)? & 0xff;
                if !matches!(flags, 0 | 11) || length == 0 || length % 32 != 0 {
                    return Err("constant NSString section layout unsupported".into());
                }
                let end = address
                    .checked_add(length)
                    .ok_or("constant section VM overflow")?;
                if !metadata.segments.iter().any(|s| {
                    s.initprot & 1 != 0
                        && address >= s.vmaddr
                        && s.vmaddr
                            .checked_add(s.filesize)
                            .is_some_and(|file_end| end <= file_end)
                }) {
                    return Err("constant NSString section is not wholly file backed".into());
                }
                if ranges.len() >= 128 {
                    return Err("constant NSString section limit exceeded".into());
                }
                ranges.push((
                    address
                        .checked_add(slide)
                        .ok_or("constant section slide overflow")?,
                    end.checked_add(slide)
                        .ok_or("constant section slide overflow")?,
                ));
            }
        }
        offset = offset
            .checked_add(size)
            .ok_or("constant load command overflow")?;
    }
    if class == 0 || class & 7 != 0 {
        return Err("constant NSString genuine class identity invalid".into());
    }
    Ok(Constants { class, ranges })
}

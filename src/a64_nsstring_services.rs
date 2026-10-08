/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! NSString methods for an explicitly registered emulator-owned plain class.
//! Guest objects use the shared Objective-C heap and real ARC ownership; CF
//! opaque identities and arbitrary cached NSString objects are rejected.
use super::{
    bridge::{GuestBridge, ReturnValues, ServiceFrame, ServiceId},
    objc_execution_services::ObjectRuntime,
    objc_namespace::Namespace,
    A64Cpu,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
const MAX_UNITS: usize = 65536;
const MAX_PAYLOADS: usize = 65536;

pub(super) struct Constants {
    pub class: u64,
    pub ranges: Vec<(u64, u64)>,
}
struct Configuration {
    runtime: Rc<RefCell<ObjectRuntime>>,
    class: u64,
    selectors: BTreeMap<String, u64>,
    constants: Option<Constants>,
}
#[derive(Default)]
pub(super) struct Strings {
    configuration: Option<Configuration>,
    payloads: BTreeMap<u64, (u64, usize)>,
    next_generation: u64,
}
pub(super) struct Methods {
    pub init: ServiceId,
    pub length: ServiceId,
    pub append: ServiceId,
}
impl Methods {
    pub(super) fn instance_methods(&self) -> Vec<super::objc_namespace::MethodSpec<'static>> {
        [
            ("init", "@16@0:8", self.init),
            ("length", "Q16@0:8", self.length),
            ("stringByAppendingString:", "@24@0:8@16", self.append),
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
impl Strings {
    pub(super) fn configure(
        &mut self,
        runtime: Rc<RefCell<ObjectRuntime>>,
        class: u64,
        constants: Option<Constants>,
        namespace: &Namespace,
    ) -> Result<(), String> {
        if self.configuration.is_some() {
            return Err("NSString adapter already configured".into());
        }
        let r = runtime
            .try_borrow()
            .map_err(|_| "reentrant NSString configuration")?;
        if namespace.class_address("NSString") != Some(class)
            || r.registry.lookup_class("NSString") != Some(class)
        {
            return Err("NSString requires exact registered emulator class identity".into());
        }
        if !r
            .registry
            .classes()
            .any(|c| c.address == class && c.instance_size == 24 && c.flags & 1 == 0)
        {
            return Err(
                "NSString adapter requires its exact 24-byte registered instance layout".into(),
            );
        }
        let selectors = ["init", "length", "stringByAppendingString:"]
            .into_iter()
            .map(|name| {
                r.registry
                    .selector_named(name)
                    .map(|sel| (name.into(), sel))
                    .ok_or_else(|| format!("NSString selector absent: {name}"))
            })
            .collect::<Result<_, _>>()?;
        drop(r);
        if let Some(constants) = &constants {
            if constants.class == 0
                || constants.class & 7 != 0
                || constants.ranges.len() > 128
                || constants
                    .ranges
                    .iter()
                    .any(|&(a, b)| a >= b || (b - a) % 32 != 0)
            {
                return Err("NSString constant audit bounds invalid".into());
            }
        }
        self.configuration = Some(Configuration {
            runtime,
            class,
            selectors,
            constants,
        });
        Ok(())
    }
    fn configuration(&self) -> Result<&Configuration, String> {
        self.configuration
            .as_ref()
            .ok_or_else(|| "NSString emulator namespace not configured".into())
    }
    fn validate_owned(&self, frame: &mut ServiceFrame<'_>, object: u64) -> Result<(), String> {
        let config = self.configuration()?;
        let runtime = config
            .runtime
            .try_borrow()
            .map_err(|_| "reentrant NSString ownership")?;
        if !runtime
            .initialization
            .try_borrow()
            .map_err(|_| "reentrant NSString initialization")?
            .is_initialized(config.class)
        {
            return Err("NSString class initialization has not executed".into());
        }
        runtime
            .lifetime
            .try_borrow()
            .map_err(|_| "reentrant NSString lifetime")?
            .check_object(object)?;
        if object == 0
            || u64::from_le_bytes(frame.read(object, 8)?.try_into().unwrap()) != config.class
        {
            return Err("NSString receiver has foreign isa".into());
        }
        Ok(())
    }
    fn method(&self, frame: &mut ServiceFrame<'_>, name: &str) -> Result<u64, String> {
        let config = self.configuration()?;
        if config.selectors.get(name).copied() != Some(frame.integer(1)?) {
            return Err("NSString method selector is not canonical".into());
        }
        let object = frame.integer(0)?;
        self.validate_owned(frame, object)?;
        Ok(object)
    }
    fn initialize_payload(
        &mut self,
        frame: &mut ServiceFrame<'_>,
        object: u64,
        units: &[u16],
    ) -> Result<(), String> {
        self.validate_owned(frame, object)?;
        if units.len() > MAX_UNITS
            || (!self.payloads.contains_key(&object) && self.payloads.len() >= MAX_PAYLOADS)
        {
            return Err("NSString bounded payload limit".into());
        }
        let generation = self
            .next_generation
            .checked_add(1)
            .ok_or("NSString generation exhausted")?;
        let mut bytes = Vec::with_capacity(16 + units.len() * 2);
        bytes.extend_from_slice(&generation.to_le_bytes());
        bytes.extend_from_slice(&(units.len() as u64).to_le_bytes());
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        frame.write(
            object.checked_add(8).ok_or("NSString payload overflow")?,
            &bytes,
        )?;
        self.payloads.insert(object, (generation, units.len()));
        self.next_generation = generation;
        Ok(())
    }
    pub(super) fn read(&self, frame: &mut ServiceFrame<'_>, object: u64) -> Result<String, String> {
        if let Some(&(generation, length)) = self.payloads.get(&object) {
            self.validate_owned(frame, object)?;
            let header = frame.read(
                object.checked_add(8).ok_or("NSString payload overflow")?,
                16,
            )?;
            if u64::from_le_bytes(header[..8].try_into().unwrap()) != generation
                || u64::from_le_bytes(header[8..].try_into().unwrap()) != length as u64
            {
                return Err(
                    "NSString payload generation/length invalid after reuse or mutation".into(),
                );
            }
            let data = frame.read(
                object.checked_add(24).ok_or("NSString text overflow")?,
                length * 2,
            )?;
            let units = data
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>();
            return String::from_utf16(&units)
                .map_err(|_| "NSString payload contains invalid UTF16".into());
        }
        // Clang CF constant layout is admitted only at caller-audited original
        // __cfstring slots with the genuine bound constant-class identity.
        let constants = self
            .configuration()?
            .constants
            .as_ref()
            .ok_or("foreign NSString object has no supported payload")?;
        if !constants.ranges.iter().any(|&(start, end)| {
            object >= start
                && (object - start) % 32 == 0
                && object.checked_add(32).is_some_and(|next| next <= end)
        }) {
            return Err("foreign NSString is outside audited constant slots".into());
        }
        let header = frame.read(object, 32)?;
        let word = |offset| u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap());
        if word(0) != constants.class || word(8) != 0x7c8 {
            return Err("unsupported constant NSString identity/flags".into());
        }
        let length = usize::try_from(word(24)).map_err(|_| "constant NSString length overflow")?;
        if length > MAX_UNITS {
            return Err("constant NSString exceeds bounded length".into());
        }
        let data = frame.read(word(16), length + 1)?;
        if data[length] != 0 || !data[..length].is_ascii() {
            return Err("constant NSString requires audited ASCII/null-terminated layout".into());
        }
        String::from_utf8(data[..length].to_vec())
            .map_err(|_| "constant NSString encoding invalid".into())
    }
    pub(super) fn create_autoreleased(
        &mut self,
        frame: &mut ServiceFrame<'_>,
        text: &str,
    ) -> Result<u64, String> {
        let units = text.encode_utf16().collect::<Vec<_>>();
        if units.len() > MAX_UNITS {
            return Err("NSString text exceeds bounded UTF16 length".into());
        }
        let config = self.configuration()?;
        let runtime = config.runtime.clone();
        let class = config.class;
        let object = runtime
            .try_borrow_mut()
            .map_err(|_| "reentrant NSString allocation")?
            .allocate_plain(frame, class, units.len() * 2)?;
        self.initialize_payload(frame, object, &units)?;
        let lifetime = runtime
            .try_borrow()
            .map_err(|_| "reentrant NSString lifetime")?
            .lifetime
            .clone();
        lifetime
            .try_borrow_mut()
            .map_err(|_| "reentrant NSString autorelease")?
            .autorelease(object)?;
        Ok(object)
    }
}
pub(super) fn install(
    bridge: &mut GuestBridge,
    cpu: &mut A64Cpu,
    state: Rc<RefCell<Strings>>,
) -> Result<Methods, String> {
    let init = state.clone();
    let length = state.clone();
    Ok(Methods {
        init: bridge.register_service(cpu, "_touchHLE_NSString_init", move |frame| {
            let mut s = init
                .try_borrow_mut()
                .map_err(|_| "reentrant NSString init")?;
            let object = s.method(frame, "init")?;
            s.initialize_payload(frame, object, &[])?;
            Ok(ReturnValues::integer(object))
        })?,
        length: bridge.register_service(cpu, "_touchHLE_NSString_length", move |frame| {
            let s = length
                .try_borrow()
                .map_err(|_| "reentrant NSString length")?;
            let object = s.method(frame, "length")?;
            Ok(ReturnValues::integer(
                s.read(frame, object)?.encode_utf16().count() as u64,
            ))
        })?,
        append: bridge.register_service(cpu, "_touchHLE_NSString_append", move |frame| {
            let mut s = state
                .try_borrow_mut()
                .map_err(|_| "reentrant NSString append")?;
            let object = s.method(frame, "stringByAppendingString:")?;
            let mut text = s.read(frame, object)?;
            let appended = frame.integer(2)?;
            text.push_str(&s.read(frame, appended)?);
            let result = s.create_autoreleased(frame, &text)?;
            Ok(ReturnValues::integer(result))
        })?,
    })
}

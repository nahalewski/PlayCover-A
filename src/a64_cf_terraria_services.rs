/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Explicit CF host services with one bounded guest identity/lifetime registry.
//! No ObjC isa, toll-free bridging, framework init or foreign-object success.
#[path = "a64_cf_array.rs"]
mod array;
#[path = "a64_cf_data.rs"]
mod data;
#[path = "a64_cf_text.rs"]
mod text;
#[path = "a64_cf_uuid.rs"]
mod uuid_value;
use super::{
    bridge::{GuestBridge, ReturnValues, ServiceFrame, ServiceId},
    cf, cf_dictionary,
    cf_number::SignedNumber,
    cf_number_services, A64Cpu,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
const ARENA_BYTES: usize = 64 * 1024;
const MAX_OBJECTS: u64 = ARENA_BYTES as u64 / 16;
const STRING_TYPE: u64 = cf::STRING_TYPE_ID;
const UUID_TYPE: u64 = 2;
const ARRAY_TYPE: u64 = 3;
const DATA_TYPE: u64 = 4;
// Emulator-owned opaque type ID, never Apple's private runtime numeric ID.
const NUMBER_TYPE: u64 = 5;
const DICTIONARY_TYPE: u64 = 6;

#[derive(Default, Clone, Copy)]
pub(super) struct KnownConstants {
    /// Only exact audited DATA symbols, never arbitrary caller addresses.
    pub type_array_callbacks: Option<u64>,
    pub null_allocator: Option<u64>,
}
#[derive(Clone, Copy)]
enum Kind {
    String(cf::Handle),
    Uuid(uuid_value::Value),
    Array {
        handle: array::Handle,
        owns_values: bool,
        cursor: usize,
    },
    Data(data::Handle),
    Number(SignedNumber),
    Dictionary(cf_dictionary::Handle),
}
struct Object {
    kind: Kind,
    refs: u64,
}
struct NoCallbacks;
impl array::ValueLifetime for NoCallbacks {
    fn retain(&mut self, _: u64) -> Result<(), String> {
        Err("Unexpected raw-array retain callback".into())
    }
    fn release(&mut self, _: u64) -> Result<(), String> {
        Err("Unexpected raw-array release callback".into())
    }
}
struct Store {
    base: u64,
    next: u64,
    objects: BTreeMap<u64, Object>,
    strings: cf::Strings,
    arrays: array::Arrays,
    data: data::DataObjects,
    dictionaries: cf_dictionary::Dictionaries,
}
impl Store {
    fn new(base: u64) -> Self {
        Self {
            base,
            next: 0,
            objects: BTreeMap::new(),
            strings: cf::Strings::default(),
            arrays: array::Arrays::default(),
            data: data::DataObjects::default(),
            dictionaries: cf_dictionary::Dictionaries::default(),
        }
    }
    fn available(&self) -> Result<(), String> {
        if self.next >= MAX_OBJECTS {
            Err("CF guest object arena exhausted".into())
        } else {
            Ok(())
        }
    }
    fn insert(&mut self, kind: Kind) -> Result<u64, String> {
        self.available()?;
        let address = self.base + self.next * 16;
        self.next += 1;
        self.objects.insert(address, Object { kind, refs: 1 });
        Ok(address)
    }
    fn object(&self, address: u64) -> Result<&Object, String> {
        self.objects
            .get(&address)
            .filter(|o| o.refs != 0)
            .ok_or_else(|| "CF object unknown, foreign or deallocating".into())
    }
    fn string(&self, address: u64) -> Result<cf::Handle, String> {
        match self.object(address)?.kind {
            Kind::String(h) => Ok(h),
            _ => Err("CF object is not an owned string".into()),
        }
    }
    fn number(&self, address: u64) -> Result<SignedNumber, String> {
        match self.object(address)?.kind {
            Kind::Number(value) => Ok(value),
            _ => Err("CF object is not an owned signed number".into()),
        }
    }
    fn dictionary(&self, address: u64) -> Result<cf_dictionary::Handle, String> {
        match self.object(address)?.kind {
            Kind::Dictionary(handle) => Ok(handle),
            _ => Err("CF object is not an owned null-callback dictionary".into()),
        }
    }
    fn create_dictionary(&mut self, capacity: i64) -> Result<u64, String> {
        self.available()?;
        let handle = self.dictionaries.create(capacity, 64)?;
        self.insert(Kind::Dictionary(handle))
    }
    fn create_string(&mut self, units: &[u16]) -> Result<u64, String> {
        self.available()?;
        let h = self
            .strings
            .create_utf16(units)
            .map_err(|e| format!("CF string: {e:?}"))?;
        self.insert(Kind::String(h))
    }
    fn create_utf8(&mut self, bytes: &[u8]) -> Result<u64, String> {
        self.available()?;
        let h = self
            .strings
            .create_utf8(bytes)
            .map_err(|e| format!("CF UTF8: {e:?}"))?;
        self.insert(Kind::String(h))
    }
    fn units(&self, address: u64) -> Result<Vec<u16>, String> {
        let h = self.string(address)?;
        let length = self
            .strings
            .length(h)
            .map_err(|e| format!("CF length: {e:?}"))?;
        (0..length)
            .map(|i| {
                self.strings
                    .character(h, i)
                    .map_err(|e| format!("CF character: {e:?}"))
            })
            .collect()
    }
    fn retain(&mut self, address: u64) -> Result<u64, String> {
        self.object(address)?;
        let o = self.objects.get_mut(&address).unwrap();
        o.refs = o.refs.checked_add(1).ok_or("CF retain count overflow")?;
        Ok(address)
    }
    fn type_id(&self, address: u64) -> Result<u64, String> {
        Ok(match self.object(address)?.kind {
            Kind::String(_) => STRING_TYPE,
            Kind::Uuid(_) => UUID_TYPE,
            Kind::Array { .. } => ARRAY_TYPE,
            Kind::Data(_) => DATA_TYPE,
            Kind::Number(_) => NUMBER_TYPE,
            Kind::Dictionary(_) => DICTIONARY_TYPE,
        })
    }
    fn release(&mut self, address: u64, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err("CF destruction nesting limit exceeded".into());
        }
        let object = self
            .objects
            .get_mut(&address)
            .ok_or("CF release unknown/foreign object")?;
        if object.refs > 1 {
            object.refs -= 1;
            return Ok(());
        }
        object.refs = 0;
        match object.kind {
            Kind::String(h) => self
                .strings
                .release(h)
                .map_err(|e| format!("CF string release: {e:?}"))?,
            Kind::Uuid(_) | Kind::Number(_) => {}
            Kind::Dictionary(handle) => self.dictionaries.destroy(handle)?,
            Kind::Data(h) => self
                .data
                .release(h, |_| Err("Guest heap free unavailable".into()))?,
            Kind::Array {
                handle,
                owns_values,
                ..
            } => {
                if owns_values {
                    loop {
                        let cursor = match self.objects[&address].kind {
                            Kind::Array { cursor, .. } => cursor,
                            _ => unreachable!(),
                        };
                        if cursor >= self.arrays.count(handle)? {
                            break;
                        }
                        let child = self.arrays.value(handle, cursor)?;
                        self.release(child, depth + 1)?;
                        if let Kind::Array { cursor, .. } =
                            &mut self.objects.get_mut(&address).unwrap().kind
                        {
                            *cursor += 1
                        }
                    }
                }
                self.arrays.release(handle, &mut NoCallbacks)?;
            }
        }
        self.objects.remove(&address);
        Ok(())
    }
    fn create_array(&mut self, capacity: i64, owns_values: bool) -> Result<u64, String> {
        self.available()?;
        let h = self.arrays.create(capacity, array::Callbacks::None)?;
        self.insert(Kind::Array {
            handle: h,
            owns_values,
            cursor: 0,
        })
    }
    fn append(&mut self, address: u64, value: u64) -> Result<(), String> {
        let (handle, owns_values) = match self.object(address)?.kind {
            Kind::Array {
                handle,
                owns_values,
                ..
            } => (handle, owns_values),
            _ => return Err("CF append target is not an owned array".into()),
        };
        if owns_values {
            self.retain(value)?;
        }
        if let Err(error) = self.arrays.append(handle, value, &mut NoCallbacks) {
            if owns_values {
                self.objects.get_mut(&value).unwrap().refs -= 1;
            }
            return Err(error);
        }
        Ok(())
    }
}
fn signed_count(value: u64, limit: usize) -> Result<usize, String> {
    if value > limit as u64 {
        Err("CF signed length negative or excessive".into())
    } else {
        Ok(value as usize)
    }
}
fn allocator(frame: &ServiceFrame<'_>) -> Result<(), String> {
    if frame.integer(0)? == 0 {
        Ok(())
    } else {
        Err("CF custom allocator unsupported".into())
    }
}

pub(super) fn register(
    bridge: &mut GuestBridge,
    cpu: &mut A64Cpu,
    arena: u64,
) -> Result<Vec<(&'static str, ServiceId)>, String> {
    register_with_constants(bridge, cpu, arena, KnownConstants::default())
}
pub(super) fn register_with_constants(
    bridge: &mut GuestBridge,
    cpu: &mut A64Cpu,
    arena: u64,
    constants: KnownConstants,
) -> Result<Vec<(&'static str, ServiceId)>, String> {
    if arena == 0 || arena & 4095 != 0 {
        return Err("CF arena must be nonzero/page aligned".into());
    }
    let end = arena
        .checked_add(ARENA_BYTES as u64)
        .ok_or("CF arena overflow")?;
    for address in arena..end {
        if cpu.mapped_permissions(address).is_some() {
            return Err("CF arena overlaps mapped guest memory".into());
        }
    }
    cpu.map_zeroed(arena, ARENA_BYTES, 3)?;
    let store = Rc::new(RefCell::new(Store::new(arena)));
    let mut exports = Vec::new();
    let state = store.clone();
    exports.push((
        "_CFDictionaryCreateMutable",
        bridge.register_service(cpu, "_CFDictionaryCreateMutable", move |frame| {
            allocator(frame)?;
            if frame.integer(2)? != 0 || frame.integer(3)? != 0 {
                return Err(
                    "CFDictionary CFType/custom callbacks unsupported; null callbacks required"
                        .into(),
                );
            }
            let address = state
                .try_borrow_mut()
                .map_err(|_| "reentrant CF dictionary creation")?
                .create_dictionary(frame.integer(1)? as i64)?;
            Ok(ReturnValues::integer(address))
        })?,
    ));
    for name in [
        "_CFDictionaryGetValue",
        "_CFDictionaryGetCount",
        "_CFDictionarySetValue",
        "_CFDictionaryRemoveValue",
    ] {
        let state = store.clone();
        exports.push((
            name,
            bridge.register_service(cpu, name, move |frame| {
                let mut store = state
                    .try_borrow_mut()
                    .map_err(|_| "reentrant CF dictionary operation")?;
                let handle = store.dictionary(frame.integer(0)?)?;
                let value = match name {
                    "_CFDictionaryGetValue" => store
                        .dictionaries
                        .get(handle, frame.integer(1)?)?
                        .unwrap_or(0),
                    "_CFDictionaryGetCount" => store.dictionaries.count(handle)? as u64,
                    "_CFDictionarySetValue" => {
                        store
                            .dictionaries
                            .set(handle, frame.integer(1)?, frame.integer(2)?)?;
                        0
                    }
                    "_CFDictionaryRemoveValue" => {
                        store.dictionaries.remove(handle, frame.integer(1)?)?;
                        0
                    }
                    _ => unreachable!(),
                };
                Ok(ReturnValues::integer(value))
            })?,
        ));
    }
    let state = store.clone();
    exports.push((
        "_CFNumberCreate",
        bridge.register_service(cpu, "_CFNumberCreate", move |frame| {
            allocator(frame)?;
            let kind = frame.integer(1)?;
            let size = match kind {
                1 | 7 => 1,
                2 | 8 => 2,
                3 | 9 => 4,
                4 | 10 | 11 | 14 | 15 => 8,
                _ => return Err("CFNumberCreate supports signed LP64 integer input only".into()),
            };
            let bytes = frame.read(frame.integer(2)?, size)?;
            let value = match size {
                1 => i64::from(i8::from_le_bytes([bytes[0]])),
                2 => i64::from(i16::from_le_bytes(bytes.try_into().unwrap())),
                4 => i64::from(i32::from_le_bytes(bytes.try_into().unwrap())),
                8 => i64::from_le_bytes(bytes.try_into().unwrap()),
                _ => unreachable!(),
            };
            let address = state
                .try_borrow_mut()
                .map_err(|_| "reentrant CF number creation")?
                .insert(Kind::Number(SignedNumber(value)))?;
            Ok(ReturnValues::integer(address))
        })?,
    ));
    // Only register conversion with a real producer and the same authoritative
    // object/type/lifetime owner as CFRetain, CFRelease, and owning arrays.
    let state = store.clone();
    exports.push((
        "_CFNumberGetValue",
        cf_number_services::install(bridge, cpu, move |address| {
            state
                .try_borrow()
                .map_err(|_| "reentrant CF number lookup")?
                .number(address)
        })?,
    ));
    for name in [
        "_CFRetain",
        "_CFRelease",
        "_CFGetTypeID",
        "_CFGetRetainCount",
        "_CFStringGetLength",
        "_CFDataGetLength",
        "_CFDataGetBytePtr",
        "_CFUUIDGetUUIDBytes",
    ] {
        let state = store.clone();
        exports.push((
            name,
            bridge.register_service(cpu, name, move |f| {
                let address = f.integer(0)?;
                let mut s = state.borrow_mut();
                let value = match name {
                    "_CFRetain" => s.retain(address)?,
                    "_CFRelease" => {
                        s.release(address, 0)?;
                        0
                    }
                    "_CFGetTypeID" => s.type_id(address)?,
                    "_CFGetRetainCount" => s.object(address)?.refs,
                    "_CFStringGetLength" => s
                        .strings
                        .length(s.string(address)?)
                        .map_err(|e| format!("CF length: {e:?}"))?
                        as u64,
                    "_CFDataGetLength" | "_CFDataGetBytePtr" => {
                        let h = match s.object(address)?.kind {
                            Kind::Data(h) => h,
                            _ => return Err("CF object is not owned data".into()),
                        };
                        if name == "_CFDataGetLength" {
                            s.data.length(h)? as u64
                        } else {
                            s.data.byte_pointer(h)?
                        }
                    }
                    "_CFUUIDGetUUIDBytes" => {
                        let uuid = match s.object(address)?.kind {
                            Kind::Uuid(v) => v,
                            _ => return Err("CF object is not an owned UUID".into()),
                        };
                        return Ok(ReturnValues {
                            integers: uuid.registers(),
                            vectors: [[0; 2]; 4],
                        });
                    }
                    _ => unreachable!(),
                };
                Ok(ReturnValues::integer(value))
            })?,
        ));
    }
    for (name, value) in [
        ("_CFStringGetTypeID", STRING_TYPE),
        ("_CFUUIDGetTypeID", UUID_TYPE),
        ("_CFArrayGetTypeID", ARRAY_TYPE),
        ("_CFDataGetTypeID", DATA_TYPE),
        ("_CFNumberGetTypeID", NUMBER_TYPE),
        ("_CFDictionaryGetTypeID", DICTIONARY_TYPE),
    ] {
        exports.push((
            name,
            bridge.register_service(cpu, name, move |_| Ok(ReturnValues::integer(value)))?,
        ));
    }
    for name in ["_CFStringCreateWithCharacters", "_CFStringCreateWithBytes"] {
        let state = store.clone();
        exports.push((
            name,
            bridge.register_service(cpu, name, move |f| {
                allocator(f)?;
                let count = signed_count(
                    f.integer(2)?,
                    if name == "_CFStringCreateWithCharacters" {
                        512 * 1024
                    } else {
                        1024 * 1024
                    },
                )?;
                let length = if name == "_CFStringCreateWithCharacters" {
                    count * 2
                } else {
                    count
                };
                let bytes = if length == 0 {
                    Vec::new()
                } else {
                    f.read(f.integer(1)?, length)?
                };
                let address = if name == "_CFStringCreateWithCharacters" {
                    state
                        .borrow_mut()
                        .create_string(&text::characters_from_le_bytes(&bytes)?)?
                } else {
                    if f.integer(3)? != text::UTF8 || f.integer(4)? != 0 {
                        return Err(
                            "CF byte creation supports UTF8 without external representation only"
                                .into(),
                        );
                    }
                    state.borrow_mut().create_utf8(&bytes)?
                };
                Ok(ReturnValues::integer(address))
            })?,
        ));
    }
    let state = store.clone();
    exports.push((
        "_CFStringGetCString",
        bridge.register_service(cpu, "_CFStringGetCString", move |f| {
            let units = state.borrow().units(f.integer(0)?)?;
            let capacity = signed_count(f.integer(2)?, 1024 * 1024)?;
            let Some(bytes) = text::c_string(&units, f.integer(3)?, capacity)? else {
                return Ok(ReturnValues::integer(0));
            };
            f.write(f.integer(1)?, &bytes)?;
            Ok(ReturnValues::integer(1))
        })?,
    ));
    exports.push((
        "_CFStringGetMaximumSizeForEncoding",
        bridge.register_service(cpu, "_CFStringGetMaximumSizeForEncoding", |f| {
            Ok(ReturnValues::integer(
                text::maximum_size(f.integer(0)? as i64, f.integer(1)?)? as u64,
            ))
        })?,
    ));
    let state = store.clone();
    exports.push((
        "_CFStringGetBytes",
        bridge.register_service(cpu, "_CFStringGetBytes", move |f| {
            if f.integer(3)? != text::UTF8 || f.integer(4)? != 0 || f.integer(5)? != 0 {
                return Err("CF GetBytes encoding/loss/external flags unsupported".into());
            }
            let address = f.integer(0)?;
            let start = signed_count(f.integer(1)?, 1024 * 1024)?;
            let count = signed_count(f.integer(2)?, 1024 * 1024)?;
            let buffer = f.integer(6)?;
            let capacity = if buffer == 0 {
                None
            } else {
                Some(signed_count(f.integer(7)?, 1024 * 1024)?)
            };
            let used = f.stack_u64(0)?;
            let s = state.borrow();
            let (converted, bytes) = s
                .strings
                .utf8_bytes(s.string(address)?, start, count, capacity)
                .map_err(|e| format!("CF UTF8 output: {e:?}"))?;
            if buffer != 0 && !bytes.is_empty() {
                f.write(buffer, &bytes)?
            }
            if used != 0 {
                f.write(used, &(bytes.len() as u64).to_le_bytes())?
            }
            Ok(ReturnValues::integer(converted as u64))
        })?,
    ));
    for name in [
        "_CFUUIDCreate",
        "_CFUUIDCreateFromUUIDBytes",
        "_CFUUIDCreateString",
    ] {
        let state = store.clone();
        exports.push((
            name,
            bridge.register_service(cpu, name, move |f| {
                allocator(f)?;
                let mut s = state.borrow_mut();
                s.available()?;
                let address = if name == "_CFUUIDCreateString" {
                    let value = match s.object(f.integer(1)?)?.kind {
                        Kind::Uuid(v) => v,
                        _ => return Err("CF UUID string requires an owned UUID".into()),
                    };
                    s.create_utf8(value.string().as_bytes())?
                } else {
                    let value = if name == "_CFUUIDCreateFromUUIDBytes" {
                        uuid_value::Value::from_registers([f.integer(1)?, f.integer(2)?])
                    } else {
                        uuid_value::Value(*uuid::Uuid::new_v4().as_bytes())
                    };
                    s.insert(Kind::Uuid(value))?
                };
                Ok(ReturnValues::integer(address))
            })?,
        ));
    }
    exports.push((
        "_CFAbsoluteTimeGetCurrent",
        bridge.register_service(cpu, "_CFAbsoluteTimeGetCurrent", |_| {
            Ok(ReturnValues {
                integers: [0; 2],
                vectors: [
                    [
                        uuid_value::absolute_time(std::time::SystemTime::now()).to_bits(),
                        0,
                    ],
                    [0; 2],
                    [0; 2],
                    [0; 2],
                ],
            })
        })?,
    ));
    let state = store.clone();
    exports.push((
        "_CFArrayCreateMutable",
        bridge.register_service(cpu, "_CFArrayCreateMutable", move |f| {
            allocator(f)?;
            let callbacks = f.integer(2)?;
            let owns = if callbacks == 0 {
                false
            } else if constants.type_array_callbacks == Some(callbacks) {
                true
            } else {
                return Err("CF array custom/unknown callbacks unsupported".into());
            };
            let address = state
                .borrow_mut()
                .create_array(f.integer(1)? as i64, owns)?;
            Ok(ReturnValues::integer(address))
        })?,
    ));
    let state = store.clone();
    exports.push((
        "_CFArrayAppendValue",
        bridge.register_service(cpu, "_CFArrayAppendValue", move |f| {
            state.borrow_mut().append(f.integer(0)?, f.integer(1)?)?;
            Ok(ReturnValues::integer(0))
        })?,
    ));
    let state = store.clone();
    exports.push(("_CFDataCreateWithBytesNoCopy",bridge.register_service(cpu,"_CFDataCreateWithBytesNoCopy",move|f|{
        allocator(f)?;let deallocator=f.integer(3)?;if constants.null_allocator!=Some(deallocator)||deallocator==0{return Err("CFData requires positively identified kCFAllocatorNull; guest heap/custom free unavailable".into())}
        let address=f.integer(1)?;let length=signed_count(f.integer(2)?,1024*1024)?;let mut s=state.borrow_mut();s.available()?;
        let handle=s.data.create_no_copy(address,length as i64,data::Deallocator::NeverFree,|p,n|{f.read(p,n).map(|_|())})?;
        Ok(ReturnValues::integer(s.insert(Kind::Data(handle))?))
    })?));
    Ok(exports)
}

#[cfg(test)]
mod tests {
    #[test]
    fn real_dictionary_producer_pointer_lookup_mutation_and_shared_lifetime() {
        let mut cpu = super::A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        let mut bridge = super::GuestBridge::map(&mut cpu, 0x20000).unwrap();
        let exports = super::register(&mut bridge, &mut cpu, 0x50000).unwrap();
        for args in [
            vec![0, 0, 1, 0],
            vec![0, 0, 0, 1],
            vec![0, u64::MAX, 0, 0],
            vec![1, 0, 0, 0],
        ] {
            assert!(call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFDictionaryCreateMutable",
                args
            )
            .is_err());
        }
        let dictionary = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFDictionaryCreateMutable",
            vec![0, 0, 0, 0],
        )
        .unwrap()
        .integers[0];
        assert_eq!(dictionary, 0x50000);
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFGetTypeID",
                vec![dictionary]
            )
            .unwrap()
            .integers[0],
            super::DICTIONARY_TYPE
        );
        for (key, value) in [(7, 42), (8, 0), (7, 43)] {
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFDictionarySetValue",
                vec![dictionary, key, value],
            )
            .unwrap();
        }
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFDictionaryGetCount",
                vec![dictionary]
            )
            .unwrap()
            .integers[0],
            2
        );
        for (key, value) in [(7, 43), (8, 0), (9, 0)] {
            assert_eq!(
                call(
                    &mut cpu,
                    &mut bridge,
                    &exports,
                    "_CFDictionaryGetValue",
                    vec![dictionary, key]
                )
                .unwrap()
                .integers[0],
                value
            );
        }
        call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFDictionaryRemoveValue",
            vec![dictionary, 7],
        )
        .unwrap();
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFDictionaryGetCount",
                vec![dictionary]
            )
            .unwrap()
            .integers[0],
            1
        );
        cpu.write_bytes(0x40000, &42i64.to_le_bytes());
        let number = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFNumberCreate",
            vec![0, 4, 0x40000],
        )
        .unwrap()
        .integers[0];
        call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFDictionarySetValue",
            vec![dictionary, 7, number],
        )
        .unwrap();
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFGetRetainCount",
                vec![number]
            )
            .unwrap()
            .integers[0],
            1
        );
        call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFRetain",
            vec![dictionary],
        )
        .unwrap();
        call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFRelease",
            vec![dictionary],
        )
        .unwrap();
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFDictionaryGetValue",
                vec![dictionary, 7]
            )
            .unwrap()
            .integers[0],
            number
        );
        assert!(call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFDictionaryGetValue",
            vec![number, 7]
        )
        .is_err());
        assert!(call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFDictionaryGetValue",
            vec![0xdead0000, 7]
        )
        .is_err());
        call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFRelease",
            vec![dictionary],
        )
        .unwrap();
        assert!(call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFDictionaryGetValue",
            vec![dictionary, 7]
        )
        .is_err());
        // Null callbacks neither retain nor release raw key/value pointers.
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFGetRetainCount",
                vec![number]
            )
            .unwrap()
            .integers[0],
            1
        );
        call(&mut cpu, &mut bridge, &exports, "_CFRelease", vec![number]).unwrap();
    }
    #[test]
    fn owned_number_uses_shared_producer_type_retain_and_final_release() {
        let mut cpu = super::A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        cpu.write_bytes(0x40000, &(-42i64).to_le_bytes());
        let mut bridge = super::GuestBridge::map(&mut cpu, 0x20000).unwrap();
        let exports = super::register(&mut bridge, &mut cpu, 0x50000).unwrap();
        // Invalid reads and unsupported allocator/type do not consume identity.
        for args in [
            vec![0, 4, 0x40ffc],
            vec![1, 4, 0x40000],
            vec![0, 5, 0x40000],
        ] {
            assert!(call(&mut cpu, &mut bridge, &exports, "_CFNumberCreate", args).is_err());
        }
        let number = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFNumberCreate",
            vec![0, 10, 0x40000],
        )
        .unwrap()
        .integers[0];
        assert_eq!(number, 0x50000);
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFGetTypeID",
                vec![number]
            )
            .unwrap()
            .integers[0],
            super::NUMBER_TYPE
        );
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFNumberGetTypeID",
                vec![]
            )
            .unwrap()
            .integers[0],
            super::NUMBER_TYPE
        );
        call(&mut cpu, &mut bridge, &exports, "_CFRetain", vec![number]).unwrap();
        call(&mut cpu, &mut bridge, &exports, "_CFRelease", vec![number]).unwrap();
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFGetRetainCount",
                vec![number]
            )
            .unwrap()
            .integers[0],
            1
        );
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFNumberGetValue",
                vec![number, 14, 0x40100]
            )
            .unwrap()
            .integers[0],
            1
        );
        assert_eq!(cpu.read_u64(0x40100), Some((-42i64) as u64));
        assert!(call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFStringGetLength",
            vec![number]
        )
        .is_err());
        call(&mut cpu, &mut bridge, &exports, "_CFRelease", vec![number]).unwrap();
        assert!(call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFNumberGetValue",
            vec![number, 14, 0x40100]
        )
        .is_err());
        assert!(call(&mut cpu, &mut bridge, &exports, "_CFRetain", vec![number]).is_err());
        assert_eq!(cpu.read_u64(0x40100), Some((-42i64) as u64));
        cpu.write_bytes(0x40000, &[0x80]);
        let second = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFNumberCreate",
            vec![0, 7, 0x40000],
        )
        .unwrap()
        .integers[0];
        assert_ne!(second, number);
        call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFNumberGetValue",
            vec![second, 4, 0x40100],
        )
        .unwrap();
        assert_eq!(cpu.read_u64(0x40100), Some((-128i64) as u64));
    }
    #[test]
    fn owning_arrays_share_number_refs_and_exhausted_arena_never_reuses_identity() {
        let mut store = super::Store::new(0x100000);
        let number = store
            .insert(super::Kind::Number(super::SignedNumber(42)))
            .unwrap();
        let array = store.create_array(0, true).unwrap();
        store.append(array, number).unwrap();
        store.append(array, number).unwrap();
        assert_eq!(store.object(number).unwrap().refs, 3);
        store.release(number, 0).unwrap();
        store.release(array, 0).unwrap();
        assert!(store.number(number).is_err());
        assert!(store.object(array).is_err());
        store.next = super::MAX_OBJECTS - 1;
        let last = store
            .insert(super::Kind::Number(super::SignedNumber(1)))
            .unwrap();
        store.release(last, 0).unwrap();
        assert!(store
            .insert(super::Kind::Number(super::SignedNumber(2)))
            .is_err());
    }
    use super::super::bridge::GuestCall;
    use super::*;
    fn call(
        cpu: &mut A64Cpu,
        bridge: &mut GuestBridge,
        exports: &[(&str, ServiceId)],
        name: &str,
        args: Vec<u64>,
    ) -> Result<ReturnValues, String> {
        bridge.call(
            cpu,
            &GuestCall {
                entry: exports
                    .iter()
                    .find(|(n, _)| *n == name)
                    .unwrap()
                    .1
                    .guest_address(),
                integers: args,
                ..Default::default()
            },
            200,
        )
    }
    #[test]
    fn unified_ownership_array_and_uuid_roundtrip() {
        let mut cpu = A64Cpu::new_sparse();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        cpu.write_bytes(0x40000, &[0x41, 0, 0x3d, 0xd8, 0, 0xde]);
        let exports = register_with_constants(
            &mut bridge,
            &mut cpu,
            0x50000,
            KnownConstants {
                type_array_callbacks: Some(0xdead0),
                null_allocator: Some(0xdead8),
            },
        )
        .unwrap();
        let string = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFStringCreateWithCharacters",
            vec![0, 0x40000, 3],
        )
        .unwrap()
        .integers[0];
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFStringGetLength",
                vec![string]
            )
            .unwrap()
            .integers[0],
            3
        );
        let array = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFArrayCreateMutable",
            vec![0, 0, 0xdead0],
        )
        .unwrap()
        .integers[0];
        call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFArrayAppendValue",
            vec![array, string],
        )
        .unwrap();
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFGetRetainCount",
                vec![string]
            )
            .unwrap()
            .integers[0],
            2
        );
        call(&mut cpu, &mut bridge, &exports, "_CFRelease", vec![string]).unwrap();
        call(&mut cpu, &mut bridge, &exports, "_CFRelease", vec![array]).unwrap();
        assert!(call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFStringGetLength",
            vec![string]
        )
        .is_err());
        let bytes = [0x0706050403020100, 0x0f0e0d0c0b0a0908];
        let uuid = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFUUIDCreateFromUUIDBytes",
            vec![0, bytes[0], bytes[1]],
        )
        .unwrap()
        .integers[0];
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFUUIDGetUUIDBytes",
                vec![uuid]
            )
            .unwrap()
            .integers,
            bytes
        );
        let text = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFUUIDCreateString",
            vec![0, uuid],
        )
        .unwrap()
        .integers[0];
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFStringGetCString",
                vec![text, 0x40100, 37, text::UTF8]
            )
            .unwrap()
            .integers[0],
            1
        );
        let mut output = [0; 37];
        cpu.read_guest_into(0x40100, &mut output).unwrap();
        assert_eq!(&output, b"00010203-0405-0607-0809-0A0B0C0D0E0F\0");
    }
    #[test]
    fn genuine_no_copy_data_and_unsupported_edges() {
        let mut cpu = A64Cpu::new_sparse();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        let exports = register_with_constants(
            &mut bridge,
            &mut cpu,
            0x50000,
            KnownConstants {
                null_allocator: Some(0xdead8),
                ..Default::default()
            },
        )
        .unwrap();
        let data = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFDataCreateWithBytesNoCopy",
            vec![0, 0x40000, 8, 0xdead8],
        )
        .unwrap()
        .integers[0];
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFDataGetBytePtr",
                vec![data]
            )
            .unwrap()
            .integers[0],
            0x40000
        );
        assert_eq!(
            call(
                &mut cpu,
                &mut bridge,
                &exports,
                "_CFDataGetLength",
                vec![data]
            )
            .unwrap()
            .integers[0],
            8
        );
        cpu.write_bytes(0x40000, &42u64.to_le_bytes());
        assert_eq!(cpu.read_u64(0x40000), Some(42));
        call(&mut cpu, &mut bridge, &exports, "_CFRelease", vec![data]).unwrap();
        assert_eq!(cpu.read_u64(0x40000), Some(42));
        assert!(call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFDataGetLength",
            vec![data]
        )
        .is_err());
        assert!(call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFDataCreateWithBytesNoCopy",
            vec![0, 0x40000, 8, 0]
        )
        .unwrap_err()
        .contains("kCFAllocatorNull"));
        assert!(call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFArrayCreateMutable",
            vec![0, 0, 42]
        )
        .is_err());
        let raw = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFArrayCreateMutable",
            vec![0, 0, 0],
        )
        .unwrap()
        .integers[0];
        call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFArrayAppendValue",
            vec![raw, u64::MAX],
        )
        .unwrap();
        call(&mut cpu, &mut bridge, &exports, "_CFRelease", vec![raw]).unwrap();
        let time = call(
            &mut cpu,
            &mut bridge,
            &exports,
            "_CFAbsoluteTimeGetCurrent",
            vec![],
        )
        .unwrap();
        assert!(
            (f64::from_bits(time.vectors[0][0])
                - uuid_value::absolute_time(std::time::SystemTime::now()))
            .abs()
                < 2.0
        );
    }
}

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Opt-in CF services for owned opaque objects, not framework initialization.
//! The arena does not contain Objective-C objects; toll-free bridging is absent.
use super::{
    bridge::{GuestBridge, ReturnValues, ServiceId},
    cf::{Handle, Strings, STRING_TYPE_ID},
    A64Cpu,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
const UTF8: u64 = 0x08000100;
const UTF16_BE: u64 = 0x10000100;
const UTF16_LE: u64 = 0x14000100;
const ARENA_SIZE: usize = 64 * 1024;
const SLOT_SIZE: u64 = 16;

struct Store {
    strings: Strings,
    objects: BTreeMap<u64, Handle>,
    base: u64,
    next: u64,
}
impl Store {
    fn handle(&self, address: u64) -> Result<Handle, String> {
        self.objects.get(&address).copied().ok_or_else(|| "CF object is not an owned registered string; external/Objective-C objects unsupported".into())
    }
    fn create(&mut self, bytes: &[u8], encoding: u64) -> Result<u64, String> {
        if self.next >= ARENA_SIZE as u64 / SLOT_SIZE {
            return Err("CF object arena exhausted".into());
        }
        let handle = match encoding {
            UTF8 => self.strings.create_utf8(bytes),
            UTF16_BE | UTF16_LE => {
                if bytes.len() % 2 != 0 {
                    return Err("CF UTF16 byte length is odd".into());
                }
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|v| {
                        if encoding == UTF16_BE {
                            u16::from_be_bytes([v[0], v[1]])
                        } else {
                            u16::from_le_bytes([v[0], v[1]])
                        }
                    })
                    .collect();
                self.strings.create_utf16(&units)
            }
            _ => return Err("CFString encoding unsupported".into()),
        }
        .map_err(|e| format!("CFString creation: {e:?}"))?;
        let address = self.base + self.next * SLOT_SIZE;
        self.next += 1; // Never reuse a released guest identity.
        self.objects.insert(address, handle);
        Ok(address)
    }
}

/// Caller chooses non-overlapping guest arena. Returned symbol addresses are
/// explicit service exports only; the normal dyld/runtime gate is unaffected.
pub(super) fn register(
    bridge: &mut GuestBridge,
    cpu: &mut A64Cpu,
    arena: u64,
) -> Result<Vec<(&'static str, ServiceId)>, String> {
    if arena == 0 || arena & 4095 != 0 {
        return Err("CF arena must be nonzero/page aligned".into());
    }
    let end = arena
        .checked_add(ARENA_SIZE as u64)
        .ok_or("CF arena overflow")?;
    for address in arena..end {
        if cpu.mapped_permissions(address).is_some() {
            return Err("CF arena overlaps guest memory".into());
        }
    }
    cpu.map_zeroed(arena, ARENA_SIZE, 3)?;
    let store = Rc::new(RefCell::new(Store {
        strings: Strings::default(),
        objects: BTreeMap::new(),
        base: arena,
        next: 0,
    }));
    let mut exports = Vec::new();
    let state = store.clone();
    exports.push((
        "_CFStringCreateWithBytes",
        bridge.register_service(cpu, "_CFStringCreateWithBytes", move |f| {
            if f.integer(0)? != 0 || f.integer(4)? != 0 {
                return Err("CF custom allocator/external representation unsupported".into());
            }
            let len = f.integer(2)?;
            if len > 1024 * 1024 {
                return Err("CF byte input exceeds bounded service length".into());
            }
            let bytes = if len == 0 {
                Vec::new()
            } else {
                f.read(f.integer(1)?, len as usize)?
            };
            let address = state.borrow_mut().create(&bytes, f.integer(3)?)?;
            Ok(ReturnValues::integer(address))
        })?,
    ));
    for name in [
        "_CFStringGetLength",
        "_CFGetTypeID",
        "_CFGetRetainCount",
        "_CFRetain",
        "_CFRelease",
    ] {
        let state = store.clone();
        exports.push((
            name,
            bridge.register_service(cpu, name, move |f| {
                let address = f.integer(0)?;
                let mut state = state.borrow_mut();
                let h = state.handle(address)?;
                let value = match name {
                    "_CFStringGetLength" => state.strings.length(h).map(|n| n as u64),
                    "_CFGetTypeID" => state.strings.type_id(h),
                    "_CFGetRetainCount" => state.strings.retain_count(h),
                    "_CFRetain" => state.strings.retain(h).map(|_| address),
                    "_CFRelease" => {
                        state
                            .strings
                            .release(h)
                            .map_err(|e| format!("CF release: {e:?}"))?;
                        if state.strings.retain_count(h).is_err() {
                            state.objects.remove(&address);
                        }
                        Ok(0) // Void return; no CF success value fabricated.
                    }
                    _ => unreachable!(),
                }
                .map_err(|e| format!("CF operation: {e:?}"))?;
                Ok(ReturnValues::integer(value))
            })?,
        ));
    }
    exports.push((
        "_CFStringGetTypeID",
        bridge.register_service(cpu, "_CFStringGetTypeID", |_| {
            Ok(ReturnValues::integer(STRING_TYPE_ID))
        })?,
    ));
    let state = store.clone();
    exports.push((
        "_CFStringGetBytes",
        bridge.register_service(cpu, "_CFStringGetBytes", move |f| {
            if f.integer(3)? != UTF8 || f.integer(4)? != 0 || f.integer(5)? != 0 {
                return Err(
                    "CFStringGetBytes supports UTF8 without lossByte/external representation only"
                        .into(),
                );
            }
            let buffer = f.integer(6)?;
            let capacity = f.integer(7)?;
            if buffer != 0 && capacity > 1024 * 1024 {
                return Err("CF output capacity exceeds service limit".into());
            }
            let used_out = f.stack_u64(0)?; // Ninth ABI argument, after CFRange x1/x2.
            let state = state.borrow();
            let h = state.handle(f.integer(0)?)?;
            let start = usize::try_from(f.integer(1)?).map_err(|_| "CF range start overflow")?;
            let count = usize::try_from(f.integer(2)?).map_err(|_| "CF range length overflow")?;
            let (consumed, bytes) = state
                .strings
                .utf8_bytes(
                    h,
                    start,
                    count,
                    if buffer == 0 {
                        None
                    } else {
                        Some(capacity as usize)
                    },
                )
                .map_err(|e| format!("CF UTF8 conversion: {e:?}"))?;
            if buffer != 0 && !bytes.is_empty() {
                f.write(buffer, &bytes)?;
            }
            if used_out != 0 {
                f.write(used_out, &(bytes.len() as u64).to_le_bytes())?;
            }
            Ok(ReturnValues::integer(consumed as u64))
        })?,
    ));
    Ok(exports)
}

#[cfg(test)]
mod tests {
    use super::super::bridge::GuestCall;
    use super::*;
    #[test]
    fn explicit_guest_identity_and_encodings() {
        let mut s = Store {
            strings: Strings::default(),
            objects: BTreeMap::new(),
            base: 0x8000,
            next: 0,
        };
        let a = s.create(b"hi", UTF8).unwrap();
        let b = s.create(&[0xd8, 0x3d, 0xde, 0x00], UTF16_BE).unwrap();
        assert_eq!((a, b), (0x8000, 0x8010));
        assert_eq!(s.strings.length(s.handle(b).unwrap()), Ok(2));
        assert!(s.handle(0xfeed).is_err());
        assert!(s.create(&[0], UTF16_LE).is_err());
        assert!(s.create(b"", 0).is_err());
        s.next = ARENA_SIZE as u64 / SLOT_SIZE;
        assert!(s.create(b"x", UTF8).is_err());
    }
    #[test]
    fn real_guest_cf_calls_and_ninth_argument_writeback() {
        let mut cpu = A64Cpu::new_sparse();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        cpu.write_bytes(0x40000, "A😀".as_bytes());
        let exports = register(&mut bridge, &mut cpu, 0x50000).unwrap();
        let entry = |name| {
            exports
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap()
                .1
                .guest_address()
        };
        let h = bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: entry("_CFStringCreateWithBytes"),
                    integers: vec![0, 0x40000, 5, UTF8, 0],
                    ..Default::default()
                },
                100,
            )
            .unwrap()
            .integers[0];
        assert_eq!(h, 0x50000);
        assert_eq!(
            bridge
                .call(
                    &mut cpu,
                    &GuestCall {
                        entry: entry("_CFStringGetLength"),
                        integers: vec![h],
                        ..Default::default()
                    },
                    100
                )
                .unwrap()
                .integers[0],
            3
        );
        let converted = bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: entry("_CFStringGetBytes"),
                    integers: vec![h, 0, 3, UTF8, 0, 0, 0x40100, 5],
                    stack_arguments: 0x40200u64.to_le_bytes().to_vec(),
                    ..Default::default()
                },
                100,
            )
            .unwrap()
            .integers[0];
        assert_eq!(converted, 3);
        assert_eq!(cpu.read_u64(0x40200), Some(5));
        let mut bytes = [0; 5];
        cpu.read_guest_into(0x40100, &mut bytes).unwrap();
        assert_eq!(&bytes, "A😀".as_bytes());
        bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: entry("_CFRelease"),
                    integers: vec![h],
                    ..Default::default()
                },
                100,
            )
            .unwrap();
        assert!(bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: entry("_CFStringGetLength"),
                    integers: vec![h],
                    ..Default::default()
                },
                100
            )
            .unwrap_err()
            .contains("not an owned"));
        assert!(bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: entry("_CFStringCreateWithBytes"),
                    integers: vec![1, 0x40000, 5, UTF8, 0],
                    ..Default::default()
                },
                100
            )
            .unwrap_err()
            .contains("allocator"));
    }
}

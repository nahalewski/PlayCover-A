/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Owned Security.framework entry points that would otherwise need securityd.
//!
//! - `SecRandomCopyBytes` returns real host entropy from `/dev/urandom`.
//! - `SecItem*` (keychain): interim, honest "no keychain". Persisting items
//!   needs reading and creating CF dictionaries and data in whichever CF the
//!   caller uses (genuine Foundation objects, or the owned CF route). That is
//!   planned and not done yet. Until then, nothing can be stored: `Add` and
//!   `Update` fail with errSecNotAvailable, and `CopyMatching` and `Delete`
//!   report errSecItemNotFound. Both are true for a keychain that holds no
//!   items. Nothing reports a fake success.
use super::{Arena, Family};
use crate::a64::bridge::{ReturnValues, ServiceFrame};
use std::io::Read;

pub(super) const PROVIDER: &str = "/System/Library/Frameworks/Security.framework/Security";

const SYMBOLS: &[&str] = &[
    "_SecRandomCopyBytes",
    "_SecItemAdd",
    "_SecItemCopyMatching",
    "_SecItemUpdate",
    "_SecItemDelete",
];
const ERR_SEC_NOT_AVAILABLE: i32 = -25291;
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const ERR_SEC_PARAM: i32 = -50;
/// Upper bound for one SecRandomCopyBytes call (bulk budget applies too).
const MAX_RANDOM: u64 = 1024 * 1024;

#[derive(Default)]
pub(in crate::a64) struct Security {
    warned_keychain: bool,
}

fn status(code: i32) -> ReturnValues {
    ReturnValues::integer(code as u32 as u64)
}

impl Family for Security {
    fn name(&self) -> &'static str {
        "security"
    }
    fn provider(&self) -> &'static str {
        PROVIDER
    }
    fn symbols(&self) -> &'static [&'static str] {
        SYMBOLS
    }
    fn call(
        &mut self,
        index: usize,
        frame: &mut ServiceFrame<'_>,
        _arena: &mut Arena,
    ) -> Result<ReturnValues, String> {
        match index {
            0 => {
                let count = frame.integer(1)?;
                let out = frame.integer(2)?;
                if count == 0 {
                    return Ok(status(0));
                }
                if out == 0 || count > MAX_RANDOM {
                    return Ok(status(ERR_SEC_PARAM));
                }
                let mut bytes = vec![0u8; count as usize];
                let filled = std::fs::File::open("/dev/urandom")
                    .and_then(|mut source| source.read_exact(&mut bytes));
                if filled.is_err() {
                    // No host entropy: fail (SecRandomCopyBytes returns -1),
                    // never hand out predictable bytes.
                    return Ok(status(-1));
                }
                frame.write_bulk(out, &bytes)?;
                Ok(status(0))
            }
            1..=4 => {
                if !self.warned_keychain {
                    self.warned_keychain = true;
                    log!("[a64] Security: keychain persistence not implemented yet; SecItem* report an empty, unavailable keychain");
                }
                // CopyMatching's result out-parameter is cleared, as on a
                // failed real lookup.
                if index == 2 {
                    let result = frame.integer(1)?;
                    if result != 0 {
                        frame.write(result, &0u64.to_le_bytes())?;
                    }
                }
                Ok(status(match index {
                    1 | 3 => ERR_SEC_NOT_AVAILABLE,
                    _ => ERR_SEC_ITEM_NOT_FOUND,
                }))
            }
            _ => Err("Security dispatch index invalid".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::install;
    use super::*;
    use crate::a64::bridge::{GuestBridge, GuestCall};
    use crate::a64::A64Cpu;

    #[test]
    fn random_bytes_are_host_entropy_and_keychain_is_honestly_empty() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x80000, 0x2000, 3).unwrap();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        let frameworks =
            install(&mut cpu, &mut bridge, 0x200000, vec![Box::new(Security::default())]).unwrap();
        let entry = |name: &str| {
            frameworks
                .bindings
                .iter()
                .find(|b| b.symbol == name)
                .unwrap()
                .address
        };
        let result = 0x81000;
        cpu.write_bytes(result, &0xdeadu64.to_le_bytes());
        let mut call = |name: &str, integers: Vec<u64>| {
            bridge
                .call(
                    &mut cpu,
                    &GuestCall {
                        entry: entry(name),
                        integers,
                        ..Default::default()
                    },
                    100,
                )
                .unwrap()
                .integers[0] as u32 as i32
        };
        assert_eq!(call("_SecRandomCopyBytes", vec![0, 64, 0x80000]), 0);
        assert_eq!(call("_SecRandomCopyBytes", vec![0, 64, 0x80040]), 0);
        assert_eq!(call("_SecRandomCopyBytes", vec![0, 64, 0]), ERR_SEC_PARAM);
        assert_eq!(call("_SecItemCopyMatching", vec![0x1, result]), ERR_SEC_ITEM_NOT_FOUND);
        assert_eq!(call("_SecItemAdd", vec![0x1, 0]), ERR_SEC_NOT_AVAILABLE);
        assert_eq!(call("_SecItemUpdate", vec![0x1, 0x2]), ERR_SEC_NOT_AVAILABLE);
        assert_eq!(call("_SecItemDelete", vec![0x1]), ERR_SEC_ITEM_NOT_FOUND);
        drop(call);
        let a = cpu.read_bytes(0x80000, 64).unwrap().to_vec();
        let b = cpu.read_bytes(0x80040, 64).unwrap().to_vec();
        assert_ne!(a, vec![0; 64]);
        assert_ne!(a, b);
        assert_eq!(u64::from_le_bytes(cpu.read_bytes(result, 8).unwrap().try_into().unwrap()), 0);
    }
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Bounded CFDictionary null-callback storage: keys/values are opaque guest
//! pointer-sized integers, never dereferenced or retained. No CFType/custom
//! equality, hashing, copy, or lifetime callbacks are silently substituted.
//! Independently implemented using the inspected WinObjC CFDictionary.h/c
//! contract (Swift/Apple, Apache2.0 with Runtime Library Exception).
use std::collections::BTreeMap;
const MAX_DICTIONARIES: usize = 256;
const MAX_ENTRIES: usize = 1024;
const MAX_TOTAL_ENTRIES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Handle(u64);
struct Dictionary {
    pointer_bits: u8,
    entries: BTreeMap<u64, u64>,
}
#[derive(Default)]
pub(super) struct Dictionaries {
    next: u64,
    total: usize,
    dictionaries: BTreeMap<u64, Dictionary>,
}
impl Dictionary {
    fn pointer(&self, value: u64) -> Result<(), String> {
        if self.pointer_bits == 32 && value > u32::MAX as u64 {
            Err("CFDictionary pointer exceeds guest32 width".into())
        } else {
            Ok(())
        }
    }
}
impl Dictionaries {
    pub(super) fn create(&mut self, capacity: i64, pointer_bits: u8) -> Result<Handle, String> {
        if ![32, 64].contains(&pointer_bits) || capacity < 0 || capacity > MAX_ENTRIES as i64 {
            return Err("CFDictionary unsupported pointer width or capacity hint".into());
        }
        if self.dictionaries.len() >= MAX_DICTIONARIES {
            return Err("CFDictionary object budget exceeded".into());
        }
        let id = self
            .next
            .checked_add(1)
            .ok_or("CFDictionary identity exhausted")?;
        // Capacity is a hint, not a maximum entry count.
        self.dictionaries.insert(
            id,
            Dictionary {
                pointer_bits,
                entries: BTreeMap::new(),
            },
        );
        self.next = id;
        Ok(Handle(id))
    }
    fn dictionary(&self, h: Handle) -> Result<&Dictionary, String> {
        self.dictionaries
            .get(&h.0)
            .ok_or("CFDictionary handle unknown or disposed".into())
    }
    pub(super) fn get(&self, h: Handle, key: u64) -> Result<Option<u64>, String> {
        let d = self.dictionary(h)?;
        d.pointer(key)?;
        Ok(d.entries.get(&key).copied())
    }
    pub(super) fn count(&self, h: Handle) -> Result<usize, String> {
        Ok(self.dictionary(h)?.entries.len())
    }
    pub(super) fn set(&mut self, h: Handle, key: u64, value: u64) -> Result<(), String> {
        let d = self.dictionary(h)?;
        d.pointer(key)?;
        d.pointer(value)?;
        let added = !d.entries.contains_key(&key);
        if added && (d.entries.len() >= MAX_ENTRIES || self.total >= MAX_TOTAL_ENTRIES) {
            return Err("CFDictionary entry budget exceeded".into());
        }
        self.dictionaries
            .get_mut(&h.0)
            .unwrap()
            .entries
            .insert(key, value);
        if added {
            self.total += 1;
        }
        Ok(())
    }
    pub(super) fn remove(&mut self, h: Handle, key: u64) -> Result<(), String> {
        self.dictionary(h)?.pointer(key)?;
        if self
            .dictionaries
            .get_mut(&h.0)
            .unwrap()
            .entries
            .remove(&key)
            .is_some()
        {
            self.total -= 1;
        }
        Ok(())
    }
    pub(super) fn destroy(&mut self, h: Handle) -> Result<(), String> {
        let count = self.dictionary(h)?.entries.len();
        self.dictionaries.remove(&h.0);
        self.total -= count;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pointer_identity_replacement_null_values_and_guest_widths() {
        let mut dictionaries = Dictionaries::default();
        for bits in [32, 64] {
            let h = dictionaries.create(0, bits).unwrap();
            dictionaries.set(h, 0, 0).unwrap();
            assert_eq!(dictionaries.get(h, 0).unwrap(), Some(0));
            assert_eq!(dictionaries.get(h, 1).unwrap(), None);
            dictionaries.set(h, 2, 3).unwrap();
            dictionaries.set(h, 2, 4).unwrap();
            assert_eq!(dictionaries.count(h).unwrap(), 2);
            assert_eq!(dictionaries.get(h, 2).unwrap(), Some(4));
            assert_eq!(dictionaries.get(h, 3).unwrap(), None);
            if bits == 32 {
                assert!(dictionaries.set(h, 1 << 32, 4).is_err());
                assert!(dictionaries.set(h, 2, 1 << 32).is_err());
                assert!(dictionaries.remove(h, 1 << 32).is_err());
                assert_eq!(dictionaries.get(h, 2).unwrap(), Some(4));
            } else {
                dictionaries.set(h, 1 << 40, u64::MAX).unwrap();
            }
            dictionaries.remove(h, 2).unwrap();
            dictionaries.remove(h, 2).unwrap();
            dictionaries.destroy(h).unwrap();
            assert!(dictionaries.get(h, 0).is_err());
        }
        assert_eq!(dictionaries.total, 0);
    }
    #[test]
    fn capacity_hints_limits_and_failure_atomicity() {
        let mut d = Dictionaries::default();
        assert!(d.create(-1, 64).is_err());
        assert!(d.create(0, 16).is_err());
        for group in 0..8 {
            let h = d.create(1, 64).unwrap(); // Hint1 still permits growth.
            for key in 0..MAX_ENTRIES {
                d.set(h, key as u64, key as u64).unwrap();
            }
            assert!(d.set(h, MAX_ENTRIES as u64, 1).is_err());
            d.set(h, 0, 42).unwrap();
            assert_eq!(d.get(h, 0).unwrap(), Some(42));
            assert_eq!(d.count(h).unwrap(), MAX_ENTRIES);
            if group == 7 {
                let empty = d.create(0, 64).unwrap();
                assert!(d.set(empty, 1, 2).is_err());
                d.remove(h, 1).unwrap();
                d.set(empty, 1, 2).unwrap();
            }
        }
        assert_eq!(d.total, MAX_TOTAL_ENTRIES);
    }
}

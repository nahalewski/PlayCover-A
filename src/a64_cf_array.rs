/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bounded mutable CF arrays. Values remain opaque guest addresses.
//! Null callbacks mean no ownership operations. CF-type callbacks require an
//! explicit lifetime provider; custom guest callbacks are not approximated.
use std::collections::BTreeMap;
const MAX_ARRAYS: usize = 1024;
const MAX_VALUES: usize = 65536;
const MAX_TOTAL_VALUES: usize = 262144;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callbacks {
    None,
    CfType,
}
pub trait ValueLifetime {
    fn retain(&mut self, guest_object: u64) -> Result<(), String>;
    fn release(&mut self, guest_object: u64) -> Result<(), String>;
}
struct Array {
    values: Vec<u64>,
    callbacks: Callbacks,
    retains: u64,
    release_cursor: usize,
}
#[derive(Default)]
pub struct Arrays {
    next: u64,
    total_values: usize,
    objects: BTreeMap<u64, Array>,
}
impl Arrays {
    pub fn create(&mut self, capacity: i64, callbacks: Callbacks) -> Result<Handle, String> {
        if capacity < 0 || capacity as u64 > MAX_VALUES as u64 || self.objects.len() >= MAX_ARRAYS {
            return Err("CF array capacity/object limit invalid".into());
        }
        // CF capacity is an allocation hint, not a fixed logical length.
        let id = self
            .next
            .checked_add(1)
            .ok_or("CF array identity exhausted")?;
        self.next = id;
        self.objects.insert(
            id,
            Array {
                values: Vec::new(),
                callbacks,
                retains: 1,
                release_cursor: 0,
            },
        );
        Ok(Handle(id))
    }
    fn active(&self, h: Handle) -> Result<&Array, String> {
        self.objects
            .get(&h.0)
            .filter(|a| a.retains != 0)
            .ok_or_else(|| "CF array unknown or deallocating".into())
    }
    pub fn count(&self, h: Handle) -> Result<usize, String> {
        Ok(self.active(h)?.values.len())
    }
    pub fn value(&self, h: Handle, index: usize) -> Result<u64, String> {
        self.active(h)?
            .values
            .get(index)
            .copied()
            .ok_or_else(|| "CF array index out of bounds".into())
    }
    pub fn append(
        &mut self,
        h: Handle,
        value: u64,
        lifetime: &mut impl ValueLifetime,
    ) -> Result<(), String> {
        let a = self.active(h)?;
        if a.values.len() >= MAX_VALUES || self.total_values >= MAX_TOTAL_VALUES {
            return Err("CF array element limit exceeded".into());
        }
        if a.callbacks == Callbacks::CfType {
            lifetime.retain(value)?;
        }
        self.objects.get_mut(&h.0).unwrap().values.push(value);
        self.total_values += 1;
        Ok(())
    }
    pub fn retain(&mut self, h: Handle) -> Result<(), String> {
        self.active(h)?;
        let a = self.objects.get_mut(&h.0).unwrap();
        a.retains = a.retains.checked_add(1).ok_or("CF array retain overflow")?;
        Ok(())
    }
    /// If a child release fails, preserve the deallocating array and cursor.
    /// Retrying release resumes remaining children without releasing earlier
    /// children twice. Lifetime providers must leave state unchanged on error.
    pub fn release(&mut self, h: Handle, lifetime: &mut impl ValueLifetime) -> Result<(), String> {
        let a = self.objects.get_mut(&h.0).ok_or("CF array unknown")?;
        if a.retains > 1 {
            a.retains -= 1;
            return Ok(());
        }
        a.retains = 0;
        if a.callbacks == Callbacks::CfType {
            while a.release_cursor < a.values.len() {
                lifetime.release(a.values[a.release_cursor])?;
                a.release_cursor += 1;
            }
        }
        self.total_values -= a.values.len();
        self.objects.remove(&h.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Lifetime {
        counts: BTreeMap<u64, u64>,
        fail: Option<u64>,
    }
    impl ValueLifetime for Lifetime {
        fn retain(&mut self, p: u64) -> Result<(), String> {
            if p == 0 {
                return Err("unknown object".into());
            }
            *self.counts.entry(p).or_default() += 1;
            Ok(())
        }
        fn release(&mut self, p: u64) -> Result<(), String> {
            if self.fail == Some(p) {
                return Err("release blocked".into());
            }
            let n = self.counts.get_mut(&p).ok_or("unknown object")?;
            *n = n.checked_sub(1).ok_or("over release")?;
            Ok(())
        }
    }
    #[test]
    fn null_callbacks_do_not_retain_or_dereference() {
        let mut arrays = Arrays::default();
        let mut lifetime = Lifetime::default();
        let h = arrays.create(0, Callbacks::None).unwrap();
        arrays.append(h, 0, &mut lifetime).unwrap();
        arrays.append(h, u64::MAX, &mut lifetime).unwrap();
        assert_eq!(arrays.count(h).unwrap(), 2);
        assert_eq!(arrays.value(h, 1).unwrap(), u64::MAX);
        assert!(arrays.value(h, 2).is_err());
        assert!(lifetime.counts.is_empty());
        arrays.release(h, &mut lifetime).unwrap();
        assert!(arrays.count(h).is_err());
    }
    #[test]
    fn cf_callbacks_own_each_occurrence_and_resume_failed_destruction() {
        let mut arrays = Arrays::default();
        let mut lifetime = Lifetime::default();
        let h = arrays.create(1, Callbacks::CfType).unwrap();
        assert!(arrays.append(h, 0, &mut lifetime).is_err());
        assert_eq!(arrays.count(h).unwrap(), 0);
        for p in [10, 10, 20] {
            arrays.append(h, p, &mut lifetime).unwrap();
        }
        arrays.retain(h).unwrap();
        arrays.release(h, &mut lifetime).unwrap();
        assert_eq!(lifetime.counts[&10], 2);
        lifetime.fail = Some(20);
        assert!(arrays.release(h, &mut lifetime).is_err());
        assert_eq!(lifetime.counts[&10], 0);
        assert_eq!(lifetime.counts[&20], 1);
        assert!(arrays.count(h).is_err());
        assert!(arrays.retain(h).is_err());
        lifetime.fail = None;
        arrays.release(h, &mut lifetime).unwrap();
        assert_eq!(lifetime.counts[&20], 0);
        assert_eq!(arrays.total_values, 0);
        assert!(arrays.release(h, &mut lifetime).is_err());
    }
    #[test]
    fn capacity_and_unknown_handles() {
        let mut arrays = Arrays::default();
        assert!(arrays.create(-1, Callbacks::None).is_err());
        assert!(arrays
            .create(MAX_VALUES as i64 + 1, Callbacks::None)
            .is_err());
        assert!(arrays.count(Handle(123)).is_err());
    }
}

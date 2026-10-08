/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Explicit guest-thread TLS, not host-thread TLS or a pthread scheduler.
//! iOS dynamic key range: Apple libpthread types_internal.h/pthread_tsd.c.
use std::collections::BTreeMap;
pub const FIRST_KEY: u64 = 256;
pub const KEY_COUNT: u64 = 256;
pub const EAGAIN: i32 = 35;
pub const EINVAL: i32 = 22;
const MAX_THREADS: usize = 64;
const DESTRUCTOR_PASSES: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ThreadId(pub u64);
#[derive(Clone, Copy)]
struct Key {
    epoch: u64,
    destructor: u64,
}
#[derive(Clone, Copy)]
enum Phase {
    Running,
    Exiting { pass: u8, next: u64 },
    Exited,
    Quarantined,
}
struct Thread {
    phase: Phase,
    values: BTreeMap<u64, u64>,
    pending: Option<(u64, bool)>,
}
/// Private ticket fields prevent constructing a completion for another call.
#[derive(Debug)]
pub struct Destructor {
    thread: ThreadId,
    key: u64,
    epoch: u64,
    nonce: u64,
    function: u64,
    value: u64,
}
impl Destructor {
    pub fn function(&self) -> u64 {
        self.function
    }
    pub fn value(&self) -> u64 {
        self.value
    }
}
#[derive(Default)]
pub struct Tls {
    keys: BTreeMap<u64, Key>,
    threads: BTreeMap<ThreadId, Thread>,
    epoch: u64,
    nonce: u64,
}
impl Tls {
    pub fn register_thread(&mut self, id: ThreadId) -> Result<(), String> {
        if id.0 == 0 || self.threads.contains_key(&id) || self.threads.len() >= MAX_THREADS {
            return Err("TLS thread identity invalid/already registered/limit reached".into());
        }
        self.threads.insert(
            id,
            Thread {
                phase: Phase::Running,
                values: BTreeMap::new(),
                pending: None,
            },
        );
        Ok(())
    }
    pub fn create_key(&mut self, destructor: u64) -> Result<u64, i32> {
        let key = (FIRST_KEY..FIRST_KEY + KEY_COUNT)
            .find(|k| !self.keys.contains_key(k))
            .ok_or(EAGAIN)?;
        self.epoch = self.epoch.checked_add(1).ok_or(EAGAIN)?;
        self.keys.insert(
            key,
            Key {
                epoch: self.epoch,
                destructor,
            },
        );
        Ok(key)
    }
    pub fn delete_key(&mut self, key: u64) -> Result<(), i32> {
        self.keys.remove(&key).ok_or(EINVAL)?;
        for thread in self.threads.values_mut() {
            thread.values.remove(&key);
        }
        // POSIX key deletion does not call destructors.
        Ok(())
    }
    fn active_thread(&self, id: ThreadId) -> Result<&Thread, String> {
        let t = self
            .threads
            .get(&id)
            .ok_or("TLS thread is not registered")?;
        if matches!(t.phase, Phase::Exited | Phase::Quarantined) {
            return Err("TLS thread exited/quarantined".into());
        }
        Ok(t)
    }
    pub fn get(&self, id: ThreadId, key: u64) -> Result<u64, String> {
        let t = self.active_thread(id)?;
        if !self.keys.contains_key(&key) {
            return Err("TLS key unknown/deleted/static key unsupported".into());
        }
        Ok(t.values.get(&key).copied().unwrap_or(0))
    }
    pub fn set(&mut self, id: ThreadId, key: u64, value: u64) -> Result<(), i32> {
        if !self.keys.contains_key(&key) || self.active_thread(id).is_err() {
            return Err(EINVAL);
        }
        let t = self.threads.get_mut(&id).unwrap();
        if value == 0 {
            t.values.remove(&key);
        } else {
            t.values.insert(key, value);
        }
        Ok(())
    }
    pub fn begin_exit(&mut self, id: ThreadId) -> Result<(), String> {
        let t = self
            .threads
            .get_mut(&id)
            .ok_or("TLS thread is not registered")?;
        if !matches!(t.phase, Phase::Running) {
            return Err("TLS thread exit already started/finished".into());
        }
        t.phase = Phase::Exiting {
            pass: 0,
            next: FIRST_KEY,
        };
        Ok(())
    }
    /// Clear a non-null slot BEFORE returning its destructor plan. Destructor
    /// callbacks may repopulate keys; later passes revisit them, at most four.
    pub fn next_destructor(&mut self, id: ThreadId) -> Result<Option<Destructor>, String> {
        let t = self
            .threads
            .get_mut(&id)
            .ok_or("TLS thread is not registered")?;
        if t.pending.is_some() {
            return Err("TLS destructor completion still pending".into());
        }
        loop {
            let (pass, next) = match t.phase {
                Phase::Exiting { pass, next } => (pass, next),
                Phase::Exited => return Ok(None),
                _ => return Err("TLS exit not active/quarantined".into()),
            };
            if next >= FIRST_KEY + KEY_COUNT {
                if pass + 1 >= DESTRUCTOR_PASSES {
                    t.values.clear();
                    t.phase = Phase::Exited;
                    return Ok(None);
                }
                t.phase = Phase::Exiting {
                    pass: pass + 1,
                    next: FIRST_KEY,
                };
                continue;
            }
            t.phase = Phase::Exiting {
                pass,
                next: next + 1,
            };
            let Some(value) = t.values.remove(&next) else {
                continue;
            };
            let Some(key) = self.keys.get(&next) else {
                continue;
            };
            if key.destructor == 0 {
                continue;
            }
            self.nonce = self
                .nonce
                .checked_add(1)
                .ok_or("TLS destructor identity overflow")?;
            t.pending = Some((self.nonce, false));
            return Ok(Some(Destructor {
                thread: id,
                key: next,
                epoch: key.epoch,
                nonce: self.nonce,
                function: key.destructor,
                value,
            }));
        }
    }
    /// Owner must call immediately before executing the actual guest function.
    /// A deleted/reused key cancels the prepared call rather than using stale
    /// callback identity. It has no destructor side effect yet.
    pub fn start_destructor(&mut self, call: &Destructor) -> Result<bool, String> {
        let t = self
            .threads
            .get_mut(&call.thread)
            .ok_or("TLS thread unknown")?;
        if t.pending != Some((call.nonce, false)) {
            return Err("TLS destructor ticket stale/already executing".into());
        }
        if !self
            .keys
            .get(&call.key)
            .is_some_and(|k| k.epoch == call.epoch)
        {
            t.pending = None;
            return Ok(false);
        }
        t.pending = Some((call.nonce, true));
        Ok(true)
    }
    /// Only the owner that observed the actual guest return completes a ticket.
    pub fn complete_destructor(&mut self, call: Destructor) -> Result<(), String> {
        let t = self
            .threads
            .get_mut(&call.thread)
            .ok_or("TLS thread unknown")?;
        if t.pending != Some((call.nonce, true)) {
            return Err("TLS destructor ticket not executing/stale".into());
        }
        t.pending = None;
        Ok(())
    }
    /// An execution failure may already have guest side effects. Quarantine,
    /// never replay the destructor or declare a successful thread exit.
    pub fn quarantine(&mut self, call: Destructor) -> Result<(), String> {
        let t = self
            .threads
            .get_mut(&call.thread)
            .ok_or("TLS thread unknown")?;
        if t.pending != Some((call.nonce, true)) {
            return Err("TLS destructor ticket not executing/stale".into());
        }
        t.pending = None;
        t.phase = Phase::Quarantined;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolation_delete_reuse_and_no_destructor_on_delete() {
        let mut tls = Tls::default();
        let a = ThreadId(1);
        let b = ThreadId(2);
        tls.register_thread(a).unwrap();
        tls.register_thread(b).unwrap();
        let key = tls.create_key(0x1234).unwrap();
        assert_eq!(key, FIRST_KEY);
        assert_eq!(tls.get(a, key).unwrap(), 0);
        tls.set(a, key, 99).unwrap();
        assert_eq!(tls.get(a, key).unwrap(), 99);
        assert_eq!(tls.get(b, key).unwrap(), 0);
        tls.delete_key(key).unwrap();
        assert!(tls.get(a, key).is_err());
        assert_eq!(tls.delete_key(key), Err(EINVAL));
        assert_eq!(tls.create_key(0).unwrap(), key);
        assert_eq!(tls.get(a, key).unwrap(), 0);
        assert_eq!(tls.get(b, key).unwrap(), 0);
    }
    #[test]
    fn four_real_completion_rounds_and_clear_before_callback() {
        let mut tls = Tls::default();
        let t = ThreadId(1);
        tls.register_thread(t).unwrap();
        let key = tls.create_key(0x1000).unwrap();
        tls.set(t, key, 10).unwrap();
        tls.begin_exit(t).unwrap();
        let mut calls = 0;
        while let Some(call) = tls.next_destructor(t).unwrap() {
            assert_eq!(tls.get(t, key).unwrap(), 0);
            assert!(tls.next_destructor(t).is_err());
            assert!(tls.start_destructor(&call).unwrap());
            assert!(tls.start_destructor(&call).is_err());
            tls.set(t, key, call.value() + 1).unwrap();
            tls.complete_destructor(call).unwrap();
            calls += 1;
        }
        assert_eq!(calls, 4);
        assert!(tls.get(t, key).is_err());
        assert_eq!(tls.set(t, key, 1), Err(EINVAL));
    }
    #[test]
    fn key_epoch_cancellation_and_failure_quarantine() {
        let mut tls = Tls::default();
        let t = ThreadId(1);
        tls.register_thread(t).unwrap();
        let key = tls.create_key(0x1000).unwrap();
        tls.set(t, key, 10).unwrap();
        tls.begin_exit(t).unwrap();
        let call = tls.next_destructor(t).unwrap().unwrap();
        tls.delete_key(key).unwrap();
        assert_eq!(tls.create_key(0x2000).unwrap(), key);
        assert!(!tls.start_destructor(&call).unwrap());
        assert!(tls.complete_destructor(call).is_err());
        tls.set(t, key, 20).unwrap();
        let call = tls.next_destructor(t).unwrap().unwrap();
        assert!(tls.start_destructor(&call).unwrap());
        tls.quarantine(call).unwrap();
        assert!(tls.next_destructor(t).is_err());
        assert!(tls.get(t, key).is_err());
    }
    #[test]
    fn bounded_key_capacity_and_invalid_threads() {
        let mut tls = Tls::default();
        assert!(tls.register_thread(ThreadId(0)).is_err());
        for _ in 0..KEY_COUNT {
            tls.create_key(0).unwrap();
        }
        assert_eq!(tls.create_key(0), Err(EAGAIN));
        assert_eq!(tls.set(ThreadId(9), FIRST_KEY, 0), Err(EINVAL));
        assert!(tls.get(ThreadId(9), FIRST_KEY).is_err());
    }
}

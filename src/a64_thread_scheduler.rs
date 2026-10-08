/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Bounded cooperative state machine, with owned contexts and TLS lifetimes.
//! No pthread_create ABI, OS thread, mutex release or wait success is implied.
#[path = "a64_thread_priority.rs"]
mod priority;
#[path = "a64_pthread_tls.rs"]
mod tls;
pub use priority::{Priority, Qos};
use std::collections::{BTreeMap, VecDeque};
pub use tls::ThreadId;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    Mutex(u64),
    Condition(u64),
    External(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Runnable,
    Running,
    Blocked,
    Exiting,
    Exited,
    Quarantined,
}
#[derive(Debug)]
pub struct WakeToken {
    thread: ThreadId,
    nonce: u64,
}
impl WakeToken {
    pub fn thread(&self) -> ThreadId {
        self.thread
    }
}
pub struct ExitCall {
    thread: ThreadId,
    inner: tls::Destructor,
}
impl ExitCall {
    pub fn function(&self) -> u64 {
        self.inner.function()
    }
    pub fn value(&self) -> u64 {
        self.inner.value()
    }
}
struct Thread<C> {
    context: C,
    phase: Phase,
    blocked: Option<(u64, BlockReason)>,
}
pub struct Scheduler<C> {
    threads: BTreeMap<ThreadId, Thread<C>>,
    runnable: VecDeque<ThreadId>,
    current: Option<ThreadId>,
    next_id: u64,
    nonce: u64,
    tls: tls::Tls,
    priorities: priority::Priorities,
}
impl<C> Default for Scheduler<C> {
    fn default() -> Self {
        Self {
            threads: BTreeMap::new(),
            runnable: VecDeque::new(),
            current: None,
            next_id: 0,
            nonce: 0,
            tls: tls::Tls::default(),
            priorities: priority::Priorities::default(),
        }
    }
}
impl<C> Scheduler<C> {
    pub fn current(&self) -> Option<ThreadId> {
        self.current
    }
    pub fn phase(&self, id: ThreadId) -> Result<Phase, String> {
        Ok(self
            .threads
            .get(&id)
            .ok_or("unknown scheduler thread")?
            .phase)
    }
    pub fn register(&mut self, context: C) -> Result<ThreadId, String> {
        if self.threads.len() >= 64 {
            return Err("scheduler lifetime thread capacity reached".into());
        }
        let next = self
            .next_id
            .checked_add(1)
            .ok_or("scheduler identity exhausted")?;
        let id = ThreadId(next);
        self.priorities.register(id.0)?;
        if let Err(error) = self.tls.register_thread(id) {
            self.priorities.remove(id.0)?;
            return Err(error);
        }
        self.next_id = next;
        self.threads.insert(
            id,
            Thread {
                context,
                phase: Phase::Runnable,
                blocked: None,
            },
        );
        self.runnable.push_back(id);
        Ok(id)
    }
    /// Updates actual virtual scheduling policy without preempting the CPU owner.
    /// This is not a Darwin SET_SELF syscall or kernel feature advertisement.
    pub fn set_priority(&mut self, id: ThreadId, encoded: u64) -> Result<(), String> {
        if !matches!(
            self.phase(id)?,
            Phase::Runnable | Phase::Running | Phase::Blocked
        ) {
            return Err("priority update requires live scheduler thread".into());
        }
        self.priorities.set(id.0, encoded)
    }
    pub fn priority(&self, id: ThreadId) -> Result<Priority, String> {
        self.priorities.get(id.0)
    }
    /// Select a runnable snapshot only with no CPU owner. The CPU adapter must
    /// restore it before guest execution; no preemptive context capture occurs.
    pub fn select(&mut self) -> Result<Option<(ThreadId, &C)>, String> {
        if self.current.is_some() {
            return Err("scheduler CPU already owned; capture it before switching".into());
        }
        for id in &self.runnable {
            if self.phase(*id)? != Phase::Runnable {
                return Err("runnable queue phase inconsistent".into());
            }
        }
        let Some(selected) = self
            .priorities
            .select(self.runnable.iter().map(|id| id.0))?
        else {
            return Ok(None);
        };
        let position = self
            .runnable
            .iter()
            .position(|id| id.0 == selected)
            .ok_or("priority queue identity missing")?;
        let id = self.runnable.remove(position).unwrap();
        let t = self
            .threads
            .get_mut(&id)
            .ok_or("runnable queue identity missing")?;
        if t.phase != Phase::Runnable {
            return Err("runnable queue phase inconsistent".into());
        }
        t.phase = Phase::Running;
        self.current = Some(id);
        Ok(Some((id, &t.context)))
    }
    pub fn yield_current(&mut self, context: C) -> Result<(), String> {
        let id = self.current.ok_or("no running scheduler thread")?;
        let t = self.threads.get_mut(&id).unwrap();
        if t.phase != Phase::Running {
            return Err("yield requires running thread".into());
        }
        t.context = context;
        t.phase = Phase::Runnable;
        self.current = None;
        self.runnable.push_back(id);
        Ok(())
    }
    /// State foundation only: caller must already have performed any genuine
    /// lock-release/registration protocol. This method never releases a mutex.
    pub fn block_current(&mut self, context: C, reason: BlockReason) -> Result<WakeToken, String> {
        let id = self.current.ok_or("no running scheduler thread")?;
        let nonce = self.nonce.checked_add(1).ok_or("wake identity exhausted")?;
        let t = self.threads.get_mut(&id).unwrap();
        if t.phase != Phase::Running {
            return Err("block requires running thread".into());
        }
        t.context = context;
        t.phase = Phase::Blocked;
        t.blocked = Some((nonce, reason));
        self.nonce = nonce;
        self.current = None;
        Ok(WakeToken { thread: id, nonce })
    }
    /// A signal chooses actual blocked identities, not a stored notification.
    /// Consuming the private token prevents stale/double wakes and ABA reuse.
    pub fn wake(&mut self, token: WakeToken) -> Result<(), String> {
        let t = self
            .threads
            .get_mut(&token.thread)
            .ok_or("wake thread unknown")?;
        if t.phase != Phase::Blocked || !t.blocked.is_some_and(|(n, _)| n == token.nonce) {
            return Err("wake token stale/not blocked".into());
        }
        t.phase = Phase::Runnable;
        t.blocked = None;
        self.runnable.push_back(token.thread);
        Ok(())
    }
    pub fn blocked_reason(&self, id: ThreadId) -> Result<Option<BlockReason>, String> {
        Ok(self
            .threads
            .get(&id)
            .ok_or("unknown scheduler thread")?
            .blocked
            .map(|(_, r)| r))
    }
    pub fn begin_exit(&mut self, context: C) -> Result<ThreadId, String> {
        let id = self.current.ok_or("no running scheduler thread")?;
        if self.phase(id)? != Phase::Running {
            return Err("exit requires running thread".into());
        }
        self.tls.begin_exit(id)?;
        let t = self.threads.get_mut(&id).unwrap();
        t.context = context;
        t.phase = Phase::Exiting;
        self.current = None;
        Ok(id)
    }
    pub fn exit_context(&self, id: ThreadId) -> Result<&C, String> {
        let t = self.threads.get(&id).ok_or("unknown exit thread")?;
        if t.phase != Phase::Exiting {
            return Err("thread not exiting".into());
        }
        Ok(&t.context)
    }
    pub fn next_destructor(&mut self, id: ThreadId) -> Result<Option<ExitCall>, String> {
        if self.phase(id)? != Phase::Exiting {
            return Err("TLS teardown requires exiting scheduler thread".into());
        }
        let next = self.tls.next_destructor(id)?;
        if next.is_none() {
            self.threads.get_mut(&id).unwrap().phase = Phase::Exited
        }
        Ok(next.map(|inner| ExitCall { thread: id, inner }))
    }
    pub fn start_destructor(&mut self, call: &ExitCall) -> Result<bool, String> {
        self.tls.start_destructor(&call.inner)
    }
    /// Call only after observing actual guest destructor return.
    pub fn complete_destructor(&mut self, call: ExitCall) -> Result<(), String> {
        self.tls.complete_destructor(call.inner)
    }
    pub fn quarantine_destructor(&mut self, id: ThreadId, call: ExitCall) -> Result<(), String> {
        if self.phase(id)? != Phase::Exiting {
            return Err("quarantine requires exiting thread".into());
        }
        // TLS validates the ticket thread identity; never change another
        // scheduler identity after an execution failure.
        if call.thread != id {
            return Err("destructor ticket belongs to another thread".into());
        }
        self.tls.quarantine(call.inner)?;
        self.threads.get_mut(&id).unwrap().phase = Phase::Quarantined;
        Ok(())
    }
    pub fn create_key(&mut self, destructor: u64) -> Result<u64, i32> {
        self.tls.create_key(destructor)
    }
    pub fn delete_key(&mut self, key: u64) -> Result<(), i32> {
        self.tls.delete_key(key)
    }
    pub fn set_tls(&mut self, id: ThreadId, key: u64, value: u64) -> Result<(), i32> {
        self.tls.set(id, key, value)
    }
    pub fn get_tls(&self, id: ThreadId, key: u64) -> Result<u64, String> {
        self.tls.get(id, key)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn priority_switch_preserves_context_and_never_selects_blocked_thread() {
        let mut s = Scheduler::default();
        let low = s.register(10).unwrap();
        let high = s.register(20).unwrap();
        let encode = |qos, relative| u64::from(Priority { qos, relative }.encode());
        s.set_priority(high, encode(Qos::UserInteractive, -15))
            .unwrap();
        assert_eq!(s.select().unwrap(), Some((high, &20)));
        let token = s.block_current(21, BlockReason::External(1)).unwrap();
        assert_eq!(s.select().unwrap(), Some((low, &10)));
        s.set_priority(low, encode(Qos::Maintenance, 0)).unwrap();
        assert!(s.select().is_err());
        s.yield_current(11).unwrap();
        s.wake(token).unwrap();
        assert_eq!(s.select().unwrap(), Some((high, &21)));
        let old = s.priority(high).unwrap();
        assert!(s.set_priority(high, 0x2000ef).is_err());
        assert_eq!(s.priority(high).unwrap(), old);
        s.set_priority(high, encode(Qos::Maintenance, -1)).unwrap();
        s.yield_current(22).unwrap();
        assert_eq!(s.select().unwrap(), Some((low, &11)));
        s.begin_exit(12).unwrap();
        assert!(s.set_priority(low, encode(Qos::Default, 0)).is_err());
        assert_eq!(s.select().unwrap(), Some((high, &22)));
    }
    #[test]
    fn round_robin_contexts_and_wake() {
        let mut s = Scheduler::default();
        let a = s.register(10).unwrap();
        let b = s.register(20).unwrap();
        assert_eq!(s.select().unwrap(), Some((a, &10)));
        assert!(s.select().is_err());
        s.yield_current(11).unwrap();
        assert_eq!(s.select().unwrap(), Some((b, &20)));
        let token = s.block_current(21, BlockReason::Condition(8)).unwrap();
        assert_eq!(
            s.blocked_reason(b).unwrap(),
            Some(BlockReason::Condition(8))
        );
        assert_eq!(s.select().unwrap(), Some((a, &11)));
        s.yield_current(12).unwrap();
        s.wake(token).unwrap();
        assert_eq!(s.select().unwrap(), Some((a, &12)));
        s.yield_current(13).unwrap();
        assert_eq!(s.select().unwrap(), Some((b, &21)));
    }
    #[test]
    fn tls_lifetime_requires_real_completion() {
        let mut s = Scheduler::default();
        let a = s.register(1).unwrap();
        let b = s.register(2).unwrap();
        let key = s.create_key(0x1000).unwrap();
        s.set_tls(a, key, 42).unwrap();
        assert_eq!(s.get_tls(b, key).unwrap(), 0);
        s.select().unwrap();
        assert_eq!(s.begin_exit(3).unwrap(), a);
        let call = s.next_destructor(a).unwrap().unwrap();
        assert_eq!(s.phase(a).unwrap(), Phase::Exiting);
        assert_eq!(s.get_tls(a, key).unwrap(), 0);
        assert!(s.start_destructor(&call).unwrap());
        assert!(s.next_destructor(a).is_err());
        s.complete_destructor(call).unwrap();
        assert!(s.next_destructor(a).unwrap().is_none());
        assert_eq!(s.phase(a).unwrap(), Phase::Exited);
        assert!(s.get_tls(a, key).is_err());
        assert_eq!(s.select().unwrap(), Some((b, &2)));
    }
    #[test]
    fn no_cpu_owner_and_capacity() {
        let mut s = Scheduler::default();
        assert!(s.yield_current(1).is_err());
        assert!(s.block_current(1, BlockReason::External(0)).is_err());
        for i in 0..64 {
            s.register(i).unwrap();
        }
        assert!(s.register(65).is_err());
    }
    #[test]
    fn stale_wake_and_failed_destructor_are_not_replayed() {
        let mut s = Scheduler::default();
        let id = s.register(0).unwrap();
        s.select().unwrap();
        let token = s.block_current(1, BlockReason::External(1)).unwrap();
        let stale = WakeToken {
            thread: token.thread,
            nonce: token.nonce,
        };
        s.wake(token).unwrap();
        s.select().unwrap();
        let latest = s.block_current(2, BlockReason::External(2)).unwrap();
        assert!(s.wake(stale).is_err());
        assert_eq!(s.phase(id).unwrap(), Phase::Blocked);
        s.wake(latest).unwrap();
        s.select().unwrap();
        let key = s.create_key(0x1000).unwrap();
        s.set_tls(id, key, 42).unwrap();
        s.begin_exit(3).unwrap();
        let call = s.next_destructor(id).unwrap().unwrap();
        assert!(s.start_destructor(&call).unwrap());
        s.quarantine_destructor(id, call).unwrap();
        assert_eq!(s.phase(id).unwrap(), Phase::Quarantined);
        assert!(s.next_destructor(id).is_err());
        assert!(s.select().unwrap().is_none());
    }
}

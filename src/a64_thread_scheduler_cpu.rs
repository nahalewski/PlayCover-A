/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Cooperative CPU snapshot adapter, not a pthread ABI or process runner.
#[path = "a64_thread_scheduler.rs"]
mod state;
use super::{
    bridge::{GuestBridge, GuestCall},
    A64Cpu,
};
pub(super) use state::{BlockReason, Phase, Priority, Qos, Scheduler, ThreadId, WakeToken};
use std::collections::{BTreeMap, BTreeSet};
use touchHLE_dynarmic_wrapper::a64::A64Context;
#[derive(Default)]
pub(super) struct CpuScheduler {
    state: state::Scheduler<A64Context>,
    stacks: BTreeMap<ThreadId, (u64, u64)>,
    exit_owner: Option<ThreadId>,
    restartable: Option<super::restartable::Ranges>,
    reset_pcs: BTreeSet<ThreadId>,
}
impl CpuScheduler {
    fn validate(cpu: &A64Cpu, stack: (u64, u64)) -> Result<(), String> {
        let (start, end) = stack;
        let size = end
            .checked_sub(start)
            .ok_or("thread stack range reversed")?;
        if size < 16
            || size > 1024 * 1024
            || start % 16 != 0
            || end % 16 != 0
            || cpu.sp() % 16 != 0
            || cpu.sp() < start
            || cpu.sp() > end
        {
            return Err("thread stack must be bounded, aligned and contain SP".into());
        }
        cpu.validate_guest_write(start, size as usize)?;
        if cpu.pc() % 4 != 0 || !cpu.mapped_permissions(cpu.pc()).is_some_and(|p| p & 4 != 0) {
            return Err("thread PC is not aligned executable guest memory".into());
        }
        Ok(())
    }
    /// Adopt a genuinely prepared snapshot and an existing exclusive guest
    /// stack. This allocates neither pthread/TSD structures nor guest threads.
    pub(super) fn adopt(&mut self, cpu: &A64Cpu, stack: (u64, u64)) -> Result<ThreadId, String> {
        if self.state.current().is_some() || self.exit_owner.is_some() {
            return Err("capture/release owned CPU before adopting another snapshot".into());
        }
        Self::validate(cpu, stack)?;
        if self
            .stacks
            .values()
            .any(|&(a, b)| stack.0 < b && a < stack.1)
        {
            return Err("guest thread stacks overlap".into());
        }
        let id = self.state.register(cpu.save_context())?;
        self.stacks.insert(id, stack);
        Ok(id)
    }
    pub(super) fn select(&mut self, cpu: &mut A64Cpu) -> Result<Option<ThreadId>, String> {
        if self.exit_owner.is_some() {
            return Err("TLS teardown exclusively owns CPU".into());
        }
        let Some((id, context)) = self.state.select()? else {
            return Ok(None);
        };
        cpu.restore_context(context);
        if self.reset_pcs.contains(&id) {
            self.recover_current_restartable(cpu)?;
            self.reset_pcs.remove(&id);
        }
        Ok(Some(id))
    }
    /// Release-kernel registration: one immutable range table, before a second
    /// thread exists. The table belongs to this scheduler's process identity.
    pub(super) fn register_restartable_ranges(
        &mut self,
        ranges: super::restartable::Ranges,
    ) -> Result<(), u32> {
        if self.current().is_none() || self.exit_owner.is_some() {
            return Err(4);
        }
        if self.stacks.len() != 1 || self.restartable.is_some() {
            return Err(46); // KERN_NOT_SUPPORTED
        }
        self.restartable = Some(ranges);
        Ok(())
    }
    /// XNU task_restartable_ranges_synchronize requests AST_RESET_PCS for the
    /// other threads. In this single-CPU scheduler they are already off-core;
    /// no VM fault or concurrently executing thread requires acknowledgement.
    /// A blocked snapshot keeps the request until it actually returns to user.
    pub(super) fn synchronize_restartable_ranges(&mut self) -> Result<(), String> {
        let current = self.current().ok_or("restartable synchronization needs CPU owner")?;
        if self.restartable.is_none() {
            return Ok(());
        }
        for &id in self.stacks.keys() {
            if id != current && matches!(self.phase(id)?, Phase::Runnable | Phase::Blocked) {
                self.reset_pcs.insert(id);
            }
        }
        Ok(())
    }
    /// Explicit reset-PC AST / sigreturn action. Ordinary syscall yield/block
    /// and preemption alone do not request recovery (XNU restartable.c).
    pub(super) fn recover_current_restartable(&mut self, cpu: &mut A64Cpu) -> Result<bool, String> {
        let id = self.current().ok_or("restartable recovery needs CPU owner")?;
        Self::validate(cpu, self.stacks[&id])?;
        let Some(pc) = self.restartable.as_ref().and_then(|r| r.recovery_pc(cpu.pc())) else {
            return Ok(false);
        };
        if pc % 4 != 0 || !cpu.mapped_permissions(pc).is_some_and(|p| p & 4 != 0) {
            return Err("restartable recovery PC is not executable guest memory".into());
        }
        cpu.set_pc(pc);
        Ok(true)
    }
    pub(super) fn current(&self) -> Option<ThreadId> {
        self.state.current()
    }
    pub(super) fn set_priority(&mut self, id: ThreadId, encoded: u64) -> Result<(), String> {
        self.state.set_priority(id, encoded)
    }
    pub(super) fn priority(&self, id: ThreadId) -> Result<Priority, String> {
        self.state.priority(id)
    }
    pub(super) fn service_identity(&self) -> Option<ThreadId> {
        self.current().or(self
            .exit_owner
            .filter(|&id| self.state.phase(id) == Ok(Phase::Exiting)))
    }
    pub(super) fn phase(&self, id: ThreadId) -> Result<Phase, String> {
        self.state.phase(id)
    }
    pub(super) fn yield_current(&mut self, cpu: &A64Cpu) -> Result<(), String> {
        let id = self.current().ok_or("no thread owns CPU")?;
        Self::validate(cpu, self.stacks[&id])?;
        self.state.yield_current(cpu.save_context())
    }
    pub(super) fn block_current(
        &mut self,
        cpu: &A64Cpu,
        reason: BlockReason,
    ) -> Result<WakeToken, String> {
        let id = self.current().ok_or("no thread owns CPU")?;
        Self::validate(cpu, self.stacks[&id])?;
        self.state.block_current(cpu.save_context(), reason)
    }
    pub(super) fn wake(&mut self, token: WakeToken) -> Result<(), String> {
        self.state.wake(token)
    }
    pub(super) fn begin_exit(&mut self, cpu: &A64Cpu) -> Result<ThreadId, String> {
        let id = self.current().ok_or("no thread owns CPU")?;
        Self::validate(cpu, self.stacks[&id])?;
        self.state.begin_exit(cpu.save_context())
    }
    pub(super) fn create_key(&mut self, destructor: u64) -> Result<u64, i32> {
        self.state.create_key(destructor)
    }
    pub(super) fn delete_key(&mut self, key: u64) -> Result<(), i32> {
        self.state.delete_key(key)
    }
    pub(super) fn set_tls(&mut self, id: ThreadId, key: u64, value: u64) -> Result<(), i32> {
        self.state.set_tls(id, key, value)
    }
    pub(super) fn get_tls(&self, id: ThreadId, key: u64) -> Result<u64, String> {
        self.state.get_tls(id, key)
    }
    fn prepare_shared_exit(
        &mut self,
        cpu: &mut A64Cpu,
        id: ThreadId,
    ) -> Result<A64Context, String> {
        if self.current().is_some() || self.exit_owner.is_some() {
            return Err("TLS teardown cannot borrow owned CPU".into());
        }
        let context = self.state.exit_context(id)?;
        let caller = cpu.save_context();
        cpu.restore_context(context);
        self.exit_owner = Some(id);
        Ok(caller)
    }
    /// Execute real destructor calls under the exiting thread's full context.
    /// Keep CPU ownership exclusive; original caller snapshot restored on all
    /// exits. Failure quarantines instead of replaying partially executed code.
    pub(super) fn finish_exit(
        &mut self,
        cpu: &mut A64Cpu,
        bridge: &mut GuestBridge,
        id: ThreadId,
        max_calls: usize,
        per_call_ticks: u64,
    ) -> Result<(), String> {
        if self.current().is_some() {
            return Err("TLS exit cannot borrow CPU from running thread".into());
        }
        if max_calls == 0
            || max_calls > 1024
            || per_call_ticks == 0
            || per_call_ticks > 1_000_000
            || max_calls as u64 * per_call_ticks > 1_000_000
        {
            return Err("scheduler TLS destructor budget invalid".into());
        }
        let context = self.state.exit_context(id)?;
        let caller = cpu.save_context();
        cpu.restore_context(context);
        let result = (|| {
            let mut count = 0;
            loop {
                let Some(call) = self.state.next_destructor(id)? else {
                    return Ok(());
                };
                if !self.state.start_destructor(&call)? {
                    continue;
                }
                if count >= max_calls {
                    self.state.quarantine_destructor(id, call)?;
                    return Err("scheduler TLS call budget exhausted; thread quarantined".into());
                }
                count += 1;
                if let Err(error) = bridge.call(
                    cpu,
                    &GuestCall {
                        entry: call.function(),
                        integers: vec![call.value()],
                        ..Default::default()
                    },
                    per_call_ticks,
                ) {
                    self.state.quarantine_destructor(id, call)?;
                    return Err(format!("scheduler TLS guest destructor failed: {error}"));
                }
                self.state.complete_destructor(call)?;
            }
        })();
        cpu.restore_context(&caller);
        result
    }
}
/// Borrow only to advance scheduler state; release RefCell borrow BEFORE any
/// guest call, allowing genuine TLS callbacks to resolve the exiting identity.
pub(super) fn finish_exit_shared(
    owner: &std::rc::Rc<std::cell::RefCell<CpuScheduler>>,
    cpu: &mut A64Cpu,
    bridge: &mut GuestBridge,
    id: ThreadId,
    max_calls: usize,
    per_call_ticks: u64,
) -> Result<(), String> {
    if max_calls == 0
        || max_calls > 1024
        || per_call_ticks == 0
        || per_call_ticks > 1_000_000
        || max_calls as u64 * per_call_ticks > 1_000_000
    {
        return Err("shared TLS teardown budget invalid".into());
    }
    let caller = owner.borrow_mut().prepare_shared_exit(cpu, id)?;
    let result = (|| {
        let mut count = 0;
        loop {
            let next = owner.borrow_mut().state.next_destructor(id)?;
            let Some(call) = next else { return Ok(()) };
            let started = owner.borrow_mut().state.start_destructor(&call)?;
            if !started {
                continue;
            }
            if count >= max_calls {
                owner.borrow_mut().state.quarantine_destructor(id, call)?;
                return Err("shared TLS call budget exhausted; thread quarantined".into());
            }
            count += 1;
            let actual = bridge.call(
                cpu,
                &GuestCall {
                    entry: call.function(),
                    integers: vec![call.value()],
                    ..Default::default()
                },
                per_call_ticks,
            );
            if let Err(error) = actual {
                owner.borrow_mut().state.quarantine_destructor(id, call)?;
                return Err(format!("shared TLS guest destructor failed: {error}"));
            }
            owner.borrow_mut().state.complete_destructor(call)?;
        }
    })();
    owner.borrow_mut().exit_owner = None;
    cpu.restore_context(&caller);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    fn restartable_range() -> super::super::restartable::Ranges {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x50000u64.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        super::super::restartable::Ranges::parse(&bytes).unwrap()
    }
    #[test]
    fn restartable_ast_recovers_other_real_context_only_on_return() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000, 8192, 3).unwrap();
        cpu.map_zeroed(0x50000, 4096, 5).unwrap();
        cpu.set_pc(0x50004);
        cpu.set_sp(0x41000);
        cpu.set_reg(3, 11);
        let mut s = CpuScheduler::default();
        let a = s.adopt(&cpu, (0x40000, 0x41000)).unwrap();
        assert!(s.register_restartable_ranges(restartable_range()).is_err());
        s.select(&mut cpu).unwrap();
        s.register_restartable_ranges(restartable_range()).unwrap();
        assert_eq!(s.register_restartable_ranges(restartable_range()), Err(46));
        s.yield_current(&cpu).unwrap();
        cpu.set_pc(0x50008);
        cpu.set_sp(0x42000);
        cpu.set_reg(3, 22);
        cpu.set_vector(7, [33, 44]);
        let b = s.adopt(&cpu, (0x41000, 0x42000)).unwrap();
        assert_eq!(s.select(&mut cpu).unwrap(), Some(a));
        // Voluntary yield had no AST: its interior PC remains unchanged.
        assert_eq!(cpu.pc(), 0x50004);
        s.synchronize_restartable_ranges().unwrap();
        assert_eq!(cpu.pc(), 0x50004); // caller is excluded by XNU
        s.yield_current(&cpu).unwrap();
        assert_eq!(s.select(&mut cpu).unwrap(), Some(b));
        assert_eq!(cpu.pc(), 0x50020);
        assert_eq!(cpu.reg(3), 22);
        assert_eq!(cpu.vector(7), [33, 44]);
        assert_eq!(cpu.sp(), 0x42000);
        // Request consumed: a subsequent voluntary block in the range does
        // not silently recover when this same context wakes.
        cpu.set_pc(0x5000c);
        let token = s.block_current(&cpu, BlockReason::External(1)).unwrap();
        s.select(&mut cpu).unwrap();
        s.yield_current(&cpu).unwrap();
        s.wake(token).unwrap();
        s.select(&mut cpu).unwrap();
        s.yield_current(&cpu).unwrap();
        assert_eq!(s.select(&mut cpu).unwrap(), Some(b));
        assert_eq!(cpu.pc(), 0x5000c);
    }
    #[test]
    fn restartable_recovery_excludes_boundaries_and_waits_for_blocked_context() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000, 8192, 3).unwrap();
        cpu.map_zeroed(0x50000, 4096, 5).unwrap();
        cpu.set_pc(0x50000);
        cpu.set_sp(0x41000);
        let mut s = CpuScheduler::default();
        let a = s.adopt(&cpu, (0x40000, 0x41000)).unwrap();
        s.select(&mut cpu).unwrap();
        s.register_restartable_ranges(restartable_range()).unwrap();
        for pc in [0x50000, 0x50020, 0x50040] {
            cpu.set_pc(pc);
            assert!(!s.recover_current_restartable(&mut cpu).unwrap());
            assert_eq!(cpu.pc(), pc);
        }
        cpu.set_pc(0x50004);
        let token = s.block_current(&cpu, BlockReason::External(2)).unwrap();
        cpu.set_pc(0x50040);
        cpu.set_sp(0x42000);
        let b = s.adopt(&cpu, (0x41000, 0x42000)).unwrap();
        assert_eq!(s.select(&mut cpu).unwrap(), Some(b));
        s.synchronize_restartable_ranges().unwrap();
        assert_eq!(s.phase(a).unwrap(), Phase::Blocked);
        s.wake(token).unwrap();
        s.yield_current(&cpu).unwrap();
        assert_eq!(s.select(&mut cpu).unwrap(), Some(a));
        assert_eq!(cpu.pc(), 0x50020);
        assert_eq!(s.register_restartable_ranges(restartable_range()), Err(46));
    }
    #[test]
    fn priority_selects_real_cpu_snapshot_without_changing_owner_mid_call() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000, 8192, 3).unwrap();
        cpu.map_zeroed(0x50000, 4096, 5).unwrap();
        cpu.set_pc(0x50000);
        cpu.set_sp(0x41000);
        cpu.set_reg(3, 11);
        let mut scheduler = CpuScheduler::default();
        let first = scheduler.adopt(&cpu, (0x40000, 0x41000)).unwrap();
        cpu.set_sp(0x42000);
        cpu.set_reg(3, 22);
        let second = scheduler.adopt(&cpu, (0x41000, 0x42000)).unwrap();
        scheduler.set_priority(second, 0x20ff).unwrap();
        assert_eq!(scheduler.select(&mut cpu).unwrap(), Some(second));
        assert_eq!(cpu.reg(3), 22);
        let old = scheduler.priority(second).unwrap();
        assert!(scheduler.set_priority(second, 0).is_err());
        assert_eq!(scheduler.priority(second).unwrap(), old);
        scheduler.set_priority(second, 0x1ff).unwrap();
        assert_eq!(scheduler.current(), Some(second));
        scheduler.yield_current(&cpu).unwrap();
        assert_eq!(scheduler.select(&mut cpu).unwrap(), Some(first));
        assert_eq!(cpu.reg(3), 11);
    }
    #[test]
    fn actual_scalar_simd_flags_sp_and_thread_register_switch() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000, 8192, 3).unwrap();
        cpu.map_zeroed(0x50000, 4096, 5).unwrap();
        cpu.write_bytes(0x50000, &0xd53bd060u32.to_le_bytes());
        let mut s = CpuScheduler::default();
        cpu.set_pc(0x50000);
        cpu.set_sp(0x41000);
        cpu.set_reg(3, 11);
        cpu.set_vector(7, [111, 222]);
        cpu.set_pstate(0xa0000000);
        cpu.set_tpidrro_el0(0xabc);
        let a = s.adopt(&cpu, (0x40000, 0x41000)).unwrap();
        cpu.set_sp(0x42000);
        cpu.set_reg(3, 22);
        cpu.set_vector(7, [333, 444]);
        cpu.set_pstate(0x60000000);
        cpu.set_tpidrro_el0(0xdef);
        let b = s.adopt(&cpu, (0x41000, 0x42000)).unwrap();
        assert_eq!(s.select(&mut cpu).unwrap(), Some(a));
        assert_eq!(cpu.reg(3), 11);
        assert_eq!(cpu.vector(7), [111, 222]);
        assert_eq!(cpu.sp(), 0x41000);
        assert_eq!(cpu.pstate() & 0xf0000000, 0xa0000000);
        cpu.run_or_step(None);
        assert_eq!(cpu.reg(0), 0xabc);
        let token = s.block_current(&cpu, BlockReason::External(1)).unwrap();
        assert_eq!(s.select(&mut cpu).unwrap(), Some(b));
        assert_eq!(cpu.reg(3), 22);
        assert_eq!(cpu.vector(7), [333, 444]);
        assert_eq!(cpu.sp(), 0x42000);
        cpu.run_or_step(None);
        assert_eq!(cpu.reg(0), 0xdef);
        s.yield_current(&cpu).unwrap();
        s.wake(token).unwrap();
        assert_eq!(s.select(&mut cpu).unwrap(), Some(b));
        s.yield_current(&cpu).unwrap();
        assert_eq!(s.select(&mut cpu).unwrap(), Some(a));
        assert_eq!(cpu.reg(0), 0xabc);
    }
    #[test]
    fn actual_tls_destructor_observed_before_exit() {
        let mut cpu = A64Cpu::new_sparse();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        cpu.map_zeroed(0x40000, 8192, 3).unwrap();
        cpu.map_zeroed(0x50000, 4096, 5).unwrap();
        let code = [0xd2a00081u32, 0xf9000020, 0xd65f03c0];
        let bytes: Vec<u8> = code.iter().flat_map(|x| x.to_le_bytes()).collect();
        cpu.write_bytes(0x50000, &bytes);
        cpu.set_pc(0x50000);
        cpu.set_sp(0x42000);
        let mut s = CpuScheduler::default();
        let id = s.adopt(&cpu, (0x41000, 0x42000)).unwrap();
        let key = s.create_key(0x50000).unwrap();
        s.set_tls(id, key, 42).unwrap();
        s.select(&mut cpu).unwrap();
        s.begin_exit(&cpu).unwrap();
        assert_eq!(s.phase(id).unwrap(), Phase::Exiting);
        s.finish_exit(&mut cpu, &mut bridge, id, 8, 100).unwrap();
        assert_eq!(cpu.read_u64(0x40000), Some(42));
        assert_eq!(s.phase(id).unwrap(), Phase::Exited);
        assert!(s.get_tls(id, key).is_err());
        assert!(s.select(&mut cpu).unwrap().is_none());
    }
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Process registration and supported virtual SET_SELF QoS transaction.
//! No positive BSD366 feature mask is returned: voucher/workqueue/fixed-priority
//! transactions remain unavailable. A registration record is not syscall success.
use super::thread_scheduler_cpu::{Priority, Scheduler, ThreadId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub thread_start: u64,
    pub workqueue_start: u64,
    pub pthread_size: u32,
    pub dispatch_offset: u64,
    pub tsd_offset: u32,
    pub return_offset: u32,
    pub mach_thread_offset: u32,
    pub joinable_offset_bits: u32,
    pub quantum_offset: u32,
}
#[derive(Default)]
pub struct ProcessRegistration {
    record: Option<Registration>,
}
impl ProcessRegistration {
    /// Positive BSD366 capabilities require a published registration and the
    /// private receipt belonging to the actual routed control/scheduler owner.
    /// Unknown control commands and unsupported voucher policies remain errors.
    pub fn supported_features(
        &self,
        control: &super::bsdthread_ctl::InstalledCtl,
        owner: &std::rc::Rc<std::cell::RefCell<super::thread_scheduler_cpu::CpuScheduler>>,
    ) -> Result<u32, String> {
        if self.record.is_none() {
            return Err("pthread capabilities require completed registration copyout".into());
        }
        control.validate(owner)?;
        const FINE_PRIORITY: u32 = 0x2;
        const CONTROL_DISPATCH: u32 = 0x4;
        const SET_SELF_QOS: u32 = 0x8;
        const MAINTENANCE: u32 = 0x10;
        const DEFAULT_QOS: u32 = 0x40000000;
        Ok(FINE_PRIORITY | CONTROL_DISPATCH | SET_SELF_QOS | MAINTENANCE | DEFAULT_QOS)
    }
    /// Publish only after genuine normalized kernel response copyout succeeds.
    /// `write_atomic` must preflight the complete guest range and either commit
    /// all 56 bytes or preserve them. This does not advertise syscall features.
    pub fn register_copyout(
        &mut self,
        args: [u64; 6],
        data: &[u8],
        executable: impl FnMut(u64) -> bool,
        current_priority: Priority,
        mut write_atomic: impl FnMut(u64, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.record.is_some() {
            return Err("process pthread registration already exists".into());
        }
        // Revalidate the scheduler policy value rather than allowing malformed
        // externally constructed Priority fields to become guest state.
        let encoded = current_priority.encode();
        if Priority::decode(u64::from(encoded))? != current_priority {
            return Err("invalid current thread priority".into());
        }
        let mut candidate = Self::default();
        candidate.register(args, data, executable)?;
        let r = candidate.record.as_ref().unwrap();
        let mut response: [u8; 56] = data
            .try_into()
            .map_err(|_| "invalid registration response size")?;
        response[8..16].copy_from_slice(&r.dispatch_offset.to_le_bytes());
        response[16..24].copy_from_slice(&u64::from(encoded).to_le_bytes());
        response[24..28].copy_from_slice(&r.tsd_offset.to_le_bytes());
        response[28..32].copy_from_slice(&r.return_offset.to_le_bytes());
        response[32..36].copy_from_slice(&r.mach_thread_offset.to_le_bytes());
        response[36..44].fill(0); // future stack allocation hint genuinely unspecified
        response[44..48].fill(0); // default guest mutex policy, no ulock/adaptive support claim
        response[52..56].copy_from_slice(&r.quantum_offset.to_le_bytes());
        write_atomic(args[3], &response)?;
        self.record = candidate.record;
        Ok(())
    }
    pub fn record(&self) -> Option<&Registration> {
        self.record.as_ref()
    }
    /// Bounded exact modern 56-byte input, validated before publication. RX
    /// validation is physical ABI validation, not permission to run callbacks.
    pub fn register(
        &mut self,
        args: [u64; 6],
        data: &[u8],
        mut executable: impl FnMut(u64) -> bool,
    ) -> Result<(), String> {
        if self.record.is_some() {
            return Err("process pthread registration already exists".into());
        }
        if args[3] == 0 || args[4] != 56 || data.len() != 56 || args[2] == 0 || args[2] > 65536 {
            return Err("unsupported/invalid pthread registration layout or size".into());
        }
        let u32_at = |p| u32::from_le_bytes(data[p..p + 4].try_into().unwrap());
        let u64_at = |p| u64::from_le_bytes(data[p..p + 8].try_into().unwrap());
        if u64_at(0) != 56 || !executable(args[0]) || !executable(args[1]) {
            return Err("invalid pthread registration version/entrypoints".into());
        }
        let size = args[2] as u32;
        let tsd = u32_at(24);
        if tsd % 8 != 0 || tsd.checked_add(8).is_none_or(|end| end > size) {
            return Err("invalid pthread TSD base offset".into());
        }
        let max = size - tsd - 8;
        let bounded = |v: u32| if v <= max { v } else { 0 };
        let dispatch = u64_at(8);
        self.record = Some(Registration {
            thread_start: args[0],
            workqueue_start: args[1],
            pthread_size: size,
            dispatch_offset: if dispatch <= u64::from(max) {
                dispatch
            } else {
                0
            },
            tsd_offset: tsd,
            return_offset: bounded(u32_at(28)),
            mach_thread_offset: bounded(u32_at(32)),
            joinable_offset_bits: u32_at(48),
            quantum_offset: bounded(u32_at(52)),
        });
        Ok(())
    }
    /// BSDTHREAD_CTL_SET_SELF (command0x100), QoS-only flag0x1 subset. Applies
    /// to the actual current cooperative CPU owner, never a supplied foreign ID.
    /// Guest libpthread updates its own TSD after the genuine syscall result.
    pub fn set_self_qos<C>(
        &self,
        scheduler: &mut Scheduler<C>,
        priority: u64,
        voucher: u64,
        flags: u64,
    ) -> Result<(), String> {
        if self.record.is_none() {
            return Err("SET_SELF requires process pthread registration".into());
        }
        if flags != 1 || voucher != 0 {
            return Err("SET_SELF voucher/fixed/workqueue/override policy unsupported".into());
        }
        let current: ThreadId = scheduler
            .current()
            .ok_or("SET_SELF requires actual running thread")?;
        scheduler.set_priority(current, priority)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_copyout_uses_real_priority_and_publishes_only_after_write() {
        let mut p = ProcessRegistration::default();
        let args = [1, 2, 0x4000, 0x3000, 56, 160];
        let priority = Priority::decode(0x8ff).unwrap();
        assert!(p
            .register_copyout(
                args,
                &data(),
                |_| true,
                priority,
                |_, _| Err("EFAULT".into())
            )
            .is_err());
        assert!(p.record().is_none());
        let mut output = Vec::new();
        p.register_copyout(
            args,
            &data(),
            |_| true,
            priority,
            |address, bytes| {
                assert_eq!(address, 0x3000);
                output = bytes.to_vec();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            u64::from_le_bytes(output[16..24].try_into().unwrap()),
            0x8ff
        );
        assert!(output[36..48].iter().all(|b| *b == 0));
        assert!(p.record().is_some());
    }
    fn data() -> [u8; 56] {
        let mut b = [0; 56];
        b[..8].copy_from_slice(&56u64.to_le_bytes());
        b[8..16].copy_from_slice(&160u64.to_le_bytes());
        b[24..28].copy_from_slice(&224u32.to_le_bytes());
        b[32..36].copy_from_slice(&24u32.to_le_bytes());
        b
    }
    #[test]
    fn registration_is_bounded_unique_and_failure_atomic() {
        let mut p = ProcessRegistration::default();
        let args = [0x1000, 0x2000, 0x4000, 0x3000, 56, 160];
        assert!(p.register(args, &data(), |_| false).is_err());
        assert!(p.record().is_none());
        let mut invalid = data();
        invalid[24..28].copy_from_slice(&0x4000u32.to_le_bytes());
        assert!(p.register(args, &invalid, |_| true).is_err());
        assert!(p.record().is_none());
        p.register(args, &data(), |v| v == 0x1000 || v == 0x2000)
            .unwrap();
        let old = p.record().unwrap().clone();
        assert!(p.register(args, &data(), |_| true).is_err());
        assert_eq!(p.record(), Some(&old));
    }
    #[test]
    fn actual_ios16_registration_fields_preserve_kernel_relative_offsets() {
        // Scalar ABI capture from genuine ___pthread_init, no executable bytes.
        let mut bytes = data();
        bytes[28..32].copy_from_slice(&40u32.to_le_bytes());
        bytes[48..52].copy_from_slice(&392u32.to_le_bytes());
        bytes[52..56].copy_from_slice(&960u32.to_le_bytes());
        let args = [0x1d1b96724, 0x1d1b96718, 0x4000, 0x22f854ec0, 56, 160];
        let mut process = ProcessRegistration::default();
        process
            .register(args, &bytes, |entry| [args[0], args[1]].contains(&entry))
            .unwrap();
        let record = process.record().unwrap();
        assert_eq!(
            (
                record.dispatch_offset,
                record.tsd_offset,
                record.return_offset
            ),
            (160, 224, 40)
        );
        assert_eq!(
            (
                record.mach_thread_offset,
                record.joinable_offset_bits,
                record.quantum_offset
            ),
            (24, 392, 960)
        );
    }
    #[test]
    fn set_self_changes_actual_owner_selection_and_rejects_partial_policies() {
        let mut p = ProcessRegistration::default();
        p.register([1, 2, 0x4000, 3, 56, 160], &data(), |_| true)
            .unwrap();
        let mut s = Scheduler::default();
        let first = s.register(10).unwrap();
        let second = s.register(20).unwrap();
        assert!(p.set_self_qos(&mut s, 0x20ff, 0, 1).is_err());
        assert_eq!(s.select().unwrap(), Some((first, &10)));
        let old = s.priority(first).unwrap();
        assert!(p.set_self_qos(&mut s, 0x20ff, 1, 3).is_err());
        assert_eq!(s.priority(first).unwrap(), old);
        p.set_self_qos(&mut s, 0x20ff, 0, 1).unwrap();
        s.yield_current(11).unwrap();
        assert_eq!(s.select().unwrap(), Some((first, &11)));
        assert_eq!(s.priority(second).unwrap(), old);
    }
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Bounded BSD478 control service attached to the actual selected CPU owner.
use super::{pthread_registration::ProcessRegistration, thread_scheduler_cpu::{CpuScheduler, Priority, ThreadId}, A64Cpu};
use std::{cell::RefCell, rc::{Rc, Weak}};

pub(super) struct InstalledCtl {
    scheduler: Weak<RefCell<CpuScheduler>>,
    owner: ThreadId,
}
impl InstalledCtl {
    pub(super) fn validate(&self, scheduler: &Rc<RefCell<CpuScheduler>>) -> Result<(), String> {
        let original = self.scheduler.upgrade().ok_or("control scheduler no longer exists")?;
        if !Rc::ptr_eq(&original, scheduler) { return Err("control belongs to another scheduler".into()); }
        let state = scheduler.borrow();
        if state.current() != Some(self.owner) { return Err("installed control owner is not selected".into()); }
        state.priority(self.owner)?;
        // These are the real decoder used by the scheduling service, including
        // the entire supported negative relative-priority boundary.
        for value in [0x8ff, 0x1ff, 0x8f0, 0x1f0] { Priority::decode(value)?; }
        Ok(())
    }
}
pub(super) struct OwnedControl {
    scheduler: Rc<RefCell<CpuScheduler>>,
    capability: InstalledCtl,
}
impl OwnedControl {
    pub(super) fn install(scheduler: Rc<RefCell<CpuScheduler>>) -> Result<Self, String> {
        let owner = scheduler.borrow().current().ok_or("BSD478 needs a selected CPU owner")?;
        let capability = InstalledCtl { scheduler: Rc::downgrade(&scheduler), owner };
        capability.validate(&scheduler)?;
        Ok(Self { scheduler, capability })
    }
    pub(super) fn capability(&self) -> &InstalledCtl { &self.capability }
    pub(super) fn call(&self, cpu: &mut A64Cpu, registration: &ProcessRegistration, args: [u64;4]) -> Result<(),String> {
        self.capability.validate(&self.scheduler)?;
        let [command, priority, voucher, flags] = args;
        let errno = match command {
            // Modern XNU explicitly retired SET_QOS; GET_QOS is not a command.
            0x10 => Some(45), // ENOTSUP
            0x100 => {
                if registration.record().is_none() { Some(22) }
                else if flags & !0x7f != 0 { Some(22) }
                // XNU initializes all operation errors to zero and performs
                // only flag-selected operations: flags zero is a real no-op.
                else if flags == 0 { None }
                else if flags != 1 { Some(45) }
                else if Priority::decode(priority).is_err() { Some(22) }
                else {
                    // Voucher argument is unused unless SET_VOUCHER is set.
                    let _ = voucher;
                    let mut scheduler = self.scheduler.borrow_mut();
                    scheduler.set_priority(self.capability.owner, priority)?;
                    None
                }
            }
            _ => Some(22),
        };
        let carry = 1 << 29;
        cpu.set_reg(0, errno.unwrap_or(0));
        cpu.set_pstate(if errno.is_some() { cpu.pstate() | carry } else { cpu.pstate() & !carry });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn selected() -> (A64Cpu,Rc<RefCell<CpuScheduler>>) {
        let mut cpu=A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000,4096,3).unwrap();
        cpu.map_zeroed(0x50000,4096,5).unwrap();
        cpu.set_pc(0x50000); cpu.set_sp(0x41000);
        let scheduler=Rc::new(RefCell::new(CpuScheduler::default()));
        scheduler.borrow_mut().adopt(&cpu,(0x40000,0x41000)).unwrap();
        scheduler.borrow_mut().select(&mut cpu).unwrap();
        (cpu,scheduler)
    }
    #[test]
    fn capability_rejects_another_scheduler_and_missing_owner() {
        let (mut cpu,scheduler)=selected();
        let control=OwnedControl::install(scheduler.clone()).unwrap();
        let (_,other)=selected();
        assert!(control.capability().validate(&other).is_err());
        scheduler.borrow_mut().yield_current(&cpu).unwrap();
        assert!(control.call(&mut cpu,&ProcessRegistration::default(),[0x10,0,0,0]).is_err());
        assert!(OwnedControl::install(scheduler).is_err());
    }
    #[test]
    fn qos_changes_the_owned_scheduler_and_rejects_bad_priority_without_mutation() {
        let (mut cpu,scheduler)=selected();
        let control=OwnedControl::install(scheduler.clone()).unwrap();
        let mut registration=ProcessRegistration::default();
        assert!(registration.supported_features(control.capability(),&scheduler).is_err());
        let mut data=[0;56];
        data[..8].copy_from_slice(&56u64.to_le_bytes());
        data[8..16].copy_from_slice(&160u64.to_le_bytes());
        data[24..28].copy_from_slice(&224u32.to_le_bytes());
        data[32..36].copy_from_slice(&24u32.to_le_bytes());
        let priority=scheduler.borrow().priority(scheduler.borrow().current().unwrap()).unwrap();
        registration.register_copyout([0x50000,0x50000,0x4000,0x40000,56,160],&data,|p|p==0x50000,priority,|address,bytes| {
            cpu.validate_guest_write(address,bytes.len())?;
            cpu.write_bytes(address,bytes);
            Ok(())
        }).unwrap();
        assert_eq!(registration.supported_features(control.capability(),&scheduler).unwrap(),0x4000001e);
        let (_,foreign)=selected();
        assert!(registration.supported_features(control.capability(),&foreign).is_err());
        let id=scheduler.borrow().current().unwrap();
        cpu.set_pstate(0xb0000000);
        control.call(&mut cpu,&registration,[0x100,0x1f0,99,1]).unwrap();
        assert_eq!(scheduler.borrow().priority(id).unwrap(),Priority::decode(0x1f0).unwrap());
        assert_eq!(cpu.reg(0),0);
        assert_eq!(cpu.pstate()&0xf0000000,0x90000000);
        control.call(&mut cpu,&registration,[0x100,0,0,1]).unwrap();
        assert_eq!(cpu.reg(0),22);
        assert_eq!(scheduler.borrow().priority(id).unwrap(),Priority::decode(0x1f0).unwrap());
        control.call(&mut cpu,&registration,[0x100,0x8ff,0,4]).unwrap();
        assert_eq!(cpu.reg(0),45);
        assert_eq!(scheduler.borrow().priority(id).unwrap(),Priority::decode(0x1f0).unwrap());
        control.call(&mut cpu,&registration,[0x100,u64::MAX,u64::MAX,0]).unwrap();
        assert_eq!(cpu.reg(0),0);
        assert_eq!(cpu.pstate() & (1<<29),0);
        assert_eq!(scheduler.borrow().priority(id).unwrap(),Priority::decode(0x1f0).unwrap());
        scheduler.borrow_mut().yield_current(&cpu).unwrap();
        assert!(registration.supported_features(control.capability(),&scheduler).is_err());
    }
    #[test]
    fn unsupported_commands_report_bsd_errno_and_preserve_other_flags() {
        let (mut cpu,scheduler)=selected();
        let control=OwnedControl::install(scheduler.clone()).unwrap();
        let id=scheduler.borrow().current().unwrap();
        let old=scheduler.borrow().priority(id).unwrap();
        for (command,error) in [(0x10,45),(0x20,22),(0x999,22)] {
            cpu.set_pstate(0x90000000);
            control.call(&mut cpu,&ProcessRegistration::default(),[command,0,0,0]).unwrap();
            assert_eq!(cpu.reg(0),error);
            assert_eq!(cpu.pstate()&0xf0000000,0xb0000000);
            assert_eq!(scheduler.borrow().priority(id).unwrap(),old);
        }
    }
}

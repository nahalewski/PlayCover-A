/* This Source Code Form is subject to the Mozilla Public License, v. 2.0.
 * https://mozilla.org/MPL/2.0/ */
//! Darwin thread_selfid reads the actual selected virtual kernel thread ID.
use super::{A64Cpu,thread_scheduler_cpu::{CpuScheduler,ThreadId}};
use std::{rc::Rc,cell::RefCell};
pub(super) fn self_id(cpu:&mut A64Cpu,scheduler:&Rc<RefCell<CpuScheduler>>,owner:ThreadId)->Result<(),String>{
 let current=scheduler.try_borrow().map_err(|_|"thread identity scheduler borrowed")?.current().ok_or("thread_selfid requires selected kernel thread")?;
 if current!=owner||current.0==0{return Err("thread_selfid current process owner mismatch".into());}
 // XNU/libpthread _thread_selfid returns thread_tid(current_thread()).
 // This ID comes from the actual virtual scheduler's monotonic identity
 // allocator; it is neither a Mach port name nor a guest pthread address.
 cpu.set_reg(0,current.0);cpu.set_pstate(cpu.pstate()&!(1<<29));Ok(())
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn identifiers_follow_real_selected_context_and_remain_stable(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();cpu.map_zeroed(0x20000,0x8000,3).unwrap();cpu.set_pc(0x10000);cpu.set_sp(0x23ff0);
  let mut state=CpuScheduler::default();let first=state.adopt(&cpu,(0x20000,0x24000)).unwrap();cpu.set_sp(0x27ff0);let second=state.adopt(&cpu,(0x24000,0x28000)).unwrap();assert_ne!(first,second);
  let scheduler=Rc::new(RefCell::new(state));assert_eq!(scheduler.borrow_mut().select(&mut cpu).unwrap(),Some(first));cpu.set_pstate(0xb0000000);cpu.set_reg(1,0x1b3288b40);
  self_id(&mut cpu,&scheduler,first).unwrap();assert_eq!(cpu.reg(0),first.0);assert_eq!(cpu.pstate(),0x90000000);assert_eq!(cpu.reg(1),0x1b3288b40);self_id(&mut cpu,&scheduler,first).unwrap();assert_eq!(cpu.reg(0),first.0);
  scheduler.borrow_mut().yield_current(&cpu).unwrap();assert_eq!(scheduler.borrow_mut().select(&mut cpu).unwrap(),Some(second));self_id(&mut cpu,&scheduler,second).unwrap();assert_eq!(cpu.reg(0),second.0);
 }
 #[test]fn missing_or_foreign_current_owner_never_returns_invented_identity(){
  let mut cpu=A64Cpu::new_sparse();let scheduler=Rc::new(RefCell::new(CpuScheduler::default()));cpu.set_reg(0,0x123);cpu.set_pstate(0x20000000);
  assert!(self_id(&mut cpu,&scheduler,ThreadId(1)).is_err());assert_eq!(cpu.reg(0),0x123);assert_eq!(cpu.pstate(),0x20000000);
 }
}

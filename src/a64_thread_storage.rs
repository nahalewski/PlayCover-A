/* This Source Code Form is subject to the Mozilla Public License, v. 2.0.
 * https://mozilla.org/MPL/2.0/ */
//! Session-owned journal of an actual validated kernel thread-register change.
use super::{A64Cpu,thread_scheduler_cpu::{CpuScheduler,ThreadId},mach_identity::{MachIdentity,PortRight}};
use std::{cell::{Cell,RefCell},rc::Rc};
pub(super) struct ThreadStorage {owner:ThreadId,current:Cell<u64>,legacy:Cell<bool>,changed:Cell<bool>}
const PLATFORM_STUB:[u8;16]=[0x43,0,0x80,0xd2,0x10,0,0xb0,0xd2,1,0x10,0,0xd4,0xc0,3,0x5f,0xd6];
impl ThreadStorage {
 pub(super) fn new(owner:ThreadId,tsd:u64)->Self{Self{owner,current:Cell::new(tsd),legacy:Cell::new(false),changed:Cell::new(false)}}
 pub(super) fn enable_legacy(&self){self.legacy.set(true);}
 pub(super) fn current(&self)->u64{self.current.get()}
 pub(super) fn effect(&self)->Option<u64>{self.changed.get().then(||self.current.get())}
 pub(super) fn original_adopted(&self)->bool{self.legacy.get()&&self.changed.get()&&self.current.get()==0x1b3288c20}
 pub(super) fn adopt_original(&self,cpu:&mut A64Cpu,scheduler:&Rc<RefCell<CpuScheduler>>,ports:&MachIdentity)->Result<(),String>{
  let record=0x1b3288b40u64;let tsd=record+224;
  if !self.legacy.get()||cpu.pc()!=0x18097f540||cpu.reg(3)!=2||cpu.reg(0)!=tsd||cpu.tpidrro_el0()!=self.current.get()
   ||cpu.mapped_permissions(0x18097f534).is_none_or(|p|p&4==0)||cpu.read_bytes(0x18097f534,16).is_none_or(|bytes|bytes!=PLATFORM_STUB)
   ||scheduler.try_borrow().map_err(|_|"TSD adoption scheduler borrowed")?.current()!=Some(self.owner){return Err("unauthorized original kernel thread-storage transition".into());}
  cpu.validate_guest_write(record,0x100)?;
  if cpu.read_u64(record+0xd8)!=Some(self.owner.0)||cpu.read_u64(tsd)!=Some(record)||cpu.read_u64(tsd+8)!=Some(record+0x48){return Err("original pthread thread-ID/self/errno metadata differs".into());}
  // The original caller has not initialized slot3 yet. Preserve its actual
  // zero; if already present it must refer to this thread's real Mach right.
  let port=cpu.read_u64(tsd+24).ok_or("original pthread port slot unreadable")?;
  if port!=0&&!u32::try_from(port).ok().and_then(|name|ports.right(name)).is_some_and(|right|matches!(right,PortRight::ThreadSend{thread,references} if thread==self.owner.0&&references>0)){
   return Err("original pthread Mach name belongs to another owner".into());
  }
  self.current.set(tsd);self.changed.set(true);cpu.set_tpidrro_el0(tsd);cpu.set_reg(0,0);cpu.set_pstate(cpu.pstate()&!(1<<29));Ok(())
 }
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn original_record_is_verified_before_committing_thread_register(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();cpu.map_zeroed(0x18097f000,4096,5).unwrap();cpu.try_write_bytes(0x18097f534,&PLATFORM_STUB).unwrap();cpu.map_zeroed(0x20000,4096,3).unwrap();cpu.map_zeroed(0x1b3288000,8192,3).unwrap();cpu.set_pc(0x10000);cpu.set_sp(0x20ff0);
  let mut scheduler=CpuScheduler::default();let owner=scheduler.adopt(&cpu,(0x20000,0x21000)).unwrap();scheduler.select(&mut cpu).unwrap();let scheduler=Rc::new(RefCell::new(scheduler));let mut ports=MachIdentity::new(8).unwrap();ports.register_thread(owner.0).unwrap();
  let state=ThreadStorage::new(owner,0x200e0);state.enable_legacy();let record=0x1b3288b40u64;let tsd=record+224;
  cpu.set_tpidrro_el0(0x200e0);cpu.set_pc(0x18097f540);cpu.set_reg(0,tsd);cpu.set_reg(3,2);
  cpu.write_guest_into(record+0xd8,&owner.0.to_le_bytes()).unwrap();cpu.write_guest_into(tsd,&record.to_le_bytes()).unwrap();cpu.write_guest_into(tsd+8,&(record+0x48).to_le_bytes()).unwrap();
  cpu.write_guest_into(tsd,&0x123u64.to_le_bytes()).unwrap();assert!(state.adopt_original(&mut cpu,&scheduler,&ports).is_err());assert_eq!(state.current(),0x200e0);assert_eq!(cpu.tpidrro_el0(),0x200e0);
  cpu.write_guest_into(tsd,&record.to_le_bytes()).unwrap();
  for(address,wrong,correct)in[(record+0xd8,owner.0+1,owner.0),(tsd+8,record+0x49,record+0x48),(tsd+24,0xdead,0)]{
   cpu.write_guest_into(address,&wrong.to_le_bytes()).unwrap();assert!(state.adopt_original(&mut cpu,&scheduler,&ports).is_err());assert_eq!(state.current(),0x200e0);assert_eq!(cpu.tpidrro_el0(),0x200e0);cpu.write_guest_into(address,&correct.to_le_bytes()).unwrap();
  }
  cpu.set_reg(0,tsd+16);assert!(state.adopt_original(&mut cpu,&scheduler,&ports).is_err());cpu.set_reg(0,tsd);
  let foreign=ThreadStorage::new(ThreadId(owner.0+1),0x200e0);foreign.enable_legacy();assert!(foreign.adopt_original(&mut cpu,&scheduler,&ports).is_err());
  state.adopt_original(&mut cpu,&scheduler,&ports).unwrap();assert_eq!(state.effect(),Some(tsd));assert_eq!(cpu.tpidrro_el0(),tsd);assert_eq!(cpu.read_u64(tsd+24),Some(0));
 }
}

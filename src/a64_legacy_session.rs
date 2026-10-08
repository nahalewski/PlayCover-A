/* This Source Code Form is subject to the Mozilla Public License, v. 2.0.
 * https://mozilla.org/MPL/2.0/ */
//! Original15G77 execution protocol; no modern libdyld redirects or readiness.
use super::{A64Cpu,cache::CachePlan,host_services::SelectedServices,main_stack::MainStackDescriptor,
 cache_init_probe::PreparedInitialization,execution_session::{ExecutionSession,ProcessState},bridge::GuestCall};
use std::{rc::Rc,cell::RefCell,io::{Read,Seek,SeekFrom}};
const UUID:[u8;16]=[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f];
const ENTRY:u64=0x18002ca8c;
const ORIGINAL:[u8;32]=[0xf6,0x57,0xbd,0xa9,0xf4,0x4f,1,0xa9,0xfd,0x7b,2,0xa9,0xfd,0x83,0,0x91,0xf3,3,4,0xaa,0xf4,3,3,0xaa,0xf5,3,2,0xaa,0xe0,0xbb,0x15,0x90];
fn validate_entry(cpu:&A64Cpu,entry:u64)->Result<(),String>{
 if entry!=ENTRY||cpu.mapped_permissions(entry).is_none_or(|p|p&4==0)
  ||cpu.read_bytes(entry,ORIGINAL.len()).ok_or("original11 initializer unreadable")?!=ORIGINAL {
  return Err("original15G77 libSystem initializer identity/instructions differ".into());
 }Ok(())
}
fn bootstrap_extent(cpu:&A64Cpu,start:u64)->Result<u64,String>{
 let mut base=start.checked_add(4095).ok_or("legacy bootstrap arena overflow")?&!4095;
 let limit=base.checked_add(16*1024*1024).filter(|end|*end<=1u64<<48).ok_or("legacy bootstrap arena exceeds guest address space")?;
 for _ in 0..4096 {
  let end=base.checked_add(8192).filter(|end|*end<=limit).ok_or("legacy bootstrap free extent budget exhausted")?;
  if let Some(region)=(base..end).find_map(|address|cpu.protection_region(address)) {
   base=region.base.checked_add(region.len).and_then(|end|end.checked_add(4095)).ok_or("legacy bootstrap existing extent overflow")?&!4095;
  }else{return Ok(base);}
 }Err("legacy bootstrap free extent search budget exhausted".into())
}
pub(super) fn prepare(cpu:&mut A64Cpu,plan:&CachePlan,_services:&mut SelectedServices,arguments:Vec<u64>,main_stack:&MainStackDescriptor)->Result<PreparedInitialization,String>{
 let mut file=std::fs::File::open(plan.files.first().ok_or("original11 cache file absent")?).map_err(|e|e.to_string())?;
 file.seek(SeekFrom::Start(88)).map_err(|e|e.to_string())?;let mut uuid=[0;16];file.read_exact(&mut uuid).map_err(|e|e.to_string())?;
 if uuid!=UUID{return Err("legacy execution requires verified original15G77 cache UUID".into());}
 if arguments.len()!=5||cpu.read_u64(arguments[2])!=Some(0){return Err("legacy initializer requires genuine five arguments and empty environment".into());}
 let functions=super::cache_initializers::functions(cpu,plan,"/usr/lib/libSystem.B.dylib")?;
 if functions!=[ENTRY]{return Err("original11 libSystem initializer list differs".into());}validate_entry(cpu,functions[0])?;
 // The original kernel initializer consumes this original flat helper table.
 // Retain it unchanged; do not publish a modern C++ helper object/vtable.
 cpu.read_bytes(0x1ab7a8148,168).ok_or("original11 kernel callback table absent")?;
 super::commpage_ro::ensure_mapped(cpu)?;
 let mut ports=super::mach_identity::MachIdentity::new(64)?;let thread=1;ports.register_thread(thread)?;
 let thread_port=ports.thread_self(thread)?;super::cache_init_probe::append_thread_apple(cpu,&arguments,thread_port)?;
 let base=bootstrap_extent(cpu,arguments[4].checked_add(8191).ok_or("legacy bootstrap TSD overflow")?&!4095)?;
 let tsd=super::cache_init_probe::primordial_tsd(cpu,base,thread_port).map_err(|error|format!("legacy bootstrap TSD mapping {base:#x}..{:#x}: {error}",base+8192))?;
 let saved=cpu.save_context();cpu.set_tpidrro_el0(tsd);cpu.set_tpidr_el0(0);
 let result=(||{
  if main_stack.owner()!=thread||cpu.sp()!=main_stack.initial_sp(){return Err("legacy primordial CPU does not match actual retained loader stack".into());}
  let mut scheduler=super::thread_scheduler_cpu::CpuScheduler::default();let owner=scheduler.adopt(cpu,main_stack.bounds())?;
  if owner.0!=thread||scheduler.select(cpu)?!=Some(owner){return Err("legacy primordial scheduler ownership mismatch".into());}
  let scheduler=Rc::new(RefCell::new(scheduler));let control=super::bsdthread_ctl::OwnedControl::install(scheduler.clone())?;
  let process=ProcessState{identity:super::execution_session::ProcessIdentity::new(&scheduler)?,ports,vm:super::mach_vm::AnonymousVm::new(0x20_0000_0000,0x20_1000_0000)?,
   priorities:super::mach_host_info::HostPriorityPolicy::virtual_darwin(),clock:super::mach_clock::SystemClock::new(),
   semaphores:super::mach_semaphore::SemaphoreService::new(),entropy:super::entropy_fd::EntropyFds::new(),
   standard_fds:super::standard_fds::StandardFds::new(),shared_memory:super::posix_shm::ShmNamespace::new_isolated(),
   credentials:super::credentials::CredentialTaint::isolated_unprivileged(),scheduler,control,
   registration:super::pthread_registration::ProcessRegistration::default(),thread,owner,trap_count:0};
  let session=ExecutionSession::from_prepared(cpu,process,tsd)?;
  session.enable_legacy_thread_storage();
  echo!("[a64] original15G77 initializer entry={ENTRY:#x}; real primordial bootstrap owner/TSD retained; original flat callbacks unchanged, no pthread/helper/runtime readiness");
  Ok(PreparedInitialization{session,call:GuestCall{entry:ENTRY,integers:arguments,..Default::default()}})
 })();if result.is_err(){cpu.restore_context(&saved);}result
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn bootstrap_reservation_skips_actual_owned_maps_and_rejects_overflow(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x20000,4096,1).unwrap();cpu.map_zeroed(0x22000,4096,5).unwrap();
  assert_eq!(bootstrap_extent(&cpu,0x20000).unwrap(),0x23000);
  assert_eq!(cpu.mapped_permissions(0x20000),Some(1));assert_eq!(cpu.mapped_permissions(0x22000),Some(5));
  assert!(bootstrap_extent(&cpu,u64::MAX).is_err());
 }
 #[test]fn rejected_cache_provenance_preserves_prepared_cpu_context(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x30000,0x4000,3).unwrap();
  let stack=MainStackDescriptor::from_loader(&cpu,1,0x30000,0x4000,0x30000,0x4000,0x33ff0).unwrap();
  let mut services=SelectedServices::install(&mut cpu,0x100000,super::super::host_services::Selection{core_foundation:false,objc_lifetime:true},super::super::cf_terraria_services::KnownConstants::default()).unwrap();
  cpu.set_pc(0x12340);cpu.set_sp(0x33ff0);cpu.set_reg(0,0x55);cpu.set_reg(A64Cpu::LR,0x67890);cpu.set_tpidrro_el0(0xabcdef0);
  let path=std::env::temp_dir().join(format!("playcover-legacy-invalid-uuid-{}.bin",std::process::id()));std::fs::write(&path,[0u8;104]).unwrap();
  let plan=CachePlan{files:vec![path.clone()],image_count:0,images:vec![],mapped_bytes:0,mapped_span:0,regions:vec![]};
  let result=prepare(&mut cpu,&plan,&mut services,vec![0;5],&stack);std::fs::remove_file(path).unwrap();
  assert!(result.err().unwrap().contains("cache UUID"));assert_eq!(cpu.pc(),0x12340);assert_eq!(cpu.sp(),0x33ff0);assert_eq!(cpu.reg(0),0x55);assert_eq!(cpu.reg(A64Cpu::LR),0x67890);assert_eq!(cpu.tpidrro_el0(),0xabcdef0);
 }
 #[test]fn original_reply_port_trap_allocates_receive_right_and_restores_caller_context(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();
  for(i,w)in[0x92800330u32,0xd4001001,0xd65f03c0].iter().enumerate(){cpu.try_write_bytes(0x10000+i as u64*4,&w.to_le_bytes()).unwrap();}
  let mut bridge=super::super::bridge::GuestBridge::map(&mut cpu,0x40000).unwrap();let mut ports=super::super::mach_identity::MachIdentity::new(8).unwrap();
  cpu.set_pc(0x7770);cpu.set_sp(0x8880);cpu.set_reg(A64Cpu::LR,0x9990);
  let value=bridge.call_with_supervisor_handler(&mut cpu,&GuestCall{entry:0x10000,..Default::default()},100,&mut|cpu,immediate|{assert_eq!(immediate,0x80);assert_eq!(cpu.reg(16)as i64,-26);cpu.set_reg(0,u64::from(ports.trap(-26)?));Ok(())}).unwrap().integers[0];
  assert!(matches!(ports.right(value as u32),Some(super::super::mach_identity::PortRight::ReplyReceive)));assert_eq!(cpu.pc(),0x7770);assert_eq!(cpu.sp(),0x8880);assert_eq!(cpu.reg(A64Cpu::LR),0x9990);
 }
 #[test]fn original_initializer_guard_rejects_mutation_without_guest_execution(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(ENTRY&!4095,4096,5).unwrap();cpu.try_write_bytes(ENTRY,&ORIGINAL).unwrap();
  validate_entry(&cpu,ENTRY).unwrap();cpu.try_write_bytes(ENTRY,&0xd503201fu32.to_le_bytes()).unwrap();
  assert!(validate_entry(&cpu,ENTRY).is_err());assert!(validate_entry(&cpu,ENTRY+4).is_err());
 }
 #[test]fn original_task_self_trap_uses_owned_namespace_and_preserves_bootstrap_tsd(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();
  for(i,w)in[0x92800370u32,0xd4001001,0xd65f03c0].iter().enumerate(){cpu.try_write_bytes(0x10000+i as u64*4,&w.to_le_bytes()).unwrap();}
  let tsd=super::super::cache_init_probe::primordial_tsd(&mut cpu,0x20000,0x203).unwrap();cpu.set_tpidrro_el0(tsd);
  let mut bridge=super::super::bridge::GuestBridge::map(&mut cpu,0x40000).unwrap();let mut ports=super::super::mach_identity::MachIdentity::new(8).unwrap();
  let value=bridge.call_with_supervisor_handler(&mut cpu,&GuestCall{entry:0x10000,..Default::default()},100,&mut|cpu,immediate|{
   assert_eq!(immediate,0x80);assert_eq!(cpu.reg(16)as i64,-28);let name=ports.trap(-28)?;cpu.set_reg(0,u64::from(name));Ok(())
  }).unwrap().integers[0];assert!(matches!(ports.right(value as u32),Some(super::super::mach_identity::PortRight::TaskSend{..})));assert_eq!(cpu.tpidrro_el0(),tsd);
 }
}

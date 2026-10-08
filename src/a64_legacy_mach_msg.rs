/* This Source Code Form is subject to the Mozilla Public License, v. 2.0.
 * https://mozilla.org/MPL/2.0/ */
//! Evidenced synchronous old mach_msg trap31 HOST_PRIORITY_INFO operation.
use super::{A64Cpu,mach_identity::MachIdentity,mach_host_info::HostPriorityPolicy};
fn word(bytes:&[u8],offset:usize)->u32{u32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap())}
pub(super) fn reply(cpu:&mut A64Cpu,ports:&mut MachIdentity,args:[u64;8],policy:&HostPriorityPolicy,clock:&super::mach_clock::SystemClock,semaphores:&mut super::mach_semaphore::SemaphoreService)->Result<u32,String>{
 let [data,options,send_size,capacity,receive,timeout,notify,_]=args;
 if options!=3||!matches!(send_size,36|40)||!(48..=4096).contains(&capacity)||receive>u32::MAX as u64||timeout!=0||notify!=0{
  return Err(format!("unsupported original mach_msg scalar envelope: {args:x?}"));
 }
 let mut request=vec![0;send_size as usize];cpu.read_guest_into(data,&mut request)?;
 let bits=word(&request,0);let host=word(&request,8);let reply_port=word(&request,12);let id=word(&request,20);
 if bits!=0x1513||u64::from(reply_port)!=receive||word(&request,16)!=0||!matches!((id,send_size),(200,40)|(206,36)|(3418,40)){
  return Err(format!("unsupported original mach_msg header/RPC id={id}"));
 }
 // Semantic conversion of the kernel-copied scalar envelope; the original
 // request bytes stay untouched until genuine validated response delivery.
 let packed=[data,0x200000003,(send_size<<32)|u64::from(bits),(receive<<32)|u64::from(host),u64::from(id)<<32,receive<<32,capacity,0];
 // XNU mach_msg.c sets the copied header's size from the scalar send_size;
 // the original userspace field (observed0/1) is not authoritative.
 match id{200=>super::mach_host_info::reply_legacy(cpu,ports,packed,policy),206=>clock.reply_legacy(cpu,ports,packed),3418=>semaphores.reply_legacy(cpu,ports,packed),_=>unreachable!()}
}
#[cfg(test)]mod tests{
 use super::*;
 fn fixture()->(A64Cpu,MachIdentity,[u64;8]){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,3).unwrap();let mut ports=MachIdentity::new(8).unwrap();
  let host=ports.trap(-29).unwrap();let receive=ports.trap(-26).unwrap();let mut request=[0;40];
  for(offset,value)in[(0,0x1513u32),(8,host),(12,receive),(20,200),(32,5),(36,8)]{request[offset..offset+4].copy_from_slice(&value.to_le_bytes());}
  request[28]=1;cpu.write_guest_into(0x10000,&request).unwrap();(cpu,ports,[0x10000,3,40,320,u64::from(receive),0,0,0])
 }
 #[test]fn original_zero_size_header_delivers_real_priority_reply_to_ordinary_receive(){
  let(mut cpu,mut ports,args)=fixture();let host=cpu.read_u64(0x10008).unwrap()as u32;let before=ports.right(host);
  assert_eq!(reply(&mut cpu,&mut ports,args,&HostPriorityPolicy::virtual_darwin(),&super::super::mach_clock::SystemClock::new(),&mut super::super::mach_semaphore::SemaphoreService::new()).unwrap(),0);
  assert_eq!(cpu.read_u64(0x10000),Some((72u64<<32)|0x1200));assert_eq!(cpu.read_u64(0x10014).unwrap()as u32,300);
  assert_eq!(cpu.read_u64(0x10030).unwrap()>>32,31);assert_eq!(ports.right(host),before);
  assert!(matches!(ports.right(args[4]as u32),Some(super::super::mach_identity::PortRight::ReplyReceive)));
 }
 #[test]fn malformed_original_rpc_and_unsupported_options_do_not_mutate_message(){
  let(mut cpu,mut ports,mut args)=fixture();let before=cpu.read_bytes(0x10000,80).unwrap().to_vec();args[1]=0x13;
  assert!(reply(&mut cpu,&mut ports,args,&HostPriorityPolicy::virtual_darwin(),&super::super::mach_clock::SystemClock::new(),&mut super::super::mach_semaphore::SemaphoreService::new()).is_err());assert_eq!(cpu.read_bytes(0x10000,80).unwrap(),before.as_slice());
  args[1]=3;cpu.write_guest_into(0x10020,&6u32.to_le_bytes()).unwrap();let before=cpu.read_bytes(0x10000,80).unwrap().to_vec();
  assert!(reply(&mut cpu,&mut ports,args,&HostPriorityPolicy::virtual_darwin(),&super::super::mach_clock::SystemClock::new(),&mut super::super::mach_semaphore::SemaphoreService::new()).is_err());assert_eq!(cpu.read_bytes(0x10000,80).unwrap(),before.as_slice());
 }
 #[test]fn original_clock_scalar_size_delivers_owned_live_clock_and_complex_reply(){
  let(mut cpu,mut ports,mut args)=fixture();let clock=super::super::mach_clock::SystemClock::new();args[2]=36;args[3]=48;
  cpu.write_guest_into(args[0]+4,&1u32.to_le_bytes()).unwrap();cpu.write_guest_into(args[0]+20,&206u32.to_le_bytes()).unwrap();cpu.write_guest_into(args[0]+32,&0u32.to_le_bytes()).unwrap();let host=word(cpu.read_bytes(args[0],36).unwrap(),8);let host_before=ports.right(host);
  assert_eq!(reply(&mut cpu,&mut ports,args,&HostPriorityPolicy::virtual_darwin(),&clock,&mut super::super::mach_semaphore::SemaphoreService::new()).unwrap(),0);
  let bytes=cpu.read_bytes(args[0],48).unwrap();assert_eq!(word(bytes,0),0x80001200);assert_eq!(word(bytes,4),40);assert_eq!(word(bytes,20),306);assert_eq!(word(bytes,24),1);assert_eq!(bytes[38],17);assert_eq!(bytes[39],0);assert_eq!(word(bytes,44),8);
  assert!(matches!(ports.right(word(bytes,28)),Some(super::super::mach_identity::PortRight::ClockSend{clock:0,references:1})));assert_eq!(ports.right(host),host_before);assert!(matches!(ports.right(args[4]as u32),Some(super::super::mach_identity::PortRight::ReplyReceive)));assert!(clock.time().is_ok());
 }
 #[test]fn original_calendar_request_is_not_fabricated_and_modern_size_remains_strict(){
  let(mut cpu,mut ports,mut args)=fixture();let clock=super::super::mach_clock::SystemClock::new();args[2]=36;args[3]=48;
  cpu.write_guest_into(args[0]+4,&1u32.to_le_bytes()).unwrap();cpu.write_guest_into(args[0]+20,&206u32.to_le_bytes()).unwrap();cpu.write_guest_into(args[0]+32,&1u32.to_le_bytes()).unwrap();let before=cpu.read_bytes(args[0],48).unwrap().to_vec();
  assert!(reply(&mut cpu,&mut ports,args,&HostPriorityPolicy::virtual_darwin(),&clock,&mut super::super::mach_semaphore::SemaphoreService::new()).is_err());assert_eq!(cpu.read_bytes(args[0],48).unwrap(),before.as_slice());assert!(ports.right(args[4]as u32+0x100).is_none());
  let task=ports.trap(-28).unwrap();let receive=ports.construct_reply(task,0,0x1000).unwrap();cpu.write_guest_into(args[0]+12,&receive.to_le_bytes()).unwrap();cpu.write_guest_into(args[0]+32,&0u32.to_le_bytes()).unwrap();let host=word(cpu.read_bytes(args[0],36).unwrap(),8);
  let modern=[args[0],0x200000003,0x2400001513,(u64::from(receive)<<32)|u64::from(host),206u64<<32,u64::from(receive)<<32,48,0];assert!(clock.reply(&mut cpu,&mut ports,modern).is_err());
 }
 #[test]fn original_task_semaphore_create_delivers_right_and_retains_owned_zero_count(){
  let(mut cpu,mut ports,mut args)=fixture();let task=ports.trap(-28).unwrap();args[3]=48;let clock=super::super::mach_clock::SystemClock::new();let mut semaphores=super::super::mach_semaphore::SemaphoreService::new();
  for(offset,value)in[(4,40u32),(8,task),(20,3418),(32,0),(36,0)]{cpu.write_guest_into(args[0]+offset,&value.to_le_bytes()).unwrap();}
  assert_eq!(reply(&mut cpu,&mut ports,args,&HostPriorityPolicy::virtual_darwin(),&clock,&mut semaphores).unwrap(),0);
  let bytes=cpu.read_bytes(args[0],48).unwrap();let name=word(bytes,28);assert_eq!(word(bytes,20),3518);assert_eq!(word(bytes,24),1);assert_eq!(bytes[38],17);assert_eq!(word(bytes,44),8);
  assert!(matches!(ports.right(name),Some(super::super::mach_identity::PortRight::SemaphoreSend{task:owner,references:1})if owner==task));assert_eq!(semaphores.record(name),Some(&super::super::mach_semaphore::Semaphore{count:0,policy:0}));
  assert_eq!(ports.deallocate(task,name),0);assert!(ports.right(name).is_none());assert!(semaphores.record(name).is_some());assert!(matches!(ports.right(args[4]as u32),Some(super::super::mach_identity::PortRight::ReplyReceive)));
 }
 #[test]fn unsupported_original_semaphore_policy_does_not_publish_task_object(){
  let(mut cpu,mut ports,mut args)=fixture();let task=ports.trap(-28).unwrap();args[3]=48;let clock=super::super::mach_clock::SystemClock::new();let mut semaphores=super::super::mach_semaphore::SemaphoreService::new();
  for(offset,value)in[(8,task),(20,3418u32),(32,1),(36,0)]{cpu.write_guest_into(args[0]+offset,&value.to_le_bytes()).unwrap();}let before=cpu.read_bytes(args[0],48).unwrap().to_vec();let next=args[4]as u32+0x100;
  assert!(reply(&mut cpu,&mut ports,args,&HostPriorityPolicy::virtual_darwin(),&clock,&mut semaphores).is_err());assert_eq!(cpu.read_bytes(args[0],48).unwrap(),before.as_slice());assert!(ports.right(next).is_none());assert!(semaphores.record(next).is_none());
 }
}

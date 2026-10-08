/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Darwin task restartable critical sequences, with strict-interior recovery.
#[derive(Clone,Debug)]
pub(super) struct Range {pub location:u64,pub length:u16,pub recovery_offs:u16,pub flags:u32}
#[derive(Clone,Debug)]
pub(super) struct Ranges {ranges:Vec<Range>}
impl Ranges {
 pub(super) fn parse(bytes:&[u8])->Result<Self,u32>{
  if bytes.is_empty()||bytes.len()%16!=0||bytes.len()/16>64{return Err(4);}
  let mut ranges:Vec<_>=bytes.chunks_exact(16).map(|b|Range{location:u64::from_le_bytes(b[..8].try_into().unwrap()),length:u16::from_le_bytes(b[8..10].try_into().unwrap()),recovery_offs:u16::from_le_bytes(b[10..12].try_into().unwrap()),flags:u32::from_le_bytes(b[12..16].try_into().unwrap())}).collect();
  ranges.sort_by_key(|r|r.location);
  for(index,range)in ranges.iter().enumerate(){
   if range.length>4096||range.recovery_offs>4096||range.flags!=0{return Err(4);}
   let end=range.location.checked_add(u64::from(range.length)).ok_or(4u32)?;
   range.location.checked_add(u64::from(range.recovery_offs)).ok_or(4u32)?;
   if ranges.get(index+1).is_some_and(|next|end>next.location){return Err(4);}
  }
  Ok(Self{ranges})
 }
 pub(super) fn ranges(&self)->&[Range]{&self.ranges}
 pub(super) fn recovery_pc(&self,pc:u64)->Option<u64>{self.ranges.iter().find(|r|r.location<pc&&pc<r.location+u64::from(r.length)).map(|r|r.location+u64::from(r.recovery_offs))}
}
/// Exact synchronous task_restartable_ranges_register RPC8000.
pub(super) fn reply(cpu:&mut super::A64Cpu,ports:&mut super::mach_identity::MachIdentity,
 scheduler:&std::rc::Rc<std::cell::RefCell<super::thread_scheduler_cpu::CpuScheduler>>,args:[u64;8])->Result<u32,String>{
 use super::mach_identity::PortRight;
 let[data,options,bits_size,remote_local,voucher_id,descriptors_receive,capacity_priority,timeout]=args;
 let size=(bits_size>>32)as usize;let task=remote_local as u32;let receive=(remote_local>>32)as u32;let capacity=capacity_priority as u32;
 if options!=0x200000003||bits_size as u32!=0x1513||!(36..=1060).contains(&size)||voucher_id!=8000u64<<32||descriptors_receive!=(receive as u64)<<32||capacity_priority>>32!=0||!(44..=4096).contains(&capacity)||timeout!=0{return Err("unsupported restartable registration Mach envelope".into());}
 if !matches!(ports.right(task),Some(PortRight::TaskSend{references})if references>0){return Ok(0x10000003);}
 if !matches!(ports.right(receive),Some(PortRight::ConstructedReplyReceive)){return Ok(0x10000009);}
 let mut request=vec![0;size];cpu.read_guest_into(data,&mut request)?;
 let word=|at:usize|u32::from_le_bytes(request[at..at+4].try_into().unwrap());
 let count=word(32)as usize;
 if word(0)!=0x1513||word(4)!=size as u32||word(8)!=task||word(12)!=receive||word(16)!=0||word(20)!=8000||request[24..32]!=[0,0,0,0,1,0,0,0]||count>64||36+count*16!=size{return Err("restartable registration header/NDR/count differs".into());}
 let parsed=Ranges::parse(&request[36..]);
 if let Ok(ranges)=parsed.as_ref(){for range in ranges.ranges(){
  let recovery=range.location+u64::from(range.recovery_offs);
  if cpu.mapped_permissions(recovery).is_none_or(|p|p&4==0)||(0..u64::from(range.length)).any(|offset|cpu.mapped_permissions(range.location+offset).is_none_or(|p|p&4==0)){return Err("restartable sequence/recovery lacks actual executable mapping".into());}
 }}
 cpu.validate_guest_write(data,capacity as usize)?;
 let ticket=ports.reserve_task_reply(task,receive)?;
 let status=match parsed {
  Ok(ranges)=>match scheduler.try_borrow_mut(){Ok(mut scheduler)=>scheduler.register_restartable_ranges(ranges).err().unwrap_or(0),Err(_)=>{ports.cancel_task_reply(ticket)?;return Err("restartable scheduler is borrowed".into());}},
  Err(status)=>status,
 };
 let mut response=[0u8;44];
 for(at,value)in[(0,0x1200u32),(4,36),(12,receive),(20,8100),(32,status),(40,8)]{response[at..at+4].copy_from_slice(&value.to_le_bytes());}response[24..32].copy_from_slice(&[0,0,0,0,1,0,0,0]);
 if let Err(error)=cpu.write_guest_into(data,&response){ports.cancel_task_reply(ticket)?;return Err(error);}
 ports.consume_task_reply(ticket)?;Ok(0)
}
#[cfg(test)]mod tests {
 use super::*;
 fn records(rows:&[(u64,u16,u16,u32)])->Vec<u8>{let mut bytes=Vec::new();for &(base,length,recovery,flags)in rows{bytes.extend_from_slice(&base.to_le_bytes());bytes.extend_from_slice(&length.to_le_bytes());bytes.extend_from_slice(&recovery.to_le_bytes());bytes.extend_from_slice(&flags.to_le_bytes());}bytes}
 #[test]fn only_interrupted_interior_recovers_and_adjacent_boundaries_stay_unchanged(){let ranges=Ranges::parse(&records(&[(0x1100,16,20,0),(0x1000,256,256,0)])).unwrap();for pc in [0xfff,0x1000,0x1100,0x1110]{assert_eq!(ranges.recovery_pc(pc),None);}assert_eq!(ranges.recovery_pc(0x1004),Some(0x1100));assert_eq!(ranges.recovery_pc(0x1104),Some(0x1114));}
 #[test]fn invalid_flags_offsets_overflow_overlap_and_count_never_publish_ranges(){for rows in [vec![(0x1000,16,16,1)],vec![(0x1000,4097,16,0)],vec![(0x1000,16,4097,0)],vec![(u64::MAX-1,4,0,0)],vec![(u64::MAX-1,0,4,0)],vec![(0x1000,32,32,0),(0x1010,16,16,0)],vec![]]{assert!(Ranges::parse(&records(&rows)).is_err());}assert!(Ranges::parse(&vec![0;65*16]).is_err());assert!(Ranges::parse(&[0;15]).is_err());}
 #[test]fn actual_five_objc_sequences_sort_and_recover_only_inside(){let ranges=Ranges::parse(&records(&[(0x1800ba8c8,0x60,0x60,0),(0x1800ba414,0x58,0x58,0),(0x1800ba64c,0x58,0x58,0),(0x1800ba514,0x60,0x60,0),(0x1800ba70c,0x60,0x60,0)])).unwrap();assert_eq!(ranges.ranges()[0].location,0x1800ba414);assert_eq!(ranges.recovery_pc(0x1800ba418),Some(0x1800ba46c));assert_eq!(ranges.recovery_pc(0x1800ba46c),None);}
 #[test]fn actual_mig_delivery_retains_scheduler_recovery_and_release_registration_is_once(){
  use std::{rc::Rc,cell::RefCell};let mut cpu=super::super::A64Cpu::new_sparse();cpu.map_zeroed(0x20000,4096,3).unwrap();cpu.map_zeroed(0x1800ba000,4096,5).unwrap();cpu.map_zeroed(0x90000,4096,3).unwrap();cpu.set_pc(0x1800ba400);cpu.set_sp(0x91000);
  let mut scheduler=super::super::thread_scheduler_cpu::CpuScheduler::default();scheduler.adopt(&cpu,(0x90000,0x91000)).unwrap();scheduler.select(&mut cpu).unwrap();let scheduler=Rc::new(RefCell::new(scheduler));
  let mut ports=super::super::mach_identity::MachIdentity::new(16).unwrap();let task=ports.trap(-28).unwrap();let receive=ports.construct_reply(task,0,0x1000).unwrap();
  let mut request=vec![0u8;116];for(at,value)in[(0,0x1513u32),(4,116),(8,task),(12,receive),(20,8000),(32,5)]{request[at..at+4].copy_from_slice(&value.to_le_bytes());}request[24..32].copy_from_slice(&[0,0,0,0,1,0,0,0]);request[36..].copy_from_slice(&records(&[(0x1800ba8c8,0x60,0x60,0),(0x1800ba414,0x58,0x58,0),(0x1800ba64c,0x58,0x58,0),(0x1800ba514,0x60,0x60,0),(0x1800ba70c,0x60,0x60,0)]));
  let args=[0x20000,0x200000003,0x7400001513,(u64::from(receive)<<32)|u64::from(task),8000u64<<32,u64::from(receive)<<32,44,0];
  cpu.write_guest_into(0x20000,&request).unwrap();assert_eq!(reply(&mut cpu,&mut ports,&scheduler,args).unwrap(),0);assert_eq!(cpu.read_u64(0x20000),Some((36u64<<32)|0x1200));assert_eq!(cpu.read_bytes(0x20020,4).unwrap(),&0u32.to_le_bytes());
  cpu.set_pc(0x1800ba418);assert!(scheduler.borrow_mut().recover_current_restartable(&mut cpu).unwrap());assert_eq!(cpu.pc(),0x1800ba46c);
  cpu.write_guest_into(0x20000,&request).unwrap();assert_eq!(reply(&mut cpu,&mut ports,&scheduler,args).unwrap(),0);assert_eq!(cpu.read_bytes(0x20020,4).unwrap(),&46u32.to_le_bytes());
 }
}

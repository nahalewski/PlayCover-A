/* MPL-2.0: https://mozilla.org/MPL/2.0/ */
//! A finite permit for one byte-verified original initializer diagnostic.
use super::{A64Cpu,cache,dyld_objc_callbacks::ObjcImage};
const ENTRY:u64=0x1d1c296c4;
const UUID:[u8;16]=[0x32,0x03,0x55,0x64,0x85,0x3b,0x38,0x8b,0xa1,0xef,0x2e,0xa3,0x54,0xc1,0x96,0xe1];
const ORIGINAL:[u8;32]=[0xff,0x83,0x01,0xd1,0xf6,0x57,0x03,0xa9,0xf4,0x4f,0x04,0xa9,0xfd,0x7b,0x05,0xa9,0xfd,0x43,0x01,0x91,0xf5,0x03,0x04,0xaa,0xf4,0x03,0x03,0xaa,0xf3,0x03,0x02,0xaa];
pub(super) struct InitializerBudget {header:u64,count:usize,budget:u64}
fn allowance(count:usize)->Result<u64,String>{if count==0||count>4096{return Err("initializer permit requires bounded real notification count".into());}Ok((100_000+count as u64*25_000).min(20_000_000))}
impl InitializerBudget{
 pub(super) fn verified(cpu:&A64Cpu,plan:&cache::CachePlan,images:&[ObjcImage])->Result<Self,String>{
  let main=plan.files.first().ok_or("initializer permit cache identity absent")?;
  let header=plan.regions.iter().find(|region|&region.file==main&&region.file_offset==0).ok_or("initializer permit actual cache header absent")?.vmaddr;
  let budget=allowance(images.len())?;
  let mut seen=std::collections::HashSet::new();
  for image in images{if !seen.insert(image.header){return Err("initializer permit duplicate notification identity".into());}if cpu.read_bytes(image.header,4).is_none_or(|bytes|bytes!=0xfeedfacfu32.to_le_bytes()){return Err("initializer permit notification header changed".into());}}
  if super::cache_initializers::functions(cpu,plan,"/usr/lib/libSystem.B.dylib")?!=[ENTRY]{return Err("initializer permit provider initializer differs".into());}
  let permit=Self{header,count:images.len(),budget};permit.validate(cpu,ENTRY,budget)?;Ok(permit)
 }
 pub(super) fn ticks(&self)->u64{self.budget}
 pub(super) fn validate(&self,cpu:&A64Cpu,entry:u64,budget:u64)->Result<(),String>{
  if entry!=ENTRY||budget==0||budget>self.budget||self.budget!=allowance(self.count)?{return Err("initializer budget permit target/count/allowance differs".into());}
  if cpu.read_bytes(self.header.checked_add(88).ok_or("initializer permit UUID overflow")?,16).is_none_or(|bytes|bytes!=UUID)
   ||cpu.read_bytes(ENTRY,32).is_none_or(|bytes|bytes!=ORIGINAL)
   ||cpu.mapped_permissions(ENTRY).is_none_or(|permissions|permissions&4==0){return Err("initializer budget permit original UUID/instructions/RX changed".into());}
  Ok(())
 }
}
#[cfg(test)]mod tests{use super::*;
 #[test]fn finite_notification_allowance(){assert!(allowance(0).is_err());assert!(allowance(4097).is_err());assert_eq!(allowance(753).unwrap(),18_925_000);assert_eq!(allowance(4096).unwrap(),20_000_000);}
 #[test]fn permits_reject_foreign_entry_overbudget_and_changed_original(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,1).unwrap();cpu.try_write_bytes(0x10058,&UUID).unwrap();
  cpu.map_zeroed(ENTRY&!4095,4096,5).unwrap();cpu.try_write_bytes(ENTRY,&ORIGINAL).unwrap();
  let permit=InitializerBudget{header:0x10000,count:753,budget:allowance(753).unwrap()};
  permit.validate(&cpu,ENTRY,permit.ticks()).unwrap();assert!(permit.validate(&cpu,ENTRY+4,1).is_err());assert!(permit.validate(&cpu,ENTRY,permit.ticks()+1).is_err());
  cpu.try_write_bytes(ENTRY,&[0;4]).unwrap();assert!(permit.validate(&cpu,ENTRY,1).is_err());cpu.try_write_bytes(ENTRY,&ORIGINAL).unwrap();
  cpu.try_write_bytes(0x10058,&[0;16]).unwrap();assert!(permit.validate(&cpu,ENTRY,1).is_err());
 }
 #[test]fn scoped_permit_never_changes_general_call_limit(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,1).unwrap();cpu.try_write_bytes(0x10058,&UUID).unwrap();
  cpu.map_zeroed(ENTRY&!4095,4096,5).unwrap();cpu.try_write_bytes(ENTRY,&ORIGINAL).unwrap();cpu.try_write_bytes(ENTRY+32,&[0x01,0x00,0x00,0xd4]).unwrap();
  let permit=InitializerBudget{header:0x10000,count:753,budget:allowance(753).unwrap()};
  let mut bridge=super::super::bridge::GuestBridge::map(&mut cpu,0x20000).unwrap();
  let foreign=super::super::bridge::GuestCall{entry:ENTRY+4,..Default::default()};
  assert!(bridge.call_initializer_with_supervisor(&mut cpu,&foreign,&permit,&mut |_,_|Err("trap".into())).is_err());
  let original=super::super::bridge::GuestCall{entry:ENTRY,..Default::default()};
  assert!(bridge.call_initializer_with_supervisor(&mut cpu,&original,&permit,&mut |_,_|Err("trap".into())).is_err());
  assert!(bridge.call(&mut cpu,&original,1_000_001).unwrap_err().contains("budget outside"));
 }
}

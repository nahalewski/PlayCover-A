/* MPL-2.0: https://mozilla.org/MPL/2.0/ */
//! A real loader-owned dyld2 lookup endpoint for explicitly implemented APIs.
use super::{A64Cpu,cache::CachePlan,bridge::{GuestBridge,ReturnValues},dyld_slide::ImageSlides};
use std::{rc::Rc,cell::RefCell};
const LOOKUP:u64=0x18084e234;
const SLOT:u64=0x1b3285db8;
const ORIGINAL:[u8;16]=[0xa8,0x51,0x19,0xf0,0x08,0xc1,0x36,0x91,0x02,0x05,0x40,0xf9,0x40,0x00,0x1f,0xd6];
const HELPERS:u64=0x1b1a15ec0;
const TABLE:[u64;22]=[13,0x18084e100,0x18084e168,0x18084fa74,0x1809b6090,0x1809b55bc,0x1808ec740,0x18084fb14,0x18084fb18,0,0,0x180b1cc48,0x180b1c1d0,0x1809b649c,0x180b28d5c,0x1808ec968,0x18084dfc0,0x18084fb1c,0x18084fb58,0x18095d4f8,0x18095c650,0x1808ec770];
#[derive(Default)]pub(super) struct HelperRegistration{table:Option<[u64;22]>}
impl HelperRegistration{
 pub(super) fn require_registered(&self)->Result<(),String>{if self.table==Some(TABLE){Ok(())}else{Err("legacy actual flat thread helpers not registered".into())}}
 #[cfg(test)]pub(super) fn test_registered()->Self{Self{table:Some(TABLE)}}
 fn register(&mut self,address:u64,bytes:&[u8],mut executable:impl FnMut(u64)->Result<(),String>)->Result<(),String>{
  if self.table.is_some(){return Err("legacy flat helpers already registered; replay refused".into());}
  if address!=HELPERS||bytes.len()!=176{return Err("legacy flat helper table identity/extent differs".into());}
  let mut table=[0;22];for(index,word)in bytes.chunks_exact(8).enumerate(){table[index]=u64::from_le_bytes(word.try_into().unwrap());}
  if table!=TABLE{return Err("original15G77 version13 helper callback identities differ".into());}
  for(index,&callback)in table.iter().enumerate().skip(1){if matches!(index,9|10){continue;}executable(callback)?;}
  self.table=Some(table);Ok(())
 }
}
fn guard(cpu:&A64Cpu,header:u64)->Result<(),String>{
 let uuid=[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f];
 if cpu.read_bytes(header.checked_add(88).ok_or("legacy dyld UUID overflow")?,16).is_none_or(|bytes|bytes!=uuid)
  ||cpu.read_bytes(LOOKUP,16).is_none_or(|bytes|bytes!=ORIGINAL)
  ||cpu.read_bytes(0x1b1a16748,1).is_none_or(|bytes|bytes!=[0])
  ||cpu.read_u64(SLOT)!=Some(0)||cpu.mapped_permissions(LOOKUP).is_none_or(|p|p&4==0){return Err("original15G77 dyld2 lookup provenance/mode/bootstrap slot differs".into());}
 cpu.validate_guest_write(SLOT,8)?;Ok(())
}
pub(super) fn install(cpu:&mut A64Cpu,bridge:&mut GuestBridge,plan:&CachePlan,slides:ImageSlides)->Result<u64,String>{
 let main=plan.files.first().ok_or("legacy lookup main cache absent")?;
 let header=plan.regions.iter().find(|region|&region.file==main&&region.file_offset==0).ok_or("legacy lookup original cache header absent")?.vmaddr;
 install_at(cpu,bridge,header,slides)
}
pub(super) fn install_with_images(cpu:&mut A64Cpu,bridge:&mut GuestBridge,plan:&CachePlan,slides:ImageSlides,images:Vec<super::legacy_add_images::Image>,objc_images:Vec<super::dyld_objc_callbacks::ObjcImage>)->Result<u64,String>{
 let main=plan.files.first().ok_or("legacy lookup main cache absent")?;
 let header=plan.regions.iter().find(|region|&region.file==main&&region.file_offset==0).ok_or("legacy lookup original cache header absent")?.vmaddr;
 guard(cpu,header)?;let objc=super::legacy_objc_notify::install(cpu,bridge,objc_images)?;
 install_with_state(cpu,bridge,header,slides,Rc::new(RefCell::new(HelperRegistration::default())),Some(images),Some(objc))
}
fn install_at(cpu:&mut A64Cpu,bridge:&mut GuestBridge,header:u64,slides:ImageSlides)->Result<u64,String>{
 install_with_state(cpu,bridge,header,slides,Rc::new(RefCell::new(HelperRegistration::default())),None,None)
}
fn install_with_state(cpu:&mut A64Cpu,bridge:&mut GuestBridge,header:u64,slides:ImageSlides,helpers:Rc<RefCell<HelperRegistration>>,images:Option<Vec<super::legacy_add_images::Image>>,objc:Option<u64>)->Result<u64,String>{
 guard(cpu,header)?;
 let add_images=images.map(|images|super::legacy_add_images::install(cpu,bridge,images,helpers.clone())).transpose()?;
 let slide=bridge.register_service(cpu,"legacy_actual_image_slide",move|frame|Ok(ReturnValues::integer(slides.lookup(frame.integer(0)?)?as u64)))?.guest_address();
 // This is the enforced declared-path loader policy, not the host process's
 // credentials or a fabricated platform security decision.
 let policy=super::dyld_slide::LoaderPolicy::declared_paths_only();
 if policy.environment_path_override("/").is_ok(){return Err("legacy loader unexpectedly permits DYLD environment overrides".into());}
 let restricted=bridge.register_service(cpu,"legacy_declared_path_process_restricted",move|_|Ok(ReturnValues::integer(u64::from(policy.is_restricted()))))?.guest_address();
 let registration=bridge.register_service(cpu,"legacy_real_version13_thread_helpers",move|frame|{
  let address=frame.integer(0)?;let bytes=frame.read(address,176)?;
  helpers.try_borrow_mut().map_err(|_|"legacy helper registration reentrant")?.register(address,&bytes,|callback|frame.validate_executable_pointer(callback))?;
  echo!("[a64] original15G77 version13 flat thread helpers retained with21 actual guest callback slots (initializer locks9/10 absent); no initialization receipt");
  Ok(ReturnValues::integer(0))
 })?.guest_address();
 let lookup=bridge.register_service(cpu,"legacy_owned_dyld_function_lookup",move|frame|{
  let name=frame.integer(0)?;let output=frame.integer(1)?;let mut bytes=Vec::new();
  for offset in 0..=128{let byte=frame.read(name.checked_add(offset).ok_or("legacy lookup CString overflow")?,1)?[0];if byte==0{
   let target=if bytes==b"__dyld_get_image_slide"{slide}else if bytes==b"__dyld_process_is_restricted"{restricted}else if bytes==b"__dyld_register_thread_helpers"{registration}else if bytes==b"__dyld_register_func_for_add_image"{add_images.ok_or("legacy actualadd-image notification provider absent")?}else if bytes==b"__dyld_objc_notify_register"{objc.ok_or("legacy actual ObjC notification provider absent")?}else{return Err(format!("legacy dyld lookup service not implemented: {}",String::from_utf8_lossy(&bytes)));};
   frame.write(output,&target.to_le_bytes())?;return Ok(ReturnValues::integer(1));
  }if offset==128{return Err("legacy lookup name unterminated".into());}bytes.push(byte);}
  unreachable!()
 })?.guest_address();
 // Original flat libdyld code and output caches remain untouched. Publish the
 // actual registered loader callback into its genuine dyld bootstrap slot.
 cpu.write_guest_into(SLOT,&lookup.to_le_bytes())?;Ok(lookup)
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn retained_original_helper_registration_is_atomic_and_allows_absent_initializer_locks(){
  let bytes=TABLE.iter().flat_map(|word|word.to_le_bytes()).collect::<Vec<_>>();let mut helpers=HelperRegistration::default();
  assert!(helpers.register(HELPERS+8,&bytes,|_|Ok(())).is_err());assert!(helpers.table.is_none());
  assert!(helpers.register(HELPERS,&bytes,|address|if address==TABLE[14]{Err("not RX".into())}else{Ok(())}).is_err());assert!(helpers.table.is_none());
  let mut seen=Vec::new();helpers.register(HELPERS,&bytes,|address|{assert_ne!(address,0);seen.push(address);Ok(())}).unwrap();assert_eq!(seen.len(),19);assert_eq!(helpers.table,Some(TABLE));assert!(helpers.register(HELPERS,&bytes,|_|Ok(())).is_err());
 }
 #[test]fn foreign_original_or_preexisting_bootstrap_endpoint_rejected(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,1).unwrap();cpu.try_write_bytes(0x10058,&[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f]).unwrap();cpu.map_zeroed(LOOKUP&!4095,4096,5).unwrap();cpu.try_write_bytes(LOOKUP,&ORIGINAL).unwrap();cpu.map_zeroed(SLOT&!4095,4096,3).unwrap();cpu.map_zeroed(0x1b1a16000,4096,3).unwrap();guard(&cpu,0x10000).unwrap();cpu.write_guest_into(SLOT,&1u64.to_le_bytes()).unwrap();assert!(guard(&cpu,0x10000).is_err());cpu.write_guest_into(SLOT,&0u64.to_le_bytes()).unwrap();cpu.try_write_bytes(LOOKUP,&[0;4]).unwrap();assert!(guard(&cpu,0x10000).is_err());
 }
 #[test]fn original_guest_lookup_returns_real_signed_slide(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,1).unwrap();cpu.try_write_bytes(0x10058,&[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f]).unwrap();cpu.map_zeroed(LOOKUP&!4095,4096,5).unwrap();cpu.try_write_bytes(LOOKUP,&ORIGINAL).unwrap();cpu.map_zeroed(SLOT&!4095,4096,3).unwrap();cpu.map_zeroed(0x1b1a16000,4096,3).unwrap();cpu.map_zeroed(0x30000,4096,3).unwrap();cpu.try_write_bytes(0x30000,b"__dyld_get_image_slide\0").unwrap();
  let helpers=Rc::new(RefCell::new(HelperRegistration::default()));
  let mut bridge=GuestBridge::map(&mut cpu,0x40000).unwrap();install_with_state(&mut cpu,&mut bridge,0x10000,ImageSlides::new(&[(0x10000,-4096)]).unwrap(),helpers.clone(),None,None).unwrap();
  let returned=bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:LOOKUP,integers:vec![0x30000,0x30100],..Default::default()},1000).unwrap();assert_eq!(returned.integers[0],1);
  let target=cpu.read_u64(0x30100).unwrap();let returned=bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:target,integers:vec![0x10000],..Default::default()},1000).unwrap();assert_eq!(returned.integers[0],(-4096i64)as u64);
  assert!(bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:target,integers:vec![0x10004],..Default::default()},1000).is_err());
  cpu.try_write_bytes(0x30000,b"__dyld_process_is_restricted\0").unwrap();
  bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:LOOKUP,integers:vec![0x30000,0x30100],..Default::default()},1000).unwrap();
  let restricted=cpu.read_u64(0x30100).unwrap();cpu.set_reg(19,0x123456);let sp=cpu.sp();let lr=cpu.reg(30);
  let result=bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:restricted,..Default::default()},1000).unwrap();assert_eq!(result.integers[0],1);assert_eq!(cpu.reg(19),0x123456);assert_eq!(cpu.sp(),sp);assert_eq!(cpu.reg(30),lr);
  cpu.map_zeroed(HELPERS&!4095,4096,1).unwrap();cpu.try_write_bytes(HELPERS,&TABLE.iter().flat_map(|word|word.to_le_bytes()).collect::<Vec<_>>()).unwrap();
  for &callback in TABLE.iter().skip(1).filter(|&&callback|callback!=0){if cpu.mapped_permissions(callback).is_none(){cpu.map_zeroed(callback&!4095,4096,5).unwrap();}}
  cpu.try_write_bytes(0x30000,b"__dyld_register_thread_helpers\0").unwrap();bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:LOOKUP,integers:vec![0x30000,0x30100],..Default::default()},1000).unwrap();
  let registration=cpu.read_u64(0x30100).unwrap();bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:registration,integers:vec![HELPERS],..Default::default()},1000).unwrap();assert_eq!(helpers.borrow().table,Some(TABLE));
  assert!(bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:registration,integers:vec![HELPERS],..Default::default()},1000).is_err());
  cpu.try_write_bytes(0x30000,b"__dyld_not_implemented\0").unwrap();let before=cpu.read_u64(0x30100);
  assert!(bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:LOOKUP,integers:vec![0x30000,0x30100],..Default::default()},1000).is_err());assert_eq!(cpu.read_u64(0x30100),before);
 }
}

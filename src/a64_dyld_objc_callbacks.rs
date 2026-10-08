/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Version-one original ObjC mapped notification, not constructor readiness.
use super::{A64Cpu,bridge::{GuestBridge,GuestCall,ReturnValues},execution_session::SessionLease,thread_scheduler_cpu::CpuScheduler};
use std::{rc::Rc,cell::{Cell,RefCell},collections::BTreeSet};
pub(super) const ENTRY:u64=0x1a6c7f2d4;
const ORIGINAL:[u8;24]=[0xe1,3,0,0xaa,0x88,0x85,0x1d,0xb0,0,0xc5,0x41,0xf9,8,0,0x40,0xf9,2,0xc1,0x41,0xf9,0x40,0,0x1f,0xd6];
#[derive(Clone,Debug)]
pub(super) struct ObjcImage {pub header:u64,pub path:String,readonly:Vec<(u64,u64)>}
fn u32at(b:&[u8],at:usize)->Result<u32,String>{Ok(u32::from_le_bytes(b.get(at..at.checked_add(4).ok_or("ObjC metadata overflow")?).ok_or("truncated ObjC metadata")?.try_into().unwrap()))}
fn u64at(b:&[u8],at:usize)->Result<u64,String>{Ok(u64::from_le_bytes(b.get(at..at.checked_add(8).ok_or("ObjC metadata overflow")?).ok_or("truncated ObjC metadata")?.try_into().unwrap()))}
fn name(b:&[u8])->&[u8]{&b[..b.iter().position(|x|*x==0).unwrap_or(b.len())]}
impl ObjcImage {
 pub(super) fn readonly_ranges(&self)->&[(u64,u64)]{&self.readonly}
 pub(super) fn read(cpu:&A64Cpu,header:u64,path:String,in_cache:bool)->Result<Option<Self>,String>{
  let context=format!("loaded ObjC candidate {path} header={header:#x} original_cache={in_cache}");
  Self::read_inner(cpu,header,path,in_cache).map_err(|error|format!("{context}: {error}"))
 }
 fn read_inner(cpu:&A64Cpu,header:u64,path:String,in_cache:bool)->Result<Option<Self>,String>{
  if header==0||header%4!=0||path.is_empty()||path.len()>4096||path.contains('\0'){return Err("invalid loaded ObjC image identity".into());}
  let mut h=[0;32];cpu.read_guest_into(header,&mut h)?;
  if u32at(&h,0)?!=0xfeedfacf||u32at(&h,4)?!=0x100000c||!matches!(u32at(&h,12)?,2|6){return Err("ObjC notification image is not mapped ARM64 Mach-O".into());}
  let count=u32at(&h,16)? as usize;let size=u32at(&h,20)? as usize;
  if count>4096||size>1024*1024{return Err("ObjC command metadata budget".into());}
  let mut commands=vec![0;size];cpu.read_guest_into(header.checked_add(32).ok_or("ObjC commands overflow")?,&mut commands)?;
  let mut at=0usize;let mut segments=Vec::new();let mut preferred=None;let mut sections=0usize;let mut has_objc=false;let mut has_const_objc=false;
  for _ in 0..count {
   let command=u32at(&commands,at)?;let len=u32at(&commands,at+4)? as usize;
   if len<8||len%8!=0{return Err("invalid ObjC command alignment".into());}
   let next=at.checked_add(len).ok_or("ObjC command overflow")?;let bytes=commands.get(at..next).ok_or("truncated ObjC command")?;
   if command==0x19 {
    if len<72{return Err("truncated ObjC segment".into());}
    let nsects=u32at(bytes,64)? as usize;sections=sections.checked_add(nsects).ok_or("ObjC section overflow")?;
    if sections>4096||72usize.checked_add(nsects.checked_mul(80).ok_or("ObjC section overflow")?).ok_or("ObjC section overflow")?!=len{return Err("invalid ObjC section array".into());}
    let vmaddr=u64at(bytes,24)?;let vmsize=u64at(bytes,32)?;let fileoff=u64at(bytes,40)?;let filesize=u64at(bytes,48)?;
    if filesize>=32&&((!in_cache&&fileoff==0)||(in_cache&&name(&bytes[8..24])==b"__TEXT"&&vmaddr==header)) {if preferred.replace(vmaddr).is_some(){return Err("duplicate ObjC header segment".into());}}
    let const_segment=matches!(name(&bytes[8..24]),b"__DATA_CONST"|b"__AUTH_CONST");let mut const_objc=false;
    for section in bytes[72..].chunks_exact(80){
     let sectname=name(&section[..16]);let address=u64at(section,32)?;let length=u64at(section,40)?;
     if length!=0&&(address<vmaddr||address.checked_add(length).ok_or("ObjC section range overflow")?>vmaddr.checked_add(vmsize).ok_or("ObjC segment range overflow")?){return Err("ObjC section exceeds segment".into());}
     if sectname==b"__objc_imageinfo"&&length>=8{has_objc=true;}
     if const_segment&&sectname.starts_with(b"__objc_")&&length!=0{const_objc=true;}
    }
    has_const_objc|=const_objc;
    if !in_cache&&u32at(bytes,68)?&0x10!=0&&vmsize!=0 {segments.push((vmaddr,vmsize));}
   }
   at=next;
  }
  if at!=size{return Err("ObjC command size/count disagreement".into());}
  if !has_objc{return Ok(None);}
  if !has_const_objc{segments.clear();}
  let preferred=preferred.ok_or("ObjC image has no actual header segment")?;let slide=i128::from(header)-i128::from(preferred);
  let mut readonly=Vec::new();for(base,len)in segments {let base=u64::try_from(i128::from(base)+slide).map_err(|_|"ObjC segment slide overflow")?;base.checked_add(len).ok_or("ObjC protection range overflow")?;readonly.push((base,len));}
  Ok(Some(Self{header,path,readonly}))
 }
}
#[cfg(test)]mod tests {
 use super::*;
 fn fixture(objc:bool)->A64Cpu {
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,1).unwrap();cpu.map_zeroed_with_max(0x20000,4096,1,3).unwrap();cpu.map_zeroed_with_max(0x30000,4096,1,3).unwrap();
  let mut bytes=vec![0u8;32+72+152+72];for(at,value)in[(0,0xfeedfacfu32),(4,0x100000c),(12,6),(16,3),(20,296)]{bytes[at..at+4].copy_from_slice(&value.to_le_bytes());}
  for(at,address,fileoff,sects,flags,label)in[(32,0x10000u64,0u64,0u32,0u32,b"__TEXT".as_slice()),(104,0x20000,4096,1,16,b"__DATA_CONST".as_slice()),(256,0x30000,8192,0,16,b"__DATA_OTHER".as_slice())]{let len=72+sects*80;for(off,value)in[(0,0x19u32),(4,len),(56,3),(60,1),(64,sects),(68,flags)]{bytes[at+off..at+off+4].copy_from_slice(&value.to_le_bytes());}bytes[at+8..at+8+label.len()].copy_from_slice(label);for(off,value)in[(24,address),(32,4096),(40,fileoff),(48,4096)]{bytes[at+off..at+off+8].copy_from_slice(&value.to_le_bytes());}}
  let name=if objc{b"__objc_imageinfo".as_slice()}else{b"__plain".as_slice()};bytes[176..176+name.len()].copy_from_slice(name);bytes[208..216].copy_from_slice(&0x20000u64.to_le_bytes());bytes[216..224].copy_from_slice(&8u64.to_le_bytes());cpu.try_write_bytes(0x10000,&bytes).unwrap();cpu
 }
 #[test]fn actual_image_metadata_selects_objc_and_reopens_all_ordinary_readonly_segments(){let cpu=fixture(true);let ordinary=ObjcImage::read(&cpu,0x10000,"/Game".into(),false).unwrap().unwrap();assert_eq!(ordinary.readonly,vec![(0x20000,4096),(0x30000,4096)]);let cached=ObjcImage::read(&cpu,0x10000,"/Cache".into(),true).unwrap().unwrap();assert!(cached.readonly.is_empty());assert!(ObjcImage::read(&fixture(false),0x10000,"/NoObjc".into(),false).unwrap().is_none());}
 #[test]fn malformed_section_metadata_does_not_create_notification_record(){let mut cpu=fixture(true);cpu.try_write_bytes(0x10000+104+64,&2u32.to_le_bytes()).unwrap();assert!(ObjcImage::read(&cpu,0x10000,"/Game".into(),false).is_err());assert_eq!(cpu.mapped_permissions(0x20000),Some(1));}
 #[test]fn original_cached_header_uses_verified_text_vm_address_not_cache_file_offset_zero(){let mut cpu=fixture(true);cpu.try_write_bytes(0x10000+32+40,&0x12345000u64.to_le_bytes()).unwrap();assert!(ObjcImage::read(&cpu,0x10000,"/OriginalCache".into(),true).unwrap().is_some());let error=ObjcImage::read(&cpu,0x10000,"/Ordinary".into(),false).unwrap_err();assert!(error.contains("/Ordinary header=0x10000"));cpu.try_write_bytes(0x10000+32+24,&0x11000u64.to_le_bytes()).unwrap();assert!(ObjcImage::read(&cpu,0x10000,"/WrongHeader".into(),true).is_err());}
}
struct Registration {callbacks:Option<[u64;5]>,pending:bool,outcome:Option<Result<(),String>>,delivered:bool}
pub(super) fn install(cpu:&mut A64Cpu,bridge:&mut GuestBridge,entry:u64,images:Vec<ObjcImage>,lease:SessionLease,scheduler:Rc<RefCell<CpuScheduler>>,owner:u64)->Result<u64,String>{
 lease.validate(&scheduler,owner)?;
 if entry!=ENTRY||cpu.mapped_permissions(entry).is_none_or(|p|p&4==0)||cpu.read_bytes(entry,24).ok_or("unreadable ObjC callback wrapper")?!=ORIGINAL{return Err("original ObjC callback wrapper identity/instructions differ".into());}
 // Diagnostic only: exact original20H392 cache and observed add-header store.
 // A different callback remains untraced; this grants no execution permission.
 let uuid=[0x32,0x03,0x55,0x64,0x85,0x3b,0x38,0x8b,0xa1,0xef,0x2e,0xa3,0x54,0xc1,0x96,0xe1];
 if cpu.read_bytes(0x180000058,16).is_some_and(|bytes|bytes==uuid)
    &&cpu.read_bytes(0x1800c4100,4).is_some_and(|bytes|bytes==0xf900011cu32.to_le_bytes())
    &&images.iter().any(|image|image.path=="/usr/lib/libobjc.A.dylib"&&image.header<=0x1800c3bc0&&0x1800c728c-image.header<=1024*1024){
  bridge.trace_callback_progress(cpu,0x1800c5770,0x1800c3bc0,0x1800c728c,0x1e1d2fce0,0x1e1d30118)?;
 }
 if images.len()>4096{return Err("ObjC notification image count budget".into());}
 let mut identities=BTreeSet::new();let mut bytes=vec![0u8;images.len()*16];let mut readonly=Vec::new();
 for(index,image)in images.iter().enumerate(){if !identities.insert(image.header){return Err("duplicate loaded ObjC notification header".into());}readonly.extend_from_slice(&image.readonly);let path_offset=bytes.len()as u64;bytes[index*8..index*8+8].copy_from_slice(&path_offset.to_le_bytes());let at=images.len()*8+index*8;bytes[at..at+8].copy_from_slice(&image.header.to_le_bytes());bytes.extend_from_slice(image.path.as_bytes());bytes.push(0);}
 if bytes.len()>1024*1024{return Err("ObjC notification argument byte budget".into());}
 let size=(bytes.len().max(1)as u64+0x3fff)&!0x3fff;let mut arena=(bridge.scratch_end()?.checked_add(0x3fff).ok_or("ObjC provider arena overflow")?)&!0x3fff;
 for _ in 0..4096 {let end=arena.checked_add(size).ok_or("ObjC arena overflow")?;let collision=(arena..end).find_map(|address|cpu.protection_region(address));if let Some(region)=collision{arena=(region.base.checked_add(region.len).ok_or("ObjC arena mapping overflow")?.checked_add(0x3fff).ok_or("ObjC arena alignment overflow")?)&!0x3fff;}else{break;}}
 let end=arena.checked_add(size).ok_or("ObjC arena overflow")?;if (arena..end).any(|address|cpu.mapped_permissions(address).is_some()){return Err("ObjC provider arena cannot reserve disjoint mapping".into());}
 for index in 0..images.len(){let offset=u64::from_le_bytes(bytes[index*8..index*8+8].try_into().unwrap());bytes[index*8..index*8+8].copy_from_slice(&arena.checked_add(offset).ok_or("ObjC path pointer overflow")?.to_le_bytes());}
 cpu.map_zeroed(arena,size as usize,1)?;cpu.try_write_bytes(arena,&bytes)?;
 let headers=arena.checked_add(images.len()as u64*8).ok_or("ObjC headers array overflow")?;let count=images.len()as u64;
 let state=Rc::new(RefCell::new(Registration{callbacks:None,pending:false,outcome:None,delivered:false}));let target=Rc::new(Cell::new(0));let reentry=target.clone();
 let service=bridge.register_service(cpu,"original_ObjC_v1_loaded_mapped_notification",move|frame|{
  lease.validate(&scheduler,owner)?;let mut current=state.try_borrow_mut().map_err(|_|"ObjC notification registration is reentrant")?;
  if current.pending {let outcome=current.outcome.take().ok_or("original ObjC mapped callback has not returned")?;outcome?;current.pending=false;current.delivered=true;echo!("[a64] original ObjC mapped callback returned for {count} actual loaded headers; no initialization or +load receipt");return Ok(ReturnValues::integer(0));}
  if current.callbacks.is_some(){return Err("ObjC callbacks already registered; duplicate/replay refused".into());}
  let descriptor=frame.read(frame.integer(0)?,40)?;let mut callbacks=[0u64;5];for(index,word)in descriptor.chunks_exact(8).enumerate(){callbacks[index]=u64::from_le_bytes(word.try_into().unwrap());}
  if callbacks[0]!=1{return Err(format!("unsupported original ObjC callback descriptor version{}",callbacks[0]));}
  for &callback in &callbacks[1..]{frame.validate_executable_pointer(callback)?;}
  current.callbacks=Some(callbacks);
  if count==0 {current.delivered=true;return Ok(ReturnValues::integer(0));}
  current.pending=true;let completed=state.clone();drop(current);
  echo!("[a64] invoking original ObjC version1 mapped callback for {count} actual loaded headers; no initialization receipt");
  frame.request_guest_call_with_writable_ranges(GuestCall{entry:callbacks[1],integers:vec![count,arena,headers],..Default::default()},readonly.clone(),move|outcome|{let mut state=completed.try_borrow_mut().map_err(|_|"ObjC callback completion state borrowed")?;state.outcome=Some(outcome.map(|_|()));Ok(())})?;
  frame.request_tail_dispatch(reentry.get(),1)?;Ok(ReturnValues::integer(0))
 })?.guest_address();target.set(service);
 let mut patch=Vec::new();for i in 0..4u32{let instruction=(if i==0{0xd2800000}else{0xf2800000})|(i<<21)|((((service>>(i*16))&0xffff)as u32)<<5)|16;patch.extend_from_slice(&instruction.to_le_bytes());}patch.extend_from_slice(&0xd61f0200u32.to_le_bytes());patch.extend_from_slice(&0xd503201fu32.to_le_bytes());cpu.try_write_bytes(entry,&patch)?;Ok(service)
}

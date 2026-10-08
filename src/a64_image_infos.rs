/* This Source Code Form is subject to the Mozilla Public License, v. 2.0.
 * https://mozilla.org/MPL/2.0/ */
//! Explicit loader-owned LP64 dyld image-info compatibility implementation.
//! ABI: Apple mach-o/dyld_images.h version1, not a forged cached export.
use super::A64Cpu;
use std::collections::{BTreeMap,BTreeSet,HashMap};
const PAGE:u64=0x4000;
const LIMIT:usize=4096;
#[derive(Clone,Debug)]
pub(super) struct Record {pub header:u64,pub path:String}
pub(super) struct ImageInfos {pub address:u64,pub getter:u64,pub count:usize}
fn valid_record(cpu:&A64Cpu,r:&Record)->Result<(),String>{
 if r.header==0||r.header&3!=0||r.path.is_empty()||r.path.len()>4096||r.path.contains('\0') {return Err("invalid owned image record".into());}
 let mut h=[0;32];cpu.read_guest_into(r.header,&mut h)?;
 if u32::from_le_bytes(h[..4].try_into().unwrap())!=0xfeedfacf||u32::from_le_bytes(h[4..8].try_into().unwrap())!=0x100000c {return Err("owned image record is not a mapped ARM64 Mach header".into());}
 if !matches!(u32::from_le_bytes(h[12..16].try_into().unwrap()),2|6)||u32::from_le_bytes(h[16..20].try_into().unwrap())>4096||u32::from_le_bytes(h[20..24].try_into().unwrap())>1024*1024 {return Err("owned image record has invalid Mach metadata".into());}
 Ok(())
}
impl ImageInfos {
 pub(super) fn map(cpu:&mut A64Cpu,base:u64,records:&[Record])->Result<Self,String>{
  if base%PAGE!=0||records.is_empty()||records.len()>LIMIT {return Err("owned image-info alignment/count limit".into());}
  let mut headers=BTreeSet::new();let mut paths=BTreeSet::new();
  for r in records {valid_record(cpu,r)?;if !headers.insert(r.header)||!paths.insert(&r.path){return Err("duplicate owned image identity".into());}}
  let data=base.checked_add(PAGE).ok_or("image-info base overflow")?;
  let mut bytes=vec![0u8;32+records.len()*24];
  bytes[..4].copy_from_slice(&1u32.to_le_bytes()); // version1: no initialized flag.
  bytes[4..8].copy_from_slice(&(records.len() as u32).to_le_bytes());
  bytes[8..16].copy_from_slice(&data.checked_add(32).ok_or("image array overflow")?.to_le_bytes());
  // notification=NULL: fixed startup snapshot has no observer registrations.
  // processDetachedFromSharedRegion=false; selected original cache stays mapped.
  for (i,r) in records.iter().enumerate(){
   let path=data.checked_add(bytes.len() as u64).ok_or("image path overflow")?;
   let slot=32+i*24;
   bytes[slot..slot+8].copy_from_slice(&r.header.to_le_bytes());
   bytes[slot+8..slot+16].copy_from_slice(&path.to_le_bytes());
   // modification time0: no trustworthy device filesystem mtime is available.
   bytes.extend_from_slice(r.path.as_bytes());bytes.push(0);
  }
  if bytes.len()>1024*1024{return Err("owned image-info byte budget".into());}
  let size=(bytes.len() as u64+PAGE-1)&!(PAGE-1);
  data.checked_add(size).ok_or("image-info range overflow")?;
  let end=data.checked_add(size).ok_or("image-info range overflow")?;
  if (base..end).any(|a|cpu.mapped_permissions(a).is_some()) {return Err("owned image-info region overlaps mapping".into());}
  cpu.map_zeroed(base,PAGE as usize,5)?;
  cpu.map_zeroed(data,size as usize,1)?;
  cpu.try_write_bytes(data,&bytes)?;
  let mut code=Vec::new();
  for shift in 0..4u32 {
   let imm=((data>>(shift*16))&65535) as u32;
   let op=if shift==0 {0xd2800000} else {0xf2800000};
   code.extend_from_slice(&(op|(shift<<21)|(imm<<5)).to_le_bytes());
  }
  code.extend_from_slice(&0xd65f03c0u32.to_le_bytes()); // genuine C getter RET.
  cpu.try_write_bytes(base,&code)?;
  Ok(Self{address:data,getter:base,count:records.len()})
 }
 pub(super) fn compatibility_symbol(&self,provider:&str,name:&str)->Option<u64>{
  (name=="__dyld_get_all_image_infos"&&matches!(provider,"/usr/lib/libSystem.B.dylib"|"/usr/lib/libSystem.dylib"|"/usr/lib/system/libdyld.dylib")).then_some(self.getter)
 }
}
/// Select only actual dependency closure, never every image merely present in
/// the mapped cache. No constructor/load/initialization receipt is generated.
pub(super) fn cached_closure(cpu:&A64Cpu,catalogue:&HashMap<String,u64>,roots:impl IntoIterator<Item=String>)->Result<Vec<Record>,String>{
 let mut todo:Vec<_>=roots.into_iter().collect();let mut selected=BTreeMap::new();let mut edges=0usize;
 if todo.len()>LIMIT{return Err("image-info root budget".into());}
 while let Some(path)=todo.pop(){
  if selected.contains_key(&path){continue;}
  if selected.len()>=LIMIT {return Err("image-info dependency closure limit".into());}
  let Some(&header)=catalogue.get(&path) else {continue};
  let record=Record{header,path:path.clone()};valid_record(cpu,&record)?;
  let mut h=[0;32];cpu.read_guest_into(header,&mut h)?;
  let count=u32::from_le_bytes(h[16..20].try_into().unwrap()) as usize;
  let size=u32::from_le_bytes(h[20..24].try_into().unwrap()) as usize;
  if count>4096||size>1024*1024{return Err("image-info cache command budget".into());}
  let mut commands=vec![0;size];cpu.read_guest_into(header.checked_add(32).ok_or("image command overflow")?,&mut commands)?;
  let mut pos=0usize;
  for _ in 0..count {
   let prefix=commands.get(pos..pos.checked_add(8).ok_or("command overflow")?).ok_or("truncated image-info command")?;
   let cmd=u32::from_le_bytes(prefix[..4].try_into().unwrap());let length=u32::from_le_bytes(prefix[4..].try_into().unwrap()) as usize;
   if length<8||length%8!=0{return Err("invalid image-info command length".into());}
   let b=commands.get(pos..pos.checked_add(length).ok_or("command overflow")?).ok_or("truncated image-info command")?;
   if matches!(cmd,0xc|0x80000018|0x8000001f|0x80000023){
    if b.len()<24{return Err("truncated image dependency".into());}
    let no=u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
    if no<24{return Err("invalid dependency name offset".into());}
    let name=b.get(no..).ok_or("dependency name outside command")?;
    let end=name.iter().position(|x|*x==0).ok_or("unterminated dependency name")?;
    let name=std::str::from_utf8(&name[..end]).map_err(|_|"invalid dependency UTF8")?;
    if catalogue.contains_key(name){edges+=1;if edges>65536{return Err("image-info dependency edge budget".into());}todo.push(name.into());}
   }
   pos+=length;
  }
  if pos!=size{return Err("image-info command size mismatch".into());}
  selected.insert(path,header);
 }
 let mut identities=BTreeSet::new();
 Ok(selected.into_iter().filter(|(_,header)|identities.insert(*header)).map(|(path,header)|Record{header,path}).collect())
}
#[cfg(test)] mod tests{
 use super::*;
 fn cpu()->A64Cpu {let mut c=A64Cpu::new_sparse();c.map_zeroed(0x10000,0x4000,1).unwrap();let mut h=[0;32];h[..8].copy_from_slice(&[0xcf,0xfa,0xed,0xfe,0xc,0,0,1]);h[12..16].copy_from_slice(&2u32.to_le_bytes());c.try_write_bytes(0x10000,&h).unwrap();c}
 #[test] fn actual_guest_getter_returns_validated_image_array(){
  let mut c=cpu();let i=ImageInfos::map(&mut c,0x20000,&[Record{header:0x10000,path:"/Game.app/Game".into()}]).unwrap();
  assert_eq!(i.count,1);assert_eq!(c.read_u64(i.address+8),Some(i.address+32));assert_eq!(c.read_u64(i.address+32),Some(0x10000));
  c.map_zeroed(0x50000,0x4000,5).unwrap();c.try_write_bytes(0x50000,&0xd4000fc1u32.to_le_bytes()).unwrap();
  c.set_reg(30,0x50000);c.set_pc(i.getter);
  let mut ticks=100;assert_eq!(c.run_or_step(Some(&mut ticks)),super::super::A64State::Svc(0x7e));assert_eq!(c.pc(),0x50004);
  assert_eq!(c.read_u64(i.address),Some(0x100000001));
  assert_eq!(c.reg(0),i.address);assert_eq!(c.mapped_permissions(i.address),Some(1));
  assert_eq!(i.compatibility_symbol("/usr/lib/libSystem.B.dylib","__dyld_get_all_image_infos"),Some(i.getter));
  assert_eq!(i.compatibility_symbol("/other","__dyld_get_all_image_infos"),None);
 }
 #[test] fn colliding_data_does_not_leave_code_mapping(){let mut c=cpu();c.map_zeroed(0x24000,0x4000,3).unwrap();assert!(ImageInfos::map(&mut c,0x20000,&[Record{header:0x10000,path:"/Game".into()}]).is_err());assert!(c.mapped_permissions(0x20000).is_none());assert_eq!(c.mapped_permissions(0x24000),Some(3));}
 #[test] fn rejects_bad_or_duplicate_records_before_mapping(){
  let mut c=cpu();let r=Record{header:0x10000,path:"/Game".into()};
  assert!(ImageInfos::map(&mut c,0x20000,&[r.clone(),r]).is_err());assert!(c.mapped_permissions(0x20000).is_none());
  assert!(ImageInfos::map(&mut c,0x20000,&[Record{header:0,path:"/Game".into()}]).is_err());
 }
}

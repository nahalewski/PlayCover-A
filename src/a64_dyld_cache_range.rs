/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Original cache mappedSize ABI, including its real address-space gaps.
use super::{A64Cpu,cache::CachePlan,bridge::{GuestBridge,ReturnValues}};
const ENTRY:u64=0x1a6c7da04;
const ORIGINAL:[u8;24]=[0xe1,3,0,0xaa,0x88,0x85,0x1d,0xf0,0,0xc5,0x41,0xf9,8,0,0x40,0xf9,2,0x0d,0x41,0xf9,0x40,0,0x1f,0xd6];
#[derive(Clone,Debug)]pub(super) struct CacheRange{base:u64,size:u64}
fn geometry(base:u64,size:u64,regions:impl IntoIterator<Item=(u64,u64)>)->Result<CacheRange,String>{
 let end=base.checked_add(size).ok_or("cache range overflow")?;
 if base==0||size==0||size>64*1024*1024*1024{return Err("invalid original cache range".into());}
 for(address,length)in regions{if length==0||address<base||address.checked_add(length).is_none_or(|limit|limit>end){return Err("original cache region exceeds declared mappedSize".into());}}
 Ok(CacheRange{base,size})
}
impl CacheRange{
 pub(super) fn read(cpu:&A64Cpu,plan:&CachePlan)->Result<Self,String>{
  let main=plan.files.first().ok_or("original cache main file absent")?;
  let mut starts=plan.regions.iter().filter(|r|&r.file==main&&r.file_offset==0);
  let base=starts.next().ok_or("original cache header mapping absent")?.vmaddr;
  if starts.next().is_some(){return Err("ambiguous original cache header mapping".into());}
  let mut header=[0;0x190];cpu.read_guest_into(base,&mut header)?;
  if &header[..7]!=b"dyld_v1"||!header[..16].windows(5).any(|s|s==b"arm64"){return Err("cache range header is not original ARM64 cache".into());}
  let offset=u32::from_le_bytes(header[16..20].try_into().unwrap());
  let size=if offset>=0x18c{
   if u64::from_le_bytes(header[0xe0..0xe8].try_into().unwrap())!=base{return Err("original cache sharedRegionStart differs from mapped header".into());}
   u64::from_le_bytes(header[0xe8..0xf0].try_into().unwrap())
  }else{plan.mapped_span};
  geometry(base,size,plan.regions.iter().map(|r|(r.vmaddr,r.size)))
 }
}
pub(super) fn install(cpu:&mut A64Cpu,bridge:&mut GuestBridge,entry:u64,range:CacheRange)->Result<u64,String>{
 if entry!=ENTRY||cpu.mapped_permissions(entry).is_none_or(|p|p&4==0)||cpu.read_bytes(entry,24).ok_or("cache range wrapper is unreadable")?!=ORIGINAL{return Err("original cache range wrapper identity/instructions differ".into());}
 let target=bridge.register_service(cpu,"dyld_get_shared_cache_range_original",move|frame|{
  let output=frame.integer(0)?;
  frame.write(output,&range.size.to_le_bytes())?;
  Ok(ReturnValues::integer(range.base))
 })?.guest_address();
 let mut patch=Vec::new();for i in 0..4u32{let word=(if i==0{0xd2800000}else{0xf2800000})|(i<<21)|((((target>>(i*16))&0xffff)as u32)<<5)|16;patch.extend_from_slice(&word.to_le_bytes());}patch.extend_from_slice(&0xd61f0200u32.to_le_bytes());patch.extend_from_slice(&0xd503201fu32.to_le_bytes());cpu.try_write_bytes(entry,&patch)?;Ok(target)
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn real_span_includes_unmapped_gaps_without_claiming_bytes(){let range=geometry(0x10000,0x90000,[(0x10000,0x4000),(0x90000,0x4000)]).unwrap();assert_eq!(range.base,0x10000);assert_eq!(range.size,0x90000);assert!(geometry(0x10000,0x80000,[(0x90000,0x4000)]).is_err());assert!(geometry(u64::MAX-2,4,[]).is_err());}
 #[test]fn real_guest_redirect_writes_size_and_returns_original_base(){
  let mut cpu=A64Cpu::new_sparse();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();
  cpu.map_zeroed(0x100000,0x4000,3).unwrap();cpu.map_zeroed(ENTRY&!4095,4096,5).unwrap();cpu.try_write_bytes(ENTRY,&ORIGINAL).unwrap();
  let range=geometry(0x180000000,0xa0000000,[(0x180000000,0x4000)]).unwrap();
  let mut wrong=ORIGINAL;wrong[0]^=4;cpu.try_write_bytes(ENTRY,&wrong).unwrap();
  assert!(install(&mut cpu,&mut bridge,ENTRY,range.clone()).is_err());assert_eq!(cpu.read_bytes(ENTRY,24).unwrap(),wrong);
  cpu.try_write_bytes(ENTRY,&ORIGINAL).unwrap();install(&mut cpu,&mut bridge,ENTRY,range).unwrap();
  let result=bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:ENTRY,integers:vec![0x100000],..Default::default()},100).unwrap();
  assert_eq!(result.integers[0],0x180000000);assert_eq!(cpu.read_u64(0x100000),Some(0xa0000000));
  assert!(bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:ENTRY,integers:vec![0],..Default::default()},100).is_err());
  assert_eq!(cpu.read_u64(0x100000),Some(0xa0000000));
 }
}

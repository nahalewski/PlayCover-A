/* MPL-2.0: https://mozilla.org/MPL/2.0/ */
//! Exact selected main-executable identity, not an image-index guess.
use super::{A64Cpu,bridge::{GuestBridge,ReturnValues}};
const ENTRY:u64=0x1a6c7e2b8;
const ORIGINAL:[u8;20]=[0x88,0x85,0x1d,0xd0,0x00,0xc5,0x41,0xf9,0x08,0x00,0x40,0xf9,0x01,0x8d,0x41,0xf9,0x20,0x00,0x1f,0xd6];
pub(super) struct MainHeader{address:u64,original:Vec<u8>}
fn u32at(bytes:&[u8],at:usize)->Result<u32,String>{Ok(u32::from_le_bytes(bytes.get(at..at+4).ok_or("main header field truncated")?.try_into().unwrap()))}
impl MainHeader{
 pub(super) fn read(cpu:&A64Cpu,address:u64)->Result<Self,String>{
  let header=cpu.read_bytes(address,32).ok_or("selected main header unreadable")?;
  let commands=u32at(&header,16)?as usize;let size=u32at(&header,20)?as usize;
  if u32at(&header,0)?!=0xfeedfacf||u32at(&header,4)?!=0x100000c||u32at(&header,12)?!=2||commands==0||commands>4096||size>256*1024||size<commands*8{return Err("selected main header must be bounded ARM64 MH_EXECUTE".into());}
  let bytes=cpu.read_bytes(address.checked_add(32).ok_or("main header overflow")?,size).ok_or("selected main commands unreadable")?;
  let mut offset=0;let mut text=false;
  for _ in 0..commands{let command=u32at(&bytes,offset)?;let length=u32at(&bytes,offset+4)?as usize;let end=offset.checked_add(length).filter(|&end|end<=bytes.len()).ok_or("main load command outside header")?;
   if length<8||length&7!=0{return Err("invalid main load command size".into());}
   if command==0x19{if length<72{return Err("main segment truncated".into());}let data=&bytes[offset..end];let name=data[8..24].split(|&byte|byte==0).next().unwrap();let vm=u64::from_le_bytes(data[24..32].try_into().unwrap());let file=u64::from_le_bytes(data[40..48].try_into().unwrap());let filesize=u64::from_le_bytes(data[48..56].try_into().unwrap());if name==b"__TEXT"&&vm==address&&file==0&&filesize>=32+size as u64{text=true;}}
   offset=end;
  }
  if offset!=bytes.len()||!text||cpu.mapped_permissions(address).is_none_or(|permissions|permissions&4==0){return Err("selected main header has no genuine executable header segment".into());}
  Ok(Self{address,original:header.to_vec()})
 }
}
pub(super) fn install(cpu:&mut A64Cpu,bridge:&mut GuestBridge,entry:u64,main:MainHeader)->Result<u64,String>{
 if entry!=ENTRY||cpu.read_bytes(entry,20).is_none_or(|bytes|bytes!=ORIGINAL)||cpu.mapped_permissions(entry).is_none_or(|p|p&4==0){return Err("original main-header wrapper identity/bytes/RX differs".into());}
 let target=bridge.register_service(cpu,"actual_selected_main_executable_header",move|frame|{if frame.read(main.address,32)?!=main.original{return Err("selected main header identity changed".into());}Ok(ReturnValues::integer(main.address))})?.guest_address();
 let mut patch=Vec::new();for i in 0..4u32{let word=(if i==0{0xd2800000}else{0xf2800000})|(i<<21)|((((target>>(i*16))&0xffff)as u32)<<5)|16;patch.extend_from_slice(&word.to_le_bytes());}patch.extend_from_slice(&0xd61f0200u32.to_le_bytes());cpu.try_write_bytes(entry,&patch)?;Ok(target)
}
#[cfg(test)]mod tests{
 use super::*;
 fn cpu()->A64Cpu{let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();let mut bytes=vec![0;104];for(at,value)in[(0,0xfeedfacfu32),(4,0x100000c),(12,2),(16,1),(20,72),(32,0x19),(36,72)]{bytes[at..at+4].copy_from_slice(&value.to_le_bytes());}bytes[40..46].copy_from_slice(b"__TEXT");bytes[56..64].copy_from_slice(&0x10000u64.to_le_bytes());bytes[80..88].copy_from_slice(&104u64.to_le_bytes());cpu.try_write_bytes(0x10000,&bytes).unwrap();cpu}
 #[test]fn selected_main_identity_requires_execute_and_actual_segment(){let mut cpu=cpu();assert_eq!(MainHeader::read(&cpu,0x10000).unwrap().address,0x10000);cpu.try_write_bytes(0x1000c,&6u32.to_le_bytes()).unwrap();assert!(MainHeader::read(&cpu,0x10000).is_err());cpu.try_write_bytes(0x1000c,&2u32.to_le_bytes()).unwrap();cpu.try_write_bytes(0x10038,&0x11000u64.to_le_bytes()).unwrap();assert!(MainHeader::read(&cpu,0x10000).is_err());}
 #[test]fn real_guest_getter_returns_selected_header(){let mut cpu=cpu();cpu.map_zeroed(ENTRY&!4095,4096,5).unwrap();cpu.try_write_bytes(ENTRY,&ORIGINAL).unwrap();let main=MainHeader::read(&cpu,0x10000).unwrap();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();install(&mut cpu,&mut bridge,ENTRY,main).unwrap();let values=bridge.call(&mut cpu,&super::super::bridge::GuestCall{entry:ENTRY,..Default::default()},1000).unwrap();assert_eq!(values.integers[0],0x10000);}
}

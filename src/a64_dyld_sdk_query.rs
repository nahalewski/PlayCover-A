/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Read the selected executable's SDK, independently of virtual OS version.
//! dyld-1042.1 comparison ABI; only the evidenced fall-2020 set is mapped.
use super::{A64Cpu,bridge::{GuestBridge,ReturnValues}};
pub(super) const ENTRY:u64=0x1a6c7d7e4;
const ORIGINAL:[u8;24]=[0xe1,3,0,0xaa,0x88,0x85,0x1d,0xf0,0,0xc5,0x41,0xf9,8,0,0x40,0xf9,2,0x2d,0x41,0xf9,0x40,0,0x1f,0xd6];
#[derive(Clone,Debug)]
pub(super) struct ProgramSdk {sdk:u32}
fn word(bytes:&[u8],offset:usize)->Result<u32,String>{Ok(u32::from_le_bytes(bytes.get(offset..offset.checked_add(4).ok_or("SDK metadata overflow")?).ok_or("truncated SDK metadata")?.try_into().unwrap()))}
impl ProgramSdk {
 pub(super) fn parse(file:&[u8])->Result<Self,String>{
  if word(file,0)?!=0xfeedfacf||word(file,4)?!=0x100000c||word(file,12)?!=2{return Err("SDK query requires selected ARM64 executable Mach-O".into());}
  let count=word(file,16)? as usize;let end=32usize.checked_add(word(file,20)? as usize).ok_or("SDK load command range overflow")?;
  if count>4096||end>file.len(){return Err("SDK load commands exceed bounded executable metadata".into());}
  let mut at=32usize;let mut selected=None;
  for _ in 0..count {
   let command=word(file,at)?;let size=word(file,at+4)? as usize;let next=at.checked_add(size).ok_or("SDK command overflow")?;
   if size<8||size%8!=0||next>end{return Err("invalid SDK load command size".into());}
   let value=match command {
    0x32=>{if size<24{return Err("truncated LC_BUILD_VERSION".into());}let tools=word(file,at+20)? as usize;if 24usize.checked_add(tools.checked_mul(8).ok_or("SDK tool overflow")?).ok_or("SDK tool overflow")?!=size{return Err("invalid SDK build tool array".into());}if word(file,at+8)?!=2{return Err("SDK query currently supports actual iOS device platform only".into());}Some(word(file,at+16)?)},
    0x25=>{if size!=16{return Err("invalid legacy iOS version command".into());}Some(word(file,at+12)?)},
    _=>None,
   };
   if let Some(value)=value {if value==0{return Err("selected executable has unspecified SDK; no invented SDK version".into());}if selected.is_some_and(|old|old!=value){return Err("conflicting executable SDK metadata".into());}selected=Some(value);}
   at=next;
  }
  if at!=end{return Err("SDK load command count/size disagreement".into());}
  Ok(Self{sdk:selected.ok_or("selected executable has no supported SDK metadata")?})
 }
 pub(super) fn at_least(&self,packed:u64)->Result<bool,String>{
  let platform=packed as u32;let version=(packed>>32) as u32;
  match platform {
   2=>Ok(self.sdk>=version),
   u32::MAX=>{if version!=0x07e40901{return Err(format!("unsupported dyld SDK version set {version:#x}"));}Ok(self.sdk>=0x000e0000)},
   _=>Ok(false),
  }
 }
}
pub(super) fn install(cpu:&mut A64Cpu,bridge:&mut GuestBridge,entry:u64,program:ProgramSdk)->Result<u64,String>{
 if entry!=ENTRY||cpu.mapped_permissions(entry).is_none_or(|p|p&4==0)||cpu.read_bytes(entry,24).ok_or("SDK query wrapper is not readable")?!=ORIGINAL{return Err("original SDK query wrapper identity/instructions differ".into());}
 let target=bridge.register_service(cpu,"dyld_program_sdk_at_least_selected_main",move|frame|Ok(ReturnValues::integer(u64::from(program.at_least(frame.integer(0)?)?))))?.guest_address();
 let mut patch=Vec::new();for i in 0..4u32{let instruction=(if i==0{0xd2800000}else{0xf2800000})|(i<<21)|((((target>>(i*16))&0xffff)as u32)<<5)|16;patch.extend_from_slice(&instruction.to_le_bytes());}patch.extend_from_slice(&0xd61f0200u32.to_le_bytes());patch.extend_from_slice(&0xd503201fu32.to_le_bytes());cpu.try_write_bytes(entry,&patch)?;Ok(target)
}
#[cfg(test)]mod tests {
 use super::*;
 fn executable(sdk:u32,legacy:bool)->Vec<u8>{let mut bytes=vec![0;32];for(offset,value)in[(0,0xfeedfacfu32),(4,0x100000c),(12,2),(16,1),(20,if legacy{16}else{24})]{bytes[offset..offset+4].copy_from_slice(&value.to_le_bytes());}let words=if legacy{vec![0x25,16,0x90000,sdk]}else{vec![0x32,24,2,0x90000,sdk,0]};for value in words{bytes.extend_from_slice(&value.to_le_bytes());}bytes}
 #[test]fn actual_main_sdk_controls_version_checks_not_minimum_or_virtual_os(){
  for legacy in [false,true]{let old=ProgramSdk::parse(&executable(0xd0000,legacy)).unwrap();let current=ProgramSdk::parse(&executable(0xe0000,legacy)).unwrap();assert!(!old.at_least(0x07e40901ffffffff).unwrap());assert!(current.at_least(0x07e40901ffffffff).unwrap());assert!(old.at_least((0xc0000u64<<32)|2).unwrap());assert!(!old.at_least((0xc0000u64<<32)|1).unwrap());assert!(old.at_least(0x07e50901ffffffff).is_err());}
 }
 #[test]fn unspecified_truncated_and_wrong_platform_metadata_is_not_guessed(){assert!(ProgramSdk::parse(&executable(0,false)).is_err());let mut file=executable(0xe0000,false);file[40..44].copy_from_slice(&7u32.to_le_bytes());assert!(ProgramSdk::parse(&file).is_err());assert!(ProgramSdk::parse(&executable(0xe0000,false)[..50]).is_err());}
}

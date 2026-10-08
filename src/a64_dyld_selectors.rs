/* MPL-2.0: https://mozilla.org/MPL/2.0/ */
//! Genuine original cache selector-table lookup; never allocates a new SEL.
//! Format: Apple dyld OptimizerObjC.h StringHashTable/SelectorHashTable.
//! lookup8 is a translation of Bob Jenkins'1997 algorithm, whose embedded
//! notice permits private, educational and commercial use with attribution.
use super::{A64Cpu,cache::CachePlan,bridge::{GuestBridge,ReturnValues}};
const ENTRY:u64=0x1a6c7d874;
const ORIGINAL:[u8;24]=[0xe1,0x03,0x00,0xaa,0x88,0x85,0x1d,0xf0,0x00,0xc5,0x41,0xf9,0x08,0x00,0x40,0xf9,0x02,0x65,0x41,0xf9,0x40,0x00,0x1f,0xd6];
fn mix(a:&mut u64,b:&mut u64,c:&mut u64){for(right,left,third)in[(43,9,8),(38,23,5),(35,49,11),(12,18,22)]{*a=a.wrapping_sub(*b).wrapping_sub(*c)^(*c>>right);*b=b.wrapping_sub(*c).wrapping_sub(*a)^(*a<<left);*c=c.wrapping_sub(*a).wrapping_sub(*b)^(*b>>third);}}
fn lookup8(key:&[u8],salt:u64)->u64{
 let(mut a,mut b,mut c)=(salt,salt,0x9e3779b97f4a7c13u64);let mut chunks=key.chunks_exact(24);
 for chunk in &mut chunks{a=a.wrapping_add(u64::from_le_bytes(chunk[..8].try_into().unwrap()));b=b.wrapping_add(u64::from_le_bytes(chunk[8..16].try_into().unwrap()));c=c.wrapping_add(u64::from_le_bytes(chunk[16..24].try_into().unwrap()));mix(&mut a,&mut b,&mut c);}
 c=c.wrapping_add(key.len()as u64);for(index,&byte)in chunks.remainder().iter().enumerate(){if index<8{a=a.wrapping_add((byte as u64)<<(index*8));}else if index<16{b=b.wrapping_add((byte as u64)<<((index-8)*8));}else{c=c.wrapping_add((byte as u64)<<((index-15)*8));}}mix(&mut a,&mut b,&mut c);c
}
pub(super) struct SelectorTable{base:u64,capacity:u32,shift:u32,mask:u32,salt:u64,scramble:[u32;256],tab:u64,checks:u64,offsets:u64,ranges:Vec<(u64,u64)>}
impl SelectorTable{
 pub(super) fn read(cpu:&A64Cpu,plan:&CachePlan)->Result<Self,String>{
  let main=plan.files.first().ok_or("selector cache file absent")?;let header=plan.regions.iter().find(|region|&region.file==main&&region.file_offset==0).ok_or("selector cache header absent")?.vmaddr;
  let uuid=[0x32,0x03,0x55,0x64,0x85,0x3b,0x38,0x8b,0xa1,0xef,0x2e,0xa3,0x54,0xc1,0x96,0xe1];
  if cpu.read_bytes(header.checked_add(88).ok_or("selector UUID overflow")?,16).is_none_or(|bytes|bytes!=uuid){return Err("selector lookup requires verified original20H392 cache".into());}
  let base=super::cache_objc_context::selector_table(cpu,plan)?;
  let ranges=plan.regions.iter().filter(|region|region.init_prot&1!=0).map(|region|Ok((region.vmaddr,region.vmaddr.checked_add(region.size).ok_or("selector mapping overflow")?))).collect::<Result<Vec<_>,String>>()?;
  let bytes=cpu.read_bytes(base,1056).ok_or("selector table fixed header unreadable")?;
  let field=|at:usize|u32::from_le_bytes(bytes[at..at+4].try_into().unwrap());
  let(capacity,occupied,shift,mask)=(field(4),field(8),field(12),field(16));
  if field(0)!=0||field(20)!=0||capacity==0||capacity>4*1024*1024||occupied>capacity||shift>64||mask==u32::MAX||!(mask+1).is_power_of_two()||mask>4*1024*1024{return Err("unsupported original selector table geometry/version".into());}
  let tab=base.checked_add(1056).ok_or("selector tab overflow")?;let checks=tab.checked_add(mask as u64+1).ok_or("selector checks overflow")?;let offsets=checks.checked_add(capacity as u64).ok_or("selector offsets overflow")?;
  let end=offsets.checked_add(capacity as u64*4).ok_or("selector table overflow")?;
  if !ranges.iter().any(|&(start,limit)|base>=start&&end<=limit){return Err("selector table outside original readable cache extent".into());}
  let mut scramble=[0;256];for(index,word)in bytes[32..].chunks_exact(4).enumerate(){scramble[index]=u32::from_le_bytes(word.try_into().unwrap());}
  Ok(Self{base,capacity,shift,mask,salt:u64::from_le_bytes(bytes[24..32].try_into().unwrap()),scramble,tab,checks,offsets,ranges})
 }
 fn lookup(&self,key:&[u8],mut read:impl FnMut(u64,usize)->Result<Vec<u8>,String>)->Result<Option<u64>,String>{
  if key.len()>4096||key.contains(&0){return Err("selector input exceeds bounded CString contract".into());}
  let val=lookup8(key,self.salt);let tab=read(self.tab+(val&self.mask as u64),1)?;if tab.len()!=1{return Err("selector tab truncated".into());}
  let index=(if self.shift==64{0}else{(val>>self.shift)as u32})^self.scramble[tab[0]as usize];
  if index>=self.capacity{return Err("selector hash index outside capacity".into());}
  let check=read(self.checks+index as u64,1)?;let expected=((key.first().copied().unwrap_or(0)&7)<<5)|(key.len()as u8&31);
  if check.as_slice()!=[expected]{return Ok(None);}
  let offset=read(self.offsets+index as u64*4,4)?;let offset=i32::from_le_bytes(offset.try_into().map_err(|_|"selector offset truncated")?);if offset==0{return Ok(None);}
  let address=u64::try_from(self.base as i128+offset as i128).map_err(|_|"selector canonical address overflow")?;
  let end=address.checked_add(key.len()as u64+1).ok_or("selector CString overflow")?;
  if !self.ranges.iter().any(|&(start,limit)|address>=start&&end<=limit){return Err("selector canonical string outside original cache".into());}
  let bytes=read(address,key.len()+1)?;Ok((bytes.len()==key.len()+1&&bytes[..key.len()]==*key&&bytes[key.len()]==0).then_some(address))
 }
}
pub(super) fn install(cpu:&mut A64Cpu,bridge:&mut GuestBridge,entry:u64,table:SelectorTable)->Result<u64,String>{
 if entry!=ENTRY||cpu.read_bytes(entry,24).is_none_or(|bytes|bytes!=ORIGINAL)||cpu.mapped_permissions(entry).is_none_or(|p|p&4==0){return Err("original selector wrapper identity/bytes/RX differs".into());}
 let service=bridge.register_service(cpu,"original_cache_selector_lookup",move|frame|{
  let input=frame.integer(0)?;if input==0{return Err("selector query has null name".into());}let mut key=Vec::new();
  for offset in 0..=4096{let byte=frame.read(input.checked_add(offset).ok_or("selector input overflow")?,1)?[0];if byte==0{let address=table.lookup(&key,|address,length|frame.read(address,length))?;return Ok(ReturnValues::integer(address.unwrap_or(0)));}if offset==4096{return Err("selector input unterminated".into());}key.push(byte);}
  unreachable!()
 })?.guest_address();
 let mut patch=Vec::new();for i in 0..4u32{let word=(if i==0{0xd2800000}else{0xf2800000})|(i<<21)|((((service>>(i*16))&0xffff)as u32)<<5)|16;patch.extend_from_slice(&word.to_le_bytes());}patch.extend_from_slice(&0xd61f0200u32.to_le_bytes());patch.extend_from_slice(&0xd503201fu32.to_le_bytes());cpu.try_write_bytes(entry,&patch)?;Ok(service)
}
#[cfg(test)]mod tests{
 use super::*;
 fn fixture()->(A64Cpu,SelectorTable){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x1000,0x3000,1).unwrap();cpu.try_write_bytes(0x1000,b"hello\0").unwrap();
  cpu.try_write_bytes(0x2500,&[((b'h'&7)<<5)|5]).unwrap();cpu.try_write_bytes(0x2600,&(-0x1000i32).to_le_bytes()).unwrap();
  (cpu,SelectorTable{base:0x2000,capacity:1,shift:64,mask:0,salt:7,scramble:[0;256],tab:0x2400,checks:0x2500,offsets:0x2600,ranges:vec![(0x1000,0x4000)]})
 }
 #[test]fn canonical_pointer_and_real_hash_collision_miss(){
  let(cpu,table)=fixture();let mut read=|address,length|cpu.read_bytes(address,length).map(|bytes|bytes.to_vec()).ok_or_else(||"unmapped".to_string());
  assert_eq!(table.lookup(b"hello",&mut read).unwrap(),Some(0x1000));
  // Same checkbyte and hash index, but different actual string.
  assert_eq!(table.lookup(b"hallo",&mut read).unwrap(),None);
  assert_eq!(table.lookup(b"other",&mut read).unwrap(),None);
 }
 #[test]fn foreign_canonical_offset_and_truncated_metadata_rejected(){
  let(mut cpu,table)=fixture();cpu.try_write_bytes(0x2600,&0x10000i32.to_le_bytes()).unwrap();
  assert!(table.lookup(b"hello",|address,length|cpu.read_bytes(address,length).map(|bytes|bytes.to_vec()).ok_or_else(||"unmapped".into())).is_err());
  assert!(table.lookup(b"hello",|_,_|Ok(vec![])).is_err());
  assert!(table.lookup(b"a\0b",|_,_|unreachable!()).is_err());
 }
}

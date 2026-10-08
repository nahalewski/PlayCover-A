/* MPL-2.0: https://mozilla.org/MPL/2.0/ */
//! Synchronous original dyld2 add-image notifications with real guest returns.
use super::{A64Cpu,bridge::{GuestBridge,GuestCall,ReturnValues},legacy_dyld_lookup::HelperRegistration};
use std::{rc::Rc,cell::{RefCell,Cell}};
const CALLBACK:u64=0x1808501b8;
const ORIGINAL:[u8;16]=[0xff,0x83,0x02,0xd1,0xfc,0x6f,0x04,0xa9,0xfa,0x67,0x05,0xa9,0xf8,0x5f,0x06,0xa9];
pub(super) struct Image{header:u64,slide:i64,original:Vec<u8>,writable:Vec<(u64,u64)>}
fn u32at(bytes:&[u8],at:usize)->Result<u32,String>{Ok(u32::from_le_bytes(bytes.get(at..at+4).ok_or("legacy add-image metadata truncated")?.try_into().unwrap()))}
fn u64at(bytes:&[u8],at:usize)->Result<u64,String>{Ok(u64::from_le_bytes(bytes.get(at..at+8).ok_or("legacy add-image metadata truncated")?.try_into().unwrap()))}
fn slid(address:u64,slide:i64)->Result<u64,String>{u64::try_from(address as i128+slide as i128).map_err(|_|"legacy TLV section slide overflow".into())}
impl Image{
 pub(super) fn read(cpu:&A64Cpu,header:u64,slide:i64)->Result<Self,String>{
  let original=cpu.read_bytes(header,32).ok_or("selected legacy image header unreadable")?.to_vec();
  if u32at(&original,0)?!=0xfeedfacf||u32at(&original,4)?!=0x100000c||!matches!(u32at(&original,12)?,2|6){return Err("legacy notification requires actualARM64 selectedimage".into());}
  let mut writable=Vec::new();
  if u32at(&original,24)?&0x800000!=0{
   let count=u32at(&original,16)?as usize;let size=u32at(&original,20)?as usize;
   if count==0||count>4096||size>256*1024||size<count*8{return Err("legacy TLV load command budget".into());}
   let bytes=cpu.read_bytes(header.checked_add(32).ok_or("legacy commands overflow")?,size).ok_or("legacy TLV commands unreadable")?;let mut offset=0;
   for _ in 0..count{let command=u32at(&bytes,offset)?;let length=u32at(&bytes,offset+4)?as usize;let end=offset.checked_add(length).filter(|&end|end<=bytes.len()).ok_or("legacy TLV command outside header")?;if length<8||length&7!=0{return Err("legacy TLV command invalidsize".into());}
    if command==0x19{let segment=&bytes[offset..end];if length<72{return Err("legacy TLV segment truncated".into());}let sections=u32at(segment,64)?as usize;if sections>256||72+sections*80>length{return Err("legacy TLV sections outside command".into());}
     let start=slid(u64at(segment,24)?,slide)?;let limit=start.checked_add(u64at(segment,32)?).ok_or("legacy TLV segment overflow")?;
     for index in 0..sections{let section=&segment[72+index*80..72+(index+1)*80];if u32at(section,64)?&255!=0x13{continue;}let size=u64at(section,40)?;if size==0{continue;}if size%24!=0||size>65536*24{return Err("legacy TLV descriptor size invalid".into());}let address=slid(u64at(section,32)?,slide)?;let end=address.checked_add(size).ok_or("legacy TLV descriptor overflow")?;if address<start||end>limit{return Err("legacy TLV descriptors outside actual segment".into());}
      let mut cursor=address;while cursor<end{let region=cpu.protection_region(cursor).ok_or("legacy TLV descriptor unmapped")?;if region.protection&1==0||region.protection&4!=0||region.max_protection&3!=3{return Err("legacy TLV descriptor lacks actualmaximumRW".into());}let next=region.base.checked_add(region.len).ok_or("legacy TLV region overflow")?.min(end);if next<=cursor{return Err("legacy TLV mapping nonprogress".into());}cursor=next;}
      writable.push((address,size));
     }
    }offset=end;
   }if offset!=bytes.len(){return Err("legacy TLV load commands trailingdata".into());}
  }
  writable.sort_unstable();if writable.len()>256{return Err("legacy TLV descriptor range count exceeded".into());}let mut end=0;for &(address,size)in &writable{if address<end{return Err("legacy TLV descriptor ranges overlap".into());}end=address+size;}
  Ok(Self{header,slide,original,writable})
 }
}
#[cfg(test)]
mod tests{
 use super::*;
 fn fixture(count:usize,failure:bool)->(A64Cpu,GuestBridge,Vec<Image>){
  let mut cpu=A64Cpu::new_sparse();
  cpu.map_zeroed(CALLBACK&!4095,4096,5).unwrap();
  cpu.try_write_bytes(CALLBACK,&ORIGINAL).unwrap();
  // Declared synthetic callback body, following the audited production prologue.
  let words:Vec<u32>=if failure{vec![0xd4200000]}else{vec![0xf9408009,0x91000529,0xf9008009,0xf9008401,0x910283ff,0xd65f03c0]};
  let code:Vec<u8>=words.into_iter().flat_map(u32::to_le_bytes).collect();
  cpu.try_write_bytes(CALLBACK+16,&code).unwrap();
  cpu.map_zeroed(0x10000,count*4096,3).unwrap();
  let mut images=Vec::new();
  for index in 0..count{let header=0x10000+index as u64*4096;let mut bytes=[0u8;104];
   for(offset,value)in[(0,0xfeedfacfu32),(4,0x100000c),(12,6),(16,1),(20,72),(32,0x19),(36,72)]{bytes[offset..offset+4].copy_from_slice(&value.to_le_bytes());}
   bytes[40..46].copy_from_slice(b"__TEXT");bytes[56..64].copy_from_slice(&header.to_le_bytes());bytes[64..72].copy_from_slice(&4096u64.to_le_bytes());bytes[80..88].copy_from_slice(&104u64.to_le_bytes());
   cpu.try_write_bytes(header,&bytes).unwrap();images.push(Image::read(&cpu,header,-(index as i64)*4096).unwrap());
  }
  let bridge=GuestBridge::map_runtime(&mut cpu,0x1000000).unwrap();(cpu,bridge,images)
 }
 #[test]fn all_427_callbacks_execute_at_bounded_depth_and_preserve_context(){
  let(mut cpu,mut bridge,images)=fixture(427,false);let helpers=Rc::new(RefCell::new(HelperRegistration::test_registered()));
  let entry=install(&mut cpu,&mut bridge,images,helpers).unwrap();cpu.set_reg(19,0xabc);cpu.set_vector(8,[123,456]);let sp=cpu.sp();let lr=cpu.reg(30);
  bridge.call(&mut cpu,&GuestCall{entry,integers:vec![CALLBACK],..Default::default()},100_000).unwrap();
  for index in 0..427u64{let header=0x10000+index*4096;assert_eq!(cpu.read_u64(header+256),Some(1));assert_eq!(cpu.read_u64(header+264),Some((-(index as i64)*4096)as u64));}
  assert_eq!(cpu.reg(19),0xabc);assert_eq!(cpu.vector(8),[123,456]);assert_eq!(cpu.sp(),sp);assert_eq!(cpu.reg(30),lr);
  assert!(bridge.call(&mut cpu,&GuestCall{entry,integers:vec![CALLBACK],..Default::default()},100_000).is_err());assert_eq!(cpu.read_u64(0x10000+256),Some(1));
 }
 #[test]fn callback_failure_never_advances_to_next_image_or_reports_completion(){
  let(mut cpu,mut bridge,images)=fixture(12,true);let entry=install(&mut cpu,&mut bridge,images,Rc::new(RefCell::new(HelperRegistration::test_registered()))).unwrap();
  let call=GuestCall{entry,integers:vec![CALLBACK],..Default::default()};assert!(bridge.call(&mut cpu,&call,10_000).is_err());assert!(bridge.call(&mut cpu,&call,10_000).is_err());
  for index in 0..12u64{assert_eq!(cpu.read_u64(0x10000+index*4096+256),Some(0));}
 }
}
#[derive(Default)]struct Delivery{callback:Option<u64>,cursor:usize,pending:bool,outcome:Option<Result<(),String>>,complete:bool,failed:bool}
pub(super) fn install(cpu:&mut A64Cpu,bridge:&mut GuestBridge,images:Vec<Image>,helpers:Rc<RefCell<HelperRegistration>>)->Result<u64,String>{
 if images.is_empty()||images.len()>900{return Err("legacy notification count outside bounded sequentialdriver".into());}
 let mut seen=std::collections::HashSet::new();if images.iter().any(|image|!seen.insert(image.header)){return Err("legacy add-image duplicate selectedheader".into());}
 if cpu.read_bytes(CALLBACK,16).is_none_or(|bytes|bytes!=ORIGINAL)||cpu.mapped_permissions(CALLBACK).is_none_or(|p|p&4==0){return Err("original15G77 TLV notification callback identity/bytes/RX differs".into());}
 let state=Rc::new(RefCell::new(Delivery::default()));let target=Rc::new(Cell::new(0));let tail=target.clone();
 let service=bridge.register_service(cpu,"legacy_real_synchronous_add_images",move|frame|{
  helpers.try_borrow().map_err(|_|"legacy helpers borrowed duringnotification")?.require_registered()?;
  let mut current=state.try_borrow_mut().map_err(|_|"legacy add-image registration reentrant")?;
  if current.failed{return Err("legacy add-image partialdelivery quarantined".into());}
  if current.complete{return Err("legacy add-image registration replay refused".into());}
  if current.pending{let outcome=current.outcome.take().ok_or("legacy callback completion missing")?;current.pending=false;if let Err(error)=outcome{current.failed=true;return Err(error);}current.cursor+=1;}
  if current.callback.is_none(){let callback=frame.integer(0)?;if callback!=CALLBACK||frame.read(callback,16)?!=ORIGINAL{return Err("unsupported legacy add-image callback identity".into());}frame.validate_executable_pointer(callback)?;current.callback=Some(callback);echo!("[a64] retained originallegacy TLV add-image callback; synchronously delivering {} actual selectedheaders",images.len());}
  if current.cursor==images.len(){current.complete=true;echo!("[a64] originallegacy add-image callback returned for all{} actual selectedheaders; no runtime readiness receipt",images.len());return Ok(ReturnValues::integer(0));}
  let image=&images[current.cursor];if frame.read(image.header,32)?!=image.original{current.failed=true;return Err("legacy notification image header mutated".into());}
  current.pending=true;let completion=state.clone();drop(current);
  frame.request_guest_call_with_writable_ranges(GuestCall{entry:CALLBACK,integers:vec![image.header,image.slide as u64],..Default::default()},image.writable.clone(),move|result|{let mut state=completion.try_borrow_mut().map_err(|_|"legacy notification completion borrowed")?;state.outcome=Some(result.map(|_|()));Ok(())})?;
  frame.request_tail_dispatch(tail.get(),1)?;Ok(ReturnValues::integer(0))
 })?.guest_address();target.set(service);Ok(service)
}

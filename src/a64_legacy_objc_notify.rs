/* MPL-2.0: https://mozilla.org/MPL/2.0/ */
//! dyld2 three-callback ABI; mapping notification is not an initializer receipt.
use super::{A64Cpu,bridge::{GuestBridge,GuestCall,ReturnValues},dyld_objc_callbacks::ObjcImage};
use std::{rc::Rc,cell::{Cell,RefCell},collections::HashSet};
#[derive(Default)]struct Registration{callbacks:Option<[u64;3]>,pending:bool,outcome:Option<Result<(),String>>,delivered:bool,failed:bool}
/// Failure-only address/length evidence for the observed original protocol scan.
/// No name bytes are returned or printed; inspection is capped at4096 bytes.
pub(super) fn protocol_scan_evidence(cpu:&A64Cpu)->Option<String>{
 if !(0x1800bec9c..0x1800becac).contains(&cpu.pc())||cpu.read_bytes(0x1800beca8,4)?!=0x35ffff8au32.to_le_bytes(){return None;}
 let uuid=[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f];if cpu.read_bytes(0x180000058,16)?!=uuid{return None;}
 let start=cpu.reg(19);let cursor=cpu.reg(8);let distance=cursor.checked_sub(start);let mut length=None;let mut readable=0usize;
 for offset in 0..4096u64{let Some(address)=start.checked_add(offset)else{break;};let Some(byte)=cpu.read_bytes(address,1)else{break;};readable+=1;if byte[0]==0{length=Some(offset);break;}}
 Some(format!("original bounded protocol-name scan start={start:#x} cursor={cursor:#x} cursor_distance={distance:?} mapped_bytes_checked={readable} nul_distance={length:?}; no name bytes logged"))
}
pub(super) fn bottom_up(cpu:&A64Cpu,images:Vec<(u64,String,bool)>)->Result<Vec<(u64,String,bool)>,String>{
 if images.len()>4096{return Err("legacy dependency ordering image budget".into());}
 let paths:std::collections::HashMap<_,_>=images.iter().enumerate().map(|(i,(_,p,_))|(p.as_str(),i)).collect();let mut edges=vec![Vec::new();images.len()];let mut total=0usize;
 for(index,(header,_,_))in images.iter().enumerate(){let h=cpu.read_bytes(*header,32).ok_or("legacy ordering header unreadable")?;let size=u32::from_le_bytes(h[20..24].try_into().unwrap())as usize;let count=u32::from_le_bytes(h[16..20].try_into().unwrap())as usize;if count>4096||size>1024*1024{return Err("legacy ordering metadata budget".into());}let b=cpu.read_bytes(header.checked_add(32).ok_or("legacy ordering overflow")?,size).ok_or("legacy ordering commands unreadable")?;let mut at=0usize;
  for _ in 0..count{let prefix=b.get(at..at+8).ok_or("legacy ordering command truncated")?;let cmd=u32::from_le_bytes(prefix[..4].try_into().unwrap());let len=u32::from_le_bytes(prefix[4..].try_into().unwrap())as usize;if len<8{return Err("legacy ordering command invalid".into());}let end=at.checked_add(len).ok_or("legacy ordering overflow")?;let command=b.get(at..end).ok_or("legacy ordering command extent")?;
   if matches!(cmd,0xc|0x80000018|0x8000001f|0x80000023){let offset=command.get(8..12).ok_or("legacy ordering dylib truncated")?;let offset=u32::from_le_bytes(offset.try_into().unwrap())as usize;let raw=command.get(offset..).ok_or("legacy ordering dylib name extent")?;let nul=raw.iter().position(|&x|x==0).ok_or("legacy ordering dylib name unterminated")?;let name=std::str::from_utf8(&raw[..nul]).map_err(|_|"legacy ordering dylib name invalid")?;
    let target=paths.get(name).copied().or_else(||name.strip_prefix("@rpath/").and_then(|suffix|{let mut matches=images.iter().enumerate().filter(|(_,(_,p,_))|p.ends_with(&format!("/{suffix}")));let first=matches.next()?.0;if matches.next().is_none(){Some(first)}else{None}}));if let Some(target)=target{edges[index].push(target);total+=1;if total>65536{return Err("legacy ordering dependency budget".into());}}
   }at=end;
  }if at!=size{return Err("legacy ordering command count mismatch".into());}
 }
 let mut state=vec![0u8;images.len()];let mut order=Vec::new();
 for root in 0..images.len(){if state[root]!=0{continue;}let mut stack=vec![(root,false)];while let Some((index,finish))=stack.pop(){if finish{state[index]=2;order.push(index);continue;}if state[index]!=0{continue;}state[index]=1;stack.push((index,true));for &dependency in edges[index].iter().rev(){if state[dependency]==0{stack.push((dependency,false));}}if stack.len()>65536{return Err("legacy ordering traversal budget".into());}}}
 Ok(order.into_iter().map(|index|images[index].clone()).collect())
}
pub(super) fn read_image(cpu:&A64Cpu,header:u64,path:String,in_cache:bool)->Result<Option<ObjcImage>,String>{
 let h=cpu.read_bytes(header,32).ok_or("legacy ObjC selected header unreadable")?;
 let flags=u32::from_le_bytes(h[24..28].try_into().unwrap());
 if in_cache&&flags&0x40000000==0{return Ok(None);}
 let count=u32::from_le_bytes(h[16..20].try_into().unwrap())as usize;let size=u32::from_le_bytes(h[20..24].try_into().unwrap())as usize;
 if count>4096||size>1024*1024{return Err("legacy ObjC command budget".into());}
 let commands=cpu.read_bytes(header.checked_add(32).ok_or("legacy ObjC header overflow")?,size).ok_or("legacy ObjC commands unreadable")?;
 let mut at=0;let mut eligible=false;
 for _ in 0..count{let prefix=commands.get(at..at+8).ok_or("legacy ObjC command truncated")?;let cmd=u32::from_le_bytes(prefix[..4].try_into().unwrap());let len=u32::from_le_bytes(prefix[4..].try_into().unwrap())as usize;if len<8{return Err("legacy ObjC command invalid".into());}let end=at.checked_add(len).ok_or("legacy ObjC command overflow")?;let bytes=commands.get(at..end).ok_or("legacy ObjC command extent")?;
  if cmd==0x19{if len<72{return Err("legacy ObjC segment truncated".into());}let n=u32::from_le_bytes(bytes[64..68].try_into().unwrap())as usize;if n>4096||72+n*80!=len{return Err("legacy ObjC section extent".into());}if bytes[8..24].starts_with(b"__DATA"){for section in bytes[72..].chunks_exact(80){if section[..16].starts_with(b"__objc_imageinfo"){eligible=true;}}}}
  at=end;
 }
 if at!=size{return Err("legacy ObjC command count mismatch".into());}
 if !eligible{return Ok(None);}super::dyld_objc_callbacks::ObjcImage::read(cpu,header,path,in_cache)
}
pub(super) fn install(cpu:&mut A64Cpu,bridge:&mut GuestBridge,images:Vec<ObjcImage>)->Result<u64,String>{
 // Address-only samples of original appendHeader globals. These do not change
 // the shared execution budget or stand in for a callback return.
 let uuid=[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f];
 if cpu.read_bytes(0x180000058,16).is_some_and(|bytes|bytes==uuid)
    &&cpu.read_bytes(0x1800bf310,4).is_some_and(|bytes|bytes==0xb900a109u32.to_le_bytes())
    &&cpu.read_bytes(0x1800b3aa4,4).is_some_and(|bytes|bytes==0xa9bd57f6u32.to_le_bytes()){
  bridge.trace_callback_progress(cpu,0x1800b3aa4,0x1800b3aa4,0x1800c3400,0x1b3282090,0x1b3282098)?;
 }
 if images.len()>4096{return Err("legacy ObjC notification image budget".into());}
 let mut seen=HashSet::new();let mut bytes=vec![0u8;images.len()*16];let readonly:Vec<(u64,u64)>=Vec::new();
 for(index,image)in images.iter().enumerate(){if !seen.insert(image.header){return Err("legacy ObjC duplicate selected image".into());}if !image.readonly_ranges().is_empty(){return Err("legacy ObjC constant-data transition requires original-version policy".into());}let offset=bytes.len()as u64;bytes[index*8..index*8+8].copy_from_slice(&offset.to_le_bytes());let at=images.len()*8+index*8;bytes[at..at+8].copy_from_slice(&image.header.to_le_bytes());bytes.extend_from_slice(image.path.as_bytes());bytes.push(0);}
 if bytes.len()>1024*1024{return Err("legacy ObjC notification arguments exceed budget".into());}
 let size=(bytes.len().max(1)as u64+0x3fff)&!0x3fff;let mut arena=bridge.scratch_end()?.checked_add(0x3fff).ok_or("legacy ObjC arena overflow")?&!0x3fff;
 let initial=arena;
 for _ in 0..4096{let end=arena.checked_add(size).ok_or("legacy ObjC arena overflow")?;let collision=(arena..end).find_map(|address|cpu.protection_region(address));if let Some(region)=collision{arena=region.base.checked_add(region.len).and_then(|x|x.checked_add(0x3fff)).ok_or("legacy ObjC arena extent overflow")?&!0x3fff;if arena-initial>16*1024*1024{return Err("legacy ObjC arena search budget".into());}}else{break;}}
 let end=arena.checked_add(size).ok_or("legacy ObjC arena overflow")?;if(arena..end).any(|address|cpu.mapped_permissions(address).is_some()){return Err("legacy ObjC arena overlaps mapped storage".into());}
 for index in 0..images.len(){let offset=u64::from_le_bytes(bytes[index*8..index*8+8].try_into().unwrap());bytes[index*8..index*8+8].copy_from_slice(&arena.checked_add(offset).ok_or("legacy ObjC path overflow")?.to_le_bytes());}
 cpu.map_zeroed(arena,size as usize,1)?;cpu.try_write_bytes(arena,&bytes)?;
 let count=images.len()as u64;let headers=arena+count*8;let state=Rc::new(RefCell::new(Registration::default()));let target=Rc::new(Cell::new(0));let tail=target.clone();
 let service=bridge.register_service(cpu,"legacy_three_callback_ObjC_notify",move|frame|{
  let mut current=state.try_borrow_mut().map_err(|_|"legacy ObjC registration reentrant")?;
  if current.failed{return Err("legacy ObjC partial notification quarantined".into());}
  if current.pending{let outcome=current.outcome.take().ok_or("legacy ObjC mapped completion missing")?;current.pending=false;if let Err(error)=outcome{current.failed=true;return Err(error);}current.delivered=true;echo!("[a64] original legacy ObjC mapped callback returned for {count} selected ObjC images; no +load or initialization receipt");return Ok(ReturnValues::integer(0));}
  if current.callbacks.is_some(){return Err("legacy ObjC callbacks already registered; replay refused".into());}
  let callbacks=[frame.integer(0)?,frame.integer(1)?,frame.integer(2)?];
  const EXPECTED:[u64;3]=[0x1800b3aa4,0x1800b3b0c,0x1800b3cb4];
  const BYTES:[[u8;16];3]=[[0xf6,0x57,0xbd,0xa9,0xf4,0x4f,1,0xa9,0xfd,0x7b,2,0xa9,0xfd,0x83,0,0x91],[0xf4,0x4f,0xbe,0xa9,0xfd,0x7b,1,0xa9,0xfd,0x43,0,0x91,0xf3,3,1,0xaa],[0xf4,0x4f,0xbe,0xa9,0xfd,0x7b,1,0xa9,0xfd,0x43,0,0x91,0xf3,3,1,0xaa]];
  if callbacks!=EXPECTED{return Err("original15G77 three ObjC callback identities differ".into());}for(index,&callback)in callbacks.iter().enumerate(){frame.validate_executable_pointer(callback)?;if frame.read(callback,16)?!=BYTES[index]{return Err("original15G77 ObjC callback instructions differ".into());}}
  current.callbacks=Some(callbacks);
  if count==0{current.delivered=true;return Ok(ReturnValues::integer(0));}
  current.pending=true;let completion=state.clone();drop(current);
  echo!("[a64] retained legacy three-callback ObjC ABI; executing original mapped callback for {count} selected ObjC images");
  frame.request_guest_call_with_writable_ranges(GuestCall{entry:callbacks[0],integers:vec![count,arena,headers],..Default::default()},readonly.clone(),move|result|{completion.try_borrow_mut().map_err(|_|"legacy ObjC completion borrowed")?.outcome=Some(result.map(|_|()));Ok(())})?;
  frame.request_tail_dispatch(tail.get(),1)?;Ok(ReturnValues::integer(0))
 })?.guest_address();target.set(service);Ok(service)
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn protocol_stop_metadata_is_bounded_and_never_returns_name_contents(){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x180000000,4096,1).unwrap();cpu.try_write_bytes(0x180000058,&[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f]).unwrap();cpu.map_zeroed(0x1800be000,4096,5).unwrap();cpu.try_write_bytes(0x1800beca8,&0x35ffff8au32.to_le_bytes()).unwrap();cpu.map_zeroed(0x10000,4096,1).unwrap();cpu.try_write_bytes(0x10000,b"private.name\0").unwrap();cpu.set_pc(0x1800beca8);cpu.set_reg(19,0x10000);cpu.set_reg(8,0x10004);
  let metadata=protocol_scan_evidence(&cpu).unwrap();assert!(metadata.contains("cursor_distance=Some(4)"));assert!(metadata.contains("nul_distance=Some(12)"));assert!(!metadata.contains("private.name"));
  cpu.try_write_bytes(0x10000,&vec![1;4096]).unwrap();let metadata=protocol_scan_evidence(&cpu).unwrap();assert!(metadata.contains("mapped_bytes_checked=4096"));assert!(metadata.contains("nul_distance=None"));cpu.set_pc(0x1800becac);assert!(protocol_scan_evidence(&cpu).is_none());
 }
 fn fixture()->(A64Cpu,GuestBridge,Vec<ObjcImage>){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,3).unwrap();cpu.map_zeroed(0x14000,4096,3).unwrap();let mut bytes=vec![0u8;256];
  for(at,value)in[(0,0xfeedfacfu32),(4,0x100000c),(12,6),(16,2),(20,224)]{bytes[at..at+4].copy_from_slice(&value.to_le_bytes());}
  for(at,address,fileoff,nsects,label)in[(32,0x10000u64,0u64,0u32,b"__TEXT".as_slice()),(104,0x14000,4096,1,b"__DATA".as_slice())]{for(off,value)in[(0,0x19u32),(4,72+nsects*80),(56,3),(60,3),(64,nsects)]{bytes[at+off..at+off+4].copy_from_slice(&value.to_le_bytes());}bytes[at+8..at+8+label.len()].copy_from_slice(label);for(off,value)in[(24,address),(32,4096),(40,fileoff),(48,4096)]{bytes[at+off..at+off+8].copy_from_slice(&value.to_le_bytes());}}
  bytes[176..192].copy_from_slice(b"__objc_imageinfo");bytes[208..216].copy_from_slice(&0x14000u64.to_le_bytes());bytes[216..224].copy_from_slice(&8u64.to_le_bytes());cpu.try_write_bytes(0x10000,&bytes).unwrap();
  cpu.map_zeroed(0x1800b3000,4096,5).unwrap();let first=[0xf6,0x57,0xbd,0xa9,0xf4,0x4f,1,0xa9,0xfd,0x7b,2,0xa9,0xfd,0x83,0,0x91];let other=[0xf4,0x4f,0xbe,0xa9,0xfd,0x7b,1,0xa9,0xfd,0x43,0,0x91,0xf3,3,1,0xaa];
  cpu.try_write_bytes(0x1800b3aa4,&first).unwrap();for entry in[0x1800b3b0c,0x1800b3cb4]{cpu.try_write_bytes(entry,&other).unwrap();}
  // Declared fixture callback verifies direct count/paths/header-array ABI.
  let code:Vec<u8>=[0xf9400049u32,0xf9008120,0x9100c3ff,0xd65f03c0].into_iter().flat_map(u32::to_le_bytes).collect();cpu.try_write_bytes(0x1800b3ab4,&code).unwrap();
  let image=read_image(&cpu,0x10000,"/Fixture".into(),false).unwrap().unwrap();let bridge=GuestBridge::map_runtime(&mut cpu,0x100000).unwrap();(cpu,bridge,vec![image])
 }
 #[test]fn direct_three_callback_abi_executes_mapped_before_return_and_rejects_replay(){let(mut cpu,mut bridge,images)=fixture();let entry=install(&mut cpu,&mut bridge,images).unwrap();let sp=cpu.sp();let call=GuestCall{entry,integers:vec![0x1800b3aa4,0x1800b3b0c,0x1800b3cb4],..Default::default()};bridge.call(&mut cpu,&call,1000).unwrap();assert_eq!(cpu.read_u64(0x10100),Some(1));assert_eq!(cpu.sp(),sp);assert!(bridge.call(&mut cpu,&call,1000).is_err());}
 #[test]fn foreign_callback_rejected_before_publication_then_valid_registration_works(){let(mut cpu,mut bridge,images)=fixture();let entry=install(&mut cpu,&mut bridge,images).unwrap();assert!(bridge.call(&mut cpu,&GuestCall{entry,integers:vec![0x1800b3aa4,0x1800b3b0c,0x1800b3b0c],..Default::default()},1000).is_err());assert_eq!(cpu.read_u64(0x10100),Some(0));bridge.call(&mut cpu,&GuestCall{entry,integers:vec![0x1800b3aa4,0x1800b3b0c,0x1800b3cb4],..Default::default()},1000).unwrap();assert_eq!(cpu.read_u64(0x10100),Some(1));}
 #[test]fn old_cached_predicate_requires_actual_has_objc_flag(){let(cpu,_,_)=fixture();assert!(read_image(&cpu,0x10000,"/Cache".into(),true).unwrap().is_none());}
}

/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Completion of the audited dyld-1042.1 helper association, not runtime readiness.
use super::{dyld_helpers::Helpers,execution_session::SessionLease,
 thread_scheduler_cpu::CpuScheduler,tlv::Plan,tlv_lazy::Catalogue};
use std::{rc::Rc,cell::RefCell};
fn keys(keys:[u64;2])->Result<(),String>{
 if keys.iter().any(|k|!(256..512).contains(k))||keys[0]==keys[1]{
  return Err("dyld global callback keys are invalid or shared".into());
 }Ok(())
}
pub(super) fn validate(lease:&SessionLease,scheduler:&Rc<RefCell<CpuScheduler>>,owner:u64,
 helpers:&Helpers,global_keys:[u64;2],plans:&[Plan],catalogue:&Catalogue,
 lazy_installed:bool,lock_released:bool,mut read:impl FnMut(u64,usize)->Result<Vec<u8>,String>)->Result<(),String>{
 lease.validate(scheduler,owner)?;
 keys(global_keys)?;
 if !lazy_installed||!lock_released{return Err("dyld completion lacks installed lazy route or released loader lock".into());}
 if helpers.object!=super::dyld_helpers::HELPER_OBJECT||plans.len()>4096||catalogue.images.len()!=plans.len(){return Err("dyld completion association/catalogue identity differs".into());}
 let object=read(helpers.object,8)?;
 if object.len()!=8||u64::from_le_bytes(object.try_into().unwrap())!=helpers.vtable{return Err("dyld helper object changed during initialization".into());}
 let table=read(helpers.vtable,176)?;
 if table.len()!=176||helpers.functions.iter().enumerate().any(|(i,f)|u64::from_le_bytes(table[i*8..i*8+8].try_into().unwrap())!=*f){return Err("dyld helper vtable changed during initialization".into());}
 let version=read(helpers.functions[0],8)?;
 if version!=[0xc0,0,0x80,0x52,0xc0,3,0x5f,0xd6]{return Err("dyld completion requires audited helper version six".into());}
 for (index,plan) in plans.iter().enumerate(){
  let image=&catalogue.images[index];
  let slots:Vec<_>=plan.descriptors.iter().map(|d|(d.slot,d.offset)).collect();
  if !plan.initializers.is_empty()||image.header!=plan.header||image.template!=plan.template
   ||image.alignment!=plan.alignment||image.slots!=slots||image.key==0||image.key>=512
   ||global_keys.contains(&image.key)||catalogue.images[..index].iter().any(|i|i.header==image.header||i.key==image.key)
   ||plan.preallocated_key.is_some_and(|key|key!=image.key){return Err("dyld TLV completion differs from actual image plan".into());}
  catalogue.storage.validate_catalogued(owner,image.header,image.key,image.template.len()as u64,&slots)?;
  for &(slot,offset) in &slots{
   let descriptor=read(slot,24)?;
   if descriptor.len()!=24{return Err("truncated published TLV descriptor".into());}
   let words:Vec<_>=descriptor.chunks_exact(8).map(|b|u64::from_le_bytes(b.try_into().unwrap())).collect();
   if words!=[0x1a6c7d8d0,image.key,offset]{return Err("published TLV descriptor changed before completion".into());}
  }
 }
 lease.validate(scheduler,owner)
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn global_keys_require_real_distinct_dynamic_slots(){assert!(keys([256,257]).is_ok());for pair in [[0,257],[126,257],[256,256],[256,512]]{assert!(keys(pair).is_err());}}
}

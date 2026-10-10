/* This Source Code Form is subject to the Mozilla Public License, v. 2.0.
 * https://mozilla.org/MPL/2.0/ */
//! Persistent mapped CPU, bridge, and kernel owner. An attempt is never replayed
//! after guest side effects; one returned initializer is not full runtime ready.
use super::{bridge::GuestCall,cache_linker::PreparedCacheApp,execution_session::ExecutionSession};
use std::path::Path;
pub(super) struct Inputs {
 pub arguments:Vec<u64>,
 pub profile:Profile,
}
pub(super) enum Profile{Modern(ModernInputs),Legacy(Option<super::legacy_initializer_budget::LegacyBudget>)}
pub(super) struct ModernInputs {
 pub slide_route:(u64,super::dyld_slide::ImageSlides),
 pub restricted_entry:u64,
 pub immutable_route:(u64,super::dyld_slide::ImmutableRanges),
 pub tlv_images:Vec<(u64,u64,bool)>,
 pub sdk_query:Option<(u64,super::dyld_sdk_query::ProgramSdk)>,
 pub objc_callbacks:Option<(u64,Vec<super::dyld_objc_callbacks::ObjcImage>)>,
 pub cache_range:Option<(u64,super::dyld_cache_range::CacheRange)>,
 pub dyld_overridden:Option<u64>,
 pub dyld_add_image:Option<u64>,
 pub dyld_objc:Option<super::dyld_objc::DyldObjcEntries>,
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
enum Stage {Prepared,Attempting,Stopped,InitializerReturned}
impl Stage {
 fn begin(&mut self)->Result<(),String>{
  if *self!=Self::Prepared{return Err("cached initializer attempt cannot be replayed; persistent guest state may have side effects".into());}
  *self=Self::Attempting;Ok(())
 }
 fn finish(&mut self,returned:bool){*self=if returned{Self::InitializerReturned}else{Self::Stopped};}
}
pub(super) struct CachedSession {
 app:PreparedCacheApp,
 process:ExecutionSession,
 initializer:GuestCall,
 stage:Stage,
 initializer_budget:u64,
 initializer_permit:Option<super::initializer_budget::InitializerBudget>,
 legacy_budget:Option<super::legacy_initializer_budget::LegacyBudget>,
}
impl CachedSession {
 pub(super) fn prepare(bytes:&[u8],path:&str,cache:&Path,reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool)->Result<Self,String>{
  Self::prepare_inner(bytes,path,cache,reader,with_unity,false)
 }
 pub(super) fn prepare_image_infos(bytes:&[u8],path:&str,cache:&Path,reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool)->Result<Self,String>{
  Self::prepare_inner(bytes,path,cache,reader,with_unity,true)
 }
 fn prepare_inner(bytes:&[u8],path:&str,cache:&Path,reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool,image_infos:bool)->Result<Self,String>{
  let (mut app,inputs)=if image_infos{
   super::cache_linker::prepare_bundle_initialization_image_infos(bytes,path,cache,reader,with_unity)?
  }else{super::cache_linker::prepare_bundle_initialization(bytes,path,cache,reader,with_unity)?};
  let initializer_permit=match &inputs.profile{
   Profile::Modern(routes)=>routes.objc_callbacks.as_ref().filter(|(_,images)|!images.is_empty()).map(|(_,images)|super::initializer_budget::InitializerBudget::verified(&app.link.loaded.cpu,&app._plan,images)).transpose()?,
   Profile::Legacy(_)=>None,
  };
  let services=app.link.host_services.as_mut().ok_or("persistent selected services absent")?;
  let initializer_budget=match &inputs.profile{
   Profile::Legacy(budget)=>budget.as_ref().map_or(100_000,|budget|budget.ticks()),
   Profile::Modern(routes)=>notification_budget(routes.objc_callbacks.as_ref().map_or(0,|(_,images)|images.len()))?,
  };
  let (initialization,legacy_budget)=match inputs.profile{
   Profile::Legacy(budget)=>(super::legacy_session::prepare(&mut app.link.loaded.cpu,&app._plan,services,inputs.arguments,&app.link.main_stack)?,budget),
   Profile::Modern(routes)=>(super::cache_init_probe::prepare_session(&mut app.link.loaded.cpu,&app._plan,services,
      inputs.arguments,&app.link.main_stack,routes.slide_route,routes.restricted_entry,
      routes.immutable_route,routes.tlv_images,routes.sdk_query,routes.objc_callbacks,routes.cache_range,routes.dyld_overridden,routes.dyld_add_image,routes.dyld_objc)?,None),
  };
  Ok(Self{app,process:initialization.session,initializer:initialization.call,stage:Stage::Prepared,initializer_budget,initializer_permit,legacy_budget})
 }
 pub(super) fn initialize_libsystem(&mut self)->Result<(),String>{
  if let Some(budget)=&self.legacy_budget{budget.validate(&self.app.link.loaded.cpu,self.initializer.entry)?;}
  let services=self.app.link.host_services.as_mut().ok_or("persistent selected services absent")?;
  self.stage.begin()?;
  echo!("[a64] bounded retained initializer diagnostic tick allowance={}; no runtime readiness receipt",self.initializer_permit.as_ref().map_or(self.initializer_budget,|permit|permit.ticks()));
  let result=if let Some(permit)=&self.initializer_permit{self.process.call_initializer(&mut self.app.link.loaded.cpu,services,&self.initializer,permit)}else if let Some(permit)=&self.legacy_budget{self.process.call_legacy_initializer(&mut self.app.link.loaded.cpu,services,&self.initializer,permit)}else{self.process.call(&mut self.app.link.loaded.cpu,services,&self.initializer,self.initializer_budget)};
  self.stage.finish(result.is_ok());
  match result {
   Ok(_)=>{
    echo!("[a64] original libSystem initializer returned in retained execution session; same mapped CPU/kernel/TSD ownership retained; full dependency/runtime readiness not issued");Ok(())
   }
   Err(error)=>Err(format!("persistent original libSystem initialization stopped; session quarantined against replay: {error}")),
  }
 }
 pub(super) fn execution_gate(&self)->Result<(),String>{
  self.app.execution_gate()
 }
}
/// Actual original mapper advanced through seven header-list updates within
/// 100k ticks for a verified753-image notification. Reserve finite per-image
/// work (including list traversal), without changing general guest-call caps.
fn notification_budget(count:usize)->Result<u64,String>{
 if count>4096{return Err("notification diagnostic image count exceeds bound".into());}
 let budget=100_000u64.checked_add((count as u64).checked_mul(25_000).ok_or("notification work budget overflow")?).ok_or("notification work budget overflow")?;
 // Respect the existing GuestBridge general-call ceiling. A future larger
 // diagnostic requires a separately audited permit, never a global cap raise.
 Ok(budget.min(1_000_000))
}
pub(super) fn diagnostic(bytes:&[u8],path:&str,cache:&Path,reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool)->Result<(),String>{
 diagnostic_inner(bytes,path,cache,reader,with_unity,false)
}
pub(super) fn diagnostic_image_infos(bytes:&[u8],path:&str,cache:&Path,reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool)->Result<(),String>{
 diagnostic_inner(bytes,path,cache,reader,with_unity,true)
}
fn diagnostic_inner(bytes:&[u8],path:&str,cache:&Path,reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool,image_infos:bool)->Result<(),String>{
 let mut session=if image_infos{CachedSession::prepare_image_infos(bytes,path,cache,reader,with_unity)?}
  else{CachedSession::prepare(bytes,path,cache,reader,with_unity)?};
 let result=session.initialize_libsystem();
 if session.execution_gate().is_ok(){return Err("session diagnostic requires a separate full runtime initialization policy".into());}
 result
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn notification_work_allowance_is_finite_and_scoped(){assert_eq!(notification_budget(0).unwrap(),100_000);assert_eq!(notification_budget(753).unwrap(),1_000_000);assert_eq!(notification_budget(4096).unwrap(),1_000_000);assert!(notification_budget(4097).is_err());}
 #[test]fn partially_executed_attempts_are_not_replayed(){let mut s=Stage::Prepared;s.begin().unwrap();s.finish(false);assert_eq!(s,Stage::Stopped);assert!(s.begin().is_err());}
 #[test]fn returned_initializer_does_not_become_general_ready(){let mut s=Stage::Prepared;s.begin().unwrap();s.finish(true);assert_eq!(s,Stage::InitializerReturned);assert!(s.begin().is_err());}
}

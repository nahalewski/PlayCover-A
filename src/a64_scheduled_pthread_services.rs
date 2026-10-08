/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Dynamic scheduler identity for selected diagnostic pthread callbacks.
//! Call/switch boundaries must remain outside GuestBridge's scratch stacks.
#[path="a64_pthread_mutex.rs"]mod mutex;
#[path="a64_pthread_cond.rs"]mod cond;
use super::{A64Cpu,bridge::{GuestBridge,ServiceFrame,ReturnValues,ServiceId},thread_scheduler_cpu::CpuScheduler};
use std::{cell::RefCell,rc::Rc,collections::BTreeMap};
#[derive(Clone,Default)]struct State{mutexes:mutex::Mutexes,attrs:BTreeMap<u64,mutex::Kind>,conditions:cond::Conditions}
const MUTEX_SIG:u64=0x4d555458;
const ATTR_SIG:u64=0x4d545841;
const COND_STATIC:u64=0x3cb0b1bb;
const COND_PRISTINE:u64=0x434f4e44;
pub(super) fn register(bridge:&mut GuestBridge,cpu:&mut A64Cpu,scheduler:Rc<RefCell<CpuScheduler>>)->Result<Vec<(&'static str,ServiceId)>,String>{
    let state=Rc::new(RefCell::new(State::default()));let mut exports=Vec::new();
    for name in ["_pthread_key_create","_pthread_key_delete","_pthread_setspecific","_pthread_getspecific","_pthread_mutex_init","_pthread_mutex_destroy","_pthread_mutex_lock","_pthread_mutex_trylock","_pthread_mutex_unlock","_pthread_mutexattr_init","_pthread_mutexattr_destroy","_pthread_mutexattr_settype","_pthread_cond_init","_pthread_cond_destroy","_pthread_cond_signal","_pthread_cond_broadcast","_pthread_cond_wait","_pthread_cond_timedwait","_pthread_cond_timedwait_relative_np"]{
        let owner=scheduler.clone();let shared=state.clone();exports.push((name,bridge.register_service(cpu,name,move|frame|{
            let thread=owner.try_borrow().map_err(|_|"scheduler is already borrowed during pthread callback")?.service_identity().ok_or("pthread callback has no selected guest thread or owned teardown")?;
            let value=match name{
                "_pthread_key_create"=>{let output=frame.integer(0)?;let destructor=frame.integer(1)?;let created=owner.borrow_mut().create_key(destructor);match created{Ok(key)=>{if let Err(error)=frame.write(output,&key.to_le_bytes()){owner.borrow_mut().delete_key(key).map_err(|_|"scheduled TLS allocation rollback failed")?;return Err(error)}0},Err(errno)=>errno as u64}},
                "_pthread_key_delete"=>owner.borrow_mut().delete_key(frame.integer(0)?).err().unwrap_or(0)as u64,
                "_pthread_setspecific"=>owner.borrow_mut().set_tls(thread,frame.integer(0)?,frame.integer(1)?).err().unwrap_or(0)as u64,
                "_pthread_getspecific"=>owner.borrow().get_tls(thread,frame.integer(0)?)?,
                _=>{let mut next=shared.borrow().clone();let value=if name.starts_with("_pthread_cond_"){condition(frame,&mut next,name,thread.0)?}else{mutex(frame,&mut next,name,thread.0)?};*shared.borrow_mut()=next;value},
            };Ok(ReturnValues::integer(value))
        })?));
    }Ok(exports)
}
fn mutex(frame:&mut ServiceFrame<'_>,state:&mut State,name:&str,thread:u64)->Result<u64,String>{
    let addr=frame.integer(0)?;if addr==0||addr%8!=0{return Ok(mutex::EINVAL)}let attr=name.starts_with("_pthread_mutexattr_");let bytes=frame.read(addr,if attr{16}else{64})?;let sig=u64::from_le_bytes(bytes[..8].try_into().unwrap());
    if attr{return match name{
        "_pthread_mutexattr_init"=>{if state.attrs.contains_key(&addr){Ok(mutex::EBUSY)}else if state.attrs.len()>=4096{Ok(mutex::EAGAIN)}else{let mut data=[0u8;16];data[..8].copy_from_slice(&ATTR_SIG.to_le_bytes());frame.write(addr,&data)?;state.attrs.insert(addr,mutex::Kind::Normal);Ok(0)}},
        "_pthread_mutexattr_destroy"=>{if sig!=ATTR_SIG||!state.attrs.contains_key(&addr){Ok(mutex::EINVAL)}else{frame.write(addr,&[0u8;16])?;state.attrs.remove(&addr);Ok(0)}},
        "_pthread_mutexattr_settype"=>{let kind=mutex::Kind::from_raw(frame.integer(1)?);if sig!=ATTR_SIG||!state.attrs.contains_key(&addr)||kind.is_none(){Ok(mutex::EINVAL)}else{state.attrs.insert(addr,kind.unwrap());Ok(0)}},_=>unreachable!(),
    }}
    if name!="_pthread_mutex_init"&&!state.mutexes.contains(addr){let kind=match sig{0x32aaaba7=>Some(mutex::Kind::Normal),0x32aaaba1=>Some(mutex::Kind::ErrorCheck),0x32aaaba2=>Some(mutex::Kind::Recursive),_=>None};let Some(kind)=kind else{return Ok(mutex::EINVAL)};if bytes[8..].iter().any(|&b|b!=0){return Err("foreign mutated static mutex storage unsupported".into())}let errno=state.mutexes.init(addr,kind)?;if errno!=0{return Ok(errno)}}
    else if name!="_pthread_mutex_init"&&sig!=MUTEX_SIG&&!matches!(sig,0x32aaaba7|0x32aaaba1|0x32aaaba2){return Err("managed mutex signature overwritten".into())}
    match name{
        "_pthread_mutex_init"=>{let attr=frame.integer(1)?;let kind=if attr==0{mutex::Kind::Normal}else{let data=frame.read(attr,16)?;if u64::from_le_bytes(data[..8].try_into().unwrap())!=ATTR_SIG{return Ok(mutex::EINVAL)}*state.attrs.get(&attr).ok_or("foreign mutex attributes unsupported")?};let errno=state.mutexes.init(addr,kind)?;if errno==0{let mut data=[0u8;64];data[..8].copy_from_slice(&MUTEX_SIG.to_le_bytes());frame.write(addr,&data)?}Ok(errno)},
        "_pthread_mutex_destroy"=>{let errno=state.mutexes.destroy(addr);if errno==0{frame.write(addr,&[0u8;64])?}Ok(errno)},
        "_pthread_mutex_lock"=>state.mutexes.lock(addr,thread,false),
        "_pthread_mutex_trylock"=>state.mutexes.lock(addr,thread,true),
        "_pthread_mutex_unlock"=>state.mutexes.unlock(addr,thread),_=>unreachable!(),
    }
}
fn condition(frame:&mut ServiceFrame<'_>,state:&mut State,name:&str,thread:u64)->Result<u64,String>{
    let addr=frame.integer(0)?;if addr==0||addr%4!=0{return Ok(cond::EINVAL)}let bytes=frame.read(addr,48)?;let sig=u64::from_le_bytes(bytes[..8].try_into().unwrap());
    if name!="_pthread_cond_init"{if !state.conditions.contains(addr){if sig!=COND_STATIC{return Ok(cond::EINVAL)}if bytes[8..].iter().any(|&b|b!=0){return Err("mutated static condition storage unsupported".into())}let errno=state.conditions.init(addr);if errno!=0{return Ok(errno)}}else if !matches!(sig,COND_STATIC|COND_PRISTINE){return Err("managed condition signature overwritten".into())}}
    match name{
        "_pthread_cond_init"=>{if frame.integer(1)?!=0{return Err("foreign/process-shared condition attributes unsupported".into())}let errno=state.conditions.init(addr);if errno==0{let mut data=[0u8;48];data[..8].copy_from_slice(&COND_PRISTINE.to_le_bytes());frame.write(addr,&data)?}Ok(errno)},
        "_pthread_cond_destroy"=>{let errno=state.conditions.destroy(addr);if errno==0{frame.write(addr,&[0u8;48])?}Ok(errno)},
        "_pthread_cond_signal"|"_pthread_cond_broadcast"=>{let errno=state.conditions.signal(addr);if errno==0&&sig==COND_STATIC{frame.write(addr,&COND_PRISTINE.to_le_bytes())?}Ok(errno)},
        _=>state.conditions.wait(addr,thread),
    }
}
#[cfg(test)]mod tests{
    use super::*;use super::super::bridge::GuestCall;
    fn destructor_fixture(limit:usize)->(bool,u64,super::super::thread_scheduler_cpu::Phase){
        use super::super::thread_scheduler_cpu::{finish_exit_shared,Phase};
        let mut cpu=A64Cpu::new_sparse();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();cpu.map_zeroed(0x40000,4096,3).unwrap();cpu.map_zeroed(0x50000,4096,5).unwrap();cpu.map_zeroed(0x60000,4096,3).unwrap();cpu.set_pc(0x50000);cpu.set_sp(0x61000);let owner=Rc::new(RefCell::new(CpuScheduler::default()));let id=owner.borrow_mut().adopt(&cpu,(0x60000,0x61000)).unwrap();let exports=register(&mut bridge,&mut cpu,owner.clone()).unwrap();let set=exports.iter().find(|(n,_)|*n=="_pthread_setspecific").unwrap().1.guest_address();let key=owner.borrow_mut().create_key(0x50000).unwrap();owner.borrow_mut().set_tls(id,key,42).unwrap();
        // Count real guest execution in memory, repopulate the same TLS key
        // through the registered service, and tail-call to preserve guest LR.
        let code=[0xd2a00082u32,0xf9400443,0x91000463,0xf9000443,0xd2800000|((key as u32)<<5),0xd2800001|(99<<5),0xd2800010|(((set&0xffff)as u32)<<5),0xf2a00010|((((set>>16)&0xffff)as u32)<<5),0xd61f0200];let bytes:Vec<u8>=code.iter().flat_map(|w|w.to_le_bytes()).collect();cpu.write_bytes(0x50000,&bytes);
        owner.borrow_mut().select(&mut cpu).unwrap();owner.borrow_mut().begin_exit(&cpu).unwrap();cpu.set_reg(3,777);let result=finish_exit_shared(&owner,&mut cpu,&mut bridge,id,limit,100);assert_eq!(cpu.reg(3),777);assert_eq!(cpu.sp(),0x61000);assert!(owner.borrow().service_identity().is_none());let phase=owner.borrow().phase(id).unwrap();assert!(matches!(phase,Phase::Exited|Phase::Quarantined));(result.is_ok(),cpu.read_u64(0x40008).unwrap(),phase)
    }
    #[test]fn teardown_callbacks_use_exiting_thread_without_refcell_borrow(){let(ok,count,phase)=destructor_fixture(8);assert!(ok);assert_eq!(count,4);assert_eq!(phase,super::super::thread_scheduler_cpu::Phase::Exited);}
    #[test]fn teardown_budget_quarantines_without_replay(){let(ok,count,phase)=destructor_fixture(2);assert!(!ok);assert_eq!(count,2);assert_eq!(phase,super::super::thread_scheduler_cpu::Phase::Quarantined);}
    #[test]fn switched_threads_isolate_tls_and_mutex_ownership(){
        let mut cpu=A64Cpu::new_sparse();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();cpu.map_zeroed(0x40000,4096,3).unwrap();cpu.map_zeroed(0x50000,4096,5).unwrap();cpu.map_zeroed(0x60000,8192,3).unwrap();cpu.write_bytes(0x50000,&0xd65f03c0u32.to_le_bytes());cpu.set_pc(0x50000);cpu.set_sp(0x61000);cpu.set_reg(3,111);let scheduler=Rc::new(RefCell::new(CpuScheduler::default()));let a=scheduler.borrow_mut().adopt(&cpu,(0x60000,0x61000)).unwrap();cpu.set_sp(0x62000);cpu.set_reg(3,222);let b=scheduler.borrow_mut().adopt(&cpu,(0x61000,0x62000)).unwrap();let exports=register(&mut bridge,&mut cpu,scheduler.clone()).unwrap();
        let invoke=|cpu:&mut A64Cpu,bridge:&mut GuestBridge,name:&str,args:Vec<u64>|{let entry=exports.iter().find(|(n,_)|*n==name).unwrap().1.guest_address();bridge.call(cpu,&GuestCall{entry,integers:args,..Default::default()},100)};
        assert!(invoke(&mut cpu,&mut bridge,"_pthread_getspecific",vec![256]).is_err());assert_eq!(scheduler.borrow_mut().select(&mut cpu).unwrap(),Some(a));
        assert_eq!(invoke(&mut cpu,&mut bridge,"_pthread_key_create",vec![0x40000,0]).unwrap().integers[0],0);let key=cpu.read_u64(0x40000).unwrap();assert_eq!(invoke(&mut cpu,&mut bridge,"_pthread_setspecific",vec![key,11]).unwrap().integers[0],0);assert_eq!(invoke(&mut cpu,&mut bridge,"_pthread_mutex_init",vec![0x40100,0]).unwrap().integers[0],0);assert_eq!(invoke(&mut cpu,&mut bridge,"_pthread_mutex_lock",vec![0x40100]).unwrap().integers[0],0);assert_eq!(cpu.sp(),0x61000);scheduler.borrow_mut().yield_current(&cpu).unwrap();assert_eq!(scheduler.borrow_mut().select(&mut cpu).unwrap(),Some(b));assert_eq!(cpu.reg(3),222);
        assert_eq!(invoke(&mut cpu,&mut bridge,"_pthread_getspecific",vec![key]).unwrap().integers[0],0);assert_eq!(invoke(&mut cpu,&mut bridge,"_pthread_setspecific",vec![key,22]).unwrap().integers[0],0);assert_eq!(invoke(&mut cpu,&mut bridge,"_pthread_mutex_trylock",vec![0x40100]).unwrap().integers[0],mutex::EBUSY);assert!(invoke(&mut cpu,&mut bridge,"_pthread_mutex_lock",vec![0x40100]).is_err());assert!(invoke(&mut cpu,&mut bridge,"_pthread_mutex_unlock",vec![0x40100]).is_err());assert_eq!(cpu.sp(),0x62000);scheduler.borrow_mut().yield_current(&cpu).unwrap();assert_eq!(scheduler.borrow_mut().select(&mut cpu).unwrap(),Some(a));assert_eq!(cpu.reg(3),111);assert_eq!(invoke(&mut cpu,&mut bridge,"_pthread_getspecific",vec![key]).unwrap().integers[0],11);assert_eq!(invoke(&mut cpu,&mut bridge,"_pthread_mutex_unlock",vec![0x40100]).unwrap().integers[0],0);
        assert_eq!(invoke(&mut cpu,&mut bridge,"_pthread_cond_init",vec![0x40200,0]).unwrap().integers[0],0);assert!(invoke(&mut cpu,&mut bridge,"_pthread_cond_wait",vec![0x40200,0x40100]).is_err());
    }
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Selected diagnostic mutex calls only. Host state is not Darwin's internal
//! kernel lock layout; never mix these objects with original libpthread calls.
#[path="a64_pthread_mutex.rs"]mod mutex;
use super::{bridge::{GuestBridge,ReturnValues,ServiceId},A64Cpu};
use std::{cell::RefCell,rc::Rc,collections::BTreeMap};
const MUTEX_SIG:u64=0x4d555458;
const ATTR_SIG:u64=0x4d545841;
#[derive(Clone,Default)]struct State{mutexes:mutex::Mutexes,attrs:BTreeMap<u64,mutex::Kind>}
pub(super) fn register(bridge:&mut GuestBridge,cpu:&mut A64Cpu,thread:u64)->Result<Vec<(&'static str,ServiceId)>,String>{
    if thread==0{return Err("mutex services require explicit diagnostic thread identity".into())}
    let state=Rc::new(RefCell::new(State::default()));let mut exports=Vec::new();
    for name in ["_pthread_mutex_init","_pthread_mutex_destroy","_pthread_mutex_lock","_pthread_mutex_trylock","_pthread_mutex_unlock","_pthread_mutexattr_init","_pthread_mutexattr_destroy","_pthread_mutexattr_settype"]{
        let shared=state.clone();exports.push((name,bridge.register_service(cpu,name,move|frame|{
            // Transactional copy: guest memory validation/writes precede commit.
            let addr=frame.integer(0)?;if addr==0||addr%8!=0{return Ok(ReturnValues::integer(mutex::EINVAL))}
            let mut next=shared.borrow().clone();let attr=name.starts_with("_pthread_mutexattr_");
            let bytes=frame.read(addr,if attr{16}else{64})?;let sig=u64::from_le_bytes(bytes[..8].try_into().unwrap());
            let result=if attr{match name{
                "_pthread_mutexattr_init"=>{if next.attrs.contains_key(&addr){mutex::EBUSY}else if next.attrs.len()>=4096{mutex::EAGAIN}else{let mut data=[0u8;16];data[..8].copy_from_slice(&ATTR_SIG.to_le_bytes());frame.write(addr,&data)?;next.attrs.insert(addr,mutex::Kind::Normal);0}},
                "_pthread_mutexattr_destroy"=>{if sig!=ATTR_SIG||!next.attrs.contains_key(&addr){mutex::EINVAL}else{frame.write(addr,&[0u8;16])?;next.attrs.remove(&addr);0}},
                "_pthread_mutexattr_settype"=>{let kind=mutex::Kind::from_raw(frame.integer(1)?);if sig!=ATTR_SIG||!next.attrs.contains_key(&addr)||kind.is_none(){mutex::EINVAL}else{next.attrs.insert(addr,kind.unwrap());0}},
                _=>unreachable!(),
            }}else{
                if name!="_pthread_mutex_init"&&!next.mutexes.contains(addr){
                    let kind=match sig{0x32aaaba7=>Some(mutex::Kind::Normal),0x32aaaba1=>Some(mutex::Kind::ErrorCheck),0x32aaaba2=>Some(mutex::Kind::Recursive),_=>None};
                    if let Some(kind)=kind{if bytes[8..].iter().any(|&b|b!=0){return Err("unsupported mutated Darwin static mutex layout".into())}let errno=next.mutexes.init(addr,kind)?;if errno!=0{return Ok(ReturnValues::integer(errno))}}
                    else{return Ok(ReturnValues::integer(mutex::EINVAL))}
                }else if name!="_pthread_mutex_init"&&sig!=MUTEX_SIG&&!matches!(sig,0x32aaaba7|0x32aaaba1|0x32aaaba2){return Err("managed mutex guest signature was overwritten".into())}
                match name{
                    "_pthread_mutex_init"=>{let ap=frame.integer(1)?;let kind=if ap==0{mutex::Kind::Normal}else{let data=frame.read(ap,16)?;if u64::from_le_bytes(data[..8].try_into().unwrap())!=ATTR_SIG{return Ok(ReturnValues::integer(mutex::EINVAL))}let Some(kind)=next.attrs.get(&ap).copied()else{return Err("foreign mutex attributes are unsupported".into())};kind};let errno=next.mutexes.init(addr,kind)?;if errno==0{let mut data=[0u8;64];data[..8].copy_from_slice(&MUTEX_SIG.to_le_bytes());frame.write(addr,&data)?}errno},
                    "_pthread_mutex_destroy"=>{let errno=next.mutexes.destroy(addr);if errno==0{frame.write(addr,&[0u8;64])?}errno},
                    "_pthread_mutex_lock"=>next.mutexes.lock(addr,thread,false)?,
                    "_pthread_mutex_trylock"=>next.mutexes.lock(addr,thread,true)?,
                    "_pthread_mutex_unlock"=>next.mutexes.unlock(addr,thread)?,
                    _=>unreachable!(),
                }
            };*shared.borrow_mut()=next;Ok(ReturnValues::integer(result))
        })?));
    }Ok(exports)
}
#[cfg(test)]mod tests{
    use super::*;use super::super::bridge::GuestCall;
    #[test]fn real_guest_recursive_mutex_calls(){let mut cpu=A64Cpu::new_sparse();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();cpu.map_zeroed(0x40000,4096,3).unwrap();let exports=register(&mut bridge,&mut cpu,1).unwrap();let mut call=|name:&str,args:Vec<u64>|{let entry=exports.iter().find(|(n,_)|*n==name).unwrap().1.guest_address();bridge.call(&mut cpu,&GuestCall{entry,integers:args,..Default::default()},100).unwrap().integers[0]};assert_eq!(call("_pthread_mutexattr_init",vec![0x40100]),0);assert_eq!(call("_pthread_mutexattr_settype",vec![0x40100,2]),0);assert_eq!(call("_pthread_mutex_init",vec![0x40000,0x40100]),0);for _ in 0..2{assert_eq!(call("_pthread_mutex_lock",vec![0x40000]),0)}assert_eq!(call("_pthread_mutex_destroy",vec![0x40000]),mutex::EBUSY);for _ in 0..2{assert_eq!(call("_pthread_mutex_unlock",vec![0x40000]),0)}assert_eq!(call("_pthread_mutex_destroy",vec![0x40000]),0);}
    #[test]fn static_errorcheck_and_invalid_output(){let mut cpu=A64Cpu::new_sparse();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();cpu.map_zeroed(0x40000,4096,3).unwrap();cpu.write_bytes(0x40000,&0x32aaaba1u64.to_le_bytes());let exports=register(&mut bridge,&mut cpu,9).unwrap();let entry=exports.iter().find(|(n,_)|*n=="_pthread_mutex_lock").unwrap().1.guest_address();for expected in [0,mutex::EDEADLK]{assert_eq!(bridge.call(&mut cpu,&GuestCall{entry,integers:vec![0x40000],..Default::default()},100).unwrap().integers[0],expected)}assert!(bridge.call(&mut cpu,&GuestCall{entry,integers:vec![0xdead000],..Default::default()},100).is_err());}
}

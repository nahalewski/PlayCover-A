/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Diagnostic-only condition lifecycle. Never mix host-managed objects with
//! original Darwin pthread routines, kernel wait queues or process sharing.
#[path="a64_pthread_cond.rs"]mod cond;
use super::{bridge::{GuestBridge,ReturnValues,ServiceId},A64Cpu};
use std::{cell::RefCell,rc::Rc};
const STATIC_SIG:u64=0x3cb0b1bb;
const PRISTINE_SIG:u64=0x434f4e44;
pub(super) fn register(bridge:&mut GuestBridge,cpu:&mut A64Cpu,thread:u64)->Result<Vec<(&'static str,ServiceId)>,String>{
    if thread==0{return Err("condition services require explicit diagnostic thread identity".into())}
    let state=Rc::new(RefCell::new(cond::Conditions::default()));let mut exports=Vec::new();
    for name in ["_pthread_cond_init","_pthread_cond_destroy","_pthread_cond_signal","_pthread_cond_broadcast","_pthread_cond_wait","_pthread_cond_timedwait","_pthread_cond_timedwait_relative_np"]{
        let shared=state.clone();exports.push((name,bridge.register_service(cpu,name,move|frame|{
            let addr=frame.integer(0)?;if addr==0||addr%4!=0{return Ok(ReturnValues::integer(cond::EINVAL))}
            let bytes=frame.read(addr,48)?;let sig=u64::from_le_bytes(bytes[..8].try_into().unwrap());let mut next=shared.borrow().clone();
            if name!="_pthread_cond_init"{
                if !next.contains(addr){if sig!=STATIC_SIG{return Ok(ReturnValues::integer(cond::EINVAL))}if bytes[8..].iter().any(|&b|b!=0){return Err("unsupported mutated static condition storage".into())}let errno=next.init(addr);if errno!=0{return Ok(ReturnValues::integer(errno))}}
                else if !matches!(sig,STATIC_SIG|PRISTINE_SIG){return Err("managed condition signature overwritten".into())}
            }
            let value=match name{
                "_pthread_cond_init"=>{if frame.integer(1)?!=0{return Err("foreign/process-shared condition attributes unsupported".into())}let errno=next.init(addr);if errno==0{let mut data=[0u8;48];data[..8].copy_from_slice(&PRISTINE_SIG.to_le_bytes());frame.write(addr,&data)?}errno},
                "_pthread_cond_destroy"=>{let errno=next.destroy(addr);if errno==0{frame.write(addr,&[0u8;48])?}errno},
                "_pthread_cond_signal"|"_pthread_cond_broadcast"=>{let errno=next.signal(addr);if errno==0&&sig==STATIC_SIG{frame.write(addr,&PRISTINE_SIG.to_le_bytes())?}errno},
                // Reject all waiting paths before changing the condition or
                // touching/releasing the passed mutex. Expired waits also need
                // actual ownership/reacquisition; do not synthesize timeout.
                _=>next.wait(addr,thread)?,
            };*shared.borrow_mut()=next;Ok(ReturnValues::integer(value))
        })?));
    }Ok(exports)
}
#[cfg(test)]mod tests{
    use super::*;use super::super::bridge::GuestCall;
    #[test]fn real_guest_empty_lifecycle(){let mut cpu=A64Cpu::new_sparse();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();cpu.map_zeroed(0x40000,4096,3).unwrap();let exports=register(&mut bridge,&mut cpu,1).unwrap();let mut call=|name:&str,args:Vec<u64>|{let entry=exports.iter().find(|(n,_)|*n==name).unwrap().1.guest_address();bridge.call(&mut cpu,&GuestCall{entry,integers:args,..Default::default()},100)};assert_eq!(call("_pthread_cond_init",vec![0x40004,0]).unwrap().integers[0],0);assert_eq!(call("_pthread_cond_signal",vec![0x40004]).unwrap().integers[0],0);assert!(call("_pthread_cond_wait",vec![0x40004,0x40100]).is_err());assert_eq!(call("_pthread_cond_broadcast",vec![0x40004]).unwrap().integers[0],0);assert_eq!(call("_pthread_cond_destroy",vec![0x40004]).unwrap().integers[0],0);}
    #[test]fn static_lazy_signal_and_wait_error_preserve_mutex(){let mut cpu=A64Cpu::new_sparse();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();cpu.map_zeroed(0x40000,4096,3).unwrap();cpu.write_bytes(0x40000,&STATIC_SIG.to_le_bytes());cpu.write_bytes(0x40100,&123u64.to_le_bytes());let exports=register(&mut bridge,&mut cpu,2).unwrap();let entry=|name:&str|exports.iter().find(|(n,_)|*n==name).unwrap().1.guest_address();assert!(bridge.call(&mut cpu,&GuestCall{entry:entry("_pthread_cond_timedwait"),integers:vec![0x40000,0x40100,0],..Default::default()},100).is_err());assert_eq!(cpu.read_u64(0x40000),Some(STATIC_SIG));assert_eq!(cpu.read_u64(0x40100),Some(123));assert_eq!(bridge.call(&mut cpu,&GuestCall{entry:entry("_pthread_cond_signal"),integers:vec![0x40000],..Default::default()},100).unwrap().integers[0],0);assert_eq!(cpu.read_u64(0x40000),Some(PRISTINE_SIG));}
}

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Explicit diagnostic guest-thread TLS services. No pthread_create/self,
//! scheduling, Mach thread object or static Darwin TSD slot fabrication.
#[path="a64_pthread_tls.rs"]mod tls;
use super::{bridge::{GuestBridge,GuestCall,ReturnValues,ServiceId},A64Cpu};
use std::{cell::RefCell,rc::Rc};
pub(super) struct Services{pub exports:Vec<(&'static str,ServiceId)>,state:Rc<RefCell<tls::Tls>>,thread:tls::ThreadId}
pub(super) fn register(bridge:&mut GuestBridge,cpu:&mut A64Cpu,thread_id:u64)->Result<Services,String>{
    let thread=tls::ThreadId(thread_id);let mut state=tls::Tls::default();state.register_thread(thread)?;let state=Rc::new(RefCell::new(state));let mut exports=Vec::new();
    for name in ["_pthread_key_create","_pthread_key_delete","_pthread_setspecific","_pthread_getspecific"]{
        let shared=state.clone();exports.push((name,bridge.register_service(cpu,name,move|frame|{
            let value=match name{
                "_pthread_key_create"=>{let target=frame.integer(0)?;let destructor=frame.integer(1)?;let created=shared.borrow_mut().create_key(destructor);match created{
                    Ok(key)=>{if let Err(error)=frame.write(target,&key.to_le_bytes()){shared.borrow_mut().delete_key(key).map_err(|_|"TLS create rollback failed")?;return Err(error)}0},Err(errno)=>errno as u64,
                }},
                "_pthread_key_delete"=>shared.borrow_mut().delete_key(frame.integer(0)?).err().unwrap_or(0)as u64,
                "_pthread_setspecific"=>shared.borrow_mut().set(thread,frame.integer(0)?,frame.integer(1)?).err().unwrap_or(0)as u64,
                "_pthread_getspecific"=>shared.borrow().get(thread,frame.integer(0)?)?,
                _=>unreachable!(),
            };Ok(ReturnValues::integer(value))
        })?));
    }
    Ok(Services{exports,state,thread})
}
impl Services{
    /// Real guest destructor execution; callers provide a shared total budget.
    /// CF/ObjC teardown must be ordered by the thread owner, not assumed here.
    pub(super) fn exit(&mut self,bridge:&mut GuestBridge,cpu:&mut A64Cpu,max_calls:usize,per_call_ticks:u64)->Result<(),String>{
        if max_calls==0||max_calls>1024||per_call_ticks==0||per_call_ticks>1_000_000||max_calls as u64*per_call_ticks>1_000_000{return Err("TLS aggregate destructor budget invalid".into())}
        self.state.borrow_mut().begin_exit(self.thread)?;let mut count=0;
        loop{
            let call=self.state.borrow_mut().next_destructor(self.thread)?;let Some(call)=call else{return Ok(())};
            if !self.state.borrow_mut().start_destructor(&call)?{continue}
            if count>=max_calls{self.state.borrow_mut().quarantine(call)?;return Err("TLS destructor call budget exhausted".into())}count+=1;
            let result=bridge.call(cpu,&GuestCall{entry:call.function(),integers:vec![call.value()],..Default::default()},per_call_ticks);
            if let Err(error)=result{self.state.borrow_mut().quarantine(call)?;return Err(format!("TLS guest destructor failed; thread quarantined: {error}"))}
            self.state.borrow_mut().complete_destructor(call)?;
        }
    }
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn real_guest_tls_and_destructor_store(){
        let mut cpu=A64Cpu::new_sparse();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();cpu.map_zeroed(0x40000,4096,3).unwrap();cpu.map_zeroed(0x50000,4096,5).unwrap();
        // movz x1,#0x40000 (shift16); str x0,[x1,#8]; ret
        let code=[0xd2a00081u32,0xf9000420,0xd65f03c0];let bytes:Vec<u8>=code.iter().flat_map(|w|w.to_le_bytes()).collect();cpu.write_bytes(0x50000,&bytes);
        let mut services=register(&mut bridge,&mut cpu,1).unwrap();let entry=|name|services.exports.iter().find(|(n,_)|*n==name).unwrap().1.guest_address();
        let create=entry("_pthread_key_create");let set=entry("_pthread_setspecific");let get=entry("_pthread_getspecific");
        assert_eq!(bridge.call(&mut cpu,&GuestCall{entry:create,integers:vec![0x40000,0x50000],..Default::default()},100).unwrap().integers[0],0);
        let key=cpu.read_u64(0x40000).unwrap();assert_eq!(key,tls::FIRST_KEY);
        assert_eq!(bridge.call(&mut cpu,&GuestCall{entry:set,integers:vec![key,42],..Default::default()},100).unwrap().integers[0],0);
        assert_eq!(bridge.call(&mut cpu,&GuestCall{entry:get,integers:vec![key],..Default::default()},100).unwrap().integers[0],42);
        services.exit(&mut bridge,&mut cpu,8,100).unwrap();assert_eq!(cpu.read_u64(0x40008),Some(42));
        assert!(bridge.call(&mut cpu,&GuestCall{entry:get,integers:vec![key],..Default::default()},100).is_err());
    }
    #[test]fn invalid_key_output_rolls_back_allocation(){
        let mut cpu=A64Cpu::new_sparse();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();let services=register(&mut bridge,&mut cpu,1).unwrap();
        let entry=services.exports.iter().find(|(n,_)|*n=="_pthread_key_create").unwrap().1.guest_address();
        assert!(bridge.call(&mut cpu,&GuestCall{entry,integers:vec![0xdead000,0],..Default::default()},100).is_err());
        assert_eq!(services.state.borrow_mut().create_key(0),Ok(tls::FIRST_KEY));
    }
}

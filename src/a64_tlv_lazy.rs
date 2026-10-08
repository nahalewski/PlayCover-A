/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Verified original TLV slow-path key -> base ABI; original thunk adds offset.
//! Original malloc/pthread callbacks run on the selected CPU, under the real
//! process loader lock. Allocations remain owned by original malloc/destructors.
use super::{A64Cpu,bridge::{GuestBridge,GuestCall,ReturnValues},dyld_helpers::Helpers,
    loader_lock::LoaderLock,tlv_storage::TlvStorage,thread_scheduler_cpu::CpuScheduler};
use std::{rc::Rc,cell::{RefCell,Cell}};
pub(super) const ENTRY:u64=0x1a6c7f0d0;
const ORIGINAL:[u8;24]=[0xe1,3,0,0xaa,0x88,0x85,0x1d,0xb0,0,0xc5,0x41,0xf9,8,0,0x40,0xf9,2,5,0x40,0xf9,0x40,0,0x1f,0xd6];
pub(super) struct Image {pub header:u64,pub key:u64,pub template:Vec<u8>,pub alignment:u64,pub slots:Vec<(u64,u64)>}
#[derive(Default)]pub(super) struct Catalogue {pub images:Vec<Image>,pub storage:TlvStorage}
#[derive(Clone,Copy)]enum Phase {Get,Allocate,Set,Verify}
struct Pending {owner:u64,image:usize,base:u64,phase:Phase,result:Option<u64>}
/// Register the exact original-cache entry only after byte and RX validation.
pub(super) fn install(cpu:&mut A64Cpu,bridge:&mut GuestBridge,helpers:Helpers,
    catalogue:Rc<RefCell<Catalogue>>,lock:Rc<RefCell<LoaderLock>>,
    scheduler:Rc<RefCell<CpuScheduler>>)->Result<u64,String> {
    let mut original=[0;24];cpu.read_guest_into(ENTRY,&mut original)?;
    if original!=ORIGINAL||cpu.mapped_permissions(ENTRY).is_none_or(|p|p&4==0) {
        return Err("original TLV lazy entry byte/protection mismatch".into());
    }
    let pending:Rc<RefCell<Option<Pending>>>=Rc::new(RefCell::new(None));
    let target=Rc::new(Cell::new(0));let address=target.clone();
    let id=bridge.register_service(cpu,"__dyld_owned_lazy_TLV_base",move|frame| {
        let owner=scheduler.try_borrow().map_err(|_|"selected TLS scheduler is borrowed")?
            .current().ok_or("TLV lazy callback has no selected CPU owner")?.0;
        let mut current=pending.try_borrow_mut().map_err(|_|"TLV lazy state reentry")?;
        if current.is_none() {
            let key=frame.integer(0)?;
            let image=catalogue.borrow().images.iter().position(|image|image.key==key)
                .ok_or("TLV lazy key has no loader-owned template")?;
            lock.borrow_mut().enter(owner)?;
            *current=Some(Pending{owner,image,base:0,phase:Phase::Get,result:None});
        }
        let state=current.as_mut().unwrap();
        if state.owner!=owner {return Err("CPU owner changed during TLV callbacks".into());}
        lock.borrow().require_owned(owner)?;
        let catalog=catalogue.borrow();let image=&catalog.images[state.image];
        let mut call=None;
        match (state.phase,state.result.take()) {
            (Phase::Get,None)=>call=Some((8,vec![helpers.object,image.key])),
            (Phase::Get,Some(base))=>{
                if base!=0 {
                    // A previously allocated block must belong to this owner.
                    let slot=image.slots.first().ok_or("TLV image has no descriptors")?.0;
                    let access=RefCell::new(&mut *frame);
                    let address=catalog.storage.address_with_memory(owner,slot,base,
                        |a,n|access.borrow_mut().read(a,n),
                        |a,n|access.borrow_mut().validate_read_write_range(a,n as usize))?;
                    let result=address.checked_sub(image.slots[0].1).ok_or("TLV base underflow")?;
                    drop(catalog);lock.borrow_mut().leave(owner)?;*current=None;
                    return Ok(ReturnValues::integer(result));
                }
                state.phase=Phase::Allocate;call=Some((1,vec![helpers.object,image.template.len()as u64]));
            }
            (Phase::Allocate,Some(base))=>{
                if base==0||base%image.alignment!=0 {return Err("original lazy TLV malloc is null/misaligned".into());}
                // Entire mapping and live-allocation overlap checked before copy.
                frame.validate_read_write_range(base,image.template.len())?;
                catalog.storage.validate_new_block(base,image.template.len()as u64)?;
                frame.write(base,&image.template)?;state.base=base;state.phase=Phase::Set;
                call=Some((9,vec![helpers.object,image.key,base]));
            }
            (Phase::Set,Some(status))=>{
                if status as u32!=0 {return Err(format!("original lazy pthread_setspecific failed {status:#x}"));}
                state.phase=Phase::Verify;call=Some((8,vec![helpers.object,image.key]));
            }
            (Phase::Verify,Some(actual))=>{
                let (header,key,base,size,alignment,slots)=(image.header,image.key,state.base,image.template.len()as u64,image.alignment,image.slots.clone());
                drop(catalog);
                let access=RefCell::new(&mut *frame);
                catalogue.borrow_mut().storage.record_bound_with_memory(owner,header,key,base,size,alignment,&slots,actual,
                    |a,n|access.borrow_mut().read(a,n),|a,n|access.borrow_mut().validate_read_write_range(a,n as usize))?;
                lock.borrow_mut().leave(owner)?;*current=None;
                return Ok(ReturnValues::integer(base));
            }
            _=>return Err("missing/invalid genuine TLV callback continuation".into()),
        }
        let (slot,args)=call.ok_or("missing TLV continuation operation")?;
        drop(catalog);drop(current);
        let completed=pending.clone();
        frame.request_guest_call(GuestCall{entry:helpers.functions[slot],integers:args,..Default::default()},move|result| {
            let value=result?.integers[0];let mut pending=completed.borrow_mut();
            let current=pending.as_mut().ok_or("TLV callback completed without owner")?;
            if current.result.replace(value).is_some() {return Err("duplicate lazy TLV callback completion".into());}Ok(())
        })?;
        frame.request_tail_dispatch(address.get(),1)?;Ok(ReturnValues::integer(0))
    })?;
    let thunk=id.guest_address();target.set(thunk);
    let mut replacement=Vec::new();
    for i in 0..4u32 {let instruction=(if i==0{0xd2800000}else{0xf2800000})|(i<<21)|((((thunk>>(i*16))&0xffff)as u32)<<5)|16;replacement.extend_from_slice(&instruction.to_le_bytes());}
    replacement.extend_from_slice(&0xd61f0200u32.to_le_bytes());replacement.extend_from_slice(&0xd503201fu32.to_le_bytes());
    cpu.try_write_bytes(ENTRY,&replacement)?;Ok(thunk)
}
#[cfg(test)]mod tests {
    use super::*;
    fn code(cpu:&mut A64Cpu,address:u64,words:&[u32]) {for (i,word) in words.iter().enumerate(){cpu.try_write_bytes(address+i as u64*4,&word.to_le_bytes()).unwrap();}}
    #[test]fn real_selected_threads_get_distinct_templates_via_guest_callbacks() {
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();
        cpu.map_zeroed(0x40000,0x8000,3).unwrap();cpu.map_zeroed(0x90000,8192,3).unwrap();
        cpu.map_zeroed(ENTRY&!4095,4096,5).unwrap();cpu.try_write_bytes(ENTRY,&ORIGINAL).unwrap();
        // Guest fixture callbacks genuinely read/write the selected thread's
        // TPIDRRO state, rather than a host-owned map or manufactured results.
        code(&mut cpu,0x10000,&[0xd65f03c0]);
        code(&mut cpu,0x10100,&[0xd53bd069,0xf9400520,0xd65f03c0]); // malloc: per-owner fixture allocation
        code(&mut cpu,0x10200,&[0xd53bd069,0xf9000122,0x52800000,0xd65f03c0]); // set
        code(&mut cpu,0x10300,&[0xd53bd060,0xf9400000,0xd65f03c0]); // get
        cpu.write_guest_into(0x40008,&0x44000u64.to_le_bytes()).unwrap();cpu.write_guest_into(0x40108,&0x45000u64.to_le_bytes()).unwrap();
        cpu.write_guest_into(0x42008,&256u64.to_le_bytes()).unwrap();cpu.write_guest_into(0x42010,&1u64.to_le_bytes()).unwrap();
        let catalogue=Rc::new(RefCell::new(Catalogue::default()));catalogue.borrow_mut().images.push(Image{header:0x50000,key:256,template:vec![0xaa,0xbb,0],alignment:8,slots:vec![(0x42000,1)]});
        let mut scheduler=CpuScheduler::default();cpu.set_pc(0x10000);cpu.set_sp(0x91000);cpu.set_tpidrro_el0(0x40000);
        let first=scheduler.adopt(&cpu,(0x90000,0x91000)).unwrap();
        cpu.set_sp(0x92000);cpu.set_tpidrro_el0(0x40100);let second=scheduler.adopt(&cpu,(0x91000,0x92000)).unwrap();
        let scheduler=Rc::new(RefCell::new(scheduler));scheduler.borrow_mut().select(&mut cpu).unwrap();
        assert_eq!(scheduler.borrow().current(),Some(first));
        let mut functions=[0x10000;22];functions[1]=0x10100;functions[8]=0x10300;functions[9]=0x10200;
        let mut bridge=GuestBridge::map(&mut cpu,0x60000).unwrap();
        let entry=install(&mut cpu,&mut bridge,Helpers{object:0x43000,vtable:0,functions},catalogue.clone(),Rc::new(RefCell::new(LoaderLock::new())),scheduler.clone()).unwrap();
        let call=GuestCall{entry,integers:vec![256],..Default::default()};
        assert_eq!(bridge.call(&mut cpu,&call,1000).unwrap().integers[0],0x44000);
        cpu.write_guest_into(0x44000,&[9]).unwrap();
        assert_eq!(bridge.call(&mut cpu,&call,1000).unwrap().integers[0],0x44000);
        scheduler.borrow_mut().yield_current(&cpu).unwrap();scheduler.borrow_mut().set_priority(second,0x20ff).unwrap();
        assert_eq!(scheduler.borrow_mut().select(&mut cpu).unwrap(),Some(second));
        assert_eq!(bridge.call(&mut cpu,&call,1000).unwrap().integers[0],0x45000);
        assert_eq!(cpu.read_bytes(0x45000,3).unwrap(),&[0xaa,0xbb,0]);assert_eq!(cpu.read_bytes(0x44000,1).unwrap(),&[9]);
        assert!(bridge.call(&mut cpu,&GuestCall{entry,integers:vec![999],..Default::default()},100).unwrap_err().contains("no loader-owned template"));
    }
    #[test]fn wrong_original_entry_does_not_install_redirect() {
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(ENTRY&!4095,4096,5).unwrap();
        let mut bridge=GuestBridge::map(&mut cpu,0x60000).unwrap();
        assert!(install(&mut cpu,&mut bridge,Helpers{object:1,vtable:0,functions:[0;22]},Rc::new(RefCell::new(Catalogue::default())),Rc::new(RefCell::new(LoaderLock::new())),Rc::new(RefCell::new(CpuScheduler::default()))).is_err());
        assert_eq!(cpu.read_bytes(ENTRY,24).unwrap(),&[0;24]);
    }
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Audited version-six libdyld upcall descriptor; never a readiness receipt.
//! ABI reference: Apple APSL2 LibSystemHelpers.h/.cpp. Only stable prefix slots.
use super::A64Cpu;
use super::bridge::GuestBridge;
pub(super) const INITIALIZER:u64=0x1a6c7df20;
pub(super) const HELPER_OBJECT:u64=0x1e1d30380;
pub(super) const GAPIS_POINTER:u64=0x1e1d30388;
pub(super) fn install_prefix(cpu:&mut A64Cpu,bridge:&mut GuestBridge,entry:u64,helpers:Helpers,arena:u64,plans:Vec<super::tlv::Plan>,owner:u64)->Result<u64,String> {
    install_prefix_inner(cpu,bridge,entry,helpers,arena,plans,owner,None,None)
}
pub(super) fn install_owned_prefix(cpu:&mut A64Cpu,bridge:&mut GuestBridge,entry:u64,helpers:Helpers,arena:u64,plans:Vec<super::tlv::Plan>,owner:u64,scheduler:std::rc::Rc<std::cell::RefCell<super::thread_scheduler_cpu::CpuScheduler>>,lease:super::execution_session::SessionLease)->Result<u64,String> {
    install_prefix_inner(cpu,bridge,entry,helpers,arena,plans,owner,Some(scheduler),Some(lease))
}
fn install_prefix_inner(cpu:&mut A64Cpu,bridge:&mut GuestBridge,entry:u64,helpers:Helpers,arena:u64,plans:Vec<super::tlv::Plan>,owner:u64,scheduler:Option<std::rc::Rc<std::cell::RefCell<super::thread_scheduler_cpu::CpuScheduler>>>,lease:Option<super::execution_session::SessionLease>)->Result<u64,String> {
    const ORIGINAL:[u8;28]=[0x88,0x85,0x1d,0xf0,0,0xc5,0x41,0xf9,8,0,0x40,0xf9,2,9,0x40,0xf9,0x81,0x85,0x1d,0xf0,0x21,0,0x0e,0x91,0x40,0,0x1f,0xd6];
    if entry!=INITIALIZER || helpers.object!=HELPER_OBJECT || arena&4095!=0 {return Err("unsupported original dyld helper prefix identity/arena".into());}
    let mut original=[0u8;28];cpu.read_guest_into(entry,&mut original)?;
    if original!=ORIGINAL {return Err("original dyld initializer instructions mismatch".into());}
    cpu.map_zeroed(arena,4096,3)?;
    if cpu.mapped_permissions(GAPIS_POINTER).is_some_and(|p| p & 2 != 0) {
        let fallback_stub = bridge.register_service(cpu, "dyld_gapis_fallback", |_frame| {
            echo!("[a64] dyld gapis fallback stub invoked; returning 0");
            Ok(super::bridge::ReturnValues::integer(0))
        })?.guest_address();
        let vtable = arena + 256;
        for i in 0..128u64 {
            cpu.try_write_bytes(vtable + i * 8, &fallback_stub.to_le_bytes())?;
        }
        let object = arena + 1280;
        cpu.try_write_bytes(object, &vtable.to_le_bytes())?;
        cpu.try_write_bytes(GAPIS_POINTER, &object.to_le_bytes())?;
        echo!("[a64] initialized dyld gAPIs pointer at {GAPIS_POINTER:#x} -> object={object:#x} vtable={vtable:#x}");
    }
    let target=match scheduler {
        Some(scheduler)=>super::tlv_bootstrap::install_owned(cpu,bridge,helpers,arena,plans,owner,scheduler,lease.ok_or("owned bootstrap missing live session lease")?)?,
        None=>super::tlv_bootstrap::install(cpu,bridge,helpers,arena,plans,owner)?,
    };
    let mut replacement=Vec::new();
    for index in 0..4u32 {let word=(if index==0{0xd2800000}else{0xf2800000})|(index<<21)|((((target>>(index*16))&0xffff)as u32)<<5)|16;replacement.extend_from_slice(&word.to_le_bytes());}
    replacement.extend_from_slice(&0xd61f0200u32.to_le_bytes());replacement.extend_from_slice(&0xd503201fu32.to_le_bytes());replacement.extend_from_slice(&0xd503201fu32.to_le_bytes());
    cpu.try_write_bytes(entry,&replacement)?;Ok(target)
}
#[derive(Clone,Debug)]
pub(super) struct Helpers {pub object:u64,pub vtable:u64,pub functions:[u64;22]}
impl Helpers {
    pub(super) fn read(cpu:&A64Cpu,object:u64,mut cache_readable:impl FnMut(u64,usize)->bool,
        mut physical_rx:impl FnMut(u64,usize)->bool)->Result<Self,String> {
        if object==0 || object&7!=0 || !cache_readable(object,8) {return Err("libSystem helper object outside verified cache".into());}
        let mut bytes=[0u8;8];cpu.read_guest_into(object,&mut bytes)?;
        let vtable=u64::from_le_bytes(bytes);
        if vtable==0 || vtable&7!=0 || !cache_readable(vtable,176) {return Err("libSystem helper vtable outside verified cache".into());}
        let mut table=[0u8;176];cpu.read_guest_into(vtable,&mut table)?;
        let functions=std::array::from_fn(|i|u64::from_le_bytes(table[i*8..i*8+8].try_into().unwrap()));
        if functions.iter().any(|&entry|entry==0 || entry&3!=0 || !physical_rx(entry,4)) {return Err("libSystem helper prefix contains unverified executable pointer".into());}
        let mut version=[0u8;8];cpu.read_guest_into(functions[0],&mut version)?;
        // Actual original 20H392 version() is MOV W0,#6; RET. Latest fetched
        // Apple source returns7 and has a different initializer; reject it.
        if version!=[0xc0,0,0x80,0x52,0xc0,3,0x5f,0xd6] {return Err("unsupported libSystem helper version function".into());}
        Ok(Self{object,vtable,functions})
    }
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn genuine_version6_prefix_requires_verified_cache_and_executable_callbacks() {
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,1).unwrap();cpu.map_zeroed(0x20000,4096,5).unwrap();
        cpu.try_write_bytes(0x10000,&0x10100u64.to_le_bytes()).unwrap();
        for slot in 0..22 {cpu.try_write_bytes(0x10100+slot*8,&0x20000u64.to_le_bytes()).unwrap();}
        cpu.try_write_bytes(0x20000,&[0xc0,0,0x80,0x52,0xc0,3,0x5f,0xd6]).unwrap();
        let readable=|a:u64,n:usize|a>=0x10000&&a+n as u64<=0x11000;
        let rx=|a:u64,n:usize|a>=0x20000&&a+n as u64<=0x21000;
        let helpers=Helpers::read(&cpu,0x10000,readable,rx).unwrap();assert_eq!(helpers.functions[7],0x20000);
        assert!(Helpers::read(&cpu,0x10000,|_,_|false,rx).is_err());
        assert!(Helpers::read(&cpu,0x10000,readable,|_,_|false).is_err());
        cpu.try_write_bytes(0x20000,&[0xe0,0,0x80,0x52]).unwrap();
        assert!(Helpers::read(&cpu,0x10000,readable,rx).is_err());
    }
}

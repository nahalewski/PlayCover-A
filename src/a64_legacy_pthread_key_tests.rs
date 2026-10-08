/* This Source Code Form is subject to the Mozilla Public License, v. 2.0.
 * https://mozilla.org/MPL/2.0/ */
//! Opt-in execution of original15G77 callbacks, not a mocked pthread runtime.
use super::{A64Cpu,bridge::{GuestBridge,GuestCall},thread_scheduler_cpu::CpuScheduler,
    thread_storage::ThreadStorage,mach_identity::MachIdentity};
use std::{cell::RefCell,rc::Rc,io::{Read,Seek,SeekFrom},path::PathBuf};

#[test]
#[ignore = "Requires PLAYCOVER_NATIVE_CACHE pointing to original15G77 cache; executes Apple pthread callbacks"]
fn original_flat_pthread_callbacks_use_adopted_static_tsd_and_real_key_table() {
    let path=PathBuf::from(std::env::var_os("PLAYCOVER_NATIVE_CACHE").expect("explicit original15G77 cache required"));
    let mut file=std::fs::File::open(&path).unwrap();
    file.seek(SeekFrom::Start(88)).unwrap();let mut uuid=[0;16];file.read_exact(&mut uuid).unwrap();
    assert_eq!(uuid,[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f],"wrong original cache profile");
    let (_plan,mut cpu)=super::cache_map::map(&path).unwrap();
    // Original pthread lock dispatch reads NCPUS/capabilities at0xfffffc023.
    // Cache bytes alone are not a kernel context: use the same genuine
    // read-only virtual commpage as original-cache application preparation.
    super::commpage::ensure_mapped(&mut cpu).unwrap();
    let table=0x1b1a15ec0;
    assert_eq!(cpu.read_u64(table),Some(13));
    let create=cpu.read_u64(table+11*8).unwrap();
    let set=cpu.read_u64(table+12*8).unwrap();
    let get=cpu.read_u64(table+14*8).unwrap();
    assert_eq!((create,set,get),(0x180b1cc48,0x180b1c1d0,0x180b28d5c));
    for entry in [create,set,get] { assert!(cpu.mapped_permissions(entry).unwrap()&4!=0); }

    cpu.map_zeroed(0x3000000000,0x10000,3).unwrap();
    cpu.set_sp(0x300000fff0);cpu.set_pc(create);cpu.set_tpidrro_el0(0x30000000e0);
    let mut scheduler=CpuScheduler::default();
    let owner=scheduler.adopt(&cpu,(0x3000000000,0x3000010000)).unwrap();
    assert_eq!(scheduler.select(&mut cpu).unwrap(),Some(owner));
    let scheduler=Rc::new(RefCell::new(scheduler));
    let mut ports=MachIdentity::new(8).unwrap();ports.register_thread(owner.0).unwrap();
    let record=0x1b3288b40u64;let tsd=record+224;
    // These are the exact preceding original _pthread_set_self stores; the
    // real kernel adoption validator checks them against the selected owner.
    cpu.write_guest_into(record+0xd8,&owner.0.to_le_bytes()).unwrap();
    cpu.write_guest_into(tsd,&record.to_le_bytes()).unwrap();
    cpu.write_guest_into(tsd+8,&(record+0x48).to_le_bytes()).unwrap();
    cpu.write_guest_into(tsd+24,&0u64.to_le_bytes()).unwrap();
    let journal=Rc::new(ThreadStorage::new(owner,cpu.tpidrro_el0()));journal.enable_legacy();
    cpu.set_pc(0x18097f540);cpu.set_reg(0,tsd);cpu.set_reg(3,2);
    journal.adopt_original(&mut cpu,&scheduler,&ports).unwrap();
    assert!(journal.original_adopted());
    let mut bridge=GuestBridge::map_runtime(&mut cpu,0x3100000000).unwrap();
    bridge.set_thread_storage(journal);
    let output=0x3000000100;
    let invoke=|bridge:&mut GuestBridge,cpu:&mut A64Cpu,entry,integers| {
        bridge.call(cpu,&GuestCall{entry,integers,..Default::default()},100_000)
            .expect("original pthread callback must genuinely return").integers[0]
    };
    cpu.write_guest_into(output,&u64::MAX.to_le_bytes()).unwrap();
    assert_eq!(invoke(&mut bridge,&mut cpu,create,vec![output,0]),0);
    let key=cpu.read_u64(output).unwrap();assert!((256..512).contains(&key));
    assert_eq!(cpu.read_u64(0x1b3289c98+key*8),Some(u64::MAX));
    assert_eq!(invoke(&mut bridge,&mut cpu,get,vec![key]),0);
    let value=0x3000000200;
    assert_eq!(invoke(&mut bridge,&mut cpu,set,vec![key,value]),0);
    assert_eq!(invoke(&mut bridge,&mut cpu,get,vec![key]),value);
    assert_eq!(cpu.read_u64(tsd+key*8),Some(value));
    assert_eq!(cpu.tpidrro_el0(),tsd);
    assert_eq!(scheduler.borrow().current(),Some(owner));
    // Exhaust real original keycreate through its callbacks, not host-seeded
    // destructor markers. No game initialization/readiness is implied.
    let mut allocated=vec![key];
    for _ in 0..256 {
        cpu.write_guest_into(output,&u64::MAX.to_le_bytes()).unwrap();
        let status=invoke(&mut bridge,&mut cpu,create,vec![output,0]);
        if status==35 {
            assert_eq!(cpu.read_u64(output),Some(u64::MAX));
            assert_eq!(invoke(&mut bridge,&mut cpu,get,vec![key]),value);
            println!("PLAYCOVER_LEGACY_KEYS: original callbacks passed; {} keys allocated; no startup receipt",allocated.len());
            return;
        }
        assert_eq!(status,0);let next=cpu.read_u64(output).unwrap();
        assert!((256..512).contains(&next));assert!(!allocated.contains(&next));allocated.push(next);
    }
    panic!("original key table exceeded its evidenced256 dynamic slots");
}

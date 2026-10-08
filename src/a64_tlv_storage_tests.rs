/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Real guest-memory ownership tests; callback closures are test fixtures only.
use super::{A64Cpu,tlv_storage::TlvStorage};
fn fixture()->A64Cpu {
    let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,3).unwrap();
    cpu.map_zeroed_with_max(0x20000,4096,3,7).unwrap();
    cpu.write_guest_into(0x10008,&256u64.to_le_bytes()).unwrap();
    cpu.write_guest_into(0x10010,&8u64.to_le_bytes()).unwrap();cpu
}
#[test]
fn owner_and_descriptor_control_address() {
    let cpu=fixture();let mut storage=TlvStorage::new();
    storage.record_bound(&cpu,1,0x30000,256,0x20000,32,16,&[(0x10000,8)],|key|{assert_eq!(key,256);Ok(0x20000)}).unwrap();
    assert_eq!(storage.address(&cpu,1,0x10000,|_|Ok(0x20000)).unwrap(),0x20008);
    assert!(storage.address(&cpu,2,0x10000,|_|panic!("foreign owner must not call guest")).is_err());
    assert!(storage.address(&cpu,1,0x10018,|_|panic!("foreign descriptor must not call guest")).is_err());
    assert!(storage.address(&cpu,1,0x10000,|_|Ok(0)).is_err());
}
#[test]
fn failed_readback_and_shared_block_do_not_bind() {
    let cpu=fixture();let mut storage=TlvStorage::new();
    assert!(storage.record_bound(&cpu,1,0x30000,256,0x20000,32,16,&[(0x10000,8)],|_|Ok(0)).is_err());
    storage.record_bound(&cpu,1,0x30000,256,0x20000,32,16,&[(0x10000,8)],|_|Ok(0x20000)).unwrap();
    assert!(storage.record_bound(&cpu,2,0x30000,256,0x20000,32,16,&[(0x10000,8)],|_|panic!("shared memory rejected before callback")).is_err());
}
#[test]
fn permissions_alignment_and_changed_descriptor_rejected() {
    let mut cpu=fixture();let mut storage=TlvStorage::new();
    assert!(storage.record_bound(&cpu,1,0x30000,256,0x20001,32,16,&[(0x10000,8)],|_|panic!()).is_err());
    assert!(storage.record_bound(&cpu,1,0x30000,256,0x20000,8,16,&[(0x10000,8)],|_|panic!()).is_err());
    storage.record_bound(&cpu,1,0x30000,256,0x20000,32,16,&[(0x10000,8)],|_|Ok(0x20000)).unwrap();
    cpu.set_protection(0x20000,4096,0).unwrap();
    assert!(storage.address(&cpu,1,0x10000,|_|panic!("revoked mapping rejected first")).is_err());
    cpu.set_protection(0x20000,4096,3).unwrap();cpu.write_guest_into(0x10008,&257u64.to_le_bytes()).unwrap();
    assert!(storage.address(&cpu,1,0x10000,|_|panic!("changed descriptor rejected first")).is_err());
}
#[test]
fn frame_reader_checks_actual_lookup_and_complete_descriptor() {
    let cpu=fixture();let mut storage=TlvStorage::new();
    let read=|address,length| {let mut bytes=vec![0;length];cpu.read_guest_into(address,&mut bytes)?;Ok(bytes)};
    let rw=|address,length:u64|cpu.validate_guest_write(address,length as usize);
    assert!(storage.record_bound_with_memory(1,0x30000,256,0x20000,32,16,&[(0x10000,8)],0,read,rw).is_err());
    assert!(storage.record_bound_with_memory(1,0x30000,256,0x20000,32,16,&[(0x10000,8)],0x20000,|_,_|Ok(vec![0;16]),rw).is_err());
    storage.record_bound_with_memory(1,0x30000,256,0x20000,32,16,&[(0x10000,8)],0x20000,read,rw).unwrap();
    assert_eq!(storage.address(&cpu,1,0x10000,|_|Ok(0x20000)).unwrap(),0x20008);
}

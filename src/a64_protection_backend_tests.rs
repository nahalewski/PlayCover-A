/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Real interpreter/backend protection tests included in the main lib target.
use super::{A64Cpu,A64State};
#[test]
fn protection_clips_backing_preserves_maximum_and_faults_actual_store() {
    let mut cpu=A64Cpu::new_sparse();
    cpu.map_zeroed(0x10000,4096,5).unwrap();
    cpu.map_zeroed_with_max(0x20000,0xc000,3,7).unwrap();
    let mut code=0xf9000020u32.to_le_bytes().to_vec();code.extend_from_slice(&0xd4001001u32.to_le_bytes());
    cpu.write_bytes(0x10000,&code);
    cpu.write_guest_into(0x24000,&7u64.to_le_bytes()).unwrap();
    cpu.set_protection(0x24000,0x4000,1).unwrap();
    assert_eq!(cpu.protection_region(0x24000).unwrap().max_protection,7);
    assert_eq!(cpu.mapped_permissions(0x20000),Some(3));assert_eq!(cpu.mapped_permissions(0x28000),Some(3));
    cpu.set_reg(0,9);cpu.set_reg(1,0x24000);cpu.set_pc(0x10000);let mut ticks=100;
    assert_eq!(cpu.run_or_step(Some(&mut ticks)),A64State::MemoryError(0x24000));
    assert_eq!(cpu.read_u64(0x24000),Some(7));assert!(cpu.write_guest_into(0x24000,&[9]).is_err());
    cpu.set_protection(0x24000,0x4000,3).unwrap();cpu.set_pc(0x10000);let mut ticks=100;
    assert_eq!(cpu.run_or_step(Some(&mut ticks)),A64State::Svc(0x80));assert_eq!(cpu.read_u64(0x24000),Some(9));
}
#[test]
fn protection_preflight_holes_overflow_and_maximum_are_atomic() {
    let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x20000,0x4000,3).unwrap();cpu.map_zeroed(0x28000,0x4000,1).unwrap();
    assert!(cpu.set_protection(0x20000,0xc000,1).is_err());assert_eq!(cpu.mapped_permissions(0x20000),Some(3));
    assert!(cpu.set_protection(u64::MAX-1,4,1).is_err());assert!(cpu.set_protection(0x28000,0x4000,3).is_err());
    assert_eq!(cpu.mapped_permissions(0x28000),Some(1));
    cpu.set_protection(0x20000,0x4000,0).unwrap();let mut out=[77];
    assert!(cpu.read_guest_into(0x20000,&mut out).is_err());assert_eq!(out,[77]);assert!(cpu.validate_guest_write(0x20000,1).is_err());
    cpu.set_protection(0x20000,0x4000,3).unwrap();cpu.write_guest_into(0x20000,&[5]).unwrap();
}
#[test]
fn execute_permission_change_invalidates_already_translated_code() {
    let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();
    cpu.write_bytes(0x10000,&0xd4001001u32.to_le_bytes());cpu.set_pc(0x10000);let mut ticks=100;
    assert_eq!(cpu.run_or_step(Some(&mut ticks)),A64State::Svc(0x80));
    cpu.set_protection(0x10000,4096,1).unwrap();cpu.set_pc(0x10000);let mut ticks=100;
    assert_eq!(cpu.run_or_step(Some(&mut ticks)),A64State::MemoryError(0x10000));
    cpu.set_protection(0x10000,4096,5).unwrap();cpu.set_pc(0x10000);let mut ticks=100;
    assert_eq!(cpu.run_or_step(Some(&mut ticks)),A64State::Svc(0x80));
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Planner tests use real mapped ARM64 Mach-O headers and section bytes.
//! A successfully parsed plan is not an executed TLS initializer receipt.
use super::{A64Cpu,tlv};
const HEADER:u64=0x10000;
fn put32(bytes:&mut[u8],offset:usize,value:u32) {bytes[offset..offset+4].copy_from_slice(&value.to_le_bytes());}
fn put64(bytes:&mut[u8],offset:usize,value:u64) {bytes[offset..offset+8].copy_from_slice(&value.to_le_bytes());}
fn section(index:u64)->u64 {HEADER+32+72+index*80}
fn word(cpu:&mut A64Cpu,address:u64,value:u32) {cpu.write_guest_into(address,&value.to_le_bytes()).unwrap();}
fn wide(cpu:&mut A64Cpu,address:u64,value:u64) {cpu.write_guest_into(address,&value.to_le_bytes()).unwrap();}
fn fixture()->A64Cpu {
    let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(HEADER,0x8000,3).unwrap();cpu.map_zeroed(0x20000,4096,5).unwrap();
    cpu.write_bytes(0x20000,&0xd65f03c0u32.to_le_bytes());
    let mut header=[0;32];put32(&mut header,0,0xfeedfacf);put32(&mut header,4,0x100000c);put32(&mut header,12,6);
    put32(&mut header,16,1);put32(&mut header,20,392);cpu.write_guest_into(HEADER,&header).unwrap();
    let mut command=vec![0;392];put32(&mut command,0,0x19);put32(&mut command,4,392);
    put64(&mut command,24,HEADER);put64(&mut command,32,0x14000);put64(&mut command,48,0x8000);put32(&mut command,56,7);put32(&mut command,60,7);put32(&mut command,64,4);
    for (index,(address,size,kind,align)) in [(0x14000,16,0x11,4),(0x14010,16,0x12,4),(0x15000,24,0x13,3),(0x16000,8,0x15,3)].into_iter().enumerate() {
        let offset=72+index*80;put64(&mut command,offset+32,address);put64(&mut command,offset+40,size);if kind!=0x12 {put32(&mut command,offset+48,(address-HEADER) as u32);}put32(&mut command,offset+52,align);put32(&mut command,offset+64,kind);
    }
    cpu.write_guest_into(HEADER+32,&command).unwrap();cpu.write_guest_into(0x14000,&[7;16]).unwrap();
    wide(&mut cpu,0x15000,0x20000);wide(&mut cpu,0x15010,8);wide(&mut cpu,0x16000,0x20000);cpu
}
#[test]
fn mapped_regular_and_zero_fill_sections_form_exact_aligned_template() {
    let cpu=fixture();let plan=tlv::read(&cpu,HEADER,0).unwrap().unwrap();
    assert_eq!(plan.header,HEADER);assert_eq!(plan.alignment,16);assert_eq!(plan.template,[vec![7;16],vec![0;16]].concat());
    assert_eq!(plan.descriptors.len(),1);assert_eq!(plan.descriptors[0].slot,0x15000);assert_eq!(plan.descriptors[0].offset,8);
    assert_eq!(plan.initializers,[0x20000]);
    let mut key=[0;8];cpu.read_guest_into(0x15008,&mut key).unwrap();assert_eq!(key,[0;8]);
}
#[test]
fn malformed_macho_commands_and_foreign_sections_fail_before_plan_publication() {
    let mut cpu=fixture();word(&mut cpu,HEADER,0);assert!(tlv::read(&cpu,HEADER,0).is_err());
    let mut cpu=fixture();word(&mut cpu,HEADER+32+4,7);assert!(tlv::read(&cpu,HEADER,0).is_err());
    let mut cpu=fixture();word(&mut cpu,HEADER+20,72);assert!(tlv::read(&cpu,HEADER,0).is_err());
    let mut cpu=fixture();wide(&mut cpu,section(2)+32,0x30000);assert!(tlv::read(&cpu,HEADER,0).is_err());
    let cpu=fixture();assert!(tlv::read(&cpu,HEADER,u64::MAX).is_err());
}
#[test]
fn overlapping_templates_and_unbounded_alignment_are_rejected() {
    let mut cpu=fixture();wide(&mut cpu,section(1)+32,0x14008);assert!(tlv::read(&cpu,HEADER,0).is_err());
    let mut cpu=fixture();word(&mut cpu,section(0)+52,21);assert!(tlv::read(&cpu,HEADER,0).is_err());
}
#[test]
fn preexisting_key_foreign_offset_and_readonly_descriptor_are_rejected() {
    let mut cpu=fixture();wide(&mut cpu,0x15008,9);assert!(tlv::read(&cpu,HEADER,0).is_err());assert_eq!(cpu.read_u64(0x15008),Some(9));
    let mut cpu=fixture();wide(&mut cpu,0x15010,32);assert!(tlv::read(&cpu,HEADER,0).is_err());
    let mut cpu=fixture();cpu.set_protection(0x15000,4096,1).unwrap();assert!(tlv::read(&cpu,HEADER,0).is_err());
    assert_eq!(cpu.read_u64(0x15008),Some(0));
}
#[test]
fn initializer_must_be_nonzero_aligned_and_genuinely_executable() {
    for address in [0,0x20001,0x14000,0x30000] {
        let mut cpu=fixture();wide(&mut cpu,0x16000,address);assert!(tlv::read(&cpu,HEADER,0).is_err());
    }
    let mut cpu=fixture();cpu.set_protection(0x20000,4096,1).unwrap();assert!(tlv::read(&cpu,HEADER,0).is_err());
}

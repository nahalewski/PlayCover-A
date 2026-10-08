/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Exact loaded-header lookup, never synthetic dyld-wide initialization.
use std::collections::BTreeMap;
use super::{A64Cpu, bridge::{GuestBridge, ReturnValues}};
pub(super) const IMMUTABLE_ENTRY:u64=0x1a6c7d844;
pub(super) struct ImmutableRanges { ranges:Vec<(u64,u64,u32,bool)> }
#[cfg(test)] mod immutable_tests {
    use super::*;
    #[test] fn maximum_write_and_permanent_provenance_define_immutability() {
        let ranges=ImmutableRanges::new(vec![(0x1000,0x2000,5,true),(0x2000,0x3000,3,true),(0x4000,0x5000,1,false)]).unwrap();
        assert!(ranges.query(0x1100,8));assert!(!ranges.query(0x2100,8));
        assert!(ranges.query(0x4000,0x1000));assert!(!ranges.query(0x6000,8));
        assert!(!ranges.query(u64::MAX,4));assert!(!ranges.query(0x1ffc,8));
    }
    #[test] fn cached_region_edges_follow_primary_strict_containment() {
        let ranges=ImmutableRanges::new(vec![(0x1000,0x2000,5,true)]).unwrap();
        assert!(!ranges.query(0x1000,1));assert!(!ranges.query(0x1fff,1));assert!(ranges.query(0x1001,1));
        assert!(ImmutableRanges::new(vec![(0x2000,0x1000,5,true)]).is_err());
    }
}
impl ImmutableRanges {
    pub(super) fn new(ranges:Vec<(u64,u64,u32,bool)>)->Result<Self,String> {
        if ranges.len()>16384 || ranges.iter().any(|&(a,b,p,_)|a==0||a>=b||p&!7!=0) {
            return Err("invalid permanent-image maximum-protection ledger".into());
        }
        Ok(Self{ranges})
    }
    pub(super) fn query(&self,address:u64,len:u64)->bool {
        let Some(end)=address.checked_add(len) else{return false;};
        self.ranges.iter().any(|&(start,limit,max,cache)| max&2==0 &&
            if cache {address>start && end<limit} else {address>=start && end<=limit})
    }
}
pub(super) fn install_immutable(cpu:&mut A64Cpu,bridge:&mut GuestBridge,entry:u64,ranges:ImmutableRanges)->Result<u64,String> {
    const ORIGINAL:[u8;28]=[0xe2,3,1,0xaa,0xe1,3,0,0xaa,0x88,0x85,0x1d,0xf0,0,0xc5,0x41,0xf9,8,0,0x40,0xf9,3,5,0x41,0xf9,0x60,0,0x1f,0xd6];
    if entry!=IMMUTABLE_ENTRY || cpu.mapped_permissions(entry).is_none_or(|p|p&4==0) {return Err("immutable-query export does not match audited libdyld entry".into());}
    let mut original=[0u8;28];cpu.read_guest_into(entry,&mut original)?;
    if original!=ORIGINAL {return Err("audited immutable-query bytes mismatch".into());}
    let service=bridge.register_service(cpu,"__dyld_is_memory_immutable",move |frame|Ok(ReturnValues::integer(u64::from(ranges.query(frame.integer(0)?,frame.integer(1)?)))))?;
    let target=service.guest_address();let mut replacement=Vec::new();
    for index in 0..4u32 {let word=(if index==0{0xd2800000}else{0xf2800000})|(index<<21)|((((target>>(index*16))&0xffff)as u32)<<5)|16;replacement.extend_from_slice(&word.to_le_bytes());}
    replacement.extend_from_slice(&0xd61f0200u32.to_le_bytes());
    replacement.extend_from_slice(&0xd503201fu32.to_le_bytes());replacement.extend_from_slice(&0xd503201fu32.to_le_bytes());
    cpu.try_write_bytes(entry,&replacement)?;Ok(target)
}
pub(super) const AUDITED_ENTRY: u64 = 0x1a6c7dd38;
pub(super) const RESTRICTED_ENTRY:u64=0x1a6c7e70c;
/// The declared-path loader accepts no DYLD environment search override.
/// This policy describes that actual restriction; it grants no sandbox rights.
pub(super) struct LoaderPolicy;
impl LoaderPolicy {
    pub(super) fn declared_paths_only()->Self {Self}
    pub(super) fn is_restricted(&self)->bool {true}
    pub(super) fn environment_path_override(&self,_path:&str)->Result<(),String> {
        Err("DYLD environment path overrides denied by declared-path loader policy".into())
    }
}
pub(super) fn install_restricted(cpu:&mut A64Cpu,bridge:&mut GuestBridge,entry:u64,policy:LoaderPolicy)->Result<u64,String> {
    const ORIGINAL:[u8;20]=[0x88,0x85,0x1d,0xd0,0,0xc5,0x41,0xf9,8,0,0x40,0xf9,1,0xd1,0x40,0xf9,0x20,0,0x1f,0xd6];
    if entry!=RESTRICTED_ENTRY || cpu.mapped_permissions(entry).is_none_or(|p|p&4==0) {
        return Err("restricted-query export does not match audited libdyld entry".into());
    }
    let mut bytes=[0u8;20];cpu.read_guest_into(entry,&mut bytes)?;
    if bytes!=ORIGINAL {return Err("audited restricted-query bytes mismatch".into());}
    // Enforce the same denial before making the policy visible to guest code.
    if policy.environment_path_override("/").is_ok() {return Err("loader policy permits unexpected environment overrides".into());}
    let service=bridge.register_service(cpu,"_dyld_process_is_restricted",move |_|Ok(ReturnValues::integer(u64::from(policy.is_restricted()))))?;
    let target=service.guest_address();
    let mut replacement=Vec::new();
    for index in 0..4u32 {
        let instruction=(if index==0 {0xd2800000}else{0xf2800000})|(index<<21)|((((target>>(index*16))&0xffff)as u32)<<5)|16;
        replacement.extend_from_slice(&instruction.to_le_bytes());
    }
    replacement.extend_from_slice(&0xd61f0200u32.to_le_bytes());
    cpu.try_write_bytes(entry,&replacement)?;
    Ok(target)
}
const ORIGINAL: [u8;24] = [0xe1,3,0,0xaa,0x88,0x85,0x1d,0xf0,0,0xc5,0x41,0xf9,8,0,0x40,0xf9,2,0xb5,0x40,0xf9,0x40,0,0x1f,0xd6];
pub(super) fn install(cpu:&mut A64Cpu, bridge:&mut GuestBridge, entry:u64, ledger:ImageSlides)->Result<u64,String> {
    if entry != AUDITED_ENTRY || cpu.mapped_permissions(entry).is_none_or(|p| p & 4 == 0) {
        return Err("dyld slide export does not match audited original provider entry".into());
    }
    let mut original=[0u8;24]; cpu.read_guest_into(entry,&mut original)?;
    if original != ORIGINAL {return Err("audited dyld slide function bytes mismatch".into());}
    let service=bridge.register_service(cpu,"_dyld_get_image_slide",move |frame| {
        Ok(ReturnValues::integer(ledger.lookup(frame.integer(0)?)? as u64))
    })?;
    let target=service.guest_address();
    let mut instructions=Vec::new();
    for index in 0..4u32 {
        let immediate=((target>>(index*16))&0xffff) as u32;
        let instruction=(if index==0 {0xd2800000} else {0xf2800000}) | (index<<21) | (immediate<<5) | 16;
        instructions.extend_from_slice(&instruction.to_le_bytes());
    }
    instructions.extend_from_slice(&0xd61f0200u32.to_le_bytes()); // BR x16, preserves LR/header
    instructions.extend_from_slice(&0xd503201fu32.to_le_bytes());
    cpu.try_write_bytes(entry,&instructions)?;
    Ok(target)
}
pub(super) struct ImageSlides { images: BTreeMap<u64, i64> }
impl ImageSlides {
    pub(super) fn new(entries: &[(u64, i64)]) -> Result<Self, String> {
        if entries.is_empty() || entries.len() > 8192 {
            return Err("invalid loaded-image slide ledger size".into());
        }
        let mut images = BTreeMap::new();
        for &(header, slide) in entries {
            if header == 0 || header & 3 != 0 {
                return Err("invalid loaded Mach-O header identity".into());
            }
            if images.insert(header, slide).is_some_and(|old| old != slide) {
                return Err("conflicting loaded-image slide identity".into());
            }
        }
        Ok(Self { images })
    }
    pub(super) fn lookup(&self, header: u64) -> Result<i64, String> {
        self.images.get(&header).copied()
            .ok_or_else(|| format!("unsupported dyld slide lookup for unregistered header {header:#x}"))
    }
}

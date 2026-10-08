/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Borrowed, process-owned TLV blocks. This ledger neither allocates memory nor
//! creates pthread keys; callback execution and block lifetime remain with dyld.
use super::A64Cpu;
#[derive(Debug)]
struct Binding { owner:u64, header:u64, key:u64, base:u64, size:u64, slots:Vec<(u64,u64)> }
#[derive(Default)]
pub(super) struct TlvStorage { bindings:Vec<Binding> }
fn mapped_rw(cpu:&A64Cpu,base:u64,size:u64)->Result<(),String> {
    let end=base.checked_add(size).ok_or("TLV block overflow")?;
    if size==0 {return Err("empty TLV block".into());}
    let mut cursor=base;
    while cursor<end {
        let region=cpu.protection_region(cursor).ok_or("unmapped TLV block")?;
        if region.protection&3!=3 {return Err("TLV block is not readable/writable".into());}
        cursor=region.base.checked_add(region.len).ok_or("TLV mapping overflow")?.min(end);
        if cursor<=base {return Err("invalid TLV mapping extent".into());}
    }
    Ok(())
}
#[cfg(test)]mod completion_tests {
 use super::*;
 #[test]fn completion_cannot_adopt_foreign_or_changed_storage(){
  let storage=TlvStorage{bindings:vec![Binding{owner:1,header:0x1000,key:256,base:0x4000,size:8,slots:vec![(0x2000,0)]}]};
  assert!(storage.validate_catalogued(1,0x1000,256,8,&[(0x2000,0)]).is_ok());
  for (owner,header,key,size,slots) in [
   (2,0x1000,256,8,vec![(0x2000,0)]),
   (1,0x1001,256,8,vec![(0x2000,0)]),
   (1,0x1000,257,8,vec![(0x2000,0)]),
   (1,0x1000,256,9,vec![(0x2000,0)]),
   (1,0x1000,256,8,vec![(0x2000,1)])]{
   assert!(storage.validate_catalogued(owner,header,key,size,&slots).is_err());
  }
 }
}
impl TlvStorage {
    /// Check the retained ownership ledger after real callback publication.
    pub(super) fn validate_catalogued(&self,owner:u64,header:u64,key:u64,size:u64,slots:&[(u64,u64)])->Result<(),String> {
        let binding=self.bindings.iter().find(|b|b.owner==owner&&b.header==header&&b.key==key)
            .ok_or("TLV completion has no retained callback-backed storage")?;
        if binding.size!=size||binding.slots!=slots {return Err("TLV completion storage catalogue differs".into());}
        Ok(())
    }
    pub(super) fn new()->Self {Self::default()}
    pub(super) fn validate_new_block(&self,base:u64,size:u64)->Result<(),String> {
        let end=base.checked_add(size).ok_or("TLV block overflow")?;
        if size==0||self.bindings.iter().any(|b|base<b.base+b.size&&b.base<end) {
            return Err("empty/shared live TLV block".into());}Ok(())
    }
    /// Record only after the original guest pthread callback reads this block.
    /// The caller keeps its real allocation alive; no initializer receipt exists.
    pub(super) fn record_bound<F>(&mut self,cpu:&A64Cpu,owner:u64,header:u64,key:u64,
        base:u64,size:u64,alignment:u64,slots:&[(u64,u64)],mut get_specific:F)->Result<(),String>
        where F:FnMut(u64)->Result<u64,String> {
        self.record_with_access(owner,header,key,base,size,alignment,slots,||get_specific(key),
            |address,length| {let mut bytes=vec![0;length];cpu.read_guest_into(address,&mut bytes)?;Ok(bytes)},
            |address,length|mapped_rw(cpu,address,length))
    }
    /// ServiceFrame variant: actual_lookup must be the preceding original
    /// getspecific callback result; it is checked rather than manufactured here.
    pub(super) fn record_bound_with_memory<R,W>(&mut self,owner:u64,header:u64,key:u64,
        base:u64,size:u64,alignment:u64,slots:&[(u64,u64)],actual_lookup:u64,read:R,validate_rw:W)->Result<(),String>
        where R:FnMut(u64,usize)->Result<Vec<u8>,String>,W:FnMut(u64,u64)->Result<(),String> {
        self.record_with_access(owner,header,key,base,size,alignment,slots,||Ok(actual_lookup),read,validate_rw)
    }
    fn record_with_access<R,W,G>(&mut self,owner:u64,header:u64,key:u64,base:u64,size:u64,
        alignment:u64,slots:&[(u64,u64)],mut lookup:G,mut read:R,mut validate_rw:W)->Result<(),String>
        where R:FnMut(u64,usize)->Result<Vec<u8>,String>,W:FnMut(u64,u64)->Result<(),String>,G:FnMut()->Result<u64,String> {
        if owner==0||header==0||key==0||alignment==0||!alignment.is_power_of_two()
            ||alignment>1<<20||base%alignment!=0 {return Err("invalid TLV binding identity/alignment".into());}
        if self.bindings.len()>=4096||slots.is_empty()||slots.len()>4096 {return Err("TLV binding budget exceeded".into());}
        if size==0||base.checked_add(size).is_none() {return Err("invalid TLV block size".into());}
        validate_rw(base,size)?;
        if self.bindings.iter().any(|b| b.owner==owner&&(b.key==key||b.header==header)
            || base<b.base+b.size&&b.base<base+size) {return Err("duplicate or shared TLV block".into());}
        for (i,&(slot,offset)) in slots.iter().enumerate() {
            if offset>=size||slots[..i].iter().any(|s|s.0==slot) {return Err("invalid TLV descriptor ownership".into());}
            let bytes=read(slot,24)?;
            if bytes.len()!=24 {return Err("truncated TLV descriptor read".into());}
            if u64::from_le_bytes(bytes[8..16].try_into().unwrap())!=key
                ||u64::from_le_bytes(bytes[16..24].try_into().unwrap())!=offset {return Err("TLV descriptor differs from bound image".into());}
        }
        let owned_slots=slots.to_vec();
        self.bindings.try_reserve(1).map_err(|_|"TLV binding allocation failed")?;
        if lookup()?!=base {return Err("original pthread callback did not bind TLV block".into());}
        self.bindings.push(Binding{owner,header,key,base,size,slots:owned_slots});Ok(())
    }
    pub(super) fn address<F>(&self,cpu:&A64Cpu,owner:u64,slot:u64,mut get_specific:F)->Result<u64,String>
        where F:FnMut(u64)->Result<u64,String> {
        let binding=self.bindings.iter().find(|b|b.owner==owner&&b.slots.iter().any(|s|s.0==slot))
            .ok_or("foreign/unbound thread TLV descriptor")?;
        // Ownership/memory is checked before invoking any guest callback.
        let address=self.address_with_memory(owner,slot,binding.base,
            |address,length| {let mut bytes=vec![0;length];cpu.read_guest_into(address,&mut bytes)?;Ok(bytes)},
            |address,length|mapped_rw(cpu,address,length))?;
        if get_specific(binding.key)?!=binding.base {return Err("pthread TLS binding changed".into());}Ok(address)
    }
    pub(super) fn address_with_memory<R,W>(&self,owner:u64,slot:u64,actual_lookup:u64,
        mut read:R,mut validate_rw:W)->Result<u64,String>
        where R:FnMut(u64,usize)->Result<Vec<u8>,String>,W:FnMut(u64,u64)->Result<(),String> {
        let binding=self.bindings.iter().find(|b|b.owner==owner&&b.slots.iter().any(|s|s.0==slot))
            .ok_or("foreign/unbound thread TLV descriptor")?;
        let expected=binding.slots.iter().find(|s|s.0==slot).unwrap().1;
        let bytes=read(slot,24)?;if bytes.len()!=24 {return Err("truncated TLV descriptor read".into());}
        if u64::from_le_bytes(bytes[8..16].try_into().unwrap())!=binding.key
            ||u64::from_le_bytes(bytes[16..24].try_into().unwrap())!=expected {return Err("TLV descriptor changed after binding".into());}
        validate_rw(binding.base,binding.size)?;
        if actual_lookup!=binding.base {return Err("pthread TLS binding changed".into());}
        binding.base.checked_add(expected).ok_or("TLV address overflow".into())
    }
}

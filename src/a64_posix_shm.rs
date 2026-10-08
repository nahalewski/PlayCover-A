/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Read-only lookup in an explicitly isolated virtual-kernel POSIX shm namespace.
//! No external Apple daemon is running in this namespace and no host shm names
//! are imported. Creation, mappings and feature-flag contents are unsupported.
use super::A64Cpu;
use std::collections::BTreeSet;
pub(super) struct ShmNamespace { names:BTreeSet<Vec<u8>> }
impl ShmNamespace {
    pub(super) fn new_isolated()->Self {Self {names:BTreeSet::new()}}
    pub(super) fn open(&self,cpu:&mut A64Cpu,args:[u64;3])->Result<(),String> {
        let [address,flags,mode]=args;
        if flags!=0 || mode!=0 {return Err("unsupported virtual POSIX shm create/write/mode operation".into());}
        let mut name=Vec::new();let mut terminated=false;
        // Darwin PSHMNAMLEN bounds include the terminating byte.
        for offset in 0..32u64 {
            let Some(ptr)=address.checked_add(offset) else {finish(cpu,14);return Ok(());};
            let mut byte=[0];if cpu.read_guest_into(ptr,&mut byte).is_err() {finish(cpu,14);return Ok(());}
            if byte[0]==0 {terminated=true;break;}name.push(byte[0]);
        }
        if !terminated {finish(cpu,63);return Ok(());}
        if name.is_empty() {finish(cpu,22);return Ok(());}
        if self.names.contains(&name) {return Err("existing virtual POSIX shm object has no descriptor/mapping service".into());}
        // Real absent-name result in this explicitly owned namespace; this is
        // not a declaration that an Apple host daemon or its shm object exists.
        finish(cpu,2);Ok(())
    }
}
fn finish(cpu:&mut A64Cpu,errno:u64) {cpu.set_reg(0,errno);cpu.set_pstate(cpu.pstate()|(1<<29));}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolated_namespace_lookup_is_absent_without_fabricating_a_file() {
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,3).unwrap();
        cpu.write_guest_into(0x10000,b"com.apple.featureflags.shm\0").unwrap();cpu.set_pstate(0x90000000);
        let namespace=ShmNamespace::new_isolated();namespace.open(&mut cpu,[0x10000,0,0]).unwrap();
        assert_eq!(cpu.reg(0),2);assert_eq!(cpu.pstate()&0xf0000000,0xb0000000);assert!(namespace.names.is_empty());
        assert!(namespace.open(&mut cpu,[0x10000,0x200,0]).is_err());assert!(namespace.names.is_empty());
        cpu.write_guest_into(0x10000,b"\0").unwrap();namespace.open(&mut cpu,[0x10000,0,0]).unwrap();assert_eq!(cpu.reg(0),22);
    }
    #[test]
    fn known_object_is_not_misreported_absent_and_bad_names_are_bounded() {
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,3).unwrap();
        cpu.write_guest_into(0x10000,b"test\0").unwrap();let mut namespace=ShmNamespace::new_isolated();namespace.names.insert(b"test".to_vec());
        assert!(namespace.open(&mut cpu,[0x10000,0,0]).is_err());
        cpu.write_guest_into(0x10000,&[b'x';32]).unwrap();namespace.open(&mut cpu,[0x10000,0,0]).unwrap();assert_eq!(cpu.reg(0),63);
        namespace.open(&mut cpu,[u64::MAX,0,0]).unwrap();assert_eq!(cpu.reg(0),14);
    }
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Process-owned guest descriptors for the evidenced readonly entropy device.
//! No guest path is passed through to the host filesystem.
use super::A64Cpu;
use std::{collections::BTreeMap,fs::File,io::Read};
pub(super) struct EntropyFds {files:BTreeMap<u32,File>,next:u32}
impl EntropyFds {
    pub(super) fn new()->Self {Self {files:BTreeMap::new(),next:3}}
    /// BSD500 exact-fill kernel entropy operation; no guest FD is allocated.
    pub(super) fn getentropy(&self,cpu:&mut A64Cpu,args:[u64;2])->Result<(),String> {
        let [address,count]=args;
        // XNU randomdev.c returns EINVAL above its 256-byte stack buffer.
        if count>256 {finish(cpu,22,true);return Ok(());}
        if count==0 {finish(cpu,0,false);return Ok(());}
        if cpu.validate_guest_write(address,count as usize).is_err() {finish(cpu,14,true);return Ok(());}
        let mut source=match File::open("/dev/urandom") {
            Ok(source)=>source,Err(error)=>{finish(cpu,io_errno(&error),true);return Ok(());}
        };
        let mut bytes=[0;256];
        if let Err(error)=source.read_exact(&mut bytes[..count as usize]) {finish(cpu,io_errno(&error),true);return Ok(());}
        cpu.write_guest_into(address,&bytes[..count as usize])?;
        finish(cpu,0,false);Ok(())
    }
    pub(super) fn open(&mut self,cpu:&mut A64Cpu,args:[u64;3])->Result<(),String> {
        let [address,flags,mode]=args;
        let mut path=Vec::new();let mut terminated=false;
        for offset in 0..256u64 {
            let Some(ptr)=address.checked_add(offset) else {finish(cpu,14,true);return Ok(());};
            let mut byte=[0];if cpu.read_guest_into(ptr,&mut byte).is_err() {finish(cpu,14,true);return Ok(());}
            if byte[0]==0 {terminated=true;break;}path.push(byte[0]);
        }
        if !terminated {finish(cpu,63,true);return Ok(());} // ENAMETOOLONG
        if path!=b"/dev/urandom" {return Err("unsupported guest filesystem path in entropy-only service".into());}
        if flags!=0 || mode!=0 {finish(cpu,22,true);return Ok(());}
        if self.files.len()>=16 || self.next>i32::MAX as u32 {finish(cpu,24,true);return Ok(());}
        let file=match File::open("/dev/urandom") {Ok(file)=>file,Err(error)=>{finish(cpu,io_errno(&error),true);return Ok(());}};
        let name=self.next;self.next+=1;self.files.insert(name,file);finish(cpu,name as u64,false);Ok(())
    }
    pub(super) fn read(&mut self,cpu:&mut A64Cpu,args:[u64;3])->Result<(),String> {
        let [name,address,count]=args;
        let Ok(name)=u32::try_from(name) else {finish(cpu,9,true);return Ok(());};
        let Some(file)=self.files.get_mut(&name) else {finish(cpu,9,true);return Ok(());};
        if count>1024*1024 {return Err("entropy read exceeds bounded transfer budget".into());}
        if count==0 {finish(cpu,0,false);return Ok(());}
        if cpu.validate_guest_write(address,count as usize).is_err() {finish(cpu,14,true);return Ok(());}
        let mut bytes=vec![0;count as usize];
        let actual=loop {match file.read(&mut bytes) {
            Ok(actual)=>break actual,
            Err(error) if error.kind()==std::io::ErrorKind::Interrupted=>continue,
            Err(error)=>{finish(cpu,io_errno(&error),true);return Ok(());}
        }};
        cpu.write_guest_into(address,&bytes[..actual])?;
        finish(cpu,actual as u64,false);Ok(())
    }
    pub(super) fn close(&mut self,cpu:&mut A64Cpu,name:u64)->Result<(),String> {
        let removed=u32::try_from(name).ok().and_then(|name|self.files.remove(&name));
        finish(cpu,if removed.is_some(){0}else{9},removed.is_none());Ok(())
    }
}
fn finish(cpu:&mut A64Cpu,value:u64,error:bool) {
    let carry=1<<29;cpu.set_reg(0,value);
    cpu.set_pstate(if error {cpu.pstate()|carry}else{cpu.pstate()&!carry});
}
fn io_errno(error:&std::io::Error)->u64 {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied=>13,
        std::io::ErrorKind::NotFound=>2,
        std::io::ErrorKind::Interrupted=>4,
        _=> {
            // EBADF is 9 on the supported Unix/Android entropy backends.
            if cfg!(unix) && error.raw_os_error()==Some(9) {9} else {5}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->A64Cpu {
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,3).unwrap();
        cpu.write_guest_into(0x10000,b"/dev/urandom\0").unwrap();cpu
    }
    #[cfg(unix)]
    #[test]
    fn actual_entropy_descriptor_reads_guest_bytes_and_closes_once() {
        let mut cpu=fixture();let mut files=EntropyFds::new();cpu.set_pstate(0xb0000000);
        files.open(&mut cpu,[0x10000,0,0]).unwrap();let fd=cpu.reg(0);assert_eq!(fd,3);assert_eq!(cpu.pstate()&0xf0000000,0x90000000);
        files.read(&mut cpu,[fd,0x10100,32]).unwrap();assert_eq!(cpu.reg(0),32);
        assert_eq!(files.files.len(),1);
        files.close(&mut cpu,fd).unwrap();assert_eq!(cpu.reg(0),0);
        files.close(&mut cpu,fd).unwrap();assert_eq!(cpu.reg(0),9);assert_ne!(cpu.pstate()&(1<<29),0);
    }
    #[test]
    fn virtual_paths_and_bad_descriptors_do_not_open_host_files() {
        let mut cpu=fixture();let mut files=EntropyFds::new();
        cpu.write_guest_into(0x10000,b"/etc/passwd\0").unwrap();cpu.set_reg(0,0x123);
        assert!(files.open(&mut cpu,[0x10000,0,0]).is_err());assert_eq!(cpu.reg(0),0x123);assert!(files.files.is_empty());
        files.read(&mut cpu,[3,0x10000,8]).unwrap();assert_eq!(cpu.reg(0),9);
        files.open(&mut cpu,[0xffffffffffffffff,0,0]).unwrap();assert_eq!(cpu.reg(0),14);
    }
    #[cfg(unix)]
    #[test]
    fn getentropy_exact_fill_is_bounded_and_allocates_no_guest_descriptor() {
        let mut cpu=fixture();let files=EntropyFds::new();
        cpu.write_guest_into(0x10100,&[0x77;18]).unwrap();
        files.getentropy(&mut cpu,[0x10101,16]).unwrap();assert_eq!(cpu.reg(0),0);
        let mut bytes=[0;18];cpu.read_guest_into(0x10100,&mut bytes).unwrap();assert_eq!(bytes[0],0x77);assert_eq!(bytes[17],0x77);
        assert!(files.files.is_empty());assert_eq!(files.next,3);
        files.getentropy(&mut cpu,[0x10100,257]).unwrap();assert_eq!(cpu.reg(0),22);
        cpu.map_zeroed(0x20000,4096,1).unwrap();files.getentropy(&mut cpu,[0x20000,16]).unwrap();assert_eq!(cpu.reg(0),14);
        files.getentropy(&mut cpu,[0,0]).unwrap();assert_eq!(cpu.reg(0),0);
    }
    #[cfg(unix)]
    #[test]
    fn bad_output_and_transfer_budget_preserve_descriptor_and_guest_memory() {
        let mut cpu=fixture();let mut files=EntropyFds::new();files.open(&mut cpu,[0x10000,0,0]).unwrap();let fd=cpu.reg(0);
        cpu.map_zeroed(0x20000,4096,1).unwrap();files.read(&mut cpu,[fd,0x20000,8]).unwrap();assert_eq!(cpu.reg(0),14);assert_eq!(files.files.len(),1);
        assert!(files.read(&mut cpu,[fd,0x10100,1024*1024+1]).is_err());
        files.read(&mut cpu,[fd,0,0]).unwrap();assert_eq!(cpu.reg(0),0);
    }
}

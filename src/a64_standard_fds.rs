/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Process-owned standard streams: EOF input and bounded captured output.
//! Output sinks have no terminal/device driver; FIODTYPE returns ENOTTY.
use super::A64Cpu;
pub(super) struct StandardFds { output:[Vec<u8>;2] }
impl StandardFds {
    pub(super) fn new()->Self {Self{output:[Vec::new(),Vec::new()]}}
    pub(super) fn ioctl(&self,cpu:&mut A64Cpu,args:[u64;3])->Result<(),String> {
        let [fd,request,buffer]=args;
        if fd>2 {result(cpu,9);return Ok(());}
        let output_size=match request {
            0x4004667a=>4, // FIODTYPE, int
            0x40487413=>72, // TIOCGETA, LP64 struct termios
            _=>return Err(format!("unsupported standard-stream ioctl {request:#x}")),
        };
        // XNU ioctl preflights IOC_OUT storage before invoking a non-device's
        // ioctl operation. Preserve invalid-buffer EFAULT rather than ENOTTY.
        if cpu.validate_guest_write(buffer,output_size).is_err() {result(cpu,14);return Ok(());}
        result(cpu,25); // ENOTTY: captured stream is not a character device.
        Ok(())
    }
    pub(super) fn write(&mut self,cpu:&mut A64Cpu,args:[u64;3])->Result<(),String> {
        let [fd,address,count]=args;
        if !(1..=2).contains(&fd) {result(cpu,9);return Ok(());}
        let count=usize::try_from(count).map_err(|_|"standard output size overflow")?;
        if count>65536 || self.output[fd as usize-1].len().checked_add(count).is_none_or(|n|n>1048576) {
            return Err("captured standard output budget exceeded".into());
        }
        let mut bytes=vec![0u8;count];
        if cpu.read_guest_into(address,&mut bytes).is_err() {result(cpu,14);return Ok(());}
        self.output[fd as usize-1].extend_from_slice(&bytes);
        cpu.set_reg(0,count as u64);cpu.set_pstate(cpu.pstate()&!(1<<29));Ok(())
    }
}
fn result(cpu:&mut A64Cpu,errno:u64) {cpu.set_reg(0,errno);cpu.set_pstate(cpu.pstate()|(1<<29));}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn genuine_nonterminal_sink_returns_enotty_without_output_write() {
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,3).unwrap();
        cpu.write_guest_into(0x10000,&123u32.to_le_bytes()).unwrap();
        let mut fds=StandardFds::new();cpu.write_guest_into(0x10020,b"message").unwrap();
        fds.write(&mut cpu,[2,0x10020,7]).unwrap();assert_eq!(&fds.output[1],b"message");
        fds.ioctl(&mut cpu,[2,0x4004667a,0x10000]).unwrap();
        assert_eq!(cpu.reg(0),25);assert_ne!(cpu.pstate()&(1<<29),0);assert_eq!(cpu.read_u64(0x10000),Some(123));
    }
    #[test] fn invalid_descriptor_and_storage_have_distinct_errors() {
        let mut cpu=A64Cpu::new_sparse();let fds=StandardFds::new();
        fds.ioctl(&mut cpu,[42,0x4004667a,0]).unwrap();assert_eq!(cpu.reg(0),9);
        fds.ioctl(&mut cpu,[2,0x4004667a,0]).unwrap();assert_eq!(cpu.reg(0),14);
        assert!(fds.ioctl(&mut cpu,[2,123,0]).is_err());
    }
    #[test] fn termios_request_checks_full_lp64_output_before_non_tty_error() {
        let mut cpu=A64Cpu::new_sparse();let fds=StandardFds::new();
        cpu.map_zeroed(0x10000,71,3).unwrap();
        fds.ioctl(&mut cpu,[2,0x40487413,0x10000]).unwrap();assert_eq!(cpu.reg(0),14);
        cpu.map_zeroed(0x10047,1,3).unwrap();
        fds.ioctl(&mut cpu,[2,0x40487413,0x10000]).unwrap();assert_eq!(cpu.reg(0),25);
        let mut bytes=[1u8;72];cpu.read_guest_into(0x10000,&mut bytes).unwrap();assert_eq!(bytes,[0;72]);
    }
}

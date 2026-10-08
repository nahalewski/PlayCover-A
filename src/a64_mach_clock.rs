/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Evidenced host_get_clock_service(206), SYSTEM_CLOCK only.
//! Kernel clock service is a monotonic virtual-kernel uptime, never calendar time.
use super::{mach_identity::{MachIdentity,PortRight}, A64Cpu};
use std::time::Instant;
const NDR:[u8;8]=[0,0,0,0,1,0,0,0];
pub(super) struct SystemClock { epoch:Instant }
impl SystemClock {
    pub(super) fn new()->Self { Self {epoch:Instant::now()} }
    pub(super) fn time(&self)->Result<(u32,u32),String> {
        let elapsed=self.epoch.elapsed();
        let seconds=u32::try_from(elapsed.as_secs()).map_err(|_|"virtual clock seconds overflow")?;
        Ok((seconds,elapsed.subsec_nanos()))
    }
    pub(super) fn reply(&self,cpu:&mut A64Cpu,ports:&mut MachIdentity,args:[u64;8])->Result<u32,String> {
        self.reply_inner(cpu,ports,args,false)
    }
    pub(super) fn reply_legacy(&self,cpu:&mut A64Cpu,ports:&mut MachIdentity,args:[u64;8])->Result<u32,String>{self.reply_inner(cpu,ports,args,true)}
    fn reply_inner(&self,cpu:&mut A64Cpu,ports:&mut MachIdentity,args:[u64;8],legacy:bool)->Result<u32,String>{
        let [data,options,bits_size,remote_local,voucher_id,descriptors_receive,receive_priority,timeout]=args;
        let host=remote_local as u32;
        let receive=(remote_local>>32) as u32;
        let capacity=receive_priority as u32;
        if options!=0x200000003 || bits_size!=0x2400001513 || voucher_id!=206u64<<32
            || descriptors_receive!=(receive as u64)<<32 || receive_priority>>32!=0
            || capacity<48 || capacity>4096 || timeout!=0 {
            return Err("unsupported host clock message envelope".into());
        }
        if !matches!(ports.right(host),Some(PortRight::HostSend{references}) if references>0) {return Ok(0x10000003);}
        if !matches!(ports.right(receive),Some(PortRight::ConstructedReplyReceive))&&!(legacy&&matches!(ports.right(receive),Some(PortRight::ReplyReceive))) {return Ok(0x10000009);}
        let mut request=[0;36];
        cpu.read_guest_into(data,&mut request)?;
        for (offset,value) in [(0,0x1513),(4,36),(8,host),(12,receive),(16,0),(20,206),(32,0)] {
            // mach_msg_overwrite_trap sets copied msgh_size from send_size.
            if legacy&&offset==4{continue;}
            if word(&request,offset)!=value {return Err("unsupported host clock request".into());}
        }
        if request[24..32]!=NDR {return Err("unsupported host clock NDR".into());}
        cpu.validate_guest_write(data,capacity as usize)?;
        // The endpoint is backed by this actual live clock provider.
        self.time()?;
        let ticket=ports.prepare_system_clock_reply(host,receive)?;
        let mut response=[0;48];
        for (offset,value) in [(0,0x80001200),(4,40),(12,receive),(20,306),(24,1),(28,ticket.name()),(44,8)] {
            put(&mut response,offset,value);
        }
        // Receiver-side port descriptor: send right, MACH_MSG_PORT_DESCRIPTOR.
        response[38]=17;
        if let Err(error)=cpu.write_guest_into(data,&response) {
            ports.cancel_clock_reply(ticket)?;
            return Err(error);
        }
        ports.commit_clock_reply(ticket)?;
        Ok(0)
    }
}
fn word(bytes:&[u8],offset:usize)->u32 {u32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap())}
fn put(bytes:&mut[u8],offset:usize,value:u32) {bytes[offset..offset+4].copy_from_slice(&value.to_le_bytes());}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(A64Cpu,MachIdentity,[u64;8]) {
        let mut cpu=A64Cpu::new_sparse(); cpu.map_zeroed(0x10000,0x4000,3).unwrap();
        let mut ports=MachIdentity::new(16).unwrap();
        let task=ports.trap(-28).unwrap(); let host=ports.trap(-29).unwrap();
        let receive=ports.construct_reply(task,0,0x1000).unwrap();
        let mut request=[0;36];
        for (offset,value) in [(0,0x1513),(4,36),(8,host),(12,receive),(20,206)] {put(&mut request,offset,value);}
        request[24..32].copy_from_slice(&NDR); cpu.write_guest_into(0x10000,&request).unwrap();
        (cpu,ports,[0x10000,0x200000003,0x2400001513,(receive as u64)<<32|host as u64,206u64<<32,(receive as u64)<<32,48,0])
    }
    #[test]
    fn actual_complex_reply_transfers_owned_clock_service() {
        let (mut cpu,mut ports,args)=fixture(); let clock=SystemClock::new();
        let host=ports.right(args[3] as u32);
        assert_eq!(clock.reply(&mut cpu,&mut ports,args).unwrap(),0);
        let mut bytes=[0;48];cpu.read_guest_into(args[0],&mut bytes).unwrap();
        assert_eq!(word(&bytes,0),0x80001200);assert_eq!(word(&bytes,4),40);
        assert_eq!(word(&bytes,8),0);assert_eq!(word(&bytes,12),(args[3]>>32) as u32);
        assert_eq!(word(&bytes,20),306);assert_eq!(word(&bytes,24),1);
        assert!(ports.right(word(&bytes,28)).is_some());assert_eq!(bytes[38],17);assert_eq!(bytes[39],0);
        assert_eq!(word(&bytes,40),0);assert_eq!(word(&bytes,44),8);
        assert_eq!(ports.right(args[3] as u32),host);
        let first=clock.time().unwrap();let second=clock.time().unwrap();assert!(second>=first);
    }
    #[test]
    fn unknown_clock_is_not_fabricated() {
        let (mut cpu,mut ports,args)=fixture();
        cpu.write_guest_into(args[0]+32,&1u32.to_le_bytes()).unwrap();
        assert!(SystemClock::new().reply(&mut cpu,&mut ports,args).is_err());
        let mut header=[0;24];cpu.read_guest_into(args[0],&mut header).unwrap();assert_eq!(word(&header,20),206);
    }
    #[test]
    fn readonly_copyout_is_rejected_before_right_transfer() {
        let (mut cpu,mut ports,mut args)=fixture();
        let mut request=[0;36];cpu.read_guest_into(args[0],&mut request).unwrap();
        cpu.map_zeroed(0x20000,4096,1).unwrap();cpu.try_write_bytes(0x20000,&request).unwrap();
        args[0]=0x20000;
        let host=ports.right(args[3] as u32);let receive=ports.right((args[3]>>32) as u32);
        assert!(SystemClock::new().reply(&mut cpu,&mut ports,args).is_err());
        assert_eq!(ports.right(args[3] as u32),host);assert_eq!(ports.right((args[3]>>32) as u32),receive);
        let mut after=[0;36];cpu.read_guest_into(args[0],&mut after).unwrap();assert_eq!(after,request);
    }
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Actual task semaphore_create RPC3418, bounded FIFO counting objects.
//! Creation retains real zero-count state. No absent wait/signal is acknowledged.
use super::{mach_identity::{MachIdentity,PortRight}, A64Cpu};
use std::collections::BTreeMap;
const NDR:[u8;8]=[0,0,0,0,1,0,0,0];
#[derive(Debug,PartialEq,Eq)]
pub(super) struct Semaphore {pub(super) count:u32,pub(super) policy:u32}
pub(super) struct SemaphoreService {objects:BTreeMap<u32,Semaphore>}
impl SemaphoreService {
    pub(super) fn new()->Self {Self {objects:BTreeMap::new()}}
    pub(super) fn record(&self,name:u32)->Option<&Semaphore> {self.objects.get(&name)}
    pub(super) fn reply(&mut self,cpu:&mut A64Cpu,ports:&mut MachIdentity,args:[u64;8])->Result<u32,String> {
        self.reply_inner(cpu,ports,args,false)
    }
    pub(super) fn reply_legacy(&mut self,cpu:&mut A64Cpu,ports:&mut MachIdentity,args:[u64;8])->Result<u32,String>{self.reply_inner(cpu,ports,args,true)}
    fn reply_inner(&mut self,cpu:&mut A64Cpu,ports:&mut MachIdentity,args:[u64;8],legacy:bool)->Result<u32,String>{
        let [data,options,bits_size,remote_local,voucher_id,descriptors_receive,receive_priority,timeout]=args;
        let task=remote_local as u32;let receive=(remote_local>>32) as u32;let capacity=receive_priority as u32;
        if options!=0x200000003 || bits_size!=0x2800001513 || voucher_id!=3418u64<<32
            || descriptors_receive!=(receive as u64)<<32 || receive_priority>>32!=0
            || capacity<48 || capacity>4096 || timeout!=0 {return Err("unsupported semaphore_create envelope".into());}
        if !matches!(ports.right(task),Some(PortRight::TaskSend{references}) if references>0) {return Ok(0x10000003);}
        if !matches!(ports.right(receive),Some(PortRight::ConstructedReplyReceive))&&!(legacy&&matches!(ports.right(receive),Some(PortRight::ReplyReceive))) {return Ok(0x10000009);}
        let mut request=[0;40];cpu.read_guest_into(data,&mut request)?;
        for (offset,value) in [(0,0x1513),(4,40),(8,task),(12,receive),(16,0),(20,3418),(32,0),(36,0)] {
            if legacy&&offset==4{continue;}
            if word(&request,offset)!=value {return Err("unsupported semaphore_create body or FIFO count".into());}
        }
        if request[24..32]!=NDR {return Err("unsupported semaphore_create NDR".into());}
        cpu.validate_guest_write(data,capacity as usize)?;
        if self.objects.len()>=64 {return Err("virtual semaphore object budget exhausted".into());}
        let ticket=ports.prepare_semaphore_reply(task,receive)?;let name=ticket.name();
        if self.objects.contains_key(&name) {ports.cancel_semaphore_reply(ticket)?;return Err("semaphore namespace collision".into());}
        let mut response=[0;48];
        for (offset,value) in [(0,0x80001200),(4,40),(12,receive),(20,3518),(24,1),(28,name),(44,8)] {put(&mut response,offset,value);}
        response[38]=17;
        self.objects.insert(name,Semaphore{count:0,policy:0});
        if let Err(error)=cpu.write_guest_into(data,&response) {
            self.objects.remove(&name);ports.cancel_semaphore_reply(ticket)?;return Err(error);
        }
        ports.commit_semaphore_reply(ticket)?;
        Ok(0)
    }
}
fn word(bytes:&[u8],offset:usize)->u32 {u32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap())}
fn put(bytes:&mut[u8],offset:usize,value:u32) {bytes[offset..offset+4].copy_from_slice(&value.to_le_bytes());}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(A64Cpu,MachIdentity,[u64;8]) {
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,0x4000,3).unwrap();
        let mut ports=MachIdentity::new(16).unwrap();let task=ports.trap(-28).unwrap();let receive=ports.construct_reply(task,0,0x1000).unwrap();
        let mut request=[0;40];for (offset,value) in [(0,0x1513),(4,40),(8,task),(12,receive),(20,3418)] {put(&mut request,offset,value);}
        request[24..32].copy_from_slice(&NDR);cpu.write_guest_into(0x10000,&request).unwrap();
        (cpu,ports,[0x10000,0x200000003,0x2800001513,(receive as u64)<<32|task as u64,3418u64<<32,(receive as u64)<<32,48,0])
    }
    #[test]
    fn creation_delivers_descriptor_and_retains_real_zero_count_object() {
        let (mut cpu,mut ports,args)=fixture();let mut service=SemaphoreService::new();
        assert_eq!(service.reply(&mut cpu,&mut ports,args).unwrap(),0);
        let mut reply=[0;48];cpu.read_guest_into(args[0],&mut reply).unwrap();
        assert_eq!(word(&reply,0),0x80001200);assert_eq!(word(&reply,4),40);assert_eq!(word(&reply,20),3518);
        assert_eq!(word(&reply,24),1);assert_eq!(reply[38],17);assert_eq!(reply[39],0);assert_eq!(word(&reply,44),8);
        let name=word(&reply,28);assert!(ports.right(name).is_some());assert_eq!(service.record(name),Some(&Semaphore{count:0,policy:0}));
    }
    #[test]
    fn unsupported_count_and_readonly_copyout_do_not_create_objects() {
        let (mut cpu,mut ports,mut args)=fixture();let mut service=SemaphoreService::new();
        cpu.write_guest_into(args[0]+36,&1u32.to_le_bytes()).unwrap();assert!(service.reply(&mut cpu,&mut ports,args).is_err());assert!(service.objects.is_empty());
        cpu.write_guest_into(args[0]+36,&0u32.to_le_bytes()).unwrap();let mut request=[0;40];cpu.read_guest_into(args[0],&mut request).unwrap();
        cpu.map_zeroed(0x20000,4096,1).unwrap();cpu.try_write_bytes(0x20000,&request).unwrap();args[0]=0x20000;
        assert!(service.reply(&mut cpu,&mut ports,args).is_err());assert!(service.objects.is_empty());
    }
}

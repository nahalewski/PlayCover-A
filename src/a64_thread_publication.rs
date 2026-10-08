/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Validated diagnostic publication into the cooperative scheduler, never a
//! successful production pthread_create or a fabricated Darwin pthread_t.
use super::{A64Cpu,pthread_create_prepare::{Prepared,Layout,Attributes,CreateArguments},thread_scheduler_cpu::{CpuScheduler,ThreadId}};
pub(super) struct Failure{pub error:String,pub prepared:Prepared}
#[derive(Clone,Copy)]pub(super) struct Receipt{thread:ThreadId,layout:Layout}
impl Receipt{pub(super) fn thread(&self)->ThreadId{self.thread}pub(super) fn record(&self)->u64{self.layout.record.0}}
pub(super) struct Publications{records:Vec<Receipt>,forbidden:Vec<(u64,u64)>}
fn overlap(a:(u64,u64),b:(u64,u64))->bool{a.0<b.1&&b.0<a.1}
fn ranges(layout:Layout)->[(u64,u64);4]{[layout.guard,layout.stack,layout.record,layout.code]}
impl Publications{
    /// Owner must include WHOLE bridge reserved ranges, including guard gaps,
    /// and all non-thread runtime arenas. An empty policy is not permitted.
    pub(super) fn new(forbidden:Vec<(u64,u64)>)->Result<Self,String>{if forbidden.is_empty()||forbidden.len()>64||forbidden.iter().any(|&(a,b)|a>=b){return Err("thread publication needs bounded explicit reserved arena ranges".into())}Ok(Self{records:Vec::new(),forbidden})}
    pub(super) fn lookup(&self,record:u64)->Option<ThreadId>{self.records.iter().find(|r|r.record()==record).map(|r|r.thread)}
    /// All registry allocation and guest validation happen before scheduler
    /// adoption. After adoption only an already-reserved infallible Vec push
    /// commits identity. Errors return owning Prepared for retry/quarantine.
    pub(super) fn publish(&mut self,cpu:&mut A64Cpu,scheduler:&mut CpuScheduler,prepared:Prepared)->Result<Receipt,Failure>{
        let result=(||{
            if self.records.len()>=64{return Err("diagnostic publication capacity reached".into())}
            if scheduler.service_identity().is_some(){return Err("publication cannot borrow a running/exiting thread's CPU".into())}
            self.validate_storage(cpu,&prepared)?;self.records.try_reserve(1).map_err(|_|"publication registry allocation failed")?;
            let caller=cpu.save_context();cpu.restore_context(&prepared.context);
            let adopted=(||{self.validate_context(cpu,&prepared)?;scheduler.adopt(cpu,prepared.layout.stack)})();
            cpu.restore_context(&caller);let thread=adopted?;
            let receipt=Receipt{thread,layout:prepared.layout};self.records.push(receipt);Ok(receipt)
        })();result.map_err(|error|Failure{error,prepared})
    }
    fn validate_storage(&self,cpu:&A64Cpu,prepared:&Prepared)->Result<(),String>{
        let l=prepared.layout;let a=prepared.arguments;CreateArguments::from_registers([a.output,a.attributes,a.routine,a.argument])?;
        if a.attributes!=0{return Err("publication rejects unaudited guest attributes".into())}
        let canonical=Layout::new(l.stack.0,l.record.0,l.code.0,Attributes{stack_size:l.stack.1.checked_sub(l.stack.0).ok_or("stack range invalid")?,..Default::default()})?;if l!=canonical{return Err("publication layout fields disagree".into())}
        for own in ranges(l){if self.forbidden.iter().any(|&reserved|overlap(own,reserved)){return Err("thread storage intersects reserved runtime/bridge arena".into())}if self.records.iter().any(|old|ranges(old.layout).iter().any(|&range|overlap(own,range))){return Err("thread storage already published or aliases another thread".into())}}
        if self.forbidden.iter().any(|&(start,end)|a.routine>=start&&a.routine<end){return Err("thread entry is inside reserved runtime/bridge arena".into())}
        let output=(a.output,a.output.checked_add(8).ok_or("output range overflow")?);if ranges(l).iter().any(|&r|overlap(output,r)){return Err("output pointer aliases unpublished storage".into())}
        cpu.validate_guest_write(a.output,8)?;cpu.validate_guest_write(l.stack.0,(l.stack.1-l.stack.0)as usize)?;cpu.validate_guest_write(l.record.0,(l.record.1-l.record.0)as usize)?;
        for address in l.guard.0..l.guard.1{if cpu.mapped_permissions(address).is_some(){return Err("publication guard is mapped".into())}}
        let expected_record=l.record_bytes();let mut actual_record=vec![0;expected_record.len()];cpu.read_guest_into(l.record.0,&mut actual_record)?;if actual_record!=expected_record{return Err("prepared pthread/TSD storage changed before publication".into())}
        let expected_code:Vec<u8>=l.trampoline().iter().flat_map(|w|w.to_le_bytes()).collect();let mut actual_code=[0;16];cpu.read_guest_into(l.code.0,&mut actual_code)?;if actual_code.as_slice()!=expected_code.as_slice(){return Err("prepared start trampoline changed".into())}
        for address in l.code.0..l.code.1{if !cpu.mapped_permissions(address).is_some_and(|p|p&5==5){return Err("prepared trampoline no longer executable/readable".into())}}
        if !cpu.mapped_permissions(a.routine).is_some_and(|p|p&4!=0){return Err("prepared start routine no longer executable".into())}let mut instruction=[0;4];cpu.read_guest_into(a.routine,&mut instruction)?;Ok(())
    }
    fn validate_context(&self,cpu:&A64Cpu,p:&Prepared)->Result<(),String>{if cpu.pc()!=p.layout.code.0||cpu.sp()!=p.layout.stack.1||cpu.reg(0)!=p.arguments.argument||cpu.reg(19)!=p.arguments.routine||cpu.reg(20)!=p.layout.completion_pc()||cpu.tpidr_el0()!=0{return Err("prepared CPU entry/stack/argument/thread register contract invalid".into())}Ok(())}
}
#[cfg(test)]mod tests{
    use super::*;use super::super::pthread_create_prepare::prepare;
    fn fixture()->(A64Cpu,Prepared){let mut cpu=A64Cpu::new_sparse();for(base,size,perm)in[(0x10000,4096,3),(0x20000,8192,3),(0x30000,4096,5),(0x50000,4096,5),(0x80000,512*1024,3)]{cpu.map_zeroed(base,size,perm).unwrap();}cpu.write_bytes(0x50000,&[0x91000400u32,0xd65f03c0].iter().flat_map(|w|w.to_le_bytes()).collect::<Vec<_>>());cpu.set_reg(7,777);cpu.set_tpidr_el0(0xabcdef);let l=Layout::new(0x80000,0x20000,0x30000,Attributes::default()).unwrap();let a=CreateArguments::from_registers([0x10000,0,0x50000,41]).unwrap();let p=prepare(&mut cpu,a,l).unwrap();(cpu,p)}
    #[test]fn publication_adopts_real_context_without_darwin_success(){let(mut cpu,p)=fixture();let mut s=CpuScheduler::default();let mut registry=Publications::new(vec![(0x1000000,0x1011000)]).unwrap();let receipt=match registry.publish(&mut cpu,&mut s,p){Ok(r)=>r,Err(f)=>panic!("{}",f.error)};assert_eq!(registry.lookup(0x20000),Some(receipt.thread()));assert_eq!(cpu.reg(7),777);assert_eq!(cpu.tpidr_el0(),0xabcdef);assert_eq!(cpu.read_u64(0x10000),Some(0));assert_eq!(cpu.read_u64(0x20000),Some(0));assert_eq!(s.select(&mut cpu).unwrap(),Some(receipt.thread()));for _ in 0..4{cpu.run_or_step(None);}assert_eq!(cpu.reg(0),42);assert_eq!(cpu.pc(),0x30008);}
    #[test]fn mutated_storage_rejection_preserves_identity_and_retry_context(){let(mut cpu,p)=fixture();cpu.write_bytes(0x20000+224,&123u64.to_le_bytes());let mut s=CpuScheduler::default();let mut registry=Publications::new(vec![(0x1000000,0x1011000)]).unwrap();let failure=match registry.publish(&mut cpu,&mut s,p){Err(f)=>f,Ok(_)=>panic!("changed TSD published")};assert!(failure.error.contains("storage changed"));assert!(registry.lookup(0x20000).is_none());assert!(s.select(&mut cpu).unwrap().is_none());cpu.write_bytes(0x20000,&failure.prepared.layout.record_bytes());assert!(registry.publish(&mut cpu,&mut s,failure.prepared).is_ok());}
    #[test]fn runtime_arena_overlap_never_adopts(){let(mut cpu,p)=fixture();let mut s=CpuScheduler::default();let mut registry=Publications::new(vec![(0x30000,0x41000)]).unwrap();assert!(registry.publish(&mut cpu,&mut s,p).is_err());assert!(s.select(&mut cpu).unwrap().is_none());assert_eq!(cpu.reg(7),777);}
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Pristine, empty condition-variable state. There is no guest wait scheduler.
use std::collections::BTreeSet;
pub const EINVAL:u64=22;
pub const EBUSY:u64=16;
pub const EAGAIN:u64=35;
#[derive(Clone,Default)]pub struct Conditions{objects:BTreeSet<u64>}
impl Conditions{
    pub fn contains(&self,addr:u64)->bool{self.objects.contains(&addr)}
    pub fn init(&mut self,addr:u64)->u64{
        // Darwin supports 4-byte-aligned condvars even for LP64.
        if addr==0||addr%4!=0{return EINVAL}
        if self.contains(addr){return EBUSY}
        if self.objects.len()>=4096{return EAGAIN}
        self.objects.insert(addr);0
    }
    pub fn destroy(&mut self,addr:u64)->u64{if self.objects.remove(&addr){0}else{EINVAL}}
    /// No sleepers can be registered until an actual scheduler owns their
    /// atomic mutex release, suspension and reacquisition. Empty signal is a
    /// real no-op, not a remembered notification or fabricated wake receipt.
    pub fn signal(&self,addr:u64)->u64{if self.contains(addr){0}else{EINVAL}}
    pub fn wait(&self,addr:u64,thread:u64)->Result<u64,String>{
        if thread==0{return Err("condition wait requires explicit guest-thread identity".into())}
        if !self.contains(addr){return Ok(EINVAL)}
        Err("pthread condition wait needs atomic mutex release, suspension and reacquisition; no effects performed".into())
    }
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn empty_signals_do_not_store_notifications(){let mut s=Conditions::default();assert_eq!(s.init(4),0);for _ in 0..10{assert_eq!(s.signal(4),0)}assert!(s.wait(4,1).is_err());assert_eq!(s.destroy(4),0);assert_eq!(s.signal(4),EINVAL);}
    #[test]fn unsupported_wait_preserves_lifecycle(){let mut s=Conditions::default();s.init(8);assert!(s.wait(8,1).is_err());assert_eq!(s.init(8),EBUSY);assert!(s.wait(8,0).is_err());assert_eq!(s.destroy(8),0);assert_eq!(s.wait(8,1),Ok(EINVAL));assert_eq!(s.init(8),0);}
    #[test]fn address_and_resource_bounds(){let mut s=Conditions::default();assert_eq!(s.init(0),EINVAL);assert_eq!(s.init(2),EINVAL);for i in 1..=4096{assert_eq!(s.init(i*4),0)}assert_eq!(s.init(4097*4),EAGAIN);assert_eq!(s.destroy(4),0);assert_eq!(s.init(4097*4),0);}
}

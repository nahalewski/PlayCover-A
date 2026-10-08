/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Single diagnostic-thread mutex ownership, without blocking or scheduling.
use std::collections::BTreeMap;
pub const EPERM:u64=1;
pub const EDEADLK:u64=11;
pub const EBUSY:u64=16;
pub const EINVAL:u64=22;
pub const EAGAIN:u64=35;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Kind{Normal,ErrorCheck,Recursive}
impl Kind{pub fn from_raw(raw:u64)->Option<Self>{match raw{0=>Some(Self::Normal),1=>Some(Self::ErrorCheck),2=>Some(Self::Recursive),_=>None}}}
#[derive(Clone,Debug)]struct Mutex{kind:Kind,owner:Option<u64>,depth:u16}
#[derive(Clone,Default)]pub struct Mutexes{objects:BTreeMap<u64,Mutex>}
impl Mutexes{
    pub fn contains(&self,addr:u64)->bool{self.objects.contains_key(&addr)}
    pub fn init(&mut self,addr:u64,kind:Kind)->Result<u64,String>{
        if addr==0||addr%8!=0{return Ok(EINVAL)}
        if self.contains(addr){return Ok(EBUSY)}
        if self.objects.len()>=4096{return Ok(EAGAIN)}
        self.objects.insert(addr,Mutex{kind,owner:None,depth:0});Ok(0)
    }
    pub fn destroy(&mut self,addr:u64)->u64{match self.objects.get(&addr){None=>EINVAL,Some(m)if m.owner.is_some()=>EBUSY,Some(_)=>{self.objects.remove(&addr);0}}}
    pub fn lock(&mut self,addr:u64,thread:u64,try_lock:bool)->Result<u64,String>{
        if thread==0{return Err("mutex diagnostic thread identity is absent".into())}
        let Some(m)=self.objects.get_mut(&addr)else{return Ok(EINVAL)};
        match m.owner{
            None=>{m.owner=Some(thread);m.depth=1;Ok(0)},
            Some(owner)if owner==thread&&m.kind==Kind::Recursive=>{if m.depth==u16::MAX{Ok(EAGAIN)}else{m.depth+=1;Ok(0)}},
            Some(_)if try_lock=>Ok(EBUSY),
            Some(owner)if owner==thread&&m.kind==Kind::ErrorCheck=>Ok(EDEADLK),
            Some(_)=>Err("pthread_mutex_lock needs blocking guest scheduling; ownership unchanged".into()),
        }
    }
    pub fn unlock(&mut self,addr:u64,thread:u64)->Result<u64,String>{
        if thread==0{return Err("mutex diagnostic thread identity is absent".into())}
        let Some(m)=self.objects.get_mut(&addr)else{return Ok(EINVAL)};
        if m.owner!=Some(thread){return if m.kind==Kind::Normal{Err("normal mutex non-owner unlock is undefined; rejected".into())}else{Ok(EPERM)}}
        m.depth-=1;if m.depth==0{m.owner=None}Ok(0)
    }
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn ownership_and_contention_are_not_faked(){let mut s=Mutexes::default();assert_eq!(s.init(8,Kind::Normal),Ok(0));assert_eq!(s.lock(8,1,false),Ok(0));assert!(s.lock(8,2,false).is_err());assert!(s.lock(8,1,false).is_err());assert_eq!(s.lock(8,2,true),Ok(EBUSY));assert_eq!(s.destroy(8),EBUSY);assert!(s.unlock(8,2).is_err());assert_eq!(s.unlock(8,1),Ok(0));assert_eq!(s.lock(8,2,false),Ok(0));}
    #[test]fn errorcheck_returns_darwin_errors(){let mut s=Mutexes::default();s.init(8,Kind::ErrorCheck).unwrap();s.lock(8,1,false).unwrap();assert_eq!(s.lock(8,1,false),Ok(EDEADLK));assert_eq!(s.lock(8,1,true),Ok(EBUSY));assert_eq!(s.unlock(8,2),Ok(EPERM));assert_eq!(s.unlock(8,1),Ok(0));assert_eq!(s.unlock(8,1),Ok(EPERM));assert_eq!(s.destroy(8),0);assert_eq!(s.destroy(8),EINVAL);}
    #[test]fn recursive_depth_and_overflow(){let mut s=Mutexes::default();s.init(8,Kind::Recursive).unwrap();for _ in 0..u16::MAX{assert_eq!(s.lock(8,1,false),Ok(0))}assert_eq!(s.lock(8,1,true),Ok(EAGAIN));for _ in 0..u16::MAX{assert_eq!(s.unlock(8,1),Ok(0))}assert_eq!(s.destroy(8),0);}
    #[test]fn bounds_and_reinit(){let mut s=Mutexes::default();assert_eq!(s.init(1,Kind::Normal),Ok(EINVAL));for i in 1..=4096{s.init(i*8,Kind::Normal).unwrap();}assert_eq!(s.init(8,Kind::Normal),Ok(EBUSY));assert_eq!(s.init(4097*8,Kind::Normal),Ok(EAGAIN));assert!(s.lock(8,0,false).is_err());}
}

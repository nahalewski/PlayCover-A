/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Recursive process loader lock for exclusively scheduled guest callbacks.
//! Contention is an explicit unsupported blocking boundary, never success.
#[derive(Default)]
pub(super) struct LoaderLock {owner:Option<u64>,depth:u32}
impl LoaderLock {
    pub(super) fn is_unlocked(&self)->bool {self.owner.is_none() && self.depth==0}
    pub(super) fn new()->Self {Self::default()}
    pub(super) fn enter(&mut self,owner:u64)->Result<(),String> {
        if owner==0 {return Err("loader lock requires an actual selected thread owner".into());}
        if self.owner.is_some_and(|current|current!=owner) {
            return Err("loader lock contention requires genuine scheduler blocking".into());
        }
        let depth=self.depth.checked_add(1).ok_or("loader lock recursion overflow")?;
        self.owner=Some(owner);self.depth=depth;Ok(())
    }
    pub(super) fn leave(&mut self,owner:u64)->Result<(),String> {
        if self.owner!=Some(owner)||self.depth==0 {return Err("loader lock release by nonowner".into());}
        self.depth-=1;if self.depth==0 {self.owner=None;}Ok(())
    }
    pub(super) fn require_owned(&self,owner:u64)->Result<(),String> {
        if self.owner!=Some(owner)||self.depth==0 {return Err("loader catalogue accessed without owned lock".into());}Ok(())
    }
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn recursion_retains_owner_until_final_release() {
        let mut lock=LoaderLock::new();lock.enter(1).unwrap();lock.enter(1).unwrap();
        lock.leave(1).unwrap();lock.require_owned(1).unwrap();assert!(lock.enter(2).is_err());
        assert!(lock.leave(2).is_err());lock.require_owned(1).unwrap();lock.leave(1).unwrap();
        assert!(lock.require_owned(1).is_err());lock.enter(2).unwrap();lock.leave(2).unwrap();
    }
    #[test]fn invalid_owner_overflow_and_unbalanced_release_are_atomic() {
        let mut lock=LoaderLock::new();assert!(lock.enter(0).is_err());assert!(lock.leave(1).is_err());
        lock.enter(1).unwrap();lock.depth=u32::MAX;assert!(lock.enter(1).is_err());
        assert_eq!(lock.depth,u32::MAX);lock.require_owned(1).unwrap();assert!(lock.enter(2).is_err());
    }
}

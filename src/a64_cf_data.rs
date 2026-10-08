/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! CFData no-copy guest ranges: no host pointer casts or copied-snapshot fiction.
//! Registration validates actual guest memory; reads observe current contents.
//! Guest heap freeing must be performed by the caller's real allocator bridge.
use std::collections::BTreeMap;
const MAX_OBJECTS: usize = 4096;
const MAX_LENGTH: u64 = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deallocator {
    /// Caller has positively identified kCFAllocatorNull.
    NeverFree,
    /// Caller has verified this range belongs to its default guest heap.
    GuestHeap,
}
struct Data {
    address: u64,
    length: usize,
    deallocator: Deallocator,
    retains: u64,
}
#[derive(Default)]
pub struct DataObjects {
    next: u64,
    objects: BTreeMap<u64, Data>,
}
impl DataObjects {
    pub fn create_no_copy(
        &mut self,
        address: u64,
        length: i64,
        deallocator: Deallocator,
        validate: impl FnOnce(u64, usize) -> Result<(), String>,
    ) -> Result<Handle, String> {
        if length < 0 || length as u64 > MAX_LENGTH || self.objects.len() >= MAX_OBJECTS {
            return Err("CFData length/object limit invalid".into());
        }
        address
            .checked_add(length as u64)
            .ok_or("CFData guest range overflow")?;
        if length != 0 {
            validate(address, length as usize)?;
        }
        let id = self
            .next
            .checked_add(1)
            .ok_or("CFData identity exhausted")?;
        self.next = id;
        self.objects.insert(
            id,
            Data {
                address,
                length: length as usize,
                deallocator,
                retains: 1,
            },
        );
        Ok(Handle(id))
    }
    fn active(&self, h: Handle) -> Result<&Data, String> {
        self.objects
            .get(&h.0)
            .filter(|d| d.retains != 0)
            .ok_or_else(|| "CFData unknown or deallocating".into())
    }
    pub fn byte_pointer(&self, h: Handle) -> Result<u64, String> {
        Ok(self.active(h)?.address)
    }
    pub fn length(&self, h: Handle) -> Result<usize, String> {
        Ok(self.active(h)?.length)
    }
    pub fn read(
        &self,
        h: Handle,
        offset: usize,
        count: usize,
        read_guest: impl FnOnce(u64, usize) -> Result<Vec<u8>, String>,
    ) -> Result<Vec<u8>, String> {
        let data = self.active(h)?;
        if count > 1024 * 1024
            || offset
                .checked_add(count)
                .is_none_or(|end| end > data.length)
        {
            return Err("CFData read bounds/budget exceeded".into());
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        let bytes = read_guest(data.address + offset as u64, count)?;
        if bytes.len() != count {
            return Err("CFData guest read length mismatch".into());
        }
        Ok(bytes)
    }
    pub fn retain(&mut self, h: Handle) -> Result<(), String> {
        self.active(h)?;
        let data = self.objects.get_mut(&h.0).unwrap();
        data.retains = data
            .retains
            .checked_add(1)
            .ok_or("CFData retain overflow")?;
        Ok(())
    }
    /// Caller must implement a real guest heap free, leaving state unchanged
    /// on error. A failed final free preserves deallocating state for retry.
    pub fn release(
        &mut self,
        h: Handle,
        guest_free: impl FnOnce(u64) -> Result<(), String>,
    ) -> Result<(), String> {
        let data = self.objects.get_mut(&h.0).ok_or("CFData unknown")?;
        if data.retains > 1 {
            data.retains -= 1;
            return Ok(());
        }
        data.retains = 0;
        if data.deallocator == Deallocator::GuestHeap && data.address != 0 {
            guest_free(data.address)?;
        }
        self.objects.remove(&h.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_copy_identity_and_live_guest_changes() {
        let mut objects = DataObjects::default();
        let h = objects
            .create_no_copy(0x1000, 3, Deallocator::NeverFree, |p, n| {
                assert_eq!((p, n), (0x1000, 3));
                Ok(())
            })
            .unwrap();
        let mut guest = vec![1, 2, 3];
        assert_eq!(objects.byte_pointer(h).unwrap(), 0x1000);
        assert_eq!(objects.length(h).unwrap(), 3);
        assert_eq!(
            objects.read(h, 0, 3, |_, _| Ok(guest.clone())).unwrap(),
            guest
        );
        guest[0] = 9;
        assert_eq!(
            objects.read(h, 0, 3, |_, _| Ok(guest.clone())).unwrap()[0],
            9
        );
        assert!(objects.read(h, 2, 2, |_, _| unreachable!()).is_err());
        objects
            .release(h, |_| panic!("null deallocator must not free"))
            .unwrap();
        assert!(objects.byte_pointer(h).is_err());
    }
    #[test]
    fn default_free_once_and_error_retry() {
        let mut objects = DataObjects::default();
        let h = objects
            .create_no_copy(0x2000, 8, Deallocator::GuestHeap, |_, _| Ok(()))
            .unwrap();
        objects.retain(h).unwrap();
        objects.release(h, |_| panic!("not final")).unwrap();
        assert!(objects
            .release(h, |_| Err("heap unavailable".into()))
            .is_err());
        assert!(objects.length(h).is_err());
        let mut frees = 0;
        objects
            .release(h, |p| {
                assert_eq!(p, 0x2000);
                frees += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(frees, 1);
        assert!(objects.release(h, |_| unreachable!()).is_err());
    }
    #[test]
    fn invalid_ranges_and_permission_failure() {
        let mut objects = DataObjects::default();
        assert!(objects
            .create_no_copy(u64::MAX, 2, Deallocator::NeverFree, |_, _| Ok(()))
            .is_err());
        assert!(objects
            .create_no_copy(0, -1, Deallocator::NeverFree, |_, _| Ok(()))
            .is_err());
        assert!(objects
            .create_no_copy(0x1000, 1, Deallocator::NeverFree, |_, _| Err(
                "unmapped".into()
            ))
            .is_err());
        let h = objects
            .create_no_copy(0, 0, Deallocator::NeverFree, |_, _| unreachable!())
            .unwrap();
        assert_eq!(
            objects.read(h, 0, 0, |_, _| unreachable!()).unwrap(),
            Vec::<u8>::new()
        );
    }
}

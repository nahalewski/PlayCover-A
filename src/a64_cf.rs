/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bounded owned CFString state. Handles are opaque host tokens, never guest
//! pointers. A bridge must map tokens to guest addresses; no toll-free ObjC
//! bridging or external constant-string interpretation is provided here.
use std::collections::BTreeMap;

pub const STRING_TYPE_ID: u64 = 1;
const MAX_UNITS: usize = 1024 * 1024;
const MAX_OBJECTS: usize = 4096;
const MAX_TOTAL_UNITS: usize = 8 * MAX_UNITS;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle(u64);
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    ExternalObject,
    InvalidEncoding,
    Bounds,
    Limit,
    RefcountOverflow,
}
struct OwnedString {
    units: Vec<u16>,
    retains: u64,
}
#[derive(Default)]
pub struct Strings {
    next: u64,
    total_units: usize,
    objects: BTreeMap<u64, OwnedString>,
}
impl Strings {
    pub fn create_utf8(&mut self, bytes: &[u8]) -> Result<Handle, Error> {
        if bytes.len() > MAX_UNITS * 4 {
            return Err(Error::Limit);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::InvalidEncoding)?;
        self.create_utf16(&text.encode_utf16().collect::<Vec<_>>())
    }
    pub fn create_utf16(&mut self, units: &[u16]) -> Result<Handle, Error> {
        if units.len() > MAX_UNITS
            || self.objects.len() >= MAX_OBJECTS
            || self.total_units + units.len() > MAX_TOTAL_UNITS
        {
            return Err(Error::Limit);
        }
        let id = self.next.checked_add(1).ok_or(Error::Limit)?;
        self.next = id; // Never reuse tokens after release.
        self.total_units += units.len();
        self.objects.insert(
            id,
            OwnedString {
                units: units.to_vec(),
                retains: 1,
            },
        );
        Ok(Handle(id))
    }
    fn object(&self, h: Handle) -> Result<&OwnedString, Error> {
        self.objects.get(&h.0).ok_or(Error::ExternalObject)
    }
    pub fn type_id(&self, h: Handle) -> Result<u64, Error> {
        self.object(h)?;
        Ok(STRING_TYPE_ID)
    }
    pub fn length(&self, h: Handle) -> Result<usize, Error> {
        Ok(self.object(h)?.units.len())
    }
    pub fn character(&self, h: Handle, index: usize) -> Result<u16, Error> {
        self.object(h)?
            .units
            .get(index)
            .copied()
            .ok_or(Error::Bounds)
    }
    pub fn retain_count(&self, h: Handle) -> Result<u64, Error> {
        Ok(self.object(h)?.retains)
    }
    pub fn retain(&mut self, h: Handle) -> Result<Handle, Error> {
        let s = self.objects.get_mut(&h.0).ok_or(Error::ExternalObject)?;
        s.retains = s.retains.checked_add(1).ok_or(Error::RefcountOverflow)?;
        Ok(h)
    }
    pub fn release(&mut self, h: Handle) -> Result<(), Error> {
        let s = self.objects.get_mut(&h.0).ok_or(Error::ExternalObject)?;
        s.retains -= 1;
        if s.retains == 0 {
            self.total_units -= s.units.len();
            self.objects.remove(&h.0);
        }
        Ok(())
    }
    /// UTF-8 CFStringGetBytes subset: range is in UTF-16 code units; result is
    /// (units converted, bytes). No terminator, lossy substitution or external
    /// representation/BOM. A None capacity queries all available bytes.
    pub fn utf8_bytes(
        &self,
        h: Handle,
        start: usize,
        count: usize,
        capacity: Option<usize>,
    ) -> Result<(usize, Vec<u8>), Error> {
        let units = &self.object(h)?.units;
        let end = start.checked_add(count).ok_or(Error::Bounds)?;
        let range = units.get(start..end).ok_or(Error::Bounds)?;
        let mut bytes = Vec::new();
        let mut consumed = 0;
        for scalar in char::decode_utf16(range.iter().copied()) {
            let scalar = scalar.map_err(|_| Error::InvalidEncoding)?;
            if capacity.is_some_and(|n| bytes.len() + scalar.len_utf8() > n) {
                break;
            }
            let mut buffer = [0; 4];
            bytes.extend_from_slice(scalar.encode_utf8(&mut buffer).as_bytes());
            consumed += scalar.len_utf16();
        }
        Ok((consumed, bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_and_capacity() {
        let mut s = Strings::default();
        let h = s.create_utf8("A😀é\0".as_bytes()).unwrap();
        assert_eq!(s.length(h), Ok(5));
        assert_eq!(s.character(h, 1), Ok(0xd83d));
        assert_eq!(s.character(h, 2), Ok(0xde00));
        assert_eq!(s.utf8_bytes(h, 0, 5, Some(4)), Ok((1, b"A".to_vec())));
        assert_eq!(
            s.utf8_bytes(h, 1, 2, Some(4)),
            Ok((2, "😀".as_bytes().to_vec()))
        );
        assert_eq!(
            s.utf8_bytes(h, 0, 5, None),
            Ok((5, "A😀é\0".as_bytes().to_vec()))
        );
        assert_eq!(s.utf8_bytes(h, 2, 1, None), Err(Error::InvalidEncoding));
        assert_eq!(s.character(h, 5), Err(Error::Bounds));
        assert_eq!(s.utf8_bytes(h, usize::MAX, 1, None), Err(Error::Bounds));
    }
    #[test]
    fn ownership_and_unknown_objects() {
        let mut s = Strings::default();
        let h = s.create_utf16(&[65]).unwrap();
        assert_eq!(s.type_id(h), Ok(STRING_TYPE_ID));
        s.retain(h).unwrap();
        s.release(h).unwrap();
        assert_eq!(s.retain_count(h), Ok(1));
        s.release(h).unwrap();
        assert_eq!(s.length(h), Err(Error::ExternalObject));
        assert_eq!(s.release(h), Err(Error::ExternalObject));
        assert_eq!(s.total_units, 0);
        assert_ne!(h, s.create_utf8(b"new").unwrap());
        assert_eq!(s.retain(Handle(0xfeed)), Err(Error::ExternalObject));
    }
    #[test]
    fn encoding_and_limits() {
        let mut s = Strings::default();
        assert_eq!(s.create_utf8(&[0xff]), Err(Error::InvalidEncoding));
        assert_eq!(s.create_utf16(&vec![0; MAX_UNITS + 1]), Err(Error::Limit));
        let h = s.create_utf16(&[0xd800]).unwrap();
        assert_eq!(s.length(h), Ok(1));
        assert_eq!(s.utf8_bytes(h, 0, 1, None), Err(Error::InvalidEncoding));
        s.objects.get_mut(&h.0).unwrap().retains = u64::MAX;
        assert_eq!(s.retain(h), Err(Error::RefcountOverflow));
    }
}

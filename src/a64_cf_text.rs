/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Terraria CFString helper contracts. No guest pointers or allocator stubs.
pub const UTF8: u64 = 0x08000100;
pub const ASCII: u64 = 0x0600;
const MAX_UNITS: usize = 1024 * 1024;

/// CFStringGetMaximumSizeForEncoding returns an upper byte bound (excluding
/// the C terminator), or -1 when that encoding is unsupported.
pub fn maximum_size(length: i64, encoding: u64) -> Result<i64, String> {
    if length < 0 {
        return Err("negative CF string length".into());
    }
    match encoding {
        UTF8 => Ok(length.checked_mul(3).unwrap_or(-1)),
        ASCII => Ok(length),
        _ => Ok(-1),
    }
}

pub fn characters_from_le_bytes(bytes: &[u8]) -> Result<Vec<u16>, String> {
    if bytes.len() % 2 != 0 || bytes.len() / 2 > MAX_UNITS {
        return Err("CF character buffer length invalid/excessive".into());
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect())
}

/// CFStringGetCString succeeds only if the entire conversion plus a NUL fits.
/// None is genuine conversion/capacity failure, not a successful empty string.
pub fn c_string(units: &[u16], encoding: u64, capacity: usize) -> Result<Option<Vec<u8>>, String> {
    if units.len() > MAX_UNITS || capacity > MAX_UNITS * 3 + 1 {
        return Err("CF CString conversion limit exceeded".into());
    }
    if !matches!(encoding, UTF8 | ASCII) {
        return Err("CF CString encoding unsupported".into());
    }
    let mut output = Vec::new();
    for scalar in char::decode_utf16(units.iter().copied()) {
        let Ok(scalar) = scalar else {
            return Ok(None);
        };
        if encoding == ASCII && !scalar.is_ascii() {
            return Ok(None);
        }
        let mut bytes = [0; 4];
        let encoded = scalar.encode_utf8(&mut bytes).as_bytes();
        if output.len() + encoded.len() + 1 > capacity {
            return Ok(None);
        }
        output.extend_from_slice(encoded);
    }
    if output.len() + 1 > capacity {
        return Ok(None);
    }
    output.push(0);
    Ok(Some(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_c_string_capacity_and_failure() {
        let units: Vec<u16> = "é😀".encode_utf16().collect();
        assert_eq!(
            c_string(&units, UTF8, 7).unwrap(),
            Some("é😀\0".as_bytes().to_vec())
        );
        assert_eq!(c_string(&units, UTF8, 6).unwrap(), None);
        assert_eq!(c_string(&units, ASCII, 20).unwrap(), None);
        assert_eq!(c_string(&[0xd800], UTF8, 8).unwrap(), None);
        assert_eq!(c_string(&[], UTF8, 0).unwrap(), None);
        assert_eq!(c_string(&[], UTF8, 1).unwrap(), Some(vec![0]));
        assert!(c_string(&[], 42, 1).is_err());
    }
    #[test]
    fn maximum_size_and_character_input() {
        assert_eq!(maximum_size(3, UTF8).unwrap(), 9);
        assert_eq!(maximum_size(i64::MAX, UTF8).unwrap(), -1);
        assert_eq!(maximum_size(1, 42).unwrap(), -1);
        assert!(maximum_size(-1, UTF8).is_err());
        assert_eq!(
            characters_from_le_bytes(&[0x3d, 0xd8, 0, 0xde]).unwrap(),
            vec![0xd83d, 0xde00]
        );
        assert!(characters_from_le_bytes(&[0]).is_err());
    }
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Integer-backed CFNumber conversion for explicitly owned values on Apple
//! LP64. No decoding of cached CF/NSNumber objects, tagged pointers or isa.
//! Independently implemented from the CFNumber public type contract and
//! inspected CFNumber.c compatibility behavior; no upstream code copied.

#[derive(Clone, Copy, Debug)]
pub(super) struct SignedNumber(pub i64);

#[derive(Debug)]
pub(super) struct Conversion {
    pub bytes: Vec<u8>,
    /// CFNumberGetValue's Boolean, including its documented-in-source small
    /// unsigned-value compatibility for the 8- and 16-bit signed targets.
    pub success: bool,
}

impl SignedNumber {
    pub(super) fn convert(self, kind: u64) -> Result<Conversion, String> {
        let value = self.0;
        let (bytes, success) = match kind {
            1 | 7 => {
                let converted = value as i8;
                (
                    converted.to_le_bytes().to_vec(),
                    i64::from(converted) == value || (0..256).contains(&value),
                )
            }
            2 | 8 => {
                let converted = value as i16;
                (
                    converted.to_le_bytes().to_vec(),
                    i64::from(converted) == value || (0..65536).contains(&value),
                )
            }
            3 | 9 => {
                let converted = value as i32;
                (
                    converted.to_le_bytes().to_vec(),
                    i64::from(converted) == value,
                )
            }
            4 | 10 | 11 | 14 | 15 => (value.to_le_bytes().to_vec(), true),
            5 | 12 => (
                (value as f32).to_le_bytes().to_vec(),
                exactly_representable(value, 24),
            ),
            6 | 13 | 16 => (
                (value as f64).to_le_bytes().to_vec(),
                exactly_representable(value, 53),
            ),
            _ => return Err(format!("unsupported CFNumber LP64 type {kind}")),
        };
        Ok(Conversion { bytes, success })
    }
}

// Avoid an out-of-range float -> i64 cast when checking the +2^63 rounded
// representation of i64::MAX. Integer bits establish exactness directly.
fn exactly_representable(value: i64, precision: u32) -> bool {
    let magnitude = value.unsigned_abs();
    let bits = 64 - magnitude.leading_zeros();
    bits <= precision || magnitude.trailing_zeros() >= bits - precision
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lp64_aliases_and_integer_loss_follow_compatibility_contract() {
        for kind in [4, 10, 11, 14, 15] {
            let value = SignedNumber(i64::MIN).convert(kind).unwrap();
            assert_eq!(value.bytes, i64::MIN.to_le_bytes());
            assert!(value.success);
        }
        let small = SignedNumber(255).convert(1).unwrap();
        assert_eq!(small.bytes, [255]);
        assert!(small.success);
        let lost = SignedNumber(256).convert(1).unwrap();
        assert_eq!(lost.bytes, [0]);
        assert!(!lost.success);
        assert!(!SignedNumber(-129).convert(7).unwrap().success);
        assert!(SignedNumber(65535).convert(8).unwrap().success);
        let lost = SignedNumber(i64::from(i32::MAX) + 1).convert(9).unwrap();
        assert_eq!(lost.bytes, i32::MIN.to_le_bytes());
        assert!(!lost.success);
        for kind in [0, 17, u64::MAX] {
            assert!(SignedNumber(1).convert(kind).is_err());
        }
    }
    #[test]
    fn float_conversion_reports_precision_loss_at_boundaries() {
        for kind in [5, 12] {
            assert!(SignedNumber(1 << 24).convert(kind).unwrap().success);
            assert!(!SignedNumber((1 << 24) + 1).convert(kind).unwrap().success);
            assert!(SignedNumber(i64::MIN).convert(kind).unwrap().success);
            assert!(!SignedNumber(i64::MAX).convert(kind).unwrap().success);
        }
        for kind in [6, 13, 16] {
            assert!(SignedNumber(1 << 53).convert(kind).unwrap().success);
            assert!(!SignedNumber((1 << 53) + 1).convert(kind).unwrap().success);
            assert!(
                !SignedNumber(-((1 << 53) + 1))
                    .convert(kind)
                    .unwrap()
                    .success
            );
            assert!(SignedNumber(0).convert(kind).unwrap().success);
            assert!(!SignedNumber(i64::MAX).convert(kind).unwrap().success);
        }
    }
}

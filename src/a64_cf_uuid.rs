/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! CF UUID/time value contracts needed by Terraria Unity and PlayFabParty.
//! No fabricated UUID entropy, CF object pointers or framework initialization.
use std::time::{SystemTime, UNIX_EPOCH};
const CF_EPOCH_UNIX_SECONDS: f64 = 978_307_200.0;

pub fn absolute_time(time: SystemTime) -> f64 {
    let unix = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    };
    unix - CF_EPOCH_UNIX_SECONDS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Value(pub [u8; 16]);
impl Value {
    /// Entropy must come from the caller's actual OS random source. Failure of
    /// that source must propagate, not substitute time/counters/zero bytes.
    pub fn version4(entropy: [u8; 16]) -> Self {
        let mut bytes = entropy;
        bytes[6] = (bytes[6] & 15) | 0x40;
        bytes[8] = (bytes[8] & 63) | 0x80;
        Self(bytes)
    }
    /// CFUUIDBytes is a 16-byte struct, returned in x0/x1 on Apple ARM64.
    pub fn registers(self) -> [u64; 2] {
        [
            u64::from_le_bytes(self.0[..8].try_into().unwrap()),
            u64::from_le_bytes(self.0[8..].try_into().unwrap()),
        ]
    }
    /// CFUUIDCreateFromUUIDBytes receives allocator x0 and the struct x1/x2.
    pub fn from_registers(registers: [u64; 2]) -> Self {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&registers[0].to_le_bytes());
        bytes[8..].copy_from_slice(&registers[1].to_le_bytes());
        Self(bytes)
    }
    pub fn string(self) -> String {
        let mut result = String::with_capacity(36);
        for (i, byte) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                result.push('-');
            }
            use std::fmt::Write;
            write!(&mut result, "{byte:02X}").unwrap();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn uuid_byte_order_format_and_version() {
        let value = Value(std::array::from_fn(|i| i as u8));
        assert_eq!(Value::from_registers(value.registers()), value);
        assert_eq!(value.string(), "00010203-0405-0607-0809-0A0B0C0D0E0F");
        let generated = Value::version4([0xff; 16]);
        assert_eq!(generated.0[6], 0x4f);
        assert_eq!(generated.0[8], 0xbf);
    }
    #[test]
    fn cf_time_epoch_fraction_and_before_unix() {
        assert_eq!(
            absolute_time(UNIX_EPOCH + Duration::from_secs(978_307_200)),
            0.0
        );
        assert_eq!(
            absolute_time(UNIX_EPOCH + Duration::new(978_307_201, 500_000_000)),
            1.5
        );
        assert_eq!(
            absolute_time(UNIX_EPOCH - Duration::from_secs(1)),
            -978_307_201.0
        );
    }
}

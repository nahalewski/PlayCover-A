/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Typed CFNumberGetValue adapter. The caller supplies the existing CF object's
//! authoritative lifetime/kind lookup; this module owns no competing arena or
//! reference counts. A foreign/cached/Objective-C identity must return Err.
use super::bridge::{GuestBridge, ReturnValues, ServiceId};
use super::cf_number::SignedNumber;
use super::A64Cpu;

pub(super) fn install(
    bridge: &mut GuestBridge,
    cpu: &mut A64Cpu,
    mut owned_integer: impl FnMut(u64) -> Result<SignedNumber, String> + 'static,
) -> Result<ServiceId, String> {
    bridge.register_service(cpu, "_CFNumberGetValue", move |frame| {
        let object = frame.integer(0)?;
        let kind = frame.integer(1)?;
        let output = frame.integer(2)?;
        if object == 0 {
            return Err("CFNumberGetValue requires a known live number".into());
        }
        let conversion = owned_integer(object)?.convert(kind)?;
        // CFNumberGetValue permits a null output: perform the conversion and
        // return its Boolean without writing. Non-null output is bounded and
        // validated atomically by ServiceFrame.write, even on loss (false).
        if output != 0 {
            frame.write(output, &conversion.bytes)?;
        }
        Ok(ReturnValues::integer(conversion.success as u64))
    })
}

#[cfg(test)]
mod tests {
    use super::super::bridge::GuestCall;
    use super::*;
    #[test]
    fn owned_integer_conversion_writes_real_bytes_and_rejects_foreign_identity() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        let mut bridge = GuestBridge::map(&mut cpu, 0x50000).unwrap();
        let entry = install(&mut bridge, &mut cpu, |object| match object {
            0x100000 => Ok(SignedNumber(1i64 << 32)),
            _ => Err("foreign or released CF identity".into()),
        })
        .unwrap()
        .guest_address();
        let call = |cpu: &mut A64Cpu, bridge: &mut GuestBridge, object, kind, output| {
            bridge.call(
                cpu,
                &GuestCall {
                    entry,
                    integers: vec![object, kind, output],
                    ..Default::default()
                },
                100,
            )
        };
        cpu.try_write_bytes(0x40000, &[0xa5; 16]).unwrap();
        assert_eq!(
            call(&mut cpu, &mut bridge, 0x100000, 9, 0x40004)
                .unwrap()
                .integers[0],
            0
        );
        let mut bytes = [0; 16];
        cpu.read_guest_into(0x40000, &mut bytes).unwrap();
        assert_eq!(
            bytes,
            [0xa5, 0xa5, 0xa5, 0xa5, 0, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5]
        );
        assert_eq!(
            call(&mut cpu, &mut bridge, 0x100000, 14, 0)
                .unwrap()
                .integers[0],
            1
        );
        assert!(call(&mut cpu, &mut bridge, 0x100008, 14, 0x40000).is_err());
        assert!(call(&mut cpu, &mut bridge, 0x100000, 17, 0x40000).is_err());
        assert!(call(&mut cpu, &mut bridge, 0x100000, 14, 0x40ffc).is_err());
        let mut after = [0; 16];
        cpu.read_guest_into(0x40000, &mut after).unwrap();
        assert_eq!(after, bytes);
        let mut tail = [1; 4];
        cpu.read_guest_into(0x40ffc, &mut tail).unwrap();
        assert_eq!(tail, [0; 4]);
    }
}

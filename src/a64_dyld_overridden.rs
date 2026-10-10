/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Implementation of _dyld_shared_cache_some_image_overridden.
//! Reports whether any image in the shared cache has been overridden.
//! In standard execution, returns false (0).
use super::{A64Cpu, bridge::{GuestBridge, ReturnValues}};

pub(super) const ENTRY: u64 = 0x1a6c7db04;
const ORIGINAL: [u8; 20] = [
    0x88, 0x85, 0x1d, 0xf0,
    0x00, 0xc5, 0x41, 0xf9,
    0x08, 0x00, 0x40, 0xf9,
    0x01, 0xd5, 0x40, 0xf9,
    0x20, 0x00, 0x1f, 0xd6,
];

pub(super) fn install(cpu: &mut A64Cpu, bridge: &mut GuestBridge, entry: u64) -> Result<u64, String> {
    if entry != ENTRY
        || cpu.mapped_permissions(entry).is_none_or(|p| p & 4 == 0)
        || cpu.read_bytes(entry, 20).ok_or("shared cache overridden wrapper is unreadable")? != ORIGINAL
    {
        return Err("original dyld shared cache overridden wrapper identity/instructions differ".into());
    }
    let target = bridge.register_service(cpu, "dyld_shared_cache_some_image_overridden", move |_frame| {
        echo!("[a64] dyld_shared_cache_some_image_overridden -> false");
        Ok(ReturnValues::integer(0))
    })?.guest_address();
    let mut patch = Vec::new();
    for i in 0..4u32 {
        let word = (if i == 0 { 0xd2800000 } else { 0xf2800000 })
            | (i << 21)
            | ((((target >> (i * 16)) & 0xffff) as u32) << 5)
            | 16;
        patch.extend_from_slice(&word.to_le_bytes());
    }
    patch.extend_from_slice(&0xd61f0200u32.to_le_bytes());
    cpu.try_write_bytes(entry, &patch)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dyld_shared_cache_some_image_overridden() {
        let mut cpu = A64Cpu::new_sparse();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        cpu.map_zeroed(ENTRY & !4095, 4096, 5).unwrap();
        cpu.try_write_bytes(ENTRY, &ORIGINAL).unwrap();

        let mut wrong = ORIGINAL;
        wrong[0] ^= 1;
        cpu.try_write_bytes(ENTRY, &wrong).unwrap();
        assert!(install(&mut cpu, &mut bridge, ENTRY).is_err());

        cpu.try_write_bytes(ENTRY, &ORIGINAL).unwrap();
        let target = install(&mut cpu, &mut bridge, ENTRY).unwrap();
        assert_ne!(target, 0);

        let result = bridge.call(
            &mut cpu,
            &super::super::bridge::GuestCall {
                entry: ENTRY,
                ..Default::default()
            },
            100,
        ).unwrap();
        assert_eq!(result.integers[0], 0);
    }
}

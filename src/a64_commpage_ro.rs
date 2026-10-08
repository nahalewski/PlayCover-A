/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Modern ARM64 read-only commpage fields required by libkernel mach_init.
//! ABI: local apple-xnu/osfmk/arm/cpu_capabilities.h. Actual iOS16 callers
//! at 0x1c2617898 and 0x1c26178d4 read offsets 0x37 and 0x25 respectively.
//! Other services (clocks, kernel ports and TLS) remain separate requirements.

use super::A64Cpu;

pub(super) const BASE: u64 = 0x0000_000f_ffff_4000;
pub(super) const LENGTH: usize = 4096;

fn fields() -> [u8; LENGTH] {
    let mut page = [0; LENGTH];
    // The guest sparse allocator and ordinary Mach-O linker use 16 KiB pages.
    page[0x25] = 14;
    page[0x37] = 14;
    page
}

pub(super) fn ensure_mapped(cpu: &mut A64Cpu) -> Result<(), String> {
    if cpu.read_bytes(BASE, 1).is_none() {
        cpu.map_zeroed(BASE, LENGTH, 1)?;
        cpu.try_write_bytes(BASE, &fields())?;
    }
    let mut existing = [0; LENGTH];
    cpu.read_guest_into(BASE, &mut existing)?;
    if existing != fields()
        || (0..LENGTH).any(|offset| cpu.validate_guest_write(BASE + offset as u64, 1).is_ok())
    {
        return Err("Modern read-only commpage does not match virtual page ABI".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_libkernel_fields_and_permissions() {
        let mut cpu = A64Cpu::new_sparse();
        ensure_mapped(&mut cpu).unwrap();
        ensure_mapped(&mut cpu).unwrap();
        let mut byte = [0];
        for address in [0xfffff4025, 0xfffff4037] {
            cpu.read_guest_into(address, &mut byte).unwrap();
            assert_eq!(byte[0], 14);
            assert!(cpu.write_guest_into(address, &[12]).is_err());
        }
        assert!(cpu.read_guest_into(BASE + LENGTH as u64, &mut byte).is_err());
    }
    #[test]
    fn foreign_or_corrupted_pages_fail() {
        let mut writable = A64Cpu::new_sparse();
        writable.map_zeroed(BASE, LENGTH, 3).unwrap();
        writable.try_write_bytes(BASE, &fields()).unwrap();
        assert!(ensure_mapped(&mut writable).is_err());
        let mut cpu = A64Cpu::new_sparse();
        ensure_mapped(&mut cpu).unwrap();
        cpu.try_write_bytes(BASE + 0x37, &[12]).unwrap();
        assert!(ensure_mapped(&mut cpu).is_err());
    }
}

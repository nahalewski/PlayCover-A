//! Minimal fixed ARM64 commpage fields for the audited iOS 11 resolver probe.
//! This does not establish a complete Darwin commpage or runtime environment.
//!
//! Exact ABI source: apple-oss-distributions/xnu tag xnu-4570.71.2,
//! osfmk/arm/cpu_capabilities.h, osfmk/arm/commpage/commpage.c and
//! osfmk/arm/pmap.c. pmap_create_sharedpage zeroes the page; ARM
//! commpage_populate does not write an ASCII signature into offset zero.
//! Hardware/approximate-clock flags remain disabled. Zero timestamp fields
//! are unavailable fast paths, not implemented clocks or valid timestamps.

use super::A64Cpu;

pub(super) const BASE: u64 = 0x0000_000f_ffff_c000;
pub(super) const LENGTH: usize = 4096;
pub(super) const CAPABILITIES_OFFSET: u64 = 0x20;
pub(super) const ARMV81_ATOMICS: u32 = 0x0200_0000;
// The virtual processor exposes baseline Advanced SIMD/VFP/FMA and one CPU.
// LL/SC is baseline AArch64, with no separate capability flag in this ABI.
// Do not advertise LSE, crypto, event wakeups or initialized fast TLS.
pub(super) const CAPABILITIES: u32 =
    0x0000_0100 | 0x0000_0400 | 0x0000_2000 | 0x0000_8000 | 0x0001_0000;

fn fixed_fields() -> [u8; LENGTH] {
    let mut page = [0u8; LENGTH];
    page[0x1e..0x20].copy_from_slice(&3u16.to_le_bytes());
    page[0x20..0x24].copy_from_slice(&CAPABILITIES.to_le_bytes());
    // The virtual allocator uses 16 KiB pages for anonymous guest mappings.
    page[0x25] = 14;
    page[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    page[0x34..0x37].fill(1); // active, physical and logical virtual CPUs
    page
}

pub(super) fn map(cpu: &mut A64Cpu) -> Result<(), String> {
    cpu.map_zeroed(BASE, LENGTH, 1)?;
    // Bounded host initialization bypasses guest writes; guest mapping stays R.
    cpu.try_write_bytes(BASE, &fixed_fields())
}

/// Reuse only the exact initialized, read-only virtual commpage. An existing
/// foreign mapping is an error, rather than permission to advertise host CPUs.
pub(super) fn ensure_mapped(cpu: &mut A64Cpu) -> Result<(), String> {
    if cpu.read_bytes(BASE, 1).is_none() {
        return map(cpu);
    }
    let mut existing = [0u8; LENGTH];
    cpu.read_guest_into(BASE, &mut existing)?;
    if existing != fixed_fields()
        || (0..LENGTH).any(|offset| cpu.validate_guest_write(BASE + offset as u64, 1).is_ok())
    {
        return Err("existing commpage differs from the initialized virtual ABI".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_legacy_layout_does_not_advertise_lse_or_clock_fast_paths() {
        let page = fixed_fields();
        assert_eq!(BASE, 0xfffffc000);
        assert_eq!(&page[..16], &[0; 16]);
        assert_eq!(u16::from_le_bytes(page[0x1e..0x20].try_into().unwrap()), 3);
        let caps = u32::from_le_bytes(page[0x20..0x24].try_into().unwrap());
        assert_eq!(caps, 0x1a500);
        assert_eq!(caps & ARMV81_ATOMICS, 0);
        assert_eq!(page[0x22], 1); // NCPUS overlaps packed capability word
        assert_eq!(page[0x90], 0); // USER_TIMEBASE unsupported
        assert_eq!(page[0x91], 0); // CONT_HWCLOCK unsupported
        assert_eq!(page[0xc8], 0); // APPROX_TIME_SUPPORTED disabled
    }

    #[test]
    fn reuse_requires_exact_readonly_fields() {
        let mut cpu = A64Cpu::new_sparse();
        ensure_mapped(&mut cpu).unwrap();
        ensure_mapped(&mut cpu).unwrap();
        cpu.try_write_bytes(BASE + CAPABILITIES_OFFSET, &ARMV81_ATOMICS.to_le_bytes())
            .unwrap();
        assert!(ensure_mapped(&mut cpu).unwrap_err().contains("differs"));
        let mut writable = A64Cpu::new_sparse();
        writable.map_zeroed(BASE, LENGTH, 3).unwrap();
        writable.try_write_bytes(BASE, &fixed_fields()).unwrap();
        assert!(ensure_mapped(&mut writable).is_err());
    }
    #[test]
    fn guest_can_read_but_cannot_write_commpage() {
        let mut cpu = A64Cpu::new_sparse();
        map(&mut cpu).unwrap();
        let mut caps = [0u8; 4];
        cpu.read_guest_into(BASE + CAPABILITIES_OFFSET, &mut caps)
            .unwrap();
        assert_eq!(u32::from_le_bytes(caps), CAPABILITIES);
        assert!(cpu
            .write_guest_into(BASE + CAPABILITIES_OFFSET, &[0xff])
            .is_err());
        assert!(cpu.validate_guest_write(BASE, LENGTH).is_err());
        assert!(cpu
            .read_guest_into(BASE + LENGTH as u64, &mut caps)
            .is_err());
        assert!(map(&mut cpu).is_err()); // duplicate mappings fail closed
    }
}

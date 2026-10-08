/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Darwin BSD syscall 74 `mprotect(addr, len, prot)` over real guest mappings.
//!
//! Primary sources (local `runtime-sources/apple-xnu`):
//! - `bsd/kern/kern_mman.c:1152-1314` (`mprotect_sanitize`, `mprotect`):
//!   unaligned start is EINVAL, size zero falls through, W or X implies R,
//!   KERN_PROTECTION_FAILURE -> EACCES, KERN_INVALID_ADDRESS -> ENOMEM.
//! - `osfmk/vm/vm_sanitize.c:450-549` (`vm_sanitize_addr_size`): the alignment
//!   check precedes the size-zero check; `addr + len` overflow and a rounded
//!   end that wraps are KERN_INVALID_ARGUMENT (EINVAL).
//! - `osfmk/vm/vm_user.c:296-316` (`mach_vm_protect`): size zero succeeds
//!   before any map lookup, so an aligned, zero-length request on unmapped
//!   memory returns 0 on real XNU. That is reproduced exactly, not faked.
//! - `osfmk/vm/vm_map.c:5799-6144` (`vm_map_protect`): one validation pass over
//!   every entry (start unmapped or hole -> ENOMEM; `(prot & max) != prot` ->
//!   EACCES; write+execute on a non-JIT entry with VM_MAP_POLICY_WX_FAIL ->
//!   EACCES) before any entry is clipped or changed.
//! - `osfmk/ipc/mach_kernelrpc.c:122-124`: anonymous `mach_vm_map` memory has
//!   maximum protection VM_PROT_ALL.
//!
//! Callers: libmalloc `mprotect(zone, 0x98, PROT_READ)` at 0x19455cdc0 and
//! 0x19455bed4, 0x19455c050, 0x19455c418, 0x19455c4c8 (libSystem audit).
//!
//! Deviation, per the task specification (the coordinator should confirm):
//! XNU silently masks protection bits outside
//! `VM_PROT_ALL | VM_PROT_TRUSTED | VM_PROT_STRIP_READ`
//! (`vm_sanitize_prot_bsd`, `vm_sanitize.c:768-779`); VM_PROT_TRUSTED takes
//! the code-signing branch, ENOTSUP without dynamic code signing
//! (`kern_mman.c:1274-1300`); VM_PROT_STRIP_READ is rejected as
//! KERN_INVALID_ARGUMENT -> EINVAL by `vm_map_protect_sanitize`
//! (`vm_map.c:5775`, extra mask VM_PROT_COPY), only once the size is non-zero.
//! This emulator reports EINVAL for any bit outside VM_PROT_ALL instead,
//! before the size-zero check.
//!
//! The guest-memory backend is abstracted by [`ProtectionBackend`]; the
//! A64Cpu adapter changes both native execution and host copy permissions,
//! preserving backing bytes and invalidating translated code.

use super::A64Cpu;

/// BSD syscall number (`svc #0x80`, x16 = 74).
pub(super) const SYS_MPROTECT: u64 = 74;
/// arm64 iOS user page size.
pub(super) const PAGE_SIZE: u64 = 0x4000;
const PAGE_MASK: u64 = PAGE_SIZE - 1;

pub(super) const VM_PROT_READ: u32 = 1;
pub(super) const VM_PROT_WRITE: u32 = 2;
pub(super) const VM_PROT_EXECUTE: u32 = 4;
pub(super) const VM_PROT_ALL: u32 = VM_PROT_READ | VM_PROT_WRITE | VM_PROT_EXECUTE;

pub(super) const ENOMEM: u64 = 12;
pub(super) const EACCES: u64 = 13;
pub(super) const EINVAL: u64 = 22;

/// Upper bound on distinct protection extents changed by one call. Real
/// guest requests touch a few pages; this only bounds host allocation.
const MAX_EXTENTS: usize = 1 << 16;

const CARRY: u32 = 1 << 29;

/// One mapped extent of uniform current and maximum protection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RegionView {
    pub base: u64,
    pub len: u64,
    /// Current protection, VM_PROT_* bits.
    pub protection: u32,
    /// Maximum protection fixed when the memory was mapped, VM_PROT_* bits.
    pub max_protection: u32,
}

/// Guest-memory operations mprotect needs, implemented by the real A64Cpu.
///
/// Required semantics for a real implementation:
/// - `region_containing(addr)` returns the extent containing `addr`, with
///   `base <= addr < base + len`, `len > 0` and no overflow; `None` when
///   `addr` is not mapped. Extents need not be maximal (adjacent extents are
///   treated as contiguous, like adjacent XNU map entries).
/// - `set_protection(base, len, prot)` changes the current protection of
///   exactly `[base, base + len)`, splitting extents at both ends while
///   keeping each byte's maximum protection and backing bytes. It must update
///   both the guest fault path (C++ `A64Environment::validate`) and host
///   copyin/copyout checks (`read_guest_into` / `validate_guest_write`), and
///   invalidate translated code for the range (`InvalidateCacheRange`), since
///   dynarmic checks execute permission only at translation time. It is only
///   called for ranges this module has already validated as fully mapped
///   with `prot` within the maximum. On error it must leave the range
///   unchanged.
pub(super) trait ProtectionBackend {
    fn region_containing(&self, addr: u64) -> Option<RegionView>;
    fn set_protection(&mut self, base: u64, len: u64, protection: u32) -> Result<(), String>;
}

impl ProtectionBackend for A64Cpu {
    fn region_containing(&self,addr:u64)->Option<RegionView> {
        self.protection_region(addr).map(|region|RegionView {
            base:region.base,len:region.len,protection:region.protection,max_protection:region.max_protection,
        })
    }
    fn set_protection(&mut self,base:u64,len:u64,protection:u32)->Result<(),String> {
        A64Cpu::set_protection(self,base,len,protection)
    }
}

/// Read the syscall arguments: x0 = addr, x1 = len (both 64-bit), x2 = prot
/// (C `int`; only the low 32 bits are defined by AAPCS64).
pub(super) fn decode_args(cpu: &A64Cpu) -> (u64, u64, u32) {
    (cpu.reg(0), cpu.reg(1), cpu.reg(2) as u32)
}

/// Complete a BSD syscall: success writes x0 = 0 and clears carry; failure
/// writes the positive errno to x0 and sets carry. Other NZCV bits are kept.
pub(super) fn complete_bsd(cpu: &mut A64Cpu, errno: u64) {
    cpu.set_reg(0, errno);
    let pstate = cpu.pstate();
    cpu.set_pstate(if errno != 0 { pstate | CARRY } else { pstate & !CARRY });
}

/// Darwin mprotect. `Ok(0)` is success and `Ok(errno)` a guest-visible
/// failure; in both error cases nothing has been changed. `Err` means the
/// backend broke its contract or failed while applying an already validated
/// change (any applied pieces are rolled back first); it is a host fault and
/// must stop emulation rather than be reported to the guest.
pub(super) fn mprotect<B: ProtectionBackend + ?Sized>(
    backend: &mut B,
    addr: u64,
    len: u64,
    prot: u32,
) -> Result<u64, String> {
    // vm_sanitize_addr_size: alignment is checked before size zero.
    if addr & PAGE_MASK != 0 {
        return Ok(EINVAL);
    }
    // Deviation from vm_sanitize_prot_bsd masking; see the module docs.
    if prot & !VM_PROT_ALL != 0 {
        return Ok(EINVAL);
    }
    // mach_vm_protect: size zero succeeds without any lookup.
    if len == 0 {
        return Ok(0);
    }
    let Some(end_unaligned) = addr.checked_add(len) else {
        return Ok(EINVAL);
    };
    let Some(end) = end_unaligned.checked_add(PAGE_MASK).map(|value| value & !PAGE_MASK) else {
        return Ok(EINVAL);
    };
    if end <= addr {
        return Ok(EINVAL);
    }
    // kern_mman.c "#if 3936456": write or execute implies read.
    let mut prot = prot;
    if prot & (VM_PROT_EXECUTE | VM_PROT_WRITE) != 0 {
        prot |= VM_PROT_READ;
    }

    // Validation pass. Nothing is changed until every extent passed.
    let mut plan: Vec<(u64, u64, u32)> = Vec::new();
    let mut cursor = addr;
    let mut first = true;
    while cursor < end {
        let Some(view) = backend.region_containing(cursor) else {
            return Ok(ENOMEM);
        };
        let view_end = view
            .base
            .checked_add(view.len)
            .ok_or("mprotect backend extent overflows")?;
        if view.len == 0 || view.base > cursor || view_end <= cursor {
            return Err(format!(
                "mprotect backend extent {:#x}+{:#x} does not contain {cursor:#x}",
                view.base, view.len
            ));
        }
        if view.protection & !VM_PROT_ALL != 0 || view.max_protection & !VM_PROT_ALL != 0 {
            return Err("mprotect backend reported invalid protection bits".into());
        }
        if prot & view.max_protection != prot {
            return Ok(EACCES);
        }
        // vm_map_protect: W+X on a non-JIT entry fails (VM_MAP_POLICY_WX_FAIL).
        // No MAP_JIT entries exist in this emulator.
        if first && prot & VM_PROT_WRITE != 0 && prot & VM_PROT_EXECUTE != 0 {
            return Ok(EACCES);
        }
        first = false;
        let piece_end = view_end.min(end);
        if plan.len() >= MAX_EXTENTS {
            return Err("mprotect range spans too many extents".into());
        }
        plan.try_reserve(1)
            .map_err(|_| "mprotect plan allocation failed")?;
        plan.push((cursor, piece_end, view.protection));
        cursor = piece_end;
    }

    // Apply pass, rolled back on a backend failure.
    for (index, &(start, piece_end, _)) in plan.iter().enumerate() {
        if let Err(error) = backend.set_protection(start, piece_end - start, prot) {
            for &(old_start, old_end, old) in plan[..index].iter().rev() {
                if let Err(rollback) = backend.set_protection(old_start, old_end - old_start, old) {
                    return Err(format!(
                        "mprotect apply failed ({error}); rollback also failed ({rollback})"
                    ));
                }
            }
            return Err(format!("mprotect apply failed and was rolled back: {error}"));
        }
    }
    Ok(0)
}

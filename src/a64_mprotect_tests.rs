/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Regression coverage for Darwin mprotect (BSD 74). The model backend below
//! enforces protections on its own read/write path, so a test that sees a
//! write fault proves the protection change, not only bookkeeping. The real
//! real A64Cpu test also exercises native execution permission faults.
use super::mprotect::{
    complete_bsd, decode_args, mprotect, ProtectionBackend, RegionView, EACCES, EINVAL, ENOMEM,
    PAGE_SIZE, VM_PROT_ALL, VM_PROT_EXECUTE, VM_PROT_READ, VM_PROT_WRITE,
};
use super::{A64Cpu, A64State};
use std::collections::HashMap;

const RW: u32 = VM_PROT_READ | VM_PROT_WRITE;
const BASE: u64 = 0x20_0000_0000;

/// Guest memory model: sorted, non-overlapping extents, never coalesced.
#[derive(Default)]
struct Model {
    extents: Vec<RegionView>,
    bytes: HashMap<u64, u8>,
    applies: usize,
    fail_on_apply: Option<usize>,
}

impl Model {
    fn map(&mut self, base: u64, len: u64, protection: u32, max_protection: u32) {
        assert!(len > 0 && protection & !max_protection == 0);
        let end = base + len;
        assert!(self.extents.iter().all(|e| end <= e.base || e.base + e.len <= base));
        let at = self.extents.partition_point(|e| e.base < base);
        self.extents.insert(at, RegionView { base, len, protection, max_protection });
    }

    fn protection_at(&self, addr: u64) -> Option<u32> {
        self.region_containing(addr).map(|view| view.protection)
    }

    /// Guest store: validates every byte before writing any (like the CPU).
    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), u64> {
        for offset in 0..data.len() as u64 {
            let address = addr + offset;
            match self.protection_at(address) {
                Some(protection) if protection & VM_PROT_WRITE != 0 => {}
                _ => return Err(address),
            }
        }
        for (offset, &byte) in data.iter().enumerate() {
            self.bytes.insert(addr + offset as u64, byte);
        }
        Ok(())
    }

    fn read(&self, addr: u64, len: u64) -> Result<Vec<u8>, u64> {
        (addr..addr + len)
            .map(|address| match self.protection_at(address) {
                Some(protection) if protection & VM_PROT_READ != 0 => {
                    Ok(*self.bytes.get(&address).unwrap_or(&0))
                }
                _ => Err(address),
            })
            .collect()
    }

    fn split_at(&mut self, point: u64) -> Result<(), String> {
        if let Some(index) = self
            .extents
            .iter()
            .position(|e| e.base < point && point < e.base + e.len)
        {
            let view = self.extents[index];
            self.extents[index].len = point - view.base;
            self.extents.insert(
                index + 1,
                RegionView { base: point, len: view.base + view.len - point, ..view },
            );
        }
        Ok(())
    }
}

impl ProtectionBackend for Model {
    fn region_containing(&self, addr: u64) -> Option<RegionView> {
        self.extents
            .iter()
            .copied()
            .find(|e| e.base <= addr && addr - e.base < e.len)
    }

    fn set_protection(&mut self, base: u64, len: u64, protection: u32) -> Result<(), String> {
        // One-shot injected failure, so the rollback calls that follow succeed.
        if self.fail_on_apply == Some(self.applies) {
            self.fail_on_apply = None;
            return Err("injected backend failure".into());
        }
        self.applies += 1;
        let end = base + len;
        // Verify complete coverage and maximum before mutating anything.
        let mut cursor = base;
        while cursor < end {
            let view = self.region_containing(cursor).ok_or("model gap")?;
            if protection & !view.max_protection != 0 {
                return Err("model maximum exceeded".into());
            }
            cursor = view.base + view.len;
        }
        self.split_at(base)?;
        self.split_at(end)?;
        for extent in &mut self.extents {
            if extent.base >= base && extent.base + extent.len <= end {
                extent.protection = protection;
            }
        }
        Ok(())
    }
}

fn three_rw_pages() -> Model {
    let mut model = Model::default();
    model.map(BASE, 3 * PAGE_SIZE, RW, VM_PROT_ALL);
    model
}

#[test]
fn protected_write_faults_and_restoring_write_permission_allows_it() {
    let mut model = three_rw_pages();
    model.write(BASE + 8, &[1, 2, 3, 4]).unwrap();
    assert_eq!(mprotect(&mut model, BASE, PAGE_SIZE, VM_PROT_READ).unwrap(), 0);
    assert_eq!(model.write(BASE + 8, &[9]), Err(BASE + 8));
    assert_eq!(model.read(BASE + 8, 4).unwrap(), vec![1, 2, 3, 4]);
    // The following page keeps its write permission.
    model.write(BASE + PAGE_SIZE, &[5]).unwrap();
    assert_eq!(mprotect(&mut model, BASE, PAGE_SIZE, RW).unwrap(), 0);
    model.write(BASE + 8, &[9]).unwrap();
    assert_eq!(model.read(BASE + 8, 1).unwrap(), vec![9]);
}

#[test]
fn prot_none_makes_reads_and_writes_fault() {
    let mut model = three_rw_pages();
    assert_eq!(mprotect(&mut model, BASE, PAGE_SIZE, 0).unwrap(), 0);
    assert_eq!(model.read(BASE, 1), Err(BASE));
    assert_eq!(model.write(BASE, &[1]), Err(BASE));
    assert_eq!(model.read(BASE + PAGE_SIZE, 1).unwrap(), vec![0]);
}

#[test]
fn length_rounds_up_to_one_16k_page() {
    let mut model = three_rw_pages();
    // Actual libmalloc request: mprotect(zone, 0x98, PROT_READ).
    assert_eq!(mprotect(&mut model, BASE, 0x98, VM_PROT_READ).unwrap(), 0);
    assert_eq!(model.protection_at(BASE), Some(VM_PROT_READ));
    assert_eq!(model.protection_at(BASE + PAGE_SIZE - 1), Some(VM_PROT_READ));
    assert_eq!(model.protection_at(BASE + PAGE_SIZE), Some(RW));
    assert_eq!(model.write(BASE + PAGE_SIZE - 1, &[1]), Err(BASE + PAGE_SIZE - 1));
    model.write(BASE + PAGE_SIZE, &[1]).unwrap();
    // A length one byte past a page boundary covers the second page as well.
    let mut model = three_rw_pages();
    assert_eq!(mprotect(&mut model, BASE, PAGE_SIZE + 1, VM_PROT_READ).unwrap(), 0);
    assert_eq!(model.protection_at(BASE + 2 * PAGE_SIZE - 1), Some(VM_PROT_READ));
    assert_eq!(model.protection_at(BASE + 2 * PAGE_SIZE), Some(RW));
}

#[test]
fn middle_page_protection_splits_one_region_into_three() {
    let mut model = three_rw_pages();
    assert_eq!(mprotect(&mut model, BASE + PAGE_SIZE, PAGE_SIZE, VM_PROT_READ).unwrap(), 0);
    assert_eq!(
        model.extents,
        vec![
            RegionView { base: BASE, len: PAGE_SIZE, protection: RW, max_protection: VM_PROT_ALL },
            RegionView { base: BASE + PAGE_SIZE, len: PAGE_SIZE, protection: VM_PROT_READ, max_protection: VM_PROT_ALL },
            RegionView { base: BASE + 2 * PAGE_SIZE, len: PAGE_SIZE, protection: RW, max_protection: VM_PROT_ALL },
        ]
    );
    model.write(BASE + PAGE_SIZE - 1, &[1]).unwrap();
    assert_eq!(model.write(BASE + PAGE_SIZE, &[1]), Err(BASE + PAGE_SIZE));
    model.write(BASE + 2 * PAGE_SIZE, &[1]).unwrap();
}

#[test]
fn unaligned_address_is_einval_and_changes_nothing() {
    let mut model = three_rw_pages();
    for addr in [BASE + 1, BASE + 0x1000, BASE + PAGE_SIZE - 1] {
        assert_eq!(mprotect(&mut model, addr, PAGE_SIZE, VM_PROT_READ).unwrap(), EINVAL);
        // Alignment is checked before the zero-length shortcut.
        assert_eq!(mprotect(&mut model, addr, 0, VM_PROT_READ).unwrap(), EINVAL);
    }
    assert_eq!(model.applies, 0);
    model.write(BASE, &[1]).unwrap();
}

#[test]
fn unmapped_range_is_enomem_without_partial_change() {
    // Page 0 mapped, page 1 a hole, page 2 mapped.
    let mut model = Model::default();
    model.map(BASE, PAGE_SIZE, RW, VM_PROT_ALL);
    model.map(BASE + 2 * PAGE_SIZE, PAGE_SIZE, RW, VM_PROT_ALL);
    assert_eq!(mprotect(&mut model, BASE, 3 * PAGE_SIZE, VM_PROT_READ).unwrap(), ENOMEM);
    // Start in the hole.
    assert_eq!(mprotect(&mut model, BASE + PAGE_SIZE, PAGE_SIZE, VM_PROT_READ).unwrap(), ENOMEM);
    // Range running past the last mapping.
    assert_eq!(mprotect(&mut model, BASE + 2 * PAGE_SIZE, 2 * PAGE_SIZE, VM_PROT_READ).unwrap(), ENOMEM);
    // Wholly unmapped.
    assert_eq!(mprotect(&mut model, 0x4000, PAGE_SIZE, VM_PROT_READ).unwrap(), ENOMEM);
    assert_eq!(model.applies, 0);
    model.write(BASE, &[1]).unwrap();
    model.write(BASE + 2 * PAGE_SIZE, &[1]).unwrap();
}

#[test]
fn maximum_protection_violation_is_eacces_without_partial_change() {
    let mut model = Model::default();
    model.map(BASE, PAGE_SIZE, RW, VM_PROT_ALL);
    model.map(BASE + PAGE_SIZE, PAGE_SIZE, VM_PROT_READ, VM_PROT_READ);
    assert_eq!(mprotect(&mut model, BASE, 2 * PAGE_SIZE, RW).unwrap(), EACCES);
    assert_eq!(mprotect(&mut model, BASE + PAGE_SIZE, PAGE_SIZE, VM_PROT_EXECUTE).unwrap(), EACCES);
    assert_eq!(model.applies, 0);
    assert_eq!(model.protection_at(BASE), Some(RW));
    assert_eq!(model.protection_at(BASE + PAGE_SIZE), Some(VM_PROT_READ));
    // Lowering within the maximum is allowed for both pages.
    assert_eq!(mprotect(&mut model, BASE, 2 * PAGE_SIZE, VM_PROT_READ).unwrap(), 0);
    assert_eq!(model.write(BASE, &[1]), Err(BASE));
}

#[test]
fn write_implies_read_including_against_the_maximum() {
    let mut model = three_rw_pages();
    assert_eq!(mprotect(&mut model, BASE, PAGE_SIZE, VM_PROT_WRITE).unwrap(), 0);
    assert_eq!(model.protection_at(BASE), Some(RW));
    // Write-only maximum: the implied read makes the request exceed it.
    let mut write_only = Model::default();
    write_only.map(BASE, PAGE_SIZE, VM_PROT_WRITE, VM_PROT_WRITE);
    assert_eq!(mprotect(&mut write_only, BASE, PAGE_SIZE, VM_PROT_WRITE).unwrap(), EACCES);
    assert_eq!(write_only.applies, 0);
}

#[test]
fn write_and_execute_together_is_eacces() {
    let mut model = three_rw_pages();
    for prot in [VM_PROT_WRITE | VM_PROT_EXECUTE, VM_PROT_ALL] {
        assert_eq!(mprotect(&mut model, BASE, PAGE_SIZE, prot).unwrap(), EACCES);
    }
    assert_eq!(model.applies, 0);
    // Read+execute alone is permitted under an RWX maximum.
    assert_eq!(mprotect(&mut model, BASE, PAGE_SIZE, VM_PROT_EXECUTE).unwrap(), 0);
    assert_eq!(model.protection_at(BASE), Some(VM_PROT_READ | VM_PROT_EXECUTE));
}

#[test]
fn invalid_protection_bits_are_einval() {
    let mut model = three_rw_pages();
    // 0x10 VM_PROT_COPY, 0x20 VM_PROT_TRUSTED, 0x80 VM_PROT_STRIP_READ.
    for prot in [8, 0x10, 0x20, 0x40, 0x80, 0x100, 0x8000_0000, u32::MAX] {
        assert_eq!(mprotect(&mut model, BASE, PAGE_SIZE, prot).unwrap(), EINVAL);
        assert_eq!(mprotect(&mut model, BASE, 0, prot).unwrap(), EINVAL);
    }
    assert_eq!(model.applies, 0);
}

#[test]
fn zero_length_succeeds_without_lookup_or_change() {
    let mut model = three_rw_pages();
    assert_eq!(mprotect(&mut model, BASE, 0, VM_PROT_READ).unwrap(), 0);
    // XNU mach_vm_protect returns success for size zero before any lookup.
    assert_eq!(mprotect(&mut model, 0x40_0000_0000, 0, VM_PROT_READ).unwrap(), 0);
    assert_eq!(model.applies, 0);
    model.write(BASE, &[1]).unwrap();
}

#[test]
fn overflowing_ranges_are_einval() {
    let mut model = Model::default();
    let last_page = u64::MAX & !(PAGE_SIZE - 1);
    model.map(last_page, PAGE_SIZE - 1, RW, VM_PROT_ALL);
    // addr + len overflows.
    assert_eq!(mprotect(&mut model, last_page, PAGE_SIZE, VM_PROT_READ).unwrap(), EINVAL);
    assert_eq!(mprotect(&mut model, BASE, u64::MAX, VM_PROT_READ).unwrap(), EINVAL);
    // addr + len fits, but rounding the end up to a page wraps.
    assert_eq!(mprotect(&mut model, last_page, 1, VM_PROT_READ).unwrap(), EINVAL);
    assert_eq!(model.applies, 0);
}

#[test]
fn adjacent_regions_are_one_contiguous_range() {
    let mut model = Model::default();
    model.map(BASE, PAGE_SIZE, RW, VM_PROT_ALL);
    model.map(BASE + PAGE_SIZE, PAGE_SIZE, RW, RW);
    assert_eq!(mprotect(&mut model, BASE, 2 * PAGE_SIZE, VM_PROT_READ).unwrap(), 0);
    assert_eq!(model.write(BASE, &[1]), Err(BASE));
    assert_eq!(model.write(BASE + PAGE_SIZE, &[1]), Err(BASE + PAGE_SIZE));
    assert_eq!(model.extents[1].max_protection, RW);
}

#[test]
fn backend_failure_is_a_host_error_and_rolls_back_applied_pieces() {
    let mut model = Model::default();
    model.map(BASE, PAGE_SIZE, RW, VM_PROT_ALL);
    model.map(BASE + PAGE_SIZE, PAGE_SIZE, VM_PROT_READ, VM_PROT_ALL);
    model.fail_on_apply = Some(1);
    assert!(mprotect(&mut model, BASE, 2 * PAGE_SIZE, 0).is_err());
    assert_eq!(model.protection_at(BASE), Some(RW));
    assert_eq!(model.protection_at(BASE + PAGE_SIZE), Some(VM_PROT_READ));
    model.write(BASE, &[1]).unwrap();
}

struct BrokenBackend;
impl ProtectionBackend for BrokenBackend {
    fn region_containing(&self, _addr: u64) -> Option<RegionView> {
        // Does not contain the queried address; must not loop or succeed.
        Some(RegionView { base: 0, len: PAGE_SIZE, protection: RW, max_protection: VM_PROT_ALL })
    }
    fn set_protection(&mut self, _base: u64, _len: u64, _prot: u32) -> Result<(), String> {
        panic!("set_protection must not be reached for an invalid extent");
    }
}

#[test]
fn backend_contract_violation_is_a_host_error() {
    assert!(mprotect(&mut BrokenBackend, BASE, PAGE_SIZE, VM_PROT_READ).is_err());
}

#[test]
fn bsd_completion_sets_errno_and_carry_preserving_other_flags() {
    let mut cpu = A64Cpu::new_sparse();
    cpu.set_reg(0, BASE + 1);
    cpu.set_reg(1, 0x98);
    cpu.set_reg(2, 0xdead_beef_0000_0001);
    // Only the low 32 bits of the C int prot argument are used.
    assert_eq!(decode_args(&cpu), (BASE + 1, 0x98, 1));
    cpu.set_pstate(0x9000_0000);
    complete_bsd(&mut cpu, EINVAL);
    assert_eq!(cpu.reg(0), EINVAL);
    assert_eq!(cpu.pstate() & 0xf000_0000, 0xb000_0000);
    complete_bsd(&mut cpu, 0);
    assert_eq!(cpu.reg(0), 0);
    assert_eq!(cpu.pstate() & 0xf000_0000, 0x9000_0000);
}

/// The real backend clips permissions and invalidates translated code.
fn backend_protect(cpu: &mut A64Cpu, base: u64, len: u64, prot: u32) -> Result<(), String> {
    cpu.set_protection(base,len,prot)
}

fn guest_store(cpu: &mut A64Cpu, code: u64, address: u64, value: u64) -> A64State {
    cpu.set_reg(0, value);
    cpu.set_reg(1, address);
    cpu.set_pc(code);
    let mut ticks = 100;
    cpu.run_or_step(Some(&mut ticks))
}

#[test]
fn real_a64_backend_protected_store_faults() {
    let mut cpu = A64Cpu::new_sparse();
    let code = 0x1_0000_0000;
    cpu.map_zeroed(code, PAGE_SIZE as usize, A64Cpu::READ | A64Cpu::EXECUTE).unwrap();
    let mut program = 0xf900_0020u32.to_le_bytes().to_vec(); // str x0,[x1]
    program.extend_from_slice(&0xd400_1001u32.to_le_bytes()); // svc #0x80
    cpu.write_bytes(code, &program);
    cpu.map_zeroed(BASE, 2 * PAGE_SIZE as usize, A64Cpu::READ | A64Cpu::WRITE).unwrap();

    assert_eq!(guest_store(&mut cpu, code, BASE, 7), A64State::Svc(0x80));
    assert_eq!(cpu.read_u64(BASE), Some(7));

    backend_protect(&mut cpu, BASE, PAGE_SIZE, A64Cpu::READ).unwrap();
    assert_eq!(guest_store(&mut cpu, code, BASE, 9), A64State::MemoryError(BASE));
    assert_eq!(cpu.read_u64(BASE), Some(7));
    assert!(cpu.write_guest_into(BASE, &[1]).is_err());
    let mut byte = [0];
    cpu.read_guest_into(BASE, &mut byte).unwrap();
    // The neighbouring page is unaffected.
    assert_eq!(guest_store(&mut cpu, code, BASE + PAGE_SIZE, 5), A64State::Svc(0x80));

    backend_protect(&mut cpu, BASE, PAGE_SIZE, A64Cpu::READ | A64Cpu::WRITE).unwrap();
    assert_eq!(guest_store(&mut cpu, code, BASE, 9), A64State::Svc(0x80));
    assert_eq!(cpu.read_u64(BASE), Some(9));

    // Removing execute must also stop already translated code.
    backend_protect(&mut cpu, code, PAGE_SIZE, A64Cpu::READ).unwrap();
    assert_eq!(guest_store(&mut cpu, code, BASE, 1), A64State::MemoryError(code));
}

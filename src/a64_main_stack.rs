/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Validated startup context from the loader's actual main-stack mapping.
//! Never infer stack bounds from callback SP or adjacent writable mappings.
//! The descriptor identifies an emulator CPU owner, not completed Darwin
//! pthread initialization. Caller retains the exact descriptor at map creation.
use super::A64Cpu;

const PAGE: u64 = 0x4000;
const MAX_STACK: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct MainStackDescriptor {
    owner: u64,
    stack_base: u64,
    stack_size: u64,
    allocation_base: u64,
    allocation_size: u64,
    initial_sp: u64,
}

impl MainStackDescriptor {
    pub fn from_loader(
        cpu: &A64Cpu,
        owner: u64,
        stack_base: u64,
        stack_size: u64,
        allocation_base: u64,
        allocation_size: u64,
        initial_sp: u64,
    ) -> Result<Self, String> {
        if owner == 0 || stack_base == 0 || stack_size == 0 || stack_size > MAX_STACK
            || allocation_size == 0 || allocation_size > MAX_STACK
            || stack_base % PAGE != 0 || stack_size % PAGE != 0
            || allocation_base % PAGE != 0 || allocation_size % PAGE != 0
            || initial_sp & 15 != 0
        {
            return Err("Invalid loader main-stack descriptor".into());
        }
        let stack_top = stack_base.checked_add(stack_size).ok_or("Main-stack range overflow")?;
        let allocation_top = allocation_base.checked_add(allocation_size).ok_or("Main-stack allocation overflow")?;
        if stack_top > (1 << 48) || allocation_top > (1 << 48)
            || allocation_base > stack_base || allocation_top < stack_top
            || initial_sp < stack_base || initial_sp >= stack_top
        {
            return Err("Main-stack bounds/owner SP do not match loader allocation".into());
        }
        // Validate every byte through the CPU's bounded whole-range permission
        // checker; start/end samples would miss holes or read-only middle pages.
        cpu.validate_guest_write(stack_base, stack_size as usize)?;
        // The initializer may read stack-local structures; RW is required.
        // Read check walks the entire range without copying guest stack data.
        let mut address = stack_base;
        while address < stack_top {
            if cpu.mapped_permissions(address) != Some(3) {
                return Err("Loader main-stack mapping does not have exact RW/NX permissions".into());
            }
            // Loader descriptors are page-aligned, but the sparse CPU can have
            // byte-sized mappings. Inspect all addresses, rather than silently
            // accepting an unreadable partial-page region.
            address += 1;
        }
        Ok(Self { owner, stack_base, stack_size, allocation_base, allocation_size, initial_sp })
    }

    pub fn owner(&self) -> u64 { self.owner }
    pub fn initial_sp(&self) -> u64 { self.initial_sp }
    pub fn bounds(&self) -> (u64, u64) { (self.stack_base, self.stack_base + self.stack_size) }

    /// Place this string in writable guest memory: pthread initialization wipes
    /// the value after parsing. `main_stack` numeric fields use hexadecimal.
    pub fn apple_entry(&self) -> String {
        format!("main_stack={:#x},{:#x},{:#x},{:#x}", self.stack_base + self.stack_size,
                self.stack_size, self.allocation_base, self.allocation_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_loader_range_formats_real_stack_context() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 0x100000, 3).unwrap();
        let stack = MainStackDescriptor::from_loader(&cpu, 1, 0x10000, 0x100000, 0x10000, 0x100000, 0x10ff00).unwrap();
        assert_eq!(stack.owner(), 1);
        assert_eq!(stack.initial_sp(), 0x10ff00);
        assert_eq!(stack.bounds(), (0x10000, 0x110000));
        assert_eq!(stack.apple_entry(), "main_stack=0x110000,0x100000,0x10000,0x100000");
    }
    #[test]
    fn callback_sp_and_overflow_cannot_impersonate_main_stack() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 0x4000, 3).unwrap();
        assert!(MainStackDescriptor::from_loader(&cpu, 1, 0x10000, 0x4000, 0x10000, 0x4000, 0x90000).is_err());
        assert!(MainStackDescriptor::from_loader(&cpu, 0, 0x10000, 0x4000, 0x10000, 0x4000, 0x13ff0).is_err());
        assert!(MainStackDescriptor::from_loader(&cpu, 1, 0xffffffffffffc000, 0x4000, 0xffffffffffffc000, 0x4000, 0xffffffffffffc000).is_err());
    }
    #[test]
    fn middle_readonly_page_or_mapping_hole_is_rejected() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 0x4000, 3).unwrap();
        cpu.map_zeroed(0x14000, 0x4000, 1).unwrap();
        cpu.map_zeroed(0x18000, 0x4000, 3).unwrap();
        assert!(MainStackDescriptor::from_loader(&cpu, 1, 0x10000, 0xc000, 0x10000, 0xc000, 0x1bff0).is_err());
        let mut hole = A64Cpu::new_sparse();
        hole.map_zeroed(0x10000, 0x4000, 3).unwrap();
        hole.map_zeroed(0x18000, 0x4000, 3).unwrap();
        assert!(MainStackDescriptor::from_loader(&hole, 1, 0x10000, 0xc000, 0x10000, 0xc000, 0x1bff0).is_err());
    }
    #[test]
    fn unreadable_partial_page_is_rejected_without_start_end_guessing() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 7, 3).unwrap();
        cpu.map_zeroed(0x10007, 1, 2).unwrap();
        cpu.map_zeroed(0x10008, 0x3ff8, 3).unwrap();
        assert!(MainStackDescriptor::from_loader(&cpu, 1, 0x10000, 0x4000, 0x10000, 0x4000, 0x13ff0).is_err());
    }
}

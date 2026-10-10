/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Implementation of dyld Objective-C optimization query wrappers.
//! These routines forward through gProcessInfo in unhooked dyld4;
//! we provide the runtime hooks so libobjc can query pre-optimized cache state.
use super::{A64Cpu, bridge::{GuestBridge, ReturnValues}};

pub(super) const ENTRY_FOR_EACH_CLASS: u64 = 0x1a6c7d7fc;
pub(super) const ENTRY_CLASS_COUNT: u64 = 0x1a6c7d860;
pub(super) const ENTRY_FOR_EACH_PROTOCOL: u64 = 0x1a6c7da1c;
pub(super) const ENTRY_FIND_PROTOCOL: u64 = 0x1a6c7d894;
pub(super) const ENTRY_IS_CONSTANT: u64 = 0x1a6c7d8b4;
pub(super) const ENTRY_HAS_INTERPOSING: u64 = 0x1a6c7da38;

const ORIGINAL_FOR_EACH_CLASS: [u8; 28] = [
    0xe2, 0x03, 0x01, 0xaa, 0xe1, 0x03, 0x00, 0xaa, 0x88, 0x85, 0x1d, 0xf0,
    0x00, 0xc5, 0x41, 0xf9, 0x08, 0x00, 0x40, 0xf9, 0x03, 0x69, 0x41, 0xf9,
    0x60, 0x00, 0x1f, 0xd6,
];

const ORIGINAL_CLASS_COUNT: [u8; 20] = [
    0x88, 0x85, 0x1d, 0xf0, 0x00, 0xc5, 0x41, 0xf9, 0x08, 0x00, 0x40, 0xf9,
    0x01, 0x99, 0x41, 0xf9, 0x20, 0x00, 0x1f, 0xd6,
];

const ORIGINAL_FOR_EACH_PROTOCOL: [u8; 28] = [
    0xe2, 0x03, 0x01, 0xaa, 0xe1, 0x03, 0x00, 0xaa, 0x88, 0x85, 0x1d, 0xf0,
    0x00, 0xc5, 0x41, 0xf9, 0x08, 0x00, 0x40, 0xf9, 0x03, 0x6d, 0x41, 0xf9,
    0x60, 0x00, 0x1f, 0xd6,
];

const ORIGINAL_FIND_PROTOCOL: [u8; 32] = [
    0xe3, 0x03, 0x02, 0xaa, 0xe2, 0x03, 0x01, 0xaa, 0xe1, 0x03, 0x00, 0xaa,
    0x88, 0x85, 0x1d, 0xf0, 0x00, 0xc5, 0x41, 0xf9, 0x08, 0x00, 0x40, 0xf9,
    0x04, 0xa1, 0x41, 0xf9, 0x80, 0x00, 0x1f, 0xd6,
];

const ORIGINAL_IS_CONSTANT: [u8; 28] = [
    0xe2, 0x03, 0x01, 0xaa, 0xe1, 0x03, 0x00, 0xaa, 0x88, 0x85, 0x1d, 0xf0,
    0x00, 0xc5, 0x41, 0xf9, 0x08, 0x00, 0x40, 0xf9, 0x03, 0x75, 0x41, 0xf9,
    0x60, 0x00, 0x1f, 0xd6,
];

const ORIGINAL_HAS_INTERPOSING: [u8; 20] = [
    0x88, 0x85, 0x1d, 0xf0, 0x00, 0xc5, 0x41, 0xf9, 0x08, 0x00, 0x40, 0xf9,
    0x01, 0x4d, 0x41, 0xf9, 0x20, 0x00, 0x1f, 0xd6,
];

fn write_trampoline(cpu: &mut A64Cpu, entry: u64, target: u64, total_len: usize) -> Result<(), String> {
    if total_len < 20 {
        return Err("trampoline requires at least 20 bytes".into());
    }
    let mut patch = Vec::with_capacity(total_len);
    for i in 0..4u32 {
        let instruction = (if i == 0 { 0xd2800000 } else { 0xf2800000 })
            | (i << 21)
            | ((((target >> (i * 16)) & 0xffff) as u32) << 5)
            | 16;
        patch.extend_from_slice(&instruction.to_le_bytes());
    }
    patch.extend_from_slice(&0xd61f0200u32.to_le_bytes()); // br x16
    while patch.len() < total_len {
        patch.extend_from_slice(&0xd503201fu32.to_le_bytes()); // nop
    }
    cpu.try_write_bytes(entry, &patch)
}

fn check_stub(cpu: &A64Cpu, entry: u64, expected: &[u8]) -> Result<(), String> {
    if cpu.mapped_permissions(entry).is_none_or(|p| p & 4 == 0) {
        return Err("stub is not executable".into());
    }
    let actual = cpu.read_bytes(entry, expected.len()).ok_or("stub is unreadable")?;
    if actual != expected {
        return Err("stub bytes differ from expected dyld shared cache instructions".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DyldObjcEntries {
    pub for_each_class: u64,
    pub class_count: u64,
    pub for_each_protocol: u64,
    pub find_protocol: u64,
    pub is_constant: u64,
    pub has_interposing: u64,
}

pub(super) fn install(
    cpu: &mut A64Cpu,
    bridge: &mut GuestBridge,
    entries: DyldObjcEntries,
) -> Result<(), String> {
    check_stub(cpu, entries.for_each_class, &ORIGINAL_FOR_EACH_CLASS)?;
    check_stub(cpu, entries.class_count, &ORIGINAL_CLASS_COUNT)?;
    check_stub(cpu, entries.for_each_protocol, &ORIGINAL_FOR_EACH_PROTOCOL)?;
    check_stub(cpu, entries.find_protocol, &ORIGINAL_FIND_PROTOCOL)?;
    check_stub(cpu, entries.is_constant, &ORIGINAL_IS_CONSTANT)?;
    check_stub(cpu, entries.has_interposing, &ORIGINAL_HAS_INTERPOSING)?;

    // 1. __dyld_for_each_objc_class(className, callback)
    let target = bridge.register_service(cpu, "dyld_for_each_objc_class", move |frame| {
        let name_ptr = frame.integer(0)?;
        let callback_ptr = frame.integer(1)?;
        let name = if name_ptr != 0 {
            let mut s = Vec::new();
            for off in 0..512 {
                let b = frame.read(name_ptr.checked_add(off).ok_or("name ptr overflow")?, 1)?[0];
                if b == 0 { break; }
                s.push(b);
            }
            String::from_utf8(s).ok()
        } else {
            None
        };
        echo!("[a64] __dyld_for_each_objc_class name={name:?} callback={callback_ptr:#x}");
        Ok(ReturnValues::integer(0))
    })?.guest_address();
    write_trampoline(cpu, entries.for_each_class, target, ORIGINAL_FOR_EACH_CLASS.len())?;

    // 2. __dyld_objc_class_count() -> size_t
    let target = bridge.register_service(cpu, "dyld_objc_class_count", move |_frame| {
        echo!("[a64] __dyld_objc_class_count -> 0");
        Ok(ReturnValues::integer(0))
    })?.guest_address();
    write_trampoline(cpu, entries.class_count, target, ORIGINAL_CLASS_COUNT.len())?;

    // 3. __dyld_for_each_objc_protocol(protocolName, callback)
    let target = bridge.register_service(cpu, "dyld_for_each_objc_protocol", move |frame| {
        let name_ptr = frame.integer(0)?;
        let callback_ptr = frame.integer(1)?;
        let name = if name_ptr != 0 {
            let mut s = Vec::new();
            for off in 0..512 {
                let b = frame.read(name_ptr.checked_add(off).ok_or("name ptr overflow")?, 1)?[0];
                if b == 0 { break; }
                s.push(b);
            }
            String::from_utf8(s).ok()
        } else {
            None
        };
        echo!("[a64] __dyld_for_each_objc_protocol name={name:?} callback={callback_ptr:#x}");
        Ok(ReturnValues::integer(0))
    })?.guest_address();
    write_trampoline(cpu, entries.for_each_protocol, target, ORIGINAL_FOR_EACH_PROTOCOL.len())?;

    // 4. __dyld_find_protocol_conformance(protocol, class, &outConformance) -> bool/void*
    let target = bridge.register_service(cpu, "dyld_find_protocol_conformance", move |_frame| {
        echo!("[a64] __dyld_find_protocol_conformance -> 0");
        Ok(ReturnValues::integer(0))
    })?.guest_address();
    write_trampoline(cpu, entries.find_protocol, target, ORIGINAL_FIND_PROTOCOL.len())?;

    // 5. __dyld_is_objc_constant(type, ptr) -> bool
    let target = bridge.register_service(cpu, "dyld_is_objc_constant", move |_frame| {
        echo!("[a64] __dyld_is_objc_constant -> false");
        Ok(ReturnValues::integer(0))
    })?.guest_address();
    write_trampoline(cpu, entries.is_constant, target, ORIGINAL_IS_CONSTANT.len())?;

    // 6. _dyld_has_inserted_or_interposing_libraries() -> bool
    let target = bridge.register_service(cpu, "dyld_has_inserted_or_interposing_libraries", move |_frame| {
        echo!("[a64] _dyld_has_inserted_or_interposing_libraries -> false");
        Ok(ReturnValues::integer(0))
    })?.guest_address();
    write_trampoline(cpu, entries.has_interposing, target, ORIGINAL_HAS_INTERPOSING.len())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dyld_objc_install() {
        let mut cpu = A64Cpu::new_sparse();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        let base = ENTRY_FOR_EACH_CLASS & !4095;
        cpu.map_zeroed(base, 0x4000, 5).unwrap();

        cpu.try_write_bytes(ENTRY_FOR_EACH_CLASS, &ORIGINAL_FOR_EACH_CLASS).unwrap();
        cpu.try_write_bytes(ENTRY_CLASS_COUNT, &ORIGINAL_CLASS_COUNT).unwrap();
        cpu.try_write_bytes(ENTRY_FOR_EACH_PROTOCOL, &ORIGINAL_FOR_EACH_PROTOCOL).unwrap();
        cpu.try_write_bytes(ENTRY_FIND_PROTOCOL, &ORIGINAL_FIND_PROTOCOL).unwrap();
        cpu.try_write_bytes(ENTRY_IS_CONSTANT, &ORIGINAL_IS_CONSTANT).unwrap();
        cpu.try_write_bytes(ENTRY_HAS_INTERPOSING, &ORIGINAL_HAS_INTERPOSING).unwrap();

        let entries = DyldObjcEntries {
            for_each_class: ENTRY_FOR_EACH_CLASS,
            class_count: ENTRY_CLASS_COUNT,
            for_each_protocol: ENTRY_FOR_EACH_PROTOCOL,
            find_protocol: ENTRY_FIND_PROTOCOL,
            is_constant: ENTRY_IS_CONSTANT,
            has_interposing: ENTRY_HAS_INTERPOSING,
        };

        install(&mut cpu, &mut bridge, entries).unwrap();
    }
}

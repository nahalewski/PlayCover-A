/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Exact verified class-reference and selector substitution, never a scan of
//! arbitrary guest words. Broader class isa/superclass binding sites need
//! actual per-slot dyld evidence and are deliberately not inferred here.
use super::fixups::ChainedImport;
use super::objc_metadata::{image_slots, read_class};
use super::A64Cpu;
use std::collections::BTreeMap;

fn file_bytes(file: &[u8], at: usize, len: usize) -> Result<&[u8], String> {
    file.get(at..at.checked_add(len).ok_or("GOT metadata range overflow")?)
        .ok_or("truncated GOT metadata".into())
}
fn u32file(file: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        file_bytes(file, at, 4)?.try_into().unwrap(),
    ))
}
fn u64file(file: &[u8], at: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        file_bytes(file, at, 8)?.try_into().unwrap(),
    ))
}

fn message_got_site(
    file: &[u8],
    slide: u64,
    dependencies: &[String],
    imports: &[ChainedImport],
) -> Result<u64, String> {
    const OBJC: &str = "/usr/lib/libobjc.A.dylib";
    if u32file(file, 0)? != 0xfeedfacf || u32file(file, 4)? != 0x0100000c {
        return Err("message GOT requires selected thin ARM64 image".into());
    }
    let count = u32file(file, 16)? as usize;
    let command_bytes = u32file(file, 20)? as usize;
    if count > 4096 || command_bytes > 8 * 1024 * 1024 {
        return Err("message GOT load command budget exceeded".into());
    }
    file_bytes(file, 32, command_bytes)?;
    let end = 32 + command_bytes;
    let mut command = 32;
    let mut sections = Vec::new();
    let mut symtab = None;
    let mut indirect = None;
    for _ in 0..count {
        if command > end || end - command < 8 {
            return Err("truncated message GOT load command".into());
        }
        let kind = u32file(file, command)?;
        let size = u32file(file, command + 4)? as usize;
        if size < 8 || size > end - command {
            return Err("invalid message GOT load command size".into());
        }
        match kind {
            2 => {
                if size != 24 || symtab.is_some() {
                    return Err("invalid or duplicate LC_SYMTAB".into());
                }
                symtab = Some((
                    u32file(file, command + 8)? as usize,
                    u32file(file, command + 12)? as usize,
                    u32file(file, command + 16)? as usize,
                    u32file(file, command + 20)? as usize,
                ));
            }
            0xb => {
                if size != 80 || indirect.is_some() {
                    return Err("invalid or duplicate LC_DYSYMTAB".into());
                }
                indirect = Some((
                    u32file(file, command + 56)? as usize,
                    u32file(file, command + 60)? as usize,
                ));
            }
            0x19 => {
                if size < 72 {
                    return Err("short message GOT segment".into());
                }
                let nsects = u32file(file, command + 64)? as usize;
                if nsects > 4096
                    || nsects.checked_mul(80).and_then(|n| n.checked_add(72)) != Some(size)
                {
                    return Err("invalid message GOT segment section count".into());
                }
                let vm = u64file(file, command + 24)?;
                let vm_size = u64file(file, command + 32)?;
                let file_off = u64file(file, command + 40)?;
                let file_size = u64file(file, command + 48)?;
                let file_end = file_off
                    .checked_add(file_size)
                    .ok_or("message GOT segment file overflow")?;
                if file_end > file.len() as u64 || vm.checked_add(vm_size).is_none() {
                    return Err("message GOT segment outside file or VM bounds".into());
                }
                for index in 0..nsects {
                    let at = command + 72 + index * 80;
                    let name = file_bytes(file, at, 16)?;
                    let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(16)];
                    if name != b"__got" {
                        continue;
                    }
                    if u32file(file, at + 64)? & 0xff != 6 {
                        return Err("message GOT section is not non-lazy symbol pointers".into());
                    }
                    let addr = u64file(file, at + 32)?;
                    let bytes = u64file(file, at + 40)?;
                    let offset = u32file(file, at + 48)? as u64;
                    let delta = addr
                        .checked_sub(vm)
                        .ok_or("message GOT section before segment")?;
                    let relative_end = delta
                        .checked_add(bytes)
                        .ok_or("message GOT section overflow")?;
                    if addr & 7 != 0
                        || bytes & 7 != 0
                        || relative_end > vm_size
                        || relative_end > file_size
                        || file_off.checked_add(delta) != Some(offset)
                    {
                        return Err(
                            "message GOT section is not aligned file-backed segment storage".into(),
                        );
                    }
                    let slots =
                        usize::try_from(bytes / 8).map_err(|_| "message GOT size overflow")?;
                    if slots > 65536 || sections.len() >= 64 {
                        return Err("message GOT section budget exceeded".into());
                    }
                    sections.push((addr, slots, u32file(file, at + 68)? as usize));
                }
            }
            _ => {}
        }
        command += size;
    }
    if command != end {
        return Err("message GOT command size mismatch".into());
    }
    let (symoff, nsyms, stroff, strsize) = symtab.ok_or("message GOT lacks LC_SYMTAB")?;
    let (indirectoff, nindirect) = indirect.ok_or("message GOT lacks LC_DYSYMTAB")?;
    if nsyms > 1_000_000 || nindirect > 1_000_000 || strsize > 16 * 1024 * 1024 {
        return Err("message GOT symbol table budget exceeded".into());
    }
    file_bytes(
        file,
        symoff,
        nsyms
            .checked_mul(16)
            .ok_or("message GOT nlist size overflow")?,
    )?;
    let strings = file_bytes(file, stroff, strsize)?;
    file_bytes(
        file,
        indirectoff,
        nindirect
            .checked_mul(4)
            .ok_or("message GOT indirect table size overflow")?,
    )?;
    let mut target = None;
    let mut examined = 0usize;
    for (address, slots, first) in sections {
        if first.checked_add(slots).is_none_or(|n| n > nindirect) {
            return Err("message GOT indirect section range invalid".into());
        }
        examined = examined
            .checked_add(slots)
            .ok_or("message GOT slot count overflow")?;
        if examined > 65536 {
            return Err("message GOT total slot budget exceeded".into());
        }
        for index in 0..slots {
            let symbol = u32file(file, indirectoff + (first + index) * 4)?;
            if symbol & 0xc0000000 != 0 {
                continue;
            } // local/absolute indirect entries
            let symbol = symbol as usize;
            if symbol >= nsyms {
                return Err("message GOT indirect symbol index invalid".into());
            }
            let at = symoff + symbol * 16;
            let string_index = u32file(file, at)? as usize;
            let name = strings
                .get(string_index..)
                .ok_or("message GOT nlist string index invalid")?;
            if !name.starts_with(b"_objc_msgSend\0") {
                continue;
            }
            let entry = file_bytes(file, at, 16)?;
            let desc = u16::from_le_bytes(entry[6..8].try_into().unwrap());
            let ordinal = usize::from(desc >> 8);
            if entry[4] != 1
                || entry[5] != 0
                || u64file(file, at + 8)? != 0
                || desc & 0xc0 != 0
                || ordinal == 0
                || ordinal >= 0xfe
                || dependencies.get(ordinal - 1).map(String::as_str) != Some(OBJC)
            {
                return Err(
                    "message GOT symbol is not a strong external undefined libobjc import".into(),
                );
            }
            if !imports.iter().any(|import| {
                import.name == "_objc_msgSend"
                    && !import.weak
                    && import.addend == 0
                    && import.library_ordinal == ordinal as i32
            }) {
                return Err("message GOT lacks matching zero-addend dyld import evidence".into());
            }
            let site = address
                .checked_add((index * 8) as u64)
                .and_then(|v| v.checked_add(slide))
                .ok_or("message GOT slid address overflow")?;
            if target.replace(site).is_some() {
                return Err("message GOT requires exactly one objc_msgSend site".into());
            }
        }
    }
    target.ok_or("message GOT has no proven objc_msgSend site".into())
}

/// Original must be the independently verified strong libobjc cache export;
/// replacement must be the registered owned objc_msgSend service. This narrow
/// API patches exactly one indirect-table-proven site with a proven zero addend.
pub(super) fn patch_message_got(
    file: &[u8],
    slide: u64,
    dependencies: &[String],
    imports: &[ChainedImport],
    original: u64,
    replacement: u64,
    cpu: &mut A64Cpu,
) -> Result<usize, String> {
    for address in [original, replacement] {
        if address == 0 || address & 3 != 0 {
            return Err("message GOT function address invalid".into());
        }
        for offset in 0..4 {
            if !cpu
                .mapped_permissions(
                    address
                        .checked_add(offset)
                        .ok_or("message GOT function address overflow")?,
                )
                .is_some_and(|p| p & 4 != 0)
            {
                return Err("message GOT function address is not executable".into());
            }
        }
    }
    let slot = message_got_site(file, slide, dependencies, imports)?;
    if pointer(cpu, slot)? != original {
        return Err(
            "message GOT current value differs from verified original libobjc export".into(),
        );
    }
    let transaction = SlotTransaction {
        patches: vec![Patch {
            slot,
            original,
            replacement,
        }],
        class_slots: 0,
        selector_slots: 0,
    };
    transaction.commit(cpu)?;
    Ok(1)
}

pub(super) trait SlotMemory {
    fn read(&mut self, address: u64, size: usize) -> Result<Vec<u8>, String>;
    fn validate_write(&self, address: u64, size: usize) -> Result<(), String>;
    /// Implementations must allow restoration of validated writable storage.
    fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), String>;
}
impl SlotMemory for A64Cpu {
    fn read(&mut self, a: u64, n: usize) -> Result<Vec<u8>, String> {
        let mut b = vec![0; n];
        self.read_guest_into(a, &mut b)?;
        Ok(b)
    }
    fn validate_write(&self, a: u64, n: usize) -> Result<(), String> {
        self.validate_guest_write(a, n)
    }
    fn write(&mut self, a: u64, b: &[u8]) -> Result<(), String> {
        self.write_guest_into(a, b)
    }
}
/// Caller obtains this original definition through the original cache export
/// trie, bypassing emulator service substitution. Only a strong exact export
/// qualifies; ordinal/provider/name are independently checked against imports.
pub(super) struct VerifiedClassExport<'a> {
    pub provider: &'a str,
    pub name: &'a str,
    pub original_address: u64,
    pub weak_definition: bool,
}
#[derive(Debug)]
struct Patch {
    slot: u64,
    original: u64,
    replacement: u64,
}
#[derive(Debug, Default)]
pub(super) struct SlotTransaction {
    patches: Vec<Patch>,
    pub class_slots: usize,
    pub selector_slots: usize,
}
fn pointer(memory: &mut impl SlotMemory, slot: u64) -> Result<u64, String> {
    let b = memory.read(slot, 8)?;
    if b.len() != 8 {
        return Err("short ObjC substitution slot read".into());
    }
    Ok(u64::from_le_bytes(b.try_into().unwrap()))
}
fn string(
    memory: &mut impl SlotMemory,
    address: u64,
    budget: &mut usize,
) -> Result<String, String> {
    if address == 0 {
        return Err("null ObjC selector identity".into());
    }
    let mut bytes = Vec::new();
    for offset in 0..4096 {
        *budget = budget
            .checked_sub(1)
            .ok_or("ObjC selector text budget exceeded")?;
        let b = memory.read(
            address
                .checked_add(offset)
                .ok_or("ObjC selector address overflow")?,
            1,
        )?;
        if b.len() != 1 {
            return Err("short ObjC selector read".into());
        }
        if b[0] == 0 {
            return String::from_utf8(bytes).map_err(|_| "ObjC selector is not UTF-8".into());
        }
        bytes.push(b[0]);
    }
    Err("ObjC selector exceeds bounded length".into())
}
impl SlotTransaction {
    /// Owned exports must come from Namespace::exports after root linkage.
    /// Dependency strings are this selected image's actual ordinal table.
    pub(super) fn prepare(
        file: &[u8],
        slide: u64,
        imports: &[ChainedImport],
        dependencies: &[String],
        originals: &[VerifiedClassExport<'_>],
        owned_exports: &BTreeMap<String, u64>,
        mut owned_selector: impl FnMut(&str) -> Option<u64>,
        memory: &mut impl SlotMemory,
    ) -> Result<Self, String> {
        if originals.len() > 16 {
            return Err("too many owned class substitutions".into());
        }
        let slots = image_slots(file, slide)?;
        let mut replacements = BTreeMap::new();
        for original in originals {
            let class = original
                .name
                .strip_prefix("_OBJC_CLASS_$_")
                .ok_or("only normal Objective-C class exports may replace classrefs")?;
            if !matches!(
                original.provider,
                "/System/Library/Frameworks/Foundation.framework/Foundation"
                    | "/usr/lib/libobjc.A.dylib"
            ) || original.weak_definition
                || original.original_address == 0
                || original.original_address & 7 != 0
            {
                return Err(
                    "original class export/provider is not a verified strong Foundation definition"
                        .into(),
                );
            }
            let proven = imports.iter().any(|import| {
                import.name == original.name
                    && !import.weak
                    && import.addend == 0
                    && import.library_ordinal > 0
                    && dependencies
                        .get(import.library_ordinal as usize - 1)
                        .is_some_and(|p| p == original.provider)
            });
            if !proven {
                return Err(format!(
                    "class substitution lacks exact strong zero-addend provider import {}",
                    original.name
                ));
            }
            let replacement = *owned_exports
                .get(original.name)
                .ok_or("class export is not implemented by owned namespace")?;
            let metadata = read_class(replacement, |a, n| memory.read(a, n))?;
            if metadata.name != class || metadata.flags & 1 != 0 {
                return Err(
                    "owned replacement class identity does not match original export".into(),
                );
            }
            if let Some(previous) = replacements.insert(original.original_address, replacement) {
                if previous != replacement {
                    return Err("original class export aliases incompatible owned classes".into());
                }
            }
        }
        let mut result = Self::default();
        let mut seen = BTreeMap::new();
        for slot in slots.class_ref_slots {
            let current = pointer(memory, slot)?;
            if let Some(&replacement) = replacements.get(&current) {
                if current != replacement {
                    seen.insert(slot, (current, replacement));
                    result.class_slots += 1;
                }
            }
        }
        let mut budget = 1024 * 1024;
        for slot in slots.selector_ref_slots {
            let current = pointer(memory, slot)?;
            let name = string(memory, current, &mut budget)?;
            if let Some(replacement) = owned_selector(&name) {
                if string(memory, replacement, &mut budget)? != name {
                    return Err("owned canonical selector name mismatch".into());
                }
                if current != replacement {
                    if let Some(previous) = seen.insert(slot, (current, replacement)) {
                        if previous != (current, replacement) {
                            return Err("conflicting ObjC slot substitutions".into());
                        }
                    }
                    result.selector_slots += 1;
                }
            }
        }
        for (slot, (original, replacement)) in seen {
            if slot & 7 != 0 {
                return Err("unaligned ObjC substitution slot".into());
            }
            memory.validate_write(slot, 8)?;
            result.patches.push(Patch {
                slot,
                original,
                replacement,
            });
        }
        Ok(result)
    }
    /// Recheck the entire plan before any mutation. Restore the failed write
    /// too, so a backend which partially wrote before returning an error cannot
    /// leave that slot altered. Restoration failure is reported explicitly.
    pub(super) fn commit(self, memory: &mut impl SlotMemory) -> Result<(usize, usize), String> {
        for patch in &self.patches {
            memory.validate_write(patch.slot, 8)?;
            if pointer(memory, patch.slot)? != patch.original {
                return Err("ObjC substitution slot changed since preparation".into());
            }
        }
        for (index, patch) in self.patches.iter().enumerate() {
            if let Err(error) = memory.write(patch.slot, &patch.replacement.to_le_bytes()) {
                let mut failures = Vec::new();
                for previous in self.patches[..=index].iter().rev() {
                    if memory
                        .write(previous.slot, &previous.original.to_le_bytes())
                        .is_err()
                    {
                        failures.push(previous.slot);
                    }
                }
                return if failures.is_empty() {
                    Err(format!(
                        "ObjC substitution failed and was rolled back: {error}"
                    ))
                } else {
                    Err(format!(
                        "ObjC substitution failed: {error}; rollback incomplete at {failures:x?}"
                    ))
                };
            }
        }
        Ok((self.class_slots, self.selector_slots))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn got_fixture() -> (Vec<u8>, Vec<String>, Vec<ChainedImport>) {
        let mut file = vec![0; 512];
        let put32 = |file: &mut [u8], at: usize, value: u32| {
            file[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        let put64 = |file: &mut [u8], at: usize, value: u64| {
            file[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        put32(&mut file, 0, 0xfeedfacf);
        put32(&mut file, 4, 0x0100000c);
        put32(&mut file, 16, 3);
        put32(&mut file, 20, 256);
        put32(&mut file, 32, 0x19);
        put32(&mut file, 36, 152);
        put64(&mut file, 56, 0x1000);
        put64(&mut file, 64, 512);
        put64(&mut file, 80, 512);
        put32(&mut file, 96, 1);
        file[104..109].copy_from_slice(b"__got");
        put64(&mut file, 136, 0x1120);
        put64(&mut file, 144, 8);
        put32(&mut file, 152, 288);
        put32(&mut file, 168, 6);
        put32(&mut file, 184, 2);
        put32(&mut file, 188, 24);
        put32(&mut file, 192, 304);
        put32(&mut file, 196, 1);
        put32(&mut file, 200, 320);
        put32(&mut file, 204, 15);
        put32(&mut file, 208, 0xb);
        put32(&mut file, 212, 80);
        put32(&mut file, 264, 336);
        put32(&mut file, 268, 1);
        put32(&mut file, 304, 1);
        file[308] = 1;
        file[310..312].copy_from_slice(&0x100u16.to_le_bytes());
        file[321..335].copy_from_slice(b"_objc_msgSend\0");
        (
            file,
            vec!["/usr/lib/libobjc.A.dylib".into()],
            vec![ChainedImport {
                name: "_objc_msgSend".into(),
                library_ordinal: 1,
                weak: false,
                addend: 0,
            }],
        )
    }
    #[test]
    fn message_got_requires_exact_indirect_symbol_and_import_evidence() {
        let (file, dependencies, imports) = got_fixture();
        assert_eq!(
            message_got_site(&file, 0x2000, &dependencies, &imports).unwrap(),
            0x3120
        );
        assert!(message_got_site(&file, 0, &dependencies, &[]).is_err());
        assert!(message_got_site(&file, 0, &["wrong-provider".into()], &imports).is_err());
        let mut weak = file.clone();
        weak[310] = 0x40;
        assert!(message_got_site(&weak, 0, &dependencies, &imports).is_err());
        let mut invalid_index = file.clone();
        invalid_index[336..340].copy_from_slice(&1u32.to_le_bytes());
        assert!(message_got_site(&invalid_index, 0, &dependencies, &imports).is_err());
        assert!(message_got_site(&file[..335], 0, &dependencies, &imports).is_err());
    }
    struct Memory {
        bytes: [u8; 32],
        writes: usize,
        fail_on: Option<usize>,
    }
    impl SlotMemory for Memory {
        fn read(&mut self, a: u64, n: usize) -> Result<Vec<u8>, String> {
            self.bytes
                .get(a as usize..a as usize + n)
                .map(|b| b.to_vec())
                .ok_or("out of range".into())
        }
        fn validate_write(&self, a: u64, n: usize) -> Result<(), String> {
            if a.checked_add(n as u64).is_some_and(|end| end <= 32) {
                Ok(())
            } else {
                Err("out of range".into())
            }
        }
        fn write(&mut self, a: u64, b: &[u8]) -> Result<(), String> {
            self.writes += 1;
            if self.fail_on == Some(self.writes) {
                self.bytes[a as usize] = 0xff;
                return Err("injected partial write".into());
            }
            self.bytes[a as usize..a as usize + b.len()].copy_from_slice(b);
            Ok(())
        }
    }
    fn transaction() -> SlotTransaction {
        SlotTransaction {
            patches: vec![
                Patch {
                    slot: 0,
                    original: 1,
                    replacement: 10,
                },
                Patch {
                    slot: 8,
                    original: 2,
                    replacement: 20,
                },
            ],
            class_slots: 1,
            selector_slots: 1,
        }
    }
    fn memory() -> Memory {
        let mut bytes = [0; 32];
        bytes[..8].copy_from_slice(&1u64.to_le_bytes());
        bytes[8..16].copy_from_slice(&2u64.to_le_bytes());
        Memory {
            bytes,
            writes: 0,
            fail_on: None,
        }
    }
    #[test]
    fn failure_rolls_back_previous_and_partially_written_current_slot() {
        let mut m = memory();
        let before = m.bytes;
        m.fail_on = Some(2);
        assert!(transaction()
            .commit(&mut m)
            .unwrap_err()
            .contains("rolled back"));
        assert_eq!(m.bytes, before);
    }
    #[test]
    fn stale_or_unwritable_plan_never_starts_writes() {
        let mut m = memory();
        m.bytes[8] = 3;
        assert!(transaction().commit(&mut m).is_err());
        assert_eq!(m.writes, 0);
        let mut bad = transaction();
        bad.patches[1].slot = 32;
        assert!(bad.commit(&mut memory()).is_err());
    }
    #[test]
    fn successful_commit_reports_actual_class_and_selector_counts() {
        let mut m = memory();
        assert_eq!(transaction().commit(&mut m).unwrap(), (1, 1));
        assert_eq!(u64::from_le_bytes(m.bytes[..8].try_into().unwrap()), 10);
    }
}

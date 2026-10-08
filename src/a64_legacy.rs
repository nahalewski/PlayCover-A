/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bounded pointer-only legacy dyld opcodes. Lazy bindings are eagerly resolved.
//! Format: Apple mach-o/loader.h and dyld mach_o/{Bind,Rebase}Opcodes.cpp.
use super::{fixups::ChainedImport, Segment64};
use std::collections::BTreeMap;
use touchHLE_dynarmic_wrapper::a64::A64Cpu;

const LIMIT: usize = 1_000_000;
const MAX_STREAM: usize = 16 * 1024 * 1024;

fn bind_skip_step(skip: u64) -> Result<u64, String> {
    // Apple's DO_BIND_ADD_ADDR_ULEB uses uint64_t (skip + pointerSize).
    // Real PlayFabParty encodes -8 as ULEB, requesting zero advancement so
    // another binding can replace this same slot. Permit only this proven
    // overflow form; all other skips retain checked arithmetic.
    if skip == u64::MAX - 7 {
        Ok(0)
    } else {
        skip.checked_add(8)
            .ok_or_else(|| "legacy bind skip overflow".into())
    }
}

struct Bytes<'a> {
    bytes: &'a [u8],
    cursor: usize,
}
impl Bytes<'_> {
    fn byte(&mut self) -> Result<u8, String> {
        let value = *self
            .bytes
            .get(self.cursor)
            .ok_or("truncated legacy dyld operand")?;
        self.cursor += 1;
        Ok(value)
    }
    fn uleb(&mut self) -> Result<u64, String> {
        let mut value = 0u64;
        for i in 0..10 {
            let byte = self.byte()?;
            if i == 9 && byte > 1 {
                return Err("legacy ULEB overflow".into());
            }
            value |= u64::from(byte & 0x7f) << (i * 7);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err("legacy ULEB overflow".into())
    }
    fn sleb(&mut self) -> Result<i64, String> {
        let mut value = 0i128;
        for i in 0..10 {
            let byte = self.byte()?;
            value |= i128::from(byte & 0x7f) << (i * 7);
            if byte & 0x80 == 0 {
                if byte & 0x40 != 0 {
                    value |= !0i128 << ((i + 1) * 7);
                }
                return i64::try_from(value).map_err(|_| "legacy SLEB overflow".into());
            }
        }
        Err("legacy SLEB overflow".into())
    }
    fn name(&mut self) -> Result<String, String> {
        let tail = &self.bytes[self.cursor..];
        let length = tail
            .iter()
            .take(4097)
            .position(|b| *b == 0)
            .ok_or("unterminated or oversized legacy symbol")?;
        if length == 0 || length > 4096 {
            return Err("invalid legacy symbol name".into());
        }
        let name = std::str::from_utf8(&tail[..length])
            .map_err(|_| "invalid legacy symbol UTF-8")?
            .to_owned();
        self.cursor += length + 1;
        Ok(name)
    }
}

#[derive(Default)]
struct Location {
    segment: Option<usize>,
    offset: u64,
}
impl Location {
    fn set(&mut self, segments: &[Segment64], index: usize, offset: u64) -> Result<(), String> {
        let segment = segments
            .get(index)
            .ok_or("legacy segment index outside image")?;
        if offset > segment.vmsize {
            return Err("legacy segment offset outside image".into());
        }
        self.segment = Some(index);
        self.offset = offset;
        Ok(())
    }
    fn advance(&mut self, amount: u64) -> Result<(), String> {
        self.offset = self
            .offset
            .checked_add(amount)
            .ok_or("legacy segment offset overflow")?;
        Ok(())
    }
    fn bind_address_add(&mut self, segments: &[Segment64], amount: u64) -> Result<(), String> {
        if let Some(offset) = self.offset.checked_add(amount) {
            self.offset = offset;
            return Ok(());
        }
        // Apple's uint64_t ADD_ADDR_ULEB arithmetic permits backward deltas
        // encoded as unsigned two's complement (BindOpcodes.cpp). Accept that
        // form only when checked subtraction lands inside the selected segment.
        let segment = self
            .segment
            .and_then(|index| segments.get(index))
            .ok_or("missing legacy segment selection for backward adjustment")?;
        let distance = (!amount)
            .checked_add(1)
            .ok_or("legacy backward delta overflow")?;
        let offset = self
            .offset
            .checked_sub(distance)
            .filter(|offset| *offset <= segment.vmsize && segment.initprot != 0)
            .ok_or("legacy backward adjustment outside mapped segment")?;
        self.offset = offset;
        Ok(())
    }
    fn address(&self, segments: &[Segment64], slide: u64) -> Result<u64, String> {
        let segment = self
            .segment
            .and_then(|i| segments.get(i))
            .ok_or("missing legacy segment selection")?;
        if segment.initprot == 0
            || self
                .offset
                .checked_add(8)
                .is_none_or(|end| end > segment.vmsize)
        {
            return Err("legacy pointer outside mapped segment".into());
        }
        segment
            .vmaddr
            .checked_add(slide)
            .and_then(|v| v.checked_add(self.offset))
            .filter(|v| v.checked_add(8).is_some())
            .ok_or_else(|| "legacy pointer address overflow".into())
    }
}

fn stream(file: &[u8], range: (usize, usize)) -> Result<Bytes<'_>, String> {
    let (offset, size) = range;
    if size > MAX_STREAM {
        return Err("legacy opcode stream exceeds loader limit".into());
    }
    let end = offset
        .checked_add(size)
        .ok_or("legacy stream range overflow")?;
    Ok(Bytes {
        bytes: file.get(offset..end).ok_or("legacy stream outside file")?,
        cursor: 0,
    })
}

struct Writes {
    values: BTreeMap<u64, u64>,
    operations: usize,
}
impl Writes {
    fn stage(&mut self, address: u64, value: u64) -> Result<(), String> {
        self.operations += 1;
        if self.operations > LIMIT {
            return Err("legacy fixup work exceeds loader limit".into());
        }
        if self
            .values
            .range(..address)
            .next_back()
            .is_some_and(|(prior, _)| prior.checked_add(8).is_none_or(|end| end > address))
        {
            return Err("overlapping legacy pointer slots".into());
        }
        use std::ops::Bound::{Excluded, Unbounded};
        if self
            .values
            .range((Excluded(address), Unbounded))
            .next()
            .is_some_and(|(next, _)| address + 8 > *next)
        {
            return Err("overlapping legacy pointer slots".into());
        }
        // Weak-binding/coalescing can replace an ordinary bind at the same slot.
        self.values.insert(address, value);
        Ok(())
    }
}

fn rebase(
    mut bytes: Bytes<'_>,
    segments: &[Segment64],
    slide: u64,
    cpu: &A64Cpu,
    writes: &mut Writes,
) -> Result<(), String> {
    let mut location = Location::default();
    let mut typ = 0;
    let mut opcodes = 0;
    while bytes.cursor < bytes.bytes.len() {
        opcodes += 1;
        if opcodes > LIMIT {
            return Err("legacy rebase opcode limit exceeded".into());
        }
        let opcode = bytes.byte()?;
        let immediate = opcode & 15;
        let (count, step) = match opcode & 0xf0 {
            0x00 => return Ok(()),
            0x10 => {
                if immediate != 1 {
                    return Err("unsupported legacy rebase type (only pointer type 1)".into());
                }
                typ = 1;
                continue;
            }
            0x20 => {
                let offset = bytes.uleb()?;
                location.set(segments, immediate as usize, offset)?;
                continue;
            }
            0x30 => {
                let amount = bytes.uleb()?;
                location.advance(amount)?;
                continue;
            }
            0x40 => {
                location.advance(u64::from(immediate) * 8)?;
                continue;
            }
            0x50 => (u64::from(immediate), 8),
            0x60 => (bytes.uleb()?, 8),
            0x70 => (
                1,
                bytes
                    .uleb()?
                    .checked_add(8)
                    .ok_or("legacy rebase skip overflow")?,
            ),
            0x80 => {
                let count = bytes.uleb()?;
                (
                    count,
                    bytes
                        .uleb()?
                        .checked_add(8)
                        .ok_or("legacy rebase skip overflow")?,
                )
            }
            _ => return Err(format!("unsupported legacy rebase opcode {opcode:#x}")),
        };
        if count > LIMIT as u64 || count > (LIMIT - writes.operations) as u64 {
            return Err("legacy rebase repeat limit exceeded".into());
        }
        for _ in 0..count {
            if typ != 1 {
                return Err("legacy rebase has no pointer type".into());
            }
            let address = location.address(segments, slide)?;
            if writes.values.contains_key(&address) {
                return Err("duplicate legacy rebase location".into());
            }
            let old = cpu
                .read_u64(address)
                .ok_or("legacy rebase points outside guest memory")?;
            writes.stage(
                address,
                old.checked_add(slide)
                    .ok_or("legacy rebase target overflow")?,
            )?;
            location.advance(step)?;
        }
    }
    // Real linkers may delimit a stream by its declared size without DONE.
    Ok(())
}

fn bind(
    mut bytes: Bytes<'_>,
    segments: &[Segment64],
    slide: u64,
    cpu: Option<&A64Cpu>,
    kind: usize,
    writes: &mut Writes,
    resolver: &mut impl FnMut(&ChainedImport) -> Result<u64, String>,
    notify_strong: &mut impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    let mut location = Location::default();
    let mut typ = if kind == 3 { 1 } else { 0 };
    let mut ordinal = if kind == 2 { Some(-3) } else { None };
    let mut name = None;
    let mut weak = false;
    let mut addend = 0;
    let mut resolved = None;
    let mut opcodes = 0;
    while bytes.cursor < bytes.bytes.len() {
        opcodes += 1;
        if opcodes > LIMIT {
            return Err("legacy bind opcode limit exceeded".into());
        }
        let opcode = bytes.byte()?;
        let immediate = opcode & 15;
        let (count, step) = match opcode & 0xf0 {
            0x00 => {
                if kind == 3 {
                    continue;
                }
                return Ok(());
            }
            0x10 => {
                if kind == 2 {
                    return Err("weak bind cannot change implicit weak-lookup ordinal".into());
                }
                ordinal = Some(i32::from(immediate));
                resolved = None;
                continue;
            }
            0x20 => {
                if kind == 2 {
                    return Err("weak bind cannot change implicit weak-lookup ordinal".into());
                }
                ordinal = Some(
                    i32::try_from(bytes.uleb()?).map_err(|_| "legacy library ordinal overflow")?,
                );
                resolved = None;
                continue;
            }
            0x30 => {
                if kind == 2 {
                    return Err("weak bind cannot change implicit weak-lookup ordinal".into());
                }
                let value = if immediate == 0 {
                    0
                } else {
                    i32::from(immediate) - 16
                };
                if value < -3 {
                    return Err("unsupported legacy special library ordinal".into());
                }
                ordinal = Some(value);
                resolved = None;
                continue;
            }
            0x40 => {
                if immediate & !9 != 0 || (immediate & 8 != 0 && kind != 2) {
                    return Err("unsupported legacy symbol flags".into());
                }
                let symbol = bytes.name()?;
                if immediate & 8 != 0 {
                    notify_strong(&symbol)?;
                }
                name = Some(symbol);
                weak = immediate & 1 != 0;
                resolved = None;
                continue;
            }
            0x50 => {
                if immediate != 1 {
                    return Err("unsupported legacy bind type (only pointer type 1)".into());
                }
                typ = 1;
                continue;
            }
            0x60 => {
                addend = bytes.sleb()?;
                resolved = None;
                continue;
            }
            0x70 => {
                let offset = bytes.uleb()?;
                location.set(segments, immediate as usize, offset)?;
                continue;
            }
            0x80 => {
                let amount = bytes.uleb()?;
                location.bind_address_add(segments, amount)?;
                continue;
            }
            0x90 => (1, 8),
            0xa0 => (1, bind_skip_step(bytes.uleb()?)?),
            0xb0 => (1, u64::from(immediate) * 8 + 8),
            0xc0 => {
                let count = bytes.uleb()?;
                (count, bind_skip_step(bytes.uleb()?)?)
            }
            0xd0 => return Err("unsupported legacy threaded bind opcode".into()),
            _ => return Err(format!("unsupported legacy bind opcode {opcode:#x}")),
        };
        if count > LIMIT as u64 || count > (LIMIT - writes.operations) as u64 {
            return Err("legacy bind repeat limit exceeded".into());
        }
        for _ in 0..count {
            if typ != 1 {
                return Err("legacy bind has no pointer type".into());
            }
            let address = location.address(segments, slide)?;
            if cpu.is_some_and(|cpu| cpu.read_u64(address).is_none()) {
                return Err("legacy bind points outside guest memory".into());
            }
            let value = match resolved {
                Some(value) => value,
                None => {
                    let import = ChainedImport {
                        library_ordinal: ordinal.ok_or("legacy bind has no library ordinal")?,
                        weak,
                        name: name.as_ref().ok_or("legacy bind has no symbol")?.clone(),
                        addend,
                    };
                    let symbol = resolver(&import).map_err(|error| format!(
                        "legacy bind {} ordinal {} weak {} addend {}: {error}",
                        import.name, import.library_ordinal, import.weak, import.addend
                    ))?;
                    let value = if cpu.is_some() {
                        // dyld Loader::forEachBindTarget adds the signed addend
                        // to uint64_t targetRuntimeOffset, modulo 2^64. Real
                        // Quest PhysX RTTI-name binds use INT64_MIN to tag the
                        // resolved pointer's high bit. Slot/range arithmetic
                        // remains checked; only the stored pointer value wraps.
                        symbol.wrapping_add(addend as u64)
                    } else {
                        0
                    };
                    resolved = Some(value);
                    value
                }
            };
            if cpu.is_some() {
                writes.stage(address, value)?;
            } else {
                writes.operations += 1;
            }
            location.advance(step)?;
        }
    }
    Ok(())
}

/// Streams are `[rebase, bind, weak_bind, lazy_bind]` `(file offset, size)`
/// pairs from LC_DYLD_INFO. Segments retain preferred addresses; resolved symbol
/// addresses already include their own slide. Resolver does not apply addends.
/// Nonempty weak-bind streams remain unsupported until strong/weak-definition
/// coalescing is implemented. No guest writes occur until all streams succeed.
/// Symbol resolvers can
/// have host-side effects; those are not rolled back on malformed input.
pub(super) fn apply(
    file: &[u8],
    streams: [(usize, usize); 4],
    segments: &[Segment64],
    slide: u64,
    cpu: &mut A64Cpu,
    mut resolver: impl FnMut(&ChainedImport) -> Result<u64, String>,
) -> Result<(), String> {
    // Check every file range before consulting the resolver or guest memory.
    for range in streams {
        stream(file, range)?;
    }
    if streams[2].1 != 0 {
        return Err(
            "legacy weak-bind stream requires unsupported weak/strong-definition coalescing".into(),
        );
    }
    apply_with_weak_notifications(file, streams, segments, slide, cpu, &mut resolver, |_| {
        Err("strong-definition notification requires a coalescing linker".into())
    })
}

/// A coalescing-aware linker must collect `weak_declarations` from every image
/// before applying any image, then resolve -3 lookups in load order with strong
/// exports taking precedence over weak exports. `notify_strong` receives names
/// only: flag8 declarations carry neither addresses nor implicit pointer writes.
/// Callback errors abort before committing guest writes; callback state itself
/// is owned by the caller. Missing required weak definitions must return errors.
pub(super) fn apply_with_weak_notifications(
    file: &[u8],
    streams: [(usize, usize); 4],
    segments: &[Segment64],
    slide: u64,
    cpu: &mut A64Cpu,
    mut resolver: impl FnMut(&ChainedImport) -> Result<u64, String>,
    mut notify_strong: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    for range in streams {
        stream(file, range)?;
    }
    let mut writes = Writes {
        values: BTreeMap::new(),
        operations: 0,
    };
    rebase(stream(file, streams[0])?, segments, slide, cpu, &mut writes)?;
    // Lazy pointers are eagerly bound here. Apply weak coalescing last so its
    // canonical target also replaces any eager lazy binding at the same slot.
    for kind in [1, 3, 2] {
        bind(
            stream(file, streams[kind])?,
            segments,
            slide,
            Some(cpu),
            kind,
            &mut writes,
            &mut resolver,
            &mut notify_strong,
        )?;
    }
    for (address, value) in writes.values {
        cpu.write_bytes(address, &value.to_le_bytes());
    }
    Ok(())
}

/// Collect strong-definition notifications without inventing addresses. Every
/// bind action is still validated, but no CPU memory is read or modified.
pub(super) fn weak_declarations(
    file: &[u8],
    streams: [(usize, usize); 4],
    segments: &[Segment64],
) -> Result<Vec<String>, String> {
    let mut names = std::collections::BTreeSet::new();
    let mut name_bytes = 0usize;
    let mut writes = Writes {
        values: BTreeMap::new(),
        operations: 0,
    };
    bind(
        stream(file, streams[2])?,
        segments,
        0,
        None,
        2,
        &mut writes,
        &mut |_| Ok(0),
        &mut |name| {
            if !names.contains(name) {
                name_bytes = name_bytes
                    .checked_add(name.len())
                    .filter(|n| *n <= MAX_STREAM)
                    .ok_or("weak declaration name storage limit exceeded")?;
                if names.len() == 100_000 {
                    return Err("weak declaration count exceeds loader limit".into());
                }
                names.insert(name.to_owned());
            }
            Ok(())
        },
    )?;
    Ok(names.into_iter().collect())
}

/// Actual ordinary/weak/lazy pointer imports, excluding addressless strong
/// declaration notifications. Unique symbol/ordinal/weak/addend tuples are
/// retained so repeated pointer locations cannot amplify audit memory.
pub(super) fn imports(
    file: &[u8],
    streams: [(usize, usize); 4],
    segments: &[Segment64],
) -> Result<Vec<ChainedImport>, String> {
    for range in streams {
        stream(file, range)?;
    }
    let mut unique = BTreeMap::new();
    let mut name_bytes = 0usize;
    let mut writes = Writes {
        values: BTreeMap::new(),
        operations: 0,
    };
    for kind in 1..4 {
        bind(
            stream(file, streams[kind])?,
            segments,
            0,
            None,
            kind,
            &mut writes,
            &mut |import| {
                let key = (
                    import.library_ordinal,
                    import.weak,
                    import.addend,
                    import.name.clone(),
                );
                if !unique.contains_key(&key) {
                    if unique.len() == 100_000 {
                        return Err("legacy import count exceeds loader limit".into());
                    }
                    name_bytes = name_bytes
                        .checked_add(import.name.len())
                        .filter(|n| *n <= MAX_STREAM)
                        .ok_or("legacy import name storage limit exceeded")?;
                    unique.insert(key, import.clone());
                }
                Ok(0)
            },
            &mut |_| Ok(()),
        )?;
    }
    Ok(unique.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: u64 = 0x1_0000_0000;
    fn fixture() -> (Vec<Segment64>, A64Cpu) {
        (
            vec![Segment64 {
                name: "__DATA".into(),
                vmaddr: BASE,
                vmsize: 0x1000,
                fileoff: 0,
                filesize: 0x1000,
                initprot: 3,
            }],
            A64Cpu::new(BASE + 0x1000, 0x1000),
        )
    }
    fn ranges(parts: &[&[u8]; 4]) -> (Vec<u8>, [(usize, usize); 4]) {
        let mut file = Vec::new();
        let mut result = [(0, 0); 4];
        for (index, part) in parts.iter().enumerate() {
            result[index] = (file.len(), part.len());
            file.extend_from_slice(part);
        }
        (file, result)
    }
    #[test]
    fn rebases_pointer_repeats_skips_and_address_additions() {
        let (segments, mut cpu) = fixture();
        // Two immediate, one ULEB repeat, one skip, two counted skips, then
        // address additions followed by another immediate rebase.
        let rebase = [
            0x11, 0x20, 0, 0x52, 0x60, 1, 0x70, 8, 0x80, 2, 8, 0x30, 8, 0x41, 0x51,
        ];
        for offset in [0, 8, 16, 24, 40, 56, 88] {
            cpu.write_bytes(BASE + 0x1000 + offset, &(BASE + 0x200u64).to_le_bytes());
        }
        let (file, streams) = ranges(&[&rebase, &[], &[], &[]]);
        apply(
            &file,
            streams,
            &segments,
            0x1000,
            &mut cpu,
            |_| unreachable!(),
        )
        .unwrap();
        for offset in [0, 8, 16, 24, 40, 56, 88] {
            assert_eq!(cpu.read_u64(BASE + 0x1000 + offset), Some(BASE + 0x1200));
        }
    }
    #[test]
    fn binds_signed_addends_and_repeat_skip_opcodes() {
        let (segments, mut cpu) = fixture();
        let bind = [
            0x11, 0x40, b'_', b'f', 0, 0x51, 0x60, 0x78, 0x70, 0, 0x90, 0xa0, 8, 0xb1, 0xc0, 2, 8,
        ];
        let (file, streams) = ranges(&[&[], &bind, &[], &[]]);
        let mut calls = 0;
        apply(&file, streams, &segments, 0x1000, &mut cpu, |import| {
            calls += 1;
            assert_eq!(import.library_ordinal, 1);
            assert_eq!(import.name, "_f");
            assert_eq!(import.addend, -8);
            Ok(0x2000)
        })
        .unwrap();
        assert_eq!(calls, 1);
        for offset in [0, 8, 24, 40, 56] {
            assert_eq!(cpu.read_u64(BASE + 0x1000 + offset), Some(0x1ff8));
        }
    }
    #[test]
    fn eager_lazy_stream_defaults_to_pointer_and_continues_done_records() {
        let (segments, mut cpu) = fixture();
        let lazy = [
            0x70, 0, 0x11, 0x40, b'_', b'a', 0, 0x90, 0, 0x70, 8, 0x3f, 0x41, b'_', b'b', 0, 0x90,
            0,
        ];
        let (file, streams) = ranges(&[&[], &[], &[], &lazy]);
        apply(&file, streams, &segments, 0x1000, &mut cpu, |import| {
            if import.name == "_a" {
                assert_eq!(import.library_ordinal, 1);
                Ok(42)
            } else {
                assert_eq!(import.library_ordinal, -1);
                assert!(import.weak);
                Ok(0)
            }
        })
        .unwrap();
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(42));
        assert_eq!(cpu.read_u64(BASE + 0x1008), Some(0));
    }
    #[test]
    fn real_unsigned_backward_bind_delta_stays_in_mapped_segment() {
        let (segments, mut cpu) = fixture();
        // Actual CydiaSubstrate ADD_ADDR_ULEB: 0x588 + (-40) -> 0x560.
        let bind = [
            0x11, 0x40, b'_', b'f', 0, 0x51, 0x70, 0x88, 0x0b, 0x80, 0xd8, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 1, 0x90,
        ];
        let (file, streams) = ranges(&[&[], &bind, &[], &[]]);
        assert_eq!(imports(&file, streams, &segments).unwrap().len(), 1);
        apply(&file, streams, &segments, 0x1000, &mut cpu, |_| Ok(42)).unwrap();
        assert_eq!(cpu.read_u64(BASE + 0x1560), Some(42));
        let mut invalid = bind;
        invalid[7] = 0x88;
        invalid[8] = 0; // offset 8 cannot subtract 40
        let (file, streams) = ranges(&[&[], &invalid, &[], &[]]);
        assert!(apply(&file, streams, &segments, 0x1000, &mut cpu, |_| Ok(99)).is_err());
        assert_eq!(cpu.read_u64(BASE + 0x1560), Some(42));
    }
    #[test]
    fn signed_pointer_addends_wrap_like_dyld_without_relaxing_slots() {
        let (segments, mut cpu) = fixture();
        // Real Quest weak-coalescing high-bit addend encoded as signed LEB.
        let bind = [0x11,0x40,b'_',b'p',0,0x51,0x60,
            0x80,0x80,0x80,0x80,0x80,0x80,0x80,0x80,0x80,0x7f,
            0x70,0,0x90,0];
        let (file, streams) = ranges(&[&[],&bind,&[],&[]]);
        apply(&file, streams, &segments, 0x1000, &mut cpu, |import| {
            assert_eq!(import.addend, i64::MIN);
            Ok(0x1a974d000)
        }).unwrap();
        assert_eq!(cpu.read_u64(BASE+0x1000), Some(0x80000001a974d000));
        let negative = [0x11,0x40,b'_',b'p',0,0x51,0x60,0x7f,0x70,8,0x90,0];
        let (file, streams) = ranges(&[&[],&negative,&[],&[]]);
        apply(&file, streams, &segments, 0x1000, &mut cpu, |_| Ok(0)).unwrap();
        assert_eq!(cpu.read_u64(BASE+0x1008), Some(u64::MAX));
        // Invalid slot stream still rejects before committing a prior write.
        let (file, streams) = ranges(&[&[],&bind,&[],&[0xd0]]);
        assert!(apply(&file,streams,&segments,0x1000,&mut cpu, |_| Ok(99)).is_err());
        assert_eq!(cpu.read_u64(BASE+0x1000), Some(0x80000001a974d000));
    }
    #[test]
    fn real_negative_pointer_skip_rebinds_same_slot_atomically() {
        let (segments, mut cpu) = fixture();
        // PlayFabParty opcode 0xa0 encodes skip=-8, net advancement zero.
        let bind = [
            0x11, 0x40, b'_', b'a', 0, 0x51, 0x70, 0, 0xa0, 0xf8, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 1, 0x40, b'_', b'b', 0, 0x90,
        ];
        let (file, streams) = ranges(&[&[], &bind, &[], &[]]);
        assert_eq!(imports(&file, streams, &segments).unwrap().len(), 2);
        apply(&file, streams, &segments, 0x1000, &mut cpu, |import| {
            Ok(if import.name == "_a" { 21 } else { 42 })
        })
        .unwrap();
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(42));
        assert_eq!(cpu.read_u64(BASE + 0x1008), Some(0));
        let (file, streams) = ranges(&[&[], &bind, &[], &[0xd0]]);
        assert!(apply(&file, streams, &segments, 0x1000, &mut cpu, |_| Ok(99)).is_err());
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(42));
        assert!(bind_skip_step(u64::MAX).is_err());
        assert!(bind_skip_step(u64::MAX - 6).is_err());
        assert_eq!(bind_skip_step(u64::MAX - 7), Ok(0));
        // Repeated zero-step writes are still subject to the work bound.
        let repeated = [
            0x11, 0x40, b'_', b'a', 0, 0x51, 0x70, 0, 0xc0, 0xc1, 0x84, 0x3d, 0xf8, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 1,
        ];
        let (file, streams) = ranges(&[&[], &repeated, &[], &[]]);
        assert!(
            apply(&file, streams, &segments, 0x1000, &mut cpu, |_| Ok(99))
                .unwrap_err()
                .contains("repeat limit")
        );
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(42));
    }
    #[test]
    fn self_binding_requires_explicit_ordinal_and_ordinary_pointer_type() {
        let (segments, mut cpu) = fixture();
        let bind = [0x30, 0x40, b'_', b'f', 0, 0x51, 0x70, 0, 0x90];
        let (file, streams) = ranges(&[&[], &bind, &[], &[]]);
        apply(&file, streams, &segments, 0x1000, &mut cpu, |import| {
            assert_eq!(import.library_ordinal, 0);
            Ok(42)
        })
        .unwrap();
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(42));
        let (file, streams) = ranges(&[&[], &bind[1..], &[], &[]]);
        assert!(
            apply(&file, streams, &segments, 0x1000, &mut cpu, |_| Ok(99))
                .unwrap_err()
                .contains("ordinal")
        );
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(42));
        let missing_type = [0x30, 0x40, b'_', b'f', 0, 0x70, 0, 0x90];
        let (file, streams) = ranges(&[&[], &missing_type, &[], &[]]);
        assert!(
            apply(&file, streams, &segments, 0x1000, &mut cpu, |_| Ok(99))
                .unwrap_err()
                .contains("pointer type")
        );
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(42));
    }
    #[test]
    fn weak_notifications_are_addressless_and_metadata_imports_are_real_refs() {
        let (segments, mut cpu) = fixture();
        let weak = [
            0x48, b'_', b'a', 0, 0x51, 0x40, b'_', b'f', 0, 0x70, 0, 0x90, 0,
        ];
        let (file, streams) = ranges(&[&[], &[], &weak, &[]]);
        assert_eq!(
            weak_declarations(&file, streams, &segments).unwrap(),
            vec!["_a"]
        );
        let list = imports(&file, streams, &segments).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "_f");
        assert_eq!(list[0].library_ordinal, -3);
        assert!(!list[0].weak);
        let mut notified = Vec::new();
        apply_with_weak_notifications(
            &file,
            streams,
            &segments,
            0x1000,
            &mut cpu,
            |import| {
                assert_eq!(import.name, "_f");
                assert_eq!(import.library_ordinal, -3);
                Ok(0x3000)
            },
            |name| {
                notified.push(name.to_owned());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(notified, vec!["_a"]);
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(0x3000));
    }
    #[test]
    fn coalescing_callback_prefers_strong_then_first_weak_and_overrides_eager_lazy() {
        let (segments, mut cpu) = fixture();
        let ordinary = [0x11, 0x40, b'_', b'f', 0, 0x51, 0x70, 0, 0x90];
        let lazy = [0x11, 0x40, b'_', b'f', 0, 0x70, 0, 0x90, 0];
        let weak = [0x51, 0x40, b'_', b'f', 0, 0x70, 0, 0x90];
        let (file, streams) = ranges(&[&[], &ordinary, &weak, &lazy]);
        for definitions in [
            vec![(0x1111u64, true), (0x3333, true), (0x2222, false)],
            vec![(0x1111u64, true), (0x3333, true)],
        ] {
            let expected = definitions
                .iter()
                .find(|(_, weak)| !*weak)
                .unwrap_or(&definitions[0])
                .0;
            apply_with_weak_notifications(
                &file,
                streams,
                &segments,
                0x1000,
                &mut cpu,
                |import| {
                    Ok(if import.library_ordinal == -3 {
                        expected
                    } else {
                        definitions[0].0
                    })
                },
                |_| Ok(()),
            )
            .unwrap();
            assert_eq!(cpu.read_u64(BASE + 0x1000), Some(expected));
        }
    }
    #[test]
    fn weak_resolution_or_notification_errors_preserve_all_guest_writes() {
        let (segments, mut cpu) = fixture();
        cpu.write_bytes(BASE + 0x1000, &100u64.to_le_bytes());
        let rebase = [0x11, 0x20, 0, 0x51];
        let weak = [
            0x48, b'_', b'a', 0, 0x51, 0x40, b'_', b'f', 0, 0x70, 8, 0x90,
        ];
        let (file, streams) = ranges(&[&rebase, &[], &weak, &[]]);
        assert!(apply_with_weak_notifications(
            &file,
            streams,
            &segments,
            0x1000,
            &mut cpu,
            |_| Err("required weak definition is missing".into()),
            |_| Ok(())
        )
        .is_err());
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(100));
        assert_eq!(cpu.read_u64(BASE + 0x1008), Some(0));
        assert!(apply_with_weak_notifications(
            &file,
            streams,
            &segments,
            0x1000,
            &mut cpu,
            |_| Ok(200),
            |_| Err("unsupported cache interposition".into())
        )
        .is_err());
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(100));
    }
    #[test]
    fn malformed_late_stream_preserves_rebase_and_bind_slots() {
        let (segments, mut cpu) = fixture();
        cpu.write_bytes(BASE + 0x1000, &100u64.to_le_bytes());
        let (file, streams) = ranges(&[
            &[0x11, 0x20, 0, 0x51],
            &[0x11, 0x40, b'_', b'f', 0, 0x51, 0x70, 8, 0x90],
            &[],
            &[0xd0],
        ]);
        assert!(
            apply(&file, streams, &segments, 0x1000, &mut cpu, |_| Ok(200))
                .unwrap_err()
                .contains("threaded")
        );
        assert_eq!(cpu.read_u64(BASE + 0x1000), Some(100));
        assert_eq!(cpu.read_u64(BASE + 0x1008), Some(0));
    }
    #[test]
    fn rejects_resource_overflow_truncation_and_unmapped_slots() {
        let (segments, mut cpu) = fixture();
        for rebase in [
            &[0x11, 0x20, 0x80][..],
            &[0x11, 0x20, 0, 0x60, 0x80, 0x80, 0x80, 0x80, 0x10][..],
            &[0x11, 0x20, 0xf8, 0x1f, 0x52][..],
            &[0x12][..],
        ] {
            let (file, streams) = ranges(&[rebase, &[], &[], &[]]);
            assert!(apply(&file, streams, &segments, 0x1000, &mut cpu, |_| Ok(0)).is_err());
        }
        let (file, streams) = ranges(&[&[], &[], &[0], &[]]);
        assert!(
            apply(&file, streams, &segments, 0x1000, &mut cpu, |_| Ok(0))
                .unwrap_err()
                .contains("coalescing")
        );
        let mut bytes = Bytes {
            bytes: &[0x80; 11],
            cursor: 0,
        };
        assert!(bytes.uleb().is_err());
        let mut bytes = Bytes {
            bytes: &[0x80; 11],
            cursor: 0,
        };
        assert!(bytes.sleb().is_err());
    }
}

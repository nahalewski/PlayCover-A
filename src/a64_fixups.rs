/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Ordinary 64-bit chained rebases and binds from Apple's fixup-chains.h.
//! https://github.com/apple-oss-distributions/dyld/blob/main/include/mach-o/fixup-chains.h
use super::{rd_u32, rd_u64, A64Cpu, MachO64};

// Bound both allocation and work when many imports point at the same string.
const MAX_IMPORTS: usize = 100_000;
const MAX_IMPORT_NAME: usize = 4096;
const MAX_COPIED_IMPORT_NAMES: usize = 16 * 1024 * 1024;

fn u16_at(data: &[u8], offset: usize) -> Result<u16, String> {
    data.get(offset..offset.checked_add(2).ok_or("fixups offset overflow")?)
        .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
        .ok_or_else(|| "truncated chained fixups".into())
}

pub(super) fn validate_header(data: &[u8]) -> Result<(), String> {
    if data.len() < 28 || rd_u32(data, 0)? != 0 {
        return Err("unsupported or truncated chained fixups header".into());
    }
    if rd_u32(data, 24)? != 0 {
        return Err("compressed chained fixup symbols are not supported".into());
    }
    for offset in [4, 8, 12] {
        if rd_u32(data, offset)? as usize > data.len() {
            return Err("chained fixups header offset outside payload".into());
        }
    }
    parse_imports(data)?;
    Ok(())
}

/// The resolver chooses the dependency (including special ordinals) and must
/// return an already relocated symbol address. It may return zero for a missing
/// weak import; missing strong imports must produce an error. Table and pointer
/// addends are applied here, rather than by the resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChainedImport {
    pub library_ordinal: i32,
    pub weak: bool,
    pub name: String,
    pub addend: i64,
}

/// Parse validated imports for offline availability audits, without changing
/// guest memory or resolving any symbol.
pub(super) fn imports(data: &[u8]) -> Result<Vec<ChainedImport>, String> {
    validate_header(data)?;
    parse_imports(data)
}

fn parse_imports(data: &[u8]) -> Result<Vec<ChainedImport>, String> {
    let count = rd_u32(data, 16)? as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    if count > MAX_IMPORTS {
        return Err("chained import count exceeds loader limit".into());
    }
    let format = rd_u32(data, 20)?;
    let stride = match format {
        1 => 4usize,
        2 => 8,
        3 => 16,
        _ => return Err(format!("unsupported chained import format {format}")),
    };
    let offset = rd_u32(data, 8)? as usize;
    let symbols_offset = rd_u32(data, 12)? as usize;
    let table_end = count
        .checked_mul(stride)
        .and_then(|size| offset.checked_add(size))
        .ok_or("chained imports table overflow")?;
    if offset < 28 || symbols_offset < table_end || symbols_offset > data.len() {
        return Err("chained imports or symbols outside payload or overlapping".into());
    }
    let table = data
        .get(offset..table_end)
        .ok_or("truncated chained imports table")?;
    let symbols = &data[symbols_offset..];
    let mut imports = Vec::with_capacity(count);
    let mut copied_name_bytes = 0usize;
    for entry in table.chunks_exact(stride) {
        let (ordinal, weak, name_offset, addend) = if format == 3 {
            let raw = rd_u64(entry, 0)?;
            if (raw >> 17) & 0x7fff != 0 {
                return Err("nonzero reserved chained import bits".into());
            }
            let ordinal = (raw & 0xffff) as i32;
            (
                if ordinal > 0xfff0 {
                    ordinal - 0x10000
                } else {
                    ordinal
                },
                raw & (1 << 16) != 0,
                (raw >> 32) as usize,
                rd_u64(entry, 8)? as i64,
            )
        } else {
            let raw = rd_u32(entry, 0)?;
            let ordinal = (raw & 0xff) as i32;
            (
                if ordinal > 0xf0 {
                    ordinal - 0x100
                } else {
                    ordinal
                },
                raw & (1 << 8) != 0,
                (raw >> 9) as usize,
                if format == 2 {
                    rd_u32(entry, 4)? as i32 as i64
                } else {
                    0
                },
            )
        };
        let name_tail = symbols
            .get(name_offset..)
            .ok_or("chained import name outside symbols")?;
        let length = name_tail
            .iter()
            .take(MAX_IMPORT_NAME + 1)
            .position(|b| *b == 0)
            .ok_or_else(|| {
                if name_tail.len() > MAX_IMPORT_NAME {
                    "chained import name exceeds loader limit"
                } else {
                    "unterminated chained import name"
                }
            })?;
        if length == 0 {
            return Err("empty chained import name".into());
        }
        copied_name_bytes = copied_name_bytes
            .checked_add(length)
            .filter(|size| *size <= MAX_COPIED_IMPORT_NAMES)
            .ok_or("copied chained import names exceed loader limit")?;
        let name = std::str::from_utf8(&name_tail[..length])
            .map_err(|_| "invalid UTF-8 chained import name")?
            .to_owned();
        imports.push(ChainedImport {
            library_ordinal: ordinal,
            weak,
            name,
            addend,
        });
    }
    Ok(imports)
}

pub(super) fn apply(data: &[u8], macho: &MachO64, cpu: &mut A64Cpu) -> Result<(), String> {
    apply_with_resolver(data, macho, cpu, 0, |import| {
        Err(format!("unresolved ARM64 chained bind {} (library ordinal {}); dynamic symbol binding is unavailable", import.name, import.library_ordinal))
    })
}

/// `macho` retains preferred addresses; `slide` is added to guest fixup slots
/// and rebases. Resolved bind addresses already include their own image slide.
pub(super) fn apply_with_resolver(
    data: &[u8],
    macho: &MachO64,
    cpu: &mut A64Cpu,
    slide: u64,
    mut resolver: impl FnMut(&ChainedImport) -> Result<u64, String>,
) -> Result<(), String> {
    validate_header(data)?;
    let imports = parse_imports(data)?;
    let starts_offset = rd_u32(data, 4)? as usize;
    if starts_offset < 28 {
        return Err("chained starts overlap fixups header".into());
    }
    let starts_end = [rd_u32(data, 8)? as usize, rd_u32(data, 12)? as usize]
        .into_iter()
        .filter(|offset| *offset > starts_offset)
        .min()
        .unwrap_or(data.len());
    let starts = data
        .get(starts_offset..starts_end)
        .ok_or("invalid chained starts")?;
    let count = rd_u32(starts, 0)? as usize;
    if count != macho.segments.len() || count > starts.len().saturating_sub(4) / 4 {
        return Err("invalid chained segment table".into());
    }
    let image_base = macho
        .segments
        .iter()
        .find(|s| s.fileoff == 0 && s.filesize != 0)
        .ok_or("no image base for chained fixups")?
        .vmaddr;
    // Validate every chain before writing, so an unresolved bind or bad later
    // chain cannot leave partially decoded guest pointers in a loaded image.
    let mut writes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (index, segment) in macho.segments.iter().enumerate() {
        let offset = rd_u32(starts, 4 + index * 4)? as usize;
        if offset == 0 {
            continue;
        }
        if offset < 4 + count * 4 {
            return Err("chained segment starts overlap segment table".into());
        }
        let record = starts
            .get(offset..)
            .ok_or("chained segment starts outside payload")?;
        let size = rd_u32(record, 0)? as usize;
        let record = record
            .get(..size)
            .filter(|r| r.len() >= 22)
            .ok_or("truncated chained segment starts")?;
        let page_size = u16_at(record, 4)? as u64;
        let format = u16_at(record, 6)?;
        if !matches!(format, 2 | 6) {
            return Err(format!("unsupported chained pointer format {format}"));
        }
        if !matches!(page_size, 0x1000 | 0x4000) {
            return Err("invalid chained fixup page size".into());
        }
        let segment_offset = rd_u64(record, 8)?;
        if image_base.checked_add(segment_offset) != Some(segment.vmaddr)
            || segment.vmsize == 0
            || (segment.initprot == 0 && segment.filesize == 0)
        {
            return Err("chained fixups refer to an unmapped or mismatched segment".into());
        }
        let pages = u16_at(record, 20)? as usize;
        if pages > record.len().saturating_sub(22) / 2
            || (pages != 0 && (pages as u64 - 1) * page_size >= segment.vmsize)
        {
            return Err("chained page table exceeds segment".into());
        }
        for page in 0..pages {
            let start = u16_at(record, 22 + page * 2)?;
            if start == 0xffff {
                continue;
            }
            if start & 0x8000 != 0 {
                return Err("multiple chained starts per page are not supported".into());
            }
            let page_offset = page as u64 * page_size;
            let page_len = page_size.min(segment.vmsize - page_offset);
            let mut within = start as u64;
            loop {
                // A next field is strictly forward and limited to this page.
                // This also bounds the walk to at most page_size / 4 entries.
                if within % 4 != 0 || within.checked_add(8).filter(|v| *v <= page_len).is_none() {
                    return Err("chained pointer outside page or misaligned".into());
                }
                let address = segment
                    .vmaddr
                    .checked_add(page_offset)
                    .and_then(|v| v.checked_add(within))
                    .and_then(|v| v.checked_add(slide))
                    .ok_or("chained address overflow")?;
                if seen.contains(&address)
                    || address.checked_sub(4).is_some_and(|v| seen.contains(&v))
                    || address.checked_add(4).is_some_and(|v| seen.contains(&v))
                {
                    return Err("overlapping chained pointer locations".into());
                }
                seen.insert(address);
                let raw = cpu
                    .read_u64(address)
                    .ok_or("chained pointer outside guest memory")?;
                let value = if raw >> 63 != 0 {
                    if (raw >> 32) & 0x7ffff != 0 {
                        return Err("nonzero reserved chained bind bits".into());
                    }
                    let ordinal = (raw & 0xffffff) as usize;
                    let import = imports
                        .get(ordinal)
                        .ok_or("chained bind ordinal outside imports table")?;
                    let symbol = resolver(import)?;
                    // Apple's ordinary 64-bit embedded addend is unsigned
                    // (0..255); only the imports table addend is signed.
                    let addend = import.addend as i128 + ((raw >> 24) & 0xff) as i128;
                    u64::try_from(symbol as i128 + addend)
                        .map_err(|_| "chained bind target overflow")?
                } else {
                    if (raw >> 44) & 0x7f != 0 {
                        return Err("nonzero reserved chained pointer bits".into());
                    }
                    let low = raw & ((1u64 << 36) - 1);
                    let high = ((raw >> 36) & 0xff) << 56;
                    // Apple Loader.cpp/fixupPage64 computes target + adjust +
                    // high8, where adjust is mapped base for OFFSET, or slide.
                    let adjust = if format == 6 {
                        image_base
                            .checked_add(slide)
                            .ok_or("chained target overflow")?
                    } else {
                        slide
                    };
                    low.checked_add(adjust)
                        .and_then(|v| v.checked_add(high))
                        .ok_or("chained target overflow")?
                };
                writes.push((address, value));
                let next = (raw >> 51) & 0xfff;
                if next == 0 {
                    break;
                }
                // Stride is four, but 64-bit entries must not overlap.
                if next < 2 {
                    return Err("overlapping chained pointers".into());
                }
                within = within
                    .checked_add(next * 4)
                    .ok_or("chained next overflow")?;
            }
        }
    }
    for (address, value) in writes {
        cpu.write_bytes(address, &value.to_le_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a64::{EntryPoint, Segment64};

    fn fixture(format: u16) -> (Vec<u8>, MachO64, A64Cpu) {
        let base = 0x1_0000_0000;
        let macho = MachO64 {
            file_type: super::super::MH_EXECUTE,
            dependencies: Vec::new(),
            install_name: None,
            rpaths: Vec::new(),
            has_initializers: false,
            initializer_sections: Vec::new(),
            segments: vec![Segment64 {
                name: "__TEXT".into(),
                vmaddr: base,
                vmsize: 0x1000,
                fileoff: 0,
                filesize: 0x1000,
                initprot: 3,
            }],
            entry: EntryPoint::Main {
                entryoff: 0,
                stacksize: 0,
            },
            chained_fixups: None,
            legacy_fixups: None,
        };
        let mut data = vec![0; 60];
        data[4..8].copy_from_slice(&28u32.to_le_bytes());
        data[28..32].copy_from_slice(&1u32.to_le_bytes());
        data[32..36].copy_from_slice(&8u32.to_le_bytes());
        data[36..40].copy_from_slice(&24u32.to_le_bytes());
        data[40..42].copy_from_slice(&0x1000u16.to_le_bytes());
        data[42..44].copy_from_slice(&format.to_le_bytes());
        data[56..58].copy_from_slice(&1u16.to_le_bytes());
        data[58..60].copy_from_slice(&0x100u16.to_le_bytes());
        (data, macho, A64Cpu::new(base, 0x1000))
    }

    #[test]
    fn rebases_absolute_chain_and_high8() {
        let (data, macho, mut cpu) = fixture(2);
        cpu.write_bytes(0x1_0000_0100, &(0x1_0000_0200u64 | (2 << 51)).to_le_bytes());
        cpu.write_bytes(
            0x1_0000_0108,
            &(0x1_0000_0300u64 | (0xabu64 << 36)).to_le_bytes(),
        );
        apply(&data, &macho, &mut cpu).unwrap();
        assert_eq!(cpu.read_u64(0x1_0000_0100), Some(0x1_0000_0200));
        assert_eq!(cpu.read_u64(0x1_0000_0108), Some(0xab00_0001_0000_0300));
    }

    #[test]
    fn rebases_image_relative_pointer() {
        let (data, macho, mut cpu) = fixture(6);
        cpu.write_bytes(0x1_0000_0100, &0x200u64.to_le_bytes());
        apply(&data, &macho, &mut cpu).unwrap();
        assert_eq!(cpu.read_u64(0x1_0000_0100), Some(0x1_0000_0200));
    }

    #[test]
    fn rejects_bind_without_partial_rebase() {
        let (data, macho, mut cpu) = fixture(2);
        let raw = 0x1_0000_0200u64 | (2 << 51);
        cpu.write_bytes(0x1_0000_0100, &raw.to_le_bytes());
        cpu.write_bytes(0x1_0000_0108, &(1u64 << 63).to_le_bytes());
        assert!(apply(&data, &macho, &mut cpu).unwrap_err().contains("bind"));
        assert_eq!(cpu.read_u64(0x1_0000_0100), Some(raw));
    }

    #[test]
    fn rejects_page_escape_and_truncated_metadata() {
        let (mut data, macho, mut cpu) = fixture(2);
        cpu.write_bytes(0x1_0000_0100, &(0xfffu64 << 51).to_le_bytes());
        assert!(apply(&data, &macho, &mut cpu)
            .unwrap_err()
            .contains("outside page"));
        data.truncate(59);
        assert!(apply(&data, &macho, &mut cpu).is_err());
    }

    #[test]
    fn rejects_bad_import_format_and_unknown_pointer_formats() {
        let (mut data, macho, mut cpu) = fixture(1);
        assert!(apply(&data, &macho, &mut cpu)
            .unwrap_err()
            .contains("format"));
        data[16..20].copy_from_slice(&1u32.to_le_bytes());
        assert!(validate_header(&data)
            .unwrap_err()
            .contains("import format"));
    }

    fn add_import(data: &mut Vec<u8>, format: u32, ordinal: u16, weak: bool, addend: i64) {
        let offset = data.len();
        data[8..12].copy_from_slice(&(offset as u32).to_le_bytes());
        data[16..20].copy_from_slice(&1u32.to_le_bytes());
        data[20..24].copy_from_slice(&format.to_le_bytes());
        match format {
            1 | 2 => {
                data.extend_from_slice(&(ordinal as u32 | ((weak as u32) << 8)).to_le_bytes());
                if format == 2 {
                    data.extend_from_slice(&(addend as i32).to_le_bytes());
                }
            }
            3 => {
                data.extend_from_slice(&(ordinal as u64 | ((weak as u64) << 16)).to_le_bytes());
                data.extend_from_slice(&addend.to_le_bytes());
            }
            _ => unreachable!(),
        }
        let symbols = data.len();
        data[12..16].copy_from_slice(&(symbols as u32).to_le_bytes());
        data.extend_from_slice(b"_symbol\0");
    }

    #[test]
    fn parses_all_import_formats_and_ordinal_boundaries() {
        for (format, ordinal, expected_ordinal, addend) in [
            (1, 240, 240, 0),
            (2, 255, -1, -42),
            (3, 65520, 65520, i64::MIN),
            (3, 65534, -2, i64::MAX),
        ] {
            let (mut data, _, _) = fixture(2);
            add_import(&mut data, format, ordinal, true, addend);
            assert_eq!(
                parse_imports(&data).unwrap(),
                vec![ChainedImport {
                    library_ordinal: expected_ordinal,
                    weak: true,
                    name: "_symbol".into(),
                    addend,
                }]
            );
        }
    }

    #[test]
    fn binds_signed_table_and_unsigned_embedded_addends() {
        for format in [1, 2, 3] {
            let (mut data, macho, mut cpu) = fixture(2);
            let addend = if format == 1 { 0 } else { -32 };
            add_import(&mut data, format, 1, false, addend);
            let raw = (1u64 << 63) | (255 << 24);
            cpu.write_bytes(0x1_0000_0100, &raw.to_le_bytes());
            apply_with_resolver(&data, &macho, &mut cpu, 0, |import| {
                assert_eq!(import.library_ordinal, 1);
                assert_eq!(import.name, "_symbol");
                Ok(0x2000)
            })
            .unwrap();
            assert_eq!(
                cpu.read_u64(0x1_0000_0100),
                Some((0x2000i64 + addend + 255) as u64)
            );
        }
    }

    #[test]
    fn relocates_slots_and_rebases_without_sliding_bound_symbols() {
        for format in [2, 6] {
            let (mut data, macho, _) = fixture(format);
            add_import(&mut data, 1, 1, false, 0);
            let slide = 0x1000;
            let mut cpu = A64Cpu::new(0x1_0000_1000, 0x1000);
            let target = if format == 2 { 0x1_0000_0200u64 } else { 0x200 };
            cpu.write_bytes(0x1_0000_1100, &(target | (2 << 51)).to_le_bytes());
            cpu.write_bytes(0x1_0000_1108, &(1u64 << 63).to_le_bytes());
            apply_with_resolver(&data, &macho, &mut cpu, slide, |_| Ok(0x9999)).unwrap();
            assert_eq!(cpu.read_u64(0x1_0000_1100), Some(0x1_0000_1200));
            assert_eq!(cpu.read_u64(0x1_0000_1108), Some(0x9999));
        }
    }

    #[test]
    fn delegates_weak_resolution_and_preserves_writes_on_resolution_error() {
        let (mut data, macho, mut cpu) = fixture(2);
        add_import(&mut data, 1, 1, true, 0);
        cpu.write_bytes(0x1_0000_0100, &(1u64 << 63).to_le_bytes());
        apply_with_resolver(&data, &macho, &mut cpu, 0, |import| {
            assert!(import.weak);
            Ok(0)
        })
        .unwrap();
        assert_eq!(cpu.read_u64(0x1_0000_0100), Some(0));
        let raw = 0x1_0000_0200u64 | (2 << 51);
        cpu.write_bytes(0x1_0000_0100, &raw.to_le_bytes());
        cpu.write_bytes(0x1_0000_0108, &(1u64 << 63).to_le_bytes());
        let result =
            apply_with_resolver(&data, &macho, &mut cpu, 0, |_| Err("missing symbol".into()));
        assert_eq!(result.unwrap_err(), "missing symbol");
        assert_eq!(cpu.read_u64(0x1_0000_0100), Some(raw));
    }

    #[test]
    fn malformed_import_tables_names_and_bind_fields_do_not_write() {
        let (mut data, macho, mut cpu) = fixture(2);
        add_import(&mut data, 3, 1, false, 0);
        let mut broken = data.clone();
        broken[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate_header(&broken).is_err());
        let mut broken = data.clone();
        broken[60..68].copy_from_slice(&(1u64 << 17).to_le_bytes());
        assert!(validate_header(&broken).unwrap_err().contains("reserved"));
        let mut broken = data.clone();
        broken.pop();
        assert!(validate_header(&broken)
            .unwrap_err()
            .contains("unterminated"));
        let mut broken = data.clone();
        broken[64..68].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate_header(&broken)
            .unwrap_err()
            .contains("outside symbols"));
        for raw in [(1u64 << 63) | 1, (1u64 << 63) | (1 << 32)] {
            cpu.write_bytes(0x1_0000_0100, &raw.to_le_bytes());
            assert!(apply_with_resolver(&data, &macho, &mut cpu, 0, |_| Ok(100)).is_err());
            assert_eq!(cpu.read_u64(0x1_0000_0100), Some(raw));
        }
    }

    #[test]
    fn rejects_bind_addend_overflow_and_underflow() {
        for (table_addend, symbol, embedded) in [(i64::MAX, u64::MAX, 255), (-1, 0, 0)] {
            let (mut data, macho, mut cpu) = fixture(2);
            add_import(&mut data, 3, 1, false, table_addend);
            let raw = (1u64 << 63) | (embedded << 24);
            cpu.write_bytes(0x1_0000_0100, &raw.to_le_bytes());
            assert!(
                apply_with_resolver(&data, &macho, &mut cpu, 0, |_| Ok(symbol))
                    .unwrap_err()
                    .contains("overflow")
            );
            assert_eq!(cpu.read_u64(0x1_0000_0100), Some(raw));
        }
    }

    #[test]
    fn caps_import_count_name_length_and_total_copies() {
        let (mut data, _, _) = fixture(2);
        add_import(&mut data, 1, 1, false, 0);
        data[16..20].copy_from_slice(&((MAX_IMPORTS + 1) as u32).to_le_bytes());
        assert!(validate_header(&data)
            .unwrap_err()
            .contains("count exceeds"));
        data[16..20].copy_from_slice(&1u32.to_le_bytes());
        data.truncate(64);
        data.extend(std::iter::repeat(b'x').take(MAX_IMPORT_NAME + 1));
        data.push(0);
        assert!(validate_header(&data).unwrap_err().contains("name exceeds"));
        // A small symbol pool referenced repeatedly must not expand without a
        // bound into many copied names.
        let count = MAX_COPIED_IMPORT_NAMES / MAX_IMPORT_NAME + 1;
        let (mut data, _, _) = fixture(2);
        data[8..12].copy_from_slice(&60u32.to_le_bytes());
        data[16..20].copy_from_slice(&(count as u32).to_le_bytes());
        data[20..24].copy_from_slice(&1u32.to_le_bytes());
        data.resize(60 + count * 4, 0);
        let symbols_offset = data.len() as u32;
        data[12..16].copy_from_slice(&symbols_offset.to_le_bytes());
        data.extend(std::iter::repeat(b'x').take(MAX_IMPORT_NAME));
        data.push(0);
        assert!(validate_header(&data)
            .unwrap_err()
            .contains("copied chained import names"));
    }

    #[test]
    fn high8_is_added_to_relocated_target_in_both_formats() {
        let slide = 1u64 << 56;
        for format in [2, 6] {
            let (data, macho, _) = fixture(format);
            let mut cpu = A64Cpu::new(0x1_0000_0000 + slide, 0x1000);
            let target = if format == 2 { 0x1_0000_0200u64 } else { 0x200 };
            cpu.write_bytes(0x1_0000_0100 + slide, &(target | (1 << 36)).to_le_bytes());
            apply_with_resolver(&data, &macho, &mut cpu, slide, |_| unreachable!()).unwrap();
            assert_eq!(
                cpu.read_u64(0x1_0000_0100 + slide),
                Some(0x0200_0001_0000_0200)
            );
        }
    }
}

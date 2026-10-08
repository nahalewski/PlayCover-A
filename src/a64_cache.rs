/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Original shared-cache mapping plan, without loading or executing its code.
//! Format: Apple's include/mach-o/dyld_cache_format.h. Header offsets below
//! support monolithic arm64 caches and modern arm64 cache/subcache plans.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_METADATA: u64 = 64 * 1024 * 1024;
const MAX_TABLE: usize = 100_000;
const MAX_SLIDE_PAGES: usize = 1_000_000;
const MAX_CHAIN_WRITES: usize = 16_000_000;
const MAX_DECODE_MAPPING: usize = 512 * 1024 * 1024;
const MAX_IMAGE_NAME_BYTES: usize = 16 * 1024 * 1024;

fn fail<T>(message: &str) -> Result<T, String> {
    Err(message.to_owned())
}
fn check_range(offset: u64, size: u64, limit: u64) -> Result<(), String> {
    if offset > limit || size > limit - offset {
        return fail("cache range outside bounds");
    }
    Ok(())
}
fn u16_at(data: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = data
        .get(offset..offset.checked_add(2).ok_or("cache offset overflow")?)
        .ok_or("truncated cache u16")?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = data
        .get(offset..offset.checked_add(4).ok_or("cache offset overflow")?)
        .ok_or("truncated cache u32")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}
fn u64_at(data: &[u8], offset: usize) -> Result<u64, String> {
    let bytes = data
        .get(offset..offset.checked_add(8).ok_or("cache offset overflow")?)
        .ok_or("truncated cache u64")?;
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

struct CacheFile {
    path: PathBuf,
    file: File,
    size: u64,
    header: Vec<u8>,
    uuid: [u8; 16],
    mapping_offset: usize,
    region_start: u64,
    region_size: u64,
    legacy: bool,
}
impl CacheFile {
    fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        let mut cache = Self {
            path: path.to_owned(),
            file,
            size,
            header: Vec::new(),
            uuid: [0; 16],
            mapping_offset: 0,
            region_start: 0,
            region_size: 0,
            legacy: false,
        };
        let header = cache.read(0, 512)?;
        if &header[..16] != b"dyld_v1   arm64\0" {
            return fail("unsupported shared-cache magic");
        }
        cache.mapping_offset = u32_at(&header, 0x10)? as usize;
        if cache.mapping_offset < 0x98 {
            return fail("unsupported truncated arm64 cache header");
        }
        cache.legacy = cache.mapping_offset < 0x140;
        if !cache.legacy && cache.mapping_offset < 0x1c8 {
            return fail("unsupported intermediate arm64 cache header");
        }
        check_range(0, cache.mapping_offset as u64, size)?;
        cache.uuid.copy_from_slice(&header[0x58..0x68]);
        if !cache.legacy {
            cache.region_start = u64_at(&header, 0xe0)?;
            cache.region_size = u64_at(&header, 0xe8)?;
        }
        cache.header = header;
        Ok(cache)
    }
    fn read(&mut self, offset: u64, size: u64) -> Result<Vec<u8>, String> {
        check_range(offset, size, self.size)?;
        if size > MAX_METADATA {
            return fail("cache metadata read exceeds limit");
        }
        let mut data = vec![0; usize::try_from(size).map_err(|_| "metadata size overflow")?];
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        self.file
            .read_exact(&mut data)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(data)
    }
    fn table(&mut self, offset: u64, count: usize, stride: usize) -> Result<Vec<u8>, String> {
        if count > MAX_TABLE || (count != 0 && offset < self.mapping_offset as u64) {
            return fail("invalid or oversized cache metadata table");
        }
        self.read(
            offset,
            count.checked_mul(stride).ok_or("table size overflow")? as u64,
        )
    }
}

#[derive(Debug)]
pub(crate) struct CacheRegion {
    pub file: PathBuf,
    pub file_offset: u64,
    pub vmaddr: u64,
    pub size: u64,
    pub max_prot: u32,
    pub init_prot: u32,
    pub slide: Option<SlideInfoV2>,
}

#[derive(Debug, Clone)]
pub(crate) struct SlideInfoV2 {
    pub file_offset: u64,
    pub file_size: u64,
    pub page_size: usize,
    pub page_starts: Vec<u16>,
    pub page_extras: Vec<u16>,
    pub delta_mask: u64,
    pub value_add: u64,
}
impl SlideInfoV2 {
    fn parse(data: &[u8], mapping_size: u64, file_offset: u64) -> Result<Self, String> {
        if u32_at(data, 0)? != 2 {
            return fail("unsupported shared-cache slide info version");
        }
        let page_size = u32_at(data, 4)? as usize;
        let starts_offset = u32_at(data, 8)? as usize;
        let starts_count = u32_at(data, 12)? as usize;
        let extras_offset = u32_at(data, 16)? as usize;
        let extras_count = u32_at(data, 20)? as usize;
        let delta_mask = u64_at(data, 24)?;
        if !matches!(page_size, 4096 | 16384)
            || starts_count > MAX_SLIDE_PAGES
            || extras_count > MAX_SLIDE_PAGES
        {
            return fail("invalid or oversized slide v2 page table");
        }
        if mapping_size % page_size as u64 != 0
            || mapping_size / page_size as u64 != starts_count as u64
        {
            return fail("slide v2 page count does not match mapping");
        }
        let trailing = delta_mask.trailing_zeros();
        if delta_mask == 0 || trailing < 2 {
            return fail("invalid slide v2 delta mask");
        }
        let shifted = delta_mask >> trailing;
        if shifted & shifted.wrapping_add(1) != 0 {
            return fail("noncontiguous slide v2 delta mask");
        }
        if starts_offset < 40 || (extras_count != 0 && extras_offset < 40) {
            return fail("slide v2 tables overlap header");
        }
        check_range(
            starts_offset as u64,
            starts_count as u64 * 2,
            data.len() as u64,
        )?;
        check_range(
            extras_offset as u64,
            extras_count as u64 * 2,
            data.len() as u64,
        )?;
        if extras_count != 0
            && starts_offset < extras_offset + extras_count * 2
            && extras_offset < starts_offset + starts_count * 2
        {
            return fail("slide v2 tables overlap");
        }
        let info = Self {
            file_offset,
            file_size: data.len() as u64,
            page_size,
            page_starts: (0..starts_count)
                .map(|i| u16_at(data, starts_offset + i * 2))
                .collect::<Result<_, _>>()?,
            page_extras: (0..extras_count)
                .map(|i| u16_at(data, extras_offset + i * 2))
                .collect::<Result<_, _>>()?,
            delta_mask,
            value_add: u64_at(data, 32)?,
        };
        // Validation is linear even when several page starts share suffixes
        // in a long extras list. None represents an invalid/unclosed suffix.
        let mut suffixes = vec![None; info.page_extras.len()];
        for index in (0..info.page_extras.len()).rev() {
            let entry = info.page_extras[index];
            if entry & 0x4000 != 0 || (entry as usize & 0x3fff) * 4 + 8 > page_size {
                continue;
            }
            suffixes[index] = if entry & 0x8000 != 0 {
                Some(1usize)
            } else {
                suffixes.get(index + 1).copied().flatten().map(|n| n + 1)
            };
        }
        for &start in &info.page_starts {
            if start == 0x4000 {
                continue;
            }
            if start & 0x4000 != 0 {
                return fail("invalid slide v2 start flags");
            }
            if start & 0x8000 != 0 {
                if suffixes
                    .get(start as usize & 0x3fff)
                    .copied()
                    .flatten()
                    .is_none()
                {
                    return fail("invalid or unterminated slide v2 extras");
                }
            } else if start as usize * 4 + 8 > page_size {
                return fail("slide v2 start outside page");
            }
        }
        Ok(info)
    }
}

fn read_regions(cache: &mut CacheFile) -> Result<Vec<CacheRegion>, String> {
    let count = u32_at(&cache.header, 0x14)? as usize;
    let original = cache.table(cache.mapping_offset as u64, count, 32)?;
    if cache.legacy {
        let mut regions = Vec::with_capacity(count);
        for record in original.chunks_exact(32) {
            let vmaddr = u64_at(record, 0)?;
            let size = u64_at(record, 8)?;
            let file_offset = u64_at(record, 16)?;
            let max_prot = u32_at(record, 24)?;
            let init_prot = u32_at(record, 28)?;
            if size == 0
                || vmaddr.checked_add(size).is_none()
                || vmaddr % 4096 != 0
                || file_offset % 4096 != 0
                || size % 4096 != 0
                || max_prot & !7 != 0
                || init_prot & !max_prot != 0
            {
                return fail("invalid legacy cache mapping");
            }
            check_range(file_offset, size, cache.size)?;
            regions.push(CacheRegion {
                file: cache.path.clone(),
                file_offset,
                vmaddr,
                size,
                max_prot,
                init_prot,
                slide: None,
            });
        }
        let slide_offset = u64_at(&cache.header, 0x38)?;
        let slide_size = u64_at(&cache.header, 0x40)?;
        if slide_size != 0 {
            // Old dyld attaches its single slide table to the DATA mapping.
            // Require exactly one writable mapping rather than guessing by index.
            let writable: Vec<usize> = regions
                .iter()
                .enumerate()
                .filter_map(|(i, r)| (r.init_prot & 2 != 0).then_some(i))
                .collect();
            if writable.len() != 1 {
                return fail("legacy slide info requires exactly one DATA mapping");
            }
            let i = writable[0];
            regions[i].slide = Some(SlideInfoV2::parse(
                &cache.read(slide_offset, slide_size)?,
                regions[i].size,
                slide_offset,
            )?);
        } else if slide_offset != 0 {
            return fail("empty legacy slide info has nonzero offset");
        }
        return Ok(regions);
    }
    let slide_offset = u32_at(&cache.header, 0x138)? as u64;
    let slide_count = u32_at(&cache.header, 0x13c)? as usize;
    if count != slide_count {
        return fail("shared-cache mapping counts disagree");
    }
    let records = cache.table(slide_offset, count, 56)?;
    let mut regions = Vec::new();
    let mut slide_entries = 0usize;
    for i in 0..count {
        let record = &records[i * 56..(i + 1) * 56];
        let old = &original[i * 32..(i + 1) * 32];
        let vmaddr = u64_at(record, 0)?;
        let size = u64_at(record, 8)?;
        let file_offset = u64_at(record, 16)?;
        let max_prot = u32_at(record, 48)?;
        let init_prot = u32_at(record, 52)?;
        if vmaddr != u64_at(old, 0)?
            || size != u64_at(old, 8)?
            || file_offset != u64_at(old, 16)?
            || max_prot != u32_at(old, 24)?
            || init_prot != u32_at(old, 28)?
        {
            return fail("shared-cache mapping tables disagree");
        }
        if size == 0
            || vmaddr.checked_add(size).is_none()
            || vmaddr % 4096 != 0
            || file_offset % 4096 != 0
            || size % 4096 != 0
            || max_prot & !7 != 0
            || init_prot & !max_prot != 0
        {
            return fail("invalid cache mapping");
        }
        check_range(file_offset, size, cache.size)?;
        let slide_file_offset = u64_at(record, 24)?;
        let slide_file_size = u64_at(record, 32)?;
        let slide = if slide_file_size != 0 {
            Some(SlideInfoV2::parse(
                &cache.read(slide_file_offset, slide_file_size)?,
                size,
                slide_file_offset,
            )?)
        } else {
            if slide_file_offset != 0 {
                return fail("empty slide info has nonzero offset");
            }
            None
        };
        if let Some(info) = &slide {
            slide_entries += info.page_starts.len() + info.page_extras.len();
            if slide_entries > MAX_SLIDE_PAGES {
                return fail("cache slide metadata exceeds plan limit");
            }
        }
        regions.push(CacheRegion {
            file: cache.path.clone(),
            file_offset,
            vmaddr,
            size,
            max_prot,
            init_prot,
            slide,
        });
    }
    Ok(regions)
}

#[derive(Debug)]
pub(crate) struct CacheImage {
    pub address: u64,
    pub path: String,
}

#[derive(Debug)]
pub(crate) struct CachePlan {
    pub files: Vec<PathBuf>,
    pub image_count: usize,
    pub images: Vec<CacheImage>,
    pub mapped_bytes: u64,
    pub mapped_span: u64,
    pub regions: Vec<CacheRegion>,
}
impl CachePlan {
    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        let mut main = CacheFile::open(path)?;
        let mut regions = read_regions(&mut main)?;
        let base = if main.legacy {
            regions
                .iter()
                .map(|r| r.vmaddr)
                .min()
                .ok_or("cache has no mappings")?
        } else {
            main.region_start
        };
        let end = if main.legacy {
            regions
                .iter()
                .map(|r| r.vmaddr + r.size)
                .max()
                .ok_or("cache has no mappings")?
        } else {
            base.checked_add(main.region_size)
                .ok_or("cache region overflow")?
        };
        let sub_offset = if main.legacy {
            0
        } else {
            u32_at(&main.header, 0x188)? as u64
        };
        let sub_count = if main.legacy {
            0
        } else {
            u32_at(&main.header, 0x18c)? as usize
        };
        if sub_count > 1024 {
            return fail("cache companion count exceeds plan limit");
        }
        let entries = main.table(sub_offset, sub_count, 56)?;
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("invalid cache filename")?;
        let mut files = vec![path.to_owned()];
        let mut suffixes = std::collections::HashSet::new();
        for entry in entries.chunks_exact(56) {
            let vm_offset = u64_at(entry, 16)?;
            let terminator = entry[24..]
                .iter()
                .position(|b| *b == 0)
                .ok_or("unterminated cache suffix")?;
            let suffix = std::str::from_utf8(&entry[24..24 + terminator])
                .map_err(|_| "invalid cache suffix")?;
            if !suffix.starts_with('.')
                || suffix.len() < 2
                || suffix.contains("..")
                || !suffix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_'))
                || !suffixes.insert(suffix.to_owned())
            {
                return fail("unsafe or duplicate cache suffix");
            }
            let companion_path = path.with_file_name(format!("{filename}{suffix}"));
            let mut companion = CacheFile::open(&companion_path)?;
            if companion.uuid != entry[..16] {
                return fail("cache companion UUID mismatch");
            }
            let expected_base = base
                .checked_add(vm_offset)
                .ok_or("subcache base overflow")?;
            if !((companion.region_size == 0 && companion.region_start == expected_base)
                || (companion.region_start == base && companion.region_size == main.region_size))
            {
                return fail("subcache region mismatch");
            }
            let sub_regions = read_regions(&mut companion)?;
            if sub_regions.first().map(|r| r.vmaddr) != Some(expected_base) {
                return fail("subcache VM offset mismatch");
            }
            regions.extend(sub_regions);
            if regions.len() > MAX_TABLE
                || regions
                    .iter()
                    .filter_map(|r| r.slide.as_ref())
                    .map(|s| s.page_starts.len() + s.page_extras.len())
                    .sum::<usize>()
                    > MAX_SLIDE_PAGES
            {
                return fail("cache metadata exceeds bounded plan limit");
            }
            files.push(companion_path);
        }
        let symbol_uuid = &main.header[0x190..0x1a0];
        if !main.legacy && symbol_uuid.iter().any(|b| *b != 0) {
            let symbol_path = path.with_file_name(format!("{filename}.symbols"));
            let symbols = CacheFile::open(&symbol_path)?;
            if symbols.uuid != symbol_uuid {
                return fail("cache symbols UUID mismatch");
            }
            check_range(
                u64_at(&symbols.header, 0x48)?,
                u64_at(&symbols.header, 0x50)?,
                symbols.size,
            )?;
            files.push(symbol_path);
        }
        regions.sort_by_key(|r| r.vmaddr);
        let mut mapped_bytes = 0u64;
        let mut previous_end = base;
        for region in &regions {
            let region_end = region
                .vmaddr
                .checked_add(region.size)
                .ok_or("mapping end overflow")?;
            if region.vmaddr < previous_end || region_end > end {
                return fail("overlapping or out-of-region cache mappings");
            }
            previous_end = region_end;
            mapped_bytes = mapped_bytes
                .checked_add(region.size)
                .ok_or("mapped size overflow")?;
        }
        let mapped_span = match (regions.first(), regions.last()) {
            (Some(first), Some(last)) => last.vmaddr + last.size - first.vmaddr,
            _ => return fail("cache has no mappings"),
        };
        let image_offset = u32_at(&main.header, if main.legacy { 0x18 } else { 0x1c0 })? as u64;
        let image_count = u32_at(&main.header, if main.legacy { 0x1c } else { 0x1c4 })? as usize;
        let image_table = main.table(image_offset, image_count, 32)?;
        let mut images = Vec::with_capacity(image_count);
        let mut total_name_bytes = 0usize;
        for entry in image_table.chunks_exact(32) {
            let address = u64_at(entry, 0)?;
            let index = regions.partition_point(|r| r.vmaddr <= address);
            let region = index
                .checked_sub(1)
                .and_then(|i| regions.get(i))
                .ok_or("cache image header is unmapped")?;
            check_range(address - region.vmaddr, 32, region.size)?;
            let name_offset = u32_at(entry, 24)? as u64;
            check_range(name_offset, 1, main.size)?;
            let name = main.read(name_offset, 4097.min(main.size - name_offset))?;
            let length = name
                .iter()
                .position(|b| *b == 0)
                .ok_or("unterminated cache image name")?;
            if length == 0 || length > 4096 || std::str::from_utf8(&name[..length]).is_err() {
                return fail("invalid cache image name");
            }
            total_name_bytes = total_name_bytes
                .checked_add(length)
                .filter(|size| *size <= MAX_IMAGE_NAME_BYTES)
                .ok_or("cache image names exceed plan limit")?;
            images.push(CacheImage {
                address,
                path: std::str::from_utf8(&name[..length])
                    .map_err(|_| "invalid cache image name")?
                    .to_owned(),
            });
        }
        Ok(Self {
            files,
            image_count,
            images,
            mapped_bytes,
            mapped_span,
            regions,
        })
    }
}

/// Decode one file-backed DATA mapping into an existing guest buffer. This
/// pure helper does not allocate a mapping or execute instructions. Validate
/// every chain first so malformed metadata leaves the entire buffer unchanged.
#[allow(dead_code)]
pub(crate) fn decode_slide_v2(
    mapping: &mut [u8],
    info: &SlideInfoV2,
    slide: u64,
) -> Result<(), String> {
    if mapping.len() > MAX_DECODE_MAPPING {
        return fail("slide v2 mapping exceeds bounded decoder limit");
    }
    if !matches!(info.page_size, 4096 | 16384)
        || mapping.len() / info.page_size != info.page_starts.len()
        || mapping.len() % info.page_size != 0
        || info.delta_mask == 0
        || info.delta_mask.trailing_zeros() < 2
    {
        return fail("invalid slide v2 mapping dimensions or mask");
    }
    let shifted = info.delta_mask >> info.delta_mask.trailing_zeros();
    if shifted & shifted.wrapping_add(1) != 0 {
        return fail("noncontiguous slide v2 delta mask");
    }
    let shift = info.delta_mask.trailing_zeros() - 2;
    let mut writes = Vec::new();
    // One bit for each four-byte slot is enough to detect overlapping eight-
    // byte entries, at 1/32 of the mapping size rather than a HashSet per pointer.
    let mut seen_slots = vec![0u8; (mapping.len() / 4 + 7) / 8];
    let write_limit = MAX_CHAIN_WRITES.min(mapping.len() / 8);
    for (page, &start) in info.page_starts.iter().enumerate() {
        if start == 0x4000 {
            continue;
        }
        if start & 0x4000 != 0 {
            return fail("invalid slide v2 page flags");
        }
        let mut chain_starts = Vec::new();
        if start & 0x8000 != 0 {
            let mut index = start as usize & 0x3fff;
            loop {
                let extra = *info
                    .page_extras
                    .get(index)
                    .ok_or("unterminated slide v2 extras")?;
                if extra & 0x4000 != 0 {
                    return fail("invalid slide v2 extras flags");
                }
                chain_starts.push((extra as usize & 0x3fff) * 4);
                index += 1;
                if extra & 0x8000 != 0 {
                    break;
                }
                if chain_starts.len() > info.page_size / 4 {
                    return fail("too many page chains");
                }
            }
        } else {
            chain_starts.push(start as usize * 4);
        }
        for mut within in chain_starts {
            loop {
                if within
                    .checked_add(8)
                    .filter(|end| *end <= info.page_size)
                    .is_none()
                {
                    return fail("slide v2 pointer outside page");
                }
                let offset = page * info.page_size + within;
                let first_slot = offset / 4;
                for slot in [first_slot, first_slot + 1] {
                    let flag = 1u8 << (slot % 8);
                    if seen_slots[slot / 8] & flag != 0 {
                        return fail("overlapping slide v2 pointers");
                    }
                    seen_slots[slot / 8] |= flag;
                }
                let raw = u64_at(mapping, offset)?;
                let value = raw & !info.delta_mask;
                let target = if value == 0 {
                    0
                } else {
                    value
                        .checked_add(info.value_add)
                        .and_then(|v| v.checked_add(slide))
                        .ok_or("slide v2 target overflow")?
                };
                if writes.len() == write_limit {
                    return fail("slide v2 writes exceed bounded decoder limit");
                }
                writes.push((offset, target));
                let delta = (raw & info.delta_mask) >> shift;
                if delta == 0 {
                    break;
                }
                if delta < 8 {
                    return fail("overlapping slide v2 chain entries");
                }
                within = within
                    .checked_add(usize::try_from(delta).map_err(|_| "slide delta overflow")?)
                    .ok_or("slide delta overflow")?;
            }
        }
    }
    for (offset, target) in writes {
        mapping[offset..offset + 8].copy_from_slice(&target.to_le_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn info() -> SlideInfoV2 {
        SlideInfoV2 {
            file_offset: 0,
            file_size: 42,
            page_size: 4096,
            page_starts: vec![1],
            page_extras: vec![],
            delta_mask: 0x3ff0_0000_0000_0000,
            value_add: 0x180000000,
        }
    }
    #[test]
    fn decodes_pointer_chain_and_null_without_sliding_null() {
        let mut mapping = vec![0u8; 4096];
        mapping[4..12].copy_from_slice(&(0x100u64 | (2 << 52)).to_le_bytes());
        decode_slide_v2(&mut mapping, &info(), 0x4000).unwrap();
        assert_eq!(u64_at(&mapping, 4).unwrap(), 0x180004100);
        assert_eq!(u64_at(&mapping, 12).unwrap(), 0);
    }
    #[test]
    fn rejects_bad_chain_without_partial_writes() {
        let mut mapping = vec![0u8; 4096];
        let raw = 0x100u64 | (1023 << 52);
        mapping[4..12].copy_from_slice(&raw.to_le_bytes());
        assert!(decode_slide_v2(&mut mapping, &info(), 0).is_err());
        assert_eq!(u64_at(&mapping, 4).unwrap(), raw);
        let mut bad = info();
        bad.page_starts[0] = 0x8000;
        assert!(decode_slide_v2(&mut mapping, &bad, 0).is_err());
    }
    #[test]
    fn parses_and_bounds_slide_metadata() {
        let mut data = vec![0u8; 42];
        for (offset, value) in [(0, 2u32), (4, 4096), (8, 40), (12, 1), (16, 42)] {
            data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[24..32].copy_from_slice(&info().delta_mask.to_le_bytes());
        data[40..42].copy_from_slice(&1u16.to_le_bytes());
        assert!(SlideInfoV2::parse(&data, 4096, 100).is_ok());
        data.truncate(41);
        assert!(SlideInfoV2::parse(&data, 4096, 100).is_err());
        assert!(check_range(u64::MAX, 2, u64::MAX).is_err());
    }

    fn put32(data: &mut [u8], offset: usize, value: u32) {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(data: &mut [u8], offset: usize, value: u64) {
        data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn cache_fixture(base: u64, identity: u8, main: bool) -> Vec<u8> {
        let mut data = vec![0; 4096];
        data[..16].copy_from_slice(b"dyld_v1   arm64\0");
        data[0x58..0x68].fill(identity);
        put32(&mut data, 0x10, 512);
        put32(&mut data, 0x14, 1);
        put64(&mut data, 0xe0, base);
        put64(&mut data, 0xe8, if main { 65536 } else { 0 });
        put32(&mut data, 0x138, 544);
        put32(&mut data, 0x13c, 1);
        for offset in [512, 544] {
            put64(&mut data, offset, base);
            put64(&mut data, offset + 8, 4096);
        }
        put32(&mut data, 536, 5);
        put32(&mut data, 540, 5);
        put32(&mut data, 592, 5);
        put32(&mut data, 596, 5);
        if main {
            put32(&mut data, 0x1c0, 624);
            put32(&mut data, 0x1c4, 1);
            put64(&mut data, 624, base + 1024);
            put32(&mut data, 648, 656);
            let name = b"/usr/lib/test.dylib\0";
            data[656..656 + name.len()].copy_from_slice(name);
            put32(&mut data, 0x188, 704);
            put32(&mut data, 0x18c, 1);
            data[704..720].fill(1);
            put64(&mut data, 720, 4096);
            data[728..732].copy_from_slice(b".01\0");
        }
        data
    }
    #[test]
    fn reads_plan_and_rejects_bad_companion_identity_and_bounds() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("a64-cache-test-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let main_path = directory.join("dyld_shared_cache_arm64");
        let companion_path = directory.join("dyld_shared_cache_arm64.01");
        std::fs::write(&main_path, cache_fixture(0x180000000, 0, true)).unwrap();
        let mut companion = cache_fixture(0x180001000, 1, false);
        std::fs::write(&companion_path, &companion).unwrap();
        let plan = CachePlan::read(&main_path).unwrap();
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.image_count, 1);
        assert_eq!(plan.images.len(), 1);
        assert_eq!(plan.images[0].address, 0x180000400);
        assert_eq!(plan.images[0].path, "/usr/lib/test.dylib");
        assert_eq!(plan.mapped_bytes, 8192);
        assert_eq!(plan.mapped_span, 8192);
        companion[0x58] = 2;
        std::fs::write(&companion_path, &companion).unwrap();
        assert!(CachePlan::read(&main_path).unwrap_err().contains("UUID"));
        companion[0x58] = 1;
        put64(&mut companion, 528, 4096);
        put64(&mut companion, 560, 4096);
        std::fs::write(&companion_path, &companion).unwrap();
        assert!(CachePlan::read(&main_path).unwrap_err().contains("bounds"));
    }

    #[test]
    fn reads_monolithic_cache_and_rejects_ambiguous_slide_mapping() {
        let path =
            std::env::temp_dir().join(format!("a64-legacy-cache-{}.bin", std::process::id()));
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(path.clone());
        let mut data = vec![0u8; 12288];
        data[..16].copy_from_slice(b"dyld_v1   arm64\0");
        put32(&mut data, 0x10, 0x98);
        put32(&mut data, 0x14, 3);
        put32(&mut data, 0x18, 0x100);
        put32(&mut data, 0x1c, 1);
        put64(&mut data, 0x100, 0x180000400);
        put32(&mut data, 0x118, 0x120);
        let name = b"/usr/lib/libstdc++.6.dylib\0";
        data[0x120..0x120 + name.len()].copy_from_slice(name);
        for i in 0..3usize {
            let offset = 0x98 + i * 32;
            put64(&mut data, offset, 0x180000000 + i as u64 * 4096);
            put64(&mut data, offset + 8, 4096);
            put64(&mut data, offset + 16, i as u64 * 4096);
            put32(&mut data, offset + 24, if i == 1 { 3 } else { 5 });
            put32(&mut data, offset + 28, if i == 1 { 3 } else { 5 });
        }
        put64(&mut data, 0x38, 0x200);
        put64(&mut data, 0x40, 42);
        for (off, val) in [(0, 2u32), (4, 4096), (8, 40), (12, 1), (16, 42)] {
            put32(&mut data, 0x200 + off, val);
        }
        put64(&mut data, 0x218, info().delta_mask);
        put64(&mut data, 0x220, 0x180000000);
        data[0x228..0x22a].copy_from_slice(&0x4000u16.to_le_bytes());
        std::fs::write(&path, &data).unwrap();
        let plan = CachePlan::read(&path).unwrap();
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.images[0].path, "/usr/lib/libstdc++.6.dylib");
        assert_eq!(plan.mapped_bytes, 12288);
        assert!(plan.regions[1].slide.is_some());
        assert!(plan.regions[0].slide.is_none());
        put32(&mut data, 0x98 + 24, 3);
        put32(&mut data, 0x98 + 28, 3);
        std::fs::write(&path, &data).unwrap();
        assert!(CachePlan::read(&path)
            .unwrap_err()
            .contains("exactly one DATA"));
    }

    #[test]
    #[ignore = "requires explicitly supplied original iOS 11.4.1 cache"]
    fn actual_legacy_cache_plan() {
        let path =
            std::env::var_os("A64_LEGACY_CACHE_TEST_PATH").expect("set A64_LEGACY_CACHE_TEST_PATH");
        let plan = CachePlan::read(Path::new(&path)).unwrap();
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.image_count, 1318);
        assert_eq!(plan.regions.len(), 3);
        assert_eq!(plan.regions[0].vmaddr, 6442450944);
        assert_eq!(plan.mapped_bytes, 695894016 + 130678784 + 106332160);
        assert!(plan.regions[1].slide.is_some());
        assert!(plan
            .images
            .iter()
            .any(|image| image.path == "/usr/lib/libstdc++.6.dylib"));
    }

    #[test]
    #[ignore = "requires explicitly supplied extracted iOS 16.7 cache"]
    fn actual_cache_plan() {
        let path = std::env::var_os("A64_CACHE_TEST_PATH").expect("set A64_CACHE_TEST_PATH");
        let plan = CachePlan::read(Path::new(&path)).unwrap();
        assert_eq!(plan.files.len(), 44);
        assert_eq!(plan.image_count, 2705);
        assert_eq!(plan.images.len(), plan.image_count);
        assert!(plan
            .images
            .iter()
            .any(|image| image.path == "/usr/lib/libobjc.A.dylib"));
        assert_eq!(plan.regions.len(), 51);
        assert_eq!(plan.mapped_bytes, 2_731_491_328);
        assert_eq!(plan.mapped_span, 2_865_709_056);
    }
}

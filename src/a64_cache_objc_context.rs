/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Selector context derived from the original mapped libobjc optimization header.
use super::{cache::CachePlan, objc_metadata::CacheSelectorContext, A64Cpu};
const LIBOBJC: &str = "/usr/lib/libobjc.A.dylib";

pub(super) fn read(cpu: &A64Cpu, plan: &CachePlan) -> Result<CacheSelectorContext, String> {
    let providers = plan
        .images
        .iter()
        .filter(|image| image.path == LIBOBJC)
        .collect::<Vec<_>>();
    if providers.len() != 1 {
        return Err("selector context requires one original libobjc cache image".into());
    }
    let ranges = plan
        .regions
        .iter()
        .filter(|region| region.init_prot & 1 != 0)
        .map(|region| {
            Ok((
                region.vmaddr,
                region
                    .vmaddr
                    .checked_add(region.size)
                    .ok_or("selector cache range overflow")?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    read_image(cpu, providers[0].address, &ranges)
}
fn bytes(
    cpu: &A64Cpu,
    ranges: &[(u64, u64)],
    address: u64,
    length: usize,
) -> Result<Vec<u8>, String> {
    if length == 0
        || length > 256 * 1024
        || !ranges.iter().any(|&(start, end)| {
            address >= start
                && address
                    .checked_add(length as u64)
                    .is_some_and(|next| next <= end)
        })
    {
        return Err(
            "selector optimization metadata outside original readable cache mapping".into(),
        );
    }
    let mut result = vec![0; length];
    cpu.read_into(address, &mut result)?;
    Ok(result)
}
fn u32_at(b: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        b.get(offset..offset.checked_add(4).ok_or("selector field overflow")?)
            .ok_or("selector field truncated")?
            .try_into()
            .unwrap(),
    ))
}
fn u64_at(b: &[u8], offset: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        b.get(offset..offset.checked_add(8).ok_or("selector field overflow")?)
            .ok_or("selector field truncated")?
            .try_into()
            .unwrap(),
    ))
}
fn name(b: &[u8]) -> &[u8] {
    b.split(|&byte| byte == 0).next().unwrap_or_default()
}
fn read_image(
    cpu: &A64Cpu,
    image: u64,
    ranges: &[(u64, u64)],
) -> Result<CacheSelectorContext, String> {
    Ok(read_image_metadata(cpu,image,ranges)?.0)
}
pub(super) fn selector_table(cpu:&A64Cpu,plan:&CachePlan)->Result<u64,String>{
    let images=plan.images.iter().filter(|image|image.path==LIBOBJC).collect::<Vec<_>>();
    if images.len()!=1{return Err("selector table requires one original libobjc provider".into());}
    let ranges=plan.regions.iter().filter(|region|region.init_prot&1!=0).map(|region|Ok((region.vmaddr,region.vmaddr.checked_add(region.size).ok_or("selector region overflow")?))).collect::<Result<Vec<_>,String>>()?;
    let (_,optimization)=read_image_metadata(cpu,images[0].address,&ranges)?;
    let opt=bytes(cpu,&ranges,optimization,48)?;
    let relative=i32::from_le_bytes(opt[8..12].try_into().unwrap());
    if relative==0{return Err("original cache has no selector hash table".into());}
    let table=u64::try_from(optimization as i128+relative as i128).map_err(|_|"selector table offset overflow")?;
    bytes(cpu,&ranges,table,32)?;Ok(table)
}
fn read_image_metadata(cpu:&A64Cpu,image:u64,ranges:&[(u64,u64)])->Result<(CacheSelectorContext,u64),String>{
    let header = bytes(cpu, ranges, image, 32)?;
    if u32_at(&header, 0)? != 0xfeedfacf
        || u32_at(&header, 4)? != 0x0100000c
        || u32_at(&header, 12)? != 6
        || u32_at(&header, 24)? & 0x80000000 == 0
    {
        return Err("selector context requires original cached ARM64 MH_DYLIB header".into());
    }
    let commands = u32_at(&header, 16)? as usize;
    let command_bytes = u32_at(&header, 20)? as usize;
    if commands == 0 || commands > 4096 || command_bytes > 256 * 1024 - 32 {
        return Err("selector command budget invalid".into());
    }
    let file = bytes(cpu, ranges, image, 32 + command_bytes)?;
    let mut offset = 32;
    let mut identity = false;
    let mut optimization = None;
    for _ in 0..commands {
        let command = u32_at(&file, offset)?;
        let size = u32_at(&file, offset + 4)? as usize;
        let end = offset
            .checked_add(size)
            .filter(|&end| end <= file.len())
            .ok_or("selector load command outside header")?;
        if size < 8 || size & 7 != 0 {
            return Err("selector load command size invalid".into());
        }
        let data = &file[offset..end];
        if command == 0xd {
            let relative = u32_at(data, 8)? as usize;
            if size < 24 || relative < 24 || relative >= size || identity {
                return Err("selector libobjc identity command invalid".into());
            }
            let text = &data[relative..];
            let nul = text
                .iter()
                .position(|&b| b == 0)
                .ok_or("selector libobjc identity unterminated")?;
            if &text[..nul] != LIBOBJC.as_bytes() {
                return Err("selector cache provider identity is not libobjc".into());
            }
            identity = true;
        } else if command == 0x19 {
            if size < 72 {
                return Err("selector segment header truncated".into());
            }
            let count = u32_at(data, 64)? as usize;
            if count > 128
                || 72usize
                    .checked_add(
                        count
                            .checked_mul(80)
                            .ok_or("selector section count overflow")?,
                    )
                    .is_none_or(|required| required > size)
            {
                return Err("selector segment sections invalid".into());
            }
            for index in 0..count {
                let section = &data[72 + index * 80..72 + (index + 1) * 80];
                if name(&section[..16]) != b"__objc_opt_ro" {
                    continue;
                }
                let address = u64_at(section, 32)?;
                let length = u64_at(section, 40)?;
                let segment = u64_at(data, 24)?;
                let segment_size = u64_at(data, 32)?;
                if name(&data[8..24]) != b"__TEXT"
                    || name(&section[16..32]) != b"__TEXT"
                    || length < 48
                    || length > 1024 * 1024
                    || address < segment
                    || address.checked_add(length).is_none_or(|end| {
                        segment
                            .checked_add(segment_size)
                            .is_none_or(|limit| end > limit)
                    })
                    || optimization.is_some()
                {
                    return Err("selector optimization section identity/range invalid".into());
                }
                optimization = Some(address);
            }
        }
        offset = end;
    }
    if offset != file.len() || !identity {
        return Err("selector cached libobjc command identity mismatch".into());
    }
    let optimization =
        optimization.ok_or("original libobjc lacks selector optimization section")?;
    let opt = bytes(cpu, ranges, optimization, 48)?;
    if u32_at(&opt, 0)? != 16 {
        return Err("unsupported objc_opt_t selector version (requires16)".into());
    }
    let relative = i64::from_le_bytes(opt[40..48].try_into().unwrap());
    let base = u64::try_from(optimization as i128 + relative as i128)
        .map_err(|_| "selector relative base overflow")?;
    bytes(cpu, ranges, base, 1)?;
    let start = ranges
        .iter()
        .map(|range| range.0)
        .min()
        .ok_or("selector cache mapping list empty")?;
    let end = ranges
        .iter()
        .map(|range| range.1)
        .max()
        .ok_or("selector cache mapping list empty")?;
    let context = CacheSelectorContext { start, end, base };
    context.validate()?;
    Ok((context,optimization))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(version: u32, relative: i64) -> A64Cpu {
        let mut file = vec![0; 4096];
        for (at, value) in [
            (0, 0xfeedfacfu32),
            (4, 0x0100000c),
            (12, 6),
            (16, 2),
            (20, 208),
            (24, 0x80000000),
            (32, 0x19),
            (36, 152),
            (96, 1),
            (184, 0xd),
            (188, 56),
            (192, 24),
        ] {
            file[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        file[40..46].copy_from_slice(b"__TEXT");
        file[56..64].copy_from_slice(&0x1000u64.to_le_bytes());
        file[64..72].copy_from_slice(&0x1000u64.to_le_bytes());
        file[104..117].copy_from_slice(b"__objc_opt_ro");
        file[120..126].copy_from_slice(b"__TEXT");
        file[136..144].copy_from_slice(&0x1200u64.to_le_bytes());
        file[144..152].copy_from_slice(&48u64.to_le_bytes());
        file[208..208 + LIBOBJC.len()].copy_from_slice(LIBOBJC.as_bytes());
        file[512..516].copy_from_slice(&version.to_le_bytes());
        file[552..560].copy_from_slice(&relative.to_le_bytes());
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x1000, 4096, 1).unwrap();
        cpu.try_write_bytes(0x1000, &file).unwrap();
        cpu
    }
    #[test]
    fn derives_signed_selector_base_from_original_provider_metadata() {
        let cpu = fixture(16, -32);
        let context = read_image(&cpu, 0x1000, &[(0x1000, 0x2000)]).unwrap();
        assert_eq!(context.base, 0x11e0);
    }
    #[test]
    fn rejects_unknown_version_and_base_outside_cache() {
        assert!(read_image(&fixture(15, 0), 0x1000, &[(0x1000, 0x2000)]).is_err());
        assert!(read_image(&fixture(16, 0x10000), 0x1000, &[(0x1000, 0x2000)]).is_err());
    }
}

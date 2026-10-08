/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Map and validate original shared-cache bytes without executing Apple code.
use super::{cache, A64Cpu};
use std::{fs::File, path::Path};

pub(super) fn map(path: &Path) -> Result<(cache::CachePlan, A64Cpu), String> {
    let plan = cache::CachePlan::read(path)?;
    let mut cpu = A64Cpu::new_sparse();
    for region in &plan.regions {
        let file = File::open(&region.file).map_err(|e| e.to_string())?;
        // The verified research cache files must remain unchanged while these
        // private views exist. This command never writes or truncates them.
        unsafe {
            cpu.map_file(
                region.vmaddr,
                &file,
                region.file_offset,
                region
                    .size
                    .try_into()
                    .map_err(|_| "cache region too large")?,
                region.init_prot,
            )?;
        }
    }
    for region in &plan.regions {
        // Touch only the first word here. Remaining pages are faulted in by
        // the slide decoder as needed; no anonymous copy of the cache exists.
        cpu.read_u64(region.vmaddr)
            .ok_or("mapped cache region is unreadable")?;
        if let Some(info) = &region.slide {
            cpu.mutate_region(
                region.vmaddr,
                region
                    .size
                    .try_into()
                    .map_err(|_| "cache region too large")?,
                |bytes| cache::decode_slide_v2(bytes, info, 0),
            )?;
        }
    }
    for pair in plan.regions.windows(2) {
        let end = pair[0]
            .vmaddr
            .checked_add(pair[0].size)
            .ok_or("cache end overflow")?;
        if end < pair[1].vmaddr && cpu.read_bytes(end, 1).is_some() {
            return Err("shared-cache hole unexpectedly mapped".into());
        }
    }
    Ok((plan, cpu))
}

pub(super) fn test(path: &Path) -> Result<(), String> {
    let (plan, cpu) = map(path)?;
    let slid_regions = plan
        .regions
        .iter()
        .filter(|region| region.slide.is_some())
        .count();
    echo!("[a64] cache map test passed: {} images, {} regions, {} mapped bytes, {} slide-v2 mappings decoded; no Apple code executed", plan.image_count, plan.regions.len(), cpu.mapped_bytes(), slid_regions);
    Ok(())
}

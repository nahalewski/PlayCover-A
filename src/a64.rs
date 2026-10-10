/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! EXPERIMENTAL arm64 (AArch64) prototype. Only built with `--features a64`.
//!
//! This is a research prototype, not 64-bit app support. It contains:
//! - a minimal arm64 Mach-O parser (header, `LC_SEGMENT_64`, `LC_MAIN`,
//!   `LC_UNIXTHREAD`; fat binaries are reduced to their arm64 slice),
//! - a loader that maps segments into independent guest memory regions of an
//!   [A64Cpu] (dynarmic A64 frontend, see `src/cpu/dynarmic_wrapper/a64.cpp`),
//! - a tiny "kernel" that handles two Darwin syscalls (`write`, `exit`).
//!
//! Chained fixups and a bounded multiimage linker support ordinary ARM64
//! dylibs, exported symbols and imports. There are no frameworks or Objective-C, and
//! nothing here touches the normal 32-bit [crate::mem::Mem]/[crate::cpu::Cpu].
//! See A64_NOTES.md in the repository root.

use touchHLE_dynarmic_wrapper::a64::{A64Cpu, A64State};
pub(super) const MAX_INITIALIZERS_PER_IMAGE:u64=65536;

#[path = "a64_bridge.rs"]
mod bridge;
#[path = "a64_image_infos.rs"]
mod image_infos;
#[path = "a64_loader_lock.rs"]
mod loader_lock;
#[path = "a64_tlv_lazy.rs"]
mod tlv_lazy;
#[path = "a64_execution_session.rs"]
mod execution_session;
#[path = "a64_credentials.rs"]
mod credentials;
#[path = "a64_dyld_sdk_query.rs"]
mod dyld_sdk_query;
#[path = "a64_dyld_cache_range.rs"]
mod dyld_cache_range;
#[path = "a64_dyld_overridden.rs"]
mod dyld_overridden;
#[path = "a64_dyld_add_image.rs"]
mod dyld_add_image;
#[path = "a64_dyld_objc.rs"]
mod dyld_objc;
#[path = "a64_restartable.rs"]
mod restartable;
#[path = "a64_dyld_objc_callbacks.rs"]
mod dyld_objc_callbacks;
#[path = "a64_dyld_bootstrap_association.rs"]
mod dyld_bootstrap_association;
#[path = "a64_cached_session.rs"]
mod cached_session;
#[path = "a64_legacy_session.rs"]
mod legacy_session;
#[path = "a64_legacy_dyld_lookup.rs"]
mod legacy_dyld_lookup;
#[path = "a64_legacy_add_images.rs"]
mod legacy_add_images;
#[path = "a64_legacy_objc_notify.rs"]
mod legacy_objc_notify;
#[path = "a64_legacy_initializer_budget.rs"]
mod legacy_initializer_budget;
#[cfg(test)]
#[path = "a64_legacy_pthread_key_tests.rs"]
mod legacy_pthread_key_tests;
#[cfg(test)]
#[path = "a64_legacy_tlv_callback_tests.rs"]
mod legacy_tlv_callback_tests;
#[path = "a64_legacy_mach_msg.rs"]
mod legacy_mach_msg;
#[path = "a64_thread_identity.rs"]
mod thread_identity;
#[path = "a64_thread_storage.rs"]
mod thread_storage;
#[path = "a64_legacy_pthread_registration.rs"]
mod legacy_pthread_registration;
#[path = "a64_initializer_budget.rs"]
mod initializer_budget;
#[path = "a64_dyld_selectors.rs"]
mod dyld_selectors;
#[path = "a64_dyld_main_header.rs"]
mod dyld_main_header;
#[path = "a64_timebase.rs"]
mod timebase;
#[path = "a64_cf_number.rs"]
mod cf_number;
#[path = "a64_cf_number_services.rs"]
mod cf_number_services;
#[path = "a64_cf_dictionary.rs"]
mod cf_dictionary;
#[path = "a64_objc_arc_services.rs"]
mod objc_arc_services;
#[cfg(test)]
#[path = "a64_objc_arc_services_tests.rs"]
mod objc_arc_services_tests;
#[path = "a64_cache.rs"]
mod cache;
#[path = "a64_cache_imports.rs"]
mod cache_imports;
#[path = "a64_cache_linker.rs"]
mod cache_linker;
#[path = "a64_cache_objc_context.rs"]
mod cache_objc_context;
#[path = "a64_cache_initializers.rs"]
mod cache_initializers;
#[path = "a64_cache_init_probe.rs"]
mod cache_init_probe;
#[path = "a64_commpage_ro.rs"]
mod commpage_ro;
#[path = "a64_mach_identity.rs"]
mod mach_identity;
#[path = "a64_mach_vm.rs"]
mod mach_vm;
#[path = "a64_mach_port_construct.rs"]
mod mach_port_construct;
#[path = "a64_mach_host_info.rs"]
mod mach_host_info;
#[path = "a64_mach_atm.rs"]
mod mach_atm;
#[path = "a64_mach_clock.rs"]
mod mach_clock;
#[path = "a64_mach_semaphore.rs"]
mod mach_semaphore;
#[path = "a64_entropy_fd.rs"]
mod entropy_fd;
#[path = "a64_standard_fds.rs"]
mod standard_fds;
#[path = "a64_dyld_slide.rs"]
mod dyld_slide;
#[path = "a64_dyld_helpers.rs"]
mod dyld_helpers;
#[path = "a64_tlv.rs"]
mod tlv;
#[path = "a64_tlv_bootstrap.rs"]
mod tlv_bootstrap;
#[path = "a64_legacy_cpp.rs"]
mod legacy_cpp;
#[path = "a64_tlv_storage.rs"]
mod tlv_storage;
#[cfg(test)]
#[path = "a64_tlv_storage_tests.rs"]
mod tlv_storage_tests;
#[cfg(test)]
#[path = "a64_tlv_tests.rs"]
mod tlv_tests;
#[cfg(test)]
#[path = "a64_dyld_slide_tests.rs"]
mod dyld_slide_tests;
#[path = "a64_mprotect.rs"]
mod mprotect;
#[cfg(test)]
#[path = "a64_mprotect_tests.rs"]
mod mprotect_tests;
#[cfg(test)]
#[path = "a64_protection_backend_tests.rs"]
mod protection_backend_tests;
#[path = "a64_posix_shm.rs"]
mod posix_shm;
#[path = "a64_sysctl.rs"]
mod sysctl;
#[path = "a64_main_stack.rs"]
mod main_stack;
#[path = "a64_pthread_registration.rs"]
mod pthread_registration;
#[path = "a64_bsdthread_ctl.rs"]
mod bsdthread_ctl;
#[cfg(test)]
#[path = "a64_native_initializer_tests.rs"]
mod native_initializer_tests;
#[path = "a64_cache_map.rs"]
mod cache_map;
#[path = "a64_cache_resolver.rs"]
mod cache_resolver;
#[path = "a64_cache_symbols.rs"]
mod cache_symbols;
#[path = "a64_cf.rs"]
mod cf;
#[path = "a64_cf_array.rs"]
mod cf_array;
#[path = "a64_cf_data.rs"]
mod cf_data;
#[path = "a64_cf_services.rs"]
mod cf_services;
#[path = "a64_cf_terraria_services.rs"]
mod cf_terraria_services;
#[path = "a64_cf_text.rs"]
mod cf_text;
#[path = "a64_cf_uuid.rs"]
mod cf_uuid;
#[path = "a64_commpage.rs"]
mod commpage;
#[path = "a64_exports.rs"]
mod exports;
#[path = "a64_fixups.rs"]
mod fixups;
#[path = "a64_host_services.rs"]
mod host_services;
#[path = "a64_legacy.rs"]
mod legacy;
#[path = "a64_objc_dealloc.rs"]
mod objc_dealloc;
#[path = "a64_objc_heap.rs"]
mod objc_heap;
#[path = "a64_objc_execution.rs"]
mod objc_execution;
#[path = "a64_objc_publication.rs"]
mod objc_publication;
#[path = "a64_objc_execution_services.rs"]
mod objc_execution_services;
#[path = "a64_objc_registration.rs"]
mod objc_registration;
#[path = "a64_objc_namespace.rs"]
mod objc_namespace;
#[path = "a64_objc_slots.rs"]
mod objc_slots;
#[path = "a64_nsstring_services.rs"]
mod nsstring_services;
#[path = "a64_bundle_services.rs"]
mod bundle_services;
#[path = "a64_bundle_load.rs"]
mod bundle_load;
#[path = "a64_objc_image_load.rs"]
mod objc_image_load;
#[path = "a64_objc_cached_root.rs"]
mod objc_cached_root;
#[path = "a64_foundation_startup.rs"]
mod foundation_startup;
#[path = "a64_pthread_create_prepare.rs"]
mod pthread_create_prepare;
#[path = "a64_thread_register_tests.rs"]
mod thread_register_tests;
#[path = "a64_bundle.rs"]
mod bundle_metadata;
#[path = "a64_thread_scheduler_cpu.rs"]
mod thread_scheduler_cpu;
#[path = "a64_thread_publication.rs"]
mod thread_publication;
#[path = "a64_scheduled_pthread_services.rs"]
mod scheduled_pthread_services;
#[path = "a64_objc_lifetime.rs"]
mod objc_lifetime;
#[path = "a64_objc_lifetime_services.rs"]
mod objc_lifetime_services;
#[path = "a64_objc.rs"]
mod objc_metadata;
#[path = "a64_runtime.rs"]
mod runtime;
#[path = "a64_startup.rs"]
mod startup;
#[path = "a64_pthread_tls_services.rs"]
mod pthread_tls_services;
#[path = "a64_pthread_mutex_services.rs"]
mod pthread_mutex_services;
#[path = "a64_pthread_cond_services.rs"]
mod pthread_cond_services;

pub fn cache_prepare(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &std::path::Path,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    cache_linker::prepare_with_reader(bytes, executable_path, cache_path, reader).map(|_| ())
}

pub fn cache_services_test(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &std::path::Path,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    cache_linker::selected_service_test(bytes, executable_path, cache_path, reader)
}
pub fn cache_initializer_test(
    bytes: &[u8], executable_path: &str, cache_path: &std::path::Path,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    cache_linker::initializer_test(bytes, executable_path, cache_path, reader)
}
pub fn cache_bundle_initializer_test(
    bytes: &[u8], executable_path: &str, cache_path: &std::path::Path,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>, with_unity: bool,
) -> Result<(), String> {
    cache_linker::bundle_initializer_test(bytes, executable_path, cache_path, reader, with_unity)
}
pub fn cache_legacy_cpp_prepare(
    bytes: &[u8], executable_path: &str, cache_path: &std::path::Path,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    legacy_cpp::prepare(bytes, executable_path, cache_path, reader)
}
/// Explicit loader-owned image-info compatibility preparation. Full Apple
/// runtime initialization remains required before normal application execution.
pub fn cache_image_info_prepare(
    bytes:&[u8],executable_path:&str,cache_path:&std::path::Path,
    reader:impl FnMut(&str)->Result<Vec<u8>,String>,
) -> Result<(),String> {
    cache_linker::prepare_with_reader_image_infos(bytes,executable_path,cache_path,reader).map(|_|())
}
/// Persistent process ownership diagnostic; a provider return is distinct from
/// full runtime readiness and never bypasses ordinary main's initialization gate.
pub fn cache_session_initializer_test(
    bytes:&[u8],executable_path:&str,cache_path:&std::path::Path,
    reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool,
) -> Result<(),String> {
    cached_session::diagnostic(bytes,executable_path,cache_path,reader,with_unity)
}
/// Explicit compatibility diagnostic retaining the real loader-owned snapshot.
pub fn cache_session_image_info_test(
    bytes:&[u8],executable_path:&str,cache_path:&std::path::Path,
    reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool,
) -> Result<(),String> {
    cached_session::diagnostic_image_infos(bytes,executable_path,cache_path,reader,with_unity)
}
pub fn cache_entry_prefix_test(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &std::path::Path,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    cache_linker::entry_prefix_test(bytes, executable_path, cache_path, reader)
}
#[path = "a64_linker.rs"]
mod linker;

pub fn cache_map_test(path: &std::path::Path) -> Result<(), String> {
    cache_map::test(path)
}

pub fn cache_resolver_test(path: &std::path::Path) -> Result<(), String> {
    cache_resolver::test(path)
}

pub fn cache_import_test(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &std::path::Path,
    reader: impl FnMut(&str) -> Result<Option<Vec<u8>>, String>,
) -> Result<(), String> {
    cache_imports::inspect(bytes, executable_path, cache_path, reader)
}

const MH_MAGIC_64: u32 = 0xfeedfacf;
const FAT_MAGIC: u32 = 0xcafebabe; // stored big-endian
const CPU_TYPE_ARM64: u32 = 0x0100000c;
const MH_EXECUTE: u32 = 0x2;
const MH_DYLIB: u32 = 0x6;
const LC_SEGMENT_64: u32 = 0x19;
const LC_UNIXTHREAD: u32 = 0x5;
const LC_MAIN: u32 = 0x80000028;
const ARM_THREAD_STATE64: u32 = 6;

/// Prefer the existing ARM32 runtime for universal binaries containing ARM32.
pub fn is_arm64_only(bytes: &[u8]) -> Result<bool, String> {
    if rd_be_u32(bytes, 0)? == FAT_MAGIC {
        let count = rd_be_u32(bytes, 4)? as usize;
        if count > bytes.len().saturating_sub(8) / 20 {
            return Err("truncated fat architecture table".into());
        }
        let mut arm64 = false;
        for i in 0..count {
            match rd_be_u32(bytes, 8 + i * 20)? {
                12 => return Ok(false),
                CPU_TYPE_ARM64 => arm64 = true,
                _ => (),
            }
        }
        return Ok(arm64);
    }
    Ok(rd_u32(bytes, 0)? == MH_MAGIC_64 && rd_u32(bytes, 4)? == CPU_TYPE_ARM64)
}

#[derive(Debug, Clone)]
pub struct Segment64 {
    pub name: String,
    pub vmaddr: u64,
    pub vmsize: u64,
    pub fileoff: u64,
    pub filesize: u64,
    pub initprot: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryPoint {
    Dylib,
    /// `LC_MAIN`: offset of `main` from the start of `__TEXT`.
    Main {
        entryoff: u64,
        stacksize: u64,
    },
    /// `LC_UNIXTHREAD` with `ARM_THREAD_STATE64`.
    UnixThread {
        pc: u64,
        sp: u64,
    },
}

#[derive(Debug, Clone)]
pub struct MachO64 {
    pub segments: Vec<Segment64>,
    pub entry: EntryPoint,
    chained_fixups: Option<(usize, usize)>,
    legacy_fixups: Option<[(usize, usize); 4]>,
    pub file_type: u32,
    pub dependencies: Vec<Dependency64>,
    pub install_name: Option<String>,
    pub rpaths: Vec<String>,
    has_initializers: bool,
    // Preferred table address, element count and width (8-byte pointers or
    // 4-byte image-relative offsets emitted by modern linkers).
    initializer_sections: Vec<(u64, u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct Dependency64 {
    pub name: String,
    pub weak: bool,
    pub reexport: bool,
}

fn command_string(
    b: &[u8],
    off: usize,
    end: usize,
    field: usize,
    minimum: usize,
) -> Result<String, String> {
    let relative = rd_u32(b, off + field)? as usize;
    if relative < minimum {
        return Err("invalid load-command string offset".into());
    }
    let start = off
        .checked_add(relative)
        .filter(|start| *start < end)
        .ok_or("load-command string outside command")?;
    let raw = &b[start..end];
    let length = raw
        .iter()
        .position(|byte| *byte == 0)
        .ok_or("unterminated load-command string")?;
    String::from_utf8(raw[..length].to_vec())
        .map_err(|_| "invalid UTF-8 load-command string".into())
}

fn rd_u32(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| format!("truncated Mach-O at {off:#x}"))
}
fn rd_u64(b: &[u8], off: usize) -> Result<u64, String> {
    b.get(off..off + 8)
        .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| format!("truncated Mach-O at {off:#x}"))
}
fn rd_be_u32(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_be_bytes(s.try_into().unwrap()))
        .ok_or_else(|| format!("truncated fat header at {off:#x}"))
}

/// If `bytes` is a fat (universal) binary, return its arm64 slice.
pub fn thin_arm64_slice(bytes: &[u8]) -> Result<&[u8], String> {
    if rd_be_u32(bytes, 0)? != FAT_MAGIC {
        return Ok(bytes);
    }
    let nfat = rd_be_u32(bytes, 4)? as usize;
    for i in 0..nfat {
        let a = 8 + i * 20;
        if rd_be_u32(bytes, a)? == CPU_TYPE_ARM64 {
            let off = rd_be_u32(bytes, a + 8)? as usize;
            let size = rd_be_u32(bytes, a + 12)? as usize;
            return bytes
                .get(off..off + size)
                .ok_or_else(|| "fat arm64 slice out of range".to_string());
        }
    }
    Err("fat binary has no arm64 slice".to_string())
}

impl MachO64 {
    pub fn parse(bytes: &[u8]) -> Result<MachO64, String> {
        let image = Self::parse_image(bytes)?;
        if image.file_type != MH_EXECUTE {
            return Err("standalone loader requires MH_EXECUTE".into());
        }
        if !image.dependencies.is_empty() {
            return Err(
                "This ARM64 app needs dynamic libraries; use the experimental library resolver."
                    .into(),
            );
        }
        Ok(image)
    }

    /// Parse independently loadable ARM64 images for the experimental linker.
    pub fn parse_image(bytes: &[u8]) -> Result<MachO64, String> {
        Self::parse_image_impl(bytes)
    }

    /// Audit metadata only: legacy dyld streams are range-checked but not
    /// interpreted, rebound or applied. This is not an executable loader path.
    pub(super) fn parse_metadata(bytes: &[u8]) -> Result<MachO64, String> {
        Self::parse_image_impl(bytes)
    }

    fn parse_image_impl(bytes: &[u8]) -> Result<MachO64, String> {
        let b = thin_arm64_slice(bytes)?;
        if rd_u32(b, 0)? != MH_MAGIC_64 {
            return Err("not a 64-bit little-endian Mach-O".to_string());
        }
        if rd_u32(b, 4)? != CPU_TYPE_ARM64 {
            return Err(format!("cputype {:#x} is not arm64", rd_u32(b, 4)?));
        }
        let file_type = rd_u32(b, 12)?;
        if file_type != MH_EXECUTE && file_type != MH_DYLIB {
            return Err("only MH_EXECUTE and MH_DYLIB are supported".to_string());
        }
        if rd_u32(b, 24)? & 0x8000_0000 != 0 {
            return Err("MH_DYLIB_IN_CACHE images are not independently loadable; extracted Apple runtime libraries still require shared-cache runtime support".into());
        }
        if rd_u32(b, 24)? & 0x20 != 0 {
            return Err(
                "MH_SPLIT_SEGS images require unsupported shared-cache/split-segment relocation"
                    .into(),
            );
        }
        let ncmds = rd_u32(b, 16)?;
        let command_end = 32usize
            .checked_add(rd_u32(b, 20)? as usize)
            .filter(|end| *end <= b.len())
            .ok_or("truncated load commands")?;
        let mut off = 32; // sizeof(mach_header_64)
        let mut segments = Vec::new();
        let mut entry = None;
        let mut chained_fixups = None;
        let mut legacy_fixups = None;
        let mut dependencies = Vec::new();
        let mut install_name = None;
        let mut rpaths = Vec::new();
        let mut has_initializers = false;
        let mut initializer_sections = Vec::new();
        let mut initializer_count=0u64;
        for _ in 0..ncmds {
            let cmd = rd_u32(b, off)?;
            let cmdsize = rd_u32(b, off + 4)? as usize;
            if cmdsize < 8 {
                return Err("bad load command size".to_string());
            }
            let end = off
                .checked_add(cmdsize)
                .filter(|end| *end <= command_end)
                .ok_or("load command exceeds declared command area")?;
            let minimum = match cmd {
                LC_SEGMENT_64 => 72,
                LC_MAIN => 24,
                LC_UNIXTHREAD => 288,
                0xc | 0xd | 0x18 | 0x80000018 | 0x8000001f | 0x80000023 => 24,
                0x8000001c => 12,
                _ => 8,
            };
            if cmdsize < minimum {
                return Err("truncated load command".into());
            }
            match cmd {
                0xc | 0x18 | 0x80000018 | 0x8000001f | 0x80000023 => {
                    dependencies.push(Dependency64 {
                        name: command_string(b, off, end, 8, 24)?,
                        weak: matches!(cmd, 0x18 | 0x80000018),
                        reexport: cmd == 0x8000001f,
                    });
                }
                0xd => {
                    if install_name.is_some() {
                        return Err("multiple LC_ID_DYLIB commands".into());
                    }
                    install_name = Some(command_string(b, off, end, 8, 24)?);
                }
                0x8000001c => rpaths.push(command_string(b, off, end, 8, 12)?),
                0x22 | 0x80000022 => {
                    if cmdsize < 48 {
                        return Err("truncated dyld info command".into());
                    }
                    // Retain classic relocation streams; application happens
                    // only after the entire image and dependency graph validate.
                    for field in [8, 16, 24, 32, 40] {
                        let start = rd_u32(b, off + field)? as usize;
                        let size = rd_u32(b, off + field + 4)? as usize;
                        if size != 0
                            && (start < command_end
                                || start
                                    .checked_add(size)
                                    .filter(|end| *end <= b.len())
                                    .is_none())
                        {
                            return Err("dyld info stream outside image payload".into());
                        }
                    }
                    let mut streams = [(0, 0); 4];
                    for (stream, field) in streams.iter_mut().zip([8, 16, 24, 32]) {
                        *stream = (
                            rd_u32(b, off + field)? as usize,
                            rd_u32(b, off + field + 4)? as usize,
                        );
                    }
                    if streams.iter().any(|(_, size)| *size != 0) {
                        if legacy_fixups.replace(streams).is_some() {
                            return Err("multiple legacy dyld relocation commands".into());
                        }
                    }
                }
                0x80000034 => {
                    if cmdsize < 16 {
                        return Err("truncated chained fixups command".into());
                    }
                    let size = rd_u32(b, off + 12)? as usize;
                    if size != 0 {
                        if chained_fixups.is_some() {
                            return Err("multiple chained fixups commands".into());
                        }
                        let start = rd_u32(b, off + 8)? as usize;
                        let end = start.checked_add(size).ok_or("fixups range overflow")?;
                        let data = b.get(start..end).ok_or("chained fixups outside file")?;
                        fixups::validate_header(data)?;
                        chained_fixups = Some((start, end));
                    }
                }
                0x21 | 0x2c => {
                    if cmdsize < 20 {
                        return Err("truncated encryption command".into());
                    }
                    if rd_u32(b, off + 16)? != 0 {
                        return Err("Encrypted ARM64 executables are not supported".into());
                    }
                }
                LC_SEGMENT_64 => {
                    let sections = rd_u32(b, off + 64)? as usize;
                    if sections > (cmdsize - 72) / 80 {
                        return Err("truncated segment sections".into());
                    }
                    for section in 0..sections {
                        let section_off = off + 72 + section * 80;
                        let name = &b[section_off..section_off + 16];
                        let size = rd_u64(b, section_off + 40)?;
                        let flags = rd_u32(b, section_off + 64)?;
                        if size != 0
                            && (name.starts_with(b"__mod_init_func")
                                || matches!(flags & 0xff, 9 | 0x16))
                        {
                            has_initializers = true;
                            let width = if flags & 0xff == 0x16 { 4 } else { 8 };
                            let address = rd_u64(b, section_off + 32)?;
                            let file_offset = rd_u32(b, section_off + 48)? as u64;
                            let segment_address = rd_u64(b, off + 24)?;
                            let segment_file_offset = rd_u64(b, off + 40)?;
                            let segment_file_size = rd_u64(b, off + 48)?;
                            let relative = address
                                .checked_sub(segment_address)
                                .ok_or("initializer section precedes its segment")?;
                            // AION2 has 27498 actual 32-bit initializer offsets.
                            // Parsing a bounded table does not relax execution ticks.
                            if address % width != 0 || size % width != 0 {
                                return Err("invalid initializer array alignment or size".into());
                            }
                            initializer_count=initializer_count.checked_add(size/width)
                                .ok_or("initializer count overflow")?;
                            if initializer_count>MAX_INITIALIZERS_PER_IMAGE {
                                return Err("initializer table exceeds per-image budget: 65536".into());
                            }
                            if relative
                                .checked_add(size)
                                .filter(|end| *end <= segment_file_size)
                                .is_none()
                                || segment_file_offset.checked_add(relative) != Some(file_offset)
                            {
                                return Err("initializer array outside file-backed segment".into());
                            }
                            initializer_sections.push((address, size / width, width));
                        }
                    }
                    let name_bytes = b.get(off + 8..off + 24).ok_or("truncated segname")?;
                    let name = String::from_utf8_lossy(name_bytes)
                        .trim_end_matches('\0')
                        .to_string();
                    segments.push(Segment64 {
                        name,
                        vmaddr: rd_u64(b, off + 24)?,
                        vmsize: rd_u64(b, off + 32)?,
                        fileoff: rd_u64(b, off + 40)?,
                        filesize: rd_u64(b, off + 48)?,
                        initprot: rd_u32(b, off + 60)?,
                    });
                }
                LC_MAIN => {
                    if entry.is_some() {
                        return Err("multiple entry points".into());
                    }
                    entry = Some(EntryPoint::Main {
                        entryoff: rd_u64(b, off + 8)?,
                        stacksize: rd_u64(b, off + 16)?,
                    });
                }
                LC_UNIXTHREAD => {
                    if rd_u32(b, off + 8)? != ARM_THREAD_STATE64 {
                        return Err("LC_UNIXTHREAD flavor is not ARM_THREAD_STATE64".into());
                    }
                    // x0..x28, fp, lr, sp, pc
                    let state = off + 16;
                    if entry.is_some() {
                        return Err("multiple entry points".into());
                    }
                    entry = Some(EntryPoint::UnixThread {
                        sp: rd_u64(b, state + 31 * 8)?,
                        pc: rd_u64(b, state + 32 * 8)?,
                    });
                }
                _ => (), // everything else is ignored by this prototype
            }
            off = end;
        }
        if chained_fixups.is_some() && legacy_fixups.is_some() {
            return Err("mixed chained and legacy relocation streams are unsupported".into());
        }
        Ok(MachO64 {
            segments,
            entry: if file_type == MH_DYLIB {
                EntryPoint::Dylib
            } else {
                entry.ok_or("no LC_MAIN or LC_UNIXTHREAD")?
            },
            chained_fixups,
            legacy_fixups,
            file_type,
            dependencies,
            install_name,
            rpaths,
            has_initializers,
            initializer_sections,
        })
    }

    /// Segments that actually occupy address space (skips `__PAGEZERO`).
    fn mapped_segments(&self) -> impl Iterator<Item = &Segment64> {
        self.segments
            .iter()
            .filter(|s| s.vmsize != 0 && !(s.initprot == 0 && s.filesize == 0))
    }
}

/// Size of the stack placed directly above the image.
const STACK_SIZE: u64 = 1024 * 1024;

/// A loaded arm64 executable together with its CPU.
pub struct LoadedA64 {
    pub cpu: A64Cpu,
    /// Collected output of `write(1/2, ...)` syscalls.
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    runtime: runtime::Runtime64,
}

/// Map independent segment regions into a fresh [A64Cpu] and set up
/// PC/SP/LR. Rebase-only chained fixups are decoded without a slide.
pub fn load(file: &[u8], macho: &MachO64) -> Result<LoadedA64, String> {
    if macho.file_type != MH_EXECUTE || !macho.dependencies.is_empty() {
        return Err("standalone loader requires an executable without dynamic libraries".into());
    }
    if macho.has_initializers {
        return Err("ARM64 image initializers are not implemented".into());
    }
    let file = thin_arm64_slice(file)?;
    let _lo = macho
        .mapped_segments()
        .map(|s| s.vmaddr)
        .min()
        .ok_or("no segments")?;
    let hi = macho
        .mapped_segments()
        .map(|s| {
            s.vmaddr
                .checked_add(s.vmsize)
                .ok_or("segment address overflow")
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .unwrap();
    let page = 0x4000; // arm64 iOS uses 16KiB pages
    let image_top = hi.checked_add(page - 1).ok_or("image address overflow")? & !(page - 1);
    // [image][trampoline page][stack]
    let trampoline = image_top;
    let stack_top = trampoline
        .checked_add(page + STACK_SIZE)
        .ok_or("stack address overflow")?;
    let mapped = macho
        .mapped_segments()
        .try_fold(page + STACK_SIZE, |size, segment| {
            size.checked_add(segment.vmsize)
                .ok_or("mapped-byte count overflow")
        })?;
    if mapped > A64Cpu::MAX_MAPPED_BYTES as u64 {
        return Err("ARM64 image exceeds mapped-byte limit: 512 MiB".into());
    }
    let mut cpu = A64Cpu::new_sparse();

    for seg in macho.mapped_segments() {
        if seg.filesize > seg.vmsize {
            return Err("segment file size exceeds virtual size".into());
        }
        let start: usize = seg
            .fileoff
            .try_into()
            .map_err(|_| "file offset too large")?;
        let len: usize = seg.filesize.try_into().map_err(|_| "segment too large")?;
        let end = start
            .checked_add(len)
            .ok_or("segment file range overflow")?;
        let data = file
            .get(start..end)
            .ok_or_else(|| format!("segment {} out of file range", seg.name))?;
        cpu.map_zeroed(
            seg.vmaddr,
            seg.vmsize.try_into().map_err(|_| "segment too large")?,
            seg.initprot,
        )?;
        cpu.write_bytes(seg.vmaddr, data);
    }
    cpu.map_zeroed(trampoline, page as usize, 5)?;
    cpu.map_zeroed(trampoline + page, STACK_SIZE as usize, 3)?;

    if let Some((start, end)) = macho.chained_fixups {
        fixups::apply(&file[start..end], macho, &mut cpu)?;
    }
    if let Some(streams) = macho.legacy_fixups {
        legacy::apply(file, streams, &macho.segments, 0, &mut cpu, |import| {
            if import.weak {
                Ok(0)
            } else {
                Err(format!("unresolved required legacy symbol {}", import.name))
            }
        })?;
    }

    // If `main` returns, it returns here: `mov x16, #1; svc #0x80` = exit(x0).
    let tramp_code: [u32; 2] = [0xd2800030, 0xd4001001];
    let tramp_bytes: Vec<u8> = tramp_code.iter().flat_map(|i| i.to_le_bytes()).collect();
    cpu.write_bytes(trampoline, &tramp_bytes);

    match macho.entry {
        EntryPoint::Main { entryoff, .. } => {
            let text = macho
                .segments
                .iter()
                .find(|s| s.fileoff == 0 && s.filesize != 0)
                .ok_or("no __TEXT segment")?;
            if entryoff >= text.filesize {
                return Err("entry point outside text segment".into());
            }
            cpu.set_pc(
                text.vmaddr
                    .checked_add(entryoff)
                    .ok_or("entry point overflow")?,
            );
        }
        EntryPoint::UnixThread { pc, .. } => cpu.set_pc(pc),
        EntryPoint::Dylib => return Err("cannot execute a dylib as the main image".into()),
    }
    cpu.set_sp(stack_top - 16);
    if matches!(macho.entry, EntryPoint::Main { .. }) {
        setup_main_arguments(&mut cpu, stack_top, "/Standalone.app/Main")?;
    }
    cpu.set_reg(A64Cpu::LR, trampoline);
    LoadedA64::new(cpu)
}

impl LoadedA64 {
    pub(super) fn new(cpu: A64Cpu) -> Result<Self, String> {
        Ok(Self {
            cpu,
            stdout: Vec::new(),
            stderr: Vec::new(),
            runtime: runtime::Runtime64::new(1001)?,
        })
    }

    /// Dispatch a Darwin trap and set its architectural carry/errno result.
    pub(super) fn handle_svc(&mut self) -> Result<Option<i32>, String> {
        let number = self.cpu.reg(16);
        let args = std::array::from_fn(|index| self.cpu.reg(index));
        let outcome = self.runtime.handle(
            number,
            args,
            &mut self.cpu,
            &mut self.stdout,
            &mut self.stderr,
        );
        let (value, failed) = match outcome {
            runtime::Outcome::Success(value) => (value, false),
            runtime::Outcome::TrapSuccess(value) => {
                self.cpu.set_reg(0, value);
                return Ok(None);
            }
            runtime::Outcome::Errno(errno) => (errno as u64, true),
            runtime::Outcome::Exit(code) => return Ok(Some(code)),
            runtime::Outcome::Unsupported { number, reason } => {
                return Err(format!(
                    "unsupported Darwin syscall {} at pc {:#x}: {reason}",
                    number as i64,
                    self.cpu.pc().saturating_sub(4)
                ))
            }
        };
        self.cpu.set_reg(0, value);
        let flags = self.cpu.pstate() & !(1 << 29);
        self.cpu
            .set_pstate(flags | if failed { 1 << 29 } else { 0 });
        Ok(None)
    }

    /// Run until the guest calls `exit`, returning the exit code. Supports
    /// a bounded set of virtual Darwin services; unsupported traps fail.
    pub fn run(&mut self, max_ticks: u64) -> Result<i32, String> {
        let mut ticks = max_ticks;
        loop {
            let state = self.cpu.run_or_step(Some(&mut ticks));
            match state {
                A64State::Svc(0x80) => {
                    if let Some(code) = self.handle_svc()? {
                        return Ok(code);
                    }
                }
                A64State::Normal => {
                    if ticks == 0 {
                        return Err(format!("tick limit reached at pc {:#x}", self.cpu.pc()));
                    }
                }
                other => return Err(format!("CPU stopped: {other:?} at pc {:#x}", self.cpu.pc())),
            }
        }
    }
}

/// Raw machine code assembled from tests/a64/selftest_routine.s.
const SELFTEST_ROUTINE: &[u8] = include_bytes!("../tests/a64/selftest_routine.bin");
/// Linker-produced arm64 Mach-O built from tests/a64/hello_arm64.s.
const HELLO_MACHO: &[u8] = include_bytes!("../tests/a64/hello_arm64.macho");

fn run_selftest_routine() -> Result<u64, String> {
    let base = 0x1_0000_0000u64;
    let mut cpu = A64Cpu::new(base, 0x10000);
    let code = base + 0x1000;
    let scratch = base + 0x8000;
    cpu.write_bytes(code, SELFTEST_ROUTINE);
    cpu.set_reg(0, 100);
    cpu.set_reg(1, scratch);
    cpu.set_pc(code);
    cpu.set_sp(base + 0x10000);
    let mut ticks = 100_000;
    match cpu.run_or_step(Some(&mut ticks)) {
        A64State::Svc(0x80) => Ok(cpu.reg(0)),
        other => Err(format!("unexpected stop: {other:?}")),
    }
}

/// Run an arm64 Mach-O file with the prototype loader. Returns the exit code.
pub fn run_file(bytes: &[u8]) -> Result<i32, String> {
    let macho = MachO64::parse(bytes)?;
    for s in &macho.segments {
        echo!(
            "  segment {:<12} vmaddr {:#014x} vmsize {:#09x} fileoff {:#07x} filesize {:#07x}",
            s.name,
            s.vmaddr,
            s.vmsize,
            s.fileoff,
            s.filesize
        );
    }
    echo!("  entry: {:#x?}", macho.entry);
    let mut loaded = load(bytes, &macho)?;
    echo!(
        "  starting at pc {:#x}, sp {:#x}",
        loaded.cpu.pc(),
        loaded.cpu.sp()
    );
    let res = loaded.run(10_000_000);
    echo!(
        "  guest stdout: {:?}",
        String::from_utf8_lossy(&loaded.stdout)
    );
    res
}

/// Link a bounded collection of independently loadable ARM64 images. The
/// reader receives expanded absolute guest paths for library dependencies.
/// Set the four Darwin LC_MAIN arguments in mapped stack memory. No host
/// environment variables or paths are exposed to the guest process.
fn setup_main_arguments(cpu: &mut A64Cpu, stack_top: u64, executable: &str) -> Result<(), String> {
    if executable.is_empty() || executable.len() > 4096 || executable.contains('\0') {
        return Err("invalid guest executable path for process arguments".into());
    }
    let mut strings = executable.as_bytes().to_vec();
    strings.push(0);
    let apple_offset = strings.len();
    strings.extend_from_slice(b"executable_path=");
    strings.extend_from_slice(executable.as_bytes());
    strings.push(0);
    let base = stack_top
        .checked_sub(strings.len() as u64 + 80)
        .ok_or("process argument stack underflow")?
        & !15;
    let string_base = base + 64;
    let argv = base + 8;
    let envp = base + 24;
    let apple = base + 32;
    let words = [
        1,
        string_base,
        0,
        0,
        string_base + apple_offset as u64,
        0,
        0,
        0,
    ];
    let pointers: Vec<u8> = words.into_iter().flat_map(u64::to_le_bytes).collect();
    cpu.validate_guest_write(base, pointers.len() + strings.len())?;
    cpu.write_guest_into(base, &pointers)?;
    cpu.write_guest_into(string_base, &strings)?;
    cpu.set_sp(base);
    for (index, value) in [1, argv, envp, apple].into_iter().enumerate() {
        cpu.set_reg(index, value);
    }
    Ok(())
}

pub fn load_with_reader(
    bytes: &[u8],
    executable_path: &str,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<LoadedA64, String> {
    linker::load_with_reader(bytes, executable_path, reader)
}

pub fn run_file_with_reader(
    bytes: &[u8],
    executable_path: &str,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<i32, String> {
    let mut loaded = load_with_reader(bytes, executable_path, reader)?;
    let result = loaded.run(10_000_000);
    echo!(
        "  guest stdout: {:?}",
        String::from_utf8_lossy(&loaded.stdout)
    );
    result
}

/// Inspect shared-cache region metadata without mapping or executing it.
pub fn cache_info(path: &std::path::Path) -> Result<(), String> {
    let plan = cache::CachePlan::read(path)?;
    echo!(
        "ARM64 shared cache: {} images, {} files, {} regions",
        plan.image_count,
        plan.files.len(),
        plan.regions.len()
    );
    echo!(
        "  mapped bytes: {}; virtual span: {}",
        plan.mapped_bytes,
        plan.mapped_span
    );
    for file in &plan.files {
        echo!("  cache file: {}", file.display());
    }
    Ok(())
}

/// `--a64-selftest`: run both embedded payloads and check their results.
pub fn selftest() -> Result<(), String> {
    echo!("[a64] 1/2: raw A64 routine in a flat buffer at 0x1_0000_0000");
    let x0 = run_selftest_routine()?;
    let expected = 5050 + (0xdead << 48);
    echo!("  x0 = {x0:#x} (expected {expected:#x})");
    if x0 != expected {
        return Err("A64 routine returned the wrong value".to_string());
    }
    echo!(
        "[a64] 2/2: arm64 Mach-O hello world ({} bytes)",
        HELLO_MACHO.len()
    );
    let code = run_file(HELLO_MACHO)?;
    echo!("  exit code = {code} (expected {})", 5050 & 0xff);
    if code != 5050 & 0xff {
        return Err("hello_arm64 exited with the wrong code".to_string());
    }
    echo!("[a64] selftest passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn main_arguments_are_terminated_and_use_guest_paths() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 0x4000, 3).unwrap();
        setup_main_arguments(&mut cpu, 0x14000, "/Test.app/Test").unwrap();
        assert_eq!(cpu.reg(0), 1);
        assert_eq!(cpu.sp() & 15, 0);
        let argv = cpu.reg(1);
        let path = cpu.read_u64(argv).unwrap();
        assert_eq!(cpu.read_bytes(path, 15).unwrap(), b"/Test.app/Test\0");
        assert_eq!(cpu.read_u64(argv + 8), Some(0));
        assert_eq!(cpu.read_u64(cpu.reg(2)), Some(0));
        let apple = cpu.read_u64(cpu.reg(3)).unwrap();
        assert_eq!(
            cpu.read_bytes(apple, 31).unwrap(),
            b"executable_path=/Test.app/Test\0"
        );
        assert_eq!(cpu.read_u64(cpu.reg(3) + 8), Some(0));
        assert!(setup_main_arguments(&mut cpu, 0x14000, "bad\0path").is_err());
        assert!(setup_main_arguments(&mut cpu, 0x10010, "/Test.app/Test").is_err());
    }
    use super::*;

    #[test]
    fn a64_rejects_bad_commands_and_dependencies() {
        let mut bytes = HELLO_MACHO.to_vec();
        bytes[36..40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(MachO64::parse(&bytes).is_err());
        let mut bytes = HELLO_MACHO.to_vec();
        bytes[32..36].copy_from_slice(&0xcu32.to_le_bytes());
        assert!(MachO64::parse(&bytes).is_err());
    }

    #[test]
    fn a64_routing_preserves_universal_arm32() {
        assert_eq!(is_arm64_only(HELLO_MACHO), Ok(true));
        let mut fat = vec![0u8; 48];
        fat[0..4].copy_from_slice(&FAT_MAGIC.to_be_bytes());
        fat[4..8].copy_from_slice(&2u32.to_be_bytes());
        fat[8..12].copy_from_slice(&CPU_TYPE_ARM64.to_be_bytes());
        fat[28..32].copy_from_slice(&12u32.to_be_bytes());
        assert_eq!(is_arm64_only(&fat), Ok(false));
        fat.truncate(20);
        assert!(is_arm64_only(&fat).is_err());
    }

    #[test]
    fn a64_rejects_huge_mapped_memory() {
        let mut macho = MachO64::parse(HELLO_MACHO).unwrap();
        macho.segments[1].vmsize = A64Cpu::MAX_MAPPED_BYTES as u64 + 0x4000;
        assert!(load(HELLO_MACHO, &macho)
            .err()
            .unwrap()
            .contains("mapped-byte limit"));
    }

    #[test]
    fn a64_raw_routine() {
        assert_eq!(run_selftest_routine(), Ok(5050 + (0xdead << 48)));
    }

    #[test]
    fn a64_parse_hello_macho() {
        let m = MachO64::parse(HELLO_MACHO).unwrap();
        let names: Vec<&str> = m.segments.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["__PAGEZERO", "__TEXT", "__DATA", "__LINKEDIT"]);
        assert_eq!(m.segments[0].vmsize, 0x1_0000_0000);
        assert_eq!(m.segments[1].vmaddr, 0x1_0000_0000);
        assert!(matches!(
            m.entry,
            EntryPoint::Main {
                entryoff: 0x4000,
                ..
            }
        ));
    }

    #[test]
    fn a64_run_hello_macho() {
        let m = MachO64::parse(HELLO_MACHO).unwrap();
        let mut loaded = load(HELLO_MACHO, &m).unwrap();
        assert_eq!(loaded.run(10_000_000), Ok(5050 & 0xff));
        assert_eq!(
            loaded.stdout,
            b"Hello from arm64 Mach-O on dynarmic A64!\n".to_vec()
        );
        // The __DATA global was written by the guest.
        assert_eq!(loaded.cpu.read_u64(0x1_0000_8000), Some(5050));
    }

    #[test]
    fn a64_sparse_loader_preserves_large_unmapped_gaps() {
        let bytes = include_bytes!("../tests/a64/sparse_client.macho");
        let macho = MachO64::parse(bytes).unwrap();
        let mut standalone = load(bytes, &macho).unwrap();
        assert!(standalone.cpu.mapped_bytes() < 2 * 1024 * 1024);
        assert!(standalone.cpu.read_bytes(0x2_0000_0000, 8).is_none());
        assert_eq!(standalone.run(100_000), Ok(42));
        assert_eq!(standalone.cpu.read_u64(0x3_0000_0000), Some(42));
        let mut linked = load_with_reader(bytes, "/Test.app/Test", |_| {
            Err("unexpected library read".into())
        })
        .unwrap();
        assert!(linked.cpu.mapped_bytes() < 2 * 1024 * 1024);
        assert!(linked.cpu.read_bytes(0x2_0000_0000, 8).is_none());
        assert_eq!(linked.run(100_000), Ok(42));
    }

    #[test]
    fn a64_linked_dylib_import_executes() {
        let client = include_bytes!("../tests/a64/import_client.macho");
        let library = include_bytes!("../tests/a64/libAnswer.dylib");
        assert!(MachO64::parse(client)
            .unwrap_err()
            .contains("dynamic libraries"));
        let mut requested = Vec::new();
        let mut loaded = load_with_reader(client, "/Apps/Test.app/Test", |path| {
            requested.push(path.to_owned());
            if path == "/Apps/Test.app/Frameworks/libAnswer.dylib" {
                Ok(library.to_vec())
            } else {
                Err("missing fixture library".into())
            }
        })
        .unwrap();
        assert_eq!(requested, ["/Apps/Test.app/Frameworks/libAnswer.dylib"]);
        assert_eq!(loaded.run(100_000), Ok(42));
    }

    #[test]
    fn a64_linker_runs_initializer_array_before_main() {
        let mut loaded = load_with_reader(
            include_bytes!("../tests/a64/initializer_client.macho"),
            "/Test.app/Test",
            |_| Ok(include_bytes!("../tests/a64/libInitialized.dylib").to_vec()),
        )
        .unwrap();
        // Both initializers clobber x0/x8; the main entry context is restored.
        assert_eq!(loaded.cpu.reg(0), 1);
        assert_eq!(loaded.cpu.reg(8), 0);
        assert_eq!(loaded.cpu.sp() & 15, 0);
        assert_eq!(loaded.run(100_000), Ok(42));
    }

    #[test]
    fn a64_linker_rejects_invalid_initializer_target() {
        let mut library = include_bytes!("../tests/a64/libInitialized.dylib").to_vec();
        let macho = MachO64::parse_image(&library).unwrap();
        let (address, _, width) = macho.initializer_sections[0];
        let segment = macho
            .mapped_segments()
            .find(|s| address >= s.vmaddr && address < s.vmaddr + s.vmsize)
            .unwrap();
        let offset = (segment.fileoff + address - segment.vmaddr) as usize;
        if width == 4 {
            library[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        } else {
            let encoded = rd_u64(&library, offset).unwrap();
            library[offset..offset + 8].copy_from_slice(
                &((encoded & !0x0000_000f_ffff_ffffu64) | 0x0000_000f_ffff_ffff).to_le_bytes(),
            );
        }
        let result = load_with_reader(
            include_bytes!("../tests/a64/initializer_client.macho"),
            "/Test.app/Test",
            |_| Ok(library.clone()),
        );
        assert!(result.err().unwrap().contains("initializer target"));
    }

    #[test]
    fn a64_large_initializer_table_is_bounded_and_file_backed() {
        fn image(count: usize) -> Vec<u8> {
            let table_offset = 32 + 72 + 80;
            let length = table_offset + count * 8;
            let mut image = vec![0; length];
            for (offset, value) in [
                (0, 0xfeedfacfu32),
                (4, 0x100000c),
                (12, 6),
                (16, 1),
                (20, 152),
                (32, 25),
                (36, 152),
                (88, 3),
                (92, 3),
                (96, 1),
            ] {
                image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            image[40..46].copy_from_slice(b"__DATA");
            for (offset, value) in [(56, 0x1000u64), (64, length as u64), (80, length as u64)] {
                image[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            }
            image[104..119].copy_from_slice(b"__mod_init_func");
            image[120..126].copy_from_slice(b"__DATA");
            image[136..144].copy_from_slice(&(0x1000u64 + table_offset as u64).to_le_bytes());
            image[144..152].copy_from_slice(&(count as u64 * 8).to_le_bytes());
            image[152..156].copy_from_slice(&(table_offset as u32).to_le_bytes());
            image[156..160].copy_from_slice(&3u32.to_le_bytes());
            image[168..172].copy_from_slice(&9u32.to_le_bytes());
            image
        }
        let game_sized = image(12868);
        let parsed = MachO64::parse_image(&game_sized).unwrap();
        assert_eq!(parsed.initializer_sections[0].1, 12868);
        let mut aion_offsets=image(27498);
        aion_offsets[104..120].fill(0);aion_offsets[104..118].copy_from_slice(b"__init_offsets");
        aion_offsets[144..152].copy_from_slice(&(27498u64*4).to_le_bytes());
        aion_offsets[168..172].copy_from_slice(&0x16u32.to_le_bytes());
        assert_eq!(MachO64::parse_image(&aion_offsets).unwrap().initializer_sections[0].1,27498);
        assert_eq!(MachO64::parse_image(&aion_offsets).unwrap().initializer_sections[0].2,4);
        assert_eq!(MachO64::parse_image(&image(65536)).unwrap().initializer_sections[0].1,65536);
        assert!(MachO64::parse_image(&image(65537))
            .unwrap_err()
            .contains("initializer table exceeds per-image budget"));
        let mut out_of_bounds = game_sized;
        out_of_bounds[80..88].copy_from_slice(&184u64.to_le_bytes());
        assert!(MachO64::parse_image(&out_of_bounds)
            .unwrap_err()
            .contains("initializer array outside file-backed"));
    }

    #[test]
    fn a64_initializer_array_rejects_misaligned_length() {
        let mut library = include_bytes!("../tests/a64/libInitialized.dylib").to_vec();
        let count = rd_u32(&library, 16).unwrap();
        let mut offset = 32;
        let mut patched = false;
        for _ in 0..count {
            let size = rd_u32(&library, offset + 4).unwrap() as usize;
            if rd_u32(&library, offset).unwrap() == LC_SEGMENT_64 {
                for section in 0..rd_u32(&library, offset + 64).unwrap() as usize {
                    let section_offset = offset + 72 + 80 * section;
                    if library[section_offset..section_offset + 16].starts_with(b"__mod_init_func")
                        || matches!(
                            rd_u32(&library, section_offset + 64).unwrap() & 0xff,
                            9 | 0x16
                        )
                    {
                        library[section_offset + 40..section_offset + 48]
                            .copy_from_slice(&9u64.to_le_bytes());
                        patched = true;
                    }
                }
            }
            offset += size;
        }
        assert!(patched);
        assert!(MachO64::parse_image(&library)
            .unwrap_err()
            .contains("initializer array alignment"));
    }

    #[test]
    fn a64_initializer_tick_budget_prevents_main_execution() {
        let mut library = include_bytes!("../tests/a64/libInitialized.dylib").to_vec();
        let macho = MachO64::parse_image(&library).unwrap();
        let (table, _, width) = macho.initializer_sections[0];
        let table_segment = macho
            .mapped_segments()
            .find(|s| table >= s.vmaddr && table < s.vmaddr + s.vmsize)
            .unwrap();
        let table_offset = (table_segment.fileoff + table - table_segment.vmaddr) as usize;
        let function = if width == 4 {
            let header = macho
                .segments
                .iter()
                .find(|s| s.fileoff == 0 && s.filesize != 0)
                .unwrap()
                .vmaddr;
            header + rd_u32(&library, table_offset).unwrap() as u64
        } else {
            // The pointer-form fixture uses PTR_64_OFFSET and image base zero.
            rd_u64(&library, table_offset).unwrap() & 0x0000_000f_ffff_ffff
        };
        let text = macho
            .mapped_segments()
            .find(|s| function >= s.vmaddr && function < s.vmaddr + s.vmsize)
            .unwrap();
        let function_offset = (text.fileoff + function - text.vmaddr) as usize;
        library[function_offset..function_offset + 4].copy_from_slice(&0x14000000u32.to_le_bytes());
        let result = load_with_reader(
            include_bytes!("../tests/a64/initializer_client.macho"),
            "/Test.app/Test",
            |_| Ok(library.clone()),
        );
        assert!(result
            .err()
            .unwrap()
            .contains("initializer tick budget exhausted"));
    }

    #[test]
    fn a64_linker_reports_missing_library() {
        let result = load_with_reader(
            include_bytes!("../tests/a64/import_client.macho"),
            "/Test.app/Test",
            |_| Err("file not found".into()),
        );
        assert!(result
            .err()
            .unwrap()
            .contains("cannot load dynamic library"));
    }

    #[test]
    fn a64_linker_reports_unresolved_required_symbol() {
        let mut library = include_bytes!("../tests/a64/libAnswer.dylib").to_vec();
        // Rewrite both symbol-table and trie spellings without changing offsets.
        for index in 0..library.len().saturating_sub(6) {
            if &library[index..index + 7] == b"_answer" {
                library[index..index + 7].copy_from_slice(b"_absent");
            }
        }
        let result = load_with_reader(
            include_bytes!("../tests/a64/import_client.macho"),
            "/Test.app/Test",
            |_| Ok(library.clone()),
        );
        assert!(result
            .err()
            .unwrap()
            .contains("unresolved required symbol _answer"));
    }

    #[test]
    fn a64_linker_allows_missing_weak_library() {
        let mut client = include_bytes!("../tests/a64/import_client.macho").to_vec();
        let count = rd_u32(&client, 16).unwrap();
        let mut offset = 32;
        let mut patched = false;
        for _ in 0..count {
            let command = rd_u32(&client, offset).unwrap();
            let size = rd_u32(&client, offset + 4).unwrap() as usize;
            if command == 0xc {
                client[offset..offset + 4].copy_from_slice(&0x80000018u32.to_le_bytes());
                patched = true;
            }
            offset += size;
        }
        assert!(patched);
        // Missing weak libraries may bind to NULL; executing their functions
        // remains the guest's responsibility, so this only checks loading.
        assert!(load_with_reader(&client, "/Test.app/Test", |_| Err(
            "missing optional library".into()
        ))
        .is_ok());
    }

    #[test]
    fn a64_rejects_shared_cache_and_split_images() {
        for flag in [0x8000_0000u32, 0x20] {
            let mut bytes = include_bytes!("../tests/a64/libAnswer.dylib").to_vec();
            let old = rd_u32(&bytes, 24).unwrap();
            bytes[24..28].copy_from_slice(&(old | flag).to_le_bytes());
            assert!(MachO64::parse_image(&bytes).is_err());
        }
    }
}

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `dlfcn.h` (`dlopen()` and friends)

use crate::dyld::{export_c_func, FunctionExports};
use crate::mem::{ConstPtr, ConstVoidPtr, MutPtr, MutVoidPtr, Ptr, SafeRead};
use crate::Environment;
use std::collections::HashMap;

const RTLD_DEFAULT: MutVoidPtr = Ptr::from_bits(-2 as _);

fn is_known_library(path: &str) -> bool {
    crate::dyld::DYLIB_LIST
        .iter()
        .any(|dylib| dylib.path == path || dylib.aliases.contains(&path))
}

fn dlopen(env: &mut Environment, path: ConstPtr<u8>, _mode: i32) -> MutVoidPtr {
    if path.is_null() {
        return RTLD_DEFAULT;
    }
    // TODO: dlopen() support for real dynamic libraries.
    let path_str = env.mem.cstr_at_utf8(path).unwrap();
    if !is_known_library(path_str) {
        // Real dlopen() fails with NULL for a library that is not there, and
        // apps probe for optional ones.
        log!("dlopen({:?}) of an unknown library: returning NULL", path_str);
        return Ptr::null();
    }
    // For convenience, use the path as the handle.
    // TODO: Find out whether the handle is truly opaque on iPhone OS, and if
    // not, where it points.
    path.cast_mut().cast()
}

fn dlsym(env: &mut Environment, handle: MutVoidPtr, symbol: ConstPtr<u8>) -> MutVoidPtr {
    if handle.is_null() {
        return Ptr::null();
    }
    // RTLD_NEXT (-1), RTLD_DEFAULT (-2), RTLD_SELF (-3) and RTLD_MAIN_ONLY (-5) are
    // all looked up in the one namespace that exists here.
    let special_handle = handle.to_bits() >= 0xffff_fff0;
    if !special_handle && !is_known_library(env.mem.cstr_at_utf8(handle.cast()).unwrap_or("")) {
        log!("dlsym() with an unknown handle {:?}, returning NULL", handle);
        return Ptr::null();
    }
    // For some reason, the symbols passed to dlsym() don't have the leading _.
    let symbol = format!("_{}", env.mem.cstr_at_utf8(symbol).unwrap());
    // TODO: error handling. dlsym() should just return NULL in this case, but
    // currently it's probably more useful to have the emulator crash if there's
    // no symbol found, since it most likely indicates a missing host function.
    // TODO: Symbol lookup should be scoped to the specific library requested,
    // where appropriate!
    match env
        .dyld
        .create_proc_address(&mut env.mem, &mut env.cpu, &symbol)
    {
        Ok(addr) => Ptr::from_bits(addr.addr_with_thumb_bit()),
        Err(_) => {
            log!("dlsym() for unimplemented symbol {}, returning NULL", symbol);
            Ptr::null()
        }
    }
}

fn dlclose(env: &mut Environment, handle: MutVoidPtr) -> i32 {
    0 // success
}

/// Cached guest C strings handed out by `dladdr()` (callers don't free them).
#[derive(Default)]
pub struct State {
    strings: HashMap<String, ConstPtr<u8>>,
}

fn cached_cstr(env: &mut Environment, s: &str) -> ConstPtr<u8> {
    if let Some(&ptr) = env.libc_state.dlfcn.strings.get(s) {
        return ptr;
    }
    let ptr = env.mem.alloc_and_write_cstr(s.as_bytes()).cast_const();
    env.libc_state.dlfcn.strings.insert(s.to_string(), ptr);
    ptr
}

#[allow(non_camel_case_types)]
#[repr(C, packed)]
#[derive(Copy, Clone, Debug)]
/// `Dl_info`
struct Dl_info {
    dli_fname: ConstPtr<u8>,
    dli_fbase: ConstVoidPtr,
    dli_sname: ConstPtr<u8>,
    dli_saddr: ConstVoidPtr,
}
unsafe impl SafeRead for Dl_info {}

/// `dladdr()`: find the loaded image containing `addr` and fill in its path,
/// the address of its Mach-O header, and the nearest symbol at or below
/// `addr`. Apps use the header address to walk their own load commands.
fn dladdr(env: &mut Environment, addr: ConstVoidPtr, info: MutPtr<Dl_info>) -> i32 {
    let addr = addr.to_bits();
    let Some(bin_idx) = env.bins.iter().position(|bin| {
        bin.segment_ranges
            .iter()
            .any(|&(start, size)| addr >= start && addr - start < size)
    }) else {
        return 0;
    };
    let bin = &env.bins[bin_idx];
    let Some(header_addr) = bin.header_addr else {
        return 0;
    };
    let fname = if bin_idx == 0 {
        env.bundle.executable_path().as_str().to_string()
    } else {
        bin.name.clone()
    };
    // Nearest preceding symbol (Thumb bit ignored for the comparison).
    let symbol = bin
        .exported_symbols
        .iter()
        .filter(|&(_, &sym_addr)| (sym_addr & !1) <= addr)
        .max_by_key(|&(_, &sym_addr)| sym_addr & !1)
        .map(|(name, &sym_addr)| (name.clone(), sym_addr));

    let dli_fname = cached_cstr(env, &fname);
    let (dli_sname, dli_saddr) = match symbol {
        Some((name, sym_addr)) => {
            // dladdr() reports C names without the leading underscore.
            let name = name.strip_prefix('_').unwrap_or(&name).to_string();
            (cached_cstr(env, &name), Ptr::from_bits(sym_addr))
        }
        None => (Ptr::null(), Ptr::null()),
    };
    if !info.is_null() {
        env.mem.write(
            info,
            Dl_info {
                dli_fname,
                dli_fbase: Ptr::from_bits(header_addr),
                dli_sname,
                dli_saddr,
            },
        );
    }
    1
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(dlopen(_, _)),
    export_c_func!(dlsym(_, _)),
    export_c_func!(dlclose(_)),
    export_c_func!(dladdr(_, _)),
];

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `dlfcn.h` (`dlopen()` and friends)

use crate::dyld::{export_c_func, FunctionExports};
use crate::mem::{ConstPtr, ConstVoidPtr, MutVoidPtr, Ptr};
use crate::Environment;

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

/// `dladdr()`: nothing here maps host-side addresses to symbols, so report
/// "not found" (0), which callers have to be ready for anyway.
fn dladdr(_env: &mut Environment, _addr: ConstVoidPtr, _info: MutVoidPtr) -> i32 {
    0
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(dlopen(_, _)),
    export_c_func!(dlsym(_, _)),
    export_c_func!(dlclose(_)),
    export_c_func!(dladdr(_, _)),
];

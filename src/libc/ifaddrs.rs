/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `ifaddrs.h` (interface addresses)

use crate::dyld::FunctionExports;
use crate::export_c_func;
use crate::libc::errno::set_errno;
use crate::mem::{ConstPtr, MutPtr};
use crate::Environment;

// TODO: struct definition
#[allow(non_camel_case_types)]
struct ifaddrs {}

fn getifaddrs(env: &mut Environment, _ifap: MutPtr<MutPtr<ifaddrs>>) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);

    // TODO: implement
    -1
}

fn freeifaddrs(_env: &mut Environment, _ifp: MutPtr<ifaddrs>) {
    // TODO
}

/// Looks up a network interface index by name. touchHLE only pretends to have
/// one interface, `en0`; any other name returns 0 (and `ENXIO`).
fn if_nametoindex(env: &mut Environment, name: ConstPtr<u8>) -> u32 {
    const ENXIO: i32 = 6;
    // touchHLE pretends to have a single interface, `en0` (see sysctl.rs).
    if env.mem.cstr_at_utf8(name) == Ok("en0") {
        return 1;
    }
    set_errno(env, ENXIO);
    0
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(getifaddrs(_)),
    export_c_func!(freeifaddrs(_)),
    export_c_func!(if_nametoindex(_)),
];

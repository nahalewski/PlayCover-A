/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `uuid/uuid.h`. A `uuid_t` is a plain array of 16 bytes.

use crate::dyld::{export_c_func, FunctionExports};
use crate::mem::{ConstPtr, MutPtr};
use crate::Environment;

fn uuid_generate(env: &mut Environment, out: MutPtr<u8>) {
    let uuid = ::uuid::Uuid::new_v4();
    env.mem
        .bytes_at_mut(out, 16)
        .copy_from_slice(uuid.as_bytes());
}

fn uuid_unparse_inner(env: &mut Environment, uu: ConstPtr<u8>, out: MutPtr<u8>, upper: bool) {
    let bytes: [u8; 16] = env.mem.bytes_at(uu, 16).try_into().unwrap();
    let uuid = ::uuid::Uuid::from_bytes(bytes);
    let s = uuid.hyphenated().to_string();
    let s = if upper { s.to_uppercase() } else { s };
    assert_eq!(s.len(), 36);
    env.mem.bytes_at_mut(out, 36).copy_from_slice(s.as_bytes());
    env.mem.write(out + 36, b'\0');
}

fn uuid_unparse_lower(env: &mut Environment, uu: ConstPtr<u8>, out: MutPtr<u8>) {
    uuid_unparse_inner(env, uu, out, false)
}
fn uuid_unparse_upper(env: &mut Environment, uu: ConstPtr<u8>, out: MutPtr<u8>) {
    uuid_unparse_inner(env, uu, out, true)
}
fn uuid_unparse(env: &mut Environment, uu: ConstPtr<u8>, out: MutPtr<u8>) {
    // Darwin's uuid_unparse() produces lowercase output.
    uuid_unparse_inner(env, uu, out, false)
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(uuid_generate(_)),
    export_c_func!(uuid_unparse(_, _)),
    export_c_func!(uuid_unparse_lower(_, _)),
    export_c_func!(uuid_unparse_upper(_, _)),
];

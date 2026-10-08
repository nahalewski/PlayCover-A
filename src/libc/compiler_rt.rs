/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Compiler runtime helper functions (libgcc/compiler-rt).
//!
//! Code built for ARM chips without a hardware integer divide instruction
//! (such as the armv7 Cortex-A8 in early iPhones) calls these to divide.
//!
//! Division by zero returns 0, which is what the usual `__aeabi_idiv0`
//! fallback does. Overflowing cases (`i32::MIN / -1`) wrap.

use crate::dyld::{export_c_func, FunctionExports};
use crate::mem::MutPtr;
use crate::Environment;

fn __divsi3(_env: &mut Environment, a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}
fn __modsi3(_env: &mut Environment, a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_rem(b)
    }
}
fn __udivsi3(_env: &mut Environment, a: u32, b: u32) -> u32 {
    a.checked_div(b).unwrap_or(0)
}
fn __umodsi3(_env: &mut Environment, a: u32, b: u32) -> u32 {
    a.checked_rem(b).unwrap_or(0)
}
fn __divdi3(_env: &mut Environment, a: i64, b: i64) -> i64 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}
fn __moddi3(_env: &mut Environment, a: i64, b: i64) -> i64 {
    if b == 0 {
        0
    } else {
        a.wrapping_rem(b)
    }
}
fn __udivdi3(_env: &mut Environment, a: u64, b: u64) -> u64 {
    a.checked_div(b).unwrap_or(0)
}
fn __umoddi3(_env: &mut Environment, a: u64, b: u64) -> u64 {
    a.checked_rem(b).unwrap_or(0)
}

fn __udivmodsi4(env: &mut Environment, a: u32, b: u32, rem: MutPtr<u32>) -> u32 {
    let q = a.checked_div(b).unwrap_or(0);
    if !rem.is_null() {
        env.mem.write(rem, a.checked_rem(b).unwrap_or(0));
    }
    q
}
fn __divmodsi4(env: &mut Environment, a: i32, b: i32, rem: MutPtr<i32>) -> i32 {
    let q = if b == 0 { 0 } else { a.wrapping_div(b) };
    if !rem.is_null() {
        env.mem.write(rem, if b == 0 { 0 } else { a.wrapping_rem(b) });
    }
    q
}
fn __udivmoddi4(env: &mut Environment, a: u64, b: u64, rem: MutPtr<u64>) -> u64 {
    let q = a.checked_div(b).unwrap_or(0);
    if !rem.is_null() {
        env.mem.write(rem, a.checked_rem(b).unwrap_or(0));
    }
    q
}
fn __divmoddi4(env: &mut Environment, a: i64, b: i64, rem: MutPtr<i64>) -> i64 {
    let q = if b == 0 { 0 } else { a.wrapping_div(b) };
    if !rem.is_null() {
        env.mem.write(rem, if b == 0 { 0 } else { a.wrapping_rem(b) });
    }
    q
}
fn __ashldi3(_env: &mut Environment, a: u64, b: i32) -> u64 {
    if b >= 64 { 0 } else { a << b }
}
fn __lshrdi3(_env: &mut Environment, a: u64, b: i32) -> u64 {
    if b >= 64 { 0 } else { a >> b }
}
fn __ashrdi3(_env: &mut Environment, a: i64, b: i32) -> i64 {
    if b >= 64 { if a < 0 { -1 } else { 0 } } else { a >> b }
}
fn __clzsi2(_env: &mut Environment, a: u32) -> i32 {
    a.leading_zeros() as i32
}
fn __ctzsi2(_env: &mut Environment, a: u32) -> i32 {
    a.trailing_zeros() as i32
}
fn __popcountsi2(_env: &mut Environment, a: u32) -> i32 {
    a.count_ones() as i32
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(__udivmodsi4(_, _, _)),
    export_c_func!(__divmodsi4(_, _, _)),
    export_c_func!(__udivmoddi4(_, _, _)),
    export_c_func!(__divmoddi4(_, _, _)),
    export_c_func!(__ashldi3(_, _)),
    export_c_func!(__lshrdi3(_, _)),
    export_c_func!(__ashrdi3(_, _)),
    export_c_func!(__clzsi2(_)),
    export_c_func!(__ctzsi2(_)),
    export_c_func!(__popcountsi2(_)),
    export_c_func!(__divsi3(_, _)),
    export_c_func!(__modsi3(_, _)),
    export_c_func!(__udivsi3(_, _)),
    export_c_func!(__umodsi3(_, _)),
    export_c_func!(__divdi3(_, _)),
    export_c_func!(__moddi3(_, _)),
    export_c_func!(__udivdi3(_, _)),
    export_c_func!(__umoddi3(_, _)),
];

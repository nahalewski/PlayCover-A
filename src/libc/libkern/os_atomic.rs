/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `libkern/OSAtomic.h`
//!
//! Atomic operations.
//!
//! Right now touchHLE is a single host thread application.
//! Thus, the execution of host functions couldn't be interrupted
//! by other threads. So we consider host functions to be atomic!

use crate::dyld::FunctionExports;
use crate::export_c_func;
use crate::mem::{MutPtr, MutVoidPtr};
use crate::Environment;

fn OSAtomicAdd32(env: &mut Environment, amount: i32, value_ptr: MutPtr<i32>) -> i32 {
    OSAtomicAdd32Barrier(env, amount, value_ptr)
}

fn OSAtomicAdd32Barrier(env: &mut Environment, the_amount: i32, the_value: MutPtr<i32>) -> i32 {
    let curr = env.mem.read(the_value);
    let new = curr + the_amount;
    env.mem.write(the_value, new);
    new
}

fn OSAtomicCompareAndSwap32(
    env: &mut Environment,
    old_value: i32,
    new_value: i32,
    the_value: MutPtr<i32>,
) -> bool {
    OSAtomicCompareAndSwap32Barrier(env, old_value, new_value, the_value)
}

fn OSAtomicCompareAndSwapIntBarrier(
    env: &mut Environment,
    old_value: i32,
    new_value: i32,
    the_value: MutPtr<i32>,
) -> bool {
    OSAtomicCompareAndSwap32Barrier(env, old_value, new_value, the_value)
}

fn OSAtomicCompareAndSwap32Barrier(
    env: &mut Environment,
    old_value: i32,
    new_value: i32,
    the_value: MutPtr<i32>,
) -> bool {
    if old_value == env.mem.read(the_value) {
        env.mem.write(the_value, new_value);
        true
    } else {
        false
    }
}

fn OSAtomicCompareAndSwapPtr(
    env: &mut Environment,
    old_value: MutVoidPtr,
    new_value: MutVoidPtr,
    the_value: MutPtr<MutVoidPtr>,
) -> bool {
    OSAtomicCompareAndSwapPtrBarrier(env, old_value, new_value, the_value)
}

fn OSAtomicCompareAndSwapPtrBarrier(
    env: &mut Environment,
    old_value: MutVoidPtr,
    new_value: MutVoidPtr,
    the_value: MutPtr<MutVoidPtr>,
) -> bool {
    if old_value == env.mem.read(the_value) {
        env.mem.write(the_value, new_value);
        true
    } else {
        false
    }
}

fn OSMemoryBarrier(_env: &mut Environment) {
    // no-op
}

fn OSAtomicOr32Barrier(env: &mut Environment, mask: u32, the_value: MutPtr<u32>) -> u32 {
    let new = env.mem.read(the_value) | mask;
    env.mem.write(the_value, new);
    new
}

/// Bit `n` is counted from the most significant bit of the first byte, as
/// documented in `man 3 OSAtomicTestAndClear`. Returns the original bit value.
fn OSAtomicTestAndClearBarrier(env: &mut Environment, n: u32, the_address: MutVoidPtr) -> bool {
    let byte_ptr: MutPtr<u8> = the_address.cast::<u8>() + (n >> 3);
    let bit = 0x80u8 >> (n & 7);
    let byte = env.mem.read(byte_ptr);
    env.mem.write(byte_ptr, byte & !bit);
    (byte & bit) != 0
}

// OSSpinLock is a plain int32: 0 = unlocked, nonzero = locked.
#[allow(non_camel_case_types)]
type OSSpinLock = i32;

fn OSSpinLockTry(env: &mut Environment, lock: MutPtr<OSSpinLock>) -> bool {
    if env.mem.read(lock) == 0 {
        env.mem.write(lock, 1);
        true
    } else {
        false
    }
}

fn OSSpinLockLock(env: &mut Environment, lock: MutPtr<OSSpinLock>) {
    // Host functions are atomic relative to guest threads (see above), so
    // "spinning" means letting other guest threads run until the lock is free.
    while !OSSpinLockTry(env, lock) {
        env.sleep(std::time::Duration::from_millis(1));
    }
}

fn OSSpinLockUnlock(env: &mut Environment, lock: MutPtr<OSSpinLock>) {
    env.mem.write(lock, 0);
}

fn OSAtomicAdd64(env: &mut Environment, amount: i64, value: MutPtr<i64>) -> i64 {
    let new = env.mem.read(value).wrapping_add(amount);
    env.mem.write(value, new);
    new
}

fn OSAtomicAdd64Barrier(env: &mut Environment, amount: i64, value: MutPtr<i64>) -> i64 {
    OSAtomicAdd64(env, amount, value)
}

fn OSAtomicCompareAndSwap64(
    env: &mut Environment,
    old_value: i64,
    new_value: i64,
    the_value: MutPtr<i64>,
) -> bool {
    if old_value == env.mem.read(the_value) {
        env.mem.write(the_value, new_value);
        true
    } else {
        false
    }
}

fn OSAtomicCompareAndSwap64Barrier(
    env: &mut Environment,
    old_value: i64,
    new_value: i64,
    the_value: MutPtr<i64>,
) -> bool {
    OSAtomicCompareAndSwap64(env, old_value, new_value, the_value)
}

fn OSAtomicIncrement32(env: &mut Environment, value: MutPtr<i32>) -> i32 {
    OSAtomicAdd32Barrier(env, 1, value)
}

fn OSAtomicIncrement32Barrier(env: &mut Environment, value: MutPtr<i32>) -> i32 {
    OSAtomicAdd32Barrier(env, 1, value)
}

fn OSAtomicDecrement32(env: &mut Environment, value: MutPtr<i32>) -> i32 {
    OSAtomicAdd32Barrier(env, -1, value)
}

fn OSAtomicDecrement32Barrier(env: &mut Environment, value: MutPtr<i32>) -> i32 {
    OSAtomicAdd32Barrier(env, -1, value)
}

fn OSAtomicOr32(env: &mut Environment, mask: u32, value: MutPtr<u32>) -> u32 {
    let new = env.mem.read(value) | mask;
    env.mem.write(value, new);
    new
}

fn OSAtomicAnd32(env: &mut Environment, mask: u32, value: MutPtr<u32>) -> u32 {
    let new = env.mem.read(value) & mask;
    env.mem.write(value, new);
    new
}

fn OSAtomicXor32(env: &mut Environment, mask: u32, value: MutPtr<u32>) -> u32 {
    let new = env.mem.read(value) ^ mask;
    env.mem.write(value, new);
    new
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(OSAtomicAdd64(_, _)),
    export_c_func!(OSAtomicAdd64Barrier(_, _)),
    export_c_func!(OSAtomicCompareAndSwap64(_, _, _)),
    export_c_func!(OSAtomicCompareAndSwap64Barrier(_, _, _)),
    export_c_func!(OSAtomicIncrement32(_)),
    export_c_func!(OSAtomicIncrement32Barrier(_)),
    export_c_func!(OSAtomicDecrement32(_)),
    export_c_func!(OSAtomicDecrement32Barrier(_)),
    export_c_func!(OSAtomicOr32(_, _)),
    export_c_func!(OSAtomicAnd32(_, _)),
    export_c_func!(OSAtomicXor32(_, _)),
    export_c_func!(OSAtomicAdd32(_, _)),
    export_c_func!(OSAtomicAdd32Barrier(_, _)),
    export_c_func!(OSAtomicCompareAndSwap32(_, _, _)),
    export_c_func!(OSAtomicCompareAndSwapIntBarrier(_, _, _)),
    export_c_func!(OSAtomicCompareAndSwap32Barrier(_, _, _)),
    export_c_func!(OSAtomicCompareAndSwapPtr(_, _, _)),
    export_c_func!(OSAtomicCompareAndSwapPtrBarrier(_, _, _)),
    export_c_func!(OSMemoryBarrier()),
    export_c_func!(OSAtomicOr32Barrier(_, _)),
    export_c_func!(OSAtomicTestAndClearBarrier(_, _)),
    export_c_func!(OSSpinLockTry(_)),
    export_c_func!(OSSpinLockLock(_)),
    export_c_func!(OSSpinLockUnlock(_)),
];

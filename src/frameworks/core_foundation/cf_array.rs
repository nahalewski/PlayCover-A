/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `CFArray` and `CFMutableArray`.
//!
//! These are toll-free bridged to `NSArray` and `NSMutableArray` in Apple's
//! implementation. Here they are the same types.

use super::cf_allocator::{kCFAllocatorDefault, CFAllocatorRef};
use super::CFIndex;
use crate::dyld::{export_c_func, FunctionExports};
use crate::frameworks::foundation::NSUInteger;
use crate::mem::{ConstVoidPtr, Ptr};
use crate::objc::{id, msg, msg_class};
use crate::Environment;

#[allow(dead_code)]
pub type CFArrayRef = super::CFTypeRef;
pub type CFMutableArrayRef = super::CFTypeRef;

fn CFArrayCreateMutable(
    env: &mut Environment,
    allocator: CFAllocatorRef,
    capacity: CFIndex,
    callbacks: ConstVoidPtr, // TODO, should be `const CFArrayCallBacks*`
) -> CFMutableArrayRef {
    assert!(allocator.is_null() || allocator == kCFAllocatorDefault || env.mem.read(allocator).is_system_default()); // unimplemented
    if capacity != 0 {
        // TODO: fixed capacity support. The limit is not enforced; a
        // well-behaved app never exceeds it, so this is only a warning.
        log_once!("TODO: CFArrayCreateMutable() capacity limit is not enforced");
    }

    // `CFArrayCallBacks` is { version: CFIndex, retain: fn ptr, release: fn ptr,
    // copyDescription: fn ptr, equal: fn ptr }. NULL callbacks (or a NULL
    // retain function) means the array doesn't retain its contents.
    // TODO: support custom retain/release/equal callbacks. If the retain
    // function is set, we assume it is the standard CFType one.
    let retains = !callbacks.is_null() && {
        let retain_fn: u32 = env
            .mem
            .read(Ptr::<u32, false>::from_bits(callbacks.to_bits() + 4));
        retain_fn != 0
    };

    if retains {
        msg_class![env; NSMutableArray new]
    } else {
        msg_class![env; _touchHLE_NSMutableArray_non_retaining new]
    }
}

fn CFArrayGetCount(env: &mut Environment, array: CFArrayRef) -> CFIndex {
    let count: NSUInteger = msg![env; array count];
    count.try_into().unwrap()
}

fn CFArrayGetValueAtIndex(env: &mut Environment, array: CFArrayRef, idx: CFIndex) -> ConstVoidPtr {
    let idx: NSUInteger = idx.try_into().unwrap();
    let value: id = msg![env; array objectAtIndex:idx];
    value.cast().cast_const()
}

fn CFArrayAppendValue(env: &mut Environment, array: CFMutableArrayRef, value: ConstVoidPtr) {
    let value: id = value.cast().cast_mut();
    msg![env; array addObject:value]
}

fn CFArrayRemoveValueAtIndex(env: &mut Environment, array: CFMutableArrayRef, idx: CFIndex) {
    let idx: NSUInteger = idx.try_into().unwrap();
    msg![env; array removeObjectAtIndex:idx]
}

use crate::dyld::{ConstantExports, HostConstant};

pub const CONSTANTS: ConstantExports = &[(
    "_kCFTypeArrayCallBacks",
    HostConstant::Custom(|env| {
        // CFArrayCallBacks { version, retain, release, copyDescription, equal }.
        // CFArrayCreateMutable() above only checks that `retain` is non-NULL.
        let callbacks = env.mem.alloc(20).cast::<u32>();
        for (i, value) in [0u32, 1, 1, 0, 0].into_iter().enumerate() {
            env.mem.write(callbacks + i as u32, value);
        }
        callbacks.cast().cast_const()
    }),
)];

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CFArrayCreateMutable(_, _, _)),
    export_c_func!(CFArrayGetCount(_)),
    export_c_func!(CFArrayGetValueAtIndex(_, _)),
    export_c_func!(CFArrayAppendValue(_, _)),
    export_c_func!(CFArrayRemoveValueAtIndex(_, _)),
];

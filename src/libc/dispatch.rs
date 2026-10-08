/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Grand Central Dispatch (libdispatch).
//!
//! A minimal stand-in: there are no real queues or threads. A queue is just
//! an opaque handle, and work submitted to any queue (even a "background" or
//! serial one) is run immediately, to completion, on the calling thread.
//! That is enough for apps that only use queues to hand work off, but it
//! means timing-based APIs like `dispatch_after` are not supported.

use crate::abi::{CallFromHost, GuestFunction};
use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant};
use crate::frameworks::foundation::ns_operation_queue::run_block;
use crate::mem::{ConstPtr, MutPtr, MutVoidPtr};
use crate::objc::id;
use crate::Environment;

/// `dispatch_once_t` is a `long`, which is 32 bits on armv7.
type dispatch_once_t = i32;

/// Value `dispatch_once_t` holds once the block has run.
const DISPATCH_ONCE_DONE: dispatch_once_t = -1;

/// Run `block` exactly once for a given `predicate`.
///
/// There is only one guest thread running at a time here (and the block runs
/// to completion before returning), so no locking is needed.
fn dispatch_once(env: &mut Environment, predicate: MutPtr<dispatch_once_t>, block: id) {
    if env.mem.read(predicate) == DISPATCH_ONCE_DONE {
        return;
    }
    run_block(env, block);
    env.mem.write(predicate, DISPATCH_ONCE_DONE);
}

/// A fresh opaque queue handle. Its contents are never looked at.
fn new_queue_handle(env: &mut Environment) -> MutVoidPtr {
    env.mem.alloc(16)
}

fn dispatch_queue_create(
    env: &mut Environment,
    _label: ConstPtr<u8>,
    _attr: MutVoidPtr,
) -> MutVoidPtr {
    new_queue_handle(env)
}

fn dispatch_get_global_queue(env: &mut Environment, _priority: i32, _flags: u32) -> MutVoidPtr {
    new_queue_handle(env)
}

fn dispatch_get_current_queue(env: &mut Environment) -> MutVoidPtr {
    new_queue_handle(env)
}

fn dispatch_set_target_queue(_env: &mut Environment, _object: MutVoidPtr, _queue: MutVoidPtr) {}

// Queue handles are never freed, so reference counting them does nothing.
fn dispatch_retain(_env: &mut Environment, _object: MutVoidPtr) {}
fn dispatch_release(_env: &mut Environment, _object: MutVoidPtr) {}

fn dispatch_async(env: &mut Environment, _queue: MutVoidPtr, block: id) {
    run_block(env, block);
}

fn dispatch_sync(env: &mut Environment, _queue: MutVoidPtr, block: id) {
    run_block(env, block);
}

fn dispatch_async_f(
    env: &mut Environment,
    _queue: MutVoidPtr,
    context: MutVoidPtr,
    work: GuestFunction, // void (*)(void *)
) {
    let () = work.call_from_host(env, (context,));
}

// Dispatch groups: everything already runs inline and synchronously, so a group
// is just a handle, and waiting on it always succeeds immediately.
fn dispatch_group_create(env: &mut Environment) -> MutVoidPtr {
    new_queue_handle(env)
}
fn dispatch_group_enter(_env: &mut Environment, _group: MutVoidPtr) {}
fn dispatch_group_leave(_env: &mut Environment, _group: MutVoidPtr) {}
fn dispatch_group_wait(_env: &mut Environment, _group: MutVoidPtr, _timeout_lo: u32, _timeout_hi: u32) -> i32 {
    0 // all work has finished
}
fn dispatch_group_async(env: &mut Environment, _group: MutVoidPtr, _queue: MutVoidPtr, block: id) {
    run_block(env, block);
}
fn dispatch_group_notify(env: &mut Environment, _group: MutVoidPtr, _queue: MutVoidPtr, block: id) {
    run_block(env, block);
}
fn dispatch_group_async_f(
    env: &mut Environment,
    _group: MutVoidPtr,
    _queue: MutVoidPtr,
    context: MutVoidPtr,
    work: GuestFunction, // void (*)(void *)
) {
    let () = work.call_from_host(env, (context,));
}

fn dispatch_sync_f(
    env: &mut Environment,
    queue: MutVoidPtr,
    context: MutVoidPtr,
    work: GuestFunction, // void (*)(void *)
) {
    dispatch_async_f(env, queue, context, work)
}

/// `dispatch_get_main_queue()` is an inline function that returns the address
/// of this global variable, so the "queue" is just the address of some memory.
pub const CONSTANTS: ConstantExports = &[(
    "__dispatch_main_q",
    HostConstant::Custom(|env| env.mem.alloc(64).cast_const()),
)];

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(dispatch_once(_, _)),
    export_c_func!(dispatch_queue_create(_, _)),
    export_c_func!(dispatch_get_global_queue(_, _)),
    export_c_func!(dispatch_get_current_queue()),
    export_c_func!(dispatch_set_target_queue(_, _)),
    export_c_func!(dispatch_retain(_)),
    export_c_func!(dispatch_release(_)),
    export_c_func!(dispatch_async(_, _)),
    export_c_func!(dispatch_sync(_, _)),
    export_c_func!(dispatch_async_f(_, _, _)),
    export_c_func!(dispatch_sync_f(_, _, _)),
    export_c_func!(dispatch_group_create()),
    export_c_func!(dispatch_group_enter(_)),
    export_c_func!(dispatch_group_leave(_)),
    export_c_func!(dispatch_group_wait(_, _, _)),
    export_c_func!(dispatch_group_async(_, _, _)),
    export_c_func!(dispatch_group_notify(_, _, _)),
    export_c_func!(dispatch_group_async_f(_, _, _, _)),
];

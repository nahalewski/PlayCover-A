/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Runtime support functions for Automatic Reference Counting (ARC).
//!
//! Code built with ARC calls these instead of sending `retain`, `release` and
//! `autorelease` messages directly. They are simple wrappers over the normal
//! message-based reference counting that the rest of touchHLE uses.
//!
//! ARC itself is iOS 5+, so touchHLE's target of iOS 4 and earlier doesn't
//! need these, but some iOS 6 era apps are close enough to partly work.
//!
//! Reference: <https://clang.llvm.org/docs/AutomaticReferenceCounting.html#runtime-support>

use super::{autorelease, id, msg, msg_class, nil, release, retain};
use crate::dyld::{export_c_func, FunctionExports};
use crate::mem::MutPtr;
use crate::Environment;

fn objc_autoreleasePoolPush(env: &mut Environment) -> id {
    msg_class![env; NSAutoreleasePool new]
}

fn objc_autoreleasePoolPop(env: &mut Environment, pool: id) {
    () = msg![env; pool drain];
}

fn objc_retain(env: &mut Environment, object: id) -> id {
    retain(env, object)
}

fn objc_release(env: &mut Environment, object: id) {
    release(env, object)
}

fn objc_autorelease(env: &mut Environment, object: id) -> id {
    autorelease(env, object)
}

// These return-value variants are optimisations that skip a retain/autorelease
// pair when the caller and callee cooperate. Doing the plain operation is
// always correct.
fn objc_retainAutoreleasedReturnValue(env: &mut Environment, object: id) -> id {
    retain(env, object)
}

fn objc_autoreleaseReturnValue(env: &mut Environment, object: id) -> id {
    autorelease(env, object)
}

fn objc_retainAutorelease(env: &mut Environment, object: id) -> id {
    let object = retain(env, object);
    autorelease(env, object)
}

fn objc_retainAutoreleaseReturnValue(env: &mut Environment, object: id) -> id {
    objc_retainAutorelease(env, object)
}

fn objc_storeStrong(env: &mut Environment, location: MutPtr<id>, value: id) {
    let old = env.mem.read(location);
    if old == value {
        return;
    }
    retain(env, value);
    env.mem.write(location, value);
    release(env, old);
}

// -- `__weak` support --------------------------------------------------

/// Forget what the weak slot currently points at.
fn weak_unregister(env: &mut Environment, slot: u32) {
    if let Some(old) = env.objc.weak_slots.remove(&slot) {
        if let Some(slots) = env.objc.weak_targets.get_mut(&old) {
            slots.retain(|&s| s != slot);
            if slots.is_empty() {
                env.objc.weak_targets.remove(&old);
            }
        }
    }
}

fn weak_store(env: &mut Environment, location: MutPtr<id>, object: id) -> id {
    let slot = location.to_bits();
    weak_unregister(env, slot);
    env.mem.write(location, object);
    // Only heap objects can be deallocated; classes and other static objects
    // never go away, so they need no tracking.
    if object != nil && env.objc.objects.contains_key(&object) {
        env.objc.weak_slots.insert(slot, object);
        env.objc.weak_targets.entry(object).or_default().push(slot);
    }
    object
}

fn objc_initWeak(env: &mut Environment, location: MutPtr<id>, object: id) -> id {
    weak_store(env, location, object)
}

fn objc_storeWeak(env: &mut Environment, location: MutPtr<id>, object: id) -> id {
    weak_store(env, location, object)
}

fn objc_destroyWeak(env: &mut Environment, location: MutPtr<id>) {
    weak_unregister(env, location.to_bits());
    env.mem.write(location, nil);
}

fn objc_loadWeakRetained(env: &mut Environment, location: MutPtr<id>) -> id {
    let object = env.mem.read(location);
    if object == nil {
        return nil;
    }
    // A tracked object still alive is retained; untracked (static) ones are
    // returned as they are.
    if env.objc.objects.contains_key(&object) {
        retain(env, object)
    } else {
        object
    }
}

fn objc_loadWeak(env: &mut Environment, location: MutPtr<id>) -> id {
    let object = objc_loadWeakRetained(env, location);
    if object == nil {
        nil
    } else {
        autorelease(env, object)
    }
}

fn objc_copyWeak(env: &mut Environment, to: MutPtr<id>, from: MutPtr<id>) {
    let object = env.mem.read(from);
    weak_store(env, to, object);
}

fn objc_moveWeak(env: &mut Environment, to: MutPtr<id>, from: MutPtr<id>) {
    let object = env.mem.read(from);
    weak_unregister(env, from.to_bits());
    env.mem.write(from, nil);
    weak_store(env, to, object);
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(objc_autoreleasePoolPush()),
    export_c_func!(objc_autoreleasePoolPop(_)),
    export_c_func!(objc_retain(_)),
    export_c_func!(objc_release(_)),
    export_c_func!(objc_autorelease(_)),
    export_c_func!(objc_retainAutoreleasedReturnValue(_)),
    export_c_func!(objc_autoreleaseReturnValue(_)),
    export_c_func!(objc_retainAutorelease(_)),
    export_c_func!(objc_retainAutoreleaseReturnValue(_)),
    export_c_func!(objc_storeStrong(_, _)),
    export_c_func!(objc_initWeak(_, _)),
    export_c_func!(objc_storeWeak(_, _)),
    export_c_func!(objc_destroyWeak(_)),
    export_c_func!(objc_loadWeakRetained(_)),
    export_c_func!(objc_loadWeak(_)),
    export_c_func!(objc_copyWeak(_, _)),
    export_c_func!(objc_moveWeak(_, _)),
];

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Blocks runtime (`Block_copy()` and friends).
//!
//! Layout and rules follow the Block ABI at
//! <https://clang.llvm.org/docs/Block-ABI-Apple.html>; this is a small
//! independent implementation of it. Heap blocks and `__block` variables are
//! reference-counted in their flags word, but they are never freed here:
//! apps that use blocks only copy a few of them, and leaking is much safer
//! than freeing something an app still calls.
//!
//! Block literals begin with an `isa` pointing at `_NSConcreteStackBlock`,
//! `_NSConcreteGlobalBlock` or `_NSConcreteMallocBlock`, which are exported
//! here as the host classes below so that `[block copy]`, `retain` etc. work.

use super::{id, nil, objc_classes, ClassExports};
use crate::abi::{CallFromHost, GuestFunction};
use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant};
use crate::mem::{ConstVoidPtr, GuestUSize, MutPtr, MutVoidPtr, Ptr};
use crate::Environment;

// Block flags.
const BLOCK_REFCOUNT_MASK: u32 = 0xffff;
const BLOCK_NEEDS_FREE: u32 = 1 << 24;
const BLOCK_HAS_COPY_DISPOSE: u32 = 1 << 25;
const BLOCK_IS_GLOBAL: u32 = 1 << 28;

// `_Block_object_assign()` / `_Block_object_dispose()` field kinds.
const BLOCK_FIELD_IS_OBJECT: i32 = 3;
const BLOCK_FIELD_IS_BLOCK: i32 = 7;
const BLOCK_FIELD_IS_BYREF: i32 = 8;
const BLOCK_FIELD_KIND_MASK: i32 = 0xf;

// Offsets inside a block literal: { isa, flags, reserved, invoke, descriptor }.
const BLOCK_FLAGS: GuestUSize = 4;
const BLOCK_DESCRIPTOR: GuestUSize = 16;
// Offsets inside a block descriptor: { reserved, size, copy, dispose }.
const DESCRIPTOR_SIZE: GuestUSize = 4;
const DESCRIPTOR_COPY: GuestUSize = 8;
const DESCRIPTOR_DISPOSE: GuestUSize = 12;
// Offsets inside a `__block` variable record:
// { isa, forwarding, flags, size, keep, destroy, ...captured value }.
const BYREF_FORWARDING: GuestUSize = 4;
const BYREF_FLAGS: GuestUSize = 8;
const BYREF_SIZE: GuestUSize = 12;
const BYREF_KEEP: GuestUSize = 16;
const BYREF_DESTROY: GuestUSize = 20;

fn read_u32(env: &mut Environment, address: u32) -> u32 {
    env.mem.read(Ptr::<u32, false>::from_bits(address))
}

fn write_u32(env: &mut Environment, address: u32, value: u32) {
    env.mem.write(Ptr::<u32, true>::from_bits(address), value)
}

fn call_helper2(env: &mut Environment, function: u32, a: u32, b: u32) {
    if function == 0 {
        return;
    }
    let function = GuestFunction::from_addr_with_thumb_bit(function);
    let () = function.call_from_host(env, (a, b));
}

fn call_helper1(env: &mut Environment, function: u32, a: u32) {
    if function == 0 {
        return;
    }
    let function = GuestFunction::from_addr_with_thumb_bit(function);
    let () = function.call_from_host(env, (a,));
}

/// `_Block_copy()`: move a stack block to the heap (a no-op for global and
/// heap blocks).
fn _Block_copy(env: &mut Environment, block: ConstVoidPtr) -> ConstVoidPtr {
    let address = block.to_bits();
    if address == 0 {
        return block;
    }
    let flags = read_u32(env, address + BLOCK_FLAGS);
    if flags & BLOCK_IS_GLOBAL != 0 {
        return block;
    }
    if flags & BLOCK_NEEDS_FREE != 0 {
        // Already on the heap: just count the new reference.
        if flags & BLOCK_REFCOUNT_MASK != BLOCK_REFCOUNT_MASK {
            write_u32(env, address + BLOCK_FLAGS, flags + 1);
        }
        return block;
    }

    let descriptor = read_u32(env, address + BLOCK_DESCRIPTOR);
    let size = read_u32(env, descriptor + DESCRIPTOR_SIZE);
    let heap: MutVoidPtr = env.mem.alloc(size);
    let heap_address = heap.to_bits();
    let bytes = env.mem.bytes_at(Ptr::<u8, false>::from_bits(address), size).to_vec();
    env.mem
        .bytes_at_mut(heap.cast::<u8>(), size)
        .copy_from_slice(&bytes);
    // A heap block belongs to the `NSMallocBlock` class and starts with one reference.
    let malloc_class = env
        .objc
        .get_known_class("__NSMallocBlock__", &mut env.mem);
    write_u32(env, heap_address, malloc_class.to_bits());
    let new_flags = (flags & !BLOCK_REFCOUNT_MASK) | BLOCK_NEEDS_FREE | 1;
    write_u32(env, heap_address + BLOCK_FLAGS, new_flags);
    if flags & BLOCK_HAS_COPY_DISPOSE != 0 {
        // The copy helper retains/copies everything the block captured.
        let copy = read_u32(env, descriptor + DESCRIPTOR_COPY);
        call_helper2(env, copy, heap_address, address);
    }
    Ptr::from_bits(heap_address)
}

/// `_Block_release()`: heap blocks are leaked on purpose (see the module docs).
fn _Block_release(_env: &mut Environment, _block: ConstVoidPtr) {}

fn objc_retainBlock(env: &mut Environment, block: ConstVoidPtr) -> ConstVoidPtr {
    _Block_copy(env, block)
}

/// `_Block_object_assign()`: called from a block's copy helper for each
/// captured variable.
fn _Block_object_assign(env: &mut Environment, destination: MutPtr<u32>, source: ConstVoidPtr, flags: i32) {
    match flags & BLOCK_FIELD_KIND_MASK {
        BLOCK_FIELD_IS_OBJECT => {
            let object: id = Ptr::from_bits(source.to_bits());
            if object != nil {
                super::retain(env, object);
            }
            env.mem.write(destination, source.to_bits());
        }
        BLOCK_FIELD_IS_BLOCK => {
            let copy = _Block_copy(env, source);
            env.mem.write(destination, copy.to_bits());
        }
        BLOCK_FIELD_IS_BYREF => {
            // `source` is a stack `__block` variable record: move it to the
            // heap once, and let every block share that copy.
            let stack = source.to_bits();
            let forwarding = read_u32(env, stack + BYREF_FORWARDING);
            let flags_word = read_u32(env, forwarding + BYREF_FLAGS);
            if flags_word & BLOCK_NEEDS_FREE != 0 {
                if flags_word & BLOCK_REFCOUNT_MASK != BLOCK_REFCOUNT_MASK {
                    write_u32(env, forwarding + BYREF_FLAGS, flags_word + 1);
                }
                env.mem.write(destination, forwarding);
                return;
            }
            let size = read_u32(env, forwarding + BYREF_SIZE);
            let heap: MutVoidPtr = env.mem.alloc(size);
            let heap_address = heap.to_bits();
            let bytes = env
                .mem
                .bytes_at(Ptr::<u8, false>::from_bits(forwarding), size)
                .to_vec();
            env.mem
                .bytes_at_mut(heap.cast::<u8>(), size)
                .copy_from_slice(&bytes);
            write_u32(env, heap_address + BYREF_FORWARDING, heap_address);
            write_u32(
                env,
                heap_address + BYREF_FLAGS,
                (flags_word & !BLOCK_REFCOUNT_MASK) | BLOCK_NEEDS_FREE | 1,
            );
            // Both the stack record and the heap copy must forward to the heap.
            write_u32(env, forwarding + BYREF_FORWARDING, heap_address);
            if flags_word & BLOCK_HAS_COPY_DISPOSE != 0 {
                let keep = read_u32(env, forwarding + BYREF_KEEP);
                call_helper2(env, keep, heap_address, forwarding);
            }
            env.mem.write(destination, heap_address);
        }
        other => {
            log!("Warning: _Block_object_assign() with unknown kind {}", other);
            env.mem.write(destination, source.to_bits());
        }
    }
}

/// `_Block_object_dispose()`: called from a block's dispose helper.
fn _Block_object_dispose(env: &mut Environment, object: ConstVoidPtr, flags: i32) {
    match flags & BLOCK_FIELD_KIND_MASK {
        BLOCK_FIELD_IS_OBJECT => {
            let object: id = Ptr::from_bits(object.to_bits());
            if object != nil {
                super::release(env, object);
            }
        }
        BLOCK_FIELD_IS_BLOCK => {}
        BLOCK_FIELD_IS_BYREF => {
            let record = object.to_bits();
            if record == 0 {
                return;
            }
            let forwarding = read_u32(env, record + BYREF_FORWARDING);
            let flags_word = read_u32(env, forwarding + BYREF_FLAGS);
            if flags_word & BLOCK_NEEDS_FREE == 0 {
                return; // still the stack copy: nothing to release
            }
            let count = flags_word & BLOCK_REFCOUNT_MASK;
            if count > 1 {
                write_u32(env, forwarding + BYREF_FLAGS, flags_word - 1);
            } else if flags_word & BLOCK_HAS_COPY_DISPOSE != 0 {
                // Last reference: release what the variable held. The record
                // itself is leaked, like heap blocks.
                let destroy = read_u32(env, forwarding + BYREF_DESTROY);
                call_helper1(env, destroy, forwarding);
                write_u32(env, forwarding + BYREF_FLAGS, flags_word & !BLOCK_REFCOUNT_MASK);
            }
        }
        _ => {}
    }
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(_Block_copy(_)),
    export_c_func!(_Block_release(_)),
    export_c_func!(objc_retainBlock(_)),
    export_c_func!(_Block_object_assign(_, _, _)),
    export_c_func!(_Block_object_dispose(_, _)),
];

/// The three `_NSConcrete*Block` symbols. A block's `isa` is the *address* of
/// one of them, which here is the class object itself.
pub const CONSTANTS: ConstantExports = &[
    (
        "__NSConcreteStackBlock",
        HostConstant::Custom(|env| {
            env.objc
                .get_known_class("__NSStackBlock__", &mut env.mem)
                .cast()
                .cast_const()
        }),
    ),
    (
        "__NSConcreteGlobalBlock",
        HostConstant::Custom(|env| {
            env.objc
                .get_known_class("__NSGlobalBlock__", &mut env.mem)
                .cast()
                .cast_const()
        }),
    ),
    (
        "__NSConcreteMallocBlock",
        HostConstant::Custom(|env| {
            env.objc
                .get_known_class("__NSMallocBlock__", &mut env.mem)
                .cast()
                .cast_const()
        }),
    ),
];

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

// Blocks are not tracked as host objects, so none of the usual reference
// counting applies to them: copying goes through `_Block_copy()`, everything
// else leaves them alone.
@implementation NSBlock: NSObject

- (id)copy {
    let copy = _Block_copy(env, this.cast().cast_const());
    Ptr::from_bits(copy.to_bits())
}
- (id)retain { this }
- (())release {}
- (id)autorelease { this }

@end

@implementation __NSStackBlock__: NSBlock
@end

@implementation __NSGlobalBlock__: NSBlock
@end

@implementation __NSMallocBlock__: NSBlock
@end

};

/// Copies `block` to the heap if needed (for host code that keeps a block
/// beyond the call that received it, e.g. a completion handler).
pub(crate) fn copy_block(env: &mut Environment, block: id) -> id {
    Ptr::from_bits(_Block_copy(env, Ptr::from_bits(block.to_bits())).to_bits())
}

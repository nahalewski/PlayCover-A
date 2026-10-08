/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSOperationQueue`.
//!
//! This is a minimal stand-in: there is no real concurrency. Operations and
//! blocks added to a queue are run immediately, on the calling thread.
//! That is enough for apps that only use the queue to hop to the main thread
//! or to fire off a bit of fire-and-forget work.

use super::NSInteger;
use crate::abi::{CallFromHost, GuestFunction};
use crate::mem::Ptr;
use crate::objc::{id, msg, nil, objc_classes, ClassExports, TrivialHostObject};
use crate::Environment;

#[derive(Default)]
pub struct State {
    main_queue: Option<id>,
}

/// Offset of the `invoke` function pointer in a block literal
/// (isa, flags, reserved, invoke).
const BLOCK_INVOKE_OFFSET: u32 = 12;

/// Call a block that takes no arguments and returns nothing.
pub(crate) fn run_block(env: &mut Environment, block: id) {
    let invoke: u32 = env.mem.read(Ptr::<u32, false>::from_bits(
        block.to_bits() + BLOCK_INVOKE_OFFSET,
    ));
    let invoke = GuestFunction::from_addr_with_thumb_bit(invoke);
    let () = invoke.call_from_host(env, (block,));
}

fn main_queue(env: &mut Environment, class: id) -> id {
    if let Some(queue) = env.framework_state.foundation.ns_operation_queue.main_queue {
        queue
    } else {
        let queue = env
            .objc
            .alloc_static_object(class, Box::new(TrivialHostObject), &mut env.mem);
        env.framework_state.foundation.ns_operation_queue.main_queue = Some(queue);
        queue
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSOperationQueue: NSObject

+ (id)mainQueue {
    main_queue(env, this)
}

+ (id)currentQueue {
    main_queue(env, this)
}

// The main queue is a static (never freed) object: reference counting is a no-op.
- (id)retain { this }
- (())release {}
- (id)autorelease { this }

- (())addOperationWithBlock:(id)block { // void (^)(void)
    if block != nil {
        run_block(env, block);
    }
}

- (())addOperation:(id)op { // NSOperation *
    if op != nil {
        () = msg![env; op main];
    }
}

- (())setMaxConcurrentOperationCount:(NSInteger)_count {}
- (NSInteger)maxConcurrentOperationCount {
    -1 // NSOperationQueueDefaultMaxConcurrentOperationCount
}

- (())setName:(id)_name {} // NSString *

- (bool)isSuspended { false }
- (())setSuspended:(bool)_suspended {}

- (u32)operationCount { 0 }

- (())cancelAllOperations {}
- (())waitUntilAllOperationsAreFinished {}

@end

};

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIEvent`.

use super::ui_touch::UITouchHostObject;
use crate::frameworks::foundation::{NSTimeInterval, NSUInteger};
use crate::mem::MutVoidPtr;
use crate::objc::{
    id, msg, msg_class, nil, objc_classes, release, retain, ClassExports, HostObject, NSZonePtr,
};
use crate::Environment;

pub(super) struct UIEventHostObject {
    /// `NSSet<UITouch*>*`
    touches: id,
    timestamp: NSTimeInterval,
}
impl HostObject for UIEventHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIEvent: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(UIEventHostObject {
        touches: nil,
        timestamp: 0.0,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (())dealloc {
    let &UIEventHostObject { touches, .. } = env.objc.borrow(this);
    release(env, touches);
}

- (NSTimeInterval)timestamp {
    env.objc.borrow::<UIEventHostObject>(this).timestamp
}

- (id)touchesForView:(id)view_ {
    let &UIEventHostObject { touches, .. } = env.objc.borrow(this);

    let touches_for_view: id = msg_class![env; NSMutableSet allocWithZone:(MutVoidPtr::null())];

    let touches_arr: id = msg![env; touches allObjects];
    let touches_count: NSUInteger = msg![env; touches_arr count];
    for i in 0..touches_count {
        let touch: id = msg![env; touches_arr objectAtIndex:i];
        let &UITouchHostObject { view, .. } = env.objc.borrow(touch);
        if view_ == view {
            let _: () = msg![env; touches_for_view addObject:touch];
            if !msg![env; view isMultipleTouchEnabled] {
                break;
            }
        }
    }

    touches_for_view
}

- (id)allTouches {
    let &UIEventHostObject { touches, .. } = env.objc.borrow(this);
    touches
}

// TODO: more accessors

@end

};

/// For use by [super::ui_touch]: get the `UIEvent` carrying a set of `UITouch*`.
///
/// On iOS the same event object is reused for all touch events and apps may
/// hold on to it without retaining it, so a single instance is kept alive
/// forever and updated in place. The returned reference is +1 (callers
/// autorelease it, as with a freshly allocated object).
pub(super) fn new_event(env: &mut Environment, touches: id) -> id {
    let event: id = if let Some(event) = env.framework_state.uikit.ui_touch.shared_event {
        retain(env, event)
    } else {
        let event: id = msg_class![env; UIEvent alloc];
        // The extra reference is the one the state keeps forever.
        retain(env, event);
        env.framework_state.uikit.ui_touch.shared_event = Some(event);
        event
    };
    retain(env, touches);
    let timestamp: NSTimeInterval = {
        let process_info = msg_class![env; NSProcessInfo processInfo];
        msg![env; process_info systemUptime]
    };
    let old_touches = {
        let borrow = env.objc.borrow_mut::<UIEventHostObject>(event);
        let old = borrow.touches;
        borrow.touches = touches;
        borrow.timestamp = timestamp;
        old
    };
    release(env, old_touches);
    event
}

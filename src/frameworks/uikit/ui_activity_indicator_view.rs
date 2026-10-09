/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIActivityIndicatorView`.

use crate::frameworks::foundation::NSInteger;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, objc_classes, todo_objc_setter, ClassExports,
    NSZonePtr,
};

use std::sync::atomic::{AtomicU32, Ordering};

type UIActivityIndicatorViewStyle = NSInteger;

/// Number of activity indicators currently animating. Apps show one while
/// loading, so the frame limiter is lifted then (see `eagl.rs`): loading is
/// often paced per frame and a low gameplay cap makes it needlessly slow.
static ANIMATING_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn any_animating() -> bool {
    ANIMATING_COUNT.load(Ordering::Relaxed) > 0
}

pub struct UIActivityIndicatorViewHostObject {
    superclass: super::ui_view::UIViewHostObject,
    animating: bool,
}
impl_HostObject_with_superclass!(UIActivityIndicatorViewHostObject);

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIActivityIndicatorView: UIView

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(UIActivityIndicatorViewHostObject {
        superclass: Default::default(),
        animating: false,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithActivityIndicatorStyle:(UIActivityIndicatorViewStyle)_style {
    // TODO: proper init
    msg![env; this init]
}

- (())setActivityIndicatorViewStyle:(UIActivityIndicatorViewStyle)style {
    todo_objc_setter!(this, style);
}

- (())startAnimating {
    log!("TODO: [(UIActivityIndicatorView *){:?} startAnimating]", this);
    let host_object = env.objc.borrow_mut::<UIActivityIndicatorViewHostObject>(this);
    if !host_object.animating {
        host_object.animating = true;
        ANIMATING_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}
- (())stopAnimating {
    log!("TODO: [(UIActivityIndicatorView *){:?} stopAnimating]", this);
    let host_object = env.objc.borrow_mut::<UIActivityIndicatorViewHostObject>(this);
    if host_object.animating {
        host_object.animating = false;
        ANIMATING_COUNT.fetch_sub(1, Ordering::Relaxed);
    }
}

- (bool)isAnimating {
    env.objc.borrow::<UIActivityIndicatorViewHostObject>(this).animating
}

- (())setHidesWhenStopped:(bool)_hides {
    // TODO
}

@end

};

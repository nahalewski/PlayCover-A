/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIGestureRecognizer`.
//!
//! Stub: gesture recognizers are accepted but never recognise anything
//! (touchHLE only delivers plain touch events). Apps whose gestures are an
//! optional extra keep working; ones that depend on them won't.

use crate::frameworks::core_graphics::CGPoint;
use crate::objc::{id, nil, objc_classes, ClassExports, SEL};

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIGestureRecognizer: NSObject

- (id)initWithTarget:(id)_target action:(SEL)_action {
    log!("TODO: UIGestureRecognizer is a stub and will never recognise any gesture");
    this
}

- (())addTarget:(id)_target action:(SEL)_action {}
- (())removeTarget:(id)_target action:(SEL)_action {}

- (id)delegate { nil }
- (())setDelegate:(id)_delegate {}

- (bool)isEnabled { true }
- (())setEnabled:(bool)_enabled {}

- (id)view { nil }

- (CGPoint)locationInView:(id)_view { // UIView *
    CGPoint { x: 0.0, y: 0.0 }
}

@end

};

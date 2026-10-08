/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIProgressView`.
//!
//! Drawn as a plain track with a filled portion, not the real iPhone OS
//! gradient bar.

use crate::frameworks::core_graphics::cg_context::{CGContextFillRect, CGContextSetRGBFillColor};
use crate::frameworks::core_graphics::{CGFloat, CGRect, CGSize};
use crate::frameworks::foundation::NSInteger;
use crate::frameworks::uikit::ui_graphics::UIGraphicsGetCurrentContext;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_super, objc_classes, ClassExports, NSZonePtr,
};

#[derive(Default)]
struct UIProgressViewHostObject {
    superclass: super::UIViewHostObject,
    progress: f32,
    style: NSInteger,
}
impl_HostObject_with_superclass!(UIProgressViewHostObject);

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIProgressView: UIView

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<UIProgressViewHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithProgressViewStyle:(NSInteger)style {
    // The real class is 9 points high (11 for the bar style) and as wide as
    // its superview gives it; apps set the frame afterwards anyway.
    let frame = CGRect {
        origin: crate::frameworks::core_graphics::CGPoint { x: 0.0, y: 0.0 },
        size: CGSize { width: 150.0, height: 9.0 },
    };
    let this: id = msg_super![env; this initWithFrame:frame];
    () = msg![env; this setOpaque:false];
    env.objc.borrow_mut::<UIProgressViewHostObject>(this).style = style;
    this
}

- (id)initWithFrame:(CGRect)frame {
    let this: id = msg_super![env; this initWithFrame:frame];
    () = msg![env; this setOpaque:false];
    this
}

- (NSInteger)progressViewStyle {
    env.objc.borrow::<UIProgressViewHostObject>(this).style
}
- (())setProgressViewStyle:(NSInteger)style {
    env.objc.borrow_mut::<UIProgressViewHostObject>(this).style = style;
}

- (f32)progress {
    env.objc.borrow::<UIProgressViewHostObject>(this).progress
}
- (())setProgress:(f32)progress {
    env.objc.borrow_mut::<UIProgressViewHostObject>(this).progress = progress.clamp(0.0, 1.0);
    () = msg![env; this setNeedsDisplay];
}
- (())setProgress:(f32)progress animated:(bool)_animated {
    () = msg![env; this setProgress:progress];
}

- (())drawRect:(CGRect)rect {
    let progress = env.objc.borrow::<UIProgressViewHostObject>(this).progress;
    let context = UIGraphicsGetCurrentContext(env);
    CGContextSetRGBFillColor(env, context, 0.8, 0.8, 0.8, 1.0);
    CGContextFillRect(env, context, rect);
    let mut fill = rect;
    fill.size.width *= progress as CGFloat;
    CGContextSetRGBFillColor(env, context, 0.2, 0.45, 0.95, 1.0);
    CGContextFillRect(env, context, fill);
}

@end

};

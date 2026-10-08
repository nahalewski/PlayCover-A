/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIImageView`.

use crate::frameworks::core_graphics::cg_image::CGImageRef;
use crate::frameworks::core_graphics::{CGPoint, CGRect, CGSize};
use crate::frameworks::foundation::ns_string::get_static_str;
use crate::frameworks::foundation::NSTimeInterval;
use crate::frameworks::core_foundation::time::CFAbsoluteTimeGetCurrent;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_class, msg_super, nil, objc_classes, release,
    retain, ClassExports, NSZonePtr,
};
use crate::Environment;

#[derive(Default)]
struct UIImageViewHostObject {
    superclass: super::UIViewHostObject,
    /// `UIImage*`
    image: id,
    /// `NSArray<UIImage*>*`, retained
    animation_images: id,
    animation_duration: NSTimeInterval,
    /// 0 means repeat forever
    animation_repeat_count: i32,
    animating: bool,
    /// `NSTimer*` driving the animation (the timer retains the view)
    animation_timer: id,
    /// `CFAbsoluteTime` at which the animation started
    animation_start: f64,
}
impl_HostObject_with_superclass!(UIImageViewHostObject);

fn stop_animation_timer(env: &mut Environment, view: id) {
    let timer = {
        let host_obj = env.objc.borrow_mut::<UIImageViewHostObject>(view);
        host_obj.animating = false;
        std::mem::replace(&mut host_obj.animation_timer, nil)
    };
    if timer != nil {
        () = msg![env; timer invalidate];
        release(env, timer);
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIImageView: UIView

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<UIImageViewHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithFrame:(CGRect)frame {
    let this: id = msg_super![env; this initWithFrame:frame];
    // Not sure if UIImageView does this unconditionally, or only for images
    // with alpha channels.
    () = msg![env; this setOpaque:false];
    // Unlike other views, image views ignore touches by default, so that
    // decorative images drawn over buttons don't swallow their events.
    () = msg![env; this setUserInteractionEnabled:false];
    this
}

- (())dealloc {
    stop_animation_timer(env, this);
    let &UIImageViewHostObject {
        image,
        animation_images,
        ..
    } = env.objc.borrow(this);
    release(env, image);
    release(env, animation_images);
    msg_super![env; this dealloc]
}

// NSCoding implementation
- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];

    let key_ns_string = get_static_str(env, "UIImage");
    let image: id = msg![env; coder decodeObjectForKey:key_ns_string];

    () = msg![env; this setImage:image];

    this
}

- (id)initWithImage:(id)image { // UIImage*
    let size: CGSize = msg![env; image size];
    let frame = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size
    };
    let this = msg_super![env; this initWithFrame:frame];
    () = msg![env; this setImage:image];
    // Not sure if UIImageView does this unconditionally, or only for images
    // with alpha channels.
    () = msg![env; this setOpaque:false];
    () = msg![env; this setUserInteractionEnabled:false];
    this
}

- (id)image {
    env.objc.borrow::<UIImageViewHostObject>(this).image
}

- (())setImage:(id)new_image { // UIImage*
    let host_obj = env.objc.borrow_mut::<UIImageViewHostObject>(this);
    let old_image = std::mem::replace(&mut host_obj.image, new_image);
    retain(env, new_image);
    release(env, old_image);

    let layer: id = msg![env; this layer];
    let cg_image: CGImageRef = msg![env; new_image CGImage];
    () = msg![env; layer setContents:cg_image];
}

- (())setAnimationImages:(id)images { // NSArray<UIImage *>*
    retain(env, images);
    let old = std::mem::replace(
        &mut env.objc.borrow_mut::<UIImageViewHostObject>(this).animation_images,
        images,
    );
    release(env, old);
    // Until the animation runs, the first frame is shown.
    if images != nil {
        let count: crate::frameworks::foundation::NSUInteger = msg![env; images count];
        if count > 0 && env.objc.borrow::<UIImageViewHostObject>(this).image == nil {
            let first_image: id = msg![env; images objectAtIndex:0u32];
            () = msg![env; this setImage:first_image];
        }
    }
}

- (id)animationImages {
    env.objc.borrow::<UIImageViewHostObject>(this).animation_images
}

- (())setAnimationDuration:(NSTimeInterval)duration {
    env.objc.borrow_mut::<UIImageViewHostObject>(this).animation_duration = duration;
}

- (NSTimeInterval)animationDuration {
    env.objc.borrow::<UIImageViewHostObject>(this).animation_duration
}

- (())setAnimationRepeatCount:(i32)count {
    env.objc.borrow_mut::<UIImageViewHostObject>(this).animation_repeat_count = count;
}

- (i32)animationRepeatCount {
    env.objc.borrow::<UIImageViewHostObject>(this).animation_repeat_count
}

- (bool)isAnimating {
    env.objc.borrow::<UIImageViewHostObject>(this).animating
}

- (())startAnimating {
    let images = env.objc.borrow::<UIImageViewHostObject>(this).animation_images;
    if images == nil {
        return;
    }
    let count: crate::frameworks::foundation::NSUInteger = msg![env; images count];
    if count == 0 {
        return;
    }
    stop_animation_timer(env, this);
    let now = CFAbsoluteTimeGetCurrent(env);
    {
        let host_obj = env.objc.borrow_mut::<UIImageViewHostObject>(this);
        host_obj.animating = true;
        host_obj.animation_start = now;
        if host_obj.animation_duration <= 0.0 {
            // UIKit's default is 1/30 s per frame.
            host_obj.animation_duration = count as f64 / 30.0;
        }
    }
    let duration = env.objc.borrow::<UIImageViewHostObject>(this).animation_duration;
    let interval = (duration / count as f64).max(1.0 / 60.0);
    let selector = env.objc.lookup_selector("_touchHLE_advanceAnimation:").unwrap();
    let timer: id = msg_class![env; NSTimer scheduledTimerWithTimeInterval:interval
                                                                     target:this
                                                                   selector:selector
                                                                   userInfo:nil
                                                                    repeats:true];
    retain(env, timer);
    env.objc.borrow_mut::<UIImageViewHostObject>(this).animation_timer = timer;
    () = msg![env; this _touchHLE_advanceAnimation:nil];
}

- (())stopAnimating {
    stop_animation_timer(env, this);
}

// Timer callback for startAnimating.
- (())_touchHLE_advanceAnimation:(id)_timer {
    let &UIImageViewHostObject {
        animation_images: images,
        animation_duration: duration,
        animation_repeat_count: repeat_count,
        animation_start: start,
        animating,
        ..
    } = env.objc.borrow(this);
    if !animating || images == nil {
        return;
    }
    let count: crate::frameworks::foundation::NSUInteger = msg![env; images count];
    if count == 0 {
        return;
    }
    let elapsed = CFAbsoluteTimeGetCurrent(env) - start;
    if repeat_count > 0 && elapsed >= duration * f64::from(repeat_count) {
        // Finished: the last frame stays on screen.
        let last: id = msg![env; images objectAtIndex:(count - 1)];
        () = msg![env; this setImage:last];
        stop_animation_timer(env, this);
        return;
    }
    let frame = ((elapsed / duration * count as f64) as u64 % count as u64) as crate::frameworks::foundation::NSUInteger;
    let image: id = msg![env; images objectAtIndex:frame];
    () = msg![env; this setImage:image];
}

@end

};

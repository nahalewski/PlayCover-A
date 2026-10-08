/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UISlider`.

use crate::frameworks::core_graphics::{CGPoint, CGRect, CGSize};
use crate::frameworks::foundation::ns_string::get_static_str;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_class, msg_super, nil, objc_classes, release,
    retain, ClassExports, NSZonePtr,
};
use crate::Environment;

#[derive(Clone, Copy)]
struct SliderState {
    minimum: f32,
    maximum: f32,
    value: f32,
    continuous: bool,
}
impl Default for SliderState {
    fn default() -> Self {
        Self {
            minimum: 0.0,
            maximum: 1.0,
            value: 0.0,
            continuous: true,
        }
    }
}
impl SliderState {
    fn set_value(&mut self, value: f32) {
        if !value.is_nan() {
            self.value = value.clamp(self.minimum, self.maximum);
        }
    }
    fn set_minimum(&mut self, minimum: f32) {
        if !minimum.is_finite() {
            return;
        }
        self.minimum = minimum;
        self.maximum = self.maximum.max(minimum);
        self.set_value(self.value);
    }
    fn set_maximum(&mut self, maximum: f32) {
        if !maximum.is_finite() {
            return;
        }
        self.maximum = maximum;
        self.minimum = self.minimum.min(maximum);
        self.set_value(self.value);
    }
    fn fraction(&self) -> f32 {
        if self.maximum == self.minimum {
            0.0
        } else {
            (self.value - self.minimum) / (self.maximum - self.minimum)
        }
    }
    fn set_position(&mut self, x: f32, width: f32) {
        let fraction = if width > 20.0 {
            ((x - 10.0) / (width - 20.0)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.set_value(self.minimum + fraction * (self.maximum - self.minimum));
    }
}

#[derive(Default)]
struct UISliderHostObject {
    superclass: super::UIControlHostObject,
    state: SliderState,
    minimum_image: id,
    maximum_image: id,
    track: id,
    filled_track: id,
    thumb: id,
}
impl_HostObject_with_superclass!(UISliderHostObject);

fn track_rect(bounds: CGRect) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: bounds.origin.x + 10.0,
            y: bounds.origin.y + bounds.size.height / 2.0 - 2.0,
        },
        size: CGSize {
            width: (bounds.size.width - 20.0).max(0.0),
            height: 4.0,
        },
    }
}

fn thumb_rect(track: CGRect, fraction: f32) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: track.origin.x + track.size.width * fraction.clamp(0.0, 1.0) - 10.0,
            y: track.origin.y + track.size.height / 2.0 - 10.0,
        },
        size: CGSize {
            width: 20.0,
            height: 20.0,
        },
    }
}

fn init_common(env: &mut Environment, this: id) -> id {
    let track: id = msg_class![env; UIView new];
    let filled_track: id = msg_class![env; UIView new];
    let thumb: id = msg_class![env; UIView new];
    let gray: id = msg_class![env; UIColor lightGrayColor];
    let blue: id = msg_class![env; UIColor blueColor];
    let white: id = msg_class![env; UIColor whiteColor];
    () = msg![env; track setBackgroundColor:gray];
    () = msg![env; filled_track setBackgroundColor:blue];
    () = msg![env; thumb setBackgroundColor:white];
    for view in [track, filled_track, thumb] {
        () = msg![env; view setUserInteractionEnabled:false];
        () = msg![env; this addSubview:view];
        release(env, view); // retained by the slider's subview array
    }
    let host = env.objc.borrow_mut::<UISliderHostObject>(this);
    host.track = track;
    host.filled_track = filled_track;
    host.thumb = thumb;
    () = msg![env; this layoutSubviews];
    this
}

fn update_touch(env: &mut Environment, this: id, touch: id, event: id, ended: bool) {
    if touch == nil {
        return;
    }
    let point: CGPoint = msg![env; touch locationInView:this];
    let bounds: CGRect = msg![env; this bounds];
    let track: CGRect = msg![env; this trackRectForBounds:bounds];
    let host = env.objc.borrow_mut::<UISliderHostObject>(this);
    let old_value = host.state.value;
    // Use the same overridable track geometry as rendering.
    host.state
        .set_position(point.x - track.origin.x + 10.0, track.size.width + 20.0);
    let notify = if host.state.continuous {
        old_value != host.state.value
    } else {
        ended
    };
    () = msg![env; this layoutSubviews];
    if notify {
        super::send_actions(env, this, event, super::UIControlEventValueChanged);
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UISlider: UIControl

+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::<UISliderHostObject>::default(), &mut env.mem)
}

- (())dealloc {
    let host = env.objc.borrow_mut::<UISliderHostObject>(this);
    let minimum = std::mem::replace(&mut host.minimum_image, nil);
    let maximum = std::mem::replace(&mut host.maximum_image, nil);
    release(env, minimum);
    release(env, maximum);
    msg_super![env; this dealloc]
}

- (id)initWithFrame:(CGRect)frame {
    let this: id = msg_super![env; this initWithFrame:frame];
    init_common(env, this)
}

// NSCoding implementation
- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];
    let mut state = SliderState::default();
    for (key, field) in [("UIMinValue", 0), ("UIMaxValue", 1), ("UIValue", 2)] {
        let key = get_static_str(env, key);
        if msg![env; coder containsValueForKey:key] {
            let value: f32 = msg![env; coder decodeFloatForKey:key];
            match field { 0 => state.set_minimum(value), 1 => state.set_maximum(value), _ => state.set_value(value) }
        }
    }
    env.objc.borrow_mut::<UISliderHostObject>(this).state = state;
    init_common(env, this)
}

- (f32)value { env.objc.borrow::<UISliderHostObject>(this).state.value }
- (())setValue:(f32)value {
    env.objc.borrow_mut::<UISliderHostObject>(this).state.set_value(value);
    () = msg![env; this layoutSubviews];
}
- (())setValue:(f32)value animated:(bool)_animated {
    // Apply the destination immediately; no UIView animation scheduler exists.
    () = msg![env; this setValue:value];
}
- (f32)minimumValue { env.objc.borrow::<UISliderHostObject>(this).state.minimum }
- (())setMinimumValue:(f32)value {
    env.objc.borrow_mut::<UISliderHostObject>(this).state.set_minimum(value);
    () = msg![env; this layoutSubviews];
}
- (f32)maximumValue { env.objc.borrow::<UISliderHostObject>(this).state.maximum }
- (())setMaximumValue:(f32)value {
    env.objc.borrow_mut::<UISliderHostObject>(this).state.set_maximum(value);
    () = msg![env; this layoutSubviews];
}
- (bool)isContinuous { env.objc.borrow::<UISliderHostObject>(this).state.continuous }
- (())setContinuous:(bool)value { env.objc.borrow_mut::<UISliderHostObject>(this).state.continuous = value; }

- (())layoutSubviews {
    () = msg_super![env; this layoutSubviews];
    let bounds: CGRect = msg![env; this bounds];
    let host = env.objc.borrow::<UISliderHostObject>(this);
    let (track, filled, thumb, fraction, value) = (host.track, host.filled_track, host.thumb, host.state.fraction(), host.state.value);
    let track_rect: CGRect = msg![env; this trackRectForBounds:bounds];
    () = msg![env; track setFrame:track_rect];
    let filled_rect = CGRect { size: CGSize { width: track_rect.size.width * fraction, height: track_rect.size.height }, ..track_rect };
    () = msg![env; filled setFrame:filled_rect];
    let thumb_rect: CGRect = msg![env; this thumbRectForBounds:bounds trackRect:track_rect value:value];
    () = msg![env; thumb setFrame:thumb_rect];
}

- (CGRect)trackRectForBounds:(CGRect)bounds { track_rect(bounds) }
- (CGRect)thumbRectForBounds:(CGRect)_bounds trackRect:(CGRect)track value:(f32)value {
    let mut state = env.objc.borrow::<UISliderHostObject>(this).state;
    state.set_value(value);
    thumb_rect(track, state.fraction())
}

- (bool)beginTrackingWithTouch:(id)touch withEvent:(id)event {
    if !msg![env; this isEnabled] { return false; }
    update_touch(env, this, touch, event, false);
    true
}
- (bool)continueTrackingWithTouch:(id)touch withEvent:(id)event {
    update_touch(env, this, touch, event, false);
    true
}
- (())endTrackingWithTouch:(id)touch withEvent:(id)event {
    update_touch(env, this, touch, event, true);
    msg_super![env; this endTrackingWithTouch:touch withEvent:event]
}

- (())setMinimumValueImage:(id)img { // UIImage *
    retain(env, img);
    let old = std::mem::replace(&mut env.objc.borrow_mut::<UISliderHostObject>(this).minimum_image, img);
    release(env, old);
}
- (())setMaximumValueImage:(id)img { // UIImage *
    retain(env, img);
    let old = std::mem::replace(&mut env.objc.borrow_mut::<UISliderHostObject>(this).maximum_image, img);
    release(env, old);
}
- (id)minimumValueImage { env.objc.borrow::<UISliderHostObject>(this).minimum_image }
- (id)maximumValueImage { env.objc.borrow::<UISliderHostObject>(this).maximum_image }

@end

};

#[test]
fn slider_range_and_touch_position() {
    let mut state = SliderState::default();
    state.set_value(2.0);
    assert_eq!(state.value, 1.0);
    state.set_value(-2.0);
    assert_eq!(state.value, 0.0);
    state.set_minimum(2.0);
    assert_eq!((state.minimum, state.maximum, state.value), (2.0, 2.0, 2.0));
    state.set_maximum(6.0);
    state.set_position(60.0, 120.0);
    assert_eq!(state.value, 4.0);
    state.set_position(-10.0, 120.0);
    assert_eq!(state.value, 2.0);
    state.set_position(200.0, 120.0);
    assert_eq!(state.value, 6.0);
    state.set_maximum(1.0);
    assert_eq!((state.minimum, state.maximum, state.value), (1.0, 1.0, 1.0));
    assert_eq!(state.fraction(), 0.0);
    state.set_value(f32::NAN);
    assert_eq!(state.value, 1.0);
}

#[test]
fn slider_geometry_uses_bounds_origin_and_track() {
    let bounds = CGRect {
        origin: CGPoint { x: 12.0, y: 7.0 },
        size: CGSize {
            width: 120.0,
            height: 40.0,
        },
    };
    let track = track_rect(bounds);
    assert_eq!(
        (track.origin.x, track.origin.y, track.size.width),
        (22.0, 25.0, 100.0)
    );
    let left = thumb_rect(track, 0.0);
    let middle = thumb_rect(track, 0.5);
    let right = thumb_rect(track, 1.0);
    assert_eq!(
        (left.origin.x, middle.origin.x, right.origin.x),
        (12.0, 62.0, 112.0)
    );
    let middle_y = middle.origin.y;
    assert_eq!(middle_y, 17.0);
    let custom_track = CGRect {
        origin: CGPoint { x: 30.0, y: 45.0 },
        size: CGSize {
            width: 80.0,
            height: 6.0,
        },
    };
    let custom_thumb = thumb_rect(custom_track, 0.5);
    assert_eq!((custom_thumb.origin.x, custom_thumb.origin.y), (60.0, 38.0));
    let narrow_width = track_rect(CGRect {
        size: CGSize {
            width: 5.0,
            height: 20.0,
        },
        ..bounds
    })
    .size
    .width;
    assert_eq!(narrow_width, 0.0);
}

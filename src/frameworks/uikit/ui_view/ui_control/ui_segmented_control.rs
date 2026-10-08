/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UISegmentedControl`.

use crate::frameworks::core_graphics::{CGPoint, CGRect, CGSize};
use crate::frameworks::foundation::ns_string::get_static_str;
use crate::frameworks::foundation::{NSInteger, NSUInteger};
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_class, msg_super, nil, objc_classes, release,
    retain, ClassExports, NSZonePtr,
};
use crate::Environment;

struct SegmentedHostObject {
    superclass: super::UIControlHostObject,
    segments: Vec<id>,
    selected: NSInteger,
    momentary: bool,
}
impl_HostObject_with_superclass!(SegmentedHostObject);
impl Default for SegmentedHostObject {
    fn default() -> Self {
        Self {
            superclass: Default::default(),
            segments: Vec::new(),
            selected: -1,
            momentary: false,
        }
    }
}
#[derive(Default)]
struct SegmentHostObject {
    superclass: super::UIControlHostObject,
    title: id,
    label: id,
}
impl_HostObject_with_superclass!(SegmentHostObject);
fn segment_at(point: CGPoint, bounds: CGRect, count: usize) -> Option<usize> {
    if count == 0
        || bounds.size.width <= 0.0
        || !point.x.is_finite()
        || !point.y.is_finite()
        || point.x < bounds.origin.x
        || point.x >= bounds.origin.x + bounds.size.width
        || point.y < bounds.origin.y
        || point.y >= bounds.origin.y + bounds.size.height
    {
        return None;
    }
    Some((((point.x - bounds.origin.x) / bounds.size.width) * count as f32) as usize)
}
fn update(env: &mut Environment, this: id) {
    let host = env.objc.borrow::<SegmentedHostObject>(this);
    let (segments, selected) = (host.segments.clone(), host.selected);
    let bounds: CGRect = msg![env; this bounds];
    let width = bounds.size.width / segments.len().max(1) as f32;
    let blue: id =
        msg_class![env; UIColor colorWithRed:0.15f32 green:0.4f32 blue:0.8f32 alpha:1.0f32];
    let white: id = msg_class![env; UIColor whiteColor];
    let gray: id = msg_class![env; UIColor lightGrayColor];
    for (i, segment) in segments.into_iter().enumerate() {
        let frame = CGRect {
            origin: CGPoint {
                x: bounds.origin.x + width * i as f32,
                y: bounds.origin.y,
            },
            size: CGSize {
                width,
                height: bounds.size.height,
            },
        };
        () = msg![env; segment setFrame:frame];
        let selected = selected == i as NSInteger;
        () = msg![env; segment setSelected:selected];
        () = msg![env; segment setBackgroundColor:(if selected { blue } else { gray })];
        let label = env.objc.borrow::<SegmentHostObject>(segment).label;
        () = msg![env; label setTextColor:(if selected { white } else { blue })];
        () = msg![env; segment layoutSubviews];
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UISegmentedControl: UIControl

+ (id)allocWithZone:(NSZonePtr)_zone { env.objc.alloc_object(this, Box::<SegmentedHostObject>::default(), &mut env.mem) }

- (id)initWithFrame:(CGRect)frame {
    msg_super![env; this initWithFrame:frame]
}

// NSCoding implementation
- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];
    let key = get_static_str(env, "UISegments");
    let array: id = msg![env; coder decodeObjectForKey:key];
    let count: NSUInteger = msg![env; array count];
    for i in 0..count {
        let segment: id = msg![env; array objectAtIndex:i];
        retain(env, segment);
        env.objc.borrow_mut::<SegmentedHostObject>(this).segments.push(segment);
        () = msg![env; this addSubview:segment];
    }
    let key = get_static_str(env, "UISelectedSegmentIndex");
    if msg![env; coder containsValueForKey:key] {
        let value: NSInteger = msg![env; coder decodeIntForKey:key];
        () = msg![env; this setSelectedSegmentIndex:value];
    }
    let key = get_static_str(env, "UIMomentary");
    let value: bool = msg![env; coder decodeBoolForKey:key];
    env.objc.borrow_mut::<SegmentedHostObject>(this).momentary = value;
    update(env, this);
    this
}

- (())dealloc {
    let segments = std::mem::take(&mut env.objc.borrow_mut::<SegmentedHostObject>(this).segments);
    for segment in segments { release(env, segment); }
    msg_super![env; this dealloc]
}
- (NSUInteger)numberOfSegments { env.objc.borrow::<SegmentedHostObject>(this).segments.len() as NSUInteger }
- (NSInteger)selectedSegmentIndex { env.objc.borrow::<SegmentedHostObject>(this).selected }
- (())setSelectedSegmentIndex:(NSInteger)index {
    let host = env.objc.borrow_mut::<SegmentedHostObject>(this);
    host.selected = if index >= 0 && (index as usize) < host.segments.len() { index } else { -1 };
    update(env, this);
}
- (bool)isMomentary { env.objc.borrow::<SegmentedHostObject>(this).momentary }
- (())setMomentary:(bool)value { env.objc.borrow_mut::<SegmentedHostObject>(this).momentary = value; }
- (())layoutSubviews { () = msg_super![env; this layoutSubviews]; update(env, this); }
- (id)titleForSegmentAtIndex:(NSUInteger)index {
    let segment = env.objc.borrow::<SegmentedHostObject>(this).segments[index as usize];
    env.objc.borrow::<SegmentHostObject>(segment).title
}
- (())setTitle:(id)title forSegmentAtIndex:(NSUInteger)index {
    let segment = env.objc.borrow::<SegmentedHostObject>(this).segments[index as usize];
    () = msg![env; segment _touchHLESetTitle:title];
}
- (())setEnabled:(bool)enabled forSegmentAtIndex:(NSUInteger)index {
    let segment = env.objc.borrow::<SegmentedHostObject>(this).segments[index as usize];
    () = msg![env; segment setEnabled:enabled];
    () = msg![env; segment setAlpha:(if enabled { 1.0f32 } else { 0.5f32 })];
}
- (bool)isEnabledForSegmentAtIndex:(NSUInteger)index {
    let segment = env.objc.borrow::<SegmentedHostObject>(this).segments[index as usize];
    msg![env; segment isEnabled]
}
- (bool)beginTrackingWithTouch:(id)_touch withEvent:(id)_event { msg![env; this isEnabled] }
- (())endTrackingWithTouch:(id)touch withEvent:(id)event {
    let point: CGPoint = msg![env; touch locationInView:this];
    let bounds: CGRect = msg![env; this bounds];
    let host = env.objc.borrow::<SegmentedHostObject>(this);
    let (index, old, momentary) = (segment_at(point, bounds, host.segments.len()), host.selected, host.momentary);
    if let Some(index) = index {
        let enabled: bool = msg![env; this isEnabledForSegmentAtIndex:(index as NSUInteger)];
        if enabled {
            () = msg![env; this setSelectedSegmentIndex:(index as NSInteger)];
            if momentary || old != index as NSInteger { super::send_actions(env, this, event, super::UIControlEventValueChanged); }
            if momentary { () = msg![env; this setSelectedSegmentIndex:(-1i32)]; }
        }
    }
    () = msg_super![env; this endTrackingWithTouch:touch withEvent:event];
}

@end

// Undocumented class used by UISegmentedControl
@implementation UISegment: UIControl

+ (id)allocWithZone:(NSZonePtr)_zone { env.objc.alloc_object(this, Box::<SegmentHostObject>::default(), &mut env.mem) }

- (id)initWithFrame:(CGRect)frame {
    msg_super![env; this initWithFrame:frame]
}

// NSCoding implementation
- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];
    let label: id = msg_class![env; UILabel new];
    () = msg![env; label setTextAlignment:1i32];
    () = msg![env; label setUserInteractionEnabled:false];
    () = msg![env; this addSubview:label];
    env.objc.borrow_mut::<SegmentHostObject>(this).label = label;
    release(env, label);
    () = msg![env; this setUserInteractionEnabled:false];
    let key = get_static_str(env, "UISegmentInfo");
    let title: id = msg![env; coder decodeObjectForKey:key];
    () = msg![env; this _touchHLESetTitle:title];
    this
}

- (())_touchHLESetTitle:(id)title {
    let title: id = msg![env; title copy];
    let host = env.objc.borrow_mut::<SegmentHostObject>(this);
    let old = std::mem::replace(&mut host.title, title);
    let label = host.label;
    release(env, old);
    () = msg![env; label setText:title];
}
- (())layoutSubviews {
    () = msg_super![env; this layoutSubviews];
    let bounds: CGRect = msg![env; this bounds];
    let label = env.objc.borrow::<SegmentHostObject>(this).label;
    () = msg![env; label setFrame:bounds];
}
- (())dealloc {
    let title = std::mem::replace(&mut env.objc.borrow_mut::<SegmentHostObject>(this).title, nil);
    release(env, title);
    msg_super![env; this dealloc]
}

@end

};

#[test]
fn segmented_touch_respects_bounds_and_segment_boundaries() {
    let bounds = CGRect {
        origin: CGPoint { x: 5.0, y: 2.0 },
        size: CGSize {
            width: 100.0,
            height: 20.0,
        },
    };
    assert_eq!(segment_at(CGPoint { x: 54.0, y: 3.0 }, bounds, 2), Some(0));
    assert_eq!(segment_at(CGPoint { x: 55.0, y: 3.0 }, bounds, 2), Some(1));
    assert_eq!(segment_at(CGPoint { x: 105.0, y: 3.0 }, bounds, 2), None);
    assert_eq!(segment_at(CGPoint { x: 55.0, y: -1.0 }, bounds, 2), None);
    assert_eq!(segment_at(CGPoint { x: 55.0, y: 3.0 }, bounds, 0), None);
}

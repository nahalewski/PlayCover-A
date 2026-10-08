/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIToolbar`.
//!
//! The toolbar keeps its items, bar style, tint and translucency so apps (and
//! nibs) can set and read them back, but it does not draw the items yet.

use crate::frameworks::core_graphics::{CGRect, CGSize};
use crate::frameworks::foundation::{ns_array, ns_string::get_static_str, NSInteger, NSUInteger};
use crate::objc::{
    autorelease, id, impl_HostObject_with_superclass, msg, msg_super, nil, objc_classes, release,
    retain, ClassExports, NSZonePtr,
};

#[derive(Default)]
struct UIToolbarHostObject {
    superclass: super::UIViewHostObject,
    items: Vec<id>,
    tint: id,
    style: NSInteger,
    translucent: bool,
}
impl_HostObject_with_superclass!(UIToolbarHostObject);

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIToolbar: UIView

+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::<UIToolbarHostObject>::default(), &mut env.mem)
}

- (id)initWithFrame:(CGRect)frame {
    msg_super![env; this initWithFrame:frame]
}

- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];
    let key = get_static_str(env, "UIItems");
    let present: bool = msg![env; coder containsValueForKey:key];
    if present {
        let items: id = msg![env; coder decodeObjectForKey:key];
        () = msg![env; this setItems:items animated:false];
    }
    let key = get_static_str(env, "UIBarStyle");
    let present: bool = msg![env; coder containsValueForKey:key];
    if present {
        let style: NSInteger = msg![env; coder decodeIntegerForKey:key];
        () = msg![env; this setBarStyle:style];
    }
    let key = get_static_str(env, "UITintColor");
    let present: bool = msg![env; coder containsValueForKey:key];
    if present {
        let tint: id = msg![env; coder decodeObjectForKey:key];
        () = msg![env; this setTintColor:tint];
    }
    let key = get_static_str(env, "UITranslucent");
    let present: bool = msg![env; coder containsValueForKey:key];
    if present {
        let translucent: bool = msg![env; coder decodeBoolForKey:key];
        () = msg![env; this setTranslucent:translucent];
    }
    this
}

- (())dealloc {
    let state = env.objc.borrow_mut::<UIToolbarHostObject>(this);
    let items = std::mem::take(&mut state.items);
    let tint = state.tint;
    for item in items {
        release(env, item);
    }
    release(env, tint);
    msg_super![env; this dealloc]
}

- (id)items {
    let items = env.objc.borrow::<UIToolbarHostObject>(this).items.clone();
    for item in &items {
        retain(env, *item);
    }
    let array = ns_array::from_vec(env, items);
    autorelease(env, array)
}
- (())setItems:(id)items {
    () = msg![env; this setItems:items animated:false];
}
- (())setItems:(id)items animated:(bool)_animated {
    let mut replacement = Vec::new();
    if items != nil {
        let count: NSUInteger = msg![env; items count];
        for index in 0..count {
            let item: id = msg![env; items objectAtIndex:index];
            if item != nil {
                retain(env, item);
                replacement.push(item);
            }
        }
    }
    let old = std::mem::replace(
        &mut env.objc.borrow_mut::<UIToolbarHostObject>(this).items,
        replacement,
    );
    for item in old {
        release(env, item);
    }
}

- (NSInteger)barStyle {
    env.objc.borrow::<UIToolbarHostObject>(this).style
}
- (())setBarStyle:(NSInteger)style {
    env.objc.borrow_mut::<UIToolbarHostObject>(this).style = style;
}
- (id)tintColor {
    env.objc.borrow::<UIToolbarHostObject>(this).tint
}
- (())setTintColor:(id)tint {
    retain(env, tint);
    let old = std::mem::replace(&mut env.objc.borrow_mut::<UIToolbarHostObject>(this).tint, tint);
    release(env, old);
}
- (bool)isTranslucent {
    env.objc.borrow::<UIToolbarHostObject>(this).translucent
}
- (())setTranslucent:(bool)translucent {
    env.objc.borrow_mut::<UIToolbarHostObject>(this).translucent = translucent;
}

// A standard toolbar is 44 points tall and as wide as it is asked to be.
- (CGSize)sizeThatFits:(CGSize)size {
    CGSize { width: size.width, height: 44.0 }
}
- (())sizeToFit {
    let frame: CGRect = msg![env; this frame];
    let mut frame = frame;
    frame.size.height = 44.0;
    () = msg![env; this setFrame:frame];
}

@end

};

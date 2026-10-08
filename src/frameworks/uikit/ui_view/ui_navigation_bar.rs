/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! UINavigationBar with retained item state and synchronous title layout.
use crate::frameworks::core_graphics::{CGPoint, CGRect, CGSize};
use crate::frameworks::foundation::{ns_array, ns_string::get_static_str, NSInteger, NSUInteger};
use crate::frameworks::uikit::ui_font::UITextAlignmentCenter;
use crate::objc::{
    autorelease, id, impl_HostObject_with_superclass, msg, msg_class, msg_super, nil, objc_classes,
    release, retain, ClassExports, NSZonePtr,
};
use crate::Environment;

#[derive(Default)]
struct UINavigationBarHostObject {
    superclass: super::UIViewHostObject,
    items: Vec<id>,
    delegate: id, // UIKit delegates are nonretaining.
    tint: id,
    style: NSInteger,
    translucent: bool,
    label: id,
    visible_title: id,    // owned by the UIView subview list
    accessories: Vec<id>, // owned by the UIView subview list
}
impl_HostObject_with_superclass!(UINavigationBarHostObject);

fn appearance(env: &mut Environment, this: id) {
    let state = env.objc.borrow::<UINavigationBarHostObject>(this);
    let (style, translucent, tint, label) =
        (state.style, state.translucent, state.tint, state.label);
    let color: id = if tint != nil {
        tint
    } else if style == 1 {
        msg_class![env; UIColor blackColor]
    } else {
        msg_class![env; UIColor grayColor]
    };
    let color: id =
        msg![env; color colorWithAlphaComponent:(if translucent {0.8f32} else {1.0f32})];
    () = msg![env; this setBackgroundColor:color];
    let text: id = if style == 1 {
        msg_class![env; UIColor whiteColor]
    } else {
        msg_class![env; UIColor blackColor]
    };
    () = msg![env; label setTextColor:text];
    () = msg![env; this setNeedsDisplay];
}

fn initialize(env: &mut Environment, this: id) {
    if env.objc.borrow::<UINavigationBarHostObject>(this).label != nil {
        return;
    }
    let label: id = msg_class![env; UILabel new];
    let clear: id = msg_class![env; UIColor clearColor];
    let font: id = msg_class![env; UIFont boldSystemFontOfSize:20.0f32];
    () = msg![env; label setBackgroundColor:clear];
    () = msg![env; label setFont:font];
    () = msg![env; label setTextAlignment:UITextAlignmentCenter];
    env.objc.borrow_mut::<UINavigationBarHostObject>(this).label = label;
    appearance(env, this);
}

fn responds(env: &mut Environment, delegate: id, selector: &str) -> bool {
    let sel = env
        .objc
        .register_host_selector(selector.to_owned(), &mut env.mem);
    msg![env; delegate respondsToSelector:sel]
}

pub const CLASSES: ClassExports = objc_classes! {
(env, this, _cmd);
@implementation UINavigationBar: UIView
+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::<UINavigationBarHostObject>::default(), &mut env.mem)
}
- (id)initWithFrame:(CGRect)frame {
    let this: id = msg_super![env; this initWithFrame:frame];
    initialize(env, this);
    this
}
- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];
    initialize(env, this);
    let items: id = msg![env; coder decodeObjectForKey:(get_static_str(env, "UIItems"))];
    let tint: id = msg![env; coder decodeObjectForKey:(get_static_str(env, "UITintColor"))];
    let style: NSInteger = msg![env; coder decodeIntegerForKey:(get_static_str(env, "UIBarStyle"))];
    // The archived weak delegate can point back to the controller currently
    // decoding this bar. NSKeyedUnarchiver does not cache in-progress objects;
    // the owning controller reconnects this nonretaining relationship.
    () = msg![env; this setTintColor:tint];
    () = msg![env; this setBarStyle:style];
    let key = get_static_str(env, "UITranslucent");
    let present: bool = msg![env; coder containsValueForKey:key];
    if present {
        let translucent: bool = msg![env; coder decodeBoolForKey:key];
        () = msg![env; this setTranslucent:translucent];
    }
    () = msg![env; this setItems:items animated:false];
    this
}
- (())dealloc {
    let state = env.objc.borrow_mut::<UINavigationBarHostObject>(this);
    let items = std::mem::take(&mut state.items);
    let (label, tint) = (state.label, state.tint);
    for item in items {
        crate::frameworks::uikit::ui_navigation_item::set_navigation_bar(env, item, nil);
        release(env, item);
    }
    release(env, label);
    release(env, tint);
    msg_super![env; this dealloc]
}
- (id)delegate { env.objc.borrow::<UINavigationBarHostObject>(this).delegate }
- (())setDelegate:(id)delegate { env.objc.borrow_mut::<UINavigationBarHostObject>(this).delegate = delegate; }
- (id)topItem { env.objc.borrow::<UINavigationBarHostObject>(this).items.last().copied().unwrap_or(nil) }
- (id)backItem {
    let items = &env.objc.borrow::<UINavigationBarHostObject>(this).items;
    items.len().checked_sub(2).map(|index| items[index]).unwrap_or(nil)
}
- (id)items {
    let items = env.objc.borrow::<UINavigationBarHostObject>(this).items.clone();
    for item in &items { retain(env, *item); }
    let array = ns_array::from_vec(env, items);
    autorelease(env, array)
}
- (())setItems:(id)items { () = msg![env; this setItems:items animated:false]; }
- (())setItems:(id)items animated:(bool)animated {
    if animated { log_dbg!("UINavigationBar item replacement is synchronous"); }
    let count: NSUInteger = msg![env; items count];
    let mut replacement = Vec::new();
    for index in 0..count {
        let item: id = msg![env; items objectAtIndex:index];
        assert!(item != nil && !replacement.contains(&item));
        retain(env, item);
        replacement.push(item);
    }
    let old = std::mem::replace(&mut env.objc.borrow_mut::<UINavigationBarHostObject>(this).items, replacement);
    for item in old {
        crate::frameworks::uikit::ui_navigation_item::set_navigation_bar(env, item, nil);
        release(env, item);
    }
    let current = env.objc.borrow::<UINavigationBarHostObject>(this).items.clone();
    for item in current { crate::frameworks::uikit::ui_navigation_item::set_navigation_bar(env, item, this); }
    () = msg![env; this layoutSubviews];
}
- (())pushNavigationItem:(id)item animated:(bool)animated {
    assert!(item != nil && !env.objc.borrow::<UINavigationBarHostObject>(this).items.contains(&item));
    let delegate = env.objc.borrow::<UINavigationBarHostObject>(this).delegate;
    if responds(env, delegate, "navigationBar:shouldPushItem:") {
        let allowed: bool = msg![env; delegate navigationBar:this shouldPushItem:item];
        if !allowed { return; }
    }
    if animated { log_dbg!("UINavigationBar push is synchronous"); }
    retain(env, item);
    env.objc.borrow_mut::<UINavigationBarHostObject>(this).items.push(item);
    crate::frameworks::uikit::ui_navigation_item::set_navigation_bar(env, item, this);
    () = msg![env; this layoutSubviews];
    if responds(env, delegate, "navigationBar:didPushItem:") {
        () = msg![env; delegate navigationBar:this didPushItem:item];
    }
}
- (id)popNavigationItemAnimated:(bool)animated {
    let item: id = msg![env; this topItem];
    if item == nil { return nil; }
    let delegate = env.objc.borrow::<UINavigationBarHostObject>(this).delegate;
    if responds(env, delegate, "navigationBar:shouldPopItem:") {
        let allowed: bool = msg![env; delegate navigationBar:this shouldPopItem:item];
        if !allowed { return nil; }
    }
    if animated { log_dbg!("UINavigationBar pop is synchronous"); }
    env.objc.borrow_mut::<UINavigationBarHostObject>(this).items.pop();
    crate::frameworks::uikit::ui_navigation_item::set_navigation_bar(env, item, nil);
    () = msg![env; this layoutSubviews];
    if responds(env, delegate, "navigationBar:didPopItem:") {
        () = msg![env; delegate navigationBar:this didPopItem:item];
    }
    autorelease(env, item)
}
- (NSInteger)barStyle { env.objc.borrow::<UINavigationBarHostObject>(this).style }
- (())setBarStyle:(NSInteger)style {
    assert!(style == 0 || style == 1);
    env.objc.borrow_mut::<UINavigationBarHostObject>(this).style = style;
    appearance(env, this);
}
- (id)tintColor { env.objc.borrow::<UINavigationBarHostObject>(this).tint }
- (())setTintColor:(id)tint {
    retain(env, tint);
    let old = std::mem::replace(&mut env.objc.borrow_mut::<UINavigationBarHostObject>(this).tint, tint);
    release(env, old);
    appearance(env, this);
}
- (bool)isTranslucent { env.objc.borrow::<UINavigationBarHostObject>(this).translucent }
- (())setTranslucent:(bool)translucent {
    env.objc.borrow_mut::<UINavigationBarHostObject>(this).translucent = translucent;
    appearance(env, this);
}
- (CGSize)sizeThatFits:(CGSize)size { CGSize {width: size.width, height: 44.0} }
- (())setFrame:(CGRect)frame {
    () = msg_super![env; this setFrame:frame];
    () = msg![env; this layoutSubviews];
}
- (())layoutSubviews {
    let label = env.objc.borrow::<UINavigationBarHostObject>(this).label;
    if label == nil { return; }
    let top: id = msg![env; this topItem];
    let custom: id = msg![env; top titleView];
    let title: id = msg![env; top title];
    () = msg![env; label setText:title];
    let visible = if custom != nil { custom } else if top != nil {label} else {nil};
    let old = env.objc.borrow::<UINavigationBarHostObject>(this).visible_title;
    if old != visible {
        () = msg![env; old removeFromSuperview];
        if visible != nil { () = msg![env; this addSubview:visible]; }
        env.objc.borrow_mut::<UINavigationBarHostObject>(this).visible_title = visible;
    }
    let bounds: CGRect = msg![env; this bounds];
    let old_accessories = std::mem::take(&mut env.objc.borrow_mut::<UINavigationBarHostObject>(this).accessories);
    for view in old_accessories { () = msg![env; view removeFromSuperview]; }
    let mut left = Vec::new();
    let mut right = Vec::new();
    let left_items: id = msg![env; top leftBarButtonItems];
    let right_items: id = msg![env; top rightBarButtonItems];
    for (array, views) in [(left_items, &mut left), (right_items, &mut right)] {
        let count: NSUInteger = msg![env; array count];
        for index in 0..count {
            let item: id = msg![env; array objectAtIndex:index];
            let view = crate::frameworks::uikit::ui_navigation_item::button_view(env, item);
            if view != nil { views.push(view); }
        }
    }
    let hides_back: bool = msg![env; top hidesBackButton];
    let previous: id = msg![env; this backItem];
    if top != nil && previous != nil && !hides_back && left.is_empty() {
        let back_item: id = msg![env; previous backBarButtonItem];
        let back_title: id = if back_item != nil { msg![env; back_item title] } else {msg![env; previous title]};
        let back_title = if back_title == nil {get_static_str(env, "Back")} else {back_title};
        let button: id = msg_class![env; UIButton buttonWithType:1i32];
        () = msg![env; button setTitle:back_title forState:0u32];
        let action = env.objc.register_host_selector("_touchHLEPopNavigationItem:".to_owned(), &mut env.mem);
        () = msg![env; button addTarget:this action:action forControlEvents:64u32];
        left.push(button);
    }
    // Keep accessory groups inside bounds and reserve their occupied space
    // for the centered title. Very narrow bars clip rather than overlap.
    let available = (bounds.size.width - 16.0).max(0.0);
    let button_width = if left.len() + right.len() == 0 {0.0} else {
        72.0f32.min(available / (left.len() + right.len()) as f32)
    };
    let title_margin = 8.0 + button_width * left.len().max(right.len()) as f32;
    let mut accessories = Vec::new();
    for (views, is_right) in [(left, false), (right, true)] {
        for (index, view) in views.into_iter().enumerate() {
            let x = if is_right {bounds.origin.x + bounds.size.width - 8.0 - button_width * (index + 1) as f32}
                else {bounds.origin.x + 8.0 + button_width * index as f32};
            let frame = CGRect {origin: CGPoint {x, y: bounds.origin.y},
                size: CGSize {width: button_width, height: bounds.size.height}};
            () = msg![env; view setFrame:frame];
            // Accessory controls may have been created during this layout
            // pass, after UIApplication's view snapshot. Size their label and
            // image subviews now instead of relying on that startup snapshot.
            () = msg![env; view layoutSubviews];
            () = msg![env; this addSubview:view];
            accessories.push(view);
        }
    }
    env.objc.borrow_mut::<UINavigationBarHostObject>(this).accessories = accessories;
    let frame = CGRect {origin: CGPoint {x: bounds.origin.x + title_margin, y: bounds.origin.y},
        size: CGSize {width: (bounds.size.width - title_margin * 2.0).max(0.0), height: bounds.size.height}};
    () = msg![env; visible setFrame:frame];
}
- (())_touchHLEPopNavigationItem:(id)_sender {
    let delegate = env.objc.borrow::<UINavigationBarHostObject>(this).delegate;
    if responds(env, delegate, "popViewControllerAnimated:") {
        let _: id = msg![env; delegate popViewControllerAnimated:true];
    } else {
        let _: id = msg![env; this popNavigationItemAnimated:true];
    }
}
@end
};

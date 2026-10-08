/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UINavigationController`.

use crate::frameworks::core_graphics::CGRect;
use crate::frameworks::foundation::ns_string::get_static_str;
use crate::frameworks::foundation::{ns_array, NSUInteger};
use crate::objc::{
    autorelease, id, impl_HostObject_with_superclass, msg, msg_class, msg_super, nil, objc_classes,
    release, retain, ClassExports, NSZonePtr,
};
use crate::Environment;

// TODO: toolbar
// TODO: animations

#[derive(Default)]
struct UINavigationControllerHostObject {
    superclass: super::UIViewControllerHostObject,
    /// something implementing UINavigationControllerDelegate
    delegate: id,
    /// Navigation stack of view controllers, non-retaining
    /// (we explicitly retain/release on push/pop messages)
    navigation_stack: Vec<id>,
    navigation_bar: id,
    bar_hidden: bool,
}
impl_HostObject_with_superclass!(UINavigationControllerHostObject);

fn synchronize_bar(env: &mut Environment, this: id) {
    let bar: id = msg![env; this navigationBar];
    let state = env.objc.borrow::<UINavigationControllerHostObject>(this);
    let (stack, hidden) = (state.navigation_stack.clone(), state.bar_hidden);
    let mut items = Vec::new();
    for controller in stack {
        let item: id = msg![env; controller navigationItem];
        retain(env, item);
        items.push(item);
    }
    let items = ns_array::from_vec(env, items);
    () = msg![env; bar setItems:items animated:false];
    () = msg![env; bar setHidden:hidden];
    release(env, items);
}

fn show_top(env: &mut Environment, this: id, previous: id, top: id, animated: bool) {
    let root = env
        .objc
        .borrow::<super::UIViewControllerHostObject>(this)
        .view;
    if root == nil || previous == top {
        return;
    }
    let delegate = env
        .objc
        .borrow::<UINavigationControllerHostObject>(this)
        .delegate;
    let top_view: id = if top == nil { nil } else { msg![env; top view] };
    let will = env.objc.register_host_selector(
        "navigationController:willShowViewController:animated:".into(),
        &mut env.mem,
    );
    let responds: bool = msg![env; delegate respondsToSelector:will];
    if top != nil && responds {
        () = msg![env; delegate navigationController:this willShowViewController:top animated:animated];
    }
    () = msg![env; previous viewWillDisappear:animated];
    () = msg![env; top viewWillAppear:animated];
    let previous_view = if previous == nil {
        nil
    } else {
        env.objc
            .borrow::<super::UIViewControllerHostObject>(previous)
            .view
    };
    () = msg![env; previous_view removeFromSuperview];
    let bar: id = msg![env; this navigationBar];
    let mut frame: CGRect = msg![env; root bounds];
    let mut bar_frame = frame;
    bar_frame.size.height = 44.0;
    () = msg![env; bar setFrame:bar_frame];
    if !env
        .objc
        .borrow::<UINavigationControllerHostObject>(this)
        .bar_hidden
    {
        frame.origin.y += 44.0;
        frame.size.height = (frame.size.height - 44.0).max(0.0);
    }
    if top != nil {
        () = msg![env; top_view setFrame:frame];
        () = msg![env; root addSubview:top_view];
    }
    () = msg![env; root addSubview:bar];
    () = msg![env; previous viewDidDisappear:animated];
    () = msg![env; top viewDidAppear:animated];
    let did = env.objc.register_host_selector(
        "navigationController:didShowViewController:animated:".into(),
        &mut env.mem,
    );
    let responds: bool = msg![env; delegate respondsToSelector:did];
    if top != nil && responds {
        () = msg![env; delegate navigationController:this didShowViewController:top animated:animated];
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UINavigationController: UIViewController

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<UINavigationControllerHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithRootViewController:(id)root_vc { // UIViewController *
    () = msg![env; this pushViewController:root_vc animated:false];
    this
}

- (id)initWithCoder:(id)coder {
    let _: id = msg_super![env; this initWithCoder:coder];
    let key = get_static_str(env, "UINavigationBar");
    let bar: id = msg![env; coder decodeObjectForKey:key];
    retain(env, bar);
    env.objc.borrow_mut::<UINavigationControllerHostObject>(this).navigation_bar = bar;
    () = msg![env; bar setDelegate:this];
    let key = get_static_str(env, "UIViewControllers");
    let controllers: id = msg![env; coder decodeObjectForKey:key];
    let count: NSUInteger = msg![env; controllers count];
    for index in 0..count {
        let controller: id = msg![env; controllers objectAtIndex:index];
        retain(env, controller);
        env.objc.borrow_mut::<super::UIViewControllerHostObject>(controller).parent_controller = this;
        env.objc.borrow_mut::<UINavigationControllerHostObject>(this).navigation_stack.push(controller);
    }
    this
}

- (())loadView {
    () = msg_super![env; this loadView];
    synchronize_bar(env, this);
    let top: id = msg![env; this topViewController];
    show_top(env, this, nil, top, false);
    if top == nil {
        let root = env.objc.borrow::<super::UIViewControllerHostObject>(this).view;
        let bar: id = msg![env; this navigationBar];
        let mut frame: CGRect = msg![env; root bounds];
        frame.size.height = 44.0;
        () = msg![env; bar setFrame:frame];
        () = msg![env; root addSubview:bar];
    }
}

- (())dealloc {
    let stack = std::mem::take(&mut env.objc.borrow_mut::<UINavigationControllerHostObject>(this).navigation_stack);
    for controller in stack {
        env.objc.borrow_mut::<super::UIViewControllerHostObject>(controller).parent_controller = nil;
        release(env, controller);
    }
    let bar = env.objc.borrow::<UINavigationControllerHostObject>(this).navigation_bar;
    release(env, bar);
    () = msg_super![env; this dealloc];
}

// weak/non-retaining
- (())setDelegate:(id)delegate { // something implementing UINavigationControllerDelegate
    log_dbg!("[(UINavigationController*){:?} setDelegate:{:?}]", this, delegate);
    let host_object = env.objc.borrow_mut::<UINavigationControllerHostObject>(this);
    host_object.delegate = delegate;
}
- (id)delegate {
    env.objc.borrow::<UINavigationControllerHostObject>(this).delegate
}

- (())pushViewController:(id)view_controller animated:(bool)animated {
    assert!(view_controller != nil);
    assert!(!env.objc.borrow::<UINavigationControllerHostObject>(this).navigation_stack.contains(&view_controller));
    let parent = env.objc.borrow::<super::UIViewControllerHostObject>(view_controller).parent_controller;
    assert!(parent == nil || parent == this);
    let previous: id = msg![env; this topViewController];
    retain(env, view_controller);
    env.objc.borrow_mut::<UINavigationControllerHostObject>(this).navigation_stack.push(view_controller);
    env.objc.borrow_mut::<super::UIViewControllerHostObject>(view_controller).parent_controller = this;
    synchronize_bar(env, this);
    show_top(env, this, previous, view_controller, animated);
}

- (id)topViewController {
    if let Some(top_vc) = env.objc.borrow::<UINavigationControllerHostObject>(this).navigation_stack.last() {
        *top_vc
    } else {
        nil
    }
}

- (id)visibleViewController { msg![env; this topViewController] }
- (id)popViewControllerAnimated:(bool)animated {
    if env.objc.borrow::<UINavigationControllerHostObject>(this).navigation_stack.len() < 2 { return nil; }
    let removed = env.objc.borrow_mut::<UINavigationControllerHostObject>(this).navigation_stack.pop().unwrap();
    let top: id = msg![env; this topViewController];
    synchronize_bar(env, this);
    show_top(env, this, removed, top, animated);
    env.objc.borrow_mut::<super::UIViewControllerHostObject>(removed).parent_controller = nil;
    autorelease(env, removed)
}

- (bool)navigationBar:(id)_bar shouldPopItem:(id)item {
    let top: id = msg![env; this topViewController];
    let current: id = msg![env; top navigationItem];
    if item == current {
        let _removed: id = msg![env; this popViewControllerAnimated:true];
        // Controller pop synchronized the complete bar stack already.
        false
    } else { true }
}

- (id)viewControllers {
    let vcs = env.objc.borrow::<UINavigationControllerHostObject>(this).navigation_stack.to_vec();
    for vc in &vcs {
        retain(env, *vc);
    }
    let res = ns_array::from_vec(env, vcs);
    autorelease(env, res)
}
- (())setViewControllers:(id)controllers { // NSArray *
    msg![env; this setViewControllers:controllers animated:false]
}

- (())setViewControllers:(id)controllers animated:(bool)animated {
    let count: NSUInteger = msg![env; controllers count];
    let mut replacement = Vec::new();
    // Validate the complete replacement before changing existing ownership.
    for index in 0..count {
        let controller: id = msg![env; controllers objectAtIndex:index];
        assert!(controller != nil && !replacement.contains(&controller));
        let parent = env.objc.borrow::<super::UIViewControllerHostObject>(controller).parent_controller;
        assert!(parent == nil || parent == this);
        replacement.push(controller);
    }
    for controller in &replacement { retain(env, *controller); }
    let previous: id = msg![env; this topViewController];
    retain(env, previous);
    let top = replacement.last().copied().unwrap_or(nil);
    let old = std::mem::replace(&mut env.objc.borrow_mut::<UINavigationControllerHostObject>(this).navigation_stack, replacement);
    let current = env.objc.borrow::<UINavigationControllerHostObject>(this).navigation_stack.clone();
    for controller in &current {
        env.objc.borrow_mut::<super::UIViewControllerHostObject>(*controller).parent_controller = this;
    }
    synchronize_bar(env, this);
    show_top(env, this, previous, top, animated);
    release(env, previous);
    for controller in old {
        if !current.contains(&controller) {
            let view = env.objc.borrow::<super::UIViewControllerHostObject>(controller).view;
            () = msg![env; view removeFromSuperview];
            env.objc.borrow_mut::<super::UIViewControllerHostObject>(controller).parent_controller = nil;
        }
        release(env, controller);
    }
}

- (id)navigationBar {
    let bar = env.objc.borrow::<UINavigationControllerHostObject>(this).navigation_bar;
    if bar != nil { return bar; }
    let bar: id = msg_class![env; UINavigationBar alloc];
    let bar: id = msg![env; bar initWithFrame:(CGRect::default())];
    env.objc.borrow_mut::<UINavigationControllerHostObject>(this).navigation_bar = bar;
    () = msg![env; bar setDelegate:this];
    bar
}
- (bool)isNavigationBarHidden { env.objc.borrow::<UINavigationControllerHostObject>(this).bar_hidden }
- (())setNavigationBarHidden:(bool)hidden {
    () = msg![env; this setNavigationBarHidden:hidden animated:false];
}
- (())setNavigationBarHidden:(bool)hidden animated:(bool)_animated {
    env.objc.borrow_mut::<UINavigationControllerHostObject>(this).bar_hidden = hidden;
    let bar: id = msg![env; this navigationBar];
    () = msg![env; bar setHidden:hidden];
    // Preserve lazy nib loading: resizing an unloaded controller does not load it.
    let root = env.objc.borrow::<super::UIViewControllerHostObject>(this).view;
    if root != nil {
        let top: id = msg![env; this topViewController];
        let view = if top == nil {nil} else {env.objc.borrow::<super::UIViewControllerHostObject>(top).view};
        let mut frame: CGRect = msg![env; root bounds];
        if !hidden { frame.origin.y += 44.0; frame.size.height = (frame.size.height - 44.0).max(0.0); }
        () = msg![env; view setFrame:frame];
    }
}


@end

};

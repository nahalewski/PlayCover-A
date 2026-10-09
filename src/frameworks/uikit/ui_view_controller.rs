/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIViewController`.
//!
//! Resources:
//! - [View Controller Programming Guide for iOS (Legacy)](https://developer.apple.com/library/archive/documentation/WindowsViews/Conceptual/ViewControllerPGforiOSLegacy/BasicViewControllers/BasicViewControllers.html)

use crate::frameworks::core_graphics::CGRect;
use crate::frameworks::foundation::ns_objc_runtime::NSStringFromClass;
use crate::frameworks::foundation::ns_string::{from_rust_string, get_static_str, to_rust_string};
use crate::frameworks::foundation::NSInteger;
use crate::frameworks::uikit::ui_application::{
    UIInterfaceOrientation, UIInterfaceOrientationPortrait,
};
use crate::frameworks::uikit::ui_view::set_view_controller;
use crate::frameworks::uikit::ui_nib::instantiate_nib_at_path;
use crate::objc::{
    id, msg, msg_class, nil, objc_classes, release, retain, todo_objc_setter, Class, ClassExports,
    HostObject, NSZonePtr,
};
use crate::Environment;

pub mod ui_navigation_controller;

#[derive(Default)]
struct UIViewControllerHostObject {
    /// The root view.
    /// `UIView*`
    view: id,
    /// Nib name to be used at the load
    /// of the root view, may be nil.
    /// `NSString*`
    nib_name: id,
    /// Bundle to be used for load
    /// of the nib by name, may be nil.
    /// `NSBundle*`
    bundle: id,
    navigation_item: id,
    parent_controller: id,
    /// Absolute path of the `.storyboardc` directory this controller was
    /// instantiated from, if any (its view nib lives there).
    storyboard_dir: Option<String>,
    /// Scene instantiated by the app (not the window's initial controller):
    /// its view keeps the layout archived in the nib and is scaled to fit
    /// whatever frame the app gives it (Auto Layout is not solved).
    scale_to_fit: bool,
    /// Size of the view as archived in the storyboard, for `scale_to_fit`.
    native_view_size: Option<(f32, f32)>,
    /// Controller presented modally by this one (retained), and the
    /// controller that presented this one (weak).
    presented_controller: id,
    presenting_controller: id,
}
impl HostObject for UIViewControllerHostObject {}

type UIModalTransitionStyle = NSInteger;

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIViewController: UIResponder

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<UIViewControllerHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

// TODO: this should be a designated initializer
- (id)initWithNibName:(id)nib_name // NSString *
               bundle:(id)bundle { // NSBundle *
    retain(env, nib_name);
    retain(env, bundle);

    log_dbg!("[(UIViewController*){:?} initWithNibName:{:?} bundle:{:?}]", this, nib_name, bundle);

    env.objc.borrow_mut::<UIViewControllerHostObject>(this).nib_name = nib_name;
    env.objc.borrow_mut::<UIViewControllerHostObject>(this).bundle = bundle;

    this
}

- (id)initWithCoder:(id)coder {
    let key_ns_string = get_static_str(env, "UIView");
    let view: id = msg![env; coder decodeObjectForKey:key_ns_string];

    () = msg![env; this setView:view];

    let key = get_static_str(env, "UINavigationItem");
    let item: id = msg![env; coder decodeObjectForKey:key];
    retain(env, item);
    env.objc.borrow_mut::<UIViewControllerHostObject>(this).navigation_item = item;
    // The owning container assigns this weak relationship after decoding its
    // children. Decoding the parent here would recurse through the same NIB UID.
    let key = get_static_str(env, "UINibName");
    let name: id = msg![env; coder decodeObjectForKey:key];
    retain(env, name);
    env.objc.borrow_mut::<UIViewControllerHostObject>(this).nib_name = name;

    this
}

- (())dealloc {
    let &UIViewControllerHostObject { view, nib_name, bundle, navigation_item, .. } = env.objc.borrow(this);

    if view != nil {
        set_view_controller(env, view, nil);
    }
    release(env, view);
    release(env, nib_name);
    release(env, bundle);
    release(env, navigation_item);

    env.objc.dealloc_object(this, &mut env.mem);
}

- (())loadView {
    let storyboard_dir = env.objc.borrow::<UIViewControllerHostObject>(this).storyboard_dir.clone();
    if let Some(dir) = storyboard_dir {
        let nib_name: id = env.objc.borrow::<UIViewControllerHostObject>(this).nib_name;
        if nib_name != nil {
            let path = format!("{}/{}.nib", dir, to_rust_string(env, nib_name));
            log_dbg!("Load {:?} view controller's view from storyboard nib {}", this, path);
            let _ = instantiate_nib_at_path(env, path, this);
            if env.objc.borrow::<UIViewControllerHostObject>(this).view != nil {
                return;
            }
            log!("Storyboard view nib for {:?} did not provide a view", this);
        }
    }

    let bundle: id = env.objc.borrow::<UIViewControllerHostObject>(this).bundle;
    let bundle: id = if bundle == nil {
        msg_class![env; NSBundle mainBundle]
    } else {
        bundle
    };

    let nib_name: id = get_nib_name(env, this, bundle);
    if nib_name != nil {
        // If we do have nib name, try to load it!
        log_dbg!(
            "Load {:?} view controller's view by nib, using name {}", this, to_rust_string(env, nib_name)
        );

        let nib: id = msg_class![env; UINib nibWithNibName:nib_name bundle:bundle];
        release(env, nib_name);

        // The NIB's File's Owner will be substituted by `this`,
        // implicitly loading the view as well
        let _: id = msg![env; nib instantiateWithOwner:this options:nil];

        let view = env.objc.borrow::<UIViewControllerHostObject>(this).view;
        // Having nil view at this point probably mean that
        // out nib's parsing is wrong.
        // Also we assume here the case of a "detached nib file"
        // TODO: support "integrated nib file"
        assert!(view != nil);

        return;
    };

    // As a last resort, use plain UIVIew for the root view
    let class: Class = msg![env; this class];
    log!("Unable to load {:?} {} view controller's view by nib, using plain UIView", this, env.objc.get_class_name(class).to_string());
    let view: id = msg_class![env; UIView alloc];
    // Docs are saying that "an empty UIView" is created,
    // but testing reveals that frame matches the screen one
    // (at least on the simulator)
    let screen: id = msg_class![env; UIScreen mainScreen];
    let mut app_frame: CGRect = msg![env; screen applicationFrame];
    if env.framework_state.uikit.ui_application.landscape_adapted {
        // The window's root views were already turned into landscape views.
        let screen_bounds: CGRect = msg![env; screen bounds];
        app_frame = CGRect {
            origin: crate::frameworks::core_graphics::CGPoint { x: 0.0, y: 0.0 },
            size: crate::frameworks::core_graphics::CGSize {
                width: screen_bounds.size.height,
                height: screen_bounds.size.width,
            },
        };
    }
    let view: id = msg![env; view initWithFrame:app_frame];
    () = msg![env; this setView:view];
}

- (id)storyboard { nil }
- (())setStoryboard:(id)_storyboard {}
- (())setTopLayoutGuide:(id)_guide {}
- (())setBottomLayoutGuide:(id)_guide {}

- (())setView:(id)new_view { // UIView*
    let host_obj = env.objc.borrow_mut::<UIViewControllerHostObject>(this);
    let old_view = std::mem::replace(&mut host_obj.view, new_view);
    if old_view != nil {
        set_view_controller(env, old_view, nil);
    }
    if new_view != nil {
        set_view_controller(env, new_view, this);
    }
    retain(env, new_view);
    release(env, old_view);
}
- (id)view {
    let view = env.objc.borrow_mut::<UIViewControllerHostObject>(this).view;
    if view == nil {
        () = msg![env; this loadView];
        let view = env.objc.borrow_mut::<UIViewControllerHostObject>(this).view;
        // A storyboard scene's view is archived with a placeholder size (e.g.
        // 600x600); apps that read `view.bounds` in `viewDidLoad` expect the
        // size of a full-screen view (the device's screen in its current
        // orientation), so size it like one before `viewDidLoad` runs.
        let from_storyboard = env.objc.borrow::<UIViewControllerHostObject>(this).storyboard_dir.is_some();
        let scale_to_fit = env.objc.borrow::<UIViewControllerHostObject>(this).scale_to_fit;
        if scale_to_fit && view != nil {
            let b: CGRect = msg![env; view bounds];
            env.objc.borrow_mut::<UIViewControllerHostObject>(this).native_view_size =
                Some(({ b.size.width }, { b.size.height }));
        } else if from_storyboard && view != nil {
            use crate::window::DeviceOrientation;
            let (w, h) = env.window().portrait_size();
            let (w, h) = match env.window().current_rotation() {
                DeviceOrientation::LandscapeLeft | DeviceOrientation::LandscapeRight => (h, w),
                _ => (w, h),
            };
            let frame = CGRect {
                origin: crate::frameworks::core_graphics::CGPoint { x: 0.0, y: 0.0 },
                size: crate::frameworks::core_graphics::CGSize { width: w as f32, height: h as f32 },
            };
            () = msg![env; view setFrame:frame];
        }
        () = msg![env; this viewDidLoad];
        view
    } else {
        view
    }
}

// Usually overridden by the application
- (())viewDidLoad {
    log_dbg!("[(UIViewController*){:?} viewDidLoad]", this);
}
- (())viewWillAppear:(bool)animated {
    log_dbg!("[(UIViewController*){:?} viewWillAppear:{}]", this, animated);
}
- (())viewDidAppear:(bool)animated {
    log_dbg!("[(UIViewController*){:?} viewDidAppear:{}]", this, animated);
}
- (())viewWillDisappear:(bool)animated {
    log_dbg!("[(UIViewController*){:?} viewWillDisappear:{}]", this, animated);
}
- (())viewDidDisappear:(bool)animated {
    log_dbg!("[(UIViewController*){:?} viewDidDisappear:{}]", this, animated);
}

- (())setTitle:(id)title { // NSString *
    let item: id = msg![env; this navigationItem];
    () = msg![env; item setTitle:title];
}
- (id)navigationItem {
    let item = env.objc.borrow::<UIViewControllerHostObject>(this).navigation_item;
    if item != nil { return item; }
    let item: id = msg_class![env; UINavigationItem new];
    env.objc.borrow_mut::<UIViewControllerHostObject>(this).navigation_item = item;
    item
}
- (id)title {
    let item: id = msg![env; this navigationItem];
    msg![env; item title]
}
- (id)parentViewController { env.objc.borrow::<UIViewControllerHostObject>(this).parent_controller }
- (id)navigationController {
    let class = env.objc.get_known_class("UINavigationController", &mut env.mem);
    let mut parent = env.objc.borrow::<UIViewControllerHostObject>(this).parent_controller;
    while parent != nil {
        let parent_class: Class = msg![env; parent class];
        if env.objc.class_is_subclass_of(parent_class, class) { return parent; }
        parent = env.objc.borrow::<UIViewControllerHostObject>(parent).parent_controller;
    }
    nil
}
- (())setEditing:(bool)editing {
    todo_objc_setter!(this, editing);
}
- (())setWantsFullScreenLayout:(bool)wants {
    todo_objc_setter!(this, wants);
}
- (())setHidesBottomBarWhenPushed:(bool)hides {
    todo_objc_setter!(this, hides);
}
- (())setModalTransitionStyle:(UIModalTransitionStyle)style {
    todo_objc_setter!(this, style);
}

- (())dismissModalViewControllerAnimated:(bool)animated {
    () = msg![env; this dismissViewControllerAnimated:animated completion:nil];
}
- (id)presentedViewController {
    env.objc.borrow::<UIViewControllerHostObject>(this).presented_controller
}
- (id)presentingViewController {
    env.objc.borrow::<UIViewControllerHostObject>(this).presenting_controller
}

// Modal presentation is shown as the presented view covering the presenter's
// view (no animation).
- (())presentViewController:(id)view_controller
                   animated:(bool)_animated
                 completion:(id)completion { // void (^)(void)
    if view_controller == nil {
        if completion != nil {
            crate::frameworks::foundation::ns_operation_queue::run_block(env, completion);
        }
        return;
    }
    // Alert controllers are shown by the Android alert overlay.
    let alert_class = env.objc.get_known_class("UIAlertController", &mut env.mem);
    let presented_class: Class = msg![env; view_controller class];
    if env.objc.class_is_subclass_of(presented_class, alert_class) {
        () = msg![env; view_controller _touchHLE_showAlert];
        if completion != nil {
            crate::frameworks::foundation::ns_operation_queue::run_block(env, completion);
        }
        return;
    }
    let presenter_view: id = msg![env; this view];
    let presented_view: id = msg![env; view_controller view];
    let bounds: CGRect = msg![env; presenter_view bounds];
    let from_storyboard = env.objc.borrow::<UIViewControllerHostObject>(view_controller).storyboard_dir.is_some();
    let native: CGRect = msg![env; presented_view bounds];
    let (bw, bh) = ({ bounds.size.width }, { bounds.size.height });
    let (nw, nh) = ({ native.size.width }, { native.size.height });
    if from_storyboard && nw > 0.0 && nh > 0.0 && (nw > bw || nh > bh) {
        // Storyboard scenes are laid out with Auto Layout, which is not solved
        // here; a scene bigger than the presenter (e.g. an archived 600x600
        // freeform view on a landscape phone) is scaled down to fit instead.
        use crate::frameworks::core_graphics::cg_affine_transform::CGAffineTransform;
        use crate::frameworks::core_graphics::{CGPoint, CGSize};
        let scale = (bw / nw).min(bh / nh);
        let native_bounds = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize { width: nw, height: nh },
        };
        () = msg![env; presented_view setBounds:native_bounds];
        () = msg![env; presented_view setCenter:(CGPoint { x: bw / 2.0, y: bh / 2.0 })];
        let transform = CGAffineTransform::make_scale(scale, scale);
        () = msg![env; presented_view setTransform:transform];
    } else {
        () = msg![env; presented_view setFrame:bounds];
    }
    () = msg![env; presenter_view addSubview:presented_view];
    retain(env, view_controller);
    env.objc.borrow_mut::<UIViewControllerHostObject>(this).presented_controller = view_controller;
    env.objc.borrow_mut::<UIViewControllerHostObject>(view_controller).presenting_controller = this;
    () = msg![env; view_controller viewWillAppear:false];
    () = msg![env; view_controller viewDidAppear:false];
    if completion != nil {
        crate::frameworks::foundation::ns_operation_queue::run_block(env, completion);
    }
}

- (())dismissViewControllerAnimated:(bool)_animated
                        completion:(id)completion {
    // Dismissing the presented controller itself dismisses it from its
    // presenter.
    let presenting = env.objc.borrow::<UIViewControllerHostObject>(this).presenting_controller;
    let owner = if presenting != nil { presenting } else { this };
    let presented = env.objc.borrow::<UIViewControllerHostObject>(owner).presented_controller;
    if presented != nil {
        () = msg![env; presented viewWillDisappear:false];
        let view: id = msg![env; presented view];
        () = msg![env; view removeFromSuperview];
        () = msg![env; presented viewDidDisappear:false];
        env.objc.borrow_mut::<UIViewControllerHostObject>(presented).presenting_controller = nil;
        env.objc.borrow_mut::<UIViewControllerHostObject>(owner).presented_controller = nil;
        release(env, presented);
    }
    if completion != nil {
        crate::frameworks::foundation::ns_operation_queue::run_block(env, completion);
    }
}

- (())dismissMoviePlayerViewControllerAnimated {
    log!("TODO: [(UIViewController*){:?} dismissMoviePlayerViewControllerAnimated]", this); // TODO
}

- (bool)shouldAutorotateToInterfaceOrientation:(UIInterfaceOrientation)interface_orientation {
    interface_orientation == UIInterfaceOrientationPortrait
}

// UIResponder implementation
// From the Apple UIView docs regarding [UIResponder nextResponder]:
// "UIViewController similarly implements the method
// and returns its view’s superview."
// https://developer.apple.com/documentation/uikit/uiresponder/next?language=objc
- (id)nextResponder {
    let view = msg![env; this view];
    let next_responder = msg![env; view superview];
    log_dbg!("[(UIView*){:?} nextResponder] => {:?}", this, next_responder);
    next_responder
}

@end

};

/// A helper function to resolve suitable NIB name for a `view_controller`
/// in the `bundle`. Returns nil if fails.
///
/// Note: It's a responsibility of a caller to release the returned name
/// if not-nil!
fn get_nib_name(env: &mut Environment, view_controller: id, bundle: id) -> id {
    let provider_nib_name: id = env
        .objc
        .borrow::<UIViewControllerHostObject>(view_controller)
        .nib_name;
    if provider_nib_name != nil {
        // TODO: it's not clear how to handle situation when
        // provided nib name do not exist in the bundle.
        // It probably means that our bundle resource loading
        // is faulty, to check
        assert!(check_nib_exists(env, bundle, provider_nib_name));

        retain(env, provider_nib_name);
        return provider_nib_name;
    };

    let class: Class = msg![env; view_controller class];
    let class_name: id = NSStringFromClass(env, class);
    let class_name_str = to_rust_string(env, class_name);

    if let Some(name) = class_name_str.strip_suffix("Controller") {
        let ns_name: id = from_rust_string(env, name.to_string());
        if check_nib_exists(env, bundle, ns_name) {
            release(env, class_name);
            return ns_name;
        }
    }

    if check_nib_exists(env, bundle, class_name) {
        class_name
    } else {
        release(env, class_name);
        nil
    }
}

/// A helper function to check if `nib_name` NIB actually
/// existing in the `bundle`
fn check_nib_exists(env: &mut Environment, bundle: id, nib_name: id) -> bool {
    let type_: id = get_static_str(env, "nib");
    let res: id = msg![env; bundle pathForResource:nib_name ofType:type_];
    res != nil
}

/// Remember which `.storyboardc` directory `view_controller` came from, so its
/// view can be loaded from the storyboard's nibs.
pub fn set_storyboard_dir(env: &mut Environment, view_controller: id, dir: &str) {
    env.objc
        .borrow_mut::<UIViewControllerHostObject>(view_controller)
        .storyboard_dir = Some(dir.to_string());
}

/// Marks a storyboard scene as scale-to-fit (see `UIViewControllerHostObject`).
pub fn set_scale_to_fit(env: &mut Environment, view_controller: id) {
    env.objc
        .borrow_mut::<UIViewControllerHostObject>(view_controller)
        .scale_to_fit = true;
}

/// If `view_controller` is a scale-to-fit scene, the size its view was
/// archived with.
pub fn scale_to_fit_size(env: &mut Environment, view_controller: id) -> Option<(f32, f32)> {
    let host = env.objc.borrow::<UIViewControllerHostObject>(view_controller);
    if host.scale_to_fit {
        host.native_view_size
    } else {
        None
    }
}

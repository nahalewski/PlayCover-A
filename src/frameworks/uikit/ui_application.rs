/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIApplication` and `UIApplicationMain`.

use super::ui_device::*;
use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant};
use crate::frameworks::foundation::ns_string::{from_rust_string, get_static_str};
use crate::frameworks::foundation::{ns_array, ns_string, NSInteger, NSUInteger};
use crate::mem::MutPtr;
use crate::objc::{
    autorelease, id, msg, msg_class, nil, objc_classes, release, retain, todo_objc_setter,
    ClassExports, HostObject, NSZonePtr,
};
use crate::window::DeviceOrientation;
use crate::Environment;

#[derive(Default)]
pub struct State {
    /// [UIApplication sharedApplication]
    shared_application: Option<id>,
    pub(super) status_bar_hidden: bool,
    /// Set once the window's root views were turned into landscape views
    /// (see `adapt_root_views_to_landscape()`), so that views the app creates
    /// later without a frame get landscape bounds too.
    pub landscape_adapted: bool,
}

struct UIApplicationHostObject {
    delegate: id,
    delegate_is_retained: bool,
}
impl HostObject for UIApplicationHostObject {}

pub type UIInterfaceOrientation = UIDeviceOrientation;
#[allow(unused)]
pub const UIInterfaceOrientationPortrait: UIInterfaceOrientation = UIDeviceOrientationPortrait;
#[allow(unused)]
pub const UIInterfaceOrientationPortraitUpsideDown: UIInterfaceOrientation =
    UIDeviceOrientationPortraitUpsideDown;
// These are intentionally swapped and documented as such (the UI on the device
// rotates in the opposite direction to how the device is rotated).
pub const UIInterfaceOrientationLandscapeLeft: UIInterfaceOrientation =
    UIDeviceOrientationLandscapeRight;
pub const UIInterfaceOrientationLandscapeRight: UIInterfaceOrientation =
    UIDeviceOrientationLandscapeLeft;

type UIRemoteNotificationType = NSUInteger;
type UIStatusBarAnimation = NSInteger;
type UIStatusBarStyle = NSInteger;

/// Old apps that never declare an orientation in Info.plist but call
/// `-[UIApplication setStatusBarOrientation:]` (e.g. Secret of Mana) expect the
/// window to turn with the status bar, so their full-screen views end up
/// landscape-sized (480x320) and their `layoutSubviews` rebuilds the GL
/// renderbuffer. Emulate that for the window's top-level views by giving them
/// landscape bounds plus the rotation transform (the same trick touchHLE uses
/// for view-controller autorotation), and resize their full-screen subviews.
fn adapt_root_views_to_landscape(env: &mut Environment, rotation: DeviceOrientation) {
    use crate::frameworks::core_graphics::cg_affine_transform::CGAffineTransform;
    use crate::frameworks::core_graphics::{CGPoint, CGRect, CGSize};
    let angle = match rotation {
        DeviceOrientation::LandscapeLeft => std::f32::consts::FRAC_PI_2,
        DeviceOrientation::LandscapeRight => -std::f32::consts::FRAC_PI_2,
        _ => return,
    };

    fn resize_children(env: &mut Environment, view: id, old: CGSize, new: CGSize) {
        let subviews: id = msg![env; view subviews];
        let count: NSUInteger = msg![env; subviews count];
        for i in 0..count {
            let child: id = msg![env; subviews objectAtIndex:i];
            let frame: CGRect = msg![env; child frame];
            if frame.size.width == old.width && frame.size.height == old.height {
                let new_frame = CGRect {
                    origin: CGPoint { x: 0.0, y: 0.0 },
                    size: new,
                };
                () = msg![env; child setFrame:new_frame];
                resize_children(env, child, old, new);
                () = msg![env; child layoutSubviews];
            }
        }
    }

    let windows = env.framework_state.uikit.ui_view.ui_window.windows.clone();
    for window in windows {
        let window_frame: CGRect = msg![env; window frame];
        let old = window_frame.size;
        let new = CGSize {
            width: old.height,
            height: old.width,
        };
        let subviews: id = msg![env; window subviews];
        let count: NSUInteger = msg![env; subviews count];
        for i in 0..count {
            let view: id = msg![env; subviews objectAtIndex:i];
            let frame: CGRect = msg![env; view frame];
            if frame.size.width != old.width || frame.size.height != old.height {
                continue;
            }
            log!("Rotating root view {:?} to landscape bounds {:?}", view, new);
            env.framework_state.uikit.ui_application.landscape_adapted = true;
            let bounds = CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: new,
            };
            let center = CGPoint {
                x: old.width / 2.0,
                y: old.height / 2.0,
            };
            let transform = CGAffineTransform::make_rotation(angle);
            () = msg![env; view setTransform:transform];
            () = msg![env; view setBounds:bounds];
            () = msg![env; view setCenter:center];
            resize_children(env, view, old, new);
            () = msg![env; view layoutSubviews];
        }
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIApplication: UIResponder

// This should only be called by UIApplicationMain
+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(UIApplicationHostObject {
        delegate: nil,
        delegate_is_retained: false,
    });
    env.objc.alloc_static_object(this, host_object, &mut env.mem)
}

+ (id)sharedApplication {
    env.framework_state.uikit.ui_application.shared_application.unwrap_or(nil)
}

// This should only be called by UIApplicationMain
- (id)init {
    assert!(env.framework_state.uikit.ui_application.shared_application.is_none());
    env.framework_state.uikit.ui_application.shared_application = Some(this);
    this
}

// This is a singleton, it shouldn't be deallocated.
- (id)retain { this }
- (id)autorelease { this }
- (())release {}

- (id)delegate {
    env.objc.borrow::<UIApplicationHostObject>(this).delegate
}
- (())setDelegate:(id)delegate { // something implementing UIApplicationDelegate
    let host_object = env.objc.borrow_mut::<UIApplicationHostObject>(this);
    // This property is quasi-non-retaining: https://stackoverflow.com/a/14271150/736162
    let old_delegate = std::mem::replace(&mut host_object.delegate, delegate);
    if host_object.delegate_is_retained {
        host_object.delegate_is_retained = false;
        if delegate != old_delegate {
            release(env, old_delegate);
        }
    }
}

- (bool)isStatusBarHidden {
    env.framework_state.uikit.ui_application.status_bar_hidden
}
- (())setStatusBarHidden:(bool)hidden {
    env.framework_state.uikit.ui_application.status_bar_hidden = hidden;
}
- (())setStatusBarHidden:(bool)hidden
                animated:(bool)_animated {
    // TODO: animation
    msg![env; this setStatusBarHidden:hidden]
}
- (())setStatusBarHidden:(bool)hidden
           withAnimation:(UIStatusBarAnimation)_animation {
    // TODO: animation
    msg![env; this setStatusBarHidden:hidden]
}

- (())setStatusBarStyle:(UIStatusBarStyle)style {
    todo_objc_setter!(this, style);
}
- (())setStatusBarStyle:(UIStatusBarStyle)style
               animated:(bool)_animated {
    // TODO: animation
    msg![env; this setStatusBarStyle:style]
}

- (UIInterfaceOrientation)statusBarOrientation {
    match env.window().current_rotation() {
        DeviceOrientation::Portrait => UIDeviceOrientationPortrait,
        DeviceOrientation::PortraitUpsideDown => UIDeviceOrientationPortraitUpsideDown,
        DeviceOrientation::LandscapeLeft => UIDeviceOrientationLandscapeLeft,
        DeviceOrientation::LandscapeRight => UIDeviceOrientationLandscapeRight
    }
}
- (())setStatusBarOrientation:(UIInterfaceOrientation)orientation {
    let prev_orientation = env.window().current_rotation();
    // Zenonia 5 (cocos2d-x) asks for landscape and then, still during launch,
    // for portrait. On iOS that only touched the status bar: the app keeps the
    // landscape launch orientation (applied with
    // `--landscape-left`, see touchHLE_default_options.txt).
    if env.bundle.bundle_identifier() == "com.gamevil.zenonia5free" {
        log!("Ignoring setStatusBarOrientation:{} for this app (keeps launch orientation)", orientation);
        return;
    }
    if let Some(requested) = match orientation {
        UIDeviceOrientationPortrait => Some(DeviceOrientation::Portrait),
        UIDeviceOrientationPortraitUpsideDown => Some(DeviceOrientation::PortraitUpsideDown),
        UIDeviceOrientationLandscapeLeft => Some(DeviceOrientation::LandscapeLeft),
        UIDeviceOrientationLandscapeRight => Some(DeviceOrientation::LandscapeRight),
        _ => None,
    } {
        crate::window::note_app_requested_orientation(requested);
    }
    env.on_parent_stack_in_coroutine(|window, _| {window.rotate_device(match orientation {
        UIDeviceOrientationPortrait => DeviceOrientation::Portrait,
        UIDeviceOrientationPortraitUpsideDown => DeviceOrientation::PortraitUpsideDown,
        UIDeviceOrientationLandscapeLeft => DeviceOrientation::LandscapeLeft,
        UIDeviceOrientationLandscapeRight => DeviceOrientation::LandscapeRight,
        _ => unimplemented!("Orientation {} not handled yet", orientation),
    })});
    let new_orientation = env.window().current_rotation();
    if prev_orientation != new_orientation {
        if env.options.landscape_view_adaptation && !env.bundle.declares_interface_orientation() {
            adapt_root_views_to_landscape(env, new_orientation);
        }
        generate_device_orientation_notification(env);
    }
}
- (())setStatusBarOrientation:(UIInterfaceOrientation)orientation
                     animated:(bool)_animated {
    // TODO: animation
    msg![env; this setStatusBarOrientation:orientation]
}

- (bool)isIdleTimerDisabled {
    !env.window().is_screen_saver_enabled()
}
- (())setIdleTimerDisabled:(bool)disabled {
    env.on_parent_stack_in_coroutine(|window, _| window.set_screen_saver_enabled(!disabled))
}

- (())setNetworkActivityIndicatorVisible:(bool)visible {
    todo_objc_setter!(this, visible);
}

- (bool)openURL:(id)url { // NSURL
    let ns_string = msg![env; url absoluteString];
    let url_string = ns_string::to_rust_string(env, ns_string);
    if let Err(e) = crate::window::open_url(env, &url_string) {
        echo!("App opened URL {:?} unsuccessfully ({}), exiting.", url_string, e);
    } else {
        echo!("App opened URL {:?}, exiting.", url_string);
    }

    // iPhone OS doesn't really do multitasking, so the app expects to close
    // when a URL is opened, e.g. Super Monkey Ball keeps opening the URL every
    // frame! Super Monkey Ball also doesn't check whether opening failed, so
    // it's probably best to always exit.
    exit(env);
    true
}

// TODO: ignore touches
-(())beginIgnoringInteractionEvents {
    log!("TODO: ignoring beginIgnoringInteractionEvents");
}
- (bool)isIgnoringInteractionEvents {
    false
}
-(())endIgnoringInteractionEvents {
    log!("TODO: ignoring endIgnoringInteractionEvents");
}

- (id)keyWindow {
    let Some(key_window) = env
        .framework_state
        .uikit
        .ui_view
        .ui_window
        .key_window else {
        return nil;
    };
    assert!(env
        .framework_state
        .uikit
        .ui_view
        .ui_window
        .windows
        .contains(&key_window));
    key_window
}

- (id)windows {
    let windows: Vec<id> = (*env
        .framework_state
        .uikit
        .ui_view
        .ui_window
        .windows).to_vec();
    for window in &windows {
        retain(env, *window);
    }
    let windows = ns_array::from_vec(env, windows);
    autorelease(env, windows)
}

- (())registerForRemoteNotificationTypes:(UIRemoteNotificationType)types {
    log!("TODO: ignoring registerForRemoteNotificationTypes:{}", types);
}

- (NSInteger)applicationIconBadgeNumber {
    0 // default value
}
- (())setApplicationIconBadgeNumber:(NSInteger)bn {
    log!("TODO: ignoring setApplicationIconBadgeNumber:{}", bn);
}

- (bool)applicationSupportsShakeToEdit {
    true // default value
}
- (())setApplicationSupportsShakeToEdit:(bool)enable {
    log!("TODO: ignoring setApplicationSupportsShakeToEdit:{}", enable);
}

// UIResponder implementation
// From the Apple UIView docs regarding [UIResponder nextResponder]:
// "The shared UIApplication object normally returns nil, but it returns its
//  app delegate if that object is a subclass of UIResponder and hasn’t
//  already been called to handle the event."
- (id)nextResponder {
    let delegate = msg![env; this delegate];
    let app_delegate_class = msg![env; delegate class];
    let ui_responder_class = env.objc.get_known_class("UIResponder", &mut env.mem);
    if env.objc.class_is_subclass_of(app_delegate_class, ui_responder_class) {
        // TODO: Send nil if it's already been called to handle the event
        delegate
    } else {
        nil
    }
}

- (())cancelAllLocalNotifications {
    log!("TODO: [(UIApplication*){:?} cancelAllLocalNotifications", this);
}
- (())scheduleLocalNotification:(id)local_notif { // UILocalNotification *
    log!("TODO: [(UIApplication*){:?} scheduleLocalNotification:{:?}", this, local_notif);
}

@end

};

/// `UIApplicationMain`, the entry point of the application.
///
/// This function should never return.
pub(super) fn UIApplicationMain(
    env: &mut Environment,
    _argc: i32,
    _argv: MutPtr<MutPtr<u8>>,
    principal_class_name: id, // NSString*
    delegate_class_name: id,  // NSString*
) {
    // UIKit creates and drains autorelease pools when handling events.
    // It's not clear what granularity this should happen with, but this
    // granularity has already caught several bugs. :)

    let ui_application = {
        let pool: id = msg_class![env; NSAutoreleasePool new];

        let principal_class = if principal_class_name != nil {
            let name = ns_string::to_rust_string(env, principal_class_name);
            env.objc.get_known_class(&name, &mut env.mem)
        } else {
            env.objc.get_known_class("UIApplication", &mut env.mem)
        };
        let ui_application: id = msg![env; principal_class new];

        let device_family = env.options.device_family;
        if let Some(main_nib_filename) = env.bundle.main_nib_filename(device_family) {
            let ns_main_nib_filename = from_rust_string(env, main_nib_filename.to_string());
            // We need to check first if main nib file exists,
            // as `UINib nibWithNibName:bundle:` will crash on nonexistent
            // nib otherwise
            let type_: id = get_static_str(env, "nib");
            let bundle: id = msg_class![env; NSBundle mainBundle];
            let res: id = msg![env; bundle pathForResource:ns_main_nib_filename ofType:type_];
            if res != nil {
                let nib: id = msg_class![env; UINib nibWithNibName:ns_main_nib_filename bundle:nil];
                release(env, ns_main_nib_filename);
                let _: id = msg![env; nib instantiateWithOwner:ui_application
                                               options:nil];
            } else {
                log!(
                    "Warning: couldn't load main nib file {:?}",
                    env.bundle.main_nib_filename(device_family)
                );
            }
        }

        if env.bundle.status_bar_hidden() {
            let _: () = msg![env; ui_application setStatusBarHidden:true];
        }

        let delegate: id = msg![env; ui_application delegate];
        if delegate != nil {
            // The delegate was created while loading the nib file.
            // Retain it so it doesn't get deallocated when the autorelease pool
            // is drained. (See discussion in `setDelegate:`.)
            env.objc
                .borrow_mut::<UIApplicationHostObject>(ui_application)
                .delegate_is_retained = true;
            retain(env, delegate);
        } else {
            assert!(delegate_class_name != nil);
            if msg![env; delegate_class_name isEqual:principal_class_name] {
                // If same non-nil class name is used for both principal and
                // delegate, it means that app is using itself as a delegate
                let _: () = msg![env; ui_application setDelegate:ui_application];
            } else {
                // We have to construct the delegate.
                let name = ns_string::to_rust_string(env, delegate_class_name);
                let class = env.objc.get_known_class(&name, &mut env.mem);
                let delegate: id = msg![env; class new];
                let _: () = msg![env; ui_application setDelegate:delegate];
                assert!(delegate != nil);
            }
        };
        // Apps with a main storyboard get their window and initial view
        // controller created by UIKit before the app is told it launched.
        if let Some(storyboard_name) = env.bundle.main_storyboard_filename().map(str::to_string) {
            launch_main_storyboard(env, ui_application, storyboard_name);
        }
        // We can't hang on to the delegate, the guest app may change it at any
        // time.

        let _: () = msg![env; pool drain];

        ui_application
    };

    {
        let pool: id = msg_class![env; NSAutoreleasePool new];
        let delegate: id = msg![env; ui_application delegate];
        // iOS 3+ apps usually use application:didFinishLaunchingWithOptions:,
        // and it seems to be prioritized over applicationDidFinishLaunching:.
        if env.objc.object_has_method_named(
            &env.mem,
            delegate,
            "application:didFinishLaunchingWithOptions:",
        ) {
            let empty_dict: id = msg_class![env; NSDictionary dictionary];
            () = msg![env; delegate application:ui_application didFinishLaunchingWithOptions:empty_dict];
        } else if env.objc.object_has_method_named(
            &env.mem,
            delegate,
            "applicationDidFinishLaunching:",
        ) {
            () = msg![env; delegate applicationDidFinishLaunching:ui_application];
        }

        let center: id = msg_class![env; NSNotificationCenter defaultCenter];
        let notif_name = get_static_str(env, UIApplicationDidFinishLaunchingNotification);
        // TODO: launch options in `userInfo` if it'll ever become a concern
        () = msg![env; center postNotificationName:notif_name object:ui_application userInfo:nil];

        let _: () = msg![env; pool drain];
    }

    // Call layoutSubviews on all views in the view hierarchy.
    // See https://medium.com/geekculture/uiview-lifecycle-part-5-faa2d44511c9
    let views = env.framework_state.uikit.ui_view.views.clone();
    // Layout can replace subviews and release views later in this snapshot.
    // Keep each receiver alive until its layout callback has completed.
    for view in &views {
        retain(env, *view);
    }
    for view in views {
        () = msg![env; view layoutSubviews];
        release(env, view);
    }

    // Send applicationDidBecomeActive now that the application is ready to
    // become active.
    {
        let pool: id = msg_class![env; NSAutoreleasePool new];
        let delegate: id = msg![env; ui_application delegate];
        if env
            .objc
            .object_has_method_named(&env.mem, delegate, "applicationDidBecomeActive:")
        {
            () = msg![env; delegate applicationDidBecomeActive:ui_application];
        }

        let center: id = msg_class![env; NSNotificationCenter defaultCenter];
        let notif_name = get_static_str(env, UIApplicationDidBecomeActiveNotification);
        () = msg![env; center postNotificationName:notif_name object:ui_application userInfo:nil];

        // The storyboard's root view controller appears now that the app is active.
        let root_vc_bits = STORYBOARD_ROOT_VC.swap(0, std::sync::atomic::Ordering::Relaxed);
        if root_vc_bits != 0 {
            let root_vc: id = crate::mem::Ptr::from_bits(root_vc_bits);
            () = msg![env; root_vc viewWillAppear:false];
            () = msg![env; root_vc viewDidAppear:false];
        }

        if env
            .framework_state
            .uikit
            .ui_device
            .is_generating_device_orientation_notifications()
        {
            // This is a bit hacky...
            //
            // Some apps (e.g. "Dead Space") setup window and views only after
            // receiving a device orientation change notification.
            // Setup for this is usually done by calling
            // `[UIDevice beginGeneratingDeviceOrientationNotifications]` and
            // registering for UIDeviceOrientationDidChangeNotification
            // notification in `application:didFinishLaunchingWithOptions:`.
            //
            // Here we're helping by seeding a first device orientation change
            // just after the application becomes active.
            generate_device_orientation_notification(env);
        }

        let _: () = msg![env; pool drain];
    }

    // FIXME: There are more messages we should send.

    // TODO: It might be nicer to return from this function (even though it's
    // conceptually noreturn) and set some global flag that changes how the
    // execution works from this point onwards, though the only real advantages
    // would be a prettier backtrace and maybe the quit button not having to
    // panic.
    let run_loop: id = msg_class![env; NSRunLoop mainRunLoop];
    let _: () = msg![env; run_loop run];
}

/// Tell the app it's about to quit and then exit.
pub(super) fn exit(env: &mut Environment) {
    let ui_application: id = msg_class![env; UIApplication sharedApplication];

    let center: id = msg_class![env; NSNotificationCenter defaultCenter];

    {
        let pool: id = msg_class![env; NSAutoreleasePool new];

        // Skip NSUserDefaults code while in the app picker, otherwise we get
        // a strange error when existing touchHLE due to the fake bundle.
        if !env.is_app_picker {
            // Apple's docs (used to) vaguely mention that `synchronize` is
            // invoked on periodic intervals.
            // Second best - and implemented here - is to save before app exits.
            // TODO: call `synchronize` periodically
            let user_defaults: id = msg_class![env; NSUserDefaults standardUserDefaults];
            let _: bool = msg![env; user_defaults synchronize];
        }

        let delegate: id = msg![env; ui_application delegate];
        if env
            .objc
            .object_has_method_named(&env.mem, delegate, "applicationWillResignActive:")
        {
            () = msg![env; delegate applicationWillResignActive:ui_application];
        }

        let notif_name = get_static_str(env, UIApplicationWillResignActiveNotification);
        () = msg![env; center postNotificationName:notif_name object:ui_application userInfo:nil];

        let _: () = msg![env; pool drain];
    };

    {
        let pool: id = msg_class![env; NSAutoreleasePool new];
        let delegate: id = msg![env; ui_application delegate];
        if env
            .objc
            .object_has_method_named(&env.mem, delegate, "applicationWillTerminate:")
        {
            () = msg![env; delegate applicationWillTerminate:ui_application];
        }

        let notif_name = get_static_str(env, UIApplicationWillTerminateNotification);
        () = msg![env; center postNotificationName:notif_name object:ui_application userInfo:nil];

        let _: () = msg![env; pool drain];
    };

    std::process::exit(0);
}

/// App life-cycle notifications
const UIApplicationDidFinishLaunchingNotification: &str =
    "UIApplicationDidFinishLaunchingNotification";
const UIApplicationDidBecomeActiveNotification: &str = "UIApplicationDidBecomeActiveNotification";
const UIApplicationDidEnterBackgroundNotification: &str =
    "UIApplicationDidEnterBackgroundNotification";
const UIApplicationWillEnterForegroundNotification: &str =
    "UIApplicationWillEnterForegroundNotification";
const UIApplicationWillResignActiveNotification: &str = "UIApplicationWillResignActiveNotification";
const UIApplicationWillTerminateNotification: &str = "UIApplicationWillTerminateNotification";
/// Other app notifications
const UIApplicationLaunchOptionsRemoteNotificationKey: &str =
    "UIApplicationLaunchOptionsRemoteNotificationKey";
const UIApplicationDidReceiveMemoryWarningNotification: &str =
    "UIApplicationDidReceiveMemoryWarningNotification";

/// `UIApplicationLaunchOptionsKey` and `NSNotificationName` values.
/// (Both types are strings)
pub const CONSTANTS: ConstantExports = &[
    // Struct constants: the app reads the fields straight from the symbol.
    (
        "_UIEdgeInsetsZero",
        HostConstant::Custom(|env| {
            let zero = env.mem.alloc(16);
            env.mem.bytes_at_mut(zero.cast::<u8>(), 16).fill(0);
            zero.cast().cast_const()
        }),
    ),
    (
        "_UIOffsetZero",
        HostConstant::Custom(|env| {
            let zero = env.mem.alloc(8);
            env.mem.bytes_at_mut(zero.cast::<u8>(), 8).fill(0);
            zero.cast().cast_const()
        }),
    ),
    (
        "_UIBackgroundTaskInvalid",
        HostConstant::Custom(|env| env.mem.alloc_and_write(u32::MAX).cast().cast_const()),
    ),
    (
        "_UIApplicationDidFinishLaunchingNotification",
        HostConstant::NSString(UIApplicationDidFinishLaunchingNotification),
    ),
    (
        "_UIApplicationDidBecomeActiveNotification",
        HostConstant::NSString(UIApplicationDidBecomeActiveNotification),
    ),
    (
        "_UIApplicationDidEnterBackgroundNotification",
        HostConstant::NSString(UIApplicationDidEnterBackgroundNotification),
    ),
    (
        "_UIApplicationWillEnterForegroundNotification",
        HostConstant::NSString(UIApplicationWillEnterForegroundNotification),
    ),
    (
        "_UIApplicationWillResignActiveNotification",
        HostConstant::NSString(UIApplicationWillResignActiveNotification),
    ),
    (
        "_UIApplicationWillTerminateNotification",
        HostConstant::NSString(UIApplicationWillTerminateNotification),
    ),
    (
        "_UIApplicationDidReceiveMemoryWarningNotification",
        HostConstant::NSString(UIApplicationDidReceiveMemoryWarningNotification),
    ),
    (
        "_UIApplicationLaunchOptionsRemoteNotificationKey",
        HostConstant::NSString(UIApplicationLaunchOptionsRemoteNotificationKey),
    ),
];

pub const FUNCTIONS: FunctionExports = &[export_c_func!(UIApplicationMain(_, _, _, _))];

/// Root view controller created from the main storyboard; it is told it appeared
/// once the app is active (like UIKit does after the window is shown).
static STORYBOARD_ROOT_VC: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Creates the window and initial view controller of `UIMainStoryboardFile`.
fn launch_main_storyboard(env: &mut Environment, ui_application: id, name: String) {
    let ns_name = from_rust_string(env, name.clone());
    let bundle: id = msg_class![env; NSBundle mainBundle];
    let storyboard: id = msg_class![env; UIStoryboard storyboardWithName:ns_name bundle:bundle];
    let vc: id = msg![env; storyboard instantiateInitialViewController];
    if vc == nil {
        log!("Warning: couldn't load the initial view controller of storyboard {:?}", name);
        return;
    }
    let screen: id = msg_class![env; UIScreen mainScreen];
    let frame: crate::frameworks::core_graphics::CGRect = msg![env; screen bounds];
    let window: id = msg_class![env; UIWindow alloc];
    let window: id = msg![env; window initWithFrame:frame];
    () = msg![env; window setRootViewController:vc];
    let delegate: id = msg![env; ui_application delegate];
    if delegate != nil
        && env
            .objc
            .object_has_method_named(&env.mem, delegate, "setWindow:")
    {
        () = msg![env; delegate setWindow:window];
    }
    STORYBOARD_ROOT_VC.store(vc.to_bits(), std::sync::atomic::Ordering::Relaxed);
    () = msg![env; window makeKeyAndVisible];
    log!("Launched main storyboard {:?}: window {:?}, root view controller {:?}", name, window, vc);
}

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Emulator-owned UIKit layer for the experimental ARM64 runtime.
//! Design: dev-docs/ARM64_UIKIT_PLAN.md.
//!
//! Genuine UIKit cannot run here (it needs SpringBoard/BackBoard/FrontBoard
//! and CARenderServer). Instead this module publishes one synthetic Mach-O
//! image whose objc2 classes (UIKit, the QuartzCore CALayer, and the
//! MetalANGLE MGLKit stand-in) have owned trampoline IMPs, and keeps all
//! state host-side ([state::Model]). Guest subclasses (the app delegate,
//! Corona's CoronaViewController/CoronaView) chain into these classes through
//! their ordinary superclass pointers.
//!
//! Implemented: per-method index dispatch through ONE bridge service; the
//! UIApplication/UIScreen/UIDevice singletons; UIView geometry, hierarchy
//! and +layerClass backing layers (created by guest code, so guest overrides
//! of +layerClass are honoured); UIWindow; UIViewController view loading;
//! CALayer; MGLContext/MGLKView/MGLKViewController over a [mgl::GlesHost];
//! `UIApplicationMain` as a guest-side event pump driven by a MainWindow.nib
//! launch plan ([app]). Not yet: touches, notifications/NSString constants,
//! NSArray-returning selectors, CADisplayLink objects.
// Nothing outside the desktop tests installs this layer yet: the session
// wiring (image notification, bind routing) is pending agent A's loader work.
#![allow(dead_code)]
#[path = "a64_uikit_app.rs"]
pub(super) mod app;
#[path = "a64_uikit_asm.rs"]
pub(super) mod asm;
#[path = "a64_uikit_image.rs"]
pub(super) mod image;
#[path = "a64_uikit_mgl.rs"]
pub(super) mod mgl;
#[path = "a64_uikit_quartz.rs"]
pub(super) mod quartz;
#[path = "a64_uikit_state.rs"]
pub(super) mod state;
#[cfg(test)]
#[path = "a64_uikit_tests.rs"]
mod tests;

use super::{
    bridge::{GuestBridge, GuestCall, ReturnValues, ServiceFrame, ServiceId},
    A64Cpu,
};
use image::{ClassDef, External, FunctionDef, ImageSpec, Layout, MethodDef, StaticObject};
use state::{ApplicationState, Model, Point, Rect, ViewKind};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    rc::{Rc, Weak},
};

/// The identities the app links against. When this image is active the
/// cached UIKit/UIKitCore (and the bundled MetalANGLE) must not be loaded.
pub(super) const INSTALL_NAME: &str = "/System/Library/Frameworks/UIKit.framework/UIKit";
pub(super) const QUARTZCORE: &str = "/System/Library/Frameworks/QuartzCore.framework/QuartzCore";
pub(super) const METALANGLE: &str = "@rpath/MetalANGLE.framework/MetalANGLE";

/// Runtime entry points UIKit calls back into (genuine cached libobjc or the
/// emulator-owned services). Supplied after the image has been registered.
#[derive(Clone, Copy, Debug)]
pub(super) struct Links {
    pub msg_send: u64,
    pub msg_send_super2: u64,
    pub retain: u64,
    pub release: u64,
    pub alloc_init: u64,
    pub get_class: u64,
}

type Handler = fn(&mut Call<'_, '_>) -> Result<ReturnValues, String>;

pub(super) struct Method {
    selector: &'static str,
    types: &'static str,
    class_method: bool,
    handler: Handler,
}
pub(super) const fn i(selector: &'static str, types: &'static str, handler: Handler) -> Method {
    Method { selector, types, class_method: false, handler }
}
pub(super) const fn c(selector: &'static str, types: &'static str, handler: Handler) -> Method {
    Method { selector, types, class_method: true, handler }
}

pub(super) struct ClassTable {
    name: &'static str,
    provider: &'static str,
    parent: Option<&'static str>,
    methods: &'static [Method],
    /// Owned -dealloc thunk that clears host state before [super dealloc].
    cleanup: bool,
}

/// Dispatcher entries below the method indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Internal {
    Cleanup = 0,
    PumpStart = 1,
    PumpNext = 2,
    AttachLayer = 3,
}
const INTERNAL_COUNT: u16 = 4;

enum Entry {
    Internal(Internal),
    Method { class: &'static str, method: &'static Method },
}

/// Selectors the host or owned code sends; their canonical SELs are read
/// from this image's __objc_selrefs slots after registration.
const SENT_SELECTORS: &[&str] = &[
    "loadView",
    "viewDidLoad",
    "view",
    "layerClass",
    "drawRect:",
    "display",
    "frameStep",
    "update",
    "mglkView:drawInRect:",
    "mglkViewControllerUpdate:",
    "setWindow:",
    "respondsToSelector:",
    "application:willFinishLaunchingWithOptions:",
    "application:didFinishLaunchingWithOptions:",
    "applicationDidBecomeActive:",
    "viewWillAppear:",
    "viewDidAppear:",
];
const GOT: &[&str] = &["objc_msgSend", "objc_alloc_init"];
const SCRATCH_BYTES: u64 = 4096;

pub(super) struct UiKit {
    pub model: Model,
    layout: Option<Layout>,
    links: Option<Links>,
    loading: BTreeSet<u64>,
    layer_pending: BTreeSet<u64>,
    layer_getter: BTreeSet<u64>,
    me: Weak<RefCell<UiKit>>,
    pub mgl: mgl::MglState,
    pub launcher: app::Launcher,
    /// Methods invoked on emulator classes, in first-use order (diagnostics).
    pub first_use: Vec<String>,
}

pub(super) struct Installed {
    pub state: Rc<RefCell<UiKit>>,
    pub layout: Layout,
    pub services: Vec<(String, ServiceId)>,
}

pub(super) struct Call<'a, 'b> {
    kit: &'a mut UiKit,
    frame: &'a mut ServiceFrame<'b>,
    receiver: u64,
    selector: u64,
}

impl Call<'_, '_> {
    fn arg(&self, index: usize) -> Result<u64, String> {
        self.frame.integer(index)
    }
    fn bool_arg(&self, index: usize) -> Result<bool, String> {
        Ok(self.frame.integer(index)? & 0xff != 0)
    }
    fn f64_arg(&self, index: usize) -> Result<f64, String> {
        Ok(f64::from_bits(self.frame.vector(index)?[0]))
    }
    fn f32_arg(&self, index: usize) -> Result<f32, String> {
        Ok(f32::from_bits(self.frame.vector(index)?[0] as u32))
    }
    fn rect_arg(&self, first: usize) -> Result<Rect, String> {
        Ok(Rect::new(
            self.f64_arg(first)?,
            self.f64_arg(first + 1)?,
            self.f64_arg(first + 2)?,
            self.f64_arg(first + 3)?,
        ))
    }
    fn links(&self) -> Result<Links, String> {
        self.kit.links()
    }
    fn layout(&self) -> Result<&Layout, String> {
        self.kit.layout()
    }
    fn class(&self, name: &str) -> Result<u64, String> {
        self.layout()?
            .class(name)
            .ok_or_else(|| format!("UIKit image class {name} absent"))
    }
    fn static_object(&self, class: &str) -> Result<u64, String> {
        self.layout()?
            .static_object(class)
            .ok_or_else(|| format!("UIKit static {class} absent"))
    }
    /// Canonical SEL, as fixed up in this image's own __objc_selrefs slot.
    fn sent_selector(&mut self, name: &str) -> Result<u64, String> {
        sent_selector(self.kit, self.frame, name)
    }
    fn queue(&mut self, entry: u64, integers: Vec<u64>) -> Result<(), String> {
        self.queue_call(GuestCall { entry, integers, ..Default::default() })
    }
    fn queue_call(&mut self, call: GuestCall) -> Result<(), String> {
        self.frame.request_guest_call(call, |result| result.map(|_| ()))
    }
    /// Queue `objc_msgSend(receiver, selector-by-name, args...)`.
    fn send(&mut self, receiver: u64, selector: &str, args: &[u64]) -> Result<(), String> {
        let sel = self.sent_selector(selector)?;
        let entry = self.links()?.msg_send;
        let mut integers = vec![receiver, sel];
        integers.extend_from_slice(args);
        self.queue(entry, integers)
    }
    fn retain(&mut self, object: u64) -> Result<(), String> {
        if object != 0 {
            let entry = self.links()?.retain;
            self.queue(entry, vec![object])?;
        }
        Ok(())
    }
    fn release(&mut self, object: u64) -> Result<(), String> {
        if object != 0 {
            let entry = self.links()?.release;
            self.queue(entry, vec![object])?;
        }
        Ok(())
    }
    /// Run the owned guest routine that sends +layerClass to the view's class
    /// and allocates the backing layer, unless one exists or is in progress.
    fn ensure_backing_layer(&mut self, view: u64) -> Result<bool, String> {
        if self.kit.model.view(view)?.layer != 0 || self.kit.layer_pending.contains(&view) {
            return Ok(false);
        }
        let entry = self
            .layout()?
            .function(quartz::CREATE_LAYER)
            .ok_or("UIKit layer creation routine absent")?;
        self.kit.layer_pending.insert(view);
        self.queue(entry, vec![view])?;
        Ok(true)
    }
}

fn sent_selector(kit: &UiKit, frame: &mut ServiceFrame<'_>, name: &str) -> Result<u64, String> {
    let slot = *kit
        .layout()?
        .selector_refs
        .get(name)
        .ok_or_else(|| format!("UIKit sends unlisted selector {name}"))?;
    let value = u64::from_le_bytes(frame.read(slot, 8)?.try_into().unwrap());
    if value == 0 {
        return Err(format!("UIKit selector reference {name} is unbound"));
    }
    Ok(value)
}

pub(super) fn ret(value: u64) -> Result<ReturnValues, String> {
    Ok(ReturnValues::integer(value))
}
pub(super) fn ret_void() -> Result<ReturnValues, String> {
    ret(0)
}
pub(super) fn ret_bool(value: bool) -> Result<ReturnValues, String> {
    ret(value as u64)
}
pub(super) fn ret_f64s(values: &[f64]) -> Result<ReturnValues, String> {
    let mut result = ReturnValues::integer(0);
    for (index, value) in values.iter().enumerate() {
        result.vectors[index] = [value.to_bits(), 0];
    }
    Ok(result)
}
pub(super) fn ret_f32(value: f32) -> Result<ReturnValues, String> {
    let mut result = ReturnValues::integer(0);
    result.vectors[0] = [value.to_bits() as u64, 0];
    Ok(result)
}
pub(super) fn ret_rect(rect: Rect) -> Result<ReturnValues, String> {
    ret_f64s(&[rect.origin.x, rect.origin.y, rect.size.width, rect.size.height])
}

// ------------------------------------------------------------ shared handlers
pub(super) fn no_op(_: &mut Call) -> Result<ReturnValues, String> {
    ret_void()
}
pub(super) fn returns_nil(_: &mut Call) -> Result<ReturnValues, String> {
    ret(0)
}
pub(super) fn returns_no(_: &mut Call) -> Result<ReturnValues, String> {
    ret_bool(false)
}
pub(super) fn returns_yes(_: &mut Call) -> Result<ReturnValues, String> {
    ret_bool(true)
}

// ---------------------------------------------------------------- UIResponder
const UIRESPONDER: &[Method] = &[
    i("nextResponder", "@16@0:8", returns_nil),
    i("canBecomeFirstResponder", "B16@0:8", returns_no),
    i("becomeFirstResponder", "B16@0:8", returns_no),
    i("resignFirstResponder", "B16@0:8", returns_yes),
    i("isFirstResponder", "B16@0:8", returns_no),
    i("touchesBegan:withEvent:", "v32@0:8@16@24", no_op),
    i("touchesMoved:withEvent:", "v32@0:8@16@24", no_op),
    i("touchesEnded:withEvent:", "v32@0:8@16@24", no_op),
    i("touchesCancelled:withEvent:", "v32@0:8@16@24", no_op),
];

// -------------------------------------------------------------- UIApplication
fn shared_application(call: &mut Call) -> Result<ReturnValues, String> {
    let object = call.static_object("UIApplication")?;
    call.kit.model.application.object = object;
    ret(object)
}
fn application_delegate(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.model.application.delegate)
}
fn set_application_delegate(call: &mut Call) -> Result<ReturnValues, String> {
    // UIApplication.delegate is not retained by UIKit (weak/assign).
    call.kit.model.application.delegate = call.arg(2)?;
    ret_void()
}
fn key_window(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.model.application.key_window)
}
fn application_state(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.model.application.state as u64)
}
fn idle_timer_disabled(call: &mut Call) -> Result<ReturnValues, String> {
    ret_bool(call.kit.model.application.idle_timer_disabled)
}
fn set_idle_timer_disabled(call: &mut Call) -> Result<ReturnValues, String> {
    call.kit.model.application.idle_timer_disabled = call.bool_arg(2)?;
    ret_void()
}
fn status_bar_hidden(call: &mut Call) -> Result<ReturnValues, String> {
    ret_bool(call.kit.model.application.status_bar_hidden)
}
fn set_status_bar_hidden(call: &mut Call) -> Result<ReturnValues, String> {
    call.kit.model.application.status_bar_hidden = call.bool_arg(2)?;
    ret_void()
}
fn status_bar_orientation(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.model.application.status_bar_orientation as u64)
}
fn status_bar_frame(call: &mut Call) -> Result<ReturnValues, String> {
    if call.kit.model.application.status_bar_hidden {
        return ret_rect(Rect::default());
    }
    let width = call.kit.model.screen_bounds().size.width;
    ret_rect(Rect::new(0.0, 0.0, width, 20.0))
}
const UIAPPLICATION: &[Method] = &[
    c("sharedApplication", "@16@0:8", shared_application),
    i("delegate", "@16@0:8", application_delegate),
    i("setDelegate:", "v24@0:8@16", set_application_delegate),
    i("keyWindow", "@16@0:8", key_window),
    i("applicationState", "q16@0:8", application_state),
    i("isIdleTimerDisabled", "B16@0:8", idle_timer_disabled),
    i("setIdleTimerDisabled:", "v20@0:8B16", set_idle_timer_disabled),
    i("isStatusBarHidden", "B16@0:8", status_bar_hidden),
    i("setStatusBarHidden:", "v20@0:8B16", set_status_bar_hidden),
    i("setStatusBarHidden:withAnimation:", "v28@0:8B16q20", set_status_bar_hidden),
    i("statusBarOrientation", "q16@0:8", status_bar_orientation),
    i("statusBarFrame", "{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8", status_bar_frame),
    i("isNetworkActivityIndicatorVisible", "B16@0:8", returns_no),
    i("setNetworkActivityIndicatorVisible:", "v20@0:8B16", no_op),
    i("beginReceivingRemoteControlEvents", "v16@0:8", no_op),
    i("endReceivingRemoteControlEvents", "v16@0:8", no_op),
    i("registerForRemoteNotifications", "v16@0:8", no_op),
    i("isRegisteredForRemoteNotifications", "B16@0:8", returns_no),
];

// ------------------------------------------------------------------- UIScreen
fn main_screen(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.static_object("UIScreen")?)
}
fn screen_bounds(call: &mut Call) -> Result<ReturnValues, String> {
    ret_rect(call.kit.model.screen_bounds())
}
fn screen_native_bounds(call: &mut Call) -> Result<ReturnValues, String> {
    ret_rect(call.kit.model.screen.native_bounds())
}
fn screen_scale(call: &mut Call) -> Result<ReturnValues, String> {
    ret_f64s(&[call.kit.model.screen.scale])
}
fn screen_maximum_fps(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.model.screen.maximum_frames_per_second as u64)
}
const UISCREEN: &[Method] = &[
    c("mainScreen", "@16@0:8", main_screen),
    i("bounds", "{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8", screen_bounds),
    i("applicationFrame", "{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8", screen_bounds),
    i("nativeBounds", "{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8", screen_native_bounds),
    i("scale", "d16@0:8", screen_scale),
    i("nativeScale", "d16@0:8", screen_scale),
    i("maximumFramesPerSecond", "q16@0:8", screen_maximum_fps),
];

// ------------------------------------------------------------------- UIDevice
fn current_device(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.static_object("UIDevice")?)
}
fn device_idiom(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.model.device.idiom as u64)
}
fn device_orientation(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.model.device.orientation as u64)
}
const UIDEVICE: &[Method] = &[
    c("currentDevice", "@16@0:8", current_device),
    i("userInterfaceIdiom", "q16@0:8", device_idiom),
    i("orientation", "q16@0:8", device_orientation),
    i("beginGeneratingDeviceOrientationNotifications", "v16@0:8", no_op),
    i("endGeneratingDeviceOrientationNotifications", "v16@0:8", no_op),
    i("isGeneratingDeviceOrientationNotifications", "B16@0:8", returns_yes),
    i("isMultitaskingSupported", "B16@0:8", returns_yes),
    i("isBatteryMonitoringEnabled", "B16@0:8", returns_no),
    i("setBatteryMonitoringEnabled:", "v20@0:8B16", no_op),
];

// --------------------------------------------------------------------- UIView
fn view_state(call: &mut Call) -> Result<u64, String> {
    let kind = if call.kit.model.has_view(call.receiver) {
        call.kit.model.view(call.receiver)?.kind
    } else {
        ViewKind::View
    };
    call.kit.model.ensure_view(call.receiver, kind)?;
    Ok(call.receiver)
}
/// Shared by every -initWithFrame:/-init override: host state, then the
/// backing layer through the class's own +layerClass.
pub(super) fn init_view(call: &mut Call, kind: ViewKind, frame: Rect) -> Result<u64, String> {
    call.kit.model.init_view(call.receiver, kind, frame)?;
    call.ensure_backing_layer(call.receiver)?;
    Ok(call.receiver)
}
fn view_init_with_frame(call: &mut Call) -> Result<ReturnValues, String> {
    let frame = call.rect_arg(0)?;
    ret(init_view(call, ViewKind::View, frame)?)
}
fn view_init(call: &mut Call) -> Result<ReturnValues, String> {
    ret(init_view(call, ViewKind::View, Rect::default())?)
}
fn view_layer_class(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.class("CALayer")?)
}
/// -layer: lazily create the backing layer (views initialised through paths
/// we do not intercept), then re-dispatch so the result is the new layer.
fn view_layer(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    let layer = call.kit.model.view(view)?.layer;
    if layer != 0 {
        call.kit.layer_getter.remove(&view);
        return ret(layer);
    }
    if !call.kit.layer_getter.insert(view) || call.kit.layer_pending.contains(&view) {
        call.kit.layer_getter.remove(&view);
        return Err("UIView backing layer creation did not complete".into());
    }
    call.ensure_backing_layer(view)?;
    let entry = call.links()?.msg_send;
    let selector = call.selector;
    call.frame.request_tail_dispatch(entry, selector)?;
    ret(0)
}
fn view_frame(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    ret_rect(call.kit.model.view(view)?.frame)
}
fn view_set_frame(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    let frame = call.rect_arg(0)?;
    call.kit.model.set_frame(view, frame)?;
    ret_void()
}
fn view_bounds(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    ret_rect(call.kit.model.view(view)?.bounds)
}
fn view_set_bounds(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    let bounds = call.rect_arg(0)?;
    call.kit.model.set_bounds(view, bounds)?;
    ret_void()
}
fn view_center(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    let center = call.kit.model.view(view)?.frame.center();
    ret_f64s(&[center.x, center.y])
}
fn view_set_center(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    let center = Point { x: call.f64_arg(0)?, y: call.f64_arg(1)? };
    call.kit.model.set_center(view, center)?;
    ret_void()
}
fn view_add_subview(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    let child = call.arg(2)?;
    if child == 0 {
        return ret_void();
    }
    call.kit.model.ensure_view(child, ViewKind::View)?;
    if call.kit.model.add_subview(view, child)? {
        // The superview owns a strong reference to each subview.
        call.retain(child)?;
    }
    ret_void()
}
fn view_remove_from_superview(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    if call.kit.model.remove_from_superview(view)? != 0 {
        call.release(view)?;
    }
    ret_void()
}
fn view_superview(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    ret(call.kit.model.view(view)?.superview)
}
fn view_window(call: &mut Call) -> Result<ReturnValues, String> {
    let mut cursor = view_state(call)?;
    for _ in 0..state::MAX_VIEWS {
        let view = call.kit.model.view(cursor)?;
        if view.kind == ViewKind::Window {
            return ret(cursor);
        }
        if view.superview == 0 {
            return ret(0);
        }
        cursor = view.superview;
    }
    Err("UIKit hierarchy depth limit".into())
}
macro_rules! view_property {
    ($get:ident, $set:ident, $field:ident, bool) => {
        fn $get(call: &mut Call) -> Result<ReturnValues, String> {
            let view = view_state(call)?;
            ret_bool(call.kit.model.view(view)?.$field)
        }
        fn $set(call: &mut Call) -> Result<ReturnValues, String> {
            let view = view_state(call)?;
            let value = call.bool_arg(2)?;
            call.kit.model.with_view(view, |v| v.$field = value)?;
            ret_void()
        }
    };
    ($get:ident, $set:ident, $field:ident, f64) => {
        fn $get(call: &mut Call) -> Result<ReturnValues, String> {
            let view = view_state(call)?;
            ret_f64s(&[call.kit.model.view(view)?.$field])
        }
        fn $set(call: &mut Call) -> Result<ReturnValues, String> {
            let view = view_state(call)?;
            let value = call.f64_arg(0)?;
            if !value.is_finite() {
                return Err(concat!("UIKit ", stringify!($field), " is not finite").into());
            }
            call.kit.model.with_view(view, |v| v.$field = value)?;
            ret_void()
        }
    };
    ($get:ident, $set:ident, $field:ident, i64) => {
        fn $get(call: &mut Call) -> Result<ReturnValues, String> {
            let view = view_state(call)?;
            ret(call.kit.model.view(view)?.$field as u64)
        }
        fn $set(call: &mut Call) -> Result<ReturnValues, String> {
            let view = view_state(call)?;
            let value = call.arg(2)? as i64;
            call.kit.model.with_view(view, |v| v.$field = value)?;
            ret_void()
        }
    };
}
view_property!(view_hidden, view_set_hidden, hidden, bool);
view_property!(view_opaque, view_set_opaque, opaque, bool);
view_property!(view_interaction, view_set_interaction, user_interaction_enabled, bool);
view_property!(view_multitouch, view_set_multitouch, multiple_touch_enabled, bool);
view_property!(view_alpha, view_set_alpha, alpha, f64);
view_property!(view_scale, view_set_scale, content_scale_factor, f64);
view_property!(view_tag, view_set_tag, tag, i64);
fn view_background_color(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    ret(call.kit.model.view(view)?.background_color)
}
fn view_set_background_color(call: &mut Call) -> Result<ReturnValues, String> {
    let view = view_state(call)?;
    let color = call.arg(2)?;
    let old = call.kit.model.with_view(view, |v| std::mem::replace(&mut v.background_color, color))?;
    if old != color {
        call.retain(color)?;
        call.release(old)?;
    }
    ret_void()
}
const UIVIEW: &[Method] = &[
    c("layerClass", "#16@0:8", view_layer_class),
    i("initWithFrame:", "@48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", view_init_with_frame),
    i("init", "@16@0:8", view_init),
    i("layer", "@16@0:8", view_layer),
    i("frame", "{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8", view_frame),
    i("setFrame:", "v48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", view_set_frame),
    i("bounds", "{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8", view_bounds),
    i("setBounds:", "v48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", view_set_bounds),
    i("center", "{CGPoint=dd}16@0:8", view_center),
    i("setCenter:", "v32@0:8{CGPoint=dd}16", view_set_center),
    i("addSubview:", "v24@0:8@16", view_add_subview),
    i("removeFromSuperview", "v16@0:8", view_remove_from_superview),
    i("superview", "@16@0:8", view_superview),
    i("window", "@16@0:8", view_window),
    i("isHidden", "B16@0:8", view_hidden),
    i("setHidden:", "v20@0:8B16", view_set_hidden),
    i("isOpaque", "B16@0:8", view_opaque),
    i("setOpaque:", "v20@0:8B16", view_set_opaque),
    i("isUserInteractionEnabled", "B16@0:8", view_interaction),
    i("setUserInteractionEnabled:", "v20@0:8B16", view_set_interaction),
    i("isMultipleTouchEnabled", "B16@0:8", view_multitouch),
    i("setMultipleTouchEnabled:", "v20@0:8B16", view_set_multitouch),
    i("alpha", "d16@0:8", view_alpha),
    i("setAlpha:", "v24@0:8d16", view_set_alpha),
    i("contentScaleFactor", "d16@0:8", view_scale),
    i("setContentScaleFactor:", "v24@0:8d16", view_set_scale),
    i("tag", "q16@0:8", view_tag),
    i("setTag:", "v24@0:8q16", view_set_tag),
    i("backgroundColor", "@16@0:8", view_background_color),
    i("setBackgroundColor:", "v24@0:8@16", view_set_background_color),
    i("setNeedsLayout", "v16@0:8", no_op),
    i("layoutIfNeeded", "v16@0:8", no_op),
    i("layoutSubviews", "v16@0:8", no_op),
    i("setNeedsDisplay", "v16@0:8", no_op),
    i("drawRect:", "v48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", no_op),
    i("setAutoresizingMask:", "v24@0:8Q16", no_op),
    i("setExclusiveTouch:", "v20@0:8B16", no_op),
    i("didMoveToWindow", "v16@0:8", no_op),
    i("didMoveToSuperview", "v16@0:8", no_op),
    i("willMoveToWindow:", "v24@0:8@16", no_op),
    i("willMoveToSuperview:", "v24@0:8@16", no_op),
];

// ------------------------------------------------------------------- UIWindow
fn window_init_with_frame(call: &mut Call) -> Result<ReturnValues, String> {
    let frame = call.rect_arg(0)?;
    ret(init_view(call, ViewKind::Window, frame)?)
}
fn window_init(call: &mut Call) -> Result<ReturnValues, String> {
    let frame = call.kit.model.screen_bounds();
    ret(init_view(call, ViewKind::Window, frame)?)
}
fn window_state(call: &mut Call) -> Result<u64, String> {
    call.kit.model.ensure_view(call.receiver, ViewKind::Window)?;
    Ok(call.receiver)
}
fn make_key_and_visible(call: &mut Call) -> Result<ReturnValues, String> {
    let window = window_state(call)?;
    call.kit.model.make_key_and_visible(window)?;
    ret_void()
}
fn is_key_window(call: &mut Call) -> Result<ReturnValues, String> {
    ret_bool(call.kit.model.application.key_window == call.receiver)
}
fn root_view_controller(call: &mut Call) -> Result<ReturnValues, String> {
    let window = window_state(call)?;
    ret(call.kit.model.view(window)?.root_view_controller)
}
fn set_root_view_controller(call: &mut Call) -> Result<ReturnValues, String> {
    let window = window_state(call)?;
    let controller = call.arg(2)?;
    let old = call.kit.model.set_root_view_controller(window, controller)?;
    if old != controller {
        call.retain(controller)?;
        call.release(old)?;
    }
    ret_void()
}
fn window_screen(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.static_object("UIScreen")?)
}
const UIWINDOW: &[Method] = &[
    i("initWithFrame:", "@48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", window_init_with_frame),
    i("init", "@16@0:8", window_init),
    i("makeKeyAndVisible", "v16@0:8", make_key_and_visible),
    i("makeKeyWindow", "v16@0:8", make_key_and_visible),
    i("isKeyWindow", "B16@0:8", is_key_window),
    i("rootViewController", "@16@0:8", root_view_controller),
    i("setRootViewController:", "v24@0:8@16", set_root_view_controller),
    i("screen", "@16@0:8", window_screen),
];

// ----------------------------------------------------------- UIViewController
pub(super) fn controller_init(call: &mut Call) -> Result<ReturnValues, String> {
    call.kit.model.ensure_controller(call.receiver)?;
    ret(call.receiver)
}
/// -view: load on first access by sending the receiver's own (possibly
/// guest-overridden) -loadView and -viewDidLoad, then re-dispatch -view.
fn controller_view(call: &mut Call) -> Result<ReturnValues, String> {
    call.kit.model.ensure_controller(call.receiver)?;
    let controller = call.kit.model.controller(call.receiver).unwrap();
    if controller.view_loaded {
        let view = controller.view;
        call.kit.loading.remove(&call.receiver);
        return ret(view);
    }
    let receiver = call.receiver;
    if !call.kit.loading.insert(receiver) {
        // Re-dispatched after -loadView/-viewDidLoad ran and still no view
        // (UIKit would recurse forever here; fail explicitly instead).
        call.kit.loading.remove(&receiver);
        return Err("UIViewController -loadView did not set a view".into());
    }
    call.send(receiver, "loadView", &[])?;
    call.send(receiver, "viewDidLoad", &[])?;
    let entry = call.links()?.msg_send;
    let selector = call.selector;
    call.frame.request_tail_dispatch(entry, selector)?;
    ret(0)
}
fn controller_view_if_loaded(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.model.controller(call.receiver).map_or(0, |c| c.view))
}
fn controller_is_view_loaded(call: &mut Call) -> Result<ReturnValues, String> {
    ret_bool(call.kit.model.controller(call.receiver).is_some_and(|c| c.view_loaded))
}
pub(super) fn controller_set_view(call: &mut Call) -> Result<ReturnValues, String> {
    let view = call.arg(2)?;
    if view != 0 {
        call.kit.model.ensure_view(view, ViewKind::View)?;
    }
    let old = call.kit.model.set_controller_view(call.receiver, view)?;
    if old != view {
        call.retain(view)?;
        call.release(old)?;
    }
    ret_void()
}
/// Default -loadView: a plain UIView filling the screen, owned (+1) by the
/// controller. The allocation is a genuine guest objc_alloc_init.
fn controller_load_view(call: &mut Call) -> Result<ReturnValues, String> {
    let links = call.links()?;
    let class = call.class("UIView")?;
    let frame = call.kit.model.screen_bounds();
    let controller = call.receiver;
    call.kit.model.ensure_controller(controller)?;
    let me = call.kit.me.clone();
    call.frame.request_guest_call(
        GuestCall { entry: links.alloc_init, integers: vec![class], ..Default::default() },
        move |result| {
            let object = result?.integers[0];
            if object == 0 {
                return Err("default -loadView allocation returned nil".into());
            }
            let kit = me.upgrade().ok_or("UIKit state dropped")?;
            let mut kit = kit.try_borrow_mut().map_err(|_| "reentrant UIKit loadView completion")?;
            kit.model.set_frame(object, frame)?;
            let old = kit.model.set_controller_view(controller, object)?;
            if old != 0 && old != object {
                return Err("default -loadView replaced an existing view".into());
            }
            Ok(())
        },
    )?;
    ret_void()
}
fn controller_supported_orientations(_: &mut Call) -> Result<ReturnValues, String> {
    // UIInterfaceOrientationMaskAllButUpsideDown (iPhone default).
    ret(0x1a)
}
const UIVIEWCONTROLLER: &[Method] = &[
    i("init", "@16@0:8", controller_init),
    i("initWithNibName:bundle:", "@32@0:8@16@24", controller_init),
    i("view", "@16@0:8", controller_view),
    i("setView:", "v24@0:8@16", controller_set_view),
    i("viewIfLoaded", "@16@0:8", controller_view_if_loaded),
    i("isViewLoaded", "B16@0:8", controller_is_view_loaded),
    i("loadView", "v16@0:8", controller_load_view),
    i("viewDidLoad", "v16@0:8", no_op),
    i("viewWillAppear:", "v20@0:8B16", no_op),
    i("viewDidAppear:", "v20@0:8B16", no_op),
    i("viewWillDisappear:", "v20@0:8B16", no_op),
    i("viewDidDisappear:", "v20@0:8B16", no_op),
    i("viewWillLayoutSubviews", "v16@0:8", no_op),
    i("viewDidLayoutSubviews", "v16@0:8", no_op),
    i("didReceiveMemoryWarning", "v16@0:8", no_op),
    i("shouldAutorotate", "B16@0:8", returns_yes),
    i("supportedInterfaceOrientations", "Q16@0:8", controller_supported_orientations),
    i("prefersStatusBarHidden", "B16@0:8", returns_yes),
    i("prefersHomeIndicatorAutoHidden", "B16@0:8", returns_yes),
    i("setNeedsStatusBarAppearanceUpdate", "v16@0:8", no_op),
    i("setNeedsUpdateOfHomeIndicatorAutoHidden", "v16@0:8", no_op),
    i("setWantsFullScreenLayout:", "v20@0:8B16", no_op),
    i("wantsFullScreenLayout", "B16@0:8", returns_yes),
    i("nibName", "@16@0:8", returns_nil),
    i("nibBundle", "@16@0:8", returns_nil),
    i("parentViewController", "@16@0:8", returns_nil),
    i("presentedViewController", "@16@0:8", returns_nil),
];

const CLASSES: &[ClassTable] = &[
    ClassTable { name: "UIResponder", provider: INSTALL_NAME, parent: None, methods: UIRESPONDER, cleanup: false },
    ClassTable { name: "UIApplication", provider: INSTALL_NAME, parent: Some("UIResponder"), methods: UIAPPLICATION, cleanup: false },
    ClassTable { name: "UIScreen", provider: INSTALL_NAME, parent: None, methods: UISCREEN, cleanup: false },
    ClassTable { name: "UIDevice", provider: INSTALL_NAME, parent: None, methods: UIDEVICE, cleanup: false },
    ClassTable { name: "UIView", provider: INSTALL_NAME, parent: Some("UIResponder"), methods: UIVIEW, cleanup: true },
    ClassTable { name: "UIWindow", provider: INSTALL_NAME, parent: Some("UIView"), methods: UIWINDOW, cleanup: false },
    ClassTable { name: "UIViewController", provider: INSTALL_NAME, parent: Some("UIResponder"), methods: UIVIEWCONTROLLER, cleanup: true },
    ClassTable { name: "CALayer", provider: QUARTZCORE, parent: None, methods: quartz::CALAYER, cleanup: true },
    ClassTable { name: "MGLLayer", provider: METALANGLE, parent: Some("CALayer"), methods: quartz::MGLLAYER, cleanup: false },
    ClassTable { name: "MGLContext", provider: METALANGLE, parent: None, methods: mgl::MGLCONTEXT, cleanup: true },
    ClassTable { name: "MGLKView", provider: METALANGLE, parent: Some("UIView"), methods: mgl::MGLKVIEW, cleanup: false },
    ClassTable { name: "MGLKViewController", provider: METALANGLE, parent: Some("UIViewController"), methods: mgl::MGLKVIEWCONTROLLER, cleanup: false },
];

impl UiKit {
    fn links(&self) -> Result<Links, String> {
        self.links.ok_or_else(|| "UIKit runtime links are not configured".into())
    }
    fn layout(&self) -> Result<&Layout, String> {
        self.layout.as_ref().ok_or_else(|| "UIKit image is not installed".into())
    }
    /// Bind runtime entry points (after the image's classes were registered
    /// and its selector references fixed up by that runtime).
    pub(super) fn link(&mut self, cpu: &mut A64Cpu, links: Links) -> Result<(), String> {
        if self.links.is_some() {
            return Err("UIKit runtime links already configured".into());
        }
        for entry in [links.msg_send, links.msg_send_super2, links.retain, links.release, links.alloc_init, links.get_class] {
            if entry == 0 || entry & 3 != 0 || cpu.mapped_permissions(entry).is_none_or(|p| p & 4 == 0) {
                return Err("UIKit runtime link entry is not mapped executable code".into());
            }
        }
        let layout = self.layout()?;
        for (slot, value) in [
            ("objc_msgSend", links.msg_send),
            ("objc_msgSendSuper2", links.msg_send_super2),
            ("objc_alloc_init", links.alloc_init),
        ] {
            let address = *layout.got.get(slot).ok_or("UIKit GOT slot absent")?;
            cpu.write_guest_into(address, &value.to_le_bytes())?;
        }
        self.links = Some(links);
        Ok(())
    }
    /// Host-side part of the owned -dealloc thunk. Strong references held by
    /// the dying object are released through queued guest calls.
    fn cleanup(&mut self, frame: &mut ServiceFrame<'_>, object: u64) -> Result<(), String> {
        let mut releases = Vec::new();
        if self.model.has_view(object) {
            let view = self.model.view(object)?;
            releases.push(view.root_view_controller);
            releases.push(view.background_color);
            releases.push(view.layer);
            releases.extend(self.model.forget_view(object));
            releases.extend(self.mgl.forget_view(object));
        }
        if self.model.has_layer(object) {
            releases.extend(self.model.forget_layer(object));
        }
        if let Some(controller) = self.model.forget_controller(object) {
            releases.push(controller.view);
            self.mgl.forget_controller(object);
        }
        releases.extend(self.mgl.forget_context(object)?);
        self.loading.remove(&object);
        self.layer_pending.remove(&object);
        self.layer_getter.remove(&object);
        releases.retain(|&o| o != 0);
        if releases.is_empty() {
            return Ok(());
        }
        let links = self.links()?;
        if releases.len() > 7 {
            return Err("UIKit -dealloc would release more than 7 objects; batched release is a later milestone".into());
        }
        for object in releases {
            frame.request_guest_call(
                GuestCall { entry: links.release, integers: vec![object], ..Default::default() },
                |result| result.map(|_| ()),
            )?;
        }
        Ok(())
    }
    fn internal(&mut self, frame: &mut ServiceFrame<'_>, kind: Internal) -> Result<ReturnValues, String> {
        match kind {
            Internal::Cleanup => {
                let object = frame.integer(0)?;
                self.cleanup(frame, object)?;
                ret_void()
            }
            Internal::AttachLayer => {
                let (view, layer) = (frame.integer(0)?, frame.integer(1)?);
                self.layer_pending.remove(&view);
                if layer == 0 {
                    return Err("UIView +layerClass allocation returned nil".into());
                }
                self.model.attach_layer(view, layer)?;
                ret_void()
            }
            Internal::PumpStart => app::pump_start(self, frame),
            Internal::PumpNext => app::pump_next(self, frame),
        }
    }
}

/// Register the dispatcher service, build and map the image. The caller then
/// registers the image's classes with its Objective-C runtime (applying the
/// selector-reference fixups), binds app references through
/// [Layout::exports], and finally calls [UiKit::link].
pub(super) fn install(
    cpu: &mut A64Cpu,
    bridge: &mut GuestBridge,
    base: u64,
    external: External,
    model: Model,
    gles: Box<dyn mgl::GlesHost>,
    launch: Option<app::LaunchPlan>,
) -> Result<Installed, String> {
    let state = Rc::new_cyclic(|me| {
        RefCell::new(UiKit {
            model,
            layout: None,
            links: None,
            loading: BTreeSet::new(),
            layer_pending: BTreeSet::new(),
            layer_getter: BTreeSet::new(),
            me: me.clone(),
            mgl: mgl::MglState::new(gles),
            launcher: app::Launcher::new(launch),
            first_use: Vec::new(),
        })
    });
    let mut entries: Vec<Entry> = [Internal::Cleanup, Internal::PumpStart, Internal::PumpNext, Internal::AttachLayer]
        .into_iter()
        .map(Entry::Internal)
        .collect();
    debug_assert_eq!(entries.len(), INTERNAL_COUNT as usize);
    let mut defs = Vec::new();
    for table in CLASSES {
        let mut instance_methods = Vec::new();
        let mut class_methods = Vec::new();
        for method in table.methods {
            let index = u16::try_from(entries.len()).map_err(|_| "UIKit entry index overflow")?;
            entries.push(Entry::Method { class: table.name, method });
            let def = MethodDef { selector: method.selector, types: method.types, index };
            if method.class_method {
                class_methods.push(def);
            } else {
                instance_methods.push(def);
            }
        }
        defs.push(ClassDef {
            name: table.name,
            provider: table.provider,
            parent: table.parent,
            instance_size: 8,
            instance_methods,
            class_methods,
            dealloc_cleanup: table.cleanup,
        });
    }
    let entry_count = u16::try_from(entries.len()).map_err(|_| "UIKit entry count overflow")?;
    let entries = Rc::new(entries);
    let dispatcher = {
        let state = state.clone();
        let entries = entries.clone();
        bridge.register_service(cpu, "_touchHLE_UIKit", move |frame| {
            let index = frame.dispatch_index() as usize;
            let entry = entries
                .get(index)
                .ok_or_else(|| format!("UIKit dispatch index {index} out of range"))?;
            let mut kit = state.try_borrow_mut().map_err(|_| "reentrant UIKit call")?;
            match entry {
                Entry::Internal(kind) => kit.internal(frame, *kind),
                Entry::Method { class, method } => {
                    let receiver = frame.integer(0)?;
                    let selector = frame.integer(1)?;
                    if kit.first_use.len() < 4096 {
                        let sign = if method.class_method { '+' } else { '-' };
                        let name = format!("{sign}[{class} {}]", method.selector);
                        if !kit.first_use.contains(&name) {
                            kit.first_use.push(name);
                        }
                    }
                    (method.handler)(&mut Call { kit: &mut kit, frame, receiver, selector })
                }
            }
        })?
    };
    let statics = [
        StaticObject { class: "UIApplication", size: 16 },
        StaticObject { class: "UIScreen", size: 16 },
        StaticObject { class: "UIDevice", size: 16 },
    ];
    let functions = [
        FunctionDef { symbol: app::UI_APPLICATION_MAIN, provider: Some(INSTALL_NAME), generate: app::pump_code },
        FunctionDef { symbol: quartz::CREATE_LAYER, provider: None, generate: quartz::create_layer_code },
    ];
    let built = image::build(&ImageSpec {
        base,
        install_name: INSTALL_NAME,
        classes: &defs,
        statics: &statics,
        functions: &functions,
        send_selectors: SENT_SELECTORS,
        got: GOT,
        entry_count,
        cleanup_entry: Some(Internal::Cleanup as u16),
        dispatcher: dispatcher.guest_address(),
        scratch_bytes: SCRATCH_BYTES,
        external,
    })?;
    built.map(cpu)?;
    let layout = built.layout.clone();
    {
        let mut kit = state.borrow_mut();
        kit.model.application.object = layout.static_object("UIApplication").unwrap_or(0);
        kit.model.application.state = ApplicationState::Inactive;
        kit.layout = Some(layout.clone());
    }
    Ok(Installed {
        state,
        layout,
        services: vec![("_touchHLE_UIKit".to_string(), dispatcher)],
    })
}

/// Method/class counts, for documentation and budget checks.
pub(super) fn table_counts() -> (usize, usize) {
    (CLASSES.len(), CLASSES.iter().map(|c| c.methods.len()).sum())
}

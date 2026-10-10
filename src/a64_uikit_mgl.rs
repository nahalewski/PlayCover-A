/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! MetalANGLE MGLKit stand-in: MGLContext, MGLKView, MGLKViewController.
//!
//! Decision (see dev-docs/ARM64_UIKIT_PLAN.md section 3.6): the bundled
//! MetalANGLE cannot work here. Its MGLLayer picks Metal
//! (CAMetalLayer/MTLDevice) or its legacy GL backend (CAEAGLLayer + EAGL from
//! the cached OpenGLES.framework), and both need GPU kernel services
//! (IOGPU/IOSurface) that the ARM64 runtime does not provide. Coromon imports
//! exactly three MetalANGLE classes (MGLContext, MGLKView, MGLKViewController)
//! and 72 `gl*` functions, so the framework is replaced: these classes are
//! owned here (subclassing our UIView/UIViewController), and `gl*` is
//! forwarded to host GLES by the frameworks layer (its M7 `gles` family).
//! Behaviour follows MetalANGLE's MGLKit sources (MGLKView.mm,
//! MGLKViewController.mm), minus the Metal/EAGL internals.
//!
//! Context/surface operations go through [GlesHost], implemented over the
//! host GL by the frameworks layer; [RecordingGles] records them for tests.
use super::{
    c, controller_init, controller_set_view, i, init_view, no_op, quartz, ret, ret_bool, ret_f64s,
    ret_void, returns_nil, state::ViewKind, Call, Method,
};
use super::super::bridge::{GuestCall, ReturnValues};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

/// The GL drawable for one MGLLayer.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::a64) struct Surface {
    pub layer: u64,
    pub width: u32,
    pub height: u32,
    pub color: i64,
    pub depth: i64,
    pub stencil: i64,
}

/// Host GLES services (EGL-like). Implemented over the host GL context by the
/// frameworks layer; contexts are opaque nonzero handles.
pub(in crate::a64) trait GlesHost {
    fn create_context(&mut self, api: u64) -> Result<u64, String>;
    fn destroy_context(&mut self, handle: u64);
    /// `handle` 0 releases the current context.
    fn make_current(&mut self, handle: u64, surface: Option<&Surface>) -> Result<(), String>;
    fn bind_default_framebuffer(&mut self, surface: &Surface) -> Result<(), String>;
    fn present(&mut self, handle: u64, surface: &Surface) -> Result<(), String>;
}

/// Records every call (desktop tests; also a safe default before the host
/// GLES family exists: it renders nothing but keeps the frame loop honest).
#[derive(Default)]
pub(in crate::a64) struct RecordingGles {
    pub log: Rc<RefCell<Vec<String>>>,
    next: u64,
}
impl RecordingGles {
    pub(in crate::a64) fn new(log: Rc<RefCell<Vec<String>>>) -> Self {
        Self { log, next: 0 }
    }
}
impl GlesHost for RecordingGles {
    fn create_context(&mut self, api: u64) -> Result<u64, String> {
        if !(1..=3).contains(&api) {
            return Err(format!("MGLContext API {api} unsupported"));
        }
        self.next += 1;
        self.log.borrow_mut().push(format!("create {} api={api}", self.next));
        Ok(self.next)
    }
    fn destroy_context(&mut self, handle: u64) {
        self.log.borrow_mut().push(format!("destroy {handle}"));
    }
    fn make_current(&mut self, handle: u64, surface: Option<&Surface>) -> Result<(), String> {
        let s = surface.map_or("none".to_string(), |s| format!("{}x{}", s.width, s.height));
        self.log.borrow_mut().push(format!("current {handle} {s}"));
        Ok(())
    }
    fn bind_default_framebuffer(&mut self, surface: &Surface) -> Result<(), String> {
        self.log.borrow_mut().push(format!("bind {}x{}", surface.width, surface.height));
        Ok(())
    }
    fn present(&mut self, handle: u64, surface: &Surface) -> Result<(), String> {
        self.log.borrow_mut().push(format!("present {handle} {}x{}", surface.width, surface.height));
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub(in crate::a64) struct LayerFormats {
    pub color: i64,
    pub depth: i64,
    pub stencil: i64,
    pub multisample: i64,
}
#[derive(Clone, Debug)]
pub(in crate::a64) struct GlView {
    pub context: u64,
    pub delegate: u64,
    pub controller: u64,
    pub enable_set_needs_display: bool,
    pub retained_backing: bool,
    pub drawing: bool,
}
#[derive(Clone, Debug)]
pub(in crate::a64) struct GlController {
    pub preferred_fps: i64,
    pub paused: bool,
    pub gl_view: u64,
    pub delegate: u64,
    pub frames_displayed: i64,
    pub time_since_last_update: f64,
    pub last_update: Option<f64>,
}
impl Default for GlController {
    fn default() -> Self {
        // MGLKViewController -constructor defaults.
        Self {
            preferred_fps: 30,
            paused: true,
            gl_view: 0,
            delegate: 0,
            frames_displayed: 0,
            time_since_last_update: 0.0,
            last_update: None,
        }
    }
}

pub(in crate::a64) struct MglState {
    gles: Box<dyn GlesHost>,
    contexts: BTreeMap<u64, (u64, u64)>,
    current: u64,
    views: BTreeMap<u64, GlView>,
    controllers: BTreeMap<u64, GlController>,
    layers: BTreeMap<u64, LayerFormats>,
    /// Resumed controllers, driven by the frame source in frame order.
    pub frame_targets: Vec<u64>,
    /// Host media time in seconds (advanced by the frame source).
    pub media_time: f64,
}
impl MglState {
    pub(in crate::a64) fn new(gles: Box<dyn GlesHost>) -> Self {
        Self {
            gles,
            contexts: BTreeMap::new(),
            current: 0,
            views: BTreeMap::new(),
            controllers: BTreeMap::new(),
            layers: BTreeMap::new(),
            frame_targets: Vec::new(),
            media_time: 0.0,
        }
    }
    pub(in crate::a64) fn ensure_layer(&mut self, layer: u64) -> &mut LayerFormats {
        self.layers.entry(layer).or_default()
    }
    fn ensure_view(&mut self, view: u64) -> &mut GlView {
        self.views.entry(view).or_insert(GlView {
            context: 0,
            delegate: 0,
            controller: 0,
            enable_set_needs_display: true,
            retained_backing: false,
            drawing: false,
        })
    }
    pub(in crate::a64) fn view(&self, view: u64) -> Option<&GlView> {
        self.views.get(&view)
    }
    fn ensure_controller(&mut self, controller: u64) -> &mut GlController {
        self.controllers.entry(controller).or_default()
    }
    pub(in crate::a64) fn controller(&self, controller: u64) -> Option<&GlController> {
        self.controllers.get(&controller)
    }
    /// Returns strong references to release (the view's context).
    pub(in crate::a64) fn forget_view(&mut self, view: u64) -> Vec<u64> {
        let Some(v) = self.views.remove(&view) else { return Vec::new() };
        for c in self.controllers.values_mut() {
            if c.gl_view == view {
                c.gl_view = 0;
            }
        }
        vec![v.context]
    }
    pub(in crate::a64) fn forget_controller(&mut self, controller: u64) {
        self.controllers.remove(&controller);
        self.frame_targets.retain(|&c| c != controller);
        for v in self.views.values_mut() {
            if v.controller == controller {
                v.controller = 0;
            }
            if v.delegate == controller {
                v.delegate = 0;
            }
        }
    }
    pub(in crate::a64) fn forget_context(&mut self, context: u64) -> Result<Vec<u64>, String> {
        if let Some((handle, _)) = self.contexts.remove(&context) {
            if self.current == context {
                self.gles.make_current(0, None)?;
                self.current = 0;
            }
            self.gles.destroy_context(handle);
        }
        Ok(Vec::new())
    }
    fn handle(&self, context: u64) -> Result<u64, String> {
        self.contexts
            .get(&context)
            .map(|&(h, _)| h)
            .ok_or_else(|| format!("MGLContext {context:#x} was not initialised"))
    }
}

fn surface(call: &mut Call, view: u64) -> Result<Surface, String> {
    let layer = call.kit.model.view(view)?.layer;
    if layer == 0 {
        return Err("MGLKView has no backing MGLLayer".into());
    }
    let (w, h) = quartz::drawable_size(call, layer)?;
    let formats = call.kit.mgl.ensure_layer(layer).clone();
    Ok(Surface {
        layer,
        width: w.max(0.0) as u32,
        height: h.max(0.0) as u32,
        color: formats.color,
        depth: formats.depth,
        stencil: formats.stencil,
    })
}

// ----------------------------------------------------------------- MGLContext
fn context_init_with_api(call: &mut Call) -> Result<ReturnValues, String> {
    let api = call.arg(2)?;
    if call.kit.mgl.contexts.contains_key(&call.receiver) {
        return Err("MGLContext initialised twice".into());
    }
    let handle = call.kit.mgl.gles.create_context(api)?;
    call.kit.mgl.contexts.insert(call.receiver, (handle, api));
    ret(call.receiver)
}
fn context_api(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.mgl.contexts.get(&call.receiver).map_or(0, |&(_, api)| api))
}
fn context_current(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.mgl.current)
}
fn make_current(call: &mut Call, context: u64, view: Option<u64>) -> Result<(), String> {
    let surface = match view {
        Some(view) => Some(surface(call, view)?),
        None => None,
    };
    let handle = if context == 0 { 0 } else { call.kit.mgl.handle(context)? };
    call.kit.mgl.gles.make_current(handle, surface.as_ref())?;
    call.kit.mgl.current = context;
    Ok(())
}
fn context_set_current(call: &mut Call) -> Result<ReturnValues, String> {
    let context = call.arg(2)?;
    make_current(call, context, None)?;
    ret_bool(true)
}
fn view_of_layer(call: &Call, layer: u64) -> Result<Option<u64>, String> {
    if layer == 0 {
        return Ok(None);
    }
    let view = call.kit.model.layer(layer)?.view;
    Ok((view != 0).then_some(view))
}
fn context_set_current_for_layer(call: &mut Call) -> Result<ReturnValues, String> {
    let (context, layer) = (call.arg(2)?, call.arg(3)?);
    let view = view_of_layer(call, layer)?;
    make_current(call, context, view)?;
    ret_bool(true)
}
fn context_present(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = call.arg(2)?;
    let view = view_of_layer(call, layer)?.ok_or("MGLContext -present: layer has no view")?;
    let s = surface(call, view)?;
    let handle = call.kit.mgl.handle(call.receiver)?;
    call.kit.mgl.gles.present(handle, &s)?;
    ret_bool(true)
}
pub(in crate::a64) const MGLCONTEXT: &[Method] = &[
    c("currentContext", "@16@0:8", context_current),
    c("setCurrentContext:", "B24@0:8@16", context_set_current),
    c("setCurrentContext:forLayer:", "B32@0:8@16@24", context_set_current_for_layer),
    i("initWithAPI:", "@24@0:8Q16", context_init_with_api),
    i("initWithAPI:sharegroup:", "@32@0:8Q16@24", context_init_with_api),
    i("API", "Q16@0:8", context_api),
    i("present:", "B24@0:8@16", context_present),
];

// ------------------------------------------------------------------- MGLKView
fn gl_view_init(call: &mut Call) -> Result<u64, String> {
    let frame = call.rect_arg(0)?;
    let view = init_view(call, ViewKind::View, frame)?;
    call.kit.mgl.ensure_view(view);
    Ok(view)
}
fn mglkview_init_with_frame(call: &mut Call) -> Result<ReturnValues, String> {
    ret(gl_view_init(call)?)
}
fn set_context(call: &mut Call, view: u64, context: u64) -> Result<(), String> {
    let v = call.kit.mgl.ensure_view(view);
    if v.drawing {
        return Err("Changing GL context when drawing is not allowed".into());
    }
    let old = std::mem::replace(&mut v.context, context);
    if old != context {
        call.retain(context)?;
        call.release(old)?;
    }
    Ok(())
}
fn mglkview_init_with_frame_context(call: &mut Call) -> Result<ReturnValues, String> {
    let view = gl_view_init(call)?;
    let context = call.arg(2)?;
    set_context(call, view, context)?;
    ret(view)
}
fn mglkview_layer_class(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.class("MGLLayer")?)
}
fn mglkview_context(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.mgl.view(call.receiver).map_or(0, |v| v.context))
}
fn mglkview_set_context(call: &mut Call) -> Result<ReturnValues, String> {
    let (view, context) = (call.receiver, call.arg(2)?);
    set_context(call, view, context)?;
    ret_void()
}
fn mglkview_delegate(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.mgl.view(call.receiver).map_or(0, |v| v.delegate))
}
fn mglkview_set_delegate(call: &mut Call) -> Result<ReturnValues, String> {
    let delegate = call.arg(2)?;
    call.kit.mgl.ensure_view(call.receiver).delegate = delegate;
    ret_void()
}
fn mglkview_gl_layer(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.model.view(call.receiver)?.layer)
}
fn mglkview_drawable_size(call: &mut Call) -> Result<ReturnValues, String> {
    let s = surface(call, call.receiver)?;
    ret_f64s(&[s.width as f64, s.height as f64])
}
fn mglkview_drawable_width(call: &mut Call) -> Result<ReturnValues, String> {
    ret(surface(call, call.receiver)?.width as u64)
}
fn mglkview_drawable_height(call: &mut Call) -> Result<ReturnValues, String> {
    ret(surface(call, call.receiver)?.height as u64)
}
macro_rules! mglkview_format {
    ($get:ident, $set:ident, $field:ident) => {
        fn $get(call: &mut Call) -> Result<ReturnValues, String> {
            let layer = call.kit.model.view(call.receiver)?.layer;
            ret(call.kit.mgl.ensure_layer(layer).$field as u64)
        }
        fn $set(call: &mut Call) -> Result<ReturnValues, String> {
            let layer = call.kit.model.view(call.receiver)?.layer;
            if layer == 0 {
                return Err("MGLKView drawable format set before its layer exists".into());
            }
            let value = call.arg(2)? as i64;
            call.kit.mgl.ensure_layer(layer).$field = value;
            ret_void()
        }
    };
}
mglkview_format!(mglkview_color, mglkview_set_color, color);
mglkview_format!(mglkview_depth, mglkview_set_depth, depth);
mglkview_format!(mglkview_stencil, mglkview_set_stencil, stencil);
mglkview_format!(mglkview_multisample, mglkview_set_multisample, multisample);
fn mglkview_enable_set_needs_display(call: &mut Call) -> Result<ReturnValues, String> {
    ret_bool(call.kit.mgl.view(call.receiver).is_some_and(|v| v.enable_set_needs_display))
}
fn mglkview_set_enable_set_needs_display(call: &mut Call) -> Result<ReturnValues, String> {
    let value = call.bool_arg(2)?;
    call.kit.mgl.ensure_view(call.receiver).enable_set_needs_display = value;
    ret_void()
}
fn mglkview_retained_backing(call: &mut Call) -> Result<ReturnValues, String> {
    ret_bool(call.kit.mgl.view(call.receiver).is_some_and(|v| v.retained_backing))
}
fn mglkview_set_retained_backing(call: &mut Call) -> Result<ReturnValues, String> {
    let value = call.bool_arg(2)?;
    call.kit.mgl.ensure_view(call.receiver).retained_backing = value;
    ret_void()
}
fn mglkview_bind_drawable(call: &mut Call) -> Result<ReturnValues, String> {
    let s = surface(call, call.receiver)?;
    call.kit.mgl.gles.bind_default_framebuffer(&s)?;
    ret_void()
}
/// -display: make the context current for this layer, send the receiver's
/// own -drawRect: (Corona overrides it), then present.
fn mglkview_display(call: &mut Call) -> Result<ReturnValues, String> {
    let view = call.receiver;
    let context = call.kit.mgl.ensure_view(view).context;
    if context != 0 {
        make_current(call, context, Some(view))?;
    }
    call.kit.mgl.ensure_view(view).drawing = true;
    let bounds = call.kit.model.view(view)?.bounds;
    let selector = call.sent_selector("drawRect:")?;
    let entry = call.links()?.msg_send;
    let me = call.kit.me.clone();
    call.frame.request_guest_call(
        GuestCall {
            entry,
            integers: vec![view, selector],
            vectors: [bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height]
                .iter()
                .map(|v| [v.to_bits(), 0])
                .collect(),
            ..Default::default()
        },
        move |result| {
            let kit = me.upgrade().ok_or("UIKit state dropped")?;
            let mut kit = kit.try_borrow_mut().map_err(|_| "reentrant MGLKView display completion")?;
            kit.mgl.ensure_view(view).drawing = false;
            result?;
            if context == 0 {
                return Err("Failed to present framebuffer: MGLKView has no context".into());
            }
            let layer = kit.model.view(view)?.layer;
            let l = kit.model.layer(layer)?;
            let (w, h) = (l.bounds.size.width * l.contents_scale, l.bounds.size.height * l.contents_scale);
            let formats = kit.mgl.ensure_layer(layer).clone();
            let s = Surface {
                layer,
                width: w.max(0.0) as u32,
                height: h.max(0.0) as u32,
                color: formats.color,
                depth: formats.depth,
                stencil: formats.stencil,
            };
            let handle = kit.mgl.handle(context)?;
            kit.mgl.gles.present(handle, &s)
        },
    )?;
    ret_void()
}
/// Default -drawRect: forwards to the MGLKViewDelegate.
fn mglkview_draw_rect(call: &mut Call) -> Result<ReturnValues, String> {
    let view = call.receiver;
    let delegate = call.kit.mgl.view(view).map_or(0, |v| v.delegate);
    if delegate == 0 {
        return ret_void();
    }
    let selector = call.sent_selector("mglkView:drawInRect:")?;
    let entry = call.links()?.msg_send;
    let rect = call.rect_arg(0)?;
    call.queue_call(GuestCall {
        entry,
        integers: vec![delegate, selector, view],
        vectors: [rect.origin.x, rect.origin.y, rect.size.width, rect.size.height]
            .iter()
            .map(|v| [v.to_bits(), 0])
            .collect(),
        ..Default::default()
    })?;
    ret_void()
}
pub(in crate::a64) const MGLKVIEW: &[Method] = &[
    c("layerClass", "#16@0:8", mglkview_layer_class),
    i("initWithFrame:", "@48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", mglkview_init_with_frame),
    i("initWithFrame:context:", "@56@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16@48", mglkview_init_with_frame_context),
    i("context", "@16@0:8", mglkview_context),
    i("setContext:", "v24@0:8@16", mglkview_set_context),
    i("delegate", "@16@0:8", mglkview_delegate),
    i("setDelegate:", "v24@0:8@16", mglkview_set_delegate),
    i("glLayer", "@16@0:8", mglkview_gl_layer),
    i("drawableSize", "{CGSize=dd}16@0:8", mglkview_drawable_size),
    i("drawableWidth", "q16@0:8", mglkview_drawable_width),
    i("drawableHeight", "q16@0:8", mglkview_drawable_height),
    i("defaultOpenGLFrameBufferID", "I16@0:8", returns_nil),
    i("drawableColorFormat", "q16@0:8", mglkview_color),
    i("setDrawableColorFormat:", "v24@0:8q16", mglkview_set_color),
    i("drawableDepthFormat", "q16@0:8", mglkview_depth),
    i("setDrawableDepthFormat:", "v24@0:8q16", mglkview_set_depth),
    i("drawableStencilFormat", "q16@0:8", mglkview_stencil),
    i("setDrawableStencilFormat:", "v24@0:8q16", mglkview_set_stencil),
    i("drawableMultisample", "q16@0:8", mglkview_multisample),
    i("setDrawableMultisample:", "v24@0:8q16", mglkview_set_multisample),
    i("enableSetNeedsDisplay", "B16@0:8", mglkview_enable_set_needs_display),
    i("setEnableSetNeedsDisplay:", "v20@0:8B16", mglkview_set_enable_set_needs_display),
    i("retainedBacking", "B16@0:8", mglkview_retained_backing),
    i("setRetainedBacking:", "v20@0:8B16", mglkview_set_retained_backing),
    i("bindDrawable", "v16@0:8", mglkview_bind_drawable),
    i("display", "v16@0:8", mglkview_display),
    i("drawRect:", "v48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", mglkview_draw_rect),
    i("snapshot", "@16@0:8", returns_nil),
    i("setNeedsDisplay", "v16@0:8", no_op),
];

// --------------------------------------------------------- MGLKViewController
fn glc_init(call: &mut Call) -> Result<ReturnValues, String> {
    call.kit.mgl.ensure_controller(call.receiver);
    controller_init(call)
}
/// -setView: as MGLKViewController: UIKit's setView: plus the glView link.
fn glc_set_view(call: &mut Call) -> Result<ReturnValues, String> {
    controller_set_view(call)?;
    let (controller, view) = (call.receiver, call.arg(2)?);
    let is_gl = view != 0 && call.kit.mgl.view(view).is_some();
    let old = call.kit.mgl.ensure_controller(controller).gl_view;
    if is_gl {
        call.kit.mgl.ensure_controller(controller).gl_view = view;
        let v = call.kit.mgl.ensure_view(view);
        v.enable_set_needs_display = false;
        if v.delegate == 0 {
            v.delegate = controller;
        }
        v.controller = controller;
    } else {
        if let Some(v) = call.kit.mgl.views.get_mut(&old) {
            if v.delegate == controller {
                v.delegate = 0;
            }
            if v.controller == controller {
                v.controller = 0;
            }
        }
        call.kit.mgl.ensure_controller(controller).gl_view = 0;
    }
    ret_void()
}
fn pause(call: &mut Call, controller: u64) {
    let c = call.kit.mgl.ensure_controller(controller);
    if c.paused {
        return;
    }
    c.paused = true;
    call.kit.mgl.frame_targets.retain(|&t| t != controller);
}
fn resume(call: &mut Call, controller: u64) {
    let c = call.kit.mgl.ensure_controller(controller);
    if !c.paused || c.gl_view == 0 {
        return;
    }
    c.paused = false;
    c.last_update = None;
    if !call.kit.mgl.frame_targets.contains(&controller) {
        call.kit.mgl.frame_targets.push(controller);
    }
}
fn glc_pause(call: &mut Call) -> Result<ReturnValues, String> {
    pause(call, call.receiver);
    ret_void()
}
fn glc_resume(call: &mut Call) -> Result<ReturnValues, String> {
    resume(call, call.receiver);
    ret_void()
}
fn glc_is_paused(call: &mut Call) -> Result<ReturnValues, String> {
    ret_bool(call.kit.mgl.ensure_controller(call.receiver).paused)
}
fn glc_set_paused(call: &mut Call) -> Result<ReturnValues, String> {
    if call.bool_arg(2)? {
        pause(call, call.receiver);
    } else {
        resume(call, call.receiver);
    }
    ret_void()
}
fn glc_fps(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.mgl.ensure_controller(call.receiver).preferred_fps as u64)
}
fn glc_set_fps(call: &mut Call) -> Result<ReturnValues, String> {
    let fps = call.arg(2)? as i64;
    call.kit.mgl.ensure_controller(call.receiver).preferred_fps = fps;
    // MetalANGLE: [self pause]; [self resume];
    pause(call, call.receiver);
    resume(call, call.receiver);
    ret_void()
}
fn glc_gl_view(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.mgl.ensure_controller(call.receiver).gl_view)
}
fn glc_delegate(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.mgl.ensure_controller(call.receiver).delegate)
}
fn glc_set_delegate(call: &mut Call) -> Result<ReturnValues, String> {
    let delegate = call.arg(2)?;
    call.kit.mgl.ensure_controller(call.receiver).delegate = delegate;
    ret_void()
}
fn glc_frames(call: &mut Call) -> Result<ReturnValues, String> {
    ret(call.kit.mgl.ensure_controller(call.receiver).frames_displayed as u64)
}
fn glc_time_since(call: &mut Call) -> Result<ReturnValues, String> {
    ret_f64s(&[call.kit.mgl.ensure_controller(call.receiver).time_since_last_update])
}
fn glc_view_did_appear(call: &mut Call) -> Result<ReturnValues, String> {
    // MetalANGLE also observes resign/become-active notifications here; the
    // host application lifecycle drives pause/resume directly instead.
    resume(call, call.receiver);
    ret_void()
}
fn glc_view_did_disappear(call: &mut Call) -> Result<ReturnValues, String> {
    pause(call, call.receiver);
    ret_void()
}
fn glc_update(call: &mut Call) -> Result<ReturnValues, String> {
    let (controller, delegate) = (call.receiver, call.kit.mgl.ensure_controller(call.receiver).delegate);
    if delegate != 0 {
        call.send(delegate, "mglkViewControllerUpdate:", &[controller])?;
    }
    ret_void()
}
/// The display-link callback: [self update]; [glView display].
fn glc_frame_step(call: &mut Call) -> Result<ReturnValues, String> {
    let controller = call.receiver;
    let now = call.kit.mgl.media_time;
    let c = call.kit.mgl.ensure_controller(controller);
    c.time_since_last_update = c.last_update.map_or(0.0, |last| now - last);
    c.last_update = Some(now);
    c.frames_displayed += 1;
    let view = c.gl_view;
    call.send(controller, "update", &[])?;
    if view != 0 {
        call.send(view, "display", &[])?;
    }
    ret_void()
}
pub(in crate::a64) const MGLKVIEWCONTROLLER: &[Method] = &[
    i("init", "@16@0:8", glc_init),
    i("initWithNibName:bundle:", "@32@0:8@16@24", glc_init),
    i("setView:", "v24@0:8@16", glc_set_view),
    i("glView", "@16@0:8", glc_gl_view),
    i("delegate", "@16@0:8", glc_delegate),
    i("setDelegate:", "v24@0:8@16", glc_set_delegate),
    i("preferredFramesPerSecond", "q16@0:8", glc_fps),
    i("setPreferredFramesPerSecond:", "v24@0:8q16", glc_set_fps),
    i("framesDisplayed", "q16@0:8", glc_frames),
    i("timeSinceLastUpdate", "d16@0:8", glc_time_since),
    i("isPaused", "B16@0:8", glc_is_paused),
    i("setPaused:", "v20@0:8B16", glc_set_paused),
    i("pause", "v16@0:8", glc_pause),
    i("resume", "v16@0:8", glc_resume),
    i("viewDidAppear:", "v20@0:8B16", glc_view_did_appear),
    i("viewDidDisappear:", "v20@0:8B16", glc_view_did_disappear),
    i("mglkView:drawInRect:", "v56@0:8@16{CGRect={CGPoint=dd}{CGSize=dd}}24", no_op),
    i("update", "v16@0:8", glc_update),
    i("frameStep", "v16@0:8", glc_frame_step),
];

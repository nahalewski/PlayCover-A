/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! GLKit: `GLKView`, `GLKViewController` and a few constants.
//!
//! `GLKView` is a `UIView` whose layer is a `CAEAGLLayer` and which owns the
//! framebuffer the app draws into; `GLKViewController` drives it with a timer,
//! calling the app's `update` and then the view's `display`.
//!
//! Per-object state lives in tables keyed by the guest object's address, so
//! the classes can sit on top of `UIView`/`UIViewController` without needing
//! access to their private host objects.

use crate::dyld::{ConstantExports, HostConstant};
use crate::frameworks::core_graphics::CGRect;
use crate::frameworks::foundation::ns_string::get_static_str;
use crate::frameworks::foundation::NSInteger;
use crate::frameworks::opengles::eagl::{kEAGLColorFormatRGBA8, kEAGLDrawablePropertyColorFormat};
use crate::frameworks::opengles::gles_guest::{glk_bind_drawable, glk_create_drawable};
use crate::gles::gles11_raw as gles11; // constants only
use crate::objc::{
    id, msg, msg_class, msg_super, nil, objc_classes, release, retain, ClassExports, Class,
};
use crate::Environment;
use std::collections::HashMap;
use std::sync::Mutex;

pub const GLKIT_DEFAULT_FPS: NSInteger = 30;

#[derive(Default)]
struct GlkViewState {
    context: u32,
    delegate: u32,
    framebuffer: u32,
    color_renderbuffer: u32,
    /// 0 = none, 1 = 16 bit, 2 = 24 bit (`GLKViewDrawableDepthFormat`)
    depth_format: NSInteger,
    width: u32,
    height: u32,
}

#[derive(Default)]
struct GlkControllerState {
    preferred_fps: NSInteger,
    paused: bool,
    delegate: u32,
    timer: u32,
    last_update: Option<std::time::Instant>,
    time_since_last_update: f64,
}

static VIEWS: Mutex<Option<HashMap<u32, GlkViewState>>> = Mutex::new(None);
static CONTROLLERS: Mutex<Option<HashMap<u32, GlkControllerState>>> = Mutex::new(None);

fn with_view<R>(view: id, f: impl FnOnce(&mut GlkViewState) -> R) -> R {
    let mut guard = VIEWS.lock().unwrap();
    f(guard.get_or_insert_with(HashMap::new).entry(view.to_bits()).or_default())
}
fn with_controller<R>(vc: id, f: impl FnOnce(&mut GlkControllerState) -> R) -> R {
    let mut guard = CONTROLLERS.lock().unwrap();
    f(guard
        .get_or_insert_with(HashMap::new)
        .entry(vc.to_bits())
        .or_insert_with(|| GlkControllerState {
            preferred_fps: 0,
            ..Default::default()
        }))
}

fn setup_layer(env: &mut Environment, view: id) {
    let layer: id = msg![env; view layer];
    let key = get_static_str(env, kEAGLDrawablePropertyColorFormat);
    let value = get_static_str(env, kEAGLColorFormatRGBA8);
    let props: id = msg_class![env; NSDictionary dictionaryWithObject:value forKey:key];
    () = msg![env; layer setDrawableProperties:props];
}

fn depth_bits(format: NSInteger) -> u32 {
    match format {
        1 => 16,
        2 => 24,
        _ => 0,
    }
}

fn effective_fps(vc: id) -> NSInteger {
    let fps = with_controller(vc, |s| s.preferred_fps);
    if fps <= 0 {
        GLKIT_DEFAULT_FPS
    } else {
        fps
    }
}

/// (Re)starts the controller's frame timer for the current frame rate.
fn restart_timer(env: &mut Environment, vc: id) {
    let old = with_controller(vc, |s| std::mem::replace(&mut s.timer, 0));
    if old != 0 {
        let old_timer: id = crate::mem::Ptr::from_bits(old);
        () = msg![env; old_timer invalidate];
    }
    let interval = 1.0 / effective_fps(vc) as f64;
    let sel = env
        .objc
        .register_host_selector("_touchHLE_glkTick:".to_string(), &mut env.mem);
    let timer: id = msg_class![env; NSTimer scheduledTimerWithTimeInterval:interval
                                                                   target:vc
                                                                 selector:sel
                                                                 userInfo:nil
                                                                  repeats:true];
    log!("GLKViewController {:?}: frame timer started ({} fps)", vc, effective_fps(vc));
    with_controller(vc, |s| s.timer = timer.to_bits());
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation GLKView: UIView

+ (Class)layerClass {
    env.objc.get_known_class("CAEAGLLayer", &mut env.mem)
}

- (id)initWithFrame:(CGRect)frame {
    let this: id = msg_super![env; this initWithFrame:frame];
    setup_layer(env, this);
    this
}

- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];
    setup_layer(env, this);
    this
}

- (id)initWithFrame:(CGRect)frame context:(id)context {
    let this: id = msg![env; this initWithFrame:frame];
    () = msg![env; this setContext:context];
    this
}

- (id)context {
    crate::mem::Ptr::from_bits(with_view(this, |s| s.context))
}
- (())setContext:(id)context { // EAGLContext*
    retain(env, context);
    let old = with_view(this, |s| std::mem::replace(&mut s.context, context.to_bits()));
    if old != 0 {
        let old: id = crate::mem::Ptr::from_bits(old);
        release(env, old);
    }
}

- (id)delegate {
    crate::mem::Ptr::from_bits(with_view(this, |s| s.delegate))
}
- (())setDelegate:(id)delegate {
    with_view(this, |s| s.delegate = delegate.to_bits());
}

- (())setDrawableColorFormat:(NSInteger)_format {}
- (())setDrawableDepthFormat:(NSInteger)format {
    with_view(this, |s| s.depth_format = format);
}
- (())setDrawableStencilFormat:(NSInteger)_format {}
- (())setDrawableMultisample:(NSInteger)_samples {}
- (())setEnableSetNeedsDisplay:(bool)_enable {}
- (bool)enableSetNeedsDisplay { false }

- (NSInteger)drawableWidth { with_view(this, |s| s.width as NSInteger) }
- (NSInteger)drawableHeight { with_view(this, |s| s.height as NSInteger) }

- (())bindDrawable {
    let context: id = msg![env; this context];
    if context == nil {
        return;
    }
    let _: bool = msg_class![env; EAGLContext setCurrentContext:context];
    let (framebuffer, color) = with_view(this, |s| (s.framebuffer, s.color_renderbuffer));
    if framebuffer == 0 {
        let layer: id = msg![env; this layer];
        let depth = with_view(this, |s| depth_bits(s.depth_format));
        let (fbo, rb, width, height) = glk_create_drawable(env, context, layer, depth);
        with_view(this, |s| {
            s.framebuffer = fbo;
            s.color_renderbuffer = rb;
            s.width = width;
            s.height = height;
        });
        log!("GLKView {:?}: created drawable {}x{} (framebuffer {}, renderbuffer {})", this, width, height, fbo, rb);
    } else {
        glk_bind_drawable(env, framebuffer, color);
    }
}

- (())display {
    let context: id = msg![env; this context];
    if context == nil {
        return;
    }
    () = msg![env; this bindDrawable];
    let bounds: CGRect = msg![env; this bounds];
    let delegate: id = msg![env; this delegate];
    if delegate != nil {
        let responds: bool = msg![env; delegate respondsToSelector:(env.objc.register_host_selector("glkView:drawInRect:".to_string(), &mut env.mem))];
        if responds {
            () = msg![env; delegate glkView:this drawInRect:bounds];
        }
    } else {
        () = msg![env; this drawRect:bounds];
    }
    let color = with_view(this, |s| s.color_renderbuffer);
    if color != 0 {
        // The app may have bound something else while drawing.
        let framebuffer = with_view(this, |s| s.framebuffer);
        glk_bind_drawable(env, framebuffer, color);
        let _: bool = msg![env; context presentRenderbuffer:(gles11::RENDERBUFFER_OES as u32)];
    }
}

- (())drawRect:(CGRect)_rect {}

- (())dealloc {
    let context = with_view(this, |s| std::mem::replace(&mut s.context, 0));
    if context != 0 {
        let context: id = crate::mem::Ptr::from_bits(context);
        release(env, context);
    }
    VIEWS.lock().unwrap().as_mut().map(|m| m.remove(&this.to_bits()));
    msg_super![env; this dealloc]
}

@end

@implementation GLKViewController: UIViewController

- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];
    restart_timer(env, this);
    this
}

- (id)initWithNibName:(id)nib_name bundle:(id)bundle {
    let this: id = msg_super![env; this initWithNibName:nib_name bundle:bundle];
    restart_timer(env, this);
    this
}

- (id)delegate {
    crate::mem::Ptr::from_bits(with_controller(this, |s| s.delegate))
}
- (())setDelegate:(id)delegate {
    with_controller(this, |s| s.delegate = delegate.to_bits());
}

- (NSInteger)preferredFramesPerSecond {
    effective_fps(this)
}
- (())setPreferredFramesPerSecond:(NSInteger)fps {
    with_controller(this, |s| s.preferred_fps = fps);
    restart_timer(env, this);
}
- (NSInteger)framesPerSecond {
    effective_fps(this)
}

- (bool)isPaused {
    with_controller(this, |s| s.paused)
}
- (bool)paused {
    with_controller(this, |s| s.paused)
}
- (())setPaused:(bool)paused {
    with_controller(this, |s| s.paused = paused);
}
- (())setPauseOnWillResignActive:(bool)_pause {}
- (())setResumeOnDidBecomeActive:(bool)_resume {}

- (f64)timeSinceLastUpdate {
    with_controller(this, |s| s.time_since_last_update)
}

- (())_touchHLE_glkTick:(id)_timer {
    if with_controller(this, |s| s.paused) {
        return;
    }
    let now = std::time::Instant::now();
    with_controller(this, |s| {
        s.time_since_last_update = s.last_update.map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        s.last_update = Some(now);
    });
    let view: id = msg![env; this view];
    if view == nil {
        return;
    }
    {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
            let class: Class = msg![env; view class];
            log!("GLKViewController {:?}: tick, view {:?} of class {}", this, view, env.objc.get_class_name(class));
        }
    }
    // The delegate is the controller itself unless one was set.
    let mut delegate: id = msg![env; this delegate];
    if delegate == nil {
        delegate = this;
    }
    let update_sel = env.objc.register_host_selector("glkViewControllerUpdate:".to_string(), &mut env.mem);
    let responds: bool = msg![env; delegate respondsToSelector:update_sel];
    if responds {
        () = msg![env; delegate glkViewControllerUpdate:this];
    } else {
        let update_sel = env.objc.register_host_selector("update".to_string(), &mut env.mem);
        let responds: bool = msg![env; this respondsToSelector:update_sel];
        if responds {
            () = msg![env; this update];
        }
    }
    // Controllers draw their own view, using themselves as its delegate.
    let glk_view_class = env.objc.get_known_class("GLKView", &mut env.mem);
    let view_class: Class = msg![env; view class];
    if env.objc.class_is_subclass_of(view_class, glk_view_class) {
        let view_delegate: id = msg![env; view delegate];
        if view_delegate == nil {
            () = msg![env; view setDelegate:this];
        }
        () = msg![env; view display];
    }
}

- (())dealloc {
    let timer = with_controller(this, |s| std::mem::replace(&mut s.timer, 0));
    if timer != 0 {
        let timer: id = crate::mem::Ptr::from_bits(timer);
        () = msg![env; timer invalidate];
    }
    CONTROLLERS.lock().unwrap().as_mut().map(|m| m.remove(&this.to_bits()));
    msg_super![env; this dealloc]
}

@end

};

/// `GLKMatrix4Identity`: a `GLKMatrix4` (16 floats, column-major).
pub const CONSTANTS: ConstantExports = &[(
    "_GLKMatrix4Identity",
    HostConstant::Custom(|env| {
        let ptr = env.mem.alloc(64).cast::<f32>();
        for i in 0..16u32 {
            let value = if i % 5 == 0 { 1.0f32 } else { 0.0f32 };
            env.mem.write(ptr + i, value);
        }
        ptr.cast().cast_const()
    }),
)];

pub const DYLIB: crate::dyld::HostDylib = crate::dyld::HostDylib {
    path: "/System/Library/Frameworks/GLKit.framework/GLKit",
    aliases: &[],
    class_exports: &[CLASSES],
    constant_exports: &[CONSTANTS],
    function_exports: &[],
};

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Owned CALayer (QuartzCore) and the MetalANGLE MGLLayer stand-in.
//!
//! There is no compositor: layers are host records (geometry, scale,
//! hierarchy) that the presenting GL surface reads. A view's backing layer is
//! created by owned guest code ([create_layer_code]) that sends +layerClass to
//! the view's real class, so guest overrides are honoured, then allocates it
//! with objc_alloc_init exactly as UIKit does.
use super::{
    asm::Asm,
    c, i,
    image::Symbols,
    no_op, ret, ret_bool, ret_f32, ret_f64s, ret_rect, ret_void,
    state::Point,
    Call, Internal, Method,
};
use super::super::bridge::ReturnValues;

pub(in crate::a64) const CREATE_LAYER: &str = "_touchHLE_UIKit_createLayer";

/// x0 = view. `[[object_getClass(view) layerClass] alloc] init]`, then the
/// AttachLayer entry with (view, layer). The isa is masked with objc4's
/// arm64 ISA_MASK, valid for both raw and non-pointer isa words.
pub(in crate::a64) fn create_layer_code(sym: &Symbols<'_>) -> Asm {
    let mut a = Asm::default();
    a.prologue(32)
        .str(0, 31, 16)
        .ldr(9, 0, 0)
        .and_isa_mask(0, 9)
        .ldr_indirect(1, sym.selref("layerClass"))
        .ldr_indirect(16, sym.got("objc_msgSend"))
        .blr(16)
        .ldr_indirect(16, sym.got("objc_alloc_init"))
        .blr(16)
        .mov(1, 0)
        .ldr(0, 31, 16)
        .ldr_literal(16, sym.entry(Internal::AttachLayer as u16))
        .blr(16)
        .epilogue(32);
    a
}

fn layer_state(call: &mut Call) -> Result<u64, String> {
    call.kit.model.init_layer(call.receiver)?;
    Ok(call.receiver)
}
fn layer_init(call: &mut Call) -> Result<ReturnValues, String> {
    ret(layer_state(call)?)
}
/// +layer: an initialised instance (objc_alloc_init via tail dispatch).
fn layer_class_layer(call: &mut Call) -> Result<ReturnValues, String> {
    let entry = call.links()?.alloc_init;
    let selector = call.selector;
    call.frame.request_tail_dispatch(entry, selector)?;
    ret(0)
}
fn layer_frame(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    ret_rect(call.kit.model.layer(layer)?.frame())
}
fn layer_set_frame(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let frame = call.rect_arg(0)?;
    if !frame.is_finite() {
        return Err("CALayer frame is not finite".into());
    }
    let l = call.kit.model.layer_mut(layer)?;
    l.bounds.size = frame.size;
    l.position = frame.center();
    ret_void()
}
fn layer_bounds(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    ret_rect(call.kit.model.layer(layer)?.bounds)
}
fn layer_set_bounds(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let bounds = call.rect_arg(0)?;
    if !bounds.is_finite() {
        return Err("CALayer bounds is not finite".into());
    }
    call.kit.model.layer_mut(layer)?.bounds = bounds;
    ret_void()
}
fn layer_position(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let p = call.kit.model.layer(layer)?.position;
    ret_f64s(&[p.x, p.y])
}
fn layer_set_position(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let p = Point { x: call.f64_arg(0)?, y: call.f64_arg(1)? };
    call.kit.model.layer_mut(layer)?.position = p;
    ret_void()
}
fn layer_contents_scale(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    ret_f64s(&[call.kit.model.layer(layer)?.contents_scale])
}
fn layer_set_contents_scale(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let scale = call.f64_arg(0)?;
    if !(scale.is_finite() && scale > 0.0) {
        return Err("CALayer contentsScale invalid".into());
    }
    call.kit.model.layer_mut(layer)?.contents_scale = scale;
    ret_void()
}
fn layer_hidden(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    ret_bool(call.kit.model.layer(layer)?.hidden)
}
fn layer_set_hidden(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let value = call.bool_arg(2)?;
    call.kit.model.layer_mut(layer)?.hidden = value;
    ret_void()
}
fn layer_opaque(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    ret_bool(call.kit.model.layer(layer)?.opaque)
}
fn layer_set_opaque(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let value = call.bool_arg(2)?;
    call.kit.model.layer_mut(layer)?.opaque = value;
    ret_void()
}
fn layer_opacity(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    ret_f32(call.kit.model.layer(layer)?.opacity)
}
fn layer_set_opacity(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let value = call.f32_arg(0)?;
    call.kit.model.layer_mut(layer)?.opacity = value;
    ret_void()
}
fn layer_add_sublayer(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let child = call.arg(2)?;
    if child == 0 {
        return ret_void();
    }
    call.kit.model.init_layer(child)?;
    if call.kit.model.add_sublayer(layer, child)? {
        call.retain(child)?;
    }
    ret_void()
}
fn layer_remove_from_superlayer(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    if call.kit.model.remove_from_superlayer(layer)? != 0 {
        call.release(layer)?;
    }
    ret_void()
}
fn layer_superlayer(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    ret(call.kit.model.layer(layer)?.superlayer)
}
fn layer_delegate(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    ret(call.kit.model.layer(layer)?.delegate)
}
fn layer_set_delegate(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let delegate = call.arg(2)?;
    call.kit.model.layer_mut(layer)?.delegate = delegate;
    ret_void()
}
fn layer_anchor_point(_: &mut Call) -> Result<ReturnValues, String> {
    ret_f64s(&[0.5, 0.5])
}
pub(in crate::a64) const CALAYER: &[Method] = &[
    c("layer", "@16@0:8", layer_class_layer),
    i("init", "@16@0:8", layer_init),
    i("frame", "{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8", layer_frame),
    i("setFrame:", "v48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", layer_set_frame),
    i("bounds", "{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8", layer_bounds),
    i("setBounds:", "v48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16", layer_set_bounds),
    i("position", "{CGPoint=dd}16@0:8", layer_position),
    i("setPosition:", "v32@0:8{CGPoint=dd}16", layer_set_position),
    i("anchorPoint", "{CGPoint=dd}16@0:8", layer_anchor_point),
    i("contentsScale", "d16@0:8", layer_contents_scale),
    i("setContentsScale:", "v24@0:8d16", layer_set_contents_scale),
    i("isHidden", "B16@0:8", layer_hidden),
    i("setHidden:", "v20@0:8B16", layer_set_hidden),
    i("isOpaque", "B16@0:8", layer_opaque),
    i("setOpaque:", "v20@0:8B16", layer_set_opaque),
    i("opacity", "f16@0:8", layer_opacity),
    i("setOpacity:", "v20@0:8f16", layer_set_opacity),
    i("addSublayer:", "v24@0:8@16", layer_add_sublayer),
    i("removeFromSuperlayer", "v16@0:8", layer_remove_from_superlayer),
    i("superlayer", "@16@0:8", layer_superlayer),
    i("delegate", "@16@0:8", layer_delegate),
    i("setDelegate:", "v24@0:8@16", layer_set_delegate),
    i("setNeedsDisplay", "v16@0:8", no_op),
    i("setNeedsLayout", "v16@0:8", no_op),
    i("layoutSublayers", "v16@0:8", no_op),
    i("display", "v16@0:8", no_op),
    i("setMasksToBounds:", "v20@0:8B16", no_op),
    i("setContents:", "v24@0:8@16", no_op),
];

// ------------------------------------------------- MGLLayer (MetalANGLE)
fn mgl_layer_init(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    call.kit.mgl.ensure_layer(layer);
    ret(layer)
}
/// Drawable size = bounds × contentsScale (MGLLayer's non-Metal path).
pub(in crate::a64) fn drawable_size(call: &Call, layer: u64) -> Result<(f64, f64), String> {
    let l = call.kit.model.layer(layer)?;
    Ok((l.bounds.size.width * l.contents_scale, l.bounds.size.height * l.contents_scale))
}
fn mgl_layer_drawable_size(call: &mut Call) -> Result<ReturnValues, String> {
    let layer = layer_state(call)?;
    let (w, h) = drawable_size(call, layer)?;
    ret_f64s(&[w, h])
}
macro_rules! mgl_layer_format {
    ($get:ident, $set:ident, $field:ident) => {
        fn $get(call: &mut Call) -> Result<ReturnValues, String> {
            let layer = layer_state(call)?;
            ret(call.kit.mgl.ensure_layer(layer).$field as u64)
        }
        fn $set(call: &mut Call) -> Result<ReturnValues, String> {
            let layer = layer_state(call)?;
            let value = call.arg(2)? as i64;
            call.kit.mgl.ensure_layer(layer).$field = value;
            ret_void()
        }
    };
}
mgl_layer_format!(mgl_layer_color, mgl_layer_set_color, color);
mgl_layer_format!(mgl_layer_depth, mgl_layer_set_depth, depth);
mgl_layer_format!(mgl_layer_stencil, mgl_layer_set_stencil, stencil);
mgl_layer_format!(mgl_layer_multisample, mgl_layer_set_multisample, multisample);
pub(in crate::a64) const MGLLAYER: &[Method] = &[
    i("init", "@16@0:8", mgl_layer_init),
    i("drawableSize", "{CGSize=dd}16@0:8", mgl_layer_drawable_size),
    i("drawableColorFormat", "q16@0:8", mgl_layer_color),
    i("setDrawableColorFormat:", "v24@0:8q16", mgl_layer_set_color),
    i("drawableDepthFormat", "q16@0:8", mgl_layer_depth),
    i("setDrawableDepthFormat:", "v24@0:8q16", mgl_layer_set_depth),
    i("drawableStencilFormat", "q16@0:8", mgl_layer_stencil),
    i("setDrawableStencilFormat:", "v24@0:8q16", mgl_layer_set_stencil),
    i("drawableMultisample", "q16@0:8", mgl_layer_multisample),
    i("setDrawableMultisample:", "v24@0:8q16", mgl_layer_set_multisample),
];


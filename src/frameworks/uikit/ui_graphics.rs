/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIGraphics.h`

use crate::dyld::{export_c_func, FunctionExports};
use crate::frameworks::core_graphics::cg_context::{
    kCGBlendModeCopy, CGContextFillRect, CGContextRef, CGContextRelease, CGContextRestoreGState,
    CGContextRetain, CGContextSaveGState, CGContextSetBlendMode,
};
use crate::frameworks::core_graphics::cg_bitmap_context::{CGBitmapContextCreate, CGBitmapContextCreateImage};
use crate::frameworks::core_graphics::cg_color_space::{CGColorSpaceCreateDeviceRGB, CGColorSpaceRelease};
use crate::frameworks::core_graphics::cg_context::{CGContextScaleCTM, CGContextTranslateCTM};
use crate::frameworks::core_graphics::cg_image::{kCGImageAlphaPremultipliedLast, CGImageRelease};
use crate::frameworks::core_graphics::{CGRect, CGSize};
use crate::mem::GuestUSize;
use crate::objc::{id, msg_class};
use crate::objc::nil;
use crate::Environment;

#[derive(Default)]
pub(super) struct State {
    pub(super) context_stack: Vec<CGContextRef>,
}

pub fn UIGraphicsPushContext(env: &mut Environment, context: CGContextRef) {
    CGContextRetain(env, context);
    env.framework_state
        .uikit
        .ui_graphics
        .context_stack
        .push(context);
}
pub fn UIGraphicsPopContext(env: &mut Environment) {
    let context = env.framework_state.uikit.ui_graphics.context_stack.pop();
    CGContextRelease(env, context.unwrap());
}
pub fn UIGraphicsGetCurrentContext(env: &mut Environment) -> CGContextRef {
    env.framework_state
        .uikit
        .ui_graphics
        .context_stack
        .last()
        .copied()
        .unwrap_or(nil)
}

/// Offscreen drawing for `UIGraphicsGetImageFromCurrentImageContext()`: a
/// premultiplied RGBA bitmap, flipped so (0, 0) is the top-left like UIKit.
fn UIGraphicsBeginImageContextWithOptions(env: &mut Environment, size: CGSize, _opaque: bool, scale: f32) {
    let scale = if scale <= 0.0 { 1.0 } else { scale };
    let width = ((size.width * scale).ceil().max(1.0)) as GuestUSize;
    let height = ((size.height * scale).ceil().max(1.0)) as GuestUSize;
    let color_space = CGColorSpaceCreateDeviceRGB(env);
    let context = CGBitmapContextCreate(
        env,
        crate::mem::Ptr::null(),
        width,
        height,
        8,
        0,
        color_space,
        kCGImageAlphaPremultipliedLast,
    );
    CGColorSpaceRelease(env, color_space);
    CGContextTranslateCTM(env, context, 0.0, height as f32);
    CGContextScaleCTM(env, context, scale, -scale);
    UIGraphicsPushContext(env, context);
    CGContextRelease(env, context); // the stack holds its own reference
}

fn UIGraphicsBeginImageContext(env: &mut Environment, size: CGSize) {
    UIGraphicsBeginImageContextWithOptions(env, size, false, 1.0)
}

fn UIGraphicsGetImageFromCurrentImageContext(env: &mut Environment) -> id {
    let context = UIGraphicsGetCurrentContext(env);
    if context == nil {
        return nil;
    }
    let cg_image = CGBitmapContextCreateImage(env, context);
    let image: id = msg_class![env; UIImage imageWithCGImage:cg_image];
    CGImageRelease(env, cg_image);
    image
}

fn UIGraphicsEndImageContext(env: &mut Environment) {
    if !env.framework_state.uikit.ui_graphics.context_stack.is_empty() {
        UIGraphicsPopContext(env);
    }
}

fn UIRectFill(env: &mut Environment, rect: CGRect) {
    let context = UIGraphicsGetCurrentContext(env);
    if context != nil {
        CGContextSaveGState(env, context);
        CGContextSetBlendMode(env, context, kCGBlendModeCopy);
        CGContextFillRect(env, context, rect);
        CGContextRestoreGState(env, context);
    }
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(UIGraphicsPushContext(_)),
    export_c_func!(UIGraphicsPopContext()),
    export_c_func!(UIGraphicsGetCurrentContext()),
    export_c_func!(UIRectFill(_)),
    export_c_func!(UIGraphicsBeginImageContext(_)),
    export_c_func!(UIGraphicsBeginImageContextWithOptions(_, _, _)),
    export_c_func!(UIGraphicsGetImageFromCurrentImageContext()),
    export_c_func!(UIGraphicsEndImageContext()),
];

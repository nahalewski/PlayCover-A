/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! The Core Text framework.
//!
//! Only the small font-metrics and glyph-lookup subset that games with their
//! own text renderer use (a `CGFont` plus `CTFont` metrics, then glyphs drawn
//! with `CGContextShowGlyphsAtPoint()`). There is no text layout here.

use crate::dyld::{export_c_func, FunctionExports, HostDylib};
use crate::frameworks::core_foundation::{CFRelease, CFRetain, CFTypeRef};
use crate::frameworks::core_graphics::cg_font::{CGFontHostObject, CGFontRef};
use crate::frameworks::core_graphics::{CGFloat, CGSize};
use crate::frameworks::foundation::ns_string::from_rust_string;
use crate::frameworks::foundation::{unichar, NSRange};
use crate::mem::{ConstPtr, GuestUSize, MutPtr};
use crate::objc::{id, nil, objc_classes, ClassExports, HostObject};
use crate::Environment;

pub const DYLIB: HostDylib = HostDylib {
    path: "/System/Library/Frameworks/CoreText.framework/CoreText",
    aliases: &[],
    class_exports: &[CLASSES],
    constant_exports: &[],
    function_exports: &[FUNCTIONS],
};

type CTFontRef = CFTypeRef;

struct CTFontHostObject {
    /// Retained `CGFontRef`.
    cg_font: CGFontRef,
    size: CGFloat,
}
impl HostObject for CTFontHostObject {}

const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation _touchHLE_CTFont: NSObject

- (())dealloc {
    let cg_font = env.objc.borrow::<CTFontHostObject>(this).cg_font;
    CFRelease(env, cg_font);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

};

fn new_ct_font(env: &mut Environment, cg_font: CGFontRef, size: CGFloat) -> CTFontRef {
    CFRetain(env, cg_font);
    let class = env.objc.get_known_class("_touchHLE_CTFont", &mut env.mem);
    env.objc.alloc_object(
        class,
        Box::new(CTFontHostObject { cg_font, size }),
        &mut env.mem,
    )
}

fn CTFontCreateWithGraphicsFont(
    env: &mut Environment,
    graphics_font: CGFontRef,
    size: CGFloat,
    _matrix: ConstPtr<u8>, // const CGAffineTransform*, unsupported
    _attributes: id,       // CTFontDescriptorRef, unsupported
) -> CTFontRef {
    if graphics_font.is_null() {
        return nil;
    }
    let size = if size == 0.0 { 12.0 } else { size };
    new_ct_font(env, graphics_font, size)
}

fn font_and_size(env: &Environment, font: CTFontRef) -> (&crate::font::Font, CGFloat) {
    let host_object = env.objc.borrow::<CTFontHostObject>(font);
    let cg_font = env.objc.borrow::<CGFontHostObject>(host_object.cg_font);
    (&cg_font.font, host_object.size)
}

fn CTFontGetSize(env: &mut Environment, font: CTFontRef) -> CGFloat {
    env.objc.borrow::<CTFontHostObject>(font).size
}
fn CTFontGetAscent(env: &mut Environment, font: CTFontRef) -> CGFloat {
    let (f, size) = font_and_size(env, font);
    f.ascent(size)
}
fn CTFontGetDescent(env: &mut Environment, font: CTFontRef) -> CGFloat {
    let (f, size) = font_and_size(env, font);
    // CoreText reports descent as a positive distance.
    -f.descent(size)
}
fn CTFontGetLeading(env: &mut Environment, font: CTFontRef) -> CGFloat {
    let (f, size) = font_and_size(env, font);
    f.line_gap(size)
}
fn CTFontGetCapHeight(env: &mut Environment, font: CTFontRef) -> CGFloat {
    // TODO: read it from the OS/2 table
    env.objc.borrow::<CTFontHostObject>(font).size * 0.7
}
fn CTFontGetXHeight(env: &mut Environment, font: CTFontRef) -> CGFloat {
    // TODO: read it from the OS/2 table
    env.objc.borrow::<CTFontHostObject>(font).size * 0.5
}
fn CTFontGetUnderlinePosition(env: &mut Environment, font: CTFontRef) -> CGFloat {
    -env.objc.borrow::<CTFontHostObject>(font).size * 0.1
}

fn CTFontGetGlyphsForCharacters(
    env: &mut Environment,
    font: CTFontRef,
    characters: ConstPtr<unichar>,
    glyphs: MutPtr<u16>,
    count: i32,
) -> bool {
    let mut all_found = true;
    for i in 0..count as GuestUSize {
        let c: unichar = env.mem.read(characters + i);
        // Lone surrogates have no glyph.
        let glyph = if (0xD800..0xE000).contains(&c) {
            all_found = false;
            0
        } else {
            let (f, _) = font_and_size(env, font);
            f.glyph_id_for_char(c).0
        };
        env.mem.write(glyphs + i, glyph);
    }
    all_found
}

fn CTFontGetAdvancesForGlyphs(
    env: &mut Environment,
    font: CTFontRef,
    _orientation: u32,
    glyphs: ConstPtr<u16>,
    advances: MutPtr<CGSize>,
    count: i32,
) -> f64 {
    let mut total = 0.0f64;
    for i in 0..count as GuestUSize {
        let glyph = env.mem.read(glyphs + i);
        let (f, size) = font_and_size(env, font);
        let advance = f
            .glyph_hor_advance(glyph)
            .map(|a| f32::from(a) / f32::from(f.units_per_em()) * size)
            .unwrap_or(0.0);
        total += f64::from(advance);
        if !advances.is_null() {
            env.mem.write(
                advances + i,
                CGSize {
                    width: advance,
                    height: 0.0,
                },
            );
        }
    }
    total
}

/// "Create" function: the caller owns the result.
fn CTFontCreateForString(
    env: &mut Environment,
    font: CTFontRef,
    _string: id,
    _range: NSRange,
) -> CTFontRef {
    // No font fallback: the same font is used for every string.
    CFRetain(env, font)
}

fn CTFontCopyDisplayName(env: &mut Environment, font: CTFontRef) -> id {
    let cg_font = env.objc.borrow::<CTFontHostObject>(font).cg_font;
    let name = env.objc.borrow::<CGFontHostObject>(cg_font).name.clone();
    from_rust_string(env, name)
}

const FUNCTIONS: FunctionExports = &[
    export_c_func!(CTFontCreateWithGraphicsFont(_, _, _, _)),
    export_c_func!(CTFontGetSize(_)),
    export_c_func!(CTFontGetAscent(_)),
    export_c_func!(CTFontGetDescent(_)),
    export_c_func!(CTFontGetLeading(_)),
    export_c_func!(CTFontGetCapHeight(_)),
    export_c_func!(CTFontGetXHeight(_)),
    export_c_func!(CTFontGetUnderlinePosition(_)),
    export_c_func!(CTFontGetGlyphsForCharacters(_, _, _, _)),
    export_c_func!(CTFontGetAdvancesForGlyphs(_, _, _, _, _)),
    export_c_func!(CTFontCreateForString(_, _, _)),
    export_c_func!(CTFontCopyDisplayName(_)),
];

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `CGImage.h`

use super::cg_color_space::{
    kCGColorSpaceGenericRGB, kCGColorSpaceModelRGB, CGColorSpaceCreateDeviceGray,
    CGColorSpaceCreateWithName, CGColorSpaceGetModel, CGColorSpaceRef,
};
use super::cg_data_provider::{self, CGDataProviderRef};
use super::cg_geometry::{CGPointZero, CGRectIntegral, CGRectIntersection, CGRectNull};
use super::{CGFloat, CGRect, CGSize};
use crate::dyld::{export_c_func, FunctionExports};
use crate::frameworks::core_foundation::{CFRelease, CFRetain, CFTypeRef};
use crate::frameworks::foundation::ns_string;
use crate::image::Image;
use crate::mem::{ConstPtr, GuestUSize};
use crate::objc::{autorelease, nil, objc_classes, ClassExports, HostObject, ObjC};
use crate::Environment;

pub type CGImageAlphaInfo = u32;
pub const kCGImageAlphaNone: CGImageAlphaInfo = 0;
pub const kCGImageAlphaPremultipliedLast: CGImageAlphaInfo = 1;
pub const kCGImageAlphaPremultipliedFirst: CGImageAlphaInfo = 2;
pub const kCGImageAlphaLast: CGImageAlphaInfo = 3;
pub const kCGImageAlphaFirst: CGImageAlphaInfo = 4;
pub const kCGImageAlphaNoneSkipLast: CGImageAlphaInfo = 5;
pub const kCGImageAlphaNoneSkipFirst: CGImageAlphaInfo = 6;
pub const kCGImageAlphaOnly: CGImageAlphaInfo = 7;

pub type CGImageByteOrderInfo = u32;
pub const kCGImageByteOrderMask: CGImageByteOrderInfo = 0x7000;
pub const kCGImageByteOrderDefault: CGImageByteOrderInfo = 0 << 12;
#[allow(dead_code)]
pub const kCGImageByteOrder16Little: CGImageByteOrderInfo = 1 << 12;
pub const kCGImageByteOrder32Little: CGImageByteOrderInfo = 2 << 12;
#[allow(dead_code)]
pub const kCGImageByteOrder16Big: CGImageByteOrderInfo = 3 << 12;
pub const kCGImageByteOrder32Big: CGImageByteOrderInfo = 4 << 12;

pub type CGBitmapInfo = u32;
pub const kCGBitmapAlphaInfoMask: CGBitmapInfo = 0x1F; // huh, it's not 0x7?
pub const kCGBitmapByteOrderMask: CGBitmapInfo = kCGImageByteOrderMask;
// TODO: other stuff in this enum (for now, always assert the rest is 0)

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

// CGImage seems to be a CFType-based type, but in our implementation those
// are just Objective-C types, so we need a class for it, but its name is not
// visible anywhere.
@implementation _touchHLE_CGImage: NSObject
@end

};

/// Where an image's pixels came from, when that is memory the app may keep
/// changing (a `CGDataProvider` over a buffer). Real Quartz reads such memory
/// when the image is drawn, so apps (e.g. Sonic 1's emulator) create the image
/// once and then rewrite the buffer every frame.
#[derive(Copy, Clone)]
struct LiveSource {
    provider: CGDataProviderRef,
    width: GuestUSize,
    height: GuestUSize,
    bits_per_component: GuestUSize,
    bits_per_pixel: GuestUSize,
    bytes_per_row: GuestUSize,
    bitmap_info: CGBitmapInfo,
}

struct CGImageHostObject {
    image: Image,
    live: Option<LiveSource>,
}
impl HostObject for CGImageHostObject {}

pub type CGImageRef = CFTypeRef;
pub fn CGImageRelease(env: &mut Environment, c: CGImageRef) {
    if !c.is_null() {
        CFRelease(env, c);
    }
}
pub fn CGImageRetain(env: &mut Environment, c: CGImageRef) -> CGImageRef {
    if !c.is_null() {
        CFRetain(env, c)
    } else {
        c
    }
}

/// CGImages are immutable, so a copy can share the original, retained.
pub fn CGImageCreateCopy(env: &mut Environment, c: CGImageRef) -> CGImageRef {
    if c.is_null() {
        return c;
    }
    let live = env.objc.borrow::<CGImageHostObject>(c).live;
    let Some(source) = live else {
        // Immutable pixels: a copy can share the original.
        return CGImageRetain(env, c);
    };
    // Re-read the app's buffer so the copy shows what it holds now.
    let pixels = live_pixels(env, &source);
    let image = Image::from_pixel_vec(pixels, (source.width, source.height));
    let host_obj = Box::new(CGImageHostObject { image, live: Some(source) });
    let class = env.objc.get_known_class("_touchHLE_CGImage", &mut env.mem);
    CFRetain(env, source.provider);
    env.objc.alloc_object(class, host_obj, &mut env.mem)
}

fn live_pixels(env: &mut Environment, source: &LiveSource) -> Vec<u8> {
    let bytes = cg_data_provider::borrow_bytes(env, source.provider);
    match (source.bits_per_component, source.bits_per_pixel) {
        (5, 16) => rgb555_pixels_to_rgba(bytes, source.width, source.height, source.bytes_per_row, source.bitmap_info),
        (8, 32) => rgb_pixels_to_rgba(bytes, source.width, source.height, source.bytes_per_row, source.bitmap_info),
        _ => unimplemented!(
            "CGImageCreate component depth {}, pixel depth {}",
            source.bits_per_component,
            source.bits_per_pixel
        ),
    }
}

/// Shortcut for use by `UIImage`: directly construct a `CGImage` instance from
/// an [Image] instance.
pub fn from_image(env: &mut Environment, image: Image) -> CGImageRef {
    let host_obj = Box::new(CGImageHostObject { image, live: None });
    let class = env.objc.get_known_class("_touchHLE_CGImage", &mut env.mem);
    env.objc.alloc_object(class, host_obj, &mut env.mem)
}

/// Shortcut for use by `CGBitmapContext` etc: borrow the [Image] from a
/// `CGImage` instance.
pub fn borrow_image(objc: &ObjC, image: CGImageRef) -> &Image {
    &objc.borrow::<CGImageHostObject>(image).image
}

/// Shortcut used by the app picker, counterpart to [borrow_image].
/// FIXME: This should not exist!
pub fn borrow_image_mut(objc: &mut ObjC, image: CGImageRef) -> &mut Image {
    &mut objc.borrow_mut::<CGImageHostObject>(image).image
}

// TODO: More create methods.

/// Component locations in the guest's byte stream. Little-endian reverses
/// the four bytes of the logical ARGB/RGBA word, not just its RGB channels.
pub(super) fn rgb_pixel_offsets(info: CGBitmapInfo) -> (usize, usize, usize, Option<usize>) {
    let alpha = info & kCGBitmapAlphaInfoMask;
    let order = info & kCGBitmapByteOrderMask;
    assert_eq!(info, alpha | order);
    assert!(matches!(
        order,
        kCGImageByteOrderDefault | kCGImageByteOrder32Big | kCGImageByteOrder32Little
    ));
    let offsets = match alpha {
        kCGImageAlphaNone | kCGImageAlphaNoneSkipLast => (0, 1, 2, None),
        kCGImageAlphaPremultipliedLast | kCGImageAlphaLast => (0, 1, 2, Some(3)),
        kCGImageAlphaPremultipliedFirst | kCGImageAlphaFirst => (1, 2, 3, Some(0)),
        kCGImageAlphaNoneSkipFirst => (1, 2, 3, None),
        kCGImageAlphaOnly => (0, 0, 0, Some(0)),
        _ => panic!("unsupported RGB alpha info {alpha}"),
    };
    if order == kCGImageByteOrder32Little {
        assert!(!matches!(alpha, kCGImageAlphaNone | kCGImageAlphaOnly));
        (
            3 - offsets.0,
            3 - offsets.1,
            3 - offsets.2,
            offsets.3.map(|a| 3 - a),
        )
    } else {
        offsets
    }
}

pub(super) fn rgb_pixels_to_rgba(
    bytes: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    info: CGBitmapInfo,
) -> Vec<u8> {
    let alpha = info & kCGBitmapAlphaInfoMask;
    let bpp = match alpha {
        kCGImageAlphaNone => 3,
        kCGImageAlphaOnly => 1,
        _ => 4,
    };
    let offsets = rgb_pixel_offsets(info);
    assert!(u64::from(stride) >= u64::from(width) * bpp);
    assert!(bytes.len() as u64 >= u64::from(stride) * u64::from(height));
    let mut out =
        Vec::with_capacity(usize::try_from(u64::from(width) * u64::from(height) * 4).unwrap());
    for y in 0..height {
        for x in 0..width {
            let start =
                usize::try_from(u64::from(y) * u64::from(stride) + u64::from(x) * bpp).unwrap();
            let p = &bytes[start..];
            let a = offsets.3.map_or(255, |i| p[i]);
            let channel = |i| {
                if matches!(alpha, kCGImageAlphaFirst | kCGImageAlphaLast) {
                    ((u16::from(p[i]) * u16::from(a) + 127) / 255) as u8
                } else if alpha == kCGImageAlphaOnly {
                    0
                } else {
                    p[i]
                }
            };
            out.extend_from_slice(&[
                channel(offsets.0),
                channel(offsets.1),
                channel(offsets.2),
                a,
            ]);
        }
    }
    out
}

#[test]
fn little_endian_bitmap_conversion_preserves_alpha_and_row_padding() {
    let bgra = kCGImageByteOrder32Little | kCGImageAlphaPremultipliedFirst;
    assert_eq!(rgb_pixel_offsets(bgra), (2, 1, 0, Some(3)));
    assert_eq!(
        rgb_pixels_to_rgba(
            &[10, 20, 40, 80, 99, 99, 99, 99, 30, 60, 90, 120, 99, 99, 99, 99],
            1,
            2,
            8,
            bgra
        ),
        [40, 20, 10, 80, 90, 60, 30, 120]
    );
    let bgrx = kCGImageByteOrder32Little | kCGImageAlphaNoneSkipFirst;
    assert_eq!(
        rgb_pixels_to_rgba(&[10, 20, 40, 0], 1, 1, 4, bgrx),
        [40, 20, 10, 255]
    );
    let abgr = kCGImageByteOrder32Little | kCGImageAlphaLast;
    assert_eq!(
        rgb_pixels_to_rgba(&[128, 20, 40, 80], 1, 1, 4, abgr),
        [40, 20, 10, 128]
    );
}

/// Quartz's 5-bit RGB format is XRGB1555, with the high bit skipped.
/// It is distinct from RGB565: the green channel also has five bits.
fn rgb555_pixels_to_rgba(
    bytes: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    info: CGBitmapInfo,
) -> Vec<u8> {
    let order = info & kCGBitmapByteOrderMask;
    assert_eq!(info & !kCGBitmapByteOrderMask, kCGImageAlphaNoneSkipFirst);
    assert!(matches!(
        order,
        kCGImageByteOrder16Little | kCGImageByteOrder16Big | kCGImageByteOrderDefault
    ));
    assert!(u64::from(stride) >= u64::from(width) * 2);
    assert!(bytes.len() as u64 >= u64::from(stride) * u64::from(height));
    let mut out =
        Vec::with_capacity(usize::try_from(u64::from(width) * u64::from(height) * 4).unwrap());
    let expand = |v: u16| ((v << 3) | (v >> 2)) as u8;
    for y in 0..height {
        for x in 0..width {
            let offset =
                usize::try_from(u64::from(y) * u64::from(stride) + u64::from(x) * 2).unwrap();
            let pair = [bytes[offset], bytes[offset + 1]];
            let pixel = if order == kCGImageByteOrder16Little {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            };
            out.extend_from_slice(&[
                expand((pixel >> 10) & 31),
                expand((pixel >> 5) & 31),
                expand(pixel & 31),
                255,
            ]);
        }
    }
    out
}

#[test]
fn rgb555_framebuffer_decodes_colour_skip_bit_and_stride() {
    let info = kCGImageByteOrder16Little | kCGImageAlphaNoneSkipFirst;
    assert_eq!(
        rgb555_pixels_to_rgba(
            &[0, 0x7c, 0xe0, 3, 0xaa, 0xaa, 0x1f, 0x80, 0xff, 0xff, 0xbb, 0xbb],
            2,
            2,
            6,
            info
        ),
        [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]
    );
    assert_eq!(
        rgb555_pixels_to_rgba(
            &[0x42, 0x10],
            1,
            1,
            2,
            kCGImageByteOrder16Big | kCGImageAlphaNoneSkipFirst
        ),
        [132, 132, 132, 255]
    );
}

fn CGImageCreate(
    env: &mut Environment,
    width: GuestUSize,
    height: GuestUSize,
    bits_per_component: GuestUSize,
    bits_per_pixel: GuestUSize,
    bytes_per_row: GuestUSize,
    colorspace: CGColorSpaceRef,
    bitmap_info: CGBitmapInfo,
    provider: CGDataProviderRef,
    decode: ConstPtr<CGFloat>,
    _should_interpolate: bool, // TODO
    _intent: i32,              // TODO (should be CGColorRenderingIntent)
) -> CGImageRef {
    log_dbg!(
        "CGImageCreate w {}, h {}, bpc {}, bpp {}, bpr {}, bi {}",
        width,
        height,
        bits_per_component,
        bits_per_pixel,
        bytes_per_row,
        bitmap_info
    );
    assert!(decode.is_null()); // TODO
    assert_eq!(CGColorSpaceGetModel(env, colorspace), kCGColorSpaceModelRGB);
    let source = LiveSource {
        provider,
        width,
        height,
        bits_per_component,
        bits_per_pixel,
        bytes_per_row,
        bitmap_info,
    };
    let pixels = live_pixels(env, &source);
    let image = Image::from_pixel_vec(pixels, (width, height));
    let live = if cg_data_provider::is_guest_memory(env, provider) {
        // Keep the provider alive: the app may release it right after this call.
        CFRetain(env, provider);
        Some(source)
    } else {
        None
    };
    let host_obj = Box::new(CGImageHostObject { image, live });
    let class = env.objc.get_known_class("_touchHLE_CGImage", &mut env.mem);
    env.objc.alloc_object(class, host_obj, &mut env.mem)
}

fn CGImageCreateCopyWithColorSpace(
    env: &mut Environment,
    image: CGImageRef,
    color_space: CGColorSpaceRef,
) -> CGImageRef {
    let image_color_space = CGImageGetColorSpace(env, image);
    let source_model = CGColorSpaceGetModel(env, image_color_space);
    let target_model = CGColorSpaceGetModel(env, color_space);
    // Grayscale sources are stored as RGBA already, so converting to RGB is
    // just a relabel.
    let gray_to_rgb = borrow_image(&env.objc, image).source_is_gray()
        && target_model == kCGColorSpaceModelRGB;
    if !gray_to_rgb {
        assert_eq!(source_model, target_model);
    }
    // If color space matches, we could just create a copy.
    let mut new_image = env.objc.borrow::<CGImageHostObject>(image).image.clone();
    if gray_to_rgb {
        new_image.mark_source_rgba();
    }
    from_image(env, new_image)
}

fn CGImageCreateWithPNGDataProvider(
    env: &mut Environment,
    source: CGDataProviderRef,
    decode: ConstPtr<CGFloat>,
    _should_interpolate: bool, // TODO
    _intent: i32,              // TODO (should be CGColorRenderingIntent)
) -> CGImageRef {
    assert!(decode.is_null()); // TODO

    let bytes = cg_data_provider::borrow_bytes(env, source);
    let Ok(image) = Image::from_bytes(bytes) else {
        // Docs don't say what happens on failure, but this would make sense.
        return nil;
    };

    from_image(env, image)
}

fn CGImageCreateWithJPEGDataProvider(
    env: &mut Environment,
    source: CGDataProviderRef,
    decode: ConstPtr<CGFloat>,
    _should_interpolate: bool, // TODO
    _intent: i32,              // TODO (should be CGColorRenderingIntent)
) -> CGImageRef {
    assert!(decode.is_null());

    let bytes = cg_data_provider::borrow_bytes(env, source);
    let Ok(image) = Image::from_bytes(bytes) else {
        // Docs don't say what happens on failure, but this would make sense.
        return nil;
    };

    from_image(env, image)
}

fn CGImageGetAlphaInfo(env: &mut Environment, image: CGImageRef) -> CGImageAlphaInfo {
    // A grayscale file without alpha (e.g. a bitmap font atlas) has no alpha
    // on iOS either; apps use that to pick a luminance/alpha texture path.
    let image = borrow_image(&env.objc, image);
    if image.source_is_gray() && !image.source_has_alpha() {
        return kCGImageAlphaNone;
    }
    // our Image type always returns premultiplied RGBA
    // (the premultiplied part must match what the real UIImage does, but
    // considering CgBI's design, maybe the order doesn't?)
    kCGImageAlphaPremultipliedLast
}

fn CGImageGetColorSpace(env: &mut Environment, image: CGImageRef) -> CGColorSpaceRef {
    // Caller must release
    // FIXME: what if a loaded image is not sRGB?

    // Report grayscale source files as such (pixels are still stored as RGBA
    // internally, with R = G = B = the gray value).
    if borrow_image(&env.objc, image).source_is_gray() {
        return CGColorSpaceCreateDeviceGray(env);
    }

    let srgb_name = ns_string::get_static_str(env, kCGColorSpaceGenericRGB);
    CGColorSpaceCreateWithName(env, srgb_name)
}

pub fn CGImageGetWidth(env: &mut Environment, image: CGImageRef) -> GuestUSize {
    let (width, _height) = env
        .objc
        .borrow::<CGImageHostObject>(image)
        .image
        .dimensions();
    width
}
pub fn CGImageGetHeight(env: &mut Environment, image: CGImageRef) -> GuestUSize {
    let (_width, height) = env
        .objc
        .borrow::<CGImageHostObject>(image)
        .image
        .dimensions();
    height
}
fn CGImageGetBitsPerPixel(_env: &mut Environment, _image: CGImageRef) -> GuestUSize {
    32
}
fn CGImageGetBytesPerRow(env: &mut Environment, image: CGImageRef) -> GuestUSize {
    let (width, _height) = env
        .objc
        .borrow::<CGImageHostObject>(image)
        .image
        .dimensions();
    width * 4
}

fn CGImageGetDataProvider(env: &mut Environment, image: CGImageRef) -> CGDataProviderRef {
    // CGImageGetDataProvider() seems to be intended to return the underlying
    // data provider that is retained by the CGImage. That's not how CGImage is
    // implemented here though, so instead we make a data provider that
    // retains the CGImage: exactly the opposite approach!
    let cg_data_provider = cg_data_provider::from_cg_image(env, image);
    // CGImageGetDataProvider() isn't meant to return a new object, so the
    // caller won't free this. The CGImage can't retain the CGDataProvider
    // without causing a cycle, so let's autorelease it instead.
    autorelease(env, cg_data_provider)
}

fn CGImageGetBitsPerComponent(_: &mut Environment, _: CGImageRef) -> GuestUSize {
    8 // Fix this when we support anything else
}

fn CGImageCreateWithImageInRect(
    env: &mut Environment,
    image: CGImageRef,
    rect: CGRect,
) -> CGImageRef {
    let (img_w, img_h) = borrow_image(&env.objc, image).dimensions();
    log_dbg!(
        "CGImageCreateWithImageInRect: rect {:?}, img dim {:?}",
        rect,
        (img_w, img_h)
    );
    let rect = CGRectIntegral(env, rect);
    let intersect = CGRectIntersection(
        env,
        rect,
        CGRect {
            origin: CGPointZero,
            size: CGSize {
                width: img_w as CGFloat,
                height: img_h as CGFloat,
            },
        },
    );
    assert!(intersect != CGRectNull);
    let (x, y, w, h) = (
        intersect.origin.x as u32,
        intersect.origin.y as u32,
        intersect.size.width as u32,
        intersect.size.height as u32,
    );
    assert_eq!(32, CGImageGetBitsPerPixel(env, image));
    let mut new_pixels = Vec::with_capacity((w * h * 4) as usize);
    let old_pixels = borrow_image(&env.objc, image).pixels();
    for i in 0..h {
        new_pixels.extend_from_slice(
            &old_pixels[((y + i) * img_w * 4 + x * 4) as usize..][..(w * 4) as usize],
        );
    }
    // Note: instead of keeping reference to the orig image,
    // we're creating a copy here
    let new_img = Image::from_pixel_vec(new_pixels, (w, h));
    from_image(env, new_img)
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CGImageRelease(_)),
    export_c_func!(CGImageRetain(_)),
    export_c_func!(CGImageCreateCopy(_)),
    export_c_func!(CGImageCreate(_, _, _, _, _, _, _, _, _, _, _)),
    export_c_func!(CGImageCreateCopyWithColorSpace(_, _)),
    export_c_func!(CGImageCreateWithPNGDataProvider(_, _, _, _)),
    export_c_func!(CGImageCreateWithJPEGDataProvider(_, _, _, _)),
    export_c_func!(CGImageGetAlphaInfo(_)),
    export_c_func!(CGImageGetColorSpace(_)),
    export_c_func!(CGImageGetWidth(_)),
    export_c_func!(CGImageGetHeight(_)),
    export_c_func!(CGImageGetBitsPerPixel(_)),
    export_c_func!(CGImageGetBytesPerRow(_)),
    export_c_func!(CGImageGetDataProvider(_)),
    export_c_func!(CGImageGetBitsPerComponent(_)),
    export_c_func!(CGImageCreateWithImageInRect(_, _)),
];

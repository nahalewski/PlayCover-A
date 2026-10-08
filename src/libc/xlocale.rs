/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `xlocale.h`: `locale_t` and the `*_l` variants of C library functions.
//!
//! libc++ calls these everywhere. Only the "C" locale (with UTF-8 multibyte
//! conversion) exists here, so a `locale_t` is just a token and each `*_l`
//! function ignores it and does what its plain counterpart does.

use super::stdio::printf::{sscanf_common, vasprintf, vsnprintf};
use super::string::strcmp;
use super::time::{strftime, tm};
use crate::abi::{DotDotDot, VaList};
use crate::dyld::{export_c_func, FunctionExports};
use crate::mem::{ConstPtr, GuestUSize, MutPtr, MutVoidPtr, Ptr};
use crate::Environment;

type LocaleT = MutVoidPtr;
/// `wchar_t` is 32-bit on Darwin.
type WcharT = i32;

const LC_GLOBAL_LOCALE: u32 = u32::MAX;

#[derive(Default)]
pub struct State {
    current: Option<LocaleT>,
}

fn newlocale(env: &mut Environment, _mask: i32, _name: ConstPtr<u8>, _base: LocaleT) -> LocaleT {
    // Every locale name is accepted and behaves like "C".
    let token = env.mem.alloc(8);
    env.mem.bytes_at_mut(token.cast::<u8>(), 8).fill(0);
    token
}

fn freelocale(env: &mut Environment, locale: LocaleT) -> i32 {
    if !locale.is_null() && locale.to_bits() != LC_GLOBAL_LOCALE {
        env.mem.free(locale);
    }
    0
}

fn duplocale(env: &mut Environment, _locale: LocaleT) -> LocaleT {
    newlocale(env, 0, Ptr::null(), Ptr::null())
}

fn uselocale(env: &mut Environment, locale: LocaleT) -> LocaleT {
    let previous = env
        .libc_state
        .xlocale
        .current
        .unwrap_or(Ptr::from_bits(LC_GLOBAL_LOCALE));
    if !locale.is_null() {
        env.libc_state.xlocale.current = Some(locale);
    }
    previous
}

// -- ctype --------------------------------------------------------------

fn ___mb_cur_max_l(_env: &mut Environment, _locale: LocaleT) -> i32 {
    1
}

fn ___tolower_l(_env: &mut Environment, c: i32, _locale: LocaleT) -> i32 {
    if (0..256).contains(&c) {
        i32::from((c as u8).to_ascii_lowercase())
    } else {
        c
    }
}

fn ___toupper_l(_env: &mut Environment, c: i32, _locale: LocaleT) -> i32 {
    if (0..256).contains(&c) {
        i32::from((c as u8).to_ascii_uppercase())
    } else {
        c
    }
}

fn ___maskrune_l(env: &mut Environment, rune: i32, mask: u32, _locale: LocaleT) -> i32 {
    super::ctype::__maskrune(env, rune, mask)
}

// -- formatted I/O -------------------------------------------------------

fn snprintf_l(
    env: &mut Environment,
    dest: MutPtr<u8>,
    n: GuestUSize,
    _locale: LocaleT,
    format: ConstPtr<u8>,
    args: DotDotDot,
) -> i32 {
    vsnprintf(env, dest, n, format, args.start())
}

fn asprintf_l(
    env: &mut Environment,
    ret: MutPtr<MutPtr<u8>>,
    _locale: LocaleT,
    format: ConstPtr<u8>,
    args: DotDotDot,
) -> i32 {
    let va: VaList = args.start();
    vasprintf(env, ret, format, va)
}

fn sscanf_l(
    env: &mut Environment,
    src: ConstPtr<u8>,
    _locale: LocaleT,
    format: ConstPtr<u8>,
    args: DotDotDot,
) -> i32 {
    sscanf_common(env, src, format, args.start())
}

// -- string conversion and comparison ------------------------------------

fn strcoll_l(env: &mut Environment, a: ConstPtr<u8>, b: ConstPtr<u8>, _locale: LocaleT) -> i32 {
    strcmp(env, a, b)
}

fn strxfrm_l(
    env: &mut Environment,
    dst: MutPtr<u8>,
    src: ConstPtr<u8>,
    n: GuestUSize,
    _locale: LocaleT,
) -> GuestUSize {
    let bytes = env.mem.cstr_at(src).to_vec();
    let len = bytes.len() as GuestUSize;
    if n > 0 && !dst.is_null() {
        let copy = len.min(n - 1);
        env.mem
            .bytes_at_mut(dst, copy)
            .copy_from_slice(&bytes[..copy as usize]);
        env.mem.write(dst + copy, 0u8);
    }
    len
}

fn wcscoll_l(
    env: &mut Environment,
    a: ConstPtr<WcharT>,
    b: ConstPtr<WcharT>,
    _locale: LocaleT,
) -> i32 {
    let mut i: GuestUSize = 0;
    loop {
        let x: WcharT = env.mem.read(a + i);
        let y: WcharT = env.mem.read(b + i);
        if x != y {
            return if x < y { -1 } else { 1 };
        }
        if x == 0 {
            return 0;
        }
        i += 1;
    }
}

fn strftime_l(
    env: &mut Environment,
    s: MutPtr<u8>,
    max: GuestUSize,
    format: ConstPtr<u8>,
    time: ConstPtr<tm>,
    _locale: LocaleT,
) -> GuestUSize {
    strftime(env, s, max, format, time)
}

fn localeconv_l(env: &mut Environment, _locale: LocaleT) -> MutPtr<u8> {
    super::clocale::localeconv(env)
}

/// `strtold()` is `strtod()` on ARM (`long double` is 64-bit there).
fn strtold(env: &mut Environment, nptr: ConstPtr<u8>, endptr: MutPtr<MutPtr<u8>>) -> f64 {
    super::stdlib::strtod(env, nptr, endptr)
}

fn strtold_l(
    env: &mut Environment,
    nptr: ConstPtr<u8>,
    endptr: MutPtr<MutPtr<u8>>,
    _locale: LocaleT,
) -> f64 {
    super::stdlib::strtod(env, nptr, endptr)
}

/// Parse an integer the way `strtoll`/`strtoull` do. Returns the value
/// (saturated to the type's range) and how many bytes were consumed.
fn parse_integer(text: &[u8], base: i32, signed: bool) -> (i128, usize) {
    let mut i = 0;
    while i < text.len() && (text[i] == b' ' || (9..=13).contains(&text[i])) {
        i += 1;
    }
    let mut negative = false;
    if i < text.len() && (text[i] == b'+' || text[i] == b'-') {
        negative = text[i] == b'-';
        i += 1;
    }
    let mut base = base as u32;
    let has_0x = i + 1 < text.len() && text[i] == b'0' && (text[i + 1] | 0x20) == b'x';
    if (base == 0 || base == 16) && has_0x {
        i += 2;
        base = 16;
    } else if base == 0 {
        base = if i < text.len() && text[i] == b'0' { 8 } else { 10 };
    }
    let digits_start = i;
    let mut value: i128 = 0;
    while i < text.len() {
        let digit = match text[i] {
            c @ b'0'..=b'9' => u32::from(c - b'0'),
            c @ b'a'..=b'z' => u32::from(c - b'a') + 10,
            c @ b'A'..=b'Z' => u32::from(c - b'A') + 10,
            _ => break,
        };
        if digit >= base {
            break;
        }
        value = (value * i128::from(base) + i128::from(digit)).min(1 << 70);
        i += 1;
    }
    if i == digits_start {
        return (0, 0);
    }
    if negative {
        value = -value;
    }
    let (min, max) = if signed {
        (i128::from(i64::MIN), i128::from(i64::MAX))
    } else {
        (0, i128::from(u64::MAX))
    };
    (value.clamp(min, max), i)
}

fn strto_ll_common(
    env: &mut Environment,
    nptr: ConstPtr<u8>,
    endptr: MutPtr<MutPtr<u8>>,
    base: i32,
    signed: bool,
) -> i128 {
    let text = env.mem.cstr_at(nptr).to_vec();
    let (value, used) = parse_integer(&text, base, signed);
    if !endptr.is_null() {
        env.mem.write(endptr, (nptr + used as GuestUSize).cast_mut());
    }
    value
}

fn strtoll(
    env: &mut Environment,
    nptr: ConstPtr<u8>,
    endptr: MutPtr<MutPtr<u8>>,
    base: i32,
) -> i64 {
    strto_ll_common(env, nptr, endptr, base, true) as i64
}

fn strtoll_l(
    env: &mut Environment,
    nptr: ConstPtr<u8>,
    endptr: MutPtr<MutPtr<u8>>,
    base: i32,
    _locale: LocaleT,
) -> i64 {
    strto_ll_common(env, nptr, endptr, base, true) as i64
}

fn strtoull_l(
    env: &mut Environment,
    nptr: ConstPtr<u8>,
    endptr: MutPtr<MutPtr<u8>>,
    base: i32,
    _locale: LocaleT,
) -> u64 {
    strto_ll_common(env, nptr, endptr, base, false) as u64
}

// -- multibyte (UTF-8) conversion ----------------------------------------

const MB_ERROR: GuestUSize = u32::MAX; // (size_t)-1
const MB_INCOMPLETE: GuestUSize = u32::MAX - 1; // (size_t)-2

/// Decode one UTF-8 character from the start of `bytes`:
/// `Ok((char, length))`, or `Err(MB_INCOMPLETE / MB_ERROR)`.
fn decode_one(bytes: &[u8]) -> Result<(char, usize), GuestUSize> {
    let Some(&lead) = bytes.first() else {
        return Err(MB_INCOMPLETE);
    };
    let len = match lead {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Err(MB_ERROR),
    };
    if bytes.len() < len {
        return Err(MB_INCOMPLETE);
    }
    match std::str::from_utf8(&bytes[..len]) {
        Ok(s) => Ok((s.chars().next().unwrap(), len)),
        Err(_) => Err(MB_ERROR),
    }
}

fn mbrtowc_l(
    env: &mut Environment,
    pwc: MutPtr<WcharT>,
    s: ConstPtr<u8>,
    n: GuestUSize,
    _ps: MutVoidPtr,
    _locale: LocaleT,
) -> GuestUSize {
    if s.is_null() {
        return 0;
    }
    let take = n.min(4);
    let bytes = env.mem.bytes_at(s, take).to_vec();
    match decode_one(&bytes) {
        Ok((ch, len)) => {
            if !pwc.is_null() {
                env.mem.write(pwc, ch as WcharT);
            }
            if ch == '\0' {
                0
            } else {
                len as GuestUSize
            }
        }
        Err(code) => code,
    }
}

fn mbrlen_l(
    env: &mut Environment,
    s: ConstPtr<u8>,
    n: GuestUSize,
    ps: MutVoidPtr,
    locale: LocaleT,
) -> GuestUSize {
    mbrtowc_l(env, Ptr::null(), s, n, ps, locale)
}

fn wcrtomb_l(
    env: &mut Environment,
    s: MutPtr<u8>,
    wc: WcharT,
    _ps: MutVoidPtr,
    _locale: LocaleT,
) -> GuestUSize {
    if s.is_null() {
        return 1;
    }
    let Some(ch) = char::from_u32(wc as u32) else {
        return MB_ERROR;
    };
    let mut buffer = [0u8; 4];
    let encoded = ch.encode_utf8(&mut buffer).as_bytes();
    env.mem
        .bytes_at_mut(s, encoded.len() as GuestUSize)
        .copy_from_slice(encoded);
    encoded.len() as GuestUSize
}

fn btowc_l(_env: &mut Environment, c: i32, _locale: LocaleT) -> WcharT {
    if (0..0x80).contains(&c) {
        c
    } else {
        -1 // WEOF
    }
}

fn wctob_l(_env: &mut Environment, c: WcharT, _locale: LocaleT) -> i32 {
    if (0..0x80).contains(&c) {
        c
    } else {
        -1 // EOF
    }
}

/// Shared by `mbsrtowcs_l()` and `mbsnrtowcs_l()`: convert up to `max_bytes`
/// bytes of the string `*src` into at most `len` wide characters at `dst`
/// (or just count them when `dst` is NULL).
fn mbs_to_wcs(
    env: &mut Environment,
    dst: MutPtr<WcharT>,
    src: MutPtr<ConstPtr<u8>>,
    max_bytes: GuestUSize,
    len: GuestUSize,
) -> GuestUSize {
    let start: ConstPtr<u8> = env.mem.read(src);
    let mut offset: GuestUSize = 0;
    let mut written: GuestUSize = 0;
    loop {
        if !dst.is_null() && written >= len {
            env.mem.write(src, start + offset);
            return written;
        }
        let available = max_bytes.saturating_sub(offset).min(4);
        if available == 0 {
            env.mem.write(src, start + offset);
            return written;
        }
        let bytes = env.mem.bytes_at(start + offset, available).to_vec();
        match decode_one(&bytes) {
            Ok((ch, used)) => {
                if !dst.is_null() {
                    env.mem.write(dst + written, ch as WcharT);
                }
                if ch == '\0' {
                    if !dst.is_null() {
                        env.mem.write(src, Ptr::null());
                    }
                    return written;
                }
                offset += used as GuestUSize;
                written += 1;
            }
            Err(code) => {
                env.mem.write(src, start + offset);
                return code;
            }
        }
    }
}

fn mbsrtowcs_l(
    env: &mut Environment,
    dst: MutPtr<WcharT>,
    src: MutPtr<ConstPtr<u8>>,
    len: GuestUSize,
    _ps: MutVoidPtr,
    _locale: LocaleT,
) -> GuestUSize {
    mbs_to_wcs(env, dst, src, u32::MAX, len)
}

fn mbsnrtowcs_l(
    env: &mut Environment,
    dst: MutPtr<WcharT>,
    src: MutPtr<ConstPtr<u8>>,
    nms: GuestUSize,
    len: GuestUSize,
    _ps: MutVoidPtr,
    _locale: LocaleT,
) -> GuestUSize {
    mbs_to_wcs(env, dst, src, nms, len)
}

fn wcsnrtombs_l(
    env: &mut Environment,
    dst: MutPtr<u8>,
    src: MutPtr<ConstPtr<WcharT>>,
    nwc: GuestUSize,
    len: GuestUSize,
    _ps: MutVoidPtr,
    _locale: LocaleT,
) -> GuestUSize {
    let start: ConstPtr<WcharT> = env.mem.read(src);
    let mut index: GuestUSize = 0;
    let mut written: GuestUSize = 0;
    while index < nwc {
        let wc: WcharT = env.mem.read(start + index);
        let Some(ch) = char::from_u32(wc as u32) else {
            env.mem.write(src, start + index);
            return MB_ERROR;
        };
        let mut buffer = [0u8; 4];
        let encoded = ch.encode_utf8(&mut buffer).as_bytes();
        if !dst.is_null() {
            if written + encoded.len() as GuestUSize > len {
                break;
            }
            env.mem
                .bytes_at_mut(dst + written, encoded.len() as GuestUSize)
                .copy_from_slice(encoded);
        }
        if ch == '\0' {
            if !dst.is_null() {
                env.mem.write(src, Ptr::null());
            }
            return written;
        }
        written += encoded.len() as GuestUSize;
        index += 1;
    }
    if !dst.is_null() {
        env.mem.write(src, start + index);
    }
    written
}

// -- message catalogs (never present) ------------------------------------

fn catopen(_env: &mut Environment, _name: ConstPtr<u8>, _flag: i32) -> MutVoidPtr {
    Ptr::from_bits(u32::MAX) // (nl_catd)-1: failure
}

fn catgets(_env: &mut Environment, _catalog: MutVoidPtr, _set: i32, _msg: i32, default: ConstPtr<u8>) -> ConstPtr<u8> {
    default
}

fn catclose(_env: &mut Environment, _catalog: MutVoidPtr) -> i32 {
    0
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(newlocale(_, _, _)),
    export_c_func!(freelocale(_)),
    export_c_func!(duplocale(_)),
    export_c_func!(uselocale(_)),
    export_c_func!(___mb_cur_max_l(_)),
    export_c_func!(___tolower_l(_, _)),
    export_c_func!(___toupper_l(_, _)),
    export_c_func!(___maskrune_l(_, _, _)),
    export_c_func!(snprintf_l(_, _, _, _, _)),
    export_c_func!(asprintf_l(_, _, _, _)),
    export_c_func!(sscanf_l(_, _, _, _)),
    export_c_func!(strcoll_l(_, _, _)),
    export_c_func!(strxfrm_l(_, _, _, _)),
    export_c_func!(wcscoll_l(_, _, _)),
    export_c_func!(strftime_l(_, _, _, _, _)),
    export_c_func!(localeconv_l(_)),
    export_c_func!(strtold(_, _)),
    export_c_func!(strtold_l(_, _, _)),
    export_c_func!(strtoll(_, _, _)),
    export_c_func!(strtoll_l(_, _, _, _)),
    export_c_func!(strtoull_l(_, _, _, _)),
    export_c_func!(mbrtowc_l(_, _, _, _, _)),
    export_c_func!(mbrlen_l(_, _, _, _)),
    export_c_func!(wcrtomb_l(_, _, _, _)),
    export_c_func!(btowc_l(_, _)),
    export_c_func!(wctob_l(_, _)),
    export_c_func!(mbsrtowcs_l(_, _, _, _, _)),
    export_c_func!(mbsnrtowcs_l(_, _, _, _, _, _)),
    export_c_func!(wcsnrtombs_l(_, _, _, _, _, _)),
    export_c_func!(catopen(_, _)),
    export_c_func!(catgets(_, _, _, _)),
    export_c_func!(catclose(_)),
];

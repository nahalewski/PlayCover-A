/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `errno.h`

use crate::dyld::FunctionExports;
use crate::export_c_func;
use crate::mem::{ConstPtr, MutPtr};
use crate::Environment;
use std::io::Write;

// TODO: if you add values here, make sure to update `strerror()` too!
pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const ESRCH: i32 = 3;
pub const EINTR: i32 = 4;
pub const EIO: i32 = 5;
pub const EBADF: i32 = 9;
pub const ECHILD: i32 = 10;
pub const EDEADLK: i32 = 11;
pub const ENOMEM: i32 = 12;
pub const EACCES: i32 = 13;
pub const EFAULT: i32 = 14;
pub const EBUSY: i32 = 16;
pub const EEXIST: i32 = 17;
pub const ENOTDIR: i32 = 20;
pub const EISDIR: i32 = 21;
pub const EINVAL: i32 = 22;
pub const ESPIPE: i32 = 29;
pub const EPIPE: i32 = 32;
pub const EROFS: i32 = 30;
pub const EAGAIN: i32 = 35;
pub const EPROTONOSUPPORT: i32 = 43;
pub const ENOTSUP: i32 = 45;
pub const ECONNRESET: i32 = 54;
pub const ETIMEDOUT: i32 = 60;
pub const EOVERFLOW: i32 = 84;

#[derive(Default)]
pub struct State {
    errnos: std::collections::HashMap<crate::ThreadId, MutPtr<i32>>,
    strings_cache: std::collections::HashMap<i32, ConstPtr<u8>>,
}
impl State {
    fn errno_ptr_for_thread(
        &mut self,
        mem: &mut crate::mem::Mem,
        thread: crate::ThreadId,
    ) -> MutPtr<i32> {
        *self
            .errnos
            .entry(thread)
            .or_insert_with(|| mem.alloc_and_write(0i32))
    }

    pub fn set_errno_for_thread(
        &mut self,
        mem: &mut crate::mem::Mem,
        thread: crate::ThreadId,
        val: i32,
    ) {
        let ptr = self.errno_ptr_for_thread(mem, thread);
        mem.write(ptr, val);
    }
}

/// Helper functions, not a part of libc errno
pub fn set_errno(env: &mut Environment, val: i32) {
    env.libc_state
        .errno
        .set_errno_for_thread(&mut env.mem, env.current_thread, val);
}
pub fn get_errno(env: &mut Environment) -> i32 {
    let ptr = env
        .libc_state
        .errno
        .errno_ptr_for_thread(&mut env.mem, env.current_thread);
    env.mem.read(ptr)
}

fn __error(env: &mut Environment) -> MutPtr<i32> {
    env.libc_state
        .errno
        .errno_ptr_for_thread(&mut env.mem, env.current_thread)
}

fn perror(env: &mut Environment, s: ConstPtr<u8>) {
    let errno_ptr = __error(env);
    let str_error = strerror(env, env.mem.read(errno_ptr));
    let errno_msg = format!("{}\n", env.mem.cstr_at_utf8(str_error).unwrap());
    let msg = if !s.is_null() {
        if let Ok(str) = env.mem.cstr_at_utf8(s) {
            format!("{str}: {errno_msg}")
        } else {
            errno_msg.to_string()
        }
    } else {
        errno_msg.to_string()
    };
    let _ = std::io::stderr().write_all(msg.as_bytes());
}

fn strerror(env: &mut Environment, err_num: i32) -> ConstPtr<u8> {
    if let Some(&c_str) = env.libc_state.errno.strings_cache.get(&err_num) {
        c_str
    } else {
        let str = match err_num {
            0 => "Undefined error: 0",
            6 => "Device not configured",
            7 => "Argument list too long",
            8 => "Exec format error",
            12 => "Cannot allocate memory",
            18 => "Cross-device link",
            19 => "Operation not supported by device",
            23 => "Too many open files in system",
            24 => "Too many open files",
            25 => "Inappropriate ioctl for device",
            27 => "File too large",
            28 => "No space left on device",
            31 => "Too many links",
            32 => "Broken pipe",
            33 => "Numerical argument out of domain",
            34 => "Result too large",
            36 => "Operation now in progress",
            37 => "Operation already in progress",
            38 => "Socket operation on non-socket",
            48 => "Address already in use",
            49 => "Can't assign requested address",
            50 => "Network is down",
            51 => "Network is unreachable",
            53 => "Software caused connection abort",
            55 => "No buffer space available",
            56 => "Socket is already connected",
            57 => "Socket is not connected",
            61 => "Connection refused",
            62 => "Too many levels of symbolic links",
            63 => "File name too long",
            65 => "No route to host",
            66 => "Directory not empty",
            EPERM => "Operation not permitted",
            ENOENT => "No such file or directory",
            ESRCH => "No such process",
            EINTR => "Interrupted system call",
            EIO => "Input/output error",
            EBADF => "Bad file descriptor",
            ECHILD => "No child processes",
            EDEADLK => "Resource deadlock avoided",
            ENOMEM => "Cannot allocate memory",
            EACCES => "Permission denied",
            EFAULT => "Bad address",
            EBUSY => "Resource busy",
            EEXIST => "File exists",
            ENOTDIR => "Not a directory",
            EISDIR => "Is a directory",
            EINVAL => "Invalid argument",
            ESPIPE => "Illegal seek",
            EROFS => "Read-only file system",
            EAGAIN => "Resource temporarily unavailable",
            EPROTONOSUPPORT => "Protocol not supported",
            ENOTSUP => "Operation not supported",
            ECONNRESET => "Connection reset by peer",
            ETIMEDOUT => "Operation timed out",
            EOVERFLOW => "Value too large to be stored in data type",
            _ => "",
        };
        let unknown;
        let str = if str.is_empty() {
            unknown = format!("Unknown error: {err_num}");
            unknown.as_str()
        } else {
            str
        };
        let new_c_str = env.mem.alloc_and_write_cstr(str.as_bytes()).cast_const();
        env.libc_state
            .errno
            .strings_cache
            .insert(err_num, new_c_str);
        new_c_str
    }
}

/// Thread-safe `strerror()`: copies the message into the caller's buffer.
/// Returns 0, or `ERANGE` if the buffer is too small (the message is truncated).
fn strerror_r(env: &mut Environment, err_num: i32, buf: MutPtr<u8>, buf_len: u32) -> i32 {
    let message = strerror(env, err_num);
    let text = env.mem.cstr_at(message).to_vec();
    if buf.is_null() || buf_len == 0 {
        return 34; // ERANGE
    }
    let copy_len = text.len().min(buf_len as usize - 1);
    let dest = env.mem.bytes_at_mut(buf, copy_len as u32 + 1);
    dest[..copy_len].copy_from_slice(&text[..copy_len]);
    dest[copy_len] = 0;
    if copy_len < text.len() {
        34 // ERANGE
    } else {
        0
    }
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(__error()),
    export_c_func!(perror(_)),
    export_c_func!(strerror(_)),
    export_c_func!(strerror_r(_, _, _)),
];

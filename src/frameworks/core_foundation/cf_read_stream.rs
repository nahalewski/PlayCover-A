/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `CFReadStream` for files, and `CFURLCreatePropertyFromResource()`.
//!
//! Some games (e.g. Zenonia) read their resources with `CFReadStream` and ask
//! `CFURLCreatePropertyFromResource()` for a file's length first. A file read
//! stream here simply reads the whole file into memory when it is created.

use super::cf_allocator::CFAllocatorRef;
use super::cf_url::CFURLRef;
use super::{CFIndex, CFTypeRef};
use crate::dyld::{ConstantExports, FunctionExports, HostConstant};
use crate::export_c_func;
use crate::frameworks::foundation::{ns_string, ns_url};
use crate::mem::MutPtr;
use crate::objc::{id, msg_class, nil, objc_classes, ClassExports, HostObject, NSZonePtr};
use crate::Environment;

type CFReadStreamRef = CFTypeRef;
type CFStreamStatus = CFIndex;
type CFStringRef = id;

const kCFStreamStatusNotOpen: CFStreamStatus = 0;
const kCFStreamStatusOpen: CFStreamStatus = 2;
const kCFStreamStatusAtEnd: CFStreamStatus = 4;
const kCFStreamStatusClosed: CFStreamStatus = 5;
const kCFStreamStatusError: CFStreamStatus = 6;

const kCFURLFileExists: &str = "kCFURLFileExists";
const kCFURLFileDirectoryContents: &str = "kCFURLFileDirectoryContents";
const kCFURLFileLength: &str = "kCFURLFileLength";
const kCFURLFileLastModificationTime: &str = "kCFURLFileLastModificationTime";
const kCFURLFilePOSIXMode: &str = "kCFURLFilePOSIXMode";
const kCFURLFileOwnerID: &str = "kCFURLFileOwnerID";
const kCFURLHTTPStatusCode: &str = "kCFURLHTTPStatusCode";
const kCFURLHTTPStatusLine: &str = "kCFURLHTTPStatusLine";

pub const CONSTANTS: ConstantExports = &[
    ("_kCFURLFileExists", HostConstant::NSString(kCFURLFileExists)),
    ("_kCFURLFileDirectoryContents", HostConstant::NSString(kCFURLFileDirectoryContents)),
    ("_kCFURLFileLength", HostConstant::NSString(kCFURLFileLength)),
    ("_kCFURLFileLastModificationTime", HostConstant::NSString(kCFURLFileLastModificationTime)),
    ("_kCFURLFilePOSIXMode", HostConstant::NSString(kCFURLFilePOSIXMode)),
    ("_kCFURLFileOwnerID", HostConstant::NSString(kCFURLFileOwnerID)),
    ("_kCFURLHTTPStatusCode", HostConstant::NSString(kCFURLHTTPStatusCode)),
    ("_kCFURLHTTPStatusLine", HostConstant::NSString(kCFURLHTTPStatusLine)),
];

struct CFReadStreamHostObject {
    /// `None` if the file could not be read.
    data: Option<Vec<u8>>,
    position: usize,
    status: CFStreamStatus,
}
impl HostObject for CFReadStreamHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation _touchHLE_CFReadStream: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(CFReadStreamHostObject {
        data: None,
        position: 0,
        status: kCFStreamStatusNotOpen,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

@end

};

fn CFReadStreamCreateWithFile(
    env: &mut Environment,
    _allocator: CFAllocatorRef,
    url: CFURLRef,
) -> CFReadStreamRef {
    let path = ns_url::to_rust_path(env, url);
    let data = env.fs.read(&path).ok();
    log_dbg!("CFReadStreamCreateWithFile({:?}) => {:?} bytes", path, data.as_ref().map(|d| d.len()));
    let stream: id = msg_class![env; _touchHLE_CFReadStream alloc];
    env.objc.borrow_mut::<CFReadStreamHostObject>(stream).data = data;
    stream
}

fn CFReadStreamOpen(env: &mut Environment, stream: CFReadStreamRef) -> bool {
    let host = env.objc.borrow_mut::<CFReadStreamHostObject>(stream);
    if host.data.is_some() {
        host.status = kCFStreamStatusOpen;
        true
    } else {
        host.status = kCFStreamStatusError;
        false
    }
}

fn CFReadStreamRead(
    env: &mut Environment,
    stream: CFReadStreamRef,
    buffer: MutPtr<u8>,
    buffer_length: CFIndex,
) -> CFIndex {
    let host = env.objc.borrow_mut::<CFReadStreamHostObject>(stream);
    let Some(ref data) = host.data else {
        return -1;
    };
    if host.status != kCFStreamStatusOpen && host.status != kCFStreamStatusAtEnd {
        return -1;
    }
    let wanted = buffer_length.max(0) as usize;
    let available = data.len().saturating_sub(host.position);
    let count = wanted.min(available);
    let chunk = data[host.position..host.position + count].to_vec();
    host.position += count;
    if host.position >= data.len() {
        host.status = kCFStreamStatusAtEnd;
    }
    if count > 0 {
        env.mem
            .bytes_at_mut(buffer, count as u32)
            .copy_from_slice(&chunk);
    }
    count as CFIndex
}

fn CFReadStreamHasBytesAvailable(env: &mut Environment, stream: CFReadStreamRef) -> bool {
    let host = env.objc.borrow::<CFReadStreamHostObject>(stream);
    host.data.as_ref().is_some_and(|d| host.position < d.len())
}

fn CFReadStreamGetStatus(env: &mut Environment, stream: CFReadStreamRef) -> CFStreamStatus {
    env.objc.borrow::<CFReadStreamHostObject>(stream).status
}

fn CFReadStreamClose(env: &mut Environment, stream: CFReadStreamRef) {
    env.objc.borrow_mut::<CFReadStreamHostObject>(stream).status = kCFStreamStatusClosed;
}

fn CFReadStreamCopyError(_env: &mut Environment, _stream: CFReadStreamRef) -> CFTypeRef {
    nil
}

fn CFURLCreatePropertyFromResource(
    env: &mut Environment,
    _allocator: CFAllocatorRef,
    url: CFURLRef,
    property: CFStringRef,
    error_code: MutPtr<i32>,
) -> CFTypeRef {
    let property = ns_string::to_rust_string(env, property).into_owned();
    let path = ns_url::to_rust_path(env, url);
    let exists = env.fs.exists(&path);
    let result: id = match property.as_str() {
        kCFURLFileExists => msg_class![env; NSNumber numberWithBool:exists],
        kCFURLFileLength if exists => {
            let size = env.fs.size(&path).unwrap_or(0) as i64;
            msg_class![env; NSNumber numberWithLongLong:size]
        }
        kCFURLFileLastModificationTime if exists => nil, // TODO
        _ => nil,
    };
    log_dbg!("CFURLCreatePropertyFromResource({:?}, {}) => {:?}", path, property, result);
    if !error_code.is_null() {
        // kCFURLUnknownError (-10) / kCFURLResourceNotFoundError (-11)
        let code = if result != nil {
            0
        } else if !exists {
            -11
        } else {
            -10
        };
        env.mem.write(error_code, code);
    }
    // A +1 reference, like any "Create" function: the number above is
    // autoreleased, so take our own reference for the caller to release.
    if result != nil {
        crate::objc::retain(env, result);
    }
    result
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CFReadStreamCreateWithFile(_, _)),
    export_c_func!(CFReadStreamOpen(_)),
    export_c_func!(CFReadStreamRead(_, _, _)),
    export_c_func!(CFReadStreamHasBytesAvailable(_)),
    export_c_func!(CFReadStreamGetStatus(_)),
    export_c_func!(CFReadStreamClose(_)),
    export_c_func!(CFReadStreamCopyError(_)),
    export_c_func!(CFURLCreatePropertyFromResource(_, _, _, _)),
];

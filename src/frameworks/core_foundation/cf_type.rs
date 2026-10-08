/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `CFType` (type-generic functions etc).

use super::{CFHashCode, CFIndex};
use crate::dyld::{export_c_func, FunctionExports};
use crate::frameworks::foundation::NSUInteger;
use crate::objc::Class;
use crate::{msg, objc};
use crate::{msg_class, Environment};

pub type CFTypeRef = objc::id;

pub fn CFRetain(env: &mut Environment, object: CFTypeRef) -> CFTypeRef {
    assert!(!object.is_null()); // not allowed, unlike for normal objc objects
    objc::retain(env, object)
}
pub fn CFRelease(env: &mut Environment, object: CFTypeRef) {
    objc::release(env, object);
}

pub fn CFGetRetainCount(env: &mut Environment, object: CFTypeRef) -> CFIndex {
    let count: NSUInteger = msg![env; object retainCount];
    count as CFIndex
}

pub fn CFEqual(env: &mut Environment, object1: CFTypeRef, object2: CFTypeRef) -> bool {
    if object1 == object2 {
        return true;
    }
    // TODO: other classes
    let str_class: Class = msg_class![env; NSString class];
    let object1_class: Class = msg![env; object1 class];
    assert!(msg![env; object1_class isKindOfClass:str_class]);
    let object2_class: Class = msg![env; object2 class];
    assert!(msg![env; object2_class isKindOfClass:str_class]);
    // TODO: use isEqual: once it is fixed
    msg![env; object1 isEqualToString:object2]
}

pub fn CFHash(env: &mut Environment, object: CFTypeRef) -> CFHashCode {
    msg![env; object hash]
}

/// Type ids follow Apple's order closely enough for apps that compare them
/// with `CFxxxGetTypeID()` (they never look at the numbers themselves).
pub type CFTypeID = u32;
const TYPE_ID_NULL: CFTypeID = 4;
const TYPE_ID_ARRAY: CFTypeID = 19;
const TYPE_ID_BOOLEAN: CFTypeID = 21;
const TYPE_ID_DATA: CFTypeID = 20;
const TYPE_ID_DATE: CFTypeID = 42;
const TYPE_ID_DICTIONARY: CFTypeID = 18;
const TYPE_ID_NUMBER: CFTypeID = 22;
const TYPE_ID_SET: CFTypeID = 17;
const TYPE_ID_STRING: CFTypeID = 7;
const TYPE_ID_URL: CFTypeID = 29;
const TYPE_ID_UNKNOWN: CFTypeID = 1;

pub fn CFGetTypeID(env: &mut Environment, object: CFTypeRef) -> CFTypeID {
    if object.is_null() {
        return TYPE_ID_UNKNOWN;
    }
    let object_class: Class = msg![env; object class];
    for (name, id) in [
        ("NSString", TYPE_ID_STRING),
        ("NSNumber", TYPE_ID_NUMBER),
        ("NSArray", TYPE_ID_ARRAY),
        ("NSDictionary", TYPE_ID_DICTIONARY),
        ("NSData", TYPE_ID_DATA),
        ("NSDate", TYPE_ID_DATE),
        ("NSNull", TYPE_ID_NULL),
        ("NSSet", TYPE_ID_SET),
        ("NSURL", TYPE_ID_URL),
    ] {
        let candidate: Class = env.objc.get_known_class(name, &mut env.mem);
        if msg![env; object_class isKindOfClass:candidate] {
            // NSNumber also covers booleans.
            return id;
        }
    }
    TYPE_ID_UNKNOWN
}

fn CFStringGetTypeID(_env: &mut Environment) -> CFTypeID {
    TYPE_ID_STRING
}
fn CFNumberGetTypeID(_env: &mut Environment) -> CFTypeID {
    TYPE_ID_NUMBER
}
fn CFBooleanGetTypeID(_env: &mut Environment) -> CFTypeID {
    TYPE_ID_BOOLEAN
}
fn CFArrayGetTypeID(_env: &mut Environment) -> CFTypeID {
    TYPE_ID_ARRAY
}
fn CFDictionaryGetTypeID(_env: &mut Environment) -> CFTypeID {
    TYPE_ID_DICTIONARY
}
fn CFDataGetTypeID(_env: &mut Environment) -> CFTypeID {
    TYPE_ID_DATA
}
fn CFDateGetTypeID(_env: &mut Environment) -> CFTypeID {
    TYPE_ID_DATE
}
fn CFNullGetTypeID(_env: &mut Environment) -> CFTypeID {
    TYPE_ID_NULL
}
fn CFURLGetTypeID(_env: &mut Environment) -> CFTypeID {
    TYPE_ID_URL
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CFRetain(_)),
    export_c_func!(CFRelease(_)),
    export_c_func!(CFGetRetainCount(_)),
    export_c_func!(CFEqual(_, _)),
    export_c_func!(CFHash(_)),
    export_c_func!(CFGetTypeID(_)),
    export_c_func!(CFStringGetTypeID()),
    export_c_func!(CFNumberGetTypeID()),
    export_c_func!(CFBooleanGetTypeID()),
    export_c_func!(CFArrayGetTypeID()),
    export_c_func!(CFDictionaryGetTypeID()),
    export_c_func!(CFDataGetTypeID()),
    export_c_func!(CFDateGetTypeID()),
    export_c_func!(CFNullGetTypeID()),
    export_c_func!(CFURLGetTypeID()),
];

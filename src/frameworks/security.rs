/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! The Security framework: only the keychain item constants and functions.
//!
//! There is no keychain. The `SecItem*` functions report that none is
//! available, so apps that store secrets in it behave as if the keychain is
//! locked or missing. The constants exist so apps that read them (e.g. to
//! build a query dictionary) get real strings rather than null pointers.

use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant, HostDylib};
use crate::mem::MutPtr;
use crate::objc::{id, nil};
use crate::Environment;

pub const DYLIB: HostDylib = HostDylib {
    path: "/System/Library/Frameworks/Security.framework/Security",
    aliases: &[],
    class_exports: &[],
    constant_exports: &[CONSTANTS],
    function_exports: &[FUNCTIONS],
};

type OSStatus = i32;
/// `errSecNotAvailable`: no keychain is available.
const errSecNotAvailable: OSStatus = -25291;

macro_rules! sec_constants {
    ( $( $name:literal ),* $(,)? ) => {
        &[ $( (concat!("_", $name), HostConstant::NSString($name)) ),* ]
    };
}

pub const CONSTANTS: ConstantExports = sec_constants![
    "kSecClass",
    "kSecClassGenericPassword",
    "kSecClassInternetPassword",
    "kSecAttrAccount",
    "kSecAttrService",
    "kSecAttrGeneric",
    "kSecAttrServer",
    "kSecAttrType",
    "kSecAttrSecurityDomain",
    "kSecAttrAuthenticationType",
    "kSecAttrAuthenticationTypeDefault",
    "kSecMatchLimit",
    "kSecMatchLimitOne",
    "kSecReturnAttributes",
    "kSecReturnData",
    "kSecValueData",
];

fn SecItemAdd(env: &mut Environment, _attributes: id, result: MutPtr<id>) -> OSStatus {
    if !result.is_null() {
        env.mem.write(result, nil);
    }
    errSecNotAvailable
}
fn SecItemCopyMatching(env: &mut Environment, _query: id, result: MutPtr<id>) -> OSStatus {
    if !result.is_null() {
        env.mem.write(result, nil);
    }
    errSecNotAvailable
}
fn SecItemUpdate(_env: &mut Environment, _query: id, _attributes_to_update: id) -> OSStatus {
    errSecNotAvailable
}
fn SecItemDelete(_env: &mut Environment, _query: id) -> OSStatus {
    errSecNotAvailable
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(SecItemAdd(_, _)),
    export_c_func!(SecItemCopyMatching(_, _)),
    export_c_func!(SecItemUpdate(_, _)),
    export_c_func!(SecItemDelete(_)),
];

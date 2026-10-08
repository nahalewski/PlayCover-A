/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `CFPreferences`.
//!
//! According to Apple's docs, it's not toll-free bridged to `NSUserDefaults`,
//! but we are still implementing one atop of another.

use super::cf_string::CFStringRef;
use super::CFTypeRef;
use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant};
use crate::frameworks::foundation::ns_string;
use crate::objc::{id, msg, msg_class};
use crate::Environment;

type CFPropertyListRef = CFTypeRef;

fn CFPreferencesCopyAppValue(
    env: &mut Environment,
    key: CFStringRef,
    app_id: CFStringRef,
) -> CFPropertyListRef {
    note_other_app(env, app_id);
    let user_defaults: id = msg_class![env; NSUserDefaults standardUserDefaults];
    let value: id = msg![env; user_defaults objectForKey:key];
    msg![env; value copy]
}

/// Only the app's own preferences exist here: other application ids, hosts and
/// users all map to them.
fn note_other_app(env: &mut Environment, app_id: CFStringRef) {
    let current_app = ns_string::get_static_str(env, kCFPreferencesCurrentApplication);
    if app_id.is_null() || !msg![env; app_id isEqualToString:current_app] {
        log_once!("TODO: CFPreferences for another application id are treated as the current app's");
    }
}

fn CFPreferencesCopyValue(
    env: &mut Environment,
    key: CFStringRef,
    app_id: CFStringRef,
    _user: CFStringRef,
    _host: CFStringRef,
) -> CFPropertyListRef {
    CFPreferencesCopyAppValue(env, key, app_id)
}

fn CFPreferencesSetValue(
    env: &mut Environment,
    key: CFStringRef,
    value: CFPropertyListRef,
    app_id: CFStringRef,
    _user: CFStringRef,
    _host: CFStringRef,
) {
    CFPreferencesSetAppValue(env, key, value, app_id)
}

fn CFPreferencesSynchronize(
    env: &mut Environment,
    app_id: CFStringRef,
    _user: CFStringRef,
    _host: CFStringRef,
) -> bool {
    CFPreferencesAppSynchronize(env, app_id)
}

fn CFPreferencesCopyKeyList(
    env: &mut Environment,
    app_id: CFStringRef,
    _user: CFStringRef,
    _host: CFStringRef,
) -> CFPropertyListRef {
    note_other_app(env, app_id);
    let user_defaults: id = msg_class![env; NSUserDefaults standardUserDefaults];
    let all: id = msg![env; user_defaults dictionaryRepresentation];
    if all.is_null() {
        return msg_class![env; NSArray new];
    }
    let keys: id = msg![env; all allKeys];
    msg![env; keys copy]
}

fn CFPreferencesSetAppValue(
    env: &mut Environment,
    key: CFStringRef,
    value: CFPropertyListRef,
    app_id: CFStringRef,
) {
    note_other_app(env, app_id);
    let user_defaults: id = msg_class![env; NSUserDefaults standardUserDefaults];
    if value.is_null() {
        msg![env; user_defaults removeObjectForKey:key]
    } else {
        msg![env; user_defaults setObject:value forKey:key]
    }
}

fn CFPreferencesAppSynchronize(env: &mut Environment, app_id: CFStringRef) -> bool {
    note_other_app(env, app_id);
    let user_defaults: id = msg_class![env; NSUserDefaults standardUserDefaults];
    msg![env; user_defaults synchronize]
}

pub const kCFPreferencesCurrentApplication: &str = "kCFPreferencesCurrentApplication";

pub const CONSTANTS: ConstantExports = &[
    (
        "_kCFPreferencesCurrentApplication",
        HostConstant::NSString(kCFPreferencesCurrentApplication),
    ),
    ("_kCFPreferencesAnyApplication", HostConstant::NSString("kCFPreferencesAnyApplication")),
    ("_kCFPreferencesAnyHost", HostConstant::NSString("kCFPreferencesAnyHost")),
    ("_kCFPreferencesCurrentHost", HostConstant::NSString("kCFPreferencesCurrentHost")),
    ("_kCFPreferencesAnyUser", HostConstant::NSString("kCFPreferencesAnyUser")),
    ("_kCFPreferencesCurrentUser", HostConstant::NSString("kCFPreferencesCurrentUser")),
];

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CFPreferencesCopyAppValue(_, _)),
    export_c_func!(CFPreferencesSetAppValue(_, _, _)),
    export_c_func!(CFPreferencesAppSynchronize(_)),
    export_c_func!(CFPreferencesCopyValue(_, _, _, _)),
    export_c_func!(CFPreferencesSetValue(_, _, _, _, _)),
    export_c_func!(CFPreferencesSynchronize(_, _, _)),
    export_c_func!(CFPreferencesCopyKeyList(_, _, _)),
];

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Separate module just for the dylib list, so it gets its own git history.

use crate::frameworks;
use crate::libc;
use crate::objc;

/// The single list of host dylibs that the linker (and Objective-C runtime)
/// searches through.
pub const DYLIB_LIST: &[&super::HostDylib] = &[
    &libc::DYLIB,
    &objc::DYLIB,
    &crate::environment::app_picker::DYLIB, // Not a real library; special internal classes.
    &frameworks::audio_toolbox::DYLIB,
    &frameworks::avfoundation::DYLIB,
    &frameworks::cf_network::DYLIB,
    &frameworks::core_animation::DYLIB,
    &frameworks::core_foundation::DYLIB,
    &frameworks::core_graphics::DYLIB,
    &frameworks::core_location::DYLIB,
    &frameworks::core_motion::DYLIB,
    &frameworks::core_text::DYLIB,
    &frameworks::core_telephony::DYLIB,
    &frameworks::security::DYLIB,
    &frameworks::foundation::DYLIB,
    &frameworks::game_kit::DYLIB,
    &frameworks::media_player::DYLIB,
    &frameworks::message_ui::DYLIB,
    &frameworks::openal::DYLIB,
    &frameworks::opengles::DYLIB,
    &frameworks::social::DYLIB,
    &frameworks::store_kit::DYLIB,
    &frameworks::system_configuration::DYLIB,
    &frameworks::uikit::DYLIB,
];

#[cfg(test)]
mod tests {
    use crate::objc::ClassTemplate;

    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_duplicate_classes() {
        let mut seen_classes = HashSet::new();

        for (class_name, template) in DYLIB_LIST
            .iter()
            .flat_map(|dylib| dylib.class_exports)
            .copied()
            .flatten()
        {
            if !seen_classes.insert(class_name) {
                panic!("Found duplicate class export {class_name}");
            }
            let ClassTemplate {
                class_methods,
                instance_methods,
                ..
            } = template;

            let mut seen_class_methods = HashSet::with_capacity(class_methods.len());

            for (method_name, _) in *class_methods {
                if !seen_class_methods.insert(method_name) {
                    panic!("Found duplicate class method {method_name} for class {class_name}")
                }
            }

            let mut seen_instance_methods = HashSet::with_capacity(instance_methods.len());

            for (method_name, _) in *instance_methods {
                if !seen_instance_methods.insert(method_name) {
                    panic!("Found duplicate instance method {method_name} for class {class_name}")
                }
            }
        }
    }

    #[test]
    fn no_duplicate_functions() {
        let mut seen = HashSet::new();

        for (function_name, _) in DYLIB_LIST
            .iter()
            .flat_map(|dylib| dylib.function_exports)
            .copied()
            .flatten()
        {
            if !seen.insert(function_name) {
                panic!("Found duplicate function export {function_name}");
            }
        }
    }

    #[test]
    fn no_duplicate_constants() {
        let mut seen = HashSet::new();

        for (constant_name, _) in DYLIB_LIST
            .iter()
            .flat_map(|dylib| dylib.constant_exports)
            .copied()
            .flatten()
        {
            if !seen.insert(constant_name) {
                panic!("Found duplicate constant export {constant_name}");
            }
        }
    }
}

/// Everything the host implements, as text, for the Android launcher's
/// compatibility check (one `F|C|K name` entry per line: functions,
/// constants, classes). Needs no emulator state.
pub fn exported_symbol_list() -> String {
    let mut out = String::new();
    for dylib in DYLIB_LIST {
        for table in dylib.function_exports {
            for (name, _) in table.iter() {
                out.push_str("F|");
                out.push_str(name);
                out.push('\n');
            }
        }
        for table in dylib.constant_exports {
            for (name, _) in table.iter() {
                out.push_str("C|");
                out.push_str(name);
                out.push('\n');
            }
        }
        for table in dylib.class_exports {
            for (name, _) in table.iter() {
                out.push_str("K|");
                out.push_str(name);
                out.push('\n');
            }
        }
    }
    out
}

/// JNI entry point for `org.touchhle.android.SymbolIndex.exportedSymbols()`.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_org_touchhle_android_SymbolIndex_exportedSymbols(
    env: *mut *const *const std::ffi::c_void,
    _this: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    type NewStringUtf =
        unsafe extern "system" fn(*mut *const *const std::ffi::c_void, *const std::ffi::c_char) -> *mut std::ffi::c_void;
    let text = std::ffi::CString::new(exported_symbol_list().replace('\0', "")).unwrap();
    unsafe {
        // NewStringUTF is entry 167 of the JNI function table.
        let table = *env as *const *const std::ffi::c_void;
        let new_string_utf: NewStringUtf = std::mem::transmute(table.add(167).read());
        new_string_utf(env, text.as_ptr())
    }
}

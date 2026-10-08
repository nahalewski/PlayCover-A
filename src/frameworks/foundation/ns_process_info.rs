/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSProcessInfo`.

use super::NSTimeInterval;
use crate::frameworks::foundation::ns_string;
use crate::libc::mach::host::PHYSICAL_MEMORY;
use crate::objc::{autorelease, id, msg, msg_class, objc_classes, release, ClassExports};
use crate::Environment;
use std::time::Instant;

#[derive(Default)]
pub struct State {
    /// `NSProcessInfo*`
    process_info: Option<id>,
}

fn assert_process_info_singleton(env: &mut Environment, this: id) {
    assert_eq!(
        this,
        env.framework_state
            .foundation
            .ns_process_info
            .process_info
            .unwrap()
    );
}

// POSIX values are bytes; Foundation can represent only decodable strings.
// Never substitute lossy text for an environment key or value.
fn environment_strings(key: &[u8], value: &[u8]) -> Option<(String, String)> {
    Some((std::str::from_utf8(key).ok()?.to_owned(),
          std::str::from_utf8(value).ok()?.to_owned()))
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSProcessInfo: NSObject

+ (id)processInfo {
    if let Some(existing) = env.framework_state.foundation.ns_process_info.process_info {
        existing
    } else {
        let process_info: id = msg![env; this new];
        env.framework_state.foundation.ns_process_info.process_info = Some(process_info);
        process_info
    }
}

- (NSTimeInterval)systemUptime {
    assert_process_info_singleton(env, this); // TODO
    Instant::now().duration_since(env.startup_time).as_secs_f64()
}

- (u64)physicalMemory {
    assert_process_info_singleton(env, this); // TODO
    PHYSICAL_MEMORY.into()
}

- (id)processName {
    // This function probably just needs to return a unique value
    // Testing on macOS appears CFBundleName is used
    assert_process_info_singleton(env, this); // TODO
    let main_bundle: id = msg_class![env; NSBundle mainBundle];
    let name_key: id = ns_string::get_static_str(env, "CFBundleName");
    msg![env; main_bundle objectForInfoDictionaryKey:name_key]
}

- (id)environment {
    assert_process_info_singleton(env, this);
    // Same guest-owned map as getenv/setenv. In particular, HOME names the
    // guest sandbox; host process variables are not part of this process.
    let values: Vec<_> = env.env_vars.iter().filter_map(|(key, pointer)| {
        environment_strings(key, env.mem.cstr_at(*pointer))
    }).collect();
    let mutable: id = msg_class![env; NSMutableDictionary new];
    for (key, value) in values {
        let key = ns_string::from_rust_string(env, key);
        let value = ns_string::from_rust_string(env, value);
        () = msg![env; mutable setObject:value forKey:key];
    }
    let snapshot: id = msg![env; mutable copy];
    release(env, mutable);
    autorelease(env, snapshot)
}

@end

};

#[cfg(test)]
mod tests {
    use super::environment_strings;

    #[test]
    fn environment_snapshot_owns_strings_and_preserves_empty_values() {
        let mut value = b"/guest/Library".to_vec();
        let snapshot = environment_strings(b"HOME", &value).unwrap();
        value.clear();
        assert_eq!(snapshot, ("HOME".into(), "/guest/Library".into()));
        assert_eq!(environment_strings(b"debugger", b""),
                   Some(("debugger".into(), String::new())));
        assert!(environment_strings(&[0xff], b"value").is_none());
        assert!(environment_strings(b"key", &[0xff]).is_none());
    }
}

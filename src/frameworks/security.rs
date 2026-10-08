/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! The Security framework: only the keychain item constants and functions.
//!
//! The keychain is a small per-app store of items (attribute dictionaries
//! plus optional secret data), kept in the app sandbox so it persists across
//! launches like the real keychain does. Apps rely on that persistence: e.g.
//! the commonly embedded `SFHFKeychainUtils` stores a device UUID there that
//! games (Zenonia 5) embed in their save files and check on load, so a
//! keychain that forgets items makes every existing save look foreign.
//!
//! Only what typical apps use is implemented: generic items matched by their
//! attributes, `kSecReturnData`/`kSecReturnAttributes`, `kSecMatchLimit`.
//! Each constant's value is its own name (see [CONSTANTS]), so attribute keys
//! are stored and compared by that name.

use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant, HostDylib};
use crate::frameworks::foundation::{ns_array, ns_dictionary, ns_string, NSUInteger};
use crate::fs::GuestPathBuf;
use crate::mem::{ConstVoidPtr, MutPtr};
use crate::objc::{id, msg, msg_class, nil, release, Class};
use crate::Environment;
use plist::{Dictionary, Value};
use std::io::Cursor;

pub const DYLIB: HostDylib = HostDylib {
    path: "/System/Library/Frameworks/Security.framework/Security",
    aliases: &[],
    class_exports: &[],
    constant_exports: &[CONSTANTS],
    function_exports: &[FUNCTIONS],
};

type OSStatus = i32;
const errSecSuccess: OSStatus = 0;
/// `errSecParam`: one or more parameters passed to a function were not valid.
const errSecParam: OSStatus = -50;
/// `errSecDuplicateItem`: the item already exists.
const errSecDuplicateItem: OSStatus = -25299;
/// `errSecItemNotFound`: the item cannot be found.
const errSecItemNotFound: OSStatus = -25300;

macro_rules! sec_constants {
    ( $( $name:literal ),* $(,)? ) => {
        &[ $( (concat!("_", $name), HostConstant::NSString($name)) ),* ]
    };
}

pub const CONSTANTS: ConstantExports = sec_constants![
    "kSecClass",
    "kSecClassGenericPassword",
    "kSecClassInternetPassword",
    "kSecAttrAccessGroup",
    "kSecAttrAccessible",
    "kSecAttrAccount",
    "kSecAttrComment",
    "kSecAttrDescription",
    "kSecAttrLabel",
    "kSecAttrService",
    "kSecAttrGeneric",
    "kSecAttrServer",
    "kSecAttrType",
    "kSecAttrSecurityDomain",
    "kSecAttrAuthenticationType",
    "kSecAttrAuthenticationTypeDefault",
    "kSecMatchLimit",
    "kSecMatchLimitOne",
    "kSecMatchLimitAll",
    "kSecReturnAttributes",
    "kSecReturnData",
    "kSecValueData",
];

const VALUE_DATA: &str = "kSecValueData";

/// Keys of a query that control the search/result rather than describing the
/// item's attributes.
fn is_control_key(key: &str) -> bool {
    key.starts_with("kSecReturn") || key.starts_with("kSecMatch") || key.starts_with("kSecUse")
}

fn keychain_path(env: &Environment) -> GuestPathBuf {
    env.fs
        .home_directory()
        .join("Library")
        .join("Keychains")
        .join("touchHLE_keychain.plist")
}

fn load_items(env: &Environment) -> Vec<Dictionary> {
    let Ok(bytes) = env.fs.read(keychain_path(env)) else {
        return Vec::new();
    };
    match Value::from_reader(Cursor::new(bytes)) {
        Ok(Value::Array(items)) => items
            .into_iter()
            .filter_map(|item| item.into_dictionary())
            .collect(),
        _ => {
            log!("Warning: keychain file is unreadable, treating it as empty");
            Vec::new()
        }
    }
}

fn save_items(env: &mut Environment, items: &[Dictionary]) {
    let value = Value::Array(items.iter().cloned().map(Value::Dictionary).collect());
    let mut buf = Vec::new();
    value.to_writer_xml(&mut buf).unwrap();
    let path = keychain_path(env);
    _ = env.fs.create_dir_all(path.parent().unwrap());
    if let Err(e) = env.fs.write(&*path, &buf) {
        log!("Warning: couldn't write keychain file {:?}: {:?}", path, e);
    }
}

/// Converts a guest attribute value (`NSString`, `NSData`, `NSNumber`...) to
/// a plist value.
fn guest_value_to_plist(env: &mut Environment, value: id) -> Value {
    let class: Class = msg![env; value class];
    let string_class = env.objc.get_known_class("NSString", &mut env.mem);
    let data_class = env.objc.get_known_class("NSData", &mut env.mem);
    if env.objc.class_is_subclass_of(class, string_class) {
        Value::String(ns_string::to_rust_string(env, value).into_owned())
    } else if env.objc.class_is_subclass_of(class, data_class) {
        let bytes: ConstVoidPtr = msg![env; value bytes];
        let length: NSUInteger = msg![env; value length];
        let data = if length == 0 {
            Vec::new()
        } else {
            env.mem.bytes_at(bytes.cast(), length).to_vec()
        };
        Value::Data(data)
    } else {
        // e.g. NSNumber (kSecAttrType etc.): compare by description.
        let description: id = msg![env; value description];
        Value::String(ns_string::to_rust_string(env, description).into_owned())
    }
}

/// Converts a plist value back into a new guest object (+1 reference).
fn plist_to_guest_value(env: &mut Environment, value: &Value) -> id {
    match value {
        Value::Data(data) => {
            let length: NSUInteger = data.len().try_into().unwrap();
            let buffer = env.mem.alloc(length.max(1));
            env.mem.bytes_at_mut(buffer.cast(), length).copy_from_slice(data);
            let object: id = msg_class![env; NSData alloc];
            let object: id = msg![env; object initWithBytes:(buffer.cast_const()) length:length];
            env.mem.free(buffer);
            object
        }
        Value::String(s) => ns_string::from_rust_string(env, s.clone()),
        other => ns_string::from_rust_string(env, format!("{other:?}")),
    }
}

/// Reads a guest `NSDictionary` into (key name, value) pairs.
fn read_guest_dict(env: &mut Environment, dict: id) -> Vec<(String, id)> {
    if dict == nil {
        return Vec::new();
    }
    let keys: id = msg![env; dict allKeys];
    let count: NSUInteger = msg![env; keys count];
    let mut result = Vec::with_capacity(count as usize);
    for i in 0..count {
        let key: id = msg![env; keys objectAtIndex:i];
        let value: id = msg![env; dict objectForKey:key];
        let key_str = ns_string::to_rust_string(env, key).into_owned();
        result.push((key_str, value));
    }
    result
}

fn is_true(env: &mut Environment, value: Option<id>) -> bool {
    match value {
        Some(value) if value != nil => msg![env; value boolValue],
        _ => false,
    }
}

/// A parsed query: attributes that must match, plus result controls.
struct Query {
    attributes: Vec<(String, Value)>,
    return_data: bool,
    return_attributes: bool,
    match_all: bool,
}

fn parse_query(env: &mut Environment, query: id) -> Query {
    let mut parsed = Query {
        attributes: Vec::new(),
        return_data: false,
        return_attributes: false,
        match_all: false,
    };
    for (key, value) in read_guest_dict(env, query) {
        match key.as_str() {
            "kSecReturnData" => parsed.return_data = is_true(env, Some(value)),
            "kSecReturnAttributes" => parsed.return_attributes = is_true(env, Some(value)),
            "kSecMatchLimit" => {
                let limit = guest_value_to_plist(env, value);
                parsed.match_all = limit.as_string() == Some("kSecMatchLimitAll");
            }
            VALUE_DATA => (),
            other if is_control_key(other) => {
                log!("TODO: keychain query key {:?} ignored", other);
            }
            _ => {
                let value = guest_value_to_plist(env, value);
                parsed.attributes.push((key, value));
            }
        }
    }
    parsed
}

fn item_matches(item: &Dictionary, attributes: &[(String, Value)]) -> bool {
    attributes
        .iter()
        .all(|(key, value)| item.get(key) == Some(value))
}

/// Builds the result object for one item (+1 reference), or `nil` if the
/// query didn't ask for anything to be returned.
fn item_result(env: &mut Environment, item: &Dictionary, query: &Query) -> id {
    if query.return_attributes {
        let mut pairs = Vec::new();
        for (key, value) in item.iter() {
            if key == VALUE_DATA && !query.return_data {
                continue;
            }
            let key = ns_string::from_rust_string(env, key.clone());
            let value = plist_to_guest_value(env, value);
            pairs.push((key, value));
        }
        let dict = ns_dictionary::dict_from_keys_and_objects(env, &pairs);
        for (key, value) in pairs {
            release(env, key);
            release(env, value);
        }
        dict
    } else if query.return_data {
        match item.get(VALUE_DATA) {
            Some(data) => plist_to_guest_value(env, data),
            None => plist_to_guest_value(env, &Value::Data(Vec::new())),
        }
    } else {
        nil
    }
}

fn write_result(env: &mut Environment, result: MutPtr<id>, items: &[Dictionary], query: &Query) {
    if result.is_null() {
        return;
    }
    let object = if !query.return_data && !query.return_attributes {
        nil
    } else if query.match_all {
        let objects = items.iter().map(|item| item_result(env, item, query)).collect();
        ns_array::from_vec(env, objects)
    } else {
        item_result(env, &items[0], query)
    };
    env.mem.write(result, object);
}

fn SecItemAdd(env: &mut Environment, attributes: id, result: MutPtr<id>) -> OSStatus {
    if !result.is_null() {
        env.mem.write(result, nil);
    }
    if attributes == nil {
        return errSecParam;
    }
    let query = parse_query(env, attributes);
    let mut item = Dictionary::new();
    for (key, value) in &query.attributes {
        item.insert(key.clone(), value.clone());
    }
    let data_key = ns_string::get_static_str(env, VALUE_DATA);
    let data: id = msg![env; attributes objectForKey:data_key];
    if data != nil {
        let data = guest_value_to_plist(env, data);
        item.insert(VALUE_DATA.to_string(), data);
    }

    let mut items = load_items(env);
    // An item's identity is its class plus its primary-key attributes; for the
    // item classes apps use, every attribute given at creation is close
    // enough, minus the descriptive ones.
    let identity: Vec<(String, Value)> = query
        .attributes
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                "kSecAttrLabel" | "kSecAttrComment" | "kSecAttrDescription" | "kSecAttrGeneric"
                    | "kSecAttrAccessible"
            )
        })
        .cloned()
        .collect();
    if items.iter().any(|existing| item_matches(existing, &identity)) {
        log_dbg!("SecItemAdd({:?}) => errSecDuplicateItem", item);
        return errSecDuplicateItem;
    }
    write_result(env, result, std::slice::from_ref(&item), &query);
    items.push(item);
    save_items(env, &items);
    log_dbg!("SecItemAdd() => errSecSuccess");
    errSecSuccess
}

fn SecItemCopyMatching(env: &mut Environment, query: id, result: MutPtr<id>) -> OSStatus {
    if !result.is_null() {
        env.mem.write(result, nil);
    }
    if query == nil {
        return errSecParam;
    }
    let query = parse_query(env, query);
    let items: Vec<Dictionary> = load_items(env)
        .into_iter()
        .filter(|item| item_matches(item, &query.attributes))
        .collect();
    log_dbg!(
        "SecItemCopyMatching({:?}) => {} item(s)",
        query.attributes,
        items.len()
    );
    if items.is_empty() {
        return errSecItemNotFound;
    }
    write_result(env, result, &items, &query);
    errSecSuccess
}

fn SecItemUpdate(env: &mut Environment, query: id, attributes_to_update: id) -> OSStatus {
    if query == nil || attributes_to_update == nil {
        return errSecParam;
    }
    let query = parse_query(env, query);
    let mut updates = Vec::new();
    for (key, value) in read_guest_dict(env, attributes_to_update) {
        if is_control_key(&key) {
            continue;
        }
        let value = guest_value_to_plist(env, value);
        updates.push((key, value));
    }
    let mut items = load_items(env);
    let mut found = false;
    for item in items
        .iter_mut()
        .filter(|item| item_matches(item, &query.attributes))
    {
        found = true;
        for (key, value) in &updates {
            item.insert(key.clone(), value.clone());
        }
    }
    if !found {
        return errSecItemNotFound;
    }
    save_items(env, &items);
    errSecSuccess
}

fn SecItemDelete(env: &mut Environment, query: id) -> OSStatus {
    if query == nil {
        return errSecParam;
    }
    let query = parse_query(env, query);
    let mut items = load_items(env);
    let before = items.len();
    items.retain(|item| !item_matches(item, &query.attributes));
    if items.len() == before {
        return errSecItemNotFound;
    }
    save_items(env, &items);
    errSecSuccess
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(SecItemAdd(_, _)),
    export_c_func!(SecItemCopyMatching(_, _)),
    export_c_func!(SecItemUpdate(_, _)),
    export_c_func!(SecItemDelete(_)),
];

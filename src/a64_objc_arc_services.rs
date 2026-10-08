/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! objc_storeStrong and registered-class lookup/introspection services over
//! the existing Registry and Lifetime. Only raw-pointer isa values, registered
//! class objects and Lifetime-owned instances are accepted; tagged pointers,
//! encoded isa, weak references and unknown objects are explicit errors.
//! objc_sync_enter/exit are deliberately absent until real thread ownership,
//! recursion, blocking and wakeup exist.
//! References: apple-oss-distributions/objc4 runtime/NSObject.mm
//! (objc_storeStrong, objc_opt_class, objc_opt_isKindOfClass) and
//! runtime/objc-runtime.mm (objc_getClass, objc_lookUpClass, object_getClass).

use super::bridge::{GuestBridge, GuestCall, ReturnValues, ServiceFrame, ServiceId};
use super::objc_execution_services::ObjectRuntime;
use super::objc_lifetime::Lifetime;
use super::objc_metadata::{Class, Registry};
use super::A64Cpu;
use std::cell::RefCell;
use std::rc::Rc;

/// Class data RO_META bit, as decoded by the metadata Registry.
const META: u32 = 1;
/// Class names are bounded like bundle principal-class names.
const MAX_CLASS_NAME: usize = 4096;
const NAME_CHUNK: usize = 64;
/// Matches the Registry's class graph limit.
const MAX_CHAIN: usize = 4096;

/// Register the services. `release_entry` must be the guest address of the
/// already registered `_objc_release` service of this bridge (lifetime-only or
/// the execution runtime's upgraded release). objc_storeStrong releases the old
/// value through it, so a final release performs exactly what that service
/// performs (real guest -dealloc, or an explicit error) and is never faked.
/// `registry` must be the Registry whose classes the Lifetime knows as class
/// objects; objects outside it and outside the Lifetime are rejected.
pub(super) fn install(
    bridge: &mut GuestBridge,
    cpu: &mut A64Cpu,
    runtime: Rc<RefCell<ObjectRuntime>>,
    release_entry: u64,
) -> Result<Vec<(&'static str, ServiceId)>, String> {
    bridge.validate_registered_service(cpu, "_objc_release", release_entry)?;
    let lifetime = runtime
        .try_borrow()
        .map_err(|_| "reentrant ARC installation")?
        .lifetime
        .clone();
    let mut bindings = Vec::new();

    let store_lifetime = lifetime.clone();
    bindings.push((
        "_objc_storeStrong",
        bridge.register_service(cpu, "_objc_storeStrong", move |frame| {
            store_strong(frame, &store_lifetime, release_entry)
        })?,
    ));

    // objc4's objc_getClass additionally consults the getClass hook/class
    // handler; none is installed or supported here, so both are identical
    // lookups of a registered, non-meta class name. Neither initializes.
    for symbol in ["_objc_getClass", "_objc_lookUpClass"] {
        let runtime = runtime.clone();
        bindings.push((
            symbol,
            bridge.register_service(cpu, symbol, move |frame| {
                let name = frame.integer(0)?;
                let registry = active_registry(&runtime)?;
                Ok(ReturnValues::integer(lookup_class(frame, &registry, name)?))
            })?,
        ));
    }

    let (get_registry, get_lifetime) = (runtime.clone(), lifetime.clone());
    bindings.push((
        "_object_getClass",
        bridge.register_service(cpu, "_object_getClass", move |frame| {
            let object = frame.integer(0)?;
            if object == 0 {
                return Ok(ReturnValues::integer(0));
            }
            let registry = active_registry(&get_registry)?;
            let isa = object_isa(frame, &registry, &get_lifetime, object)?;
            Ok(ReturnValues::integer(isa))
        })?,
    ));

    let (class_registry, class_lifetime) = (runtime.clone(), lifetime.clone());
    bindings.push((
        "_objc_opt_class",
        bridge.register_service(cpu, "_objc_opt_class", move |frame| {
            let object = frame.integer(0)?;
            if object == 0 {
                return Ok(ReturnValues::integer(0));
            }
            let registry = active_registry(&class_registry)?;
            let isa = object_isa(frame, &registry, &class_lifetime, object)?;
            let chain = superclass_chain(&registry, isa)?;
            reject_custom_core(&chain, "objc_opt_class")?;
            // Default +class returns self; default -class returns the isa.
            let result = if chain[0].flags & META != 0 {
                object
            } else {
                isa
            };
            Ok(ReturnValues::integer(result))
        })?,
    ));

    let (kind_registry, kind_lifetime) = (runtime, lifetime);
    bindings.push((
        "_objc_opt_isKindOfClass",
        bridge.register_service(cpu, "_objc_opt_isKindOfClass", move |frame| {
            let object = frame.integer(0)?;
            let other = frame.integer(1)?;
            if object == 0 {
                return Ok(ReturnValues::integer(0));
            }
            let registry = active_registry(&kind_registry)?;
            let isa = object_isa(frame, &registry, &kind_lifetime, object)?;
            let chain = superclass_chain(&registry, isa)?;
            reject_custom_core(&chain, "objc_opt_isKindOfClass")?;
            // The receiver's complete chain is registered, so an unregistered
            // `other` genuinely cannot occur in it and the answer is NO.
            let found = chain.iter().any(|class| class.address == other);
            Ok(ReturnValues::integer(found as u64))
        })?,
    ));
    Ok(bindings)
}

fn active_registry(runtime: &RefCell<ObjectRuntime>) -> Result<Rc<Registry>, String> {
    Ok(runtime
        .try_borrow()
        .map_err(|_| "reentrant ARC registry lookup")?
        .registry
        .clone())
}

fn read_u64(frame: &mut ServiceFrame<'_>, address: u64) -> Result<u64, String> {
    let bytes: [u8; 8] = frame
        .read(address, 8)?
        .try_into()
        .map_err(|_| "short Objective-C pointer read".to_string())?;
    Ok(u64::from_le_bytes(bytes))
}

/// objc4: `if (obj == prev) return; objc_retain(obj); *location = obj;
/// objc_release(prev);`. Both values and the slot are validated before any
/// change. The old value's release is queued as a real guest call to the
/// registered release service; it runs only if this handler succeeds.
fn store_strong(
    frame: &mut ServiceFrame<'_>,
    lifetime: &RefCell<Lifetime>,
    release_entry: u64,
) -> Result<ReturnValues, String> {
    let location = frame.integer(0)?;
    let object = frame.integer(1)?;
    if location == 0 || location & 7 != 0 || location >> 63 != 0 {
        return Err(format!(
            "objc_storeStrong location {location:#x} is null, unaligned or tagged"
        ));
    }
    let previous = read_u64(frame, location)?;
    let mut lifetime = lifetime
        .try_borrow_mut()
        .map_err(|_| "reentrant Objective-C lifetime service is unsupported")?;
    lifetime.check_object(object)?;
    lifetime.check_object(previous)?;
    if object == previous {
        return Ok(ReturnValues::integer(0));
    }
    // Rewriting the current value proves the slot writable without changing it.
    frame.write(location, &previous.to_le_bytes())?;
    if previous != 0 {
        frame.request_guest_call(
            GuestCall {
                entry: release_entry,
                integers: vec![previous],
                ..Default::default()
            },
            |result| result.map(|_| ()),
        )?;
    }
    lifetime.retain(object)?;
    if let Err(error) = frame.write(location, &object.to_le_bytes()) {
        lifetime
            .release(object)
            .map_err(|rollback| format!("{error}; objc_storeStrong retain rollback: {rollback}"))?;
        return Err(error);
    }
    Ok(ReturnValues::integer(0))
}

/// Bounded C-string read. Chunks never cross a 4KiB boundary; a chunk that
/// still meets an unreadable byte falls back to exact single-byte reads, so
/// only a genuinely unreadable byte before the terminator is an error.
fn read_c_string(frame: &mut ServiceFrame<'_>, address: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut cursor = address;
    while bytes.len() < MAX_CLASS_NAME {
        let to_page = 4096 - (cursor & 4095) as usize;
        let want = NAME_CHUNK.min(to_page).min(MAX_CLASS_NAME - bytes.len());
        let chunk = match frame.read(cursor, want) {
            Ok(chunk) => chunk,
            Err(_) => frame.read(cursor, 1)?,
        };
        if let Some(end) = chunk.iter().position(|&b| b == 0) {
            bytes.extend_from_slice(&chunk[..end]);
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk);
        cursor = cursor
            .checked_add(chunk.len() as u64)
            .ok_or("Objective-C class name address overflow")?;
    }
    Err("Objective-C class name exceeds 4096 bytes without a terminator".into())
}

fn lookup_class(
    frame: &mut ServiceFrame<'_>,
    registry: &Registry,
    name: u64,
) -> Result<u64, String> {
    if name == 0 {
        return Ok(0); // objc4 returns Nil for a NULL name.
    }
    let bytes = read_c_string(frame, name)?;
    // A non-UTF-8 name cannot equal any registered class name.
    Ok(std::str::from_utf8(&bytes)
        .ok()
        .and_then(|name| registry.lookup_class(name))
        .unwrap_or(0))
}

fn registered_class(registry: &Registry, address: u64) -> Option<&Class> {
    registry.classes().find(|class| class.address == address)
}

/// The raw-pointer isa of a registered class object (its metaclass) or of a
/// Lifetime-owned instance (a registered normal class). Anything else, and any
/// mismatch between guest memory and the registered graph, is an error.
fn object_isa(
    frame: &mut ServiceFrame<'_>,
    registry: &Registry,
    lifetime: &RefCell<Lifetime>,
    object: u64,
) -> Result<u64, String> {
    if object & 7 != 0 || object >> 63 != 0 {
        return Err(format!(
            "unsupported tagged or unaligned Objective-C object {object:#x}"
        ));
    }
    let class_object = registered_class(registry, object);
    if class_object.is_none()
        && !lifetime
            .try_borrow()
            .map_err(|_| "reentrant Objective-C lifetime service is unsupported")?
            .contains_identity(object)
    {
        return Err(format!(
            "unknown Objective-C object {object:#x} is neither a registered class nor an owned instance"
        ));
    }
    let isa = read_u64(frame, object)?;
    if isa & 7 != 0 {
        return Err(format!(
            "unsupported encoded Objective-C isa {isa:#x} for object {object:#x}"
        ));
    }
    let isa_class = registered_class(registry, isa)
        .ok_or_else(|| format!("Objective-C object {object:#x} has unregistered isa {isa:#x}"))?;
    let consistent = match class_object {
        Some(class) => class.isa == isa && isa_class.flags & META != 0,
        None => isa_class.flags & META == 0,
    };
    if !consistent {
        return Err(format!(
            "Objective-C object {object:#x} isa {isa:#x} contradicts the registered class graph"
        ));
    }
    Ok(isa)
}

fn superclass_chain(registry: &Registry, start: u64) -> Result<Vec<&Class>, String> {
    let mut chain = Vec::new();
    let mut cursor = start;
    while cursor != 0 {
        if chain.len() >= MAX_CHAIN {
            return Err("Objective-C superclass chain limit exceeded".into());
        }
        let class = registered_class(registry, cursor)
            .ok_or_else(|| format!("unregistered Objective-C superclass {cursor:#x}"))?;
        chain.push(class);
        cursor = class.superclass;
    }
    if chain.is_empty() {
        return Err("Objective-C object has no class".into());
    }
    Ok(chain)
}

/// objc4's opt fast paths apply only without custom core methods (+new,
/// ±class, ±self, ±isKindOfClass:, ±respondsToSelector:). Otherwise objc4
/// sends a real message; that slow path is not implemented here, so a custom
/// core override is an explicit error rather than a guessed answer.
fn reject_custom_core(chain: &[&Class], service: &str) -> Result<(), String> {
    for class in chain {
        for method in &class.methods {
            let core = matches!(
                method.selector.as_str(),
                "class" | "self" | "isKindOfClass:" | "respondsToSelector:"
            ) || (class.flags & META != 0 && method.selector == "new");
            if core {
                return Err(format!(
                    "{service}: class {} overrides core method {}; message-send slow path is not implemented",
                    class.name, method.selector
                ));
            }
        }
    }
    Ok(())
}

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Explicit ARC service bindings for bridge-owned, registered guest identities.
//! No class allocation/initialization, weak references or dealloc is fabricated.

use super::bridge::{GuestBridge, ReturnValues, ServiceId};
use super::objc_lifetime::Lifetime;
use super::A64Cpu;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy)]
enum Operation {
    Retain,
    Release,
    Autorelease,
    RetainAutorelease,
    CheckIdentity,
    PushPool,
    PopPool,
}

/// Returns only symbols genuinely implemented by this ownership backend.
/// Unknown Apple objects, no-pool autorelease, and final guest -dealloc are
/// explicit errors. The caller must keep this single-thread backend isolated
/// from Apple's cached runtime and route imports deliberately, not by fallback.
pub(super) fn install(
    bridge: &mut GuestBridge,
    cpu: &mut A64Cpu,
    lifetime: Rc<RefCell<Lifetime>>,
) -> Result<Vec<(&'static str, ServiceId)>, String> {
    install_inner(bridge, cpu, lifetime, true)
}

/// Execution runtime supplies the real final-release service; avoid duplicate
/// symbol ownership while preserving all other ARC/pool semantics.
pub(super) fn install_without_release(
    bridge: &mut GuestBridge,
    cpu: &mut A64Cpu,
    lifetime: Rc<RefCell<Lifetime>>,
) -> Result<Vec<(&'static str, ServiceId)>, String> {
    install_inner(bridge, cpu, lifetime, false)
}

fn install_inner(
    bridge: &mut GuestBridge,
    cpu: &mut A64Cpu,
    lifetime: Rc<RefCell<Lifetime>>,
    include_release: bool,
) -> Result<Vec<(&'static str, ServiceId)>, String> {
    let mut bindings = Vec::new();
    for (symbol, operation) in [
        ("_objc_retain", Operation::Retain),
        ("_objc_release", Operation::Release),
        ("_objc_autorelease", Operation::Autorelease),
        ("_objc_autoreleaseReturnValue", Operation::Autorelease),
        ("_objc_retainAutoreleasedReturnValue", Operation::Retain),
        ("_objc_retainAutorelease", Operation::RetainAutorelease),
        (
            "_objc_retainAutoreleaseReturnValue",
            Operation::RetainAutorelease,
        ),
        (
            "_objc_unsafeClaimAutoreleasedReturnValue",
            Operation::CheckIdentity,
        ),
        ("_objc_autoreleasePoolPush", Operation::PushPool),
        ("_objc_autoreleasePoolPop", Operation::PopPool),
    ] {
        if !include_release && symbol == "_objc_release" {
            continue;
        }
        let lifetime = lifetime.clone();
        let service = bridge.register_service(cpu, symbol, move |frame| {
            let mut lifetime = lifetime
                .try_borrow_mut()
                .map_err(|_| "reentrant Objective-C lifetime service is unsupported")?;
            let object = frame.integer(0)?;
            let result = match operation {
                Operation::Retain => lifetime.retain(object)?,
                Operation::Release => {
                    lifetime.release(object)?;
                    0
                }
                Operation::Autorelease => lifetime.autorelease(object)?,
                Operation::RetainAutorelease => lifetime.retain_autorelease(object)?,
                // The unoptimized producer already enqueued its +0 return;
                // unsafeClaim consequently neither retains nor releases it.
                Operation::CheckIdentity => {
                    lifetime.check_object(object)?;
                    object
                }
                Operation::PushPool => lifetime.push_pool()?,
                Operation::PopPool => {
                    lifetime.pop_pool(object)?;
                    0
                }
            };
            Ok(ReturnValues::integer(result))
        })?;
        bindings.push((symbol, service));
    }
    Ok(bindings)
}

#[cfg(test)]
mod tests {
    use super::super::bridge::GuestCall;
    use super::super::objc_lifetime::ReleasePlan;
    use super::*;

    #[test]
    fn real_guest_trampolines_execute_terraria_arc_return_and_pool_sequence() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        cpu.write_guest_into(0x40000, &0x40040u64.to_le_bytes())
            .unwrap();
        let mut bridge = GuestBridge::map(&mut cpu, 0x50000).unwrap();
        let lifetime = Rc::new(RefCell::new(Lifetime::default()));
        // Independent owned fixture allocation, not an imported Apple object.
        lifetime
            .borrow_mut()
            .register_object(0x40000, 0x40040, 1)
            .unwrap();
        let bindings = install(&mut bridge, &mut cpu, lifetime.clone()).unwrap();
        let entry = |name: &str| {
            bindings
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap()
                .1
                .guest_address()
        };
        let mut call = |name: &str, argument| {
            bridge.call(
                &mut cpu,
                &GuestCall {
                    entry: entry(name),
                    integers: vec![argument],
                    ..Default::default()
                },
                100,
            )
        };
        let cookie = call("_objc_autoreleasePoolPush", 0).unwrap().integers[0];
        assert_eq!(
            call("_objc_autoreleaseReturnValue", 0x40000)
                .unwrap()
                .integers[0],
            0x40000
        );
        assert_eq!(
            call("_objc_retainAutoreleasedReturnValue", 0x40000)
                .unwrap()
                .integers[0],
            0x40000
        );
        call("_objc_autoreleasePoolPop", cookie).unwrap();
        assert_eq!(
            lifetime.borrow().plan_release(0x40000).unwrap(),
            ReleasePlan::Deallocate {
                object: 0x40000,
                class: 0x40040
            }
        );
        assert!(call("_objc_release", 0x40000)
            .unwrap_err()
            .contains("guest -dealloc"));
        assert!(call("_objc_retain", 0x40080)
            .unwrap_err()
            .contains("unregistered"));
        assert_eq!(
            call("_objc_autoreleaseReturnValue", 0).unwrap().integers[0],
            0
        );
    }
}

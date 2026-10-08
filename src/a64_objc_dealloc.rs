/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Final-release execution for an explicitly initialized, registered guest
//! instance. This does not implement NSObject destruction or Apple weak tables.
//! The trusted allocation/destructor route must record actual disposal using
//! the returned ticket; an IMP return is deliberately not a disposal receipt.
use super::bridge::{GuestCall, ServiceFrame};
use super::objc_lifetime::{DeallocationTicket, Lifetime, ReleasePlan};
use super::objc_metadata::Invocation;
use std::cell::RefCell;
use std::rc::Rc;

fn validate_invocation(
    invocation: &Invocation,
    object: u64,
    class: u64,
    dealloc_selector: u64,
    initialized: bool,
) -> Result<(), String> {
    if !initialized {
        return Err("guest Objective-C class initialization is not complete".into());
    }
    if object == 0
        || object & 7 != 0
        || class == 0
        || class & 7 != 0
        || dealloc_selector == 0
        || invocation.receiver != object
        || invocation.receiver_class != class
        || invocation.selector != dealloc_selector
        || invocation.implementation == 0
        || invocation.implementation & 3 != 0
    {
        return Err("guest -dealloc invocation does not match reserved instance".into());
    }
    // Apple LP64 ordinary void -(void)method has two pointer parameters.
    // Compact encodings omit frame/argument offsets. No extra arguments,
    // forwarding trampoline, stret or guessed return ABI is accepted here.
    if invocation.types != "v16@0:8" && invocation.types != "v@:" {
        return Err(format!(
            "unsupported guest -dealloc type encoding {}",
            invocation.types
        ));
    }
    Ok(())
}

/// Invocation must originate from Registry::plan_message for the canonical
/// selector named "dealloc", with current executable mapping validation.
/// `initialized` is actual caller runtime state, never inferred from metadata.
/// The bridge validates executable memory again and supplies disjoint bounded
/// guest stacks. Callers must route the returned ticket to a real destruction
/// handler while the IMP is running. Without that handler, completion fails.
pub(super) fn schedule_final_release(
    frame: &mut ServiceFrame<'_>,
    lifetime: Rc<RefCell<Lifetime>>,
    invocation: &Invocation,
    dealloc_selector: u64,
    initialized: bool,
) -> Result<DeallocationTicket, String> {
    let (object, class) = {
        let state = lifetime
            .try_borrow()
            .map_err(|_| "reentrant Objective-C lifetime access")?;
        match state.plan_release(invocation.receiver)? {
            ReleasePlan::Deallocate { object, class } => (object, class),
            _ => return Err("guest -dealloc execution requires a final release".into()),
        }
    };
    validate_invocation(invocation, object, class, dealloc_selector, initialized)?;
    let ticket = lifetime
        .try_borrow_mut()
        .map_err(|_| "reentrant Objective-C lifetime access")?
        .begin_deallocation(object)?;
    let completion_lifetime = lifetime.clone();
    let queued = frame.request_guest_call(
        GuestCall {
            entry: invocation.implementation,
            integers: vec![object, dealloc_selector],
            ..GuestCall::default()
        },
        move |result| {
            completion_lifetime
                .try_borrow_mut()
                .map_err(|_| "reentrant Objective-C deallocation completion")?
                .finish_deallocation(ticket, result.map(|_| ()))
        },
    );
    if let Err(error) = queued {
        lifetime
            .try_borrow_mut()
            .map_err(|_| "reentrant Objective-C lifetime cancellation")?
            .cancel_deallocation(ticket)?;
        return Err(error);
    }
    // request_guest_call only queues; no guest instruction has run yet.
    lifetime
        .try_borrow_mut()
        .map_err(|_| "reentrant Objective-C lifetime start")?
        .mark_deallocation_started(ticket)?;
    Ok(ticket)
}

#[cfg(test)]
mod tests {
    use super::super::bridge::{GuestBridge, ReturnValues};
    use super::super::A64Cpu;
    use super::*;
    fn invocation() -> Invocation {
        Invocation {
            receiver: 0x1000,
            receiver_class: 0x2000,
            selector: 0x3000,
            implementation: 0x4000,
            method_owner: 0x2000,
            types: "v16@0:8".into(),
        }
    }
    #[test]
    fn validates_real_void_instance_call_and_initialization_policy() {
        let mut plan = invocation();
        validate_invocation(&plan, 0x1000, 0x2000, 0x3000, true).unwrap();
        assert!(validate_invocation(&plan, 0x1000, 0x2000, 0x3000, false).is_err());
        assert!(validate_invocation(&plan, 0x1000, 0x2008, 0x3000, true).is_err());
        assert!(validate_invocation(&plan, 0x1000, 0x2000, 0x3008, true).is_err());
        plan.types = "v24@0:8@16".into();
        assert!(validate_invocation(&plan, 0x1000, 0x2000, 0x3000, true).is_err());
        plan.types = "v@:".into();
        validate_invocation(&plan, 0x1000, 0x2000, 0x3000, true).unwrap();
        plan.implementation |= 1;
        assert!(validate_invocation(&plan, 0x1000, 0x2000, 0x3000, true).is_err());
    }

    #[test]
    fn guest_dealloc_calls_explicit_fixture_allocator_disposal() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        cpu.write_guest_into(0x40000, &0x40040u64.to_le_bytes())
            .unwrap();
        let mut bridge = GuestBridge::map(&mut cpu, 0x50000).unwrap();
        let lifetime = Rc::new(RefCell::new(Lifetime::default()));
        lifetime
            .borrow_mut()
            .register_object(0x40000, 0x40040, 1)
            .unwrap();
        let receipt = Rc::new(RefCell::new(None::<DeallocationTicket>));
        let disposal_lifetime = lifetime.clone();
        let disposal_receipt = receipt.clone();
        let disposer = bridge
            .register_service(&mut cpu, "fixture_allocator_dispose", move |frame| {
                let ticket = disposal_receipt
                    .borrow()
                    .ok_or("fixture disposal ticket missing")?;
                if frame.integer(0)? != ticket.object() {
                    return Err("fixture disposal receiver mismatch".into());
                }
                // This fixture allocator has one eight-byte object, no weak table
                // or strong ivars. Clear its storage before issuing the receipt.
                frame.write(ticket.object(), &[0; 8])?;
                disposal_lifetime.borrow_mut().record_disposal(ticket)?;
                Ok(ReturnValues::integer(0))
            })
            .unwrap();
        let mut code = Vec::new();
        for word in [
            0xa9bf7bfdu32,
            0x910003fd,
            0x580000d0,
            0xd63f0200,
            0xa8c17bfd,
            0xd65f03c0,
            0xd503201f,
            0xd503201f,
        ] {
            code.extend_from_slice(&word.to_le_bytes());
        }
        code.extend_from_slice(&disposer.guest_address().to_le_bytes());
        cpu.map_zeroed(0x70000, 4096, 7).unwrap();
        cpu.write_guest_into(0x70000, &code).unwrap();
        let release_lifetime = lifetime.clone();
        let release = bridge
            .register_service(&mut cpu, "fixture_final_release", move |frame| {
                let plan = Invocation {
                    receiver: frame.integer(0)?,
                    receiver_class: 0x40040,
                    selector: 0x40080,
                    implementation: 0x70000,
                    types: "v16@0:8".into(),
                    method_owner: 0x40040,
                };
                let ticket =
                    schedule_final_release(frame, release_lifetime.clone(), &plan, 0x40080, true)?;
                *receipt.borrow_mut() = Some(ticket);
                Ok(ReturnValues::integer(0))
            })
            .unwrap();
        bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: release.guest_address(),
                    integers: vec![0x40000],
                    ..GuestCall::default()
                },
                1000,
            )
            .unwrap();
        let mut bytes = [0xff; 8];
        cpu.read_guest_into(0x40000, &mut bytes).unwrap();
        assert_eq!(bytes, [0; 8]);
        assert!(lifetime.borrow().check_object(0x40000).is_err());
        lifetime
            .borrow_mut()
            .register_object(0x40000, 0x40040, 1)
            .unwrap();
    }
}

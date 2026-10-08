/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bounded single-guest-thread ARC ownership and autorelease pools.
//!
//! Objects enter only through an allocator/bridge that has verified their guest
//! allocation and registered class identity. This is not a decoder for Apple's
//! private packed refcounts, weak tables, tagged objects or CF host handles.
//! Final guest -dealloc requires a real synchronous guest call; until that
//! bridge exists it is an explicit error, with ownership/pool state unchanged.
//! ARC return-value helpers use the valid unoptimized retain/autorelease path.
//! Reference: apple-oss-distributions/objc4/runtime/NSObject.mm.

use std::collections::BTreeMap;

const MAX_OBJECTS: usize = 65536;
const MAX_POOLS: usize = 1024;
const MAX_AUTORELEASES: usize = 65536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ownership {
    Managed {
        class: u64,
        references: u32,
    },
    Immortal,
    Deallocating {
        ticket: u64,
        started: bool,
        disposed: bool,
        failed: bool,
    },
}

/// A reservation is not evidence that guest storage has been disposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DeallocationTicket {
    object: u64,
    class: u64,
    serial: u64,
}
impl DeallocationTicket {
    pub(super) fn object(self) -> u64 {
        self.object
    }
    pub(super) fn class(self) -> u64 {
        self.class
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReleasePlan {
    NoOp,
    Decrement { remaining: u32 },
    Deallocate { object: u64, class: u64 },
}

#[derive(Debug, Clone)]
struct Pool {
    token: u64,
    objects: Vec<u64>,
}

#[derive(Debug, Default, Clone)]
pub(super) struct Lifetime {
    objects: BTreeMap<u64, Ownership>,
    pools: Vec<Pool>,
    next_token: u64,
    queued: usize,
    next_deallocation: u64,
}

fn valid_identity(address: u64) -> Result<(), String> {
    if address == 0 || address & 7 != 0 || address >> 63 != 0 {
        Err("unsupported null, tagged or unaligned Objective-C ownership identity".into())
    } else {
        Ok(())
    }
}

impl Lifetime {
    /// Stage known, validated class/metaclass identities without altering live
    /// ownership. Existing immortal identities are idempotent; managed or
    /// deallocating addresses are collisions, never promoted to immortality.
    /// The publication caller holds the live mutable borrow until commit.
    pub(super) fn with_class_identities(&self, identities: &[u64]) -> Result<Self, String> {
        if identities.len() > MAX_OBJECTS {
            return Err("Objective-C class ownership enrollment limit exceeded".into());
        }
        let mut staged = self.clone();
        for &identity in identities {
            valid_identity(identity)?;
            match staged.objects.get(&identity) {
                Some(Ownership::Immortal) => {}
                Some(_) => return Err("Objective-C class ownership identity collision".into()),
                None => staged.insert(identity, Ownership::Immortal)?,
            }
        }
        Ok(staged)
    }

    pub(super) fn contains_identity(&self, object: u64) -> bool {
        self.objects.contains_key(&object)
    }

    pub(super) fn validate_disposal(&self, ticket: DeallocationTicket) -> Result<(), String> {
        match self.objects.get(&ticket.object) {
            Some(Ownership::Deallocating {
                ticket: serial,
                started: true,
                disposed: false,
                failed: false,
            }) if *serial == ticket.serial => Ok(()),
            _ => Err("invalid Objective-C disposal reservation".into()),
        }
    }
    /// Caller must have allocated real guest storage and initialized a class
    /// isa validated by the metadata registry, with ordinary NSObject ownership
    /// (custom retain/release implementations require guest dispatch instead).
    /// Never auto-register an arbitrary
    /// incoming object pointer merely to make retain/release succeed.
    pub(super) fn register_object(
        &mut self,
        object: u64,
        class: u64,
        references: u32,
    ) -> Result<(), String> {
        valid_identity(object)?;
        valid_identity(class)?;
        if references == 0 {
            return Err("Objective-C object must have initial ownership".into());
        }
        self.insert(object, Ownership::Managed { class, references })
    }

    /// Known class objects may have immortal ownership; foreign Foundation/CF
    /// constants are not inferred to be immortal from their address or name.
    pub(super) fn register_immortal(&mut self, object: u64) -> Result<(), String> {
        valid_identity(object)?;
        self.insert(object, Ownership::Immortal)
    }

    fn insert(&mut self, object: u64, ownership: Ownership) -> Result<(), String> {
        if self.objects.contains_key(&object) {
            return Err("Objective-C ownership identity already registered".into());
        }
        if self.objects.len() >= MAX_OBJECTS {
            return Err("Objective-C ownership object limit exceeded".into());
        }
        self.objects.insert(object, ownership);
        Ok(())
    }

    pub(super) fn check_object(&self, object: u64) -> Result<(), String> {
        if matches!(
            self.objects.get(&object),
            Some(Ownership::Deallocating { .. })
        ) {
            Err("Objective-C object is deallocating or quarantined".into())
        } else if object == 0 || self.objects.contains_key(&object) {
            Ok(())
        } else {
            Err(format!(
                "unregistered Objective-C ownership object {object:#x}"
            ))
        }
    }

    pub(super) fn retain(&mut self, object: u64) -> Result<u64, String> {
        self.check_object(object)?;
        if let Some(Ownership::Managed { references, .. }) = self.objects.get_mut(&object) {
            *references = references
                .checked_add(1)
                .ok_or("Objective-C reference count overflow")?;
        }
        Ok(object)
    }

    /// A read-only plan. Deallocate must not be reported as a completed release
    /// until actual -dealloc, weak clearing and allocation disposal are wired.
    pub(super) fn plan_release(&self, object: u64) -> Result<ReleasePlan, String> {
        self.check_object(object)?;
        Ok(match self.objects.get(&object) {
            None | Some(Ownership::Immortal) => ReleasePlan::NoOp,
            Some(Ownership::Managed { references, .. }) if *references > 1 => {
                ReleasePlan::Decrement {
                    remaining: *references - 1,
                }
            }
            Some(Ownership::Managed { class, .. }) => ReleasePlan::Deallocate {
                object,
                class: *class,
            },
            Some(Ownership::Deallocating { .. }) => unreachable!(),
        })
    }

    /// Reserve the final reference before a real guest call. Retain/release and
    /// autorelease now reject this object, preventing resurrection/reentrancy.
    pub(super) fn begin_deallocation(&mut self, object: u64) -> Result<DeallocationTicket, String> {
        let ReleasePlan::Deallocate { class, .. } = self.plan_release(object)? else {
            return Err("Objective-C deallocation requires the final reference".into());
        };
        let serial = self
            .next_deallocation
            .checked_add(1)
            .ok_or("Objective-C deallocation ticket overflow")?;
        self.next_deallocation = serial;
        self.objects.insert(
            object,
            Ownership::Deallocating {
                ticket: serial,
                started: false,
                disposed: false,
                failed: false,
            },
        );
        Ok(DeallocationTicket {
            object,
            class,
            serial,
        })
    }

    /// Only use when enqueue failed before any guest instruction executed.
    pub(super) fn cancel_deallocation(&mut self, ticket: DeallocationTicket) -> Result<(), String> {
        if self.objects.get(&ticket.object)
            != Some(&Ownership::Deallocating {
                ticket: ticket.serial,
                started: false,
                disposed: false,
                failed: false,
            })
        {
            return Err("invalid Objective-C deallocation cancellation".into());
        }
        self.objects.insert(
            ticket.object,
            Ownership::Managed {
                class: ticket.class,
                references: 1,
            },
        );
        Ok(())
    }

    pub(super) fn mark_deallocation_started(
        &mut self,
        ticket: DeallocationTicket,
    ) -> Result<(), String> {
        let Some(Ownership::Deallocating {
            ticket: serial,
            started,
            failed,
            ..
        }) = self.objects.get_mut(&ticket.object)
        else {
            return Err("unreserved Objective-C deallocation start".into());
        };
        if *serial != ticket.serial || *started || *failed {
            return Err("invalid Objective-C deallocation start".into());
        }
        *started = true;
        Ok(())
    }

    /// Trusted destruction route calls this only AFTER real ivar/weak cleanup
    /// and allocation disposal. Guest -dealloc returning alone is insufficient.
    pub(super) fn record_disposal(&mut self, ticket: DeallocationTicket) -> Result<(), String> {
        self.validate_disposal(ticket)?;
        let Some(Ownership::Deallocating {
            ticket: serial,
            started,
            disposed,
            failed,
        }) = self.objects.get_mut(&ticket.object)
        else {
            return Err("unreserved Objective-C disposal".into());
        };
        if *serial != ticket.serial || !*started || *disposed || *failed {
            return Err("invalid or repeated Objective-C disposal receipt".into());
        }
        *disposed = true;
        Ok(())
    }

    /// Guest execution cannot be rolled back. On failure or missing disposal
    /// receipt retain a quarantined tombstone; never fabricate a completed free.
    pub(super) fn finish_deallocation(
        &mut self,
        ticket: DeallocationTicket,
        guest_result: Result<(), String>,
    ) -> Result<(), String> {
        let Some(Ownership::Deallocating {
            ticket: serial,
            started,
            disposed,
            failed,
        }) = self.objects.get_mut(&ticket.object)
        else {
            return Err("unreserved Objective-C deallocation completion".into());
        };
        if *serial != ticket.serial || !*started || *failed {
            return Err("invalid Objective-C deallocation completion".into());
        }
        let error = guest_result.err().or_else(|| {
            (!*disposed)
                .then(|| "guest -dealloc returned without verified allocation disposal".to_string())
        });
        if let Some(error) = error {
            *failed = true;
            return Err(format!("Objective-C deallocation quarantined: {error}"));
        }
        self.objects.remove(&ticket.object);
        Ok(())
    }

    pub(super) fn release(&mut self, object: u64) -> Result<(), String> {
        match self.plan_release(object)? {
            ReleasePlan::NoOp => Ok(()),
            ReleasePlan::Decrement { remaining } => {
                let Some(Ownership::Managed { references, .. }) = self.objects.get_mut(&object) else { unreachable!() };
                *references = remaining;
                Ok(())
            }
            ReleasePlan::Deallocate { object, class } => Err(format!("Objective-C object {object:#x} class {class:#x} requires synchronous guest -dealloc; ownership unchanged")),
        }
    }

    /// Opaque pool cookies are bridge-owned identities, not guest allocations.
    /// They must never be dereferenced or interpreted as an Apple pool page.
    pub(super) fn push_pool(&mut self) -> Result<u64, String> {
        if self.pools.len() >= MAX_POOLS {
            return Err("Objective-C autorelease pool depth limit exceeded".into());
        }
        let token = self
            .next_token
            .checked_add(1)
            .ok_or("Objective-C autorelease cookie overflow")?;
        self.next_token = token;
        self.pools.push(Pool {
            token,
            objects: Vec::new(),
        });
        Ok(token)
    }

    fn can_enqueue(&self, object: u64) -> Result<bool, String> {
        self.check_object(object)?;
        if object == 0 || self.objects.get(&object) == Some(&Ownership::Immortal) {
            return Ok(false);
        }
        if self.pools.is_empty() {
            return Err("Objective-C autorelease without a guest pool is unsupported".into());
        }
        if self.queued >= MAX_AUTORELEASES {
            return Err("Objective-C autorelease entry limit exceeded".into());
        }
        Ok(true)
    }

    pub(super) fn autorelease(&mut self, object: u64) -> Result<u64, String> {
        if self.can_enqueue(object)? {
            self.pools.last_mut().unwrap().objects.push(object);
            self.queued += 1;
        }
        Ok(object)
    }

    /// Validate enqueue and retain before mutating either, so pool/queue failures
    /// cannot leak a retain. This is the unoptimized ARC return handshake path.
    pub(super) fn retain_autorelease(&mut self, object: u64) -> Result<u64, String> {
        let enqueue = self.can_enqueue(object)?;
        self.retain(object)?;
        if enqueue {
            self.pools.last_mut().unwrap().objects.push(object);
            self.queued += 1;
        }
        Ok(object)
    }

    /// Drain the requested boundary and all nested pools. Release order is
    /// newest pool/object first. Preflight every release before changing state;
    /// pending guest deallocation or invalid cookies leave all pools intact.
    pub(super) fn pop_pool(&mut self, token: u64) -> Result<(), String> {
        let boundary = self
            .pools
            .iter()
            .position(|p| p.token == token)
            .ok_or("invalid or expired Objective-C autorelease cookie")?;
        let mut releases = BTreeMap::<u64, u32>::new();
        let mut drained = 0usize;
        for pool in self.pools[boundary..].iter().rev() {
            for &object in pool.objects.iter().rev() {
                let count = releases.entry(object).or_default();
                *count = count
                    .checked_add(1)
                    .ok_or("Objective-C pool release count overflow")?;
                drained += 1;
            }
        }
        for (&object, &count) in &releases {
            let Some(Ownership::Managed { class, references }) = self.objects.get(&object) else {
                return Err("Objective-C autorelease object ownership changed".into());
            };
            if count > *references {
                return Err("Objective-C autorelease exceeds retained ownership".into());
            }
            if count == *references {
                return Err(format!("Objective-C pool object {object:#x} class {class:#x} requires synchronous guest -dealloc; pool unchanged"));
            }
        }
        for (object, count) in releases {
            let Some(Ownership::Managed { references, .. }) = self.objects.get_mut(&object) else {
                unreachable!()
            };
            *references -= count;
        }
        self.queued -= drained;
        self.pools.truncate(boundary);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn class_enrollment_stages_idempotent_immortals_without_promoting_objects() {
        let mut live = super::Lifetime::default();
        live.register_immortal(0x1000).unwrap();
        live.register_object(0x2000, 0x1000, 2).unwrap();
        let staged = live
            .with_class_identities(&[0x1000, 0x3000, 0x3000])
            .unwrap();
        assert!(!live.contains_identity(0x3000));
        assert_eq!(
            staged.plan_release(0x3000).unwrap(),
            super::ReleasePlan::NoOp
        );
        assert_eq!(
            staged.plan_release(0x2000).unwrap(),
            super::ReleasePlan::Decrement { remaining: 1 }
        );
        assert!(live.with_class_identities(&[0x3000, 0x2000]).is_err());
        assert!(!live.contains_identity(0x3000));
        assert!(live.with_class_identities(&[0x3000, 0]).is_err());
        let ticket = live.begin_deallocation(0x2000);
        assert!(ticket.is_err()); // Two references cannot begin final disposal.
        live.release(0x2000).unwrap();
        live.begin_deallocation(0x2000).unwrap();
        assert!(live.with_class_identities(&[0x2000]).is_err());
    }
    use super::*;
    const OBJECT: u64 = 0x1_0000_1000;
    const CLASS: u64 = 0x1_0000_2000;
    #[test]
    fn real_disposal_receipt_required_and_reentrancy_rejected() {
        let mut l = Lifetime::default();
        l.register_object(OBJECT, CLASS, 1).unwrap();
        let ticket = l.begin_deallocation(OBJECT).unwrap();
        assert!(l.retain(OBJECT).is_err());
        assert!(l.release(OBJECT).is_err());
        assert!(l.record_disposal(ticket).is_err());
        l.cancel_deallocation(ticket).unwrap();
        let ticket = l.begin_deallocation(OBJECT).unwrap();
        l.mark_deallocation_started(ticket).unwrap();
        assert!(l.cancel_deallocation(ticket).is_err());
        l.record_disposal(ticket).unwrap();
        assert!(l.record_disposal(ticket).is_err());
        l.finish_deallocation(ticket, Ok(())).unwrap();
        assert!(l.retain(OBJECT).is_err());
        assert!(l.finish_deallocation(ticket, Ok(())).is_err());
        l.register_object(OBJECT, CLASS, 1).unwrap(); // allocator may reuse after disposal
    }
    #[test]
    fn guest_error_or_missing_disposal_quarantines_without_fake_free() {
        for disposed in [false, true] {
            let mut l = Lifetime::default();
            l.register_object(OBJECT, CLASS, 1).unwrap();
            let ticket = l.begin_deallocation(OBJECT).unwrap();
            l.mark_deallocation_started(ticket).unwrap();
            if disposed {
                l.record_disposal(ticket).unwrap();
            }
            let result = if disposed {
                Err("guest trap".into())
            } else {
                Ok(())
            };
            assert!(l.finish_deallocation(ticket, result).is_err());
            assert!(l.register_object(OBJECT, CLASS, 1).is_err());
            assert!(l.retain(OBJECT).is_err());
            assert!(l.record_disposal(ticket).is_err());
            assert!(l.cancel_deallocation(ticket).is_err());
        }
    }
    #[test]
    fn real_unoptimized_arc_return_handshake_balances_ownership() {
        let mut l = Lifetime::default();
        l.register_object(OBJECT, CLASS, 1).unwrap();
        let cookie = l.push_pool().unwrap();
        assert_eq!(l.autorelease(OBJECT).unwrap(), OBJECT); // autoreleaseReturnValue
        assert_eq!(l.retain(OBJECT).unwrap(), OBJECT); // retainAutoreleasedReturnValue
        l.pop_pool(cookie).unwrap();
        assert_eq!(
            l.plan_release(OBJECT).unwrap(),
            ReleasePlan::Deallocate {
                object: OBJECT,
                class: CLASS
            }
        );
        assert!(l.release(OBJECT).unwrap_err().contains("guest -dealloc"));
        assert_eq!(
            l.objects[&OBJECT],
            Ownership::Managed {
                class: CLASS,
                references: 1
            }
        );
    }
    #[test]
    fn nested_boundary_pop_drains_repeated_entries_and_expires_children() {
        let mut l = Lifetime::default();
        l.register_object(OBJECT, CLASS, 4).unwrap();
        let outer = l.push_pool().unwrap();
        l.autorelease(OBJECT).unwrap();
        let inner = l.push_pool().unwrap();
        l.autorelease(OBJECT).unwrap();
        l.autorelease(OBJECT).unwrap();
        l.pop_pool(outer).unwrap();
        assert!(l.pop_pool(inner).is_err());
        assert!(l.pop_pool(outer).is_err());
        assert_eq!(l.queued, 0);
        assert_eq!(
            l.objects[&OBJECT],
            Ownership::Managed {
                class: CLASS,
                references: 1
            }
        );
    }
    #[test]
    fn final_deallocation_blocks_drain_atomically() {
        let mut l = Lifetime::default();
        l.register_object(OBJECT, CLASS, 2).unwrap();
        l.register_object(OBJECT + 8, CLASS, 1).unwrap();
        let cookie = l.push_pool().unwrap();
        l.autorelease(OBJECT).unwrap();
        l.autorelease(OBJECT + 8).unwrap();
        assert!(l.pop_pool(cookie).is_err());
        assert_eq!(l.queued, 2);
        assert_eq!(l.pools.len(), 1);
        assert_eq!(
            l.objects[&OBJECT],
            Ownership::Managed {
                class: CLASS,
                references: 2
            }
        );
        l.retain(OBJECT + 8).unwrap();
        l.pop_pool(cookie).unwrap();
    }
    #[test]
    fn nil_and_registered_classes_have_actual_immortal_ownership() {
        let mut l = Lifetime::default();
        l.register_immortal(CLASS).unwrap();
        for object in [0, CLASS] {
            assert_eq!(l.retain(object).unwrap(), object);
            assert_eq!(l.autorelease(object).unwrap(), object);
            l.release(object).unwrap();
        }
        assert!(l.retain(OBJECT).is_err());
        assert!(l.autorelease(OBJECT).is_err());
        assert!(l.register_object(CLASS, CLASS, 1).is_err());
    }
    #[test]
    fn bounded_failures_do_not_add_retains_or_pool_entries() {
        let mut l = Lifetime::default();
        l.register_object(OBJECT, CLASS, 1).unwrap();
        assert!(l.retain_autorelease(OBJECT).is_err());
        assert_eq!(
            l.objects[&OBJECT],
            Ownership::Managed {
                class: CLASS,
                references: 1
            }
        );
        let token = l.push_pool().unwrap();
        l.queued = MAX_AUTORELEASES;
        assert!(l.retain_autorelease(OBJECT).is_err());
        assert!(l.pools[0].objects.is_empty());
        l.queued = 0;
        l.pop_pool(token).unwrap();
        l.objects.insert(
            OBJECT,
            Ownership::Managed {
                class: CLASS,
                references: u32::MAX,
            },
        );
        assert!(l.retain(OBJECT).is_err());
        assert_eq!(
            l.objects[&OBJECT],
            Ownership::Managed {
                class: CLASS,
                references: u32::MAX
            }
        );
        assert!(l.register_object(1, CLASS, 1).is_err());
        assert!(l.register_object(OBJECT + 8, CLASS, 0).is_err());
        l.next_token = u64::MAX;
        assert!(l.push_pool().is_err());
        assert!(l.pools.is_empty());
    }
}

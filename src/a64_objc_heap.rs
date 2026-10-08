/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Real bounded guest object storage for explicitly authorized plain classes.
//! No Apple C++ constructor/destructor, weak table or strong ivar destruction
//! is substituted. Such classes are rejected. Arena blocks become reusable
//! only after genuine disposal AND successful guest -dealloc completion.
use super::objc_lifetime::{DeallocationTicket, Lifetime};
use super::objc_metadata::read_class;
use super::A64Cpu;
use std::collections::{BTreeMap, BTreeSet};

const MAX_ARENA: usize = 16 * 1024 * 1024;
const MAX_OBJECT: usize = 1024 * 1024;
const MAX_ALLOCATIONS: usize = 65536;

#[derive(Debug)]
pub(super) struct PlainClass {
    address: u64,
    size: usize,
}
impl PlainClass {
    /// The caller must have completed real class initialization and supplied
    /// the root whose ordinary NSObject ownership implementation it supports.
    /// Metadata does not establish initialization or authorize the root.
    pub(super) fn read(
        address: u64,
        initialized: bool,
        ordinary_root: u64,
        mut read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
    ) -> Result<Self, String> {
        if !initialized || ordinary_root == 0 {
            return Err("guest plain object allocation requires initialized class and supported ownership root".into());
        }
        let mut current = address;
        let mut visited = BTreeSet::new();
        let mut size = None;
        let mut child_start = None;
        loop {
            if visited.len() >= 64 || !visited.insert(current) {
                return Err("guest allocation superclass chain is cyclic or excessive".into());
            }
            let class = read_class(current, &mut read)?;
            if class.flags & 1 != 0 || class.flags & ((1 << 2) | (1 << 8)) != 0 {
                return Err(
                    "guest allocation rejects metaclass or C++ lifetime requirements".into(),
                );
            }
            if class.instance_size < 8 || class.instance_size as usize > MAX_OBJECT {
                return Err("guest object size exceeds allocator bounds".into());
            }
            if child_start.is_some_and(|start| class.instance_size > start) {
                return Err("guest subclass storage overlaps superclass instance".into());
            }
            if size.is_none() {
                size = Some(class.instance_size as usize);
            }
            child_start = Some(class.instance_start);
            let raw = read(current, 40)?;
            if raw.len() != 40 {
                return Err("short guest class storage read".into());
            }
            let ro = u64::from_le_bytes(raw[32..40].try_into().unwrap()) & !7;
            let bytes = read(ro, 72)?;
            if bytes.len() != 72 {
                return Err("short guest class_ro storage read".into());
            }
            for offset in [16, 48, 56] {
                if u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) != 0 {
                    return Err(
                        "guest plain allocation requires no ivar list, strong or weak layout"
                            .into(),
                    );
                }
            }
            if current != ordinary_root
                && class.methods.iter().any(|m| {
                    matches!(
                        m.selector.as_str(),
                        "retain"
                            | "release"
                            | "autorelease"
                            | "retainCount"
                            | "_tryRetain"
                            | "_isDeallocating"
                            | "allowsWeakReference"
                            | "retainWeakReference"
                    )
                })
            {
                return Err("guest class overrides ordinary reference counting".into());
            }
            if class.superclass == 0 {
                if current != ordinary_root || class.flags & 2 == 0 {
                    return Err("guest allocation does not reach authorized ownership root".into());
                }
                break;
            }
            if current == ordinary_root {
                return Err("authorized ownership root has a superclass".into());
            }
            current = class.superclass;
        }
        Ok(Self {
            address,
            size: size.unwrap(),
        })
    }
}

#[derive(Debug)]
struct Allocation {
    class: u64,
    size: usize,
    disposed: Option<DeallocationTicket>,
}
#[derive(Debug)]
pub(super) struct GuestObjectHeap {
    free: BTreeMap<u64, usize>,
    allocations: BTreeMap<u64, Allocation>,
}
impl GuestObjectHeap {
    pub(super) fn map(cpu: &mut A64Cpu, base: u64, size: usize) -> Result<Self, String> {
        if base == 0
            || base & 4095 != 0
            || size == 0
            || size & 4095 != 0
            || size > MAX_ARENA
            || base
                .checked_add(size as u64)
                .is_none_or(|end| end >> 63 != 0)
        {
            return Err("guest object arena address or size invalid".into());
        }
        cpu.map_zeroed(base, size, 3)?; // RW, never executable
        Ok(Self {
            free: BTreeMap::from([(base, size)]),
            allocations: BTreeMap::new(),
        })
    }

    /// Writer must validate the entire destination before mutation. Registration
    /// failure only dirties an unallocated free block; no object is published.
    pub(super) fn allocate(
        &mut self,
        class: &PlainClass,
        extra_bytes: usize,
        lifetime: &mut Lifetime,
        mut write: impl FnMut(u64, &[u8]) -> Result<(), String>,
    ) -> Result<u64, String> {
        if self.allocations.len() >= MAX_ALLOCATIONS {
            return Err("guest allocation count limit".into());
        }
        let requested = class
            .size
            .checked_add(extra_bytes)
            .ok_or("guest object size overflow")?;
        if requested > MAX_OBJECT {
            return Err("guest object size limit".into());
        }
        let size = requested
            .checked_add(15)
            .ok_or("guest object alignment overflow")?
            & !15;
        let (&object, &available) = self
            .free
            .iter()
            .find(|(_, &len)| len >= size)
            .ok_or("guest object arena exhausted")?;
        let mut storage = vec![0; size];
        storage[..8].copy_from_slice(&class.address.to_le_bytes());
        write(object, &storage)?;
        lifetime.register_object(object, class.address, 1)?;
        self.free.remove(&object);
        if available > size {
            self.free.insert(object + size as u64, available - size);
        }
        self.allocations.insert(
            object,
            Allocation {
                class: class.address,
                size,
                disposed: None,
            },
        );
        Ok(object)
    }

    /// Applicable only to this arena's previously validated plain object. This
    /// is the real disposal route for the supported no-cleanup class subset.
    /// It must be reached through actual guest deallocation policy, not called
    /// merely because a final-reference plan was generated.
    pub(super) fn dispose(
        &mut self,
        ticket: DeallocationTicket,
        lifetime: &mut Lifetime,
        mut write: impl FnMut(u64, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let object = ticket.object();
        let allocation = self
            .allocations
            .get_mut(&object)
            .ok_or("object is not owned by guest arena")?;
        if allocation.class != ticket.class() || allocation.disposed.is_some() {
            return Err("guest arena disposal identity mismatch or duplicate".into());
        }
        lifetime.validate_disposal(ticket)?;
        write(object, &vec![0; allocation.size])?;
        lifetime.record_disposal(ticket)?;
        allocation.disposed = Some(ticket);
        Ok(())
    }

    /// Called after completion removed ownership. A failed or still-running
    /// destructor retains its arena block, preventing unsafe address reuse.
    pub(super) fn reclaim(
        &mut self,
        ticket: DeallocationTicket,
        lifetime: &Lifetime,
    ) -> Result<(), String> {
        let object = ticket.object();
        let allocation = self
            .allocations
            .get(&object)
            .ok_or("unknown guest arena reclaim")?;
        if allocation.disposed != Some(ticket) || lifetime.contains_identity(object) {
            return Err("guest arena reclaim before successful deallocation completion".into());
        }
        let mut base = object;
        let mut size = allocation.size;
        if let Some((&previous, &length)) = self.free.range(..object).next_back() {
            if previous + length as u64 == object {
                base = previous;
                size += length;
                self.free.remove(&previous);
            }
        }
        if let Some(length) = self.free.remove(&(object + allocation.size as u64)) {
            size += length;
        }
        self.allocations.remove(&object);
        self.free.insert(base, size);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plain_root_bytes() -> Vec<u8> {
        let mut bytes = vec![0; 4096];
        bytes[0..8].copy_from_slice(&0x1200u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&0x1100u64.to_le_bytes());
        bytes[0x100..0x104].copy_from_slice(&2u32.to_le_bytes());
        bytes[0x108..0x10c].copy_from_slice(&8u32.to_le_bytes());
        bytes[0x118..0x120].copy_from_slice(&0x1180u64.to_le_bytes());
        bytes[0x180..0x189].copy_from_slice(b"NSObject\0");
        bytes
    }
    fn class_policy(bytes: &[u8]) -> Result<PlainClass, String> {
        PlainClass::read(0x1000, true, 0x1000, |address, len| {
            let offset = address
                .checked_sub(0x1000)
                .ok_or("fixture address underflow")? as usize;
            let end = offset.checked_add(len).ok_or("fixture range overflow")?;
            bytes
                .get(offset..end)
                .map(|b| b.to_vec())
                .ok_or("fixture read outside mapping".into())
        })
    }
    #[test]
    fn class_policy_rejects_cleanup_metadata_and_uninitialized_state() {
        let bytes = plain_root_bytes();
        assert_eq!(class_policy(&bytes).unwrap().size, 8);
        for offset in [0x110, 0x130, 0x138] {
            let mut unsupported = bytes.clone();
            unsupported[offset..offset + 8].copy_from_slice(&0x1400u64.to_le_bytes());
            assert!(class_policy(&unsupported).unwrap_err().contains("layout"));
        }
        let mut cxx = bytes.clone();
        cxx[0x100..0x104].copy_from_slice(&6u32.to_le_bytes());
        assert!(class_policy(&cxx).unwrap_err().contains("C++"));
        assert!(class_policy(&bytes[..0x120]).is_err());
        assert!(PlainClass::read(0x1000, false, 0x1000, |_, _| panic!(
            "must not read uninitialized class"
        ))
        .is_err());
        assert!(PlainClass::read(0x1000, true, 0, |_, _| panic!(
            "must not read unauthorized root"
        ))
        .is_err());
    }
    #[test]
    fn actual_storage_zero_isa_alignment_disposal_and_reuse() {
        let mut cpu = A64Cpu::new_sparse();
        let mut heap = GuestObjectHeap::map(&mut cpu, 0x80000, 4096).unwrap();
        let mut lifetime = Lifetime::default();
        let class = PlainClass {
            address: 0x40000,
            size: 17,
        };
        let object = heap
            .allocate(&class, 0, &mut lifetime, |a, b| cpu.write_guest_into(a, b))
            .unwrap();
        let second = heap
            .allocate(&class, 0, &mut lifetime, |a, b| cpu.write_guest_into(a, b))
            .unwrap();
        assert_eq!(second - object, 32);
        let mut bytes = [0xff; 32];
        cpu.read_guest_into(object, &mut bytes).unwrap();
        assert_eq!(&bytes[..8], &class.address.to_le_bytes());
        assert_eq!(&bytes[8..], &[0; 24]);
        let ticket = lifetime.begin_deallocation(object).unwrap();
        assert!(heap
            .dispose(ticket, &mut lifetime, |a, b| cpu.write_guest_into(a, b))
            .is_err());
        lifetime.mark_deallocation_started(ticket).unwrap();
        heap.dispose(ticket, &mut lifetime, |a, b| cpu.write_guest_into(a, b))
            .unwrap();
        assert!(heap.reclaim(ticket, &lifetime).is_err());
        lifetime.finish_deallocation(ticket, Ok(())).unwrap();
        heap.reclaim(ticket, &lifetime).unwrap();
        assert_eq!(
            heap.allocate(&class, 0, &mut lifetime, |a, b| cpu.write_guest_into(a, b))
                .unwrap(),
            object
        );
        assert_eq!(cpu.mapped_permissions(object), Some(3));
    }
    #[test]
    fn failed_memory_write_or_destructor_does_not_recycle_storage() {
        let mut cpu = A64Cpu::new_sparse();
        let mut heap = GuestObjectHeap::map(&mut cpu, 0x80000, 4096).unwrap();
        let mut lifetime = Lifetime::default();
        let class = PlainClass {
            address: 0x40000,
            size: 4096,
        };
        assert!(heap
            .allocate(&class, 0, &mut lifetime, |_, _| Err("write failure".into()))
            .is_err());
        assert!(!lifetime.contains_identity(0x80000));
        let object = heap
            .allocate(&class, 0, &mut lifetime, |a, b| cpu.write_guest_into(a, b))
            .unwrap();
        let ticket = lifetime.begin_deallocation(object).unwrap();
        lifetime.mark_deallocation_started(ticket).unwrap();
        assert!(heap
            .dispose(ticket, &mut lifetime, |_, _| Err("write failure".into()))
            .is_err());
        heap.dispose(ticket, &mut lifetime, |a, b| cpu.write_guest_into(a, b))
            .unwrap();
        assert!(lifetime
            .finish_deallocation(ticket, Err("guest failure".into()))
            .is_err());
        assert!(heap.reclaim(ticket, &lifetime).is_err());
        assert!(heap
            .allocate(&class, 0, &mut lifetime, |a, b| cpu.write_guest_into(a, b))
            .is_err());
    }
}

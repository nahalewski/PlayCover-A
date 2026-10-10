/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Emulator-owned non-UIKit system frameworks for the ARM64 runtime.
//!
//! See dev-docs/ARM64_FRAMEWORKS_PLAN.md. Daemon-backed frameworks
//! (OpenAL/audio HAL, ...) cannot run genuinely from the shared cache, so their
//! C functions are owned here and bound only through the existing selected-
//! service route, i.e. only after the genuine cached export was verified.
//!
//! Family dispatch: each framework family costs ONE bridge service slot.
//! Every routed symbol gets an 8-byte trampoline in a page owned by this
//! module, `movz x17,#index; b <family service stub>`. x17 (IP1) is
//! call-clobbered under AAPCS64/Apple ABI; LR is untouched, so the family
//! stub's `ret` returns straight to the caller and the bridge still sees the
//! service trap at exactly `stub + 8`.
use super::{
    bridge::{GuestBridge, ReturnValues, ServiceFrame},
    A64Cpu,
};
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

#[path = "a64_frameworks_openal.rs"]
pub(super) mod openal;

const PAGE: u64 = 4096;
/// One RX trampoline page followed by the RW arena.
pub(super) const ARENA_BYTES: u64 = 3 * PAGE;
pub(super) const RESERVED_BYTES: u64 = PAGE + ARENA_BYTES;
const TRAMPOLINE_BYTES: u64 = 8;
const MAX_TRAMPOLINES: usize = (PAGE / TRAMPOLINE_BYTES) as usize;
const HANDLE_BYTES: u64 = 16;

/// Guest-visible storage owned by the framework layer: opaque 16-byte handles
/// (never host pointers) and immutable NUL-terminated strings.
pub(super) struct Arena {
    base: u64,
    len: u64,
    next: u64,
    free_handles: Vec<u64>,
    live_handles: BTreeSet<u64>,
}
impl Arena {
    fn new(base: u64, len: u64) -> Self {
        Self {
            base,
            len,
            next: base,
            free_handles: Vec::new(),
            live_handles: BTreeSet::new(),
        }
    }
    fn bump(&mut self, len: u64) -> Result<u64, String> {
        let len = len.checked_add(15).ok_or("framework arena size overflow")? & !15;
        let address = self.next;
        let end = address
            .checked_add(len)
            .ok_or("framework arena address overflow")?;
        if end > self.base + self.len {
            return Err("framework arena exhausted".into());
        }
        self.next = end;
        Ok(address)
    }
    /// A unique nonzero opaque handle. Its bytes stay zero; guest code only
    /// compares/passes it back.
    pub(super) fn allocate_handle(&mut self) -> Result<u64, String> {
        let handle = match self.free_handles.pop() {
            Some(handle) => handle,
            None => self.bump(HANDLE_BYTES)?,
        };
        self.live_handles.insert(handle);
        Ok(handle)
    }
    pub(super) fn release_handle(&mut self, handle: u64) -> Result<(), String> {
        if !self.live_handles.remove(&handle) {
            return Err(format!("framework handle {handle:#x} is not live"));
        }
        self.free_handles.push(handle);
        Ok(())
    }
    /// Copy an immutable C string into the arena (callers cache the result).
    pub(super) fn store_c_string(
        &mut self,
        frame: &mut ServiceFrame<'_>,
        text: &[u8],
    ) -> Result<u64, String> {
        if text.contains(&0) {
            return Err("framework C string contains NUL".into());
        }
        let address = self.bump(text.len() as u64 + 1)?;
        let mut bytes = text.to_vec();
        bytes.push(0);
        frame.write(address, &bytes)?;
        Ok(address)
    }
}

/// One framework family: a fixed symbol table and a dispatcher.
pub(super) trait Family {
    fn name(&self) -> &'static str;
    /// Exact cached provider install name (`SelectedServices` binding key).
    fn provider(&self) -> &'static str;
    /// Mach-O symbol names (leading underscore), indexed by dispatch index.
    fn symbols(&self) -> &'static [&'static str];
    /// Told once, after installation, where each symbol's trampoline lives
    /// (e.g. for `alGetProcAddress`).
    fn bind_entries(&mut self, _entries: &[(&'static str, u64)]) {}
    fn call(
        &mut self,
        index: usize,
        frame: &mut ServiceFrame<'_>,
        arena: &mut Arena,
    ) -> Result<ReturnValues, String>;
}

pub(super) struct Binding {
    pub provider: &'static str,
    pub symbol: &'static str,
    pub address: u64,
}

pub(super) struct Frameworks {
    code: u64,
    used_trampolines: usize,
    pub bindings: Vec<Binding>,
}
impl Frameworks {
    pub(super) fn instruction_range(&self) -> (u64, u64) {
        (self.code, self.code + self.used_trampolines as u64 * TRAMPOLINE_BYTES)
    }
}

fn movz_x17(index: usize) -> Result<u32, String> {
    if index > 0xffff {
        return Err("framework dispatch index exceeds movz immediate".into());
    }
    Ok(0xd2800011 | ((index as u32) << 5))
}
fn branch(from: u64, to: u64) -> Result<u32, String> {
    let delta = (to as i64).wrapping_sub(from as i64);
    if delta & 3 != 0 || !(-(1 << 27)..(1 << 27)).contains(&delta) {
        return Err("framework trampoline branch out of range".into());
    }
    Ok(0x14000000 | (((delta >> 2) as u32) & 0x03ff_ffff))
}

/// Map the trampoline page and arena at `base` (page aligned, unmapped) and
/// register one bridge service per family.
pub(super) fn install(
    cpu: &mut A64Cpu,
    bridge: &mut GuestBridge,
    base: u64,
    families: Vec<Box<dyn Family>>,
) -> Result<Frameworks, String> {
    if base == 0 || base & (PAGE - 1) != 0 {
        return Err("framework scratch must be nonzero and page aligned".into());
    }
    let end = base
        .checked_add(RESERVED_BYTES)
        .ok_or("framework scratch overflow")?;
    let mut address = base;
    while address < end {
        if cpu.mapped_permissions(address).is_some() {
            return Err("framework scratch overlaps existing memory".into());
        }
        address += PAGE;
    }
    let total: usize = families.iter().map(|f| f.symbols().len()).sum();
    if total > MAX_TRAMPOLINES {
        return Err("framework trampoline page capacity exceeded".into());
    }
    cpu.map_zeroed(base, PAGE as usize, 5)?;
    cpu.map_zeroed(base + PAGE, ARENA_BYTES as usize, 3)?;
    let arena = Rc::new(RefCell::new(Arena::new(base + PAGE, ARENA_BYTES)));
    let mut bindings = Vec::new();
    let mut next = base;
    for family in families {
        let name = family.name();
        let provider = family.provider();
        let symbols = family.symbols();
        let family = Rc::new(RefCell::new(family));
        let handler_family = family.clone();
        let handler_arena = arena.clone();
        let count = symbols.len();
        let service = bridge.register_service(
            cpu,
            &format!("_touchHLE_a64_frameworks_{name}_dispatch"),
            move |frame| {
                let index = frame.dispatch_index() as usize;
                if index >= count {
                    return Err(format!("framework {name} dispatch index {index} invalid"));
                }
                handler_family
                    .borrow_mut()
                    .call(index, frame, &mut handler_arena.borrow_mut())
            },
        )?;
        let mut entries = Vec::with_capacity(count);
        for (index, &symbol) in symbols.iter().enumerate() {
            let code = [
                movz_x17(index)?,
                branch(next + 4, service.guest_address())?,
            ];
            let bytes: Vec<u8> = code.into_iter().flat_map(u32::to_le_bytes).collect();
            cpu.try_write_bytes(next, &bytes)?;
            entries.push((symbol, next));
            bindings.push(Binding {
                provider,
                symbol,
                address: next,
            });
            next += TRAMPOLINE_BYTES;
        }
        family.borrow_mut().bind_entries(&entries);
    }
    Ok(Frameworks {
        code: base,
        used_trampolines: total,
        bindings,
    })
}

/// The default family set for ARM64 app sessions.
pub(super) fn default_families() -> Vec<Box<dyn Family>> {
    vec![Box::new(openal::OpenAl::default())]
}

// ABI helpers shared by families.
pub(super) fn arg_f32(frame: &ServiceFrame<'_>, index: usize) -> Result<f32, String> {
    Ok(f32::from_bits(frame.vector(index)?[0] as u32))
}
pub(super) fn arg_i32(frame: &ServiceFrame<'_>, index: usize) -> Result<i32, String> {
    Ok(frame.integer(index)? as u32 as i32)
}
pub(super) fn arg_u32(frame: &ServiceFrame<'_>, index: usize) -> Result<u32, String> {
    Ok(frame.integer(index)? as u32)
}
pub(super) fn ret_f32(value: f32) -> ReturnValues {
    let mut values = ReturnValues::integer(0);
    values.vectors[0] = [value.to_bits() as u64, 0];
    values
}
pub(super) fn ret_void() -> ReturnValues {
    ReturnValues::integer(0)
}
/// Read a NUL-terminated guest string of at most `limit` bytes.
pub(super) fn read_c_string(
    frame: &mut ServiceFrame<'_>,
    address: u64,
    limit: usize,
) -> Result<Vec<u8>, String> {
    if address == 0 {
        return Err("framework C string pointer is NULL".into());
    }
    let mut out = Vec::new();
    while out.len() < limit {
        let byte = frame.read(address + out.len() as u64, 1)?[0];
        if byte == 0 {
            return Ok(out);
        }
        out.push(byte);
    }
    Err("framework C string exceeds limit".into())
}
pub(super) fn read_u32s(
    frame: &mut ServiceFrame<'_>,
    address: u64,
    count: usize,
) -> Result<Vec<u32>, String> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if address == 0 {
        return Err("framework array pointer is NULL".into());
    }
    let bytes = frame.read(address, count.checked_mul(4).ok_or("array size overflow")?)?;
    Ok(bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect())
}
pub(super) fn write_u32s(
    frame: &mut ServiceFrame<'_>,
    address: u64,
    values: &[u32],
) -> Result<(), String> {
    if values.is_empty() {
        return Ok(());
    }
    if address == 0 {
        return Err("framework output pointer is NULL".into());
    }
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    frame.write(address, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a64::bridge::GuestCall;

    struct Echo {
        seen: Rc<RefCell<Vec<(usize, u64)>>>,
    }
    impl Family for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn provider(&self) -> &'static str {
            "/test/Echo"
        }
        fn symbols(&self) -> &'static [&'static str] {
            &["_first", "_second", "_third"]
        }
        fn call(
            &mut self,
            index: usize,
            frame: &mut ServiceFrame<'_>,
            arena: &mut Arena,
        ) -> Result<ReturnValues, String> {
            self.seen.borrow_mut().push((index, frame.integer(0)?));
            match index {
                0 => Ok(ReturnValues::integer(frame.integer(0)? + 1)),
                1 => Ok(ret_f32(arg_f32(frame, 0)? * 2.0)),
                _ => Ok(ReturnValues::integer(arena.store_c_string(frame, b"hi")?)),
            }
        }
    }

    #[test]
    fn family_trampolines_share_one_service_and_preserve_abi() {
        let mut cpu = A64Cpu::new_sparse();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let frameworks = install(
            &mut cpu,
            &mut bridge,
            0x200000,
            vec![Box::new(Echo { seen: seen.clone() })],
        )
        .unwrap();
        assert_eq!(frameworks.bindings.len(), 3);
        assert_eq!(frameworks.instruction_range(), (0x200000, 0x200018));
        // Exactly one service slot was consumed.
        assert_eq!(bridge.instruction_ranges().len(), 2);
        let call = |cpu: &mut A64Cpu, bridge: &mut GuestBridge, entry, integers, vectors| {
            bridge
                .call(
                    cpu,
                    &GuestCall {
                        entry,
                        integers,
                        vectors,
                        ..Default::default()
                    },
                    100,
                )
                .unwrap()
        };
        let first = frameworks.bindings[0].address;
        assert_eq!(call(&mut cpu, &mut bridge, first, vec![41], vec![]).integers[0], 42);
        let second = frameworks.bindings[1].address;
        let doubled = call(
            &mut cpu,
            &mut bridge,
            second,
            vec![7],
            vec![[1.5f32.to_bits() as u64, 0]],
        );
        assert_eq!(f32::from_bits(doubled.vectors[0][0] as u32), 3.0);
        let third = frameworks.bindings[2].address;
        let text = call(&mut cpu, &mut bridge, third, vec![0], vec![]).integers[0];
        assert_eq!(cpu.read_bytes(text, 3).unwrap(), b"hi\0");
        assert_eq!(cpu.mapped_permissions(text), Some(3));
        assert_eq!(*seen.borrow(), vec![(0, 41), (1, 7), (2, 0)]);
        // The scratch range is refused a second time (no overlap).
        assert!(install(&mut cpu, &mut bridge, 0x200000, vec![]).is_err());
    }

    #[test]
    fn selected_services_route_owned_openal_only_over_genuine_exports() {
        use crate::a64::cache_symbols::CacheDefinition;
        use crate::a64::host_services::{SelectedServices, Selection};
        let mut cpu = A64Cpu::new_sparse();
        // A stand-in "genuine" strong executable export.
        cpu.map_zeroed(0x10000, 4096, 5).unwrap();
        cpu.write_bytes(0x10000, &0xd65f03c0u32.to_le_bytes());
        let mut services = SelectedServices::install(
            &mut cpu,
            0x50000,
            Selection {
                core_foundation: true,
                objc_lifetime: true,
            },
            Default::default(),
        )
        .unwrap();
        // install() already enabled the default families exactly once.
        assert!(services.scratch_end() >= 0x50000 + RESERVED_BYTES);
        assert!(services
            .enable_frameworks(&mut cpu, default_families())
            .is_err());
        let real = CacheDefinition {
            address: 0x10000,
            weak: false,
        };
        let entry = services
            .route(&cpu, openal::PROVIDER, "_alcGetCurrentContext", real)
            .unwrap()
            .unwrap();
        assert!(services
            .instruction_ranges()
            .iter()
            .any(|&(start, end)| entry >= start && entry + 8 <= end));
        // Wrong provider or a non-genuine export never routes.
        assert_eq!(
            services
                .route(&cpu, "/fake/OpenAL", "_alcGetCurrentContext", real)
                .unwrap(),
            None
        );
        assert!(services
            .route(
                &cpu,
                openal::PROVIDER,
                "_alcGetCurrentContext",
                CacheDefinition {
                    address: 0x10000,
                    weak: true
                }
            )
            .is_err());
        // No context yet: the owned implementation answers NULL.
        let result = services
            .call(
                &mut cpu,
                &GuestCall {
                    entry,
                    ..Default::default()
                },
                100,
            )
            .unwrap();
        assert_eq!(result.integers[0], 0);
    }

    #[test]
    fn handles_are_unique_reused_and_checked() {
        let mut arena = Arena::new(0x1000, 64);
        let a = arena.allocate_handle().unwrap();
        let b = arena.allocate_handle().unwrap();
        assert_ne!(a, b);
        arena.release_handle(a).unwrap();
        assert!(arena.release_handle(a).is_err());
        assert_eq!(arena.allocate_handle().unwrap(), a);
        arena.allocate_handle().unwrap();
        arena.allocate_handle().unwrap();
        assert!(arena.allocate_handle().is_err());
    }

    #[test]
    fn branch_encoding_is_checked() {
        assert_eq!(branch(0x1000, 0x1000).unwrap(), 0x14000000);
        assert_eq!(branch(0x1004, 0x1000).unwrap(), 0x17ffffff);
        assert!(branch(0, 1 << 28).is_err());
        assert!(movz_x17(0x10000).is_err());
    }
}

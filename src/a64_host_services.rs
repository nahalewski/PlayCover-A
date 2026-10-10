/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Explicit selected-service routing, never a missing-framework fallback.
//! The ordinary-image linker first verifies the genuine cached provider/export.
//! Only a selected, implemented function may then bind to a guest trampoline.
use super::{
    bridge::{GuestBridge, GuestCall, ReturnValues},
    cache_symbols::{CacheDefinition, CacheSymbols},
    cf_terraria_services::{self, KnownConstants},
    objc_lifetime::{self, Lifetime},
    objc_lifetime_services, A64Cpu,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

pub(super) const CORE_FOUNDATION: &str =
    "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";
pub(super) const OBJC: &str = "/usr/lib/libobjc.A.dylib";
pub(super) const ARENA_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Default)]
pub(super) struct Selection {
    pub core_foundation: bool,
    pub objc_lifetime: bool,
}
impl Selection {
    pub(super) fn validate(self) -> Result<(), String> {
        if !self.core_foundation && !self.objc_lifetime {
            return Err("selected host services require an explicit nonempty selection".into());
        }
        Ok(())
    }
    pub(super) fn mapped_bytes(self) -> u64 {
        super::bridge::RUNTIME_MAPPED_BYTES + if self.core_foundation { ARENA_BYTES } else { 0 }
    }
}

pub(super) struct SelectedServices {
    bridge: GuestBridge,
    bindings: BTreeMap<(&'static str, String), u64>,
    /// Shared only with actual registered guest allocation/lifetime routes.
    /// CF identities are separate opaque objects, not Objective-C instances.
    pub lifetime: Rc<RefCell<Lifetime>>,
    scratch_end: u64,
    pub foundation: Option<super::foundation_startup::Foundation>,
}
impl SelectedServices {
    pub(super) fn set_thread_storage(&mut self,state:std::rc::Rc<super::thread_storage::ThreadStorage>){self.bridge.set_thread_storage(state);}
    pub(super) fn install_dyld_helper_prefix(&mut self,cpu:&mut A64Cpu,entry:u64,helpers:super::dyld_helpers::Helpers,arena:u64,plans:Vec<super::tlv::Plan>,owner:u64)->Result<u64,String> {
        super::dyld_helpers::install_prefix(cpu,&mut self.bridge,entry,helpers,arena,plans,owner)
    }
    pub(super) fn install_dyld_owned_helper_prefix(&mut self,cpu:&mut A64Cpu,entry:u64,helpers:super::dyld_helpers::Helpers,arena:u64,plans:Vec<super::tlv::Plan>,owner:u64,scheduler:std::rc::Rc<std::cell::RefCell<super::thread_scheduler_cpu::CpuScheduler>>,lease:super::execution_session::SessionLease)->Result<u64,String> {
        super::dyld_helpers::install_owned_prefix(cpu,&mut self.bridge,entry,helpers,arena,plans,owner,scheduler,lease)
    }
    pub(super) fn install_dyld_immutable(&mut self,cpu:&mut A64Cpu,entry:u64,ranges:super::dyld_slide::ImmutableRanges)->Result<u64,String> {
        super::dyld_slide::install_immutable(cpu,&mut self.bridge,entry,ranges)
    }
    pub(super) fn install_dyld_sdk_query(&mut self,cpu:&mut A64Cpu,entry:u64,program:super::dyld_sdk_query::ProgramSdk)->Result<u64,String> {
        super::dyld_sdk_query::install(cpu,&mut self.bridge,entry,program)
    }
    pub(super) fn install_dyld_objc_callbacks(&mut self,cpu:&mut A64Cpu,entry:u64,images:Vec<super::dyld_objc_callbacks::ObjcImage>,lease:super::execution_session::SessionLease,scheduler:std::rc::Rc<std::cell::RefCell<super::thread_scheduler_cpu::CpuScheduler>>,owner:u64)->Result<u64,String>{
        super::dyld_objc_callbacks::install(cpu,&mut self.bridge,entry,images,lease,scheduler,owner)
    }
    pub(super) fn install_legacy_dyld_lookup(&mut self,cpu:&mut A64Cpu,plan:&super::cache::CachePlan,slides:super::dyld_slide::ImageSlides,images:Vec<super::legacy_add_images::Image>,objc_images:Vec<super::dyld_objc_callbacks::ObjcImage>)->Result<u64,String>{
        super::legacy_dyld_lookup::install_with_images(cpu,&mut self.bridge,plan,slides,images,objc_images)
    }
    pub(super) fn install_dyld_main_header(&mut self,cpu:&mut A64Cpu,entry:u64,main:super::dyld_main_header::MainHeader)->Result<u64,String>{
        super::dyld_main_header::install(cpu,&mut self.bridge,entry,main)
    }
    pub(super) fn install_dyld_selectors(&mut self,cpu:&mut A64Cpu,entry:u64,table:super::dyld_selectors::SelectorTable)->Result<u64,String>{
        super::dyld_selectors::install(cpu,&mut self.bridge,entry,table)
    }
    pub(super) fn install_dyld_cache_range(&mut self,cpu:&mut A64Cpu,entry:u64,range:super::dyld_cache_range::CacheRange)->Result<u64,String>{
        super::dyld_cache_range::install(cpu,&mut self.bridge,entry,range)
    }
    pub(super) fn install_dyld_overridden(&mut self,cpu:&mut A64Cpu,entry:u64)->Result<u64,String>{
        super::dyld_overridden::install(cpu,&mut self.bridge,entry)
    }
    pub(super) fn install_dyld_add_image(&mut self,cpu:&mut A64Cpu,entry:u64)->Result<u64,String>{
        super::dyld_add_image::install(cpu,&mut self.bridge,entry)
    }
    pub(super) fn install_dyld_objc(&mut self,cpu:&mut A64Cpu,entries:super::dyld_objc::DyldObjcEntries)->Result<(),String>{
        super::dyld_objc::install(cpu,&mut self.bridge,entries)
    }
    pub(super) fn install_dyld_restricted(&mut self,cpu:&mut A64Cpu,entry:u64)->Result<u64,String> {
        super::dyld_slide::install_restricted(cpu,&mut self.bridge,entry,super::dyld_slide::LoaderPolicy::declared_paths_only())
    }
    pub(super) fn install_dyld_slide(&mut self,cpu:&mut A64Cpu,entry:u64,ledger:super::dyld_slide::ImageSlides)->Result<u64,String> {
        super::dyld_slide::install(cpu,&mut self.bridge,entry,ledger)
    }
    pub(super) fn install(
        cpu: &mut A64Cpu,
        scratch: u64,
        selection: Selection,
        constants: KnownConstants,
    ) -> Result<Self, String> {
        selection.validate()?;
        let mut bridge = GuestBridge::map_runtime(cpu, scratch)?;
        let lifetime = Rc::new(RefCell::new(Lifetime::default()));
        let mut bindings = BTreeMap::new();
        if selection.core_foundation {
            let arena = scratch
                .checked_add(super::bridge::RUNTIME_RESERVED_BYTES)
                .ok_or("host CF arena overflow")?;
            for (name, id) in
                cf_terraria_services::register_with_constants(&mut bridge, cpu, arena, constants)?
            {
                if bindings
                    .insert((CORE_FOUNDATION, name.into()), id.guest_address())
                    .is_some()
                {
                    return Err("duplicate selected CF service".into());
                }
            }
        }
        if selection.objc_lifetime {
            for (name, id) in objc_lifetime_services::install(&mut bridge, cpu, lifetime.clone())? {
                if bindings
                    .insert((OBJC, name.into()), id.guest_address())
                    .is_some()
                {
                    return Err("duplicate selected Objective-C service".into());
                }
            }
        }
        Ok(Self {
            bridge,
            bindings,
            lifetime,
            scratch_end: scratch
                .checked_add(super::bridge::RUNTIME_RESERVED_BYTES)
                .and_then(|end| {
                    end.checked_add(if selection.core_foundation {
                        ARENA_BYTES
                    } else {
                        0
                    })
                })
                .ok_or("selected service scratch end overflow")?,
            foundation: None,
        })
    }
    pub(super) fn enable_foundation(
        &mut self,
        cpu: &mut A64Cpu,
        inputs: super::foundation_startup::Inputs,
    ) -> Result<(), String> {
        if self.foundation.is_some() {
            return Err("owned Foundation already installed".into());
        }
        let base = self
            .scratch_end
            .checked_add(4095)
            .ok_or("Foundation scratch alignment overflow")?
            & !4095;
        let foundation = super::foundation_startup::install(
            cpu,
            &mut self.bridge,
            self.lifetime.clone(),
            base,
            inputs,
        )?;
        for (name, id) in &foundation.services {
            self.bindings
                .insert((OBJC, (*name).into()), id.guest_address());
        }
        self.scratch_end = base
            .checked_add(super::foundation_startup::RESERVED_BYTES)
            .ok_or("Foundation scratch end overflow")?;
        self.foundation = Some(foundation);
        Ok(())
    }
    pub(super) fn route(
        &self,
        cpu: &A64Cpu,
        provider: &str,
        symbol: &str,
        definition: CacheDefinition,
    ) -> Result<Option<u64>, String> {
        let Some((_, &address)) = self
            .bindings
            .iter()
            .find(|((path, name), _)| *path == provider && name == symbol)
        else {
            return Ok(None);
        };
        // Replacement is function-only and cannot create a missing provider,
        // patch DATA exports, coalesce weak definitions or use arbitrary ABI.
        if definition.weak || definition.address == 0 || definition.address & 3 != 0 {
            return Err("selected host service requires a genuine strong function export".into());
        }
        for offset in 0..4 {
            if !cpu
                .mapped_permissions(definition.address + offset)
                .is_some_and(|p| p & 4 != 0)
            {
                return Err("selected host service original export is not executable".into());
            }
        }
        Ok(Some(address))
    }
    pub(super) fn owns_address(&self, address: u64) -> bool {
        self.bindings.values().any(|&value| value == address)
    }
    pub(super) fn registered_address(&self, provider: &str, symbol: &str) -> Option<u64> {
        self.bindings
            .iter()
            .find(|((path, name), _)| *path == provider && name == symbol)
            .map(|(_, &address)| address)
    }
    pub(super) fn instruction_ranges(&self) -> Vec<(u64, u64)> {
        let mut ranges = self.bridge.instruction_ranges();
        if let Some(foundation) = &self.foundation {
            ranges.push(foundation.owned_code());
        }
        ranges
    }
    pub(super) fn call_with_instruction_policy(
        &mut self,
        cpu: &mut A64Cpu,
        call: &GuestCall,
        ticks: u64,
        policy: &mut dyn FnMut(&A64Cpu) -> Result<(), String>,
    ) -> Result<ReturnValues, String> {
        self.bridge
            .call_with_instruction_policy(cpu, call, ticks, policy)
    }
    pub(super) fn scratch_end(&self) -> u64 {
        self.scratch_end
    }
    pub(super) fn call_with_supervisor_handler(
        &mut self,
        cpu: &mut A64Cpu,
        call: &GuestCall,
        ticks: u64,
        handler: &mut dyn FnMut(&mut A64Cpu, u16) -> Result<(), String>,
    ) -> Result<ReturnValues, String> {
        self.bridge
            .call_with_supervisor_handler(cpu, call, ticks, handler)
    }
    pub(super) fn call_initializer_with_supervisor(&mut self,cpu:&mut A64Cpu,call:&GuestCall,permit:&super::initializer_budget::InitializerBudget,handler:&mut dyn FnMut(&mut A64Cpu,u16)->Result<(),String>)->Result<ReturnValues,String>{
        self.bridge.call_initializer_with_supervisor(cpu,call,permit,handler)
    }
    pub(super) fn call_legacy_initializer_with_supervisor(&mut self,cpu:&mut A64Cpu,call:&GuestCall,permit:&super::legacy_initializer_budget::LegacyBudget,handler:&mut dyn FnMut(&mut A64Cpu,u16)->Result<(),String>)->Result<ReturnValues,String>{
        self.bridge.call_legacy_initializer_with_supervisor(cpu,call,permit,handler)
    }
    pub(super) fn call(
        &mut self,
        cpu: &mut A64Cpu,
        call: &GuestCall,
        ticks: u64,
    ) -> Result<ReturnValues, String> {
        self.bridge.call(cpu, call, ticks)
    }
}

/// Inspect the two exact CF DATA exports before enabling their limited
/// callback/deallocator semantics. The null allocator argument is the VALUE
/// of its exported pointer variable, never the variable's address.
pub(super) fn cf_constants(
    cpu: &A64Cpu,
    symbols: &mut CacheSymbols,
) -> Result<KnownConstants, String> {
    let mut result = KnownConstants::default();
    if let Some(definition) =
        symbols.resolve_definition(cpu, CORE_FOUNDATION, "_kCFTypeArrayCallBacks")?
    {
        if definition.weak {
            return Err("CF type callbacks DATA export is weak".into());
        }
        let mut bytes = [0; 40];
        cpu.read_guest_into(definition.address, &mut bytes)?;
        if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != 0 {
            return Err("CF type callback version unsupported".into());
        }
        result.type_array_callbacks = Some(definition.address);
    }
    if let Some(definition) =
        symbols.resolve_definition(cpu, CORE_FOUNDATION, "_kCFAllocatorNull")?
    {
        if definition.weak {
            return Err("CF null allocator DATA export is weak".into());
        }
        let mut bytes = [0; 8];
        cpu.read_guest_into(definition.address, &mut bytes)?;
        let object = u64::from_le_bytes(bytes);
        if object == 0 || object & 7 != 0 {
            return Err("CF null allocator value invalid".into());
        }
        let mut header = [0; 16];
        cpu.read_guest_into(object, &mut header)?;
        result.null_allocator = Some(object);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_provider_only_routes_real_export_and_executes_owned_cf_state() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 5).unwrap();
        cpu.write_bytes(0x10000, &0xd65f03c0u32.to_le_bytes());
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        cpu.write_bytes(0x40000, b"abc");
        let mut services = SelectedServices::install(
            &mut cpu,
            0x50000,
            Selection {
                core_foundation: true,
                objc_lifetime: true,
            },
            KnownConstants::default(),
        )
        .unwrap();
        let real = CacheDefinition {
            address: 0x10000,
            weak: false,
        };
        assert_eq!(
            services
                .route(
                    &cpu,
                    "/fake/CoreFoundation",
                    "_CFStringCreateWithBytes",
                    real
                )
                .unwrap(),
            None
        );
        assert_eq!(
            services
                .route(&cpu, CORE_FOUNDATION, "_CFNotImplemented", real)
                .unwrap(),
            None
        );
        let entry = services
            .route(&cpu, CORE_FOUNDATION, "_CFStringCreateWithBytes", real)
            .unwrap()
            .unwrap();
        let object = services
            .call(
                &mut cpu,
                &GuestCall {
                    entry,
                    integers: vec![0, 0x40000, 3, 0x08000100, 0],
                    ..Default::default()
                },
                100,
            )
            .unwrap()
            .integers[0];
        let entry = services
            .route(&cpu, CORE_FOUNDATION, "_CFStringGetLength", real)
            .unwrap()
            .unwrap();
        assert_eq!(
            services
                .call(
                    &mut cpu,
                    &GuestCall {
                        entry,
                        integers: vec![object],
                        ..Default::default()
                    },
                    100
                )
                .unwrap()
                .integers[0],
            3
        );
        assert!(services
            .call(
                &mut cpu,
                &GuestCall {
                    entry,
                    integers: vec![0x40000],
                    ..Default::default()
                },
                100
            )
            .unwrap_err()
            .contains("foreign"));
        assert!(services
            .route(
                &cpu,
                CORE_FOUNDATION,
                "_CFStringGetLength",
                CacheDefinition {
                    address: 0x40000,
                    weak: false
                }
            )
            .is_err());
        assert!(services
            .route(
                &cpu,
                CORE_FOUNDATION,
                "_CFStringGetLength",
                CacheDefinition {
                    address: 0x10000,
                    weak: true
                }
            )
            .is_err());
        let entry = services
            .route(&cpu, OBJC, "_objc_retain", real)
            .unwrap()
            .unwrap();
        assert_eq!(
            services
                .call(
                    &mut cpu,
                    &GuestCall {
                        entry,
                        integers: vec![0],
                        ..Default::default()
                    },
                    100
                )
                .unwrap()
                .integers[0],
            0
        );
        assert!(services
            .call(
                &mut cpu,
                &GuestCall {
                    entry,
                    integers: vec![object],
                    ..Default::default()
                },
                100
            )
            .is_err());
        assert_eq!(
            services.route(&cpu, OBJC, "_objc_msgSend", real).unwrap(),
            None
        );
    }
}

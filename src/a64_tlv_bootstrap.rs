/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Original guest helper calls for primordial-thread TLVs. Partial progress
//! never constitutes a dyld initialization or future-thread readiness receipt.
use super::bridge::{GuestBridge, GuestCall, ReturnValues, ServiceFrame};
use super::{dyld_helpers::Helpers, tlv::Plan, A64Cpu};
use std::{cell::{Cell, RefCell}, rc::Rc};

const ORIGINAL_TLV_THUNK: u64 = 0x1a6c7d8d0;
#[derive(Clone, Copy, Debug)]
enum Phase { Start, GlobalFree, GlobalExit, Thunk, Key, Allocate, Set, Get, Finished }
struct State {
    phase: Phase,
    result: Option<ReturnValues>,
    plans: Vec<Plan>,
    image: usize,
    key: u64,
    base: u64,
    keys: Vec<u64>,
    published: usize,
    catalogue: Rc<RefCell<super::tlv_lazy::Catalogue>>,
    blocks: Vec<(u64, u64)>,
    global_keys:[u64;2],
}
fn result(state: &mut State) -> Result<u64, String> {
    state.result.take().map(|r| r.integers[0]).ok_or_else(|| "missing genuine guest TLV callback result".into())
}
fn success(state: &mut State, operation: &str) -> Result<(), String> {
    let code = result(state)?;
    if code as u32 != 0 { return Err(format!("original TLV {operation} failed: {code:#x}")); }
    Ok(())
}
fn word(frame: &mut ServiceFrame<'_>, address: u64) -> Result<u64, String> {
    Ok(u64::from_le_bytes(frame.read(address, 8)?.try_into().unwrap()))
}
fn queue(frame: &mut ServiceFrame<'_>, state: &Rc<RefCell<State>>, target: u64,
    entry: u64, arguments: Vec<u64>, phase: Phase) -> Result<ReturnValues, String> {
    state.borrow_mut().phase = phase;
    let completion = Rc::clone(state);
    frame.request_guest_call(GuestCall { entry, integers: arguments, ..Default::default() }, move |outcome| {
        let returned = outcome?;
        let mut state = completion.try_borrow_mut().map_err(|_| "TLV completion state is borrowed")?;
        if state.result.is_some() { return Err("duplicate TLV callback completion".into()); }
        state.result = Some(returned);
        Ok(())
    })?;
    // This is a C initializer continuation, not Objective-C dispatch. x1 is
    // unused on re-entry; the bridge requires a nonzero continuation marker.
    frame.request_tail_dispatch(target, 1)?;
    Ok(ReturnValues::integer(0))
}
/// Register a bounded, flat continuation driver. Every allocator/key operation
/// executes an original guest member function with the real helper `this`.
pub(super) fn install(cpu: &mut A64Cpu, bridge: &mut GuestBridge, helpers: Helpers,
    arena: u64, plans: Vec<Plan>, owner: u64) -> Result<u64, String> {
    install_inner(cpu,bridge,helpers,arena,plans,owner,None,None)
}
pub(super) fn install_owned(cpu:&mut A64Cpu,bridge:&mut GuestBridge,helpers:Helpers,
    arena:u64,plans:Vec<Plan>,owner:u64,scheduler:Rc<RefCell<super::thread_scheduler_cpu::CpuScheduler>>,lease:super::execution_session::SessionLease)->Result<u64,String> {
    install_inner(cpu,bridge,helpers,arena,plans,owner,Some(scheduler),Some(lease))
}
fn install_inner(cpu:&mut A64Cpu,bridge:&mut GuestBridge,helpers:Helpers,
    arena:u64,plans:Vec<Plan>,owner:u64,scheduler:Option<Rc<RefCell<super::thread_scheduler_cpu::CpuScheduler>>>,lease:Option<super::execution_session::SessionLease>)->Result<u64,String> {
    if owner == 0 { return Err("TLV bootstrap requires an actual thread owner".into()); }
    if plans.len() > 4096 { return Err("TLV image count exceeds bootstrap budget".into()); }
    if cpu.mapped_permissions(ORIGINAL_TLV_THUNK).is_none_or(|p| p & 4 == 0) {
        return Err("audited original TLV thunk is not physically executable".into());
    }
    // A real malloc return must meet the declared alignment, not an adjusted
    // interior pointer which the registered free destructor could not own.
    if plans.iter().any(|p| p.alignment > 16 || p.template.len() > 512 * 1024) {
        return Err("TLV bootstrap requires unsupported aligned/large allocation policy".into());
    }
    for (index, plan) in plans.iter().enumerate() {
        if plans[..index].iter().any(|other| other.header == plan.header
            || plan.preallocated_key.is_some() && plan.preallocated_key == other.preallocated_key) {
            return Err("duplicate TLV image/preassigned-key ownership before bootstrap".into());
        }
    }
    let catalogue=Rc::new(RefCell::new(super::tlv_lazy::Catalogue::default()));
    let loader_lock=Rc::new(RefCell::new(super::loader_lock::LoaderLock::new()));
    let lazy_installed=if let Some(scheduler)=scheduler.as_ref() {
        if scheduler.borrow().current().map(|id|id.0)!=Some(owner) {return Err("TLV install owner is not the selected CPU".into());}
        super::tlv_lazy::install(cpu,bridge,helpers.clone(),catalogue.clone(),loader_lock.clone(),scheduler.clone())?;
        true
    }else{false};
    let state = Rc::new(RefCell::new(State { phase: Phase::Start, result: None,
        plans, image: 0, key: 0, base: 0, keys: Vec::new(), published: 0,
        catalogue, blocks: Vec::new(),global_keys:[0;2] }));
    let target = Rc::new(Cell::new(0));
    let address = Rc::clone(&target);
    let id = bridge.register_service(cpu, "__dyld_initializer_owned_TLV_prefix", move |frame| {
        if let Some(scheduler)=scheduler.as_ref() {
            if scheduler.borrow().current().map(|id|id.0)!=Some(owner) {return Err("TLV bootstrap CPU owner changed".into());}
        }
        let mut current = state.try_borrow_mut().map_err(|_| "TLV bootstrap is reentrant")?;
        let this = helpers.object;
        let dispatch = address.get();
        let call = match current.phase {
            Phase::Start => {loader_lock.borrow_mut().enter(owner)?;Some((6, vec![this, arena], Phase::GlobalFree))},
            Phase::GlobalFree => {
                success(&mut current, "global free-key creation")?;
                current.global_keys[0]=word(frame,arena)?;
                if current.global_keys[0] == 0 { return Err("original global TLV key is zero".into()); }
                Some((7, vec![this, arena + 8], Phase::GlobalExit))
            }
            Phase::GlobalExit => {
                success(&mut current, "thread-exit key creation")?;
                current.global_keys[1]=word(frame,arena+8)?;
                if current.global_keys[1] == 0 { return Err("original thread-exit key is zero".into()); }
                Some((18, vec![this], Phase::Thunk))
            }
            Phase::Thunk => {
                if result(&mut current)? != ORIGINAL_TLV_THUNK { return Err("original helper returned a different TLV thunk".into()); }
                None
            }
            Phase::Key => {
                success(&mut current, "image key initialization")?;
                let key = match current.plans[current.image].preallocated_key {
                    Some(key) => key,
                    None => word(frame, arena + 16)?,
                };
                if key == 0 || key >= 512 || current.keys.contains(&key) {
                    return Err("original TLV key is invalid or already owned by another image".into());
                }
                current.key = key;
                let size = current.plans[current.image].template.len() as u64;
                Some((1, vec![this, size], Phase::Allocate))
            }
            Phase::Allocate => {
                let base = result(&mut current)?;
                let plan = &current.plans[current.image];
                if base == 0 || base % plan.alignment != 0 { return Err("original TLV malloc returned null/misaligned storage".into()); }
                let size = plan.template.len() as u64;
                let end = base.checked_add(size).ok_or("original TLV malloc range overflow")?;
                if current.blocks.iter().any(|&(other, len)| base < other + len && other < end) {
                    return Err("original TLV malloc reused another image's live allocation".into());
                }
                // The allocation is owned by the real original malloc/free
                // pair, and the original bytes include zero-fill sections.
                frame.write(base, &plan.template)?;
                current.base = base;
                current.blocks.push((base, size));
                Some((9, vec![this, current.key, base], Phase::Set))
            }
            Phase::Set => {
                success(&mut current, "pthread_setspecific")?;
                Some((8, vec![this, current.key], Phase::Get))
            }
            Phase::Get => {
                let actual_lookup = result(&mut current)?;
                if actual_lookup != current.base { return Err("original pthread_getspecific did not return the actual TLV allocation".into()); }
                let plan = &current.plans[current.image];
                // Validate every original descriptor before publishing any.
                for descriptor in &plan.descriptors {
                    let bytes = frame.read(descriptor.slot, 24)?;
                    if u64::from_le_bytes(bytes[0..8].try_into().unwrap()) != descriptor.thunk
                        || u64::from_le_bytes(bytes[8..16].try_into().unwrap()) != descriptor.key
                        || u64::from_le_bytes(bytes[16..24].try_into().unwrap()) != descriptor.offset {
                        return Err("TLV descriptor changed before publication".into());
                    }
                }
                for descriptor in &plan.descriptors {
                    if descriptor.thunk != ORIGINAL_TLV_THUNK || descriptor.key != current.key {
                        let mut bytes = ORIGINAL_TLV_THUNK.to_le_bytes().to_vec();
                        bytes.extend_from_slice(&current.key.to_le_bytes());
                        frame.write(descriptor.slot, &bytes)?;
                    }
                }
                let header = plan.header;
                let bytes = plan.template.len();
                let descriptors = plan.descriptors.len();
                let pending_initializers = !plan.initializers.is_empty();
                let alignment = plan.alignment;
                let slots = plan.descriptors.iter().map(|d| (d.slot, d.offset)).collect::<Vec<_>>();
                let key = current.key;
                let base = current.base;
                {
                    let access = RefCell::new(&mut *frame);
                    current.catalogue.borrow_mut().storage.record_bound_with_memory(owner, header, key, base,
                        bytes as u64, alignment, &slots, actual_lookup,
                        |address, length| access.borrow_mut().read(address, length),
                        |address, length| access.borrow_mut().validate_read_write_range(address,
                            usize::try_from(length).map_err(|_| "TLV validation range exceeds host usize")?))?;
                }
                current.keys.push(key);
                current.published += descriptors;
                echo!("[a64] actual primordial TLV storage image={header:#x} key={key} bytes={bytes} descriptors={descriptors}; original malloc/setspecific/getspecific verified");
                if pending_initializers {
                    return Err("actual TLV initializer callbacks remain unexecuted; no image/TLS readiness receipt".into());
                }
                let plan=&current.plans[current.image];
                current.catalogue.borrow_mut().images.push(super::tlv_lazy::Image{
                    header,key,template:plan.template.clone(),alignment,slots});
                current.image += 1;
                None
            }
            Phase::Finished => return Err("partial TLV bootstrap cannot be replayed".into()),
        };
        if let Some((slot, args, phase)) = call {
            drop(current);
            return queue(frame, &state, dispatch, helpers.functions[slot], args, phase);
        }
        if current.image == current.plans.len() {
            loader_lock.borrow().require_owned(owner)?;
            loader_lock.borrow_mut().leave(owner)?;
            current.phase = Phase::Finished;
            if let (Some(lease),Some(scheduler))=(lease.as_ref(),scheduler.as_ref()) {
                if [word(frame,arena)?,word(frame,arena+8)?]!=current.global_keys {return Err("original global key cells changed before completion".into());}
                let catalogue=current.catalogue.try_borrow().map_err(|_|"TLV catalogue borrowed during completion")?;
                super::dyld_bootstrap_association::validate(lease,scheduler,owner,&helpers,current.global_keys,&current.plans,&catalogue,lazy_installed,loader_lock.borrow().is_unlocked(),|address,len|frame.read(address,len))?;
                echo!("[a64] audited v6 helper initialization returned with real keys/catalogue retained by live process session; no full runtime readiness receipt");
                return Ok(ReturnValues::integer(0));
            }
            if lazy_installed {
                echo!("[a64] real recursive loader lock released; selected-owner original-callback lazy TLV route installed for {} retained templates",current.image);
                return Err(format!("genuine primordial TLV storage published for {} images/{} descriptors; real loader lock and selected-thread lazy callback route installed; persistent process-session and remaining dyld bootstrap required; no dyld initialization receipt",current.image,current.published));
            }
            return Err(format!("genuine primordial TLV storage published for {} images/{} descriptors; loader-lock and future-thread lazy TLV publication remain required; no dyld initialization receipt", current.image, current.published));
        }
        let plan = &current.plans[current.image];
        let (slot, args) = match plan.preallocated_key {
            Some(key) => (21, vec![this, key]),
            None => { frame.write(arena + 16, &[0; 8])?; (6, vec![this, arena + 16]) },
        };
        drop(current);
        queue(frame, &state, dispatch, helpers.functions[slot], args, Phase::Key)
    })?;
    target.set(id.guest_address());
    Ok(id.guest_address())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn code(cpu: &mut A64Cpu, address: u64, words: &[u32]) {
        for (i, word) in words.iter().enumerate() {
            cpu.try_write_bytes(address + i as u64 * 4, &word.to_le_bytes()).unwrap();
        }
    }
    fn fixture(cached: bool, failure: bool) -> (A64Cpu, GuestBridge, u64) {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 5).unwrap();
        cpu.map_zeroed(0x40000, 16384, 3).unwrap();
        cpu.map_zeroed(ORIGINAL_TLV_THUNK & !4095, 4096, 5).unwrap();
        cpu.try_write_bytes(0x40008, &256u64.to_le_bytes()).unwrap();
        code(&mut cpu, 0x10000, &[0xd65f03c0]);
        // Real guest callback: increment the fixture's key source, store the
        // result through its AAPCS pointer argument, return pthread success.
        code(&mut cpu, 0x10100, &[0xf9400409, 0x91000529, 0xf9000409, 0xf9000029, 0x52800000, 0xd65f03c0]);
        code(&mut cpu, 0x10200, &[0xd2820000, 0xf2a00080, 0xd65f03c0]); // malloc fixture block 0x41000
        code(&mut cpu, 0x10300, &[0xf9000802, 0x52800000, 0xd65f03c0]); // setspecific
        code(&mut cpu, 0x10400, &[0xf9400800, 0xd65f03c0]); // getspecific
        let mut thunk_words = Vec::new();
        for i in 0..4u32 {
            thunk_words.push((if i == 0 { 0xd2800000 } else { 0xf2800000 })
                | (i << 21) | (((ORIGINAL_TLV_THUNK >> (i * 16)) as u32 & 0xffff) << 5));
        }
        thunk_words.push(0xd65f03c0);
        code(&mut cpu, 0x10500, &thunk_words);
        code(&mut cpu, 0x10600, &[if failure { 0x528000a0 } else { 0x52800000 }, 0xd65f03c0]);
        let mut functions = [0x10000; 22];
        functions[1] = 0x10200;
        functions[6] = 0x10100;
        functions[7] = 0x10100;
        functions[8] = 0x10400;
        functions[9] = 0x10300;
        functions[18] = 0x10500;
        functions[21] = 0x10600;
        let key = if cached { 126 } else { 0 };
        let thunk = if cached { ORIGINAL_TLV_THUNK } else { 0x10000 };
        cpu.try_write_bytes(0x42000, &thunk.to_le_bytes()).unwrap();
        cpu.try_write_bytes(0x42008, &(key as u64).to_le_bytes()).unwrap();
        cpu.try_write_bytes(0x42010, &1u64.to_le_bytes()).unwrap();
        let plan = Plan { header: 0x50000, template: vec![0xaa, 0xbb, 0], alignment: 8,
            descriptors: vec![super::super::tlv::Descriptor { slot: 0x42000, offset: 1, thunk, key }],
            initializers: vec![], preallocated_key: cached.then_some(key) };
        let mut bridge = GuestBridge::map(&mut cpu, 0x60000).unwrap();
        let entry = install(&mut cpu, &mut bridge, Helpers { object: 0x40000, vtable: 0, functions }, 0x43000, vec![plan], 1).unwrap();
        (cpu, bridge, entry)
    }
    #[test]
    fn real_guest_callbacks_publish_ordinary_template_without_readiness_receipt() {
        let (mut cpu, mut bridge, entry) = fixture(false, false);
        let error = bridge.call(&mut cpu, &GuestCall { entry, ..Default::default() }, 1000).unwrap_err();
        assert!(error.contains("storage published for 1 images/1 descriptors"), "{error}");
        assert_eq!(cpu.read_u64(0x42000), Some(ORIGINAL_TLV_THUNK));
        assert_eq!(cpu.read_u64(0x42008), Some(259));
        assert_eq!(cpu.read_u64(0x40010), Some(0x41000));
        assert_eq!(cpu.read_bytes(0x41000, 3).unwrap(), &[0xaa, 0xbb, 0]);
    }
    #[test]
    fn cached_key_is_adopted_and_callback_failure_does_not_publish() {
        let (mut cpu, mut bridge, entry) = fixture(true, false);
        let error = bridge.call(&mut cpu, &GuestCall { entry, ..Default::default() }, 1000).unwrap_err();
        assert!(error.contains("storage published for 1 images/1 descriptors"), "{error}");
        assert_eq!(cpu.read_u64(0x42008), Some(126));
        assert_eq!(cpu.read_u64(0x40008), Some(258)); // no extra key was minted
        let (mut cpu, mut bridge, entry) = fixture(true, true);
        let error = bridge.call(&mut cpu, &GuestCall { entry, ..Default::default() }, 1000).unwrap_err();
        assert!(error.contains("image key initialization failed"), "{error}");
        assert_eq!(cpu.read_u64(0x42008), Some(126));
        assert_eq!(cpu.read_u64(0x40010), Some(0));
        assert_eq!(cpu.read_bytes(0x41000, 3).unwrap(), &[0, 0, 0]);
    }
    #[test]
    fn service_storage_validation_requires_read_permission_without_writes() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x40000, 4096, 2).unwrap();
        cpu.try_write_bytes(0x40000, &[0x42]).unwrap();
        let mut bridge = GuestBridge::map(&mut cpu, 0x60000).unwrap();
        let service = bridge.register_service(&mut cpu, "validate_TLV_RW", |frame| {
            frame.validate_read_write_range(0x40000, 16)?;
            Ok(ReturnValues::integer(0))
        }).unwrap();
        let error = bridge.call(&mut cpu, &GuestCall { entry: service.guest_address(), ..Default::default() }, 20).unwrap_err();
        assert!(error.contains("not readable/writable"), "{error}");
        assert_eq!(cpu.read_bytes(0x40000, 1).unwrap(), &[0x42]);
    }
}

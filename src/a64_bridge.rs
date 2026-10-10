/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Explicit, bounded nested guest calls and registered host service traps.
//! Callers classify arguments themselves: this is not an Objective-C type
//! encoding parser, runtime initializer, or a general host function-pointer ABI.
use super::{A64Cpu, A64State};

const RETURN_SVC: u16 = 0x7c;
const SERVICE_SVC: u16 = 0x7d;
const CODE_SIZE: usize = 4096;
const STACK_SIZE: usize = 64 * 1024;
const MAX_SERVICES: usize = 128;
const MAX_TICKS: u64 = 1_000_000;
const MAX_SERVICE_MEMORY: usize = 1024 * 1024;
pub(super) const MAX_METADATA_MEMORY: usize = 16 * 1024 * 1024;
const MAX_METADATA_REQUEST: usize = 64 * 1024;
const MAX_METADATA_READS: usize = 1_000_000;
const MAX_DEPTH: usize = 8;
const STACK_SLICE: usize = STACK_SIZE / MAX_DEPTH;
const STACK_WINDOW: usize = STACK_SLICE / 2;
const RUNTIME_WINDOW:usize=64*1024;
const GUARD_BYTES:usize=4096;
pub(super) const RUNTIME_RESERVED_BYTES:u64=(CODE_SIZE+(RUNTIME_WINDOW+GUARD_BYTES)*MAX_DEPTH)as u64;
pub(super) const RUNTIME_MAPPED_BYTES:u64=(CODE_SIZE+RUNTIME_WINDOW*MAX_DEPTH)as u64;
fn restore_callback_writes(cpu:&mut A64Cpu,journal:&[(u64,u64,u32)])->Result<(),String>{
    let mut failure=None;
    for &(base,len,protection) in journal.iter().rev(){if let Err(error)=cpu.set_protection(base,len,protection){failure=Some(error);}}
    failure.map_or(Ok(()),Err)
}
fn prepare_callback_writes(cpu:&mut A64Cpu,ranges:&[(u64,u64)])->Result<Vec<(u64,u64,u32)>,String>{
    if ranges.len()>256{return Err("callback writable range count limit".into());}
    let mut sorted=ranges.to_vec();sorted.sort_unstable();let mut end_before=0;let mut total=0u64;let mut journal=Vec::new();
    for &(base,len) in &sorted {
        let end=base.checked_add(len).ok_or("callback protection overflow")?;
        total=total.checked_add(len).ok_or("callback protection size overflow")?;
        if len==0||base<end_before||total>64*1024*1024{return Err("callback writable ranges overlap or exceed budget".into());}end_before=end;
        let mut cursor=base;
        while cursor<end {
            let region=cpu.protection_region(cursor).ok_or("callback data range unmapped")?;
            if region.protection&1==0||region.protection&4!=0||region.max_protection&3!=3{return Err("callback data mapping does not permit bounded RW transition".into());}
            let finish=end.min(region.base.checked_add(region.len).ok_or("callback mapping overflow")?);
            if finish<=cursor||journal.len()>=4096{return Err("callback protection extent budget".into());}
            if region.protection&2==0 {journal.push((cursor,finish-cursor,region.protection));}
            cursor=finish;
        }
    }
    for (index,&(base,len,protection)) in journal.iter().enumerate(){
        if let Err(error)=cpu.set_protection(base,len,protection|2){restore_callback_writes(cpu,&journal[..index])?;return Err(error);}
    }
    Ok(journal)
}
pub(super) const RESERVED_BYTES: u64 = (CODE_SIZE + STACK_SIZE) as u64;
pub(super) const MAPPED_BYTES: u64 = (CODE_SIZE + STACK_WINDOW * MAX_DEPTH) as u64;

/// Already classified AAPCS64 register and stack arguments. Stack bytes are
/// laid out by the caller for the guest ABI (Apple and generic AAPCS packing
/// differ). Homogeneous aggregates use the SIMD argument registers. Larger
/// indirect results require a separately validated writable guest range.
#[derive(Default)]
pub(super) struct GuestCall {
    pub entry: u64,
    pub integers: Vec<u64>,
    pub vectors: Vec<[u64; 2]>,
    pub stack_arguments: Vec<u8>,
    pub indirect_result: Option<(u64, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReturnValues {
    pub integers: [u64; 2],
    pub vectors: [[u64; 2]; 4],
}
impl ReturnValues {
    pub(super) fn integer(value: u64) -> Self {
        Self {
            integers: [value, 0],
            vectors: [[0; 2]; 4],
        }
    }
    fn capture(cpu: &A64Cpu) -> Self {
        Self {
            integers: [cpu.reg(0), cpu.reg(1)],
            vectors: std::array::from_fn(|i| cpu.vector(i)),
        }
    }
    fn apply(&self, cpu: &mut A64Cpu) {
        for (i, &value) in self.integers.iter().enumerate() {
            cpu.set_reg(i, value);
        }
        for (i, &value) in self.vectors.iter().enumerate() {
            cpu.set_vector(i, value);
        }
    }
}

/// A service receives data arguments and bounded guest memory access only;
/// it cannot alter guest PC, LR, SP or cast a guest address to a host pointer.
pub(super) struct ServiceFrame<'a> {
    cpu: &'a mut A64Cpu,
    remaining_memory: usize,
    stack_range: (u64, u64),
    depth: usize,
    continuations: &'a mut Vec<GuestContinuation>,
    tail_dispatch: &'a mut Option<(u64, u64, Option<u64>)>,
}
/// A distinct, read-only parser budget. Ordinary service arguments and writes
/// retain their smaller budget; this reader never grants execution permission.
pub(super) struct MetadataReader<'a> {
    cpu: &'a A64Cpu,
    limit: usize,
    used: usize,
    requests: usize,
}
impl MetadataReader<'_> {
    pub(super) fn statistics(&self) -> (usize, usize) {
        (self.used, self.requests)
    }
    pub(super) fn read(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if length > MAX_METADATA_REQUEST || self.requests >= MAX_METADATA_READS {
            return Err("metadata-only request/count limit exceeded".into());
        }
        let next = self
            .used
            .checked_add(length)
            .filter(|&next| next <= self.limit)
            .ok_or("metadata-only read budget exceeded before allocation")?;
        self.used = next;
        self.requests += 1;
        let mut result = vec![0; length];
        self.cpu.read_guest_into(address, &mut result)?;
        Ok(result)
    }
}
struct GuestContinuation {
    writable_ranges: Vec<(u64,u64)>,
    call: GuestCall,
    start: Option<Box<dyn FnOnce() -> Result<bool, String>>>,
    completion: Box<dyn FnOnce(Result<ReturnValues, String>) -> Result<(), String>>,
}
impl ServiceFrame<'_> {
    pub(super) fn metadata_reader(&self, limit: usize) -> Result<MetadataReader<'_>, String> {
        if limit == 0 || limit > MAX_METADATA_MEMORY {
            return Err("metadata-only budget request invalid".into());
        }
        Ok(MetadataReader {
            cpu: self.cpu,
            limit,
            used: 0,
            requests: 0,
        })
    }
    /// Preserve the original unknown message ABI. Pending initialization
    /// continuations run first; only PC and the canonical SEL register change.
    pub(super) fn request_tail_dispatch(
        &mut self,
        entry: u64,
        canonical_selector: u64,
    ) -> Result<(), String> {
        validate_executable(self.cpu, entry, 4)?;
        if canonical_selector == 0 || self.tail_dispatch.is_some() {
            return Err("tail dispatch requires one nonzero canonical selector".into());
        }
        *self.tail_dispatch = Some((entry, canonical_selector, None));
        Ok(())
    }
    /// objc_msgSendSuper2's record argument becomes the genuine receiver.
    /// All other unknown message arguments retain their original ABI state.
    pub(super) fn request_tail_dispatch_receiver(
        &mut self,
        entry: u64,
        canonical_selector: u64,
        receiver: u64,
    ) -> Result<(), String> {
        self.request_tail_dispatch(entry, canonical_selector)?;
        self.tail_dispatch.as_mut().unwrap().2 = Some(receiver);
        Ok(())
    }
    pub(super) fn integer(&self, index: usize) -> Result<u64, String> {
        if index >= 8 {
            return Err("host service integer argument index exceeds x7".into());
        }
        Ok(self.cpu.reg(index))
    }
    pub(super) fn vector(&self, index: usize) -> Result<[u64; 2], String> {
        if index >= 8 {
            return Err("host service SIMD argument index exceeds v7".into());
        }
        Ok(self.cpu.vector(index))
    }
    pub(super) fn indirect_result(&self) -> u64 {
        self.cpu.reg(8)
    }
    pub(super) fn caller_return_address(&self) -> u64 {
        self.cpu.reg(30)
    }
    /// Queue a real guest call after this handler releases its CPU borrow.
    /// Completion receives all post-enqueue validation/execution failures.
    /// Guest memory cannot be rolled back by this bridge; lifecycle owners
    /// must quarantine partially executed disposal, never fake a receipt.
    pub(super) fn request_guest_call(
        &mut self,
        call: GuestCall,
        completion: impl FnOnce(Result<ReturnValues, String>) -> Result<(), String> + 'static,
    ) -> Result<(), String> {
        if self.depth + 1 >= MAX_DEPTH || self.continuations.len() >= 8 {
            return Err("guest continuation depth/count limit exceeded".into());
        }
        self.continuations.push(GuestContinuation {
            writable_ranges:Vec::new(),
            call,
            start: None,
            completion: Box::new(completion),
        });
        Ok(())
    }
    /// Only temporarily reopen actual data mappings whose real maximum permits
    /// writes. The bridge restores protections even when guest execution fails.
    pub(super) fn request_guest_call_with_writable_ranges(&mut self,call:GuestCall,ranges:Vec<(u64,u64)>,completion:impl FnOnce(Result<ReturnValues,String>)->Result<(),String>+'static)->Result<(),String>{
        if ranges.len()>256{return Err("callback protection range budget exceeded".into());}
        self.request_guest_call(call,completion)?;
        self.continuations.last_mut().unwrap().writable_ranges=ranges;
        Ok(())
    }
    pub(super) fn validate_executable_pointer(&mut self,address:u64)->Result<(),String>{self.charge(4)?;validate_executable(self.cpu,address,4)}
    /// Class initialization owners transition state in execution order, after
    /// earlier parent continuations completed. Completion still handles all
    /// errors, including a rejected start or later ABI validation failure.
    pub(super) fn request_guest_call_with_start(
        &mut self,
        call: GuestCall,
        start: impl FnOnce() -> Result<bool, String> + 'static,
        completion: impl FnOnce(Result<ReturnValues, String>) -> Result<(), String> + 'static,
    ) -> Result<(), String> {
        self.request_guest_call(call, completion)?;
        self.continuations.last_mut().unwrap().start = Some(Box::new(start));
        Ok(())
    }
    /// Read a naturally aligned, caller-classified stack argument. The
    /// trampoline leaves SP untouched, so offset zero is the first stack slot.
    /// This intentionally does not guess Apple's smaller-argument packing.
    pub(super) fn stack_u64(&mut self, offset: usize) -> Result<u64, String> {
        if offset & 7 != 0 || self.cpu.sp() & 15 != 0 {
            return Err("host service stack argument alignment invalid".into());
        }
        let address = self
            .cpu
            .sp()
            .checked_add(offset as u64)
            .ok_or("host service stack address overflow")?;
        let end = address
            .checked_add(8)
            .ok_or("host service stack range overflow")?;
        if address < self.stack_range.0 || end > self.stack_range.1 {
            return Err("host service stack argument outside bridge stack".into());
        }
        Ok(u64::from_le_bytes(
            self.read(address, 8)?.try_into().unwrap(),
        ))
    }
    fn charge(&mut self, len: usize) -> Result<(), String> {
        self.remaining_memory = self
            .remaining_memory
            .checked_sub(len)
            .ok_or("host service memory budget exceeded")?;
        Ok(())
    }
    pub(super) fn read(&mut self, address: u64, len: usize) -> Result<Vec<u8>, String> {
        self.charge(len)?;
        let mut bytes = vec![0; len];
        self.cpu.read_guest_into(address, &mut bytes)?;
        Ok(bytes)
    }
    pub(super) fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), String> {
        self.charge(bytes.len())?;
        self.cpu.write_guest_into(address, bytes)
    }
    /// Validate a whole caller-owned read/write range without exposing the CPU or
    /// allocating a buffer. Validation consumes the ordinary service budget.
    pub(super) fn validate_read_write_range(&mut self, address: u64, length: usize) -> Result<(), String> {
        self.charge(length)?;
        self.cpu.validate_guest_write(address, length)?;
        let end = address.checked_add(length as u64).ok_or("service read/write range overflow")?;
        let mut cursor = address;
        while cursor < end {
            let region = self.cpu.protection_region(cursor).ok_or("service read/write range unmapped")?;
            if region.protection & 3 != 3 { return Err("service range is not readable/writable".into()); }
            let next = region.base.checked_add(region.len).ok_or("service mapping overflow")?.min(end);
            if next <= cursor { return Err("invalid service read/write mapping extent".into()); }
            cursor = next;
        }
        Ok(())
    }
}

type Handler = Box<dyn FnMut(&mut ServiceFrame<'_>) -> Result<ReturnValues, String>>;
struct Service {
    name: String,
    address: u64,
    handler: Handler,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ServiceId {
    address: u64,
}
impl ServiceId {
    pub(super) fn guest_address(self) -> u64 {
        self.address
    }
}

pub(super) struct GuestBridge {
    code: u64,
    stack: u64,
    stack_size:usize,
    stack_slice:usize,
    stack_window:usize,
    services: Vec<Service>,
    progress: Option<ProgressTrace>,
    thread_storage:Option<std::rc::Rc<super::thread_storage::ThreadStorage>>,
}
/// Address-only diagnostic sampling. It never grants execution permissions or
/// changes the caller's total tick budget.
struct ProgressTrace { entry:u64, start:u64, end:u64, first:u64, last:u64, samples:usize, until_sample:u64 }
fn exhausted_evidence(cpu:&A64Cpu,depth:usize)->String{
    let mut error=format!("guest call tick budget exhausted at {:#x}; guest lr={:#x} sp={:#x} depth={depth} before context restoration",cpu.pc(),cpu.reg(A64Cpu::LR),cpu.sp());
    if let Some(metadata)=super::legacy_objc_notify::protocol_scan_evidence(cpu){error.push_str("; ");error.push_str(&metadata);}
    error
}

fn validate_executable(cpu: &A64Cpu, address: u64, len: usize) -> Result<(), String> {
    if address == 0 || address & 3 != 0 || len == 0 {
        return Err("guest executable address is null, misaligned or empty".into());
    }
    address
        .checked_add(len as u64)
        .ok_or("guest executable range overflow")?;
    for offset in 0..len {
        if !cpu
            .mapped_permissions(address + offset as u64)
            .is_some_and(|p| p & 4 != 0)
        {
            return Err("guest call target is outside executable memory".into());
        }
    }
    Ok(())
}

impl GuestBridge {
    pub(super) fn trace_callback_progress(&mut self,cpu:&A64Cpu,entry:u64,start:u64,end:u64,first:u64,last:u64)->Result<(),String>{
        if end<=start||end-start>1024*1024||entry<start||entry>=end{return Err("invalid bounded callback trace range".into());}
        validate_executable(cpu,entry,4)?;
        for address in [first,last]{cpu.read_u64(address).ok_or("callback trace metadata is unreadable")?;}
        self.progress=Some(ProgressTrace{entry,start,end,first,last,samples:0,until_sample:0});Ok(())
    }
    pub(super) fn scratch_end(&self)->Result<u64,String>{self.stack.checked_add(self.stack_size as u64).ok_or_else(||"bridge scratch end overflow".into())}
    pub(super) fn map_runtime(cpu:&mut A64Cpu,base:u64)->Result<Self,String>{Self::map_with_window(cpu,base,RUNTIME_WINDOW)}
    /// Reserve one RX trampoline page and separate RW, non-executable stacks.
    /// The caller owns placement; mapping overlap is an error. There is no
    /// Each nested call gets a 4KiB window separated by an unmapped 4KiB guard.
    /// The caller's original stack is never reused.
    pub(super) fn map(cpu: &mut A64Cpu, base: u64) -> Result<Self, String> {
        Self::map_with_window(cpu,base,STACK_WINDOW)
    }
    fn map_with_window(cpu:&mut A64Cpu,base:u64,window:usize)->Result<Self,String>{
        if window<4096||window>1024*1024||window%4096!=0{return Err("invalid bounded guest stack window".into());}
        let stack_slice=window.checked_add(GUARD_BYTES).ok_or("bridge slice overflow")?;
        let stack_size=stack_slice.checked_mul(MAX_DEPTH).ok_or("bridge stack size overflow")?;
        if base == 0 || base & 4095 != 0 {
            return Err("bridge scratch must be nonzero and page aligned".into());
        }
        let stack = base
            .checked_add(CODE_SIZE as u64)
            .ok_or("bridge scratch overflow")?;
        let end = stack
            .checked_add(stack_size as u64)
            .ok_or("bridge stack overflow")?;
        // Validate both ranges before introducing any mapping.
        for address in base..end {
            if cpu.mapped_permissions(address).is_some() {
                return Err("bridge scratch overlaps existing memory".into());
            }
        }
        cpu.map_zeroed(base, CODE_SIZE, 5)?;
        for depth in 0..MAX_DEPTH {
            let top = end - (depth * stack_slice) as u64;
            cpu.map_zeroed(top - window as u64, window, 3)?;
        }
        cpu.try_write_bytes(
            base,
            &(0xd4000001u32 | ((RETURN_SVC as u32) << 5)).to_le_bytes(),
        )?;
        Ok(Self {
            code: base,
            stack,
            stack_size,stack_slice,stack_window:window,
            services: vec![],
            progress: None,
            thread_storage:None,
        })
    }
    pub(super) fn set_thread_storage(&mut self,state:std::rc::Rc<super::thread_storage::ThreadStorage>){self.thread_storage=Some(state);}
    pub(super) fn register_service(
        &mut self,
        cpu: &mut A64Cpu,
        name: &str,
        handler: impl FnMut(&mut ServiceFrame<'_>) -> Result<ReturnValues, String> + 'static,
    ) -> Result<ServiceId, String> {
        if name.is_empty() || name.len() > 256 || self.services.iter().any(|s| s.name == name) {
            return Err("host service name empty, too long or already registered".into());
        }
        if self.services.len() >= MAX_SERVICES {
            return Err("host service registry limit exceeded".into());
        }
        let token = self.services.len() + 1;
        let address = self.code + (token as u64) * 16;
        validate_executable(cpu, address, 12)?;
        let instructions = [
            0xd2800010u32 | ((token as u32) << 5), // movz x16, token
            0xd4000001u32 | ((SERVICE_SVC as u32) << 5),
            0xd65f03c0, // ret; LR remains the guest caller's return address
        ];
        let bytes: Vec<u8> = instructions
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        cpu.try_write_bytes(address, &bytes)?;
        self.services.push(Service {
            name: name.into(),
            address,
            handler: Box::new(handler),
        });
        Ok(ServiceId { address })
    }
    /// Execute a nested guest call. CPU registers (including SIMD/FP state)
    /// are restored on success or error; guest memory and deliberate host
    /// service state changes persist. A tick limit cannot preempt a host
    /// callback: registered callbacks must themselves be bounded and nonblocking.
    /// Prove a service belongs to this bridge and still contains its exact
    /// registered token/SVC/RET trampoline. RX membership alone is insufficient.
    pub(super) fn validate_registered_service(
        &self,
        cpu: &A64Cpu,
        name: &str,
        address: u64,
    ) -> Result<ServiceId, String> {
        let (index, service) = self
            .services
            .iter()
            .enumerate()
            .find(|(_, service)| service.name == name)
            .ok_or("cannot validate an unregistered host service")?;
        if address != service.address {
            return Err("registered host service address mismatch".into());
        }
        validate_executable(cpu, address, 12)?;
        let expected = [
            0xd2800010u32 | (((index + 1) as u32) << 5),
            0xd4000001 | ((SERVICE_SVC as u32) << 5),
            0xd65f03c0,
        ];
        let mut bytes = [0; 12];
        cpu.read_guest_into(address, &mut bytes)?;
        for (word, expected) in bytes.chunks_exact(4).zip(expected) {
            if u32::from_le_bytes(word.try_into().unwrap()) != expected {
                return Err("registered service trampoline changed".into());
            }
        }
        Ok(ServiceId { address })
    }

    pub(super) fn replace_registered_service(
        &mut self,
        cpu: &A64Cpu,
        name: &str,
        handler: impl FnMut(&mut ServiceFrame<'_>) -> Result<ReturnValues, String> + 'static,
    ) -> Result<ServiceId, String> {
        let (index, service) = self
            .services
            .iter_mut()
            .enumerate()
            .find(|(_, s)| s.name == name)
            .ok_or("cannot upgrade an unregistered host service")?;
        validate_executable(cpu, service.address, 12)?;
        let expected = [
            0xd2800010u32 | (((index + 1) as u32) << 5),
            0xd4000001 | ((SERVICE_SVC as u32) << 5),
            0xd65f03c0,
        ];
        let mut bytes = [0; 12];
        cpu.read_guest_into(service.address, &mut bytes)?;
        for (word, expected) in bytes.chunks_exact(4).zip(expected) {
            if u32::from_le_bytes(word.try_into().unwrap()) != expected {
                return Err("registered service trampoline changed before upgrade".into());
            }
        }
        service.handler = Box::new(handler);
        Ok(ServiceId {
            address: service.address,
        })
    }
    pub(super) fn call(
        &mut self,
        cpu: &mut A64Cpu,
        call: &GuestCall,
        budget: u64,
    ) -> Result<ReturnValues, String> {
        if budget == 0 || budget > MAX_TICKS {
            return Err("guest call tick budget outside bounded range".into());
        }
        let mut ticks = budget;
        self.call_inner(
            cpu,
            call,
            &mut ticks,
            0,
            false,
            &mut |_| Ok(()),
            &mut reject_supervisor,
        )
    }
    /// Diagnostic entry-prefix execution checks every instruction boundary.
    /// It cannot cross an uninitialized runtime boundary within a JIT block.
    pub(super) fn call_with_instruction_policy(
        &mut self,
        cpu: &mut A64Cpu,
        call: &GuestCall,
        budget: u64,
        policy: &mut dyn FnMut(&A64Cpu) -> Result<(), String>,
    ) -> Result<ReturnValues, String> {
        if budget == 0 || budget > MAX_TICKS {
            return Err("guest call tick budget outside bounded range".into());
        }
        let mut ticks = budget;
        self.call_inner(
            cpu,
            call,
            &mut ticks,
            0,
            true,
            policy,
            &mut reject_supervisor,
        )
    }
    /// Explicit diagnostic supervisor services; ordinary calls retain rejection.
    pub(super) fn call_with_supervisor_handler(
        &mut self,
        cpu: &mut A64Cpu,
        call: &GuestCall,
        budget: u64,
        handler: &mut dyn FnMut(&mut A64Cpu, u16) -> Result<(), String>,
    ) -> Result<ReturnValues, String> {
        if budget == 0 || budget > MAX_TICKS {
            return Err("guest call tick budget outside bounded range".into());
        }
        let mut ticks = budget;
        self.call_inner(cpu, call, &mut ticks, 0, false, &mut |_| Ok(()), handler)
    }
    /// This separate diagnostic path requires an unforgeable, revalidated
    /// original-initializer permit. General call limits remain MAX_TICKS.
    pub(super) fn call_initializer_with_supervisor(
        &mut self,cpu:&mut A64Cpu,call:&GuestCall,
        permit:&super::initializer_budget::InitializerBudget,
        handler:&mut dyn FnMut(&mut A64Cpu,u16)->Result<(),String>,
    )->Result<ReturnValues,String>{
        permit.validate(cpu,call.entry,permit.ticks())?;
        let mut ticks=permit.ticks();
        self.call_inner(cpu,call,&mut ticks,0,false,&mut |_|Ok(()),handler)
    }
    pub(super) fn call_legacy_initializer_with_supervisor(
        &mut self,cpu:&mut A64Cpu,call:&GuestCall,
        permit:&super::legacy_initializer_budget::LegacyBudget,
        handler:&mut dyn FnMut(&mut A64Cpu,u16)->Result<(),String>,
    )->Result<ReturnValues,String>{
        permit.validate(cpu,call.entry)?;
        let mut ticks=permit.ticks();
        self.call_inner(cpu,call,&mut ticks,0,false,&mut |_|Ok(()),handler)
    }
    pub(super) fn instruction_ranges(&self) -> Vec<(u64, u64)> {
        std::iter::once((self.code, self.code + 4))
            .chain(
                self.services
                    .iter()
                    .map(|service| (service.address, service.address + 12)),
            )
            .collect()
    }
    fn call_inner(
        &mut self,
        cpu: &mut A64Cpu,
        call: &GuestCall,
        ticks: &mut u64,
        depth: usize,
        single_step: bool,
        policy: &mut dyn FnMut(&A64Cpu) -> Result<(), String>,
        supervisor: &mut dyn FnMut(&mut A64Cpu, u16) -> Result<(), String>,
    ) -> Result<ReturnValues, String> {
        if *ticks == 0 || depth >= MAX_DEPTH {
            return Err("guest continuation depth/tick budget exhausted".into());
        }
        validate_executable(cpu, call.entry, 4)?;
        if call.integers.len() > 8 || call.vectors.len() > 8 {
            return Err("guest arguments exceed register ABI".into());
        }
        let stack_len = call
            .stack_arguments
            .len()
            .checked_add(15)
            .ok_or("guest stack arguments overflow")?
            & !15;
        if stack_len > self.stack_window / 2 {
            return Err("guest stack argument limit exceeded".into());
        }
        if let Some((address, len)) = call.indirect_result {
            if address == 0 || len == 0 || len > MAX_SERVICE_MEMORY {
                return Err("invalid guest indirect result range".into());
            }
            cpu.validate_guest_write(address, len)?;
        }
        validate_executable(cpu, self.code, 4)?;
        let saved = cpu.save_context();
        let result = (|| {
            let slice_top = self.stack + self.stack_size as u64 - (depth * self.stack_slice) as u64;
            let slice_bottom = slice_top - self.stack_window as u64;
            let sp = slice_top - stack_len as u64;
            if sp & 15 != 0 {
                return Err("guest bridge stack is not 16-byte aligned".into());
            }
            cpu.write_guest_into(sp, &call.stack_arguments)?;
            for i in 0..8 {
                cpu.set_reg(i, call.integers.get(i).copied().unwrap_or(0));
                cpu.set_vector(i, call.vectors.get(i).copied().unwrap_or([0; 2]));
            }
            cpu.set_reg(8, call.indirect_result.map(|r| r.0).unwrap_or(0));
            cpu.set_sp(sp);
            cpu.set_reg(A64Cpu::LR, self.code);
            cpu.set_pc(call.entry);
            let mut service_calls = 0;
            loop {
                policy(cpu)?;
                let state = if single_step {
                    if *ticks == 0 {
                        return Err(exhausted_evidence(cpu,depth));
                    }
                    *ticks -= 1;
                    cpu.run_or_step(None)
                } else {
                    if self.progress.as_ref().is_some_and(|trace|trace.entry==call.entry&&trace.samples<8){
                        let trace=self.progress.as_mut().unwrap();
                        if trace.until_sample==0{trace.until_sample=(*ticks/(8-trace.samples)as u64).max(1);}
                        let allowance=trace.until_sample.min(*ticks);
                        let mut remaining=allowance;
                        let state=cpu.run_or_step(Some(&mut remaining));
                        *ticks-=allowance-remaining;
                        let trace=self.progress.as_mut().unwrap();
                        trace.until_sample-=allowance-remaining;
                        // An early syscall/host-service return is not a timed
                        // landmark. Retain its unspent window through handling.
                        if trace.until_sample==0||*ticks==0{
                            trace.samples+=1;
                            let header=if cpu.pc()>=trace.start&&cpu.pc()<trace.end{Some(cpu.reg(28))}else{None};
                            echo!("[a64] bounded callback progress sample={} remaining_ticks={} pc={:#x} lr={:#x} sp={:#x} depth={} current_header={:?} first_header={:?} last_header={:?}",trace.samples,*ticks,cpu.pc(),cpu.reg(A64Cpu::LR),cpu.sp(),depth,header,cpu.read_u64(trace.first),cpu.read_u64(trace.last));
                            if trace.entry==0x1800b3aa4&&(0x1800ad244..0x1800ad34c).contains(&cpu.pc())
                                &&cpu.read_bytes(0x1800ad2d8,4).is_some_and(|bytes|bytes==0x8b2852d8u32.to_le_bytes()){
                                let table=cpu.reg(21);let buckets=cpu.reg(22);
                                if let Some(bytes)=table.checked_add(8).and_then(|at|cpu.read_bytes(at,8)){
                                    let count=u32::from_le_bytes(bytes[..4].try_into().unwrap());let mask=u32::from_le_bytes(bytes[4..].try_into().unwrap());
                                    if mask<=0x00ff_ffff&&count as u64<=mask as u64+1{
                                        echo!("[a64] bounded original NXMap metadata table={table:#x} buckets={buckets:#x} count={count} mask={mask} initial_bucket={} current_bucket={}; no keys or strings logged",cpu.reg(23)as u32,cpu.reg(25)as u32);
                                    }
                                }
                            }
                        }
                        state
                    }else{cpu.run_or_step(Some(ticks))}
                };
                match state {
                    A64State::Svc(RETURN_SVC) if cpu.pc() == self.code + 4 => {
                        return Ok(ReturnValues::capture(cpu))
                    }
                    A64State::Svc(SERVICE_SVC) => {
                        service_calls += 1;
                        if service_calls > 65536 {
                            return Err("host service call budget exceeded".into());
                        }
                        let token = cpu.reg(16);
                        let index = usize::try_from(token)
                            .ok()
                            .and_then(|n| n.checked_sub(1))
                            .filter(|&i| i < self.services.len())
                            .ok_or("unregistered host service token")?;
                        let service = &mut self.services[index];
                        if service_calls % 5000 == 0 {
                            echo!("[a64] host service call count={service_calls} latest={}", service.name);
                        }
                        if cpu.pc() != service.address + 8 {
                            return Err("host service trap is outside registered trampoline".into());
                        }
                        let mut continuations = Vec::new();
                        let mut tail_dispatch = None;
                        let outcome = (service.handler)(&mut ServiceFrame {
                            cpu,
                            remaining_memory: MAX_SERVICE_MEMORY,
                            stack_range: (slice_bottom, slice_top),
                            depth,
                            continuations: &mut continuations,
                            tail_dispatch: &mut tail_dispatch,
                        });
                        let values = match outcome {
                            Ok(values) => values,
                            Err(error) => {
                                let reason = format!("host service {}: {error}", service.name);
                                for continuation in continuations {
                                    let _ = (continuation.completion)(Err(
                                        "host handler failed before queued guest execution".into(),
                                    ));
                                }
                                return Err(reason);
                            }
                        };
                        let mut pending = continuations.into_iter();
                        while let Some(continuation) = pending.next() {
                            let preflight = validate_executable(cpu, continuation.call.entry, 4)
                                .and_then(|_| {
                                    if *ticks == 0 || depth + 1 >= MAX_DEPTH {
                                        Err("guest continuation depth/tick budget exhausted".into())
                                    } else {
                                        Ok(())
                                    }
                                });
                            let mut restored=Vec::new();
                            let result = preflight
                                .and_then(|_| continuation.start.map_or(Ok(true), |start| start()))
                                .and_then(|execute| {
                                    if execute {
                                        restored=prepare_callback_writes(cpu,&continuation.writable_ranges)?;
                                        self.call_inner(
                                            cpu,
                                            &continuation.call,
                                            ticks,
                                            depth + 1,
                                            single_step,
                                            policy,
                                            supervisor,
                                        )
                                    } else {
                                        Ok(ReturnValues::integer(0))
                                    }
                                });
                            let result=match restore_callback_writes(cpu,&restored){Ok(())=>result,Err(error)=>Err(format!("callback protection restoration failed: {error}; guest outcome={result:?}"))};
                            let error = result.as_ref().err().cloned();
                            let completion_error = (continuation.completion)(result).err();
                            if let Some(error) = error.or(completion_error) {
                                for canceled in pending {
                                    let _ = (canceled.completion)(Err("previous queued guest call failed; this call did not execute".into()));
                                }
                                return Err(error);
                            }
                        }
                        if let Some((entry, selector, receiver)) = tail_dispatch {
                            // Revalidate after guest initialization; it may
                            // have changed memory. Registers remain those of
                            // the original service caller, including x8/SP/LR.
                            validate_executable(cpu, entry, 4)?;
                            cpu.set_reg(1, selector);
                            if let Some(receiver) = receiver {
                                cpu.set_reg(0, receiver);
                            }
                            cpu.set_pc(entry);
                        } else {
                            values.apply(cpu);
                        }
                    }
                    A64State::Svc(immediate) => supervisor(cpu, immediate)?,
                    A64State::Normal if *ticks != 0 => (),
                    A64State::Normal => return Err(exhausted_evidence(cpu,depth)),
                    state => {
                        let regs: Vec<u64> = (0..31).map(|i| cpu.reg(i)).collect();
                        let lr = cpu.reg(A64Cpu::LR);
                        let insns = lr.checked_sub(16).and_then(|at| cpu.read_bytes(at, 24));
                        echo!("[a64] guest bridge stopped: {:?} at {:#x} lr={:#x} sp={:#x}\nregs={:x?}\ninsns={:x?}", state, cpu.pc(), lr, cpu.sp(), regs, insns);
                        return Err(format!(
                            "guest bridge stopped: {state:?} at {:#x}; guest lr={:#x} sp={:#x} before context restoration",
                            cpu.pc(), cpu.reg(A64Cpu::LR), cpu.sp()
                        ))
                    }
                }
                if *ticks == 0 {
                    return Err(exhausted_evidence(cpu,depth));
                }
            }
        })();
        let unauthorized_tsd=self.thread_storage.as_ref().is_some_and(|state|cpu.tpidrro_el0()!=state.current());
        cpu.restore_context(&saved);
        if let Some(tsd)=self.thread_storage.as_ref().and_then(|state|state.effect()){cpu.set_tpidrro_el0(tsd);}
        if unauthorized_tsd{return Err("guest thread-storage changed without approved kernel transition".into());}
        result
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn early_syscalls_do_not_consume_late_progress_landmarks(){
        use super::*;let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();
        let code:Vec<u8>=[0xd4001001u32,0x17ffffff].into_iter().flat_map(u32::to_le_bytes).collect();cpu.try_write_bytes(0x10000,&code).unwrap();cpu.map_zeroed(0x11000,4096,3).unwrap();
        let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();bridge.trace_callback_progress(&cpu,0x10000,0x10000,0x10008,0x11000,0x11008).unwrap();let mut calls=0;
        let error=bridge.call_with_supervisor_handler(&mut cpu,&GuestCall{entry:0x10000,..Default::default()},80_000,&mut|_,_|{calls+=1;if calls==8{Err("fixture stops after eight early syscalls".into())}else{Ok(())}}).unwrap_err();
        assert!(error.contains("fixture stops"));assert_eq!(calls,8);assert_eq!(bridge.progress.as_ref().unwrap().samples,0);assert!(bridge.progress.as_ref().unwrap().until_sample>0);
    }
    #[test]
    fn progress_sampling_preserves_total_budget_and_caller_context(){
        use super::*;
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();
        cpu.try_write_bytes(0x10000,&0x14000000u32.to_le_bytes()).unwrap();
        cpu.map_zeroed(0x11000,4096,3).unwrap();
        let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();
        bridge.trace_callback_progress(&cpu,0x10000,0x10000,0x11000,0x11000,0x11008).unwrap();
        cpu.set_pc(0x10040);cpu.set_sp(0x12340);cpu.set_reg(A64Cpu::LR,0x12380);
        let mut ticks=25_001;
        let error=bridge.call_inner(&mut cpu,&GuestCall{entry:0x10000,..Default::default()},&mut ticks,0,false,&mut |_|Ok(()),&mut |_,_|Ok(())).unwrap_err();
        assert!(error.contains("tick budget exhausted"));assert_eq!(ticks,0);
        assert_eq!(bridge.progress.as_ref().unwrap().samples,8);
        assert_eq!(cpu.pc(),0x10040);assert_eq!(cpu.sp(),0x12340);assert_eq!(cpu.reg(A64Cpu::LR),0x12380);
        assert!(bridge.trace_callback_progress(&cpu,0x10000,0x10000,0x11000,0xdead,0x11008).is_err());
    }
    #[test]
    fn exhausted_budget_reports_active_guest_addresses_before_restore() {
        use super::*;
        for single_step in [false,true] {
            let mut cpu=A64Cpu::new_sparse();
            cpu.map_zeroed(0x10000,4096,5).unwrap();
            cpu.try_write_bytes(0x10000,&0x14000000u32.to_le_bytes()).unwrap();
            let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();
            cpu.set_pc(0x10040);cpu.set_sp(0x12340);cpu.set_reg(A64Cpu::LR,0x12380);
            let saved=cpu.save_context();let mut ticks=8;
            let error=bridge.call_inner(&mut cpu,&GuestCall{entry:0x10000,..Default::default()},&mut ticks,0,single_step,&mut |_|Ok(()),&mut |_,_|Ok(())).unwrap_err();
            assert!(error.contains("tick budget exhausted at 0x10000"),"{error}");
            assert!(error.contains("guest lr=0x20000"),"{error}");
            assert!(error.contains("depth=0 before context restoration"),"{error}");
            assert_eq!(cpu.pc(),0x10040);assert_eq!(cpu.sp(),0x12340);assert_eq!(cpu.reg(A64Cpu::LR),0x12380);
            cpu.restore_context(&saved);
        }
    }
    #[test]
    fn explicit_upgrade_preserves_bound_address_and_rejects_modified_trampoline() {
        use super::*;
        let mut cpu = A64Cpu::new_sparse();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        let original = bridge
            .register_service(&mut cpu, "upgrade", |_| Ok(ReturnValues::integer(1)))
            .unwrap();
        let call = GuestCall {
            entry: original.guest_address(),
            ..Default::default()
        };
        assert_eq!(
            bridge
                .validate_registered_service(&cpu, "upgrade", original.guest_address())
                .unwrap()
                .guest_address(),
            original.guest_address()
        );
        assert!(bridge
            .validate_registered_service(&cpu, "_objc_release", original.guest_address())
            .is_err());
        assert!(bridge
            .validate_registered_service(&cpu, "upgrade", original.guest_address() + 16)
            .is_err());
        assert_eq!(bridge.call(&mut cpu, &call, 20).unwrap().integers[0], 1);
        let upgraded = bridge
            .replace_registered_service(&cpu, "upgrade", |_| Ok(ReturnValues::integer(42)))
            .unwrap();
        assert_eq!(upgraded.guest_address(), original.guest_address());
        assert!(bridge
            .validate_registered_service(&cpu, "upgrade", upgraded.guest_address())
            .is_ok());
        assert_eq!(bridge.call(&mut cpu, &call, 20).unwrap().integers[0], 42);
        assert!(bridge
            .replace_registered_service(&cpu, "absent", |_| Ok(ReturnValues::integer(0)))
            .is_err());
        cpu.write_bytes(original.guest_address(), &0xd503201fu32.to_le_bytes());
        assert!(bridge
            .validate_registered_service(&cpu, "upgrade", original.guest_address())
            .is_err());
        assert!(bridge
            .replace_registered_service(&cpu, "upgrade", |_| Ok(ReturnValues::integer(0)))
            .unwrap_err()
            .contains("changed"));
    }
    #[test]
    fn tail_dispatch_preserves_unknown_abi_after_ordered_initialization() {
        use super::*;
        use std::{cell::Cell, rc::Rc};
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 5).unwrap();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        for (i, instruction) in [
            0xaa0203e0u32,
            0xf9000100,
            0xf94003e3,
            0x8b030000,
            0xd65f03c0,
        ]
        .into_iter()
        .enumerate()
        {
            cpu.write_bytes(0x10000 + (i * 4) as u64, &instruction.to_le_bytes());
        }
        cpu.write_bytes(0x10100, &0xd65f03c0u32.to_le_bytes());
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        let order = Rc::new(Cell::new(0));
        let seen = order.clone();
        let service = bridge
            .register_service(&mut cpu, "tail", move |frame| {
                assert_eq!(frame.integer(2)?, 42);
                let start = seen.clone();
                let complete = seen.clone();
                frame.request_guest_call_with_start(
                    GuestCall {
                        entry: 0x10100,
                        ..Default::default()
                    },
                    move || {
                        assert_eq!(start.get(), 0);
                        start.set(1);
                        Ok(true)
                    },
                    move |result| {
                        result?;
                        assert_eq!(complete.get(), 1);
                        complete.set(2);
                        Ok(())
                    },
                )?;
                // This queued initializer was already completed reentrantly.
                frame.request_guest_call_with_start(
                    GuestCall {
                        entry: 0x10100,
                        ..Default::default()
                    },
                    || Ok(false),
                    |result| {
                        assert_eq!(result?.integers[0], 0);
                        Ok(())
                    },
                )?;
                frame.request_tail_dispatch(0x10000, 0x4567)?;
                Ok(ReturnValues::integer(999))
            })
            .unwrap();
        let result = bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: service.guest_address(),
                    integers: vec![9, 8, 42],
                    vectors: vec![[123, 456]],
                    stack_arguments: 7u64.to_le_bytes().to_vec(),
                    indirect_result: Some((0x40000, 8)),
                },
                1000,
            )
            .unwrap();
        assert_eq!(result.integers, [49, 0x4567]);
        assert_eq!(result.vectors[0], [123, 456]);
        assert_eq!(cpu.read_u64(0x40000), Some(42));
        assert_eq!(order.get(), 2);
    }
    use super::*;
    fn setup(words: &[u32]) -> (A64Cpu, GuestBridge) {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 5).unwrap();
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        cpu.write_bytes(0x10000, &bytes);
        cpu.set_pc(0x4444);
        cpu.set_sp(0x8888);
        cpu.set_reg(0, 99);
        cpu.set_vector(0, [123, 456]);
        let bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        (cpu, bridge)
    }
    #[test]
    fn metadata_budget_rejects_before_allocation_and_leaves_service_budget_unchanged() {
        let (mut cpu, mut bridge) = setup(&[0xd65f03c0]);
        let id = bridge
            .register_service(&mut cpu, "metadata_budget_fixture", |frame| {
                {
                    let mut metadata = frame.metadata_reader(8)?;
                    assert!(metadata
                        .read(u64::MAX, 16)
                        .unwrap_err()
                        .contains("before allocation"));
                    assert_eq!(metadata.statistics(), (0, 0));
                    assert_eq!(metadata.read(0x10000, 4)?.len(), 4);
                    assert_eq!(metadata.statistics(), (4, 1));
                    assert!(metadata.read(u64::MAX, MAX_METADATA_REQUEST + 1).is_err());
                }
                assert_eq!(frame.remaining_memory, MAX_SERVICE_MEMORY);
                assert_eq!(frame.read(0x10000, 4)?.len(), 4);
                assert_eq!(frame.remaining_memory, MAX_SERVICE_MEMORY - 4);
                Ok(ReturnValues::integer(42))
            })
            .unwrap();
        assert_eq!(
            bridge
                .call(
                    &mut cpu,
                    &GuestCall {
                        entry: id.guest_address(),
                        ..Default::default()
                    },
                    256
                )
                .unwrap()
                .integers[0],
            42
        );
    }
    #[test]
    fn metadata_reader_keeps_guest_read_permissions() {
        let (mut cpu, _) = setup(&[0xd65f03c0]);
        cpu.map_zeroed(0x50000, 4096, 2).unwrap();
        let mut reader = MetadataReader {
            cpu: &cpu,
            limit: 16,
            used: 0,
            requests: 0,
        };
        assert!(reader.read(0x50000, 4).is_err());
    }
    #[test]
    fn guest_integer_simd_results_and_context_restore() {
        // add x0,x0,x2; fadd d0,d0,d1; ret
        let (mut cpu, mut bridge) = setup(&[0x8b020000, 0x1e612800, 0xd65f03c0]);
        let call = GuestCall {
            entry: 0x10000,
            integers: vec![20, 7, 22],
            vectors: vec![[1.5f64.to_bits(), 0], [2.5f64.to_bits(), 0]],
            ..Default::default()
        };
        let result = bridge.call(&mut cpu, &call, 100).unwrap();
        assert_eq!(result.integers, [42, 7]);
        assert_eq!(f64::from_bits(result.vectors[0][0]), 4.0);
        assert_eq!(cpu.reg(0), 99);
        assert_eq!(cpu.vector(0), [123, 456]);
        assert_eq!(cpu.pc(), 0x4444);
        assert_eq!(cpu.sp(), 0x8888);
    }
    #[test]
    fn registered_service_runs_via_guest_trampoline_and_writes_real_memory() {
        // Save LR, BL first service stub at0x20010, restore LR, RET.
        let (mut cpu, mut bridge) =
            setup(&[0xa9bf7bfd, 0x910003fd, 0x94004002, 0xa8c17bfd, 0xd65f03c0]);
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        let id = bridge
            .register_service(&mut cpu, "test_add", |frame| {
                let value = frame.integer(0)? + frame.integer(2)?;
                frame.write(frame.integer(3)?, &value.to_le_bytes())?;
                assert_eq!(frame.read(frame.integer(3)?, 8)?, value.to_le_bytes());
                Ok(ReturnValues::integer(value))
            })
            .unwrap();
        assert_eq!(id.guest_address(), 0x20010);
        let result = bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: 0x10000,
                    integers: vec![20, 0, 22, 0x40000],
                    ..Default::default()
                },
                100,
            )
            .unwrap();
        assert_eq!(result.integers[0], 42);
        assert_eq!(cpu.read_u64(0x40000), Some(42));
        assert_eq!(cpu.pc(), 0x4444);
    }
    #[test]
    fn guest_stack_and_indirect_result_are_real_and_validated() {
        // ldr x0,[sp]; str x0,[x8]; ret
        let (mut cpu, mut bridge) = setup(&[0xf94003e0, 0xf9000100, 0xd65f03c0]);
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        let call = GuestCall {
            entry: 0x10000,
            stack_arguments: 42u64.to_le_bytes().to_vec(),
            indirect_result: Some((0x40000, 8)),
            ..Default::default()
        };
        assert_eq!(bridge.call(&mut cpu, &call, 100).unwrap().integers[0], 42);
        assert_eq!(cpu.read_u64(0x40000), Some(42));
        let service = bridge
            .register_service(&mut cpu, "ninth_scalar", |frame| {
                Ok(ReturnValues::integer(frame.stack_u64(0)?))
            })
            .unwrap();
        let mut call = call;
        call.entry = service.guest_address();
        assert_eq!(bridge.call(&mut cpu, &call, 100).unwrap().integers[0], 42);
        call.indirect_result = Some((0x10000, 8));
        assert!(bridge.call(&mut cpu, &call, 100).is_err());
        assert_eq!(cpu.pc(), 0x4444);
    }
    #[test]
    fn nested_guest_calls_use_disjoint_guarded_stack_and_report_validation_errors() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let (mut cpu, mut bridge) = setup(&[0xf94003e0, 0xd65f03c0]); // read stack argument; ret
        let result = Rc::new(RefCell::new(None));
        let output = result.clone();
        let entry = bridge
            .register_service(&mut cpu, "nested", move |frame| {
                assert_eq!(frame.stack_u64(0)?, 7);
                let output = output.clone();
                frame.request_guest_call(
                    GuestCall {
                        entry: 0x10000,
                        stack_arguments: 42u64.to_le_bytes().to_vec(),
                        ..Default::default()
                    },
                    move |value| {
                        *output.borrow_mut() = Some(value?.integers[0]);
                        Ok(())
                    },
                )?;
                Ok(ReturnValues::integer(99))
            })
            .unwrap();
        let value = bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: entry.guest_address(),
                    stack_arguments: 7u64.to_le_bytes().to_vec(),
                    ..Default::default()
                },
                100,
            )
            .unwrap();
        assert_eq!(value.integers[0], 99);
        assert_eq!(*result.borrow(), Some(42));
        assert_eq!(cpu.pc(), 0x4444);
        assert!(cpu
            .mapped_permissions(bridge.stack + STACK_SIZE as u64 - STACK_SLICE as u64)
            .is_none());
        let error = Rc::new(RefCell::new(None));
        let output = error.clone();
        let entry = bridge
            .register_service(&mut cpu, "bad_nested", move |frame| {
                let output = output.clone();
                frame.request_guest_call(
                    GuestCall {
                        entry: 0xdead0000,
                        ..Default::default()
                    },
                    move |value| {
                        *output.borrow_mut() = value.err();
                        Ok(())
                    },
                )?;
                Ok(ReturnValues::integer(0))
            })
            .unwrap();
        assert!(bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: entry.guest_address(),
                    ..Default::default()
                },
                100
            )
            .is_err());
        assert!(error.borrow().as_ref().unwrap().contains("executable"));
        assert_eq!(cpu.pc(), 0x4444);
    }
    #[test]
    fn scoped_callback_write_permissions_restore_after_return_and_guest_failure() {
        use std::{rc::Rc,cell::RefCell};
        for failure in [false,true] {
            let(mut cpu,mut bridge)=setup(&[0xf9000020,if failure{0xd4001001}else{0xd65f03c0}]);
            cpu.map_zeroed_with_max(0x40000,4096,1,3).unwrap();
            let observed=Rc::new(RefCell::new(None));let complete=observed.clone();
            let service=bridge.register_service(&mut cpu,"scoped_original_data_write",move|frame|{
                let complete=complete.clone();frame.request_guest_call_with_writable_ranges(GuestCall{entry:0x10000,integers:vec![0x55,0x40000],..Default::default()},vec![(0x40000,4096)],move|result|{*complete.borrow_mut()=Some(result.is_ok());Ok(())})?;Ok(ReturnValues::integer(0))
            }).unwrap();
            let outcome=bridge.call(&mut cpu,&GuestCall{entry:service.guest_address(),..Default::default()},100);
            assert_eq!(outcome.is_err(),failure);assert_eq!(*observed.borrow(),Some(!failure));assert_eq!(cpu.mapped_permissions(0x40000),Some(1));assert_eq!(cpu.read_u64(0x40000),Some(0x55));assert!(cpu.write_guest_into(0x40000,&[1]).is_err());
        }
    }
    #[test]
    fn runtime_callback_executes_real_large_stack_probe_with_distinct_guards(){
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();
        // Probe SP-8KiB, actually allocate/store in that frame, then restore SP.
        for(index,instruction)in[0xd1400be9u32,0xf9400120,0xd1400bff,0xf90003e1,0x91400bff,0xd65f03c0].into_iter().enumerate(){cpu.try_write_bytes(0x10000+index as u64*4,&instruction.to_le_bytes()).unwrap();}
        let mut bridge=GuestBridge::map_runtime(&mut cpu,0x20000).unwrap();
        assert_eq!(bridge.scratch_end().unwrap(),0x20000+RUNTIME_RESERVED_BYTES);
        for depth in 0..MAX_DEPTH{let top=bridge.scratch_end().unwrap()-(depth*bridge.stack_slice)as u64;assert_eq!(cpu.mapped_permissions(top-1),Some(3));assert_eq!(cpu.mapped_permissions(top-bridge.stack_window as u64-1),None);}
        let service=bridge.register_service(&mut cpu,"original_large_callback_stack",|frame|{frame.request_guest_call(GuestCall{entry:0x10000,integers:vec![0,0x55],..Default::default()},|result|{result?;Ok(())})?;Ok(ReturnValues::integer(0))}).unwrap();
        bridge.call(&mut cpu,&GuestCall{entry:service.guest_address(),..Default::default()},100).unwrap();
        let nested_top=bridge.scratch_end().unwrap()-bridge.stack_slice as u64;assert_eq!(cpu.read_u64(nested_top-8192),Some(0x55));assert_eq!(cpu.mapped_permissions(nested_top-bridge.stack_window as u64-1),None);
    }
    #[test]
    fn rejects_bad_targets_unknown_traps_budget_and_service_failures() {
        let (mut cpu, mut bridge) = setup(&[0x14000000]); // infinite branch
        assert!(bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: 0x10000,
                    ..Default::default()
                },
                20
            )
            .unwrap_err()
            .contains("budget"));
        assert_eq!(cpu.pc(), 0x4444);
        assert!(bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: 0x21000,
                    ..Default::default()
                },
                20
            )
            .unwrap_err()
            .contains("executable"));
        cpu.write_bytes(
            0x10000,
            &(0xd4000001u32 | ((SERVICE_SVC as u32) << 5)).to_le_bytes(),
        );
        assert!(bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: 0x10000,
                    ..Default::default()
                },
                20
            )
            .unwrap_err()
            .contains("token"));
        let id = bridge
            .register_service(&mut cpu, "failure", |_| {
                Err("unsupported real operation".into())
            })
            .unwrap();
        assert!(bridge
            .call(
                &mut cpu,
                &GuestCall {
                    entry: id.guest_address(),
                    ..Default::default()
                },
                20
            )
            .unwrap_err()
            .contains("unsupported real operation"));
        assert_eq!(cpu.reg(0), 99);
        assert_eq!(cpu.vector(0), [123, 456]);
    }
}

fn reject_supervisor(cpu: &mut A64Cpu, immediate: u16) -> Result<(), String> {
    Err(format!(
        "unregistered supervisor SVC {immediate:#x} at {:#x}",
        cpu.pc()
    ))
}

#[cfg(test)]
mod supervisor_tests {
    use super::*;
    #[test]
    fn explicit_mach_handler_restores_context_and_default_rejects() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 5).unwrap();
        for (i, word) in [0x92800370u32, 0xd4001001, 0xd65f03c0].iter().enumerate() {
            cpu.write_bytes(0x10000 + i as u64 * 4, &word.to_le_bytes());
        }
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        cpu.set_pc(0x4444);
        cpu.set_reg(0, 99);
        cpu.set_vector(0, [123, 456]);
        let call = GuestCall {
            entry: 0x10000,
            ..Default::default()
        };
        assert!(bridge
            .call(&mut cpu, &call, 100)
            .unwrap_err()
            .contains("unregistered supervisor"));
        let result = bridge
            .call_with_supervisor_handler(&mut cpu, &call, 100, &mut |cpu, svc| {
                assert_eq!(svc, 0x80);
                assert_eq!(cpu.reg(16) as i64, -28);
                cpu.set_reg(0, 0x100);
                Ok(())
            })
            .unwrap();
        assert_eq!(result.integers[0], 0x100);
        assert_eq!(cpu.pc(), 0x4444);
        assert_eq!(cpu.reg(0), 99);
        assert_eq!(cpu.vector(0), [123, 456]);
        assert!(bridge
            .call_with_supervisor_handler(&mut cpu, &call, 100, &mut |_, _| Err(
                "unsupported identity".into()
            ))
            .unwrap_err()
            .contains("unsupported identity"));
        assert_eq!(cpu.pc(), 0x4444);
    }
}

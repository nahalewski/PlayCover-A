/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bounded anonymous current-task mapping for __kernelrpc_mach_vm_map_trap.
//! LP64 trap ABI: target, address pointer, size, mask, flags, current protection.
//! Evidence: apple-xnu/osfmk/mach/mach_traps.h and vm_statistics.h; libplatform
//! alloc_once requests VM_FLAGS_ANYWHERE | VM_MEMORY_OS_ALLOC_ONCE (tag 73).
//! This is an explicit isolated arena, not a general Darwin VM implementation.

use super::{mach_identity::{MachIdentity, PortRight}, A64Cpu};

const PAGE: u64 = 0x4000;
const MAX_BUDGET: u64 = 256 * 1024 * 1024;
const INVALID_ADDRESS: u32 = 1;
const NO_SPACE: u32 = 3;
const INVALID_ARGUMENT: u32 = 4;
// The fast kernelrpc trap uses this value so libsyscall can select MIG fallback.
// ABI evidence: apple-xnu/osfmk/vm/vm_user.c / mach_kernelrpc.c trap wrapper.
const INVALID_TASK: u32 = 0x1000_0003; // MACH_SEND_INVALID_DEST

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnonymousMapping {
    pub address: u64,
    pub size: u64,
    pub protection: u32,
    pub tag: u8,
}

pub struct AnonymousVm {
    base: u64,
    next: u64,
    end: u64,
    mapped: u64,
    mappings: Vec<AnonymousMapping>,
    reusable: Vec<(u64,u64)>,
}

impl AnonymousVm {
    pub fn new(base: u64, end: u64) -> Result<Self, String> {
        if base == 0 || base % PAGE != 0 || end % PAGE != 0 || end <= base
            || end - base > MAX_BUDGET || end > (1 << 48)
        {
            return Err("Invalid bounded anonymous VM arena".into());
        }
        Ok(Self { base, next: base, end, mapped: 0, mappings: vec![], reusable:vec![] })
    }

    pub fn mappings(&self) -> &[AnonymousMapping] {
        &self.mappings
    }

    /// XNU4570.71.2 madvise7/8 -> VM_BEHAVIOR_REUSABLE/REUSE. These are
    /// permissions to reclaim, not mandatory immediate discard. This bounded
    /// virtual kernel conservatively retains backing/content while tracking
    /// actual owned reusable ranges; it never marks foreign mappings reusable.
    pub(super) fn advise(&mut self,cpu:&A64Cpu,address:u64,size:u64,advice:u64)->Result<u32,String>{
        if !matches!(advice,7|8){return Err(format!("unsupported owned anonymous madvise behavior {advice}"));}
        if address==0&&size!=0{return Ok(22);}
        let Some(limit)=address.checked_add(size) else{return Ok(22);};
        if size==0{return Ok(0);}
        let start=address&!(PAGE-1);
        let Some(end)=limit.checked_add(PAGE-1).map(|value|value&!(PAGE-1)) else{return Ok(22);};
        if start>=end||end>1u64<<48{return Ok(22);}
        let mut cursor=start;
        while cursor<end{
            let Some(mapping)=self.mappings.iter().find(|mapping|mapping.address<=cursor&&cursor<mapping.address+mapping.size) else{return Ok(22);};
            let Some(region)=cpu.protection_region(cursor) else{return Ok(22);};
            let Some(region_end)=region.base.checked_add(region.len) else{return Ok(22);};
            let next=end.min(mapping.address+mapping.size).min(region_end);
            if next<=cursor{return Ok(22);}cursor=next;
        }
        let mut ranges=Vec::new();ranges.try_reserve(self.reusable.len()+2).map_err(|_|"reusable VM metadata allocation failed")?;
        if advice==7{
            ranges.extend_from_slice(&self.reusable);ranges.push((start,end));ranges.sort_unstable();
            let mut merged:Vec<(u64,u64)>=Vec::new();merged.try_reserve(ranges.len()).map_err(|_|"reusable VM metadata allocation failed")?;
            for range in ranges{if let Some(last)=merged.last_mut(){if range.0<=last.1{last.1=last.1.max(range.1);continue;}}merged.push(range);}
            ranges=merged;
        }else{
            for &(left,right)in &self.reusable{
                if right<=start||left>=end{ranges.push((left,right));continue;}
                if left<start{ranges.push((left,start));}if right>end{ranges.push((end,right));}
            }
        }
        if ranges.len()>4096{return Ok(12);}
        self.reusable=ranges;Ok(0)
    }

    /// LP64 Mach -10 has four actual arguments (the old five-word macro counts
    /// the 64-bit size twice). Anonymous allocation defaults to current RW and
    /// maximum ALL; both syscall forms share the actual owned map and ledger.
    pub fn allocate(&mut self,cpu:&mut A64Cpu,ports:&MachIdentity,args:[u64;4])->Result<u32,String> {
        let [task,output,size,flags]=args;
        if size==0 {
            // vm_kern.c SIZE_ZERO_SUCCEEDS writes address zero without creating
            // an entry. Fast trap still validates its task and copyin/copyout.
            let Ok(task_name)=u32::try_from(task) else {return Ok(INVALID_TASK);};
            if !matches!(ports.right(task_name),Some(PortRight::TaskSend{references}) if references>0) {return Ok(INVALID_TASK);}
            if flags>u32::MAX as u64 || flags & 0x00ff_ffff != 1 {return Err(format!("Unsupported anonymous Mach VM flags {flags:#x}"));}
            if cpu.validate_guest_write(output,8).is_err() || cpu.read_guest_into(output,&mut[0;8]).is_err() {return Ok(INVALID_ADDRESS);}
            cpu.write_guest_into(output,&0u64.to_le_bytes())?;
            return Ok(0);
        }
        self.map(cpu,ports,[task,output,size,0,flags,3])
    }

    /// Actual Mach -12 LP64 ABI is task,address,size. XNU's zero-size sanitize
    /// path bypasses address-range validation and removes no mapping, even for
    /// a poisoned address. Nonempty removal requires a real backend unmap.
    pub fn deallocate(&mut self,cpu:&mut A64Cpu,ports:&MachIdentity,args:[u64;3])->Result<u32,String> {
        let [task,address,size]=args;
        let Ok(task_name)=u32::try_from(task) else {return Ok(INVALID_TASK);};
        if !matches!(ports.right(task_name),Some(PortRight::TaskSend{references}) if references>0) {return Ok(INVALID_TASK);}
        if size==0 {return Ok(0);}
        let Some(limit)=address.checked_add(size) else{return Ok(INVALID_ARGUMENT);};
        let start=address&!(PAGE-1);
        let Some(end)=limit.checked_add(PAGE-1).map(|v|v&!(PAGE-1)) else{return Ok(INVALID_ARGUMENT);};
        let mut cursor=start;
        while cursor<end {
            let Some(m)=self.mappings.iter().find(|m|m.address<=cursor&&cursor<m.address+m.size) else {
                return Err(format!("VM deallocation includes unsupported unowned/hole extent at {cursor:#x}"));
            };
            cursor=end.min(m.address+m.size);
        }
        let mut mappings=Vec::new();mappings.try_reserve(self.mappings.len()+1).map_err(|_|"VM deallocation ledger allocation failed")?;
        for &m in &self.mappings {
            let stop=m.address+m.size;
            if stop<=start||m.address>=end{mappings.push(m);continue;}
            if m.address<start{mappings.push(AnonymousMapping{size:start-m.address,..m});}
            if stop>end{mappings.push(AnonymousMapping{address:end,size:stop-end,..m});}
        }
        let mut reusable=Vec::new();reusable.try_reserve(self.reusable.len()+1).map_err(|_|"VM deallocation advice allocation failed")?;
        for &(left,right) in &self.reusable {
            if right<=start||left>=end{reusable.push((left,right));continue;}
            if left<start{reusable.push((left,start));}if right>end{reusable.push((end,right));}
        }
        cpu.unmap_anonymous(start,end-start)?;
        self.mappings=mappings;self.reusable=reusable;self.mapped-=end-start;Ok(0)
    }

    pub fn map(&mut self, cpu: &mut A64Cpu, ports: &MachIdentity, args: [u64; 6]) -> Result<u32, String> {
        let [task, output, size, mask, flags, protection] = args;
        let task_name: u32 = match task.try_into() {
            Ok(name) => name,
            Err(_) => return Ok(INVALID_TASK),
        };
        if !matches!(ports.right(task_name), Some(PortRight::TaskSend { references }) if references > 0) {
            return Ok(INVALID_TASK);
        }
        // Tag bits are accounting metadata. FIXED and ANYWHERE are bounded by
        // the same explicit task arena/budget; overwrite/purgable remain absent.
        let mode = flags & 0x00ff_ffff;
        if flags > u32::MAX as u64 || mode > 1 {
            return Err(format!("Unsupported anonymous Mach VM flags {flags:#x}"));
        }
        // XNU vm_map.c requires (start & mask)==0. This bounded allocator
        // implements contiguous low-bit alignment masks, including malloc's
        // actual 0xfffff (1MiB), without claiming arbitrary sparse masks.
        let alignment = match mask.checked_add(1) {
            Some(value) if value.is_power_of_two() => value.max(PAGE),
            None => return Ok(NO_SPACE),
            _ => return Err(format!("Unsupported noncontiguous anonymous Mach VM alignment mask {mask:#x}")),
        };
        if size == 0 || protection & !7 != 0 {
            return Ok(INVALID_ARGUMENT);
        }
        let rounded = match size.checked_add(PAGE - 1) {
            Some(value) => value & !(PAGE - 1),
            None => return Ok(INVALID_ARGUMENT),
        };
        if cpu.validate_guest_write(output, 8).is_err() {
            return Ok(INVALID_ADDRESS);
        }
        let mut hint = [0; 8];
        if cpu.read_guest_into(output, &mut hint).is_err() {
            return Ok(INVALID_ADDRESS);
        }
        let requested = u64::from_le_bytes(hint);
        let candidate = if mode == 0 {
            // vm_map_enter rounds fixed starts down to the task page and never
            // relocates a fixed request to satisfy its alignment mask.
            let start = requested & !(PAGE - 1);
            if start & mask != 0 { return Ok(NO_SPACE); }
            start
        } else {
            match self.next.max(requested).checked_add(alignment - 1) {
                Some(value) => value & !(alignment - 1),
                None => return Ok(NO_SPACE),
            }
        };
        let end = match candidate.checked_add(rounded) {
            Some(end) if candidate >= self.base && end <= self.end => end,
            _ => return Ok(NO_SPACE),
        };
        if self.mapped.checked_add(rounded).map_or(true, |total| total > MAX_BUDGET) {
            return Ok(NO_SPACE);
        }
        let length: usize = rounded.try_into().map_err(|_| "Anonymous mapping does not fit host size")?;
        self.mappings.try_reserve(1).map_err(|_|"Anonymous mapping ledger allocation failed")?;
        // CPU verifies the complete mapping interval for overlap before adding
        // zero-filled memory. No state/output publication occurs on failure.
        // Actual anonymous kernelrpc mapping fixes maximum to VM_PROT_ALL.
        cpu.map_zeroed_with_max(candidate, length, protection as u32, 7)?;
        // The output was validated before mapping and cannot change during this
        // exclusive &mut CPU operation. Mapping cannot overlap its existing page.
        cpu.write_guest_into(output, &candidate.to_le_bytes())?;
        self.next = self.next.max(end);
        self.mapped += rounded;
        self.mappings.push(AnonymousMapping { address: candidate, size: rounded,
            protection: protection as u32, tag: (flags >> 24) as u8 });
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (A64Cpu, MachIdentity, u64, AnonymousVm) {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, PAGE as usize, 3).unwrap();
        let mut ports = MachIdentity::new(8).unwrap();
        let task = ports.trap(-28).unwrap() as u64;
        let vm = AnonymousVm::new(0x20_0000_0000, 0x20_0100_0000).unwrap();
        (cpu, ports, task, vm)
    }
    #[test]
    fn quota_sized_arena_allows_allocations_beyond_old_window_without_raising_quota() {
        let (mut cpu, ports, task, _) = fixture();
        let mut vm = AnonymousVm::new(0x20_0000_0000, 0x20_1000_0000).unwrap();
        assert_eq!(vm.allocate(&mut cpu, &ports, [task, 0x10000, 20 * 1024 * 1024, 1]).unwrap(), 0);
        let first = cpu.read_u64(0x10000).unwrap();
        cpu.write_guest_into(first + 17 * 1024 * 1024, &[91]).unwrap();
        assert_eq!(vm.allocate(&mut cpu, &ports, [task, 0x10000, PAGE, 1]).unwrap(), 0);
        let second = cpu.read_u64(0x10000).unwrap();
        assert_eq!(second, first + 20 * 1024 * 1024);
        assert_eq!(cpu.read_bytes(first + 17 * 1024 * 1024, 1).unwrap(), &[91]);
        let mapped = vm.mapped;
        assert_eq!(vm.allocate(&mut cpu, &ports, [task, 0x10000, 257 * 1024 * 1024, 1]).unwrap(), NO_SPACE);
        assert_eq!(cpu.read_u64(0x10000), Some(second));
        assert_eq!(vm.mapped, mapped);
    }
    #[test]
    fn actual_unaligned_tiny_malloc_advice_tracks_reusable_page_without_discard(){
        let(mut cpu,ports,task,mut vm)=fixture();let base=0x2000100000u64;
        cpu.write_guest_into(0x10000,&base.to_le_bytes()).unwrap();
        assert_eq!(vm.allocate(&mut cpu,&ports,[task,0x10000,3*PAGE,1]).unwrap(),0);
        cpu.write_guest_into(base+0x182,&[31;12]).unwrap();
        let region=cpu.protection_region(base).unwrap();
        assert_eq!(vm.advise(&cpu,base+0x182,12,7).unwrap(),0);
        assert_eq!(vm.reusable,vec![(base,base+PAGE)]);
        assert_eq!(cpu.read_bytes(base+0x182,12).unwrap(),&[31;12]);
        let after=cpu.protection_region(base).unwrap();
        assert_eq!((after.base,after.len,after.protection,after.max_protection),
            (region.base,region.len,region.protection,region.max_protection));
        assert_eq!(vm.advise(&cpu,base,3*PAGE,7).unwrap(),0);
        assert_eq!(vm.reusable,vec![(base,base+3*PAGE)]);
        assert_eq!(vm.advise(&cpu,base+PAGE+3,12,8).unwrap(),0);
        assert_eq!(vm.reusable,vec![(base,base+PAGE),(base+2*PAGE,base+3*PAGE)]);
        assert_eq!(cpu.read_bytes(base+0x182,12).unwrap(),&[31;12]);
    }
    #[test]
    fn advice_rejects_holes_overflow_foreign_memory_and_preserves_prior_policy(){
        let(mut cpu,ports,task,mut vm)=fixture();vm.allocate(&mut cpu,&ports,[task,0x10000,PAGE,1]).unwrap();
        let base=vm.mappings()[0].address;assert_eq!(vm.advise(&cpu,base,PAGE,7).unwrap(),0);
        let before=vm.reusable.clone();
        for(address,size)in[(0,12),(u64::MAX-2,12),(base+PAGE-1,2),(0x10000,12)] {
            assert_eq!(vm.advise(&cpu,address,size,7).unwrap(),22);assert_eq!(vm.reusable,before);
        }
        assert!(vm.advise(&cpu,base,12,5).is_err());assert_eq!(vm.reusable,before);
        assert_eq!(vm.advise(&cpu,u64::MAX,0,7).unwrap(),0);assert_eq!(vm.reusable,before);
        assert_eq!(vm.mappings().len(),1);
    }
    #[test]
    fn actual_alloc_once_request_maps_zeroed_rw_memory_and_writes_address() {
        let (mut cpu, ports, task, mut vm) = fixture();
        assert_eq!(vm.map(&mut cpu, &ports, [task, 0x10000, 32768, 0, 0x49000001, 3]).unwrap(), 0);
        let mapping = vm.mappings()[0];
        assert_eq!(mapping.address, 0x20_0000_0000);
        assert_eq!(mapping.size, 32768);
        assert_eq!(mapping.tag, 73);
        let mut output = [0; 8];
        cpu.read_guest_into(0x10000, &mut output).unwrap();
        assert_eq!(u64::from_le_bytes(output), mapping.address);
        let mut memory = vec![1; 32768];
        cpu.read_guest_into(mapping.address, &mut memory).unwrap();
        assert!(memory.iter().all(|&byte| byte == 0));
        cpu.write_guest_into(mapping.address + 32767, &[9]).unwrap();
        assert!(cpu.write_guest_into(mapping.address + 32768, &[9]).is_err());
    }
    #[test]
    fn original_nano_fixed_reservation_exceeds_owned_arena_without_mutation() {
        let (mut cpu, ports, task, mut vm) = fixture();
        let requested = 0x1c0000000u64;
        cpu.write_guest_into(0x10000, &requested.to_le_bytes()).unwrap();
        // Original 15G77 malloc_init1809c0e4c..e68: tag11, FIXED,
        // 512MiB, RW, mask0. Its caller handles NO_SPACE by disabling nano.
        assert_eq!(vm.map(&mut cpu, &ports,
            [task, 0x10000, 0x20000000, 0, 0x0b000000, 3]).unwrap(), NO_SPACE);
        assert_eq!(cpu.read_u64(0x10000), Some(requested));
        assert!(vm.mappings().is_empty());
        assert!(cpu.protection_region(requested).is_none());
        assert_eq!(vm.mapped, 0);
        // Failed fixed request must not move the genuine ANYWHERE cursor.
        assert_eq!(vm.map(&mut cpu, &ports,
            [task, 0x10000, PAGE, 0, 1, 3]).unwrap(), 0);
        assert_eq!(cpu.read_u64(0x10000), Some(vm.base));
    }
    #[test]
    fn fixed_mapping_uses_requested_address_and_preserves_arena_bounds() {
        let (mut cpu, ports, task, mut vm) = fixture();
        let requested = vm.base + 2 * PAGE;
        cpu.write_guest_into(0x10000, &requested.to_le_bytes()).unwrap();
        assert_eq!(vm.map(&mut cpu, &ports,
            [task, 0x10000, PAGE, 0, 0x0b000000, 3]).unwrap(), 0);
        let mapping = vm.mappings()[0];
        assert_eq!(mapping.address, requested);
        assert_eq!(mapping.tag, 11);
        assert_eq!(cpu.read_u64(0x10000), Some(requested));
        let region = cpu.protection_region(requested).unwrap();
        assert_eq!((region.protection, region.max_protection), (3, 7));
        assert_eq!(cpu.read_u64(requested), Some(0));
        cpu.write_guest_into(requested, &27u64.to_le_bytes()).unwrap();
        // Never overwrite an existing fixed range, including its contents.
        assert!(vm.map(&mut cpu, &ports,
            [task, 0x10000, PAGE, 0, 0x0b000000, 3]).is_err());
        assert_eq!(cpu.read_u64(requested), Some(27));
        assert_eq!(vm.mappings(), &[mapping]);
        cpu.write_guest_into(0x10000, &(vm.end - PAGE).to_le_bytes()).unwrap();
        assert_eq!(vm.map(&mut cpu, &ports,
            [task, 0x10000, 2 * PAGE, 0, 0x0b000000, 3]).unwrap(), NO_SPACE);
        assert_eq!(cpu.read_u64(0x10000), Some(vm.end - PAGE));
        assert_eq!(vm.mappings(), &[mapping]);
    }
    #[test]
    fn invalid_task_output_flags_and_budget_do_not_allocate() {
        let (mut cpu, ports, task, mut vm) = fixture();
        assert_eq!(vm.map(&mut cpu, &ports, [0, 0x10000, 32768, 0, 1, 3]).unwrap(), INVALID_TASK);
        assert_eq!(INVALID_TASK, 0x1000_0003); // libsyscall's MIG fallback sentinel
        assert_eq!(vm.map(&mut cpu, &ports, [task, 0, 32768, 0, 1, 3]).unwrap(), INVALID_ADDRESS);
        assert!(vm.map(&mut cpu, &ports, [task, 0x10000, 32768, 0, 3, 3]).is_err());
        assert!(vm.map(&mut cpu, &ports, [task, 0x10000, 32768, 0xfffe, 1, 3]).is_err());
        assert_eq!(vm.map(&mut cpu, &ports, [task, 0x10000, 0x2000000, 0, 1, 3]).unwrap(), NO_SPACE);
        assert!(vm.mappings().is_empty());
        assert!(cpu.read_bytes(0x20_0000_0000, 1).is_none());
    }
    #[test]
    fn actual_mach_allocate_four_argument_request_maps_owned_rw_max_all() {
        let (mut cpu,ports,task,mut vm)=fixture();
        assert_eq!(vm.allocate(&mut cpu,&ports,[task,0x10000,0x4000,0x1000001]).unwrap(),0);
        let address=cpu.read_u64(0x10000).unwrap();let mapping=vm.mappings()[0];
        assert_eq!(address,mapping.address);assert_eq!(mapping.size,PAGE);assert_eq!(mapping.tag,1);assert_eq!(mapping.protection,3);
        let region=cpu.protection_region(address).unwrap();assert_eq!(region.protection,3);assert_eq!(region.max_protection,7);
        let mut zero=vec![1;PAGE as usize];cpu.read_guest_into(address,&mut zero).unwrap();assert!(zero.iter().all(|&b|b==0));
        cpu.write_guest_into(address+PAGE-1,&[7]).unwrap();cpu.set_protection(address,PAGE,0).unwrap();assert!(cpu.write_guest_into(address,&[1]).is_err());
        cpu.set_protection(address,PAGE,3).unwrap();let mut last=[0];cpu.read_guest_into(address+PAGE-1,&mut last).unwrap();assert_eq!(last,[7]);
    }
    #[test]
    fn allocate_failures_preserve_output_and_shared_mapping_ledger() {
        let (mut cpu,ports,task,mut vm)=fixture();let old=0x1234u64;cpu.write_guest_into(0x10000,&old.to_le_bytes()).unwrap();
        assert_eq!(vm.allocate(&mut cpu,&ports,[0,0x10000,PAGE,1]).unwrap(),INVALID_TASK);
        assert_eq!(cpu.read_u64(0x10000),Some(old));assert!(vm.mappings().is_empty());
        cpu.map_zeroed(0x20000,PAGE as usize,1).unwrap();assert_eq!(vm.allocate(&mut cpu,&ports,[task,0x20000,PAGE,1]).unwrap(),INVALID_ADDRESS);
        assert!(vm.allocate(&mut cpu,&ports,[task,0x10000,PAGE,3]).is_err());assert!(vm.mappings().is_empty());
        assert_eq!(cpu.read_u64(0x10000),Some(old));
        assert_eq!(vm.allocate(&mut cpu,&ports,[task,0x10000,0,1]).unwrap(),0);assert_eq!(cpu.read_u64(0x10000),Some(0));assert!(vm.mappings().is_empty());
        cpu.write_guest_into(0x10000,&0u64.to_le_bytes()).unwrap();
        vm.allocate(&mut cpu,&ports,[task,0x10000,PAGE,1]).unwrap();let first=cpu.read_u64(0x10000).unwrap();
        vm.map(&mut cpu,&ports,[task,0x10000,PAGE,0,1,3]).unwrap();assert_eq!(vm.mappings().len(),2);assert_eq!(cpu.read_u64(0x10000),Some(first+PAGE));
    }
    #[test]
    fn zero_deallocate_accepts_poisoned_address_without_removing_real_owned_memory() {
        let (mut cpu,ports,task,mut vm)=fixture();
        vm.allocate(&mut cpu,&ports,[task,0x10000,PAGE,1]).unwrap();let mapping=vm.mappings()[0];
        cpu.write_guest_into(mapping.address,&[19]).unwrap();
        assert_eq!(vm.deallocate(&mut cpu,&ports,[task,0xdeaddeaddeaddead,0]).unwrap(),0);
        assert_eq!(vm.deallocate(&mut cpu,&ports,[task,mapping.address,0]).unwrap(),0);
        assert_eq!(vm.mappings(),&[mapping]);let mut byte=[0];cpu.read_guest_into(mapping.address,&mut byte).unwrap();assert_eq!(byte,[19]);
        assert_eq!(vm.deallocate(&mut cpu,&ports,[0,u64::MAX,0]).unwrap(),INVALID_TASK);
        assert_eq!(vm.deallocate(&mut cpu,&ports,[task,mapping.address,PAGE]).unwrap(),0);
        assert!(vm.mappings().is_empty());assert!(cpu.read_guest_into(mapping.address,&mut byte).is_err());
    }
    #[test]
    fn partial_deallocation_clips_owned_advice_and_faults_real_cpu_store() {
        use super::super::A64State;
        let (mut cpu,ports,task,mut vm)=fixture();
        assert_eq!(vm.allocate(&mut cpu,&ports,[task,0x10000,PAGE*3,1]).unwrap(),0);
        let base=vm.mappings()[0].address;
        cpu.write_guest_into(base,&[11]).unwrap();cpu.write_guest_into(base+PAGE*2,&[22]).unwrap();
        assert_eq!(vm.advise(&cpu,base,PAGE*3,7).unwrap(),0);
        let backing=cpu.anonymous_backing_bytes();let mapped=cpu.mapped_bytes();
        assert_eq!(vm.deallocate(&mut cpu,&ports,[task,base+PAGE+3,7]).unwrap(),0);
        assert_eq!(cpu.anonymous_backing_bytes(),backing);assert_eq!(cpu.mapped_bytes(),mapped-PAGE as usize);
        assert_eq!(vm.reusable,vec![(base,base+PAGE),(base+PAGE*2,base+PAGE*3)]);
        assert_eq!(cpu.read_bytes(base,1),Some(&[11][..]));assert_eq!(cpu.read_bytes(base+PAGE*2,1),Some(&[22][..]));
        assert!(cpu.read_bytes(base+PAGE,1).is_none());assert!(cpu.try_write_bytes(base+PAGE,&[1]).is_err());
        assert!(cpu.mutate_region(base+PAGE,1,|_|Ok(())).is_err());
        cpu.map_zeroed(0x18000,4096,5).unwrap();
        cpu.write_bytes(0x18000, &[0xf9000020u32, 0xd4000001u32].iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<_>>().as_slice());cpu.set_reg(0,9);cpu.set_reg(1,base+PAGE);cpu.set_pc(0x18000);
        let mut ticks=100;assert_eq!(cpu.run_or_step(Some(&mut ticks)),A64State::MemoryError(base+PAGE));
        assert!(vm.deallocate(&mut cpu,&ports,[task,base,PAGE*3]).is_err());
        assert_eq!(cpu.read_bytes(base,1),Some(&[11][..]));
        vm.deallocate(&mut cpu,&ports,[task,base,PAGE]).unwrap();vm.deallocate(&mut cpu,&ports,[task,base+PAGE*2,PAGE]).unwrap();
        assert!(vm.mappings().is_empty());assert!(vm.reusable.is_empty());
        assert_eq!(cpu.anonymous_backing_bytes(),backing-PAGE as usize*3+4096);
        cpu.map_zeroed(base,PAGE as usize*3,3).unwrap();assert_eq!(cpu.read_bytes(base,1),Some(&[0][..]));
    }
    #[test]
    fn unmap_invalidates_translated_code_and_full_backing_is_released() {
        use super::super::A64State;
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();
        cpu.write_bytes(0x10000,&0xd4001001u32.to_le_bytes());cpu.set_pc(0x10000);let mut ticks=100;
        assert_eq!(cpu.run_or_step(Some(&mut ticks)),A64State::Svc(0x80));
        assert!(cpu.unmap_anonymous(0x10000,8192).is_err());assert_eq!(cpu.mapped_bytes(),4096);
        cpu.unmap_anonymous(0x10000,4096).unwrap();assert_eq!(cpu.anonymous_backing_bytes(),0);
        cpu.set_pc(0x10000);let mut ticks=100;assert_eq!(cpu.run_or_step(Some(&mut ticks)),A64State::MemoryError(0x10000));
        cpu.map_zeroed(0x10000,4096,5).unwrap();cpu.write_bytes(0x10000,&0xd4001021u32.to_le_bytes());
        cpu.set_pc(0x10000);let mut ticks=100;assert_eq!(cpu.run_or_step(Some(&mut ticks)),A64State::Svc(0x81));
    }
    #[test]
    fn rounds_size_honors_hint_and_readonly_protection() {
        let (mut cpu, ports, task, mut vm) = fixture();
        cpu.write_guest_into(0x10000, &0x20_0000_4001u64.to_le_bytes()).unwrap();
        assert_eq!(vm.map(&mut cpu, &ports, [task, 0x10000, 1, 0, 1, 1]).unwrap(), 0);
        assert_eq!(vm.mappings()[0].address, 0x20_0000_8000);
        assert_eq!(vm.mappings()[0].size, PAGE);
        assert!(cpu.write_guest_into(0x20_0000_8000, &[1]).is_err());
    }
    #[test]
    fn overlap_and_overflow_do_not_publish_mapping() {
        let (mut cpu, ports, task, mut vm) = fixture();
        cpu.map_zeroed(0x20_0000_0000, PAGE as usize, 3).unwrap();
        assert!(vm.map(&mut cpu, &ports, [task, 0x10000, PAGE, 0, 1, 3]).is_err());
        assert!(vm.mappings().is_empty());
        assert_eq!(vm.map(&mut cpu, &ports, [task, 0x10000, u64::MAX, 0, 1, 3]).unwrap(), INVALID_ARGUMENT);
        assert_eq!(vm.map(&mut cpu, &ports, [task, 0x10000, PAGE, 0, 1, 8]).unwrap(), INVALID_ARGUMENT);
    }
    #[test]
    fn malloc_megabyte_alignment_maps_only_real_zeroed_requested_pages() {
        let (mut cpu,ports,task,mut vm)=fixture();
        vm.allocate(&mut cpu,&ports,[task,0x10000,PAGE,1]).unwrap();
        cpu.write_guest_into(0x10000,&0u64.to_le_bytes()).unwrap();
        assert_eq!(vm.map(&mut cpu,&ports,[task,0x10000,PAGE,0xfffff,0x1000001,3]).unwrap(),0);
        let address=cpu.read_u64(0x10000).unwrap();assert_eq!(address,0x20_0010_0000);
        assert_eq!(address&0xfffff,0);assert_eq!(vm.mappings()[1].size,PAGE);
        assert!(cpu.read_bytes(address-PAGE,1).is_none());
        let mut bytes=vec![1;PAGE as usize];cpu.read_guest_into(address,&mut bytes).unwrap();assert!(bytes.iter().all(|&b|b==0));
        assert_eq!(cpu.protection_region(address).unwrap().max_protection,7);
        cpu.write_guest_into(address+PAGE-1,&[23]).unwrap();
    }
    #[test]
    fn alignment_hint_exhaustion_overflow_and_irregular_masks_preserve_state() {
        let (mut cpu,ports,task,mut vm)=fixture();
        let hint=0x20_0001_0001u64;cpu.write_guest_into(0x10000,&hint.to_le_bytes()).unwrap();
        assert_eq!(vm.map(&mut cpu,&ports,[task,0x10000,PAGE,0xffff,1,3]).unwrap(),0);
        assert_eq!(cpu.read_u64(0x10000),Some(0x20_0002_0000));
        let ledger=vm.mappings().to_vec();
        for (bad_hint,mask) in [(u64::MAX,0xfffff),(0x20_00ff_ffff,0xfffff),(0,u64::MAX),(0,1u64<<63)] {
            cpu.write_guest_into(0x10000,&bad_hint.to_le_bytes()).unwrap();
            let result=vm.map(&mut cpu,&ports,[task,0x10000,PAGE,mask,1,3]);
            assert!(result.is_err()||result.unwrap()==NO_SPACE);
            assert_eq!(cpu.read_u64(0x10000),Some(bad_hint));assert_eq!(vm.mappings(),ledger.as_slice());
        }
    }
}

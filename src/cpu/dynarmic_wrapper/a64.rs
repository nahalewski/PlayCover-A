/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! EXPERIMENTAL: bindings for the AArch64 (A64) dynarmic wrapper in `a64.cpp`.
//!
//! This is a research prototype towards 64-bit guest support. It is entirely
//! separate from the 32-bit CPU used by touchHLE proper: it does not use
//! touchHLE's `Mem`, but sparse owned regions at 64-bit guest addresses, with
//! every guest access going through bounds and permission checked callbacks.

use std::ffi::c_void;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

#[cfg(unix)]
struct FileMapping {
    raw: NonNull<u8>,
    mapping_size: usize,
    view_offset: usize,
    view_size: usize,
}

#[cfg(unix)]
impl Drop for FileMapping {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.raw.as_ptr().cast(), self.mapping_size);
        }
    }
}

enum Backing {
    Anonymous(Box<[u8]>),
    #[cfg(unix)]
    File(FileMapping),
}

impl Deref for Backing {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Anonymous(bytes) => bytes,
            #[cfg(unix)]
            Self::File(mapping) => unsafe {
                std::slice::from_raw_parts(
                    mapping.raw.as_ptr().add(mapping.view_offset),
                    mapping.view_size,
                )
            },
        }
    }
}

impl DerefMut for Backing {
    fn deref_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Anonymous(bytes) => bytes,
            #[cfg(unix)]
            Self::File(mapping) => unsafe {
                std::slice::from_raw_parts_mut(
                    mapping.raw.as_ptr().add(mapping.view_offset),
                    mapping.view_size,
                )
            },
        }
    }
}

#[allow(non_camel_case_types)]
pub type touchHLE_A64Wrapper = c_void;

#[allow(non_camel_case_types)]
pub type touchHLE_A64Context = c_void;

extern "C" {
    pub fn touchHLE_A64Wrapper_new(buf: *mut u8, len: usize, base: u64)
        -> *mut touchHLE_A64Wrapper;
    pub fn touchHLE_A64Wrapper_delete(cpu: *mut touchHLE_A64Wrapper);
    pub fn touchHLE_A64Wrapper_counter_ticks(cpu:*mut touchHLE_A64Wrapper)->u64;
    pub fn touchHLE_A64Wrapper_counter_frequency(cpu:*const touchHLE_A64Wrapper)->u32;
    pub fn touchHLE_A64Wrapper_map(
        cpu: *mut touchHLE_A64Wrapper,
        buf: *mut u8,
        size: usize,
        base: u64,
        permissions: u32,
    ) -> bool;
    pub fn touchHLE_A64Wrapper_map_with_max(cpu:*mut touchHLE_A64Wrapper,buf:*mut u8,size:usize,base:u64,permissions:u32,max_permissions:u32)->bool;
    pub fn touchHLE_A64Wrapper_unmap(cpu:*mut touchHLE_A64Wrapper,base:u64,size:usize)->bool;
    pub fn touchHLE_A64Wrapper_protect(cpu:*mut touchHLE_A64Wrapper,base:u64,size:usize,permissions:u32)->bool;
    pub fn touchHLE_A64Context_new() -> *mut touchHLE_A64Context;
    pub fn touchHLE_A64Context_delete(context: *mut touchHLE_A64Context);
    pub fn touchHLE_A64Wrapper_save_context(
        cpu: *const touchHLE_A64Wrapper,
        context: *mut touchHLE_A64Context,
    );
    pub fn touchHLE_A64Wrapper_restore_context(
        cpu: *mut touchHLE_A64Wrapper,
        context: *const touchHLE_A64Context,
    );
    pub fn touchHLE_A64Wrapper_get_reg(cpu: *const touchHLE_A64Wrapper, idx: usize) -> u64;
    pub fn touchHLE_A64Wrapper_set_reg(cpu: *mut touchHLE_A64Wrapper, idx: usize, v: u64);
    pub fn touchHLE_A64Wrapper_get_vector(cpu: *const touchHLE_A64Wrapper, idx: usize, lanes: *mut u64);
    pub fn touchHLE_A64Wrapper_set_vector(cpu: *mut touchHLE_A64Wrapper, idx: usize, lanes: *const u64);
    pub fn touchHLE_A64Wrapper_get_pc(cpu: *const touchHLE_A64Wrapper) -> u64;
    pub fn touchHLE_A64Wrapper_set_pc(cpu: *mut touchHLE_A64Wrapper, v: u64);
    pub fn touchHLE_A64Wrapper_get_sp(cpu: *const touchHLE_A64Wrapper) -> u64;
    pub fn touchHLE_A64Wrapper_set_sp(cpu: *mut touchHLE_A64Wrapper, v: u64);
    pub fn touchHLE_A64Wrapper_get_pstate(cpu: *const touchHLE_A64Wrapper) -> u32;
    pub fn touchHLE_A64Wrapper_set_pstate(cpu: *mut touchHLE_A64Wrapper, value: u32);
    pub fn touchHLE_A64Wrapper_set_tpidrro_el0(cpu: *mut touchHLE_A64Wrapper, v: u64);
    pub fn touchHLE_A64Wrapper_get_tpidrro_el0(cpu: *const touchHLE_A64Wrapper) -> u64;
    pub fn touchHLE_A64Wrapper_get_tpidr_el0(cpu: *const touchHLE_A64Wrapper) -> u64;
    pub fn touchHLE_A64Wrapper_set_tpidr_el0(cpu: *mut touchHLE_A64Wrapper, v: u64);
    pub fn touchHLE_A64Wrapper_mem_error_addr(cpu: *const touchHLE_A64Wrapper) -> u64;
    pub fn touchHLE_A64Wrapper_invalidate_cache_range(
        cpu: *mut touchHLE_A64Wrapper,
        start: u64,
        size: usize,
    );
    pub fn touchHLE_A64Wrapper_run_or_step(
        cpu: *mut touchHLE_A64Wrapper,
        ticks: Option<&mut u64>,
    ) -> i32;
}

/// Why A64 execution stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum A64State {
    /// Ran out of ticks, or a single step completed.
    Normal,
    /// `svc #imm` was executed. PC already points after the SVC.
    Svc(u16),
    /// Access to an unmapped or protected guest address.
    MemoryError(u64),
    UndefinedInstruction,
    /// `brk` instruction.
    Breakpoint,
}

/// Owned snapshot of scalar, SIMD, floating-point control/status and thread
/// registers. Guest memory is deliberately not included. Restoring clears the
/// exclusive reservation, which must not survive an initializer call boundary.
pub struct A64Context {
    raw: NonNull<touchHLE_A64Context>,
}

impl Drop for A64Context {
    fn drop(&mut self) {
        unsafe { touchHLE_A64Context_delete(self.raw.as_ptr()) }
    }
}

struct Region {
    base: u64,
    // The box allocation remains stable even when its descriptor moves.
    bytes: Backing,
}

/// Permission extents are separate from backing allocations, so clipping a
/// range never moves or duplicates an owned allocation or file mapping.
#[derive(Clone,Copy,Debug)]
pub struct ProtectionRegion {pub base:u64,pub len:u64,pub protection:u32,pub max_protection:u32}

/// An AArch64 CPU plus sparse guest regions. Address gaps allocate no memory.
pub struct A64Cpu {
    raw: *mut touchHLE_A64Wrapper,
    regions: Vec<Region>,
    protections: Vec<ProtectionRegion>,
    mapped_bytes: usize,
    anonymous_bytes: usize,
    file_mapping_bytes: u64,
}

impl Drop for A64Cpu {
    fn drop(&mut self) {
        unsafe {
            touchHLE_A64Wrapper_delete(self.raw);
        }
    }
}

impl A64Cpu {
    pub const LR: usize = 30;
    pub const FP: usize = 29;
    pub const READ: u32 = 1;
    pub const WRITE: u32 = 2;
    pub const EXECUTE: u32 = 4;
    pub const MAX_MAPPED_BYTES: usize = 512 * 1024 * 1024;

    /// Actual steady virtual-kernel counter used by supported CNTPCT_EL0 reads.
    /// Its epoch is not saved/restored with a guest thread context.
    pub fn counter_ticks(&self)->u64 {unsafe {touchHLE_A64Wrapper_counter_ticks(self.raw)}}
    pub fn counter_frequency(&self)->u32 {unsafe {touchHLE_A64Wrapper_counter_frequency(self.raw)}}

    /// Legacy constructor: one readable, writable and executable region.
    pub fn new(base: u64, size: usize) -> A64Cpu {
        let mut cpu = Self::new_sparse();
        cpu.map_zeroed(base, size, Self::READ | Self::WRITE | Self::EXECUTE)
            .expect("A64 legacy memory mapping failed");
        cpu
    }

    /// Construct a CPU with no mapped memory.
    pub fn new_sparse() -> A64Cpu {
        let raw = unsafe { touchHLE_A64Wrapper_new(std::ptr::null_mut(), 0, 0) };
        assert!(!raw.is_null(), "A64 CPU allocation failed");
        A64Cpu {
            raw,
            regions: Vec::new(),
            protections: Vec::new(),
            mapped_bytes: 0,
            anonymous_bytes: 0,
            file_mapping_bytes: 0,
        }
    }

    /// Map an exact byte range; alignment is not required. Permission bits use
    /// Mach VM_PROT_READ/WRITE/EXECUTE (1/2/4). Zero permissions are allowed.
    /// Actual allocated bytes are capped at 512 MiB, independent of address span.
    pub fn map_zeroed(&mut self, base: u64, size: usize, permissions: u32) -> Result<(), String> {
        self.map_zeroed_with_max(base,size,permissions,permissions)
    }
    pub fn map_zeroed_with_max(&mut self,base:u64,size:usize,permissions:u32,max_permissions:u32)->Result<(),String> {
        if size == 0 || permissions & !7 != 0 || max_permissions & !7 !=0 || permissions & max_permissions != permissions {
            return Err("invalid A64 memory mapping size or permissions".into());
        }
        let end = base
            .checked_add(u64::try_from(size).map_err(|_| "A64 mapping size overflow")?)
            .ok_or("A64 mapping address overflow")?;
        let total = self
            .anonymous_bytes
            .checked_add(size)
            .ok_or("A64 mapped byte count overflow")?;
        if total > Self::MAX_MAPPED_BYTES {
            return Err("A64 mapped allocation exceeds 512 MiB".into());
        }
        let total_mapped = self
            .mapped_bytes
            .checked_add(size)
            .ok_or("A64 mapped total overflow")?;
        let position = self.regions.partition_point(|region| region.base < base);
        if self
            .regions
            .get(position)
            .is_some_and(|region| region.base < end)
            || position > 0
                && self.regions[position - 1].base + self.regions[position - 1].bytes.len() as u64
                    > base
        {
            return Err("overlapping A64 memory mapping".into());
        }
        self.regions
            .try_reserve(1)
            .map_err(|_| "A64 region table allocation failed")?;
        self.protections.try_reserve(1).map_err(|_|"A64 protection table allocation failed")?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| "A64 memory allocation failed")?;
        bytes.resize(size, 0);
        let mut bytes = bytes.into_boxed_slice();
        if !unsafe {
            touchHLE_A64Wrapper_map_with_max(self.raw, bytes.as_mut_ptr(), size, base, permissions,max_permissions)
        } {
            return Err("A64 native region table rejected mapping".into());
        }
        self.regions.insert(
            position,
            Region {
                base,
                bytes: Backing::Anonymous(bytes),
            },
        );
        self.anonymous_bytes = total;
        let at=self.protections.partition_point(|region|region.base<base);
        self.protections.insert(at,ProtectionRegion{base,len:size as u64,protection:permissions,max_protection:max_permissions});
        self.mapped_bytes = total_mapped;
        Ok(())
    }

    /// Map a bounded file range privately. Pages are demand-loaded; host changes
    /// use copy-on-write and do not alter the original file. Guest protections
    /// remain enforced by the CPU callbacks even though the host view is RW.
    ///
    /// # Safety
    /// The caller must ensure the source file is not truncated or modified by
    /// any process for the lifetime of this CPU. Private/COW host mutations of
    /// the mapped view are allowed and do not modify the source file.
    #[cfg(unix)]
    pub unsafe fn map_file(
        &mut self,
        base: u64,
        file: &std::fs::File,
        offset: u64,
        size: usize,
        permissions: u32,
    ) -> Result<(), String> {
        use std::os::fd::AsRawFd;
        if size == 0 || permissions & !7 != 0 {
            return Err("invalid A64 file mapping size or permissions".into());
        }
        let size_u64 = u64::try_from(size).map_err(|_| "A64 file mapping size overflow")?;
        let end = base
            .checked_add(size_u64)
            .ok_or("A64 file mapping address overflow")?;
        let file_end = offset
            .checked_add(size_u64)
            .ok_or("A64 file mapping offset overflow")?;
        if file_end
            > file
                .metadata()
                .map_err(|e| format!("A64 file metadata: {e}"))?
                .len()
        {
            return Err("A64 file mapping exceeds file length".into());
        }
        let position = self.regions.partition_point(|region| region.base < base);
        if self
            .regions
            .get(position)
            .is_some_and(|region| region.base < end)
            || position > 0
                && self.regions[position - 1].base + self.regions[position - 1].bytes.len() as u64
                    > base
        {
            return Err("overlapping A64 file mapping".into());
        }
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let page_size = u64::try_from(page_size).map_err(|_| "invalid host page size")?;
        if page_size == 0 {
            return Err("zero host page size".into());
        }
        let aligned_offset = offset - offset % page_size;
        let view_offset = usize::try_from(offset - aligned_offset)
            .map_err(|_| "A64 file mapping prefix overflow")?;
        let mapping_size = size
            .checked_add(view_offset)
            .ok_or("A64 file mapping length overflow")?;
        let rounded_size = (mapping_size as u64)
            .checked_add(page_size - 1)
            .ok_or("A64 mmap rounding overflow")?
            / page_size
            * page_size;
        let total_file = self
            .file_mapping_bytes
            .checked_add(rounded_size)
            .ok_or("A64 file map total overflow")?;
        if total_file > 4 * 1024 * 1024 * 1024u64 {
            return Err("A64 file mappings exceed 4 GiB".into());
        }
        let total = self
            .mapped_bytes
            .checked_add(size)
            .ok_or("A64 mapped total overflow")?;
        let aligned_offset = libc::off_t::try_from(aligned_offset)
            .map_err(|_| "A64 file offset exceeds host range")?;
        self.regions
            .try_reserve(1)
            .map_err(|_| "A64 region table allocation failed")?;
        self.protections.try_reserve(1).map_err(|_|"A64 protection table allocation failed")?;
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                mapping_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE,
                file.as_raw_fd(),
                aligned_offset,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(format!(
                "A64 file mmap: {}",
                std::io::Error::last_os_error()
            ));
        }
        let Some(raw) = NonNull::new(raw.cast::<u8>()) else {
            unsafe {
                libc::munmap(raw, mapping_size);
            }
            return Err("A64 file mmap returned null".into());
        };
        let mut bytes = Backing::File(FileMapping {
            raw,
            mapping_size,
            view_offset,
            view_size: size,
        });
        if !unsafe {
            touchHLE_A64Wrapper_map(self.raw, bytes.as_mut_ptr(), size, base, permissions)
        } {
            return Err("A64 native region table rejected file mapping".into());
        }
        self.regions.insert(
            position,
            Region {
                base,
                bytes,
            },
        );
        self.mapped_bytes = total;
        let at=self.protections.partition_point(|region|region.base<base);
        self.protections.insert(at,ProtectionRegion{base,len:size as u64,protection:permissions,max_protection:permissions});
        self.file_mapping_bytes = total_file;
        Ok(())
    }

    #[cfg(not(unix))]
    pub unsafe fn map_file(
        &mut self,
        _base: u64,
        _file: &std::fs::File,
        _offset: u64,
        _size: usize,
        _permissions: u32,
    ) -> Result<(), String> {
        Err("A64 file-backed mappings require a Unix host".into())
    }

    /// Initialize one mapped backing in place; no guest execution may occur
    /// while the closure borrows its bytes. Partial writes on Err are retained,
    /// and the affected JIT range is invalidated on both success and failure.
    pub fn mutate_region(
        &mut self,
        base: u64,
        len: usize,
        f: impl FnOnce(&mut [u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        struct Invalidate {
            raw: *mut touchHLE_A64Wrapper,
            base: u64,
            len: usize,
        }
        impl Drop for Invalidate {
            fn drop(&mut self) {
                if self.len != 0 {
                    unsafe {
                        touchHLE_A64Wrapper_invalidate_cache_range(self.raw, self.base, self.len);
                    }
                }
            }
        }
        self.validate_host_range(base,len)?;
        let index = self
            .region_at(base)
            .ok_or("A64 mutation starts outside mapped regions")?;
        let region = &mut self.regions[index];
        let offset =
            usize::try_from(base - region.base).map_err(|_| "A64 mutation offset overflow")?;
        let end = offset
            .checked_add(len)
            .ok_or("A64 mutation length overflow")?;
        let bytes = region
            .bytes
            .get_mut(offset..end)
            .ok_or("A64 mutation spans beyond one backing")?;
        let _invalidate = Invalidate {
            raw: self.raw,
            base,
            len,
        };
        f(bytes)
    }

    /// Lowest mapped guest address, or zero when no regions exist.
    pub fn base(&self) -> u64 {
        self.regions.first().map_or(0, |region| region.base)
    }
    /// Total mapped bytes, not the span between the lowest and highest address.
    pub fn size(&self) -> usize {
        self.mapped_bytes
    }
    pub fn mapped_bytes(&self) -> usize {
        self.mapped_bytes
    }

    pub fn mapped_permissions(&self, addr: u64) -> Option<u32> {
        self.protection_region(addr).map(|region|region.protection)
    }
    pub fn protection_region(&self,addr:u64)->Option<ProtectionRegion> {
        let index=self.protections.partition_point(|region|region.base<=addr).checked_sub(1)?;
        let region=self.protections[index];(addr-region.base<region.len).then_some(region)
    }
    /// Atomically clip current protections while preserving bytes and maximums.
    /// The C++ execution callbacks and Rust copyin/copyout use the same extents.
    pub fn set_protection(&mut self,base:u64,len:u64,protection:u32)->Result<(),String> {
        let end=base.checked_add(len).ok_or("A64 protection address overflow")?;
        if len==0 || protection & !7 !=0 {return Err("invalid A64 protection change".into());}
        let size=usize::try_from(len).map_err(|_|"A64 protection size overflow")?;
        let mut cursor=base;
        while cursor<end {
            let region=self.protection_region(cursor).ok_or("unmapped A64 protection range")?;
            if protection & region.max_protection != protection {return Err("A64 protection exceeds mapping maximum".into());}
            cursor=end.min(region.base+region.len);
        }
        if self.protections.len()>65534 {return Err("A64 protection extent budget exceeded".into());}
        let mut changed=Vec::new();changed.try_reserve_exact(self.protections.len()+2).map_err(|_|"A64 protection allocation failed")?;
        for &region in &self.protections {
            let stop=region.base+region.len;
            if stop<=base || region.base>=end {changed.push(region);continue;}
            let start=region.base.max(base);let finish=stop.min(end);
            if region.base<start {changed.push(ProtectionRegion{len:start-region.base,..region});}
            changed.push(ProtectionRegion{base:start,len:finish-start,protection,..region});
            if finish<stop {changed.push(ProtectionRegion{base:finish,len:stop-finish,..region});}
        }
        if !unsafe {touchHLE_A64Wrapper_protect(self.raw,base,size,protection)} {return Err("A64 native protection transaction rejected".into());}
        self.protections=changed;Ok(())
    }
    /// Remove logical anonymous extents. Partial backing remains allocated and
    /// charged until its final extent disappears; retained holes cannot remap.
    pub fn unmap_anonymous(&mut self,base:u64,len:u64)->Result<(),String> {
        let end=base.checked_add(len).ok_or("A64 unmap overflow")?;
        if len==0{return Err("empty A64 unmap".into());}
        let size=usize::try_from(len).map_err(|_|"A64 unmap host size overflow")?;
        self.validate_host_range(base,size)?;
        let mut cursor=base;
        while cursor<end {
            let r=&self.regions[self.region_at(cursor).ok_or("A64 unmap backing missing")?];
            if !matches!(&r.bytes,Backing::Anonymous(_)){return Err("A64 unmap requires anonymous backing".into());}
            cursor=end.min(r.base+r.bytes.len() as u64);
        }
        if self.protections.len()>65534{return Err("A64 unmap extent budget exceeded".into());}
        let mut changed=Vec::new();changed.try_reserve_exact(self.protections.len()+1).map_err(|_|"A64 unmap metadata allocation failed")?;
        for &r in &self.protections {
            let stop=r.base+r.len;
            if stop<=base||r.base>=end{changed.push(r);continue;}
            if r.base<base{changed.push(ProtectionRegion{len:base-r.base,..r});}
            if stop>end{changed.push(ProtectionRegion{base:end,len:stop-end,..r});}
        }
        if !unsafe{touchHLE_A64Wrapper_unmap(self.raw,base,size)}{return Err("A64 native unmap transaction rejected".into());}
        self.protections=changed;self.mapped_bytes-=size;
        let protections=&self.protections;
        let mut released=0;
        self.regions.retain(|r| {
            let keep=protections.iter().any(|p|p.base<r.base+r.bytes.len() as u64&&r.base<p.base+p.len);
            if !keep {released+=r.bytes.len();}keep
        });
        self.anonymous_bytes-=released;Ok(())
    }
    pub fn anonymous_backing_bytes(&self)->usize {self.anonymous_bytes}
    fn validate_permissions(&self,mut addr:u64,mut len:usize,required:u32)->Result<(),String> {
        self.validate_host_range(addr,len)?;
        while len!=0 {
            let region=self.protection_region(addr).ok_or("A64 protection metadata missing")?;
            if region.protection & required != required {return Err(format!("A64 guest permission failure at {addr:#x}"));}
            let count=len.min((region.base+region.len-addr) as usize);
            addr+=count as u64;len-=count;
        }
        Ok(())
    }

    fn region_at(&self, addr: u64) -> Option<usize> {
        let index = self
            .regions
            .partition_point(|region| region.base <= addr)
            .checked_sub(1)?;
        let region = &self.regions[index];
        ((addr - region.base) < region.bytes.len() as u64).then_some(index)
    }

    fn validate_host_range(&self, mut addr: u64, mut len: usize) -> Result<(), String> {
        addr.checked_add(u64::try_from(len).map_err(|_| "A64 host access size overflow")?)
            .ok_or("A64 host access address overflow")?;
        while len != 0 {
            let region = &self.regions[self
                .region_at(addr)
                .ok_or_else(|| format!("unmapped A64 host address {addr:#x}"))?];
            let offset = (addr - region.base) as usize;
            let logical=self.protection_region(addr).ok_or_else(||format!("unmapped A64 host address {addr:#x}"))?;
            let count = len.min(region.bytes.len() - offset).min((logical.base+logical.len-addr) as usize);
            addr += count as u64;
            len -= count;
        }
        Ok(())
    }

    /// Host inspection of bytes within one region, bypassing guest permissions.
    /// Use read_into for an access spanning adjacent regions.
    pub fn read_bytes(&self, addr: u64, len: usize) -> Option<&[u8]> {
        self.validate_host_range(addr,len).ok()?;
        let region = &self.regions[self.region_at(addr)?];
        let offset = usize::try_from(addr - region.base).ok()?;
        region.bytes.get(offset..offset.checked_add(len)?)
    }

    /// Bounded host inspection supporting an access across adjacent regions.
    /// Validation completes before the caller's output buffer is modified.
    pub fn read_into(&self, mut addr: u64, output: &mut [u8]) -> Result<(), String> {
        self.validate_host_range(addr, output.len())?;
        let mut copied = 0;
        while copied < output.len() {
            let region = &self.regions[self.region_at(addr).unwrap()];
            let offset = (addr - region.base) as usize;
            let count = (output.len() - copied).min(region.bytes.len() - offset);
            output[copied..copied + count].copy_from_slice(&region.bytes[offset..offset + count]);
            addr += count as u64;
            copied += count;
        }
        Ok(())
    }
    /// Copy a guest buffer only when every byte is mapped and readable.
    /// A permission failure leaves the output buffer unchanged.
    pub fn read_guest_into(&self, addr: u64, output: &mut [u8]) -> Result<(), String> {
        self.validate_permissions(addr,output.len(),Self::READ)?;
        self.read_into(addr, output)
    }
    /// Validate an entire kernel copyout before changing any guest bytes.
    pub fn validate_guest_write(&self, addr: u64, len: usize) -> Result<(), String> {
        self.validate_permissions(addr,len,Self::WRITE)
    }

    pub fn write_guest_into(&mut self, addr: u64, data: &[u8]) -> Result<(), String> {
        self.validate_guest_write(addr, data.len())?;
        self.try_write_bytes(addr, data)
    }

    pub fn read_u64(&self, addr: u64) -> Option<u64> {
        let mut bytes = [0; 8];
        self.read_into(addr, &mut bytes).ok()?;
        Some(u64::from_le_bytes(bytes))
    }

    /// Host initialization write bypasses guest write protection, but validates
    /// the full mapped range before modifying bytes. Invalidates affected JIT code.
    /// Panics for gaps or overflow; use try_write_bytes to receive an error.
    pub fn write_bytes(&mut self, addr: u64, data: &[u8]) {
        self.try_write_bytes(addr, data)
            .expect("invalid A64 host write");
    }

    pub fn try_write_bytes(&mut self, mut addr: u64, data: &[u8]) -> Result<(), String> {
        self.validate_host_range(addr, data.len())?;
        let start = addr;
        let mut copied = 0;
        while copied < data.len() {
            let index = self.region_at(addr).unwrap();
            let region = &mut self.regions[index];
            let offset = (addr - region.base) as usize;
            let count = (data.len() - copied).min(region.bytes.len() - offset);
            region.bytes[offset..offset + count].copy_from_slice(&data[copied..copied + count]);
            addr += count as u64;
            copied += count;
        }
        if !data.is_empty() {
            unsafe {
                touchHLE_A64Wrapper_invalidate_cache_range(self.raw, start, data.len());
            }
        }
        Ok(())
    }

    pub fn reg(&self, idx: usize) -> u64 {
        assert!(idx < 31);
        unsafe { touchHLE_A64Wrapper_get_reg(self.raw, idx) }
    }
    pub fn set_reg(&mut self, idx: usize, v: u64) {
        assert!(idx < 31);
        unsafe { touchHLE_A64Wrapper_set_reg(self.raw, idx, v) }
    }
    /// Raw low/high 64-bit lanes of a guest SIMD register, preserving NaN bits.
    pub fn vector(&self, idx: usize) -> [u64; 2] {
        assert!(idx < 32);
        let mut lanes = [0; 2];
        unsafe { touchHLE_A64Wrapper_get_vector(self.raw, idx, lanes.as_mut_ptr()) }
        lanes
    }
    pub fn set_vector(&mut self, idx: usize, lanes: [u64; 2]) {
        assert!(idx < 32);
        unsafe { touchHLE_A64Wrapper_set_vector(self.raw, idx, lanes.as_ptr()) }
    }
    pub fn pc(&self) -> u64 {
        unsafe { touchHLE_A64Wrapper_get_pc(self.raw) }
    }
    pub fn set_pc(&mut self, v: u64) {
        unsafe { touchHLE_A64Wrapper_set_pc(self.raw, v) }
    }
    pub fn sp(&self) -> u64 {
        unsafe { touchHLE_A64Wrapper_get_sp(self.raw) }
    }
    pub fn set_sp(&mut self, v: u64) {
        unsafe { touchHLE_A64Wrapper_set_sp(self.raw, v) }
    }
    pub fn pstate(&self) -> u32 {
        unsafe { touchHLE_A64Wrapper_get_pstate(self.raw) }
    }
    /// Update architectural status flags after a guest syscall return.
    pub fn set_pstate(&mut self, value: u32) {
        unsafe { touchHLE_A64Wrapper_set_pstate(self.raw, value) }
    }
    pub fn set_tpidrro_el0(&mut self, v: u64) {
        unsafe { touchHLE_A64Wrapper_set_tpidrro_el0(self.raw, v) }
    }
    pub fn tpidrro_el0(&self) -> u64 {
        unsafe {touchHLE_A64Wrapper_get_tpidrro_el0(self.raw)}
    }
    pub fn tpidr_el0(&self) -> u64 {
        unsafe { touchHLE_A64Wrapper_get_tpidr_el0(self.raw) }
    }
    /// Architectural register update, matching TPIDRRO_EL0 setter semantics.
    /// A thread switch must use restore_context, which clears reservations.
    pub fn set_tpidr_el0(&mut self, value: u64) {
        unsafe { touchHLE_A64Wrapper_set_tpidr_el0(self.raw, value) }
    }

    pub fn save_context(&self) -> A64Context {
        let raw = NonNull::new(unsafe { touchHLE_A64Context_new() })
            .expect("A64 context allocation failed");
        unsafe { touchHLE_A64Wrapper_save_context(self.raw, raw.as_ptr()) }
        A64Context { raw }
    }

    pub fn restore_context(&mut self, context: &A64Context) {
        unsafe { touchHLE_A64Wrapper_restore_context(self.raw, context.raw.as_ptr()) }
    }

    /// Run until something needs host attention or `ticks` runs out
    /// (`ticks: Some`), or execute one instruction (`ticks: None`).
    #[must_use]
    pub fn run_or_step(&mut self, ticks: Option<&mut u64>) -> A64State {
        let res = unsafe { touchHLE_A64Wrapper_run_or_step(self.raw, ticks) };
        match res {
            -1 => A64State::Normal,
            -2 => A64State::MemoryError(unsafe { touchHLE_A64Wrapper_mem_error_addr(self.raw) }),
            -3 => A64State::UndefinedInstruction,
            -4 => A64State::Breakpoint,
            svc if svc >= 0 => A64State::Svc(svc as u16),
            _ => panic!("Unexpected A64 execution result {res}"),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn objc_nil_movi_zeroes_both_vector_lanes() {
        let mut cpu = sparse_test_cpu();
        cpu.set_vector(0, [u64::MAX, u64::MAX]);
        run_instructions(&mut cpu, &[0x2f00e400]); // movi d0,#0
        assert_eq!(cpu.vector(0), [0, 0]);
    }

    #[test]
    fn simd_accessors_match_guest_load_store_and_context_restore() {
        let mut cpu = sparse_test_cpu();
        let address = 0x2_0000_0000;
        cpu.map_zeroed(address, 16, A64Cpu::READ | A64Cpu::WRITE).unwrap();
        let lanes = [0x7ff8_1234_5678_9abc, 0xfedc_ba98_7654_3210];
        cpu.set_vector(0, lanes);
        cpu.set_reg(1, address);
        run_instructions(&mut cpu, &[0x3d800020]); // str q0,[x1]
        let bytes = [lanes[0].to_le_bytes(), lanes[1].to_le_bytes()].concat();
        assert_eq!(cpu.read_bytes(address, 16).unwrap(), bytes.as_slice());
        run_instructions(&mut cpu, &[0x3dc0003f]); // ldr q31,[x1]
        assert_eq!(cpu.vector(31), lanes);
        let context = cpu.save_context();
        cpu.set_vector(0, [0, 0]);
        cpu.set_vector(31, [1, 2]);
        cpu.restore_context(&context);
        assert_eq!(cpu.vector(0), lanes);
        assert_eq!(cpu.vector(31), lanes);
    }

    #[test]
    fn kernel_copyout_validates_all_regions_before_writing() {
        let mut cpu = super::A64Cpu::new_sparse();
        cpu.map_zeroed(0x1000, 4, 3).unwrap();
        cpu.map_zeroed(0x1004, 4, 1).unwrap();
        assert!(cpu.write_guest_into(0x1002, &[7; 4]).is_err());
        assert_eq!(cpu.read_bytes(0x1000, 8), None);
        assert_eq!(cpu.read_bytes(0x1000, 4).unwrap(), &[0; 4]);
        cpu.write_guest_into(0x1000, &[1, 2, 3, 4]).unwrap();
        assert_eq!(cpu.read_bytes(0x1000, 4).unwrap(), &[1, 2, 3, 4]);
        assert!(cpu.validate_guest_write(u64::MAX, 2).is_err());
        assert!(cpu.validate_guest_write(0x1002, 12).is_err());
    }
    use super::*;

    /// Raw machine code assembled from tests/a64/selftest_routine.s.
    const ROUTINE: &[u8] = include_bytes!("../../../tests/a64/selftest_routine.bin");

    fn sparse_test_cpu() -> A64Cpu {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x1_0000_0000, 0x2000, A64Cpu::READ | A64Cpu::EXECUTE)
            .unwrap();
        cpu
    }

    #[test]
    fn guest_buffer_reads_respect_permissions_before_copying() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x1000, 2, A64Cpu::READ).unwrap();
        cpu.map_zeroed(0x1002, 2, A64Cpu::WRITE).unwrap();
        cpu.write_bytes(0x1000, b"abcd");
        let mut output = [0xff; 4];
        assert!(cpu.read_guest_into(0x1000, &mut output).is_err());
        assert_eq!(output, [0xff; 4]);
        cpu.read_guest_into(0x1000, &mut output[..2]).unwrap();
        assert_eq!(&output[..2], b"ab");
        assert!(cpu.read_guest_into(u64::MAX, &mut output).is_err());
    }

    #[test]
    fn syscall_carry_flag_is_observed_by_guest_arithmetic() {
        let mut cpu = sparse_test_cpu();
        cpu.set_pstate(1 << 29);
        run_instructions(&mut cpu, &[0x9a1f03e0]); // adc x0,xzr,xzr
        assert_eq!(cpu.reg(0), 1);
        cpu.set_pstate(cpu.pstate() & !(1 << 29));
        run_instructions(&mut cpu, &[0x9a1f03e0]);
        assert_eq!(cpu.reg(0), 0);
    }

    #[test]
    fn sparse_regions_allocate_only_mapped_bytes_and_preserve_context() {
        let mut cpu = sparse_test_cpu();
        let far = 0x4_0000_0000;
        cpu.map_zeroed(far, 8, A64Cpu::READ | A64Cpu::WRITE)
            .unwrap();
        cpu.write_bytes(far, &123u64.to_le_bytes());
        cpu.set_reg(1, far);
        run_instructions(&mut cpu, &[0xf9400020]); // ldr x0,[x1]
        assert_eq!(cpu.reg(0), 123);
        assert_eq!(cpu.mapped_bytes(), 0x2008);
        assert_eq!(cpu.size(), cpu.mapped_bytes());
        let saved = cpu.save_context();
        cpu.map_zeroed(far + 0x1000, 16, 0).unwrap();
        cpu.set_reg(0, 0);
        cpu.restore_context(&saved);
        assert_eq!(cpu.reg(0), 123);
        assert_eq!(cpu.read_u64(far), Some(123));
        assert_eq!(cpu.mapped_permissions(far + 0x1000), Some(0));
        assert!(cpu.map_zeroed(far + 4, 8, 3).is_err());
        assert!(cpu.map_zeroed(u64::MAX, 8, 3).is_err());
        assert!(cpu.map_zeroed(0, A64Cpu::MAX_MAPPED_BYTES + 1, 3).is_err());
    }

    #[test]
    fn guest_gap_read_and_execute_permissions_fault() {
        let mut cpu = sparse_test_cpu();
        let far = 0x4_0000_0000;
        cpu.set_reg(1, far);
        let code = cpu.base() + 0x1000;
        cpu.write_bytes(code, &0xf9400020u32.to_le_bytes()); // ldr x0,[x1]
        cpu.set_pc(code);
        let mut ticks = 100;
        assert_eq!(
            cpu.run_or_step(Some(&mut ticks)),
            A64State::MemoryError(far)
        );
        cpu.map_zeroed(far, 16, A64Cpu::WRITE).unwrap();
        cpu.set_pc(code);
        assert_eq!(
            cpu.run_or_step(Some(&mut ticks)),
            A64State::MemoryError(far)
        );
        cpu.write_bytes(far, &0xd4001001u32.to_le_bytes()); // host can initialize protected memory
        cpu.set_pc(far);
        assert_eq!(
            cpu.run_or_step(Some(&mut ticks)),
            A64State::MemoryError(far)
        );
    }

    #[test]
    fn unsupported_guest_instruction_halts_without_aborting_host() {
        let mut cpu = sparse_test_cpu();
        let code = cpu.base() + 0x1000;
        cpu.write_bytes(code, &[0, 0, 0, 0, 1, 0x10, 0, 0xd4]); // undefined; svc #0x80
        cpu.set_pc(code);
        let mut ticks = 100;
        assert_eq!(
            cpu.run_or_step(Some(&mut ticks)),
            A64State::UndefinedInstruction
        );
    }

    #[test]
    fn cross_region_scalar_and_vector_accesses_validate_whole_write() {
        let mut cpu = sparse_test_cpu();
        let far = 0x4_0000_0000;
        cpu.map_zeroed(far, 4, 3).unwrap();
        cpu.map_zeroed(far + 4, 4, 3).unwrap();
        cpu.write_bytes(far, &0x8877665544332211u64.to_le_bytes());
        assert_eq!(cpu.read_u64(far), Some(0x8877665544332211));
        cpu.set_reg(1, far);
        run_instructions(&mut cpu, &[0xf9400020]);
        assert_eq!(cpu.reg(0), 0x8877665544332211);
        cpu.set_reg(0, 99);
        run_instructions(&mut cpu, &[0xf9000020]); // str x0,[x1]
        assert_eq!(cpu.read_u64(far), Some(99));

        let vector_address = far + 0x1000;
        cpu.map_zeroed(vector_address, 8, 3).unwrap();
        cpu.map_zeroed(vector_address + 8, 8, 1).unwrap();
        cpu.write_bytes(vector_address, &[0x77; 16]);
        cpu.set_reg(1, vector_address);
        let code = cpu.base() + 0x1000;
        cpu.write_bytes(code, &0x3d800020u32.to_le_bytes()); // str q0,[x1]
        cpu.set_pc(code);
        let mut ticks = 100;
        assert_eq!(
            cpu.run_or_step(Some(&mut ticks)),
            A64State::MemoryError(vector_address + 8)
        );
        let mut bytes = [0; 16];
        cpu.read_into(vector_address, &mut bytes).unwrap();
        assert_eq!(bytes, [0x77; 16]); // first half must not have been written
        assert!(cpu.try_write_bytes(far + 6, &[0; 8]).is_err());
        assert_eq!(cpu.read_u64(far), Some(99));
        let mut output = [0xff; 8];
        assert!(cpu.read_into(far + 6, &mut output).is_err());
        assert_eq!(output, [0xff; 8]);
    }

    #[test]
    fn mutate_region_changes_protected_memory_and_checks_backing_bounds() {
        let mut cpu = sparse_test_cpu();
        let base = cpu.base();
        cpu.mutate_region(base, 8, |bytes| {
            bytes.copy_from_slice(&42u64.to_le_bytes());
            Ok(())
        })
        .unwrap();
        assert_eq!(cpu.read_u64(base), Some(42));
        assert!(cpu.mutate_region(base + 0x1fff, 2, |_| Ok(())).is_err());
        assert!(cpu
            .mutate_region(base, 8, |bytes| {
                bytes[0] = 13;
                Err("test partial mutation".into())
            })
            .is_err());
        assert_eq!(cpu.read_u64(base), Some(13));
    }

    #[cfg(unix)]
    #[test]
    fn file_mapping_unaligned_offsets_are_private_and_bounded() {
        use std::io::{Read, Seek, SeekFrom, Write};
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "touchhle-a64-mmap-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut source = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        source.write_all(b"0123456789abcdef").unwrap();
        source.flush().unwrap();
        let mut cpu = A64Cpu::new_sparse();
        // Source remains unchanged and untruncated while this view exists.
        unsafe {
            cpu.map_file(0x4_0000_0000, &source, 3, 8, A64Cpu::READ)
                .unwrap();
        }
        assert_eq!(cpu.read_bytes(0x4_0000_0000, 8).unwrap(), b"3456789a");
        cpu.mutate_region(0x4_0000_0000, 8, |bytes| {
            bytes[0] = b'X';
            Ok(())
        })
        .unwrap();
        assert_eq!(cpu.read_bytes(0x4_0000_0000, 8).unwrap(), b"X456789a");
        unsafe {
            assert!(cpu.map_file(0x5_0000_0000, &source, 15, 2, 1).is_err());
        }
        drop(cpu);
        source.seek(SeekFrom::Start(0)).unwrap();
        let mut original = Vec::new();
        source.read_to_end(&mut original).unwrap();
        assert_eq!(original, b"0123456789abcdef");
        drop(source);
        std::fs::remove_file(path).unwrap();
    }

    fn run_instructions(cpu: &mut A64Cpu, instructions: &[u32]) {
        let code = cpu.base() + 0x1000;
        let mut bytes: Vec<u8> = instructions
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        bytes.extend_from_slice(&0xd4001001u32.to_le_bytes()); // svc #0x80
        cpu.write_bytes(code, &bytes);
        cpu.set_pc(code);
        let mut ticks = 1000;
        assert_eq!(cpu.run_or_step(Some(&mut ticks)), A64State::Svc(0x80));
    }

    #[test]
    fn a64_context_restores_scalar_simd_status_and_thread_registers() {
        let base = 0x1_0000_0000;
        let mut cpu = A64Cpu::new(base, 0x10000);
        let scratch = base + 0x8000;
        let vector: Vec<u8> = (0..16).map(|i| i * 13).collect();
        cpu.write_bytes(scratch, &vector);
        cpu.set_reg(1, scratch);
        run_instructions(&mut cpu, &[0x3dc00020]); // ldr q0,[x1]
        cpu.set_reg(0, 0xa0000000);
        run_instructions(&mut cpu, &[0xd51b4200]); // msr nzcv,x0
        cpu.set_reg(0, 0x00400000);
        run_instructions(&mut cpu, &[0xd51b4400]); // msr fpcr,x0
        cpu.set_reg(0, 0x08000001);
        run_instructions(&mut cpu, &[0xd51b4420]); // msr fpsr,x0
        for index in 0..31 {
            cpu.set_reg(index, 0x12340000 + index as u64);
        }
        cpu.set_sp(base + 0xff00);
        cpu.set_tpidrro_el0(base + 0x9000);
        let expected_pc = cpu.pc();
        let expected_pstate = cpu.pstate();
        let saved = cpu.save_context();

        cpu.write_bytes(scratch, &[0; 16]);
        cpu.set_reg(1, scratch);
        cpu.set_reg(0, 0);
        run_instructions(&mut cpu, &[0x3dc00020, 0xd51b4200, 0xd51b4400, 0xd51b4420]);
        cpu.set_sp(0);
        cpu.set_tpidrro_el0(0);
        cpu.restore_context(&saved);
        for index in 0..31 {
            assert_eq!(cpu.reg(index), 0x12340000 + index as u64);
        }
        assert_eq!(cpu.sp(), base + 0xff00);
        assert_eq!(cpu.pc(), expected_pc);
        assert_eq!(cpu.pstate(), expected_pstate);
        assert_eq!(cpu.read_bytes(scratch, 16).unwrap(), &[0; 16]); // memory is not restored

        cpu.set_reg(1, scratch);
        run_instructions(&mut cpu, &[0x3d800020]); // str q0,[x1]
        assert_eq!(cpu.read_bytes(scratch, 16).unwrap(), vector);
        run_instructions(&mut cpu, &[0xd53b4400]); // mrs x0,fpcr
        assert_eq!(cpu.reg(0), 0x00400000);
        run_instructions(&mut cpu, &[0xd53b4420]); // mrs x0,fpsr
        assert_eq!(cpu.reg(0), 0x08000001);
        run_instructions(&mut cpu, &[0xd53bd060]); // mrs x0,tpidrro_el0
        assert_eq!(cpu.reg(0), base + 0x9000);
    }

    #[test]
    fn a64_routine_runs_and_returns_result() {
        // Map the buffer above 4GiB on purpose, like an iOS arm64 executable.
        let base = 0x1_0000_0000u64;
        let mut cpu = A64Cpu::new(base, 0x10000);
        let code = base + 0x1000;
        let scratch = base + 0x8000;
        cpu.write_bytes(code, ROUTINE);
        cpu.set_reg(0, 100);
        cpu.set_reg(1, scratch);
        cpu.set_pc(code);
        cpu.set_sp(base + 0x10000);

        let mut ticks = 100_000;
        let state = cpu.run_or_step(Some(&mut ticks));
        assert_eq!(state, A64State::Svc(0x80));
        assert_eq!(cpu.pc(), code + ROUTINE.len() as u64);
        assert_eq!(cpu.reg(0), 5050 + (0xdead << 48));
        assert_eq!(cpu.read_u64(scratch), Some(5050));
        assert_eq!(cpu.read_u64(scratch + 32), Some(5050));
        assert_eq!(cpu.read_u64(scratch + 40), Some(0xdead << 48));
    }

    #[test]
    fn a64_out_of_range_access_is_reported() {
        let base = 0x1_0000_0000u64;
        let mut cpu = A64Cpu::new(base, 0x10000);
        // ldr x0, [x1] ; svc #0
        let code = [0x20, 0x00, 0x40, 0xf9, 0x01, 0x00, 0x00, 0xd4];
        cpu.write_bytes(base, &code);
        cpu.set_reg(1, 0x10); // below the mapped range ("null page")
        cpu.set_pc(base);
        let mut ticks = 1000;
        assert_eq!(
            cpu.run_or_step(Some(&mut ticks)),
            A64State::MemoryError(0x10)
        );
    }
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Unpublished iOS LP64 diagnostic thread-storage preparation.
pub const PAGE:u64=16384;
pub const DEFAULT_STACK:u64=512*1024;
pub const RECORD_SIZE:u64=8192;
pub const TSD_OFFSET:u64=224;
pub const TSD_SLOTS:usize=512;
pub const ERRNO_OFFSET:u64=172;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Detach{Joinable,Detached}
#[derive(Clone,Copy,Debug)]pub struct Attributes{pub stack_size:u64,pub detach:Detach}
impl Default for Attributes{fn default()->Self{Self{stack_size:DEFAULT_STACK,detach:Detach::Joinable}}}
impl Attributes{
    pub fn validate(self)->Result<Self,String>{if self.stack_size<PAGE||self.stack_size>1024*1024||self.stack_size%PAGE!=0{return Err("prepared stack must be iOS-page aligned and 16KiB..1MiB".into())}Ok(self)}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Layout{pub guard:(u64,u64),pub stack:(u64,u64),pub record:(u64,u64),pub code:(u64,u64),pub tsd:u64,pub errno:u64}
impl Layout{
    pub fn new(stack_base:u64,record_base:u64,code_base:u64,attrs:Attributes)->Result<Self,String>{
        let attrs=attrs.validate()?;if stack_base<PAGE||stack_base%PAGE!=0||record_base==0||record_base%16!=0||code_base==0||code_base%4!=0{return Err("thread storage alignment/base invalid".into())}
        let stack=(stack_base,stack_base.checked_add(attrs.stack_size).ok_or("stack overflow")?);let guard=(stack_base-PAGE,stack_base);let record=(record_base,record_base.checked_add(RECORD_SIZE).ok_or("record overflow")?);let code=(code_base,code_base.checked_add(16).ok_or("code overflow")?);
        let ranges=[guard,stack,record,code];for i in 0..ranges.len(){for j in i+1..ranges.len(){if ranges[i].0<ranges[j].1&&ranges[j].0<ranges[i].1{return Err("prepared thread storage overlaps".into())}}}
        Ok(Self{guard,stack,record,code,tsd:record_base+TSD_OFFSET,errno:record_base+ERRNO_OFFSET})
    }
    pub fn record_bytes(self)->Vec<u8>{
        let mut bytes=vec![0u8;RECORD_SIZE as usize];
        // Keep signature ZERO: this record is not a registered Darwin pthread.
        bytes[TSD_OFFSET as usize..TSD_OFFSET as usize+8].copy_from_slice(&self.record.0.to_le_bytes());
        bytes[TSD_OFFSET as usize+8..TSD_OFFSET as usize+16].copy_from_slice(&self.errno.to_le_bytes());bytes
    }
    pub fn trampoline(self)->[u32;4]{[0xd63f0260,0xd61f0280,0xd4200000,0xd503201f]} // blr x19; br x20; brk #0; nop
    pub fn completion_pc(self)->u64{self.code.0+8}
}
/// AAPCS64 pthread_create arguments, not a completed creation receipt.
#[derive(Clone,Copy,Debug)]pub struct CreateArguments{pub output:u64,pub attributes:u64,pub routine:u64,pub argument:u64}
impl CreateArguments{pub fn from_registers(x:[u64;4])->Result<Self,String>{if x[0]==0||x[0]%8!=0||x[2]==0||x[2]%4!=0{return Err("pthread_create output/routine invalid".into())}Ok(Self{output:x[0],attributes:x[1],routine:x[2],argument:x[3]})}}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn source_audited_tsd_self_errno_and_unpublished_signature(){let l=Layout::new(0x80000,0x20000,0x30000,Attributes::default()).unwrap();let data=l.record_bytes();assert_eq!(u64::from_le_bytes(data[0..8].try_into().unwrap()),0);assert_eq!(u64::from_le_bytes(data[224..232].try_into().unwrap()),0x20000);assert_eq!(u64::from_le_bytes(data[232..240].try_into().unwrap()),0x20000+172);assert_eq!(data[224+511*8..224+512*8],[0;8]);assert_eq!(l.stack.1-l.stack.0,DEFAULT_STACK);}
    #[test]fn layout_overlap_overflow_and_bounds(){assert!(Layout::new(0x80000,0x80000,0x30000,Attributes::default()).is_err());assert!(Layout::new(u64::MAX-0x3fff,0x20000,0x30000,Attributes::default()).is_err());assert!(Attributes{stack_size:PAGE-1,..Default::default()}.validate().is_err());assert!(Attributes{stack_size:2*1024*1024,..Default::default()}.validate().is_err());}
    #[test]fn abi_preserves_argument_and_does_not_publish_output(){let args=CreateArguments::from_registers([0x1000,0,0x5000,0xdeadbeef]).unwrap();assert_eq!(args.argument,0xdeadbeef);assert_eq!(args.output,0x1000);assert!(CreateArguments::from_registers([0,0,0x5000,0]).is_err());}
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Only prepares an unpublished diagnostic context. Never binds pthread_create.
#[path="a64_pthread_create_plan.rs"]mod plan;
pub(super) use plan::{Attributes,CreateArguments,Layout};
use super::A64Cpu;
use touchHLE_dynarmic_wrapper::a64::A64Context;
pub(super) struct Prepared{pub context:A64Context,pub layout:Layout,pub arguments:CreateArguments}
pub(super) fn prepare(cpu:&mut A64Cpu,args:CreateArguments,layout:Layout)->Result<Prepared,String>{
    CreateArguments::from_registers([args.output,args.attributes,args.routine,args.argument])?;
    let canonical=Layout::new(layout.stack.0,layout.record.0,layout.code.0,Attributes{stack_size:layout.stack.1.checked_sub(layout.stack.0).ok_or("invalid prepared stack range")?,..Default::default()})?;
    if layout!=canonical{return Err("prepared layout fields disagree with audited layout".into())}
    let output_end=args.output.checked_add(8).ok_or("pthread output range overflow")?;
    for(start,end)in[layout.guard,layout.stack,layout.record,layout.code]{if args.output<end&&start<output_end{return Err("pthread output overlaps unpublished thread storage".into())}}
    if args.routine>=layout.code.0&&args.routine<layout.code.1{return Err("start routine aliases its own preparation trampoline".into())}
    if args.attributes!=0{return Err("nondefault Darwin thread attributes require audited decoding; not prepared".into())}
    cpu.validate_guest_write(args.output,8)?;
    let mut instruction=[0;4];cpu.read_guest_into(args.routine,&mut instruction)?;if !cpu.mapped_permissions(args.routine).is_some_and(|p|p&4!=0){return Err("thread start routine not executable".into())}
    cpu.validate_guest_write(layout.stack.0,(layout.stack.1-layout.stack.0)as usize)?;cpu.validate_guest_write(layout.record.0,(layout.record.1-layout.record.0)as usize)?;
    for addr in layout.guard.0..layout.guard.1{if cpu.mapped_permissions(addr).is_some(){return Err("thread guard must remain entirely unmapped".into())}}
    for addr in layout.code.0..layout.code.1{if !cpu.mapped_permissions(addr).is_some_and(|p|p&5==5){return Err("start trampoline must occupy dedicated readable executable storage".into())}}
    let mut code=[0;16];cpu.read_into(layout.code.0,&mut code)?;if code.iter().any(|&b|b!=0){return Err("trampoline storage is not fresh/exclusive".into())}
    let mut record=vec![0;plan::RECORD_SIZE as usize];cpu.read_into(layout.record.0,&mut record)?;if record.iter().any(|&b|b!=0){return Err("thread record is not fresh/exclusive".into())}
    // Validate all fallible operations before storing prepared guest bytes.
    cpu.write_guest_into(layout.record.0,&layout.record_bytes())?;let bytes:Vec<u8>=layout.trampoline().iter().flat_map(|w|w.to_le_bytes()).collect();cpu.write_bytes(layout.code.0,&bytes);
    let caller=cpu.save_context();for i in 0..31{cpu.set_reg(i,0)}for i in 0..32{cpu.set_vector(i,[0,0])}cpu.set_reg(0,args.argument);cpu.set_reg(19,args.routine);cpu.set_reg(20,layout.completion_pc());cpu.set_sp(layout.stack.1);cpu.set_pc(layout.code.0);cpu.set_pstate(0);cpu.set_tpidrro_el0(layout.tsd);cpu.set_tpidr_el0(0);let context=cpu.save_context();cpu.restore_context(&caller);
    // Caller FP control/status remains inherited by the opaque CPU
    // API. This context must remain diagnostic/unpublished until that policy
    // and real Darwin registration/startup policy are explicitly established.
    Ok(Prepared{context,layout,arguments:args})
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn genuine_start_argument_tsd_and_completion_trap_without_publication(){let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,3).unwrap();cpu.map_zeroed(0x20000,8192,3).unwrap();cpu.map_zeroed(0x30000,4096,5).unwrap();cpu.map_zeroed(0x50000,4096,5).unwrap();cpu.map_zeroed(0x80000,512*1024,3).unwrap();cpu.write_bytes(0x50000,&[0x91000400u32,0xd65f03c0].iter().flat_map(|w|w.to_le_bytes()).collect::<Vec<_>>());cpu.set_reg(7,777);let layout=Layout::new(0x80000,0x20000,0x30000,Attributes::default()).unwrap();let args=CreateArguments::from_registers([0x10000,0,0x50000,41]).unwrap();let prepared=prepare(&mut cpu,args,layout).unwrap();assert_eq!(cpu.reg(7),777);assert_eq!(cpu.read_u64(0x10000),Some(0));assert_eq!(cpu.read_u64(0x20000),Some(0));cpu.restore_context(&prepared.context);for _ in 0..4{cpu.run_or_step(None);}assert_eq!(cpu.pc(),layout.completion_pc());assert_eq!(cpu.reg(0),42);assert_eq!(cpu.sp(),layout.stack.1);}
    #[test]fn mapped_guard_rejects_before_guest_changes(){let mut cpu=A64Cpu::new_sparse();for(base,size,perm)in[(0x10000,4096,3),(0x20000,8192,3),(0x30000,4096,5),(0x50000,4096,5),(0x7c000,16384,3),(0x80000,512*1024,3)]{cpu.map_zeroed(base,size,perm).unwrap();}let layout=Layout::new(0x80000,0x20000,0x30000,Attributes::default()).unwrap();let args=CreateArguments::from_registers([0x10000,0,0x50000,1]).unwrap();assert!(prepare(&mut cpu,args,layout).is_err());assert_eq!(cpu.read_u64(0x20000+224),Some(0));assert_eq!(cpu.read_u64(0x30000),Some(0));}
}

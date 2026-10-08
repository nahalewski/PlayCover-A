/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Native thread-register contract fixtures; central builder registers module.
#[cfg(test)]mod tests{
    use super::super::A64Cpu;
    #[test]fn real_tpidr_el0_mrs_msr_and_snapshot_restore(){
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x50000,4096,5).unwrap();cpu.set_tpidr_el0(0x12345678);cpu.set_tpidrro_el0(0xabcdef00);assert_eq!(cpu.tpidr_el0(),0x12345678);
        cpu.write_bytes(0x50000,&0xd53bd040u32.to_le_bytes());cpu.set_pc(0x50000);cpu.run_or_step(None);assert_eq!(cpu.reg(0),0x12345678);
        let saved=cpu.save_context();cpu.set_tpidr_el0(0);cpu.set_tpidrro_el0(0);cpu.restore_context(&saved);assert_eq!(cpu.tpidr_el0(),0x12345678);
        cpu.write_bytes(0x50004,&0xd53bd060u32.to_le_bytes());cpu.set_pc(0x50004);cpu.run_or_step(None);assert_eq!(cpu.reg(0),0xabcdef00);
        cpu.write_bytes(0x50008,&0xd51bd040u32.to_le_bytes());cpu.set_reg(0,0x76543210);cpu.set_pc(0x50008);cpu.run_or_step(None);assert_eq!(cpu.tpidr_el0(),0x76543210);
    }
    #[test]fn preparation_clears_new_thread_register_but_preserves_caller(){
        use super::super::pthread_create_prepare::{prepare,Attributes,CreateArguments,Layout};
        let mut cpu=A64Cpu::new_sparse();for(base,size,perm)in[(0x10000,4096,3),(0x20000,8192,3),(0x30000,4096,5),(0x50000,4096,5),(0x80000,512*1024,3)]{cpu.map_zeroed(base,size,perm).unwrap();}
        cpu.set_tpidr_el0(0xfeedface);cpu.set_tpidrro_el0(0x12345000);cpu.write_bytes(0x50000,&[0xd53bd041u32,0xd53bd062,0xd65f03c0].iter().flat_map(|w|w.to_le_bytes()).collect::<Vec<_>>());
        let layout=Layout::new(0x80000,0x20000,0x30000,Attributes::default()).unwrap();let args=CreateArguments::from_registers([0x10000,0,0x50000,42]).unwrap();let prepared=prepare(&mut cpu,args,layout).unwrap();assert_eq!(cpu.tpidr_el0(),0xfeedface);
        cpu.restore_context(&prepared.context);assert_eq!(cpu.tpidr_el0(),0);for _ in 0..5{cpu.run_or_step(None);}assert_eq!(cpu.reg(1),0);assert_eq!(cpu.reg(2),layout.tsd);assert_eq!(cpu.pc(),layout.completion_pc());assert_eq!(cpu.read_u64(0x10000),Some(0));
    }
    #[test]fn thread_context_restore_drops_exclusive_reservation(){
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x40000,4096,3).unwrap();cpu.map_zeroed(0x50000,4096,5).unwrap();cpu.write_bytes(0x50000,&[0xc85f7c20u32,0xc8027c23].iter().flat_map(|w|w.to_le_bytes()).collect::<Vec<_>>());cpu.set_reg(1,0x40000);cpu.set_reg(3,99);cpu.set_tpidr_el0(0x1234);cpu.set_pc(0x50000);cpu.run_or_step(None);let saved=cpu.save_context();cpu.set_tpidr_el0(0);cpu.restore_context(&saved);cpu.run_or_step(None);assert_eq!(cpu.tpidr_el0(),0x1234);assert_eq!(cpu.reg(2),1);assert_eq!(cpu.read_u64(0x40000),Some(0));
    }
}

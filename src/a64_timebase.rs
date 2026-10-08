/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Checked conversion from the actual CPU-owned steady counter frequency.
//! XNU arm/rtclock.c reduces (1e9 nanoseconds / actual counter frequency).
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(super) struct Timebase {numer:u32,denom:u32}
impl Timebase {
    pub(super) fn for_cpu(cpu:&super::A64Cpu)->Result<Self,String> {Self::from_frequency(cpu.counter_frequency() as u64)}
    pub(super) fn from_frequency(frequency:u64)->Result<Self,String> {
        if frequency==0 {return Err("counter frequency must be nonzero".into());}
        let mut a=1_000_000_000u64;let mut b=frequency;
        while b!=0 {let remainder=a%b;a=b;b=remainder;}
        Ok(Self {
            numer:u32::try_from(1_000_000_000/a).map_err(|_|"timebase numerator overflow")?,
            denom:u32::try_from(frequency/a).map_err(|_|"timebase denominator overflow")?,
        })
    }
    pub(super) fn encode(self)->[u8;8] {
        let mut bytes=[0;8];bytes[..4].copy_from_slice(&self.numer.to_le_bytes());bytes[4..].copy_from_slice(&self.denom.to_le_bytes());bytes
    }
    pub(super) fn nanoseconds(self,ticks:u64)->Result<u64,String> {
        let result=(ticks as u128)*(self.numer as u128)/(self.denom as u128);
        u64::try_from(result).map_err(|_|"counter nanoseconds overflow".into())
    }
}
pub(super) fn absolute_time(cpu:&super::A64Cpu)->u64 {cpu.counter_ticks()}
pub(super) fn mach_timebase_info(cpu:&mut super::A64Cpu,output:u64)->Result<u32,String> {
    let info=Timebase::for_cpu(cpu)?.encode();
    // XNU clock.c407 intentionally ignores copyout errors, then returns0.
    // An invalid output is not silently made readable or initialized.
    let _=cpu.write_guest_into(output,&info);
    Ok(0)
}
/// Narrow signed Mach trap adapter. Returns false without mutations for any
/// other selector; supervisor routing and unknown-service refusal remain owned
/// by the actual process dispatcher. Mach results preserve condition flags.
pub(super) fn handle(cpu:&mut super::A64Cpu,selector:i32)->Result<bool,String> {
    match selector {
        -3=> {let ticks=absolute_time(cpu);cpu.set_reg(0,ticks);Ok(true)}
        -89=> {let output=cpu.reg(0);let status=mach_timebase_info(cpu,output)?;cpu.set_reg(0,status as u64);Ok(true)}
        _=>Ok(false),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ratio_tracks_actual_frequency_instead_of_unconditional_one_to_one() {
        assert_eq!(Timebase::from_frequency(600_000_000).unwrap(),Timebase{numer:5,denom:3});
        assert_eq!(Timebase::from_frequency(24_000_000).unwrap(),Timebase{numer:125,denom:3});
        assert_eq!(Timebase::from_frequency(1_000_000_000).unwrap(),Timebase{numer:1,denom:1});
        for frequency in [24_000_000,600_000_000,1_000_000_000] {
            assert_eq!(Timebase::from_frequency(frequency).unwrap().nanoseconds(frequency).unwrap(),1_000_000_000);
        }
    }
    #[test]
    fn ABI_is_two_u32_fields_and_invalid_or_overflowing_values_stop() {
        let ratio=Timebase::from_frequency(600_000_000).unwrap();
        assert_eq!(ratio.encode(),[5,0,0,0,3,0,0,0]);
        assert!(Timebase::from_frequency(0).is_err());assert!(Timebase::from_frequency(u64::MAX).is_err());
        assert!(Timebase::from_frequency(1).unwrap().nanoseconds(u64::MAX).is_err());
    }
    #[test]
    fn actual_cntpct_and_cntfrq_instructions_match_the_trap_counter() {
        use super::super::{A64Cpu,A64State};
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();
        let code:Vec<_>=[0xd53be000u32,0xd53be021,0xd4001001].into_iter().flat_map(|op|op.to_le_bytes()).collect();
        cpu.write_bytes(0x10000,&code);cpu.set_pc(0x10000);
        let before=absolute_time(&cpu);let mut ticks=100;
        assert_eq!(cpu.run_or_step(Some(&mut ticks)),A64State::Svc(0x80));
        let after=absolute_time(&cpu);
        assert_eq!(cpu.reg(0),cpu.counter_frequency() as u64);
        assert_eq!(cpu.reg(0),1_000_000_000);
        assert!(cpu.reg(1)>=before && cpu.reg(1)<=after);assert!(after>0);
        let ratio=Timebase::for_cpu(&cpu).unwrap();assert_eq!(ratio.nanoseconds(after).unwrap(),after);
    }
    #[test]
    fn timebase_copyout_uses_shared_cpu_ratio_and_preserves_exact_kernel_error_rule() {
        use super::super::A64Cpu;
        let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,3).unwrap();
        assert_eq!(mach_timebase_info(&mut cpu,0x10000).unwrap(),0);
        let mut info=[0;8];cpu.read_guest_into(0x10000,&mut info).unwrap();assert_eq!(info,[1,0,0,0,1,0,0,0]);
        assert_eq!(mach_timebase_info(&mut cpu,u64::MAX).unwrap(),0);
        cpu.map_zeroed(0x20000,4096,1).unwrap();assert_eq!(mach_timebase_info(&mut cpu,0x20000).unwrap(),0);
        cpu.read_guest_into(0x20000,&mut info).unwrap();assert_eq!(info,[0;8]);
    }
    #[test]
    fn counter_epoch_keeps_progressing_across_saved_thread_contexts() {
        let mut cpu=super::super::A64Cpu::new_sparse();
        let snapshot=cpu.save_context();let first=absolute_time(&cpu);
        cpu.set_reg(19,7);cpu.restore_context(&snapshot);let second=absolute_time(&cpu);
        assert!(second>=first);assert_eq!(cpu.reg(19),0);assert_eq!(cpu.counter_frequency(),1_000_000_000);
    }
    #[test]
    fn real_guest_time_traps_share_counter_and_preserve_mach_flags() {
        use super::super::{A64Cpu,bridge::{GuestBridge,GuestCall}};
        let mut cpu=A64Cpu::new_sparse();let mut bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();
        cpu.map_zeroed(0x10000,4096,5).unwrap();cpu.map_zeroed(0x40000,4096,3).unwrap();
        // movn x16,#88;svc80;mrs x1,cntpct;movn x16,#2;svc80;ret
        let code:Vec<_>=[0x92800b10u32,0xd4001001,0xd53be021,0x92800050,0xd4001001,0xd65f03c0].into_iter().flat_map(|op|op.to_le_bytes()).collect();
        cpu.write_bytes(0x10000,&code);cpu.set_pstate(0xb0000000);cpu.set_reg(19,0x1234);
        let before=absolute_time(&cpu);let mut seen=Vec::new();
        let result=bridge.call_with_supervisor_handler(&mut cpu,&GuestCall{entry:0x10000,integers:vec![0x40000],..Default::default()},100,&mut|cpu,svc| {
            assert_eq!(svc,0x80);let selector=cpu.reg(16) as u32 as i32;let flags=cpu.pstate();let saved=cpu.reg(19);
            assert!(handle(cpu,selector)?);assert_eq!(cpu.pstate(),flags);assert_eq!(cpu.reg(19),saved);seen.push(selector);Ok(())
        }).unwrap();let after=absolute_time(&cpu);
        assert_eq!(seen,[-89,-3]);assert!(result.integers[0]>=before && result.integers[0]<=after);
        assert!(result.integers[0]>=result.integers[1]);
        let mut info=[0;8];cpu.read_guest_into(0x40000,&mut info).unwrap();assert_eq!(info,[1,0,0,0,1,0,0,0]);
        cpu.set_reg(0,77);let flags=cpu.pstate();assert!(!handle(&mut cpu,-90).unwrap());assert_eq!(cpu.reg(0),77);assert_eq!(cpu.pstate(),flags);
    }
}

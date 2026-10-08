/* This Source Code Form is subject to the Mozilla Public License, v. 2.0.
 * https://mozilla.org/MPL/2.0/ */
//! Original40-byte LP64 registration ABI translated to the retained domain model.
use super::{A64Cpu,pthread_registration::ProcessRegistration,thread_scheduler_cpu::Priority};
pub(super) fn register(cpu:&mut A64Cpu,process:&mut ProcessRegistration,args:[u64;6],priority:Priority)->Result<(),String>{
 if args[4]!=40||args[3]==0{return Err("unsupported legacy pthread registration extent".into());}
 let mut source=[0;40];cpu.read_guest_into(args[3],&mut source)?;
 if u64::from_le_bytes(source[..8].try_into().unwrap())!=40{return Err("legacy pthread registration version/size differs".into());}
 // The old structure ends after mach_thread_self_offset plus zero padding.
 // It has no modern stackhint/mutex/joinable/quantum input fields.
 if source[36..40]!=[0;4]{return Err("unsupported original pthread registration tail padding".into());}
 cpu.validate_guest_write(args[3],40)?;
 let entrypoints=[args[0],args[1]];let executable=entrypoints.map(|address|address&3==0&&cpu.mapped_permissions(address).is_some_and(|p|p&4!=0));
 let mut normalized=[0u8;56];normalized[..36].copy_from_slice(&source[..36]);normalized[..8].copy_from_slice(&56u64.to_le_bytes());
 let mut domain_args=args;domain_args[4]=56;
 process.register_copyout(domain_args,&normalized,|address|entrypoints.iter().zip(executable).any(|(&entry,rx)|address==entry&&rx),priority,|address,response|{
  let mut legacy=[0;40];legacy[..36].copy_from_slice(&response[..36]);legacy[..8].copy_from_slice(&40u64.to_le_bytes());cpu.write_guest_into(address,&legacy)
 })
}
#[cfg(test)]mod tests{
 use super::*;
 fn fixture()->(A64Cpu,[u64;6]){
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x180b1b000,4096,5).unwrap();cpu.map_zeroed(0x20000,4096,3).unwrap();let mut bytes=[0u8;40];
  bytes[..8].copy_from_slice(&40u64.to_le_bytes());bytes[8..16].copy_from_slice(&160u64.to_le_bytes());bytes[24..28].copy_from_slice(&224u32.to_le_bytes());bytes[28..32].copy_from_slice(&40u32.to_le_bytes());bytes[32..36].copy_from_slice(&24u32.to_le_bytes());cpu.write_guest_into(0x20000,&bytes).unwrap();cpu.write_guest_into(0x20028,&[0xab;16]).unwrap();
  (cpu,[0x180b1bb0c,0x180b1bb04,0x4000,0x20000,40,160])
 }
 #[test]fn original_registration_keeps_exact_guest_extent_and_real_domain_offsets(){
  let(mut cpu,args)=fixture();let mut process=ProcessRegistration::default();register(&mut cpu,&mut process,args,Priority::decode(0x8ff).unwrap()).unwrap();let record=process.record().unwrap();
  assert_eq!((record.tsd_offset,record.return_offset,record.mach_thread_offset),(224,40,24));assert_eq!((record.joinable_offset_bits,record.quantum_offset),(0,0));assert_eq!(cpu.read_u64(0x20000),Some(40));assert_eq!(cpu.read_u64(0x20010),Some(0x8ff));assert_eq!(cpu.read_bytes(0x20028,16).unwrap(),&[0xab;16]);
 }
 #[test]fn invalid_original_registration_never_publishes_or_partially_copies(){
  let(mut cpu,args)=fixture();let mut process=ProcessRegistration::default();cpu.write_guest_into(0x20018,&0x4000u32.to_le_bytes()).unwrap();let before=cpu.read_bytes(0x20000,56).unwrap().to_vec();
  assert!(register(&mut cpu,&mut process,args,Priority::decode(0x8ff).unwrap()).is_err());assert!(process.record().is_none());assert_eq!(cpu.read_bytes(0x20000,56).unwrap(),before.as_slice());
 }
}

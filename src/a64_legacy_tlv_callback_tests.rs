/* This Source Code Form is subject to the Mozilla Public License, v. 2.0.
 * https://mozilla.org/MPL/2.0/ */
//! Original callback execution with explicit prior catalogue capacity.
//! This isolates callback semantics; it does not test cold malloc/bootstrap.
use super::{A64Cpu,bridge::{GuestBridge,GuestCall},thread_scheduler_cpu::CpuScheduler,
 thread_storage::ThreadStorage,mach_identity::MachIdentity};
use std::{cell::RefCell,rc::Rc,io::{Read,Seek,SeekFrom},path::PathBuf};
fn word(cpu:&A64Cpu,address:u64)->u32 {
 let mut bytes=[0;4];cpu.read_guest_into(address,&mut bytes).unwrap();u32::from_le_bytes(bytes)
}
fn original_cache()->A64Cpu {
 let path=PathBuf::from(std::env::var_os("PLAYCOVER_NATIVE_CACHE").expect("explicit original15G77 cache required"));
 let mut file=std::fs::File::open(&path).unwrap();file.seek(SeekFrom::Start(88)).unwrap();
 let mut uuid=[0;16];file.read_exact(&mut uuid).unwrap();
 assert_eq!(uuid,[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f]);
 let (_,mut cpu)=super::cache_map::map(&path).unwrap();super::commpage::ensure_mapped(&mut cpu).unwrap();cpu
}

#[test]
#[ignore = "Requires PLAYCOVER_NATIVE_CACHE original15G77; executes original TLV notification/key/mutex callbacks"]
fn original_tlv_notification_publishes_real_key_thunk_and_preserves_offset() {
 let path=PathBuf::from(std::env::var_os("PLAYCOVER_NATIVE_CACHE").expect("explicit original15G77 cache required"));
 let mut file=std::fs::File::open(&path).unwrap();file.seek(SeekFrom::Start(88)).unwrap();
 let mut uuid=[0;16];file.read_exact(&mut uuid).unwrap();
 assert_eq!(uuid,[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f]);
 let (_plan,mut cpu)=super::cache_map::map(&path).unwrap();super::commpage::ensure_mapped(&mut cpu).unwrap();
 cpu.map_zeroed(0x3000000000,0x10000,3).unwrap();cpu.set_sp(0x300000fff0);
 cpu.set_pc(0x1808501b8);cpu.set_tpidrro_el0(0x30000000e0);
 let mut scheduler=CpuScheduler::default();let owner=scheduler.adopt(&cpu,(0x3000000000,0x3000010000)).unwrap();
 assert_eq!(scheduler.select(&mut cpu).unwrap(),Some(owner));let scheduler=Rc::new(RefCell::new(scheduler));
 let mut ports=MachIdentity::new(8).unwrap();ports.register_thread(owner.0).unwrap();
 let record=0x1b3288b40u64;let tsd=record+224;
 cpu.write_guest_into(record+0xd8,&owner.0.to_le_bytes()).unwrap();cpu.write_guest_into(tsd,&record.to_le_bytes()).unwrap();
 cpu.write_guest_into(tsd+8,&(record+0x48).to_le_bytes()).unwrap();cpu.write_guest_into(tsd+24,&0u64.to_le_bytes()).unwrap();
 let journal=Rc::new(ThreadStorage::new(owner,cpu.tpidrro_el0()));journal.enable_legacy();
 cpu.set_pc(0x18097f540);cpu.set_reg(0,tsd);cpu.set_reg(3,2);journal.adopt_original(&mut cpu,&scheduler,&ports).unwrap();
 let mut bridge=GuestBridge::map_runtime(&mut cpu,0x3100000000).unwrap();bridge.set_thread_storage(journal);

 let header=0x3000002000u64;let descriptor=0x3000004000u64;let catalogue=0x3000005000u64;
 let mut metadata=vec![0u8;32+152];
 for(offset,value)in[(0,0xfeedfacfu32),(4,0x100000c),(8,0),(12,6),(16,1),(20,152),(24,0x800000),
  (32,0x19),(36,152),(32+56,7),(32+60,3),(32+64,1),(32+72+64,0x13)] {
  metadata[offset..offset+4].copy_from_slice(&value.to_le_bytes());
 }
 metadata[40..46].copy_from_slice(b"__DATA");
 for(offset,value)in[(32+24,header),(32+32,0x4000),(32+40,0),(32+48,0x4000),
  (32+72+32,descriptor),(32+72+40,24)] { metadata[offset..offset+8].copy_from_slice(&value.to_le_bytes()); }
 metadata[32+72..32+72+13].copy_from_slice(b"__thread_vars");
 metadata[32+72+16..32+72+22].copy_from_slice(b"__DATA");
 cpu.write_guest_into(header,&metadata).unwrap();
 let sentinel=0x18u64;cpu.write_guest_into(descriptor,&0xabcdu64.to_le_bytes()).unwrap();
 cpu.write_guest_into(descriptor+8,&0u64.to_le_bytes()).unwrap();cpu.write_guest_into(descriptor+16,&sentinel.to_le_bytes()).unwrap();
 // Explicit valid prior catalogue capacity isolates the actual notification
 // from cold original malloc. Only this private fixture's cache pages change.
 cpu.write_guest_into(0x1b1a166b8,&0u32.to_le_bytes()).unwrap();
 cpu.write_guest_into(0x1b1a166c0,&catalogue.to_le_bytes()).unwrap();
 cpu.write_guest_into(0x1b1a166c8,&8u32.to_le_bytes()).unwrap();
 bridge.call(&mut cpu,&GuestCall{entry:0x1808501b8,integers:vec![header,0],..Default::default()},100_000)
  .expect("original notification/key/mutex must genuinely execute");
 let key=cpu.read_u64(descriptor+8).unwrap();assert!((256..512).contains(&key));
 assert_eq!(cpu.read_u64(descriptor),Some(0x180850408));assert_eq!(cpu.read_u64(descriptor+16),Some(sentinel));
 assert_eq!(cpu.read_u64(catalogue),Some(key));assert_eq!(cpu.read_u64(catalogue+8),Some(header));
 assert_eq!(word(&cpu,0x1b1a166b8),1);
 assert_eq!(cpu.read_u64(0x1b3289c98+key*8),Some(!0x1808503dcu64));
 assert_eq!(cpu.read_u64(tsd+key*8),Some(0)); // Notification does not fake lazy allocation.
 assert_eq!(cpu.tpidrro_el0(),tsd);assert_eq!(scheduler.borrow().current(),Some(owner));
 // Header lacking MH_HAS_TLV_DESCRIPTORS must not publish another image/key.
 cpu.write_guest_into(header+24,&0u32.to_le_bytes()).unwrap();
 bridge.call(&mut cpu,&GuestCall{entry:0x1808501b8,integers:vec![header,0],..Default::default()},100_000).unwrap();
 assert_eq!(word(&cpu,0x1b1a166b8),1);assert_eq!(cpu.read_u64(descriptor+8),Some(key));
 assert_eq!(cpu.read_u64(descriptor+16),Some(sentinel));
 println!("PLAYCOVER_LEGACY_TLV: actual notification/key/mutex passed with explicit prior capacity; no cold-start or gameplay receipt");
}

fn original_clear_flag_notifications()->(A64Cpu,GuestBridge,u64,Vec<u64>) {
 let mut cpu=original_cache();let base=0x3000000000u64;
 cpu.map_zeroed(base,427*64,3).unwrap();let mut images=Vec::new();let mut headers=Vec::new();
 for index in 0..427u64 {
  let header=base+index*64;let mut bytes=[0u8;32];
  for(offset,value)in[(0,0xfeedfacfu32),(4,0x100000c),(12,6)]{bytes[offset..offset+4].copy_from_slice(&value.to_le_bytes());}
  cpu.write_guest_into(header,&bytes).unwrap();
  images.push(super::legacy_add_images::Image::read(&cpu,header,index as i64*0x4000).unwrap());headers.push(header);
 }
 let mut bridge=GuestBridge::map_runtime(&mut cpu,0x3100000000).unwrap();
 // Explicit helper-registration test prerequisite; callback instructions are
 // untouched original cache bytes. Actual registration is tested separately.
 let helpers=Rc::new(RefCell::new(super::legacy_dyld_lookup::HelperRegistration::test_registered()));
 let entry=super::legacy_add_images::install(&mut cpu,&mut bridge,images,helpers).unwrap();
 (cpu,bridge,entry,headers)
}

#[test]
#[ignore = "Requires original15G77 PLAYCOVER_NATIVE_CACHE;427 actual original flag-clear callbacks"]
fn all_427_original_callbacks_return_under_one_bounded_driver_call() {
 let(mut cpu,mut bridge,entry,headers)=original_clear_flag_notifications();
 cpu.set_pc(0x12340);cpu.set_sp(0x45670);cpu.set_reg(A64Cpu::LR,0x78900);
 cpu.set_reg(19,0xabcd);cpu.set_vector(8,[123,456]);cpu.set_tpidrro_el0(0x1b3288c20);
 let call=GuestCall{entry,integers:vec![0x1808501b8],..Default::default()};
 bridge.call(&mut cpu,&call,100_000).expect("all427 actual original callbacks must complete with shared bounded ticks");
 assert_eq!((cpu.pc(),cpu.sp(),cpu.reg(A64Cpu::LR)),(0x12340,0x45670,0x78900));
 assert_eq!(cpu.reg(19),0xabcd);assert_eq!(cpu.vector(8),[123,456]);assert_eq!(cpu.tpidrro_el0(),0x1b3288c20);
 for header in headers {assert_eq!(word(&cpu,header),0xfeedfacf);assert_eq!(word(&cpu,header+24),0);}
 assert!(bridge.call(&mut cpu,&call,100_000).is_err(),"completed notification registration cannot replay");
 println!("PLAYCOVER_LEGACY_TLV_427: actual original callbacks returned under shared100000ticks; no cold-start receipt");
}

#[test]
#[ignore = "Requires original15G77 PLAYCOVER_NATIVE_CACHE; real callback driver partial-delivery quarantine"]
fn original_callback_driver_quarantines_mutated_last_header_after_partial_delivery() {
 let(mut cpu,mut bridge,entry,headers)=original_clear_flag_notifications();let last=*headers.last().unwrap();
 cpu.write_guest_into(last,&0u32.to_le_bytes()).unwrap();
 let call=GuestCall{entry,integers:vec![0x1808501b8],..Default::default()};
 let error=bridge.call(&mut cpu,&call,100_000).unwrap_err();assert!(error.contains("header mutated"),"{error}");
 cpu.write_guest_into(last,&0xfeedfacfu32.to_le_bytes()).unwrap();
 let retry=bridge.call(&mut cpu,&call,100_000).unwrap_err();assert!(retry.contains("quarantined"),"{retry}");
 println!("PLAYCOVER_LEGACY_TLV_PARTIAL: actual original callbacks before mutated finalheader; repair cannot replay partial delivery");
}

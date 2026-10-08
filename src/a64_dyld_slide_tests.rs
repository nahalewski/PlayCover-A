/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Regression coverage for exact-header image provenance, not guessed slides.
use super::dyld_slide::ImageSlides;
use super::{A64Cpu,bridge::{GuestBridge,GuestCall},dyld_slide::{LoaderPolicy,install_restricted,RESTRICTED_ENTRY}};

#[test]
fn cache_zero_and_relocated_ordinary_headers_have_distinct_real_slides() {
    let cache_header=0x0000_0001_9454_0000;
    let preferred_header=0x1_0000_0000u64;
    let ordinary_slide=0x20_0000i64;
    let actual_header=preferred_header+ordinary_slide as u64;
    let ledger=ImageSlides::new(&[(cache_header,0),(actual_header,ordinary_slide)]).unwrap();
    assert_eq!(ledger.lookup(cache_header).unwrap(),0);
    assert_eq!(ledger.lookup(actual_header).unwrap(),ordinary_slide);
    assert!(ledger.lookup(preferred_header).is_err());
    assert!(ledger.lookup(actual_header+32).is_err());
}

#[test]
fn unknown_header_never_becomes_a_fabricated_zero_slide() {
    let ledger=ImageSlides::new(&[(0x194540000,0)]).unwrap();
    for unknown in [0,4,u64::MAX,0x194540001,0x194550000] {
        assert!(ledger.lookup(unknown).is_err(),"unknown header {unknown:#x} unexpectedly resolved");
    }
}

#[test]
fn conflicting_header_provenance_is_rejected_before_any_lookup() {
    assert!(ImageSlides::new(&[(0x194540000,0),(0x194540000,0x4000)]).is_err());
}

#[test]
fn negative_signed_slide_is_preserved_and_invalid_ledger_is_rejected() {
    let ledger=ImageSlides::new(&[(0x100000000,-0x4000)]).unwrap();
    assert_eq!(ledger.lookup(0x100000000).unwrap(),-0x4000);
    assert!(ImageSlides::new(&[(0,0)]).is_err());
    assert!(ImageSlides::new(&[(0x100000001,0)]).is_err());
    let oversized:Vec<_>=(0..8193u64).map(|i|(0x100000000+i*0x4000,0i64)).collect();
    assert!(ImageSlides::new(&oversized).is_err());
}

#[test]
fn restricted_policy_denies_environment_search_overrides() {
    let policy=LoaderPolicy::declared_paths_only();
    assert!(policy.is_restricted());
    for path in ["/","/tmp/frameworks","@rpath/Injected.framework",""] {
        assert!(policy.environment_path_override(path).is_err());
    }
}

fn restricted_fixture()->(A64Cpu,GuestBridge) {
    let mut cpu=A64Cpu::new_sparse();
    let bridge=GuestBridge::map(&mut cpu,0x20000).unwrap();
    cpu.map_zeroed(RESTRICTED_ENTRY&!4095,4096,5).unwrap();
    let original=[0x88,0x85,0x1d,0xd0,0,0xc5,0x41,0xf9,8,0,0x40,0xf9,1,0xd1,0x40,0xf9,0x20,0,0x1f,0xd6];
    cpu.try_write_bytes(RESTRICTED_ENTRY,&original).unwrap();
    (cpu,bridge)
}

#[test]
fn restricted_query_executes_real_guest_redirect_and_preserves_caller_context() {
    let (mut cpu,mut bridge)=restricted_fixture();
    cpu.set_reg(19,0x123456);cpu.set_vector(8,[0xabcdef,0x987654]);cpu.set_pstate(0xa0000000);
    let sp=cpu.sp();let lr=cpu.reg(30);
    install_restricted(&mut cpu,&mut bridge,RESTRICTED_ENTRY,LoaderPolicy::declared_paths_only()).unwrap();
    let result=bridge.call(&mut cpu,&GuestCall{entry:RESTRICTED_ENTRY,..Default::default()},100).unwrap();
    assert_eq!(result.integers[0],1);
    assert_eq!(cpu.reg(19),0x123456);assert_eq!(cpu.vector(8),[0xabcdef,0x987654]);
    assert_eq!(cpu.sp(),sp);assert_eq!(cpu.reg(30),lr);assert_eq!(cpu.pstate()&0xf0000000,0xa0000000);
}

#[test]
fn restricted_redirect_rejects_foreign_entry_or_original_bytes_before_mutation() {
    let (mut cpu,mut bridge)=restricted_fixture();
    let mut before=[0;20];cpu.read_guest_into(RESTRICTED_ENTRY,&mut before).unwrap();
    assert!(install_restricted(&mut cpu,&mut bridge,RESTRICTED_ENTRY+4,LoaderPolicy::declared_paths_only()).is_err());
    let mut after=[0;20];cpu.read_guest_into(RESTRICTED_ENTRY,&mut after).unwrap();assert_eq!(after,before);
    cpu.try_write_bytes(RESTRICTED_ENTRY,&0xd503201fu32.to_le_bytes()).unwrap();
    cpu.read_guest_into(RESTRICTED_ENTRY,&mut before).unwrap();
    assert!(install_restricted(&mut cpu,&mut bridge,RESTRICTED_ENTRY,LoaderPolicy::declared_paths_only()).is_err());
    cpu.read_guest_into(RESTRICTED_ENTRY,&mut after).unwrap();assert_eq!(after,before);
}

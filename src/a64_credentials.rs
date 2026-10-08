/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Credential taint of the isolated guest process, never the host's identity.
//! No set-ID executable transitions or elevated guest credentials are granted.
use super::A64Cpu;
pub(super) struct CredentialTaint {tainted:bool}
impl CredentialTaint {
 pub(super) fn isolated_unprivileged()->Self {Self{tainted:false}}
 /// An actual future set-ID transition must retain taint for the process.
 /// This records state only; it does not perform or authorize that transition.
 pub(super) fn record_set_id_transition(&mut self){self.tainted=true;}
 pub(super) fn issetugid(&self,cpu:&mut A64Cpu){
  cpu.set_reg(0,u64::from(self.tainted));
  cpu.set_pstate(cpu.pstate()&!(1<<29));
 }
}
#[cfg(test)]mod tests {
 use super::*;
 #[test]fn isolated_guest_state_does_not_inherit_host_credentials_and_taint_is_retained(){
  let mut cpu=A64Cpu::new_sparse();let mut state=CredentialTaint::isolated_unprivileged();cpu.set_pstate(0xb0000000);cpu.set_reg(1,77);
  state.issetugid(&mut cpu);assert_eq!(cpu.reg(0),0);assert_eq!(cpu.pstate(),0x90000000);assert_eq!(cpu.reg(1),77);
  state.record_set_id_transition();state.issetugid(&mut cpu);assert_eq!(cpu.reg(0),1);
  state.record_set_id_transition();state.issetugid(&mut cpu);assert_eq!(cpu.reg(0),1);
  let other=CredentialTaint::isolated_unprivileged();other.issetugid(&mut cpu);assert_eq!(cpu.reg(0),0);
  state.issetugid(&mut cpu);assert_eq!(cpu.reg(0),1);
 }
}

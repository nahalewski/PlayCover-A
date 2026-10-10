/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Real persistent process state; successfully returning a call is not readiness.
use super::{A64Cpu,bridge::{GuestCall,ReturnValues},host_services::SelectedServices};
use std::{rc::{Rc,Weak},cell::{RefCell,Cell}};
// Process IDs live in this virtual kernel namespace, not Android/Linux's.
// Never reuse an ID while this host kernel instance remains alive.
static NEXT_PROCESS_ID:std::sync::atomic::AtomicU32=std::sync::atomic::AtomicU32::new(1);
pub(super) struct ProcessIdentity {
 id:u32,
 scheduler:Weak<RefCell<super::thread_scheduler_cpu::CpuScheduler>>,
}
impl ProcessIdentity {
 pub(super) fn new(scheduler:&Rc<RefCell<super::thread_scheduler_cpu::CpuScheduler>>)->Result<Self,String>{
  if scheduler.try_borrow().map_err(|_|"process identity scheduler borrowed")?.current().is_none(){return Err("process identity requires selected owned context".into());}
  let id=NEXT_PROCESS_ID.fetch_update(std::sync::atomic::Ordering::Relaxed,std::sync::atomic::Ordering::Relaxed,|id| {
   (id>0&&id<=i32::MAX as u32).then(||id+1)
  }).map_err(|_|"virtual process ID namespace exhausted")?;
  Ok(Self{id,scheduler:Rc::downgrade(scheduler)})
 }
 fn getpid(&self,cpu:&mut A64Cpu,scheduler:&Rc<RefCell<super::thread_scheduler_cpu::CpuScheduler>>)->Result<(),String>{
  let retained=self.scheduler.upgrade().ok_or("virtual process owner dropped")?;
  if !Rc::ptr_eq(&retained,scheduler)||scheduler.try_borrow().map_err(|_|"process scheduler borrowed")?.current().is_none(){return Err("getpid requires a selected context in its owning process".into());}
  cpu.set_reg(0,u64::from(self.id));cpu.set_pstate(cpu.pstate()&!(1<<29));Ok(())
 }
}
fn trap_number(cpu:&A64Cpu)->Result<i64,String> {
 let raw=cpu.reg(16);
 // mach_absolute_time uses MOVN W16,#2, zero-extending -3 into X16 (0xffff_fffd).
 // Only audited original stubs get the decoding; all other selectors retain their ABI.
 if raw==0xffff_fffd {
  let stub = match cpu.pc() {
   0x18095bbe4 => 0x18095bbdc,
   0x1c260d35c => 0x1c260d354,
   _ => return Ok(raw as i64),
  };
  let original=[0x50,0,0x80,0x12,1,0x10,0,0xd4,0xc0,3,0x5f,0xd6];
  if cpu.mapped_permissions(stub).is_none_or(|p|p&4==0)
   ||cpu.read_bytes(stub,12).is_none_or(|bytes|bytes!=original) {
   return Err("original32-bit Mach absolute-time selector stub differs".into());
  }
  return Ok(-3);
 }
 Ok(raw as i64)
}
#[derive(Clone)]
pub(super) struct SessionLease {scheduler:Weak<RefCell<super::thread_scheduler_cpu::CpuScheduler>>,owner:u64,live:Rc<Cell<bool>>}
impl SessionLease {
 pub(super) fn validate(&self,scheduler:&Rc<RefCell<super::thread_scheduler_cpu::CpuScheduler>>,owner:u64)->Result<(),String> {
  let retained=self.scheduler.upgrade().ok_or("session owner dropped")?;
  if !self.live.get()||owner!=self.owner||!Rc::ptr_eq(&retained,scheduler)||scheduler.try_borrow().map_err(|_|"session scheduler borrowed")?.current().map(|id|id.0)!=Some(owner) {return Err("session lease is not live for selected process owner".into());}Ok(())
 }
}
pub(super) struct ProcessState {
 pub identity:ProcessIdentity,
 pub ports:super::mach_identity::MachIdentity,
 pub vm:super::mach_vm::AnonymousVm,
 pub priorities:super::mach_host_info::HostPriorityPolicy,
 pub clock:super::mach_clock::SystemClock,
 pub semaphores:super::mach_semaphore::SemaphoreService,
 pub entropy:super::entropy_fd::EntropyFds,
 pub standard_fds:super::standard_fds::StandardFds,
 pub shared_memory:super::posix_shm::ShmNamespace,
 pub credentials:super::credentials::CredentialTaint,
 pub scheduler:Rc<RefCell<super::thread_scheduler_cpu::CpuScheduler>>,
 pub control:super::bsdthread_ctl::OwnedControl,
 pub registration:super::pthread_registration::ProcessRegistration,
 pub thread:u64,
 pub owner:super::thread_scheduler_cpu::ThreadId,
 pub trap_count:usize,
}
impl ProcessState {
 pub(super) fn trap_count(&self)->usize {self.trap_count}
 pub(super) fn dispatch(&mut self,cpu:&mut A64Cpu,immediate:u16)->Result<(),String> {
  self.dispatch_with_storage(cpu,immediate,None)
 }
 fn dispatch_with_storage(&mut self,cpu:&mut A64Cpu,immediate:u16,storage:Option<&super::thread_storage::ThreadStorage>)->Result<(),String>{
  let Self{identity,ports,vm,priorities,clock,semaphores,entropy,standard_fds,shared_memory,credentials,
   scheduler,control,registration,thread,owner,trap_count}=self;

            *trap_count += 1;
            if *trap_count > 256 {
                return Err("initializer supervisor trap budget exceeded".into());
            }
            let number = trap_number(cpu)?;
            let pc = cpu.pc();
            if immediate != 0x80 {
                return Err(format!(
                    "unsupported initializer SVC {immediate:#x} at {pc:#x}"
                ));
            }
            if number == 535 {
                // Hardware assistance is independent of the thread register.
                // XNU bsd/dev/arm/stubs.c: objc_bp_assist_cfg_np returns
                // KERN_FAILURE when the Apple hardware BP helper is absent.
                // This is a BSD errno return: preserve N/Z/V and set carry.
                // The interpreter has no corresponding hardware registers.
                cpu.set_reg(0,5);
                cpu.set_pstate(cpu.pstate()|(1<<29));
                echo!("[a64] ObjC hardware branch-predictor assistance unavailable; original Darwin BSD535 error5 returned at {pc:#x}");
                return Ok(());
            }
            if number==0x80000000 {
                storage.ok_or("platform thread-storage change requires retained execution session")?.adopt_original(cpu,scheduler,ports)?;
                echo!("[a64] original platform selector2 adopted verified static pthread TSD for actual retained owner; no pthread registration receipt");
                return Ok(());
            }
            if number == 327 {
                credentials.issetugid(cpu);
                echo!("[a64] issetugid reads isolated guest process credential taint at {pc:#x}; no host credential mapping");
                return Ok(());
            }
            if number==20 {
                identity.getpid(cpu,scheduler)?;
                echo!("[a64] getpid reads stable owning virtual process identity at {pc:#x}; no host PID or Mach-name substitution");
                return Ok(());
            }
            if number == 372 {
                super::thread_identity::self_id(cpu,scheduler,*owner)?;
                echo!("[a64] thread_selfid reads actual selected virtual kernel thread identity at {pc:#x}; no Mach port or pthread-address substitution");
                return Ok(());
            }
            if number == -3 || number == -89 {
                super::timebase::handle(cpu,number as i32)?;
                echo!("[a64] genuine shared CPU clock trap={number} pc={pc:#x}; no counter values logged");
                return Ok(());
            }
            if number == -15 {
                let args = std::array::from_fn(|i| cpu.reg(i));
                let result = vm.map(cpu, ports, args).map_err(|error| format!("initializer anonymous VM trap at {pc:#x}: {error}"))?;
                cpu.set_reg(0, u64::from(result));
                echo!("[a64] genuine anonymous Mach VM trap pc={pc:#x} args={args:x?} result={result:#x} mappings={}", vm.mappings().len());
                return Ok(());
            }
            if number == -10 {
                let args=std::array::from_fn(|i|cpu.reg(i));
                let status=vm.allocate(cpu,ports,args)?;
                cpu.set_reg(0,u64::from(status));
                echo!("[a64] genuine anonymous Mach allocation pc={pc:#x} args={args:x?} status={status:#x} mappings={}",vm.mappings().len());
                return Ok(());
            }
            if number == -12 {
                let args=std::array::from_fn(|i|cpu.reg(i));
                let status=vm.deallocate(cpu,ports,args)?;
                cpu.set_reg(0,u64::from(status));
                echo!("[a64] genuine Mach deallocation pc={pc:#x} args={args:x?} status={status:#x} mappings={}",vm.mappings().len());
                return Ok(());
            }
            if number == -24 {
                let args = std::array::from_fn(|i| cpu.reg(i));
                let result = super::mach_port_construct::construct(cpu, ports, args)
                    .map_err(|error| format!("initializer Mach port construct at {pc:#x}: {error}"))?;
                cpu.set_reg(0, u64::from(result));
                echo!("[a64] genuine Mach reply-port construct pc={pc:#x} args={args:x?} result={result:#x}");
                return Ok(());
            }
            if number == -18 {
                let task = cpu.reg(0);
                let name = cpu.reg(1);
                let result = match (u32::try_from(task), u32::try_from(name)) {
                    (Err(_), _) => 0x1000_0003, // MACH_SEND_INVALID_DEST.
                    (_, Err(_)) => 15, // KERN_INVALID_NAME.
                    (Ok(task), Ok(name)) => ports.deallocate(task, name),
                };
                cpu.set_reg(0, u64::from(result));
                echo!("[a64] genuine Mach send-right deallocate pc={pc:#x} task={task:#x} name={name:#x} result={result:#x}");
                return Ok(());
            }
            if number == -19 {
                let task = cpu.reg(0);
                let name = cpu.reg(1);
                let right = cpu.reg(2);
                let delta = cpu.reg(3) as i64 as i32;
                let result = match (u32::try_from(task), u32::try_from(name), u32::try_from(right)) {
                    (Err(_), _, _) => 0x1000_0003, // MACH_SEND_INVALID_DEST
                    (_, Err(_), _) => 15, // KERN_INVALID_NAME
                    (_, _, Err(_)) => 17, // KERN_INVALID_RIGHT
                    (Ok(task), Ok(name), Ok(right)) => ports.mod_refs(task, name, right, delta),
                };
                cpu.set_reg(0, u64::from(result));
                echo!("[a64] genuine Mach port mod_refs pc={pc:#x} task={task:#x} name={name:#x} right={right} delta={delta} result={result:#x}");
                return Ok(());
            }
            if number == -47 {
                let args = std::array::from_fn(|i| cpu.reg(i));
                let message_id = args[4] >> 32;
                let reply = match message_id {
                    200 => super::mach_host_info::reply(cpu, ports, args, priorities),
                    206 => clock.reply(cpu, ports, args),
                    225 => super::mach_atm::reply(cpu, ports, args),
                    3409 => ports.reply_task_special_port(cpu, args),
                    3410 => ports.reply_task_set_special_port(cpu, args),
                    3418 => semaphores.reply(cpu, ports, args),
                    8000 => super::restartable::reply(cpu,ports,scheduler,args),
                    _ => Err(format!("unsupported initializer Mach RPC {message_id}")),
                };
                match reply {
                    Ok(result) => {
                        cpu.set_reg(0, u64::from(result));
                        echo!("[a64] genuine bounded Mach RPC id={message_id} reply pc={pc:#x} args={args:x?} result={result:#x}");
                        return Ok(());
                    }
                    Err(error) => {
                        let details = capture_message(cpu, number, pc, args)
                            .unwrap_or_else(|capture_error| capture_error);
                        return Err(format!("{error}; {details}"));
                    }
                }
            }
            if number == 366 {
                let args: [u64; 6] = std::array::from_fn(|i| cpu.reg(i));
                if args[4]==40 {
                    if cpu.pc()!=0x18097c13c||storage.is_none_or(|state|!state.original_adopted()){return Err("original40-byte registration requires verified adopted legacy pthread owner".into());}
                    control.capability().validate(scheduler)?;let priority=scheduler.borrow().priority(*owner)?;
                    super::legacy_pthread_registration::register(cpu,registration,args,priority)?;
                    let features=registration.supported_features(control.capability(),scheduler)?;cpu.set_reg(0,u64::from(features));cpu.set_pstate(cpu.pstate()&!(1<<29));
                    echo!("[a64] genuine original40-byte BSD366 registration owner={owner:?} args={args:x?} capabilities={features:#x}; actual40-byte copyout, no workqueue creation receipt");return Ok(());
                }
                if args[4] != 56 {
                    return Err(format!("unsupported BSD366 registration size at {pc:#x}: args={args:x?}"));
                }
                let mut data = [0u8; 56];
                cpu.read_guest_into(args[3], &mut data)?;
                control.capability().validate(scheduler)?;
                let priority = scheduler.borrow().priority(*owner)?;
                let entrypoints = [args[0], args[1]];
                let executable = entrypoints.map(|entry| entry & 3 == 0 && cpu.mapped_permissions(entry).is_some_and(|p| p & 4 != 0));
                cpu.validate_guest_write(args[3], 56)?;
                registration.register_copyout(args, &data,
                    |entry| entrypoints.iter().zip(executable).any(|(&address, rx)| address == entry && rx),
                    priority, |address, response| cpu.write_guest_into(address, response))?;
                let features = registration.supported_features(control.capability(), scheduler)?;
                cpu.set_reg(0, u64::from(features));
                cpu.set_pstate(cpu.pstate() & !(1 << 29));
                echo!("[a64] genuine BSD366 process registration owner={owner:?} pc={pc:#x} args={args:x?} installed-control features={features:#x}; no workqueue/thread creation receipt");
                return Ok(());
            }
            if number == 398 || number == 5 {
                let args: [u64; 3] = std::array::from_fn(|i| cpu.reg(i));
                let path_addr = args[0];
                let path_bytes = if path_addr != 0 {
                    if let Some(bytes) = cpu.read_bytes(path_addr, 256) {
                        let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                        bytes[..len].to_vec()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                if path_bytes == b"/dev/urandom" {
                    entropy.open(cpu, args)?;
                    echo!("[a64] genuine owned entropy open pc={pc:#x} result={:#x}", cpu.reg(0));
                    return Ok(());
                }
                let path_str = String::from_utf8_lossy(&path_bytes);
                echo!("[a64] genuine BSD{number} open pc={pc:#x} path={path_str:?} flags={:#x} mode={:#x} -> ENOENT (2)", args[1], args[2]);
                cpu.set_reg(0, 2); // ENOENT
                cpu.set_pstate(cpu.pstate() | (1 << 29)); // carry flag set
                return Ok(());
            }
            if number == 54 {
                let args = std::array::from_fn(|i| cpu.reg(i));
                standard_fds.ioctl(cpu,args)?;
                echo!("[a64] genuine standard non-TTY stream ioctl pc={pc:#x} args={args:x?} result={:#x}",cpu.reg(0));
                return Ok(());
            }
            if number == 74 {
                let (address,len,protection)=super::mprotect::decode_args(cpu);
                let errno=super::mprotect::mprotect(cpu,address,len,protection)?;
                super::mprotect::complete_bsd(cpu,u64::from(errno));
                echo!("[a64] genuine mapped-memory mprotect pc={pc:#x} address={address:#x} len={len:#x} prot={protection:#x} errno={errno}");
                return Ok(());
            }
            if number == 75 {
                let (address,size,advice)=(cpu.reg(0),cpu.reg(1),cpu.reg(2));
                let errno=vm.advise(cpu,address,size,advice)?;
                super::mprotect::complete_bsd(cpu,u64::from(errno));
                echo!("[a64] genuine owned anonymous reusable-memory advice pc={pc:#x} address={address:#x} size={size:#x} advice={advice} errno={errno}; backing retained conservatively");
                return Ok(());
            }
            if number == 266 {
                let args: [u64; 3] = std::array::from_fn(|i| cpu.reg(i));
                shared_memory.open(cpu, args)?;
                echo!("[a64] genuine isolated POSIX-shm lookup pc={pc:#x} result={:#x}; no external Apple featureflag daemon or objects imported", cpu.reg(0));
                return Ok(());
            }
            if number == 396 {
                let args = std::array::from_fn(|i| cpu.reg(i));
                entropy.read(cpu, args)?;
                echo!("[a64] genuine owned entropy read pc={pc:#x} result={:#x}; bytes not logged", cpu.reg(0));
                return Ok(());
            }
            if number == 500 {
                let args = std::array::from_fn(|i| cpu.reg(i));
                entropy.getentropy(cpu, args)?;
                echo!("[a64] genuine OS getentropy pc={pc:#x} result={:#x}; bytes not logged", cpu.reg(0));
                return Ok(());
            }
            if number == 399 {
                let fd = cpu.reg(0);
                entropy.close(cpu, fd)?;
                echo!("[a64] genuine owned entropy close pc={pc:#x} fd={fd} result={:#x}", cpu.reg(0));
                return Ok(());
            }
            if number == 478 {
                let args = std::array::from_fn(|i| cpu.reg(i));
                control.call(cpu, registration, args)?;
                echo!("[a64] routed BSD478 owned thread control pc={pc:#x} args={args:x?} result={:#x}", cpu.reg(0));
                return Ok(());
            }
            if number == 202 {
                super::sysctl::sysctl(cpu)?;
                return Ok(());
            }
            if number == 336 {
                let callnum = cpu.reg(0) as i32;
                let pid = cpu.reg(1) as i32;
                let flavor = cpu.reg(2) as u32;
                let _arg = cpu.reg(3);
                let buffer = cpu.reg(4);
                let buffersize = cpu.reg(5) as i32;

                if callnum == 2 && flavor == 13 { // PROC_INFO_CALL_PIDINFO, PROC_PIDT_SHORTBSDINFO
                    if (buffersize as usize) < 64 {
                        cpu.set_reg(0, 22); // EINVAL
                        cpu.set_pstate(cpu.pstate() | (1 << 29));
                        return Ok(());
                    }
                    if cpu.validate_guest_write(buffer, 64).is_err() {
                        cpu.set_reg(0, 14); // EFAULT
                        cpu.set_pstate(cpu.pstate() | (1 << 29));
                        return Ok(());
                    }
                    let mut info = [0u8; 64];
                    let proc_id = identity.id;
                    info[0..4].copy_from_slice(&proc_id.to_le_bytes()); // pbsi_pid
                    info[4..8].copy_from_slice(&1u32.to_le_bytes());    // pbsi_ppid
                    info[8..12].copy_from_slice(&proc_id.to_le_bytes()); // pbsi_pgid
                    info[12..16].copy_from_slice(&2u32.to_le_bytes());   // pbsi_status (SRUN)
                    let comm = b"Terraria\0\0\0\0\0\0\0\0";
                    info[16..32].copy_from_slice(&comm[..16]);
                    info[32..36].copy_from_slice(&4u32.to_le_bytes());   // pbsi_flags (PROC_FLAG_LP64)
                    info[36..40].copy_from_slice(&501u32.to_le_bytes()); // pbsi_uid
                    info[40..44].copy_from_slice(&501u32.to_le_bytes()); // pbsi_gid
                    info[44..48].copy_from_slice(&501u32.to_le_bytes()); // pbsi_ruid
                    info[48..52].copy_from_slice(&501u32.to_le_bytes()); // pbsi_rgid
                    info[52..56].copy_from_slice(&501u32.to_le_bytes()); // pbsi_svuid
                    info[56..60].copy_from_slice(&501u32.to_le_bytes()); // pbsi_svgid
                    let _ = cpu.write_guest_into(buffer, &info);
                    cpu.set_reg(0, 64);
                    cpu.set_pstate(cpu.pstate() & !(1 << 29));
                    echo!("[a64] genuine BSD336 proc_info PIDINFO/SHORTBSDINFO pid={pid} -> 64 bytes written");
                    return Ok(());
                }
                cpu.set_reg(0, 22); // EINVAL
                cpu.set_pstate(cpu.pstate() | (1 << 29));
                echo!("[a64] unsupported BSD336 proc_info callnum={callnum} flavor={flavor}");
                return Ok(());
            }
            if number == 220 {
                let path_addr = cpu.reg(0);
                let path = if path_addr != 0 {
                    if let Some(bytes) = cpu.read_bytes(path_addr, 256) {
                        let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                        String::from_utf8_lossy(&bytes[..len]).to_string()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                cpu.set_reg(0, 2); // ENOENT
                cpu.set_pstate(cpu.pstate() | (1 << 29)); // set carry flag
                echo!("[a64] genuine BSD220 getattrlist pc={pc:#x} path={path:?} -> ENOENT (2)");
                return Ok(());
            }
            if number == 116 {
                let tp = cpu.reg(0);
                let tzp = cpu.reg(1);
                let mach_time_ptr = cpu.reg(2);

                for (address, size) in [(tp, 16), (tzp, 8), (mach_time_ptr, 8)] {
                    if address != 0 && cpu.validate_guest_write(address, size).is_err() {
                        cpu.set_reg(0, 14); // EFAULT
                        cpu.set_pstate(cpu.pstate() | (1 << 29));
                        return Ok(());
                    }
                }

                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                let mut timeval = [0u8; 16];
                timeval[..8].copy_from_slice(&(now.as_secs() as i64).to_le_bytes());
                timeval[8..12].copy_from_slice(&(now.subsec_micros() as i32).to_le_bytes());

                if tp != 0 {
                    let _ = cpu.write_guest_into(tp, &timeval);
                }
                if tzp != 0 {
                    let _ = cpu.write_guest_into(tzp, &[0u8; 8]);
                }
                if mach_time_ptr != 0 {
                    let ticks = (now.as_nanos().min(u64::MAX as u128) as u64).to_le_bytes();
                    let _ = cpu.write_guest_into(mach_time_ptr, &ticks);
                }

                cpu.set_reg(0, 0);
                cpu.set_pstate(cpu.pstate() & !(1 << 29));
                echo!("[a64] genuine gettimeofday pc={pc:#x} tp={tp:#x} sec={}", now.as_secs());
                return Ok(());
            }
            if number == -31 {
                let args: [u64; 8] = std::array::from_fn(|i| cpu.reg(i));
                let result=super::legacy_mach_msg::reply(cpu,ports,args,priorities,clock,semaphores).map_err(|error|format!("{error}; {}",capture_message(cpu,number,pc,args).unwrap_or_else(|error|error)))?;
                cpu.set_reg(0,u64::from(result));
                echo!("[a64] genuine original mach_msg owned host service reply pc={pc:#x} result={result:#x}; owned ordinary reply receive retained");
                return Ok(());
            }
            if number == 33 {
                let path_addr = cpu.reg(0);
                let mode = cpu.reg(1);
                let path_bytes = if path_addr != 0 {
                    if let Some(bytes) = cpu.read_bytes(path_addr, 256) {
                        let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                        bytes[..len].to_vec()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                let path_str = String::from_utf8_lossy(&path_bytes);
                echo!("[a64] genuine BSD33 access pc={pc:#x} path={path_str:?} mode={mode:#x} -> ENOENT (2)");
                cpu.set_reg(0, 2); // ENOENT
                cpu.set_pstate(cpu.pstate() | (1 << 29)); // carry flag set
                return Ok(());
            }
            if number == 381 {
                let policy_addr = cpu.reg(0);
                let call = cpu.reg(1) as u32;
                let arg = cpu.reg(2);
                let policy_bytes = if policy_addr != 0 {
                    if let Some(bytes) = cpu.read_bytes(policy_addr, 64) {
                        let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                        bytes[..len].to_vec()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                let policy_str = String::from_utf8_lossy(&policy_bytes);
                echo!("[a64] genuine BSD381 __mac_syscall pc={pc:#x} policy={policy_str:?} call={call} arg={arg:#x} -> ENOENT (2)");
                cpu.set_reg(0, 2); // ENOENT (not sandboxed / no container)
                cpu.set_pstate(cpu.pstate() | (1 << 29)); // carry flag set
                return Ok(());
            }
            if number == 169 {
                let pid = cpu.reg(0) as i32;
                let ops = cpu.reg(1) as u32;
                let useraddr = cpu.reg(2);
                let usersize = cpu.reg(3) as usize;

                if ops == 0 {
                    // CS_OPS_STATUS
                    if usersize < 4 {
                        cpu.set_reg(0, 22); // EINVAL
                        cpu.set_pstate(cpu.pstate() | (1 << 29));
                        return Ok(());
                    }
                    let status: u32 = 0x2000_0001; // CS_VALID | CS_SIGNED
                    if cpu.write_guest_into(useraddr, &status.to_le_bytes()).is_err() {
                        cpu.set_reg(0, 14); // EFAULT
                        cpu.set_pstate(cpu.pstate() | (1 << 29));
                        return Ok(());
                    }
                    cpu.set_reg(0, 0);
                    cpu.set_pstate(cpu.pstate() & !(1 << 29)); // carry flag clear
                    echo!("[a64] genuine BSD169 csops CS_OPS_STATUS pid={pid} status={status:#x} -> 0");
                    return Ok(());
                }
                cpu.set_reg(0, 22); // EINVAL
                cpu.set_pstate(cpu.pstate() | (1 << 29));
                echo!("[a64] unsupported BSD169 csops ops={ops} pid={pid}");
                return Ok(());
            }
            if number > 0 {
                return Err(format!(
                    "unsupported BSD syscall {number} at {pc:#x}; lr={:#x} sp={:#x} x0..x5={:x?}",
                    cpu.reg(30),
                    cpu.sp(),
                    std::array::from_fn::<_, 6, _>(|i| cpu.reg(i))
                ));
            }
            let name = ports.trap_for_thread(number, *thread).map_err(|error| {
                format!(
                    "initializer trap {number} at {pc:#x}: {error}; lr={:#x} sp={:#x} x0..x5={:x?}",
                    cpu.reg(30),
                    cpu.sp(),
                    std::array::from_fn::<_, 6, _>(|i| cpu.reg(i))
                )
            })?;
            // These are port-name returns, not kern_return_t: preserve NZCV.
            cpu.set_reg(0, u64::from(name));
            echo!("[a64] genuine Mach identity trap={number} pc={pc:#x} name={name:#x}");
            Ok(())
 }
}
pub(super) struct ExecutionSession {process:ProcessState,tsd:u64,quarantined:bool,calls_returned:u64,lease:SessionLease,thread_storage:Rc<super::thread_storage::ThreadStorage>}
impl ExecutionSession {
 pub(super) fn from_prepared(cpu:&A64Cpu,process:ProcessState,tsd:u64)->Result<Self,String> {
  process.control.capability().validate(&process.scheduler)?;
  if tsd==0||cpu.tpidrro_el0()!=tsd||process.scheduler.try_borrow().map_err(|_|"session scheduler is borrowed")?.current()!=Some(process.owner)||process.owner.0!=process.thread {
   return Err("persistent session requires actual selected thread/TSD context".into());}
  cpu.read_guest_into(tsd,&mut[0;8])?;
  let lease=SessionLease{scheduler:Rc::downgrade(&process.scheduler),owner:process.owner.0,live:Rc::new(Cell::new(true))};
  let thread_storage=Rc::new(super::thread_storage::ThreadStorage::new(process.owner,tsd));
  Ok(Self{process,tsd,quarantined:false,calls_returned:0,lease,thread_storage})
 }
 pub(super) fn call(&mut self,cpu:&mut A64Cpu,services:&mut SelectedServices,call:&GuestCall,budget:u64)->Result<ReturnValues,String> {
  services.set_thread_storage(self.thread_storage.clone());
  self.call_using(cpu,call,budget,|cpu,call,budget,handler|services.call_with_supervisor_handler(cpu,call,budget,handler))
 }
 pub(super) fn call_initializer(&mut self,cpu:&mut A64Cpu,services:&mut SelectedServices,call:&GuestCall,permit:&super::initializer_budget::InitializerBudget)->Result<ReturnValues,String>{
  services.set_thread_storage(self.thread_storage.clone());
  self.call_using(cpu,call,permit.ticks(),|cpu,call,_budget,handler|services.call_initializer_with_supervisor(cpu,call,permit,handler))
 }
 pub(super) fn call_legacy_initializer(&mut self,cpu:&mut A64Cpu,services:&mut SelectedServices,call:&GuestCall,permit:&super::legacy_initializer_budget::LegacyBudget)->Result<ReturnValues,String>{
  services.set_thread_storage(self.thread_storage.clone());
  self.call_using(cpu,call,permit.ticks(),|cpu,call,_budget,handler|services.call_legacy_initializer_with_supervisor(cpu,call,permit,handler))
 }
 fn call_using<F>(&mut self,cpu:&mut A64Cpu,call:&GuestCall,budget:u64,mut run:F)->Result<ReturnValues,String>
 where F:FnMut(&mut A64Cpu,&GuestCall,u64,&mut dyn FnMut(&mut A64Cpu,u16)->Result<(),String>)->Result<ReturnValues,String> {
  if self.quarantined {return Err("partial guest session is quarantined; replay denied".into());}
  let owner_matches=self.process.scheduler.try_borrow().map(|scheduler|scheduler.current()==Some(self.process.owner)).unwrap_or(false);
  if cpu.tpidrro_el0()!=self.tsd||!owner_matches {
   self.quarantined=true;self.lease.live.set(false);return Err("persistent session CPU/TSD ownership changed".into());}
  self.process.trap_count=0;
  let process=&mut self.process;let storage=self.thread_storage.clone();
  let result=run(cpu,call,budget,&mut|cpu,immediate|process.dispatch_with_storage(cpu,immediate,Some(&storage)));
  self.tsd=self.thread_storage.current();
  match result {Ok(value)=>{
    let same_owner=self.process.scheduler.try_borrow().map(|s|s.current()==Some(self.process.owner)).unwrap_or(false);
    if cpu.tpidrro_el0()!=self.tsd||!same_owner {self.quarantined=true;self.lease.live.set(false);return Err("CPU/TSD ownership changed while guest call ran".into());}
    self.calls_returned=self.calls_returned.checked_add(1).ok_or("session call count overflow")?;Ok(value)},
   Err(error)=>{self.quarantined=true;self.lease.live.set(false);Err(error)}}
 }
 pub(super) fn lease(&self)->SessionLease {self.lease.clone()}
 pub(super) fn enable_legacy_thread_storage(&self){self.thread_storage.enable_legacy();}
 pub(super) fn process(&self)->&ProcessState {&self.process}
 pub(super) fn is_quarantined(&self)->bool {self.quarantined}
}
impl Drop for ExecutionSession {fn drop(&mut self){self.lease.live.set(false);}}
#[cfg(test)]mod tests {
 use super::*;
 fn fixture()->(A64Cpu,super::super::bridge::GuestBridge,ExecutionSession) {
  let mut cpu=A64Cpu::new_sparse();cpu.map_zeroed(0x10000,4096,5).unwrap();cpu.map_zeroed(0x20000,4096,3).unwrap();cpu.map_zeroed(0x90000,4096,3).unwrap();
  for (address,number) in [(0x10000,-28i64),(0x10020,-10),(0x10040,-9999)] {
   for (i,word) in [0x92800000|(((!number as u64&0xffff)as u32)<<5)|16,0xd4001001,0xd65f03c0].into_iter().enumerate(){cpu.try_write_bytes(address+i as u64*4,&word.to_le_bytes()).unwrap();}
  }
  let tsd=0x200e0;cpu.set_tpidrro_el0(tsd);cpu.set_pc(0x10000);cpu.set_sp(0x91000);
  let mut scheduler=super::super::thread_scheduler_cpu::CpuScheduler::default();let owner=scheduler.adopt(&cpu,(0x90000,0x91000)).unwrap();scheduler.select(&mut cpu).unwrap();
  let scheduler=Rc::new(RefCell::new(scheduler));let control=super::super::bsdthread_ctl::OwnedControl::install(scheduler.clone()).unwrap();
  let mut ports=super::super::mach_identity::MachIdentity::new(16).unwrap();ports.register_thread(owner.0).unwrap();
  let process=ProcessState {identity:ProcessIdentity::new(&scheduler).unwrap(),ports,vm:super::super::mach_vm::AnonymousVm::new(0x20_0000_0000,0x20_0100_0000).unwrap(),
   priorities:super::super::mach_host_info::HostPriorityPolicy::virtual_darwin(),clock:super::super::mach_clock::SystemClock::new(),semaphores:super::super::mach_semaphore::SemaphoreService::new(),entropy:super::super::entropy_fd::EntropyFds::new(),standard_fds:super::super::standard_fds::StandardFds::new(),shared_memory:super::super::posix_shm::ShmNamespace::new_isolated(),credentials:super::super::credentials::CredentialTaint::isolated_unprivileged(),scheduler,control,registration:super::super::pthread_registration::ProcessRegistration::default(),thread:owner.0,owner,trap_count:0};
  let session=ExecutionSession::from_prepared(&cpu,process,tsd).unwrap();let mut bridge=super::super::bridge::GuestBridge::map(&mut cpu,0x60000).unwrap();bridge.set_thread_storage(session.thread_storage.clone());(cpu,bridge,session)
 }
 fn original_setself_fixture(cpu:&mut A64Cpu,owner:u64)->u64{
  let record=0x1b3288b40u64;let tsd=record+224;cpu.map_zeroed(record&!4095,8192,3).unwrap();cpu.map_zeroed(0x18097f000,4096,5).unwrap();
  for(i,word)in[0xd2800043u32,0xd2b00010,0xd4001001,0xd65f03c0].iter().enumerate(){cpu.try_write_bytes(0x18097f534+i as u64*4,&word.to_le_bytes()).unwrap();}
  cpu.write_guest_into(record+0xd8,&owner.to_le_bytes()).unwrap();cpu.write_guest_into(tsd,&record.to_le_bytes()).unwrap();cpu.write_guest_into(tsd+8,&(record+0x48).to_le_bytes()).unwrap();tsd
 }
 #[test]fn guest_getpid_is_stable_across_calls_unique_across_sessions_and_owned(){
  let(mut cpu,mut bridge,mut session)=fixture();
  // MOV X16,#20; SVC #0x80; RET -- authentic BSD ABI, no host PID call.
  for(i,word)in[0xd2800290u32,0xd4001001,0xd65f03c0].iter().enumerate(){cpu.try_write_bytes(0x10000+i as u64*4,&word.to_le_bytes()).unwrap();}
  cpu.set_pstate(0xb0000000);let mut ids=Vec::new();
  for _ in 0..2 {
   let returned=session.call_using(&mut cpu,&GuestCall{entry:0x10000,..Default::default()},1000,
    |cpu,call,budget,handler|bridge.call_with_supervisor_handler(cpu,call,budget,handler)).unwrap();
   ids.push(returned.integers[0]);
   assert_eq!(cpu.pstate(),0xb0000000);assert_eq!(cpu.tpidrro_el0(),0x200e0);
  }
  assert_eq!(ids[0],ids[1]);assert!(ids[0]>0&&ids[0]<=i32::MAX as u64);
  let(_,_,other)=fixture();assert_ne!(session.process.identity.id,other.process.identity.id);
  cpu.set_reg(0,999);cpu.set_pstate(0xf0000000);
  assert!(session.process.identity.getpid(&mut cpu,&other.process.scheduler).is_err());
  assert_eq!(cpu.reg(0),999);assert_eq!(cpu.pstate(),0xf0000000);
  cpu.set_reg(16,20);session.process.dispatch(&mut cpu,0x80).unwrap();
  assert_eq!(cpu.reg(0),ids[0]);assert_eq!(cpu.pstate(),0xd0000000);
  session.process.scheduler.borrow_mut().yield_current(&cpu).unwrap();
  cpu.set_reg(0,777);assert!(session.process.dispatch(&mut cpu,0x80).is_err());assert_eq!(cpu.reg(0),777);
 }
 #[test]fn original_w16_clock_stub_uses_real_counter_without_reclassifying_platform_or_bsd(){
  let(mut cpu,mut bridge,mut session)=fixture();cpu.map_zeroed(0x18095b000,4096,5).unwrap();
  let original=[0x50,0,0x80,0x12,1,0x10,0,0xd4,0xc0,3,0x5f,0xd6];
  cpu.try_write_bytes(0x18095bbdc,&original).unwrap();cpu.set_pstate(0xb0000000);
  let before=super::super::timebase::absolute_time(&cpu);
  let result=session.call_using(&mut cpu,&GuestCall{entry:0x18095bbdc,..Default::default()},1000,
   |cpu,call,budget,handler|bridge.call_with_supervisor_handler(cpu,call,budget,handler)).unwrap();
  let after=super::super::timebase::absolute_time(&cpu);
  assert!(result.integers[0]>=before&&result.integers[0]<=after);assert_eq!(session.process.trap_count(),1);
  assert_eq!(cpu.pstate(),0xb0000000);assert_eq!(cpu.tpidrro_el0(),0x200e0);assert!(!session.is_quarantined());
  for(raw,expected)in[(0x80000000,0x80000000i64),(372,372),(0xfffffffffffffffd,-3),(0x200000003,0x200000003)]{
   cpu.set_reg(16,raw);assert_eq!(trap_number(&cpu).unwrap(),expected);
  }
  cpu.set_reg(16,0xfffffffd);cpu.set_pc(0x12340);assert_eq!(trap_number(&cpu).unwrap(),0xfffffffd);
  cpu.set_pc(0x18095bbe4);assert_eq!(trap_number(&cpu).unwrap(),-3);
  cpu.try_write_bytes(0x18095bbdc,&0xd503201fu32.to_le_bytes()).unwrap();assert!(trap_number(&cpu).is_err());
 }
 #[test]fn reusable_advice_dispatch_returns_real_bsd_flags_for_owned_range_and_hole(){
  let(mut cpu,_,mut session)=fixture();let task=session.process.ports.trap(-28).unwrap()as u64;
  session.process.vm.allocate(&mut cpu,&session.process.ports,[task,0x20000,0x4000,1]).unwrap();
  let base=cpu.read_u64(0x20000).unwrap();cpu.write_guest_into(base+0x182,&[23;12]).unwrap();
  cpu.set_reg(16,75);cpu.set_reg(0,base+0x182);cpu.set_reg(1,12);cpu.set_reg(2,7);cpu.set_pstate(0xb0000000);
  session.process.dispatch(&mut cpu,0x80).unwrap();assert_eq!(cpu.reg(0),0);assert_eq!(cpu.pstate(),0x90000000);
  assert_eq!(cpu.read_bytes(base+0x182,12).unwrap(),&[23;12]);
  cpu.set_reg(0,base+0x4000);session.process.dispatch(&mut cpu,0x80).unwrap();
  assert_eq!(cpu.reg(0),22);assert_eq!(cpu.pstate(),0xb0000000);
 }
 #[test]fn approved_original_tsd_survives_nested_unwind_without_changing_other_context(){
  let(mut cpu,mut bridge,mut session)=fixture();session.enable_legacy_thread_storage();let tsd=original_setself_fixture(&mut cpu,session.process.owner.0);
  let completed=Rc::new(Cell::new(false));let done=completed.clone();let service=bridge.register_service(&mut cpu,"nested_original_setself",move|frame|{
   let done=done.clone();frame.request_guest_call(GuestCall{entry:0x18097f534,integers:vec![tsd],..Default::default()},move|result|{result?;done.set(true);Ok(())})?;Ok(ReturnValues::integer(0))
  }).unwrap();
  cpu.set_vector(1,[0x55,0xaa]);let pc=cpu.pc();let sp=cpu.sp();let lr=cpu.reg(A64Cpu::LR);
  session.call_using(&mut cpu,&GuestCall{entry:service.guest_address(),..Default::default()},1000,|cpu,call,budget,handler|bridge.call_with_supervisor_handler(cpu,call,budget,handler)).unwrap();
  assert!(completed.get());assert_eq!(cpu.tpidrro_el0(),tsd);assert_eq!(session.tsd,tsd);assert_eq!(cpu.pc(),pc);assert_eq!(cpu.sp(),sp);assert_eq!(cpu.reg(A64Cpu::LR),lr);assert_eq!(cpu.vector(1),[0x55,0xaa]);assert_eq!(cpu.read_u64(tsd+24),Some(0));
  session.call_using(&mut cpu,&GuestCall{entry:0x10000,..Default::default()},1000,|cpu,call,budget,handler|bridge.call_with_supervisor_handler(cpu,call,budget,handler)).unwrap();assert_eq!(session.calls_returned,2);assert!(!session.is_quarantined());assert_eq!(cpu.tpidrro_el0(),tsd);
 }
 #[test]fn invalid_original_tsd_record_quarantines_without_approved_effect(){
  let(mut cpu,mut bridge,mut session)=fixture();session.enable_legacy_thread_storage();let tsd=original_setself_fixture(&mut cpu,session.process.owner.0);cpu.write_guest_into(tsd+8,&0x123u64.to_le_bytes()).unwrap();
  let pc=cpu.pc();let sp=cpu.sp();let result=session.call_using(&mut cpu,&GuestCall{entry:0x18097f534,integers:vec![tsd],..Default::default()},1000,|cpu,call,budget,handler|bridge.call_with_supervisor_handler(cpu,call,budget,handler));
  assert!(result.unwrap_err().contains("metadata differs"));assert!(session.is_quarantined());assert_eq!(cpu.tpidrro_el0(),0x200e0);assert_eq!(session.tsd,0x200e0);assert_eq!(cpu.pc(),pc);assert_eq!(cpu.sp(),sp);
 }
 #[test]fn real_calls_retain_task_right_vm_and_tsd_then_quarantine_failure() {
  let(mut cpu,mut bridge,mut session)=fixture();
  let task=session.call_using(&mut cpu,&GuestCall{entry:0x10000,..Default::default()},1000,|cpu,call,budget,handler|bridge.call_with_supervisor_handler(cpu,call,budget,handler)).unwrap().integers[0];
  assert!(session.process.ports.right(task as u32).is_some());assert_eq!(cpu.tpidrro_el0(),0x200e0);
  let allocation=GuestCall{entry:0x10020,integers:vec![task,0x20000,0x4000,1],..Default::default()};
  assert_eq!(session.call_using(&mut cpu,&allocation,1000,|cpu,call,budget,handler|bridge.call_with_supervisor_handler(cpu,call,budget,handler)).unwrap().integers[0],0);
  let mapped=cpu.read_u64(0x20000).unwrap();assert_eq!(session.process.vm.mappings()[0].address,mapped);cpu.write_guest_into(mapped,&[17]).unwrap();
  assert_eq!(session.calls_returned,2);assert_eq!(cpu.tpidrro_el0(),0x200e0);
  assert!(session.call_using(&mut cpu,&GuestCall{entry:0x10040,..Default::default()},1000,|cpu,call,budget,handler|bridge.call_with_supervisor_handler(cpu,call,budget,handler)).is_err());
  assert!(session.is_quarantined());assert_eq!(cpu.read_bytes(mapped,1).unwrap(),&[17]);
  assert!(session.call_using(&mut cpu,&allocation,1000,|_,_,_,_|panic!("quarantined session must not replay guest")).unwrap_err().contains("replay denied"));
 }
 #[test]fn changed_owner_or_tsd_never_executes_guest() {
  let(mut cpu,_,mut session)=fixture();cpu.set_tpidrro_el0(0);
  assert!(session.call_using(&mut cpu,&GuestCall::default(),10,|_,_,_,_|panic!()).is_err());assert!(session.is_quarantined());
  let(mut cpu,_,mut session)=fixture();session.process.scheduler.borrow_mut().yield_current(&cpu).unwrap();
  assert!(session.call_using(&mut cpu,&GuestCall::default(),10,|_,_,_,_|panic!()).is_err());assert!(session.is_quarantined());
 }
 #[test]fn owner_changed_during_call_is_not_counted_successful() {
  let(mut cpu,_,mut session)=fixture();let scheduler=session.process.scheduler.clone();
  assert!(session.call_using(&mut cpu,&GuestCall::default(),10,move|cpu,_,_,_|{scheduler.borrow_mut().yield_current(cpu)?;Ok(ReturnValues::integer(0))}).is_err());
  assert!(session.is_quarantined());assert_eq!(session.calls_returned,0);
 }
 #[test]fn lease_requires_same_live_selected_process_and_revokes_on_drop_or_failure() {
  let(mut cpu,_,mut session)=fixture();let lease=session.lease();let scheduler=session.process.scheduler.clone();let owner=session.process.owner.0;
  lease.validate(&scheduler,owner).unwrap();
  assert!(lease.validate(&Rc::new(RefCell::new(super::super::thread_scheduler_cpu::CpuScheduler::default())),owner).is_err());
  assert!(lease.validate(&scheduler,owner+1).is_err());
  cpu.set_tpidrro_el0(0);assert!(session.call_using(&mut cpu,&GuestCall::default(),1,|_,_,_,_|panic!()).is_err());
  assert!(lease.validate(&scheduler,owner).is_err());
  let(_,_,session)=fixture();let lease=session.lease();let scheduler=session.process.scheduler.clone();let owner=session.process.owner.0;drop(session);
  assert!(lease.validate(&scheduler,owner).is_err());
 }
 #[test]fn objc_hardware_assist_absence_is_real_darwin_error_and_preserves_other_state() {
  let(mut cpu,_,mut session)=fixture();cpu.set_reg(16,535);cpu.set_reg(0,0x1800ba400);cpu.set_reg(1,0x80000018001c103c);cpu.set_reg(2,0x203);cpu.set_pstate(0x90000000);
  let before=cpu.read_bytes(0x20000,16).unwrap().to_vec();
  session.process.dispatch(&mut cpu,0x80).unwrap();
  assert_eq!(cpu.reg(0),5);assert_eq!(cpu.pstate(),0xb0000000);assert_eq!(cpu.reg(1),0x80000018001c103c);assert_eq!(cpu.reg(2),0x203);
  assert_eq!(cpu.read_bytes(0x20000,16).unwrap(),before.as_slice());assert_eq!(cpu.tpidrro_el0(),0x200e0);
 }
 #[test]fn leased_initializer_returns_only_after_real_callbacks_and_retains_lazy_storage(){
  let(mut cpu,mut bridge,mut session)=fixture();
  let object=super::super::dyld_helpers::HELPER_OBJECT;
  cpu.map_zeroed(object&!4095,4096,3).unwrap();cpu.map_zeroed(0x40000,0x4000,3).unwrap();
  let thunk=0x1a6c7d8d0u64;cpu.map_zeroed(thunk&!4095,4096,5).unwrap();
  let lazy=super::super::tlv_lazy::ENTRY;cpu.map_zeroed(lazy&!4095,4096,5).unwrap();cpu.try_write_bytes(lazy,&[0xe1,3,0,0xaa,0x88,0x85,0x1d,0xb0,0,0xc5,0x41,0xf9,8,0,0x40,0xf9,2,5,0x40,0xf9,0x40,0,0x1f,0xd6]).unwrap();
  let mut code=|address:u64,words:&[u32]|{for(index,word)in words.iter().enumerate(){cpu.try_write_bytes(address+index as u64*4,&word.to_le_bytes()).unwrap();}};
  code(0x10080,&[0x528000c0,0xd65f03c0]);code(0x100a0,&[0xd65f03c0]);
  code(0x10100,&[0xf9400409,0x91000529,0xf9000409,0xf9000029,0x52800000,0xd65f03c0]);
  code(0x10200,&[0xd2820000,0xf2a00080,0xd65f03c0]);code(0x10300,&[0xf9000802,0x52800000,0xd65f03c0]);code(0x10400,&[0xf9400800,0xd65f03c0]);
  let mut thunk_code=Vec::new();for index in 0..4u32{thunk_code.push((if index==0{0xd2800000}else{0xf2800000})|(index<<21)|((((thunk>>(index*16))&0xffff)as u32)<<5));}thunk_code.push(0xd65f03c0);code(0x10500,&thunk_code);drop(code);
  let mut functions=[0x100a0u64;22];functions[0]=0x10080;functions[1]=0x10200;functions[6]=0x10100;functions[7]=0x10100;functions[8]=0x10400;functions[9]=0x10300;functions[18]=0x10500;
  let vtable=0x40080u64;cpu.try_write_bytes(object,&vtable.to_le_bytes()).unwrap();cpu.try_write_bytes(object+8,&256u64.to_le_bytes()).unwrap();for(index,function)in functions.iter().enumerate(){cpu.try_write_bytes(vtable+index as u64*8,&function.to_le_bytes()).unwrap();}
  cpu.try_write_bytes(0x42000,&0x100a0u64.to_le_bytes()).unwrap();cpu.try_write_bytes(0x42010,&1u64.to_le_bytes()).unwrap();
  let plan=super::super::tlv::Plan{header:0x50000,template:vec![0xaa,0xbb,0],alignment:8,descriptors:vec![super::super::tlv::Descriptor{slot:0x42000,offset:1,thunk:0x100a0,key:0}],initializers:vec![],preallocated_key:None};
  let entry=super::super::tlv_bootstrap::install_owned(&mut cpu,&mut bridge,super::super::dyld_helpers::Helpers{object,vtable,functions},0x43000,vec![plan],session.process.owner.0,session.process.scheduler.clone(),session.lease()).unwrap();
  let returned=session.call_using(&mut cpu,&GuestCall{entry,..Default::default()},2000,|cpu,call,budget,handler|bridge.call_with_supervisor_handler(cpu,call,budget,handler)).unwrap();assert_eq!(returned.integers[0],0);
  assert_eq!(cpu.read_u64(0x43000),Some(257));assert_eq!(cpu.read_u64(0x43008),Some(258));assert_eq!(cpu.read_u64(0x42000),Some(thunk));assert_eq!(cpu.read_u64(0x42008),Some(259));assert_eq!(cpu.read_bytes(0x41000,3).unwrap(),&[0xaa,0xbb,0]);
  cpu.write_guest_into(0x41001,&[0x77]).unwrap();
  let base=session.call_using(&mut cpu,&GuestCall{entry:lazy,integers:vec![259],..Default::default()},1000,|cpu,call,budget,handler|bridge.call_with_supervisor_handler(cpu,call,budget,handler)).unwrap().integers[0];
  assert_eq!(base,0x41000);assert_eq!(cpu.read_bytes(base+1,1).unwrap(),&[0x77]);assert_eq!(session.calls_returned,2);assert_eq!(cpu.tpidrro_el0(),0x200e0);assert!(!session.is_quarantined());
 }
 #[test]fn csops_status_returns_codesign_flags_and_clears_carry(){
  let(mut cpu,_,mut session)=fixture();
  cpu.set_reg(16,169);
  cpu.set_reg(0,1);
  cpu.set_reg(1,0);
  cpu.set_reg(2,0x20000);
  cpu.set_reg(3,4);
  cpu.set_pstate(0xb0000000);
  session.process.dispatch(&mut cpu,0x80).unwrap();
  assert_eq!(cpu.reg(0),0);
  assert_eq!(cpu.pstate(),0x90000000);
  assert_eq!(cpu.read_bytes(0x20000,4).unwrap(),&0x20000001u32.to_le_bytes());
 }
}
fn capture_message(cpu: &A64Cpu, number: i64, pc: u64, args: [u64; 8]) -> Result<String, String> {
    let summary = format!(
        "unsupported Mach message trap {number} at {pc:#x}; lr={:#x} sp={:#x} x0..x7={args:x?}",
        cpu.reg(30),
        cpu.sp()
    );
    if number == -47 && args[1] & 0x1_0000_0000 != 0 {
        return Ok(format!(
            "{summary}; vector message ABI not inspected; no message delivered"
        ));
    }
    if args[1] & 1 == 0 {
        return Ok(format!(
            "{summary}; receive-only call has no send request; no message delivered"
        ));
    }
    let size = if number == -47 {
        args[2] >> 32
    } else {
        args[2]
    };
    if !(24..=4096).contains(&size) {
        return Ok(format!(
            "{summary}; scalar send size {size} outside bounded capture; no message delivered"
        ));
    }
    let mut bytes = vec![0u8; size.min(128) as usize];
    cpu.read_guest_into(args[0], &mut bytes)
        .map_err(|error| format!("{summary}; bounded message read failed: {error}"))?;
    Ok(format!(
        "{summary}; declared_size={size} message={bytes:02x?}; no message delivered"
    ))
}



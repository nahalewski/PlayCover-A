/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Isolated genuine libSystem initializer execution. Never issues load receipts.
use super::{bridge::GuestCall, cache::CachePlan, host_services::SelectedServices, A64Cpu};

/// Dyld's five-argument initializer ABI, with actual entry argv and genuine
/// pointer cells for the four ProgramVars fields written by libSystem.
pub(super) fn arguments(
    cpu: &mut A64Cpu,
    base: u64,
    main_header: u64,
    executable: &str,
    extra_apple: &[String],
) -> Result<Vec<u64>, String> {
    if base & 4095 != 0
        || executable.is_empty()
        || executable.len() > 2048
        || executable.as_bytes().contains(&0)
    {
        return Err("invalid initializer argument arena/path".into());
    }
    if extra_apple.len() > 2
        || extra_apple
            .iter()
            .any(|entry| entry.len() > 512 || entry.as_bytes().contains(&0))
    {
        return Err("initializer apple entries exceed bounded arena".into());
    }
    let argc = cpu.reg(0);
    let argv = cpu.reg(1);
    if argc == 0 || argc > 256 {
        return Err("initializer requires bounded actual main argc".into());
    }
    cpu.read_guest_into(main_header, &mut [0u8; 32])?;
    let mut actual_argv = vec![0u8; (argc as usize + 1) * 8];
    cpu.read_guest_into(argv, &mut actual_argv)?;
    if cpu.read_u64(argv + argc * 8) != Some(0) {
        return Err("actual initializer argv lacks null terminator".into());
    }
    cpu.map_zeroed(base, 4096, 3)?;
    let env = base;
    let apple = base + 16;
    let vars = base + 64;
    let argc_cell = base + 128;
    let argv_cell = base + 136;
    let environ_cell = base + 144;
    let progname_cell = base + 152;
    let path = base + 256;
    let text = format!("executable_path={executable}\0");
    cpu.write_guest_into(path, text.as_bytes())?;
    cpu.write_guest_into(apple, &path.to_le_bytes())?;
    let mut next = path + text.len() as u64;
    for (index, entry) in extra_apple.iter().enumerate() {
        let mut bytes = entry.as_bytes().to_vec();
        bytes.push(0);
        cpu.write_guest_into(apple + (index as u64 + 1) * 8, &next.to_le_bytes())?;
        cpu.write_guest_into(next, &bytes)?;
        next += bytes.len() as u64;
    }
    cpu.write_guest_into(argc_cell, &(argc as u32).to_le_bytes())?;
    cpu.write_guest_into(argv_cell, &argv.to_le_bytes())?;
    cpu.write_guest_into(environ_cell, &env.to_le_bytes())?;
    let name = path + "executable_path=".len() as u64;
    cpu.write_guest_into(progname_cell, &name.to_le_bytes())?;
    for (i, pointer) in [
        main_header,
        argc_cell,
        argv_cell,
        environ_cell,
        progname_cell,
    ]
    .iter()
    .enumerate()
    {
        cpu.write_guest_into(vars + i as u64 * 8, &pointer.to_le_bytes())?;
    }
    Ok(vec![argc, argv, env, apple, vars])
}

pub(super) fn random_munge_token() -> Result<u64, String> {
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut source = std::fs::File::open("/dev/urandom")
            .map_err(|error| format!("OS entropy unavailable: {error}"))?;
        for _ in 0..3 {
            let mut bytes = [0u8; 8];
            source
                .read_exact(&mut bytes)
                .map_err(|error| format!("OS entropy read failed: {error}"))?;
            let token = u64::from_ne_bytes(bytes);
            if token != 0 {
                return Ok(token);
            }
        }
        Err("nonzero OS pointer-munge entropy unavailable".into())
    }
    #[cfg(not(unix))]
    {
        Err("OS entropy adapter unavailable on this host".into())
    }
}

pub(super) fn append_thread_apple(cpu: &mut A64Cpu, arguments: &[u64], port: u32) -> Result<(), String> {
    let base = arguments[4]
        .checked_sub(64)
        .ok_or("invalid owned argument arena")?;
    if arguments[3] != base + 16 || port & 3 != 3 || port == u32::MAX {
        return Err("invalid owned thread apple context".into());
    }
    let apple = arguments[3];
    let slot = (0..4)
        .find(|&index| cpu.read_u64(apple + index * 8) == Some(0))
        .ok_or("owned apple vector has no bounded terminator")?;
    let text = format!("th_port={port:#x}\0");
    let address = base + 3968;
    cpu.write_guest_into(address, text.as_bytes())?;
    cpu.write_guest_into(apple + slot * 8, &address.to_le_bytes())?;
    cpu.write_guest_into(apple + (slot + 1) * 8, &0u64.to_le_bytes())?;
    Ok(())
}

pub(super) struct PreparedInitialization {
    pub session:super::execution_session::ExecutionSession,
    pub call:GuestCall,
}
pub(super) fn execute(
    cpu:&mut A64Cpu,plan:&CachePlan,services:&mut SelectedServices,arguments:Vec<u64>,
    main_stack:&super::main_stack::MainStackDescriptor,
    slide_route:(u64,super::dyld_slide::ImageSlides),restricted_entry:u64,
    immutable_route:(u64,super::dyld_slide::ImmutableRanges),tlv_images:Vec<(u64,u64,bool)>,
    sdk_query:Option<(u64,super::dyld_sdk_query::ProgramSdk)>,
    objc_callbacks:Option<(u64,Vec<super::dyld_objc_callbacks::ObjcImage>)>,
    cache_range:Option<(u64,super::dyld_cache_range::CacheRange)>,
    dyld_overridden:Option<u64>,
    dyld_add_image:Option<u64>,
    dyld_objc:Option<super::dyld_objc::DyldObjcEntries>,
)->Result<(),String> {
    let caller_context=cpu.save_context();
    let prepared=prepare_session(cpu,plan,services,arguments,main_stack,slide_route,restricted_entry,immutable_route,tlv_images,sdk_query,objc_callbacks,cache_range,dyld_overridden,dyld_add_image,dyld_objc);
    let mut prepared=match prepared {
        Ok(prepared)=>prepared,
        Err(error)=>{cpu.restore_context(&caller_context);return Err(error);}
    };
    echo!("[a64] genuine original libSystem diagnostic session entry={:#x}; no runtime receipt",prepared.call.entry);
    let result=prepared.session.call(cpu,services,&prepared.call,100_000);
    let trap_count=prepared.session.process().trap_count;
    cpu.restore_context(&caller_context);
    match result {
        Ok(_)=>{echo!("[a64] original libSystem initializer returned after {trap_count} supported traps; diagnostic only, readiness unproven");Ok(())}
        Err(error)=>Err(format!("original libSystem initializer probe stopped after {trap_count} supervisor traps: {error}; CPU context restored, partial guest-memory effects discarded with diagnostic CPU, no initialization receipt")),
    }
}
pub(super) fn prepare_session(
    cpu: &mut A64Cpu,
    plan: &CachePlan,
    services: &mut SelectedServices,
    arguments: Vec<u64>,
    main_stack: &super::main_stack::MainStackDescriptor,
    slide_route: (u64, super::dyld_slide::ImageSlides),
    restricted_entry: u64,
    immutable_route:(u64,super::dyld_slide::ImmutableRanges),
    tlv_images:Vec<(u64,u64,bool)>,
    sdk_query:Option<(u64,super::dyld_sdk_query::ProgramSdk)>,
    objc_callbacks:Option<(u64,Vec<super::dyld_objc_callbacks::ObjcImage>)>,
    cache_range:Option<(u64,super::dyld_cache_range::CacheRange)>,
    dyld_overridden:Option<u64>,
    dyld_add_image:Option<u64>,
    dyld_objc:Option<super::dyld_objc::DyldObjcEntries>,
) -> Result<PreparedInitialization, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut header = std::fs::File::open(
        plan.files
            .first()
            .ok_or("initializer cache identity absent")?,
    )
    .map_err(|error| error.to_string())?;
    header
        .seek(SeekFrom::Start(0x58))
        .map_err(|error| error.to_string())?;
    let mut uuid = [0u8; 16];
    header
        .read_exact(&mut uuid)
        .map_err(|error| error.to_string())?;
    if uuid
        != [
            0x32, 0x03, 0x55, 0x64, 0x85, 0x3b, 0x38, 0x8b, 0xa1, 0xef, 0x2e, 0xa3, 0x54, 0xc1,
            0x96, 0xe1,
        ]
    {
        return Err(
            "initializer execution probe requires audited original 20H392 cache UUID".into(),
        );
    }
    if arguments.len() != 5 {
        return Err("libSystem initializer requires five genuine dyld arguments".into());
    }
    if cpu.read_u64(arguments[2]) != Some(0) {
        return Err("declared-path initializer policy requires genuinely empty environment".into());
    }
    let functions = super::cache_initializers::functions(cpu, plan, "/usr/lib/libSystem.B.dylib")?;
    if functions.len() != 1 {
        return Err("audited libSystem probe requires exactly one original initializer".into());
    }
    let trampoline=services.install_dyld_slide(cpu,slide_route.0,slide_route.1)?;
    echo!("[a64] audited exact libdyld image-slide route entry={:#x} trampoline={trampoline:#x}; no general dyld initialization receipt",slide_route.0);
    let restricted_trampoline=services.install_dyld_restricted(cpu,restricted_entry)?;
    echo!("[a64] audited libdyld restricted query entry={restricted_entry:#x} trampoline={restricted_trampoline:#x}; declared paths only, DYLD environment search denied");
    let immutable_trampoline=services.install_dyld_immutable(cpu,immutable_route.0,immutable_route.1)?;
    echo!("[a64] audited libdyld immutable-range query entry={:#x} trampoline={immutable_trampoline:#x}; permanent image provenance and actual maximum protections",immutable_route.0);
    if let Some((entry,program))=sdk_query {
        let target=services.install_dyld_sdk_query(cpu,entry,program)?;
        echo!("[a64] original SDK query entry={entry:#x} trampoline={target:#x}; actual selected main executable SDK, independent of virtual OS version");
    }
    if let Some((entry,range))=cache_range {
        let target=services.install_dyld_cache_range(cpu,entry,range)?;
        echo!("[a64] original mapped shared-cache range entry={entry:#x} target={target:#x}; original VMspan provenance, no allocated-byte sum");
    }
    if let Some(entry)=dyld_overridden {
        let target=services.install_dyld_overridden(cpu,entry)?;
        echo!("[a64] original dyld overridden query entry={entry:#x} target={target:#x}; reported false (no cache images overridden)");
    }
    if let Some(entry)=dyld_add_image {
        let target=services.install_dyld_add_image(cpu,entry)?;
        echo!("[a64] original dyld add-image query entry={entry:#x} target={target:#x}; registered callback handler");
    }
    if let Some(entries)=dyld_objc {
        services.install_dyld_objc(cpu, entries)?;
        echo!("[a64] original dyld ObjC optimization query hooks installed");
    }
    super::commpage_ro::ensure_mapped(cpu)?;
    let mut ports = super::mach_identity::MachIdentity::new(64)?;
    let mut vm = super::mach_vm::AnonymousVm::new(0x20_0000_0000, 0x20_1000_0000)?;
    let priorities = super::mach_host_info::HostPriorityPolicy::virtual_darwin();
    let mut clock = super::mach_clock::SystemClock::new();
    let mut semaphores = super::mach_semaphore::SemaphoreService::new();
    let mut entropy = super::entropy_fd::EntropyFds::new();
    let standard_fds = super::standard_fds::StandardFds::new();
    let shared_memory = super::posix_shm::ShmNamespace::new_isolated();
    let thread = 1;
    ports.register_thread(thread)?;
    let thread_port = ports.thread_self(thread)?;
    append_thread_apple(cpu, &arguments, thread_port)?;
    let thread_base = arguments[4]
        .checked_add(8191)
        .ok_or("primordial TSD arena overflow")?
        & !4095;
    let tsd = primordial_tsd(cpu, thread_base, thread_port)?;
    // This private helper is explicitly called by libSystem rather than being
    // registered in libdyld's module-initializer sections. The decoder still
    // validates the genuine provider header/install identity; callsite + stub
    // bytes establish the actual private target independently of init lists.
    let dyld_initializers=super::cache_initializers::functions(cpu,plan,"/usr/lib/system/libdyld.dylib")?;
    let mut direct_call=[0u8;4];cpu.read_guest_into(0x1d1c29780,&mut direct_call)?;
    let mut direct_stub=[0u8;12];cpu.read_guest_into(0x1d58dfed4,&mut direct_stub)?;
    if direct_call!=[0xd5,0xd9,0xf2,0x94] || direct_stub!=[0xf0,0x9c,0xe8,0xd0,0x10,0x82,0x3c,0x91,0,2,0x1f,0xd6] {
        return Err("original libSystem-to-libdyld helper call provenance mismatch".into());
    }
    echo!("[a64] verified original libSystem direct libdyld helper call; provider initializer-list targets={} remain separate",dyld_initializers.len());
    let cache_contains=|address:u64,size:usize,execute:bool|plan.regions.iter().any(|r| {
        r.vmaddr<=address && address.checked_add(size as u64).is_some_and(|end|r.vmaddr.checked_add(r.size).is_some_and(|limit|end<=limit)) && (!execute||r.init_prot&4!=0)
    });
    let helpers=super::dyld_helpers::Helpers::read(cpu,super::dyld_helpers::HELPER_OBJECT,
        |a,n|cache_contains(a,n,false),|a,n|cache_contains(a,n,true))?;
    let helper_arena=thread_base.checked_add(8192).ok_or("dyld helper arena overflow")?;
    let mut tlv_plans=Vec::new();let mut template_bytes=0usize;
    for (header,slide,verified_cache) in tlv_images {
        if let Some(plan)=super::tlv::read_with_cache(cpu,header,slide,verified_cache).map_err(|error|format!("actual loaded-image TLV header {header:#x}: {error}"))? {
            template_bytes=template_bytes.checked_add(plan.template.len()).ok_or("TLV template budget overflow")?;
            if template_bytes>32*1048576{return Err("loaded TLV template total budget exceeded".into());}
            echo!("[a64] actual loaded TLV image={header:#x} template={} align={} descriptors={} initializers={} cached_key={:?}",plan.template.len(),plan.alignment,plan.descriptors.len(),plan.initializers.len(),plan.preallocated_key);
            tlv_plans.push(plan);
        }
    }
    let caller_context = cpu.save_context();
    cpu.set_tpidrro_el0(tsd);
    cpu.set_tpidr_el0(0);
    // Adopt the genuine main execution snapshot before entering the bridge's
    // separate initializer stack. Nested calls retain this same CPU owner.
    let mut scheduler = super::thread_scheduler_cpu::CpuScheduler::default();
    let adoption = (|| {
        if main_stack.owner() != thread || cpu.sp() != main_stack.initial_sp() {
            return Err("primordial CPU does not match retained main-stack owner/SP".into());
        }
        let owner = scheduler.adopt(cpu, main_stack.bounds())?;
        if owner.0 != thread || scheduler.select(cpu)? != Some(owner) {
            return Err("primordial scheduler ownership mismatch".into());
        }
        Ok::<_, String>(owner)
    })();
    let owner = match adoption {
        Ok(owner) => owner,
        Err(error) => {
            cpu.restore_context(&caller_context);
            return Err(error);
        }
    };
    echo!("[a64] adopted genuine primordial CPU owner={owner:?} main_stack={:x?}; nested initializer retains owner", main_stack.bounds());
    let scheduler = std::rc::Rc::new(std::cell::RefCell::new(scheduler));
    let control = match super::bsdthread_ctl::OwnedControl::install(scheduler.clone()) {
        Ok(control) => control,
        Err(error) => {
            cpu.restore_context(&caller_context);
            return Err(error);
        }
    };
    let identity=match super::execution_session::ProcessIdentity::new(&scheduler) {
        Ok(identity)=>identity,
        Err(error)=>{cpu.restore_context(&caller_context);return Err(error);}
    };
    let process=super::execution_session::ProcessState {
        identity,
        ports,vm,priorities,clock,semaphores,entropy,standard_fds,shared_memory,
        credentials:super::credentials::CredentialTaint::isolated_unprivileged(),
        scheduler,control,registration:super::pthread_registration::ProcessRegistration::default(),
        thread,owner,trap_count:0,
    };
    let session=super::execution_session::ExecutionSession::from_prepared(cpu,process,tsd)?;
    if let Some((entry,images))=objc_callbacks {
        let target=services.install_dyld_objc_callbacks(cpu,entry,images,session.lease(),session.process().scheduler.clone(),main_stack.owner())?;
        echo!("[a64] original ObjC v1 notification entry={entry:#x} target={target:#x}; selected mapped-image callback delivery required before returning");
    }
    let helper_trampoline=match services.install_dyld_owned_helper_prefix(cpu,super::dyld_helpers::INITIALIZER,helpers,helper_arena,tlv_plans,main_stack.owner(),session.process().scheduler.clone(),session.lease()) {
        Ok(target)=>target,
        Err(error)=>{cpu.restore_context(&caller_context);return Err(error);}
    };
    echo!("[a64] audited original version6 dyld helper route trampoline={helper_trampoline:#x} key_cells={helper_arena:#x}; actual selected-owner lazy TLV route, no loader initialization receipt");
    let call=GuestCall {entry:functions[0],integers:arguments,..Default::default()};
    Ok(PreparedInitialization{session,call})
}

/// Scalar trap ABI from XNU mach_traps.h; diagnostic reads only, no delivery.
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
    let mut bytes = vec![0u8; size.min(64) as usize];
    cpu.read_guest_into(args[0], &mut bytes)
        .map_err(|error| format!("{summary}; bounded message read failed: {error}"))?;
    Ok(format!(
        "{summary}; declared_size={size} message={bytes:02x?}; no message delivered"
    ))
}

/// Actual diagnostic CPU owner and send right; not a registered Darwin pthread.
/// Pre-pthread self/errno indirection follows _pthread_set_self_dyld. Signature
/// remains zero and ordinary Darwin pthread lifecycle is not claimed.
pub(super) fn primordial_tsd(cpu: &mut A64Cpu, base: u64, thread_port: u32) -> Result<u64, String> {
    if base == 0 || base & 4095 != 0 || thread_port & 3 != 3 || thread_port == u32::MAX {
        return Err("invalid primordial thread storage/right".into());
    }
    cpu.map_zeroed(base, 8192, 3)?;
    let tsd = base + 224;
    cpu.write_guest_into(tsd, &base.to_le_bytes())?;
    cpu.write_guest_into(tsd + 8, &(base + 172).to_le_bytes())?;
    cpu.write_guest_into(tsd + 24, &u64::from(thread_port).to_le_bytes())?;
    Ok(tsd)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn genuine_stack_and_owned_thread_apple_strings_are_writable() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 1).unwrap();
        cpu.map_zeroed(0x20000, 4096, 3).unwrap();
        cpu.map_zeroed(0x50000, 16384, 3).unwrap();
        let stack = super::super::main_stack::MainStackDescriptor::from_loader(
            &cpu, 1, 0x50000, 16384, 0x50000, 16384, 0x53ff0,
        )
        .unwrap();
        cpu.set_reg(0, 1);
        cpu.set_reg(1, 0x20000);
        let entries = [stack.apple_entry(), "ptr_munge=0x1234".into()];
        let args = arguments(&mut cpu, 0x30000, 0x10000, "/Game.app/Game", &entries).unwrap();
        let mut ports = super::super::mach_identity::MachIdentity::new(8).unwrap();
        ports.register_thread(1).unwrap();
        let port = ports.thread_self(1).unwrap();
        append_thread_apple(&mut cpu, &args, port).unwrap();
        for (index, expected) in [
            entries[0].clone(),
            entries[1].clone(),
            format!("th_port={port:#x}"),
        ]
        .iter()
        .enumerate()
        {
            let pointer = cpu.read_u64(args[3] + (index as u64 + 1) * 8).unwrap();
            let mut bytes = vec![0u8; expected.len() + 1];
            cpu.read_guest_into(pointer, &mut bytes).unwrap();
            assert_eq!(&bytes[..expected.len()], expected.as_bytes());
            assert_eq!(bytes[expected.len()], 0);
            cpu.write_guest_into(pointer, &vec![0u8; bytes.len()])
                .unwrap();
        }
        assert_eq!(cpu.read_u64(args[3] + 32), Some(0));
    }
    #[test]
    fn scalar_message_capture_bounds_and_vector_refusal() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 40, 1).unwrap();
        let mut args = [0u64; 8];
        args[0] = 0x10000;
        args[1] = 0x2_0000_0003;
        args[2] = 40u64 << 32 | 0x1513;
        assert!(capture_message(&cpu, -47, 0x40000, args)
            .unwrap()
            .contains("declared_size=40"));
        args[2] = 64u64 << 32;
        assert!(capture_message(&cpu, -47, 0x40000, args).is_err());
        args[0] = u64::MAX;
        args[1] |= 0x1_0000_0000;
        assert!(capture_message(&cpu, -47, 0x40000, args)
            .unwrap()
            .contains("vector message ABI not inspected"));
        args[1] = 2;
        assert!(capture_message(&cpu, -47, 0x40000, args)
            .unwrap()
            .contains("receive-only"));
    }
    #[test]
    fn primordial_guest_tsd_reads_owned_mach_port_not_thread_id() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 5).unwrap();
        cpu.write_bytes(0x10000, &0xd53bd068u32.to_le_bytes());
        cpu.write_bytes(0x10004, &0xf9400d00u32.to_le_bytes());
        let tsd = primordial_tsd(&mut cpu, 0x20000, 0x203).unwrap();
        assert_eq!(cpu.read_u64(0x20000), Some(0));
        assert_eq!(cpu.read_u64(tsd), Some(0x20000));
        cpu.set_tpidrro_el0(tsd);
        cpu.set_pc(0x10000);
        cpu.run_or_step(None);
        cpu.run_or_step(None);
        assert_eq!(cpu.reg(0), 0x203);
        assert!(primordial_tsd(&mut cpu, 0x40000, 0).is_err());
        assert!(primordial_tsd(&mut cpu, 0x40000, 0x101).is_err());
    }
    #[test]
    fn program_vars_are_real_writable_pointer_cells() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 1).unwrap();
        cpu.map_zeroed(0x20000, 4096, 3).unwrap();
        cpu.set_reg(0, 1);
        cpu.set_reg(1, 0x20000);
        let args = arguments(&mut cpu, 0x30000, 0x10000, "/Game.app/Game", &[]).unwrap();
        assert_eq!(&args[..2], &[1, 0x20000]);
        let vars = args[4];
        assert_eq!(cpu.read_u64(vars), Some(0x10000));
        let argc_cell = cpu.read_u64(vars + 8).unwrap();
        cpu.write_guest_into(argc_cell, &2u32.to_le_bytes())
            .unwrap();
        let mut bytes = [0u8; 4];
        cpu.read_guest_into(argc_cell, &mut bytes).unwrap();
        assert_eq!(u32::from_le_bytes(bytes), 2);
        let argv_cell = cpu.read_u64(vars + 16).unwrap();
        assert_eq!(cpu.read_u64(argv_cell), Some(0x20000));
        assert_eq!(cpu.read_u64(args[2]), Some(0));
        assert_eq!(cpu.read_u64(args[3] + 8), Some(0));
        assert!(arguments(&mut cpu, 0x30000, 0x10000, "/Game.app/Game", &[]).is_err());
    }
}

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bounded virtual Mach identity-port namespace for libkernel initialization.
//! Task_self (-28), host_self (-29), registered thread self (-27), reply (-26)
//! identities are implemented. Message delivery/bootstrap services remain unsupported.
//! Reply ports belong to the task IPC namespace, not a thread-special port.
//! Resource failures in these name-returning traps return MACH_PORT_NULL.
//! ABI evidence: apple-xnu/osfmk/mach/syscall_sw.h and actual iOS16 libkernel
//! stubs 0x1c260cf50 / 0x1c260cf38 called by mach_init_doit.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_NAMESPACE: AtomicU64 = AtomicU64::new(1);
/// One transient MAKE_SEND_ONCE capability for a validated synchronous host RPC.
/// It is neither a guest port name nor a clonable/replayable send right.
#[derive(Debug)]
pub struct HostReplyTicket {
    namespace: u64,
    nonce: u64,
}
#[derive(Debug)]
pub struct ClockReplyTicket {
    reply: HostReplyTicket,
    name: u32,
}
impl ClockReplyTicket {
    pub fn name(&self) -> u32 {
        self.name
    }
}
#[derive(Debug)]
pub struct SemaphoreReplyTicket {
    reply: HostReplyTicket,
    name: u32,
    task: u32,
}
impl SemaphoreReplyTicket {
    pub fn name(&self) -> u32 {
        self.name
    }
}
#[derive(Debug)]
pub struct BootstrapReplyTicket {
    reply: HostReplyTicket,
    name: u32,
}
impl BootstrapReplyTicket {
    pub fn name(&self) -> u32 {
        self.name
    }
}
// XNU MACH_PORT_MAKE(index,gen) uses index<<8 | gen>>24. The
// IE_BITS_GEN roll-mask reserves the generation's low two name bits as 3.
// These bits matter to libplatform's os_once owner/generation discriminator.
const TASK_NAME: u32 = 0x103;
const NAME_INDEX_STEP: u32 = 0x100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortRight {
    TaskSend {
        references: u32,
    },
    HostSend {
        references: u32,
    },
    ClockSend {
        clock: u32,
        references: u32,
    },
    /// Task-owned semaphore object; user send references do not own its count.
    SemaphoreSend {
        task: u32,
        references: u32,
    },
    ThreadSend {
        thread: u64,
        references: u32,
    },
    BootstrapSend {
        references: u32,
    },
    DeadName {
        references: u32,
    },
    ReplyReceive,
    /// Task-space receive right constructed with MPO_REPLY_PORT. This is not
    /// the separately thread-bound special reply port or an inserted send right.
    ConstructedReplyReceive,
    GuardedPort {
        guard: u64,
        strict: bool,
        send_references: u32,
    },
}

pub struct MachIdentity {
    ports: BTreeMap<u32, PortRight>,
    next_name: u32,
    max_ports: usize,
    alive: bool,
    threads: BTreeMap<u64, Option<u32>>,
    host_name: Option<u32>,
    namespace: u64,
    next_rpc: u64,
    host_replies: BTreeMap<u64, (u32, u32)>,
    system_clock_name: Option<u32>,
    clock_reply: Option<(u64, u32)>,
    semaphore_replies: BTreeMap<u64, (u32, u32)>,
    bootstrap_name: Option<u32>,
    bootstrap_reply: Option<(u64, u32)>,
    debug_control_port: Option<u32>,
}

impl MachIdentity {
    fn occupied_ports(&self) -> usize {
        self.ports.len()
            + usize::from(
                self.clock_reply
                    .is_some_and(|(_, name)| !self.ports.contains_key(&name)),
            )
            + usize::from(
                self.bootstrap_reply
                    .is_some_and(|(_, name)| !self.ports.contains_key(&name)),
            )
            + self.semaphore_replies.len()
    }
    pub fn new(max_ports: usize) -> Result<Self, String> {
        if max_ports == 0 || max_ports > 4096 {
            return Err("Mach namespace port budget must be in 1..=4096".into());
        }
        let mut ports = BTreeMap::new();
        ports.insert(TASK_NAME, PortRight::TaskSend { references: 0 });
        let namespace = NEXT_NAMESPACE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| "Mach namespace identity exhausted")?;
        Ok(Self {
            ports,
            next_name: 0x203,
            max_ports,
            alive: true,
            threads: BTreeMap::new(),
            host_name: None,
            namespace,
            next_rpc: 1,
            host_replies: BTreeMap::new(),
            system_clock_name: None,
            clock_reply: None,
            semaphore_replies: BTreeMap::new(),
            bootstrap_name: None,
            bootstrap_reply: None,
            debug_control_port: None,
        })
    }

    pub fn right(&self, name: u32) -> Option<PortRight> {
        self.ports
            .get(&name)
            .copied()
            .filter(|right| !matches!(right, PortRight::TaskSend { references: 0 }))
    }

    /// Exact COPY_SEND(host)/MAKE_SEND_ONCE(reply) subset used by host_info.
    /// The kernel-held send-once capability has no user-space name or uref.
    /// The caller must validate the complete request and response buffer first.
    pub fn reserve_host_reply(
        &mut self,
        host: u32,
        receive: u32,
    ) -> Result<HostReplyTicket, String> {
        if !self.alive
            || !matches!(self.right(host), Some(PortRight::HostSend { references }) if references > 0)
            || !matches!(self.right(receive),Some(PortRight::ReplyReceive|PortRight::ConstructedReplyReceive))
        {
            return Err(
                "host RPC requires owned host send and live reply receive rights".into(),
            );
        }
        self.reserve_reply_endpoint(host, receive)
    }
    fn reserve_reply_endpoint(
        &mut self,
        target: u32,
        receive: u32,
    ) -> Result<HostReplyTicket, String> {
        if !self.alive || !matches!(self.right(receive),Some(PortRight::ReplyReceive|PortRight::ConstructedReplyReceive)) {
            return Err("RPC requires live reply receive right".into());
        }
        if self.host_replies.len() >= 64 || self.host_replies.values().any(|(_, r)| *r == receive) {
            return Err("host RPC reply reservation busy or budget exhausted".into());
        }
        let nonce = self.next_rpc;
        let next = nonce
            .checked_add(1)
            .ok_or("host RPC ticket identity exhausted")?;
        self.host_replies.insert(nonce, (target, receive));
        self.next_rpc = next;
        Ok(HostReplyTicket {
            namespace: self.namespace,
            nonce,
        })
    }

    /// COPY_SEND(current task)/MAKE_SEND_ONCE(reply), with no minted guest name.
    /// RPC-specific state validation and response preflight precede reservation.
    pub fn reserve_task_reply(&mut self,task:u32,receive:u32)->Result<HostReplyTicket,String> {
        if !self.alive||!matches!(self.right(task),Some(PortRight::TaskSend{references}) if references>0) {
            return Err("task RPC requires owned current task send right".into());
        }
        self.reserve_reply_endpoint(task,receive)
    }
    pub fn cancel_task_reply(&mut self,ticket:HostReplyTicket)->Result<(),String>{self.retire_host_reply(ticket)}
    pub fn consume_task_reply(&mut self,ticket:HostReplyTicket)->Result<(),String>{self.retire_host_reply(ticket)}

    fn retire_host_reply(&mut self, ticket: HostReplyTicket) -> Result<(), String> {
        if ticket.namespace != self.namespace || !self.alive {
            return Err("foreign or terminated host RPC ticket".into());
        }
        let &(_, receive) = self
            .host_replies
            .get(&ticket.nonce)
            .ok_or("stale host RPC ticket")?;
        // COPY_SEND transferred a kernel-held capability at reservation time;
        // later deallocation of the user's host send reference cannot revoke it.
        if !matches!(self.right(receive),Some(PortRight::ReplyReceive|PortRight::ConstructedReplyReceive)) {
            return Err("host RPC rights changed before completion".into());
        }
        self.host_replies.remove(&ticket.nonce);
        Ok(())
    }

    /// Cancel before response delivery; receive ownership and host urefs survive.
    pub fn cancel_host_reply(&mut self, ticket: HostReplyTicket) -> Result<(), String> {
        self.retire_host_reply(ticket)
    }

    /// Call only after the actual response was copied into validated guest memory.
    /// Successful delivery consumes the transient send-once capability exactly once.
    pub fn consume_host_reply(&mut self, ticket: HostReplyTicket) -> Result<(), String> {
        self.retire_host_reply(ticket)
    }
    pub fn prepare_semaphore_reply(
        &mut self,
        task: u32,
        receive: u32,
    ) -> Result<SemaphoreReplyTicket, String> {
        if !matches!(self.right(task),Some(PortRight::TaskSend{references}) if references>0) {
            return Err("semaphore creation requires owned task send right".into());
        }
        if self.occupied_ports() >= self.max_ports {
            return Err("semaphore send-right budget exhausted".into());
        }
        let name = self.next_name;
        let next = name
            .checked_add(NAME_INDEX_STEP)
            .ok_or("semaphore port name exhausted")?;
        let reply = self.reserve_reply_endpoint(task, receive)?;
        self.semaphore_replies.insert(reply.nonce, (name, task));
        self.next_name = next;
        Ok(SemaphoreReplyTicket { reply, name, task })
    }
    fn validate_semaphore_reply(&self, ticket: &SemaphoreReplyTicket) -> Result<(), String> {
        if ticket.reply.namespace != self.namespace
            || !self.alive
            || self.semaphore_replies.get(&ticket.reply.nonce) != Some(&(ticket.name, ticket.task))
        {
            return Err("foreign/stale semaphore reply ticket".into());
        }
        Ok(())
    }
    pub fn cancel_semaphore_reply(&mut self, ticket: SemaphoreReplyTicket) -> Result<(), String> {
        self.validate_semaphore_reply(&ticket)?;
        let nonce = ticket.reply.nonce;
        self.retire_host_reply(ticket.reply)?;
        self.semaphore_replies.remove(&nonce);
        Ok(())
    }
    pub fn commit_semaphore_reply(&mut self, ticket: SemaphoreReplyTicket) -> Result<(), String> {
        self.validate_semaphore_reply(&ticket)?;
        if self.ports.contains_key(&ticket.name) {
            return Err("reserved semaphore name already published".into());
        }
        let nonce = ticket.reply.nonce;
        self.retire_host_reply(ticket.reply)?;
        self.semaphore_replies.remove(&nonce);
        self.ports.insert(
            ticket.name,
            PortRight::SemaphoreSend {
                task: ticket.task,
                references: 1,
            },
        );
        Ok(())
    }
    /// Reserve a SYSTEM_CLOCK send-right copyout, without exposing ownership
    /// before the complex reply descriptor is successfully delivered.
    pub fn prepare_system_clock_reply(
        &mut self,
        host: u32,
        receive: u32,
    ) -> Result<ClockReplyTicket, String> {
        if self.clock_reply.is_some() {
            return Err("system clock reply transaction already pending".into());
        }
        let existing = self.system_clock_name;
        if let Some(name) = existing {
            if !matches!(self.right(name),Some(PortRight::ClockSend{clock:0,references}) if references>0)
            {
                return Err("system clock identity inconsistent".into());
            }
        } else if self.occupied_ports() >= self.max_ports {
            return Err("system clock port budget exhausted".into());
        }
        let name = existing.unwrap_or(self.next_name);
        let next = if existing.is_none() {
            Some(
                name.checked_add(NAME_INDEX_STEP)
                    .ok_or("system clock name exhausted")?,
            )
        } else {
            None
        };
        let reply = self.reserve_host_reply(host, receive)?;
        if let Some(next) = next {
            self.next_name = next;
        }
        self.clock_reply = Some((reply.nonce, name));
        Ok(ClockReplyTicket { reply, name })
    }
    fn validate_clock_reply(&self, ticket: &ClockReplyTicket) -> Result<(), String> {
        if ticket.reply.namespace != self.namespace
            || !self.alive
            || self.clock_reply != Some((ticket.reply.nonce, ticket.name))
        {
            return Err("foreign/stale system clock reply ticket".into());
        }
        Ok(())
    }
    pub fn cancel_clock_reply(&mut self, ticket: ClockReplyTicket) -> Result<(), String> {
        self.validate_clock_reply(&ticket)?;
        self.retire_host_reply(ticket.reply)?;
        self.clock_reply = None;
        Ok(())
    }
    pub fn commit_clock_reply(&mut self, ticket: ClockReplyTicket) -> Result<(), String> {
        self.validate_clock_reply(&ticket)?;
        let refs = match self.right(ticket.name) {
            Some(PortRight::ClockSend {
                clock: 0,
                references,
            }) => references.saturating_add(1).min(65535),
            None => 1,
            _ => return Err("reserved clock right changed before delivery".into()),
        };
        self.retire_host_reply(ticket.reply)?;
        self.ports.insert(
            ticket.name,
            PortRight::ClockSend {
                clock: 0,
                references: refs,
            },
        );
        self.system_clock_name = Some(ticket.name);
        self.clock_reply = None;
        Ok(())
    }

    pub fn prepare_bootstrap_reply(
        &mut self,
        task: u32,
        receive: u32,
    ) -> Result<BootstrapReplyTicket, String> {
        if self.bootstrap_reply.is_some() {
            return Err("bootstrap reply transaction already pending".into());
        }
        let existing = self.bootstrap_name;
        if let Some(name) = existing {
            if !matches!(self.right(name), Some(PortRight::BootstrapSend { references }) if references > 0)
            {
                return Err("bootstrap port identity inconsistent".into());
            }
        } else if self.occupied_ports() >= self.max_ports {
            return Err("bootstrap port budget exhausted".into());
        }
        let name = existing.unwrap_or(self.next_name);
        let next = if existing.is_none() {
            Some(
                name.checked_add(NAME_INDEX_STEP)
                    .ok_or("bootstrap port name exhausted")?,
            )
        } else {
            None
        };
        let reply = self.reserve_task_reply(task, receive)?;
        if let Some(next) = next {
            self.next_name = next;
        }
        self.bootstrap_reply = Some((reply.nonce, name));
        Ok(BootstrapReplyTicket { reply, name })
    }

    fn validate_bootstrap_reply(&self, ticket: &BootstrapReplyTicket) -> Result<(), String> {
        if ticket.reply.namespace != self.namespace
            || !self.alive
            || self.bootstrap_reply != Some((ticket.reply.nonce, ticket.name))
        {
            return Err("foreign/stale bootstrap reply ticket".into());
        }
        Ok(())
    }

    pub fn cancel_bootstrap_reply(&mut self, ticket: BootstrapReplyTicket) -> Result<(), String> {
        self.validate_bootstrap_reply(&ticket)?;
        self.retire_host_reply(ticket.reply)?;
        self.bootstrap_reply = None;
        Ok(())
    }

    pub fn commit_bootstrap_reply(&mut self, ticket: BootstrapReplyTicket) -> Result<(), String> {
        self.validate_bootstrap_reply(&ticket)?;
        let refs = match self.right(ticket.name) {
            Some(PortRight::BootstrapSend { references }) => {
                references.saturating_add(1).min(65535)
            }
            None => 1,
            _ => return Err("reserved bootstrap right changed before delivery".into()),
        };
        self.retire_host_reply(ticket.reply)?;
        self.ports.insert(
            ticket.name,
            PortRight::BootstrapSend {
                references: refs,
            },
        );
        self.bootstrap_name = Some(ticket.name);
        self.bootstrap_reply = None;
        Ok(())
    }

    pub fn reply_task_special_port(
        &mut self,
        cpu: &mut super::A64Cpu,
        args: [u64; 8],
    ) -> Result<u32, String> {
        let [data, options, bits_size, remote_local, voucher_id, descriptors_receive, receive_priority, timeout] = args;
        let task = remote_local as u32;
        let receive = (remote_local >> 32) as u32;
        let capacity = receive_priority as u32;
        if options != 0x200000003
            || bits_size != 0x2400001513
            || voucher_id != 3409u64 << 32
            || descriptors_receive != (receive as u64) << 32
            || receive_priority >> 32 != 0
            || capacity < 48
            || capacity > 4096
            || timeout != 0
        {
            return Err("unsupported task_get_special_port message envelope".into());
        }
        if !matches!(self.right(task), Some(PortRight::TaskSend { references }) if references > 0) {
            return Ok(0x10000003); // MACH_SEND_INVALID_DEST
        }
        if !matches!(self.right(receive), Some(PortRight::ConstructedReplyReceive)) {
            return Ok(0x10000009); // MACH_SEND_INVALID_REPLY
        }
        let mut request = [0u8; 36];
        cpu.read_guest_into(data, &mut request)?;
        let word = |offset: usize| u32::from_le_bytes(request[offset..offset + 4].try_into().unwrap());
        if word(0) != 0x1513
            || word(4) != 36
            || word(8) != task
            || word(12) != receive
            || word(16) != 0
            || word(20) != 3409
        {
            return Err("unsupported task_get_special_port request header".into());
        }
        if request[24..32] != [0, 0, 0, 0, 1, 0, 0, 0] {
            return Err("unsupported task_get_special_port NDR".into());
        }
        let which_port = word(32);
        if which_port != 4 {
            return Err(format!("unsupported which_port {which_port} in task_get_special_port (expected TASK_BOOTSTRAP_PORT=4)"));
        }
        cpu.validate_guest_write(data, capacity as usize)?;
        let ticket = self.prepare_bootstrap_reply(task, receive)?;
        let mut response = [0u8; 48];
        let put = |buf: &mut [u8], offset: usize, value: u32| {
            buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        for (offset, value) in [
            (0, 0x80001200u32),
            (4, 40),
            (12, receive),
            (20, 3509),
            (24, 1),
            (28, ticket.name()),
            (44, 8),
        ] {
            put(&mut response, offset, value);
        }
        response[38] = 17; // MACH_MSG_TYPE_MOVE_SEND
        if let Err(error) = cpu.write_guest_into(data, &response) {
            self.cancel_bootstrap_reply(ticket)?;
            return Err(error);
        }
        self.commit_bootstrap_reply(ticket)?;
        Ok(0)
    }

    /// Exact synchronous Mach task_set_special_port (3410) service.
    /// Source contracts: apple-xnu task.defs, mach/task_special_ports.h.
    pub fn reply_task_set_special_port(
        &mut self,
        cpu: &mut super::A64Cpu,
        args: [u64; 8],
    ) -> Result<u32, String> {
        let [data, options, bits_size, remote_local, voucher_id, descriptors_receive, receive_priority, timeout] = args;
        let task = remote_local as u32;
        let receive = (remote_local >> 32) as u32;
        let capacity = receive_priority as u32;
        if options != 0x200000003
            || bits_size != ((52u64 << 32) | 0x80001513)
            || voucher_id != 3410u64 << 32
            || descriptors_receive != ((u64::from(receive) << 32) | 1)
            || receive_priority >> 32 != 0
            || capacity < 44
            || capacity > 4096
            || timeout != 0
        {
            return Err("unsupported task_set_special_port message envelope".into());
        }
        if !matches!(self.right(task), Some(PortRight::TaskSend { references }) if references > 0) {
            return Ok(0x10000003); // MACH_SEND_INVALID_DEST
        }
        if !matches!(self.right(receive), Some(PortRight::ConstructedReplyReceive)) {
            return Ok(0x10000009); // MACH_SEND_INVALID_REPLY
        }
        let mut request = [0u8; 52];
        cpu.read_guest_into(data, &mut request)?;
        let word = |offset: usize| u32::from_le_bytes(request[offset..offset + 4].try_into().unwrap());
        if word(0) != 0x80001513
            || word(4) != 52
            || word(8) != task
            || word(12) != receive
            || word(16) != 0
            || word(20) != 3410
            || word(24) != 1
        {
            return Err("unsupported task_set_special_port request header".into());
        }
        let special_port = word(28);
        if word(32) != 0 || request[36] != 0 || request[37] != 0 || request[39] != 0 {
            return Err("unsupported task_set_special_port descriptor padding/type".into());
        }
        let disposition = request[38];
        if !matches!(disposition, 17 | 19 | 20) {
            return Err(format!("unsupported disposition {disposition} in task_set_special_port"));
        }
        if request[40..48] != [0, 0, 0, 0, 1, 0, 0, 0] {
            return Err("unsupported task_set_special_port NDR".into());
        }
        let which_port = word(48);
        if which_port == 0 || which_port > 11 {
            return Ok(4); // KERN_INVALID_ARGUMENT
        }
        if special_port != 0 {
            let mut should_remove = false;
            match self.ports.get_mut(&special_port) {
                Some(PortRight::TaskSend { references })
                | Some(PortRight::HostSend { references })
                | Some(PortRight::ClockSend { references, .. })
                | Some(PortRight::SemaphoreSend { references, .. })
                | Some(PortRight::ThreadSend { references, .. })
                | Some(PortRight::BootstrapSend { references }) => {
                    if disposition == 19 {
                        if *references > 1 {
                            *references -= 1;
                        } else {
                            should_remove = true;
                        }
                    }
                }
                Some(PortRight::GuardedPort { send_references, .. }) => {
                    if disposition == 19 && *send_references > 0 {
                        *send_references -= 1;
                    }
                }
                _ => return Ok(0x10000008), // MACH_SEND_INVALID_RIGHT
            }
            if should_remove {
                self.ports.remove(&special_port);
            }
        }
        match which_port {
            4 => self.bootstrap_name = if special_port != 0 { Some(special_port) } else { None },
            10 => self.debug_control_port = if special_port != 0 { Some(special_port) } else { None },
            _ => {}
        }
        cpu.validate_guest_write(data, capacity as usize)?;
        let ticket = self.reserve_task_reply(task, receive)?;
        let mut response = [0u8; 44];
        let put = |buf: &mut [u8], offset: usize, value: u32| {
            buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut response, 0, 0x1200);
        put(&mut response, 4, 36);
        put(&mut response, 12, receive);
        put(&mut response, 20, 3510);
        response[24..32].copy_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0]);
        put(&mut response, 32, 0); // KERN_SUCCESS
        put(&mut response, 40, 8); // Trailer size = 8
        if let Err(error) = cpu.write_guest_into(data, &response) {
            self.cancel_task_reply(ticket)?;
            return Err(error);
        }
        self.consume_task_reply(ticket)?;
        echo!("[a64] genuine task_set_special_port task={task:#x} which_port={which_port} special_port={special_port:#x} -> KERN_SUCCESS");
        Ok(0)
    }

    /// _kernelrpc_mach_port_deallocate_trap (-18), current-task namespace only.
    /// Receive-only rights survive; this operation consumes send/dead-name urefs.
    pub fn deallocate(&mut self, task: u32, name: u32) -> u32 {
        if !matches!(self.right(task), Some(PortRight::TaskSend { references }) if references > 0) {
            return 0x10000003; // trap-specific MACH_SEND_INVALID_DEST
        }
        if name == 0 || name == u32::MAX {
            return 0;
        }
        if self.clock_reply.is_some_and(|(_, pending)| pending == name)
            || self.bootstrap_reply.is_some_and(|(_, pending)| pending == name)
        {
            return 5; /* synchronous copyout owns this name */
        }
        let Some(right) = self.right(name) else {
            return 15;
        }; // KERN_INVALID_NAME
        let references = match right {
            PortRight::TaskSend { references }
            | PortRight::HostSend { references }
            | PortRight::ClockSend { references, .. }
            | PortRight::SemaphoreSend { references, .. }
            | PortRight::ThreadSend { references, .. }
            | PortRight::BootstrapSend { references }
            | PortRight::GuardedPort { send_references: references, .. }
            | PortRight::DeadName { references } => references,
            PortRight::ReplyReceive | PortRight::ConstructedReplyReceive => return 17,
        };
        // XNU leaves overflow-pegged urefs pegged during deallocation too.
        if references == 65535 {
            return 0;
        }
        if references > 1 {
            let entry = self.ports.get_mut(&name).unwrap();
            match entry {
                PortRight::TaskSend { references }
                | PortRight::HostSend { references }
                | PortRight::ClockSend { references, .. }
                | PortRight::SemaphoreSend { references, .. }
                | PortRight::ThreadSend { references, .. }
                | PortRight::BootstrapSend { references }
                | PortRight::DeadName { references } => *references -= 1,
                PortRight::GuardedPort { ref mut send_references, .. } => *send_references -= 1,
                _ => unreachable!(),
            }
        } else {
            match right {
                PortRight::TaskSend { .. } => {
                    self.ports
                        .insert(name, PortRight::TaskSend { references: 0 });
                }
                PortRight::GuardedPort { guard, strict, .. } => {
                    self.ports
                        .insert(name, PortRight::GuardedPort { guard, strict, send_references: 0 });
                }
                PortRight::HostSend { .. } => {
                    self.ports.remove(&name);
                    self.host_name = None;
                }
                PortRight::ClockSend { .. } => {
                    self.ports.remove(&name);
                    self.system_clock_name = None;
                }
                PortRight::SemaphoreSend { .. } => {
                    self.ports.remove(&name);
                }
                PortRight::ThreadSend { thread, .. } => {
                    self.ports.remove(&name);
                    self.threads.insert(thread, None);
                }
                PortRight::BootstrapSend { .. } => {
                    self.ports.remove(&name);
                    self.bootstrap_name = None;
                }
                PortRight::DeadName { .. } => {
                    self.ports.remove(&name);
                }
                _ => unreachable!(),
            }
        }
        0
    }

    /// _kernelrpc_mach_port_mod_refs_trap (-19), current-task namespace only.
    /// Modifies user references for send rights (right=0) or dead names (right=4).
    pub fn mod_refs(&mut self, task: u32, name: u32, right: u32, delta: i32) -> u32 {
        if !matches!(self.right(task), Some(PortRight::TaskSend { references }) if references > 0) {
            return 0x10000003; // MACH_SEND_INVALID_DEST
        }
        if name == 0 || name == u32::MAX {
            return 15; // KERN_INVALID_NAME
        }
        if self.clock_reply.is_some_and(|(_, pending)| pending == name)
            || self.bootstrap_reply.is_some_and(|(_, pending)| pending == name)
        {
            return 5; /* synchronous copyout owns this name */
        }
        let Some(port_right) = self.right(name) else {
            return 15; // KERN_INVALID_NAME
        };
        match right {
            0 => {
                // MACH_PORT_RIGHT_SEND
                let references = match port_right {
                    PortRight::TaskSend { references }
                    | PortRight::HostSend { references }
                    | PortRight::ClockSend { references, .. }
                    | PortRight::SemaphoreSend { references, .. }
                    | PortRight::ThreadSend { references, .. }
                    | PortRight::BootstrapSend { references }
                    | PortRight::GuardedPort { send_references: references, .. } => references,
                    PortRight::DeadName { .. }
                    | PortRight::ReplyReceive
                    | PortRight::ConstructedReplyReceive => return 17, // KERN_INVALID_RIGHT
                };
                if references == 65535 && delta >= 0 {
                    return 0; // pegged
                }
                let new_refs = (references as i64) + (delta as i64);
                if new_refs < 0 {
                    return 17; // KERN_INVALID_RIGHT
                }
                if new_refs == 0 {
                    match port_right {
                        PortRight::TaskSend { .. } => {
                            self.ports
                                .insert(name, PortRight::TaskSend { references: 0 });
                        }
                        PortRight::GuardedPort { guard, strict, .. } => {
                            self.ports
                                .insert(name, PortRight::GuardedPort { guard, strict, send_references: 0 });
                        }
                        PortRight::HostSend { .. } => {
                            self.ports.remove(&name);
                            self.host_name = None;
                        }
                        PortRight::ClockSend { .. } => {
                            self.ports.remove(&name);
                            self.system_clock_name = None;
                        }
                        PortRight::SemaphoreSend { .. } => {
                            self.ports.remove(&name);
                        }
                        PortRight::ThreadSend { thread, .. } => {
                            self.ports.remove(&name);
                            self.threads.insert(thread, None);
                        }
                        PortRight::BootstrapSend { .. } => {
                            self.ports.remove(&name);
                            self.bootstrap_name = None;
                        }
                        _ => unreachable!(),
                    }
                } else {
                    let clamped = (new_refs as u32).min(65535);
                    let entry = self.ports.get_mut(&name).unwrap();
                    match entry {
                        PortRight::TaskSend { references }
                        | PortRight::HostSend { references }
                        | PortRight::ClockSend { references, .. }
                        | PortRight::SemaphoreSend { references, .. }
                        | PortRight::ThreadSend { references, .. }
                        | PortRight::BootstrapSend { references }
                        | PortRight::GuardedPort { send_references: references, .. } => *references = clamped,
                        _ => unreachable!(),
                    }
                }
                0
            }
            4 => {
                // MACH_PORT_RIGHT_DEAD_NAME
                let references = match port_right {
                    PortRight::DeadName { references } => references,
                    _ => return 17, // KERN_INVALID_RIGHT
                };
                let new_refs = (references as i64) + (delta as i64);
                if new_refs < 0 {
                    return 17;
                }
                if new_refs == 0 {
                    self.ports.remove(&name);
                } else {
                    let clamped = (new_refs as u32).min(65535);
                    if let Some(PortRight::DeadName { references }) = self.ports.get_mut(&name) {
                        *references = clamped;
                    }
                }
                0
            }
            _ => 17, // KERN_INVALID_RIGHT
        }
    }

    pub fn trap(&mut self, number: i64) -> Result<u32, String> {
        if !matches!(number, -29 | -28 | -26) {
            return Err(format!("Unsupported Mach identity trap {number}"));
        }
        if !self.alive {
            return Ok(0);
        }
        match number {
            -29 => {
                if let Some(name) = self.host_name {
                    let Some(PortRight::HostSend { references }) = self.ports.get_mut(&name) else {
                        return Err("virtual Mach host send identity inconsistent".into());
                    };
                    *references = references.saturating_add(1).min(65535);
                    return Ok(name);
                }
                if self.occupied_ports() >= self.max_ports {
                    return Ok(0);
                }
                let name = self.next_name;
                let Some(next) = name.checked_add(NAME_INDEX_STEP) else {
                    return Ok(0);
                };
                self.ports
                    .insert(name, PortRight::HostSend { references: 1 });
                self.host_name = Some(name);
                self.next_name = next;
                Ok(name)
            }
            -28 => {
                let PortRight::TaskSend { references } = self.ports.get_mut(&TASK_NAME).unwrap()
                else {
                    unreachable!()
                };
                // XNU ipc_right_copyout_any_send pegs user references at the
                // 16-bit maximum; it still returns the same valid send name.
                *references = references.saturating_add(1).min(65535);
                Ok(TASK_NAME)
            }
            -26 => {
                if self.occupied_ports() >= self.max_ports {
                    return Ok(0);
                }
                let name = self.next_name;
                let Some(next) = name.checked_add(NAME_INDEX_STEP) else {
                    return Ok(0);
                };
                self.ports.insert(name, PortRight::ReplyReceive);
                self.next_name = next;
                Ok(name)
            }
            _ => Err(format!("Unsupported Mach identity trap {number}")),
        }
    }
    /// Register an actual emulator-managed thread before obtaining its Mach
    /// identity. A numeric pthread/thread ID alone is never a port name.
    pub fn register_thread(&mut self, thread: u64) -> Result<(), String> {
        if !self.alive
            || thread == 0
            || self.threads.len() >= 256
            || self.threads.contains_key(&thread)
        {
            return Err("invalid/duplicate/excessive virtual Mach thread identity".into());
        }
        self.threads.insert(thread, None);
        Ok(())
    }
    pub fn thread_self(&mut self, thread: u64) -> Result<u32, String> {
        if !self.alive {
            return Ok(0);
        }
        let current = *self
            .threads
            .get(&thread)
            .ok_or("current virtual Mach thread is not registered")?;
        if let Some(name) = current {
            let Some(PortRight::ThreadSend {
                thread: owner,
                references,
            }) = self.ports.get_mut(&name)
            else {
                return Err("virtual Mach thread send identity inconsistent".into());
            };
            if *owner != thread {
                return Err("virtual Mach thread send ownership mismatch".into());
            }
            *references = references.saturating_add(1).min(65535);
            return Ok(name);
        }
        if self.occupied_ports() >= self.max_ports {
            return Ok(0);
        }
        let name = self.next_name;
        let Some(next) = name.checked_add(NAME_INDEX_STEP) else {
            return Ok(0);
        };
        self.ports.insert(
            name,
            PortRight::ThreadSend {
                thread,
                references: 1,
            },
        );
        self.threads.insert(thread, Some(name));
        self.next_name = next;
        Ok(name)
    }
    pub fn trap_for_thread(&mut self, number: i64, thread: u64) -> Result<u32, String> {
        if number == -27 {
            self.thread_self(thread)
        } else {
            self.trap(number)
        }
    }
    /// Narrow actual _kernelrpc_mach_port_construct_trap subset. The caller
    /// validates all 24 option bytes and the output range before allocation.
    /// Unknown/foreign task names take the real trap's MIG-fallback status.
    pub fn construct_reply(&mut self, task: u32, context: u64, flags: u32) -> Result<u32, u32> {
        const MACH_SEND_INVALID_DEST: u32 = 0x10000003;
        const KERN_INVALID_ARGUMENT: u32 = 4;
        const KERN_NO_SPACE: u32 = 3;
        if !matches!(self.right(task),Some(PortRight::TaskSend{references}) if references>0) {
            return Err(MACH_SEND_INVALID_DEST);
        }
        let right = if flags == 0x1000 && context == 0 {
            PortRight::ConstructedReplyReceive
        } else if flags & !0x31 == 0 && (flags & 1 == 0 || context != 0) {
            let send_references = if flags & 0x10 != 0 { 1 } else { 0 };
            let strict = flags & 0x20 != 0;
            PortRight::GuardedPort { guard: context, strict, send_references }
        } else {
            return Err(KERN_INVALID_ARGUMENT);
        };
        if self.occupied_ports() >= self.max_ports {
            return Err(KERN_NO_SPACE);
        }
        let name = self.next_name;
        let Some(next) = name.checked_add(NAME_INDEX_STEP) else {
            return Err(KERN_NO_SPACE);
        };
        self.ports.insert(name, right);
        self.next_name = next;
        Ok(name)
    }
    /// Original thread termination invalidates its kernel object, leaving
    /// existing task-space send ownership as a dead name. Notification rights
    /// and cross-task transfer remain unsupported.
    pub fn terminate_thread(&mut self, thread: u64) -> Result<(), String> {
        let name = *self
            .threads
            .get(&thread)
            .ok_or("unknown virtual Mach thread termination")?;
        if let Some(name) = name {
            let Some(PortRight::ThreadSend {
                thread: owner,
                references,
            }) = self.ports.get(&name).copied()
            else {
                return Err("virtual Mach thread termination identity inconsistent".into());
            };
            if owner != thread {
                return Err("virtual Mach thread termination ownership mismatch".into());
            }
            self.ports.insert(name, PortRight::DeadName { references });
        }
        self.threads.remove(&thread);
        Ok(())
    }

    /// Destroy only an owned ordinary receive right. No send rights or queued
    /// messages exist for these reply objects in this bounded implementation.
    pub fn destroy_reply(&mut self, name: u32) -> Result<(), String> {
        if self
            .host_replies
            .values()
            .any(|(_, receive)| *receive == name)
        {
            return Err("reply receive right has a pending host RPC".into());
        }
        match self.ports.get(&name) {
            Some(PortRight::ReplyReceive | PortRight::ConstructedReplyReceive | PortRight::GuardedPort { .. }) => {
                self.ports.remove(&name);
                Ok(())
            }
            Some(PortRight::TaskSend { .. }) => {
                Err("cannot destroy the current-task send identity as a reply receive right".into())
            }
            Some(
                PortRight::HostSend { .. }
                | PortRight::ClockSend { .. }
                | PortRight::SemaphoreSend { .. }
                | PortRight::ThreadSend { .. }
                | PortRight::BootstrapSend { .. }
                | PortRight::DeadName { .. },
            ) => Err("non-receive Mach right cannot be destroyed as a reply port".into()),
            None => Err("unknown Mach reply receive right".into()),
        }
    }
    /// Task termination invalidates this task's entire private namespace.
    /// No cross-task message/dead-name notification behavior is claimed.
    pub fn terminate_task(&mut self) {
        self.ports.clear();
        self.threads.clear();
        self.host_name = None;
        self.host_replies.clear();
        self.clock_reply = None;
        self.system_clock_name = None;
        self.semaphore_replies.clear();
        self.bootstrap_reply = None;
        self.bootstrap_name = None;
        self.debug_control_port = None;
        self.alive = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semaphore_copyout_reserves_capacity_and_final_send_release_is_not_object_destroy() {
        let mut mach = MachIdentity::new(3).unwrap();
        let task = mach.trap(-28).unwrap();
        let receive = mach.construct_reply(task, 0, 0x1000).unwrap();
        assert!(mach.prepare_semaphore_reply(receive, receive).is_err());
        let ticket = mach.prepare_semaphore_reply(task, receive).unwrap();
        let canceled = ticket.name();
        assert_eq!(mach.right(canceled), None);
        assert_eq!(mach.trap(-26).unwrap(), 0);
        assert!(mach.destroy_reply(receive).is_err());
        mach.cancel_semaphore_reply(ticket).unwrap();
        let ticket = mach.prepare_semaphore_reply(task, receive).unwrap();
        let name = ticket.name();
        assert_ne!(name, canceled);
        mach.commit_semaphore_reply(ticket).unwrap();
        assert_eq!(
            mach.right(name),
            Some(PortRight::SemaphoreSend {
                task,
                references: 1
            })
        );
        assert_eq!(mach.deallocate(task, name), 0);
        assert_eq!(mach.right(name), None);
        assert_eq!(
            mach.right(receive),
            Some(PortRight::ConstructedReplyReceive)
        );
        let ticket = mach.prepare_semaphore_reply(task, receive).unwrap();
        assert_ne!(ticket.name(), name);
        mach.terminate_task();
        assert!(mach.commit_semaphore_reply(ticket).is_err());
    }
    #[test]
    fn clock_reply_copyout_cancel_commit_and_deallocate_preserve_ownership() {
        let mut mach = MachIdentity::new(4).unwrap();
        let task = mach.trap(-28).unwrap();
        let host = mach.trap(-29).unwrap();
        let reply = mach.construct_reply(task, 0, 0x1000).unwrap();
        let ticket = mach.prepare_system_clock_reply(host, reply).unwrap();
        let canceled = ticket.name();
        assert_eq!(mach.right(canceled), None);
        assert_eq!(mach.trap(-26).unwrap(), 0);
        mach.cancel_clock_reply(ticket).unwrap();
        assert_eq!(mach.right(canceled), None);
        let ticket = mach.prepare_system_clock_reply(host, reply).unwrap();
        let clock = ticket.name();
        assert_ne!(clock, canceled);
        mach.commit_clock_reply(ticket).unwrap();
        assert_eq!(
            mach.right(clock),
            Some(PortRight::ClockSend {
                clock: 0,
                references: 1
            })
        );
        let ticket = mach.prepare_system_clock_reply(host, reply).unwrap();
        assert_eq!(ticket.name(), clock);
        mach.commit_clock_reply(ticket).unwrap();
        assert_eq!(
            mach.right(clock),
            Some(PortRight::ClockSend {
                clock: 0,
                references: 2
            })
        );
        assert_eq!(mach.deallocate(task, clock), 0);
        assert_eq!(mach.deallocate(task, clock), 0);
        assert_eq!(mach.right(clock), None);
        assert_eq!(mach.right(reply), Some(PortRight::ConstructedReplyReceive));
        assert_eq!(
            mach.right(host),
            Some(PortRight::HostSend { references: 1 })
        );
        let ticket = mach.prepare_system_clock_reply(host, reply).unwrap();
        assert_ne!(ticket.name(), clock);
        mach.terminate_task();
        assert!(mach.commit_clock_reply(ticket).is_err());
    }
    #[test]
    fn task_rpc_reply_requires_owned_rights_and_live_unique_endpoint() {
        let mut mach=MachIdentity::new(8).unwrap();
        let task=mach.trap(-28).unwrap();let host=mach.trap(-29).unwrap();
        let receive=mach.construct_reply(task,0,0x1000).unwrap();
        assert!(mach.reserve_task_reply(host,receive).is_err());
        let ticket=mach.reserve_task_reply(task,receive).unwrap();
        assert!(mach.reserve_task_reply(task,receive).is_err());
        mach.cancel_task_reply(ticket).unwrap();
        let ticket=mach.reserve_task_reply(task,receive).unwrap();
        assert_eq!(mach.right(receive),Some(PortRight::ConstructedReplyReceive));
        mach.consume_task_reply(ticket).unwrap();
        let ticket=mach.reserve_task_reply(task,receive).unwrap();
        assert!(mach.destroy_reply(receive).is_err());
        mach.cancel_task_reply(ticket).unwrap();
        mach.destroy_reply(receive).unwrap();
        assert!(mach.reserve_task_reply(task,receive).is_err());
    }
    #[test]
    fn deallocate_host_urefs_preserves_receive_and_inflight_kernel_capability() {
        let mut mach = MachIdentity::new(8).unwrap();
        let task = mach.trap(-28).unwrap();
        let host = mach.trap(-29).unwrap();
        assert_eq!(mach.trap(-29).unwrap(), host);
        let receive = mach.construct_reply(task, 0, 0x1000).unwrap();
        assert_eq!(mach.deallocate(task, host), 0);
        assert_eq!(
            mach.right(host),
            Some(PortRight::HostSend { references: 1 })
        );
        let ticket = mach.reserve_host_reply(host, receive).unwrap();
        assert_eq!(mach.deallocate(task, host), 0);
        assert_eq!(mach.right(host), None);
        mach.consume_host_reply(ticket).unwrap();
        assert_eq!(
            mach.right(receive),
            Some(PortRight::ConstructedReplyReceive)
        );
        assert_eq!(mach.deallocate(task, host), 15);
        let fresh = mach.trap(-29).unwrap();
        assert_ne!(fresh, host);
        assert_eq!(mach.deallocate(fresh, fresh), 0x10000003);
        assert_eq!(mach.deallocate(task, receive), 17);
        assert_eq!(mach.deallocate(task, 0), 0);
        assert_eq!(mach.deallocate(task, u32::MAX), 0);
    }
    #[test]
    fn deallocate_pegged_thread_dead_and_task_rights() {
        let mut mach = MachIdentity::new(8).unwrap();
        let task = mach.trap(-28).unwrap();
        let host = mach.trap(-29).unwrap();
        mach.ports
            .insert(host, PortRight::HostSend { references: 65535 });
        assert_eq!(mach.deallocate(task, host), 0);
        assert_eq!(
            mach.right(host),
            Some(PortRight::HostSend { references: 65535 })
        );
        mach.register_thread(1).unwrap();
        let first = mach.thread_self(1).unwrap();
        assert_eq!(mach.deallocate(task, first), 0);
        let second = mach.thread_self(1).unwrap();
        assert_ne!(first, second);
        mach.terminate_thread(1).unwrap();
        assert_eq!(mach.deallocate(task, second), 0);
        assert_eq!(mach.right(second), None);
        assert_eq!(mach.deallocate(task, task), 0);
        assert_eq!(mach.right(task), None);
        assert_eq!(mach.trap(-28).unwrap(), task);
    }
    #[test]
    fn host_rpc_send_once_delivery_and_cancel_preserve_user_rights() {
        let mut mach = MachIdentity::new(8).unwrap();
        let task = mach.trap(-28).unwrap();
        let host = mach.trap(-29).unwrap();
        let receive = mach.construct_reply(task, 0, 0x1000).unwrap();
        let ordinary = mach.trap(-26).unwrap();
        let rights = mach.ports.clone();
        assert!(mach.reserve_host_reply(task, receive).is_err());
        let ordinary_ticket=mach.reserve_host_reply(host,ordinary).unwrap();
        assert!(mach.destroy_reply(ordinary).is_err());
        mach.consume_host_reply(ordinary_ticket).unwrap();
        assert_eq!(mach.right(ordinary),Some(PortRight::ReplyReceive));
        let ticket = mach.reserve_host_reply(host, receive).unwrap();
        assert!(mach.reserve_host_reply(host, receive).is_err());
        assert!(mach.destroy_reply(receive).is_err());
        mach.cancel_host_reply(ticket).unwrap();
        assert_eq!(mach.ports, rights);
        let ticket = mach.reserve_host_reply(host, receive).unwrap();
        let stale = HostReplyTicket {
            namespace: ticket.namespace,
            nonce: ticket.nonce,
        };
        mach.consume_host_reply(ticket).unwrap();
        assert!(mach.consume_host_reply(stale).is_err());
        assert_eq!(mach.ports, rights);
        mach.destroy_reply(receive).unwrap();
    }
    #[test]
    fn host_rpc_tickets_reject_foreign_namespace_and_task_termination() {
        let mut mach = MachIdentity::new(8).unwrap();
        let task = mach.trap(-28).unwrap();
        let host = mach.trap(-29).unwrap();
        let receive = mach.construct_reply(task, 0, 0x1000).unwrap();
        let ticket = mach.reserve_host_reply(host, receive).unwrap();
        let forged = HostReplyTicket {
            namespace: ticket.namespace,
            nonce: ticket.nonce,
        };
        let mut other = MachIdentity::new(8).unwrap();
        assert!(other.consume_host_reply(forged).is_err());
        assert_eq!(mach.host_replies.len(), 1);
        mach.terminate_task();
        assert!(mach.consume_host_reply(ticket).is_err());
        assert!(mach.host_replies.is_empty());
    }
    #[test]
    fn constructed_reply_has_exact_task_ownership_role_and_lifecycle() {
        let mut mach = MachIdentity::new(8).unwrap();
        assert_eq!(mach.construct_reply(TASK_NAME, 0, 0x1000), Err(0x10000003));
        let task = mach.trap(-28).unwrap();
        let host = mach.trap(-29).unwrap();
        let original = mach.ports.clone();
        let next = mach.next_name;
        assert_eq!(mach.construct_reply(host, 0, 0x1000), Err(0x10000003));
        assert_eq!(mach.construct_reply(task, 1, 0x1000), Err(4));
        assert_eq!(mach.construct_reply(task, 0, 0x400), Err(4));
        assert_eq!(mach.ports, original);
        assert_eq!(mach.next_name, next);
        let ordinary = mach.trap(-26).unwrap();
        let constructed = mach.construct_reply(task, 0, 0x1000).unwrap();
        assert_ne!(ordinary, constructed);
        assert_eq!(constructed & 3, 3);
        assert_eq!(mach.right(ordinary), Some(PortRight::ReplyReceive));
        assert_eq!(
            mach.right(constructed),
            Some(PortRight::ConstructedReplyReceive)
        );
        mach.destroy_reply(constructed).unwrap();
        assert_eq!(mach.right(constructed), None);
        assert!(mach.destroy_reply(constructed).is_err());
    }
    #[test]
    fn constructed_reply_resource_failure_does_not_publish_right() {
        let mut mach = MachIdentity::new(1).unwrap();
        let task = mach.trap(-28).unwrap();
        let next = mach.next_name;
        assert_eq!(mach.construct_reply(task, 0, 0x1000), Err(3));
        assert_eq!(mach.ports.len(), 1);
        assert_eq!(mach.next_name, next);
        mach.terminate_task();
        assert_eq!(mach.construct_reply(task, 0, 0x1000), Err(0x10000003));
    }
    #[test]
    fn host_send_name_is_distinct_owned_coalesced_and_resource_bounded() {
        let mut mach = MachIdentity::new(8).unwrap();
        let task = mach.trap(-28).unwrap();
        let reply = mach.trap(-26).unwrap();
        mach.register_thread(1).unwrap();
        let thread = mach.thread_self(1).unwrap();
        let host = mach.trap(-29).unwrap();
        assert_eq!(host, 0x403);
        assert_eq!(host & 3, 3);
        assert!(![task, reply, thread].contains(&host));
        assert_eq!(mach.trap(-29).unwrap(), host);
        assert_eq!(
            mach.right(host),
            Some(PortRight::HostSend { references: 2 })
        );
        assert!(mach.destroy_reply(host).is_err());
        mach.terminate_task();
        assert_eq!(mach.right(host), None);
        assert_eq!(mach.trap(-29).unwrap(), 0);
        let mut full = MachIdentity::new(1).unwrap();
        assert_eq!(full.trap(-29).unwrap(), 0);
        assert_eq!(full.host_name, None);
    }
    #[test]
    fn current_thread_send_right_is_owned_distinct_and_becomes_dead_on_exit() {
        let mut mach = MachIdentity::new(8).unwrap();
        assert!(mach.thread_self(1).is_err());
        mach.register_thread(1).unwrap();
        mach.register_thread(2).unwrap();
        let first = mach.thread_self(1).unwrap();
        assert_eq!(mach.trap_for_thread(-27, 1).unwrap(), first);
        let second = mach.thread_self(2).unwrap();
        assert_eq!(first & 3, 3);
        assert_eq!(second & 3, 3);
        assert_eq!((first >> 8) + 1, second >> 8);
        assert_ne!(first, second);
        assert_eq!(
            mach.right(first),
            Some(PortRight::ThreadSend {
                thread: 1,
                references: 2
            })
        );
        assert!(mach.destroy_reply(first).is_err());
        mach.terminate_thread(1).unwrap();
        assert_eq!(
            mach.right(first),
            Some(PortRight::DeadName { references: 2 })
        );
        assert!(mach.thread_self(1).is_err());
        assert_eq!(
            mach.right(second),
            Some(PortRight::ThreadSend {
                thread: 2,
                references: 1
            })
        );
        assert!(mach.register_thread(2).is_err());
    }
    #[test]
    fn thread_port_exhaustion_does_not_publish_an_unowned_identity() {
        let mut mach = MachIdentity::new(1).unwrap();
        mach.register_thread(1).unwrap();
        assert_eq!(mach.thread_self(1).unwrap(), 0);
        assert_eq!(mach.threads[&1], None);
        assert!(mach.register_thread(0).is_err());
    }
    #[test]
    fn actual_init_traps_establish_distinct_rights() {
        let mut mach = MachIdentity::new(8).unwrap();
        let task = mach.trap(-28).unwrap();
        assert_eq!(mach.trap(-28).unwrap(), task);
        assert_eq!(
            mach.right(task),
            Some(PortRight::TaskSend { references: 2 })
        );
        let reply = mach.trap(-26).unwrap();
        let other_reply = mach.trap(-26).unwrap();
        assert_eq!([task, reply, other_reply], [0x103, 0x203, 0x303]);
        for name in [task, reply, other_reply] {
            assert_eq!(name & 3, 3);
        }
        assert_ne!(task, reply);
        assert_ne!(reply, other_reply);
        assert_eq!(mach.right(reply), Some(PortRight::ReplyReceive));
    }
    #[test]
    fn unsupported_and_resource_failure_preserve_namespace() {
        let mut mach = MachIdentity::new(2).unwrap();
        let reply = mach.trap(-26).unwrap();
        assert_eq!(mach.trap(-26).unwrap(), 0);
        assert!(mach.trap(-27).is_err());
        assert_eq!(mach.right(reply), Some(PortRight::ReplyReceive));
        assert_eq!(mach.ports.len(), 2);
        assert!(MachIdentity::new(0).is_err());
    }
    #[test]
    fn send_references_peg_at_kernel_limit() {
        let mut mach = MachIdentity::new(2).unwrap();
        mach.ports
            .insert(TASK_NAME, PortRight::TaskSend { references: 65535 });
        assert_eq!(mach.trap(-28).unwrap(), TASK_NAME);
        assert_eq!(
            mach.right(TASK_NAME),
            Some(PortRight::TaskSend { references: 65535 })
        );
    }
    #[test]
    fn reply_destruction_reclaims_capacity_without_reusing_stale_names() {
        let mut mach = MachIdentity::new(2).unwrap();
        assert_eq!(mach.right(TASK_NAME), None);
        let task = mach.trap(-28).unwrap();
        let old = mach.trap(-26).unwrap();
        assert!(mach.destroy_reply(task).is_err());
        assert_eq!(mach.trap(-26).unwrap(), 0);
        mach.destroy_reply(old).unwrap();
        assert!(mach.destroy_reply(old).is_err());
        let new = mach.trap(-26).unwrap();
        assert_ne!(old, new);
        assert_eq!(mach.right(old), None);
        mach.terminate_task();
        assert_eq!(mach.right(task), None);
        assert_eq!(mach.trap(-28).unwrap(), 0);
        assert_eq!(mach.trap(-26).unwrap(), 0);
        assert!(mach.trap(-27).is_err());
        assert!(MachIdentity::new(4097).is_err());
    }
    #[test]
    fn bootstrap_special_port_delivery_deallocate_and_preserve_ownership() {
        use super::super::A64Cpu;
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 0x4000, 3).unwrap();
        let mut mach = MachIdentity::new(4).unwrap();
        let task = mach.trap(-28).unwrap();
        let receive = mach.construct_reply(task, 0, 0x1000).unwrap();
        let mut request = [0u8; 36];
        for (offset, value) in [(0, 0x1513u32), (4, 36), (8, task), (12, receive), (20, 3409), (32, 4)] {
            request[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        request[24..32].copy_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0]);
        cpu.write_guest_into(0x10000, &request).unwrap();
        let args = [
            0x10000,
            0x200000003,
            0x2400001513,
            (u64::from(receive) << 32) | u64::from(task),
            3409u64 << 32,
            u64::from(receive) << 32,
            48,
            0,
        ];
        assert_eq!(mach.reply_task_special_port(&mut cpu, args).unwrap(), 0);
        let read_u32 = |cpu: &A64Cpu, addr: u64| {
            u32::from_le_bytes(cpu.read_bytes(addr, 4).unwrap().try_into().unwrap())
        };
        let reply_bits = read_u32(&cpu, 0x10000);
        assert_eq!(reply_bits, 0x80001200);
        let reply_size = read_u32(&cpu, 0x10004);
        assert_eq!(reply_size, 40);
        let reply_id = read_u32(&cpu, 0x10014);
        assert_eq!(reply_id, 3509);
        let desc_count = read_u32(&cpu, 0x10018);
        assert_eq!(desc_count, 1);
        let port_name = read_u32(&cpu, 0x1001c);
        assert_ne!(port_name, 0);
        assert_eq!(mach.right(port_name), Some(PortRight::BootstrapSend { references: 1 }));
        assert_eq!(mach.deallocate(task, port_name), 0);
        assert_eq!(mach.right(port_name), None);
    }
    #[test]
    fn mod_refs_delta_lifecycle_and_validation() {
        let mut mach = MachIdentity::new(4).unwrap();
        let task = mach.trap(-28).unwrap();
        let host = mach.trap(-29).unwrap();
        assert_eq!(mach.right(host), Some(PortRight::HostSend { references: 1 }));
        // Add 2 references: delta=+2
        assert_eq!(mach.mod_refs(task, host, 0, 2), 0);
        assert_eq!(mach.right(host), Some(PortRight::HostSend { references: 3 }));
        // Drop 1 reference: delta=-1
        assert_eq!(mach.mod_refs(task, host, 0, -1), 0);
        assert_eq!(mach.right(host), Some(PortRight::HostSend { references: 2 }));
        // Invalid right
        assert_eq!(mach.mod_refs(task, host, 1, 1), 17);
        // Foreign task
        assert_eq!(mach.mod_refs(host, host, 0, 1), 0x10000003);
        // Drop 2 references: delta=-2 -> removes right
        assert_eq!(mach.mod_refs(task, host, 0, -2), 0);
        assert_eq!(mach.right(host), None);
        // Invalid name now
        assert_eq!(mach.mod_refs(task, host, 0, 1), 15);
    }
    #[test]
    fn task_set_special_port_lifecycle_and_validation() {
        use super::super::A64Cpu;
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 0x4000, 3).unwrap();
        let mut mach = MachIdentity::new(8).unwrap();
        let task = mach.trap(-28).unwrap();
        let receive = mach.construct_reply(task, 0, 0x1000).unwrap();
        let guarded = mach.construct_reply(task, 0x1234, 0x31).unwrap();
        assert_eq!(mach.right(guarded), Some(PortRight::GuardedPort { guard: 0x1234, strict: true, send_references: 1 }));

        let mut request = [0u8; 52];
        for (offset, value) in [
            (0, 0x80001513u32),
            (4, 52),
            (8, task),
            (12, receive),
            (20, 3410),
            (24, 1),
            (28, guarded),
            (48, 10), // TASK_DEBUG_CONTROL_PORT
        ] {
            request[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        request[38] = 19; // MACH_MSG_TYPE_MOVE_SEND
        request[40..48].copy_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0]); // NDR
        cpu.write_guest_into(0x10000, &request).unwrap();

        let args = [
            0x10000,
            0x200000003,
            (52u64 << 32) | 0x80001513,
            (u64::from(receive) << 32) | u64::from(task),
            3410u64 << 32,
            (u64::from(receive) << 32) | 1,
            44,
            0,
        ];
        assert_eq!(mach.reply_task_set_special_port(&mut cpu, args).unwrap(), 0);
        let read_u32 = |cpu: &A64Cpu, addr: u64| {
            u32::from_le_bytes(cpu.read_bytes(addr, 4).unwrap().try_into().unwrap())
        };
        assert_eq!(read_u32(&cpu, 0x10000), 0x1200); // msgh_bits
        assert_eq!(read_u32(&cpu, 0x10004), 36);     // msgh_size
        assert_eq!(read_u32(&cpu, 0x1000c), receive); // msgh_local_port
        assert_eq!(read_u32(&cpu, 0x10014), 3510);   // reply msgh_id
        assert_eq!(read_u32(&cpu, 0x10020), 0);      // RetCode = KERN_SUCCESS
        assert_eq!(read_u32(&cpu, 0x10028), 8);      // trailer size

        // Guarded port's send reference was moved into kernel debug_control_port slot
        assert_eq!(mach.right(guarded), Some(PortRight::GuardedPort { guard: 0x1234, strict: true, send_references: 0 }));
    }
}

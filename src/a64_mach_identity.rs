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
    DeadName {
        references: u32,
    },
    ReplyReceive,
    /// Task-space receive right constructed with MPO_REPLY_PORT. This is not
    /// the separately thread-bound special reply port or an inserted send right.
    ConstructedReplyReceive,
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
}

impl MachIdentity {
    fn occupied_ports(&self) -> usize {
        self.ports.len()
            + usize::from(
                self.clock_reply
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

    /// _kernelrpc_mach_port_deallocate_trap (-18), current-task namespace only.
    /// Receive-only rights survive; this operation consumes send/dead-name urefs.
    pub fn deallocate(&mut self, task: u32, name: u32) -> u32 {
        if !matches!(self.right(task), Some(PortRight::TaskSend { references }) if references > 0) {
            return 0x10000003; // trap-specific MACH_SEND_INVALID_DEST
        }
        if name == 0 || name == u32::MAX {
            return 0;
        }
        if self.clock_reply.is_some_and(|(_, pending)| pending == name) {
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
                | PortRight::DeadName { references } => *references -= 1,
                _ => unreachable!(),
            }
        } else {
            match right {
                PortRight::TaskSend { .. } => {
                    self.ports
                        .insert(name, PortRight::TaskSend { references: 0 });
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
                PortRight::DeadName { .. } => {
                    self.ports.remove(&name);
                }
                _ => unreachable!(),
            }
        }
        0
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
        if flags != 0x1000 || context != 0 {
            return Err(KERN_INVALID_ARGUMENT);
        }
        if self.occupied_ports() >= self.max_ports {
            return Err(KERN_NO_SPACE);
        }
        let name = self.next_name;
        let Some(next) = name.checked_add(NAME_INDEX_STEP) else {
            return Err(KERN_NO_SPACE);
        };
        self.ports.insert(name, PortRight::ConstructedReplyReceive);
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
            Some(PortRight::ReplyReceive | PortRight::ConstructedReplyReceive) => {
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
}

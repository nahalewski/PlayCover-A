/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Exact synchronous Mach host_info / HOST_PRIORITY_INFO service.
//! This exposes a declared virtual Darwin priority policy, not Android host
//! scheduler statistics. Other RPCs, transfers, queues and timeouts are absent.
//! Source contracts: apple-xnu mach_host.defs, host_info.h, kern/host.c,
//! kern/sched.h, mach/message.h and ipc/ipc_kmsg.c receiver header copyout.
use super::{mach_identity::{MachIdentity, PortRight}, A64Cpu};

const NDR: [u8; 8] = [0, 0, 0, 0, 1, 0, 0, 0];
const REQUEST_SIZE: u32 = 40;
const REPLY_SIZE: u32 = 72;
const COPYOUT_SIZE: usize = REPLY_SIZE as usize + 8;

/// Policy values match Darwin's documented priority domain. The host kernel's
/// effective Linux priorities are deliberately not used as Mach ABI numbers.
#[derive(Clone, Copy, Debug)]
pub struct HostPriorityPolicy {
    words: [i32; 8],
}

impl HostPriorityPolicy {
    pub fn virtual_darwin() -> Self {
        // kernel, system, server, user, depressed, idle, minimum, maximum.
        // Maximum is MAXPRI_RESERVED (79), not MAXPRI_USER (63).
        Self { words: [80, 80, 64, 31, 0, 0, 0, 79] }
    }
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub fn reply(
    cpu: &mut A64Cpu,
    ports: &mut MachIdentity,
    args: [u64; 8],
    policy: &HostPriorityPolicy,
) -> Result<u32, String> {
    reply_inner(cpu,ports,args,policy,false)
}
/// Old mach_msg supplies size in the scalar trap argument; its copied header
/// may retain zero. This is not permission to relax modern mach_msg2 validation.
pub(super) fn reply_legacy(cpu:&mut A64Cpu,ports:&mut MachIdentity,args:[u64;8],policy:&HostPriorityPolicy)->Result<u32,String>{
    reply_inner(cpu,ports,args,policy,true)
}
fn reply_inner(cpu:&mut A64Cpu,ports:&mut MachIdentity,args:[u64;8],policy:&HostPriorityPolicy,legacy:bool)->Result<u32,String>{
    let [data, options, bits_size, remote_local, voucher_id, descriptors_receive, receive_priority, timeout] = args;
    let host = remote_local as u32;
    let receive = (remote_local >> 32) as u32;
    let capacity = receive_priority as u32;
    if options != 0x0000_0002_0000_0003
        || bits_size != ((REQUEST_SIZE as u64) << 32 | 0x1513)
        || voucher_id != 200u64 << 32
        || descriptors_receive != (receive as u64) << 32
        || receive_priority >> 32 != 0
        || capacity < COPYOUT_SIZE as u32 || capacity > 4096
        || timeout != 0
    {
        return Err("Unsupported host_info Mach message envelope/options".into());
    }
    if !matches!(ports.right(host), Some(PortRight::HostSend { references }) if references > 0) {
        return Ok(0x1000_0003); // MACH_SEND_INVALID_DEST
    }
    if !matches!(ports.right(receive), Some(PortRight::ConstructedReplyReceive))
        && !(legacy&&matches!(ports.right(receive),Some(PortRight::ReplyReceive))) {
        return Ok(0x1000_0009); // MACH_SEND_INVALID_REPLY
    }
    let mut request = [0; REQUEST_SIZE as usize];
    cpu.read_guest_into(data, &mut request)?;
    if word(&request, 0) != 0x1513 || (!legacy&&word(&request,4)!=REQUEST_SIZE)
        || word(&request, 8) != host || word(&request, 12) != receive
        || word(&request, 16) != 0 || word(&request, 20) != 200
        || request[24..32] != NDR || word(&request, 32) != 5 || word(&request, 36) != 8
    {
        return Err("Unsupported host_info request body/header/NDR".into());
    }
    // Check the declared receive buffer before acquiring the transient reply
    // send-once capability. No queue or pending reply is invented on failure.
    cpu.validate_guest_write(data, capacity as usize)?;
    let mut response = [0; COPYOUT_SIZE];
    // Kernel receiver copyout swaps sender remote/local dispositions and ports.
    // Destination SEND_ONCE(18) becomes local bits; no reply remote port exists.
    put(&mut response, 0, 0x1200);
    put(&mut response, 4, REPLY_SIZE);
    put(&mut response, 12, receive);
    put(&mut response, 20, 300);
    response[24..32].copy_from_slice(&NDR);
    put(&mut response, 32, 0); // Genuine completed HOST_PRIORITY_INFO operation.
    put(&mut response, 36, 8);
    for (index, value) in policy.words.iter().enumerate() {
        response[40 + index * 4..44 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    // Default minimum trailer: type MACH_MSG_TRAILER_FORMAT_0, size 8.
    put(&mut response, REPLY_SIZE as usize + 4, 8);
    let ticket = ports.reserve_host_reply(host, receive)?;
    if let Err(error) = cpu.write_guest_into(data, &response) {
        ports.cancel_host_reply(ticket)?;
        return Err(error);
    }
    ports.consume_host_reply(ticket)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (A64Cpu, MachIdentity, [u64; 8]) {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 0x4000, 3).unwrap();
        let mut ports = MachIdentity::new(8).unwrap();
        let task = ports.trap(-28).unwrap();
        let host = ports.trap(-29).unwrap();
        let receive = ports.construct_reply(task, 0, 0x1000).unwrap();
        let mut request = [0; 40];
        for (offset, value) in [(0, 0x1513), (4, 40), (8, host), (12, receive), (20, 200), (32, 5), (36, 8)] {
            put(&mut request, offset, value);
        }
        request[24..32].copy_from_slice(&NDR);
        cpu.write_guest_into(0x10000, &request).unwrap();
        (cpu, ports, [0x10000, 0x200000003, 0x2800001513,
                     (receive as u64) << 32 | host as u64,
                     200u64 << 32, (receive as u64) << 32, 320, 0])
    }
    #[test]
    fn genuine_priority_response_header_trailer_and_rights() {
        let (mut cpu, mut ports, args) = fixture();
        let host = args[3] as u32;
        let receive = (args[3] >> 32) as u32;
        let host_before = ports.right(host);
        assert_eq!(reply(&mut cpu, &mut ports, args, &HostPriorityPolicy::virtual_darwin()).unwrap(), 0);
        let mut response = [0; 80];
        cpu.read_guest_into(args[0], &mut response).unwrap();
        assert_eq!(word(&response, 0), 0x1200);
        assert_eq!(word(&response, 4), 72);
        assert_eq!(word(&response, 8), 0);
        assert_eq!(word(&response, 12), receive);
        assert_eq!(word(&response, 20), 300);
        assert_eq!(&response[24..32], &NDR);
        assert_eq!(word(&response, 32), 0);
        assert_eq!(word(&response, 36), 8);
        assert_eq!((0..8).map(|i| word(&response, 40 + i * 4)).collect::<Vec<_>>(), [80,80,64,31,0,0,0,79]);
        assert_eq!(word(&response, 72), 0);
        assert_eq!(word(&response, 76), 8);
        assert_eq!(ports.right(host), host_before);
        assert_eq!(ports.right(receive), Some(PortRight::ConstructedReplyReceive));
    }
    #[test]
    fn unknown_rpc_and_envelope_fail_without_response() {
        let (mut cpu, mut ports, mut args) = fixture();
        cpu.write_guest_into(args[0] + 32, &1u32.to_le_bytes()).unwrap();
        assert!(reply(&mut cpu, &mut ports, args, &HostPriorityPolicy::virtual_darwin()).is_err());
        let mut header = [0; 24];
        cpu.read_guest_into(args[0], &mut header).unwrap();
        assert_eq!(word(&header, 20), 200);
        args[1] |= 0x100;
        assert!(reply(&mut cpu, &mut ports, args, &HostPriorityPolicy::virtual_darwin()).is_err());
    }
    #[test]
    fn readonly_receive_buffer_cannot_deliver_or_consume_rights() {
        let (mut cpu, mut ports, mut args) = fixture();
        let mut request = [0; 40];
        cpu.read_guest_into(args[0], &mut request).unwrap();
        cpu.map_zeroed(0x20000, 0x4000, 1).unwrap();
        cpu.try_write_bytes(0x20000, &request).unwrap();
        args[0] = 0x20000;
        assert!(reply(&mut cpu, &mut ports, args, &HostPriorityPolicy::virtual_darwin()).is_err());
        let mut after = [0; 40];
        cpu.read_guest_into(args[0], &mut after).unwrap();
        assert_eq!(after, request);
        assert_eq!(ports.right((args[3] >> 32) as u32), Some(PortRight::ConstructedReplyReceive));
    }
}

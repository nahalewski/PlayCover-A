/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Exact synchronous Mach host_set_atm_diagnostic_flag (225) service.
//! Source contracts: apple-xnu mach_host.defs, mach/mach_host.h.

use super::{mach_identity::{MachIdentity, PortRight}, A64Cpu};

const NDR: [u8; 8] = [0, 0, 0, 0, 1, 0, 0, 0];
const REQUEST_SIZE: u32 = 36;
const REPLY_SIZE: u32 = 36;
const COPYOUT_SIZE: usize = REPLY_SIZE as usize + 8; // 44 bytes

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
) -> Result<u32, String> {
    let [data, options, bits_size, remote_local, voucher_id, descriptors_receive, receive_priority, timeout] = args;
    let host = remote_local as u32;
    let receive = (remote_local >> 32) as u32;
    let capacity = receive_priority as u32;
    if options != 0x0000_0002_0000_0003
        || bits_size != ((REQUEST_SIZE as u64) << 32 | 0x1513)
        || voucher_id != 225u64 << 32
        || descriptors_receive != (receive as u64) << 32
        || receive_priority >> 32 != 0
        || capacity < COPYOUT_SIZE as u32 || capacity > 4096
        || timeout != 0
    {
        return Err("Unsupported host_set_atm_diagnostic_flag Mach message envelope/options".into());
    }
    if !matches!(ports.right(host), Some(PortRight::HostSend { references }) if references > 0) {
        return Ok(0x1000_0003); // MACH_SEND_INVALID_DEST
    }
    if !matches!(ports.right(receive), Some(PortRight::ConstructedReplyReceive)) {
        return Ok(0x1000_0009); // MACH_SEND_INVALID_REPLY
    }
    let mut request = [0; REQUEST_SIZE as usize];
    cpu.read_guest_into(data, &mut request)?;
    if word(&request, 0) != 0x1513
        || word(&request, 4) != REQUEST_SIZE
        || word(&request, 8) != host
        || word(&request, 12) != receive
        || word(&request, 16) != 0
        || word(&request, 20) != 225
        || request[24..32] != NDR
    {
        return Err("Unsupported host_set_atm_diagnostic_flag request body/header/NDR".into());
    }
    let flag = word(&request, 32);

    cpu.validate_guest_write(data, capacity as usize)?;
    let mut response = [0; COPYOUT_SIZE];
    put(&mut response, 0, 0x1200);
    put(&mut response, 4, REPLY_SIZE);
    put(&mut response, 12, receive);
    put(&mut response, 20, 325); // Reply msg id: 225 + 100
    response[24..32].copy_from_slice(&NDR);
    put(&mut response, 32, 0); // KERN_SUCCESS
    // Trailer: MACH_MSG_TRAILER_FORMAT_0 (0), size 8
    put(&mut response, REPLY_SIZE as usize + 4, 8);

    let ticket = ports.reserve_host_reply(host, receive)?;
    if let Err(error) = cpu.write_guest_into(data, &response) {
        ports.cancel_host_reply(ticket)?;
        return Err(error);
    }
    ports.consume_host_reply(ticket)?;
    echo!("[a64] genuine host_set_atm_diagnostic_flag host={host:#x} flag={flag:#x} -> KERN_SUCCESS");
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
        let mut request = [0; 36];
        for (offset, value) in [(0, 0x1513), (4, 36), (8, host), (12, receive), (20, 225), (32, 1)] {
            put(&mut request, offset, value);
        }
        request[24..32].copy_from_slice(&NDR);
        cpu.write_guest_into(0x10000, &request).unwrap();
        (
            cpu,
            ports,
            [
                0x10000,
                0x200000003,
                0x2400001513,
                (receive as u64) << 32 | host as u64,
                225u64 << 32,
                (receive as u64) << 32,
                44,
                0,
            ],
        )
    }

    #[test]
    fn genuine_atm_diagnostic_response() {
        let (mut cpu, mut ports, args) = fixture();
        let host = args[3] as u32;
        let receive = (args[3] >> 32) as u32;
        let host_before = ports.right(host);
        assert_eq!(reply(&mut cpu, &mut ports, args).unwrap(), 0);
        let mut response = [0; 44];
        cpu.read_guest_into(args[0], &mut response).unwrap();
        assert_eq!(word(&response, 0), 0x1200);
        assert_eq!(word(&response, 4), 36);
        assert_eq!(word(&response, 8), 0);
        assert_eq!(word(&response, 12), receive);
        assert_eq!(word(&response, 20), 325);
        assert_eq!(&response[24..32], &NDR);
        assert_eq!(word(&response, 32), 0);
        assert_eq!(word(&response, 36), 0);
        assert_eq!(word(&response, 40), 8);
        assert_eq!(ports.right(host), host_before);
        assert_eq!(ports.right(receive), Some(PortRight::ConstructedReplyReceive));
    }
}

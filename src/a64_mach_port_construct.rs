/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Exact current-task MIG reply-port construction, kernelrpc trap -24.
//! Actual iOS16 _mig_get_reply_port copies options from 0x1c263bda0:
//! 24 bytes, flags 0x1000 and every remaining byte zero, with context zero.
//! This creates a distinct receive right; no Mach message success is implied.
use super::{mach_identity::{MachIdentity, PortRight}, A64Cpu};

const INVALID_DESTINATION: u32 = 0x1000_0003;

pub fn construct(cpu: &mut A64Cpu, ports: &mut MachIdentity, args: [u64; 4]) -> Result<u32, String> {
    let [task, options_address, context, output] = args;
    let task: u32 = match task.try_into() {
        Ok(name) => name,
        Err(_) => return Ok(INVALID_DESTINATION),
    };
    if !matches!(ports.right(task), Some(PortRight::TaskSend { references }) if references > 0) {
        return Ok(INVALID_DESTINATION);
    }
    let mut options = [0; 24];
    cpu.read_guest_into(options_address, &mut options)?;
    let flags = u32::from_le_bytes(options[..4].try_into().unwrap());
    if flags != 0x1000 || options[4..].iter().any(|&byte| byte != 0) || context != 0 {
        return Err("Unsupported Mach reply construction options/context".into());
    }
    // Validate the entire output before allocating a right. Exclusive CPU access
    // prevents guest changes between this preflight and the final four-byte copy.
    cpu.validate_guest_write(output, 4)?;
    let name = match ports.construct_reply(task, context, flags) {
        Ok(name) => name,
        Err(status) => return Ok(status),
    };
    if let Err(error) = cpu.write_guest_into(output, &name.to_le_bytes()) {
        ports.destroy_reply(name).map_err(|cleanup| format!("{error}; reply cleanup failed: {cleanup}"))?;
        return Err(error);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (A64Cpu, MachIdentity, u64) {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 0x4000, 3).unwrap();
        cpu.write_guest_into(0x10000, &0x1000u32.to_le_bytes()).unwrap();
        let mut ports = MachIdentity::new(8).unwrap();
        let task = ports.trap(-28).unwrap() as u64;
        (cpu, ports, task)
    }
    #[test]
    fn actual_options_create_distinct_owned_right_and_write_four_bytes() {
        let (mut cpu, mut ports, task) = fixture();
        cpu.write_guest_into(0x10100, &[0xff; 8]).unwrap();
        assert_eq!(construct(&mut cpu, &mut ports, [task, 0x10000, 0, 0x10100]).unwrap(), 0);
        let mut output = [0; 8];
        cpu.read_guest_into(0x10100, &mut output).unwrap();
        let name = u32::from_le_bytes(output[..4].try_into().unwrap());
        assert!(ports.right(name).is_some());
        assert!(!matches!(ports.right(name), Some(PortRight::TaskSend { .. }) | Some(PortRight::HostSend { .. })));
        assert_eq!(&output[4..], &[0xff; 4]);
        assert_ne!(name, ports.trap(-26).unwrap());
    }
    #[test]
    fn invalid_output_options_and_task_do_not_consume_names() {
        let (mut cpu, mut ports, task) = fixture();
        assert!(construct(&mut cpu, &mut ports, [task, 0x10000, 0, 0x14000]).is_err());
        assert!(construct(&mut cpu, &mut ports, [task, 0x10000, 1, 0x10100]).is_err());
        cpu.write_guest_into(0x10004, &[1]).unwrap();
        assert!(construct(&mut cpu, &mut ports, [task, 0x10000, 0, 0x10100]).is_err());
        assert_eq!(construct(&mut cpu, &mut ports, [0, 0x10000, 0, 0x10100]).unwrap(), INVALID_DESTINATION);
        // None of the failures above allocated a namespace entry.
        assert_eq!(ports.trap(-26).unwrap(), 0x203);
    }
    #[test]
    fn full_namespace_reports_failure_without_output_writeback() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 0x4000, 3).unwrap();
        cpu.write_guest_into(0x10000, &0x1000u32.to_le_bytes()).unwrap();
        cpu.write_guest_into(0x10100, &0xfeed_beefu32.to_le_bytes()).unwrap();
        let mut ports = MachIdentity::new(1).unwrap();
        let task = ports.trap(-28).unwrap() as u64;
        assert_ne!(construct(&mut cpu, &mut ports, [task, 0x10000, 0, 0x10100]).unwrap(), 0);
        let mut output = [0; 4];
        cpu.read_guest_into(0x10100, &mut output).unwrap();
        assert_eq!(u32::from_le_bytes(output), 0xfeed_beef);
    }
}

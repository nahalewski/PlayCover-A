/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Darwin BSD syscall 202 `sysctl` for arm64 execution session.
use super::A64Cpu;

const CTL_SYSCTL: i32 = 0;
const CTL_SYSCTL_NAME2OID: i32 = 3;

const CTL_KERN: i32 = 1;
const CTL_HW: i32 = 6;

const ENOENT: u64 = 2;
const EINVAL: u64 = 22;
const CARRY: u32 = 1 << 29;

#[derive(Clone)]
enum SysctlValue {
    String(&'static [u8]),
    Int32(i32),
    Int64(i64),
    Bytes(&'static [u8]),
}

struct SysctlEntry {
    name: &'static str,
    mib: &'static [i32],
    value: SysctlValue,
}

static ENTRIES: &[SysctlEntry] = &[
    // HW entries
    SysctlEntry { name: "hw.machine", mib: &[CTL_HW, 1], value: SysctlValue::String(b"iPhone10,4\0") },
    SysctlEntry { name: "hw.model", mib: &[CTL_HW, 2], value: SysctlValue::String(b"D20AP\0") },
    SysctlEntry { name: "hw.ncpu", mib: &[CTL_HW, 3], value: SysctlValue::Int32(8) },
    SysctlEntry { name: "hw.physmem", mib: &[CTL_HW, 5], value: SysctlValue::Int32(2147483647) },
    SysctlEntry { name: "hw.usermem", mib: &[CTL_HW, 6], value: SysctlValue::Int32(2147483647) },
    SysctlEntry { name: "hw.pagesize", mib: &[CTL_HW, 7], value: SysctlValue::Int64(16384) },
    SysctlEntry { name: "hw.busfrequency", mib: &[CTL_HW, 14], value: SysctlValue::Int64(100000000) },
    SysctlEntry { name: "hw.cpufrequency", mib: &[CTL_HW, 15], value: SysctlValue::Int64(2390000000) },
    SysctlEntry { name: "hw.cachelinesize", mib: &[CTL_HW, 16], value: SysctlValue::Int32(64) },
    SysctlEntry { name: "hw.l1icachesize", mib: &[CTL_HW, 17], value: SysctlValue::Int32(65536) },
    SysctlEntry { name: "hw.l1dcachesize", mib: &[CTL_HW, 18], value: SysctlValue::Int32(65536) },
    SysctlEntry { name: "hw.l2cachesize", mib: &[CTL_HW, 19], value: SysctlValue::Int32(4194304) },
    SysctlEntry { name: "hw.memsize", mib: &[CTL_HW, 24], value: SysctlValue::Int64(2147483648) },
    SysctlEntry { name: "hw.tbfrequency", mib: &[CTL_HW, 25], value: SysctlValue::Int64(24000000) },
    SysctlEntry { name: "hw.targettype", mib: &[CTL_HW, 100], value: SysctlValue::String(b"iPhone\0") },
    SysctlEntry { name: "hw.cputype", mib: &[CTL_HW, 101], value: SysctlValue::Int32(0x0100000c) }, // CPU_TYPE_ARM64
    SysctlEntry { name: "hw.cpusubtype", mib: &[CTL_HW, 102], value: SysctlValue::Int32(0) }, // CPU_SUBTYPE_ARM64_ALL
    SysctlEntry { name: "hw.byteorder", mib: &[CTL_HW, 103], value: SysctlValue::Int32(1234) },
    SysctlEntry { name: "hw.logicalcpu", mib: &[CTL_HW, 104], value: SysctlValue::Int32(8) },
    SysctlEntry { name: "hw.physicalcpu", mib: &[CTL_HW, 105], value: SysctlValue::Int32(8) },
    SysctlEntry { name: "hw.activecpu", mib: &[CTL_HW, 106], value: SysctlValue::Int32(8) },

    // KERN entries
    SysctlEntry { name: "kern.ostype", mib: &[CTL_KERN, 1], value: SysctlValue::String(b"Darwin\0") },
    SysctlEntry { name: "kern.osrelease", mib: &[CTL_KERN, 2], value: SysctlValue::String(b"22.6.0\0") },
    SysctlEntry { name: "kern.osrevision", mib: &[CTL_KERN, 3], value: SysctlValue::Int32(199506) },
    SysctlEntry { name: "kern.version", mib: &[CTL_KERN, 4], value: SysctlValue::String(b"Darwin Kernel Version 22.6.0: Mon Jul 31 21:05:40 PDT 2023; root:xnu-8796.142.1~1/RELEASE_ARM64_T8015\0") },
    SysctlEntry { name: "kern.hostname", mib: &[CTL_KERN, 10], value: SysctlValue::String(b"iPhone\0") },
    SysctlEntry { name: "kern.osversion", mib: &[CTL_KERN, 65], value: SysctlValue::String(b"20H392\0") },
    // kern.boottime: struct timeval (8-byte sec, 8-byte usec in LP64)
    SysctlEntry { name: "kern.boottime", mib: &[CTL_KERN, 100], value: SysctlValue::Bytes(&[0x60, 0xb8, 0x54, 0x65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]) },
    SysctlEntry { name: "kern.safeboot", mib: &[CTL_KERN, 101], value: SysctlValue::Int32(0) },
    SysctlEntry { name: "kern.usrstack64", mib: &[CTL_KERN, 102], value: SysctlValue::Int64(0x230000000) },
    SysctlEntry { name: "kern.maxfilesperproc", mib: &[CTL_KERN, 103], value: SysctlValue::Int32(10240) },
    SysctlEntry { name: "kern.bootargs", mib: &[CTL_KERN, 104], value: SysctlValue::String(b"\0") },
    SysctlEntry { name: "kern.bootsince", mib: &[CTL_KERN, 105], value: SysctlValue::Int64(100000) },
    SysctlEntry { name: "kern.secure_kernel", mib: &[CTL_KERN, 106], value: SysctlValue::Int32(0) },
];

fn complete_bsd(cpu: &mut A64Cpu, errno: u64) {
    cpu.set_reg(0, errno);
    let pstate = cpu.pstate();
    cpu.set_pstate(if errno != 0 { pstate | CARRY } else { pstate & !CARRY });
}

pub(super) fn sysctl(cpu: &mut A64Cpu) -> Result<(), String> {
    let name_ptr = cpu.reg(0);
    let namelen = cpu.reg(1) as usize;
    let old_ptr = cpu.reg(2);
    let oldlen_ptr = cpu.reg(3);
    let new_ptr = cpu.reg(4);
    let newlen = cpu.reg(5) as usize;

    if namelen == 0 || namelen > 12 {
        complete_bsd(cpu, EINVAL);
        return Ok(());
    }

    let mut mib = vec![0i32; namelen];
    for (i, item) in mib.iter_mut().enumerate() {
        let mut bytes = [0u8; 4];
        if cpu.read_guest_into(name_ptr + (i as u64 * 4), &mut bytes).is_err() {
            complete_bsd(cpu, EINVAL);
            return Ok(());
        }
        *item = i32::from_le_bytes(bytes);
    }

    // Case 1: CTL_SYSCTL, CTL_SYSCTL_NAME2OID (sysctlbyname name resolution)
    if namelen >= 2 && mib[0] == CTL_SYSCTL && mib[1] == CTL_SYSCTL_NAME2OID {
        if new_ptr == 0 || newlen == 0 || newlen > 256 {
            complete_bsd(cpu, EINVAL);
            return Ok(());
        }
        let mut name_bytes = vec![0u8; newlen];
        if cpu.read_guest_into(new_ptr, &mut name_bytes).is_err() {
            complete_bsd(cpu, EINVAL);
            return Ok(());
        }
        // Strip trailing null if present
        let lookup_bytes = if let Some(&0) = name_bytes.last() {
            &name_bytes[..name_bytes.len() - 1]
        } else {
            &name_bytes[..]
        };
        let name_str = String::from_utf8_lossy(lookup_bytes);

        if let Some(entry) = ENTRIES.iter().find(|e| e.name == name_str) {
            let mib_bytes_len = (entry.mib.len() * 4) as u64;
            if oldlen_ptr != 0 {
                let mut avail_bytes = [0u8; 8];
                let _ = cpu.read_guest_into(oldlen_ptr, &mut avail_bytes);
                let _ = cpu.write_guest_into(oldlen_ptr, &mib_bytes_len.to_le_bytes());
            }
            if old_ptr != 0 {
                for (i, &val) in entry.mib.iter().enumerate() {
                    let _ = cpu.write_guest_into(old_ptr + (i as u64 * 4), &val.to_le_bytes());
                }
            }
            echo!("[a64] genuine sysctl name2oid resolved {name_str} -> {:?}", entry.mib);
            complete_bsd(cpu, 0);
            return Ok(());
        } else {
            echo!("[a64] genuine sysctl name2oid unknown name {name_str}, returning ENOENT");
            complete_bsd(cpu, ENOENT);
            return Ok(());
        }
    }

    // Case 2: Direct MIB query
    if let Some(entry) = ENTRIES.iter().find(|e| e.mib == mib.as_slice()) {
        let val_bytes: Vec<u8> = match &entry.value {
            SysctlValue::String(s) => s.to_vec(),
            SysctlValue::Int32(v) => v.to_le_bytes().to_vec(),
            SysctlValue::Int64(v) => v.to_le_bytes().to_vec(),
            SysctlValue::Bytes(b) => b.to_vec(),
        };
        let len = val_bytes.len();

        if oldlen_ptr != 0 {
            let mut avail_bytes = [0u8; 8];
            let avail = if cpu.read_guest_into(oldlen_ptr, &mut avail_bytes).is_ok() {
                u64::from_le_bytes(avail_bytes) as usize
            } else {
                0
            };
            let _ = cpu.write_guest_into(oldlen_ptr, &(len as u64).to_le_bytes());

            if old_ptr != 0 {
                let copy_len = len.min(avail);
                let _ = cpu.write_guest_into(old_ptr, &val_bytes[..copy_len]);
            }
        } else if old_ptr != 0 {
            let _ = cpu.write_guest_into(old_ptr, &val_bytes);
        }

        echo!("[a64] genuine sysctl query {} (MIB {:?}) returned {} bytes", entry.name, mib, len);
        complete_bsd(cpu, 0);
        return Ok(());
    }

    echo!("[a64] genuine sysctl unhandled MIB {:?}, returning ENOENT", mib);
    complete_bsd(cpu, ENOENT);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysctl_name2oid_and_query_succeeds() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 3).unwrap();

        // 1. name2oid for "hw.machine"
        let name_str = b"hw.machine\0";
        cpu.write_guest_into(0x10100, name_str).unwrap();
        // MIB [0, 3] at 0x10000
        cpu.write_guest_into(0x10000, &0i32.to_le_bytes()).unwrap();
        cpu.write_guest_into(0x10004, &3i32.to_le_bytes()).unwrap();
        // oldlen = 32 at 0x10020
        cpu.write_guest_into(0x10020, &32u64.to_le_bytes()).unwrap();

        cpu.set_reg(0, 0x10000); // name_ptr
        cpu.set_reg(1, 2);       // namelen
        cpu.set_reg(2, 0x10030); // old_ptr
        cpu.set_reg(3, 0x10020); // oldlen_ptr
        cpu.set_reg(4, 0x10100); // new_ptr (string)
        cpu.set_reg(5, name_str.len() as u64); // newlen

        sysctl(&mut cpu).unwrap();
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(cpu.pstate() & CARRY, 0);

        let mut mib_out = [0u8; 8];
        cpu.read_guest_into(0x10030, &mut mib_out).unwrap();
        assert_eq!(i32::from_le_bytes([mib_out[0], mib_out[1], mib_out[2], mib_out[3]]), CTL_HW);
        assert_eq!(i32::from_le_bytes([mib_out[4], mib_out[5], mib_out[6], mib_out[7]]), 1);

        // 2. Query [CTL_HW, 1]
        cpu.write_guest_into(0x10020, &64u64.to_le_bytes()).unwrap();
        cpu.set_reg(0, 0x10030); // name_ptr (contains [CTL_HW, 1])
        cpu.set_reg(1, 2);       // namelen
        cpu.set_reg(2, 0x10200); // old_ptr
        cpu.set_reg(3, 0x10020); // oldlen_ptr
        cpu.set_reg(4, 0);
        cpu.set_reg(5, 0);

        sysctl(&mut cpu).unwrap();
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(cpu.pstate() & CARRY, 0);

        let mut val = [0u8; 11];
        cpu.read_guest_into(0x10200, &mut val).unwrap();
        assert_eq!(&val, b"iPhone10,4\0");
    }
}

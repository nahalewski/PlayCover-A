//! Bounded virtual Darwin process services. These do not supply Mach ports,
//! Objective-C, threads, a filesystem, or Apple framework initialization.
//! ABI references: apple-oss-distributions/xnu bsd/kern/syscalls.master,
//! bsd/kern/kern_time.c, bsd/sys/_types/_user64_timeval.h,
//! osfmk/mach/arm/traps.h, osfmk/arm64/sleh.c, and osfmk/kern/clock.c.
//! Time uses this virtual process's nanosecond timebase, not Apple hardware
//! counter/commpage state. Calendar time is anchored at process creation.

use super::A64Cpu;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Success(u64),
    /// Mach traps return x0 without changing the BSD errno/carry convention.
    TrapSuccess(u64),
    Errno(i32),
    Exit(i32),
    Unsupported {
        number: u64,
        reason: &'static str,
    },
}

trait Memory {
    fn read(&self, address: u64, bytes: &mut [u8]) -> Result<(), String>;
    fn validate_write(&self, address: u64, size: usize) -> Result<(), String>;
    fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), String>;
    fn zero(&mut self, address: u64, size: usize, permissions: u32) -> Result<(), String>;
}

impl Memory for A64Cpu {
    fn read(&self, address: u64, bytes: &mut [u8]) -> Result<(), String> {
        self.read_guest_into(address, bytes)
    }
    fn zero(&mut self, address: u64, size: usize, permissions: u32) -> Result<(), String> {
        self.map_zeroed(address, size, permissions)
    }
    fn validate_write(&self, address: u64, size: usize) -> Result<(), String> {
        self.validate_guest_write(address, size)
    }
    fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), String> {
        self.write_guest_into(address, bytes)
    }
}

pub(super) struct Runtime64 {
    pid: u32,
    next_mapping: u64,
    clock_start: Instant,
    calendar_start: Duration,
}

impl Runtime64 {
    pub(super) fn new(pid: u32) -> Result<Self, String> {
        if pid == 0 {
            return Err("Virtual process PID must be nonzero".into());
        }
        Ok(Self {
            pid,
            next_mapping: 0x6000_0000_0000,
            calendar_start: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "Host clock predates the Unix epoch")?,
            clock_start: Instant::now(),
        })
    }

    pub(super) fn handle(
        &mut self,
        number: u64,
        args: [u64; 6],
        cpu: &mut A64Cpu,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
    ) -> Outcome {
        self.service(number, args, cpu, stdout, stderr)
    }

    fn service(
        &mut self,
        number: u64,
        a: [u64; 6],
        memory: &mut impl Memory,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
    ) -> Outcome {
        // XNU's ARM64 trap selector is a signed int, even though x16 is 64 bit.
        match number as u32 as i32 {
            1 => Outcome::Exit(a[0] as i32),
            20 => Outcome::Success(self.pid as u64),
            // A virtual process currently has one thread. This is an identity,
            // not a Mach port; creating additional threads remains unsupported.
            372 => Outcome::Success(1),
            // Virtual ticks are monotonic nanoseconds, with timebase 1/1.
            // XNU arm64/sleh.c handles -3 directly and modifies only x0.
            -3 => Outcome::TrapSuccess(
                self.clock_start.elapsed().as_nanos().min(u64::MAX as u128) as u64
            ),
            -89 => {
                let mut info = [0u8; 8];
                info[..4].copy_from_slice(&1u32.to_le_bytes());
                info[4..].copy_from_slice(&1u32.to_le_bytes());
                // XNU clock.c intentionally ignores copyout's error here.
                let _ = memory.write(a[0], &info);
                Outcome::TrapSuccess(0)
            }
            116 => {
                for (address, size) in [(a[0], 16), (a[1], 8), (a[2], 8)] {
                    if address != 0 && memory.validate_write(address, size).is_err() {
                        return Outcome::Errno(14);
                    }
                }
                let elapsed = self.clock_start.elapsed();
                let Some(calendar) = self.calendar_start.checked_add(elapsed) else {
                    return Outcome::Errno(84);
                };
                let mut timeval = [0u8; 16];
                // XNU deliberately casts seconds through uint32_t for arm64.
                timeval[..8].copy_from_slice(&(calendar.as_secs() as u32 as i64).to_le_bytes());
                timeval[8..12].copy_from_slice(&(calendar.subsec_micros() as i32).to_le_bytes());
                let ticks = (elapsed.as_nanos().min(u64::MAX as u128) as u64).to_le_bytes();
                for (address, bytes) in [
                    (a[0], &timeval[..]),
                    (a[1], &[0u8; 8][..]),
                    (a[2], &ticks[..]),
                ] {
                    if address != 0 && memory.write(address, bytes).is_err() {
                        return Outcome::Errno(14);
                    }
                }
                Outcome::Success(0)
            }
            4 => {
                if a[0] != 1 && a[0] != 2 {
                    return Outcome::Errno(9);
                }
                if a[2] == 0 {
                    return Outcome::Success(0);
                }
                let remaining =
                    OUTPUT_LIMIT.saturating_sub(stdout.len().saturating_add(stderr.len()));
                if remaining == 0 {
                    return Outcome::Errno(28);
                }
                let output = match a[0] {
                    1 => stdout,
                    2 => stderr,
                    _ => return Outcome::Errno(9),
                };
                // A successful bounded partial write is observable through its count.
                let count = a[2].min(1024 * 1024).min(remaining as u64) as usize;
                if a[1].checked_add(count as u64).is_none() {
                    return Outcome::Errno(14);
                }
                let mut bytes = Vec::new();
                if bytes.try_reserve_exact(count).is_err() {
                    return Outcome::Errno(12);
                }
                bytes.resize(count, 0);
                if memory.read(a[1], &mut bytes).is_err() {
                    return Outcome::Errno(14);
                }
                if output.try_reserve(count).is_err() {
                    return Outcome::Errno(12);
                }
                output.extend_from_slice(&bytes);
                Outcome::Success(count as u64)
            }
            197 => {
                const PAGE: u64 = 16384;
                if a[3] & 0x10 != 0 {
                    return Outcome::Unsupported {
                        number,
                        reason: "MAP_FIXED requires replacement of existing mappings",
                    };
                }
                if a[3] != 0x1002 {
                    return Outcome::Unsupported {
                        number,
                        reason: "Only anonymous MAP_PRIVATE mappings are implemented",
                    };
                }
                if a[1] == 0 || a[2] & !7 != 0 || a[4] as i32 != -1 || a[5] != 0 {
                    return Outcome::Errno(22);
                }
                let Some(size) = a[1].checked_add(PAGE - 1).map(|x| x & !(PAGE - 1)) else {
                    return Outcome::Errno(12);
                };
                if size > 256 * 1024 * 1024 {
                    return Outcome::Errno(12);
                }
                let base = self.next_mapping;
                let Some(end) = base.checked_add(size) else {
                    return Outcome::Errno(12);
                };
                let Ok(size) = usize::try_from(size) else {
                    return Outcome::Errno(12);
                };
                // A non-fixed address is only a hint. The virtual process chooses
                // its reserved anonymous allocation arena; overlap fails closed.
                if memory.zero(base, size, a[2] as u32).is_err() {
                    return Outcome::Errno(12);
                }
                self.next_mapping = end;
                Outcome::Success(base)
            }
            _ => Outcome::Unsupported {
                number,
                reason: "Darwin service is not implemented",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Mock {
        regions: Vec<(u64, Vec<u8>, u32)>,
    }
    impl Memory for Mock {
        fn validate_write(&self, address: u64, size: usize) -> Result<(), String> {
            for (base, data, prot) in &self.regions {
                if *prot & 2 == 0 {
                    continue;
                }
                if let Some(offset) = address
                    .checked_sub(*base)
                    .and_then(|n| usize::try_from(n).ok())
                {
                    if data.get(offset..).and_then(|s| s.get(..size)).is_some() {
                        return Ok(());
                    }
                }
            }
            Err("Unmapped or unwritable".into())
        }
        fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), String> {
            self.validate_write(address, bytes.len())?;
            for (base, data, prot) in &mut self.regions {
                if *prot & 2 == 0 {
                    continue;
                }
                if let Some(offset) = address
                    .checked_sub(*base)
                    .and_then(|n| usize::try_from(n).ok())
                {
                    if let Some(slice) = data
                        .get_mut(offset..)
                        .and_then(|s| s.get_mut(..bytes.len()))
                    {
                        slice.copy_from_slice(bytes);
                        return Ok(());
                    }
                }
            }
            unreachable!()
        }
        fn read(&self, address: u64, out: &mut [u8]) -> Result<(), String> {
            for (base, data, prot) in &self.regions {
                if *prot & 1 == 0 {
                    continue;
                }
                if let Some(offset) = address.checked_sub(*base) {
                    if let Some(slice) =
                        data.get(offset as usize..).and_then(|s| s.get(..out.len()))
                    {
                        out.copy_from_slice(slice);
                        return Ok(());
                    }
                }
            }
            Err("Unmapped or unreadable".into())
        }
        fn zero(&mut self, address: u64, size: usize, prot: u32) -> Result<(), String> {
            self.regions.push((address, vec![0; size], prot));
            Ok(())
        }
    }
    #[test]
    fn mapped_memory_is_zeroed_and_write_is_checked() {
        let mut runtime = Runtime64::new(7).unwrap();
        let mut memory = Mock::default();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = runtime.service(
            197,
            [0, 1, 3, 0x1002, u64::MAX, 0],
            &mut memory,
            &mut out,
            &mut err,
        );
        let Outcome::Success(base) = result else {
            panic!("{result:?}");
        };
        assert_eq!(memory.regions[0].1.len(), 16384);
        assert!(memory.regions[0].1.iter().all(|x| *x == 0));
        memory.regions[0].1[..3].copy_from_slice(b"abc");
        assert_eq!(
            runtime.service(4, [1, base, 3, 0, 0, 0], &mut memory, &mut out, &mut err),
            Outcome::Success(3)
        );
        assert_eq!(out, b"abc");
        assert_eq!(
            runtime.service(
                4,
                [1, base + 16384, 1, 0, 0, 0],
                &mut memory,
                &mut out,
                &mut err
            ),
            Outcome::Errno(14)
        );
        assert_eq!(out, b"abc");
        memory.regions[0].2 = 2;
        assert_eq!(
            runtime.service(4, [2, base, 1, 0, 0, 0], &mut memory, &mut out, &mut err),
            Outcome::Errno(14)
        );
        assert!(err.is_empty());
    }
    #[test]
    fn identity_and_unsupported_services_are_explicit() {
        let mut runtime = Runtime64::new(42).unwrap();
        let (mut memory, mut out, mut err) = (Mock::default(), Vec::new(), Vec::new());
        assert_eq!(
            runtime.service(20, [0; 6], &mut memory, &mut out, &mut err),
            Outcome::Success(42)
        );
        assert_eq!(
            runtime.service(1, [9, 0, 0, 0, 0, 0], &mut memory, &mut out, &mut err),
            Outcome::Exit(9)
        );
        assert!(matches!(
            runtime.service(73, [0; 6], &mut memory, &mut out, &mut err),
            Outcome::Unsupported { .. }
        ));
        assert_eq!(
            runtime.service(
                197,
                [0, u64::MAX, 3, 0x1002, u64::MAX, 0],
                &mut memory,
                &mut out,
                &mut err
            ),
            Outcome::Errno(12)
        );
        assert!(memory.regions.is_empty());
    }

    #[test]
    fn virtual_identity_and_time_traps_have_consistent_timebase() {
        let mut runtime = Runtime64::new(7).unwrap();
        let mut memory = Mock {
            regions: vec![(0x1000, vec![0xff; 8], 2)],
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(
            runtime.service(372, [0; 6], &mut memory, &mut out, &mut err),
            Outcome::Success(1)
        );
        let Outcome::TrapSuccess(before) =
            runtime.service((-3i64) as u64, [0; 6], &mut memory, &mut out, &mut err)
        else {
            panic!("not a time trap")
        };
        let Outcome::TrapSuccess(after) = runtime.service(
            (-3i32) as u32 as u64,
            [0; 6],
            &mut memory,
            &mut out,
            &mut err,
        ) else {
            panic!("not a time trap")
        };
        assert!(after >= before);
        assert_eq!(
            runtime.service(
                (-89i64) as u64,
                [0x1000, 0, 0, 0, 0, 0],
                &mut memory,
                &mut out,
                &mut err
            ),
            Outcome::TrapSuccess(0)
        );
        assert_eq!(memory.regions[0].1, [1, 0, 0, 0, 1, 0, 0, 0]);
        // Apple's trap ignores invalid copyout, preserving memory.
        assert_eq!(
            runtime.service(
                (-89i64) as u64,
                [u64::MAX, 0, 0, 0, 0, 0],
                &mut memory,
                &mut out,
                &mut err
            ),
            Outcome::TrapSuccess(0)
        );
        assert_eq!(memory.regions[0].1, [1, 0, 0, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn gettimeofday_validates_every_output_before_writing() {
        let mut runtime = Runtime64::new(7).unwrap();
        let mut memory = Mock {
            regions: vec![(0x1000, vec![0xff; 32], 3)],
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args = [0x1000, 0x1010, 0x1018, 0, 0, 0];
        let lower = runtime.calendar_start.as_secs() as u32 as i64;
        assert_eq!(
            runtime.service(116, args, &mut memory, &mut out, &mut err),
            Outcome::Success(0)
        );
        let data = &memory.regions[0].1;
        let secs = i64::from_le_bytes(data[..8].try_into().unwrap());
        let micros = i32::from_le_bytes(data[8..12].try_into().unwrap());
        assert!(secs >= lower);
        assert!((0..1_000_000).contains(&micros));
        assert_eq!(&data[12..24], &[0; 12]);
        let snapshot = data.clone();
        let mut invalid = args;
        invalid[2] = 0x1020;
        assert_eq!(
            runtime.service(116, invalid, &mut memory, &mut out, &mut err),
            Outcome::Errno(14)
        );
        assert_eq!(memory.regions[0].1, snapshot);
        memory.regions[0].2 = 1;
        assert_eq!(
            runtime.service(116, args, &mut memory, &mut out, &mut err),
            Outcome::Errno(14)
        );
        assert_eq!(memory.regions[0].1, snapshot);
        assert_eq!(
            runtime.service(116, [0; 6], &mut memory, &mut out, &mut err),
            Outcome::Success(0)
        );
    }

    #[test]
    fn repeated_writes_share_a_bounded_output_budget() {
        let mut runtime = Runtime64::new(7).unwrap();
        let mut memory = Mock {
            regions: vec![(0x1000, vec![b'x'; 1024 * 1024], 1)],
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        for index in 0..15 {
            let fd = if index % 2 == 0 { 1 } else { 2 };
            assert_eq!(
                runtime.service(
                    4,
                    [fd, 0x1000, 1024 * 1024, 0, 0, 0],
                    &mut memory,
                    &mut out,
                    &mut err
                ),
                Outcome::Success(1024 * 1024)
            );
        }
        assert_eq!(
            runtime.service(
                4,
                [2, 0x1000, 1024 * 1024 - 3, 0, 0, 0],
                &mut memory,
                &mut out,
                &mut err
            ),
            Outcome::Success(1024 * 1024 - 3)
        );
        // Only the three remaining bytes are read, so the unreadable suffix
        // of this request must not prevent a valid partial write.
        memory.regions[0].1.truncate(3);
        assert_eq!(
            runtime.service(4, [1, 0x1000, 99, 0, 0, 0], &mut memory, &mut out, &mut err),
            Outcome::Success(3)
        );
        assert_eq!(out.len() + err.len(), OUTPUT_LIMIT);
        assert_eq!(
            runtime.service(
                4,
                [2, u64::MAX, 1, 0, 0, 0],
                &mut memory,
                &mut out,
                &mut err
            ),
            Outcome::Errno(28)
        );
        assert_eq!(
            runtime.service(
                4,
                [1, u64::MAX, 0, 0, 0, 0],
                &mut memory,
                &mut out,
                &mut err
            ),
            Outcome::Success(0)
        );
        assert_eq!(out.len() + err.len(), OUTPUT_LIMIT);
    }
}

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Explicit entry-prefix diagnostic. This is not an initialized process run.
//! Every instruction is checked before single stepping; cached framework code
//! remains forbidden until a real runtime startup policy can establish it.
use super::{
    bridge::{GuestCall, ReturnValues},
    host_services::SelectedServices,
    A64Cpu,
};

pub(super) struct DeferredInitialization {
    pub app_functions: usize,
    pub cached_dependencies: usize,
}
pub(super) enum PrefixOutcome {
    Boundary {
        pc: u64,
        instructions: u64,
        reason: String,
    },
    Returned {
        values: ReturnValues,
        instructions: u64,
    },
}

/// The caller explicitly opts into observing app entry instructions before
/// normal startup. No initializer is silently marked complete or skipped in
/// production: deferred counts remain attached to the result/log.
pub(super) fn run_prefix(
    cpu: &mut A64Cpu,
    services: &mut SelectedServices,
    call: &GuestCall,
    app_executable_ranges: &[(u64, u64)],
    initialization: &DeferredInitialization,
    budget: u64,
) -> Result<PrefixOutcome, String> {
    if app_executable_ranges.is_empty() || app_executable_ranges.len() > 128 {
        return Err("entry prefix requires bounded app executable ranges".into());
    }
    for &(start, end) in app_executable_ranges {
        if start >= end || start & 3 != 0 || end & 3 != 0 {
            return Err("entry prefix executable range invalid".into());
        }
    }
    let owned = services.instruction_ranges();
    let mut instructions = 0;
    let mut boundary = None;
    let mut policy = |cpu: &A64Cpu| {
        let pc = cpu.pc();
        let in_app = app_executable_ranges
            .iter()
            .any(|&(start, end)| pc >= start && pc.checked_add(4).is_some_and(|next| next <= end));
        let in_owned = owned
            .iter()
            .any(|&(start, end)| pc >= start && pc.checked_add(4).is_some_and(|next| next <= end));
        let reason = if !in_app && !in_owned {
            Some(format!("runtime initialization boundary: {} app initializer functions and {} cached dependencies still deferred",initialization.app_functions,initialization.cached_dependencies))
        } else if in_app {
            let mut bytes = [0; 4];
            cpu.read_guest_into(pc, &mut bytes)?;
            // App SVC calls require an explicit implemented Darwin/Mach route.
            // Do not execute even the trap instruction in this prefix probe.
            if u32::from_le_bytes(bytes) & 0xffe0001f == 0xd4000001 {
                Some("unregistered application supervisor call requires runtime service".into())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(reason) = reason {
            boundary = Some((pc, reason));
            return Err("entry prefix stopped before unsupported runtime instruction".into());
        }
        instructions += 1;
        Ok(())
    };
    let result = services.call_with_instruction_policy(cpu, call, budget, &mut policy);
    if let Some((pc, reason)) = boundary {
        return Ok(PrefixOutcome::Boundary {
            pc,
            instructions,
            reason,
        });
    }
    result.map(|values| PrefixOutcome::Returned {
        values,
        instructions,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{cf_terraria_services::KnownConstants, host_services::Selection};
    use super::*;
    #[test]
    fn prefix_stops_before_cached_instruction_and_restores_cpu() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 5).unwrap();
        cpu.map_zeroed(0x11000, 4096, 5).unwrap();
        cpu.map_zeroed(0x40000, 4096, 3).unwrap();
        cpu.write_bytes(0x10000, &0x14000400u32.to_le_bytes()); // b cache
        for (i, word) in [0xd2800540u32, 0xf9000040, 0xd65f03c0]
            .into_iter()
            .enumerate()
        {
            cpu.write_bytes(0x11000 + i as u64 * 4, &word.to_le_bytes());
        }
        cpu.set_pc(0x1234);
        cpu.set_reg(0, 99);
        let mut services = SelectedServices::install(
            &mut cpu,
            // Production callback stacks now reserve guarded 64KiB windows;
            // keep this fixture's observable app data at 0x40000 disjoint.
            0x100000,
            Selection {
                objc_lifetime: true,
                core_foundation: false,
            },
            KnownConstants::default(),
        )
        .unwrap();
        let result = run_prefix(
            &mut cpu,
            &mut services,
            &GuestCall {
                entry: 0x10000,
                integers: vec![0, 0, 0x40000],
                ..Default::default()
            },
            &[(0x10000, 0x10004)],
            &DeferredInitialization {
                app_functions: 9,
                cached_dependencies: 36,
            },
            20,
        )
        .unwrap();
        match result {
            PrefixOutcome::Boundary {
                pc,
                instructions,
                reason,
            } => {
                assert_eq!(pc, 0x11000);
                assert_eq!(instructions, 1);
                assert!(reason.contains("9 app"));
            }
            _ => panic!("cached instruction must not execute"),
        }
        assert_eq!(cpu.read_u64(0x40000), Some(0));
        assert_eq!(cpu.pc(), 0x1234);
        assert_eq!(cpu.reg(0), 99);
    }
    #[test]
    fn app_supervisor_call_is_rejected_before_execution() {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x10000, 4096, 5).unwrap();
        cpu.write_bytes(0x10000, &0xd4001001u32.to_le_bytes());
        let mut services = SelectedServices::install(
            &mut cpu,
            0x20000,
            Selection {
                objc_lifetime: true,
                core_foundation: false,
            },
            KnownConstants::default(),
        )
        .unwrap();
        match run_prefix(
            &mut cpu,
            &mut services,
            &GuestCall {
                entry: 0x10000,
                ..Default::default()
            },
            &[(0x10000, 0x10004)],
            &DeferredInitialization {
                app_functions: 0,
                cached_dependencies: 1,
            },
            20,
        )
        .unwrap()
        {
            PrefixOutcome::Boundary {
                instructions,
                reason,
                ..
            } => {
                assert_eq!(instructions, 0);
                assert!(reason.contains("supervisor"));
            }
            _ => panic!("unknown SVC must not execute"),
        }
    }
}

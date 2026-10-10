/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Owned SCNetworkReachability queries and scheduling for ARM64 guests.
//!
//! The `SCNetworkReachabilityCreateWith{Name,Address}` constructors are
//! *not* routed. They stay genuine, so targets are real CF objects that
//! genuine `CFRelease`/`CFGetTypeID` handle correctly. Only the parts that
//! need configd/mDNSResponder/nw_path XPC are owned:
//!
//! - `GetFlags` reports the host's real connectivity: whether the host has a
//!   route to the public internet. No packet is sent; UDP `connect` only
//!   consults the routing table.
//! - `SetCallback`/`Schedule*`/`SetDispatchQueue` record the registration
//!   and succeed as on iOS. This layer observes no network changes, so no
//!   callback is delivered. iOS also calls only on changes.
use super::{Arena, Family};
use crate::a64::bridge::{ReturnValues, ServiceFrame};
use std::collections::BTreeMap;

pub(super) const PROVIDER: &str =
    "/System/Library/Frameworks/SystemConfiguration.framework/SystemConfiguration";

const SYMBOLS: &[&str] = &[
    "_SCNetworkReachabilityGetFlags",
    "_SCNetworkReachabilitySetCallback",
    "_SCNetworkReachabilityScheduleWithRunLoop",
    "_SCNetworkReachabilityUnscheduleFromRunLoop",
    "_SCNetworkReachabilitySetDispatchQueue",
];
const FLAG_REACHABLE: u32 = 1 << 1;

/// Host connectivity probe, replaceable in tests.
pub(super) type Probe = fn() -> bool;
fn host_has_route() -> bool {
    let Ok(socket) = std::net::UdpSocket::bind(("0.0.0.0", 0)) else {
        return false;
    };
    socket.connect(("1.1.1.1", 53)).is_ok()
}

#[derive(Default)]
struct Registration {
    callout: u64,
    info: u64,
    run_loops: usize,
    queue: u64,
}

pub(in crate::a64) struct SystemConfiguration {
    probe: Probe,
    targets: BTreeMap<u64, Registration>,
}
impl Default for SystemConfiguration {
    fn default() -> Self {
        Self {
            probe: host_has_route,
            targets: BTreeMap::new(),
        }
    }
}

impl Family for SystemConfiguration {
    fn name(&self) -> &'static str {
        "system_configuration"
    }
    fn provider(&self) -> &'static str {
        PROVIDER
    }
    fn symbols(&self) -> &'static [&'static str] {
        SYMBOLS
    }
    fn call(
        &mut self,
        index: usize,
        frame: &mut ServiceFrame<'_>,
        _arena: &mut Arena,
    ) -> Result<ReturnValues, String> {
        let target = frame.integer(0)?;
        if target == 0 {
            // Apple's implementation sets kSCStatusInvalidArgument and fails.
            return Ok(ReturnValues::integer(0));
        }
        let ok = match index {
            0 => {
                let out = frame.integer(1)?;
                if out == 0 {
                    false
                } else {
                    let flags = if (self.probe)() { FLAG_REACHABLE } else { 0 };
                    frame.write(out, &flags.to_le_bytes())?;
                    true
                }
            }
            1 => {
                let callout = frame.integer(1)?;
                let context = frame.integer(2)?;
                let info = if context == 0 {
                    0
                } else {
                    // SCNetworkReachabilityContext {version, info, ...}
                    let bytes = frame.read(context, 16)?;
                    u64::from_le_bytes(bytes[8..16].try_into().unwrap())
                };
                let registration = self.targets.entry(target).or_default();
                registration.callout = callout;
                registration.info = info;
                true
            }
            2 => {
                self.targets.entry(target).or_default().run_loops += 1;
                true
            }
            3 => match self.targets.get_mut(&target) {
                Some(registration) if registration.run_loops > 0 => {
                    registration.run_loops -= 1;
                    true
                }
                _ => false,
            },
            4 => {
                self.targets.entry(target).or_default().queue = frame.integer(1)?;
                true
            }
            _ => return Err("SystemConfiguration dispatch index invalid".into()),
        };
        Ok(ReturnValues::integer(ok as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::super::install;
    use super::*;
    use crate::a64::bridge::{GuestBridge, GuestCall};
    use crate::a64::A64Cpu;

    fn run(probe: Probe) {
        let mut cpu = A64Cpu::new_sparse();
        cpu.map_zeroed(0x80000, 0x1000, 3).unwrap();
        let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
        let family = SystemConfiguration {
            probe,
            targets: BTreeMap::new(),
        };
        let frameworks = install(&mut cpu, &mut bridge, 0x200000, vec![Box::new(family)]).unwrap();
        let entry = |name: &str| {
            frameworks
                .bindings
                .iter()
                .find(|b| b.symbol == name)
                .unwrap()
                .address
        };
        let mut call = |name: &str, integers: Vec<u64>| {
            bridge
                .call(
                    &mut cpu,
                    &GuestCall {
                        entry: entry(name),
                        integers,
                        ..Default::default()
                    },
                    100,
                )
                .unwrap()
                .integers[0]
        };
        let target = 0x80100;
        assert_eq!(call("_SCNetworkReachabilityGetFlags", vec![target, 0x80000]), 1);
        assert_eq!(call("_SCNetworkReachabilityGetFlags", vec![0, 0x80000]), 0);
        assert_eq!(call("_SCNetworkReachabilitySetCallback", vec![target, 0x1234, 0]), 1);
        assert_eq!(call("_SCNetworkReachabilityUnscheduleFromRunLoop", vec![target, 1, 2]), 0);
        assert_eq!(call("_SCNetworkReachabilityScheduleWithRunLoop", vec![target, 1, 2]), 1);
        assert_eq!(call("_SCNetworkReachabilityUnscheduleFromRunLoop", vec![target, 1, 2]), 1);
        assert_eq!(call("_SCNetworkReachabilitySetDispatchQueue", vec![target, 0]), 1);
        let flags = u32::from_le_bytes(cpu.read_bytes(0x80000, 4).unwrap().try_into().unwrap());
        assert_eq!(flags, if probe() { FLAG_REACHABLE } else { 0 });
    }

    #[test]
    fn reachability_reports_host_connectivity_and_records_scheduling() {
        run(|| true);
        run(|| false);
    }
}

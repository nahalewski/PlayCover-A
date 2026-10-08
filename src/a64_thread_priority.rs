/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Virtual scheduler QoS policy, not Android priorities or advertised Darwin
//! kernel features. Caller supplies genuine runnable identities in FIFO order.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Qos {
    Maintenance = 1,
    Background,
    Utility,
    Default,
    UserInitiated,
    UserInteractive,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Priority {
    pub qos: Qos,
    pub relative: i8,
}
impl Priority {
    /// Flagless requested pthread priority encoding from XNU priority_private.h.
    /// Overrides, vouchers, workqueue flags and raw scheduling priorities need
    /// separate ownership/policy and are deliberately rejected here.
    pub fn decode(value: u64) -> Result<Self, String> {
        if value & !0x3fff != 0 {
            return Err("unsupported pthread priority flags/encoding".into());
        }
        let bits = ((value >> 8) & 0x3f) as u8;
        if bits.count_ones() != 1 {
            return Err("pthread priority must request exactly one QoS tier".into());
        }
        let qos = match bits.trailing_zeros() {
            0 => Qos::Maintenance,
            1 => Qos::Background,
            2 => Qos::Utility,
            3 => Qos::Default,
            4 => Qos::UserInitiated,
            5 => Qos::UserInteractive,
            _ => unreachable!(),
        };
        let relative = (value as u8).wrapping_add(1) as i8;
        if !(-15..=0).contains(&relative) {
            return Err("pthread relative priority outside -15..=0".into());
        }
        Ok(Self { qos, relative })
    }
    pub fn encode(self) -> u32 {
        (1u32 << (7 + self.qos as u32)) | u32::from((self.relative as u8).wrapping_sub(1))
    }
}

#[derive(Default)]
pub struct Priorities {
    threads: BTreeMap<u64, Priority>,
}
impl Priorities {
    pub fn register(&mut self, thread: u64) -> Result<(), String> {
        if thread == 0 || self.threads.contains_key(&thread) || self.threads.len() >= 64 {
            return Err("invalid/duplicate/excessive priority thread registration".into());
        }
        self.threads.insert(
            thread,
            Priority {
                qos: Qos::Default,
                relative: 0,
            },
        );
        Ok(())
    }
    pub fn set(&mut self, thread: u64, encoded: u64) -> Result<(), String> {
        let next = Priority::decode(encoded)?;
        *self
            .threads
            .get_mut(&thread)
            .ok_or("unknown priority thread")? = next;
        Ok(())
    }
    pub fn get(&self, thread: u64) -> Result<Priority, String> {
        self.threads
            .get(&thread)
            .copied()
            .ok_or_else(|| "unknown priority thread".into())
    }
    pub fn remove(&mut self, thread: u64) -> Result<(), String> {
        self.threads
            .remove(&thread)
            .map(|_| ())
            .ok_or_else(|| "unknown priority thread".into())
    }
    /// Highest requested tier, then relative priority. FIFO breaks ties; no
    /// blocked thread is invented or woken. Validate the whole queue first.
    pub fn select(&self, runnable: impl IntoIterator<Item = u64>) -> Result<Option<u64>, String> {
        let mut selected: Option<(u64, Priority)> = None;
        let mut seen = std::collections::BTreeSet::new();
        for id in runnable {
            if !seen.insert(id) {
                return Err("duplicate runnable priority identity".into());
            }
            let p = self.get(id)?;
            if selected.is_none_or(|(_, old)| (p.qos, p.relative) > (old.qos, old.relative)) {
                selected = Some((id, p));
            }
        }
        Ok(selected.map(|(id, _)| id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn genuine_encoding_all_tiers_and_relative_range() {
        for qos in [
            Qos::Maintenance,
            Qos::Background,
            Qos::Utility,
            Qos::Default,
            Qos::UserInitiated,
            Qos::UserInteractive,
        ] {
            for relative in -15..=0 {
                let p = Priority { qos, relative };
                assert_eq!(Priority::decode(p.encode().into()).unwrap(), p);
            }
        }
        for invalid in [0, 0x3ff, 0x8ef, 0x800, 0x400008ff, 0x100008ff, 1u64 << 32] {
            assert!(Priority::decode(invalid).is_err());
        }
    }
    #[test]
    fn selection_uses_qos_relative_and_fifo_without_mutating_on_error() {
        let mut p = Priorities::default();
        for id in 1..=3 {
            p.register(id).unwrap();
        }
        assert_eq!(p.select([3, 2, 1]).unwrap(), Some(3));
        p.set(
            1,
            Priority {
                qos: Qos::UserInitiated,
                relative: -15,
            }
            .encode()
            .into(),
        )
        .unwrap();
        p.set(
            2,
            Priority {
                qos: Qos::UserInitiated,
                relative: -1,
            }
            .encode()
            .into(),
        )
        .unwrap();
        assert_eq!(p.select([3, 1, 2]).unwrap(), Some(2));
        assert_eq!(p.select([3, 1]).unwrap(), Some(1));
        let old = p.get(2).unwrap();
        assert!(p.set(2, 0).is_err());
        assert_eq!(p.get(2).unwrap(), old);
        assert!(p.select([2, 99]).is_err());
        assert!(p.select([2, 2]).is_err());
        p.remove(2).unwrap();
        assert!(p.get(2).is_err());
        assert!(p.set(2, 0x8ff).is_err());
    }
}

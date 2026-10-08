/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Preflight reservations for mappings added to a live ARM64 address space.
//! This tracks address ownership only: committing a plan does not map memory.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmRange {
    pub start: u64,
    pub end: u64,
}

impl VmRange {
    pub fn from_size(start: u64, size: u64) -> Result<Self, String> {
        let end = start.checked_add(size).ok_or("VM range overflow")?;
        if size == 0 {
            return Err("Empty VM range".into());
        }
        Ok(Self { start, end })
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

#[derive(Debug)]
pub struct ReservationPlan {
    prior: Vec<VmRange>,
    additions: Vec<VmRange>,
}

#[derive(Debug)]
pub struct VmReservations {
    ranges: Vec<VmRange>,
    page_size: u64,
    max_ranges: usize,
    max_bytes: u64,
}

impl VmReservations {
    /// Seed with all existing occupied regions, including stacks and trampolines.
    pub fn new(
        occupied: &[VmRange],
        page_size: u64,
        max_ranges: usize,
        max_bytes: u64,
    ) -> Result<Self, String> {
        if !page_size.is_power_of_two() {
            return Err("VM page size must be a power of two".into());
        }
        let mut this = Self { ranges: vec![], page_size, max_ranges, max_bytes };
        let plan = this.plan(occupied)?;
        this.commit(plan)?;
        Ok(this)
    }

    pub fn ranges(&self) -> &[VmRange] {
        &self.ranges
    }

    /// Validate the complete batch before any CPU mapping is attempted.
    pub fn plan(&self, additions: &[VmRange]) -> Result<ReservationPlan, String> {
        self.validate(additions)?;
        Ok(ReservationPlan { prior: self.ranges.clone(), additions: additions.to_vec() })
    }

    /// Call only after actual mappings succeed. A failed mapping requires the
    /// caller to roll back its CPU mappings; this ledger remains unchanged.
    pub fn commit(&mut self, plan: ReservationPlan) -> Result<(), String> {
        if plan.prior != self.ranges {
            return Err("Stale VM reservation plan".into());
        }
        // Revalidate policy too: a plan from another ledger cannot bypass it.
        self.validate(&plan.additions)?;
        self.ranges.extend(plan.additions);
        self.ranges.sort_by_key(|r| r.start);
        Ok(())
    }

    fn validate(&self, additions: &[VmRange]) -> Result<(), String> {
        if self.ranges.len().checked_add(additions.len()).ok_or("VM range count overflow")? > self.max_ranges {
            return Err("VM reservation count limit exceeded".into());
        }
        let mut all = self.ranges.clone();
        all.extend_from_slice(additions);
        all.sort_by_key(|r| r.start);
        let mut bytes = 0u64;
        let mut previous_end = None;
        for range in all {
            if range.end <= range.start || range.start % self.page_size != 0 || range.end % self.page_size != 0 {
                return Err("Invalid or unaligned VM reservation".into());
            }
            if previous_end.map_or(false, |end| range.start < end) {
                return Err(format!("Overlapping VM reservation at {:#x}", range.start));
            }
            bytes = bytes.checked_add(range.end - range.start).ok_or("VM byte count overflow")?;
            if bytes > self.max_bytes {
                return Err("VM reservation byte limit exceeded".into());
            }
            previous_end = Some(range.end);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(start: u64, size: u64) -> VmRange { VmRange::from_size(start, size).unwrap() }
    fn ledger() -> VmReservations {
        VmReservations::new(&[r(0x1000, 0x2000)], 0x1000, 8, 0x10000).unwrap()
    }
    #[test]
    fn accepts_adjacent_ranges_but_rejects_actual_overlap() {
        let mut vm = ledger();
        let plan = vm.plan(&[r(0x3000, 0x1000), r(0x9000, 0x2000)]).unwrap();
        vm.commit(plan).unwrap();
        assert!(vm.plan(&[r(0x2000, 0x2000)]).is_err());
        assert_eq!(vm.ranges().len(), 3);
    }
    #[test]
    fn batch_conflicts_and_failures_never_publish_partial_reservations() {
        let vm = ledger();
        assert!(vm.plan(&[r(0x5000, 0x3000), r(0x7000, 0x1000)]).is_err());
        assert!(vm.plan(&[r(0x5000, 0x1000), r(0x2000, 0x1000)]).is_err());
        assert_eq!(vm.ranges(), &[r(0x1000, 0x2000)]);
    }
    #[test]
    fn concurrent_plans_cannot_commit_over_changed_address_space() {
        let mut vm = ledger();
        let old = vm.plan(&[r(0x5000, 0x1000)]).unwrap();
        let new = vm.plan(&[r(0x9000, 0x1000)]).unwrap();
        vm.commit(new).unwrap();
        assert!(vm.commit(old).is_err());
        assert_eq!(vm.ranges().len(), 2);
    }
    #[test]
    fn checks_overflow_alignment_count_budget_and_foreign_plan_policy() {
        assert!(VmRange::from_size(u64::MAX - 1, 4).is_err());
        assert!(VmRange::from_size(0, 0).is_err());
        assert!(VmReservations::new(&[], 3, 8, 0x10000).is_err());
        assert!(ledger().plan(&[VmRange { start: 0x5001, end: 0x6000 }]).is_err());
        assert!(ledger().plan(&[VmRange { start: 0x5000, end: 0x4000 }]).is_err());
        let generous = VmReservations::new(&[], 0x1000, 8, 0x10000).unwrap();
        let mut strict = VmReservations::new(&[], 0x1000, 1, 0x1000).unwrap();
        let foreign = generous.plan(&[r(0x5000, 0x2000)]).unwrap();
        assert!(strict.commit(foreign).is_err());
        assert!(strict.plan(&[r(0x5000, 0x1000), r(0x7000, 0x1000)]).is_err());
        assert!(strict.ranges().is_empty());
    }
}

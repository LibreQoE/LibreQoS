//! A read-only inventory of the pre-v3 unit names an upgrade will touch.
//!
//! The list restates the postinst's legacy-name sweep
//! (`src/src/build_dpkg.sh:206-208`) so the preflight can report what the
//! installer is about to stop/disable/remove — the card's §1.5
//! "pre-existing install state" check.

/// The pre-canonical systemd unit names the v3 upgrade stops/disables/removes.
pub const LEGACY_V2_UNITS: &[&str] = &[
    "lqosd.service",
    "lqos_scheduler.service",
    "lqos_setup.service",
    "lqos_node_manager.service",
    "lqos_netplan_helper.service",
    "lqos_api.service",
    "libreqos-uisp.service",
    "libreqos-uisp.timer",
];

/// The observed state of one unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitPresence {
    /// The unit exists and is active.
    Active,
    /// The unit exists but is inactive.
    Inactive,
    /// The unit is not present on the system.
    Absent,
}

/// One inventory row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitRow {
    /// The unit name (from [`LEGACY_V2_UNITS`]).
    pub unit: &'static str,
    /// The observed presence.
    pub presence: UnitPresence,
}

impl UnitRow {
    /// True when the upgrade will act on this unit.
    pub fn is_present(&self) -> bool {
        !matches!(self.presence, UnitPresence::Absent)
    }
}

/// Build the inventory from a caller-supplied presence probe.
///
/// The probe keeps this function pure and testable without systemd; a real
/// caller supplies a probe that asks `systemctl`/the filesystem.
pub fn inventory(probe: impl Fn(&str) -> UnitPresence) -> Vec<UnitRow> {
    LEGACY_V2_UNITS
        .iter()
        .copied()
        .map(|unit| UnitRow {
            unit,
            presence: probe(unit),
        })
        .collect()
}

/// The units present on the system, in list order.
pub fn present_units(probe: impl Fn(&str) -> UnitPresence) -> Vec<UnitRow> {
    inventory(probe)
        .into_iter()
        .filter(UnitRow::is_present)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_covers_every_legacy_unit_in_order() {
        let rows = inventory(|_| UnitPresence::Absent);
        let names: Vec<&str> = rows.iter().map(|row| row.unit).collect();
        assert_eq!(names, LEGACY_V2_UNITS);
    }

    #[test]
    fn present_units_keeps_only_present_rows() {
        let rows = present_units(|unit| {
            if unit == "lqosd.service" {
                UnitPresence::Active
            } else if unit == "lqos_scheduler.service" {
                UnitPresence::Inactive
            } else {
                UnitPresence::Absent
            }
        });
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].unit, "lqosd.service");
        assert_eq!(rows[0].presence, UnitPresence::Active);
        assert_eq!(rows[1].unit, "lqos_scheduler.service");
        assert_eq!(rows[1].presence, UnitPresence::Inactive);
    }
}

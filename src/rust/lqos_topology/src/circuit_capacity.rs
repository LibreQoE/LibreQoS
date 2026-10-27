//! §8 (`docs/lqos_conf_single_source_card.md`): the DERIVED circuit-slot
//! capacity carried on the dataplane transaction.
//!
//! The EMITTER (`publish_effective_topology_artifacts`) stamps the compiled
//! `DataplaneTransaction.circuit_capacity` with
//! `max(next_power_of_two(n_circuits), FLOOR, carried)`. The bridge reads that
//! value as its slot/array bound (flash_1 §8 (a)); the LKG inherits it because
//! the LKG stores the transaction.
//!
//! `carried` is the value the running bridge already applied. The bridge
//! REPORTS it on every applied-state report
//! (`analytics_ipc::StateReport::applied_circuit_capacity`); the broker notes
//! it here on adopt, so `max(…, carried)` never shrinks the world across a
//! bridge restart. Before the first report (cold boot) the previously
//! published desired transaction is the fallback carrier.

use std::sync::atomic::{AtomicU32, Ordering};

/// §8 FLOOR (DRIVER the unit: an empty/embryonic world still clears it, so
/// `next_power_of_two(0)`-collapses-to-1 cannot under-size.
pub const CIRCUIT_CAPACITY_FLOOR: u32 = 1000;

/// 0 = "no applied report yet". A reported capacity is always `>= 1`, so 0 is
/// an unambiguous sentinel (mirrors the bridge's carried holder).
static APPLIED_CIRCUIT_CAPACITY: AtomicU32 = AtomicU32::new(0);

/// Notes the bridge's last reported applied capacity (the broker's
/// `StateReport` decode). `None` clears the note (a pre-§8 bridge / no report).
///
/// Cold path: one `Relaxed` store.
pub fn note_applied_circuit_capacity(applied: Option<u32>) {
    APPLIED_CIRCUIT_CAPACITY.store(applied.unwrap_or(0), Ordering::Relaxed);
}

/// The bridge's last reported applied capacity, if any.
pub fn applied_circuit_capacity() -> Option<u32> {
    match APPLIED_CIRCUIT_CAPACITY.load(Ordering::Relaxed) {
        0 => None,
        value => Some(value),
    }
}

/// §8 derivation: `max(next_power_of_two(n_circuits), CIRCUIT_CAPACITY_FLOOR,
/// carried)`. Never 17; never shrinks below a carried value.
pub fn derive_circuit_capacity(n_circuits: usize, carried: Option<u32>) -> u32 {
    let npow2 = u32::try_from(n_circuits.next_power_of_two()).unwrap_or(u32::MAX);
    npow2
        .max(CIRCUIT_CAPACITY_FLOOR)
        .max(carried.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_capacity_is_next_power_of_two_floored_at_1000() {
        assert_eq!(derive_circuit_capacity(0, None), 1000, "empty -> FLOOR");
        assert_eq!(derive_circuit_capacity(3, None), 1000, "small -> FLOOR");
        assert_eq!(derive_circuit_capacity(17, None), 1000, "never 17");
        assert_eq!(derive_circuit_capacity(1000, None), 1024, "1000 -> npow2 1024");
        assert_eq!(derive_circuit_capacity(2000, None), 2048, "large -> npow2 headroom");
    }

    #[test]
    fn carried_capacity_is_never_under_sized() {
        // A carried applied value outranks the local headroom (monotonic).
        assert_eq!(derive_circuit_capacity(3, Some(8192)), 8192);
        assert_eq!(derive_circuit_capacity(9000, Some(4096)), 16384);
        // The floor still applies under a tiny carried.
        assert_eq!(derive_circuit_capacity(3, Some(1)), 1000);
    }
}

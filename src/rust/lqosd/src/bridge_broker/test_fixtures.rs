//! Shared `#[cfg(test)]` fixtures for the bridge-broker test modules.
//!
//! Two things every broker test needs and no production path does: a
//! `BrokerState` seeded with an applied base, and a lock over the REAL control
//! segment name (`step` checks `SEGMENT_NAME` for liveness, so any test that
//! creates a test segment under that name must not race another).
#![cfg(test)]

use super::*;

/// A `BrokerState` whose applied belief holds `base`.
///
/// The A1 card makes a missing base HOLD the publisher, so every publish-path
/// test must seed the one owner with the world the bridge would have reported.
pub(super) fn broker_state(
    segment: Option<SeamSegment>,
    base: DataplaneTransaction,
) -> BrokerState {
    let mut applied = applied::AppliedState::default();
    applied.fold_ack(base);
    BrokerState {
        segment,
        applied,
        pending: None,
        terminal_fence: None,
        last_desired_generation: None,
        last_desired_world_key: None,
        placement_stale_since: None,
        placement_stale_reported: false,
        placement_stale_episodes: 0,
        placement_stale_polls: 0,
        duplicate_mapping_alerts: BTreeSet::new(),
        held_ops: BTreeSet::new(),
        held_structural_key: None,
        held_retry_at: None,
        held_retries: 0,
        last_removal: None,
        deferred_sites: BTreeMap::new(),
        deferred_retires: BTreeMap::new(),
        slot_order: None,
        socket_path: None,
        artifact_cache: None,
        bad_layout_reported: false,
        attach_polls: 0,
        liveness_polls: 0,
        rejects_consecutive: 0,
        backoff_until: None,
        report_reread_at: None,
        report_reread_due: false,
        late_adopt: false,
        maintenance: lqos_config::MaintenanceWindowConfig::default(),
        maintenance_tz: None,
        maintenance_override_ops: None,
        deferred_maintenance: None,
        apply_now: false,
        break_glass_until_unix: None,
        maintenance_control_path: None,
    }
}

/// Serializes tests that own the real control-segment name: `step`'s idle
/// liveness arm compares `SEGMENT_NAME`'s inode, so two tests creating and
/// unlinking the same shm object would flake each other.
pub(super) fn seam_segment_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

//! Autopilot status snapshots for UI visibility.

use crate::autopilot::actor;
use crate::node_manager::ws::messages::{AutopilotActivityEntry, AutopilotStatusData};
use lqos_config::load_config;

/// Takes a snapshot of Autopilot status for UI publication.
///
/// This function is not pure: it sends a request to the Autopilot actor, and may fall back to
/// reading the current configuration from disk (via the config cache).
pub async fn autopilot_status_snapshot() -> AutopilotStatusData {
    if let Some(snapshot) = actor::request_status_snapshot().await {
        return snapshot;
    }

    let Ok(config) = load_config() else {
        return AutopilotStatusData {
            enabled: false,
            dry_run: true,
            cpu_max_pct: None,
            managed_nodes: 0,
            managed_circuits: 0,
            virtualized_nodes: 0,
            fq_codel_circuits: 0,
            last_action_summary: None,
            warnings: vec!["Unable to load configuration; Autopilot status unavailable.".to_string()],
        };
    };

    let ap = &config.autopilot;
    let mut warnings = Vec::new();

    if ap.enabled && ap.links.nodes.is_empty() && ap.circuits.circuits.is_empty() {
        warnings.push(
            "Autopilot is enabled but no nodes/circuits are allowlisted. No actions will occur."
                .to_string(),
        );
    }

    AutopilotStatusData {
        enabled: ap.enabled,
        dry_run: ap.dry_run,
        cpu_max_pct: None,
        managed_nodes: ap.links.nodes.len(),
        managed_circuits: ap.circuits.circuits.len(),
        virtualized_nodes: 0,
        fq_codel_circuits: 0,
        last_action_summary: None,
        warnings,
    }
}

/// Takes a snapshot of recent Autopilot activity for UI publication.
///
/// This function is not pure: it sends a request to the Autopilot actor.
/// If the actor isn't available, it returns an empty list.
pub async fn autopilot_activity_snapshot() -> Vec<AutopilotActivityEntry> {
    actor::request_activity_snapshot()
        .await
        .unwrap_or_default()
}

//! TreeGuard status snapshots for UI visibility.

use crate::node_manager::local_api::directories;
use crate::node_manager::ws::messages::{TreeguardActivityEntry, TreeguardStatusData};
use crate::treeguard::actor;
use lqos_config::load_config;

/// Takes a snapshot of TreeGuard status for UI publication.
///
/// This function is not pure: it sends a request to the TreeGuard actor, and may fall back to
/// reading the current configuration from disk (via the config cache).
pub async fn treeguard_status_snapshot() -> TreeguardStatusData {
    if let Some(snapshot) = actor::cached_status_snapshot() {
        return snapshot;
    }

    if let Some(snapshot) = actor::request_status_snapshot().await {
        return snapshot;
    }

    let Ok(config) = load_config() else {
        let totals = directories::treeguard_metadata_summary();
        return TreeguardStatusData {
            enabled: false,
            dry_run: true,
            paused_for_bakery_reload: false,
            pause_reason: None,
            cpu_max_pct: None,
            total_nodes: totals.total_nodes,
            total_circuits: totals.total_circuits,
            managed_nodes: 0,
            managed_circuits: 0,
            virtualized_nodes: 0,
            fq_codel_circuits: 0,
            last_action_summary: None,
            warnings: vec![
                "Unable to load configuration; TreeGuard status unavailable.".to_string(),
            ],
        };
    };

    let tg = &config.treeguard;
    let mut warnings = Vec::new();
    let totals = directories::treeguard_metadata_summary();

    if tg.enabled
        && !tg.links.all_nodes
        && tg.links.nodes.is_empty()
        && !tg.links.top_level_auto_virtualize
        && !tg.circuits.all_circuits
        && tg.circuits.circuits.is_empty()
    {
        warnings.push(
            "TreeGuard is enabled but no nodes/circuits are allowlisted. No actions will occur."
                .to_string(),
        );
    }

    TreeguardStatusData {
        enabled: tg.enabled,
        dry_run: tg.dry_run,
        paused_for_bakery_reload: false,
        pause_reason: None,
        cpu_max_pct: None,
        total_nodes: totals.total_nodes,
        total_circuits: totals.total_circuits,
        managed_nodes: if tg.links.all_nodes {
            totals.total_nodes
        } else {
            tg.links.nodes.len()
        },
        managed_circuits: if tg.circuits.all_circuits {
            totals.total_circuits
        } else {
            tg.circuits.circuits.len()
        },
        virtualized_nodes: 0,
        fq_codel_circuits: 0,
        last_action_summary: None,
        warnings,
    }
}

/// Takes a snapshot of recent TreeGuard activity for UI publication.
///
/// This function is not pure: it sends a request to the TreeGuard actor.
/// If the actor isn't available, it returns an empty list.
pub async fn treeguard_activity_snapshot() -> Vec<TreeguardActivityEntry> {
    if let Some(snapshot) = actor::cached_activity_snapshot() {
        return snapshot;
    }

    actor::request_activity_snapshot().await.unwrap_or_default()
}

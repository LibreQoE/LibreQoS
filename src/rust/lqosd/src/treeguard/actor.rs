//! TreeGuard actor loop.
//!
//! The actor is responsible for sampling telemetry, maintaining state machines,
//! and applying (or dry-running) any decisions.

use crate::node_manager::ws::messages::{TreeguardActivityEntry, TreeguardStatusData};
use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};
use crate::system_stats::SystemStats;
use crate::throughput_tracker::{CIRCUIT_RTT_BUFFERS, THROUGHPUT_TRACKER};
use crate::treeguard::TreeguardError;
use crate::treeguard::reload::{ReloadController, ReloadOutcome, ReloadPriority};
use crate::treeguard::state::{
    CircuitSqmState, CircuitState, LinkState, LinkVirtualState, is_sustained_idle,
    is_sustained_window,
};
use crate::treeguard::{bakery, decisions, overrides};
use crate::urgent;
use crossbeam_channel::{Receiver, Sender};
use fxhash::{FxHashMap, FxHashSet};
use lqos_bus::{UrgentSeverity, UrgentSource};
use lqos_config::load_config;
use lqos_overrides::{NetworkAdjustment, OverrideFile, OverrideLayer, OverrideStore};
use lqos_utils::hash_to_i64;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::{time_since_boot, unix_now};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

static TREEGUARD_SENDER: OnceLock<Sender<TreeguardCommand>> = OnceLock::new();
static TREEGUARD_STATUS_CACHE: OnceLock<RwLock<TreeguardStatusData>> = OnceLock::new();
static TREEGUARD_ACTIVITY_CACHE: OnceLock<RwLock<Vec<TreeguardActivityEntry>>> = OnceLock::new();

const ACTIVITY_RING_CAPACITY: usize = 200;
const UTIL_EWMA_ALPHA: f64 = 0.1;
const TOP_LEVEL_SAFE_SUSTAIN_MINUTES: u32 = 15;

/// A message sent to the TreeGuard actor.
#[derive(Debug)]
pub(crate) enum TreeguardCommand {
    /// Request a status snapshot.
    GetStatus {
        /// One-shot reply channel. Side effect: sends a snapshot to the requester.
        reply: tokio::sync::oneshot::Sender<TreeguardStatusData>,
    },
    /// Request an activity snapshot.
    GetActivity {
        /// One-shot reply channel. Side effect: sends a snapshot to the requester.
        reply: tokio::sync::oneshot::Sender<Vec<TreeguardActivityEntry>>,
    },
}

/// Starts the TreeGuard actor.
///
/// This function has side effects: it spawns the TreeGuard background thread and registers a
/// global sender used for UI snapshot requests.
pub(crate) fn start_treeguard_actor(
    system_usage_tx: Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) -> Result<(), TreeguardError> {
    if TREEGUARD_SENDER.get().is_some() {
        return Ok(());
    }

    let _ = TREEGUARD_STATUS_CACHE.set(RwLock::new(empty_status_snapshot()));
    let _ = TREEGUARD_ACTIVITY_CACHE.set(RwLock::new(Vec::new()));

    let (tx, rx) = crossbeam_channel::bounded::<TreeguardCommand>(64);
    let _ = TREEGUARD_SENDER.set(tx);

    std::thread::Builder::new()
        .name("TreeGuard".to_string())
        .spawn(move || treeguard_actor_loop(rx, system_usage_tx))?;

    Ok(())
}

pub(crate) fn cached_status_snapshot() -> Option<TreeguardStatusData> {
    Some(TREEGUARD_STATUS_CACHE.get()?.read().clone())
}

pub(crate) fn cached_activity_snapshot() -> Option<Vec<TreeguardActivityEntry>> {
    Some(TREEGUARD_ACTIVITY_CACHE.get()?.read().clone())
}

/// Requests a status snapshot from the TreeGuard actor.
///
/// This function is not pure: it sends a message to the TreeGuard actor thread.
pub(crate) async fn request_status_snapshot() -> Option<TreeguardStatusData> {
    let sender = TREEGUARD_SENDER.get()?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    if sender
        .try_send(TreeguardCommand::GetStatus { reply: tx })
        .is_err()
    {
        return None;
    }
    rx.await.ok()
}

/// Requests an activity snapshot from the TreeGuard actor.
///
/// This function is not pure: it sends a message to the TreeGuard actor thread.
pub(crate) async fn request_activity_snapshot() -> Option<Vec<TreeguardActivityEntry>> {
    let sender = TREEGUARD_SENDER.get()?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    if sender
        .try_send(TreeguardCommand::GetActivity { reply: tx })
        .is_err()
    {
        return None;
    }
    rx.await.ok()
}

/// Runs the TreeGuard actor loop, processing commands and periodic ticks.
///
/// This function has side effects: it blocks the current thread, samples telemetry, and may write
/// persistent changes (via overrides) depending on configuration.
fn treeguard_actor_loop(
    rx: Receiver<TreeguardCommand>,
    system_usage_tx: Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
    debug!("TreeGuard actor started");

    let mut status = empty_status_snapshot();
    let mut activity: VecDeque<TreeguardActivityEntry> = VecDeque::new();
    update_cached_snapshots(&status, &activity);

    let mut runtime_state = TreeguardRuntimeState::default();

    let mut tick_seconds: u64 = 1;
    let mut last_tick = Instant::now();

    loop {
        let next_tick = last_tick + Duration::from_secs(tick_seconds);
        let timeout = next_tick.saturating_duration_since(Instant::now());

        match rx.recv_timeout(timeout) {
            Ok(cmd) => handle_command(cmd, &status, &activity),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                last_tick = Instant::now();
                run_tick(
                    &mut status,
                    &mut activity,
                    &system_usage_tx,
                    &mut tick_seconds,
                    &mut runtime_state,
                );
                update_cached_snapshots(&status, &activity);
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                warn!("TreeGuard actor command channel disconnected; exiting actor");
                return;
            }
        }
    }
}

/// Handles a command received by the actor.
///
/// This function has side effects: it sends a snapshot reply over the provided one-shot channel.
fn handle_command(
    cmd: TreeguardCommand,
    status: &TreeguardStatusData,
    activity: &VecDeque<TreeguardActivityEntry>,
) {
    match cmd {
        TreeguardCommand::GetStatus { reply } => {
            let _ = reply.send(status.clone());
        }
        TreeguardCommand::GetActivity { reply } => {
            let data: Vec<TreeguardActivityEntry> = activity.iter().cloned().rev().collect();
            let _ = reply.send(data);
        }
    }
}

fn empty_status_snapshot() -> TreeguardStatusData {
    TreeguardStatusData {
        enabled: false,
        dry_run: true,
        cpu_max_pct: None,
        managed_nodes: 0,
        managed_circuits: 0,
        virtualized_nodes: 0,
        fq_codel_circuits: 0,
        last_action_summary: None,
        warnings: Vec::new(),
    }
}

fn update_cached_snapshots(
    status: &TreeguardStatusData,
    activity: &VecDeque<TreeguardActivityEntry>,
) {
    if let Some(cache) = TREEGUARD_STATUS_CACHE.get() {
        *cache.write() = status.clone();
    }
    if let Some(cache) = TREEGUARD_ACTIVITY_CACHE.get() {
        *cache.write() = activity.iter().cloned().rev().collect();
    }
}

#[derive(Default)]
struct TreeguardRuntimeState {
    link_states: FxHashMap<String, LinkState>,
    circuit_states: FxHashMap<String, CircuitState>,
    managed_nodes: FxHashSet<String>,
    managed_device_ids: FxHashSet<String>,
    duplicate_device_conflict_circuits: FxHashSet<String>,
    last_dry_run: Option<bool>,
    paused_for_bakery_reload: bool,
    reload_controller: ReloadController,
}

/// Executes a single TreeGuard tick.
///
/// This function has side effects: it samples telemetry, may read/write `lqos_overrides.treeguard.json`,
/// and appends to the activity ring buffer.
fn run_tick(
    status: &mut TreeguardStatusData,
    activity: &mut VecDeque<TreeguardActivityEntry>,
    system_usage_tx: &Sender<tokio::sync::oneshot::Sender<SystemStats>>,
    tick_seconds: &mut u64,
    runtime_state: &mut TreeguardRuntimeState,
) {
    let now_unix = unix_now().unwrap_or(0);
    let now_nanos_since_boot = time_since_boot()
        .ok()
        .map(Duration::from)
        .map(|d| d.as_nanos() as u64);

    let mut warnings = Vec::new();

    let Ok(config) = load_config() else {
        status.enabled = false;
        status.dry_run = true;
        status.cpu_max_pct = None;
        status.managed_nodes = 0;
        status.managed_circuits = 0;
        status.virtualized_nodes = 0;
        status.fq_codel_circuits = 0;
        status.last_action_summary = None;
        status.warnings = vec!["Unable to load configuration; TreeGuard inactive.".to_string()];
        return;
    };

    let tg = &config.treeguard;
    *tick_seconds = tg.tick_seconds.max(1);

    if runtime_state
        .last_dry_run
        .is_some_and(|prev| prev != tg.dry_run)
    {
        runtime_state.link_states.clear();
        runtime_state.circuit_states.clear();
        push_activity(
            activity,
            TreeguardActivityEntry {
                time: now_unix.to_string(),
                entity_type: "treeguard".to_string(),
                entity_id: "treeguard".to_string(),
                action: "dry_run_toggled".to_string(),
                persisted: false,
                reason: "Dry-run mode changed; state machines reset.".to_string(),
            },
        );
    }
    runtime_state.last_dry_run = Some(tg.dry_run);

    if pause_for_bakery_reload(status, tick_seconds, runtime_state, tg.enabled, tg.dry_run) {
        return;
    }

    let link_states = &mut runtime_state.link_states;
    let circuit_states = &mut runtime_state.circuit_states;
    let managed_nodes = &mut runtime_state.managed_nodes;
    let managed_device_ids = &mut runtime_state.managed_device_ids;
    let duplicate_device_conflict_circuits = &mut runtime_state.duplicate_device_conflict_circuits;
    let reload_controller = &mut runtime_state.reload_controller;

    let mut virtualized_nodes: usize = 0;
    let mut fq_codel_circuits: usize = 0;

    let top_level_auto_virtualize = tg.links.enabled && tg.links.top_level_auto_virtualize;
    if tg.enabled
        && !tg.links.all_nodes
        && tg.links.nodes.is_empty()
        && !top_level_auto_virtualize
        && !tg.circuits.all_circuits
        && tg.circuits.circuits.is_empty()
    {
        warnings.push(
            "TreeGuard is enabled but no nodes/circuits are allowlisted. No actions will occur."
                .to_string(),
        );
    } else if tg.enabled
        && !tg.links.all_nodes
        && tg.links.nodes.is_empty()
        && top_level_auto_virtualize
        && !tg.circuits.all_circuits
        && tg.circuits.circuits.is_empty()
    {
        warnings.push(
            "TreeGuard is enabled with empty allowlists; only top-level auto-virtualization may occur."
                .to_string(),
        );
    }

    let cpu_max_pct = (|| -> Option<u8> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        system_usage_tx.send(tx).ok()?;
        let reply = rx.blocking_recv().ok()?;
        let max = reply.cpu_usage.iter().copied().max()?;
        Some(max.min(100) as u8)
    })();

    if tg.enabled && cpu_max_pct.is_none() {
        warnings
            .push("Unable to sample CPU usage; CPU-aware behavior may be degraded.".to_string());
    }

    let managed_nodes_count: usize = if tg.links.all_nodes {
        let reader = NETWORK_JSON.read();
        reader
            .get_nodes_when_ready()
            .iter()
            .filter(|n| n.name != "Root")
            .count()
    } else {
        let mut enrolled: FxHashSet<String> = tg.links.nodes.iter().cloned().collect();
        if top_level_auto_virtualize {
            let reader = NETWORK_JSON.read();
            for node in reader.get_nodes_when_ready().iter() {
                if node.name != "Root" && node.immediate_parent == Some(0) {
                    enrolled.insert(node.name.clone());
                }
            }
        }
        enrolled.len()
    };

    let managed_circuits_count: usize = if tg.circuits.all_circuits {
        let shaped = SHAPED_DEVICES.load();
        let mut circuits: FxHashSet<&str> = FxHashSet::default();
        for d in shaped.devices.iter() {
            let id = d.circuit_id.trim();
            if !id.is_empty() {
                circuits.insert(id);
            }
        }
        circuits.len()
    } else {
        tg.circuits.circuits.len()
    };

    status.enabled = tg.enabled;
    status.dry_run = tg.dry_run;
    status.cpu_max_pct = cpu_max_pct;
    status.managed_nodes = managed_nodes_count;
    status.managed_circuits = managed_circuits_count;
    status.warnings = warnings;

    let (operator_overrides_snapshot, treeguard_overrides_snapshot) =
        if tg.enabled && (tg.links.enabled || tg.circuits.enabled) {
            let operator = match OverrideStore::load_layer(OverrideLayer::Operator) {
                Ok(o) => Some(o),
                Err(e) => {
                    status.warnings.push(format!(
                        "TreeGuard: unable to load operator overrides file: {e}"
                    ));
                    None
                }
            };
            let treeguard = match OverrideStore::load_layer(OverrideLayer::Treeguard) {
                Ok(o) => Some(o),
                Err(e) => {
                    status.warnings.push(format!(
                        "TreeGuard: unable to load TreeGuard overrides file: {e}"
                    ));
                    None
                }
            };
            (operator, treeguard)
        } else {
            (None, None)
        };

    // Conflict detection: if operator-defined overrides exist for an enrolled entity, TreeGuard
    // will refuse to manage it to avoid fights/surprises.
    let operator_virtual_node_overrides: FxHashSet<String> = operator_overrides_snapshot
        .as_ref()
        .map(|o| {
            o.network_adjustments()
                .iter()
                .filter_map(|adj| match adj {
                    NetworkAdjustment::SetNodeVirtual { node_name, .. } => Some(node_name.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    let operator_sqm_device_overrides: FxHashSet<String> = operator_overrides_snapshot
        .as_ref()
        .map(overrides_sqm_device_ids)
        .unwrap_or_default();

    // --- Link sampling + decisions (virtualization) ---
    let manage_links = tg.enabled && tg.links.enabled;
    let allowlisted_nodes: FxHashSet<String> = tg.links.nodes.iter().cloned().collect();

    // Cleanup for removed nodes or disabled links.
    if !manage_links {
        let removed: Vec<String> = match OverrideStore::load_layer(OverrideLayer::Treeguard) {
            Ok(of) => of
                .network_adjustments()
                .iter()
                .filter_map(|adj| match adj {
                    NetworkAdjustment::SetNodeVirtual { node_name, .. } => Some(node_name.clone()),
                    _ => None,
                })
                .collect(),
            Err(e) => {
                status.warnings.push(format!(
                    "TreeGuard links: unable to load TreeGuard overrides for cleanup: {e}"
                ));
                Vec::new()
            }
        };
        for node_name in removed {
            match overrides::clear_node_virtual(&node_name) {
                Ok(changed) => {
                    if changed {
                        reload_controller.request_reload(
                            ReloadPriority::Normal,
                            format!("Cleared virtual override for node '{node_name}'"),
                        );
                        push_activity(
                            activity,
                            TreeguardActivityEntry {
                                time: now_unix.to_string(),
                                entity_type: "node".to_string(),
                                entity_id: node_name.clone(),
                                action: "clear_virtual_override".to_string(),
                                persisted: true,
                                reason: "TreeGuard disabled or links disabled".to_string(),
                            },
                        );
                    }
                    managed_nodes.remove(&node_name);
                    link_states.remove(&node_name);
                }
                Err(e) => {
                    status.warnings.push(format!(
                        "TreeGuard links: failed to clear virtual override for node '{node_name}': {e}"
                    ));
                    push_activity(
                        activity,
                        TreeguardActivityEntry {
                            time: now_unix.to_string(),
                            entity_type: "node".to_string(),
                            entity_id: node_name,
                            action: "clear_virtual_override_failed".to_string(),
                            persisted: false,
                            reason: format!("Overrides write failed: {e}"),
                        },
                    );
                }
            }
        }
    } else {
        let reader = NETWORK_JSON.read();
        let top_level_nodes: FxHashSet<String> = if top_level_auto_virtualize && !tg.links.all_nodes
        {
            reader
                .get_nodes_when_ready()
                .iter()
                .filter(|n| n.name != "Root" && n.immediate_parent == Some(0))
                .map(|n| n.name.clone())
                .collect()
        } else {
            FxHashSet::default()
        };

        // Reconcile nodes removed from allowlist, or removed from network.json.
        let treeguard_nodes_with_overrides: FxHashSet<String> = treeguard_overrides_snapshot
            .as_ref()
            .map(|of| {
                of.network_adjustments()
                    .iter()
                    .filter_map(|adj| match adj {
                        NetworkAdjustment::SetNodeVirtual { node_name, .. } => {
                            Some(node_name.clone())
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let removed: Vec<String> = if tg.links.all_nodes {
            let current: FxHashSet<&str> = reader
                .get_nodes_when_ready()
                .iter()
                .filter(|n| n.name != "Root")
                .map(|n| n.name.as_str())
                .collect();
            treeguard_nodes_with_overrides
                .iter()
                .filter(|n| !current.contains(n.as_str()))
                .cloned()
                .collect()
        } else {
            treeguard_nodes_with_overrides
                .iter()
                .filter(|n| !allowlisted_nodes.contains(*n) && !top_level_nodes.contains(*n))
                .cloned()
                .collect()
        };
        for node_name in removed {
            match overrides::clear_node_virtual(&node_name) {
                Ok(changed) => {
                    if changed {
                        reload_controller.request_reload(
                            ReloadPriority::Normal,
                            format!("Cleared virtual override for node '{node_name}'"),
                        );
                        push_activity(
                            activity,
                            TreeguardActivityEntry {
                                time: now_unix.to_string(),
                                entity_type: "node".to_string(),
                                entity_id: node_name.clone(),
                                action: "clear_virtual_override".to_string(),
                                persisted: true,
                                reason: "Node removed from allowlist".to_string(),
                            },
                        );
                    }
                    managed_nodes.remove(&node_name);
                    link_states.remove(&node_name);
                }
                Err(e) => {
                    status.warnings.push(format!(
                        "TreeGuard links: failed to clear virtual override for node '{node_name}': {e}"
                    ));
                    push_activity(
                        activity,
                        TreeguardActivityEntry {
                            time: now_unix.to_string(),
                            entity_type: "node".to_string(),
                            entity_id: node_name,
                            action: "clear_virtual_override_failed".to_string(),
                            persisted: false,
                            reason: format!("Overrides write failed: {e}"),
                        },
                    );
                }
            }
        }
        if tg.links.all_nodes {
            for node in reader.get_nodes_when_ready().iter() {
                let node_name = node.name.as_str();
                if node_name == "Root" {
                    continue;
                }

                if operator_virtual_node_overrides.contains(node_name) {
                    status.warnings.push(format!(
                        "TreeGuard links: node '{node_name}' has an operator virtual override; TreeGuard will not manage it."
                    ));
                    match overrides::clear_node_virtual(node_name) {
                        Ok(changed) => {
                            if changed {
                                reload_controller.request_reload(
                                    ReloadPriority::Normal,
                                    format!(
                                        "Cleared virtual override for node '{node_name}' due to operator conflict"
                                    ),
                                );
                                push_activity(
                                    activity,
                                    TreeguardActivityEntry {
                                        time: now_unix.to_string(),
                                        entity_type: "node".to_string(),
                                        entity_id: node_name.to_string(),
                                        action: "clear_virtual_override_conflict".to_string(),
                                        persisted: true,
                                        reason: "Operator override present; TreeGuard will not manage this node.".to_string(),
                                    },
                                );
                            }
                        }
                        Err(e) => {
                            status.warnings.push(format!(
                                "TreeGuard links: failed to clear virtual override for node '{node_name}' during conflict cleanup: {e}"
                            ));
                        }
                    }
                    managed_nodes.remove(node_name);
                    link_states.remove(node_name);
                    continue;
                }

                let treeguard_virtual_override = treeguard_overrides_snapshot
                    .as_ref()
                    .and_then(|overrides| overrides_node_virtual(overrides, node_name));

                if node.virtual_node {
                    status.warnings.push(format!(
                        "TreeGuard links: node '{node_name}' is marked virtual in base network.json; TreeGuard will not manage it."
                    ));
                    if treeguard_virtual_override.is_some() {
                        let needs_reload = treeguard_virtual_override == Some(false);
                        match overrides::clear_node_virtual(node_name) {
                            Ok(changed) => {
                                if changed {
                                    if needs_reload {
                                        reload_controller.request_reload(
                                            ReloadPriority::Normal,
                                            format!(
                                                "Cleared stale TreeGuard override for base-virtual node '{node_name}'"
                                            ),
                                        );
                                    }
                                    push_activity(
                                        activity,
                                        TreeguardActivityEntry {
                                            time: now_unix.to_string(),
                                            entity_type: "node".to_string(),
                                            entity_id: node_name.to_string(),
                                            action: "clear_virtual_override_base_virtual"
                                                .to_string(),
                                            persisted: true,
                                            reason: "Node is marked virtual in base network.json; TreeGuard refuses to manage base-virtual nodes.".to_string(),
                                        },
                                    );
                                }
                            }
                            Err(e) => {
                                status.warnings.push(format!(
                                    "TreeGuard links: failed to clear stale TreeGuard override for base-virtual node '{node_name}': {e}"
                                ));
                                push_activity(
                                    activity,
                                    TreeguardActivityEntry {
                                        time: now_unix.to_string(),
                                        entity_type: "node".to_string(),
                                        entity_id: node_name.to_string(),
                                        action: "clear_virtual_override_base_virtual_failed"
                                            .to_string(),
                                        persisted: false,
                                        reason: format!(
                                            "Failed to clear stale TreeGuard override for base-virtual node: {e}"
                                        ),
                                    },
                                );
                            }
                        }
                    }
                    managed_nodes.remove(node_name);
                    link_states.remove(node_name);
                    continue;
                }

                let cap_down = node.max_throughput.0;
                let cap_up = node.max_throughput.1;
                if cap_down <= 0.0 || cap_up <= 0.0 {
                    status.warnings.push(format!(
                        "TreeGuard links: node '{node_name}' has unknown capacity; no changes will be made."
                    ));
                    continue;
                }

                let bytes_down = node.current_throughput.get_down() as f64;
                let bytes_up = node.current_throughput.get_up() as f64;
                let mbps_down = (bytes_down * 8.0) / 1_000_000.0;
                let mbps_up = (bytes_up * 8.0) / 1_000_000.0;
                let util_down_pct = (mbps_down / cap_down) * 100.0;
                let util_up_pct = (mbps_up / cap_up) * 100.0;

                let state = link_states.entry(node_name.to_string()).or_insert_with(|| {
                    let mut state = LinkState::default();
                    if let Some(overrides) = treeguard_overrides_snapshot.as_ref()
                        && let Some(v) = overrides_node_virtual(overrides, node_name)
                    {
                        state.desired = if v {
                            LinkVirtualState::Virtual
                        } else {
                            LinkVirtualState::Physical
                        };
                    }
                    state
                });
                prune_recent_changes(&mut state.recent_changes_unix, now_unix);

                let ewma_down = state
                    .down
                    .util_ewma_pct
                    .update(util_down_pct, UTIL_EWMA_ALPHA);
                let ewma_up = state.up.util_ewma_pct.update(util_up_pct, UTIL_EWMA_ALPHA);

                // Per-direction idle tracking (sustained-idle is evaluated across both directions).
                update_idle_since(
                    &mut state.down.idle_since_unix,
                    now_unix,
                    ewma_down,
                    tg.links.idle_util_pct as f64,
                );
                update_idle_since(
                    &mut state.up.idle_since_unix,
                    now_unix,
                    ewma_up,
                    tg.links.idle_util_pct as f64,
                );

                let sustained_idle = is_sustained_idle(
                    now_unix,
                    state.down.idle_since_unix,
                    state.up.idle_since_unix,
                    tg.links.idle_min_minutes,
                );

                let is_top_level = top_level_auto_virtualize && node.immediate_parent == Some(0);
                let top_level_safe_util_pct =
                    tg.links.top_level_safe_util_pct.clamp(0.0, 100.0) as f64;
                if is_top_level {
                    update_below_since(
                        &mut state.down.top_level_safe_since_unix,
                        now_unix,
                        ewma_down,
                        top_level_safe_util_pct,
                    );
                    update_below_since(
                        &mut state.up.top_level_safe_since_unix,
                        now_unix,
                        ewma_up,
                        top_level_safe_util_pct,
                    );
                }

                let rtt_missing = match now_nanos_since_boot {
                    None => true,
                    Some(now_nanos) => {
                        if node.rtt_buffer.last_seen == 0 {
                            true
                        } else {
                            let age_nanos = now_nanos.saturating_sub(node.rtt_buffer.last_seen);
                            age_nanos
                                >= u64::from(tg.links.rtt_missing_seconds)
                                    .saturating_mul(1_000_000_000)
                        }
                    }
                };

                // QoO (when available) from the node heatmap blocks (latest non-None sample).
                let qoo = node
                    .qoq_heatmap
                    .as_ref()
                    .map(|heatmap| {
                        let blocks = heatmap.blocks();
                        let latest = |values: &[Option<f32>]| values.iter().rev().find_map(|v| *v);
                        DownUpOrder {
                            down: latest(&blocks.download_total),
                            up: latest(&blocks.upload_total),
                        }
                    })
                    .unwrap_or(DownUpOrder {
                        down: None,
                        up: None,
                    });

                let util_ewma_pct = DownUpOrder {
                    down: ewma_down,
                    up: ewma_up,
                };

                let decision = if is_top_level {
                    let dwell_secs = u64::from(tg.links.min_state_dwell_minutes).saturating_mul(60);
                    let in_dwell_window = state
                        .last_change_unix
                        .is_some_and(|last| now_unix.saturating_sub(last) < dwell_secs);
                    let rate_limited = if tg.links.max_link_changes_per_hour == 0 {
                        true
                    } else {
                        state.recent_changes_unix.len()
                            >= tg.links.max_link_changes_per_hour as usize
                    };
                    if in_dwell_window || rate_limited {
                        decisions::LinkVirtualDecision::NoChange
                    } else {
                        let sustained_safe = is_sustained_window(
                            now_unix,
                            state.down.top_level_safe_since_unix,
                            state.up.top_level_safe_since_unix,
                            TOP_LEVEL_SAFE_SUSTAIN_MINUTES,
                        );
                        let util_high = util_ewma_pct.down >= top_level_safe_util_pct
                            || util_ewma_pct.up >= top_level_safe_util_pct;
                        match state.desired {
                            LinkVirtualState::Physical => {
                                if sustained_safe {
                                    decisions::LinkVirtualDecision::Set(LinkVirtualState::Virtual)
                                } else {
                                    decisions::LinkVirtualDecision::NoChange
                                }
                            }
                            LinkVirtualState::Virtual => {
                                if util_high {
                                    decisions::LinkVirtualDecision::Set(LinkVirtualState::Physical)
                                } else {
                                    decisions::LinkVirtualDecision::NoChange
                                }
                            }
                        }
                    }
                } else {
                    decisions::decide_link_virtualization(decisions::LinkVirtualizationInput {
                        now_unix,
                        allowlisted: true,
                        cpu_max_pct,
                        cpu_cfg: &tg.cpu,
                        links_cfg: &tg.links,
                        qoo_cfg: &tg.qoo,
                        rtt_missing,
                        qoo,
                        util_ewma_pct,
                        sustained_idle,
                        state,
                    })
                };

                if let decisions::LinkVirtualDecision::Set(target) = decision {
                    if target == state.desired {
                        continue;
                    }

                    let persist = !tg.dry_run;
                    let mut persisted_ok = false;
                    let mut override_changed = false;

                    if persist {
                        let new_virtual = target == LinkVirtualState::Virtual;
                        match overrides::set_node_virtual(node_name, new_virtual) {
                            Ok(changed) => {
                                persisted_ok = true;
                                override_changed = changed;
                            }
                            Err(e) => {
                                status.warnings.push(format!(
                                    "TreeGuard links: failed to persist virtual override for node '{node_name}': {e}"
                                ));
                                managed_nodes.insert(node_name.to_string());
                                push_activity(
                                    activity,
                                    TreeguardActivityEntry {
                                        time: now_unix.to_string(),
                                        entity_type: "node".to_string(),
                                        entity_id: node_name.to_string(),
                                        action: "set_virtual_override_failed".to_string(),
                                        persisted: false,
                                        reason: format!("Overrides write failed: {e}"),
                                    },
                                );
                                continue;
                            }
                        }
                    }

                    if override_changed {
                        let priority = if target == LinkVirtualState::Physical {
                            ReloadPriority::Urgent
                        } else {
                            ReloadPriority::Normal
                        };
                        reload_controller.request_reload(
                            priority,
                            format!("Node '{node_name}' virtualization changed"),
                        );
                    }

                    state.desired = target;
                    state.last_change_unix = Some(now_unix);
                    state.recent_changes_unix.push_back(now_unix);
                    prune_recent_changes(&mut state.recent_changes_unix, now_unix);
                    managed_nodes.insert(node_name.to_string());

                    push_activity(
                        activity,
                        TreeguardActivityEntry {
                            time: now_unix.to_string(),
                            entity_type: "node".to_string(),
                            entity_id: node_name.to_string(),
                            action: match target {
                                LinkVirtualState::Physical => "unvirtualize".to_string(),
                                LinkVirtualState::Virtual => "virtualize".to_string(),
                            },
                            persisted: persist && persisted_ok,
                            reason: if is_top_level {
                                match target {
                                    LinkVirtualState::Virtual => format!(
                                        "Top-level safe: sustained utilization below {:.1}% for {} minutes",
                                        top_level_safe_util_pct, TOP_LEVEL_SAFE_SUSTAIN_MINUTES
                                    ),
                                    LinkVirtualState::Physical => format!(
                                        "Top-level unsafe: utilization above {:.1}%",
                                        top_level_safe_util_pct
                                    ),
                                }
                            } else {
                                "Decision policy matched".to_string()
                            },
                        },
                    );
                    status.last_action_summary = Some(format!(
                        "{} node '{}'",
                        if target == LinkVirtualState::Virtual {
                            "Virtualized"
                        } else {
                            "Unvirtualized"
                        },
                        node_name
                    ));
                } else {
                    managed_nodes.insert(node_name.to_string());
                }

                if state.desired == LinkVirtualState::Virtual {
                    virtualized_nodes += 1;
                }
            }
        } else {
            let mut enrolled_nodes: Vec<String> = tg.links.nodes.clone();
            if top_level_auto_virtualize {
                enrolled_nodes.extend(top_level_nodes.iter().cloned());
            }
            enrolled_nodes.sort();
            enrolled_nodes.dedup();

            for node_name in enrolled_nodes.iter() {
                if operator_virtual_node_overrides.contains(node_name) {
                    status.warnings.push(format!(
                        "TreeGuard links: node '{node_name}' has an operator virtual override; TreeGuard will not manage it."
                    ));
                    match overrides::clear_node_virtual(node_name) {
                        Ok(changed) => {
                            if changed {
                                reload_controller.request_reload(
	                                ReloadPriority::Normal,
	                                format!(
	                                    "Cleared virtual override for node '{node_name}' due to operator conflict"
	                                ),
	                            );
                                push_activity(
	                                activity,
	                                TreeguardActivityEntry {
	                                    time: now_unix.to_string(),
                                    entity_type: "node".to_string(),
                                    entity_id: node_name.clone(),
                                    action: "clear_virtual_override_conflict".to_string(),
                                    persisted: true,
                                    reason: "Operator override present; TreeGuard will not manage this node.".to_string(),
                                },
                            );
                            }
                        }
                        Err(e) => {
                            status.warnings.push(format!(
                            "TreeGuard links: failed to clear virtual override for node '{node_name}' during conflict cleanup: {e}"
                        ));
                        }
                    }
                    managed_nodes.remove(node_name);
                    link_states.remove(node_name);
                    continue;
                }
                let Some(index) = reader.get_index_for_name(node_name) else {
                    status.warnings.push(format!(
                        "TreeGuard links allowlist: node '{node_name}' not found in network.json."
                    ));
                    continue;
                };
                let Some(node) = reader.get_nodes_when_ready().get(index) else {
                    status.warnings.push(format!(
                        "TreeGuard links allowlist: node '{node_name}' index not present."
                    ));
                    continue;
                };

                let treeguard_virtual_override = treeguard_overrides_snapshot
                    .as_ref()
                    .and_then(|overrides| overrides_node_virtual(overrides, node_name));

                if node.virtual_node {
                    status.warnings.push(format!(
                        "TreeGuard links: node '{node_name}' is marked virtual in base network.json; TreeGuard will not manage it."
                    ));
                    if treeguard_virtual_override.is_some() {
                        let needs_reload = treeguard_virtual_override == Some(false);
                        match overrides::clear_node_virtual(node_name) {
                            Ok(changed) => {
                                if changed {
                                    if needs_reload {
                                        reload_controller.request_reload(
                                            ReloadPriority::Normal,
                                            format!(
                                                "Cleared stale TreeGuard override for base-virtual node '{node_name}'"
                                            ),
                                        );
                                    }
                                    push_activity(
                                        activity,
                                        TreeguardActivityEntry {
                                            time: now_unix.to_string(),
                                            entity_type: "node".to_string(),
                                            entity_id: node_name.clone(),
                                            action: "clear_virtual_override_base_virtual"
                                                .to_string(),
                                            persisted: true,
                                            reason: "Node is marked virtual in base network.json; TreeGuard refuses to manage base-virtual nodes.".to_string(),
                                        },
                                    );
                                }
                            }
                            Err(e) => {
                                status.warnings.push(format!(
                                    "TreeGuard links: failed to clear stale TreeGuard override for base-virtual node '{node_name}': {e}"
                                ));
                                push_activity(
                                    activity,
                                    TreeguardActivityEntry {
                                        time: now_unix.to_string(),
                                        entity_type: "node".to_string(),
                                        entity_id: node_name.clone(),
                                        action: "clear_virtual_override_base_virtual_failed"
                                            .to_string(),
                                        persisted: false,
                                        reason: format!(
                                            "Failed to clear stale TreeGuard override for base-virtual node: {e}"
                                        ),
                                    },
                                );
                            }
                        }
                    }
                    managed_nodes.remove(node_name);
                    link_states.remove(node_name);
                    continue;
                }

                let cap_down = node.max_throughput.0;
                let cap_up = node.max_throughput.1;
                if cap_down <= 0.0 || cap_up <= 0.0 {
                    status.warnings.push(format!(
                    "TreeGuard links: node '{node_name}' has unknown capacity; no changes will be made."
                ));
                    continue;
                }

                let bytes_down = node.current_throughput.get_down() as f64;
                let bytes_up = node.current_throughput.get_up() as f64;
                let mbps_down = (bytes_down * 8.0) / 1_000_000.0;
                let mbps_up = (bytes_up * 8.0) / 1_000_000.0;
                let util_down_pct = (mbps_down / cap_down) * 100.0;
                let util_up_pct = (mbps_up / cap_up) * 100.0;

                let state = link_states.entry(node_name.clone()).or_insert_with(|| {
                    let mut state = LinkState::default();
                    if let Some(overrides) = treeguard_overrides_snapshot.as_ref()
                        && let Some(v) = overrides_node_virtual(overrides, node_name)
                    {
                        state.desired = if v {
                            LinkVirtualState::Virtual
                        } else {
                            LinkVirtualState::Physical
                        };
                    }
                    state
                });
                prune_recent_changes(&mut state.recent_changes_unix, now_unix);

                let ewma_down = state
                    .down
                    .util_ewma_pct
                    .update(util_down_pct, UTIL_EWMA_ALPHA);
                let ewma_up = state.up.util_ewma_pct.update(util_up_pct, UTIL_EWMA_ALPHA);

                // Per-direction idle tracking (sustained-idle is evaluated across both directions).
                update_idle_since(
                    &mut state.down.idle_since_unix,
                    now_unix,
                    ewma_down,
                    tg.links.idle_util_pct as f64,
                );
                update_idle_since(
                    &mut state.up.idle_since_unix,
                    now_unix,
                    ewma_up,
                    tg.links.idle_util_pct as f64,
                );

                let sustained_idle = is_sustained_idle(
                    now_unix,
                    state.down.idle_since_unix,
                    state.up.idle_since_unix,
                    tg.links.idle_min_minutes,
                );

                let is_top_level = top_level_auto_virtualize && node.immediate_parent == Some(0);
                let top_level_safe_util_pct =
                    tg.links.top_level_safe_util_pct.clamp(0.0, 100.0) as f64;
                if is_top_level {
                    update_below_since(
                        &mut state.down.top_level_safe_since_unix,
                        now_unix,
                        ewma_down,
                        top_level_safe_util_pct,
                    );
                    update_below_since(
                        &mut state.up.top_level_safe_since_unix,
                        now_unix,
                        ewma_up,
                        top_level_safe_util_pct,
                    );
                }

                let rtt_missing = match now_nanos_since_boot {
                    None => true,
                    Some(now_nanos) => {
                        if node.rtt_buffer.last_seen == 0 {
                            true
                        } else {
                            let age_nanos = now_nanos.saturating_sub(node.rtt_buffer.last_seen);
                            age_nanos
                                >= u64::from(tg.links.rtt_missing_seconds)
                                    .saturating_mul(1_000_000_000)
                        }
                    }
                };

                // QoO (when available) from the node heatmap blocks (latest non-None sample).
                let qoo = node
                    .qoq_heatmap
                    .as_ref()
                    .map(|heatmap| {
                        let blocks = heatmap.blocks();
                        let latest = |values: &[Option<f32>]| values.iter().rev().find_map(|v| *v);
                        DownUpOrder {
                            down: latest(&blocks.download_total),
                            up: latest(&blocks.upload_total),
                        }
                    })
                    .unwrap_or(DownUpOrder {
                        down: None,
                        up: None,
                    });

                let util_ewma_pct = DownUpOrder {
                    down: ewma_down,
                    up: ewma_up,
                };

                let decision = if is_top_level {
                    let dwell_secs = u64::from(tg.links.min_state_dwell_minutes).saturating_mul(60);
                    let in_dwell_window = state
                        .last_change_unix
                        .is_some_and(|last| now_unix.saturating_sub(last) < dwell_secs);
                    let rate_limited = if tg.links.max_link_changes_per_hour == 0 {
                        true
                    } else {
                        state.recent_changes_unix.len()
                            >= tg.links.max_link_changes_per_hour as usize
                    };
                    if in_dwell_window || rate_limited {
                        decisions::LinkVirtualDecision::NoChange
                    } else {
                        let sustained_safe = is_sustained_window(
                            now_unix,
                            state.down.top_level_safe_since_unix,
                            state.up.top_level_safe_since_unix,
                            TOP_LEVEL_SAFE_SUSTAIN_MINUTES,
                        );
                        let util_high = util_ewma_pct.down >= top_level_safe_util_pct
                            || util_ewma_pct.up >= top_level_safe_util_pct;
                        match state.desired {
                            LinkVirtualState::Physical => {
                                if sustained_safe {
                                    decisions::LinkVirtualDecision::Set(LinkVirtualState::Virtual)
                                } else {
                                    decisions::LinkVirtualDecision::NoChange
                                }
                            }
                            LinkVirtualState::Virtual => {
                                if util_high {
                                    decisions::LinkVirtualDecision::Set(LinkVirtualState::Physical)
                                } else {
                                    decisions::LinkVirtualDecision::NoChange
                                }
                            }
                        }
                    }
                } else {
                    decisions::decide_link_virtualization(decisions::LinkVirtualizationInput {
                        now_unix,
                        allowlisted: allowlisted_nodes.contains(node_name),
                        cpu_max_pct,
                        cpu_cfg: &tg.cpu,
                        links_cfg: &tg.links,
                        qoo_cfg: &tg.qoo,
                        rtt_missing,
                        qoo,
                        util_ewma_pct,
                        sustained_idle,
                        state,
                    })
                };

                if let decisions::LinkVirtualDecision::Set(target) = decision {
                    if target == state.desired {
                        continue;
                    }

                    let persist = !tg.dry_run;
                    let mut persisted_ok = false;
                    let mut override_changed = false;

                    if persist {
                        let new_virtual = target == LinkVirtualState::Virtual;
                        match overrides::set_node_virtual(node_name, new_virtual) {
                            Ok(changed) => {
                                persisted_ok = true;
                                override_changed = changed;
                            }
                            Err(e) => {
                                status.warnings.push(format!(
                                "TreeGuard links: failed to persist virtual override for node '{node_name}': {e}"
                            ));
                                managed_nodes.insert(node_name.clone());
                                push_activity(
                                    activity,
                                    TreeguardActivityEntry {
                                        time: now_unix.to_string(),
                                        entity_type: "node".to_string(),
                                        entity_id: node_name.clone(),
                                        action: "set_virtual_override_failed".to_string(),
                                        persisted: false,
                                        reason: format!("Overrides write failed: {e}"),
                                    },
                                );
                                continue;
                            }
                        }
                    }

                    if override_changed {
                        let priority = if target == LinkVirtualState::Physical {
                            ReloadPriority::Urgent
                        } else {
                            ReloadPriority::Normal
                        };
                        reload_controller.request_reload(
                            priority,
                            format!("Node '{}' virtualization changed", node_name.clone()),
                        );
                    }
                    state.desired = target;
                    state.last_change_unix = Some(now_unix);
                    state.recent_changes_unix.push_back(now_unix);
                    prune_recent_changes(&mut state.recent_changes_unix, now_unix);
                    managed_nodes.insert(node_name.clone());

                    push_activity(
                        activity,
                        TreeguardActivityEntry {
                            time: now_unix.to_string(),
                            entity_type: "node".to_string(),
                            entity_id: node_name.clone(),
                            action: match target {
                                LinkVirtualState::Physical => "unvirtualize".to_string(),
                                LinkVirtualState::Virtual => "virtualize".to_string(),
                            },
                            persisted: persist && persisted_ok,
                            reason: if is_top_level {
                                match target {
                                    LinkVirtualState::Virtual => format!(
                                        "Top-level safe: sustained utilization below {:.1}% for {} minutes",
                                        top_level_safe_util_pct, TOP_LEVEL_SAFE_SUSTAIN_MINUTES
                                    ),
                                    LinkVirtualState::Physical => format!(
                                        "Top-level unsafe: utilization above {:.1}%",
                                        top_level_safe_util_pct
                                    ),
                                }
                            } else {
                                "Decision policy matched".to_string()
                            },
                        },
                    );
                    status.last_action_summary = Some(format!(
                        "{} node '{}'",
                        if target == LinkVirtualState::Virtual {
                            "Virtualized"
                        } else {
                            "Unvirtualized"
                        },
                        node_name
                    ));
                } else {
                    managed_nodes.insert(node_name.clone());
                }

                if state.desired == LinkVirtualState::Virtual {
                    virtualized_nodes += 1;
                }
            }
        }
    }

    if let Some(attempt) = reload_controller.poll_reload(now_unix, tg.links.reload_cooldown_minutes)
    {
        let crate::treeguard::reload::ReloadAttempt {
            outcome,
            request_reason,
        } = attempt;

        match outcome {
            ReloadOutcome::Success { message: _ } => {
                let why = request_reason.unwrap_or_else(|| "Topology change".to_string());
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "treeguard".to_string(),
                        entity_id: "reload".to_string(),
                        action: "reload_success".to_string(),
                        persisted: true,
                        reason: why.clone(),
                    },
                );
                status.last_action_summary = Some(format!("Reloaded LibreQoS: {why}"));
            }
            ReloadOutcome::Skipped {
                reason,
                next_allowed_unix,
            } => {
                let extra = next_allowed_unix
                    .map(|t| format!(" next_allowed_unix={t}"))
                    .unwrap_or_default();
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "treeguard".to_string(),
                        entity_id: "reload".to_string(),
                        action: "reload_skipped".to_string(),
                        persisted: false,
                        reason: format!("{reason}.{extra}"),
                    },
                );
            }
            ReloadOutcome::Failed {
                error,
                next_allowed_unix,
            } => {
                let why = request_reason.unwrap_or_else(|| "Topology change".to_string());
                let extra = next_allowed_unix
                    .map(|t| format!(" next_allowed_unix={t}"))
                    .unwrap_or_default();
                status
                    .warnings
                    .push(format!("TreeGuard reload failed: {error}.{extra}"));
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "treeguard".to_string(),
                        entity_id: "reload".to_string(),
                        action: "reload_failed".to_string(),
                        persisted: false,
                        reason: format!("{error}.{extra}"),
                    },
                );
                urgent::submit(
                    UrgentSource::System,
                    UrgentSeverity::Error,
                    "treeguard_reload_failed".to_string(),
                    format!("TreeGuard failed to reload LibreQoS: {error}"),
                    Some(why),
                    Some("treeguard_reload".to_string()),
                );
            }
        }
    }

    // --- Circuit sampling + decisions (SQM switching) ---
    let manage_circuits = tg.enabled && tg.circuits.enabled;

    // Snapshot shaped devices so we can compute the enrolled circuit set.
    let shaped = SHAPED_DEVICES.load();

    let enrolled_circuits: Vec<String> = if tg.circuits.all_circuits {
        let mut set: FxHashSet<String> = FxHashSet::default();
        for d in shaped.devices.iter() {
            let id = d.circuit_id.trim();
            if !id.is_empty() {
                set.insert(id.to_string());
            }
        }
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    } else {
        let mut v = tg.circuits.circuits.clone();
        v.sort();
        v.dedup();
        v
    };

    let allowlisted_circuits: FxHashSet<String> = enrolled_circuits.iter().cloned().collect();
    status.managed_circuits = enrolled_circuits.len();

    // Compute desired device_id set from enrolled circuits.
    let desired_device_ids: FxHashSet<String> = if manage_circuits {
        if tg.circuits.all_circuits {
            shaped.devices.iter().map(|d| d.device_id.clone()).collect()
        } else {
            shaped
                .devices
                .iter()
                .filter(|d| allowlisted_circuits.contains(&d.circuit_id))
                .map(|d| d.device_id.clone())
                .collect()
        }
    } else {
        FxHashSet::default()
    };

    // Cleanup for removed circuits or disabled circuits/TreeGuard.
    if !manage_circuits {
        let removed: Vec<String> = match OverrideStore::load_layer(OverrideLayer::Treeguard) {
            Ok(of) => overrides_sqm_device_ids(&of).into_iter().collect(),
            Err(e) => {
                status.warnings.push(format!(
                    "TreeGuard circuits: unable to load TreeGuard overrides for cleanup: {e}"
                ));
                Vec::new()
            }
        };
        if !removed.is_empty() {
            match overrides::clear_device_overrides(&removed) {
                Ok(changed) => {
                    if changed {
                        push_activity(
                            activity,
                            TreeguardActivityEntry {
                                time: now_unix.to_string(),
                                entity_type: "circuits".to_string(),
                                entity_id: "*".to_string(),
                                action: "clear_sqm_overrides".to_string(),
                                persisted: true,
                                reason: "TreeGuard disabled or circuits disabled".to_string(),
                            },
                        );
                    }
                }
                Err(e) => {
                    status.warnings.push(format!(
                        "TreeGuard circuits: failed to clear TreeGuard SQM overlays during cleanup: {e}"
                    ));
                }
            }
        }
        managed_device_ids.clear();
        duplicate_device_conflict_circuits.clear();
        circuit_states.clear();
    } else {
        let mut circuits_by_device_id: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
        circuits_by_device_id.reserve(shaped.devices.len());
        for device in shaped.devices.iter() {
            let device_id = device.device_id.trim();
            let circuit_id = device.circuit_id.trim();
            if device_id.is_empty() || circuit_id.is_empty() {
                continue;
            }
            circuits_by_device_id
                .entry(device_id.to_string())
                .or_default()
                .insert(circuit_id.to_string());
        }
        let duplicate_device_ids: FxHashMap<String, Vec<String>> = circuits_by_device_id
            .into_iter()
            .filter_map(|(device_id, circuits)| {
                if circuits.len() <= 1 {
                    return None;
                }
                let mut circuits: Vec<String> = circuits.into_iter().collect();
                circuits.sort();
                Some((device_id, circuits))
            })
            .collect();
        let mut current_duplicate_device_conflict_circuits: FxHashSet<String> =
            FxHashSet::default();

        // Reconcile device IDs removed from allowlisted circuits.
        let treeguard_device_ids_with_overrides: FxHashSet<String> = treeguard_overrides_snapshot
            .as_ref()
            .map(overrides_sqm_device_ids)
            .unwrap_or_default();
        let removed: Vec<String> = treeguard_device_ids_with_overrides
            .iter()
            .filter(|d| !desired_device_ids.contains(*d))
            .cloned()
            .collect();
        if !removed.is_empty() {
            match overrides::clear_device_overrides(&removed) {
                Ok(changed) => {
                    if changed {
                        push_activity(
                            activity,
                            TreeguardActivityEntry {
                                time: now_unix.to_string(),
                                entity_type: "circuits".to_string(),
                                entity_id: "*".to_string(),
                                action: "clear_sqm_overrides".to_string(),
                                persisted: true,
                                reason: "Device removed from allowlisted circuits".to_string(),
                            },
                        );
                    }
                }
                Err(e) => {
                    status.warnings.push(format!(
                        "TreeGuard circuits: failed to clear TreeGuard SQM overlays for removed devices: {e}"
                    ));
                }
            }
            for device_id in removed.iter() {
                managed_device_ids.remove(device_id);
            }
        }

        // Snapshot RTT buffers by circuit hash.
        let rtt_snapshot = CIRCUIT_RTT_BUFFERS.load();

        let allow_hashes: Option<FxHashSet<i64>> = if tg.circuits.all_circuits {
            None
        } else {
            Some(enrolled_circuits.iter().map(|id| hash_to_i64(id)).collect())
        };

        // Capacity lookup by circuit hash (max down/up Mbps across devices in the circuit).
        let mut capacity_by_circuit: FxHashMap<i64, (f32, f32)> = FxHashMap::default();
        capacity_by_circuit.reserve(shaped.devices.len());
        for device in shaped.devices.iter() {
            let entry = capacity_by_circuit
                .entry(device.circuit_hash)
                .or_insert((device.download_max_mbps, device.upload_max_mbps));
            if device.download_max_mbps > entry.0 {
                entry.0 = device.download_max_mbps;
            }
            if device.upload_max_mbps > entry.1 {
                entry.1 = device.upload_max_mbps;
            }
        }

        // Aggregate worst (minimum) QoO and throughput per circuit hash across devices/hosts.
        let mut qoo_by_circuit: FxHashMap<i64, DownUpOrder<Option<f32>>> = FxHashMap::default();
        let mut bps_by_circuit: FxHashMap<i64, DownUpOrder<u64>> = FxHashMap::default();
        {
            let raw = THROUGHPUT_TRACKER.raw_data.lock();
            for entry in raw.values() {
                let Some(ch) = entry.circuit_hash else {
                    continue;
                };
                if allow_hashes.as_ref().is_some_and(|h| !h.contains(&ch)) {
                    continue;
                }

                let down = entry.qoq.download_total_f32();
                let up = entry.qoq.upload_total_f32();
                let slot = qoo_by_circuit.entry(ch).or_insert(DownUpOrder {
                    down: None,
                    up: None,
                });
                slot.down = min_opt_f32(slot.down, down);
                slot.up = min_opt_f32(slot.up, up);

                let bps = bps_by_circuit
                    .entry(ch)
                    .or_insert(DownUpOrder { down: 0, up: 0 });
                bps.down = bps.down.saturating_add(entry.bytes_per_second.down);
                bps.up = bps.up.saturating_add(entry.bytes_per_second.up);
            }
        }

        for circuit_id in enrolled_circuits.iter() {
            let devices: Vec<lqos_config::ShapedDevice> = shaped
                .devices
                .iter()
                .filter(|d| d.circuit_id == circuit_id.as_str())
                .cloned()
                .collect();

            let circuit_name: Option<String> = devices.iter().find_map(|d| {
                let name = d.circuit_name.trim();
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                }
            });
            let circuit_entity_id: String = match circuit_name.as_deref() {
                Some(name) => format!("{name} ({circuit_id})"),
                None => circuit_id.clone(),
            };
            let circuit_label: String = circuit_name.unwrap_or_else(|| circuit_id.clone());

            let mut circuit_device_ids: Vec<String> =
                devices.iter().map(|d| d.device_id.clone()).collect();
            circuit_device_ids.sort();
            circuit_device_ids.dedup();
            let duplicate_details: Vec<(String, Vec<String>)> = circuit_device_ids
                .iter()
                .filter_map(|device_id| {
                    duplicate_device_ids
                        .get(device_id)
                        .map(|circuits| (device_id.clone(), circuits.clone()))
                })
                .collect();
            if !duplicate_details.is_empty() {
                current_duplicate_device_conflict_circuits.insert(circuit_id.clone());
                let duplicate_reason = duplicate_details
                    .iter()
                    .map(|(device_id, circuits)| {
                        format!(
                            "device_id '{}' is shared by circuits [{}]",
                            device_id,
                            circuits.join(", ")
                        )
                    })
                    .collect::<Vec<String>>()
                    .join("; ");
                status.warnings.push(format!(
                    "TreeGuard circuits: circuit '{circuit_id}' has duplicate device IDs; TreeGuard will not manage it. {duplicate_reason}"
                ));
                if !duplicate_device_conflict_circuits.contains(circuit_id) {
                    push_activity(
                        activity,
                        TreeguardActivityEntry {
                            time: now_unix.to_string(),
                            entity_type: "circuit".to_string(),
                            entity_id: circuit_entity_id.clone(),
                            action: "skip_duplicate_device_id".to_string(),
                            persisted: false,
                            reason: format!(
                                "TreeGuard refuses circuits with duplicate device IDs. {duplicate_reason}"
                            ),
                        },
                    );
                }
                if !circuit_device_ids.is_empty() {
                    match overrides::clear_device_overrides(&circuit_device_ids) {
                        Ok(changed) => {
                            if changed {
                                push_activity(
                                    activity,
                                    TreeguardActivityEntry {
                                        time: now_unix.to_string(),
                                        entity_type: "circuit".to_string(),
                                        entity_id: circuit_entity_id.clone(),
                                        action: "clear_sqm_overrides_duplicate_device_id"
                                            .to_string(),
                                        persisted: true,
                                        reason: "Duplicate device IDs detected; cleared TreeGuard SQM overlays and skipped management.".to_string(),
                                    },
                                );
                            }
                        }
                        Err(e) => {
                            status.warnings.push(format!(
                                "TreeGuard circuits: failed to clear TreeGuard SQM overlays for duplicate device IDs on circuit '{circuit_id}': {e}"
                            ));
                        }
                    }
                    for did in circuit_device_ids.iter() {
                        managed_device_ids.remove(did);
                    }
                }
                circuit_states.remove(circuit_id);
                continue;
            }

            let state = circuit_states.entry(circuit_id.clone()).or_insert_with(|| {
                let mut state = CircuitState::default();
                if let Some(token) = infer_circuit_sqm_override_token(
                    circuit_id,
                    &shaped.devices,
                    treeguard_overrides_snapshot.as_ref(),
                ) {
                    let parsed = decisions::parse_directional_sqm_override(&token);
                    if let Some(down) = parsed.down {
                        state.down.desired = down;
                    }
                    if let Some(up) = parsed.up {
                        state.up.desired = up;
                    }
                }
                state
            });
            let circuit_hash = hash_to_i64(circuit_id);
            let (cap_down, cap_up) = capacity_by_circuit
                .get(&circuit_hash)
                .copied()
                .unwrap_or((0.0, 0.0));
            let qoo = qoo_by_circuit
                .get(&circuit_hash)
                .cloned()
                .unwrap_or(DownUpOrder {
                    down: None,
                    up: None,
                });

            let operator_conflict = circuit_device_ids
                .iter()
                .any(|did| operator_sqm_device_overrides.contains(did));
            if operator_conflict {
                status.warnings.push(format!(
                    "TreeGuard circuits: circuit '{circuit_id}' has operator SQM overrides; TreeGuard will not manage it."
                ));
                if !circuit_device_ids.is_empty() {
                    match overrides::clear_device_overrides(&circuit_device_ids) {
                        Ok(changed) => {
                            if changed {
                                push_activity(
                                    activity,
                                    TreeguardActivityEntry {
                                        time: now_unix.to_string(),
                                        entity_type: "circuit".to_string(),
                                        entity_id: circuit_entity_id.clone(),
                                        action: "clear_sqm_overrides_conflict".to_string(),
                                        persisted: true,
                                        reason: "Operator SQM overrides present; cleared TreeGuard SQM overlays.".to_string(),
                                    },
                                );
                            }
                        }
                        Err(e) => {
                            status.warnings.push(format!(
                                "TreeGuard circuits: failed to clear TreeGuard SQM overlays for circuit '{circuit_id}' during conflict cleanup: {e}"
                            ));
                        }
                    }
                    for did in circuit_device_ids.iter() {
                        managed_device_ids.remove(did);
                    }
                }
                continue;
            }

            let fq_codel = if tg.dry_run {
                let capacity_known = cap_down > 0.0 && cap_up > 0.0;
                if !capacity_known {
                    status.warnings.push(format!(
                        "TreeGuard circuits: circuit '{circuit_id}' has unknown capacity; no changes will be made."
                    ));
                    state.down.idle_since_unix = None;
                    state.up.idle_since_unix = None;
                } else {
                    let bps = bps_by_circuit
                        .get(&circuit_hash)
                        .copied()
                        .unwrap_or(DownUpOrder { down: 0, up: 0 });
                    let mbps_down = (bps.down as f64 * 8.0) / 1_000_000.0;
                    let mbps_up = (bps.up as f64 * 8.0) / 1_000_000.0;
                    let util_down_pct = (mbps_down / cap_down as f64) * 100.0;
                    let util_up_pct = (mbps_up / cap_up as f64) * 100.0;

                    let ewma_down = state
                        .down
                        .util_ewma_pct
                        .update(util_down_pct, UTIL_EWMA_ALPHA);
                    let ewma_up = state.up.util_ewma_pct.update(util_up_pct, UTIL_EWMA_ALPHA);

                    update_idle_since(
                        &mut state.down.idle_since_unix,
                        now_unix,
                        ewma_down,
                        tg.circuits.idle_util_pct as f64,
                    );
                    update_idle_since(
                        &mut state.up.idle_since_unix,
                        now_unix,
                        ewma_up,
                        tg.circuits.idle_util_pct as f64,
                    );
                }

                let rtt_missing = match now_nanos_since_boot {
                    None => true,
                    Some(now_nanos) => match rtt_snapshot.get(&circuit_hash) {
                        None => true,
                        Some(buf) => {
                            if buf.last_seen == 0 {
                                true
                            } else {
                                let age_nanos = now_nanos.saturating_sub(buf.last_seen);
                                age_nanos
                                    >= u64::from(tg.circuits.rtt_missing_seconds)
                                        .saturating_mul(1_000_000_000)
                            }
                        }
                    },
                };

                let decision = decisions::decide_circuit_sqm(decisions::CircuitSqmInput {
                    now_unix,
                    allowlisted: allowlisted_circuits.contains(circuit_id) && capacity_known,
                    cpu_max_pct,
                    cpu_cfg: &tg.cpu,
                    circuits_cfg: &tg.circuits,
                    qoo_cfg: &tg.qoo,
                    rtt_missing,
                    qoo,
                    state,
                });

                let mut proposed_down = state.down.desired;
                let mut proposed_up = state.up.desired;
                if let Some(down) = decision.down {
                    proposed_down = down;
                }
                if let Some(up) = decision.up {
                    proposed_up = up;
                }

                let changed_down = proposed_down != state.down.desired;
                let changed_up = proposed_up != state.up.desired;

                if devices.is_empty() {
                    status.warnings.push(format!(
                        "TreeGuard circuits: circuit '{circuit_id}' has no devices in ShapedDevices.csv."
                    ));
                } else {
                    for dev in devices.iter() {
                        managed_device_ids.insert(dev.device_id.clone());
                    }
                }

                if changed_down || changed_up {
                    apply_circuit_sqm_change(
                        CircuitSqmApplyContext {
                            status,
                            activity,
                            now_unix,
                            dry_run: true,
                            persist_sqm_overrides: tg.circuits.persist_sqm_overrides,
                            circuit_id,
                            circuit_entity_id: &circuit_entity_id,
                            circuit_label: &circuit_label,
                            devices: &devices,
                        },
                        state,
                        CircuitSqmTransition {
                            proposed_down,
                            proposed_up,
                            changed_down,
                            changed_up,
                        },
                        overrides::set_devices_sqm_override,
                        |circuit_id, devices, token| {
                            bakery::apply_circuit_sqm_override_live(circuit_id, devices, token)
                        },
                    );
                }

                state.down.desired == CircuitSqmState::FqCodel
                    || state.up.desired == CircuitSqmState::FqCodel
            } else {
                process_circuit_tick(
                    CircuitTickContext {
                        status,
                        activity,
                        managed_device_ids,
                        now_unix,
                        now_nanos_since_boot,
                        cpu_max_pct,
                        circuit_id,
                        circuit_entity_id: &circuit_entity_id,
                        circuit_label: &circuit_label,
                        devices: &devices,
                        allowlisted: allowlisted_circuits.contains(circuit_id),
                        cap_down,
                        cap_up,
                        bps: bps_by_circuit
                            .get(&circuit_hash)
                            .copied()
                            .unwrap_or(DownUpOrder { down: 0, up: 0 }),
                        last_rtt_seen_nanos: rtt_snapshot
                            .get(&circuit_hash)
                            .map(|buf| buf.last_seen),
                        qoo,
                        cpu_cfg: &tg.cpu,
                        circuits_cfg: &tg.circuits,
                        qoo_cfg: &tg.qoo,
                    },
                    state,
                    overrides::set_devices_sqm_override,
                    |circuit_id, devices, token| {
                        bakery::apply_circuit_sqm_override_live(circuit_id, devices, token)
                    },
                )
            };

            if fq_codel {
                fq_codel_circuits += 1;
            }
        }
        *duplicate_device_conflict_circuits = current_duplicate_device_conflict_circuits;
    }

    status.virtualized_nodes = virtualized_nodes;
    status.fq_codel_circuits = fq_codel_circuits;
}

/// Applies TreeGuard backoff while Bakery is performing a structural full reload.
///
/// This function is not pure: it mutates TreeGuard status/runtime state and may emit logs.
fn pause_for_bakery_reload(
    status: &mut TreeguardStatusData,
    tick_seconds: &mut u64,
    runtime_state: &mut TreeguardRuntimeState,
    enabled: bool,
    dry_run: bool,
) -> bool {
    pause_for_bakery_reload_with_flag(
        status,
        tick_seconds,
        runtime_state,
        enabled,
        dry_run,
        lqos_bakery::full_reload_in_progress(),
    )
}

fn pause_for_bakery_reload_with_flag(
    status: &mut TreeguardStatusData,
    tick_seconds: &mut u64,
    runtime_state: &mut TreeguardRuntimeState,
    enabled: bool,
    dry_run: bool,
    bakery_reload_in_progress: bool,
) -> bool {
    let paused = enabled && bakery_reload_in_progress;
    if paused {
        if !runtime_state.paused_for_bakery_reload {
            info!("TreeGuard: pausing while Bakery full reload is in progress");
            runtime_state.paused_for_bakery_reload = true;
        }
        *tick_seconds = (*tick_seconds).max(5);
        status.enabled = enabled;
        status.dry_run = dry_run;
        status.cpu_max_pct = None;
        status.last_action_summary =
            Some("Paused while Bakery full reload is in progress".to_string());
        status
            .warnings
            .push("TreeGuard paused while Bakery full reload is in progress.".to_string());
        return true;
    }

    if runtime_state.paused_for_bakery_reload {
        info!("TreeGuard: resuming after Bakery full reload");
        runtime_state.paused_for_bakery_reload = false;
    }

    false
}

/// Returns the current `set_node_virtual` override value for `node_name`, if present.
///
/// This function is pure: it has no side effects.
fn overrides_node_virtual(overrides: &OverrideFile, node_name: &str) -> Option<bool> {
    overrides
        .network_adjustments()
        .iter()
        .find_map(|adj| match adj {
            NetworkAdjustment::SetNodeVirtual {
                node_name: n,
                virtual_node,
            } if n == node_name => Some(*virtual_node),
            _ => None,
        })
}

/// Returns the current SQM override token for `device_id`, if present.
///
/// This function is pure: it has no side effects.
fn overrides_device_sqm(overrides: &OverrideFile, device_id: &str) -> Option<String> {
    for adj in overrides.circuit_adjustments() {
        if let lqos_overrides::CircuitAdjustment::DeviceAdjustSqm {
            device_id: current,
            sqm_override,
        } = adj
        {
            if current != device_id {
                continue;
            }
            return sqm_override
                .as_deref()
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(str::to_string);
        }
    }

    for dev in overrides.persistent_devices() {
        if dev.device_id != device_id {
            continue;
        }
        if let Some(token) = dev.sqm_override.as_deref().map(str::trim)
            && !token.is_empty()
        {
            return Some(token.to_string());
        }
    }

    None
}

/// Returns the set of device IDs carrying an SQM override in an overrides file.
///
/// This function is pure: it has no side effects.
fn overrides_sqm_device_ids(overrides: &OverrideFile) -> FxHashSet<String> {
    let mut out = FxHashSet::default();
    for adj in overrides.circuit_adjustments() {
        if let lqos_overrides::CircuitAdjustment::DeviceAdjustSqm {
            device_id,
            sqm_override,
        } = adj
            && sqm_override
                .as_deref()
                .map(str::trim)
                .is_some_and(|token| !token.is_empty())
        {
            out.insert(device_id.clone());
        }
    }
    for dev in overrides.persistent_devices() {
        if dev
            .sqm_override
            .as_deref()
            .map(str::trim)
            .is_some_and(|token| !token.is_empty())
        {
            out.insert(dev.device_id.clone());
        }
    }
    out
}

/// Infers an SQM override token for a circuit, preferring persisted override entries.
///
/// This function is pure: it has no side effects.
fn infer_circuit_sqm_override_token(
    circuit_id: &str,
    shaped_devices: &[lqos_config::ShapedDevice],
    overrides: Option<&OverrideFile>,
) -> Option<String> {
    let circuit_device_ids: FxHashSet<&str> = shaped_devices
        .iter()
        .filter(|d| d.circuit_id == circuit_id)
        .map(|d| d.device_id.as_str())
        .collect();

    if let Some(overrides) = overrides {
        for device_id in &circuit_device_ids {
            if let Some(token) = overrides_device_sqm(overrides, device_id) {
                return Some(token);
            }
        }
    }

    for dev in shaped_devices.iter().filter(|d| d.circuit_id == circuit_id) {
        if let Some(token) = dev.sqm_override.as_deref() {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    None
}

/// Appends an entry to the activity ring buffer.
///
/// This function is not pure: it mutates `activity`.
fn push_activity(activity: &mut VecDeque<TreeguardActivityEntry>, entry: TreeguardActivityEntry) {
    if entry.action.contains("failed") {
        warn!(
            "TreeGuard activity: entity_type={} entity_id={} action={} persisted={} reason={}",
            entry.entity_type, entry.entity_id, entry.action, entry.persisted, entry.reason
        );
    } else {
        info!(
            "TreeGuard activity: entity_type={} entity_id={} action={} persisted={} reason={}",
            entry.entity_type, entry.entity_id, entry.action, entry.persisted, entry.reason
        );
    }
    if activity.len() >= ACTIVITY_RING_CAPACITY {
        activity.pop_front();
    }
    activity.push_back(entry);
}

struct CircuitSqmApplyContext<'a> {
    status: &'a mut TreeguardStatusData,
    activity: &'a mut VecDeque<TreeguardActivityEntry>,
    now_unix: u64,
    dry_run: bool,
    persist_sqm_overrides: bool,
    circuit_id: &'a str,
    circuit_entity_id: &'a str,
    circuit_label: &'a str,
    devices: &'a [lqos_config::ShapedDevice],
}

struct CircuitSqmTransition {
    proposed_down: CircuitSqmState,
    proposed_up: CircuitSqmState,
    changed_down: bool,
    changed_up: bool,
}

struct CircuitTickContext<'a> {
    status: &'a mut TreeguardStatusData,
    activity: &'a mut VecDeque<TreeguardActivityEntry>,
    managed_device_ids: &'a mut FxHashSet<String>,
    now_unix: u64,
    now_nanos_since_boot: Option<u64>,
    cpu_max_pct: Option<u8>,
    circuit_id: &'a str,
    circuit_entity_id: &'a str,
    circuit_label: &'a str,
    devices: &'a [lqos_config::ShapedDevice],
    allowlisted: bool,
    cap_down: f32,
    cap_up: f32,
    bps: DownUpOrder<u64>,
    last_rtt_seen_nanos: Option<u64>,
    qoo: DownUpOrder<Option<f32>>,
    cpu_cfg: &'a lqos_config::TreeguardCpuConfig,
    circuits_cfg: &'a lqos_config::TreeguardCircuitsConfig,
    qoo_cfg: &'a lqos_config::TreeguardQooConfig,
}

fn apply_circuit_sqm_change<P, L>(
    ctx: CircuitSqmApplyContext<'_>,
    state: &mut CircuitState,
    transition: CircuitSqmTransition,
    mut persist_override: P,
    mut live_apply: L,
) where
    P: FnMut(&[String], &str) -> Result<bool, TreeguardError>,
    L: FnMut(&str, &[lqos_config::ShapedDevice], &str) -> Result<(), TreeguardError>,
{
    let CircuitSqmApplyContext {
        status,
        activity,
        now_unix,
        dry_run,
        persist_sqm_overrides,
        circuit_id,
        circuit_entity_id,
        circuit_label,
        devices,
    } = ctx;
    let CircuitSqmTransition {
        proposed_down,
        proposed_up,
        changed_down,
        changed_up,
    } = transition;

    let token = decisions::format_directional_sqm_override(proposed_down, proposed_up);

    if dry_run {
        if changed_down {
            state.down.desired = proposed_down;
            state.down.last_change_unix = Some(now_unix);
            state.down.recent_changes_unix.push_back(now_unix);
            prune_recent_changes(&mut state.down.recent_changes_unix, now_unix);
        }
        if changed_up {
            state.up.desired = proposed_up;
            state.up.last_change_unix = Some(now_unix);
            state.up.recent_changes_unix.push_back(now_unix);
            prune_recent_changes(&mut state.up.recent_changes_unix, now_unix);
        }

        push_activity(
            activity,
            TreeguardActivityEntry {
                time: now_unix.to_string(),
                entity_type: "circuit".to_string(),
                entity_id: circuit_entity_id.to_string(),
                action: format!("would_set_sqm_override:{token}"),
                persisted: false,
                reason: "Dry-run".to_string(),
            },
        );
        status.last_action_summary = Some(format!(
            "Would set SQM override for circuit '{}' -> {}",
            circuit_label, token
        ));
        return;
    }

    let mut persisted_ok = false;
    let mut can_apply_live = true;
    if persist_sqm_overrides {
        let device_ids: Vec<String> = devices
            .iter()
            .map(|device| device.device_id.clone())
            .collect();
        match persist_override(&device_ids, &token) {
            Ok(_) => {
                persisted_ok = true;
            }
            Err(e) => {
                can_apply_live = false;
                status.warnings.push(format!(
                    "TreeGuard circuits: failed to persist SQM overrides for circuit '{circuit_id}': {e}"
                ));
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "circuit".to_string(),
                        entity_id: circuit_entity_id.to_string(),
                        action: "set_sqm_override_failed".to_string(),
                        persisted: false,
                        reason: format!("Overrides write failed: {e}"),
                    },
                );
            }
        }
    }

    let live_ok = if can_apply_live {
        match live_apply(circuit_id, devices, &token) {
            Ok(()) => true,
            Err(e) => {
                status.warnings.push(format!(
                    "TreeGuard circuits: live SQM apply failed for circuit '{circuit_id}': {e}"
                ));
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "circuit".to_string(),
                        entity_id: circuit_entity_id.to_string(),
                        action: format!("apply_sqm_live_failed:{token}"),
                        persisted: persisted_ok,
                        reason: format!("Bakery live apply failed: {e}"),
                    },
                );
                false
            }
        }
    } else {
        false
    };

    if live_ok || persisted_ok {
        if changed_down {
            state.down.desired = proposed_down;
            state.down.last_change_unix = Some(now_unix);
            state.down.recent_changes_unix.push_back(now_unix);
            prune_recent_changes(&mut state.down.recent_changes_unix, now_unix);
        }
        if changed_up {
            state.up.desired = proposed_up;
            state.up.last_change_unix = Some(now_unix);
            state.up.recent_changes_unix.push_back(now_unix);
            prune_recent_changes(&mut state.up.recent_changes_unix, now_unix);
        }

        let (action, reason) = match (persisted_ok, live_ok) {
            (true, true) => (
                "set_sqm_override".to_string(),
                "Applied live + persisted".to_string(),
            ),
            (true, false) => (
                "set_sqm_override".to_string(),
                "Persisted (live apply failed)".to_string(),
            ),
            (false, true) => ("set_sqm_live".to_string(), "Applied live".to_string()),
            (false, false) => ("set_sqm_live".to_string(), "Not applied".to_string()),
        };
        push_activity(
            activity,
            TreeguardActivityEntry {
                time: now_unix.to_string(),
                entity_type: "circuit".to_string(),
                entity_id: circuit_entity_id.to_string(),
                action: format!("{action}:{token}"),
                persisted: persisted_ok,
                reason,
            },
        );

        status.last_action_summary = Some(format!(
            "SQM override for circuit '{}' -> {}",
            circuit_label, token
        ));
    }
}

fn process_circuit_tick<P, L>(
    ctx: CircuitTickContext<'_>,
    state: &mut CircuitState,
    persist_override: P,
    live_apply: L,
) -> bool
where
    P: FnMut(&[String], &str) -> Result<bool, TreeguardError>,
    L: FnMut(&str, &[lqos_config::ShapedDevice], &str) -> Result<(), TreeguardError>,
{
    let CircuitTickContext {
        status,
        activity,
        managed_device_ids,
        now_unix,
        now_nanos_since_boot,
        cpu_max_pct,
        circuit_id,
        circuit_entity_id,
        circuit_label,
        devices,
        allowlisted,
        cap_down,
        cap_up,
        bps,
        last_rtt_seen_nanos,
        qoo,
        cpu_cfg,
        circuits_cfg,
        qoo_cfg,
    } = ctx;

    prune_recent_changes(&mut state.down.recent_changes_unix, now_unix);
    prune_recent_changes(&mut state.up.recent_changes_unix, now_unix);

    let capacity_known = cap_down > 0.0 && cap_up > 0.0;
    if !capacity_known {
        status.warnings.push(format!(
            "TreeGuard circuits: circuit '{circuit_id}' has unknown capacity; no changes will be made."
        ));
        state.down.idle_since_unix = None;
        state.up.idle_since_unix = None;
    } else {
        let mbps_down = (bps.down as f64 * 8.0) / 1_000_000.0;
        let mbps_up = (bps.up as f64 * 8.0) / 1_000_000.0;
        let util_down_pct = (mbps_down / cap_down as f64) * 100.0;
        let util_up_pct = (mbps_up / cap_up as f64) * 100.0;

        let ewma_down = state
            .down
            .util_ewma_pct
            .update(util_down_pct, UTIL_EWMA_ALPHA);
        let ewma_up = state.up.util_ewma_pct.update(util_up_pct, UTIL_EWMA_ALPHA);

        update_idle_since(
            &mut state.down.idle_since_unix,
            now_unix,
            ewma_down,
            circuits_cfg.idle_util_pct as f64,
        );
        update_idle_since(
            &mut state.up.idle_since_unix,
            now_unix,
            ewma_up,
            circuits_cfg.idle_util_pct as f64,
        );
    }

    let rtt_missing = match (now_nanos_since_boot, last_rtt_seen_nanos) {
        (Some(now_nanos), Some(last_seen)) if last_seen > 0 => {
            now_nanos.saturating_sub(last_seen)
                >= u64::from(circuits_cfg.rtt_missing_seconds).saturating_mul(1_000_000_000)
        }
        _ => true,
    };

    let mut proposed_down = state.down.desired;
    let mut proposed_up = state.up.desired;
    let decision = decisions::decide_circuit_sqm(decisions::CircuitSqmInput {
        now_unix,
        allowlisted: allowlisted && capacity_known,
        cpu_max_pct,
        cpu_cfg,
        circuits_cfg,
        qoo_cfg,
        rtt_missing,
        qoo,
        state,
    });
    if let Some(down) = decision.down {
        proposed_down = down;
    }
    if let Some(up) = decision.up {
        proposed_up = up;
    }

    let changed_down = proposed_down != state.down.desired;
    let changed_up = proposed_up != state.up.desired;

    if devices.is_empty() {
        status.warnings.push(format!(
            "TreeGuard circuits: circuit '{circuit_id}' has no devices in ShapedDevices.csv."
        ));
    } else {
        for dev in devices {
            managed_device_ids.insert(dev.device_id.clone());
        }
    }

    if (changed_down || changed_up) && !devices.is_empty() {
        apply_circuit_sqm_change(
            CircuitSqmApplyContext {
                status,
                activity,
                now_unix,
                dry_run: false,
                persist_sqm_overrides: circuits_cfg.persist_sqm_overrides,
                circuit_id,
                circuit_entity_id,
                circuit_label,
                devices,
            },
            state,
            CircuitSqmTransition {
                proposed_down,
                proposed_up,
                changed_down,
                changed_up,
            },
            persist_override,
            live_apply,
        );
    }

    state.down.desired == CircuitSqmState::FqCodel || state.up.desired == CircuitSqmState::FqCodel
}

/// Removes entries older than one hour from a recent-changes ring buffer.
///
/// This function is not pure: it mutates `recent_changes`.
fn prune_recent_changes(recent_changes: &mut VecDeque<u64>, now_unix: u64) {
    while recent_changes
        .front()
        .is_some_and(|t| now_unix.saturating_sub(*t) > 3600)
    {
        recent_changes.pop_front();
    }
}

/// Updates an "idle since" timestamp based on utilization and an idle threshold.
///
/// This function is not pure: it mutates `idle_since`.
fn update_idle_since(idle_since: &mut Option<u64>, now_unix: u64, util_pct: f64, idle_pct: f64) {
    if util_pct < idle_pct {
        if idle_since.is_none() {
            *idle_since = Some(now_unix);
        }
    } else {
        *idle_since = None;
    }
}

/// Updates a "below threshold since" timestamp based on utilization and a threshold.
///
/// This function is not pure: it mutates `below_since`.
fn update_below_since(
    below_since: &mut Option<u64>,
    now_unix: u64,
    util_pct: f64,
    threshold_pct: f64,
) {
    if util_pct < threshold_pct {
        if below_since.is_none() {
            *below_since = Some(now_unix);
        }
    } else {
        *below_since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CircuitSqmApplyContext, CircuitSqmTransition, CircuitTickContext, TreeguardRuntimeState,
        apply_circuit_sqm_change, empty_status_snapshot, pause_for_bakery_reload_with_flag,
        process_circuit_tick, run_tick,
    };
    use crate::node_manager::ws::messages::TreeguardActivityEntry;
    use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};
    use crate::system_stats::SystemStats;
    use crate::throughput_tracker::CIRCUIT_RTT_BUFFERS;
    use crate::treeguard::{bakery, state::CircuitSqmState};
    use crossbeam_channel::bounded;
    use fxhash::{FxHashMap, FxHashSet};
    use lqos_bakery::BakeryCommands;
    use lqos_bus::TcHandle;
    use lqos_config::ConfigShapedDevices;
    use lqos_config::ShapedDevice;
    use lqos_queue_tracker::{QUEUE_STRUCTURE, QueueNode, QueueStructure};
    use lqos_utils::rtt::RttBuffer;
    use lqos_utils::units::DownUpOrder;
    use std::collections::VecDeque;
    use std::net::Ipv4Addr;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Once;

    #[test]
    fn bakery_reload_pause_updates_status_and_tick_backoff() {
        let mut status = empty_status_snapshot();
        let mut tick_seconds = 1;
        let mut runtime_state = TreeguardRuntimeState::default();

        let paused = pause_for_bakery_reload_with_flag(
            &mut status,
            &mut tick_seconds,
            &mut runtime_state,
            true,
            false,
            true,
        );

        assert!(paused);
        assert_eq!(tick_seconds, 5);
        assert_eq!(status.enabled, true);
        assert_eq!(status.dry_run, false);
        assert_eq!(
            status.last_action_summary.as_deref(),
            Some("Paused while Bakery full reload is in progress")
        );
        assert!(
            status
                .warnings
                .iter()
                .any(|warning| warning.contains("TreeGuard paused while Bakery full reload"))
        );
        assert!(runtime_state.paused_for_bakery_reload);

        let resumed = pause_for_bakery_reload_with_flag(
            &mut status,
            &mut tick_seconds,
            &mut runtime_state,
            true,
            false,
            false,
        );
        assert!(!resumed);
        assert!(!runtime_state.paused_for_bakery_reload);
    }

    #[test]
    fn actor_live_sqm_change_updates_state_activity_and_bakery_command() {
        let (tx, rx) = bounded(1);
        let queues = vec![QueueNode {
            circuit_id: Some("circuit-1".to_string()),
            class_minor: 0x14af,
            class_major: 0x0003,
            up_class_major: 0x0043,
            parent_class_id: TcHandle::from_string("3:20").expect("valid down parent"),
            up_parent_class_id: TcHandle::from_string("43:20").expect("valid up parent"),
            download_bandwidth_mbps_min: 50,
            upload_bandwidth_mbps_min: 10,
            download_bandwidth_mbps: 200,
            upload_bandwidth_mbps: 50,
            ..QueueNode::default()
        }];
        let devices = vec![ShapedDevice {
            circuit_id: "circuit-1".to_string(),
            circuit_name: "Circuit One".to_string(),
            device_id: "device-1".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 0, 2, 10), 32)],
            ..ShapedDevice::default()
        }];
        let mut status = empty_status_snapshot();
        let mut activity: VecDeque<TreeguardActivityEntry> = VecDeque::new();
        let mut state = crate::treeguard::state::CircuitState::default();

        apply_circuit_sqm_change(
            CircuitSqmApplyContext {
                status: &mut status,
                activity: &mut activity,
                now_unix: 1_000,
                dry_run: false,
                persist_sqm_overrides: false,
                circuit_id: "circuit-1",
                circuit_entity_id: "Circuit One (circuit-1)",
                circuit_label: "Circuit One",
                devices: &devices,
            },
            &mut state,
            CircuitSqmTransition {
                proposed_down: CircuitSqmState::FqCodel,
                proposed_up: CircuitSqmState::Cake,
                changed_down: true,
                changed_up: false,
            },
            |_device_ids, _token| Ok(false),
            |circuit_id, devices, token| {
                bakery::apply_circuit_sqm_override_live_with_sender_and_snapshot(
                    circuit_id, devices, token, &tx, &queues,
                )
            },
        );

        assert_eq!(state.down.desired, CircuitSqmState::FqCodel);
        assert_eq!(state.up.desired, CircuitSqmState::Cake);
        assert_eq!(state.down.last_change_unix, Some(1_000));
        assert_eq!(state.up.last_change_unix, None);
        assert_eq!(state.down.recent_changes_unix.len(), 1);
        assert!(state.up.recent_changes_unix.is_empty());
        assert_eq!(
            status.last_action_summary.as_deref(),
            Some("SQM override for circuit 'Circuit One' -> fq_codel/cake")
        );

        let last_activity = activity.back().expect("activity should be recorded");
        assert_eq!(last_activity.entity_type, "circuit");
        assert_eq!(last_activity.entity_id, "Circuit One (circuit-1)");
        assert_eq!(last_activity.action, "set_sqm_live:fq_codel/cake");
        assert!(!last_activity.persisted);
        assert_eq!(last_activity.reason, "Applied live");

        let command = rx.try_recv().expect("bakery command should be sent");
        let BakeryCommands::AddCircuit {
            circuit_hash,
            sqm_override,
            down_qdisc_handle,
            up_qdisc_handle,
            ..
        } = command
        else {
            panic!("expected AddCircuit");
        };
        assert_eq!(circuit_hash, lqos_utils::hash_to_i64("circuit-1"));
        assert_eq!(sqm_override, Some("fq_codel/cake".to_string()));
        assert_eq!(down_qdisc_handle, None);
        assert_eq!(up_qdisc_handle, None);
    }

    #[test]
    fn circuit_tick_snapshot_decides_and_applies_live_override() {
        let (tx, rx) = bounded(1);
        let queues = vec![QueueNode {
            circuit_id: Some("circuit-2".to_string()),
            class_minor: 0x22af,
            class_major: 0x0005,
            up_class_major: 0x0045,
            parent_class_id: TcHandle::from_string("5:20").expect("valid down parent"),
            up_parent_class_id: TcHandle::from_string("45:20").expect("valid up parent"),
            download_bandwidth_mbps_min: 25,
            upload_bandwidth_mbps_min: 5,
            download_bandwidth_mbps: 100,
            upload_bandwidth_mbps: 20,
            ..QueueNode::default()
        }];
        let devices = vec![ShapedDevice {
            circuit_id: "circuit-2".to_string(),
            circuit_name: "Circuit Two".to_string(),
            device_id: "device-2".to_string(),
            ipv4: vec![(Ipv4Addr::new(198, 51, 100, 20), 32)],
            ..ShapedDevice::default()
        }];
        let mut managed_device_ids = FxHashSet::default();
        let mut status = empty_status_snapshot();
        let mut activity: VecDeque<TreeguardActivityEntry> = VecDeque::new();
        let mut state = crate::treeguard::state::CircuitState::default();
        state.down.idle_since_unix = Some(0);
        state.up.idle_since_unix = Some(0);

        let mut circuits_cfg = lqos_config::TreeguardCircuitsConfig::default();
        circuits_cfg.persist_sqm_overrides = false;

        let fq_codel = process_circuit_tick(
            CircuitTickContext {
                status: &mut status,
                activity: &mut activity,
                managed_device_ids: &mut managed_device_ids,
                now_unix: 1_000,
                now_nanos_since_boot: Some(2_000_000_000),
                cpu_max_pct: Some(95),
                circuit_id: "circuit-2",
                circuit_entity_id: "Circuit Two (circuit-2)",
                circuit_label: "Circuit Two",
                devices: &devices,
                allowlisted: true,
                cap_down: 100.0,
                cap_up: 20.0,
                bps: DownUpOrder { down: 0, up: 0 },
                last_rtt_seen_nanos: Some(1_900_000_000),
                qoo: DownUpOrder {
                    down: Some(90.0),
                    up: Some(90.0),
                },
                cpu_cfg: &lqos_config::TreeguardCpuConfig::default(),
                circuits_cfg: &circuits_cfg,
                qoo_cfg: &lqos_config::TreeguardQooConfig::default(),
            },
            &mut state,
            |_device_ids, _token| Ok(false),
            |circuit_id, devices, token| {
                bakery::apply_circuit_sqm_override_live_with_sender_and_snapshot(
                    circuit_id, devices, token, &tx, &queues,
                )
            },
        );

        assert!(fq_codel);
        assert_eq!(state.down.desired, CircuitSqmState::FqCodel);
        assert_eq!(state.up.desired, CircuitSqmState::FqCodel);
        assert!(managed_device_ids.contains("device-2"));
        assert_eq!(
            status.last_action_summary.as_deref(),
            Some("SQM override for circuit 'Circuit Two' -> fq_codel/fq_codel")
        );

        let command = rx.try_recv().expect("bakery command should be sent");
        let BakeryCommands::AddCircuit {
            circuit_hash,
            sqm_override,
            ..
        } = command
        else {
            panic!("expected AddCircuit");
        };
        assert_eq!(circuit_hash, lqos_utils::hash_to_i64("circuit-2"));
        assert_eq!(sqm_override, Some("fq_codel/fq_codel".to_string()));
    }

    fn test_runtime_dir(name: &str) -> PathBuf {
        let unique = format!(
            "{}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("thread"),
            name
        )
        .replace(['/', ' '], "-");
        let dir = std::env::temp_dir().join(format!("libreqos-treeguard-{unique}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp runtime dir should be created");
        dir
    }

    fn write_test_config(runtime_dir: &std::path::Path) -> PathBuf {
        let config_path = runtime_dir.join("lqos.test.toml");
        let raw = include_str!("../../../lqos_config/src/etc/v15/example.toml")
            .replace(
                "lqos_directory = \"/opt/libreqos/src\"",
                &format!("lqos_directory = \"{}\"", runtime_dir.display()),
            )
            .replace("use_xdp_bridge = true", "use_xdp_bridge = false")
            .replace(
                "[treeguard.links]\nenabled = true",
                "[treeguard.links]\nenabled = false",
            )
            .replace(
                "persist_sqm_overrides = true",
                "persist_sqm_overrides = false",
            );
        std::fs::write(&config_path, raw).expect("test config should be written");
        config_path
    }

    fn install_test_bakery_sender() -> crossbeam_channel::Receiver<BakeryCommands> {
        static INIT: Once = Once::new();
        let (tx, rx) = bounded(8);
        INIT.call_once(|| {
            let _ = lqos_bakery::BAKERY_SENDER.set(tx);
        });
        rx
    }

    #[test]
    fn run_tick_end_to_end_switches_circuit_and_emits_bakery_update() {
        let runtime_dir = test_runtime_dir("run-tick");
        let config_path = write_test_config(&runtime_dir);
        let old_lqos_config = std::env::var_os("LQOS_CONFIG");
        let old_lqos_directory = std::env::var_os("LQOS_DIRECTORY");
        unsafe {
            std::env::set_var("LQOS_CONFIG", &config_path);
            std::env::set_var("LQOS_DIRECTORY", &runtime_dir);
        }
        lqos_config::clear_cached_config();

        let devices = vec![ShapedDevice {
            circuit_id: "circuit-3".to_string(),
            circuit_name: "Circuit Three".to_string(),
            device_id: "device-3".to_string(),
            device_name: "Device Three".to_string(),
            parent_node: "Node Three".to_string(),
            ipv4: vec![(Ipv4Addr::new(203, 0, 113, 30), 32)],
            download_min_mbps: 25.0,
            upload_min_mbps: 5.0,
            download_max_mbps: 100.0,
            upload_max_mbps: 20.0,
            circuit_hash: lqos_utils::hash_to_i64("circuit-3"),
            device_hash: lqos_utils::hash_to_i64("device-3"),
            parent_hash: lqos_utils::hash_to_i64("Node Three"),
            ..ShapedDevice::default()
        }];
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(devices.clone());
        let old_shaped = SHAPED_DEVICES.load_full();
        SHAPED_DEVICES.store(Arc::new(shaped));

        let old_network = NETWORK_JSON.read().nodes.clone();
        NETWORK_JSON.write().nodes = Vec::new();

        let old_queue_structure = QUEUE_STRUCTURE.load_full();
        QUEUE_STRUCTURE.store(Arc::new(QueueStructure {
            maybe_queues: Some(vec![QueueNode {
                circuit_id: Some("circuit-3".to_string()),
                class_minor: 0x33af,
                class_major: 0x0007,
                up_class_major: 0x0047,
                parent_class_id: TcHandle::from_string("7:20").expect("valid down parent"),
                up_parent_class_id: TcHandle::from_string("47:20").expect("valid up parent"),
                download_bandwidth_mbps_min: 25,
                upload_bandwidth_mbps_min: 5,
                download_bandwidth_mbps: 100,
                upload_bandwidth_mbps: 20,
                ..QueueNode::default()
            }]),
        }));

        let now_nanos = lqos_utils::unix_time::time_since_boot()
            .map(std::time::Duration::from)
            .map(|duration| duration.as_nanos() as u64)
            .expect("time since boot should be available");
        let old_rtt = CIRCUIT_RTT_BUFFERS.load_full();
        let mut rtt = RttBuffer::default();
        rtt.last_seen = now_nanos.saturating_sub(100_000_000);
        let mut rtt_map = FxHashMap::default();
        rtt_map.insert(lqos_utils::hash_to_i64("circuit-3"), rtt);
        CIRCUIT_RTT_BUFFERS.store(Arc::new(rtt_map));

        let rx = install_test_bakery_sender();
        while rx.try_recv().is_ok() {}

        let (system_tx, system_rx) = bounded::<tokio::sync::oneshot::Sender<SystemStats>>(1);
        let responder = std::thread::spawn(move || {
            let reply = system_rx
                .recv()
                .expect("system stats request should arrive");
            let _ = reply.send(SystemStats {
                cpu_usage: vec![95],
                ram_used: 0,
                total_ram: 0,
            });
        });

        let mut status = empty_status_snapshot();
        let mut activity: VecDeque<TreeguardActivityEntry> = VecDeque::new();
        let mut runtime_state = TreeguardRuntimeState::default();
        runtime_state.circuit_states.insert(
            "circuit-3".to_string(),
            crate::treeguard::state::CircuitState {
                down: crate::treeguard::state::CircuitDirectionState {
                    idle_since_unix: Some(0),
                    ..Default::default()
                },
                up: crate::treeguard::state::CircuitDirectionState {
                    idle_since_unix: Some(0),
                    ..Default::default()
                },
            },
        );
        let mut tick_seconds = 1;
        run_tick(
            &mut status,
            &mut activity,
            &system_tx,
            &mut tick_seconds,
            &mut runtime_state,
        );
        responder
            .join()
            .expect("system stats responder should join");

        assert_eq!(status.enabled, true);
        assert_eq!(status.dry_run, false);
        assert_eq!(status.managed_circuits, 1);
        assert_eq!(status.fq_codel_circuits, 1);
        assert_eq!(
            status.last_action_summary.as_deref(),
            Some("SQM override for circuit 'Circuit Three' -> fq_codel/fq_codel")
        );
        let circuit_state = runtime_state
            .circuit_states
            .get("circuit-3")
            .expect("circuit state should exist");
        assert_eq!(circuit_state.down.desired, CircuitSqmState::FqCodel);
        assert_eq!(circuit_state.up.desired, CircuitSqmState::FqCodel);
        assert!(
            activity
                .iter()
                .any(|entry| entry.action == "set_sqm_live:fq_codel/fq_codel")
        );

        let command = rx
            .try_recv()
            .expect("run_tick should send a bakery command");
        let BakeryCommands::AddCircuit {
            circuit_hash,
            sqm_override,
            ..
        } = command
        else {
            panic!("expected AddCircuit");
        };
        assert_eq!(circuit_hash, lqos_utils::hash_to_i64("circuit-3"));
        assert_eq!(sqm_override, Some("fq_codel/fq_codel".to_string()));

        SHAPED_DEVICES.store(old_shaped);
        NETWORK_JSON.write().nodes = old_network;
        QUEUE_STRUCTURE.store(old_queue_structure);
        CIRCUIT_RTT_BUFFERS.store(old_rtt);
        match old_lqos_config {
            Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
            None => unsafe { std::env::remove_var("LQOS_CONFIG") },
        }
        match old_lqos_directory {
            Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
            None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
        }
        lqos_config::clear_cached_config();
    }
}

/// Returns the minimum of two optional floats, treating `None` as "unknown".
///
/// This function is pure: it has no side effects.
fn min_opt_f32(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

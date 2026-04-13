//! TreeGuard actor loop.
//!
//! The actor is responsible for sampling telemetry, maintaining state machines,
//! and applying (or dry-running) any decisions.

use crate::node_manager::ws::messages::{TreeguardActivityEntry, TreeguardStatusData};
use crate::shaped_devices_tracker::circuit_live::fresh_circuit_live_snapshot;
use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};
use crate::system_stats::SystemStats;
use crate::throughput_tracker::CIRCUIT_RTT_BUFFERS;
use crate::treeguard::TreeguardError;
use crate::treeguard::state::{
    CircuitSqmState, CircuitState, LinkState, LinkStructuralIneligibleState,
    LinkTopologyFingerprint, LinkVirtualState, is_sustained_idle, is_sustained_window,
};
use crate::treeguard::{bakery, decisions, overrides};
use crossbeam_channel::{Receiver, Sender};
use fxhash::{FxHashMap, FxHashSet};
use lqos_bakery::{BakeryRuntimeNodeOperationFailureReason, BakeryRuntimeNodeOperationStatus};
use lqos_config::{NetworkJsonNode, ShapedDevice, load_config};
use lqos_overrides::{NetworkAdjustment, OverrideFile, OverrideLayer, OverrideStore};
use lqos_utils::hash_to_i64;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::{time_since_boot, unix_now};
use parking_lot::RwLock;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

static TREEGUARD_SENDER: OnceLock<Sender<TreeguardCommand>> = OnceLock::new();
static TREEGUARD_STATUS_CACHE: OnceLock<RwLock<TreeguardStatusData>> = OnceLock::new();
static TREEGUARD_ACTIVITY_CACHE: OnceLock<RwLock<Vec<TreeguardActivityEntry>>> = OnceLock::new();
static TREEGUARD_RUNTIME_VIRTUALIZED_NODES: OnceLock<RwLock<FxHashSet<String>>> = OnceLock::new();

const ACTIVITY_RING_CAPACITY: usize = 200;
const UTIL_EWMA_ALPHA: f64 = 0.1;
const TOP_LEVEL_SAFE_SUSTAIN_MINUTES: u32 = 15;
const TOP_LEVEL_EMERGENCY_UTIL_PCT: f64 = 95.0;
const TOP_LEVEL_EMERGENCY_SUSTAIN_SECONDS: u64 = 5;
const TREEGUARD_LINK_CHANGE_BUDGET_PER_TICK: usize = 4;
const TREEGUARD_CIRCUIT_CHANGE_BUDGET_PER_TICK: usize = 512;
const TREEGUARD_CIRCUIT_TARGET_SWEEP_SECONDS: usize = 15;
const TREEGUARD_CIRCUIT_MIN_BATCH_SIZE: usize = 128;
const TREEGUARD_LINK_VIRTUALIZATION_BACKOFF_SECONDS: u64 = 15 * 60;
const TREEGUARD_LINK_VIRTUALIZATION_DEFERRED_BACKOFF_SECONDS: u64 = 60;
const TREEGUARD_MIN_AUTO_VIRTUALIZE_SUBTREE_NODES: usize = 8;
const TREEGUARD_MIN_AUTO_VIRTUALIZE_SUBTREE_NODES_LOW_THROUGHPUT: usize = 16;
const TREEGUARD_LOW_VALUE_THROUGHPUT_MBPS: f64 = 1.0;
const TREEGUARD_MAX_AUTO_VIRTUALIZED_NODES: usize = 64;
const TREEGUARD_TOP_LEVEL_VIRTUALIZATION_VALUE_BONUS: u64 = 1_000_000_000;
const TREEGUARD_WARNING_DETAIL_LIMIT_PER_GROUP: usize = 3;

struct TreeguardWarningGroupState {
    emitted: usize,
    suppressed: usize,
    suppression_summary: String,
}

#[derive(Default)]
struct TreeguardWarningLimiter {
    groups: BTreeMap<String, TreeguardWarningGroupState>,
}

impl TreeguardWarningLimiter {
    fn push(
        &mut self,
        status: &mut TreeguardStatusData,
        group_key: impl Into<String>,
        warning: String,
        suppression_summary: String,
    ) {
        let state =
            self.groups
                .entry(group_key.into())
                .or_insert_with(|| TreeguardWarningGroupState {
                    emitted: 0,
                    suppressed: 0,
                    suppression_summary,
                });
        if state.emitted < TREEGUARD_WARNING_DETAIL_LIMIT_PER_GROUP {
            status.warnings.push(warning);
            state.emitted += 1;
        } else {
            state.suppressed += 1;
        }
    }

    fn flush(self, status: &mut TreeguardStatusData) {
        for state in self.groups.into_values() {
            if state.suppressed == 0 {
                continue;
            }
            status.warnings.push(format!(
                "Suppressed {} additional {} this tick.",
                state.suppressed, state.suppression_summary
            ));
        }
    }
}

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
    let _ = TREEGUARD_RUNTIME_VIRTUALIZED_NODES.set(RwLock::new(FxHashSet::default()));

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

pub(crate) fn is_runtime_virtualized_node(node_name: &str) -> bool {
    TREEGUARD_RUNTIME_VIRTUALIZED_NODES
        .get()
        .is_some_and(|cache| cache.read().contains(node_name))
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
                update_runtime_virtualized_cache(&runtime_state.runtime_virtualized_nodes);
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
        paused_for_bakery_reload: false,
        pause_reason: None,
        cpu_max_pct: None,
        total_nodes: 0,
        total_circuits: 0,
        managed_nodes: 0,
        managed_circuits: 0,
        virtualized_nodes: 0,
        cake_circuits: 0,
        mixed_sqm_circuits: 0,
        fq_codel_circuits: 0,
        last_action_summary: None,
        warnings: Vec::new(),
    }
}

fn current_topology_totals() -> (usize, usize) {
    let total_nodes = {
        let reader = NETWORK_JSON.read();
        reader
            .get_nodes_when_ready()
            .iter()
            .filter(|n| n.name != "Root")
            .count()
    };

    let total_circuits = {
        let shaped = SHAPED_DEVICES.load();
        let mut circuits: FxHashSet<&str> = FxHashSet::default();
        for d in shaped.devices.iter() {
            let id = d.circuit_id.trim();
            if !id.is_empty() {
                circuits.insert(id);
            }
        }
        circuits.len()
    };

    (total_nodes, total_circuits)
}

fn direct_child_site_counts_by_node(nodes: &[NetworkJsonNode]) -> Vec<usize> {
    let mut counts = vec![0usize; nodes.len()];
    for node in nodes {
        if let Some(parent) = node.immediate_parent
            && parent < counts.len()
        {
            counts[parent] = counts[parent].saturating_add(1);
        }
    }
    counts
}

fn direct_circuit_counts_by_node(shaped_devices: &[ShapedDevice]) -> FxHashMap<String, usize> {
    let mut circuits_by_node: FxHashMap<String, FxHashSet<&str>> = FxHashMap::default();
    for device in shaped_devices {
        let node_name = device.parent_node.trim();
        let circuit_id = device.circuit_id.trim();
        if node_name.is_empty() || circuit_id.is_empty() {
            continue;
        }
        circuits_by_node
            .entry(node_name.to_string())
            .or_default()
            .insert(circuit_id);
    }

    circuits_by_node
        .into_iter()
        .map(|(node_name, circuits)| (node_name, circuits.len()))
        .collect()
}

fn structural_failure_reason_label(
    reason: BakeryRuntimeNodeOperationFailureReason,
) -> &'static str {
    match reason {
        BakeryRuntimeNodeOperationFailureReason::StructuralIneligibleNoPromotableChildren => {
            "no promotable children"
        }
        BakeryRuntimeNodeOperationFailureReason::StructuralIneligibleSinglePromotableChild => {
            "single promotable child"
        }
        BakeryRuntimeNodeOperationFailureReason::StructuralIneligibleNestedRuntimeBranch => {
            "nested runtime branch"
        }
    }
}

fn top_level_structural_ineligibility(
    state: &LinkState,
    target: LinkVirtualState,
    is_top_level: bool,
) -> Option<(BakeryRuntimeNodeOperationFailureReason, String)> {
    if target != LinkVirtualState::Virtual || !is_top_level {
        return None;
    }

    let direct_promotable_children =
        state.topology_fingerprint.direct_child_sites + state.topology_fingerprint.direct_circuits;

    if direct_promotable_children == 0 {
        return Some((
            BakeryRuntimeNodeOperationFailureReason::StructuralIneligibleNoPromotableChildren,
            "has no direct child sites or direct circuits to promote safely".to_string(),
        ));
    }

    if direct_promotable_children == 1 {
        return Some((
            BakeryRuntimeNodeOperationFailureReason::StructuralIneligibleSinglePromotableChild,
            "has only one promotable direct child, so top-level runtime virtualization would not produce a deterministic v1 split point".to_string(),
        ));
    }

    None
}

fn clear_structural_ineligible_if_topology_changed(
    state: &mut LinkState,
    new_topology_fingerprint: LinkTopologyFingerprint,
) -> bool {
    if state
        .structural_ineligible
        .is_some_and(|latched| latched.topology_fingerprint != new_topology_fingerprint)
    {
        state.structural_ineligible = None;
        return true;
    }
    false
}

fn latched_structural_ineligible_reason(
    state: &LinkState,
    target: LinkVirtualState,
) -> Option<BakeryRuntimeNodeOperationFailureReason> {
    if target != LinkVirtualState::Virtual {
        return None;
    }

    state
        .structural_ineligible
        .filter(|latched| latched.topology_fingerprint == state.topology_fingerprint)
        .map(|latched| latched.reason)
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

fn update_runtime_virtualized_cache(runtime_virtualized_nodes: &FxHashSet<String>) {
    if let Some(cache) = TREEGUARD_RUNTIME_VIRTUALIZED_NODES.get() {
        *cache.write() = runtime_virtualized_nodes.clone();
    }
}

#[derive(Default)]
struct TreeguardRuntimeState {
    link_states: FxHashMap<String, LinkState>,
    circuit_states: FxHashMap<String, CircuitState>,
    circuit_inventory: CircuitInventory,
    circuit_batch_cursor: usize,
    next_sqm_batch_id: u64,
    runtime_virtualized_nodes: FxHashSet<String>,
    pending_link_operations: FxHashMap<String, PendingLinkOperation>,
    link_virtualization_backoff_until_unix: FxHashMap<String, u64>,
    managed_nodes: FxHashSet<String>,
    managed_device_ids: FxHashSet<String>,
    duplicate_device_conflict_circuits: FxHashSet<String>,
    last_dry_run: Option<bool>,
    paused_for_bakery_reload: bool,
}

#[derive(Clone, Debug)]
struct PendingLinkOperation {
    target: LinkVirtualState,
    reason: String,
}

#[derive(Clone, Debug, Default)]
struct CircuitInventoryEntry {
    circuit_hash: i64,
    circuit_entity_id: String,
    circuit_label: String,
    devices: Vec<lqos_config::ShapedDevice>,
    device_ids: Vec<String>,
    cap_down: f32,
    cap_up: f32,
    duplicate_details: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Debug, Default)]
struct CircuitInventory {
    shaped_devices_ptr: usize,
    circuit_ids: Vec<String>,
    entries: FxHashMap<String, CircuitInventoryEntry>,
    all_device_ids: FxHashSet<String>,
}

#[derive(Clone, Debug)]
struct PendingLinkVirtualizationDecision {
    node_name: String,
    node_index: usize,
    target: LinkVirtualState,
    reason: String,
    subtree_nodes: usize,
    current_subtree_throughput_mbps: f64,
    explicit_allowlist: bool,
    is_top_level: bool,
    value_score: u64,
}

fn build_subtree_node_counts(parent_by_index: &[Option<usize>]) -> Vec<usize> {
    fn dfs(index: usize, children: &[Vec<usize>], memo: &mut [usize]) -> usize {
        if memo[index] != 0 {
            return memo[index];
        }
        let total = 1usize
            + children[index]
                .iter()
                .map(|child| dfs(*child, children, memo))
                .sum::<usize>();
        memo[index] = total;
        total
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); parent_by_index.len()];
    for (index, parent) in parent_by_index.iter().enumerate() {
        if let Some(parent) = *parent
            && parent < children.len()
        {
            children[parent].push(index);
        }
    }

    let mut memo = vec![0usize; parent_by_index.len()];
    for index in 0..parent_by_index.len() {
        let _ = dfs(index, &children, &mut memo);
    }
    memo
}

fn link_virtualization_value_score(
    is_top_level: bool,
    subtree_nodes: usize,
    cap_down_mbps: f64,
    cap_up_mbps: f64,
) -> u64 {
    let top_level_bonus = if is_top_level {
        TREEGUARD_TOP_LEVEL_VIRTUALIZATION_VALUE_BONUS
    } else {
        0
    };
    let subtree_score = subtree_nodes as u64 * 1_000_000;
    let capacity_score = ((cap_down_mbps.max(0.0) + cap_up_mbps.max(0.0)) * 100.0) as u64;
    top_level_bonus
        .saturating_add(subtree_score)
        .saturating_add(capacity_score)
}

fn has_ancestor_in_set(
    node_index: usize,
    parent_by_index: &[Option<usize>],
    selected_ancestors: &FxHashSet<usize>,
) -> bool {
    let mut current = parent_by_index.get(node_index).copied().flatten();
    while let Some(parent) = current {
        if selected_ancestors.contains(&parent) {
            return true;
        }
        current = parent_by_index.get(parent).copied().flatten();
    }
    false
}

fn select_link_virtualization_candidates(
    mut candidates: Vec<PendingLinkVirtualizationDecision>,
    parent_by_index: &[Option<usize>],
    existing_virtualized_indices: &FxHashSet<usize>,
    current_virtualized_nodes: usize,
) -> (Vec<PendingLinkVirtualizationDecision>, usize, usize) {
    candidates.sort_by(|left, right| {
        let left_restore = matches!(left.target, LinkVirtualState::Physical);
        let right_restore = matches!(right.target, LinkVirtualState::Physical);
        right_restore
            .cmp(&left_restore)
            .then_with(|| right.value_score.cmp(&left.value_score))
            .then_with(|| right.subtree_nodes.cmp(&left.subtree_nodes))
            .then_with(|| left.node_name.cmp(&right.node_name))
    });

    let mut selected = Vec::new();
    let mut deferred = 0usize;
    let mut skipped_low_value = 0usize;
    let mut selected_virtualized_indices: FxHashSet<usize> = FxHashSet::default();
    let mut virtualized_nodes_total = current_virtualized_nodes;

    for candidate in candidates {
        if selected.len() >= TREEGUARD_LINK_CHANGE_BUDGET_PER_TICK {
            deferred += 1;
            continue;
        }

        if matches!(candidate.target, LinkVirtualState::Virtual) {
            if has_ancestor_in_set(
                candidate.node_index,
                parent_by_index,
                existing_virtualized_indices,
            ) || has_ancestor_in_set(
                candidate.node_index,
                parent_by_index,
                &selected_virtualized_indices,
            ) {
                deferred += 1;
                continue;
            }

            if !candidate.explicit_allowlist {
                let required_subtree_nodes = if candidate.current_subtree_throughput_mbps
                    < TREEGUARD_LOW_VALUE_THROUGHPUT_MBPS
                {
                    TREEGUARD_MIN_AUTO_VIRTUALIZE_SUBTREE_NODES_LOW_THROUGHPUT
                } else {
                    TREEGUARD_MIN_AUTO_VIRTUALIZE_SUBTREE_NODES
                };
                if !candidate.is_top_level && candidate.subtree_nodes < required_subtree_nodes {
                    skipped_low_value += 1;
                    continue;
                }
                if virtualized_nodes_total >= TREEGUARD_MAX_AUTO_VIRTUALIZED_NODES {
                    deferred += 1;
                    continue;
                }
            }

            selected_virtualized_indices.insert(candidate.node_index);
            virtualized_nodes_total = virtualized_nodes_total.saturating_add(1);
        }

        selected.push(candidate);
    }

    (selected, deferred, skipped_low_value)
}

fn ensure_circuit_inventory(
    runtime_state: &mut TreeguardRuntimeState,
    shaped: &Arc<lqos_config::ConfigShapedDevices>,
) {
    let shaped_devices_ptr = Arc::as_ptr(shaped) as usize;
    if runtime_state.circuit_inventory.shaped_devices_ptr == shaped_devices_ptr {
        return;
    }

    runtime_state.circuit_inventory = build_circuit_inventory(shaped.as_ref());
    runtime_state.circuit_batch_cursor = 0;
}

fn build_circuit_inventory(shaped: &lqos_config::ConfigShapedDevices) -> CircuitInventory {
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

    let mut by_circuit_id: FxHashMap<String, Vec<lqos_config::ShapedDevice>> = FxHashMap::default();
    by_circuit_id.reserve(shaped.devices.len());
    let mut all_device_ids = FxHashSet::default();
    all_device_ids.reserve(shaped.devices.len());
    for device in shaped.devices.iter() {
        if device.circuit_id.trim().is_empty() {
            continue;
        }
        by_circuit_id
            .entry(device.circuit_id.clone())
            .or_default()
            .push(device.clone());
        if !device.device_id.trim().is_empty() {
            all_device_ids.insert(device.device_id.clone());
        }
    }

    let mut circuit_ids: Vec<String> = by_circuit_id.keys().cloned().collect();
    circuit_ids.sort();

    let mut entries = FxHashMap::default();
    entries.reserve(circuit_ids.len());
    for circuit_id in circuit_ids.iter() {
        let Some(mut devices) = by_circuit_id.remove(circuit_id) else {
            continue;
        };

        devices.sort_by(|left, right| left.device_id.cmp(&right.device_id));

        let circuit_name = devices.iter().find_map(|device| {
            let name = device.circuit_name.trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        });
        let circuit_entity_id = match circuit_name.as_deref() {
            Some(name) => format!("{name} ({circuit_id})"),
            None => circuit_id.clone(),
        };
        let circuit_label = circuit_name.unwrap_or_else(|| circuit_id.clone());

        let mut cap_down = 0.0f32;
        let mut cap_up = 0.0f32;
        let mut device_ids: Vec<String> = devices
            .iter()
            .map(|device| device.device_id.clone())
            .collect();
        device_ids.sort();
        device_ids.dedup();
        for device in devices.iter() {
            cap_down = cap_down.max(device.download_max_mbps);
            cap_up = cap_up.max(device.upload_max_mbps);
        }
        let duplicate_details: Vec<(String, Vec<String>)> = device_ids
            .iter()
            .filter_map(|device_id| {
                duplicate_device_ids
                    .get(device_id)
                    .map(|circuits| (device_id.clone(), circuits.clone()))
            })
            .collect();

        entries.insert(
            circuit_id.clone(),
            CircuitInventoryEntry {
                circuit_hash: hash_to_i64(circuit_id),
                circuit_entity_id,
                circuit_label,
                devices,
                device_ids,
                cap_down,
                cap_up,
                duplicate_details,
            },
        );
    }

    CircuitInventory {
        shaped_devices_ptr: shaped as *const _ as usize,
        circuit_ids,
        entries,
        all_device_ids,
    }
}

fn circuit_evaluation_batch_size(managed_circuits: usize, all_circuits: bool) -> usize {
    if managed_circuits == 0 {
        return 0;
    }
    if !all_circuits {
        return managed_circuits;
    }

    let target = managed_circuits.div_ceil(TREEGUARD_CIRCUIT_TARGET_SWEEP_SECONDS);
    managed_circuits.min(target.max(TREEGUARD_CIRCUIT_MIN_BATCH_SIZE))
}

fn collect_circuit_batch<'a>(
    enrolled_circuits: &'a [String],
    cursor: &mut usize,
    batch_size: usize,
) -> Vec<&'a str> {
    if enrolled_circuits.is_empty() || batch_size == 0 {
        *cursor = 0;
        return Vec::new();
    }

    if *cursor >= enrolled_circuits.len() {
        *cursor = 0;
    }

    let mut batch: Vec<&str> = Vec::with_capacity(batch_size.min(enrolled_circuits.len()));
    let mut index = *cursor;
    for _ in 0..batch_size.min(enrolled_circuits.len()) {
        batch.push(enrolled_circuits[index].as_str());
        index += 1;
        if index >= enrolled_circuits.len() {
            index = 0;
        }
    }

    *cursor = index;
    batch
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
    let mut warning_limiter = TreeguardWarningLimiter::default();

    let Ok(config) = load_config() else {
        status.enabled = false;
        status.dry_run = true;
        status.paused_for_bakery_reload = false;
        status.pause_reason = None;
        status.cpu_max_pct = None;
        status.managed_nodes = 0;
        status.managed_circuits = 0;
        status.virtualized_nodes = 0;
        status.cake_circuits = 0;
        status.mixed_sqm_circuits = 0;
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
        runtime_state.pending_link_operations.clear();
        push_activity(
            activity,
            TreeguardActivityEntry {
                time: now_unix.to_string(),
                entity_type: "treeguard".to_string(),
                entity_id: "treeguard".to_string(),
                action: "dry_run_toggled".to_string(),
                persisted: false,
                reason: "Dry-run mode changed; state machines reset.".to_string(),
                ..Default::default()
            },
        );
    }
    runtime_state.last_dry_run = Some(tg.dry_run);

    if pause_for_bakery_reload(status, tick_seconds, runtime_state, tg.enabled, tg.dry_run) {
        return;
    }

    let shaped = SHAPED_DEVICES.load();
    ensure_circuit_inventory(runtime_state, &shaped);

    let link_states = &mut runtime_state.link_states;
    let circuit_states = &mut runtime_state.circuit_states;
    let runtime_virtualized_nodes = &mut runtime_state.runtime_virtualized_nodes;
    let pending_link_operations = &mut runtime_state.pending_link_operations;
    let link_virtualization_backoff_until_unix =
        &mut runtime_state.link_virtualization_backoff_until_unix;
    let managed_nodes = &mut runtime_state.managed_nodes;
    let managed_device_ids = &mut runtime_state.managed_device_ids;
    let duplicate_device_conflict_circuits = &mut runtime_state.duplicate_device_conflict_circuits;

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
    if let Some(notice) = lqos_config::treeguard_cpu_mode_migration_notice() {
        warnings.push(notice);
    }

    let (total_nodes_count, total_circuits_count) = current_topology_totals();

    let managed_nodes_count: usize = if tg.links.all_nodes {
        total_nodes_count
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
        total_circuits_count
    } else {
        tg.circuits.circuits.len()
    };

    status.enabled = tg.enabled;
    status.dry_run = tg.dry_run;
    status.cpu_max_pct = cpu_max_pct;
    status.total_nodes = total_nodes_count;
    status.total_circuits = total_circuits_count;
    status.managed_nodes = managed_nodes_count;
    status.managed_circuits = managed_circuits_count;
    status.warnings = warnings;

    reconcile_pending_link_operations(
        status,
        activity,
        now_unix,
        &mut warning_limiter,
        pending_link_operations,
        runtime_virtualized_nodes,
        link_virtualization_backoff_until_unix,
        link_states,
    );

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
        let mut removed: FxHashSet<String> =
            match OverrideStore::load_layer(OverrideLayer::Treeguard) {
                Ok(of) => of
                    .network_adjustments()
                    .iter()
                    .filter_map(|adj| match adj {
                        NetworkAdjustment::SetNodeVirtual { node_name, .. } => {
                            Some(node_name.clone())
                        }
                        _ => None,
                    })
                    .collect(),
                Err(e) => {
                    status.warnings.push(format!(
                        "TreeGuard links: unable to load TreeGuard overrides for cleanup: {e}"
                    ));
                    FxHashSet::default()
                }
            };
        removed.extend(runtime_virtualized_nodes.iter().cloned());
        for node_name in removed {
            clear_legacy_treeguard_virtual_override(
                status,
                activity,
                now_unix,
                &node_name,
                "TreeGuard disabled or links disabled",
            );
            restore_runtime_virtualization_if_needed(
                status,
                activity,
                now_unix,
                &node_name,
                "TreeGuard disabled or links disabled",
                tg.dry_run,
                runtime_virtualized_nodes,
                pending_link_operations,
                link_virtualization_backoff_until_unix,
                link_states,
            );
            managed_nodes.remove(&node_name);
            link_states.remove(&node_name);
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

        let mut removed: FxHashSet<String> = if tg.links.all_nodes {
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
        if tg.links.all_nodes {
            let current: FxHashSet<&str> = reader
                .get_nodes_when_ready()
                .iter()
                .filter(|n| n.name != "Root")
                .map(|n| n.name.as_str())
                .collect();
            removed.extend(
                runtime_virtualized_nodes
                    .iter()
                    .filter(|n| !current.contains(n.as_str()))
                    .cloned(),
            );
        } else {
            removed.extend(
                runtime_virtualized_nodes
                    .iter()
                    .filter(|n| !allowlisted_nodes.contains(*n) && !top_level_nodes.contains(*n))
                    .cloned(),
            );
        }
        for node_name in removed {
            clear_legacy_treeguard_virtual_override(
                status,
                activity,
                now_unix,
                &node_name,
                "Node removed from allowlist",
            );
            restore_runtime_virtualization_if_needed(
                status,
                activity,
                now_unix,
                &node_name,
                "Node removed from allowlist",
                tg.dry_run,
                runtime_virtualized_nodes,
                pending_link_operations,
                link_virtualization_backoff_until_unix,
                link_states,
            );
            managed_nodes.remove(&node_name);
            link_states.remove(&node_name);
        }
        let nodes = reader.get_nodes_when_ready();
        let parent_by_index: Vec<Option<usize>> =
            nodes.iter().map(|node| node.immediate_parent).collect();
        let subtree_node_counts = build_subtree_node_counts(&parent_by_index);
        let direct_child_site_counts = direct_child_site_counts_by_node(nodes);
        let direct_circuit_counts = direct_circuit_counts_by_node(&shaped.devices);
        let retained_runtime_branch_nodes: FxHashSet<String> =
            lqos_bakery::bakery_runtime_node_branch_snapshots()
                .into_iter()
                .map(|snapshot| snapshot.site_name)
                .collect();
        let existing_virtualized_indices: FxHashSet<usize> = nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                (runtime_virtualized_nodes.contains(&node.name)
                    || retained_runtime_branch_nodes.contains(&node.name))
                .then_some(index)
            })
            .collect();

        let mut enrolled_nodes: Vec<String> = if tg.links.all_nodes {
            nodes
                .iter()
                .filter(|node| node.name != "Root")
                .map(|node| node.name.clone())
                .collect()
        } else {
            let mut enrolled = tg.links.nodes.clone();
            if top_level_auto_virtualize {
                enrolled.extend(top_level_nodes.iter().cloned());
            }
            enrolled.sort();
            enrolled.dedup();
            enrolled
        };

        if tg.links.all_nodes {
            enrolled_nodes.sort();
        }

        let mut pending_link_decisions = Vec::new();

        for node_name in enrolled_nodes.iter() {
            if operator_virtual_node_overrides.contains(node_name) {
                status.warnings.push(format!(
                    "TreeGuard links: node '{node_name}' has an operator virtual override; TreeGuard will not manage it."
                ));
                clear_legacy_treeguard_virtual_override(
                    status,
                    activity,
                    now_unix,
                    node_name,
                    "Operator override present; TreeGuard will not manage this node.",
                );
                restore_runtime_virtualization_if_needed(
                    status,
                    activity,
                    now_unix,
                    node_name,
                    "Operator override present; TreeGuard will not manage this node.",
                    tg.dry_run,
                    runtime_virtualized_nodes,
                    pending_link_operations,
                    link_virtualization_backoff_until_unix,
                    link_states,
                );
                managed_nodes.remove(node_name);
                link_states.remove(node_name);
                continue;
            }

            let Some(index) = reader.get_index_for_name(node_name) else {
                status.warnings.push(format!(
                    "TreeGuard links allowlist: node '{node_name}' not found in network.json."
                ));
                clear_legacy_treeguard_virtual_override(
                    status,
                    activity,
                    now_unix,
                    node_name,
                    "Node no longer exists in network.json",
                );
                restore_runtime_virtualization_if_needed(
                    status,
                    activity,
                    now_unix,
                    node_name,
                    "Node no longer exists in network.json",
                    tg.dry_run,
                    runtime_virtualized_nodes,
                    pending_link_operations,
                    link_virtualization_backoff_until_unix,
                    link_states,
                );
                managed_nodes.remove(node_name);
                link_states.remove(node_name);
                continue;
            };
            let Some(node) = nodes.get(index) else {
                status.warnings.push(format!(
                    "TreeGuard links allowlist: node '{node_name}' index not present."
                ));
                clear_legacy_treeguard_virtual_override(
                    status,
                    activity,
                    now_unix,
                    node_name,
                    "Node index no longer exists in network.json",
                );
                restore_runtime_virtualization_if_needed(
                    status,
                    activity,
                    now_unix,
                    node_name,
                    "Node index no longer exists in network.json",
                    tg.dry_run,
                    runtime_virtualized_nodes,
                    pending_link_operations,
                    link_virtualization_backoff_until_unix,
                    link_states,
                );
                managed_nodes.remove(node_name);
                link_states.remove(node_name);
                continue;
            };

            if node.virtual_node {
                status.warnings.push(format!(
                    "TreeGuard links: node '{node_name}' is marked virtual in base network.json; TreeGuard will not manage it."
                ));
                clear_legacy_treeguard_virtual_override(
                    status,
                    activity,
                    now_unix,
                    node_name,
                    "Node is marked virtual in base network.json; TreeGuard refuses to manage base-virtual nodes.",
                );
                restore_runtime_virtualization_if_needed(
                    status,
                    activity,
                    now_unix,
                    node_name,
                    "Node is marked virtual in base network.json; TreeGuard refuses to manage base-virtual nodes.",
                    tg.dry_run,
                    runtime_virtualized_nodes,
                    pending_link_operations,
                    link_virtualization_backoff_until_unix,
                    link_states,
                );
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
                if runtime_virtualized_nodes.contains(node_name) {
                    state.desired = LinkVirtualState::Virtual;
                }
                state
            });
            prune_recent_changes(&mut state.recent_changes_unix, now_unix);
            let topology_fingerprint = LinkTopologyFingerprint {
                direct_child_sites: direct_child_site_counts.get(index).copied().unwrap_or(0),
                direct_circuits: direct_circuit_counts.get(node_name).copied().unwrap_or(0),
            };
            if clear_structural_ineligible_if_topology_changed(state, topology_fingerprint) {
                link_virtualization_backoff_until_unix.remove(node_name);
            }
            state.topology_fingerprint = topology_fingerprint;

            let ewma_down = state
                .down
                .util_ewma_pct
                .update(util_down_pct, UTIL_EWMA_ALPHA);
            let ewma_up = state.up.util_ewma_pct.update(util_up_pct, UTIL_EWMA_ALPHA);

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
            let top_level_safe_util_pct = tg.links.top_level_safe_util_pct.clamp(0.0, 100.0) as f64;
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
                update_above_since(
                    &mut state.down.top_level_emergency_since_unix,
                    now_unix,
                    ewma_down,
                    TOP_LEVEL_EMERGENCY_UTIL_PCT,
                );
                update_above_since(
                    &mut state.up.top_level_emergency_since_unix,
                    now_unix,
                    ewma_up,
                    TOP_LEVEL_EMERGENCY_UTIL_PCT,
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
                            >= u64::from(tg.links.rtt_missing_seconds).saturating_mul(1_000_000_000)
                    }
                }
            };

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
                let sustained_safe = is_sustained_window(
                    now_unix,
                    state.down.top_level_safe_since_unix,
                    state.up.top_level_safe_since_unix,
                    TOP_LEVEL_SAFE_SUSTAIN_MINUTES,
                );
                let emergency_util_sustained = state
                    .down
                    .top_level_emergency_since_unix
                    .is_some_and(|since| {
                        now_unix.saturating_sub(since) >= TOP_LEVEL_EMERGENCY_SUSTAIN_SECONDS
                    })
                    || state
                        .up
                        .top_level_emergency_since_unix
                        .is_some_and(|since| {
                            now_unix.saturating_sub(since) >= TOP_LEVEL_EMERGENCY_SUSTAIN_SECONDS
                        });
                decisions::decide_top_level_link_virtualization(
                    decisions::TopLevelLinkVirtualizationInput {
                        now_unix,
                        cpu_max_pct,
                        cpu_cfg: &tg.cpu,
                        links_cfg: &tg.links,
                        qoo_cfg: &tg.qoo,
                        rtt_missing,
                        qoo,
                        util_ewma_pct,
                        safe_util_pct: top_level_safe_util_pct,
                        sustained_safe,
                        emergency_util_sustained,
                        state,
                    },
                )
            } else {
                decisions::decide_link_virtualization(decisions::LinkVirtualizationInput {
                    now_unix,
                    allowlisted: tg.links.all_nodes || allowlisted_nodes.contains(node_name),
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

            if let decisions::LinkVirtualDecision::Set(target) = decision
                && target != state.desired
            {
                if let Some(reason) = latched_structural_ineligible_reason(state, target) {
                    let details = structural_failure_reason_label(reason);
                    warning_limiter.push(
                        status,
                        format!(
                            "runtime_structural_ineligible|{}|{:?}",
                            node_name, reason
                        ),
                        format!(
                            "TreeGuard links: node '{node_name}' remains ineligible for runtime virtualization ({details}) until its direct topology changes."
                        ),
                        format!(
                            "TreeGuard runtime structural-ineligible warnings for reason={details}"
                        ),
                    );
                    continue;
                }
                let reason = if is_top_level {
                    match target {
                        LinkVirtualState::Virtual => format!(
                            "Top-level safe: sustained utilization below {:.1}% for {} minutes",
                            top_level_safe_util_pct, TOP_LEVEL_SAFE_SUSTAIN_MINUTES
                        ),
                        LinkVirtualState::Physical => {
                            if state
                                .down
                                .top_level_emergency_since_unix
                                .is_some_and(|since| {
                                    now_unix.saturating_sub(since)
                                        >= TOP_LEVEL_EMERGENCY_SUSTAIN_SECONDS
                                })
                                || state
                                    .up
                                    .top_level_emergency_since_unix
                                    .is_some_and(|since| {
                                        now_unix.saturating_sub(since)
                                            >= TOP_LEVEL_EMERGENCY_SUSTAIN_SECONDS
                                    })
                            {
                                format!(
                                    "Top-level emergency restore: utilization >= {:.1}% for {}s",
                                    TOP_LEVEL_EMERGENCY_UTIL_PCT,
                                    TOP_LEVEL_EMERGENCY_SUSTAIN_SECONDS
                                )
                            } else {
                                format!(
                                    "Top-level restore: utilization above {:.1}%",
                                    top_level_safe_util_pct
                                )
                            }
                        }
                    }
                } else {
                    "Decision policy matched".to_string()
                };
                pending_link_decisions.push(PendingLinkVirtualizationDecision {
                    node_name: node_name.clone(),
                    node_index: index,
                    target,
                    reason,
                    subtree_nodes: subtree_node_counts[index],
                    current_subtree_throughput_mbps: mbps_down.max(mbps_up),
                    explicit_allowlist: allowlisted_nodes.contains(node_name),
                    is_top_level,
                    value_score: link_virtualization_value_score(
                        is_top_level,
                        subtree_node_counts[index],
                        cap_down,
                        cap_up,
                    ),
                });
            }

            managed_nodes.insert(node_name.clone());
        }

        let (selected_link_decisions, deferred_link_decisions, skipped_low_value_decisions) =
            select_link_virtualization_candidates(
                pending_link_decisions,
                &parent_by_index,
                &existing_virtualized_indices,
                runtime_virtualized_nodes.len(),
            );
        if deferred_link_decisions > 0 {
            status.warnings.push(format!(
                "TreeGuard links: deferred {deferred_link_decisions} lower-value or over-budget node virtualization changes this tick."
            ));
        }
        if skipped_low_value_decisions > 0 {
            status.warnings.push(format!(
                "TreeGuard links: skipped {skipped_low_value_decisions} low-value automatic node virtualization candidates this tick because the subtree was too small for its current throughput."
            ));
        }

        for decision in selected_link_decisions {
            let Some(state) = link_states.get_mut(&decision.node_name) else {
                continue;
            };
            apply_link_virtualization_decision(
                status,
                activity,
                now_unix,
                &decision.node_name,
                decision.target,
                decision.is_top_level,
                decision.reason,
                tg.dry_run,
                state,
                pending_link_operations,
                link_virtualization_backoff_until_unix,
            );
        }
    }

    // --- Circuit sampling + decisions (SQM switching) ---
    let manage_circuits = tg.enabled && tg.circuits.enabled;
    let circuit_inventory = &runtime_state.circuit_inventory;

    let enrolled_circuits: Vec<String> = if tg.circuits.all_circuits {
        circuit_inventory.circuit_ids.clone()
    } else {
        let mut v = tg.circuits.circuits.clone();
        v.sort();
        v.dedup();
        v
    };
    status.managed_circuits = enrolled_circuits.len();

    let allowlisted_circuits: FxHashSet<String> = if tg.circuits.all_circuits {
        FxHashSet::default()
    } else {
        enrolled_circuits.iter().cloned().collect()
    };
    let desired_device_ids: FxHashSet<String> = if manage_circuits && !tg.circuits.all_circuits {
        let mut desired = FxHashSet::default();
        for circuit_id in enrolled_circuits.iter() {
            if let Some(entry) = circuit_inventory.entries.get(circuit_id) {
                desired.extend(entry.device_ids.iter().cloned());
            }
        }
        desired
    } else {
        FxHashSet::default()
    };
    let mut circuit_change_budget_remaining = TREEGUARD_CIRCUIT_CHANGE_BUDGET_PER_TICK;
    let mut deferred_circuit_sqm_changes = 0usize;

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
                                ..Default::default()
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
        runtime_state.circuit_batch_cursor = 0;
    } else {
        circuit_states.retain(|circuit_id, _| {
            circuit_inventory.entries.contains_key(circuit_id)
                && (tg.circuits.all_circuits || allowlisted_circuits.contains(circuit_id))
        });

        let treeguard_device_ids_with_overrides: FxHashSet<String> = treeguard_overrides_snapshot
            .as_ref()
            .map(overrides_sqm_device_ids)
            .unwrap_or_default();
        let removed: Vec<String> = treeguard_device_ids_with_overrides
            .iter()
            .filter(|device_id| {
                if tg.circuits.all_circuits {
                    !circuit_inventory.all_device_ids.contains(*device_id)
                } else {
                    !desired_device_ids.contains(*device_id)
                }
            })
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
                                ..Default::default()
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

        let rtt_snapshot = CIRCUIT_RTT_BUFFERS.load();
        let live_snapshot = fresh_circuit_live_snapshot();
        let batch_size =
            circuit_evaluation_batch_size(enrolled_circuits.len(), tg.circuits.all_circuits);
        let circuit_batch = collect_circuit_batch(
            &enrolled_circuits,
            &mut runtime_state.circuit_batch_cursor,
            batch_size,
        );
        let sqm_batch_id = next_sqm_batch_id(&mut runtime_state.next_sqm_batch_id);

        for circuit_id in circuit_batch {
            let Some(entry) = circuit_inventory.entries.get(circuit_id) else {
                continue;
            };

            if !entry.duplicate_details.is_empty() {
                let was_conflicted = duplicate_device_conflict_circuits.contains(circuit_id);
                duplicate_device_conflict_circuits.insert(circuit_id.to_string());
                let duplicate_reason = entry
                    .duplicate_details
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
                if !was_conflicted {
                    push_activity(
                        activity,
                        TreeguardActivityEntry {
                            time: now_unix.to_string(),
                            entity_type: "circuit".to_string(),
                            entity_id: entry.circuit_entity_id.clone(),
                            action: "skip_duplicate_device_id".to_string(),
                            persisted: false,
                            reason: format!(
                                "TreeGuard refuses circuits with duplicate device IDs. {duplicate_reason}"
                            ),
                            ..Default::default()
                        },
                    );
                }
                if !entry.device_ids.is_empty() {
                    match overrides::clear_device_overrides(&entry.device_ids) {
                        Ok(changed) => {
                            if changed {
                                push_activity(
                                    activity,
                                    TreeguardActivityEntry {
                                        time: now_unix.to_string(),
                                        entity_type: "circuit".to_string(),
                                        entity_id: entry.circuit_entity_id.clone(),
                                        action: "clear_sqm_overrides_duplicate_device_id"
                                            .to_string(),
                                        persisted: true,
                                        reason: "Duplicate device IDs detected; cleared TreeGuard SQM overlays and skipped management.".to_string(),
                                        ..Default::default()
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
                    for device_id in entry.device_ids.iter() {
                        managed_device_ids.remove(device_id);
                    }
                }
                circuit_states.remove(circuit_id);
                continue;
            }

            duplicate_device_conflict_circuits.remove(circuit_id);

            let base_sqm = base_circuit_sqm_state(
                &entry.devices,
                operator_overrides_snapshot.as_ref(),
                &config,
                entry.cap_down,
                entry.cap_up,
            );
            let state = circuit_states
                .entry(circuit_id.to_string())
                .or_insert_with(|| {
                    let mut state = CircuitState::default();
                    state.down.desired = base_sqm.down;
                    state.up.desired = base_sqm.up;
                    if let Some(overrides) = treeguard_overrides_snapshot.as_ref()
                        && let Some(token) =
                            find_circuit_override_token_in_overrides(&entry.devices, overrides)
                    {
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

            let operator_conflict = entry
                .device_ids
                .iter()
                .any(|device_id| operator_sqm_device_overrides.contains(device_id));
            if operator_conflict {
                status.warnings.push(format!(
                    "TreeGuard circuits: circuit '{circuit_id}' has operator SQM overrides; TreeGuard will not manage it."
                ));
                if !entry.device_ids.is_empty() {
                    match overrides::clear_device_overrides(&entry.device_ids) {
                        Ok(changed) => {
                            if changed {
                                push_activity(
                                    activity,
                                    TreeguardActivityEntry {
                                        time: now_unix.to_string(),
                                        entity_type: "circuit".to_string(),
                                        entity_id: entry.circuit_entity_id.clone(),
                                        action: "clear_sqm_overrides_conflict".to_string(),
                                        persisted: true,
                                        reason: "Operator SQM overrides present; cleared TreeGuard SQM overlays.".to_string(),
                                        ..Default::default()
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
                    for device_id in entry.device_ids.iter() {
                        managed_device_ids.remove(device_id);
                    }
                }
                continue;
            }

            for device_id in entry.device_ids.iter() {
                managed_device_ids.insert(device_id.clone());
            }

            if !treeguard_manages_circuit_direction(base_sqm.down)
                && !treeguard_manages_circuit_direction(base_sqm.up)
                && state.down.desired == base_sqm.down
                && state.up.desired == base_sqm.up
            {
                continue;
            }

            let live_rollup = live_snapshot.by_circuit_id.get(circuit_id);
            process_circuit_tick(
                CircuitTickContext {
                    status,
                    activity,
                    managed_device_ids,
                    now_unix,
                    now_nanos_since_boot,
                    cpu_max_pct,
                    dry_run: tg.dry_run,
                    circuit_id,
                    circuit_entity_id: &entry.circuit_entity_id,
                    circuit_label: &entry.circuit_label,
                    devices: &entry.devices,
                    sqm_batch_id: &sqm_batch_id,
                    allowlisted: tg.circuits.all_circuits
                        || allowlisted_circuits.contains(circuit_id),
                    cap_down: entry.cap_down,
                    cap_up: entry.cap_up,
                    bps: live_rollup
                        .map(|rollup| rollup.bytes_per_second)
                        .unwrap_or(DownUpOrder { down: 0, up: 0 }),
                    last_rtt_seen_nanos: rtt_snapshot
                        .get(&entry.circuit_hash)
                        .map(|buf| buf.last_seen),
                    qoo: live_rollup.map(|rollup| rollup.qoo).unwrap_or(DownUpOrder {
                        down: None,
                        up: None,
                    }),
                    cpu_cfg: &tg.cpu,
                    circuits_cfg: &tg.circuits,
                    qoo_cfg: &tg.qoo,
                    base_sqm,
                    circuit_change_budget_remaining: &mut circuit_change_budget_remaining,
                    deferred_circuit_sqm_changes: &mut deferred_circuit_sqm_changes,
                },
                state,
                overrides::set_devices_sqm_override,
                overrides::clear_device_overrides,
                |circuit_id, devices, token| {
                    bakery::apply_circuit_sqm_override_live(circuit_id, devices, token)
                },
            );
        }
        if deferred_circuit_sqm_changes > 0 {
            status.warnings.push(format!(
                "TreeGuard circuits: deferred {} SQM changes because the per-tick circuit change budget ({}) was exhausted.",
                deferred_circuit_sqm_changes, TREEGUARD_CIRCUIT_CHANGE_BUDGET_PER_TICK
            ));
        }
    }

    status.virtualized_nodes = runtime_virtualized_nodes.len();
    let mut cake_circuits = 0usize;
    let mut mixed_sqm_circuits = 0usize;
    let mut fq_codel_circuits = 0usize;
    for state in circuit_states.values() {
        match (state.down.desired, state.up.desired) {
            (CircuitSqmState::Cake, CircuitSqmState::Cake) => {
                cake_circuits = cake_circuits.saturating_add(1);
            }
            (CircuitSqmState::FqCodel, CircuitSqmState::FqCodel) => {
                fq_codel_circuits = fq_codel_circuits.saturating_add(1);
            }
            _ => {
                mixed_sqm_circuits = mixed_sqm_circuits.saturating_add(1);
            }
        }
    }
    status.cake_circuits = cake_circuits;
    status.mixed_sqm_circuits = mixed_sqm_circuits;
    status.fq_codel_circuits = fq_codel_circuits;
    warning_limiter.flush(status);
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
        lqos_bakery::bakery_reload_required_reason(),
    )
}

fn pause_for_bakery_reload_with_flag(
    status: &mut TreeguardStatusData,
    tick_seconds: &mut u64,
    runtime_state: &mut TreeguardRuntimeState,
    enabled: bool,
    dry_run: bool,
    bakery_reload_in_progress: bool,
    bakery_reload_required_reason: Option<String>,
) -> bool {
    let pause_reason = if bakery_reload_in_progress {
        Some("Bakery full reload in progress".to_string())
    } else {
        bakery_reload_required_reason
    };
    let paused = enabled && pause_reason.is_some();
    if paused {
        let pause_reason =
            pause_reason.unwrap_or_else(|| "Bakery full reload in progress".to_string());
        if !runtime_state.paused_for_bakery_reload {
            info!("TreeGuard: pausing because {}", pause_reason);
            runtime_state.paused_for_bakery_reload = true;
        }
        *tick_seconds = (*tick_seconds).max(5);
        status.enabled = enabled;
        status.dry_run = dry_run;
        status.paused_for_bakery_reload = true;
        status.pause_reason = Some(pause_reason.clone());
        status.cpu_max_pct = None;
        status.last_action_summary = Some(format!("Paused while {}", pause_reason));
        status
            .warnings
            .push(format!("TreeGuard paused while {}.", pause_reason));
        return true;
    }

    if runtime_state.paused_for_bakery_reload {
        info!("TreeGuard: resuming after Bakery pause");
        runtime_state.paused_for_bakery_reload = false;
    }
    status.paused_for_bakery_reload = false;
    status.pause_reason = None;
    if status
        .last_action_summary
        .as_deref()
        .is_some_and(|summary| summary.starts_with("Paused while Bakery "))
    {
        status.last_action_summary = None;
    }

    false
}

fn clear_legacy_treeguard_virtual_override(
    status: &mut TreeguardStatusData,
    activity: &mut VecDeque<TreeguardActivityEntry>,
    now_unix: u64,
    node_name: &str,
    reason: &str,
) {
    match overrides::clear_node_virtual(node_name) {
        Ok(changed) => {
            if changed {
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "node".to_string(),
                        entity_id: node_name.to_string(),
                        action: "clear_virtual_override".to_string(),
                        persisted: true,
                        reason: reason.to_string(),
                        ..Default::default()
                    },
                );
            }
        }
        Err(e) => {
            status.warnings.push(format!(
                "TreeGuard links: failed to clear legacy virtual override for node '{node_name}': {e}"
            ));
            push_activity(
                activity,
                TreeguardActivityEntry {
                    time: now_unix.to_string(),
                    entity_type: "node".to_string(),
                    entity_id: node_name.to_string(),
                    action: "clear_virtual_override_failed".to_string(),
                    persisted: false,
                    reason: format!("Overrides write failed: {e}"),
                    ..Default::default()
                },
            );
        }
    }
}

fn current_link_virtual_state(
    runtime_virtualized_nodes: &FxHashSet<String>,
    node_name: &str,
) -> LinkVirtualState {
    if runtime_virtualized_nodes.contains(node_name) {
        LinkVirtualState::Virtual
    } else {
        LinkVirtualState::Physical
    }
}

#[allow(clippy::too_many_arguments)]
fn reconcile_pending_link_operations(
    status: &mut TreeguardStatusData,
    activity: &mut VecDeque<TreeguardActivityEntry>,
    now_unix: u64,
    warning_limiter: &mut TreeguardWarningLimiter,
    pending_link_operations: &mut FxHashMap<String, PendingLinkOperation>,
    runtime_virtualized_nodes: &mut FxHashSet<String>,
    link_virtualization_backoff_until_unix: &mut FxHashMap<String, u64>,
    link_states: &mut FxHashMap<String, LinkState>,
) {
    let pending_nodes: Vec<String> = pending_link_operations.keys().cloned().collect();
    for node_name in pending_nodes {
        let Some(pending) = pending_link_operations.get(&node_name).cloned() else {
            continue;
        };
        let Some(snapshot) = bakery::node_virtualization_operation_status(&node_name) else {
            continue;
        };
        match snapshot.status {
            BakeryRuntimeNodeOperationStatus::Submitted
            | BakeryRuntimeNodeOperationStatus::Applying => {}
            BakeryRuntimeNodeOperationStatus::Completed
            | BakeryRuntimeNodeOperationStatus::AppliedAwaitingCleanup => {
                if pending.target == LinkVirtualState::Virtual {
                    runtime_virtualized_nodes.insert(node_name.clone());
                } else {
                    runtime_virtualized_nodes.remove(&node_name);
                }
                link_virtualization_backoff_until_unix.remove(&node_name);
                pending_link_operations.remove(&node_name);
                if let Some(state) = link_states.get_mut(&node_name) {
                    state.desired = pending.target;
                    state.last_change_unix = Some(now_unix);
                    state.recent_changes_unix.push_back(now_unix);
                    state.structural_ineligible = None;
                    prune_recent_changes(&mut state.recent_changes_unix, now_unix);
                }
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "node".to_string(),
                        entity_id: node_name.clone(),
                        action: match pending.target {
                            LinkVirtualState::Physical => "unvirtualize".to_string(),
                            LinkVirtualState::Virtual => "virtualize".to_string(),
                        },
                        persisted: true,
                        reason: if snapshot.status
                            == BakeryRuntimeNodeOperationStatus::AppliedAwaitingCleanup
                        {
                            format!(
                                "{}. Bakery operation {} applied; cleanup pending.",
                                pending.reason, snapshot.operation_id
                            )
                        } else {
                            pending.reason
                        },
                        ..Default::default()
                    },
                );
                status.last_action_summary = Some(match pending.target {
                    LinkVirtualState::Physical => {
                        format!("Unvirtualized node '{node_name}'")
                    }
                    LinkVirtualState::Virtual => {
                        format!("Virtualized node '{node_name}'")
                    }
                });
                if snapshot.status == BakeryRuntimeNodeOperationStatus::AppliedAwaitingCleanup {
                    let target_label = if pending.target == LinkVirtualState::Virtual {
                        "virtualization"
                    } else {
                        "restore"
                    };
                    warning_limiter.push(
                        status,
                        format!("runtime_cleanup_pending|{target_label}"),
                        format!(
                            "TreeGuard links: node '{node_name}' runtime {target_label} applied in Bakery operation {} and is awaiting cleanup.",
                            snapshot.operation_id
                        ),
                        format!("TreeGuard runtime {target_label} cleanup-pending warnings"),
                    );
                }
            }
            BakeryRuntimeNodeOperationStatus::Deferred => {
                let until = snapshot.next_retry_at_unix.unwrap_or_else(|| {
                    now_unix.saturating_add(TREEGUARD_LINK_VIRTUALIZATION_DEFERRED_BACKOFF_SECONDS)
                });
                link_virtualization_backoff_until_unix.insert(node_name.clone(), until);
                pending_link_operations.remove(&node_name);
                if let Some(state) = link_states.get_mut(&node_name) {
                    state.desired =
                        current_link_virtual_state(runtime_virtualized_nodes, &node_name);
                }
                let details = snapshot.last_error.unwrap_or_else(|| {
                    format!(
                        "Bakery operation {} deferred by capacity",
                        snapshot.operation_id
                    )
                });
                let target_label = if pending.target == LinkVirtualState::Virtual {
                    "virtualization"
                } else {
                    "restore"
                };
                warning_limiter.push(
                    status,
                    format!("runtime_deferred|{target_label}|{details}"),
                    format!(
                        "TreeGuard links: deferred runtime {target_label} for node '{node_name}': {details}. Retrying after {until}."
                    ),
                    format!("TreeGuard deferred runtime {target_label} warnings for reason={details}"),
                );
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "node".to_string(),
                        entity_id: node_name.clone(),
                        action: match pending.target {
                            LinkVirtualState::Physical => "unvirtualize_deferred".to_string(),
                            LinkVirtualState::Virtual => "virtualize_deferred".to_string(),
                        },
                        persisted: false,
                        reason: format!(
                            "{}. Bakery deferred the operation: {details}",
                            pending.reason
                        ),
                        ..Default::default()
                    },
                );
            }
            BakeryRuntimeNodeOperationStatus::Failed | BakeryRuntimeNodeOperationStatus::Dirty => {
                let structural_ineligible = snapshot.failure_reason;
                let until = now_unix.saturating_add(TREEGUARD_LINK_VIRTUALIZATION_BACKOFF_SECONDS);
                if structural_ineligible.is_none() {
                    link_virtualization_backoff_until_unix.insert(node_name.clone(), until);
                } else {
                    link_virtualization_backoff_until_unix.remove(&node_name);
                }
                pending_link_operations.remove(&node_name);
                if let Some(state) = link_states.get_mut(&node_name) {
                    state.desired =
                        current_link_virtual_state(runtime_virtualized_nodes, &node_name);
                    state.structural_ineligible =
                        structural_ineligible.map(|reason| LinkStructuralIneligibleState {
                            reason,
                            topology_fingerprint: state.topology_fingerprint,
                        });
                }
                let details = snapshot.last_error.unwrap_or_else(|| {
                    format!("Bakery operation {} failed", snapshot.operation_id)
                });
                let target_label = if pending.target == LinkVirtualState::Virtual {
                    "virtualization"
                } else {
                    "restore"
                };
                if let Some(reason) = structural_ineligible {
                    let reason_label = structural_failure_reason_label(reason);
                    warning_limiter.push(
                        status,
                        format!("runtime_structural_ineligible|{target_label}|{:?}", reason),
                        format!(
                            "TreeGuard links: node '{node_name}' is structurally ineligible for runtime {target_label} ({reason_label}): {details}. TreeGuard will retry only after the node's direct topology changes."
                        ),
                        format!(
                            "TreeGuard runtime structural-ineligible {target_label} warnings for reason={reason_label}"
                        ),
                    );
                } else {
                    warning_limiter.push(
                        status,
                        format!("runtime_failed|{target_label}|{details}"),
                        format!(
                            "TreeGuard links: failed to apply runtime {target_label} for node '{node_name}': {details}. Backing off until {until}."
                        ),
                        format!(
                            "TreeGuard failed runtime {target_label} warnings for reason={details}"
                        ),
                    );
                }
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "node".to_string(),
                        entity_id: node_name.clone(),
                        action: match pending.target {
                            LinkVirtualState::Physical => "unvirtualize_failed".to_string(),
                            LinkVirtualState::Virtual => "virtualize_failed".to_string(),
                        },
                        persisted: false,
                        reason: format!(
                            "{}. Bakery runtime operation failed: {details}",
                            pending.reason
                        ),
                        ..Default::default()
                    },
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn restore_runtime_virtualization_if_needed(
    status: &mut TreeguardStatusData,
    activity: &mut VecDeque<TreeguardActivityEntry>,
    now_unix: u64,
    node_name: &str,
    reason: &str,
    dry_run: bool,
    runtime_virtualized_nodes: &mut FxHashSet<String>,
    pending_link_operations: &mut FxHashMap<String, PendingLinkOperation>,
    link_virtualization_backoff_until_unix: &mut FxHashMap<String, u64>,
    link_states: &mut FxHashMap<String, LinkState>,
) {
    if !runtime_virtualized_nodes.contains(node_name) {
        if let Some(state) = link_states.get_mut(node_name) {
            state.desired = LinkVirtualState::Physical;
        }
        pending_link_operations.remove(node_name);
        link_virtualization_backoff_until_unix.remove(node_name);
        return;
    }

    if dry_run {
        status.warnings.push(format!(
            "TreeGuard links: node '{node_name}' remains runtime-virtualized because dry-run mode will not restore live topology."
        ));
        return;
    }

    if pending_link_operations
        .get(node_name)
        .is_some_and(|pending| pending.target == LinkVirtualState::Physical)
    {
        status.warnings.push(format!(
            "TreeGuard links: restore for node '{node_name}' is already queued in Bakery."
        ));
        return;
    }

    match bakery::submit_node_virtualization_live(node_name, false) {
        Ok(()) => {
            pending_link_operations.insert(
                node_name.to_string(),
                PendingLinkOperation {
                    target: LinkVirtualState::Physical,
                    reason: reason.to_string(),
                },
            );
            if let Some(state) = link_states.get_mut(node_name) {
                state.desired = LinkVirtualState::Physical;
            }
            push_activity(
                activity,
                TreeguardActivityEntry {
                    time: now_unix.to_string(),
                    entity_type: "node".to_string(),
                    entity_id: node_name.to_string(),
                    action: "unvirtualize_requested".to_string(),
                    persisted: false,
                    reason: format!("{reason}. Queued in Bakery for live restore."),
                    ..Default::default()
                },
            );
            status.last_action_summary = Some(format!("Queued restore for node '{node_name}'"));
        }
        Err(e) => {
            let until = now_unix.saturating_add(TREEGUARD_LINK_VIRTUALIZATION_BACKOFF_SECONDS);
            link_virtualization_backoff_until_unix.insert(node_name.to_string(), until);
            status.warnings.push(format!(
                "TreeGuard links: failed to submit restore for runtime-virtualized node '{node_name}': {e}. Backing off until {until}."
            ));
            push_activity(
                activity,
                TreeguardActivityEntry {
                    time: now_unix.to_string(),
                    entity_type: "node".to_string(),
                    entity_id: node_name.to_string(),
                    action: "unvirtualize_failed".to_string(),
                    persisted: false,
                    reason: format!("{reason}. Bakery restore submission failed: {e}"),
                    ..Default::default()
                },
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_link_virtualization_decision(
    status: &mut TreeguardStatusData,
    activity: &mut VecDeque<TreeguardActivityEntry>,
    now_unix: u64,
    node_name: &str,
    target: LinkVirtualState,
    is_top_level: bool,
    reason: String,
    dry_run: bool,
    state: &mut LinkState,
    pending_link_operations: &mut FxHashMap<String, PendingLinkOperation>,
    link_virtualization_backoff_until_unix: &mut FxHashMap<String, u64>,
) {
    if target == state.desired
        && pending_link_operations
            .get(node_name)
            .is_none_or(|pending| pending.target == target)
    {
        return;
    }

    let persist = !dry_run;
    if persist {
        if let Some(until) = link_virtualization_backoff_until_unix
            .get(node_name)
            .copied()
            && now_unix < until
        {
            status.warnings.push(format!(
                "TreeGuard links: node '{node_name}' runtime virtualization is temporarily ineligible until {until}."
            ));
            return;
        }

        if pending_link_operations
            .get(node_name)
            .is_some_and(|pending| pending.target == target)
        {
            status.warnings.push(format!(
                "TreeGuard links: node '{node_name}' runtime {} is already queued in Bakery.",
                if target == LinkVirtualState::Virtual {
                    "virtualization"
                } else {
                    "restore"
                }
            ));
            return;
        }

        if let Some((failure_reason, details)) =
            top_level_structural_ineligibility(state, target, is_top_level)
        {
            let reason_label = structural_failure_reason_label(failure_reason);
            link_virtualization_backoff_until_unix.remove(node_name);
            state.structural_ineligible = Some(LinkStructuralIneligibleState {
                reason: failure_reason,
                topology_fingerprint: state.topology_fingerprint,
            });
            status.warnings.push(format!(
                "TreeGuard links: node '{node_name}' is structurally ineligible for runtime virtualization ({reason_label}): {details}. TreeGuard rejected the change before submitting it to Bakery."
            ));
            status.last_action_summary =
                Some(format!("Rejected virtualization for node '{node_name}'"));
            push_activity(
                activity,
                TreeguardActivityEntry {
                    time: now_unix.to_string(),
                    entity_type: "node".to_string(),
                    entity_id: node_name.to_string(),
                    action: "virtualize_rejected".to_string(),
                    persisted: false,
                    reason: format!(
                        "{}. TreeGuard rejected runtime virtualization before submitting to Bakery: node {details}",
                        reason
                    ),
                    ..Default::default()
                },
            );
            return;
        }

        match bakery::submit_node_virtualization_live(
            node_name,
            target == LinkVirtualState::Virtual,
        ) {
            Ok(()) => {
                pending_link_operations.insert(
                    node_name.to_string(),
                    PendingLinkOperation { target, reason },
                );
                state.desired = target;
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "node".to_string(),
                        entity_id: node_name.to_string(),
                        action: match target {
                            LinkVirtualState::Physical => "unvirtualize_requested".to_string(),
                            LinkVirtualState::Virtual => "virtualize_requested".to_string(),
                        },
                        persisted: false,
                        reason: format!(
                            "{}. Queued in Bakery for live {}.",
                            pending_link_operations
                                .get(node_name)
                                .map(|pending| pending.reason.as_str())
                                .unwrap_or("TreeGuard queued a runtime topology change"),
                            if target == LinkVirtualState::Virtual {
                                "virtualization"
                            } else {
                                "restore"
                            }
                        ),
                        ..Default::default()
                    },
                );
                status.last_action_summary = Some(format!(
                    "Queued {} for node '{}'",
                    if target == LinkVirtualState::Virtual {
                        "virtualization"
                    } else {
                        "restore"
                    },
                    node_name
                ));
                return;
            }
            Err(e) => {
                let until = now_unix.saturating_add(TREEGUARD_LINK_VIRTUALIZATION_BACKOFF_SECONDS);
                link_virtualization_backoff_until_unix.insert(node_name.to_string(), until);
                status.warnings.push(format!(
                    "TreeGuard links: failed to submit runtime {} for node '{node_name}': {e}. Backing off until {until}."
                    ,
                    if target == LinkVirtualState::Virtual {
                        "virtualization"
                    } else {
                        "restore"
                    }
                ));
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "node".to_string(),
                        entity_id: node_name.to_string(),
                        action: match target {
                            LinkVirtualState::Physical => "unvirtualize_failed".to_string(),
                            LinkVirtualState::Virtual => "virtualize_failed".to_string(),
                        },
                        persisted: false,
                        reason: format!("Bakery runtime virtualization submission failed: {e}"),
                        ..Default::default()
                    },
                );
                return;
            }
        }
    }

    state.desired = target;

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
            persisted: persist,
            reason,
            ..Default::default()
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

/// Looks up an SQM override token for a circuit from a specific overrides file.
///
/// This function is pure: it has no side effects.
fn find_circuit_override_token_in_overrides(
    devices: &[lqos_config::ShapedDevice],
    overrides: &OverrideFile,
) -> Option<String> {
    for device_id in devices.iter().map(|device| device.device_id.as_str()) {
        if let Some(token) = overrides_device_sqm(overrides, device_id) {
            return Some(token);
        }
    }

    None
}

/// Infers an SQM override token for a circuit, preferring persisted override entries.
///
/// This function is pure: it has no side effects.
fn infer_circuit_sqm_override_token(
    devices: &[lqos_config::ShapedDevice],
    overrides: Option<&OverrideFile>,
) -> Option<String> {
    if let Some(overrides) = overrides {
        for device_id in devices.iter().map(|device| device.device_id.as_str()) {
            if let Some(token) = overrides_device_sqm(overrides, device_id) {
                return Some(token);
            }
        }
    }

    for dev in devices {
        if let Some(token) = dev.sqm_override.as_deref() {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    None
}

/// Returns the default effective SQM state for a direction at the given configured rate.
///
/// This function is pure: it has no side effects.
fn default_sqm_state_for_rate(rate_mbps: f32, config: &lqos_config::Config) -> CircuitSqmState {
    let default_sqm = config.queues.default_sqm.trim().to_ascii_lowercase();
    if default_sqm.starts_with("cake") {
        let threshold = config.queues.fast_queues_fq_codel.unwrap_or(1000.0) as f32;
        if rate_mbps >= threshold {
            CircuitSqmState::FqCodel
        } else {
            CircuitSqmState::Cake
        }
    } else {
        CircuitSqmState::FqCodel
    }
}

/// Computes the base per-direction SQM state for a circuit before TreeGuard overlays are applied.
///
/// This function is pure: it has no side effects.
fn base_circuit_sqm_state(
    devices: &[lqos_config::ShapedDevice],
    operator_overrides: Option<&OverrideFile>,
    config: &lqos_config::Config,
    cap_down: f32,
    cap_up: f32,
) -> DownUpOrder<CircuitSqmState> {
    let mut down = default_sqm_state_for_rate(cap_down, config);
    let mut up = default_sqm_state_for_rate(cap_up, config);

    if let Some(token) = infer_circuit_sqm_override_token(devices, operator_overrides) {
        let parsed = decisions::parse_directional_sqm_override(&token);
        if let Some(v) = parsed.down {
            down = v;
        }
        if let Some(v) = parsed.up {
            up = v;
        }
    }

    DownUpOrder { down, up }
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
        debug!(
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
    base_sqm: DownUpOrder<CircuitSqmState>,
    batch_id: &'a str,
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
    dry_run: bool,
    circuit_id: &'a str,
    circuit_entity_id: &'a str,
    circuit_label: &'a str,
    devices: &'a [lqos_config::ShapedDevice],
    sqm_batch_id: &'a str,
    allowlisted: bool,
    cap_down: f32,
    cap_up: f32,
    bps: DownUpOrder<u64>,
    last_rtt_seen_nanos: Option<u64>,
    qoo: DownUpOrder<Option<f32>>,
    cpu_cfg: &'a lqos_config::TreeguardCpuConfig,
    circuits_cfg: &'a lqos_config::TreeguardCircuitsConfig,
    qoo_cfg: &'a lqos_config::TreeguardQooConfig,
    base_sqm: DownUpOrder<CircuitSqmState>,
    circuit_change_budget_remaining: &'a mut usize,
    deferred_circuit_sqm_changes: &'a mut usize,
}

fn treeguard_manages_circuit_direction(base_sqm: CircuitSqmState) -> bool {
    matches!(base_sqm, CircuitSqmState::Cake)
}

fn circuit_sqm_transition_from_decision(
    state: &CircuitState,
    base_sqm: DownUpOrder<CircuitSqmState>,
    decision: decisions::CircuitSqmDecision,
) -> CircuitSqmTransition {
    let mut proposed_down = state.down.desired;
    let mut proposed_up = state.up.desired;

    if treeguard_manages_circuit_direction(base_sqm.down) {
        if let Some(down) = decision.down {
            proposed_down = down;
        }
    } else {
        proposed_down = base_sqm.down;
    }

    if treeguard_manages_circuit_direction(base_sqm.up) {
        if let Some(up) = decision.up {
            proposed_up = up;
        }
    } else {
        proposed_up = base_sqm.up;
    }

    CircuitSqmTransition {
        proposed_down,
        proposed_up,
        changed_down: proposed_down != state.down.desired,
        changed_up: proposed_up != state.up.desired,
    }
}

fn try_consume_circuit_change_budget(remaining_budget: &mut usize) -> bool {
    if *remaining_budget == 0 {
        return false;
    }
    *remaining_budget -= 1;
    true
}

fn next_sqm_batch_id(next_sqm_batch_id: &mut u64) -> String {
    *next_sqm_batch_id = next_sqm_batch_id.saturating_add(1);
    format!("sqm-{}", *next_sqm_batch_id)
}

fn is_retryable_live_mutation_unavailable(error: &TreeguardError) -> Option<&str> {
    match error {
        TreeguardError::LiveMutationUnavailable { details } => Some(details.as_str()),
        _ => None,
    }
}

fn apply_circuit_sqm_change<P, C, L>(
    ctx: CircuitSqmApplyContext<'_>,
    state: &mut CircuitState,
    transition: CircuitSqmTransition,
    mut persist_override: P,
    mut clear_override: C,
    mut live_apply: L,
) where
    P: FnMut(&[String], &str) -> Result<bool, TreeguardError>,
    C: FnMut(&[String]) -> Result<bool, TreeguardError>,
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
        base_sqm,
        batch_id,
    } = ctx;
    let CircuitSqmTransition {
        proposed_down,
        proposed_up,
        changed_down,
        changed_up,
    } = transition;

    let token = decisions::format_directional_sqm_override(proposed_down, proposed_up);
    let returning_to_base = proposed_down == base_sqm.down && proposed_up == base_sqm.up;
    let live_token = if returning_to_base {
        "/"
    } else {
        token.as_str()
    };

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
                action: if returning_to_base {
                    "would_clear_sqm_override".to_string()
                } else {
                    format!("would_set_sqm_override:{token}")
                },
                persisted: false,
                reason: "Dry-run".to_string(),
                batch_id: Some(batch_id.to_string()),
                batch_kind: Some("sqm".to_string()),
            },
        );
        status.last_action_summary = Some(if returning_to_base {
            format!(
                "Would clear TreeGuard SQM override for circuit '{}' (base {})",
                circuit_label, token
            )
        } else {
            format!(
                "Would set SQM override for circuit '{}' -> {}",
                circuit_label, token
            )
        });
        return;
    }

    let mut persisted_ok = false;
    let device_ids: Vec<String> = devices
        .iter()
        .map(|device| device.device_id.clone())
        .collect();
    if persist_sqm_overrides {
        let persist_result = if returning_to_base {
            clear_override(&device_ids)
        } else {
            persist_override(&device_ids, &token)
        };
        match persist_result {
            Ok(_) => {
                persisted_ok = true;
            }
            Err(e) => {
                status.warnings.push(format!(
                    "TreeGuard circuits: failed to {} SQM overrides for circuit '{circuit_id}': {e}",
                    if returning_to_base { "clear" } else { "persist" }
                ));
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "circuit".to_string(),
                        entity_id: circuit_entity_id.to_string(),
                        action: if returning_to_base {
                            "clear_sqm_override_failed".to_string()
                        } else {
                            "set_sqm_override_failed".to_string()
                        },
                        persisted: false,
                        reason: format!("Overrides write failed: {e}"),
                        batch_id: Some(batch_id.to_string()),
                        batch_kind: Some("sqm".to_string()),
                    },
                );
            }
        }
    }

    let mut live_deferred = false;
    let live_ok = match live_apply(circuit_id, devices, live_token) {
        Ok(()) => true,
        Err(e) => {
            if let Some(details) = is_retryable_live_mutation_unavailable(&e) {
                live_deferred = true;
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "circuit".to_string(),
                        entity_id: circuit_entity_id.to_string(),
                        action: if returning_to_base {
                            "clear_sqm_live_deferred".to_string()
                        } else {
                            format!("apply_sqm_live_deferred:{token}")
                        },
                        persisted: persisted_ok,
                        reason: format!("Bakery live apply deferred: {details}"),
                        batch_id: Some(batch_id.to_string()),
                        batch_kind: Some("sqm".to_string()),
                    },
                );
            } else {
                status.warnings.push(format!(
                    "TreeGuard circuits: live SQM apply failed for circuit '{circuit_id}': {e}"
                ));
                push_activity(
                    activity,
                    TreeguardActivityEntry {
                        time: now_unix.to_string(),
                        entity_type: "circuit".to_string(),
                        entity_id: circuit_entity_id.to_string(),
                        action: if returning_to_base {
                            "clear_sqm_live_failed".to_string()
                        } else {
                            format!("apply_sqm_live_failed:{token}")
                        },
                        persisted: persisted_ok,
                        reason: format!("Bakery live apply failed: {e}"),
                        batch_id: Some(batch_id.to_string()),
                        batch_kind: Some("sqm".to_string()),
                    },
                );
            }
            false
        }
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

        let (action, reason) = match (returning_to_base, persisted_ok, live_ok) {
            (false, true, true) => (
                "set_sqm_override".to_string(),
                "Applied live + persisted".to_string(),
            ),
            (false, true, false) => (
                "set_sqm_override".to_string(),
                if live_deferred {
                    "Persisted (live apply deferred)".to_string()
                } else {
                    "Persisted (live apply failed)".to_string()
                },
            ),
            (false, false, true) => ("set_sqm_live".to_string(), "Applied live".to_string()),
            (false, false, false) => ("set_sqm_live".to_string(), "Not applied".to_string()),
            (true, true, true) => (
                "clear_sqm_override".to_string(),
                "Cleared live + persisted overlay".to_string(),
            ),
            (true, true, false) => (
                "clear_sqm_override".to_string(),
                if live_deferred {
                    "Persisted clear (live apply deferred)".to_string()
                } else {
                    "Persisted clear (live apply failed)".to_string()
                },
            ),
            (true, false, true) => (
                "clear_sqm_live".to_string(),
                "Applied live clear".to_string(),
            ),
            (true, false, false) => ("clear_sqm_live".to_string(), "Not applied".to_string()),
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
                batch_id: Some(batch_id.to_string()),
                batch_kind: Some("sqm".to_string()),
            },
        );

        status.last_action_summary = Some(if returning_to_base {
            format!(
                "Cleared TreeGuard SQM override for circuit '{}' (base {})",
                circuit_label, token
            )
        } else {
            format!("SQM override for circuit '{}' -> {}", circuit_label, token)
        });
    }
}

fn process_circuit_tick<P, C, L>(
    ctx: CircuitTickContext<'_>,
    state: &mut CircuitState,
    persist_override: P,
    clear_override: C,
    live_apply: L,
) -> bool
where
    P: FnMut(&[String], &str) -> Result<bool, TreeguardError>,
    C: FnMut(&[String]) -> Result<bool, TreeguardError>,
    L: FnMut(&str, &[lqos_config::ShapedDevice], &str) -> Result<(), TreeguardError>,
{
    let CircuitTickContext {
        status,
        activity,
        managed_device_ids,
        now_unix,
        now_nanos_since_boot,
        cpu_max_pct,
        dry_run,
        circuit_id,
        circuit_entity_id,
        circuit_label,
        devices,
        sqm_batch_id,
        allowlisted,
        cap_down,
        cap_up,
        bps,
        last_rtt_seen_nanos,
        qoo,
        cpu_cfg,
        circuits_cfg,
        qoo_cfg,
        base_sqm,
        circuit_change_budget_remaining,
        deferred_circuit_sqm_changes,
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
    let transition = circuit_sqm_transition_from_decision(state, base_sqm, decision);

    if devices.is_empty() {
        status.warnings.push(format!(
            "TreeGuard circuits: circuit '{circuit_id}' has no devices in ShapedDevices.csv."
        ));
    } else {
        for dev in devices {
            managed_device_ids.insert(dev.device_id.clone());
        }
    }

    if (transition.changed_down || transition.changed_up) && !devices.is_empty() {
        if try_consume_circuit_change_budget(circuit_change_budget_remaining) {
            apply_circuit_sqm_change(
                CircuitSqmApplyContext {
                    status,
                    activity,
                    now_unix,
                    dry_run,
                    persist_sqm_overrides: circuits_cfg.persist_sqm_overrides,
                    circuit_id,
                    circuit_entity_id,
                    circuit_label,
                    devices,
                    base_sqm,
                    batch_id: sqm_batch_id,
                },
                state,
                transition,
                persist_override,
                clear_override,
                live_apply,
            );
        } else {
            *deferred_circuit_sqm_changes = deferred_circuit_sqm_changes.saturating_add(1);
        }
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

/// Updates an "above threshold since" timestamp based on utilization and a threshold.
fn update_above_since(
    above_since: &mut Option<u64>,
    now_unix: u64,
    util_pct: f64,
    threshold_pct: f64,
) {
    if util_pct >= threshold_pct {
        if above_since.is_none() {
            *above_since = Some(now_unix);
        }
    } else {
        *above_since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CircuitSqmApplyContext, CircuitSqmTransition, CircuitTickContext, LinkVirtualState,
        PendingLinkVirtualizationDecision, TreeguardRuntimeState, apply_circuit_sqm_change,
        apply_link_virtualization_decision, base_circuit_sqm_state, circuit_evaluation_batch_size,
        circuit_sqm_transition_from_decision, clear_structural_ineligible_if_topology_changed,
        collect_circuit_batch, empty_status_snapshot, latched_structural_ineligible_reason,
        pause_for_bakery_reload_with_flag, process_circuit_tick, run_tick,
        select_link_virtualization_candidates, treeguard_manages_circuit_direction,
        try_consume_circuit_change_budget,
    };
    use crate::node_manager::ws::messages::TreeguardActivityEntry;
    use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};
    use crate::system_stats::SystemStats;
    use crate::throughput_tracker::CIRCUIT_RTT_BUFFERS;
    use crate::treeguard::decisions;
    use crate::treeguard::{
        bakery,
        errors::TreeguardError,
        state::{
            CircuitSqmState, LinkState, LinkStructuralIneligibleState, LinkTopologyFingerprint,
        },
    };
    use crossbeam_channel::bounded;
    use fxhash::{FxHashMap, FxHashSet};
    use lqos_bakery::{BakeryCommands, BakeryRuntimeNodeOperationFailureReason};
    use lqos_bus::TcHandle;
    use lqos_config::ConfigShapedDevices;
    use lqos_config::ShapedDevice;
    use lqos_queue_tracker::{QUEUE_STRUCTURE, QueueNode, QueueStructure};
    use lqos_utils::rtt::RttBuffer;
    use lqos_utils::units::DownUpOrder;
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::net::Ipv4Addr;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::{Once, OnceLock};

    #[allow(clippy::too_many_arguments)]
    fn pending_link_decision(
        node_name: &str,
        node_index: usize,
        target: LinkVirtualState,
        subtree_nodes: usize,
        current_subtree_throughput_mbps: f64,
        explicit_allowlist: bool,
        is_top_level: bool,
        value_score: u64,
    ) -> PendingLinkVirtualizationDecision {
        PendingLinkVirtualizationDecision {
            node_name: node_name.to_string(),
            node_index,
            target,
            reason: "test".to_string(),
            subtree_nodes,
            current_subtree_throughput_mbps,
            explicit_allowlist,
            is_top_level,
            value_score,
        }
    }

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
            None,
        );

        assert!(paused);
        assert_eq!(tick_seconds, 5);
        assert!(status.enabled);
        assert!(!status.dry_run);
        assert_eq!(
            status.last_action_summary.as_deref(),
            Some("Paused while Bakery full reload in progress")
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
            None,
        );
        assert!(!resumed);
        assert!(!runtime_state.paused_for_bakery_reload);
        assert!(status.last_action_summary.is_none());
    }

    #[test]
    fn bakery_reload_required_pause_sets_reason() {
        let mut status = empty_status_snapshot();
        let mut tick_seconds = 1;
        let mut runtime_state = TreeguardRuntimeState::default();

        let paused = pause_for_bakery_reload_with_flag(
            &mut status,
            &mut tick_seconds,
            &mut runtime_state,
            true,
            false,
            false,
            Some("Bakery requires full reload".to_string()),
        );

        assert!(paused);
        assert_eq!(
            status.pause_reason.as_deref(),
            Some("Bakery requires full reload")
        );
        assert_eq!(
            status.last_action_summary.as_deref(),
            Some("Paused while Bakery requires full reload")
        );
    }

    #[test]
    fn link_candidate_selection_prefers_larger_ancestor_and_budget() {
        let parent_by_index = vec![None, Some(0), Some(1), Some(1), Some(0)];
        let existing_virtualized = FxHashSet::default();
        let candidates = vec![
            pending_link_decision(
                "region",
                0,
                LinkVirtualState::Virtual,
                5,
                10.0,
                false,
                true,
                500,
            ),
            pending_link_decision(
                "pop-a",
                1,
                LinkVirtualState::Virtual,
                3,
                10.0,
                false,
                false,
                300,
            ),
            pending_link_decision(
                "ap-a1",
                2,
                LinkVirtualState::Virtual,
                1,
                10.0,
                false,
                false,
                100,
            ),
            pending_link_decision(
                "ap-a2",
                3,
                LinkVirtualState::Virtual,
                1,
                10.0,
                false,
                false,
                90,
            ),
            pending_link_decision(
                "pop-b",
                4,
                LinkVirtualState::Virtual,
                1,
                10.0,
                false,
                false,
                80,
            ),
        ];

        let (selected, deferred, skipped_low_value) = select_link_virtualization_candidates(
            candidates,
            &parent_by_index,
            &existing_virtualized,
            0,
        );

        let selected_names: Vec<&str> = selected
            .iter()
            .map(|candidate| candidate.node_name.as_str())
            .collect();
        assert_eq!(selected_names, vec!["region"]);
        assert_eq!(deferred, 4);
        assert_eq!(skipped_low_value, 0);
    }

    #[test]
    fn link_candidate_selection_allows_small_explicit_allowlist_node() {
        let parent_by_index = vec![None];
        let existing_virtualized = FxHashSet::default();
        let candidates = vec![pending_link_decision(
            "small-explicit",
            0,
            LinkVirtualState::Virtual,
            1,
            0.2,
            true,
            false,
            1,
        )];

        let (selected, deferred, skipped_low_value) = select_link_virtualization_candidates(
            candidates,
            &parent_by_index,
            &existing_virtualized,
            0,
        );

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].node_name, "small-explicit");
        assert_eq!(deferred, 0);
        assert_eq!(skipped_low_value, 0);
    }

    #[test]
    fn link_candidate_selection_prioritizes_restore_before_new_virtualization() {
        let parent_by_index = vec![None, None];
        let existing_virtualized = FxHashSet::default();
        let candidates = vec![
            pending_link_decision(
                "virtualize-me",
                0,
                LinkVirtualState::Virtual,
                10,
                10.0,
                false,
                true,
                1_000,
            ),
            pending_link_decision(
                "restore-me",
                1,
                LinkVirtualState::Physical,
                1,
                0.2,
                false,
                false,
                1,
            ),
        ];

        let (selected, _deferred, skipped_low_value) = select_link_virtualization_candidates(
            candidates,
            &parent_by_index,
            &existing_virtualized,
            0,
        );

        assert_eq!(
            selected
                .first()
                .map(|candidate| candidate.node_name.as_str()),
            Some("restore-me")
        );
        assert_eq!(skipped_low_value, 0);
    }

    #[test]
    fn link_candidate_selection_skips_small_low_throughput_automatic_nodes() {
        let parent_by_index = vec![None];
        let existing_virtualized = FxHashSet::default();
        let candidates = vec![pending_link_decision(
            "small-auto",
            0,
            LinkVirtualState::Virtual,
            8,
            0.4,
            false,
            false,
            10,
        )];

        let (selected, deferred, skipped_low_value) = select_link_virtualization_candidates(
            candidates,
            &parent_by_index,
            &existing_virtualized,
            0,
        );

        assert!(selected.is_empty());
        assert_eq!(deferred, 0);
        assert_eq!(skipped_low_value, 1);
    }

    #[test]
    fn link_candidate_selection_allows_large_low_throughput_automatic_nodes() {
        let parent_by_index = vec![None];
        let existing_virtualized = FxHashSet::default();
        let candidates = vec![pending_link_decision(
            "large-auto",
            0,
            LinkVirtualState::Virtual,
            16,
            0.4,
            false,
            false,
            10,
        )];

        let (selected, deferred, skipped_low_value) = select_link_virtualization_candidates(
            candidates,
            &parent_by_index,
            &existing_virtualized,
            0,
        );

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].node_name, "large-auto");
        assert_eq!(deferred, 0);
        assert_eq!(skipped_low_value, 0);
    }

    #[test]
    fn structural_ineligible_latch_blocks_virtualize_until_topology_changes() {
        let mut state = LinkState {
            topology_fingerprint: LinkTopologyFingerprint {
                direct_child_sites: 0,
                direct_circuits: 0,
            },
            structural_ineligible: Some(LinkStructuralIneligibleState {
                reason:
                    BakeryRuntimeNodeOperationFailureReason::StructuralIneligibleNoPromotableChildren,
                topology_fingerprint: LinkTopologyFingerprint {
                    direct_child_sites: 0,
                    direct_circuits: 0,
                },
            }),
            ..Default::default()
        };

        assert_eq!(
            latched_structural_ineligible_reason(&state, LinkVirtualState::Virtual),
            Some(BakeryRuntimeNodeOperationFailureReason::StructuralIneligibleNoPromotableChildren)
        );
        assert_eq!(
            latched_structural_ineligible_reason(&state, LinkVirtualState::Physical),
            None
        );

        let changed = clear_structural_ineligible_if_topology_changed(
            &mut state,
            LinkTopologyFingerprint {
                direct_child_sites: 1,
                direct_circuits: 0,
            },
        );
        state.topology_fingerprint = LinkTopologyFingerprint {
            direct_child_sites: 1,
            direct_circuits: 0,
        };

        assert!(changed);
        assert!(state.structural_ineligible.is_none());
        assert_eq!(
            latched_structural_ineligible_reason(&state, LinkVirtualState::Virtual),
            None
        );
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
                base_sqm: DownUpOrder {
                    down: CircuitSqmState::Cake,
                    up: CircuitSqmState::Cake,
                },
                batch_id: "sqm-test-1",
            },
            &mut state,
            CircuitSqmTransition {
                proposed_down: CircuitSqmState::FqCodel,
                proposed_up: CircuitSqmState::Cake,
                changed_down: true,
                changed_up: false,
            },
            |_device_ids, _token| Ok(false),
            |_device_ids| Ok(false),
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
        assert_eq!(last_activity.batch_id.as_deref(), Some("sqm-test-1"));
        assert_eq!(last_activity.batch_kind.as_deref(), Some("sqm"));

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
    fn actor_return_to_base_clears_treeguard_overlay_and_uses_live_clear_token() {
        let devices = vec![ShapedDevice {
            circuit_id: "circuit-1".to_string(),
            circuit_name: "Circuit One".to_string(),
            device_id: "device-1".to_string(),
            ..ShapedDevice::default()
        }];
        let mut status = empty_status_snapshot();
        let mut activity: VecDeque<TreeguardActivityEntry> = VecDeque::new();
        let mut state = crate::treeguard::state::CircuitState::default();
        state.down.desired = CircuitSqmState::FqCodel;
        state.up.desired = CircuitSqmState::Cake;
        let mut cleared_device_ids: Vec<String> = Vec::new();
        let mut live_token: Option<String> = None;

        apply_circuit_sqm_change(
            CircuitSqmApplyContext {
                status: &mut status,
                activity: &mut activity,
                now_unix: 1_000,
                dry_run: false,
                persist_sqm_overrides: true,
                circuit_id: "circuit-1",
                circuit_entity_id: "Circuit One (circuit-1)",
                circuit_label: "Circuit One",
                devices: &devices,
                base_sqm: DownUpOrder {
                    down: CircuitSqmState::Cake,
                    up: CircuitSqmState::Cake,
                },
                batch_id: "sqm-test-2",
            },
            &mut state,
            CircuitSqmTransition {
                proposed_down: CircuitSqmState::Cake,
                proposed_up: CircuitSqmState::Cake,
                changed_down: true,
                changed_up: false,
            },
            |_device_ids, _token| Ok(false),
            |device_ids| {
                cleared_device_ids = device_ids.to_vec();
                Ok(true)
            },
            |_circuit_id, _devices, token| {
                live_token = Some(token.to_string());
                Ok(())
            },
        );

        assert_eq!(state.down.desired, CircuitSqmState::Cake);
        assert_eq!(state.up.desired, CircuitSqmState::Cake);
        assert_eq!(cleared_device_ids, vec!["device-1".to_string()]);
        assert_eq!(live_token.as_deref(), Some("/"));
        assert_eq!(
            status.last_action_summary.as_deref(),
            Some("Cleared TreeGuard SQM override for circuit 'Circuit One' (base cake/cake)")
        );

        let last_activity = activity.back().expect("activity should be recorded");
        assert_eq!(last_activity.action, "clear_sqm_override:cake/cake");
        assert!(last_activity.persisted);
        assert_eq!(last_activity.reason, "Cleared live + persisted overlay");
        assert_eq!(last_activity.batch_id.as_deref(), Some("sqm-test-2"));
        assert_eq!(last_activity.batch_kind.as_deref(), Some("sqm"));
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

        let circuits_cfg = lqos_config::TreeguardCircuitsConfig {
            persist_sqm_overrides: false,
            ..lqos_config::TreeguardCircuitsConfig::default()
        };
        let mut circuit_change_budget_remaining = 1usize;
        let mut deferred_circuit_sqm_changes = 0usize;

        let fq_codel = process_circuit_tick(
            CircuitTickContext {
                status: &mut status,
                activity: &mut activity,
                managed_device_ids: &mut managed_device_ids,
                now_unix: 1_000,
                now_nanos_since_boot: Some(2_000_000_000),
                cpu_max_pct: Some(95),
                dry_run: false,
                circuit_id: "circuit-2",
                circuit_entity_id: "Circuit Two (circuit-2)",
                circuit_label: "Circuit Two",
                devices: &devices,
                sqm_batch_id: "sqm-test-3",
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
                base_sqm: DownUpOrder {
                    down: CircuitSqmState::Cake,
                    up: CircuitSqmState::Cake,
                },
                circuit_change_budget_remaining: &mut circuit_change_budget_remaining,
                deferred_circuit_sqm_changes: &mut deferred_circuit_sqm_changes,
            },
            &mut state,
            |_device_ids, _token| Ok(false),
            |_device_ids| Ok(false),
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
        let last_activity = activity.back().expect("activity should be recorded");
        assert_eq!(last_activity.batch_id.as_deref(), Some("sqm-test-3"));
        assert_eq!(last_activity.batch_kind.as_deref(), Some("sqm"));
    }

    #[test]
    fn base_circuit_sqm_state_uses_default_sqm_when_no_operator_override_exists() {
        let mut config = lqos_config::Config::default();
        config.queues.default_sqm = "fq_codel".to_string();
        config.queues.fast_queues_fq_codel = Some(1000.0);

        let shaped_devices = vec![ShapedDevice {
            circuit_id: "circuit-3".to_string(),
            device_id: "device-3".to_string(),
            ..ShapedDevice::default()
        }];

        let base = base_circuit_sqm_state(&shaped_devices, None, &config, 50.0, 20.0);

        assert_eq!(base.down, CircuitSqmState::FqCodel);
        assert_eq!(base.up, CircuitSqmState::FqCodel);
    }

    #[test]
    fn circuit_tick_treats_bakery_live_mutation_unavailable_as_deferred() {
        let devices = vec![ShapedDevice {
            circuit_id: "circuit-deferred".to_string(),
            circuit_name: "Circuit Deferred".to_string(),
            device_id: "device-deferred".to_string(),
            ipv4: vec![(Ipv4Addr::new(198, 51, 100, 30), 32)],
            ..ShapedDevice::default()
        }];
        let mut managed_device_ids = FxHashSet::default();
        let mut status = empty_status_snapshot();
        let mut activity: VecDeque<TreeguardActivityEntry> = VecDeque::new();
        let mut state = crate::treeguard::state::CircuitState::default();
        state.down.idle_since_unix = Some(0);
        state.up.idle_since_unix = Some(0);

        let circuits_cfg = lqos_config::TreeguardCircuitsConfig {
            persist_sqm_overrides: false,
            ..lqos_config::TreeguardCircuitsConfig::default()
        };
        let mut circuit_change_budget_remaining = 1usize;
        let mut deferred_circuit_sqm_changes = 0usize;

        let fq_codel = process_circuit_tick(
            CircuitTickContext {
                status: &mut status,
                activity: &mut activity,
                managed_device_ids: &mut managed_device_ids,
                now_unix: 1_000,
                now_nanos_since_boot: Some(2_000_000_000),
                cpu_max_pct: Some(95),
                dry_run: false,
                circuit_id: "circuit-deferred",
                circuit_entity_id: "Circuit Deferred (circuit-deferred)",
                circuit_label: "Circuit Deferred",
                devices: &devices,
                sqm_batch_id: "sqm-test-deferred",
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
                base_sqm: DownUpOrder {
                    down: CircuitSqmState::Cake,
                    up: CircuitSqmState::Cake,
                },
                circuit_change_budget_remaining: &mut circuit_change_budget_remaining,
                deferred_circuit_sqm_changes: &mut deferred_circuit_sqm_changes,
            },
            &mut state,
            |_device_ids, _token| Ok(false),
            |_device_ids| Ok(false),
            |_circuit_id, _devices, _token| {
                Err(TreeguardError::LiveMutationUnavailable {
                    details: "the shaping tree is not currently active".to_string(),
                })
            },
        );

        assert!(!fq_codel);
        assert_eq!(state.down.desired, CircuitSqmState::Cake);
        assert_eq!(state.up.desired, CircuitSqmState::Cake);
        assert!(status.warnings.is_empty());
        assert!(managed_device_ids.contains("device-deferred"));
        assert!(activity.iter().any(|entry| {
            entry.action == "apply_sqm_live_deferred:fq_codel/fq_codel"
                && entry.reason
                    == "Bakery live apply deferred: the shaping tree is not currently active"
        }));
        assert!(!activity.iter().any(|entry| entry.action.contains("failed")));
    }

    #[test]
    fn base_fq_codel_directions_do_not_take_treeguard_circuit_switches() {
        let mut state = crate::treeguard::state::CircuitState::default();
        state.down.desired = CircuitSqmState::Cake;
        state.up.desired = CircuitSqmState::FqCodel;

        let transition = circuit_sqm_transition_from_decision(
            &state,
            DownUpOrder {
                down: CircuitSqmState::FqCodel,
                up: CircuitSqmState::FqCodel,
            },
            decisions::CircuitSqmDecision {
                down: Some(CircuitSqmState::FqCodel),
                up: Some(CircuitSqmState::Cake),
            },
        );

        assert_eq!(transition.proposed_down, CircuitSqmState::FqCodel);
        assert_eq!(transition.proposed_up, CircuitSqmState::FqCodel);
        assert!(transition.changed_down);
        assert!(!transition.changed_up);
    }

    #[test]
    fn treeguard_circuit_change_budget_consumes_then_stops() {
        let mut remaining = 2usize;
        assert!(try_consume_circuit_change_budget(&mut remaining));
        assert_eq!(remaining, 1);
        assert!(try_consume_circuit_change_budget(&mut remaining));
        assert_eq!(remaining, 0);
        assert!(!try_consume_circuit_change_budget(&mut remaining));
        assert_eq!(remaining, 0);
    }

    #[test]
    fn treeguard_only_manages_cake_based_circuit_directions() {
        assert!(treeguard_manages_circuit_direction(CircuitSqmState::Cake));
        assert!(!treeguard_manages_circuit_direction(
            CircuitSqmState::FqCodel
        ));
    }

    #[test]
    fn treeguard_circuit_batch_size_spreads_large_all_circuit_sweeps() {
        assert_eq!(circuit_evaluation_batch_size(0, true), 0);
        assert_eq!(circuit_evaluation_batch_size(64, true), 64);
        assert_eq!(circuit_evaluation_batch_size(10_000, true), 667);
        assert_eq!(circuit_evaluation_batch_size(20_000, true), 1334);
        assert_eq!(circuit_evaluation_batch_size(32, false), 32);
    }

    #[test]
    fn treeguard_collect_circuit_batch_wraps_cursor() {
        let circuits = vec![
            "c1".to_string(),
            "c2".to_string(),
            "c3".to_string(),
            "c4".to_string(),
        ];
        let mut cursor = 3usize;

        let batch = collect_circuit_batch(&circuits, &mut cursor, 3);

        assert_eq!(batch, vec!["c4", "c1", "c2"]);
        assert_eq!(cursor, 2);
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

    struct LiveBakeryTestContext {
        _guard: std::sync::MutexGuard<'static, ()>,
        old_lqos_config: Option<OsString>,
        old_lqos_directory: Option<OsString>,
        old_shaping_tree_active: bool,
    }

    impl LiveBakeryTestContext {
        fn new(name: &str) -> Self {
            let guard = crate::test_support::runtime_config_test_lock()
                .lock()
                .expect("treeguard test lock should not be poisoned");
            let runtime_dir = test_runtime_dir(name);
            let config_path = write_test_config(&runtime_dir);
            let old_lqos_config = std::env::var_os("LQOS_CONFIG");
            let old_lqos_directory = std::env::var_os("LQOS_DIRECTORY");
            unsafe {
                std::env::set_var("LQOS_CONFIG", &config_path);
                std::env::set_var("LQOS_DIRECTORY", &runtime_dir);
            }
            lqos_config::clear_cached_config();
            let old_shaping_tree_active = lqos_bakery::set_shaping_tree_active_for_tests(true);
            Self {
                _guard: guard,
                old_lqos_config,
                old_lqos_directory,
                old_shaping_tree_active,
            }
        }
    }

    impl Drop for LiveBakeryTestContext {
        fn drop(&mut self) {
            match &self.old_lqos_config {
                Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
                None => unsafe { std::env::remove_var("LQOS_CONFIG") },
            }
            match &self.old_lqos_directory {
                Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
                None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
            }
            lqos_config::clear_cached_config();
            lqos_bakery::set_shaping_tree_active_for_tests(self.old_shaping_tree_active);
        }
    }

    fn install_test_bakery_sender() -> crossbeam_channel::Receiver<BakeryCommands> {
        static INIT: Once = Once::new();
        static RECEIVER: OnceLock<crossbeam_channel::Receiver<BakeryCommands>> = OnceLock::new();
        INIT.call_once(|| {
            let (tx, rx) = bounded(8);
            let _ = lqos_bakery::BAKERY_SENDER.set(tx);
            let _ = RECEIVER.set(rx);
        });
        RECEIVER
            .get()
            .expect("test bakery receiver should be installed")
            .clone()
    }

    #[test]
    fn run_tick_end_to_end_switches_circuit_and_emits_bakery_update() {
        let _live_bakery = LiveBakeryTestContext::new("run-tick");

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

        assert!(status.enabled);
        assert!(!status.dry_run);
        assert_eq!(status.managed_circuits, 1);
        assert_eq!(status.fq_codel_circuits, 1);
        assert_eq!(status.cake_circuits, 0);
        assert_eq!(status.mixed_sqm_circuits, 0);
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
    }

    #[test]
    fn apply_link_virtualization_decision_emits_requested_activity_when_bakery_accepts_submit() {
        let _live_bakery = LiveBakeryTestContext::new("link-virtualize");
        let rx = install_test_bakery_sender();
        while rx.try_recv().is_ok() {}

        let mut status = empty_status_snapshot();
        let mut activity: VecDeque<TreeguardActivityEntry> = VecDeque::new();
        let mut pending_link_operations = FxHashMap::default();
        let mut link_virtualization_backoff_until_unix = FxHashMap::default();
        let mut state = LinkState::default();

        apply_link_virtualization_decision(
            &mut status,
            &mut activity,
            1_000,
            "Node Requested",
            LinkVirtualState::Virtual,
            false,
            "High utilization".to_string(),
            false,
            &mut state,
            &mut pending_link_operations,
            &mut link_virtualization_backoff_until_unix,
        );

        assert_eq!(state.desired, LinkVirtualState::Virtual);
        let pending = pending_link_operations
            .get("Node Requested")
            .expect("pending operation should be tracked");
        assert_eq!(pending.target, LinkVirtualState::Virtual);
        assert_eq!(pending.reason, "High utilization");
        assert_eq!(
            status.last_action_summary.as_deref(),
            Some("Queued virtualization for node 'Node Requested'")
        );

        let last_activity = activity
            .back()
            .expect("requested activity should be recorded");
        assert_eq!(last_activity.action, "virtualize_requested");
        assert!(!last_activity.persisted);
        assert!(last_activity.reason.contains("High utilization"));
        assert!(last_activity.reason.contains("Queued in Bakery"));

        let command = rx.try_recv().expect("bakery command should be sent");
        let BakeryCommands::TreeGuardSetNodeVirtual {
            site_hash,
            virtualized,
            ..
        } = command
        else {
            panic!("expected TreeGuardSetNodeVirtual");
        };
        assert_eq!(site_hash, lqos_utils::hash_to_i64("Node Requested"));
        assert!(virtualized);
    }

    #[test]
    fn apply_link_virtualization_decision_rejects_structurally_ineligible_top_level_node() {
        let rx = install_test_bakery_sender();
        while rx.try_recv().is_ok() {}

        let mut status = empty_status_snapshot();
        let mut activity: VecDeque<TreeguardActivityEntry> = VecDeque::new();
        let mut pending_link_operations = FxHashMap::default();
        let mut link_virtualization_backoff_until_unix = FxHashMap::default();
        let mut state = LinkState {
            topology_fingerprint: LinkTopologyFingerprint {
                direct_child_sites: 1,
                direct_circuits: 0,
            },
            ..Default::default()
        };

        apply_link_virtualization_decision(
            &mut status,
            &mut activity,
            1_000,
            "Node Rejected",
            LinkVirtualState::Virtual,
            true,
            "Top-level safe".to_string(),
            false,
            &mut state,
            &mut pending_link_operations,
            &mut link_virtualization_backoff_until_unix,
        );

        assert_eq!(state.desired, LinkVirtualState::Physical);
        assert!(pending_link_operations.is_empty());
        assert!(link_virtualization_backoff_until_unix.is_empty());
        assert_eq!(
            state.structural_ineligible,
            Some(LinkStructuralIneligibleState {
                reason:
                    BakeryRuntimeNodeOperationFailureReason::StructuralIneligibleSinglePromotableChild,
                topology_fingerprint: LinkTopologyFingerprint {
                    direct_child_sites: 1,
                    direct_circuits: 0,
                },
            })
        );
        assert_eq!(
            status.last_action_summary.as_deref(),
            Some("Rejected virtualization for node 'Node Rejected'")
        );
        assert!(status.warnings.iter().any(|warning| {
            warning.contains("Node Rejected")
                && warning.contains("structurally ineligible")
                && warning.contains("single promotable child")
        }));

        let last_activity = activity
            .back()
            .expect("rejected activity should be recorded");
        assert_eq!(last_activity.action, "virtualize_rejected");
        assert!(!last_activity.persisted);
        assert!(last_activity.reason.contains("Top-level safe"));
        assert!(last_activity.reason.contains("before submitting to Bakery"));

        assert!(rx.try_recv().is_err());
    }
}

//! The Bakery is where CAKE is made!
//!
//! More specifically, this crate provides a tracker of TC queues - described by the LibreQoS.py process,
//! but tracked for changes. We're at phase 3.
//!
//! In phase 1, the Bakery will build queues and a matching structure to track them. It will act exactly
//! like the LibreQoS.py process.
//!
//! In phase 2, the Bakery will *not* create CAKE queues - just the HTB hierarchy. When circuits are
//! detected as having traffic, the associated queue will be created. Ideally, some form of timeout
//! will be used to remove queues that are no longer in use. (Saving resources)
//!
//! In phase 3, the Bakery will - after initial creation - track the queues and update them as needed.
//! This will take a "diff" approach, finding differences and only applying those changes.
//!
//! In phase 4, the Bakery will implement "live move" --- allowing queues to be moved losslessly. This will
//! complete the NLNet project goals.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]

mod commands;
mod diff;
mod qdisc_handles;
mod queue_math;
mod utils;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::{debug, error, info, warn};
use utils::current_timestamp;
pub(crate) const CHANNEL_CAPACITY: usize = 65536; // 64k capacity for Bakery commands
use crate::commands::{
    ExecutionMode, RuntimeNodeOperationAction, RuntimeNodeOperationFailureReason,
    RuntimeNodeOperationSnapshot, RuntimeNodeOperationStatus,
};
use crate::diff::{
    CircuitDiffResult, SiteDiffResult, StructuralSiteDiffDetails, diff_circuits, diff_sites,
};
use crate::qdisc_handles::QdiscHandleState;
use crate::queue_math::{SqmKind, effective_sqm_kind, format_rate_for_tc_f32, quantum, r2q};
use crate::utils::{
    ExecuteResult, LiveTcClassEntry, LiveTcQdiscEntry, MemorySnapshot, execute_in_memory,
    execute_in_memory_chunked, invalidate_live_tc_snapshots, read_live_class_snapshot,
    read_live_qdisc_handle_majors, read_live_qdisc_snapshot, read_memory_snapshot,
    tc_io_cadence_snapshot, write_command_file,
};
pub use commands::{
    BakeryCommands, RuntimeNodeOperationAction as BakeryRuntimeNodeOperationAction,
    RuntimeNodeOperationFailureReason as BakeryRuntimeNodeOperationFailureReason,
    RuntimeNodeOperationSnapshot as BakeryRuntimeNodeOperationSnapshot,
    RuntimeNodeOperationStatus as BakeryRuntimeNodeOperationStatus,
};
use lqos_bus::{
    BusRequest, BusResponse, InsightLicenseSummary, LibreqosBusClient, TcHandle, UrgentSeverity,
    UrgentSource,
};
use lqos_config::{
    CircuitIdentityGroupInput, ClassIdentityPlannerConstraints, Config, LazyQueueMode,
    PlannerCircuitIdentityState, PlannerMinorReservations, PlannerSiteIdentityState,
    SiteIdentityInput, TopLevelPlannerItem, TopLevelPlannerMode, TopLevelPlannerParams,
    build_class_identity_reservations, plan_class_identities_with_constraints,
    plan_top_level_assignments,
};
use qdisc_handles::MqDeviceLayout;
use serde::Deserialize;
use serde_json::{Map, Value};

const TEST_FAULT_ONCE_PATH: &str = "/tmp/lqos_bakery_fail_purpose_once.txt";
const ACTIVE_RUNTIME_MINOR_START: u32 = 0x1000;
const MIGRATION_VERIFICATION_MAX_RETRIES: u8 = 3;
// ---------------------- Live-Move Types and Helpers (module scope) ----------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationStage {
    PrepareShadow,
    VerifyShadowReady,
    SwapToShadow,
    BuildFinal,
    VerifyFinalReady,
    SwapToFinal,
    TeardownMigrationScaffold,
    Done,
}

#[derive(Clone, Debug)]
struct Migration {
    circuit_hash: i64,
    circuit_name: Option<String>,
    site_name: Option<String>,
    // Old majors and qdisc handles
    old_class_major: u16,
    old_up_class_major: u16,
    old_down_qdisc_handle: Option<u16>,
    old_up_qdisc_handle: Option<u16>,
    // New parent handles and majors
    parent_class_id: TcHandle,
    up_parent_class_id: TcHandle,
    class_major: u16,
    up_class_major: u16,
    down_qdisc_handle: Option<u16>,
    up_qdisc_handle: Option<u16>,
    // Old and new rates
    old_down_min: f32,
    old_down_max: f32,
    old_up_min: f32,
    old_up_max: f32,
    new_down_min: f32,
    new_down_max: f32,
    new_up_min: f32,
    new_up_max: f32,
    // Minors
    old_minor: u16,
    shadow_minor: u16,
    final_minor: u16,
    // IP list for remapping
    ips: Vec<String>,
    // Per-circuit SQM override ("cake" or "fq_codel"), if any
    sqm_override: Option<String>,
    // Desired final circuit state. This is published to the effective circuit map only after
    // live TC and IP cutover reach the final class successfully.
    desired_cmd: Arc<BakeryCommands>,
    stage: MigrationStage,
    shadow_verify_attempts: u8,
    final_verify_attempts: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MigrationDirectionVerification {
    interface_name: String,
    expected_handle: TcHandle,
    expected_parent: TcHandle,
    observed_present: bool,
    observed_parent: Option<TcHandle>,
    observed_leaf_qdisc_major: Option<u16>,
}

impl MigrationDirectionVerification {
    fn ready(&self) -> bool {
        self.observed_parent == Some(self.expected_parent)
            && self.observed_leaf_qdisc_major.is_some()
    }

    fn summary(&self, direction: &str) -> String {
        let observed = match (
            self.observed_present,
            self.observed_parent,
            self.observed_leaf_qdisc_major,
        ) {
            (false, _, _) => "observed missing".to_string(),
            (true, Some(parent), Some(leaf_major)) => format!(
                "observed parent {} with leaf qdisc 0x{:x}:",
                parent.as_tc_string(),
                leaf_major
            ),
            (true, Some(parent), None) => {
                format!(
                    "observed parent {} with no leaf qdisc",
                    parent.as_tc_string()
                )
            }
            (true, None, Some(leaf_major)) => {
                format!("observed at root with leaf qdisc 0x{:x}:", leaf_major)
            }
            (true, None, None) => "observed at root with no leaf qdisc".to_string(),
        };
        format!(
            "{direction} {} expected class {} under parent {} with a leaf qdisc, {observed}",
            self.interface_name,
            self.expected_handle.as_tc_string(),
            self.expected_parent.as_tc_string()
        )
    }
}

fn wrong_parent_prune_commands_for_direction(
    interface_name: String,
    snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    expected_handle: TcHandle,
    expected_parent: TcHandle,
) -> Vec<Vec<String>> {
    let Some(observed) = snapshot.get(&expected_handle) else {
        return Vec::new();
    };
    if observed.parent == Some(expected_parent) {
        return Vec::new();
    }

    let mut commands = Vec::new();
    if observed.leaf_qdisc_major.is_some() {
        commands.push(vec![
            "qdisc".to_string(),
            "del".to_string(),
            "dev".to_string(),
            interface_name.clone(),
            "parent".to_string(),
            expected_handle.as_tc_string(),
        ]);
    }
    commands.push(vec![
        "class".to_string(),
        "del".to_string(),
        "dev".to_string(),
        interface_name,
        "classid".to_string(),
        expected_handle.as_tc_string(),
    ]);
    commands
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MigrationBranchVerification {
    down: MigrationDirectionVerification,
    up: Option<MigrationDirectionVerification>,
}

impl MigrationBranchVerification {
    fn ready(&self) -> bool {
        self.down.ready()
            && self
                .up
                .as_ref()
                .map(MigrationDirectionVerification::ready)
                .unwrap_or(true)
    }

    fn summary(&self) -> String {
        let mut parts = vec![self.down.summary("down")];
        if let Some(up) = &self.up {
            parts.push(up.summary("up"));
        }
        parts.join("; ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StormguardOverrideKey {
    interface: String,
    class: TcHandle,
}

#[derive(Clone, Debug)]
struct VirtualizedSiteQdiscHandles {
    down: Option<u16>,
    up: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeVirtualizedActiveBranch {
    Shadow,
    Original,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeVirtualizedBranchLifecycle {
    PhysicalActive,
    FlattenBuildPending,
    CutoverPending,
    FlattenedActive,
    RestorePending,
    PhysicalActiveCleanupPending,
    Failed,
}

#[derive(Clone, Debug)]
struct VirtualizedSiteState {
    site_name: String,
    site: Arc<BakeryCommands>,
    saved_sites: HashMap<i64, Arc<BakeryCommands>>,
    saved_circuits: HashMap<i64, Arc<BakeryCommands>>,
    active_sites: HashMap<i64, Arc<BakeryCommands>>,
    active_circuits: HashMap<i64, Arc<BakeryCommands>>,
    prune_sites: HashMap<i64, Arc<BakeryCommands>>,
    prune_circuits: HashMap<i64, Arc<BakeryCommands>>,
    qdisc_handles: VirtualizedSiteQdiscHandles,
    active_branch: RuntimeVirtualizedActiveBranch,
    lifecycle: RuntimeVirtualizedBranchLifecycle,
    pending_prune: bool,
    next_prune_attempt_unix: u64,
}

impl VirtualizedSiteState {
    fn active_branch_hides_original_site(&self) -> bool {
        matches!(self.active_branch, RuntimeVirtualizedActiveBranch::Shadow)
    }
}

fn lifecycle_from_active_branch(
    active_branch: RuntimeVirtualizedActiveBranch,
    pending_prune: bool,
) -> RuntimeVirtualizedBranchLifecycle {
    match (active_branch, pending_prune) {
        (RuntimeVirtualizedActiveBranch::Shadow, _) => {
            RuntimeVirtualizedBranchLifecycle::FlattenedActive
        }
        (RuntimeVirtualizedActiveBranch::Original, true) => {
            RuntimeVirtualizedBranchLifecycle::PhysicalActiveCleanupPending
        }
        (RuntimeVirtualizedActiveBranch::Original, false) => {
            RuntimeVirtualizedBranchLifecycle::PhysicalActive
        }
    }
}

fn preserve_cutover_pending_or_lifecycle_from_active_branch(
    state: &VirtualizedSiteState,
) -> RuntimeVirtualizedBranchLifecycle {
    if state.lifecycle == RuntimeVirtualizedBranchLifecycle::CutoverPending {
        RuntimeVirtualizedBranchLifecycle::CutoverPending
    } else {
        lifecycle_from_active_branch(state.active_branch, state.pending_prune)
    }
}

fn runtime_status_for_virtualized_state(
    state: &VirtualizedSiteState,
) -> RuntimeNodeOperationStatus {
    match state.lifecycle {
        RuntimeVirtualizedBranchLifecycle::Failed => RuntimeNodeOperationStatus::Dirty,
        RuntimeVirtualizedBranchLifecycle::FlattenedActive
        | RuntimeVirtualizedBranchLifecycle::PhysicalActiveCleanupPending
            if state.pending_prune =>
        {
            RuntimeNodeOperationStatus::AppliedAwaitingCleanup
        }
        RuntimeVirtualizedBranchLifecycle::FlattenBuildPending
        | RuntimeVirtualizedBranchLifecycle::CutoverPending
        | RuntimeVirtualizedBranchLifecycle::RestorePending => RuntimeNodeOperationStatus::Applying,
        RuntimeVirtualizedBranchLifecycle::PhysicalActive
        | RuntimeVirtualizedBranchLifecycle::PhysicalActiveCleanupPending
        | RuntimeVirtualizedBranchLifecycle::FlattenedActive => {
            RuntimeNodeOperationStatus::Completed
        }
    }
}

fn runtime_lifecycle_label(lifecycle: RuntimeVirtualizedBranchLifecycle) -> &'static str {
    match lifecycle {
        RuntimeVirtualizedBranchLifecycle::PhysicalActive => "PhysicalActive",
        RuntimeVirtualizedBranchLifecycle::FlattenBuildPending => "FlattenBuildPending",
        RuntimeVirtualizedBranchLifecycle::CutoverPending => "CutoverPending",
        RuntimeVirtualizedBranchLifecycle::FlattenedActive => "FlattenedActive",
        RuntimeVirtualizedBranchLifecycle::RestorePending => "RestorePending",
        RuntimeVirtualizedBranchLifecycle::PhysicalActiveCleanupPending => {
            "PhysicalActiveCleanupPending"
        }
        RuntimeVirtualizedBranchLifecycle::Failed => "Failed",
    }
}

fn runtime_active_branch_label(active_branch: RuntimeVirtualizedActiveBranch) -> &'static str {
    match active_branch {
        RuntimeVirtualizedActiveBranch::Shadow => "Shadow",
        RuntimeVirtualizedActiveBranch::Original => "Original",
    }
}

const RUNTIME_SITE_PRUNE_RETRY_SECONDS: u64 = 30;
const RUNTIME_SITE_PRUNE_MAX_ATTEMPTS: u32 = 5;
const RUNTIME_CUTOVER_RETRY_SECONDS: u64 = 1;
const BAKERY_BACKGROUND_INTERVAL_MS: u64 = 250;
const RUNTIME_DIRTY_SUBTREE_RELOAD_THRESHOLD: usize = 3;
const RUNTIME_NODE_OPERATION_CAPACITY: usize = 32;
const RUNTIME_NODE_OPERATION_DEFERRED_RETRY_SECONDS: u64 = 60;
const BAKERY_GROUPED_EVENT_DETAIL_LIMIT: usize = 3;

#[derive(Clone, Debug)]
struct RuntimeNodeOperation {
    operation_id: u64,
    site_hash: i64,
    site_name: Option<String>,
    action: RuntimeNodeOperationAction,
    status: RuntimeNodeOperationStatus,
    attempt_count: u32,
    submitted_at_unix: u64,
    updated_at_unix: u64,
    next_retry_at_unix: Option<u64>,
    last_error: Option<String>,
    failure_reason: Option<RuntimeNodeOperationFailureReason>,
}

impl RuntimeNodeOperation {
    fn new(
        operation_id: u64,
        site_hash: i64,
        site_name: Option<String>,
        action: RuntimeNodeOperationAction,
        now_unix: u64,
    ) -> Self {
        Self {
            operation_id,
            site_hash,
            site_name,
            action,
            status: RuntimeNodeOperationStatus::Submitted,
            attempt_count: 0,
            submitted_at_unix: now_unix,
            updated_at_unix: now_unix,
            next_retry_at_unix: None,
            last_error: None,
            failure_reason: None,
        }
    }

    fn snapshot(&self) -> RuntimeNodeOperationSnapshot {
        RuntimeNodeOperationSnapshot {
            operation_id: self.operation_id,
            site_hash: self.site_hash,
            action: self.action,
            status: self.status,
            attempt_count: self.attempt_count,
            submitted_at_unix: self.submitted_at_unix,
            updated_at_unix: self.updated_at_unix,
            next_retry_at_unix: self.next_retry_at_unix,
            last_error: self.last_error.clone(),
            failure_reason: self.failure_reason,
        }
    }

    fn update_status(
        &mut self,
        status: RuntimeNodeOperationStatus,
        now_unix: u64,
        last_error: Option<String>,
        next_retry_at_unix: Option<u64>,
    ) {
        self.update_status_with_reason(status, now_unix, last_error, None, next_retry_at_unix);
    }

    fn update_status_with_reason(
        &mut self,
        status: RuntimeNodeOperationStatus,
        now_unix: u64,
        last_error: Option<String>,
        failure_reason: Option<RuntimeNodeOperationFailureReason>,
        next_retry_at_unix: Option<u64>,
    ) {
        self.status = status;
        self.updated_at_unix = now_unix;
        self.last_error = last_error;
        self.failure_reason = failure_reason;
        self.next_retry_at_unix = next_retry_at_unix;
    }
}

fn mark_migration_reload_required(circuit_hash: i64, purpose: &str, result: &ExecuteResult) {
    let summary = summarize_apply_result(purpose, result);
    mark_reload_required(format!(
        "Bakery live migration failed for circuit {} during {}: {}. A full reload is now required before further incremental topology mutations.",
        circuit_hash, purpose, summary
    ));
}

fn migration_stage_apply_succeeded(
    migration: &mut Migration,
    purpose: &str,
    result: &ExecuteResult,
    next_stage: MigrationStage,
) -> bool {
    if result.ok {
        migration.stage = next_stage;
        return true;
    }

    mark_migration_reload_required(migration.circuit_hash, purpose, result);
    migration.stage = MigrationStage::Done;
    false
}

fn parse_ip_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

const DYNAMIC_CIRCUITS_FILENAME: &str = "dynamic_circuits.json";

#[derive(Clone, Debug)]
struct DynamicCircuitOverlayEntry {
    shaped_device: lqos_config::ShapedDevice,
    class_minor: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct PersistedDynamicCircuit {
    shaped: lqos_config::ShapedDevice,
    #[allow(dead_code)]
    #[serde(default)]
    last_seen_unix: u64,
}

#[derive(Debug, Default, Deserialize)]
struct PersistedDynamicCircuitsFile {
    #[serde(default)]
    #[allow(dead_code)]
    schema_version: u32,
    #[serde(default)]
    circuits: Vec<PersistedDynamicCircuit>,
}

fn dynamic_circuits_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(DYNAMIC_CIRCUITS_FILENAME)
}

fn recompute_shaped_hashes(device: &mut lqos_config::ShapedDevice) {
    device.circuit_hash = runtime_hash_to_i64(&device.circuit_id);
    device.device_hash = runtime_hash_to_i64(&device.device_id);
    device.parent_hash = runtime_hash_to_i64(&device.parent_node);
}

fn load_dynamic_circuit_overlays_from_disk(
    config: &Config,
) -> HashMap<i64, DynamicCircuitOverlayEntry> {
    let path = dynamic_circuits_path(config);
    if !path.exists() {
        return HashMap::new();
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(value) => value,
        Err(err) => {
            warn!(
                "Bakery: unable to read dynamic circuits file {} ({err}); treating as empty",
                path.display()
            );
            return HashMap::new();
        }
    };

    if raw.trim().is_empty() {
        return HashMap::new();
    }

    let parsed: PersistedDynamicCircuitsFile = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            warn!(
                "Bakery: unable to parse dynamic circuits file {} ({err}); treating as empty",
                path.display()
            );
            return HashMap::new();
        }
    };

    let mut overlays = HashMap::new();
    for entry in parsed.circuits {
        let mut shaped = entry.shaped;
        if shaped.circuit_id.trim().is_empty() {
            continue;
        }
        if shaped.device_id.trim().is_empty() {
            continue;
        }
        recompute_shaped_hashes(&mut shaped);
        let circuit_hash = shaped.circuit_hash;
        overlays.insert(
            circuit_hash,
            DynamicCircuitOverlayEntry {
                shaped_device: shaped,
                class_minor: None,
            },
        );
    }

    overlays
}

fn ip_list_from_shaped_device(device: &lqos_config::ShapedDevice) -> String {
    let mut ips = Vec::new();
    for (ip, prefix) in device.ipv4.iter() {
        ips.push(format!("{ip}/{prefix}"));
    }
    for (ip, prefix) in device.ipv6.iter() {
        ips.push(format!("{ip}/{prefix}"));
    }
    ips.sort();
    ips.dedup();
    ips.join(",")
}

fn normalize_circuit_id_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn find_free_dynamic_circuit_minor(
    used_down: &HashSet<u16>,
    used_up: &HashSet<u16>,
) -> Option<u16> {
    let start = ACTIVE_RUNTIME_MINOR_START.min(u16::MAX as u32) as u16;
    (start..=0xFFFEu16).find(|m| !used_down.contains(m) && !used_up.contains(m))
}

fn append_dynamic_circuit_overlays_to_batch(
    batch: &mut Vec<Arc<BakeryCommands>>,
    overlays: &mut HashMap<i64, DynamicCircuitOverlayEntry>,
    migrations: &HashMap<i64, Migration>,
) {
    if overlays.is_empty() {
        return;
    }

    let planned_sites: HashMap<i64, Arc<BakeryCommands>> = batch
        .iter()
        .filter_map(|cmd| match cmd.as_ref() {
            BakeryCommands::AddSite { site_hash, .. } => Some((*site_hash, Arc::clone(cmd))),
            _ => None,
        })
        .collect();
    let planned_circuits: HashMap<i64, Arc<BakeryCommands>> = batch
        .iter()
        .filter_map(|cmd| match cmd.as_ref() {
            BakeryCommands::AddCircuit { circuit_hash, .. } => {
                Some((*circuit_hash, Arc::clone(cmd)))
            }
            _ => None,
        })
        .collect();

    let mut overlay_keys: Vec<i64> = overlays.keys().copied().collect();
    overlay_keys.sort_unstable();

    let mut used_by_major: HashMap<(u16, u16), (HashSet<u16>, HashSet<u16>)> = HashMap::new();

    for circuit_hash in overlay_keys {
        let Some(entry) = overlays.get_mut(&circuit_hash) else {
            continue;
        };

        if entry.shaped_device.circuit_id.trim().is_empty()
            || entry.shaped_device.device_id.trim().is_empty()
        {
            continue;
        }

        recompute_shaped_hashes(&mut entry.shaped_device);
        let site_hash = runtime_hash_to_i64(&entry.shaped_device.parent_node);
        let Some(site_cmd) = planned_sites.get(&site_hash) else {
            warn!(
                "Bakery: skipping dynamic circuit overlay {} because parent node '{}' is not present in the current batch sites",
                entry.shaped_device.circuit_id, entry.shaped_device.parent_node
            );
            continue;
        };
        let Some((down_parent, up_parent)) = site_class_handles(site_cmd.as_ref()) else {
            warn!(
                "Bakery: skipping dynamic circuit overlay {} because parent node '{}' did not resolve to a valid site class",
                entry.shaped_device.circuit_id, entry.shaped_device.parent_node
            );
            continue;
        };

        let down_major = down_parent.get_major_minor().0;
        let up_major = up_parent.get_major_minor().0;

        let (used_down, used_up) =
            used_by_major
                .entry((down_major, up_major))
                .or_insert_with(|| {
                    let (mut used_down, mut used_up) =
                        used_site_minors_for_majors(&planned_sites, down_major, up_major);
                    let (used_circuit_down, used_circuit_up) =
                        used_circuit_minors_for_majors(&planned_circuits, down_major, up_major);
                    used_down.extend(used_circuit_down);
                    used_up.extend(used_circuit_up);
                    let (used_shadow_down, used_shadow_up) =
                        used_pending_migration_shadow_minors_for_majors(
                            migrations, down_major, up_major,
                        );
                    used_down.extend(used_shadow_down);
                    used_up.extend(used_shadow_up);
                    (used_down, used_up)
                });

        let mut desired_minor = entry.class_minor;
        if let Some(minor) = desired_minor
            && (minor < ACTIVE_RUNTIME_MINOR_START as u16
                || used_down.contains(&minor)
                || used_up.contains(&minor))
        {
            desired_minor = None;
        }

        let class_minor = match desired_minor
            .or_else(|| find_free_dynamic_circuit_minor(used_down, used_up))
        {
            Some(minor) => minor,
            None => {
                warn!(
                    "Bakery: unable to allocate a free class_minor for dynamic circuit overlay {} (down_major=0x{:x}, up_major=0x{:x})",
                    entry.shaped_device.circuit_id, down_major, up_major
                );
                continue;
            }
        };
        entry.class_minor = Some(class_minor);
        used_down.insert(class_minor);
        used_up.insert(class_minor);

        // Ensure the overlay command is the only circuit entry for this hash.
        batch.retain(|cmd| match cmd.as_ref() {
            BakeryCommands::AddCircuit {
                circuit_hash: h, ..
            } => *h != circuit_hash,
            _ => true,
        });

        let ip_addresses = ip_list_from_shaped_device(&entry.shaped_device);
        let circuit_name = (!entry.shaped_device.circuit_name.trim().is_empty())
            .then(|| entry.shaped_device.circuit_name.clone());
        let site_name = (!entry.shaped_device.parent_node.trim().is_empty())
            .then(|| entry.shaped_device.parent_node.clone());

        batch.push(Arc::new(BakeryCommands::AddCircuit {
            circuit_hash,
            circuit_name,
            site_name,
            parent_class_id: down_parent,
            up_parent_class_id: up_parent,
            class_minor,
            download_bandwidth_min: entry.shaped_device.download_min_mbps,
            upload_bandwidth_min: entry.shaped_device.upload_min_mbps,
            download_bandwidth_max: entry.shaped_device.download_max_mbps,
            upload_bandwidth_max: entry.shaped_device.upload_max_mbps,
            class_major: down_major,
            up_class_major: up_major,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses,
            sqm_override: entry.shaped_device.sqm_override.clone(),
        }));
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_upsert_dynamic_circuit_overlay(
    mut shaped_device: lqos_config::ShapedDevice,
    overlays: &mut HashMap<i64, DynamicCircuitOverlayEntry>,
    batch_in_progress: bool,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
    mq_layout: &Option<MqDeviceLayout>,
    qdisc_handles: &mut QdiscHandleState,
    migrations: &HashMap<i64, Migration>,
) -> Result<(), String> {
    if shaped_device.circuit_id.trim().is_empty() {
        return Err("dynamic circuit requires circuit_id".to_string());
    }
    if shaped_device.device_id.trim().is_empty() {
        return Err("dynamic circuit requires device_id".to_string());
    }
    if shaped_device.parent_node.trim().is_empty() {
        return Err("dynamic circuit requires parent_node".to_string());
    }

    recompute_shaped_hashes(&mut shaped_device);
    let circuit_hash = shaped_device.circuit_hash;

    let entry = overlays
        .entry(circuit_hash)
        .or_insert_with(|| DynamicCircuitOverlayEntry {
            shaped_device: shaped_device.clone(),
            class_minor: None,
        });
    entry.shaped_device = shaped_device;

    if batch_in_progress {
        // Avoid mutating live TC while a full batch is being assembled; the overlay will be
        // applied when the batch is committed.
        return Ok(());
    }

    let Ok(config) = lqos_config::load_config() else {
        return Err("unable to load config".to_string());
    };

    if config.queues.queue_mode.is_observe() {
        return Ok(());
    }

    // If we don't yet have a baseline MQ layout or any sites, keep the overlay and let the next
    // commit batch create the circuit.
    if !MQ_CREATED.load(Ordering::Relaxed) || mq_layout.is_none() || sites.is_empty() {
        return Ok(());
    }

    // If the circuit is already present, don't attempt to live-update it here yet.
    if circuits.contains_key(&circuit_hash) {
        return Ok(());
    }

    let site_hash = runtime_hash_to_i64(&entry.shaped_device.parent_node);
    let Some(site_cmd) = sites.get(&site_hash) else {
        return Err(format!(
            "parent node '{}' is not present in current bakery site state",
            entry.shaped_device.parent_node
        ));
    };
    let Some((down_parent, up_parent)) = site_class_handles(site_cmd.as_ref()) else {
        return Err(format!(
            "parent node '{}' did not resolve to a valid site class",
            entry.shaped_device.parent_node
        ));
    };

    let down_major = down_parent.get_major_minor().0;
    let up_major = up_parent.get_major_minor().0;

    let (mut used_down, mut used_up) = used_site_minors_for_majors(sites, down_major, up_major);
    let (used_circuit_down, used_circuit_up) =
        used_circuit_minors_for_majors(circuits, down_major, up_major);
    used_down.extend(used_circuit_down);
    used_up.extend(used_circuit_up);
    let (used_shadow_down, used_shadow_up) =
        used_pending_migration_shadow_minors_for_majors(migrations, down_major, up_major);
    used_down.extend(used_shadow_down);
    used_up.extend(used_shadow_up);

    let mut desired_minor = entry.class_minor;
    if let Some(minor) = desired_minor
        && (minor < ACTIVE_RUNTIME_MINOR_START as u16
            || used_down.contains(&minor)
            || used_up.contains(&minor))
    {
        desired_minor = None;
    }
    let class_minor = desired_minor
        .or_else(|| find_free_dynamic_circuit_minor(&used_down, &used_up))
        .ok_or_else(|| "unable to allocate free class_minor for dynamic circuit".to_string())?;
    entry.class_minor = Some(class_minor);

    let ip_addresses = ip_list_from_shaped_device(&entry.shaped_device);
    let candidate = Arc::new(BakeryCommands::AddCircuit {
        circuit_hash,
        circuit_name: (!entry.shaped_device.circuit_name.trim().is_empty())
            .then(|| entry.shaped_device.circuit_name.clone()),
        site_name: (!entry.shaped_device.parent_node.trim().is_empty())
            .then(|| entry.shaped_device.parent_node.clone()),
        parent_class_id: down_parent,
        up_parent_class_id: up_parent,
        class_minor,
        download_bandwidth_min: entry.shaped_device.download_min_mbps,
        upload_bandwidth_min: entry.shaped_device.upload_min_mbps,
        download_bandwidth_max: entry.shaped_device.download_max_mbps,
        upload_bandwidth_max: entry.shaped_device.upload_max_mbps,
        class_major: down_major,
        up_class_major: up_major,
        down_qdisc_handle: None,
        up_qdisc_handle: None,
        ip_addresses,
        sqm_override: entry.shaped_device.sqm_override.clone(),
    });

    let mapped_limit = resolve_mapped_circuit_limit();
    if is_mapped_add_circuit(candidate.as_ref())
        && let Some(limit) = mapped_limit.effective_limit
        && circuits
            .values()
            .filter(|c| is_mapped_add_circuit(c.as_ref()))
            .count()
            >= limit
    {
        let stats = MappedLimitStats {
            enforced_limit: mapped_limit.effective_limit,
            requested_mapped: 1,
            allowed_mapped: 0,
            dropped_mapped: 1,
        };
        warn!(
            "Bakery mapped circuit cap enforced (dynamic circuit addition): requested={}, allowed={}, dropped={}, limit={} (licensed={}, max_circuits={:?})",
            stats.requested_mapped,
            stats.allowed_mapped,
            stats.dropped_mapped,
            format_mapped_limit(mapped_limit.effective_limit),
            mapped_limit.licensed,
            mapped_limit.max_circuits
        );
        maybe_emit_mapped_circuit_limit_urgent(&stats);
        return Err("mapped circuit limit reached".to_string());
    }

    let Some(layout) = mq_layout.as_ref() else {
        return Ok(());
    };
    let live_reserved_handles =
        snapshot_live_qdisc_handle_majors_or_empty(&config, "dynamic circuit additions");
    let enriched = with_assigned_qdisc_handles_reserved(
        &candidate,
        &config,
        layout,
        qdisc_handles,
        &live_reserved_handles,
    );
    let commands = enriched
        .to_commands(&config, ExecutionMode::Builder)
        .unwrap_or_default();
    if !commands.is_empty() {
        execute_and_record_live_change(&commands, "adding dynamic circuit");
    }
    circuits.insert(circuit_hash, enriched);
    live_circuits.remove(&circuit_hash);
    qdisc_handles.save(&config);
    update_queue_distribution_snapshot(sites, circuits);

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_remove_dynamic_circuit_overlay(
    circuit_id: &str,
    overlays: &mut HashMap<i64, DynamicCircuitOverlayEntry>,
    batch_in_progress: bool,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
    _mq_layout: &Option<MqDeviceLayout>,
    qdisc_handles: &mut QdiscHandleState,
) -> Result<(), String> {
    let normalized = normalize_circuit_id_key(circuit_id);
    let overlay_key = overlays
        .iter()
        .find(|(_, entry)| normalize_circuit_id_key(&entry.shaped_device.circuit_id) == normalized)
        .map(|(k, _)| *k)
        .or_else(|| {
            let computed = runtime_hash_to_i64(circuit_id);
            overlays.contains_key(&computed).then_some(computed)
        });

    if let Some(key) = overlay_key {
        overlays.remove(&key);
    }

    if batch_in_progress {
        // Avoid mutating live TC while a full batch is being assembled; the next commit batch
        // will reconcile the removal.
        return Ok(());
    }

    let Ok(config) = lqos_config::load_config() else {
        return Err("unable to load config".to_string());
    };

    let circuit_hash = overlay_key.unwrap_or_else(|| runtime_hash_to_i64(circuit_id));
    if let Some(circuit) = circuits.remove(&circuit_hash) {
        let was_activated = live_circuits.contains_key(&circuit_hash);
        let commands = match config.queues.lazy_queues.as_ref() {
            None | Some(LazyQueueMode::No) => circuit.to_prune(&config, true),
            Some(LazyQueueMode::Htb) => {
                if was_activated {
                    circuit.to_prune(&config, false)
                } else {
                    None
                }
            }
            Some(LazyQueueMode::Full) => {
                if was_activated {
                    circuit.to_prune(&config, true)
                } else {
                    None
                }
            }
        };
        if let Some(cmd) = commands {
            execute_and_record_live_change(&cmd, "removing dynamic circuit");
        }
        live_circuits.remove(&circuit_hash);
        qdisc_handles.release_circuit(&config.isp_interface(), circuit_hash);
        if !config.on_a_stick_mode() {
            qdisc_handles.release_circuit(&config.internet_interface(), circuit_hash);
        }
        qdisc_handles.save(&config);
        update_queue_distribution_snapshot(sites, circuits);
    }

    Ok(())
}

/*fn parse_ip_and_prefix(ip: &str) -> (String, u32) {
    if let Some((addr, pfx)) = ip.split_once('/') {
        if let Ok(n) = pfx.parse::<u32>() {
            return (addr.to_string(), n);
        }
    }
    if ip.contains(':') { (ip.to_string(), 128) } else { (ip.to_string(), 32) }
}*/

fn tc_handle_from_major_minor(major: u16, minor: u16) -> TcHandle {
    TcHandle::from_u32(((major as u32) << 16) | (minor as u32))
}

fn used_site_minors_for_majors(
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    down_major: u16,
    up_major: u16,
) -> (HashSet<u16>, HashSet<u16>) {
    let mut used_down = HashSet::new();
    let mut used_up = HashSet::new();
    for site in sites.values() {
        if let BakeryCommands::AddSite {
            parent_class_id,
            up_parent_class_id,
            class_minor,
            ..
        } = site.as_ref()
        {
            if parent_class_id.get_major_minor().0 == down_major {
                used_down.insert(*class_minor);
            }
            if up_parent_class_id.get_major_minor().0 == up_major {
                used_up.insert(*class_minor);
            }
        }
    }
    (used_down, used_up)
}

fn used_circuit_minors_for_majors(
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    down_major: u16,
    up_major: u16,
) -> (HashSet<u16>, HashSet<u16>) {
    let mut used_down = HashSet::new();
    let mut used_up = HashSet::new();
    for circuit in circuits.values() {
        if let BakeryCommands::AddCircuit {
            class_minor,
            class_major,
            up_class_major,
            ..
        } = circuit.as_ref()
        {
            if *class_major == down_major {
                used_down.insert(*class_minor);
            }
            if *up_class_major == up_major {
                used_up.insert(*class_minor);
            }
        }
    }
    (used_down, used_up)
}

fn used_pending_migration_shadow_minors_for_majors(
    migrations: &HashMap<i64, Migration>,
    down_major: u16,
    up_major: u16,
) -> (HashSet<u16>, HashSet<u16>) {
    let mut used_down = HashSet::new();
    let mut used_up = HashSet::new();
    for migration in migrations.values() {
        if migration.stage == MigrationStage::Done {
            continue;
        }
        if migration.class_major == down_major {
            used_down.insert(migration.shadow_minor);
        }
        if migration.up_class_major == up_major {
            used_up.insert(migration.shadow_minor);
        }
    }
    (used_down, used_up)
}

fn find_free_circuit_shadow_minor(
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    migrations: &HashMap<i64, Migration>,
    down_major: u16,
    up_major: u16,
) -> Option<u16> {
    let (mut used_down, mut used_up) = used_site_minors_for_majors(sites, down_major, up_major);
    let (used_circuit_down, used_circuit_up) =
        used_circuit_minors_for_majors(circuits, down_major, up_major);
    used_down.extend(used_circuit_down);
    used_up.extend(used_circuit_up);
    let (used_shadow_down, used_shadow_up) =
        used_pending_migration_shadow_minors_for_majors(migrations, down_major, up_major);
    used_down.extend(used_shadow_down);
    used_up.extend(used_shadow_up);
    for start in [0x2000u16, 0x4000, 0x6000, 0x8000, 0xA000, 0xC000, 0xE000] {
        let end = start.saturating_add(0x1FFF);
        for m in start..=end.min(0xFFFE) {
            if !used_down.contains(&m) && !used_up.contains(&m) {
                return Some(m);
            }
        }
    }
    (1..=0xFFFEu16).find(|&m| !used_down.contains(&m) && !used_up.contains(&m))
}

fn find_free_site_shadow_minor(
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    migrations: &HashMap<i64, Migration>,
    planned_sites: &HashMap<i64, PlannedSiteUpdate>,
    planned_circuits: &HashMap<i64, PlannedCircuitUpdate>,
    down_parent: &TcHandle,
    up_parent: &TcHandle,
) -> Option<u16> {
    let down_major = down_parent.get_major_minor().0;
    let up_major = up_parent.get_major_minor().0;
    let (mut used_down, mut used_up) = used_site_minors_for_majors(sites, down_major, up_major);
    let (used_circuit_down, used_circuit_up) =
        used_circuit_minors_for_majors(circuits, down_major, up_major);
    used_down.extend(used_circuit_down);
    used_up.extend(used_circuit_up);
    let (used_shadow_down, used_shadow_up) =
        used_pending_migration_shadow_minors_for_majors(migrations, down_major, up_major);
    used_down.extend(used_shadow_down);
    used_up.extend(used_shadow_up);

    for update in planned_sites.values() {
        if let BakeryCommands::AddSite {
            parent_class_id,
            up_parent_class_id,
            class_minor,
            ..
        } = update.command.as_ref()
        {
            if parent_class_id.get_major_minor().0 == down_major {
                used_down.insert(*class_minor);
            }
            if up_parent_class_id.get_major_minor().0 == up_major {
                used_up.insert(*class_minor);
            }
        }
    }

    for update in planned_circuits.values() {
        if let BakeryCommands::AddCircuit {
            class_minor,
            class_major,
            up_class_major,
            ..
        } = update.command.as_ref()
        {
            if *class_major == down_major {
                used_down.insert(*class_minor);
            }
            if *up_class_major == up_major {
                used_up.insert(*class_minor);
            }
        }
    }

    for start in [0x2000u16, 0x4000, 0x6000, 0x8000, 0xA000, 0xC000, 0xE000] {
        let end = start.saturating_add(0x1FFF);
        for minor in start..=end.min(0xFFFE) {
            if !used_down.contains(&minor) && !used_up.contains(&minor) {
                return Some(minor);
            }
        }
    }

    (1..=0xFFFEu16).find(|minor| !used_down.contains(minor) && !used_up.contains(minor))
}

fn add_commands_for_circuit(
    cmd: &BakeryCommands,
    config: &Arc<Config>,
    mode: ExecutionMode,
) -> Option<Vec<Vec<String>>> {
    cmd.to_commands(config, mode)
}

fn build_temp_add_cmd(
    base: &BakeryCommands,
    minor: u16,
    down_min: f32,
    down_max: f32,
    up_min: f32,
    up_max: f32,
    preserve_qdisc_handles: bool,
) -> Option<BakeryCommands> {
    if let BakeryCommands::AddCircuit {
        circuit_hash,
        circuit_name,
        site_name,
        parent_class_id,
        up_parent_class_id,
        class_major,
        up_class_major,
        down_qdisc_handle,
        up_qdisc_handle,
        ip_addresses,
        sqm_override,
        ..
    } = base
    {
        Some(BakeryCommands::AddCircuit {
            circuit_hash: *circuit_hash,
            circuit_name: circuit_name.clone(),
            site_name: site_name.clone(),
            parent_class_id: *parent_class_id,
            up_parent_class_id: *up_parent_class_id,
            class_minor: minor,
            download_bandwidth_min: down_min,
            upload_bandwidth_min: up_min,
            download_bandwidth_max: down_max,
            upload_bandwidth_max: up_max,
            class_major: *class_major,
            up_class_major: *up_class_major,
            down_qdisc_handle: preserve_qdisc_handles
                .then_some(*down_qdisc_handle)
                .flatten(),
            up_qdisc_handle: preserve_qdisc_handles.then_some(*up_qdisc_handle).flatten(),
            ip_addresses: ip_addresses.clone(),
            sqm_override: sqm_override.clone(),
        })
    } else {
        None
    }
}

fn shadow_qdisc_allocation_key(circuit_hash: i64, uplink: bool) -> i64 {
    let base = circuit_hash.wrapping_mul(2).wrapping_neg();
    if uplink { base.wrapping_sub(1) } else { base }
}

fn assign_shadow_qdisc_handles(
    migration: &Migration,
    config: &Arc<Config>,
    qdisc_handles: &QdiscHandleState,
    live_reserved_handles: &HashMap<String, HashSet<u16>>,
) -> (Option<u16>, Option<u16>) {
    let mut shadow_handles = qdisc_handles.clone();
    let isp_interface = config.isp_interface();
    let mut down_reserved = live_reserved_handles
        .get(&isp_interface)
        .cloned()
        .unwrap_or_default();
    if let Some(handle) = migration.old_down_qdisc_handle {
        down_reserved.insert(handle);
    }
    if let Some(handle) = migration.down_qdisc_handle {
        down_reserved.insert(handle);
    }
    let down_handle = migration.down_qdisc_handle.and_then(|_| {
        shadow_handles.assign_circuit_handle(
            &isp_interface,
            shadow_qdisc_allocation_key(migration.circuit_hash, false),
            &down_reserved,
        )
    });

    if config.on_a_stick_mode() {
        return (down_handle, None);
    }

    let internet_interface = config.internet_interface();
    let mut up_reserved = live_reserved_handles
        .get(&internet_interface)
        .cloned()
        .unwrap_or_default();
    if let Some(handle) = migration.old_up_qdisc_handle {
        up_reserved.insert(handle);
    }
    if let Some(handle) = migration.up_qdisc_handle {
        up_reserved.insert(handle);
    }
    let up_handle = migration.up_qdisc_handle.and_then(|_| {
        shadow_handles.assign_circuit_handle(
            &internet_interface,
            shadow_qdisc_allocation_key(migration.circuit_hash, true),
            &up_reserved,
        )
    });

    (down_handle, up_handle)
}

fn build_shadow_add_cmd(
    migration: &Migration,
    config: &Arc<Config>,
    qdisc_handles: &QdiscHandleState,
    live_reserved_handles: &HashMap<String, HashSet<u16>>,
) -> Option<BakeryCommands> {
    let mut temp = build_temp_add_cmd(
        &BakeryCommands::AddCircuit {
            circuit_hash: migration.circuit_hash,
            circuit_name: migration.circuit_name.clone(),
            site_name: migration.site_name.clone(),
            parent_class_id: migration.parent_class_id,
            up_parent_class_id: migration.up_parent_class_id,
            class_minor: migration.shadow_minor,
            download_bandwidth_min: migration.old_down_min,
            upload_bandwidth_min: migration.old_up_min,
            download_bandwidth_max: migration.old_down_max,
            upload_bandwidth_max: migration.old_up_max,
            class_major: migration.class_major,
            up_class_major: migration.up_class_major,
            down_qdisc_handle: migration.down_qdisc_handle,
            up_qdisc_handle: migration.up_qdisc_handle,
            ip_addresses: String::new(),
            sqm_override: migration.sqm_override.clone(),
        },
        migration.shadow_minor,
        migration.old_down_min,
        migration.old_down_max,
        migration.old_up_min,
        migration.old_up_max,
        false,
    )?;
    let (down_qdisc_handle, up_qdisc_handle) =
        assign_shadow_qdisc_handles(migration, config, qdisc_handles, live_reserved_handles);
    let BakeryCommands::AddCircuit {
        down_qdisc_handle: shadow_down_qdisc_handle,
        up_qdisc_handle: shadow_up_qdisc_handle,
        ..
    } = &mut temp
    else {
        return None;
    };
    *shadow_down_qdisc_handle = down_qdisc_handle;
    *shadow_up_qdisc_handle = up_qdisc_handle;
    Some(temp)
}

fn queue_runtime_migration(
    old_cmd: &BakeryCommands,
    new_cmd: &Arc<BakeryCommands>,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &HashMap<i64, u64>,
    migrations: &mut HashMap<i64, Migration>,
    require_live_circuit: bool,
) -> bool {
    let BakeryCommands::AddCircuit {
        circuit_hash,
        circuit_name,
        site_name,
        parent_class_id,
        up_parent_class_id,
        class_minor,
        download_bandwidth_min,
        upload_bandwidth_min,
        download_bandwidth_max,
        upload_bandwidth_max,
        class_major,
        up_class_major,
        down_qdisc_handle,
        up_qdisc_handle,
        ip_addresses,
        sqm_override,
        ..
    } = new_cmd.as_ref()
    else {
        return false;
    };

    if require_live_circuit && !live_circuits.contains_key(circuit_hash) {
        return false;
    }

    let BakeryCommands::AddCircuit {
        class_minor: old_minor,
        download_bandwidth_min: old_down_min,
        upload_bandwidth_min: old_up_min,
        download_bandwidth_max: old_down_max,
        upload_bandwidth_max: old_up_max,
        class_major: old_class_major,
        up_class_major: old_up_class_major,
        down_qdisc_handle: old_down_qdisc_handle,
        up_qdisc_handle: old_up_qdisc_handle,
        ..
    } = old_cmd
    else {
        return false;
    };

    let Some(shadow_minor) =
        find_free_circuit_shadow_minor(sites, circuits, migrations, *class_major, *up_class_major)
    else {
        return false;
    };

    let mig = Migration {
        circuit_hash: *circuit_hash,
        circuit_name: circuit_name.clone(),
        site_name: site_name.clone(),
        old_class_major: *old_class_major,
        old_up_class_major: *old_up_class_major,
        old_down_qdisc_handle: *old_down_qdisc_handle,
        old_up_qdisc_handle: *old_up_qdisc_handle,
        parent_class_id: *parent_class_id,
        up_parent_class_id: *up_parent_class_id,
        class_major: *class_major,
        up_class_major: *up_class_major,
        down_qdisc_handle: *down_qdisc_handle,
        up_qdisc_handle: *up_qdisc_handle,
        old_down_min: *old_down_min,
        old_down_max: *old_down_max,
        old_up_min: *old_up_min,
        old_up_max: *old_up_max,
        new_down_min: *download_bandwidth_min,
        new_down_max: *download_bandwidth_max,
        new_up_min: *upload_bandwidth_min,
        new_up_max: *upload_bandwidth_max,
        old_minor: *old_minor,
        shadow_minor,
        final_minor: *class_minor,
        ips: parse_ip_list(ip_addresses),
        sqm_override: sqm_override.clone(),
        desired_cmd: Arc::clone(new_cmd),
        stage: MigrationStage::PrepareShadow,
        shadow_verify_attempts: 0,
        final_verify_attempts: 0,
    };

    migrations.insert(*circuit_hash, mig);
    true
}

fn queue_live_migration(
    old_cmd: &BakeryCommands,
    new_cmd: &Arc<BakeryCommands>,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &HashMap<i64, u64>,
    migrations: &mut HashMap<i64, Migration>,
) -> bool {
    queue_runtime_migration(
        old_cmd,
        new_cmd,
        sites,
        circuits,
        live_circuits,
        migrations,
        true,
    )
}

fn queue_top_level_runtime_migration(
    old_cmd: &BakeryCommands,
    new_cmd: &Arc<BakeryCommands>,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &HashMap<i64, u64>,
    migrations: &mut HashMap<i64, Migration>,
) -> bool {
    queue_runtime_migration(
        old_cmd,
        new_cmd,
        sites,
        circuits,
        live_circuits,
        migrations,
        false,
    )
}

fn circuits_with_pending_migration_targets(
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    migrations: &HashMap<i64, Migration>,
) -> HashMap<i64, Arc<BakeryCommands>> {
    let mut overlaid = circuits.clone();
    for (circuit_hash, migration) in migrations {
        overlaid.insert(*circuit_hash, Arc::clone(&migration.desired_cmd));
    }
    overlaid
}

/// Count of Bakery-Managed circuits that are currently active.
pub static ACTIVE_CIRCUITS: AtomicUsize = AtomicUsize::new(0);
/// True while Bakery is applying a full reload batch to `tc`.
static FULL_RELOAD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
pub(crate) fn test_state_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}
/// Hard kernel limit for auto-allocated qdisc handles on a single network interface.
pub const HARD_QDISC_HANDLE_LIMIT_PER_INTERFACE: usize = 65_534;
/// Conservative operational limit used to fail a full reload before qdisc handle exhaustion.
pub const SAFE_QDISC_BUDGET_PER_INTERFACE: usize = 65_000;
const LIVE_CAPACITY_REFRESH_SECONDS: u64 = 30 * 60;
/// Conservative estimated kernel-memory cost of a non-leaf infrastructure qdisc.
///
/// These weights are not kernel ABI sizes. They are deliberately conservative
/// safety estimates used to block or stop obviously risky full reloads before
/// the host reaches OOM pressure.
pub const INFRA_QDISC_ESTIMATED_MEMORY_BYTES: u64 = 16 * 1024;
/// Conservative estimated kernel-memory cost of an `fq_codel` leaf qdisc.
///
/// This remains intentionally lower than CAKE, but is biased high enough to
/// prefer false-positive safety blocks over OOM risk on large reloads.
pub const FQ_CODEL_QDISC_ESTIMATED_MEMORY_BYTES: u64 = 64 * 1024;
/// Conservative estimated kernel-memory cost of a `cake` leaf qdisc.
///
/// Tuned upward after live production capture showed summed CAKE runtime memory
/// substantially exceeding the earlier heuristic during busy periods.
pub const CAKE_QDISC_ESTIMATED_MEMORY_BYTES: u64 = 512 * 1024;
/// Minimum memory headroom Bakery tries to leave unused after a projected or in-flight apply.
pub const BAKERY_MEMORY_GUARD_MIN_AVAILABLE_BYTES: u64 = 768 * 1024 * 1024;
/// Maximum number of mapped circuits allowed without Insight.
const DEFAULT_MAPPED_CIRCUITS_LIMIT: usize = 1000;
/// Minimum interval between repeated mapped-circuit-limit urgent issues.
const CIRCUIT_LIMIT_URGENT_INTERVAL_SECONDS: u64 = 30 * 60;
/// Last timestamp at which we emitted a mapped-circuit-limit urgent issue.
static LAST_CIRCUIT_LIMIT_URGENT_TS: AtomicU64 = AtomicU64::new(0);
const BAKERY_EVENT_LIMIT: usize = 50;
const FULL_RELOAD_TC_CHUNK_SIZE: usize = 2_500;

/// High-level Bakery execution mode for operator-facing status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeryMode {
    /// Bakery is not currently applying queue changes.
    Idle,
    /// Bakery is applying a structural full reload.
    ApplyingFullReload,
    /// Bakery is applying an incremental or live change.
    ApplyingLiveChange,
}

/// Type of the most recent apply action recorded by Bakery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeryApplyType {
    /// No apply has been recorded yet.
    None,
    /// A full reload apply.
    FullReload,
    /// A live/incremental apply.
    LiveChange,
}

/// Per-interface qdisc budget usage snapshot for Bakery UI/status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeryCapacityInterfaceSnapshot {
    /// Interface name, e.g. `ens19`.
    pub name: String,
    /// Planned qdisc count for that interface.
    pub planned_qdiscs: usize,
    /// Planned infrastructure qdiscs (`mq`, `htb`, etc.) for that interface.
    pub infra_qdiscs: usize,
    /// Planned `cake` leaf qdiscs for that interface.
    pub cake_qdiscs: usize,
    /// Planned `fq_codel` leaf qdiscs for that interface.
    pub fq_codel_qdiscs: usize,
    /// Estimated kernel memory cost for that interface's planned qdiscs.
    pub estimated_memory_bytes: u64,
}

/// Per-interface live qdisc usage snapshot for Bakery UI/status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeryLiveCapacityInterfaceSnapshot {
    /// Interface name, e.g. `ens19`.
    pub name: String,
    /// Current live qdisc handle count observed on the interface.
    pub live_qdiscs: usize,
}

/// Per-queue-root queue layout summary for Bakery UI/status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeryQueueDistributionSnapshot {
    /// Queue/root number, e.g. `1` for `Q1`.
    pub queue: u32,
    /// Number of top-level sites currently assigned to this queue.
    pub top_level_site_count: usize,
    /// Total sites currently assigned to this queue.
    pub site_count: usize,
    /// Total circuits currently assigned to this queue.
    pub circuit_count: usize,
    /// Aggregate configured downstream max Mbps for circuits on this queue.
    pub download_mbps: u64,
    /// Aggregate configured upstream max Mbps for circuits on this queue.
    pub upload_mbps: u64,
}

/// Latest Bakery-tracked runtime node operation for UI/status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeryRuntimeOperationHeadlineSnapshot {
    /// Monotonic Bakery-local operation identifier.
    pub operation_id: u64,
    /// Stable Bakery site hash derived from the node name.
    pub site_hash: i64,
    /// Human site name retained by Bakery when the runtime operation was created.
    pub site_name: Option<String>,
    /// Requested runtime action.
    pub action: RuntimeNodeOperationAction,
    /// Current operation status.
    pub status: RuntimeNodeOperationStatus,
    /// Number of attempts performed so far.
    pub attempt_count: u32,
    /// Unix timestamp when the operation last changed state.
    pub updated_at_unix: u64,
    /// Optional unix timestamp for the next retry, if waiting.
    pub next_retry_at_unix: Option<u64>,
    /// Last error observed by Bakery for this operation, if any.
    pub last_error: Option<String>,
}

/// Latest Bakery-tracked runtime branch-state snapshot for a single node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeryRuntimeNodeBranchSnapshot {
    /// Stable Bakery site hash derived from the node name.
    pub site_hash: i64,
    /// Human site name retained by Bakery when the runtime branch was created.
    pub site_name: String,
    /// Which retained branch is currently active for this node.
    pub active_branch: String,
    /// Current runtime lifecycle label for this node.
    pub lifecycle: String,
    /// Whether Bakery is still waiting to prune the inactive branch.
    pub pending_prune: bool,
    /// Optional unix timestamp for the next cleanup retry, if one is pending.
    pub next_prune_attempt_unix: Option<u64>,
    /// Hashes of branch sites currently marked active.
    pub active_site_hashes: Vec<i64>,
    /// Hashes of original/saved sites retained for restore logic.
    pub saved_site_hashes: Vec<i64>,
    /// Hashes of inactive-branch sites queued for pruning.
    pub prune_site_hashes: Vec<i64>,
    /// Observed downlink qdisc major for the runtime root, if known.
    pub qdisc_down_major: Option<u16>,
    /// Observed uplink qdisc major for the runtime root, if known.
    pub qdisc_up_major: Option<u16>,
}

/// Compact runtime-operation summary for Bakery UI/status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeryRuntimeOperationsSnapshot {
    /// Number of submitted operations waiting to start.
    pub submitted_count: usize,
    /// Number of operations deferred because runtime-node capacity is currently saturated.
    pub deferred_count: usize,
    /// Number of operations currently applying.
    pub applying_count: usize,
    /// Number of operations awaiting deferred cleanup.
    pub awaiting_cleanup_count: usize,
    /// Number of failed operations that are not classified as structural blocks.
    pub failed_count: usize,
    /// Number of operations blocked by structural runtime constraints until topology changes.
    pub blocked_count: usize,
    /// Number of operations marked Dirty.
    pub dirty_count: usize,
    /// Most recently updated runtime operation, if any.
    pub latest: Option<BakeryRuntimeOperationHeadlineSnapshot>,
}

/// Last qdisc preflight result retained for Bakery UI/status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeryPreflightSnapshot {
    /// Whether the last preflight was within budget.
    pub ok: bool,
    /// Short operator-facing summary.
    pub message: String,
    /// Safe operational budget used during the preflight.
    pub safe_budget: usize,
    /// Hard kernel limit used for reference.
    pub hard_limit: usize,
    /// Estimated total kernel memory cost of the planned qdisc model.
    pub estimated_total_memory_bytes: u64,
    /// Current host memory available during the preflight, if known.
    pub memory_available_bytes: Option<u64>,
    /// Memory floor that must remain available before the apply proceeds.
    pub memory_guard_min_available_bytes: u64,
    /// Whether the memory preflight passed.
    pub memory_ok: bool,
    /// Per-interface planned qdisc counts.
    pub interfaces: Vec<BakeryCapacityInterfaceSnapshot>,
}

/// Operator-facing Bakery status snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeryStatusSnapshot {
    /// Current Bakery mode.
    pub mode: BakeryMode,
    /// Unix timestamp when the current action started, if any.
    pub current_action_started_unix: Option<u64>,
    /// Current apply phase for the in-flight action, if any.
    pub current_apply_phase: Option<String>,
    /// Total `tc` commands planned for the in-flight action.
    pub current_apply_total_tc_commands: usize,
    /// `tc` commands already completed for the in-flight action.
    pub current_apply_completed_tc_commands: usize,
    /// Total apply chunks planned for the in-flight action.
    pub current_apply_total_chunks: usize,
    /// Apply chunks already completed for the in-flight action.
    pub current_apply_completed_chunks: usize,
    /// Currently active circuit count.
    pub active_circuits: usize,
    /// Unix timestamp of the last successful apply, if any.
    pub last_success_unix: Option<u64>,
    /// Unix timestamp of the last successful full reload, if any.
    pub last_full_reload_success_unix: Option<u64>,
    /// Unix timestamp of the last failed apply, if any.
    pub last_failure_unix: Option<u64>,
    /// Summary of the last failure, if any.
    pub last_failure_summary: Option<String>,
    /// Type of the last apply recorded by Bakery.
    pub last_apply_type: BakeryApplyType,
    /// Number of `tc` commands in the last apply.
    pub last_total_tc_commands: usize,
    /// Number of `class` commands in the last apply.
    pub last_class_commands: usize,
    /// Number of `qdisc` commands in the last apply.
    pub last_qdisc_commands: usize,
    /// Time spent expanding/building the last apply command list.
    pub last_build_duration_ms: u64,
    /// Time spent running the last apply through `tc`.
    pub last_apply_duration_ms: u64,
    /// Average interval between real unified Bakery `tc` reads/writes, in milliseconds.
    pub avg_tc_io_interval_ms: Option<u64>,
    /// Unix timestamp when Bakery last performed a real unified `tc` read/write.
    pub last_tc_io_unix: Option<u64>,
    /// Number of recent intervals contributing to the average TC I/O interval.
    pub tc_io_interval_samples: usize,
    /// Current runtime node-operation summary.
    pub runtime_operations: BakeryRuntimeOperationsSnapshot,
    /// Current queue-root distribution summary.
    pub queue_distribution: Vec<BakeryQueueDistributionSnapshot>,
    /// Current live per-interface qdisc usage snapshot.
    pub live_capacity_interfaces: Vec<BakeryLiveCapacityInterfaceSnapshot>,
    /// Safe per-interface qdisc budget used for the live usage bar.
    pub live_capacity_safe_budget: usize,
    /// Unix timestamp when the live capacity snapshot was last refreshed.
    pub live_capacity_updated_at_unix: Option<u64>,
    /// Last qdisc preflight summary known to Bakery.
    pub preflight: Option<BakeryPreflightSnapshot>,
    /// Whether Bakery has detected enough runtime drift to require a structural full reload.
    pub reload_required: bool,
    /// Operator-facing reason for why a full reload is now required, if any.
    pub reload_required_reason: Option<String>,
    /// Number of runtime node operations currently marked dirty.
    pub dirty_subtree_count: usize,
}

/// Recent operator-facing Bakery activity event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeryActivityEntry {
    /// Unix timestamp in seconds.
    pub ts: u64,
    /// Stable short event code.
    pub event: String,
    /// `info`, `warning`, or `error`.
    pub status: String,
    /// Stable Bakery site hash associated with the event, if any.
    pub site_hash: Option<i64>,
    /// Human site name retained by Bakery when the event was created, if any.
    pub site_name: Option<String>,
    /// Human-readable summary.
    pub summary: String,
}

#[derive(Clone, Debug)]
struct BakeryTelemetryState {
    mode: BakeryMode,
    current_action_started_unix: Option<u64>,
    current_apply_phase: Option<String>,
    current_apply_total_tc_commands: usize,
    current_apply_completed_tc_commands: usize,
    current_apply_total_chunks: usize,
    current_apply_completed_chunks: usize,
    last_success_unix: Option<u64>,
    last_full_reload_success_unix: Option<u64>,
    last_failure_unix: Option<u64>,
    last_failure_summary: Option<String>,
    last_apply_type: BakeryApplyType,
    last_total_tc_commands: usize,
    last_class_commands: usize,
    last_qdisc_commands: usize,
    last_build_duration_ms: u64,
    last_apply_duration_ms: u64,
    runtime_operations: BakeryRuntimeOperationsSnapshot,
    queue_distribution: Vec<BakeryQueueDistributionSnapshot>,
    live_capacity_interfaces: Vec<BakeryLiveCapacityInterfaceSnapshot>,
    live_capacity_updated_at_unix: Option<u64>,
    preflight: Option<BakeryPreflightSnapshot>,
    reload_required: bool,
    reload_required_reason: Option<String>,
    dirty_subtree_count: usize,
    runtime_operations_by_site: HashMap<i64, RuntimeNodeOperationSnapshot>,
    runtime_branch_states_by_site: HashMap<i64, BakeryRuntimeNodeBranchSnapshot>,
    activity: VecDeque<BakeryActivityEntry>,
}

struct GroupedBakeryEventState {
    event: String,
    status: String,
    emitted: usize,
    suppressed: usize,
    suppression_summary: String,
}

#[derive(Default)]
struct GroupedBakeryEventLimiter {
    groups: BTreeMap<String, GroupedBakeryEventState>,
}

impl GroupedBakeryEventLimiter {
    fn emit_with_site_name(
        &mut self,
        group_key: impl Into<String>,
        event: &str,
        status: &str,
        site: Option<(i64, Option<String>)>,
        summary: String,
        suppression_summary: String,
    ) {
        let (site_hash, site_name) = site.map_or((None, None), |(site_hash, site_name)| {
            (Some(site_hash), site_name)
        });
        let state =
            self.groups
                .entry(group_key.into())
                .or_insert_with(|| GroupedBakeryEventState {
                    event: event.to_string(),
                    status: status.to_string(),
                    emitted: 0,
                    suppressed: 0,
                    suppression_summary,
                });
        if state.emitted < BAKERY_GROUPED_EVENT_DETAIL_LIMIT {
            push_bakery_event_with_site_name(event, status, site_hash, site_name, summary);
            state.emitted += 1;
        } else {
            state.suppressed += 1;
        }
    }

    fn flush(self) {
        for state in self.groups.into_values() {
            if state.suppressed == 0 {
                continue;
            }
            push_bakery_event(
                &state.event,
                &state.status,
                format!(
                    "Suppressed {} additional {} this batch.",
                    state.suppressed, state.suppression_summary
                ),
            );
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct BakeryApplyMetrics<'a> {
    apply_type: BakeryApplyType,
    summary: &'a str,
    build_duration_ms: u64,
    apply_duration_ms: u64,
    total_tc_commands: usize,
    class_commands: usize,
    qdisc_commands: usize,
    ok: bool,
}

impl Default for BakeryTelemetryState {
    fn default() -> Self {
        Self {
            mode: BakeryMode::Idle,
            current_action_started_unix: None,
            current_apply_phase: None,
            current_apply_total_tc_commands: 0,
            current_apply_completed_tc_commands: 0,
            current_apply_total_chunks: 0,
            current_apply_completed_chunks: 0,
            last_success_unix: None,
            last_full_reload_success_unix: None,
            last_failure_unix: None,
            last_failure_summary: None,
            last_apply_type: BakeryApplyType::None,
            last_total_tc_commands: 0,
            last_class_commands: 0,
            last_qdisc_commands: 0,
            last_build_duration_ms: 0,
            last_apply_duration_ms: 0,
            runtime_operations: BakeryRuntimeOperationsSnapshot {
                submitted_count: 0,
                deferred_count: 0,
                applying_count: 0,
                awaiting_cleanup_count: 0,
                failed_count: 0,
                blocked_count: 0,
                dirty_count: 0,
                latest: None,
            },
            queue_distribution: Vec::new(),
            live_capacity_interfaces: Vec::new(),
            live_capacity_updated_at_unix: None,
            preflight: None,
            reload_required: false,
            reload_required_reason: None,
            dirty_subtree_count: 0,
            runtime_operations_by_site: HashMap::new(),
            runtime_branch_states_by_site: HashMap::new(),
            activity: VecDeque::with_capacity(BAKERY_EVENT_LIMIT),
        }
    }
}

static BAKERY_TELEMETRY: OnceLock<RwLock<BakeryTelemetryState>> = OnceLock::new();

/// Message Queue sender for the bakery
pub static BAKERY_SENDER: OnceLock<Sender<BakeryCommands>> = OnceLock::new();
static MQ_CREATED: AtomicBool = AtomicBool::new(false);
static SHAPING_TREE_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Indicates that at least one command batch has been processed and applied.
/// Used to avoid racing live activation against initial class creation.
static FIRST_COMMIT_APPLIED: AtomicBool = AtomicBool::new(false);

struct FullReloadScope;

impl Drop for FullReloadScope {
    fn drop(&mut self) {
        FULL_RELOAD_IN_PROGRESS.store(false, Ordering::Relaxed);
        let mut state = telemetry_state().write();
        clear_bakery_apply_progress(&mut state);
        if state.mode == BakeryMode::ApplyingFullReload {
            state.mode = BakeryMode::Idle;
            state.current_action_started_unix = None;
        }
    }
}

fn telemetry_state() -> &'static RwLock<BakeryTelemetryState> {
    BAKERY_TELEMETRY.get_or_init(|| RwLock::new(BakeryTelemetryState::default()))
}

fn count_tc_command_types(commands: &[Vec<String>]) -> (usize, usize, usize) {
    let total = commands.len();
    let mut class_count = 0usize;
    let mut qdisc_count = 0usize;
    for argv in commands {
        if let Some(kind) = argv.first() {
            match kind.as_str() {
                "class" => class_count += 1,
                "qdisc" => qdisc_count += 1,
                _ => {}
            }
        }
    }
    (total, class_count, qdisc_count)
}

fn push_bakery_event(event: &str, status: &str, summary: String) {
    push_bakery_event_with_site(event, status, None, summary);
}

fn push_bakery_event_with_site(event: &str, status: &str, site_hash: Option<i64>, summary: String) {
    push_bakery_event_with_site_name(event, status, site_hash, None, summary);
}

fn push_bakery_event_with_site_name(
    event: &str,
    status: &str,
    site_hash: Option<i64>,
    site_name: Option<String>,
    summary: String,
) {
    let entry = BakeryActivityEntry {
        ts: current_timestamp(),
        event: event.to_string(),
        status: status.to_string(),
        site_hash,
        site_name,
        summary,
    };
    let mut state = telemetry_state().write();
    state.activity.push_front(entry);
    while state.activity.len() > BAKERY_EVENT_LIMIT {
        state.activity.pop_back();
    }
}

fn announce_full_reload(summary: &str) {
    warn!("{summary}");
    push_bakery_event("full_reload_trigger", "warning", summary.to_string());
}

fn clear_bakery_apply_progress(state: &mut BakeryTelemetryState) {
    state.current_apply_phase = None;
    state.current_apply_total_tc_commands = 0;
    state.current_apply_completed_tc_commands = 0;
    state.current_apply_total_chunks = 0;
    state.current_apply_completed_chunks = 0;
}

fn mark_reload_required(summary: String) {
    let mut should_emit = false;
    {
        let mut state = telemetry_state().write();
        if !state.reload_required
            || state.reload_required_reason.as_deref() != Some(summary.as_str())
        {
            state.reload_required = true;
            state.reload_required_reason = Some(summary.clone());
            should_emit = true;
        }
    }
    if should_emit {
        push_bakery_event("reload_required", "error", summary);
    }
}

fn is_live_migration_reload_required_reason(reason: &str) -> bool {
    reason.starts_with("Bakery live-move ") || reason.starts_with("Bakery live migration ")
}

fn cancel_pending_migrations_for_observe_mode(
    migrations: &mut HashMap<i64, Migration>,
    reason: &str,
) -> usize {
    let canceled = migrations.len();
    if canceled > 0 {
        let summary = format!("Bakery canceled {canceled} pending live migration(s): {reason}");
        info!("{summary}");
        push_bakery_event("live_migrations_canceled", "info", summary);
        migrations.clear();
    }

    if let Some(reload_reason) = bakery_reload_required_reason()
        && is_live_migration_reload_required_reason(&reload_reason)
    {
        clear_reload_required(
            "Observe mode canceled pending Bakery live-migration verification state; incremental live-move reload requirements were cleared.",
        );
    }

    canceled
}

fn clear_reload_required(summary: &str) {
    let mut should_emit = false;
    {
        let mut state = telemetry_state().write();
        if state.reload_required || state.reload_required_reason.is_some() {
            state.reload_required = false;
            state.reload_required_reason = None;
            should_emit = true;
        }
    }
    if should_emit {
        push_bakery_event("reload_required_cleared", "info", summary.to_string());
    }
}

fn mark_bakery_action_started(mode: BakeryMode, event: &str, summary: String) {
    let ts = current_timestamp();
    {
        let mut state = telemetry_state().write();
        state.mode = mode;
        state.current_action_started_unix = Some(ts);
        clear_bakery_apply_progress(&mut state);
    }
    push_bakery_event(event, "info", summary);
}

fn update_bakery_apply_progress(
    phase: Option<&str>,
    total_tc_commands: usize,
    completed_tc_commands: usize,
    total_chunks: usize,
    completed_chunks: usize,
) {
    let mut state = telemetry_state().write();
    state.current_apply_phase = phase.map(str::to_string);
    state.current_apply_total_tc_commands = total_tc_commands;
    state.current_apply_completed_tc_commands = completed_tc_commands;
    state.current_apply_total_chunks = total_chunks;
    state.current_apply_completed_chunks = completed_chunks;
}

fn mark_bakery_action_finished(metrics: BakeryApplyMetrics<'_>) {
    let ts = current_timestamp();
    {
        let mut state = telemetry_state().write();
        state.last_apply_type = metrics.apply_type;
        state.last_total_tc_commands = metrics.total_tc_commands;
        state.last_class_commands = metrics.class_commands;
        state.last_qdisc_commands = metrics.qdisc_commands;
        state.last_build_duration_ms = metrics.build_duration_ms;
        state.last_apply_duration_ms = metrics.apply_duration_ms;
        if metrics.ok {
            state.last_success_unix = Some(ts);
            if metrics.apply_type == BakeryApplyType::FullReload {
                state.last_full_reload_success_unix = Some(ts);
            }
        } else {
            state.last_failure_unix = Some(ts);
            state.last_failure_summary = Some(metrics.summary.to_string());
        }
        clear_bakery_apply_progress(&mut state);
        if state.mode != BakeryMode::ApplyingFullReload {
            state.mode = BakeryMode::Idle;
            state.current_action_started_unix = None;
        }
    }
    push_bakery_event(
        if metrics.ok {
            "apply_finished"
        } else {
            "apply_failed"
        },
        if metrics.ok { "info" } else { "error" },
        metrics.summary.to_string(),
    );
}

/// Returns the latest Bakery status snapshot for UI/status consumers.
pub fn bakery_status_snapshot() -> BakeryStatusSnapshot {
    let state = telemetry_state().read().clone();
    let tc_io = tc_io_cadence_snapshot();
    BakeryStatusSnapshot {
        mode: state.mode,
        current_action_started_unix: state.current_action_started_unix,
        current_apply_phase: state.current_apply_phase,
        current_apply_total_tc_commands: state.current_apply_total_tc_commands,
        current_apply_completed_tc_commands: state.current_apply_completed_tc_commands,
        current_apply_total_chunks: state.current_apply_total_chunks,
        current_apply_completed_chunks: state.current_apply_completed_chunks,
        active_circuits: ACTIVE_CIRCUITS.load(Ordering::Relaxed),
        last_success_unix: state.last_success_unix,
        last_full_reload_success_unix: state.last_full_reload_success_unix,
        last_failure_unix: state.last_failure_unix,
        last_failure_summary: state.last_failure_summary,
        last_apply_type: state.last_apply_type,
        last_total_tc_commands: state.last_total_tc_commands,
        last_class_commands: state.last_class_commands,
        last_qdisc_commands: state.last_qdisc_commands,
        last_build_duration_ms: state.last_build_duration_ms,
        last_apply_duration_ms: state.last_apply_duration_ms,
        avg_tc_io_interval_ms: tc_io.avg_interval_ms,
        last_tc_io_unix: tc_io.last_event_unix,
        tc_io_interval_samples: tc_io.sample_count,
        runtime_operations: state.runtime_operations,
        queue_distribution: state.queue_distribution,
        live_capacity_interfaces: state.live_capacity_interfaces,
        live_capacity_safe_budget: SAFE_QDISC_BUDGET_PER_INTERFACE,
        live_capacity_updated_at_unix: state.live_capacity_updated_at_unix,
        preflight: state.preflight,
        reload_required: state.reload_required,
        reload_required_reason: state.reload_required_reason,
        dirty_subtree_count: state.dirty_subtree_count,
    }
}

/// Returns the latest Bakery-tracked runtime node-operation snapshot for a site, if any.
pub fn bakery_runtime_node_operation_snapshot(
    site_hash: i64,
) -> Option<BakeryRuntimeNodeOperationSnapshot> {
    telemetry_state()
        .read()
        .runtime_operations_by_site
        .get(&site_hash)
        .cloned()
}

/// Returns the latest Bakery-tracked runtime branch-state snapshot for a site, if any.
pub fn bakery_runtime_node_branch_snapshot(
    site_hash: i64,
) -> Option<BakeryRuntimeNodeBranchSnapshot> {
    telemetry_state()
        .read()
        .runtime_branch_states_by_site
        .get(&site_hash)
        .cloned()
}

/// Returns every retained Bakery runtime branch-state snapshot currently tracked.
pub fn bakery_runtime_node_branch_snapshots() -> Vec<BakeryRuntimeNodeBranchSnapshot> {
    telemetry_state()
        .read()
        .runtime_branch_states_by_site
        .values()
        .cloned()
        .collect()
}

/// Returns the current Bakery reload-required reason, if runtime drift has frozen incremental
/// topology mutation.
pub fn bakery_reload_required_reason() -> Option<String> {
    let state = telemetry_state().read();
    if state.reload_required {
        return state.reload_required_reason.clone();
    }
    None
}

/// Returns recent Bakery activity entries for UI/status consumers.
pub fn bakery_activity_snapshot() -> Vec<BakeryActivityEntry> {
    telemetry_state().read().activity.iter().cloned().collect()
}

fn configured_live_capacity_interfaces(config: &Config) -> Vec<String> {
    let mut interfaces = BTreeSet::new();
    interfaces.insert(config.isp_interface());
    interfaces.insert(config.internet_interface());
    interfaces.into_iter().collect()
}

fn refresh_live_capacity_snapshot(config: &Config, force: bool) {
    let now_unix = current_timestamp();
    {
        let state = telemetry_state().read();
        if !force
            && state
                .live_capacity_updated_at_unix
                .is_some_and(|last| now_unix.saturating_sub(last) < LIVE_CAPACITY_REFRESH_SECONDS)
        {
            return;
        }
    }

    let interfaces = configured_live_capacity_interfaces(config);
    let mut snapshot = Vec::with_capacity(interfaces.len());
    for interface in interfaces {
        let live_qdiscs = read_live_qdisc_handle_majors(&interface)
            .map(|handles| handles.len())
            .unwrap_or(0);
        snapshot.push(BakeryLiveCapacityInterfaceSnapshot {
            name: interface,
            live_qdiscs,
        });
    }

    let mut state = telemetry_state().write();
    state.live_capacity_interfaces = snapshot;
    state.live_capacity_updated_at_unix = Some(now_unix);
}

/// Stores the latest qdisc-budget preflight result for UI/status consumers.
///
/// This function is not pure: it updates retained in-memory Bakery telemetry state.
pub fn record_qdisc_preflight_snapshot(snapshot: BakeryPreflightSnapshot) {
    let ok = snapshot.ok;
    let summary = snapshot.message.clone();
    {
        let mut state = telemetry_state().write();
        state.preflight = Some(snapshot);
    }
    push_bakery_event(
        if ok {
            "preflight_ok"
        } else {
            "preflight_blocked"
        },
        if ok { "info" } else { "warning" },
        summary,
    );
}

fn summarize_apply_result(purpose: &str, result: &ExecuteResult) -> String {
    match &result.failure_summary {
        Some(summary) if !summary.is_empty() => format!("{purpose}: {summary}"),
        _ if result.ok => format!("{purpose}: completed"),
        _ => format!("{purpose}: failed"),
    }
}

fn consume_test_fault_once_from_path(path: &Path, purpose: &str) -> Option<String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return None;
    };
    let needle = raw.trim();
    if needle.is_empty() || !(needle == "*" || purpose.contains(needle)) {
        return None;
    }
    let _ = std::fs::remove_file(path);
    Some(format!(
        "synthetic Bakery test fault for purpose '{purpose}' matched selector '{needle}'"
    ))
}

fn consume_test_fault_once(purpose: &str) -> Option<String> {
    consume_test_fault_once_from_path(Path::new(TEST_FAULT_ONCE_PATH), purpose)
}

fn execute_and_record_live_change(command_buffer: &[Vec<String>], purpose: &str) -> ExecuteResult {
    let (total, class_commands, qdisc_commands) = count_tc_command_types(command_buffer);
    mark_bakery_action_started(
        BakeryMode::ApplyingLiveChange,
        "live_change_started",
        format!("{purpose}: started"),
    );
    let result = if let Some(fault_summary) = consume_test_fault_once(purpose) {
        warn!("Bakery test fault injected: {fault_summary}");
        ExecuteResult {
            ok: false,
            duration_ms: 0,
            failure_summary: Some(fault_summary),
        }
    } else {
        execute_in_memory(command_buffer, purpose)
    };
    let summary = summarize_apply_result(purpose, &result);
    mark_bakery_action_finished(BakeryApplyMetrics {
        apply_type: BakeryApplyType::LiveChange,
        summary: &summary,
        build_duration_ms: 0,
        apply_duration_ms: result.duration_ms,
        total_tc_commands: total,
        class_commands,
        qdisc_commands,
        ok: result.ok,
    });
    if result.ok
        && let Ok(config) = lqos_config::load_config()
    {
        refresh_live_capacity_snapshot(&config, true);
    }
    result
}

/// Estimated qdisc-handle usage for a planned full reload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QdiscInterfaceEstimate {
    /// Total planned qdiscs for the interface.
    pub planned_qdiscs: usize,
    /// Planned infrastructure qdiscs (`mq`, `htb`, etc.) for the interface.
    pub infra_qdiscs: usize,
    /// Planned CAKE leaf qdiscs for the interface.
    pub cake_qdiscs: usize,
    /// Planned fq_codel leaf qdiscs for the interface.
    pub fq_codel_qdiscs: usize,
    /// Estimated kernel memory cost for the interface's planned qdiscs.
    pub estimated_memory_bytes: u64,
}

/// Estimated qdisc budget and conservative memory-safety forecast for a planned full reload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QdiscBudgetEstimate {
    /// Estimated total qdisc count grouped by network interface name.
    pub interfaces: BTreeMap<String, usize>,
    /// Detailed per-interface qdisc breakdown.
    pub interface_details: BTreeMap<String, QdiscInterfaceEstimate>,
    /// Conservative operational limit enforced before a full reload is committed.
    pub safe_budget: usize,
    /// Kernel hard limit for the per-device qdisc-handle namespace.
    pub hard_limit: usize,
    /// Estimated total kernel memory cost of the planned qdisc model.
    pub estimated_total_memory_bytes: u64,
    /// Current host memory snapshot used for preflight, if available.
    pub memory_snapshot: Option<MemorySnapshot>,
    /// Whether the memory preflight passed.
    pub memory_ok: bool,
}

impl QdiscBudgetEstimate {
    /// Returns `true` when all planned per-interface counts fit within the safe budget.
    pub fn ok(&self) -> bool {
        self.interfaces
            .values()
            .all(|count| *count <= self.safe_budget)
            && self.memory_ok
    }
}

fn find_arg_value<'a>(argv: &'a [String], key: &str) -> Option<&'a str> {
    argv.windows(2)
        .find_map(|pair| (pair[0] == key).then_some(pair[1].as_str()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlannedQdiscKind {
    Infra,
    Cake,
    FqCodel,
}

fn planned_qdisc_kind(argv: &[String]) -> Option<PlannedQdiscKind> {
    if argv.len() < 2 || argv[0] != "qdisc" {
        return None;
    }
    if !matches!(argv[1].as_str(), "add" | "replace") {
        return None;
    }

    if argv.iter().any(|arg| arg == "cake") {
        return Some(if planned_qdisc_is_leaf(argv) {
            PlannedQdiscKind::Cake
        } else {
            PlannedQdiscKind::Infra
        });
    }
    if argv.iter().any(|arg| arg == "fq_codel") {
        return Some(if planned_qdisc_is_leaf(argv) {
            PlannedQdiscKind::FqCodel
        } else {
            PlannedQdiscKind::Infra
        });
    }
    Some(PlannedQdiscKind::Infra)
}

fn planned_qdisc_is_leaf(argv: &[String]) -> bool {
    let Some(parent) = find_arg_value(argv, "parent") else {
        return false;
    };
    let Ok(parent_handle) = TcHandle::from_string(parent) else {
        return false;
    };
    let (_, minor) = parent_handle.get_major_minor();
    minor != 1 && minor != 2
}

fn qdisc_kind_estimated_memory_bytes(kind: PlannedQdiscKind) -> u64 {
    match kind {
        PlannedQdiscKind::Infra => INFRA_QDISC_ESTIMATED_MEMORY_BYTES,
        PlannedQdiscKind::Cake => CAKE_QDISC_ESTIMATED_MEMORY_BYTES,
        PlannedQdiscKind::FqCodel => FQ_CODEL_QDISC_ESTIMATED_MEMORY_BYTES,
    }
}

fn planned_qdisc_identity(argv: &[String]) -> Option<(String, String)> {
    if argv.len() < 2 || argv[0] != "qdisc" {
        return None;
    }
    if !matches!(argv[1].as_str(), "add" | "replace") {
        return None;
    }

    let dev = find_arg_value(argv, "dev")?.to_string();
    if let Some(handle) = find_arg_value(argv, "handle") {
        return Some((dev, format!("handle:{handle}")));
    }
    let parent = find_arg_value(argv, "parent")?.to_string();
    Some((dev, format!("parent:{parent}")))
}

/// Estimates total qdisc usage for the current full-reload builder queue.
///
/// This expands the queued Bakery commands through the same builder-path `tc`
/// argv generation used by a structural full reload, then counts the resulting
/// qdiscs per device. Explicit qdisc handles are deduplicated by `handle`,
/// while any remaining auto-handled paths are conservatively deduplicated by
/// `parent`.
pub fn estimate_full_reload_auto_qdisc_budget(
    config: &Arc<Config>,
    queue: &[BakeryCommands],
) -> QdiscBudgetEstimate {
    let mut interfaces = BTreeMap::new();
    let mut interface_details: BTreeMap<String, QdiscInterfaceEstimate> = BTreeMap::new();
    let mut seen_qdiscs = HashSet::new();

    for command in queue {
        let Some(builder_commands) = command.to_commands(config, ExecutionMode::Builder) else {
            continue;
        };
        for argv in builder_commands {
            let Some((dev, identity)) = planned_qdisc_identity(&argv) else {
                continue;
            };
            if seen_qdiscs.insert((dev.clone(), identity)) {
                let kind = planned_qdisc_kind(&argv).unwrap_or(PlannedQdiscKind::Infra);
                *interfaces.entry(dev.clone()).or_insert(0) += 1;
                let detail =
                    interface_details
                        .entry(dev)
                        .or_insert_with(|| QdiscInterfaceEstimate {
                            planned_qdiscs: 0,
                            infra_qdiscs: 0,
                            cake_qdiscs: 0,
                            fq_codel_qdiscs: 0,
                            estimated_memory_bytes: 0,
                        });
                detail.planned_qdiscs += 1;
                match kind {
                    PlannedQdiscKind::Infra => detail.infra_qdiscs += 1,
                    PlannedQdiscKind::Cake => detail.cake_qdiscs += 1,
                    PlannedQdiscKind::FqCodel => detail.fq_codel_qdiscs += 1,
                }
                detail.estimated_memory_bytes = detail
                    .estimated_memory_bytes
                    .saturating_add(qdisc_kind_estimated_memory_bytes(kind));
            }
        }
    }

    let estimated_total_memory_bytes = interface_details.values().fold(0u64, |acc, detail| {
        acc.saturating_add(detail.estimated_memory_bytes)
    });
    let memory_snapshot = read_memory_snapshot().ok();
    let memory_ok = memory_snapshot.as_ref().is_none_or(|snapshot| {
        snapshot
            .available_bytes
            .saturating_sub(BAKERY_MEMORY_GUARD_MIN_AVAILABLE_BYTES)
            >= estimated_total_memory_bytes
    });

    QdiscBudgetEstimate {
        interfaces,
        interface_details,
        safe_budget: SAFE_QDISC_BUDGET_PER_INTERFACE,
        hard_limit: HARD_QDISC_HANDLE_LIMIT_PER_INTERFACE,
        estimated_total_memory_bytes,
        memory_snapshot,
        memory_ok,
    }
}

fn desired_shaping_tree_active(config: &Arc<Config>) -> bool {
    !config.queues.queue_mode.is_observe()
}

fn live_tree_mutation_blocker_for_config(config: &Arc<Config>) -> Option<String> {
    if FULL_RELOAD_IN_PROGRESS.load(Ordering::Relaxed) {
        return Some("a full reload is currently in progress".to_string());
    }
    if config.queues.queue_mode.is_observe() {
        return Some(
            "queue_mode is observe; root MQ is retained but the shaping tree is not live"
                .to_string(),
        );
    }
    if !SHAPING_TREE_ACTIVE.load(Ordering::Relaxed) {
        return Some("the shaping tree is not currently active".to_string());
    }
    None
}

const ROOT_MQ_MAJOR: u16 = 0x7fff;

fn managed_interfaces_for_config(config: &Arc<Config>) -> Vec<String> {
    let mut interfaces = vec![config.isp_interface()];
    if !config.on_a_stick_mode() {
        interfaces.push(config.internet_interface());
    }
    interfaces
}

fn retained_root_mq_entry(snapshot: &[LiveTcQdiscEntry]) -> Option<&LiveTcQdiscEntry> {
    snapshot.iter().find(|entry| {
        entry.is_root
            && entry.kind == "mq"
            && entry
                .handle
                .is_some_and(|handle| handle.get_major_minor().0 == ROOT_MQ_MAJOR)
    })
}

fn managed_root_child_parent_handles(snapshot: &[LiveTcQdiscEntry]) -> HashSet<TcHandle> {
    snapshot
        .iter()
        .filter(|entry| {
            entry
                .parent
                .is_some_and(|parent| parent.get_major_minor().0 == ROOT_MQ_MAJOR)
                && entry
                    .handle
                    .is_some_and(|handle| handle.get_major_minor().0 != 0)
        })
        .filter_map(|entry| entry.parent)
        .collect()
}

fn verify_root_mq_snapshot(snapshot: &[LiveTcQdiscEntry], interface: &str) -> Result<(), String> {
    if retained_root_mq_entry(snapshot).is_none() {
        return Err(format!(
            "interface {interface} does not currently have root mq handle 7fff:"
        ));
    }
    Ok(())
}

fn verify_clean_root_child_tree(
    qdisc_snapshot: &[LiveTcQdiscEntry],
    class_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    interface: &str,
) -> Result<(), String> {
    verify_root_mq_snapshot(qdisc_snapshot, interface)?;

    let child_handles = managed_root_child_parent_handles(qdisc_snapshot);
    if !child_handles.is_empty() {
        return Err(format!(
            "interface {interface} still has {} managed child qdisc(s) beneath root mq 7fff:",
            child_handles.len()
        ));
    }

    let managed_class_count = class_snapshot
        .values()
        .filter(|entry| entry.class_id.get_major_minor().0 != ROOT_MQ_MAJOR)
        .count();
    if managed_class_count != 0 {
        return Err(format!(
            "interface {interface} still has {} managed tc class(es) after root-child prune",
            managed_class_count
        ));
    }

    Ok(())
}

fn root_replace_failure_is_fallbackable(summary: &str) -> bool {
    let normalized = summary.to_ascii_lowercase();
    normalized.contains("exclusivity flag on, cannot modify")
        || normalized.contains("rtnetlink answers: file exists")
        || normalized.contains("file exists")
}

fn run_root_preflight_commands(commands: &[Vec<String>], purpose: &str) -> Result<(), String> {
    let result = execute_in_memory_chunked(commands, purpose, 1, None, |_, _, _, _| {});
    if result.ok {
        Ok(())
    } else {
        Err(result
            .failure_summary
            .unwrap_or_else(|| format!("{purpose} failed")))
    }
}

fn root_mq_replace_command(interface_name: &str) -> Vec<String> {
    vec![
        "qdisc".to_string(),
        "replace".to_string(),
        "dev".to_string(),
        interface_name.to_string(),
        "root".to_string(),
        "handle".to_string(),
        "7FFF:".to_string(),
        "mq".to_string(),
    ]
}

fn root_mq_delete_command(interface_name: &str) -> Vec<String> {
    vec![
        "qdisc".to_string(),
        "del".to_string(),
        "dev".to_string(),
        interface_name.to_string(),
        "root".to_string(),
    ]
}

fn root_mq_add_command(interface_name: &str) -> Vec<String> {
    vec![
        "qdisc".to_string(),
        "add".to_string(),
        "dev".to_string(),
        interface_name.to_string(),
        "root".to_string(),
        "handle".to_string(),
        "7FFF:".to_string(),
        "mq".to_string(),
    ]
}

fn prepare_root_mq_for_full_reload(config: &Arc<Config>) -> Result<(), String> {
    for interface in managed_interfaces_for_config(config) {
        let qdisc_snapshot = read_live_qdisc_snapshot(&interface)?;
        if retained_root_mq_entry(&qdisc_snapshot).is_some() {
            let prune_commands = managed_root_child_parent_handles(&qdisc_snapshot)
                .into_iter()
                .map(|parent| {
                    vec![
                        "qdisc".to_string(),
                        "del".to_string(),
                        "dev".to_string(),
                        interface.clone(),
                        "parent".to_string(),
                        parent.as_tc_string(),
                    ]
                })
                .collect::<Vec<_>>();
            if !prune_commands.is_empty() {
                run_root_preflight_commands(
                    &prune_commands,
                    &format!("full reload retained-root child prune on {interface}"),
                )?;
                invalidate_live_tc_snapshots();
            }

            let pruned_qdisc_snapshot = read_live_qdisc_snapshot(&interface)?;
            let pruned_class_snapshot = read_live_class_snapshot(&interface)?;
            if verify_clean_root_child_tree(
                &pruned_qdisc_snapshot,
                &pruned_class_snapshot,
                &interface,
            )
            .is_ok()
            {
                continue;
            }
        }

        let replace_summary = run_root_preflight_commands(
            &[root_mq_replace_command(&interface)],
            &format!("full reload root mq replace on {interface}"),
        )
        .err();
        invalidate_live_tc_snapshots();

        if let Some(summary) = replace_summary {
            if !root_replace_failure_is_fallbackable(&summary) {
                return Err(summary);
            }
        } else {
            let replaced_qdisc_snapshot = read_live_qdisc_snapshot(&interface)?;
            let replaced_class_snapshot = read_live_class_snapshot(&interface)?;
            if verify_clean_root_child_tree(
                &replaced_qdisc_snapshot,
                &replaced_class_snapshot,
                &interface,
            )
            .is_ok()
            {
                continue;
            }
        }

        run_root_preflight_commands(
            &[root_mq_delete_command(&interface)],
            &format!("full reload root mq delete on {interface}"),
        )?;
        invalidate_live_tc_snapshots();
        run_root_preflight_commands(
            &[root_mq_add_command(&interface)],
            &format!("full reload root mq add on {interface}"),
        )?;
        invalidate_live_tc_snapshots();

        let recovered_qdisc_snapshot = read_live_qdisc_snapshot(&interface)?;
        let recovered_class_snapshot = read_live_class_snapshot(&interface)?;
        verify_clean_root_child_tree(
            &recovered_qdisc_snapshot,
            &recovered_class_snapshot,
            &interface,
        )?;
    }

    Ok(())
}

fn live_tree_mutations_allowed(config: &Arc<Config>) -> bool {
    live_tree_mutation_blocker_for_config(config).is_none()
}

/// Returns the reason live shaping-tree mutations are currently blocked, if any.
///
/// This function is not pure: it reads runtime config and Bakery shaping-tree state.
pub fn bakery_live_tree_mutation_blocker() -> Option<String> {
    let Ok(config) = lqos_config::load_config() else {
        return Some("configuration could not be loaded".to_string());
    };
    live_tree_mutation_blocker_for_config(&config)
}

/// Overrides Bakery's shaping-tree-active flag for tests and restores callers' access to the
/// previous value.
///
/// This function has side effects: it mutates process-global Bakery runtime state.
#[doc(hidden)]
pub fn set_shaping_tree_active_for_tests(active: bool) -> bool {
    SHAPING_TREE_ACTIVE.swap(active, Ordering::Relaxed)
}

fn current_mq_layout(
    batch: &[Arc<BakeryCommands>],
    config: &Arc<Config>,
    existing: &Option<MqDeviceLayout>,
) -> Option<MqDeviceLayout> {
    let mut latest = existing.clone();
    for command in batch {
        if let BakeryCommands::MqSetup {
            queues_available,
            stick_offset,
        } = command.as_ref()
        {
            latest = Some(MqDeviceLayout::from_setup(
                config,
                *queues_available,
                *stick_offset,
            ));
        }
    }
    latest
}

#[cfg_attr(not(test), allow(dead_code))]
fn with_assigned_qdisc_handles(
    command: &Arc<BakeryCommands>,
    config: &Arc<Config>,
    mq_layout: &MqDeviceLayout,
    qdisc_handles: &mut QdiscHandleState,
) -> Arc<BakeryCommands> {
    with_assigned_qdisc_handles_reserved(command, config, mq_layout, qdisc_handles, &HashMap::new())
}

fn with_assigned_qdisc_handles_reserved(
    command: &Arc<BakeryCommands>,
    config: &Arc<Config>,
    mq_layout: &MqDeviceLayout,
    qdisc_handles: &mut QdiscHandleState,
    extra_reserved_handles: &HashMap<String, HashSet<u16>>,
) -> Arc<BakeryCommands> {
    let BakeryCommands::AddCircuit { circuit_hash, .. } = command.as_ref() else {
        return Arc::clone(command);
    };

    if config.queues.queue_mode.is_observe() {
        return Arc::clone(command);
    }

    let mut enriched = command.as_ref().clone();
    let isp_interface = config.isp_interface();
    let internet_interface = config.internet_interface();
    let mut isp_reserved = mq_layout.reserved_handles(&isp_interface);
    if let Some(extra) = extra_reserved_handles.get(&isp_interface) {
        isp_reserved.extend(extra.iter().copied());
    }
    let mut up_reserved = mq_layout.reserved_handles(&internet_interface);
    if let Some(extra) = extra_reserved_handles.get(&internet_interface) {
        up_reserved.extend(extra.iter().copied());
    }

    if let BakeryCommands::AddCircuit {
        down_qdisc_handle,
        up_qdisc_handle,
        ..
    } = &mut enriched
    {
        if down_qdisc_handle.is_none() {
            *down_qdisc_handle =
                qdisc_handles.assign_circuit_handle(&isp_interface, *circuit_hash, &isp_reserved);
        }
        if !config.on_a_stick_mode() && up_qdisc_handle.is_none() {
            *up_qdisc_handle = qdisc_handles.assign_circuit_handle(
                &internet_interface,
                *circuit_hash,
                &up_reserved,
            );
        }
    }

    Arc::new(enriched)
}

fn assign_fresh_qdisc_handles_reserved(
    command: &Arc<BakeryCommands>,
    config: &Arc<Config>,
    mq_layout: &MqDeviceLayout,
    qdisc_handles: &mut QdiscHandleState,
    extra_reserved_handles: &HashMap<String, HashSet<u16>>,
) -> Result<Arc<BakeryCommands>, String> {
    let BakeryCommands::AddCircuit { circuit_hash, .. } = command.as_ref() else {
        return Ok(Arc::clone(command));
    };

    if config.queues.queue_mode.is_observe() {
        return Ok(Arc::clone(command));
    }

    let (down_parent, up_parent) = effective_directional_qdisc_parents(command.as_ref(), config);
    let mut refreshed = command.as_ref().clone();
    let isp_interface = config.isp_interface();
    let internet_interface = config.internet_interface();
    let mut isp_reserved = mq_layout.reserved_handles(&isp_interface);
    if let Some(extra) = extra_reserved_handles.get(&isp_interface) {
        isp_reserved.extend(extra.iter().copied());
    }
    let mut up_reserved = mq_layout.reserved_handles(&internet_interface);
    if let Some(extra) = extra_reserved_handles.get(&internet_interface) {
        up_reserved.extend(extra.iter().copied());
    }

    if let BakeryCommands::AddCircuit {
        down_qdisc_handle,
        up_qdisc_handle,
        ..
    } = &mut refreshed
    {
        *down_qdisc_handle = if down_parent.is_some() {
            qdisc_handles.rotate_circuit_handle(&isp_interface, *circuit_hash, &isp_reserved)
        } else {
            None
        };
        if down_parent.is_some() && down_qdisc_handle.is_none() {
            return Err(format!(
                "Bakery could not allocate a fresh downlink qdisc handle for restored circuit {}",
                circuit_hash
            ));
        }

        *up_qdisc_handle = if up_parent.is_some() {
            qdisc_handles.rotate_circuit_handle(&internet_interface, *circuit_hash, &up_reserved)
        } else {
            None
        };
        if up_parent.is_some() && up_qdisc_handle.is_none() {
            return Err(format!(
                "Bakery could not allocate a fresh uplink qdisc handle for restored circuit {}",
                circuit_hash
            ));
        }
    }

    Ok(Arc::new(refreshed))
}

fn snapshot_live_qdisc_handle_majors(
    config: &Arc<Config>,
) -> Result<HashMap<String, HashSet<u16>>, String> {
    let mut reserved = HashMap::new();
    let isp_interface = config.isp_interface();
    reserved.insert(
        isp_interface.clone(),
        read_live_qdisc_handle_majors(&isp_interface)?,
    );

    if !config.on_a_stick_mode() {
        let internet_interface = config.internet_interface();
        if internet_interface != isp_interface {
            reserved.insert(
                internet_interface.clone(),
                read_live_qdisc_handle_majors(&internet_interface)?,
            );
        }
    }

    Ok(reserved)
}

fn parse_directional_sqm_override(
    sqm_override: &Option<String>,
) -> (Option<String>, Option<String>) {
    match sqm_override {
        None => (None, None),
        Some(s) => {
            if s.contains('/') {
                let mut it = s.splitn(2, '/');
                let down = it.next().unwrap_or("").trim();
                let up = it.next().unwrap_or("").trim();
                let map = |t: &str| -> Option<String> {
                    if t.is_empty() {
                        None
                    } else {
                        Some(t.to_string())
                    }
                };
                (map(down), map(up))
            } else {
                (Some(s.clone()), Some(s.clone()))
            }
        }
    }
}

fn effective_directional_sqm_kinds(
    command: &BakeryCommands,
    config: &Arc<Config>,
) -> (Option<SqmKind>, Option<SqmKind>) {
    let BakeryCommands::AddCircuit {
        download_bandwidth_max,
        upload_bandwidth_max,
        sqm_override,
        ..
    } = command
    else {
        return (None, None);
    };

    if config.queues.queue_mode.is_observe() {
        return (None, None);
    }

    let (down_override_opt, up_override_opt) = parse_directional_sqm_override(sqm_override);

    let down_kind =
        (!matches!(down_override_opt.as_deref(), Some(s) if s.eq_ignore_ascii_case("none")))
            .then(|| effective_sqm_kind(*download_bandwidth_max, config, &down_override_opt));
    let up_kind = (!config.on_a_stick_mode()
        && !matches!(up_override_opt.as_deref(), Some(s) if s.eq_ignore_ascii_case("none")))
    .then(|| effective_sqm_kind(*upload_bandwidth_max, config, &up_override_opt));

    (down_kind, up_kind)
}

fn effective_directional_qdisc_parents(
    command: &BakeryCommands,
    config: &Arc<Config>,
) -> (Option<TcHandle>, Option<TcHandle>) {
    let BakeryCommands::AddCircuit {
        class_minor,
        class_major,
        up_class_major,
        sqm_override,
        ..
    } = command
    else {
        return (None, None);
    };

    if config.queues.queue_mode.is_observe() {
        return (None, None);
    }

    let (down_override_opt, up_override_opt) = parse_directional_sqm_override(sqm_override);

    let down_parent =
        (!matches!(down_override_opt.as_deref(), Some(s) if s.eq_ignore_ascii_case("none")))
            .then(|| TcHandle::from_u32(((*class_major as u32) << 16) | (*class_minor as u32)));
    let up_parent = (!config.on_a_stick_mode()
        && !matches!(up_override_opt.as_deref(), Some(s) if s.eq_ignore_ascii_case("none")))
    .then(|| TcHandle::from_u32(((*up_class_major as u32) << 16) | (*class_minor as u32)));

    (down_parent, up_parent)
}

#[cfg_attr(not(test), allow(dead_code))]
fn rotate_changed_qdisc_handles(
    previous: &BakeryCommands,
    command: &Arc<BakeryCommands>,
    config: &Arc<Config>,
    mq_layout: &MqDeviceLayout,
    qdisc_handles: &mut QdiscHandleState,
) -> Arc<BakeryCommands> {
    rotate_changed_qdisc_handles_reserved(
        previous,
        command,
        config,
        mq_layout,
        qdisc_handles,
        &HashMap::new(),
    )
}

fn rotate_changed_qdisc_handles_reserved(
    previous: &BakeryCommands,
    command: &Arc<BakeryCommands>,
    config: &Arc<Config>,
    mq_layout: &MqDeviceLayout,
    qdisc_handles: &mut QdiscHandleState,
    extra_reserved_handles: &HashMap<String, HashSet<u16>>,
) -> Arc<BakeryCommands> {
    let (old_down_kind, old_up_kind) = effective_directional_sqm_kinds(previous, config);
    let (new_down_kind, new_up_kind) = effective_directional_sqm_kinds(command.as_ref(), config);
    let (old_down_parent, old_up_parent) = effective_directional_qdisc_parents(previous, config);
    let (new_down_parent, new_up_parent) =
        effective_directional_qdisc_parents(command.as_ref(), config);

    let mut rotated = command.as_ref().clone();
    let isp_interface = config.isp_interface();
    let internet_interface = config.internet_interface();
    let mut isp_reserved = mq_layout.reserved_handles(&isp_interface);
    if let Some(extra) = extra_reserved_handles.get(&isp_interface) {
        isp_reserved.extend(extra.iter().copied());
    }
    let mut up_reserved = mq_layout.reserved_handles(&internet_interface);
    if let Some(extra) = extra_reserved_handles.get(&internet_interface) {
        up_reserved.extend(extra.iter().copied());
    }

    if let BakeryCommands::AddCircuit {
        circuit_hash,
        down_qdisc_handle,
        up_qdisc_handle,
        ..
    } = &mut rotated
    {
        let down_kind_changed =
            old_down_kind.is_some() && new_down_kind.is_some() && old_down_kind != new_down_kind;
        let down_parent_changed = old_down_parent.is_some()
            && new_down_parent.is_some()
            && old_down_parent != new_down_parent;
        let down_handle_conflicts_live = down_qdisc_handle.as_ref().is_some_and(|handle| {
            isp_reserved.contains(handle) && Some(*handle) != previous_down_qdisc_handle(previous)
        });
        if down_kind_changed || down_parent_changed || down_handle_conflicts_live {
            *down_qdisc_handle =
                qdisc_handles.rotate_circuit_handle(&isp_interface, *circuit_hash, &isp_reserved);
        }
        let up_kind_changed =
            old_up_kind.is_some() && new_up_kind.is_some() && old_up_kind != new_up_kind;
        let up_parent_changed =
            old_up_parent.is_some() && new_up_parent.is_some() && old_up_parent != new_up_parent;
        let up_handle_conflicts_live = up_qdisc_handle.as_ref().is_some_and(|handle| {
            up_reserved.contains(handle) && Some(*handle) != previous_up_qdisc_handle(previous)
        });
        if up_kind_changed || up_parent_changed || up_handle_conflicts_live {
            *up_qdisc_handle = qdisc_handles.rotate_circuit_handle(
                &internet_interface,
                *circuit_hash,
                &up_reserved,
            );
        }
    }

    Arc::new(rotated)
}

fn previous_down_qdisc_handle(previous: &BakeryCommands) -> Option<u16> {
    let BakeryCommands::AddCircuit {
        down_qdisc_handle, ..
    } = previous
    else {
        return None;
    };
    *down_qdisc_handle
}

fn previous_up_qdisc_handle(previous: &BakeryCommands) -> Option<u16> {
    let BakeryCommands::AddCircuit {
        up_qdisc_handle, ..
    } = previous
    else {
        return None;
    };
    *up_qdisc_handle
}

fn snapshot_live_qdisc_handle_majors_or_empty(
    config: &Arc<Config>,
    purpose: &str,
) -> HashMap<String, HashSet<u16>> {
    match snapshot_live_qdisc_handle_majors(config) {
        Ok(handles) => handles,
        Err(error) => {
            warn!(
                "Bakery could not snapshot live qdisc handles before {purpose}; proceeding without extra live reservations: {error}"
            );
            HashMap::new()
        }
    }
}

fn runtime_site_label(site_hash: i64, site_name: Option<&str>) -> String {
    match site_name {
        Some(name) if !name.is_empty() => format!("{name} ({site_hash})"),
        _ => site_hash.to_string(),
    }
}

fn runtime_site_display_name(site_hash: i64, site_name: Option<&str>) -> String {
    match site_name {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => site_hash.to_string(),
    }
}

fn runtime_circuit_label(circuit_hash: i64, circuit_name: Option<&str>) -> String {
    match circuit_name {
        Some(name) if !name.is_empty() => format!("{name} ({circuit_hash})"),
        _ => circuit_hash.to_string(),
    }
}

fn migration_target_label(migration: &Migration) -> String {
    let circuit_label =
        runtime_circuit_label(migration.circuit_hash, migration.circuit_name.as_deref());
    match migration.site_name.as_deref() {
        Some(site_name) if !site_name.is_empty() => {
            format!("circuit {circuit_label} at site {site_name}")
        }
        _ => format!("circuit {circuit_label}"),
    }
}

fn runtime_network_json_path(config: &Config) -> std::path::PathBuf {
    let base_path = Path::new(&config.lqos_directory);
    if config
        .long_term_stats
        .enable_insight_topology
        .unwrap_or(false)
    {
        let tmp_path = base_path.join("network.insight.json");
        if tmp_path.exists() {
            return tmp_path;
        }
    }
    base_path.join("network.json")
}

fn runtime_hash_to_i64(text: &str) -> i64 {
    use std::hash::{DefaultHasher, Hasher};

    let mut hasher = DefaultHasher::new();
    hasher.write(text.as_bytes());
    hasher.finish() as i64
}

fn network_json_entry_looks_like_node(node: &Map<String, Value>) -> bool {
    node.get("type")
        .and_then(|value| value.as_str())
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("site")
                || value.eq_ignore_ascii_case("ap")
                || value.eq_ignore_ascii_case("client")
        })
        || node.contains_key("downloadBandwidthMbps")
        || node.contains_key("uploadBandwidthMbps")
        || node.contains_key("children")
}

fn collect_network_json_site_names(map: &Map<String, Value>, names: &mut HashMap<i64, String>) {
    for (name, value) in map {
        let Some(node) = value.as_object() else {
            continue;
        };
        if !network_json_entry_looks_like_node(node) {
            continue;
        }
        names
            .entry(runtime_hash_to_i64(name))
            .or_insert_with(|| name.clone());
        if let Some(Value::Object(children)) = node.get("children") {
            collect_network_json_site_names(children, names);
        }
    }
}

fn load_current_runtime_site_names(config: &Config) -> Option<HashMap<i64, String>> {
    let path = runtime_network_json_path(config);
    let raw = std::fs::read_to_string(&path).ok()?;
    let json: Value = serde_json::from_str(&raw).ok()?;
    let root = json.as_object()?;
    let mut names = HashMap::new();
    collect_network_json_site_names(root, &mut names);
    Some(names)
}

fn resolve_runtime_site_name(
    site_hash: i64,
    current_site_names: Option<&HashMap<i64, String>>,
    virtualized_sites: &HashMap<i64, VirtualizedSiteState>,
) -> Option<String> {
    current_site_names
        .and_then(|names| names.get(&site_hash).cloned())
        .or_else(|| {
            virtualized_sites
                .get(&site_hash)
                .map(|state| state.site_name.clone())
        })
}

fn stale_retained_runtime_branch_summary(
    site_hash: i64,
    saved_state: &VirtualizedSiteState,
    current_site_names: &HashMap<i64, String>,
) -> Option<String> {
    if !current_site_names.contains_key(&site_hash) {
        return Some(format!(
            "Bakery dropped stale retained runtime branch for {} because the backing node is no longer present in current topology.",
            runtime_site_label(site_hash, Some(saved_state.site_name.as_str()))
        ));
    }

    let missing_saved_site = saved_state
        .saved_sites
        .keys()
        .find(|saved_hash| !current_site_names.contains_key(saved_hash))?;
    Some(format!(
        "Bakery dropped stale retained runtime branch for {} because retained child site hash {} is no longer present in current topology.",
        runtime_site_label(site_hash, Some(saved_state.site_name.as_str())),
        missing_saved_site
    ))
}

/// Returns `true` while Bakery is applying a full reload batch to Linux `tc`.
pub fn full_reload_in_progress() -> bool {
    FULL_RELOAD_IN_PROGRESS.load(Ordering::Relaxed)
}

#[derive(Debug, Default, Clone, Copy)]
struct MappedLimitStats {
    enforced_limit: Option<usize>,
    requested_mapped: usize,
    allowed_mapped: usize,
    dropped_mapped: usize,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedMappedLimit {
    licensed: bool,
    max_circuits: Option<usize>,
    effective_limit: Option<usize>,
}

fn format_mapped_limit(limit: Option<usize>) -> String {
    limit
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unlimited".to_string())
}

fn is_mapped_add_circuit(cmd: &BakeryCommands) -> bool {
    let BakeryCommands::AddCircuit { ip_addresses, .. } = cmd else {
        return false;
    };
    !parse_ip_list(ip_addresses).is_empty()
}

fn mapped_circuit_hash(cmd: &BakeryCommands) -> Option<i64> {
    let BakeryCommands::AddCircuit { circuit_hash, .. } = cmd else {
        return None;
    };
    if is_mapped_add_circuit(cmd) {
        Some(*circuit_hash)
    } else {
        None
    }
}

fn resolve_mapped_circuit_limit() -> ResolvedMappedLimit {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            warn!("Bakery: failed to build runtime for Insight license summary: {e:?}");
            return ResolvedMappedLimit {
                licensed: false,
                max_circuits: None,
                effective_limit: Some(DEFAULT_MAPPED_CIRCUITS_LIMIT),
            };
        }
    };

    rt.block_on(async {
        let Ok(mut bus) = LibreqosBusClient::new().await else {
            return ResolvedMappedLimit {
                licensed: false,
                max_circuits: None,
                effective_limit: Some(DEFAULT_MAPPED_CIRCUITS_LIMIT),
            };
        };
        let Ok(reply) = bus
            .request(vec![BusRequest::GetInsightLicenseSummary])
            .await
        else {
            return ResolvedMappedLimit {
                licensed: false,
                max_circuits: None,
                effective_limit: Some(DEFAULT_MAPPED_CIRCUITS_LIMIT),
            };
        };

        for r in reply {
            if let BusResponse::InsightLicenseSummary(InsightLicenseSummary {
                licensed,
                max_circuits,
            }) = r
            {
                let max_circuits_usize = max_circuits.map(|n| {
                    let clamped = std::cmp::min(n, usize::MAX as u64);
                    clamped as usize
                });
                let effective_limit = if licensed {
                    max_circuits_usize
                } else {
                    Some(DEFAULT_MAPPED_CIRCUITS_LIMIT)
                };
                return ResolvedMappedLimit {
                    licensed,
                    max_circuits: max_circuits_usize,
                    effective_limit,
                };
            }
        }
        ResolvedMappedLimit {
            licensed: false,
            max_circuits: None,
            effective_limit: Some(DEFAULT_MAPPED_CIRCUITS_LIMIT),
        }
    })
}

fn filter_batch_by_mapped_circuit_limit(
    batch: Vec<Arc<BakeryCommands>>,
    existing_circuits: &HashMap<i64, Arc<BakeryCommands>>,
    effective_limit: Option<usize>,
) -> (Vec<Arc<BakeryCommands>>, MappedLimitStats) {
    let mut mapped_candidates: Vec<i64> = Vec::new();
    let mut seen = HashSet::new();

    for cmd in &batch {
        if let Some(hash) = mapped_circuit_hash(cmd.as_ref())
            && seen.insert(hash)
        {
            mapped_candidates.push(hash);
        }
    }

    let requested = mapped_candidates.len();
    let Some(effective_limit) = effective_limit else {
        return (
            batch,
            MappedLimitStats {
                enforced_limit: None,
                requested_mapped: requested,
                allowed_mapped: requested,
                dropped_mapped: 0,
            },
        );
    };

    if requested <= effective_limit {
        return (
            batch,
            MappedLimitStats {
                enforced_limit: Some(effective_limit),
                requested_mapped: requested,
                allowed_mapped: requested,
                dropped_mapped: 0,
            },
        );
    }

    let mut keep_set: HashSet<i64> = HashSet::new();

    // Preserve existing mapped circuits first to minimize churn.
    for hash in &mapped_candidates {
        if keep_set.len() >= effective_limit {
            break;
        }
        if existing_circuits
            .get(hash)
            .is_some_and(|existing| is_mapped_add_circuit(existing.as_ref()))
        {
            keep_set.insert(*hash);
        }
    }

    // Fill remaining slots in deterministic batch order.
    for hash in &mapped_candidates {
        if keep_set.len() >= effective_limit {
            break;
        }
        keep_set.insert(*hash);
    }

    let filtered = batch
        .into_iter()
        .filter(|cmd| match mapped_circuit_hash(cmd.as_ref()) {
            Some(hash) => keep_set.contains(&hash),
            None => true,
        })
        .collect::<Vec<_>>();

    let allowed = keep_set.len();
    (
        filtered,
        MappedLimitStats {
            enforced_limit: Some(effective_limit),
            requested_mapped: requested,
            allowed_mapped: allowed,
            dropped_mapped: requested.saturating_sub(allowed),
        },
    )
}

fn maybe_emit_mapped_circuit_limit_urgent(stats: &MappedLimitStats) {
    if stats.dropped_mapped == 0 {
        return;
    }

    let now = current_timestamp();
    let last = LAST_CIRCUIT_LIMIT_URGENT_TS.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < CIRCUIT_LIMIT_URGENT_INTERVAL_SECONDS {
        return;
    }
    LAST_CIRCUIT_LIMIT_URGENT_TS.store(now, Ordering::Relaxed);

    let message = format!(
        "Mapped circuit limit reached: requested {} mapped circuits, allowed {}, dropped {}.",
        stats.requested_mapped, stats.allowed_mapped, stats.dropped_mapped
    );

    let context = Some(format!(
        "{{\"requested_mapped\":{},\"allowed_mapped\":{},\"dropped_mapped\":{},\"enforced_limit\":{}}}",
        stats.requested_mapped,
        stats.allowed_mapped,
        stats.dropped_mapped,
        stats
            .enforced_limit
            .map(|n| n.to_string())
            .unwrap_or_else(|| "null".to_string())
    ));

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            warn!("Bakery: failed to build runtime for urgent issue submission: {e:?}");
            return;
        }
    };
    rt.block_on(async {
        if let Ok(mut bus) = LibreqosBusClient::new().await {
            let _ = bus
                .request(vec![BusRequest::SubmitUrgentIssue {
                    source: UrgentSource::System,
                    severity: UrgentSeverity::Warning,
                    code: "MAPPED_CIRCUIT_LIMIT".to_string(),
                    message,
                    context,
                    dedupe_key: Some("mapped_circuit_limit".to_string()),
                }])
                .await;
        }
    });
}

fn maybe_emit_memory_guard_urgent(summary: &str) {
    if !summary.contains("Bakery memory guard stopped") {
        return;
    }

    let message = summary.to_string();
    let context = read_memory_snapshot().ok().map(|snapshot| {
        format!(
            "{{\"available_bytes\":{},\"total_bytes\":{},\"memory_guard_floor_bytes\":{}}}",
            snapshot.available_bytes, snapshot.total_bytes, BAKERY_MEMORY_GUARD_MIN_AVAILABLE_BYTES
        )
    });

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            warn!("Bakery: failed to build runtime for urgent issue submission: {e:?}");
            return;
        }
    };
    rt.block_on(async {
        if let Ok(mut bus) = LibreqosBusClient::new().await {
            let _ = bus
                .request(vec![BusRequest::SubmitUrgentIssue {
                    source: UrgentSource::System,
                    severity: UrgentSeverity::Error,
                    code: "BAKERY_MEMORY_GUARD".to_string(),
                    message,
                    context,
                    dedupe_key: Some("bakery_memory_guard".to_string()),
                }])
                .await;
        }
    });
}

fn log_mapped_limit_decision(
    context: &str,
    mapped_limit: ResolvedMappedLimit,
    stats: MappedLimitStats,
) {
    warn!(
        "Bakery mapped circuit decision ({}): requested={}, allowed={}, dropped={}, effective_limit={}, licensed={}, max_circuits={:?}",
        context,
        stats.requested_mapped,
        stats.allowed_mapped,
        stats.dropped_mapped,
        format_mapped_limit(stats.enforced_limit),
        mapped_limit.licensed,
        mapped_limit.max_circuits
    );
}

/// Starts the Bakery system, returning a channel sender for sending commands to the Bakery.
pub fn start_bakery() -> anyhow::Result<crossbeam_channel::Sender<BakeryCommands>> {
    let (tx, rx) = crossbeam_channel::bounded(CHANNEL_CAPACITY);
    let inner_sender = tx.clone();
    if BAKERY_SENDER.set(tx.clone()).is_err() {
        return Err(anyhow::anyhow!("Bakery sender is already initialized."));
    }
    std::thread::Builder::new()
        .name("lqos_bakery".to_string())
        .spawn(move || {
            bakery_main(rx, inner_sender);
        })
        .map_err(|e| anyhow::anyhow!("Failed to start Bakery thread: {}", e))?;
    Ok(tx)
}

fn bakery_main(rx: Receiver<BakeryCommands>, tx: Sender<BakeryCommands>) {
    // Current operation batch
    let mut batch: Option<Vec<Arc<BakeryCommands>>> = None;
    let mut sites: HashMap<i64, Arc<BakeryCommands>> = HashMap::new();
    let mut circuits: HashMap<i64, Arc<BakeryCommands>> = HashMap::new();
    let mut live_circuits: HashMap<i64, u64> = HashMap::new();
    let mut mq_layout: Option<MqDeviceLayout> = None;
    let mut qdisc_handles = lqos_config::load_config()
        .ok()
        .map(|config| QdiscHandleState::load(&config))
        .unwrap_or_default();
    // Persist latest StormGuard ceilings keyed by interface + class so we can replay after rebuilds.
    let mut stormguard_overrides: HashMap<StormguardOverrideKey, u64> = HashMap::new();
    let mut virtualized_sites: HashMap<i64, VirtualizedSiteState> = HashMap::new();
    let mut runtime_node_operations: HashMap<i64, RuntimeNodeOperation> = HashMap::new();
    let mut next_runtime_operation_id: u64 = 1;
    let mut dynamic_circuit_overlays: HashMap<i64, DynamicCircuitOverlayEntry> =
        lqos_config::load_config()
            .ok()
            .map(|config| load_dynamic_circuit_overlays_from_disk(&config))
            .unwrap_or_default();

    // Mapping state
    #[derive(Clone, Hash, PartialEq, Eq, Debug)]
    struct MappingKey {
        ip: String,
        prefix: u32,
    }
    #[derive(Clone, Debug)]
    struct MappingVal {
        #[allow(dead_code)]
        handle: TcHandle,
        cpu: u32,
    }
    // Current kernel view (authoritative state) as tracked by the bakery
    let mut mapping_current: HashMap<MappingKey, MappingVal> = HashMap::new();
    // Next desired set staged during a batch (Python batches or other tools)
    let mut mapping_staged: Option<HashMap<MappingKey, MappingVal>> = None;
    // Keys that exist in the kernel but we couldn't classify to a known circuit (never delete automatically)
    let mut mapping_unknown: HashSet<MappingKey> = HashSet::new();
    let mut mapping_seeded: bool = false;

    let mut migrations: HashMap<i64, Migration> = HashMap::new();
    const MIGRATIONS_PER_TICK: usize = 16;

    fn parse_ip_and_prefix(ip: &str) -> (String, u32) {
        if let Some((addr, pfx)) = ip.split_once('/')
            && let Ok(n) = pfx.parse::<u32>()
        {
            return (addr.to_string(), n);
        }
        // No prefix provided; infer by address family
        // Simple heuristic: ':' suggests IPv6
        if ip.contains(':') {
            (ip.to_string(), 128)
        } else {
            (ip.to_string(), 32)
        }
    }

    fn handle_map_ip(
        ip_address: &str,
        tc_handle: TcHandle,
        cpu: u32,
        mapping_staged: &mut Option<HashMap<MappingKey, MappingVal>>,
    ) {
        let (ip, prefix) = parse_ip_and_prefix(ip_address);
        let key = MappingKey { ip, prefix };
        let val = MappingVal {
            handle: tc_handle,
            cpu,
        };
        if mapping_staged.is_none() {
            *mapping_staged = Some(HashMap::new());
        }
        if let Some(stage) = mapping_staged.as_mut() {
            stage.insert(key, val);
        }
    }

    fn handle_del_ip(
        ip_address: &str,
        mapping_staged: &mut Option<HashMap<MappingKey, MappingVal>>,
        mapping_current: &mut HashMap<MappingKey, MappingVal>,
    ) {
        // Best-effort deletion: if exact prefix was provided, remove that, else try common host prefixes
        let (ip, prefix) = parse_ip_and_prefix(ip_address);
        let key = MappingKey {
            ip: ip.clone(),
            prefix,
        };
        if let Some(stage) = mapping_staged.as_mut() {
            stage.remove(&key);
        }
        mapping_current.remove(&key);
    }

    fn mapping_key_cidr(key: &MappingKey) -> String {
        format!("{}/{}", key.ip, key.prefix)
    }

    fn rollback_migration_ip_remaps(
        remapped: &[(MappingKey, Option<MappingVal>)],
        mapping_current: &mut HashMap<MappingKey, MappingVal>,
    ) -> Result<(), String> {
        let mut failures = Vec::new();

        for (key, previous) in remapped.iter().rev() {
            let cidr = mapping_key_cidr(key);
            let rollback_result = if let Some(previous) = previous {
                lqos_sys::add_ip_to_tc(&cidr, previous.handle, previous.cpu, false, 0, 0).map(
                    |_| {
                        mapping_current.insert(key.clone(), previous.clone());
                    },
                )
            } else {
                lqos_sys::del_ip_from_tc(&cidr, false).map(|_| {
                    mapping_current.remove(key);
                })
            };

            if let Err(error) = rollback_result {
                failures.push(format!("{cidr}: {error}"));
            }
        }

        if let Err(error) = lqos_sys::clear_hot_cache() {
            failures.push(format!("clear hot cache after rollback: {error}"));
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }

    fn remap_migration_ips(
        mig: &Migration,
        target_minor: u16,
        mapping_current: &mut HashMap<MappingKey, MappingVal>,
    ) -> Result<(), String> {
        let target_handle = tc_handle_from_major_minor(mig.class_major, target_minor);
        let mut remapped: Vec<(MappingKey, Option<MappingVal>)> = Vec::new();

        for ip in &mig.ips {
            let (ip_s, prefix) = parse_ip_and_prefix(ip);
            let key = MappingKey { ip: ip_s, prefix };
            let cidr = mapping_key_cidr(&key);
            let previous = mapping_current.get(&key).cloned();
            let cpu = previous.as_ref().map(|value| value.cpu).unwrap_or(0);

            if let Err(error) = lqos_sys::add_ip_to_tc(&cidr, target_handle, cpu, false, 0, 0) {
                let rollback_summary =
                    rollback_migration_ip_remaps(&remapped, mapping_current).err();
                return Err(match rollback_summary {
                    Some(rollback_error) => format!(
                        "failed to remap {cidr} to {}: {error}; rollback also failed: {rollback_error}",
                        target_handle.as_tc_string()
                    ),
                    None => format!(
                        "failed to remap {cidr} to {}: {error}; prior remaps were rolled back",
                        target_handle.as_tc_string()
                    ),
                });
            }

            mapping_current.insert(
                key.clone(),
                MappingVal {
                    handle: target_handle,
                    cpu,
                },
            );
            remapped.push((key, previous));
        }

        if let Err(error) = lqos_sys::clear_hot_cache() {
            let rollback_summary = rollback_migration_ip_remaps(&remapped, mapping_current).err();
            return Err(match rollback_summary {
                Some(rollback_error) => format!(
                    "remap to {} succeeded but clearing the hot cache failed: {error}; rollback also failed: {rollback_error}",
                    target_handle.as_tc_string()
                ),
                None => format!(
                    "remap to {} succeeded but clearing the hot cache failed: {error}; remaps were rolled back",
                    target_handle.as_tc_string()
                ),
            });
        }

        Ok(())
    }

    fn build_known_handle_set(circuits: &HashMap<i64, Arc<BakeryCommands>>) -> HashSet<TcHandle> {
        let mut down = HashSet::new();
        for (_k, v) in circuits.iter() {
            if let BakeryCommands::AddCircuit {
                class_minor,
                class_major,
                ..
            } = v.as_ref()
            {
                let down_tc =
                    TcHandle::from_u32(((*class_major as u32) << 16) | (*class_minor as u32));
                down.insert(down_tc);
            }
        }
        down
    }

    fn attempt_seed_mappings(
        circuits: &HashMap<i64, Arc<BakeryCommands>>,
        mapping_current: &mut HashMap<MappingKey, MappingVal>,
        mapping_unknown: &mut HashSet<MappingKey>,
    ) -> anyhow::Result<()> {
        // Build classification set (known circuit handles)
        let known_set = build_known_handle_set(circuits);

        // Create a small runtime to make a one-shot bus request
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(async {
            let mut bus = LibreqosBusClient::new().await?;
            let reply = bus.request(vec![BusRequest::ListIpFlow]).await?;
            for r in reply.iter() {
                if let BusResponse::MappedIps(list) = r {
                    for m in list.iter() {
                        // m.ip_address does not include prefix, prefix_length is provided
                        let key = MappingKey {
                            ip: m.ip_address.clone(),
                            prefix: m.prefix_length,
                        };
                        if !known_set.contains(&m.tc_handle) {
                            // Unknown mapping (do not delete automatically)
                            mapping_unknown.insert(key.clone());
                        }
                        mapping_current.insert(
                            key,
                            MappingVal {
                                handle: m.tc_handle,
                                cpu: m.cpu,
                            },
                        );
                    }
                }
            }
            anyhow::Ok(())
        })
    }

    fn verify_migration_direction(
        interface_name: String,
        snapshot: &HashMap<TcHandle, crate::utils::LiveTcClassEntry>,
        expected_handle: TcHandle,
        expected_parent: TcHandle,
    ) -> MigrationDirectionVerification {
        let observed = snapshot.get(&expected_handle);
        MigrationDirectionVerification {
            interface_name,
            expected_handle,
            expected_parent,
            observed_present: observed.is_some(),
            observed_parent: observed.and_then(|entry| entry.parent),
            observed_leaf_qdisc_major: observed.and_then(|entry| entry.leaf_qdisc_major),
        }
    }

    fn migration_branch_verification(
        config: &Arc<Config>,
        down_handle: TcHandle,
        down_parent: TcHandle,
        up_handle: TcHandle,
        up_parent: TcHandle,
    ) -> Result<MigrationBranchVerification, String> {
        let down_snapshot = read_live_class_snapshot(&config.isp_interface())?;
        let down = verify_migration_direction(
            config.isp_interface(),
            &down_snapshot,
            down_handle,
            down_parent,
        );

        if config.on_a_stick_mode() {
            return Ok(MigrationBranchVerification { down, up: None });
        }

        let up_snapshot = read_live_class_snapshot(&config.internet_interface())?;
        let up = verify_migration_direction(
            config.internet_interface(),
            &up_snapshot,
            up_handle,
            up_parent,
        );

        Ok(MigrationBranchVerification { down, up: Some(up) })
    }

    fn migration_shadow_verification(
        config: &Arc<Config>,
        migration: &Migration,
    ) -> Result<MigrationBranchVerification, String> {
        let down_handle = TcHandle::from_u32(
            (u32::from(migration.class_major) << 16) | u32::from(migration.shadow_minor),
        );
        let up_handle = TcHandle::from_u32(
            (u32::from(migration.up_class_major) << 16) | u32::from(migration.shadow_minor),
        );
        migration_branch_verification(
            config,
            down_handle,
            migration.parent_class_id,
            up_handle,
            migration.up_parent_class_id,
        )
    }

    fn migration_branch_wrong_parent_prune_commands(
        config: &Arc<Config>,
        down_handle: TcHandle,
        down_parent: TcHandle,
        up_handle: TcHandle,
        up_parent: TcHandle,
    ) -> Result<Vec<Vec<String>>, String> {
        let down_snapshot = read_live_class_snapshot(&config.isp_interface())?;
        let mut commands = wrong_parent_prune_commands_for_direction(
            config.isp_interface(),
            &down_snapshot,
            down_handle,
            down_parent,
        );

        if config.on_a_stick_mode() {
            return Ok(commands);
        }

        let up_snapshot = read_live_class_snapshot(&config.internet_interface())?;
        commands.extend(wrong_parent_prune_commands_for_direction(
            config.internet_interface(),
            &up_snapshot,
            up_handle,
            up_parent,
        ));
        Ok(commands)
    }

    fn migration_shadow_wrong_parent_prune_commands(
        config: &Arc<Config>,
        migration: &Migration,
    ) -> Result<Vec<Vec<String>>, String> {
        let down_handle = TcHandle::from_u32(
            (u32::from(migration.class_major) << 16) | u32::from(migration.shadow_minor),
        );
        let up_handle = TcHandle::from_u32(
            (u32::from(migration.up_class_major) << 16) | u32::from(migration.shadow_minor),
        );
        migration_branch_wrong_parent_prune_commands(
            config,
            down_handle,
            migration.parent_class_id,
            up_handle,
            migration.up_parent_class_id,
        )
    }

    fn migration_final_verification(
        config: &Arc<Config>,
        migration: &Migration,
    ) -> Result<MigrationBranchVerification, String> {
        let down_handle = TcHandle::from_u32(
            (u32::from(migration.class_major) << 16) | u32::from(migration.final_minor),
        );
        let up_handle = TcHandle::from_u32(
            (u32::from(migration.up_class_major) << 16) | u32::from(migration.final_minor),
        );
        migration_branch_verification(
            config,
            down_handle,
            migration.parent_class_id,
            up_handle,
            migration.up_parent_class_id,
        )
    }

    fn migration_final_wrong_parent_prune_commands(
        config: &Arc<Config>,
        migration: &Migration,
    ) -> Result<Vec<Vec<String>>, String> {
        let down_handle = TcHandle::from_u32(
            (u32::from(migration.class_major) << 16) | u32::from(migration.final_minor),
        );
        let up_handle = TcHandle::from_u32(
            (u32::from(migration.up_class_major) << 16) | u32::from(migration.final_minor),
        );
        migration_branch_wrong_parent_prune_commands(
            config,
            down_handle,
            migration.parent_class_id,
            up_handle,
            migration.up_parent_class_id,
        )
    }

    fn process_pending_migrations(
        config: &Arc<Config>,
        circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
        sites: &HashMap<i64, Arc<BakeryCommands>>,
        migrations: &mut HashMap<i64, Migration>,
        mapping_current: &mut HashMap<MappingKey, MappingVal>,
        virtualized_sites: &mut HashMap<i64, VirtualizedSiteState>,
        runtime_node_operations: &mut HashMap<i64, RuntimeNodeOperation>,
    ) {
        if config.queues.queue_mode.is_observe() {
            cancel_pending_migrations_for_observe_mode(
                migrations,
                "queue_mode is observe; the shaping tree is not live.",
            );
            return;
        }

        let mut advanced = 0usize;
        let mut to_remove = Vec::new();
        let mut effective_state_changed = false;
        let persisted_qdisc_handles = QdiscHandleState::load(config);

        for (_hash, mig) in migrations.iter_mut() {
            if advanced >= MIGRATIONS_PER_TICK {
                break;
            }

            match mig.stage {
                MigrationStage::PrepareShadow => {
                    let live_reserved_handles = snapshot_live_qdisc_handle_majors_or_empty(
                        config,
                        "live-move: create shadow",
                    );
                    let target_label = migration_target_label(mig);
                    let mut cmds = match migration_shadow_wrong_parent_prune_commands(config, mig) {
                        Ok(commands) => commands,
                        Err(error) => {
                            mark_reload_required(format!(
                                "Bakery live-move shadow cleanup failed for {target_label}: {error}. A full reload is now required before further incremental topology mutations."
                            ));
                            mig.stage = MigrationStage::Done;
                            advanced += 1;
                            continue;
                        }
                    };
                    if let Some(temp) = build_shadow_add_cmd(
                        mig,
                        config,
                        &persisted_qdisc_handles,
                        &live_reserved_handles,
                    ) {
                        match config.queues.lazy_queues.as_ref() {
                            None | Some(LazyQueueMode::No) => {
                                if let Some(c) =
                                    add_commands_for_circuit(&temp, config, ExecutionMode::Builder)
                                {
                                    cmds.extend(c);
                                }
                            }
                            Some(LazyQueueMode::Htb) => {
                                if let Some(c) =
                                    add_commands_for_circuit(&temp, config, ExecutionMode::Builder)
                                {
                                    cmds.extend(c);
                                }
                                if let Some(c) = add_commands_for_circuit(
                                    &temp,
                                    config,
                                    ExecutionMode::LiveUpdate,
                                ) {
                                    cmds.extend(c);
                                }
                            }
                            Some(LazyQueueMode::Full) => {
                                if let Some(c) = add_commands_for_circuit(
                                    &temp,
                                    config,
                                    ExecutionMode::LiveUpdate,
                                ) {
                                    cmds.extend(c);
                                }
                            }
                        }
                        if !cmds.is_empty() {
                            let result =
                                execute_and_record_live_change(&cmds, "live-move: create shadow");
                            let _ = migration_stage_apply_succeeded(
                                mig,
                                "live-move: create shadow",
                                &result,
                                MigrationStage::VerifyShadowReady,
                            );
                        } else {
                            mig.stage = MigrationStage::VerifyShadowReady;
                        }
                        advanced += 1;
                    } else {
                        warn!(
                            "live-move: failed to build shadow add cmd for {}",
                            mig.circuit_hash
                        );
                        mig.stage = MigrationStage::Done;
                        advanced += 1;
                    }
                }
                MigrationStage::VerifyShadowReady => {
                    let target_label = migration_target_label(mig);
                    let verification = match migration_shadow_verification(config, mig) {
                        Ok(verification) => verification,
                        Err(e) => {
                            mark_reload_required(format!(
                                "Bakery live-move shadow verification failed for {target_label}: {e}. A full reload is now required before further incremental topology mutations."
                            ));
                            mig.stage = MigrationStage::Done;
                            advanced += 1;
                            continue;
                        }
                    };
                    if !verification.ready() {
                        if mig.shadow_verify_attempts < MIGRATION_VERIFICATION_MAX_RETRIES {
                            mig.shadow_verify_attempts =
                                mig.shadow_verify_attempts.saturating_add(1);
                            warn!(
                                "Bakery live-move shadow verification retry {}/{} for {}: {}",
                                mig.shadow_verify_attempts,
                                MIGRATION_VERIFICATION_MAX_RETRIES,
                                target_label,
                                verification.summary()
                            );
                            advanced += 1;
                            continue;
                        }
                        mark_reload_required(format!(
                            "Bakery live-move shadow verification did not find the expected shadow class/qdisc for {target_label}: {}. A full reload is now required before further incremental topology mutations.",
                            verification.summary()
                        ));
                        mig.stage = MigrationStage::Done;
                        advanced += 1;
                        continue;
                    }
                    mig.stage = MigrationStage::SwapToShadow;
                    mig.shadow_verify_attempts = 0;
                    advanced += 1;
                }
                MigrationStage::SwapToShadow => {
                    let target_label = migration_target_label(mig);
                    if let Err(summary) =
                        remap_migration_ips(mig, mig.shadow_minor, mapping_current)
                    {
                        mark_reload_required(format!(
                            "Bakery live-move shadow remap failed for {target_label}: {summary}. A full reload is now required before further incremental topology mutations."
                        ));
                        mig.stage = MigrationStage::Done;
                        advanced += 1;
                        continue;
                    }
                    mig.stage = MigrationStage::BuildFinal;
                    advanced += 1;
                }
                MigrationStage::BuildFinal => {
                    let target_label = migration_target_label(mig);
                    let mut cmds = match migration_final_wrong_parent_prune_commands(config, mig) {
                        Ok(commands) => commands,
                        Err(error) => {
                            mark_reload_required(format!(
                                "Bakery live-move final cleanup failed for {target_label}: {error}. A full reload is now required before further incremental topology mutations."
                            ));
                            mig.stage = MigrationStage::Done;
                            advanced += 1;
                            continue;
                        }
                    };
                    let live_qdisc_handles = snapshot_live_qdisc_handle_majors_or_empty(
                        config,
                        "live-move: build final",
                    );
                    let down_live_qdisc_handles = live_qdisc_handles.get(&config.isp_interface());
                    let up_live_qdisc_handles =
                        live_qdisc_handles.get(&config.internet_interface());
                    if let Some(final_cmd) = build_temp_add_cmd(
                        mig.desired_cmd.as_ref(),
                        mig.final_minor,
                        mig.new_down_min,
                        mig.new_down_max,
                        mig.new_up_min,
                        mig.new_up_max,
                        true,
                    ) {
                        if let Some(summary) =
                            build_final_qdisc_handle_rotation_invariant_error_with_live_reservations(
                                mig,
                                &final_cmd,
                                down_live_qdisc_handles,
                                up_live_qdisc_handles,
                            )
                        {
                            mark_reload_required(format!(
                                "Bakery live migration refused to build final state for {target_label} because the final qdisc handles were unsafe: {summary}. A full reload is now required before further incremental topology mutations."
                            ));
                            mig.stage = MigrationStage::Done;
                            advanced += 1;
                            continue;
                        }
                        match config.queues.lazy_queues.as_ref() {
                            None | Some(LazyQueueMode::No) => {
                                if let Some(c) = add_commands_for_circuit(
                                    &final_cmd,
                                    config,
                                    ExecutionMode::Builder,
                                ) {
                                    cmds.extend(c);
                                }
                            }
                            Some(LazyQueueMode::Htb) => {
                                if let Some(c) = add_commands_for_circuit(
                                    &final_cmd,
                                    config,
                                    ExecutionMode::Builder,
                                ) {
                                    cmds.extend(c);
                                }
                                if let Some(c) = add_commands_for_circuit(
                                    &final_cmd,
                                    config,
                                    ExecutionMode::LiveUpdate,
                                ) {
                                    cmds.extend(c);
                                }
                            }
                            Some(LazyQueueMode::Full) => {
                                if let Some(c) = add_commands_for_circuit(
                                    &final_cmd,
                                    config,
                                    ExecutionMode::LiveUpdate,
                                ) {
                                    cmds.extend(c);
                                }
                            }
                        }
                    }
                    if !cmds.is_empty() {
                        let result =
                            execute_and_record_live_change(&cmds, "live-move: build final");
                        let _ = migration_stage_apply_succeeded(
                            mig,
                            "live-move: build final",
                            &result,
                            MigrationStage::VerifyFinalReady,
                        );
                    } else {
                        mig.stage = MigrationStage::VerifyFinalReady;
                    }
                    advanced += 1;
                }
                MigrationStage::VerifyFinalReady => {
                    let target_label = migration_target_label(mig);
                    let verification = match migration_final_verification(config, mig) {
                        Ok(verification) => verification,
                        Err(e) => {
                            mark_reload_required(format!(
                                "Bakery live-move final verification failed for {target_label}: {e}. A full reload is now required before further incremental topology mutations."
                            ));
                            mig.stage = MigrationStage::Done;
                            advanced += 1;
                            continue;
                        }
                    };
                    if !verification.ready() {
                        if mig.final_verify_attempts < MIGRATION_VERIFICATION_MAX_RETRIES {
                            mig.final_verify_attempts = mig.final_verify_attempts.saturating_add(1);
                            warn!(
                                "Bakery live-move final verification retry {}/{} for {}: {}",
                                mig.final_verify_attempts,
                                MIGRATION_VERIFICATION_MAX_RETRIES,
                                target_label,
                                verification.summary()
                            );
                            advanced += 1;
                            continue;
                        }
                        mark_reload_required(format!(
                            "Bakery live-move final verification did not find the expected final class/qdisc for {target_label}: {}. A full reload is now required before further incremental topology mutations.",
                            verification.summary()
                        ));
                        mig.stage = MigrationStage::Done;
                        advanced += 1;
                        continue;
                    }
                    mig.stage = MigrationStage::SwapToFinal;
                    mig.final_verify_attempts = 0;
                    advanced += 1;
                }
                MigrationStage::SwapToFinal => {
                    let target_label = migration_target_label(mig);
                    if let Err(summary) = remap_migration_ips(mig, mig.final_minor, mapping_current)
                    {
                        mark_reload_required(format!(
                            "Bakery live-move final remap failed for {target_label}: {summary}. A full reload is now required before further incremental topology mutations."
                        ));
                        mig.stage = MigrationStage::Done;
                        advanced += 1;
                        continue;
                    }
                    circuits.insert(mig.circuit_hash, Arc::clone(&mig.desired_cmd));
                    effective_state_changed = true;
                    mig.stage = MigrationStage::TeardownMigrationScaffold;
                    advanced += 1;
                }
                MigrationStage::TeardownMigrationScaffold => {
                    let mut prune = Vec::new();
                    if let Some(shadow_cmd) = build_temp_add_cmd(
                        &BakeryCommands::AddCircuit {
                            circuit_hash: mig.circuit_hash,
                            circuit_name: mig.circuit_name.clone(),
                            site_name: mig.site_name.clone(),
                            parent_class_id: mig.parent_class_id,
                            up_parent_class_id: mig.up_parent_class_id,
                            class_minor: mig.shadow_minor,
                            download_bandwidth_min: mig.old_down_min,
                            upload_bandwidth_min: mig.old_up_min,
                            download_bandwidth_max: mig.old_down_max,
                            upload_bandwidth_max: mig.old_up_max,
                            class_major: mig.class_major,
                            up_class_major: mig.up_class_major,
                            down_qdisc_handle: None,
                            up_qdisc_handle: None,
                            ip_addresses: "".to_string(),
                            sqm_override: mig.sqm_override.clone(),
                        },
                        mig.shadow_minor,
                        mig.old_down_min,
                        mig.old_down_max,
                        mig.old_up_min,
                        mig.old_up_max,
                        false,
                    ) && let Some(shadow_prune) = shadow_cmd.to_prune(config, true)
                    {
                        prune.extend(shadow_prune);
                    }
                    if !prune.is_empty() {
                        let result = execute_and_record_live_change(
                            &prune,
                            "live-move: prune migration scaffold",
                        );
                        let _ = migration_stage_apply_succeeded(
                            mig,
                            "live-move: prune migration scaffold",
                            &result,
                            MigrationStage::Done,
                        );
                    } else {
                        mig.stage = MigrationStage::Done;
                    }
                    advanced += 1;
                }
                MigrationStage::Done => {
                    to_remove.push(mig.circuit_hash);
                }
            }
        }

        for circuit_hash in to_remove {
            migrations.remove(&circuit_hash);
        }

        flush_deferred_runtime_site_prunes(
            config,
            virtualized_sites,
            migrations,
            runtime_node_operations,
        );
        if effective_state_changed {
            update_queue_distribution_snapshot(sites, circuits);
        }
    }

    {
        let Ok(config) = lqos_config::load_config() else {
            error!("Failed to load configuration, exiting Bakery thread.");
            return;
        };
        info!(
            "Bakery thread starting. Mode: {:?}, expiration: {}s",
            config.queues.lazy_queues,
            config.queues.lazy_expire_seconds.unwrap_or(600)
        );
        push_bakery_event(
            "baseline_rebuild_startup",
            "info",
            "Bakery started with empty runtime state; the first commit will rebuild the baseline queue tree.".to_string(),
        );
    }

    loop {
        let command = match rx.recv_timeout(Duration::from_millis(BAKERY_BACKGROUND_INTERVAL_MS)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                let Ok(config) = lqos_config::load_config() else {
                    error!("Failed to load configuration while processing pending migrations.");
                    continue;
                };
                process_pending_migrations(
                    &config,
                    &mut circuits,
                    &sites,
                    &mut migrations,
                    &mut mapping_current,
                    &mut virtualized_sites,
                    &mut runtime_node_operations,
                );
                refresh_live_capacity_snapshot(&config, false);
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        debug!("Bakery received command: {:?}", command);

        match command {
            // Mapping events (mirrored from lqosd bus handling)
            BakeryCommands::MapIp {
                ip_address,
                tc_handle,
                cpu,
                ..
            } => {
                handle_map_ip(&ip_address, tc_handle, cpu, &mut mapping_staged);
            }
            BakeryCommands::BusReady => {
                if !mapping_seeded {
                    match attempt_seed_mappings(
                        &circuits,
                        &mut mapping_current,
                        &mut mapping_unknown,
                    ) {
                        Ok(_) => {
                            let total = mapping_current.len();
                            let unknown = mapping_unknown.len();
                            info!(
                                "Bakery mappings seeded: total={}, unknown={}",
                                total, unknown
                            );
                            mapping_seeded = true;
                        }
                        Err(e) => warn!("Bakery: Failed to seed IP mappings at bus-ready: {:?}", e),
                    }
                }
            }
            BakeryCommands::DelIp { ip_address, .. } => {
                handle_del_ip(&ip_address, &mut mapping_staged, &mut mapping_current);
            }
            BakeryCommands::ClearIpAll => {
                mapping_current.clear();
                mapping_unknown.clear();
                mapping_staged = None;
            }
            BakeryCommands::CommitMappings => {
                // Ensure we are seeded before first commit to avoid assuming empty kernel state.
                if !mapping_seeded {
                    match attempt_seed_mappings(
                        &circuits,
                        &mut mapping_current,
                        &mut mapping_unknown,
                    ) {
                        Ok(_) => {
                            let total = mapping_current.len();
                            let unknown = mapping_unknown.len();
                            info!(
                                "Bakery mappings seeded: total={}, unknown={}",
                                total, unknown
                            );
                            mapping_seeded = true;
                        }
                        Err(e) => warn!("Bakery: Failed to seed IP mappings: {:?}", e),
                    }
                }

                if let Some(staged) = mapping_staged.take() {
                    // Remove stale mappings: present in current, not in staged; never delete unknowns
                    let mut stale = Vec::new();
                    for k in mapping_current.keys() {
                        if mapping_unknown.contains(k) {
                            continue; // don't touch unknowns
                        }
                        if !staged.contains_key(k) {
                            stale.push(k.clone());
                        }
                    }

                    if !stale.is_empty() {
                        // Batch deletions via the bus client
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build();
                        if let Ok(rt) = rt {
                            let stale_to_delete = stale.clone();
                            rt.block_on(async move {
                                if let Ok(mut bus) = LibreqosBusClient::new().await {
                                    // chunk operations to keep request sizes reasonable
                                    const CHUNK: usize = 512;
                                    for chunk in stale_to_delete.chunks(CHUNK) {
                                        let mut reqs = Vec::with_capacity(chunk.len());
                                        for k in chunk.iter() {
                                            // Recompose an IP string with prefix if not host (/32 or /128)
                                            let ip = if k.prefix == 32 || k.prefix == 128 {
                                                k.ip.clone()
                                            } else {
                                                format!("{}/{}", k.ip, k.prefix)
                                            };
                                            reqs.push(BusRequest::DelIpFlow {
                                                ip_address: ip,
                                                upload: false,
                                            });
                                        }
                                        let _ = bus.request(reqs).await;
                                    }
                                }
                            });
                        } else {
                            warn!("Bakery: Unable to create runtime for stale IP deletions");
                        }

                        for k in stale.into_iter() {
                            mapping_current.remove(&k);
                        }
                    }

                    // Merge staged into current (they are already applied in kernel by lqosd)
                    for (k, v) in staged.into_iter() {
                        mapping_current.insert(k, v);
                    }
                }
            }
            BakeryCommands::StartBatch => {
                batch = Some(Vec::new());
            }
            BakeryCommands::CommitBatch => {
                push_bakery_event(
                    "commit_received",
                    "info",
                    "Bakery commit received.".to_string(),
                );
                handle_commit_batch(
                    &mut batch,
                    &mut sites,
                    &mut circuits,
                    &mut dynamic_circuit_overlays,
                    &mut live_circuits,
                    &mut mq_layout,
                    &mut qdisc_handles,
                    &tx,
                    &mut migrations,
                    &stormguard_overrides,
                    &mut virtualized_sites,
                    &mut runtime_node_operations,
                );
                let Ok(config) = lqos_config::load_config() else {
                    error!("Failed to load configuration while processing pending migrations.");
                    continue;
                };
                process_pending_migrations(
                    &config,
                    &mut circuits,
                    &sites,
                    &mut migrations,
                    &mut mapping_current,
                    &mut virtualized_sites,
                    &mut runtime_node_operations,
                );
            }
            BakeryCommands::MqSetup { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(Arc::new(command));
                }
            }
            BakeryCommands::AddSite { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(Arc::new(command));
                }
            }
            BakeryCommands::AddCircuit { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(Arc::new(command));
                }
            }
            BakeryCommands::UpsertDynamicCircuitOverlay {
                shaped_device,
                reply,
            } => {
                let result = handle_upsert_dynamic_circuit_overlay(
                    *shaped_device,
                    &mut dynamic_circuit_overlays,
                    batch.is_some(),
                    &sites,
                    &mut circuits,
                    &mut live_circuits,
                    &mq_layout,
                    &mut qdisc_handles,
                    &migrations,
                );
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            BakeryCommands::RemoveDynamicCircuitOverlay { circuit_id, reply } => {
                let result = handle_remove_dynamic_circuit_overlay(
                    &circuit_id,
                    &mut dynamic_circuit_overlays,
                    batch.is_some(),
                    &sites,
                    &mut circuits,
                    &mut live_circuits,
                    &mq_layout,
                    &mut qdisc_handles,
                );
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            BakeryCommands::OnCircuitActivity { circuit_ids } => {
                handle_circuit_activity(circuit_ids, &circuits, &mut live_circuits);
            }
            BakeryCommands::Tick => {
                // Reset per-cycle counters at the start of the tick
                handle_tick(&mut circuits, &mut live_circuits, &mut sites);
                let Ok(config) = lqos_config::load_config() else {
                    error!("Failed to load configuration while processing pending migrations.");
                    continue;
                };
                process_pending_migrations(
                    &config,
                    &mut circuits,
                    &sites,
                    &mut migrations,
                    &mut mapping_current,
                    &mut virtualized_sites,
                    &mut runtime_node_operations,
                );
            }
            BakeryCommands::ChangeSiteSpeedLive {
                site_hash,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
            } => {
                handle_change_site_speed_live(
                    site_hash,
                    download_bandwidth_min,
                    upload_bandwidth_min,
                    download_bandwidth_max,
                    upload_bandwidth_max,
                    &mut sites,
                );
            }
            BakeryCommands::StormGuardAdjustment {
                dry_run,
                interface_name,
                class_id,
                new_rate,
            } => {
                let has_mq_run = MQ_CREATED.load(Relaxed);
                if !has_mq_run {
                    debug!("StormGuardAdjustment received before MQ setup, skipping.");
                    continue;
                }
                let Ok(config) = lqos_config::load_config() else {
                    error!("Failed to load configuration, skipping StormGuardAdjustment.");
                    continue;
                };
                let Ok(tc_handle) = TcHandle::from_string(&class_id) else {
                    warn!(
                        "StormGuardAdjustment has invalid class_id [{}], skipping.",
                        class_id
                    );
                    continue;
                };
                if !dry_run {
                    let key = StormguardOverrideKey {
                        interface: interface_name.to_string(),
                        class: tc_handle,
                    };
                    stormguard_overrides.insert(key, new_rate);
                }
                if let Some(reason) = live_tree_mutation_blocker_for_config(&config) {
                    info!(
                        "Skipping StormGuard live class change for {} {} because {}.",
                        interface_name, class_id, reason
                    );
                    continue;
                }
                let normalized_class = tc_handle.as_tc_string();
                // Build the HTB command
                let args = vec![
                    "class".to_string(),
                    "replace".to_string(),
                    "dev".to_string(),
                    interface_name.to_string(),
                    "classid".to_string(),
                    normalized_class.clone(),
                    "htb".to_string(),
                    "rate".to_string(),
                    format!("{}mbit", new_rate.saturating_sub(1)),
                    "ceil".to_string(),
                    format!("{}mbit", new_rate),
                ];
                if dry_run {
                    info!("DRY RUN: /sbin/tc {}", args.join(" "));
                } else {
                    let output = std::process::Command::new("/sbin/tc").args(&args).output();
                    match output {
                        Err(e) => {
                            warn!("Failed to run tc command: {}", e);
                        }
                        Ok(out) => {
                            if !out.status.success() {
                                warn!(
                                    "tc command failed: {}",
                                    String::from_utf8_lossy(&out.stderr)
                                );
                            } else {
                                debug!(
                                    "tc command succeeded: {}",
                                    String::from_utf8_lossy(&out.stdout)
                                );
                            }
                        }
                    }
                }
            }
            BakeryCommands::TreeGuardSetNodeVirtual {
                site_hash,
                virtualized,
                reply,
            } => {
                let result = handle_treeguard_set_node_virtual_live(
                    site_hash,
                    virtualized,
                    &mut sites,
                    &mut circuits,
                    &live_circuits,
                    &mq_layout,
                    &mut qdisc_handles,
                    &mut migrations,
                    &mut virtualized_sites,
                    &mut runtime_node_operations,
                    &mut next_runtime_operation_id,
                );
                update_queue_distribution_snapshot(&sites, &circuits);
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
        }
    }
    error!("Bakery thread exited unexpectedly.");
}

#[allow(clippy::too_many_arguments)]
fn handle_commit_batch(
    batch: &mut Option<Vec<Arc<BakeryCommands>>>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    dynamic_circuit_overlays: &mut HashMap<i64, DynamicCircuitOverlayEntry>,
    live_circuits: &mut HashMap<i64, u64>,
    mq_layout: &mut Option<MqDeviceLayout>,
    qdisc_handles: &mut QdiscHandleState,
    tx: &Sender<BakeryCommands>,
    migrations: &mut HashMap<i64, Migration>,
    stormguard_overrides: &HashMap<StormguardOverrideKey, u64>,
    virtualized_sites: &mut HashMap<i64, VirtualizedSiteState>,
    runtime_node_operations: &mut HashMap<i64, RuntimeNodeOperation>,
) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };
    qdisc_handles.clear_retired_handles();

    let Some(raw_batch) = batch.take() else {
        debug!("CommitBatch received without a batch to commit.");
        return;
    };
    let mut raw_batch = raw_batch;
    append_dynamic_circuit_overlays_to_batch(&mut raw_batch, dynamic_circuit_overlays, migrations);
    let (baseline_sites, baseline_circuits) =
        reconstruct_structural_baseline_state(sites, circuits, virtualized_sites);
    let effective_new_batch =
        apply_runtime_virtualization_overlay(raw_batch.clone(), virtualized_sites);
    let resolved_mq_layout = current_mq_layout(&raw_batch, &config, mq_layout);
    let has_mq_been_setup = MQ_CREATED.load(std::sync::atomic::Ordering::Relaxed);
    let shaping_tree_active = SHAPING_TREE_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
    let desired_tree_active = desired_shaping_tree_active(&config);

    let mapped_limit = resolve_mapped_circuit_limit();
    let effective_limit = mapped_limit.effective_limit;
    let limit_label = format_mapped_limit(effective_limit);

    if shaping_tree_active && !desired_tree_active {
        cancel_pending_migrations_for_observe_mode(
            migrations,
            "queue mode transitioned to observe; a full reload will rebuild the retained root MQ without the shaping tree.",
        );
    }

    if let Some(reason) = bakery_reload_required_reason() {
        let summary = format!(
            "Bakery full reload triggered by reload-required state: {}",
            reason
        );
        let (new_batch, mapped_limit_stats) = filter_batch_by_mapped_circuit_limit(
            raw_batch.clone(),
            &baseline_circuits,
            effective_limit,
        );
        log_mapped_limit_decision("reload-required rebuild", mapped_limit, mapped_limit_stats);
        if mapped_limit_stats.dropped_mapped > 0 {
            warn!(
                "Bakery mapped circuit cap enforced (reload-required rebuild): requested={}, allowed={}, dropped={}, limit={} (licensed={}, max_circuits={:?})",
                mapped_limit_stats.requested_mapped,
                mapped_limit_stats.allowed_mapped,
                mapped_limit_stats.dropped_mapped,
                limit_label,
                mapped_limit.licensed,
                mapped_limit.max_circuits
            );
            maybe_emit_mapped_circuit_limit_urgent(&mapped_limit_stats);
        }
        announce_full_reload(&summary);
        full_reload(
            batch,
            sites,
            circuits,
            live_circuits,
            mq_layout,
            qdisc_handles,
            &config,
            new_batch,
            resolved_mq_layout,
            stormguard_overrides,
            virtualized_sites,
            runtime_node_operations,
            summary,
        );
        return;
    }

    if !has_mq_been_setup {
        push_bakery_event(
            "baseline_rebuild_required",
            "warning",
            "Bakery runtime state was reset by restart/cold start; performing explicit baseline full reload.".to_string(),
        );
        let (new_batch, mapped_limit_stats) = filter_batch_by_mapped_circuit_limit(
            raw_batch.clone(),
            &baseline_circuits,
            effective_limit,
        );
        log_mapped_limit_decision("full reload", mapped_limit, mapped_limit_stats);
        if mapped_limit_stats.dropped_mapped > 0 {
            warn!(
                "Bakery mapped circuit cap enforced (full reload): requested={}, allowed={}, dropped={}, limit={} (licensed={}, max_circuits={:?})",
                mapped_limit_stats.requested_mapped,
                mapped_limit_stats.allowed_mapped,
                mapped_limit_stats.dropped_mapped,
                limit_label,
                mapped_limit.licensed,
                mapped_limit.max_circuits
            );
            maybe_emit_mapped_circuit_limit_urgent(&mapped_limit_stats);
        }
        // If the MQ hasn't been created, we need to do this as a full, unadjusted run.
        let summary = "Bakery full reload triggered by baseline rebuild after restart/cold start."
            .to_string();
        info!("Bakery baseline rebuild after restart/cold start: performing explicit full reload.");
        announce_full_reload(&summary);
        full_reload(
            batch,
            sites,
            circuits,
            live_circuits,
            mq_layout,
            qdisc_handles,
            &config,
            new_batch,
            resolved_mq_layout,
            stormguard_overrides,
            virtualized_sites,
            runtime_node_operations,
            summary,
        );
        return;
    }

    if shaping_tree_active != desired_tree_active {
        let (new_batch, mapped_limit_stats) = filter_batch_by_mapped_circuit_limit(
            raw_batch.clone(),
            &baseline_circuits,
            effective_limit,
        );
        log_mapped_limit_decision("queue-mode rebuild", mapped_limit, mapped_limit_stats);
        if mapped_limit_stats.dropped_mapped > 0 {
            warn!(
                "Bakery mapped circuit cap enforced (queue-mode rebuild): requested={}, allowed={}, dropped={}, limit={} (licensed={}, max_circuits={:?})",
                mapped_limit_stats.requested_mapped,
                mapped_limit_stats.allowed_mapped,
                mapped_limit_stats.dropped_mapped,
                limit_label,
                mapped_limit.licensed,
                mapped_limit.max_circuits
            );
            maybe_emit_mapped_circuit_limit_urgent(&mapped_limit_stats);
        }
        let summary = format!(
            "Bakery full reload triggered by queue mode transition to {}.",
            if desired_tree_active {
                "shape"
            } else {
                "observe"
            }
        );
        announce_full_reload(&summary);
        full_reload(
            batch,
            sites,
            circuits,
            live_circuits,
            mq_layout,
            qdisc_handles,
            &config,
            new_batch,
            resolved_mq_layout,
            stormguard_overrides,
            virtualized_sites,
            runtime_node_operations,
            summary,
        );
        return;
    }

    let structural_site_change_mode = diff_sites(&raw_batch, &baseline_sites);
    if let SiteDiffResult::RebuildRequired { summary, details } = &structural_site_change_mode {
        if let Some(details) = details {
            log_structural_site_diff_baseline_origin(
                *details,
                sites,
                &baseline_sites,
                &raw_batch,
                virtualized_sites,
            );
        }
        let (new_batch, mapped_limit_stats) = filter_batch_by_mapped_circuit_limit(
            raw_batch.clone(),
            &baseline_circuits,
            effective_limit,
        );
        log_mapped_limit_decision("site-structure rebuild", mapped_limit, mapped_limit_stats);
        if mapped_limit_stats.dropped_mapped > 0 {
            warn!(
                "Bakery mapped circuit cap enforced (site-structure rebuild): requested={}, allowed={}, dropped={}, limit={} (licensed={}, max_circuits={:?})",
                mapped_limit_stats.requested_mapped,
                mapped_limit_stats.allowed_mapped,
                mapped_limit_stats.dropped_mapped,
                limit_label,
                mapped_limit.licensed,
                mapped_limit.max_circuits
            );
            maybe_emit_mapped_circuit_limit_urgent(&mapped_limit_stats);
        }
        // If the site structure has changed, we need to rebuild everything.
        info!("Bakery full reload: site_struct=1, circuit_struct=0");
        announce_full_reload(summary);
        full_reload(
            batch,
            sites,
            circuits,
            live_circuits,
            mq_layout,
            qdisc_handles,
            &config,
            new_batch,
            resolved_mq_layout,
            stormguard_overrides,
            virtualized_sites,
            runtime_node_operations,
            summary.clone(),
        );
        return;
    }

    let baseline_circuits_for_diff =
        circuits_with_pending_migration_targets(&baseline_circuits, migrations);
    let structural_circuit_change_mode = diff_circuits(&raw_batch, &baseline_circuits_for_diff);
    let site_change_mode = diff_sites(&effective_new_batch, sites);
    let circuits_for_diff = circuits_with_pending_migration_targets(circuits, migrations);
    let circuit_change_mode = diff_circuits(&effective_new_batch, &circuits_for_diff);

    // If neither has changed, there's nothing to do.
    if matches!(site_change_mode, SiteDiffResult::NoChange)
        && matches!(circuit_change_mode, CircuitDiffResult::NoChange)
    {
        // No changes detected, skip processing
        info!("No changes detected in batch, skipping processing.");
        return;
    }

    // If any structural changes occurred, do a full reload
    if let CircuitDiffResult::Categorized(categories) = &structural_circuit_change_mode
        && !categories.structural_changed.is_empty()
    {
        let summary = format!(
            "Bakery full reload triggered by circuit structural diff: structural_changed_count={}",
            categories.structural_changed.len()
        );
        let (new_batch, mapped_limit_stats) = filter_batch_by_mapped_circuit_limit(
            raw_batch.clone(),
            &baseline_circuits,
            effective_limit,
        );
        log_mapped_limit_decision(
            "circuit-structure rebuild",
            mapped_limit,
            mapped_limit_stats,
        );
        if mapped_limit_stats.dropped_mapped > 0 {
            warn!(
                "Bakery mapped circuit cap enforced (circuit-structure rebuild): requested={}, allowed={}, dropped={}, limit={} (licensed={}, max_circuits={:?})",
                mapped_limit_stats.requested_mapped,
                mapped_limit_stats.allowed_mapped,
                mapped_limit_stats.dropped_mapped,
                limit_label,
                mapped_limit.licensed,
                mapped_limit.max_circuits
            );
            maybe_emit_mapped_circuit_limit_urgent(&mapped_limit_stats);
        }
        info!(
            "Bakery full reload: site_struct=0, circuit_struct={}",
            categories.structural_changed.len()
        );
        announce_full_reload(&summary);
        full_reload(
            batch,
            sites,
            circuits,
            live_circuits,
            mq_layout,
            qdisc_handles,
            &config,
            new_batch,
            resolved_mq_layout,
            stormguard_overrides,
            virtualized_sites,
            runtime_node_operations,
            summary,
        );
        return;
    }

    // Declare any site speed changes that need to be applied. We're sending them
    // to ourselves as future commands via the BakeryCommands channel.
    let mut site_speed_change_count = 0usize;
    if let SiteDiffResult::SpeedChanges { changes } = site_change_mode {
        site_speed_change_count = changes.len();
        for change in &changes {
            let BakeryCommands::AddSite {
                site_hash,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
                ..
            } = change
            else {
                debug!(
                    "ChangeSiteSpeedLive received a non-site command: {:?}",
                    change
                );
                continue;
            };
            if let Err(e) = tx.try_send(BakeryCommands::ChangeSiteSpeedLive {
                site_hash: *site_hash,
                download_bandwidth_min: *download_bandwidth_min,
                upload_bandwidth_min: *upload_bandwidth_min,
                download_bandwidth_max: *download_bandwidth_max,
                upload_bandwidth_max: *upload_bandwidth_max,
            }) {
                error!("Channel full, falling back to full rebuild: {}", e);
                info!("Bakery full reload: site_struct=0, circuit_struct=0");
                let summary = format!(
                    "Bakery full reload triggered because site speed live-change enqueue failed: {}",
                    e
                );
                announce_full_reload(&summary);
                full_reload(
                    batch,
                    sites,
                    circuits,
                    live_circuits,
                    mq_layout,
                    qdisc_handles,
                    &config,
                    raw_batch.clone(),
                    resolved_mq_layout.clone(),
                    stormguard_overrides,
                    virtualized_sites,
                    runtime_node_operations,
                    summary,
                );
                return; // Skip the rest of this CommitBatch processing
            }
        }
    }

    // Now we can process circuit changes incrementally
    if let CircuitDiffResult::Categorized(categories) = circuit_change_mode {
        // One-line summary of changes (info!)
        info!(
            "Bakery changes: sites_speed={}, circuits_added={}, removed={}, speed={}, migrated={}, ip={}",
            site_speed_change_count,
            categories.newly_added.len(),
            categories.removed_circuits.len(),
            categories.speed_changed.len(),
            categories.migrated.len(),
            categories.ip_changed.len()
        );

        // 1) Removals
        if !categories.removed_circuits.is_empty() {
            for circuit_hash in categories.removed_circuits {
                if let Some(circuit) = circuits.remove(&circuit_hash) {
                    let was_activated = live_circuits.contains_key(&circuit_hash);
                    let commands = match config.queues.lazy_queues.as_ref() {
                        None | Some(LazyQueueMode::No) => circuit.to_prune(&config, true),
                        Some(LazyQueueMode::Htb) => {
                            if was_activated {
                                circuit.to_prune(&config, false)
                            } else {
                                None
                            }
                        }
                        Some(LazyQueueMode::Full) => {
                            if was_activated {
                                circuit.to_prune(&config, true)
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(cmd) = commands {
                        execute_and_record_live_change(&cmd, "removing circuit");
                    }
                    live_circuits.remove(&circuit_hash);
                    qdisc_handles.release_circuit(&config.isp_interface(), circuit_hash);
                    if !config.on_a_stick_mode() {
                        qdisc_handles.release_circuit(&config.internet_interface(), circuit_hash);
                    }
                } else {
                    debug!(
                        "RemoveCircuit received for unknown circuit: {}",
                        circuit_hash
                    );
                }
            }
        }

        // 2) Speed changes (avoid linux TC deadlock by removing qdisc first)
        if !categories.speed_changed.is_empty() {
            let live_tree_allowed = live_tree_mutations_allowed(&config);
            if !live_tree_allowed
                && let Some(reason) = live_tree_mutation_blocker_for_config(&config)
            {
                info!(
                    "Skipping live circuit speed fallback updates because {}. Runtime state will be updated in memory only.",
                    reason
                );
                push_bakery_event(
                    "live_circuit_speed_skipped",
                    "info",
                    format!(
                        "Skipping live circuit speed fallback updates because {}.",
                        reason
                    ),
                );
            }
            let live_reserved_handles = if live_tree_allowed {
                snapshot_live_qdisc_handle_majors_or_empty(&config, "circuit speed updates")
            } else {
                HashMap::new()
            };
            let mut immediate_commands = Vec::new();
            for cmd in &categories.speed_changed {
                let mut enriched_cmd = if live_tree_allowed {
                    let Some(layout) = resolved_mq_layout.as_ref() else {
                        warn!("Bakery: missing MQ layout during circuit speed updates");
                        return;
                    };
                    with_assigned_qdisc_handles_reserved(
                        cmd,
                        &config,
                        layout,
                        qdisc_handles,
                        &live_reserved_handles,
                    )
                } else {
                    Arc::clone(cmd)
                };
                let old_cmd = if let BakeryCommands::AddCircuit { circuit_hash, .. } =
                    enriched_cmd.as_ref()
                {
                    circuits.get(circuit_hash).cloned()
                } else {
                    None
                };
                if let Some(old_cmd) = old_cmd.as_ref()
                    && live_tree_allowed
                {
                    let Some(layout) = resolved_mq_layout.as_ref() else {
                        warn!("Bakery: missing MQ layout during circuit speed updates");
                        return;
                    };
                    enriched_cmd = rotate_changed_qdisc_handles_reserved(
                        old_cmd.as_ref(),
                        &enriched_cmd,
                        &config,
                        layout,
                        qdisc_handles,
                        &live_reserved_handles,
                    );
                }
                if live_tree_allowed
                    && let Some(old_cmd) = old_cmd.as_ref()
                    && queue_live_migration(
                        old_cmd.as_ref(),
                        &enriched_cmd,
                        sites,
                        circuits,
                        live_circuits,
                        migrations,
                    )
                {
                    continue;
                }
                if let BakeryCommands::AddCircuit { circuit_hash, .. } = enriched_cmd.as_ref() {
                    if !live_tree_allowed {
                        circuits.insert(*circuit_hash, enriched_cmd);
                        continue;
                    }
                    let was_activated = live_circuits.contains_key(circuit_hash);
                    // Fallback: immediate safe update
                    match config.queues.lazy_queues.as_ref() {
                        None | Some(LazyQueueMode::No) => {
                            if let Some(prune) = enriched_cmd.to_prune(&config, true) {
                                immediate_commands.extend(prune);
                            }
                            if let Some(add) =
                                enriched_cmd.to_commands(&config, ExecutionMode::Builder)
                            {
                                immediate_commands.extend(add);
                            }
                        }
                        Some(LazyQueueMode::Htb) => {
                            if was_activated {
                                if let Some(prune) = enriched_cmd.to_prune(&config, false) {
                                    immediate_commands.extend(prune);
                                }
                                if let Some(add_htb) =
                                    enriched_cmd.to_commands(&config, ExecutionMode::Builder)
                                {
                                    immediate_commands.extend(add_htb);
                                }
                                if let Some(add_qdisc) =
                                    enriched_cmd.to_commands(&config, ExecutionMode::LiveUpdate)
                                {
                                    immediate_commands.extend(add_qdisc);
                                }
                            } else if let Some(add_htb) =
                                enriched_cmd.to_commands(&config, ExecutionMode::Builder)
                            {
                                immediate_commands.extend(add_htb);
                            }
                        }
                        Some(LazyQueueMode::Full) => {
                            if was_activated {
                                if let Some(prune) = enriched_cmd.to_prune(&config, true) {
                                    immediate_commands.extend(prune);
                                }
                                if let Some(add_all) =
                                    enriched_cmd.to_commands(&config, ExecutionMode::LiveUpdate)
                                {
                                    immediate_commands.extend(add_all);
                                }
                            } else {
                                // No TC ops
                            }
                        }
                    }
                    circuits.insert(*circuit_hash, enriched_cmd);
                }
            }
            if live_tree_allowed && !immediate_commands.is_empty() {
                execute_and_record_live_change(&immediate_commands, "updating circuit speeds live");
            }
        }

        // 2b) Parent/class migrations
        if !categories.migrated.is_empty() {
            let Some(layout) = resolved_mq_layout.as_ref() else {
                warn!("Bakery: missing MQ layout during circuit migrations");
                return;
            };
            let live_reserved_handles =
                snapshot_live_qdisc_handle_majors_or_empty(&config, "circuit migrations");
            let down_live_snapshot = read_live_class_snapshot(&config.isp_interface()).ok();
            let up_live_snapshot = read_live_class_snapshot(&config.internet_interface()).ok();
            let mut immediate_commands = Vec::new();
            let mut migrated_updates = Vec::new();
            let mut prepared_migrations = Vec::new();
            let mut protected_down_classes = HashSet::new();
            let mut protected_up_classes = HashSet::new();
            for cmd in &categories.migrated {
                let mut enriched_cmd = with_assigned_qdisc_handles_reserved(
                    cmd,
                    &config,
                    layout,
                    qdisc_handles,
                    &live_reserved_handles,
                );
                let Some(old_cmd) = (if let BakeryCommands::AddCircuit { circuit_hash, .. } =
                    enriched_cmd.as_ref()
                {
                    circuits.get(circuit_hash).cloned()
                } else {
                    None
                }) else {
                    continue;
                };

                enriched_cmd = rotate_changed_qdisc_handles_reserved(
                    old_cmd.as_ref(),
                    &enriched_cmd,
                    &config,
                    layout,
                    qdisc_handles,
                    &live_reserved_handles,
                );
                let BakeryCommands::AddCircuit { circuit_hash, .. } = enriched_cmd.as_ref() else {
                    continue;
                };
                if let Some((down_class, up_class)) = circuit_class_handles(enriched_cmd.as_ref()) {
                    protected_down_classes.insert(down_class);
                    protected_up_classes.insert(up_class);
                }
                prepared_migrations.push((*circuit_hash, old_cmd, enriched_cmd));
            }

            for (circuit_hash, old_cmd, enriched_cmd) in prepared_migrations {
                if queue_live_migration(
                    old_cmd.as_ref(),
                    &enriched_cmd,
                    sites,
                    circuits,
                    live_circuits,
                    migrations,
                ) {
                    continue;
                }

                if live_circuits.contains_key(&circuit_hash) {
                    warn!(
                        "Bakery: falling back to full reload for active migrated circuit {} because live migration setup failed",
                        circuit_hash
                    );
                    let (_new_batch, mapped_limit_stats) = filter_batch_by_mapped_circuit_limit(
                        raw_batch.clone(),
                        &baseline_circuits,
                        effective_limit,
                    );
                    log_mapped_limit_decision(
                        "circuit-migration rebuild",
                        mapped_limit,
                        mapped_limit_stats,
                    );
                    let summary = format!(
                        "Bakery full reload triggered because live migration setup failed for active migrated circuit {}.",
                        circuit_hash
                    );
                    announce_full_reload(&summary);
                    full_reload(
                        batch,
                        sites,
                        circuits,
                        live_circuits,
                        mq_layout,
                        qdisc_handles,
                        &config,
                        raw_batch.clone(),
                        resolved_mq_layout.clone(),
                        stormguard_overrides,
                        virtualized_sites,
                        runtime_node_operations,
                        summary,
                    );
                    return;
                }

                match config.queues.lazy_queues.as_ref() {
                    None | Some(LazyQueueMode::No) => {
                        if let Some(prune) = observed_circuit_prune_commands(
                            &config,
                            old_cmd.as_ref(),
                            down_live_snapshot.as_ref(),
                            up_live_snapshot.as_ref(),
                            &protected_down_classes,
                            &protected_up_classes,
                        ) {
                            immediate_commands.extend(prune);
                        }
                        if let Some(add) = enriched_cmd.to_commands(&config, ExecutionMode::Builder)
                        {
                            immediate_commands.extend(add);
                        }
                    }
                    Some(LazyQueueMode::Htb) => {
                        if let Some(prune) = observed_circuit_prune_commands(
                            &config,
                            old_cmd.as_ref(),
                            down_live_snapshot.as_ref(),
                            up_live_snapshot.as_ref(),
                            &protected_down_classes,
                            &protected_up_classes,
                        ) {
                            immediate_commands.extend(prune);
                        }
                        if let Some(add_htb) =
                            enriched_cmd.to_commands(&config, ExecutionMode::Builder)
                        {
                            immediate_commands.extend(add_htb);
                        }
                    }
                    Some(LazyQueueMode::Full) => {}
                }
                migrated_updates.push((circuit_hash, enriched_cmd));
            }
            if !immediate_commands.is_empty() {
                let result = execute_and_record_live_change(
                    &immediate_commands,
                    "migrating circuits between parent nodes (fallback)",
                );
                if !result.ok {
                    let summary = summarize_apply_result(
                        "migrating circuits between parent nodes (fallback)",
                        &result,
                    );
                    mark_reload_required(format!(
                        "Bakery circuit parent-move fallback failed: {}. A full reload is now required before further incremental topology mutations.",
                        summary
                    ));
                    migrated_updates.clear();
                }
            }
            for (circuit_hash, enriched_cmd) in migrated_updates {
                circuits.insert(circuit_hash, enriched_cmd);
            }
        }

        // 3) Additions
        if !categories.newly_added.is_empty() {
            let mut accepted_additions: Vec<&Arc<BakeryCommands>> = Vec::new();
            let mut dropped_mapped_additions = 0usize;
            let mut requested_mapped_additions = 0usize;

            let mut mapped_in_state = circuits
                .values()
                .filter(|c| is_mapped_add_circuit(c.as_ref()))
                .count();

            for command in &categories.newly_added {
                if is_mapped_add_circuit(command.as_ref()) {
                    requested_mapped_additions += 1;
                    if let Some(limit) = effective_limit
                        && mapped_in_state >= limit
                    {
                        dropped_mapped_additions += 1;
                        continue;
                    }
                    mapped_in_state += 1;
                }
                accepted_additions.push(*command);
            }

            if dropped_mapped_additions > 0 {
                let stats = MappedLimitStats {
                    enforced_limit: effective_limit,
                    requested_mapped: requested_mapped_additions,
                    allowed_mapped: requested_mapped_additions
                        .saturating_sub(dropped_mapped_additions),
                    dropped_mapped: dropped_mapped_additions,
                };
                warn!(
                    "Bakery mapped circuit cap enforced (incremental additions): requested={}, allowed={}, dropped={}, limit={} (licensed={}, max_circuits={:?})",
                    stats.requested_mapped,
                    stats.allowed_mapped,
                    stats.dropped_mapped,
                    limit_label,
                    mapped_limit.licensed,
                    mapped_limit.max_circuits
                );
                maybe_emit_mapped_circuit_limit_urgent(&stats);
            }
            log_mapped_limit_decision(
                "incremental additions",
                mapped_limit,
                MappedLimitStats {
                    enforced_limit: effective_limit,
                    requested_mapped: requested_mapped_additions,
                    allowed_mapped: requested_mapped_additions
                        .saturating_sub(dropped_mapped_additions),
                    dropped_mapped: dropped_mapped_additions,
                },
            );

            let Some(layout) = resolved_mq_layout.as_ref() else {
                warn!(
                    "Bakery: missing MQ layout during incremental additions; skipping TC changes"
                );
                return;
            };
            let live_reserved_handles =
                snapshot_live_qdisc_handle_majors_or_empty(&config, "incremental additions");
            let enriched_additions: Vec<Arc<BakeryCommands>> = accepted_additions
                .iter()
                .map(|command| {
                    with_assigned_qdisc_handles_reserved(
                        command,
                        &config,
                        layout,
                        qdisc_handles,
                        &live_reserved_handles,
                    )
                })
                .collect();
            let commands: Vec<Vec<String>> = enriched_additions
                .iter()
                .filter_map(|c| c.to_commands(&config, ExecutionMode::Builder))
                .flatten()
                .collect();
            if !commands.is_empty() {
                execute_and_record_live_change(&commands, "adding new circuits");
            }
            for command in enriched_additions {
                if let BakeryCommands::AddCircuit { circuit_hash, .. } = command.as_ref() {
                    circuits.insert(*circuit_hash, command);
                }
            }
        }

        // 4) IP-only changes require no TC commands; mappings already handled by mapping engine
        // We still refresh the stored circuit snapshot for those entries
        if !categories.ip_changed.is_empty() {
            let Some(layout) = resolved_mq_layout.as_ref() else {
                warn!("Bakery: missing MQ layout during IP-only circuit updates");
                return;
            };
            let live_reserved_handles =
                snapshot_live_qdisc_handle_majors_or_empty(&config, "IP-only circuit updates");
            for command in categories.ip_changed {
                let enriched = with_assigned_qdisc_handles_reserved(
                    command,
                    &config,
                    layout,
                    qdisc_handles,
                    &live_reserved_handles,
                );
                if let BakeryCommands::AddCircuit { circuit_hash, .. } = enriched.as_ref() {
                    circuits.insert(*circuit_hash, enriched);
                }
            }
        }
    }

    *mq_layout = resolved_mq_layout;
    qdisc_handles.save(&config);
    update_queue_distribution_snapshot(sites, circuits);
}

fn handle_circuit_activity(
    circuit_ids: HashSet<i64>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };
    match config.queues.lazy_queues.as_ref() {
        None | Some(LazyQueueMode::No) => return,
        _ => {}
    }

    // Defer live activation until MQ and at least one commit has fully applied.
    if !MQ_CREATED.load(Ordering::Relaxed) || !FIRST_COMMIT_APPLIED.load(Ordering::Relaxed) {
        debug!(
            "Skipping live activation: MQ_CREATED={}, FIRST_COMMIT_APPLIED={}",
            MQ_CREATED.load(Ordering::Relaxed),
            FIRST_COMMIT_APPLIED.load(Ordering::Relaxed)
        );
        return;
    }

    let mut commands = Vec::new();
    for circuit_id in circuit_ids {
        if let Some(circuit) = live_circuits.get_mut(&circuit_id) {
            *circuit = current_timestamp();
            continue;
        }

        if let Some(command) = circuits.get(&circuit_id) {
            // On first activation, ensure HTB exists in HTB-lazy mode by prepending
            // Builder-mode HTB class creation (idempotent via "class replace").
            let mut cmd = Vec::new();
            match config.queues.lazy_queues.as_ref() {
                Some(LazyQueueMode::Htb) => {
                    if let Some(builder_cmds) = command.to_commands(&config, ExecutionMode::Builder)
                    {
                        cmd.extend(builder_cmds);
                    }
                    if let Some(live_cmds) = command.to_commands(&config, ExecutionMode::LiveUpdate)
                    {
                        cmd.extend(live_cmds);
                    }
                    if cmd.is_empty() {
                        // No commands to apply for this circuit
                        continue;
                    }
                }
                _ => {
                    // Full lazy mode handles both HTB and SQM in LiveUpdate; or other modes
                    let Some(live_cmds) = command.to_commands(&config, ExecutionMode::LiveUpdate)
                    else {
                        continue;
                    };
                    cmd.extend(live_cmds);
                }
            }
            live_circuits.insert(circuit_id, current_timestamp());
            commands.extend(cmd);
        }
    }
    if commands.is_empty() {
        return; // No commands to write
    }
    execute_and_record_live_change(&commands, "enabling live circuits");
}

fn handle_tick(
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
) {
    // This is a periodic tick to expire lazy queues
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };

    // Periodically shrink HashMap capacity if it's much larger than needed
    static mut TICK_COUNT: u64 = 0;
    unsafe {
        TICK_COUNT += 1;
        if TICK_COUNT.is_multiple_of(60) {
            // Every minute
            // Shrink if capacity is more than 2x the size
            if circuits.capacity() > circuits.len() * 2 && circuits.capacity() > 100 {
                debug!(
                    "Shrinking circuits HashMap: {} entries, {} capacity",
                    circuits.len(),
                    circuits.capacity()
                );
                circuits.shrink_to_fit();
            }
            if live_circuits.capacity() > live_circuits.len() * 2 && live_circuits.capacity() > 100
            {
                debug!(
                    "Shrinking live_circuits HashMap: {} entries, {} capacity",
                    live_circuits.len(),
                    live_circuits.capacity()
                );
                live_circuits.shrink_to_fit();
            }
            if sites.capacity() > sites.len() * 2 && sites.capacity() > 100 {
                debug!(
                    "Shrinking sites HashMap: {} entries, {} capacity",
                    sites.len(),
                    sites.capacity()
                );
                sites.shrink_to_fit();
            }
        }
    }

    match config.queues.lazy_queues.as_ref() {
        None | Some(LazyQueueMode::No) => {
            ACTIVE_CIRCUITS.store(circuits.len(), Ordering::Relaxed);
            return;
        }
        _ => {
            ACTIVE_CIRCUITS.store(live_circuits.len(), Ordering::Relaxed);
        }
    }

    // Now we know that lazy queues are enabled, we can expire them!
    let max_age_seconds = config.queues.lazy_expire_seconds.unwrap_or(600);
    if max_age_seconds == 0 {
        // If max_age_seconds is 0, we do not expire queues
        return;
    }

    let mut to_destroy = Vec::new();
    let now = current_timestamp();
    for (circuit_id, last_activity) in live_circuits.iter() {
        if now - *last_activity > max_age_seconds {
            to_destroy.push(*circuit_id);
        }
    }

    if to_destroy.is_empty() {
        return; // No queues to expire
    }

    let mut commands = Vec::new();
    for circuit_id in to_destroy {
        if let Some(command) = circuits.get(&circuit_id) {
            let Some(cmd) = command.to_prune(&config, false) else {
                continue;
            };
            live_circuits.remove(&circuit_id);
            commands.extend(cmd);
        }
    }

    if commands.is_empty() {
        return; // No commands to write
    }
    execute_and_record_live_change(&commands, "pruning lazy queues");
}

fn handle_change_site_speed_live(
    site_hash: i64,
    download_bandwidth_min: f32,
    upload_bandwidth_min: f32,
    download_bandwidth_max: f32,
    upload_bandwidth_max: f32,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };
    if let Some(site_arc) = sites.get(&site_hash) {
        let BakeryCommands::AddSite {
            site_hash: _,
            parent_class_id,
            up_parent_class_id,
            class_minor,
            ..
        } = site_arc.as_ref()
        else {
            debug!(
                "ChangeSiteSpeedLive received a non-site command: {:?}",
                site_arc
            );
            return;
        };
        let to_internet = config.internet_interface();
        let to_isp = config.isp_interface();
        let class_id = format!(
            "0x{:x}:0x{:x}",
            parent_class_id.get_major_minor().0,
            class_minor
        );
        let up_class_id = format!(
            "0x{:x}:0x{:x}",
            up_parent_class_id.get_major_minor().0,
            class_minor
        );
        let upload_bandwidth_min = if upload_bandwidth_min >= (upload_bandwidth_max - 0.5) {
            upload_bandwidth_max - 1.0
        } else {
            upload_bandwidth_min
        };
        let download_bandwidth_min = if download_bandwidth_min >= (download_bandwidth_max - 0.5) {
            download_bandwidth_max - 1.0
        } else {
            download_bandwidth_min
        };
        if let Some(reason) = live_tree_mutation_blocker_for_config(&config) {
            let summary = format!(
                "Skipping live site speed change for site {} because {}.",
                site_hash, reason
            );
            info!("{summary}");
            push_bakery_event_with_site(
                "live_site_speed_skipped",
                "info",
                Some(site_hash),
                summary,
            );
            let new_site = Arc::new(BakeryCommands::AddSite {
                site_hash,
                parent_class_id: *parent_class_id,
                up_parent_class_id: *up_parent_class_id,
                class_minor: *class_minor,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
            });
            sites.insert(site_hash, new_site);
            return;
        }
        let commands = vec![
            vec![
                "class".to_string(),
                "change".to_string(),
                "dev".to_string(),
                to_internet,
                "classid".to_string(),
                up_class_id,
                "htb".to_string(),
                "rate".to_string(),
                format_rate_for_tc_f32(upload_bandwidth_min),
                "ceil".to_string(),
                format_rate_for_tc_f32(upload_bandwidth_max),
                "prio".to_string(),
                "3".to_string(),
                "quantum".to_string(),
                quantum(
                    upload_bandwidth_max as u64,
                    r2q(config.queues.uplink_bandwidth_mbps),
                ),
            ],
            vec![
                "class".to_string(),
                "change".to_string(),
                "dev".to_string(),
                to_isp,
                "classid".to_string(),
                class_id,
                "htb".to_string(),
                "rate".to_string(),
                format_rate_for_tc_f32(download_bandwidth_min),
                "ceil".to_string(),
                format_rate_for_tc_f32(download_bandwidth_max),
                "prio".to_string(),
                "3".to_string(),
                "quantum".to_string(),
                quantum(
                    download_bandwidth_max as u64,
                    r2q(config.queues.downlink_bandwidth_mbps),
                ),
            ],
        ];
        execute_and_record_live_change(&commands, "changing site speed live");
        // Update the site speeds in the site map - create a new Arc with updated values
        let new_site = Arc::new(BakeryCommands::AddSite {
            site_hash,
            parent_class_id: *parent_class_id,
            up_parent_class_id: *up_parent_class_id,
            class_minor: *class_minor,
            download_bandwidth_min,
            upload_bandwidth_min,
            download_bandwidth_max,
            upload_bandwidth_max,
        });
        sites.insert(site_hash, new_site);
    } else {
        info!(
            "ChangeSiteSpeedLive received for unknown site: {}",
            site_hash
        );
    }
}

fn site_class_handles(site: &BakeryCommands) -> Option<(TcHandle, TcHandle)> {
    let BakeryCommands::AddSite {
        parent_class_id,
        up_parent_class_id,
        class_minor,
        ..
    } = site
    else {
        return None;
    };
    Some((
        tc_handle_from_major_minor(parent_class_id.get_major_minor().0, *class_minor),
        tc_handle_from_major_minor(up_parent_class_id.get_major_minor().0, *class_minor),
    ))
}

fn site_is_top_level(site: &BakeryCommands) -> bool {
    let BakeryCommands::AddSite {
        parent_class_id,
        up_parent_class_id,
        ..
    } = site
    else {
        return false;
    };
    let (_, down_parent_minor) = parent_class_id.get_major_minor();
    let (_, up_parent_minor) = up_parent_class_id.get_major_minor();
    matches!(down_parent_minor, 0 | 3) && matches!(up_parent_minor, 0 | 3)
}

fn site_runtime_virtualization_eligibility_error(site: &BakeryCommands) -> Option<String> {
    let BakeryCommands::AddSite {
        site_hash,
        parent_class_id,
        up_parent_class_id,
        class_minor,
        ..
    } = site
    else {
        return Some("TreeGuard runtime virtualization requires an AddSite command".to_string());
    };

    if site_is_top_level(site) {
        return Some(format!(
            "Site {} is top-level in HTB and cannot be runtime-virtualized in v1",
            site_hash
        ));
    }

    let Some((site_down_class, site_up_class)) = site_class_handles(site) else {
        return Some(format!("Site {} is not a valid AddSite command", site_hash));
    };

    let (_, down_parent_minor) = parent_class_id.get_major_minor();
    let (_, up_parent_minor) = up_parent_class_id.get_major_minor();
    if u32::from(*class_minor) >= ACTIVE_RUNTIME_MINOR_START
        || u32::from(down_parent_minor) >= ACTIVE_RUNTIME_MINOR_START
        || u32::from(up_parent_minor) >= ACTIVE_RUNTIME_MINOR_START
    {
        return Some(format!(
            "Site {} is already inside a retained runtime shadow branch and cannot be nested in v1",
            site_hash
        ));
    }

    let (parent_down_major, _) = parent_class_id.get_major_minor();
    let (parent_up_major, _) = up_parent_class_id.get_major_minor();
    let (site_down_major, _) = site_down_class.get_major_minor();
    let (site_up_major, _) = site_up_class.get_major_minor();

    if parent_down_major != site_down_major || parent_up_major != site_up_major {
        return Some(format!(
            "Site {} crosses queue/major domains (parent down/up majors {:x}/{:x}, site down/up majors {:x}/{:x}) and cannot be runtime-virtualized in v1",
            site_hash, parent_down_major, parent_up_major, site_down_major, site_up_major
        ));
    }

    None
}

fn top_level_runtime_virtualization_eligibility_error(
    site_hash: i64,
    target_site: &BakeryCommands,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
) -> Option<RuntimeNodeEligibilityError> {
    if !site_is_top_level(target_site) {
        return Some(RuntimeNodeEligibilityError::plain(format!(
            "Site {} is not top-level and cannot use the top-level runtime virtualization path",
            site_hash
        )));
    }

    if current_site_queue(target_site).is_none() {
        return Some(RuntimeNodeEligibilityError::plain(format!(
            "Site {} does not have a deterministic current queue assignment",
            site_hash
        )));
    }

    let children_by_parent = direct_child_sites_by_parent(sites);
    let child_sites = children_by_parent
        .get(&site_hash)
        .map(|children| children.len())
        .unwrap_or(0);
    let direct_circuits = site_class_handles(target_site)
        .map(|(down, up)| collect_direct_circuit_hashes(circuits, down, up).len())
        .unwrap_or(0);

    if child_sites == 0 && direct_circuits == 0 {
        return Some(RuntimeNodeEligibilityError::new(
            format!(
                "Site {} has no direct child sites or direct circuits to promote safely",
                site_hash
            ),
            RuntimeNodeOperationFailureReason::StructuralIneligibleNoPromotableChildren,
        ));
    }

    if child_sites + direct_circuits < 2 {
        return Some(RuntimeNodeEligibilityError::new(
            format!(
                "Site {} has only one promotable direct child, so top-level runtime virtualization would not produce a deterministic v1 split point",
                site_hash
            ),
            RuntimeNodeOperationFailureReason::StructuralIneligibleSinglePromotableChild,
        ));
    }

    None
}

#[derive(Clone, Debug)]
struct RuntimeNodeEligibilityError {
    message: String,
    failure_reason: Option<RuntimeNodeOperationFailureReason>,
}

impl RuntimeNodeEligibilityError {
    fn plain(message: String) -> Self {
        Self {
            message,
            failure_reason: None,
        }
    }

    fn new(message: String, failure_reason: RuntimeNodeOperationFailureReason) -> Self {
        Self {
            message,
            failure_reason: Some(failure_reason),
        }
    }
}

fn nested_runtime_shadow_branch_eligibility_error(
    site: &BakeryCommands,
) -> Option<RuntimeNodeEligibilityError> {
    let BakeryCommands::AddSite {
        site_hash,
        parent_class_id,
        up_parent_class_id,
        class_minor,
        ..
    } = site
    else {
        return None;
    };

    let (_, down_parent_minor) = parent_class_id.get_major_minor();
    let (_, up_parent_minor) = up_parent_class_id.get_major_minor();
    if u32::from(*class_minor) < ACTIVE_RUNTIME_MINOR_START
        && u32::from(down_parent_minor) < ACTIVE_RUNTIME_MINOR_START
        && u32::from(up_parent_minor) < ACTIVE_RUNTIME_MINOR_START
    {
        return None;
    }

    Some(RuntimeNodeEligibilityError::new(
        format!(
            "Site {} is already inside a retained runtime shadow branch and cannot be nested in v1",
            site_hash
        ),
        RuntimeNodeOperationFailureReason::StructuralIneligibleNestedRuntimeBranch,
    ))
}

fn site_prune_commands(
    config: &Arc<Config>,
    state: &VirtualizedSiteState,
) -> Option<Vec<Vec<String>>> {
    let mut commands = site_prune_qdisc_commands(config, &state.qdisc_handles).unwrap_or_default();
    commands.extend(site_prune_class_commands(config, state.site.as_ref())?);
    Some(commands)
}

fn site_prune_qdisc_commands(
    config: &Arc<Config>,
    handles: &VirtualizedSiteQdiscHandles,
) -> Option<Vec<Vec<String>>> {
    let mut commands = Vec::new();
    if let Some(handle) = handles.down {
        commands.push(vec![
            "qdisc".to_string(),
            "del".to_string(),
            "dev".to_string(),
            config.isp_interface(),
            "handle".to_string(),
            format!("0x{:x}:", handle),
        ]);
    }
    if let Some(handle) = handles.up {
        commands.push(vec![
            "qdisc".to_string(),
            "del".to_string(),
            "dev".to_string(),
            config.internet_interface(),
            "handle".to_string(),
            format!("0x{:x}:", handle),
        ]);
    }
    if commands.is_empty() {
        None
    } else {
        Some(commands)
    }
}

fn site_prune_class_commands(
    config: &Arc<Config>,
    site: &BakeryCommands,
) -> Option<Vec<Vec<String>>> {
    let BakeryCommands::AddSite {
        parent_class_id,
        up_parent_class_id,
        class_minor,
        ..
    } = site
    else {
        return None;
    };

    Some(vec![
        vec![
            "class".to_string(),
            "del".to_string(),
            "dev".to_string(),
            config.isp_interface(),
            "parent".to_string(),
            parent_class_id.as_tc_string(),
            "classid".to_string(),
            format!(
                "0x{:x}:0x{:x}",
                parent_class_id.get_major_minor().0,
                class_minor
            ),
        ],
        vec![
            "class".to_string(),
            "del".to_string(),
            "dev".to_string(),
            config.internet_interface(),
            "parent".to_string(),
            up_parent_class_id.as_tc_string(),
            "classid".to_string(),
            format!(
                "0x{:x}:0x{:x}",
                up_parent_class_id.get_major_minor().0,
                class_minor
            ),
        ],
    ])
}

fn rebuild_site_command(
    site: &Arc<BakeryCommands>,
    parent_class_id: TcHandle,
    up_parent_class_id: TcHandle,
    class_minor: u16,
) -> Option<Arc<BakeryCommands>> {
    let BakeryCommands::AddSite {
        site_hash,
        download_bandwidth_min,
        upload_bandwidth_min,
        download_bandwidth_max,
        upload_bandwidth_max,
        ..
    } = site.as_ref()
    else {
        return None;
    };

    Some(Arc::new(BakeryCommands::AddSite {
        site_hash: *site_hash,
        parent_class_id,
        up_parent_class_id,
        class_minor,
        download_bandwidth_min: *download_bandwidth_min,
        upload_bandwidth_min: *upload_bandwidth_min,
        download_bandwidth_max: *download_bandwidth_max,
        upload_bandwidth_max: *upload_bandwidth_max,
    }))
}

fn reparent_circuit_command(
    circuit: &Arc<BakeryCommands>,
    parent_class_id: TcHandle,
    up_parent_class_id: TcHandle,
) -> Option<Arc<BakeryCommands>> {
    let BakeryCommands::AddCircuit {
        circuit_hash,
        circuit_name,
        site_name,
        class_minor,
        download_bandwidth_min,
        upload_bandwidth_min,
        download_bandwidth_max,
        upload_bandwidth_max,
        class_major,
        up_class_major,
        ip_addresses,
        sqm_override,
        ..
    } = circuit.as_ref()
    else {
        return None;
    };

    Some(Arc::new(BakeryCommands::AddCircuit {
        circuit_hash: *circuit_hash,
        circuit_name: circuit_name.clone(),
        site_name: site_name.clone(),
        parent_class_id,
        up_parent_class_id,
        class_minor: *class_minor,
        download_bandwidth_min: *download_bandwidth_min,
        upload_bandwidth_min: *upload_bandwidth_min,
        download_bandwidth_max: *download_bandwidth_max,
        upload_bandwidth_max: *upload_bandwidth_max,
        class_major: *class_major,
        up_class_major: *up_class_major,
        down_qdisc_handle: None,
        up_qdisc_handle: None,
        ip_addresses: ip_addresses.clone(),
        sqm_override: sqm_override.clone(),
    }))
}

fn collect_direct_circuit_hashes(
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    target_down: TcHandle,
    target_up: TcHandle,
) -> Vec<i64> {
    let mut hashes: Vec<i64> = circuits
        .iter()
        .filter_map(|(circuit_hash, circuit)| {
            let BakeryCommands::AddCircuit {
                parent_class_id,
                up_parent_class_id,
                ..
            } = circuit.as_ref()
            else {
                return None;
            };
            (*parent_class_id == target_down && *up_parent_class_id == target_up)
                .then_some(*circuit_hash)
        })
        .collect();
    hashes.sort_unstable();
    hashes
}

#[derive(Clone, Debug)]
struct PlannedSiteUpdate {
    queue: u32,
    parent_site: Option<i64>,
    stage_depth: usize,
    command: Arc<BakeryCommands>,
}

#[derive(Clone, Debug)]
struct PlannedCircuitUpdate {
    queue: u32,
    parent_site: Option<i64>,
    command: Arc<BakeryCommands>,
}

#[derive(Clone, Debug)]
struct TopLevelVirtualizationPlan {
    saved_sites: HashMap<i64, Arc<BakeryCommands>>,
    saved_circuits: HashMap<i64, Arc<BakeryCommands>>,
    active_sites: HashMap<i64, PlannedSiteUpdate>,
    active_circuits: HashMap<i64, PlannedCircuitUpdate>,
    site_stages: Vec<Vec<i64>>,
}

fn site_hash_from_command(site: &BakeryCommands) -> Option<i64> {
    if let BakeryCommands::AddSite { site_hash, .. } = site {
        Some(*site_hash)
    } else {
        None
    }
}

fn circuit_hash_from_command(circuit: &BakeryCommands) -> Option<i64> {
    if let BakeryCommands::AddCircuit { circuit_hash, .. } = circuit {
        Some(*circuit_hash)
    } else {
        None
    }
}

fn site_max_weight(site: &BakeryCommands) -> f64 {
    if let BakeryCommands::AddSite {
        download_bandwidth_max,
        upload_bandwidth_max,
        ..
    } = site
    {
        f64::from(*download_bandwidth_max + *upload_bandwidth_max)
    } else {
        1.0
    }
}

fn circuit_max_weight(circuit: &BakeryCommands) -> f64 {
    if let BakeryCommands::AddCircuit {
        download_bandwidth_max,
        upload_bandwidth_max,
        ..
    } = circuit
    {
        f64::from(*download_bandwidth_max + *upload_bandwidth_max)
    } else {
        1.0
    }
}

fn current_site_queue(site: &BakeryCommands) -> Option<u32> {
    let BakeryCommands::AddSite {
        parent_class_id,
        class_minor,
        ..
    } = site
    else {
        return None;
    };
    let (major, _) = parent_class_id.get_major_minor();
    let class_handle = tc_handle_from_major_minor(major, *class_minor);
    Some(u32::from(class_handle.get_major_minor().0))
}

fn current_circuit_queue(circuit: &BakeryCommands) -> Option<u32> {
    let BakeryCommands::AddCircuit { class_major, .. } = circuit else {
        return None;
    };
    Some(u32::from(*class_major))
}

fn circuit_class_handles(circuit: &BakeryCommands) -> Option<(TcHandle, TcHandle)> {
    let BakeryCommands::AddCircuit {
        class_minor,
        class_major,
        up_class_major,
        ..
    } = circuit
    else {
        return None;
    };

    Some((
        tc_handle_from_major_minor(*class_major, *class_minor),
        tc_handle_from_major_minor(*up_class_major, *class_minor),
    ))
}

fn rebuild_queue_distribution_snapshot(
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
) -> Vec<BakeryQueueDistributionSnapshot> {
    #[derive(Default)]
    struct QueueBucket {
        top_level_site_count: usize,
        site_count: usize,
        circuit_count: usize,
        download_mbps: f64,
        upload_mbps: f64,
    }

    let mut buckets: BTreeMap<u32, QueueBucket> = BTreeMap::new();

    for site in sites.values() {
        let Some(queue) = current_site_queue(site.as_ref()) else {
            continue;
        };
        let bucket = buckets.entry(queue).or_default();
        bucket.site_count += 1;
        if site_is_top_level(site.as_ref()) {
            bucket.top_level_site_count += 1;
        }
    }

    for circuit in circuits.values() {
        let BakeryCommands::AddCircuit {
            class_major,
            download_bandwidth_max,
            upload_bandwidth_max,
            ..
        } = circuit.as_ref()
        else {
            continue;
        };
        let bucket = buckets.entry(u32::from(*class_major)).or_default();
        bucket.circuit_count += 1;
        bucket.download_mbps += f64::from(*download_bandwidth_max);
        bucket.upload_mbps += f64::from(*upload_bandwidth_max);
    }

    buckets
        .into_iter()
        .map(|(queue, bucket)| BakeryQueueDistributionSnapshot {
            queue,
            top_level_site_count: bucket.top_level_site_count,
            site_count: bucket.site_count,
            circuit_count: bucket.circuit_count,
            download_mbps: bucket.download_mbps.round().max(0.0) as u64,
            upload_mbps: bucket.upload_mbps.round().max(0.0) as u64,
        })
        .collect()
}

fn update_queue_distribution_snapshot(
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
) {
    telemetry_state().write().queue_distribution =
        rebuild_queue_distribution_snapshot(sites, circuits);
}

fn rebuild_runtime_operations_snapshot(
    runtime_node_operations: &HashMap<i64, RuntimeNodeOperation>,
) -> BakeryRuntimeOperationsSnapshot {
    let mut submitted_count = 0usize;
    let mut deferred_count = 0usize;
    let mut applying_count = 0usize;
    let mut awaiting_cleanup_count = 0usize;
    let mut failed_count = 0usize;
    let mut blocked_count = 0usize;
    let mut dirty_count = 0usize;

    let latest = runtime_node_operations
        .values()
        .inspect(|operation| match operation.status {
            RuntimeNodeOperationStatus::Submitted => submitted_count += 1,
            RuntimeNodeOperationStatus::Deferred => deferred_count += 1,
            RuntimeNodeOperationStatus::Applying => applying_count += 1,
            RuntimeNodeOperationStatus::AppliedAwaitingCleanup => awaiting_cleanup_count += 1,
            RuntimeNodeOperationStatus::Failed => {
                if operation.failure_reason.is_some() {
                    blocked_count += 1;
                } else {
                    failed_count += 1;
                }
            }
            RuntimeNodeOperationStatus::Dirty => dirty_count += 1,
            RuntimeNodeOperationStatus::Completed => {}
        })
        .max_by_key(|operation| operation.updated_at_unix)
        .map(|operation| BakeryRuntimeOperationHeadlineSnapshot {
            operation_id: operation.operation_id,
            site_hash: operation.site_hash,
            site_name: operation.site_name.clone(),
            action: operation.action,
            status: operation.status,
            attempt_count: operation.attempt_count,
            updated_at_unix: operation.updated_at_unix,
            next_retry_at_unix: operation.next_retry_at_unix,
            last_error: operation.last_error.clone(),
        });

    BakeryRuntimeOperationsSnapshot {
        submitted_count,
        deferred_count,
        applying_count,
        awaiting_cleanup_count,
        failed_count,
        blocked_count,
        dirty_count,
        latest,
    }
}

fn observed_circuit_prune_commands(
    config: &Arc<Config>,
    old_cmd: &BakeryCommands,
    down_snapshot: Option<&HashMap<TcHandle, LiveTcClassEntry>>,
    up_snapshot: Option<&HashMap<TcHandle, LiveTcClassEntry>>,
    protected_down_classes: &HashSet<TcHandle>,
    protected_up_classes: &HashSet<TcHandle>,
) -> Option<Vec<Vec<String>>> {
    let BakeryCommands::AddCircuit {
        parent_class_id,
        up_parent_class_id,
        class_minor,
        class_major,
        up_class_major,
        sqm_override,
        ..
    } = old_cmd
    else {
        return old_cmd.to_prune(config, true);
    };

    let down_class = TcHandle::from_u32((u32::from(*class_major) << 16) | u32::from(*class_minor));
    let up_class = TcHandle::from_u32((u32::from(*up_class_major) << 16) | u32::from(*class_minor));

    let (down_override_opt, up_override_opt) = match sqm_override.as_ref() {
        None => (None, None),
        Some(s) if s.contains('/') => {
            let mut it = s.splitn(2, '/');
            let down = it.next().unwrap_or("").trim();
            let up = it.next().unwrap_or("").trim();
            let map = |t: &str| -> Option<String> {
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            };
            (map(down), map(up))
        }
        Some(s) => (Some(s.clone()), Some(s.clone())),
    };

    let prune_down_qdisc =
        !matches!(down_override_opt.as_deref(), Some(s) if s.eq_ignore_ascii_case("none"));
    let prune_up_qdisc =
        !matches!(up_override_opt.as_deref(), Some(s) if s.eq_ignore_ascii_case("none"));

    let mut result = Vec::new();

    if !protected_up_classes.contains(&up_class)
        && let Some(snapshot) = up_snapshot
        && let Some(entry) = snapshot.get(&up_class)
    {
        if prune_up_qdisc && !config.on_a_stick_mode() && entry.leaf_qdisc_major.is_some() {
            result.push(vec![
                "qdisc".to_string(),
                "del".to_string(),
                "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(),
                format!("0x{:x}:0x{:x}", up_class_major, class_minor),
            ]);
        }
        result.push(vec![
            "class".to_string(),
            "del".to_string(),
            "dev".to_string(),
            config.internet_interface(),
            "parent".to_string(),
            up_parent_class_id.as_tc_string(),
            "classid".to_string(),
            format!(
                "0x{:x}:0x{:x}",
                up_parent_class_id.get_major_minor().0,
                class_minor
            ),
        ]);
    }

    if !protected_down_classes.contains(&down_class)
        && let Some(snapshot) = down_snapshot
        && let Some(entry) = snapshot.get(&down_class)
    {
        if prune_down_qdisc && entry.leaf_qdisc_major.is_some() {
            result.push(vec![
                "qdisc".to_string(),
                "del".to_string(),
                "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(),
                format!("0x{:x}:0x{:x}", class_major, class_minor),
            ]);
        }
        result.push(vec![
            "class".to_string(),
            "del".to_string(),
            "dev".to_string(),
            config.isp_interface(),
            "parent".to_string(),
            parent_class_id.as_tc_string(),
            "classid".to_string(),
            format!(
                "0x{:x}:0x{:x}",
                parent_class_id.get_major_minor().0,
                class_minor
            ),
        ]);
    }

    if result.is_empty() {
        old_cmd.to_prune(config, true).map(|_| Vec::new())
    } else {
        Some(result)
    }
}

fn site_parent_hash(
    site_hash: i64,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    class_to_site: &HashMap<(TcHandle, TcHandle), i64>,
) -> Option<i64> {
    let site = sites.get(&site_hash)?;
    let BakeryCommands::AddSite {
        parent_class_id,
        up_parent_class_id,
        ..
    } = site.as_ref()
    else {
        return None;
    };
    class_to_site
        .get(&(*parent_class_id, *up_parent_class_id))
        .copied()
}

fn direct_child_sites_by_parent(
    sites: &HashMap<i64, Arc<BakeryCommands>>,
) -> HashMap<i64, Vec<i64>> {
    let mut class_to_site = HashMap::new();
    for (site_hash, site) in sites {
        if let Some(handles) = site_class_handles(site.as_ref()) {
            class_to_site.insert(handles, *site_hash);
        }
    }

    let mut result: HashMap<i64, Vec<i64>> = HashMap::new();
    for site_hash in sites.keys().copied() {
        if let Some(parent_hash) = site_parent_hash(site_hash, sites, &class_to_site) {
            result.entry(parent_hash).or_default().push(site_hash);
        }
    }
    for children in result.values_mut() {
        children.sort_unstable();
    }
    result
}

fn collect_site_subtree_hashes(
    root_hash: i64,
    children_by_parent: &HashMap<i64, Vec<i64>>,
) -> Vec<i64> {
    let mut ordered = Vec::new();
    let mut stack = vec![root_hash];
    while let Some(site_hash) = stack.pop() {
        ordered.push(site_hash);
        if let Some(children) = children_by_parent.get(&site_hash) {
            for child in children.iter().rev() {
                stack.push(*child);
            }
        }
    }
    ordered
}

fn collect_circuits_attached_to_sites(
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    site_hashes: &[i64],
) -> Vec<i64> {
    let mut site_handles = HashSet::new();
    for site_hash in site_hashes {
        if let Some(site) = sites.get(site_hash)
            && let Some(handles) = site_class_handles(site.as_ref())
        {
            site_handles.insert(handles);
        }
    }

    let mut result: Vec<i64> = circuits
        .iter()
        .filter_map(|(circuit_hash, circuit)| {
            let BakeryCommands::AddCircuit {
                parent_class_id,
                up_parent_class_id,
                ..
            } = circuit.as_ref()
            else {
                return None;
            };
            site_handles
                .contains(&(*parent_class_id, *up_parent_class_id))
                .then_some(*circuit_hash)
        })
        .collect();
    result.sort_unstable();
    result
}

fn root_handle_for_queue(queue: u32) -> TcHandle {
    TcHandle::from_u32(queue << 16)
}

fn site_stick_offset(site: &BakeryCommands) -> u16 {
    let BakeryCommands::AddSite {
        parent_class_id,
        up_parent_class_id,
        ..
    } = site
    else {
        return 0;
    };
    let (down_major, _) = parent_class_id.get_major_minor();
    let (up_major, _) = up_parent_class_id.get_major_minor();
    up_major.saturating_sub(down_major)
}

fn top_level_bin_name(queue: u32) -> String {
    format!("CpueQueue{}", queue.saturating_sub(1))
}

fn queue_from_bin_name(name: &str) -> Option<u32> {
    name.strip_prefix("CpueQueue")
        .and_then(|v| v.parse::<u32>().ok())
        .map(|cpu| cpu + 1)
}

fn planner_site_key(site_hash: i64) -> String {
    format!("site:{site_hash}")
}

fn planner_circuit_key(circuit_hash: i64) -> String {
    format!("circuit:{circuit_hash}")
}

fn build_current_planner_state(
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
) -> (
    BTreeMap<String, PlannerSiteIdentityState>,
    BTreeMap<String, PlannerCircuitIdentityState>,
) {
    let mut class_to_site = HashMap::new();
    for (site_hash, site) in sites {
        if let Some(handles) = site_class_handles(site.as_ref()) {
            class_to_site.insert(handles, *site_hash);
        }
    }

    let mut previous_sites = BTreeMap::new();
    for (site_hash, site) in sites {
        let Some(queue) = current_site_queue(site.as_ref()) else {
            continue;
        };
        let Some(class_minor) = (if let BakeryCommands::AddSite { class_minor, .. } = site.as_ref()
        {
            Some(*class_minor)
        } else {
            None
        }) else {
            continue;
        };
        let parent_path = site_parent_hash(*site_hash, sites, &class_to_site)
            .map(planner_site_key)
            .unwrap_or_default();
        previous_sites.insert(
            planner_site_key(*site_hash),
            PlannerSiteIdentityState {
                class_minor,
                queue,
                parent_path,
                class_major: queue as u16,
                up_class_major: 0,
            },
        );
    }

    let mut previous_circuits = BTreeMap::new();
    for (circuit_hash, circuit) in circuits {
        let BakeryCommands::AddCircuit {
            class_minor,
            class_major,
            up_class_major,
            parent_class_id,
            up_parent_class_id,
            ..
        } = circuit.as_ref()
        else {
            continue;
        };
        let parent_node = class_to_site
            .get(&(*parent_class_id, *up_parent_class_id))
            .map(|site_hash| planner_site_key(*site_hash))
            .unwrap_or_else(|| format!("root:{}", class_major));
        previous_circuits.insert(
            planner_circuit_key(*circuit_hash),
            PlannerCircuitIdentityState {
                class_minor: *class_minor,
                queue: u32::from(*class_major),
                parent_node,
                class_major: *class_major,
                up_class_major: *up_class_major,
            },
        );
    }

    (previous_sites, previous_circuits)
}

fn build_effective_runtime_state(
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    virtualized_sites: &HashMap<i64, VirtualizedSiteState>,
) -> (
    HashMap<i64, Arc<BakeryCommands>>,
    HashMap<i64, Arc<BakeryCommands>>,
) {
    if virtualized_sites.is_empty() {
        return (sites.clone(), circuits.clone());
    }

    let mut effective_sites = sites.clone();
    let mut effective_circuits = circuits.clone();

    for (site_hash, state) in virtualized_sites {
        if state.active_branch_hides_original_site() {
            effective_sites.remove(site_hash);
        }
        for (hash, command) in &state.active_sites {
            effective_sites.insert(*hash, Arc::clone(command));
        }
        for (hash, command) in &state.active_circuits {
            effective_circuits.insert(*hash, Arc::clone(command));
        }
    }

    (effective_sites, effective_circuits)
}

fn reserve_live_snapshot_minors(
    reservations: &mut PlannerMinorReservations,
    snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
) {
    for handle in snapshot.keys() {
        let (major, minor) = handle.get_major_minor();
        if major == 0 || minor == 0 {
            continue;
        }
        reservations
            .entry(u32::from(major))
            .or_default()
            .insert(u32::from(minor));
    }
}

fn top_level_site_hashes(sites: &HashMap<i64, Arc<BakeryCommands>>) -> Vec<i64> {
    let mut hashes: Vec<i64> = sites
        .iter()
        .filter_map(|(site_hash, site)| site_is_top_level(site.as_ref()).then_some(*site_hash))
        .collect();
    hashes.sort_unstable();
    hashes
}

fn site_descendant_depth(site_hash: i64, parents: &HashMap<i64, Option<i64>>) -> usize {
    let mut depth = 0usize;
    let mut current = parents.get(&site_hash).copied().flatten();
    while let Some(parent) = current {
        depth += 1;
        current = parents.get(&parent).copied().flatten();
    }
    depth
}

fn build_top_level_virtualization_plan(
    target_site: Arc<BakeryCommands>,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    config: Option<&Arc<Config>>,
    virtualized_sites: &HashMap<i64, VirtualizedSiteState>,
    stick_offset: u16,
) -> Result<TopLevelVirtualizationPlan, String> {
    let Some(target_site_hash) = site_hash_from_command(target_site.as_ref()) else {
        return Err("TreeGuard top-level virtualization target is not a site".to_string());
    };
    if !site_is_top_level(target_site.as_ref()) {
        return Err(format!(
            "Site {} is not top-level and should use the v1 same-queue runtime virtualization path",
            target_site_hash
        ));
    }

    let children_by_parent = direct_child_sites_by_parent(sites);
    let Some((target_down_class, target_up_class)) = site_class_handles(target_site.as_ref())
    else {
        return Err(format!(
            "Site {} is not a valid AddSite command",
            target_site_hash
        ));
    };

    let promoted_site_roots = children_by_parent
        .get(&target_site_hash)
        .cloned()
        .unwrap_or_default();
    let direct_circuits =
        collect_direct_circuit_hashes(circuits, target_down_class, target_up_class);
    let top_level_sites = top_level_site_hashes(sites);

    let bins: Vec<String> = top_level_sites
        .iter()
        .filter_map(|site_hash| {
            sites
                .get(site_hash)
                .and_then(|site| current_site_queue(site.as_ref()))
                .map(top_level_bin_name)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if bins.is_empty() {
        return Err(
            "No top-level queues available for runtime top-level virtualization".to_string(),
        );
    }

    let target_queue = current_site_queue(target_site.as_ref()).unwrap_or(1);
    let mut planner_items = Vec::new();
    let mut prev_assign = BTreeMap::new();
    for site_hash in &promoted_site_roots {
        let Some(site) = sites.get(site_hash) else {
            continue;
        };
        let id = planner_site_key(*site_hash);
        planner_items.push(TopLevelPlannerItem {
            id: id.clone(),
            weight: site_max_weight(site.as_ref()),
        });
        prev_assign.insert(id, top_level_bin_name(target_queue));
    }
    for circuit_hash in &direct_circuits {
        let Some(circuit) = circuits.get(circuit_hash) else {
            continue;
        };
        let id = planner_circuit_key(*circuit_hash);
        planner_items.push(TopLevelPlannerItem {
            id: id.clone(),
            weight: circuit_max_weight(circuit.as_ref()),
        });
        prev_assign.insert(id, top_level_bin_name(target_queue));
    }

    let planner = plan_top_level_assignments(
        &planner_items,
        &bins,
        &prev_assign,
        &BTreeMap::new(),
        current_timestamp() as f64,
        &TopLevelPlannerParams {
            mode: TopLevelPlannerMode::StableGreedy,
            move_budget_per_run: planner_items.len().clamp(1, 32),
            ..TopLevelPlannerParams::default()
        },
    );

    let mut future_top_level_queue = HashMap::new();
    for site_hash in &top_level_sites {
        if *site_hash == target_site_hash {
            continue;
        }
        let Some(site) = sites.get(site_hash) else {
            continue;
        };
        let Some(queue) = current_site_queue(site.as_ref()) else {
            continue;
        };
        future_top_level_queue.insert(*site_hash, queue);
    }
    for site_hash in &promoted_site_roots {
        let key = planner_site_key(*site_hash);
        let assigned = planner
            .assignment
            .get(&key)
            .or_else(|| prev_assign.get(&key))
            .ok_or_else(|| {
                format!(
                    "Planner did not assign promoted top-level site {}",
                    site_hash
                )
            })?;
        let queue = queue_from_bin_name(assigned)
            .ok_or_else(|| format!("Invalid planner queue assignment {}", assigned))?;
        future_top_level_queue.insert(*site_hash, queue);
    }

    let mut moved_top_level_roots = promoted_site_roots.clone();
    moved_top_level_roots.sort_unstable();
    moved_top_level_roots.dedup();

    let mut saved_sites = HashMap::new();
    let mut saved_circuits = HashMap::new();
    let mut affected_site_hashes = HashSet::new();
    for site_hash in &moved_top_level_roots {
        for hash in collect_site_subtree_hashes(*site_hash, &children_by_parent) {
            affected_site_hashes.insert(hash);
        }
    }
    for site_hash in &affected_site_hashes {
        if let Some(site) = sites.get(site_hash) {
            saved_sites.insert(*site_hash, Arc::clone(site));
        }
    }
    let affected_site_hashes_vec: Vec<i64> = affected_site_hashes.iter().copied().collect();
    for circuit_hash in
        collect_circuits_attached_to_sites(circuits, sites, &affected_site_hashes_vec)
    {
        if let Some(circuit) = circuits.get(&circuit_hash) {
            saved_circuits.insert(circuit_hash, Arc::clone(circuit));
        }
    }
    for circuit_hash in &direct_circuits {
        if let Some(circuit) = circuits.get(circuit_hash) {
            saved_circuits.insert(*circuit_hash, Arc::clone(circuit));
        }
    }

    let (effective_sites, effective_circuits) =
        build_effective_runtime_state(sites, circuits, virtualized_sites);
    let (previous_sites, previous_circuits) =
        build_current_planner_state(&effective_sites, &effective_circuits);

    let mut future_parent_by_site: HashMap<i64, Option<i64>> = HashMap::new();
    for site_hash in future_top_level_queue.keys() {
        future_parent_by_site.insert(*site_hash, None);
    }
    for site_hash in &affected_site_hashes {
        if future_parent_by_site.contains_key(site_hash) {
            continue;
        }
        let parent_hash = site_parent_hash(*site_hash, sites, &{
            let mut class_to_site = HashMap::new();
            for (hash, site) in sites {
                if let Some(handles) = site_class_handles(site.as_ref()) {
                    class_to_site.insert(handles, *hash);
                }
            }
            class_to_site
        });
        future_parent_by_site.insert(*site_hash, parent_hash);
    }

    let mut planner_site_inputs = Vec::new();
    let mut planner_site_hashes: Vec<i64> = affected_site_hashes
        .iter()
        .copied()
        .filter(|hash| *hash != target_site_hash)
        .collect();
    planner_site_hashes
        .sort_by_key(|hash| (site_descendant_depth(*hash, &future_parent_by_site), *hash));
    for site_hash in &planner_site_hashes {
        let Some(site) = sites.get(site_hash) else {
            continue;
        };
        let queue = if let Some(root_queue) = future_top_level_queue.get(site_hash) {
            *root_queue
        } else {
            let mut cursor = *site_hash;
            loop {
                let Some(parent) = future_parent_by_site.get(&cursor).copied().flatten() else {
                    break future_top_level_queue.get(&cursor).copied().unwrap_or(1);
                };
                cursor = parent;
            }
        };
        let parent_path = future_parent_by_site
            .get(site_hash)
            .copied()
            .flatten()
            .map(planner_site_key)
            .unwrap_or_default();
        let has_children = children_by_parent
            .get(site_hash)
            .map(|children| !children.is_empty())
            .unwrap_or(false);
        planner_site_inputs.push(SiteIdentityInput {
            site_key: planner_site_key(*site_hash),
            parent_path,
            queue,
            has_children,
        });
        let _ = site;
    }

    let class_to_site = {
        let mut m = HashMap::new();
        for (hash, site) in sites {
            if let Some(handles) = site_class_handles(site.as_ref()) {
                m.insert(handles, *hash);
            }
        }
        m
    };

    let mut circuit_ids_by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (circuit_hash, circuit) in &saved_circuits {
        let BakeryCommands::AddCircuit {
            parent_class_id,
            up_parent_class_id,
            ..
        } = circuit.as_ref()
        else {
            continue;
        };
        if *parent_class_id == target_down_class && *up_parent_class_id == target_up_class {
            continue;
        }
        let parent = class_to_site
            .get(&(*parent_class_id, *up_parent_class_id))
            .map(|site_hash| planner_site_key(*site_hash))
            .unwrap_or_else(|| {
                format!(
                    "root:{}",
                    current_circuit_queue(circuit.as_ref()).unwrap_or(1)
                )
            });
        circuit_ids_by_parent
            .entry(parent)
            .or_default()
            .push(planner_circuit_key(*circuit_hash));
    }
    for circuit_hash in &direct_circuits {
        let id = planner_circuit_key(*circuit_hash);
        let assigned = planner
            .assignment
            .get(&id)
            .or_else(|| prev_assign.get(&id))
            .ok_or_else(|| format!("Planner did not assign direct circuit {}", circuit_hash))?;
        let queue = queue_from_bin_name(assigned)
            .ok_or_else(|| format!("Invalid planner queue assignment {}", assigned))?;
        circuit_ids_by_parent
            .entry(format!("root:{queue}"))
            .or_default()
            .push(id);
    }

    let mut planner_circuit_groups = Vec::new();
    for (parent_node, mut circuit_ids) in circuit_ids_by_parent {
        circuit_ids.sort();
        let queue = if let Some(root) = parent_node.strip_prefix("root:") {
            root.parse::<u32>().unwrap_or(1)
        } else if let Some(site_hash) = parent_node
            .strip_prefix("site:")
            .and_then(|s| s.parse::<i64>().ok())
        {
            planner_site_inputs
                .iter()
                .find(|site| site.site_key == planner_site_key(site_hash))
                .map(|site| site.queue)
                .unwrap_or(1)
        } else {
            1
        };
        planner_circuit_groups.push(CircuitIdentityGroupInput {
            parent_node,
            queue,
            circuit_ids,
        });
    }

    let (mut reserved_site_minors, mut reserved_circuit_minors) = build_class_identity_reservations(
        &planner_site_inputs,
        &planner_circuit_groups,
        &previous_sites,
        &previous_circuits,
    );
    if let Some(config) = config {
        let down_snapshot = read_live_class_snapshot(&config.isp_interface())?;
        let up_snapshot = read_live_class_snapshot(&config.internet_interface())?;
        reserve_live_snapshot_minors(&mut reserved_site_minors, &down_snapshot);
        reserve_live_snapshot_minors(&mut reserved_site_minors, &up_snapshot);
        reserve_live_snapshot_minors(&mut reserved_circuit_minors, &down_snapshot);
        reserve_live_snapshot_minors(&mut reserved_circuit_minors, &up_snapshot);
    }
    let constraints = ClassIdentityPlannerConstraints {
        reserved_site_minors,
        reserved_circuit_minors,
        site_minor_start: ACTIVE_RUNTIME_MINOR_START,
        circuit_minor_start: ACTIVE_RUNTIME_MINOR_START,
        stick_offset,
        circuit_padding: 0,
    };
    let identity = plan_class_identities_with_constraints(
        &planner_site_inputs,
        &planner_circuit_groups,
        &previous_sites,
        &previous_circuits,
        &constraints,
    );
    let site_identity_by_key: HashMap<String, _> = identity
        .sites
        .iter()
        .map(|entry| (entry.site_key.clone(), entry.clone()))
        .collect();
    let circuit_identity_by_key: HashMap<String, _> = identity
        .circuits
        .iter()
        .map(|entry| (entry.circuit_id.clone(), entry.clone()))
        .collect();

    let mut active_sites = HashMap::new();
    for site_hash in &planner_site_hashes {
        let Some(old_site) = sites.get(site_hash) else {
            continue;
        };
        let site_key = planner_site_key(*site_hash);
        let Some(identity_entry) = site_identity_by_key.get(&site_key) else {
            continue;
        };
        let parent_handles = future_parent_by_site
            .get(site_hash)
            .copied()
            .flatten()
            .and_then(|parent_hash| {
                site_identity_by_key
                    .get(&planner_site_key(parent_hash))
                    .map(|parent_identity| {
                        (
                            tc_handle_from_major_minor(
                                parent_identity.class_major,
                                parent_identity.class_minor,
                            ),
                            tc_handle_from_major_minor(
                                parent_identity.up_class_major,
                                parent_identity.class_minor,
                            ),
                        )
                    })
            })
            .unwrap_or_else(|| {
                (
                    root_handle_for_queue(identity_entry.queue),
                    root_handle_for_queue(u32::from(identity_entry.up_class_major)),
                )
            });
        let BakeryCommands::AddSite {
            site_hash,
            download_bandwidth_min,
            upload_bandwidth_min,
            download_bandwidth_max,
            upload_bandwidth_max,
            ..
        } = old_site.as_ref()
        else {
            continue;
        };
        active_sites.insert(
            *site_hash,
            PlannedSiteUpdate {
                queue: identity_entry.queue,
                parent_site: future_parent_by_site.get(site_hash).copied().flatten(),
                stage_depth: site_descendant_depth(*site_hash, &future_parent_by_site),
                command: Arc::new(BakeryCommands::AddSite {
                    site_hash: *site_hash,
                    parent_class_id: parent_handles.0,
                    up_parent_class_id: parent_handles.1,
                    class_minor: identity_entry.class_minor,
                    download_bandwidth_min: *download_bandwidth_min,
                    upload_bandwidth_min: *upload_bandwidth_min,
                    download_bandwidth_max: *download_bandwidth_max,
                    upload_bandwidth_max: *upload_bandwidth_max,
                }),
            },
        );
    }

    active_sites.retain(|hash, _| saved_sites.contains_key(hash));

    let mut active_circuits = HashMap::new();
    for (circuit_hash, circuit) in &saved_circuits {
        let circuit_key = planner_circuit_key(*circuit_hash);
        let Some(identity_entry) = circuit_identity_by_key.get(&circuit_key) else {
            continue;
        };
        let parent_site = previous_circuits
            .get(&circuit_key)
            .and_then(|prev| prev.parent_node.strip_prefix("site:"))
            .and_then(|s| s.parse::<i64>().ok());
        let future_parent_site = if direct_circuits.contains(circuit_hash) {
            None
        } else {
            class_to_site
                .get(&{
                    let BakeryCommands::AddCircuit {
                        parent_class_id,
                        up_parent_class_id,
                        ..
                    } = circuit.as_ref()
                    else {
                        continue;
                    };
                    (*parent_class_id, *up_parent_class_id)
                })
                .copied()
        };
        let parent_handles = if let Some(parent_site_hash) = future_parent_site {
            if let Some(parent_identity) =
                site_identity_by_key.get(&planner_site_key(parent_site_hash))
            {
                (
                    tc_handle_from_major_minor(
                        parent_identity.class_major,
                        parent_identity.class_minor,
                    ),
                    tc_handle_from_major_minor(
                        parent_identity.up_class_major,
                        parent_identity.class_minor,
                    ),
                )
            } else {
                (
                    root_handle_for_queue(identity_entry.queue),
                    root_handle_for_queue(u32::from(identity_entry.up_class_major)),
                )
            }
        } else {
            (
                root_handle_for_queue(identity_entry.queue),
                root_handle_for_queue(u32::from(identity_entry.up_class_major)),
            )
        };
        let BakeryCommands::AddCircuit {
            circuit_hash,
            circuit_name,
            site_name,
            download_bandwidth_min,
            upload_bandwidth_min,
            download_bandwidth_max,
            upload_bandwidth_max,
            ip_addresses,
            sqm_override,
            ..
        } = circuit.as_ref()
        else {
            continue;
        };
        active_circuits.insert(
            *circuit_hash,
            PlannedCircuitUpdate {
                queue: identity_entry.queue,
                parent_site: future_parent_site.or(parent_site),
                command: Arc::new(BakeryCommands::AddCircuit {
                    circuit_hash: *circuit_hash,
                    circuit_name: circuit_name.clone(),
                    site_name: site_name.clone(),
                    parent_class_id: parent_handles.0,
                    up_parent_class_id: parent_handles.1,
                    class_minor: identity_entry.class_minor,
                    download_bandwidth_min: *download_bandwidth_min,
                    upload_bandwidth_min: *upload_bandwidth_min,
                    download_bandwidth_max: *download_bandwidth_max,
                    upload_bandwidth_max: *upload_bandwidth_max,
                    class_major: identity_entry.class_major,
                    up_class_major: identity_entry.up_class_major,
                    down_qdisc_handle: None,
                    up_qdisc_handle: None,
                    ip_addresses: ip_addresses.clone(),
                    sqm_override: sqm_override.clone(),
                }),
            },
        );
    }
    let mut site_stages_map: BTreeMap<usize, Vec<i64>> = BTreeMap::new();
    for site_hash in planner_site_hashes
        .into_iter()
        .filter(|hash| active_sites.contains_key(hash))
    {
        let update = active_sites
            .get(&site_hash)
            .expect("active top-level site update");
        site_stages_map
            .entry(update.stage_depth)
            .or_default()
            .push(site_hash);
    }
    let site_stages = site_stages_map.into_values().collect();

    Ok(TopLevelVirtualizationPlan {
        saved_sites,
        saved_circuits,
        active_sites,
        active_circuits,
        site_stages,
    })
}

fn build_non_top_level_virtualization_plan(
    target_site: Arc<BakeryCommands>,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
) -> Result<TopLevelVirtualizationPlan, String> {
    let site_hash = site_hash_from_command(target_site.as_ref())
        .ok_or_else(|| "TreeGuard runtime virtualization requires an AddSite target".to_string())?;
    let BakeryCommands::AddSite {
        parent_class_id,
        up_parent_class_id,
        ..
    } = target_site.as_ref()
    else {
        return Err("TreeGuard runtime virtualization requires an AddSite target".to_string());
    };

    let children_by_parent = direct_child_sites_by_parent(sites);
    let subtree_sites = collect_site_subtree_hashes(site_hash, &children_by_parent);
    let descendant_sites: Vec<i64> = subtree_sites
        .iter()
        .copied()
        .filter(|hash| *hash != site_hash)
        .collect();
    let subtree_circuits = collect_circuits_attached_to_sites(circuits, sites, &subtree_sites);

    let saved_sites: HashMap<i64, Arc<BakeryCommands>> = descendant_sites
        .iter()
        .filter_map(|hash| sites.get(hash).cloned().map(|command| (*hash, command)))
        .collect();
    let saved_circuits: HashMap<i64, Arc<BakeryCommands>> = subtree_circuits
        .iter()
        .filter_map(|hash| circuits.get(hash).cloned().map(|command| (*hash, command)))
        .collect();

    let class_to_site = {
        let mut m = HashMap::new();
        for (hash, site) in sites {
            if let Some(handles) = site_class_handles(site.as_ref()) {
                m.insert(handles, *hash);
            }
        }
        m
    };

    let mut site_parents = HashMap::new();
    for desc_hash in &descendant_sites {
        site_parents.insert(
            *desc_hash,
            site_parent_hash(*desc_hash, sites, &class_to_site),
        );
    }

    let mut ordered_sites = descendant_sites;
    ordered_sites.sort_by_key(|hash| (site_descendant_depth(*hash, &site_parents), *hash));

    let mut active_sites = HashMap::new();
    let mut stage_depth_by_site: HashMap<i64, usize> = HashMap::new();
    let mut shadow_handles_by_site: HashMap<i64, (TcHandle, TcHandle)> = HashMap::new();
    for child_hash in &ordered_sites {
        let Some(child_site) = sites.get(child_hash).cloned() else {
            continue;
        };
        let parent_hash = site_parents.get(child_hash).copied().flatten();
        let (new_parent_down, new_parent_up, planner_parent_site, stage_depth) = match parent_hash {
            Some(parent) if parent == site_hash => {
                (*parent_class_id, *up_parent_class_id, None, 0usize)
            }
            Some(parent) => {
                let Some(handles) = shadow_handles_by_site.get(&parent).copied() else {
                    return Err(format!(
                        "Missing shadow parent handles while planning runtime virtualization for child site {}",
                        child_hash
                    ));
                };
                let parent_depth = stage_depth_by_site.get(&parent).copied().ok_or_else(|| {
                    format!(
                        "Missing shadow stage depth while planning runtime virtualization for child site {}",
                        child_hash
                    )
                })?;
                (handles.0, handles.1, Some(parent), parent_depth + 1)
            }
            None => (*parent_class_id, *up_parent_class_id, None, 0usize),
        };
        let Some(shadow_minor) = find_free_site_shadow_minor(
            sites,
            circuits,
            &HashMap::new(),
            &active_sites,
            &HashMap::new(),
            &new_parent_down,
            &new_parent_up,
        ) else {
            return Err(format!(
                "Unable to allocate runtime shadow site class for child site {}",
                child_hash
            ));
        };
        let Some(shadow_site) =
            rebuild_site_command(&child_site, new_parent_down, new_parent_up, shadow_minor)
        else {
            continue;
        };
        let Some(shadow_handles) = site_class_handles(shadow_site.as_ref()) else {
            return Err(format!(
                "Failed to derive shadow handles for runtime child site {}",
                child_hash
            ));
        };
        stage_depth_by_site.insert(*child_hash, stage_depth);
        shadow_handles_by_site.insert(*child_hash, shadow_handles);
        active_sites.insert(
            *child_hash,
            PlannedSiteUpdate {
                queue: current_site_queue(shadow_site.as_ref()).unwrap_or(1),
                parent_site: planner_parent_site,
                stage_depth,
                command: shadow_site,
            },
        );
    }

    let mut site_stages_map: BTreeMap<usize, Vec<i64>> = BTreeMap::new();
    for (site_hash, update) in &active_sites {
        site_stages_map
            .entry(update.stage_depth)
            .or_default()
            .push(*site_hash);
    }
    let mut site_stages: Vec<Vec<i64>> = site_stages_map
        .into_values()
        .map(|mut hashes| {
            hashes.sort_by_key(|hash| {
                let update = active_sites.get(hash).expect("planned site update");
                (update.queue, update.parent_site.unwrap_or_default(), *hash)
            });
            hashes
        })
        .collect();
    if site_stages.is_empty() && !active_sites.is_empty() {
        site_stages.push(active_sites.keys().copied().collect());
    }

    let mut active_circuits = HashMap::new();
    for circuit_hash in &subtree_circuits {
        let Some(old_circuit) = circuits.get(circuit_hash).cloned() else {
            continue;
        };
        let BakeryCommands::AddCircuit {
            parent_class_id: old_parent_down,
            up_parent_class_id: old_parent_up,
            ..
        } = old_circuit.as_ref()
        else {
            continue;
        };
        let Some(current_parent_site_hash) = class_to_site
            .get(&(*old_parent_down, *old_parent_up))
            .copied()
        else {
            return Err(format!(
                "Unable to identify current parent site for circuit {} during runtime virtualization",
                circuit_hash
            ));
        };
        let (new_parent_down, new_parent_up, planner_parent_site) = if current_parent_site_hash
            == site_hash
        {
            (*parent_class_id, *up_parent_class_id, None)
        } else {
            let Some(handles) = shadow_handles_by_site
                .get(&current_parent_site_hash)
                .copied()
            else {
                return Err(format!(
                    "Missing shadow parent handles for circuit {} during runtime virtualization",
                    circuit_hash
                ));
            };
            (handles.0, handles.1, Some(current_parent_site_hash))
        };
        let Some(updated_circuit) =
            reparent_circuit_command(&old_circuit, new_parent_down, new_parent_up)
        else {
            continue;
        };
        active_circuits.insert(
            *circuit_hash,
            PlannedCircuitUpdate {
                queue: current_circuit_queue(updated_circuit.as_ref()).unwrap_or(1),
                parent_site: planner_parent_site,
                command: updated_circuit,
            },
        );
    }

    Ok(TopLevelVirtualizationPlan {
        saved_sites,
        saved_circuits,
        active_sites,
        active_circuits,
        site_stages,
    })
}

fn site_has_observed_child_classes(
    site: &BakeryCommands,
    down_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    up_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
) -> bool {
    let Some((down_class, up_class)) = site_class_handles(site) else {
        return false;
    };

    down_snapshot
        .values()
        .any(|entry| entry.parent == Some(down_class))
        || up_snapshot
            .values()
            .any(|entry| entry.parent == Some(up_class))
}

fn verify_site_updates_live(
    config: &Arc<Config>,
    updates: &HashMap<i64, PlannedSiteUpdate>,
) -> Result<(), String> {
    if updates.is_empty() {
        return Ok(());
    }

    let down_snapshot = read_live_class_snapshot(&config.isp_interface())?;
    let up_snapshot = read_live_class_snapshot(&config.internet_interface())?;

    for (site_hash, update) in updates {
        let BakeryCommands::AddSite {
            parent_class_id,
            up_parent_class_id,
            class_minor,
            ..
        } = update.command.as_ref()
        else {
            continue;
        };
        let Some((down_class, up_class)) = site_class_handles(update.command.as_ref()) else {
            continue;
        };
        let Some(down_entry) = down_snapshot.get(&down_class) else {
            return Err(format!(
                "Runtime child site {} was not observed on {} after shadow create",
                site_hash,
                config.isp_interface()
            ));
        };
        let Some(up_entry) = up_snapshot.get(&up_class) else {
            return Err(format!(
                "Runtime child site {} was not observed on {} after shadow create",
                site_hash,
                config.internet_interface()
            ));
        };
        if !live_parent_matches(down_entry.parent, *parent_class_id)
            || !live_parent_matches(up_entry.parent, *up_parent_class_id)
        {
            return Err(format!(
                "Runtime child site {} shadow verify failed: expected parents {}/{} for class minor 0x{:x}, observed parents {:?}/{:?}",
                site_hash,
                parent_class_id.as_tc_string(),
                up_parent_class_id.as_tc_string(),
                class_minor,
                down_entry.parent,
                up_entry.parent
            ));
        }
    }

    Ok(())
}

fn ordered_prune_site_hashes(prune_sites: &HashMap<i64, Arc<BakeryCommands>>) -> Vec<i64> {
    let mut class_to_site = HashMap::new();
    for (site_hash, site) in prune_sites {
        if let Some(handles) = site_class_handles(site.as_ref()) {
            class_to_site.insert(handles, *site_hash);
        }
    }

    let mut parents = HashMap::new();
    for site_hash in prune_sites.keys().copied() {
        parents.insert(
            site_hash,
            site_parent_hash(site_hash, prune_sites, &class_to_site),
        );
    }

    let mut ordered: Vec<i64> = prune_sites.keys().copied().collect();
    ordered.sort_by_key(|site_hash| {
        let depth = site_descendant_depth(*site_hash, &parents);
        (std::cmp::Reverse(depth), *site_hash)
    });
    ordered
}

fn ordered_prune_circuit_hashes(prune_circuits: &HashMap<i64, Arc<BakeryCommands>>) -> Vec<i64> {
    let mut ordered: Vec<i64> = prune_circuits.keys().copied().collect();
    ordered.sort_unstable();
    ordered
}

fn apply_site_command_updates(
    config: &Arc<Config>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    updates: &HashMap<i64, PlannedSiteUpdate>,
    action_label: &str,
) -> Result<(), String> {
    let mut single_stage: Vec<i64> = updates.keys().copied().collect();
    single_stage.sort_unstable();
    apply_site_command_update_stages(config, sites, updates, &[single_stage], action_label, false)
}

fn apply_site_command_update_stages(
    config: &Arc<Config>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    updates: &HashMap<i64, PlannedSiteUpdate>,
    site_stages: &[Vec<i64>],
    action_label: &str,
    verify_each_stage: bool,
) -> Result<(), String> {
    if updates.is_empty() {
        return Ok(());
    }

    let mut remaining: BTreeSet<i64> = updates.keys().copied().collect();
    let mut effective_stages: Vec<Vec<i64>> = site_stages
        .iter()
        .filter_map(|stage| {
            let filtered: Vec<i64> = stage
                .iter()
                .copied()
                .filter(|site_hash| updates.contains_key(site_hash))
                .collect();
            (!filtered.is_empty()).then_some(filtered)
        })
        .collect();
    if effective_stages.is_empty() {
        effective_stages.push(remaining.iter().copied().collect());
    }

    for (stage_index, stage_hashes) in effective_stages.iter().enumerate() {
        let mut ordered: Vec<_> = stage_hashes
            .iter()
            .filter_map(|site_hash| updates.get(site_hash).map(|update| (site_hash, update)))
            .collect();
        ordered.sort_by_key(|(site_hash, update)| {
            (
                update.parent_site.is_some(),
                update.queue,
                update.parent_site.unwrap_or_default(),
                **site_hash,
            )
        });

        let mut commands = Vec::new();
        for (_, update) in &ordered {
            if let Some(cmds) = update.command.to_commands(config, ExecutionMode::Builder) {
                commands.extend(cmds);
            }
        }

        let stage_label = if verify_each_stage {
            format!("{action_label} [stage {}]", stage_index + 1)
        } else {
            action_label.to_string()
        };

        if !commands.is_empty() {
            let result = execute_and_record_live_change(&commands, &stage_label);
            if !result.ok {
                return Err(summarize_apply_result(&stage_label, &result));
            }
        }

        let mut stage_updates = HashMap::new();
        for (site_hash, update) in ordered {
            remaining.remove(site_hash);
            if verify_each_stage {
                stage_updates.insert(*site_hash, update.clone());
            }
            sites.insert(*site_hash, Arc::clone(&update.command));
        }

        if verify_each_stage && !stage_updates.is_empty() {
            verify_site_updates_live(config, &stage_updates)?;
        }
    }

    if !remaining.is_empty() {
        let fallback_stage: Vec<i64> = remaining.into_iter().collect();
        apply_site_command_update_stages(
            config,
            sites,
            updates,
            &[fallback_stage],
            action_label,
            verify_each_stage,
        )?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_circuit_command_updates(
    config: &Arc<Config>,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    updates: &HashMap<i64, PlannedCircuitUpdate>,
    live_circuits: &HashMap<i64, u64>,
    mq_layout: &Option<MqDeviceLayout>,
    qdisc_handles: &mut QdiscHandleState,
    migrations: &mut HashMap<i64, Migration>,
    action_label: &str,
) -> Result<(), String> {
    if updates.is_empty() {
        return Ok(());
    }
    let Some(layout) = mq_layout.as_ref() else {
        return Err("Bakery runtime virtualization requires MQ layout to be available".to_string());
    };
    let live_reserved_handles = snapshot_live_qdisc_handle_majors_or_empty(config, action_label);
    let mut immediate_commands = Vec::new();
    let mut updated_circuits = Vec::new();
    let mut ordered: Vec<_> = updates.iter().collect();
    ordered.sort_by_key(|(_, update)| {
        (
            update.parent_site.is_none(),
            update.queue,
            update.parent_site.unwrap_or_default(),
            circuit_hash_from_command(update.command.as_ref()).unwrap_or_default(),
        )
    });

    for (circuit_hash, update) in ordered {
        let Some(old_cmd) = circuits.get(circuit_hash).cloned() else {
            continue;
        };
        let enriched_base = with_assigned_qdisc_handles_reserved(
            &update.command,
            config,
            layout,
            qdisc_handles,
            &live_reserved_handles,
        );
        let enriched_cmd = rotate_changed_qdisc_handles_reserved(
            old_cmd.as_ref(),
            &enriched_base,
            config,
            layout,
            qdisc_handles,
            &live_reserved_handles,
        );
        if let Some(summary) = qdisc_handle_rotation_invariant_error_with_live_reservations(
            old_cmd.as_ref(),
            enriched_cmd.as_ref(),
            config,
            Some(&live_reserved_handles),
        ) {
            return Err(format!(
                "Bakery runtime virtualization refusing live migration for active circuit {}: {}",
                circuit_hash, summary
            ));
        }
        if live_circuits.contains_key(circuit_hash)
            && let BakeryCommands::AddCircuit {
                class_major,
                up_class_major,
                ..
            } = enriched_cmd.as_ref()
            && find_free_circuit_shadow_minor(
                sites,
                circuits,
                migrations,
                *class_major,
                *up_class_major,
            )
            .is_none()
        {
            return Err(format!(
                "Unable to queue live migration for active circuit {}: no shadow minor available",
                circuit_hash
            ));
        }
        if queue_live_migration(
            old_cmd.as_ref(),
            &enriched_cmd,
            sites,
            circuits,
            live_circuits,
            migrations,
        ) {
            continue;
        }
        if live_circuits.contains_key(circuit_hash) {
            return Err(format!(
                "Bakery runtime virtualization could not live-migrate active circuit {}",
                circuit_hash
            ));
        }
        match config.queues.lazy_queues.as_ref() {
            None | Some(LazyQueueMode::No) | Some(LazyQueueMode::Htb) => {
                if let Some(prune) = old_cmd.to_prune(config, true) {
                    immediate_commands.extend(prune);
                }
                if let Some(add) = enriched_cmd.to_commands(config, ExecutionMode::Builder) {
                    immediate_commands.extend(add);
                }
            }
            Some(LazyQueueMode::Full) => {}
        }
        updated_circuits.push((*circuit_hash, enriched_cmd));
    }

    if !immediate_commands.is_empty() {
        let result = execute_and_record_live_change(&immediate_commands, action_label);
        if !result.ok {
            return Err(summarize_apply_result(action_label, &result));
        }
    }
    for (circuit_hash, command) in updated_circuits {
        circuits.insert(circuit_hash, command);
    }
    Ok(())
}

fn circuit_qdisc_parent_changed(
    old_cmd: &BakeryCommands,
    new_cmd: &BakeryCommands,
    config: &Arc<Config>,
) -> bool {
    let (old_down_parent, old_up_parent) = effective_directional_qdisc_parents(old_cmd, config);
    let (new_down_parent, new_up_parent) = effective_directional_qdisc_parents(new_cmd, config);
    old_down_parent != new_down_parent || old_up_parent != new_up_parent
}

fn qdisc_handle_rotation_invariant_error_with_live_reservations(
    old_cmd: &BakeryCommands,
    new_cmd: &BakeryCommands,
    config: &Arc<Config>,
    live_reserved_handles: Option<&HashMap<String, HashSet<u16>>>,
) -> Option<String> {
    let (old_down_parent, old_up_parent) = effective_directional_qdisc_parents(old_cmd, config);
    let (new_down_parent, new_up_parent) = effective_directional_qdisc_parents(new_cmd, config);
    let BakeryCommands::AddCircuit {
        down_qdisc_handle: old_down_qdisc_handle,
        up_qdisc_handle: old_up_qdisc_handle,
        ..
    } = old_cmd
    else {
        return None;
    };
    let BakeryCommands::AddCircuit {
        down_qdisc_handle: new_down_qdisc_handle,
        up_qdisc_handle: new_up_qdisc_handle,
        ..
    } = new_cmd
    else {
        return None;
    };

    if old_down_parent != new_down_parent
        && old_down_qdisc_handle.is_some()
        && old_down_qdisc_handle == new_down_qdisc_handle
    {
        return Some(format!(
            "downlink qdisc handle {:?} was preserved even though the qdisc parent changed from {:?} to {:?}",
            old_down_qdisc_handle, old_down_parent, new_down_parent
        ));
    }

    if old_up_parent != new_up_parent
        && old_up_qdisc_handle.is_some()
        && old_up_qdisc_handle == new_up_qdisc_handle
    {
        return Some(format!(
            "uplink qdisc handle {:?} was preserved even though the qdisc parent changed from {:?} to {:?}",
            old_up_qdisc_handle, old_up_parent, new_up_parent
        ));
    }

    if let Some(live_reserved) = live_reserved_handles {
        let isp_interface = config.isp_interface();
        if let Some(new_down_handle) = new_down_qdisc_handle
            && live_reserved
                .get(&isp_interface)
                .is_some_and(|handles| handles.contains(new_down_handle))
            && Some(*new_down_handle) != *old_down_qdisc_handle
        {
            return Some(format!(
                "downlink qdisc handle {:?} is already live on {} and cannot be reused for a different circuit/parent",
                new_down_handle, isp_interface
            ));
        }

        let up_interface = config.internet_interface();
        if let Some(new_up_handle) = new_up_qdisc_handle
            && live_reserved
                .get(&up_interface)
                .is_some_and(|handles| handles.contains(new_up_handle))
            && Some(*new_up_handle) != *old_up_qdisc_handle
        {
            return Some(format!(
                "uplink qdisc handle {:?} is already live on {} and cannot be reused for a different circuit/parent",
                new_up_handle, up_interface
            ));
        }
    }

    None
}

fn migration_qdisc_handle_rotation_invariant_error(mig: &Migration) -> Option<String> {
    let old_down_parent = tc_handle_from_major_minor(mig.old_class_major, mig.old_minor);
    let new_down_parent = tc_handle_from_major_minor(mig.class_major, mig.final_minor);
    if old_down_parent != new_down_parent
        && mig.old_down_qdisc_handle.is_some()
        && mig.old_down_qdisc_handle == mig.down_qdisc_handle
    {
        return Some(format!(
            "downlink qdisc handle {:?} is still the old handle while the final parent changed from {} to {}",
            mig.down_qdisc_handle,
            old_down_parent.as_tc_string(),
            new_down_parent.as_tc_string()
        ));
    }

    let old_up_parent = tc_handle_from_major_minor(mig.old_up_class_major, mig.old_minor);
    let new_up_parent = tc_handle_from_major_minor(mig.up_class_major, mig.final_minor);
    if old_up_parent != new_up_parent
        && mig.old_up_qdisc_handle.is_some()
        && mig.old_up_qdisc_handle == mig.up_qdisc_handle
    {
        return Some(format!(
            "uplink qdisc handle {:?} is still the old handle while the final parent changed from {} to {}",
            mig.up_qdisc_handle,
            old_up_parent.as_tc_string(),
            new_up_parent.as_tc_string()
        ));
    }

    None
}

#[cfg_attr(not(test), allow(dead_code))]
fn build_final_qdisc_handle_rotation_invariant_error(
    mig: &Migration,
    final_cmd: &BakeryCommands,
) -> Option<String> {
    build_final_qdisc_handle_rotation_invariant_error_with_live_reservations(
        mig, final_cmd, None, None,
    )
}

fn build_final_qdisc_handle_rotation_invariant_error_with_live_reservations(
    mig: &Migration,
    final_cmd: &BakeryCommands,
    live_down_reserved_handles: Option<&HashSet<u16>>,
    live_up_reserved_handles: Option<&HashSet<u16>>,
) -> Option<String> {
    let BakeryCommands::AddCircuit {
        class_minor,
        down_qdisc_handle,
        up_qdisc_handle,
        ..
    } = final_cmd
    else {
        return Some("build-final command was not an AddCircuit".to_string());
    };

    if *class_minor != mig.final_minor {
        return Some(format!(
            "build-final command used unexpected class minor {} instead of {}",
            class_minor, mig.final_minor
        ));
    }

    if *down_qdisc_handle != mig.down_qdisc_handle {
        return Some(format!(
            "build-final command used downlink qdisc handle {:?}, expected rotated handle {:?}",
            down_qdisc_handle, mig.down_qdisc_handle
        ));
    }

    if *up_qdisc_handle != mig.up_qdisc_handle {
        return Some(format!(
            "build-final command used uplink qdisc handle {:?}, expected rotated handle {:?}",
            up_qdisc_handle, mig.up_qdisc_handle
        ));
    }

    if let Some(summary) = migration_qdisc_handle_rotation_invariant_error(mig) {
        return Some(summary);
    }

    if let Some(live_down_reserved) = live_down_reserved_handles
        && let Some(down_handle) = *down_qdisc_handle
        && live_down_reserved.contains(&down_handle)
        && Some(down_handle) != mig.old_down_qdisc_handle
    {
        return Some(format!(
            "build-final command used downlink qdisc handle {:?}, but that handle is already live on the interface under another parent",
            down_handle
        ));
    }

    if let Some(live_up_reserved) = live_up_reserved_handles
        && let Some(up_handle) = *up_qdisc_handle
        && live_up_reserved.contains(&up_handle)
        && Some(up_handle) != mig.old_up_qdisc_handle
    {
        return Some(format!(
            "build-final command used uplink qdisc handle {:?}, but that handle is already live on the interface under another parent",
            up_handle
        ));
    }

    None
}

#[allow(clippy::too_many_arguments)]
fn apply_top_level_circuit_command_updates(
    config: &Arc<Config>,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    updates: &HashMap<i64, PlannedCircuitUpdate>,
    live_circuits: &HashMap<i64, u64>,
    mq_layout: &Option<MqDeviceLayout>,
    qdisc_handles: &mut QdiscHandleState,
    migrations: &mut HashMap<i64, Migration>,
    action_label: &str,
) -> Result<(), String> {
    if updates.is_empty() {
        return Ok(());
    }
    let Some(layout) = mq_layout.as_ref() else {
        return Err("Bakery runtime virtualization requires MQ layout to be available".to_string());
    };
    let live_reserved_handles =
        snapshot_live_qdisc_handle_majors_or_empty(config, "top-level runtime virtualization");

    let mut ordered: Vec<_> = updates.iter().collect();
    ordered.sort_by_key(|(_, update)| {
        (
            update.parent_site.is_none(),
            update.queue,
            update.parent_site.unwrap_or_default(),
            circuit_hash_from_command(update.command.as_ref()).unwrap_or_default(),
        )
    });

    for (circuit_hash, update) in &ordered {
        let Some(old_cmd) = circuits.get(circuit_hash).cloned() else {
            continue;
        };
        let enriched_base = with_assigned_qdisc_handles_reserved(
            &update.command,
            config,
            layout,
            qdisc_handles,
            &live_reserved_handles,
        );
        let enriched_cmd = rotate_changed_qdisc_handles_reserved(
            old_cmd.as_ref(),
            &enriched_base,
            config,
            layout,
            qdisc_handles,
            &live_reserved_handles,
        );
        if circuit_qdisc_parent_changed(old_cmd.as_ref(), enriched_cmd.as_ref(), config)
            && let BakeryCommands::AddCircuit {
                class_major,
                up_class_major,
                ..
            } = enriched_cmd.as_ref()
            && find_free_circuit_shadow_minor(
                sites,
                circuits,
                migrations,
                *class_major,
                *up_class_major,
            )
            .is_none()
        {
            return Err(format!(
                "Unable to queue top-level runtime migration for circuit {}: no shadow minor available",
                circuit_hash
            ));
        }
    }

    let mut immediate_commands = Vec::new();
    let mut updated_circuits = Vec::new();

    for (circuit_hash, update) in ordered {
        let Some(old_cmd) = circuits.get(circuit_hash).cloned() else {
            continue;
        };
        let enriched_base = with_assigned_qdisc_handles_reserved(
            &update.command,
            config,
            layout,
            qdisc_handles,
            &live_reserved_handles,
        );
        let enriched_cmd = rotate_changed_qdisc_handles_reserved(
            old_cmd.as_ref(),
            &enriched_base,
            config,
            layout,
            qdisc_handles,
            &live_reserved_handles,
        );
        if let Some(summary) = qdisc_handle_rotation_invariant_error_with_live_reservations(
            old_cmd.as_ref(),
            enriched_cmd.as_ref(),
            config,
            Some(&live_reserved_handles),
        ) {
            return Err(format!(
                "Bakery runtime top-level virtualization refusing live migration for active circuit {}: {}",
                circuit_hash, summary
            ));
        }

        if circuit_qdisc_parent_changed(old_cmd.as_ref(), enriched_cmd.as_ref(), config) {
            if !queue_top_level_runtime_migration(
                old_cmd.as_ref(),
                &enriched_cmd,
                sites,
                circuits,
                live_circuits,
                migrations,
            ) {
                return Err(format!(
                    "Bakery runtime top-level virtualization could not queue migration for circuit {}",
                    circuit_hash
                ));
            }
            continue;
        }

        if queue_live_migration(
            old_cmd.as_ref(),
            &enriched_cmd,
            sites,
            circuits,
            live_circuits,
            migrations,
        ) {
            continue;
        }

        if live_circuits.contains_key(circuit_hash) {
            return Err(format!(
                "Bakery runtime top-level virtualization could not live-migrate active circuit {}",
                circuit_hash
            ));
        }

        match config.queues.lazy_queues.as_ref() {
            None | Some(LazyQueueMode::No) | Some(LazyQueueMode::Htb) => {
                if let Some(prune) = old_cmd.to_prune(config, true) {
                    immediate_commands.extend(prune);
                }
                if let Some(add) = enriched_cmd.to_commands(config, ExecutionMode::Builder) {
                    immediate_commands.extend(add);
                }
            }
            Some(LazyQueueMode::Full) => {}
        }
        updated_circuits.push((*circuit_hash, enriched_cmd));
    }

    if !immediate_commands.is_empty() {
        let result = execute_and_record_live_change(&immediate_commands, action_label);
        if !result.ok {
            return Err(summarize_apply_result(action_label, &result));
        }
    }

    for (circuit_hash, command) in updated_circuits {
        circuits.insert(circuit_hash, command);
    }

    Ok(())
}

fn apply_runtime_virtualization_overlay(
    batch: Vec<Arc<BakeryCommands>>,
    virtualized_sites: &HashMap<i64, VirtualizedSiteState>,
) -> Vec<Arc<BakeryCommands>> {
    if virtualized_sites.is_empty() {
        return batch;
    }

    let mut hidden_site_hashes = HashSet::new();
    let mut active_site_overrides: HashMap<i64, Arc<BakeryCommands>> = HashMap::new();
    let mut active_circuit_overrides: HashMap<i64, Arc<BakeryCommands>> = HashMap::new();
    for (site_hash, state) in virtualized_sites {
        if state.active_branch_hides_original_site() {
            hidden_site_hashes.insert(*site_hash);
        }
        for (hash, command) in &state.active_sites {
            active_site_overrides.insert(*hash, Arc::clone(command));
        }
        for (hash, command) in &state.active_circuits {
            active_circuit_overrides.insert(*hash, Arc::clone(command));
        }
    }

    batch
        .into_iter()
        .filter_map(|command| match command.as_ref() {
            BakeryCommands::AddSite { site_hash, .. } => {
                if hidden_site_hashes.contains(site_hash) {
                    return None;
                }
                if let Some(override_cmd) = active_site_overrides.get(site_hash) {
                    return Some(Arc::clone(override_cmd));
                }
                Some(command)
            }
            BakeryCommands::AddCircuit { circuit_hash, .. } => {
                if let Some(override_cmd) = active_circuit_overrides.get(circuit_hash) {
                    return Some(Arc::clone(override_cmd));
                }
                Some(command)
            }
            _ => Some(command),
        })
        .collect()
}

fn reconstruct_structural_baseline_state(
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    virtualized_sites: &HashMap<i64, VirtualizedSiteState>,
) -> (
    HashMap<i64, Arc<BakeryCommands>>,
    HashMap<i64, Arc<BakeryCommands>>,
) {
    if virtualized_sites.is_empty() {
        return (sites.clone(), circuits.clone());
    }

    let mut baseline_sites = sites.clone();
    let mut baseline_circuits = circuits.clone();

    for (site_hash, state) in virtualized_sites {
        baseline_sites.insert(*site_hash, Arc::clone(&state.site));
        for (saved_hash, saved_site) in &state.saved_sites {
            baseline_sites.insert(*saved_hash, Arc::clone(saved_site));
        }
        for (saved_hash, saved_circuit) in &state.saved_circuits {
            baseline_circuits.insert(*saved_hash, Arc::clone(saved_circuit));
        }
    }

    (baseline_sites, baseline_circuits)
}

fn site_command_structure_summary(command: &BakeryCommands) -> Option<String> {
    let BakeryCommands::AddSite {
        parent_class_id,
        up_parent_class_id,
        class_minor,
        ..
    } = command
    else {
        return None;
    };

    Some(format!(
        "parent={} up_parent={} minor=0x{:x}",
        parent_class_id.as_tc_string(),
        up_parent_class_id.as_tc_string(),
        class_minor
    ))
}

fn find_site_command_in_batch(
    batch: &[Arc<BakeryCommands>],
    site_hash: i64,
) -> Option<&Arc<BakeryCommands>> {
    batch.iter().find(|command| {
        matches!(
            command.as_ref(),
            BakeryCommands::AddSite {
                site_hash: command_site_hash,
                ..
            } if *command_site_hash == site_hash
        )
    })
}

fn format_virtualized_site_source(
    owner_site_hash: i64,
    owner_state: &VirtualizedSiteState,
    origin: &str,
) -> String {
    format!(
        "{origin}(owner_site_hash={owner_site_hash}, owner_site_name={}, lifecycle={:?}, active_branch={:?}, pending_prune={})",
        owner_state.site_name,
        owner_state.lifecycle,
        owner_state.active_branch,
        owner_state.pending_prune
    )
}

fn log_structural_site_diff_baseline_origin(
    details: StructuralSiteDiffDetails,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    baseline_sites: &HashMap<i64, Arc<BakeryCommands>>,
    raw_batch: &[Arc<BakeryCommands>],
    virtualized_sites: &HashMap<i64, VirtualizedSiteState>,
) {
    let Some(baseline_command) = baseline_sites.get(&details.site_hash) else {
        warn!(
            "Bakery structural site diff baseline origin: site_hash={} baseline_command_missing",
            details.site_hash
        );
        return;
    };

    let baseline_summary = site_command_structure_summary(baseline_command.as_ref())
        .unwrap_or_else(|| "non-site-command".to_string());
    let current_summary = sites
        .get(&details.site_hash)
        .and_then(|command| site_command_structure_summary(command.as_ref()))
        .unwrap_or_else(|| "missing".to_string());
    let new_summary = find_site_command_in_batch(raw_batch, details.site_hash)
        .and_then(|command| site_command_structure_summary(command.as_ref()))
        .unwrap_or_else(|| "missing".to_string());

    let mut baseline_sources = Vec::new();
    if let Some(current_command) = sites.get(&details.site_hash)
        && Arc::ptr_eq(current_command, baseline_command)
    {
        baseline_sources.push("current_sites".to_string());
    }

    for (owner_site_hash, owner_state) in virtualized_sites {
        if Arc::ptr_eq(&owner_state.site, baseline_command) {
            baseline_sources.push(format_virtualized_site_source(
                *owner_site_hash,
                owner_state,
                "virtualized_state.site",
            ));
        }
        if let Some(saved_site) = owner_state.saved_sites.get(&details.site_hash)
            && Arc::ptr_eq(saved_site, baseline_command)
        {
            baseline_sources.push(format_virtualized_site_source(
                *owner_site_hash,
                owner_state,
                "virtualized_state.saved_sites",
            ));
        }
    }

    if baseline_sources.is_empty() {
        baseline_sources.push("unknown".to_string());
    }

    warn!(
        "Bakery structural site diff baseline origin: site_hash={} baseline_sources=[{}] baseline={} current={} new={}",
        details.site_hash,
        baseline_sources.join(", "),
        baseline_summary,
        current_summary,
        new_summary
    );
}

fn runtime_virtualized_site_has_pending_migrations(
    state: &VirtualizedSiteState,
    migrations: &HashMap<i64, Migration>,
) -> bool {
    state
        .active_circuits
        .keys()
        .any(|circuit_hash| migrations.contains_key(circuit_hash))
}

#[cfg(test)]
fn runtime_virtualized_site_has_remaining_observed_child_classes(
    state: &VirtualizedSiteState,
    down_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    up_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
) -> bool {
    let Some((target_down_class, target_up_class)) = site_class_handles(state.site.as_ref()) else {
        return false;
    };

    down_snapshot
        .values()
        .any(|entry| entry.parent == Some(target_down_class))
        || up_snapshot
            .values()
            .any(|entry| entry.parent == Some(target_up_class))
}

fn sync_runtime_virtualized_site_qdisc_handles_from_live_snapshot(
    state: &mut VirtualizedSiteState,
    down_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    up_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
) {
    let Some((target_down_class, target_up_class)) = site_class_handles(state.site.as_ref()) else {
        return;
    };

    if state.qdisc_handles.down.is_none()
        && let Some(entry) = down_snapshot.get(&target_down_class)
    {
        state.qdisc_handles.down = entry.leaf_qdisc_major;
    }

    if state.qdisc_handles.up.is_none()
        && let Some(entry) = up_snapshot.get(&target_up_class)
    {
        state.qdisc_handles.up = entry.leaf_qdisc_major;
    }
}

fn runtime_virtualized_site_prune_ready(
    state: &VirtualizedSiteState,
    migrations: &HashMap<i64, Migration>,
    now_unix: u64,
) -> bool {
    state.pending_prune
        && state.next_prune_attempt_unix <= now_unix
        && !runtime_virtualized_site_has_pending_migrations(state, migrations)
}

fn runtime_site_prune_missing_qdisc_is_harmless(summary: &str) -> bool {
    summary
        .to_ascii_lowercase()
        .contains("cannot find specified qdisc on specified device")
}

fn remaining_inactive_branch_class_handles(
    state: &VirtualizedSiteState,
) -> (HashSet<TcHandle>, HashSet<TcHandle>) {
    let mut down = HashSet::new();
    let mut up = HashSet::new();

    for site in state.prune_sites.values() {
        if let Some((down_class, up_class)) = site_class_handles(site.as_ref()) {
            down.insert(down_class);
            up.insert(up_class);
        }
    }

    for circuit in state.prune_circuits.values() {
        if let BakeryCommands::AddCircuit {
            class_major,
            up_class_major,
            class_minor,
            ..
        } = circuit.as_ref()
        {
            down.insert(tc_handle_from_major_minor(*class_major, *class_minor));
            up.insert(tc_handle_from_major_minor(*up_class_major, *class_minor));
        }
    }

    if state.active_branch_hides_original_site()
        && let Some((down_class, up_class)) = site_class_handles(state.site.as_ref())
    {
        down.insert(down_class);
        up.insert(up_class);
    }

    (down, up)
}

fn summarize_handle_set(handles: &[TcHandle]) -> String {
    handles
        .iter()
        .take(4)
        .map(TcHandle::as_tc_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn classify_inactive_branch_observed_children(
    site: &BakeryCommands,
    down_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    up_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    expected_down: &HashSet<TcHandle>,
    expected_up: &HashSet<TcHandle>,
) -> Result<(), String> {
    let Some((parent_down_class, parent_up_class)) = site_class_handles(site) else {
        return Ok(());
    };

    let observed_down: Vec<TcHandle> = down_snapshot
        .values()
        .filter_map(|entry| (entry.parent == Some(parent_down_class)).then_some(entry.class_id))
        .collect();
    let observed_up: Vec<TcHandle> = up_snapshot
        .values()
        .filter_map(|entry| (entry.parent == Some(parent_up_class)).then_some(entry.class_id))
        .collect();

    let unexpected_down: Vec<TcHandle> = observed_down
        .iter()
        .copied()
        .filter(|handle| !expected_down.contains(handle))
        .collect();
    let unexpected_up: Vec<TcHandle> = observed_up
        .iter()
        .copied()
        .filter(|handle| !expected_up.contains(handle))
        .collect();

    if unexpected_down.is_empty() && unexpected_up.is_empty() {
        return Ok(());
    }

    let mut details = Vec::new();
    if !unexpected_down.is_empty() {
        details.push(format!("down [{}]", summarize_handle_set(&unexpected_down)));
    }
    if !unexpected_up.is_empty() {
        details.push(format!("up [{}]", summarize_handle_set(&unexpected_up)));
    }
    Err(format!(
        "Inactive branch cleanup encountered unexpected live child classes still attached: {}",
        details.join(", ")
    ))
}

fn verify_pruned_handles_absent(
    config: &Arc<Config>,
    down_handles: &[TcHandle],
    up_handles: &[TcHandle],
    context: &str,
) -> Result<(), String> {
    let down_snapshot = read_live_class_snapshot(&config.isp_interface())?;
    let up_snapshot = read_live_class_snapshot(&config.internet_interface())?;

    let lingering_down: Vec<String> = down_handles
        .iter()
        .filter(|handle| down_snapshot.contains_key(handle))
        .map(TcHandle::as_tc_string)
        .collect();
    let lingering_up: Vec<String> = up_handles
        .iter()
        .filter(|handle| up_snapshot.contains_key(handle))
        .map(TcHandle::as_tc_string)
        .collect();

    if lingering_down.is_empty() && lingering_up.is_empty() {
        return Ok(());
    }

    let mut details = Vec::new();
    if !lingering_down.is_empty() {
        details.push(format!(
            "{} still present on {}",
            lingering_down.join(", "),
            config.isp_interface()
        ));
    }
    if !lingering_up.is_empty() {
        details.push(format!(
            "{} still present on {}",
            lingering_up.join(", "),
            config.internet_interface()
        ));
    }

    Err(format!(
        "{context}: targeted old-branch class prune did not take effect: {}",
        details.join("; ")
    ))
}

fn site_commands_observed_live(
    commands: &HashMap<i64, Arc<BakeryCommands>>,
    down_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    up_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
) -> bool {
    commands.values().all(|site| {
        let BakeryCommands::AddSite {
            parent_class_id,
            up_parent_class_id,
            ..
        } = site.as_ref()
        else {
            return false;
        };
        let Some((down_class, up_class)) = site_class_handles(site.as_ref()) else {
            return false;
        };
        let Some(down_entry) = down_snapshot.get(&down_class) else {
            return false;
        };
        let Some(up_entry) = up_snapshot.get(&up_class) else {
            return false;
        };
        live_parent_matches(down_entry.parent, *parent_class_id)
            && live_parent_matches(up_entry.parent, *up_parent_class_id)
    })
}

fn live_parent_matches(observed: Option<TcHandle>, expected: TcHandle) -> bool {
    if observed == Some(expected) {
        return true;
    }

    let (_, expected_minor) = expected.get_major_minor();
    expected_minor == 0 && observed.is_none()
}

fn verify_runtime_shadow_active(
    state: &VirtualizedSiteState,
    down_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    up_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
) -> Result<(), String> {
    for (site_hash, site) in &state.active_sites {
        let BakeryCommands::AddSite {
            parent_class_id,
            up_parent_class_id,
            ..
        } = site.as_ref()
        else {
            continue;
        };
        let Some((down_class, up_class)) = site_class_handles(site.as_ref()) else {
            return Err(format!(
                "Runtime cutover activation verification could not derive class handles for active site {}",
                site_hash
            ));
        };
        let Some(down_entry) = down_snapshot.get(&down_class) else {
            return Err(format!(
                "Runtime cutover activation verification did not observe active shadow site {} on downlink",
                site_hash
            ));
        };
        let Some(up_entry) = up_snapshot.get(&up_class) else {
            return Err(format!(
                "Runtime cutover activation verification did not observe active shadow site {} on uplink",
                site_hash
            ));
        };
        if !live_parent_matches(down_entry.parent, *parent_class_id)
            || !live_parent_matches(up_entry.parent, *up_parent_class_id)
        {
            warn!(
                "Bakery: top-level cutover site {} planned down_class={} up_class={} expected parents {}/{} observed parents {:?}/{:?}",
                site_hash,
                down_class.as_tc_string(),
                up_class.as_tc_string(),
                parent_class_id.as_tc_string(),
                up_parent_class_id.as_tc_string(),
                down_entry.parent,
                up_entry.parent
            );
            return Err(format!(
                "Runtime cutover activation verification observed active shadow site {} with wrong parents {:?}/{:?}, expected {}/{}",
                site_hash,
                down_entry.parent,
                up_entry.parent,
                parent_class_id.as_tc_string(),
                up_parent_class_id.as_tc_string()
            ));
        }
    }

    for (circuit_hash, active_circuit) in &state.active_circuits {
        let BakeryCommands::AddCircuit {
            parent_class_id,
            up_parent_class_id,
            ..
        } = active_circuit.as_ref()
        else {
            continue;
        };
        let Some((active_down_class, active_up_class)) =
            circuit_class_handles(active_circuit.as_ref())
        else {
            return Err(format!(
                "Runtime cutover activation verification could not derive active circuit handles for {}",
                circuit_hash
            ));
        };
        let Some(down_entry) = down_snapshot.get(&active_down_class) else {
            return Err(format!(
                "Runtime cutover activation verification did not observe active shadow circuit {} on downlink",
                circuit_hash
            ));
        };
        let Some(up_entry) = up_snapshot.get(&active_up_class) else {
            return Err(format!(
                "Runtime cutover activation verification did not observe active shadow circuit {} on uplink",
                circuit_hash
            ));
        };
        if !live_parent_matches(down_entry.parent, *parent_class_id)
            || !live_parent_matches(up_entry.parent, *up_parent_class_id)
        {
            return Err(format!(
                "Runtime cutover activation verification observed active shadow circuit {} with wrong parents {:?}/{:?}, expected {}/{}",
                circuit_hash,
                down_entry.parent,
                up_entry.parent,
                parent_class_id.as_tc_string(),
                up_parent_class_id.as_tc_string()
            ));
        }
    }

    Ok(())
}

fn site_prune_class_commands_for_observed_state(
    config: &Arc<Config>,
    site: &BakeryCommands,
    down_exists: bool,
    up_exists: bool,
) -> Option<Vec<Vec<String>>> {
    let BakeryCommands::AddSite {
        parent_class_id,
        up_parent_class_id,
        class_minor,
        ..
    } = site
    else {
        return None;
    };

    let mut commands = Vec::new();
    if down_exists {
        commands.push(vec![
            "class".to_string(),
            "del".to_string(),
            "dev".to_string(),
            config.isp_interface(),
            "parent".to_string(),
            parent_class_id.as_tc_string(),
            "classid".to_string(),
            format!(
                "0x{:x}:0x{:x}",
                parent_class_id.get_major_minor().0,
                class_minor
            ),
        ]);
    }
    if up_exists {
        commands.push(vec![
            "class".to_string(),
            "del".to_string(),
            "dev".to_string(),
            config.internet_interface(),
            "parent".to_string(),
            up_parent_class_id.as_tc_string(),
            "classid".to_string(),
            format!(
                "0x{:x}:0x{:x}",
                up_parent_class_id.get_major_minor().0,
                class_minor
            ),
        ]);
    }

    (!commands.is_empty()).then_some(commands)
}

fn execute_runtime_site_prune(
    config: &Arc<Config>,
    state: &mut VirtualizedSiteState,
    down_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    up_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
) -> Result<(), String> {
    sync_runtime_virtualized_site_qdisc_handles_from_live_snapshot(
        state,
        down_snapshot,
        up_snapshot,
    );

    if let Some(qdisc_prune) = site_prune_qdisc_commands(config, &state.qdisc_handles) {
        let qdisc_result =
            execute_in_memory(&qdisc_prune, "TreeGuard runtime deferred site qdisc prune");
        if !qdisc_result.ok {
            let summary = summarize_apply_result(
                "TreeGuard runtime deferred site qdisc prune",
                &qdisc_result,
            );
            if !runtime_site_prune_missing_qdisc_is_harmless(&summary) {
                return Err(summary);
            }
        }
    }

    let Some((target_down_class, target_up_class)) = site_class_handles(state.site.as_ref()) else {
        return Ok(());
    };
    let down_exists = down_snapshot.contains_key(&target_down_class);
    let up_exists = up_snapshot.contains_key(&target_up_class);
    let Some(class_prune) = site_prune_class_commands_for_observed_state(
        config,
        state.site.as_ref(),
        down_exists,
        up_exists,
    ) else {
        return Ok(());
    };
    let class_result = execute_in_memory(&class_prune, "TreeGuard runtime deferred site prune");
    if !class_result.ok {
        return Err(summarize_apply_result(
            "TreeGuard runtime deferred site prune",
            &class_result,
        ));
    }

    Ok(())
}

#[derive(Debug)]
enum RuntimePrunePassResult {
    Completed,
    Progress(String),
    Pending(String),
    Failed(String),
}

fn execute_runtime_virtualized_subtree_prune(
    config: &Arc<Config>,
    state: &mut VirtualizedSiteState,
    down_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    up_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
) -> RuntimePrunePassResult {
    let (expected_inactive_down_classes, expected_inactive_up_classes) =
        remaining_inactive_branch_class_handles(state);
    let mut protected_down_classes = HashSet::new();
    let mut protected_up_classes = HashSet::new();
    for command in state.active_circuits.values() {
        if let BakeryCommands::AddCircuit {
            class_major,
            up_class_major,
            class_minor,
            ..
        } = command.as_ref()
        {
            protected_down_classes.insert(tc_handle_from_major_minor(*class_major, *class_minor));
            protected_up_classes.insert(tc_handle_from_major_minor(*up_class_major, *class_minor));
        }
    }

    let ordered_circuit_hashes = ordered_prune_circuit_hashes(&state.prune_circuits);
    for circuit_hash in ordered_circuit_hashes {
        let Some(prune_circuit) = state.prune_circuits.get(&circuit_hash).cloned() else {
            continue;
        };
        let Some(commands) = observed_circuit_prune_commands(
            config,
            prune_circuit.as_ref(),
            Some(down_snapshot),
            Some(up_snapshot),
            &protected_down_classes,
            &protected_up_classes,
        ) else {
            state.prune_circuits.remove(&circuit_hash);
            continue;
        };
        if commands.is_empty() {
            state.prune_circuits.remove(&circuit_hash);
            continue;
        }
        let result = execute_in_memory(&commands, "TreeGuard runtime deferred child circuit prune");
        if !result.ok {
            return RuntimePrunePassResult::Failed(summarize_apply_result(
                "TreeGuard runtime deferred child circuit prune",
                &result,
            ));
        }
        let BakeryCommands::AddCircuit {
            class_major,
            up_class_major,
            class_minor,
            ..
        } = prune_circuit.as_ref()
        else {
            return RuntimePrunePassResult::Failed(
                "TreeGuard runtime deferred child circuit prune targeted a non-circuit command"
                    .to_string(),
            );
        };
        if let Err(summary) = verify_pruned_handles_absent(
            config,
            &[tc_handle_from_major_minor(*class_major, *class_minor)],
            &[tc_handle_from_major_minor(*up_class_major, *class_minor)],
            "TreeGuard runtime deferred child circuit prune",
        ) {
            return RuntimePrunePassResult::Failed(summary);
        }
        state.prune_circuits.remove(&circuit_hash);
        return RuntimePrunePassResult::Progress(
            "Inactive branch circuit cleanup applied; refreshing live verification".to_string(),
        );
    }

    let ordered_hashes = ordered_prune_site_hashes(&state.prune_sites);
    for site_hash in ordered_hashes {
        let Some(prune_site) = state.prune_sites.get(&site_hash).cloned() else {
            continue;
        };
        if site_has_observed_child_classes(prune_site.as_ref(), down_snapshot, up_snapshot) {
            if let Err(summary) = classify_inactive_branch_observed_children(
                prune_site.as_ref(),
                down_snapshot,
                up_snapshot,
                &expected_inactive_down_classes,
                &expected_inactive_up_classes,
            ) {
                return RuntimePrunePassResult::Failed(summary);
            }
            return RuntimePrunePassResult::Pending(
                "Observed live child classes still attached".to_string(),
            );
        }
        let Some((down_class, up_class)) = site_class_handles(prune_site.as_ref()) else {
            state.prune_sites.remove(&site_hash);
            continue;
        };
        let down_exists = down_snapshot.contains_key(&down_class);
        let up_exists = up_snapshot.contains_key(&up_class);
        if let Some(class_prune) = site_prune_class_commands_for_observed_state(
            config,
            prune_site.as_ref(),
            down_exists,
            up_exists,
        ) {
            let result =
                execute_in_memory(&class_prune, "TreeGuard runtime deferred child site prune");
            if !result.ok {
                return RuntimePrunePassResult::Failed(summarize_apply_result(
                    "TreeGuard runtime deferred child site prune",
                    &result,
                ));
            }
            if let Err(summary) = verify_pruned_handles_absent(
                config,
                &[down_class],
                &[up_class],
                "TreeGuard runtime deferred child site prune",
            ) {
                return RuntimePrunePassResult::Failed(summary);
            }
            state.prune_sites.remove(&site_hash);
            return RuntimePrunePassResult::Progress(
                "Inactive branch site cleanup applied; refreshing live verification".to_string(),
            );
        }
        state.prune_sites.remove(&site_hash);
    }

    if state.active_branch_hides_original_site() {
        if site_has_observed_child_classes(state.site.as_ref(), down_snapshot, up_snapshot) {
            if let Err(summary) = classify_inactive_branch_observed_children(
                state.site.as_ref(),
                down_snapshot,
                up_snapshot,
                &expected_inactive_down_classes,
                &expected_inactive_up_classes,
            ) {
                return RuntimePrunePassResult::Failed(summary);
            }
            return RuntimePrunePassResult::Pending(
                "Observed live child classes still attached".to_string(),
            );
        }

        if site_prune_commands(config, state).is_none() {
            return RuntimePrunePassResult::Completed;
        }

        return match execute_runtime_site_prune(config, state, down_snapshot, up_snapshot) {
            Ok(()) => {
                let Some((down_class, up_class)) = site_class_handles(state.site.as_ref()) else {
                    return RuntimePrunePassResult::Completed;
                };
                match verify_pruned_handles_absent(
                    config,
                    &[down_class],
                    &[up_class],
                    "TreeGuard runtime deferred root site prune",
                ) {
                    Ok(()) => RuntimePrunePassResult::Progress(
                        "Inactive branch root cleanup applied; refreshing live verification"
                            .to_string(),
                    ),
                    Err(summary) => RuntimePrunePassResult::Failed(summary),
                }
            }
            Err(summary) => RuntimePrunePassResult::Failed(summary),
        };
    }

    RuntimePrunePassResult::Completed
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn build_post_restore_virtualized_state(
    saved_state: VirtualizedSiteState,
    restore_active_sites: HashMap<i64, Arc<BakeryCommands>>,
    restore_active_circuits: HashMap<i64, Arc<BakeryCommands>>,
    now_unix: u64,
) -> Option<VirtualizedSiteState> {
    let prune_shadow_sites = saved_state.active_sites.clone();
    let prune_shadow_circuits = saved_state.active_circuits.clone();

    if prune_shadow_sites.is_empty() && prune_shadow_circuits.is_empty() {
        return None;
    }

    Some(VirtualizedSiteState {
        site_name: saved_state.site_name,
        site: saved_state.site,
        saved_sites: saved_state.saved_sites,
        saved_circuits: saved_state.saved_circuits,
        active_sites: restore_active_sites,
        active_circuits: restore_active_circuits,
        prune_sites: prune_shadow_sites,
        prune_circuits: prune_shadow_circuits,
        qdisc_handles: VirtualizedSiteQdiscHandles {
            down: None,
            up: None,
        },
        active_branch: RuntimeVirtualizedActiveBranch::Original,
        lifecycle: RuntimeVirtualizedBranchLifecycle::PhysicalActiveCleanupPending,
        pending_prune: true,
        next_prune_attempt_unix: now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS),
    })
}

fn runtime_node_operation_action(virtualized: bool) -> RuntimeNodeOperationAction {
    if virtualized {
        RuntimeNodeOperationAction::Virtualize
    } else {
        RuntimeNodeOperationAction::Restore
    }
}

fn runtime_node_operation_is_active(status: RuntimeNodeOperationStatus) -> bool {
    matches!(
        status,
        RuntimeNodeOperationStatus::Submitted
            | RuntimeNodeOperationStatus::Applying
            | RuntimeNodeOperationStatus::AppliedAwaitingCleanup
    )
}

fn runtime_node_operation_consumes_capacity(status: RuntimeNodeOperationStatus) -> bool {
    matches!(
        status,
        RuntimeNodeOperationStatus::Submitted
            | RuntimeNodeOperationStatus::Applying
            | RuntimeNodeOperationStatus::AppliedAwaitingCleanup
    )
}

fn active_runtime_node_operation_count(
    runtime_node_operations: &HashMap<i64, RuntimeNodeOperation>,
) -> usize {
    runtime_node_operations
        .values()
        .filter(|operation| runtime_node_operation_consumes_capacity(operation.status))
        .count()
}

fn runtime_virtualization_target_site<'a>(
    site_hash: i64,
    sites: &'a HashMap<i64, Arc<BakeryCommands>>,
    virtualized_sites: &'a HashMap<i64, VirtualizedSiteState>,
) -> Option<&'a Arc<BakeryCommands>> {
    sites
        .get(&site_hash)
        .or_else(|| virtualized_sites.get(&site_hash).map(|state| &state.site))
}

fn runtime_virtualization_target_is_top_level(
    site_hash: i64,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    virtualized_sites: &HashMap<i64, VirtualizedSiteState>,
) -> bool {
    runtime_virtualization_target_site(site_hash, sites, virtualized_sites)
        .is_some_and(|site| site_is_top_level(site.as_ref()))
}

fn active_top_level_runtime_operation_conflict(
    site_hash: i64,
    runtime_node_operations: &HashMap<i64, RuntimeNodeOperation>,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    virtualized_sites: &HashMap<i64, VirtualizedSiteState>,
) -> Option<i64> {
    runtime_node_operations
        .iter()
        .find_map(|(other_site_hash, operation)| {
            if *other_site_hash != site_hash
                && runtime_node_operation_is_active(operation.status)
                && runtime_virtualization_target_is_top_level(
                    *other_site_hash,
                    sites,
                    virtualized_sites,
                )
            {
                Some(*other_site_hash)
            } else {
                None
            }
        })
}

fn runtime_error_suggests_material_desync(summary: &str) -> bool {
    let lower = summary.to_ascii_lowercase();
    lower.contains("rtnetlink")
        || lower.contains("specified class not found")
        || lower.contains("cannot find specified qdisc")
        || lower.contains("cannot move an existing qdisc")
        || lower.contains("device or resource busy")
        || lower.contains("top-level runtime cutover failed")
        || lower.contains("top-level runtime cutover verification failed")
        || lower.contains("unexpected live child classes")
}

fn rebuild_runtime_branch_snapshots(
    virtualized_sites: &HashMap<i64, VirtualizedSiteState>,
) -> HashMap<i64, BakeryRuntimeNodeBranchSnapshot> {
    virtualized_sites
        .iter()
        .map(|(site_hash, state)| {
            let mut active_site_hashes: Vec<i64> = state.active_sites.keys().copied().collect();
            active_site_hashes.sort_unstable();
            let mut saved_site_hashes: Vec<i64> = state.saved_sites.keys().copied().collect();
            saved_site_hashes.sort_unstable();
            let mut prune_site_hashes: Vec<i64> = state.prune_sites.keys().copied().collect();
            prune_site_hashes.sort_unstable();
            (
                *site_hash,
                BakeryRuntimeNodeBranchSnapshot {
                    site_hash: *site_hash,
                    site_name: state.site_name.clone(),
                    active_branch: runtime_active_branch_label(state.active_branch).to_string(),
                    lifecycle: runtime_lifecycle_label(state.lifecycle).to_string(),
                    pending_prune: state.pending_prune,
                    next_prune_attempt_unix: state
                        .pending_prune
                        .then_some(state.next_prune_attempt_unix),
                    active_site_hashes,
                    saved_site_hashes,
                    prune_site_hashes,
                    qdisc_down_major: state.qdisc_handles.down,
                    qdisc_up_major: state.qdisc_handles.up,
                },
            )
        })
        .collect()
}

fn update_desync_state_from_runtime_state(
    runtime_node_operations: &HashMap<i64, RuntimeNodeOperation>,
    virtualized_sites: &HashMap<i64, VirtualizedSiteState>,
) {
    let snapshot = rebuild_runtime_operations_snapshot(runtime_node_operations);
    let dirty_count = snapshot.dirty_count;
    let runtime_operations_by_site = runtime_node_operations
        .iter()
        .map(|(site_hash, operation)| (*site_hash, operation.snapshot()))
        .collect();
    let runtime_branch_states_by_site = rebuild_runtime_branch_snapshots(virtualized_sites);
    {
        let mut state = telemetry_state().write();
        state.runtime_operations = snapshot;
        state.dirty_subtree_count = dirty_count;
        state.runtime_operations_by_site = runtime_operations_by_site;
        state.runtime_branch_states_by_site = runtime_branch_states_by_site;
    }
    if dirty_count >= RUNTIME_DIRTY_SUBTREE_RELOAD_THRESHOLD
        && bakery_reload_required_reason().is_none()
    {
        mark_reload_required(format!(
            "Bakery detected {} dirty runtime subtree operations; a full reload is now required before further incremental topology mutations.",
            dirty_count
        ));
    }
}

fn flush_deferred_runtime_site_prunes(
    config: &Arc<Config>,
    virtualized_sites: &mut HashMap<i64, VirtualizedSiteState>,
    migrations: &HashMap<i64, Migration>,
    runtime_node_operations: &mut HashMap<i64, RuntimeNodeOperation>,
) {
    if bakery_reload_required_reason().is_some() {
        return;
    }
    let now_unix = unix_now();
    let mut grouped_events = GroupedBakeryEventLimiter::default();
    let pending_site_hashes: Vec<i64> = virtualized_sites
        .iter()
        .filter_map(|(site_hash, state)| {
            runtime_virtualized_site_prune_ready(state, migrations, now_unix).then_some(*site_hash)
        })
        .collect();

    if pending_site_hashes.is_empty() {
        return;
    }

    let down_snapshot = match read_live_class_snapshot(&config.isp_interface()) {
        Ok(snapshot) => snapshot,
        Err(summary) => {
            warn!(
                "Bakery: unable to snapshot live downlink classes for deferred prune pass: {summary}"
            );
            for site_hash in pending_site_hashes {
                if let Some(state) = virtualized_sites.get_mut(&site_hash) {
                    let site_name = state.site_name.clone();
                    let site_display =
                        runtime_site_display_name(site_hash, Some(site_name.as_str()));
                    let retry_at = now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS);
                    state.next_prune_attempt_unix = retry_at;
                    if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                        operation.attempt_count = operation.attempt_count.saturating_add(1);
                        if operation.attempt_count >= RUNTIME_SITE_PRUNE_MAX_ATTEMPTS {
                            state.pending_prune = false;
                            state.next_prune_attempt_unix = 0;
                            state.lifecycle = RuntimeVirtualizedBranchLifecycle::Failed;
                            operation.update_status(
                                runtime_status_for_virtualized_state(state),
                                now_unix,
                                Some(summary.clone()),
                                None,
                            );
                            grouped_events.emit_with_site_name(
                                format!("runtime_site_prune_dirty|down_snapshot|{summary}"),
                                "runtime_site_prune_dirty",
                                "error",
                                Some((site_hash, Some(site_name.clone()))),
                                format!(
                                    "Deferred runtime site prune for site {} marked Dirty after snapshot failure: {}",
                                    site_display, summary
                                ),
                                format!(
                                    "runtime site prune Dirty events due to downlink snapshot failure: {}",
                                    summary
                                ),
                            );
                        } else {
                            state.lifecycle =
                                preserve_cutover_pending_or_lifecycle_from_active_branch(state);
                            operation.update_status(
                                runtime_status_for_virtualized_state(state),
                                now_unix,
                                Some(summary.clone()),
                                Some(retry_at),
                            );
                            grouped_events.emit_with_site_name(
                                format!("runtime_site_prune_retry|down_snapshot|{summary}"),
                                "runtime_site_prune_retry",
                                "warning",
                                Some((site_hash, Some(site_name))),
                                format!(
                                    "Deferred runtime site prune retry {}/{} for site {} postponed: {}",
                                    operation.attempt_count,
                                    RUNTIME_SITE_PRUNE_MAX_ATTEMPTS,
                                    site_display,
                                    summary
                                ),
                                format!(
                                    "runtime site prune retry events due to downlink snapshot failure: {}",
                                    summary
                                ),
                            );
                        }
                    }
                }
            }
            grouped_events.flush();
            return;
        }
    };
    let up_snapshot = match read_live_class_snapshot(&config.internet_interface()) {
        Ok(snapshot) => snapshot,
        Err(summary) => {
            warn!(
                "Bakery: unable to snapshot live uplink classes for deferred prune pass: {summary}"
            );
            for site_hash in pending_site_hashes {
                if let Some(state) = virtualized_sites.get_mut(&site_hash) {
                    let site_name = state.site_name.clone();
                    let site_display =
                        runtime_site_display_name(site_hash, Some(site_name.as_str()));
                    let retry_at = now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS);
                    state.next_prune_attempt_unix = retry_at;
                    if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                        operation.attempt_count = operation.attempt_count.saturating_add(1);
                        if operation.attempt_count >= RUNTIME_SITE_PRUNE_MAX_ATTEMPTS {
                            state.pending_prune = false;
                            state.next_prune_attempt_unix = 0;
                            state.lifecycle = RuntimeVirtualizedBranchLifecycle::Failed;
                            operation.update_status(
                                runtime_status_for_virtualized_state(state),
                                now_unix,
                                Some(summary.clone()),
                                None,
                            );
                            grouped_events.emit_with_site_name(
                                format!("runtime_site_prune_dirty|up_snapshot|{summary}"),
                                "runtime_site_prune_dirty",
                                "error",
                                Some((site_hash, Some(site_name.clone()))),
                                format!(
                                    "Deferred runtime site prune for site {} marked Dirty after snapshot failure: {}",
                                    site_display, summary
                                ),
                                format!(
                                    "runtime site prune Dirty events due to uplink snapshot failure: {}",
                                    summary
                                ),
                            );
                        } else {
                            state.lifecycle =
                                preserve_cutover_pending_or_lifecycle_from_active_branch(state);
                            operation.update_status(
                                runtime_status_for_virtualized_state(state),
                                now_unix,
                                Some(summary.clone()),
                                Some(retry_at),
                            );
                            grouped_events.emit_with_site_name(
                                format!("runtime_site_prune_retry|up_snapshot|{summary}"),
                                "runtime_site_prune_retry",
                                "warning",
                                Some((site_hash, Some(site_name))),
                                format!(
                                    "Deferred runtime site prune retry {}/{} for site {} postponed: {}",
                                    operation.attempt_count,
                                    RUNTIME_SITE_PRUNE_MAX_ATTEMPTS,
                                    site_display,
                                    summary
                                ),
                                format!(
                                    "runtime site prune retry events due to uplink snapshot failure: {}",
                                    summary
                                ),
                            );
                        }
                    }
                }
            }
            grouped_events.flush();
            return;
        }
    };

    flush_deferred_runtime_site_prunes_with_snapshots(
        config,
        virtualized_sites,
        migrations,
        runtime_node_operations,
        &down_snapshot,
        &up_snapshot,
    );
}

fn flush_deferred_runtime_site_prunes_with_snapshots(
    config: &Arc<Config>,
    virtualized_sites: &mut HashMap<i64, VirtualizedSiteState>,
    migrations: &HashMap<i64, Migration>,
    runtime_node_operations: &mut HashMap<i64, RuntimeNodeOperation>,
    down_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
    up_snapshot: &HashMap<TcHandle, LiveTcClassEntry>,
) {
    if bakery_reload_required_reason().is_some() {
        return;
    }
    let now_unix = unix_now();
    let mut grouped_events = GroupedBakeryEventLimiter::default();
    let pending_site_hashes: Vec<i64> = virtualized_sites
        .iter()
        .filter_map(|(site_hash, state)| {
            runtime_virtualized_site_prune_ready(state, migrations, now_unix).then_some(*site_hash)
        })
        .collect();

    if pending_site_hashes.is_empty() {
        return;
    }

    let mut completed_restore_cleanup = Vec::new();
    for site_hash in pending_site_hashes {
        let Some(state) = virtualized_sites.get_mut(&site_hash) else {
            continue;
        };
        if state.lifecycle == RuntimeVirtualizedBranchLifecycle::CutoverPending {
            debug!(
                "Bakery: deferred runtime handler inspecting CutoverPending site {} (pending_prune={}, active_branch={:?}, next_prune_attempt_unix={})",
                site_hash, state.pending_prune, state.active_branch, state.next_prune_attempt_unix
            );
        }
        sync_runtime_virtualized_site_qdisc_handles_from_live_snapshot(
            state,
            down_snapshot,
            up_snapshot,
        );
        if state.lifecycle == RuntimeVirtualizedBranchLifecycle::CutoverPending
            && state.active_branch_hides_original_site()
        {
            let site_name = state.site_name.clone();
            let site_display = runtime_site_display_name(site_hash, Some(site_name.as_str()));
            debug!(
                "Bakery: evaluating runtime cutover activation for site {}",
                site_hash
            );
            match verify_runtime_shadow_active(state, down_snapshot, up_snapshot) {
                Ok(()) => {
                    state.pending_prune = false;
                    state.next_prune_attempt_unix = 0;
                    state.lifecycle = RuntimeVirtualizedBranchLifecycle::FlattenedActive;
                    if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                        operation.update_status(
                            runtime_status_for_virtualized_state(state),
                            now_unix,
                            None,
                            None,
                        );
                    }
                    grouped_events.emit_with_site_name(
                        "runtime_cutover_completed|completed",
                        "runtime_cutover_completed",
                        "info",
                        Some((site_hash, Some(site_name.clone()))),
                        format!(
                            "Runtime cutover completed for site {}; shadow branch is active and original branch is standby.",
                            site_display
                        ),
                        "runtime cutover completion events".to_string(),
                    );
                    continue;
                }
                Err(summary) => {
                    if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                        operation.attempt_count = operation.attempt_count.saturating_add(1);
                        if operation.attempt_count >= RUNTIME_SITE_PRUNE_MAX_ATTEMPTS {
                            warn!(
                                "Bakery: runtime cutover activation for site {} not ready on final attempt {}: {}",
                                site_hash, operation.attempt_count, summary
                            );
                            state.pending_prune = false;
                            state.next_prune_attempt_unix = 0;
                            state.lifecycle = RuntimeVirtualizedBranchLifecycle::Failed;
                            operation.update_status(
                                runtime_status_for_virtualized_state(state),
                                now_unix,
                                Some(summary.clone()),
                                None,
                            );
                            grouped_events.emit_with_site_name(
                                format!("runtime_cutover_dirty|activation_failed|{summary}"),
                                "runtime_cutover_dirty",
                                "error",
                                Some((site_hash, Some(site_name.clone()))),
                                format!(
                                    "Runtime cutover for site {} marked Dirty after {} attempts: {}",
                                    site_display, operation.attempt_count, summary
                                ),
                                format!(
                                    "runtime cutover Dirty events due to activation verification failure: {}",
                                    summary
                                ),
                            );
                        } else {
                            debug!(
                                "Bakery: runtime cutover activation for site {} not ready on retry {}/{}: {}",
                                site_hash,
                                operation.attempt_count,
                                RUNTIME_SITE_PRUNE_MAX_ATTEMPTS,
                                summary
                            );
                            let retry_at = now_unix.saturating_add(RUNTIME_CUTOVER_RETRY_SECONDS);
                            state.next_prune_attempt_unix = retry_at;
                            operation.update_status(
                                runtime_status_for_virtualized_state(state),
                                now_unix,
                                Some(summary.clone()),
                                Some(retry_at),
                            );
                            grouped_events.emit_with_site_name(
                                format!("runtime_cutover_retry|activation_failed|{summary}"),
                                "runtime_cutover_retry",
                                "warning",
                                Some((site_hash, Some(site_name.clone()))),
                                format!(
                                    "Runtime cutover retry {}/{} for site {} waiting for active/standby convergence: {}",
                                    operation.attempt_count,
                                    RUNTIME_SITE_PRUNE_MAX_ATTEMPTS,
                                    site_display,
                                    summary
                                ),
                                format!(
                                    "runtime cutover retry events due to activation verification failure: {}",
                                    summary
                                ),
                            );
                        }
                    } else {
                        debug!(
                            "Bakery: runtime cutover activation for site {} not ready with no runtime operation state: {}",
                            site_hash, summary
                        );
                    }
                    continue;
                }
            }
        }
        match execute_runtime_virtualized_subtree_prune(config, state, down_snapshot, up_snapshot) {
            RuntimePrunePassResult::Completed => {
                if state.lifecycle == RuntimeVirtualizedBranchLifecycle::CutoverPending {
                    debug!(
                        "Bakery: CutoverPending site {} fell through to generic prune Completed",
                        site_hash
                    );
                }
                state.pending_prune = false;
                state.next_prune_attempt_unix = 0;
                state.lifecycle = preserve_cutover_pending_or_lifecycle_from_active_branch(state);
                if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                    operation.update_status(
                        runtime_status_for_virtualized_state(state),
                        now_unix,
                        None,
                        None,
                    );
                }
                grouped_events.emit_with_site_name(
                    "runtime_site_prune_completed|completed",
                    "runtime_site_prune_completed",
                    "info",
                    Some((site_hash, Some(state.site_name.clone()))),
                    format!(
                        "Deferred runtime site prune completed for site {}.",
                        runtime_site_display_name(site_hash, Some(state.site_name.as_str()))
                    ),
                    "runtime site prune completion events".to_string(),
                );
                if !state.active_branch_hides_original_site() {
                    completed_restore_cleanup.push(site_hash);
                }
            }
            RuntimePrunePassResult::Progress(summary) => {
                if state.lifecycle == RuntimeVirtualizedBranchLifecycle::CutoverPending {
                    debug!(
                        "Bakery: CutoverPending site {} fell through to generic prune Progress: {}",
                        site_hash, summary
                    );
                }
                state.next_prune_attempt_unix = now_unix;
                state.lifecycle = preserve_cutover_pending_or_lifecycle_from_active_branch(state);
                if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                    operation.update_status(
                        runtime_status_for_virtualized_state(state),
                        now_unix,
                        Some(summary.clone()),
                        Some(now_unix),
                    );
                }
                grouped_events.emit_with_site_name(
                    format!("runtime_site_prune_progress|{summary}"),
                    "runtime_site_prune_progress",
                    "info",
                    Some((site_hash, Some(state.site_name.clone()))),
                    format!(
                        "Deferred runtime site prune for site {} made progress: {}",
                        runtime_site_display_name(site_hash, Some(state.site_name.as_str())),
                        summary
                    ),
                    format!("runtime site prune progress events: {}", summary),
                );
            }
            RuntimePrunePassResult::Pending(summary) => {
                if state.lifecycle == RuntimeVirtualizedBranchLifecycle::CutoverPending {
                    debug!(
                        "Bakery: CutoverPending site {} fell through to generic prune Pending: {}",
                        site_hash, summary
                    );
                }
                let retry_at = now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS);
                state.next_prune_attempt_unix = retry_at;
                state.lifecycle = preserve_cutover_pending_or_lifecycle_from_active_branch(state);
                if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                    operation.update_status(
                        runtime_status_for_virtualized_state(state),
                        now_unix,
                        Some(summary),
                        Some(retry_at),
                    );
                }
            }
            RuntimePrunePassResult::Failed(summary) => {
                if state.lifecycle == RuntimeVirtualizedBranchLifecycle::CutoverPending {
                    warn!(
                        "Bakery: CutoverPending site {} fell through to generic prune Failed: {}",
                        site_hash, summary
                    );
                }
                if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                    operation.attempt_count = operation.attempt_count.saturating_add(1);
                    if operation.attempt_count >= RUNTIME_SITE_PRUNE_MAX_ATTEMPTS {
                        state.pending_prune = false;
                        state.next_prune_attempt_unix = 0;
                        state.lifecycle = RuntimeVirtualizedBranchLifecycle::Failed;
                        operation.update_status(
                            runtime_status_for_virtualized_state(state),
                            now_unix,
                            Some(summary.clone()),
                            None,
                        );
                        grouped_events.emit_with_site_name(
                            format!("runtime_site_prune_dirty|execute_failed|{summary}"),
                            "runtime_site_prune_dirty",
                            "error",
                            Some((site_hash, Some(state.site_name.clone()))),
                            format!(
                                "Deferred runtime site prune for site {} marked Dirty after {} attempts: {}",
                                runtime_site_display_name(site_hash, Some(state.site_name.as_str())),
                                operation.attempt_count,
                                summary
                            ),
                            format!(
                                "runtime site prune Dirty events due to execution failure: {}",
                                summary
                            ),
                        );
                    } else {
                        let retry_at = now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS);
                        state.next_prune_attempt_unix = retry_at;
                        state.lifecycle =
                            preserve_cutover_pending_or_lifecycle_from_active_branch(state);
                        operation.update_status(
                            runtime_status_for_virtualized_state(state),
                            now_unix,
                            Some(summary.clone()),
                            Some(retry_at),
                        );
                        grouped_events.emit_with_site_name(
                            format!("runtime_site_prune_retry|execute_failed|{summary}"),
                            "runtime_site_prune_retry",
                            "warning",
                            Some((site_hash, Some(state.site_name.clone()))),
                            format!(
                                "Deferred runtime site prune retry {}/{} for site {} failed: {}",
                                operation.attempt_count,
                                RUNTIME_SITE_PRUNE_MAX_ATTEMPTS,
                                runtime_site_display_name(
                                    site_hash,
                                    Some(state.site_name.as_str())
                                ),
                                summary
                            ),
                            format!(
                                "runtime site prune retry events due to execution failure: {}",
                                summary
                            ),
                        );
                    }
                } else {
                    state.next_prune_attempt_unix =
                        now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS);
                    grouped_events.emit_with_site_name(
                        format!("runtime_site_prune_retry|outside_tracking|{summary}"),
                        "runtime_site_prune_retry",
                        "warning",
                        Some((site_hash, Some(state.site_name.clone()))),
                        format!(
                            "Deferred runtime site prune for site {} failed outside operation tracking: {}",
                            runtime_site_display_name(site_hash, Some(state.site_name.as_str())),
                            summary
                        ),
                        format!(
                            "runtime site prune retry events outside operation tracking: {}",
                            summary
                        ),
                    );
                }
                debug!(
                    "Bakery: deferred runtime site prune for {} failed again: {}",
                    site_hash, summary
                );
            }
        }
    }
    for site_hash in completed_restore_cleanup {
        virtualized_sites.remove(&site_hash);
    }
    grouped_events.flush();
    update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);
}

#[allow(clippy::too_many_arguments)]
fn handle_treeguard_set_node_virtual_live(
    site_hash: i64,
    virtualized: bool,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &HashMap<i64, u64>,
    mq_layout: &Option<MqDeviceLayout>,
    qdisc_handles: &mut QdiscHandleState,
    migrations: &mut HashMap<i64, Migration>,
    virtualized_sites: &mut HashMap<i64, VirtualizedSiteState>,
    runtime_node_operations: &mut HashMap<i64, RuntimeNodeOperation>,
    next_runtime_operation_id: &mut u64,
) -> RuntimeNodeOperationSnapshot {
    let now_unix = unix_now();
    let action = runtime_node_operation_action(virtualized);
    let retained_site_name = virtualized_sites
        .get(&site_hash)
        .map(|state| state.site_name.clone());
    if let Some(reason) = bakery_reload_required_reason() {
        let snapshot = if let Some(existing) = runtime_node_operations.get(&site_hash)
            && existing.action == action
        {
            existing.snapshot()
        } else {
            let operation_id = *next_runtime_operation_id;
            *next_runtime_operation_id = next_runtime_operation_id.saturating_add(1);
            let mut operation = RuntimeNodeOperation::new(
                operation_id,
                site_hash,
                retained_site_name.clone(),
                action,
                now_unix,
            );
            operation.attempt_count = 1;
            operation.update_status(
                RuntimeNodeOperationStatus::Dirty,
                now_unix,
                Some(reason.clone()),
                None,
            );
            runtime_node_operations.insert(site_hash, operation.clone());
            update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);
            operation.snapshot()
        };
        return snapshot;
    }
    if let Some(existing) = runtime_node_operations.get(&site_hash)
        && runtime_node_operation_is_active(existing.status)
    {
        return existing.snapshot();
    }

    if runtime_virtualization_target_is_top_level(site_hash, sites, virtualized_sites)
        && let Some(conflicting_site_hash) = active_top_level_runtime_operation_conflict(
            site_hash,
            runtime_node_operations,
            sites,
            virtualized_sites,
        )
    {
        let operation_id = runtime_node_operations
            .get(&site_hash)
            .map(|operation| operation.operation_id)
            .unwrap_or_else(|| {
                let next = *next_runtime_operation_id;
                *next_runtime_operation_id = next_runtime_operation_id.saturating_add(1);
                next
            });
        let retry_at = now_unix.saturating_add(RUNTIME_NODE_OPERATION_DEFERRED_RETRY_SECONDS);
        let mut operation = RuntimeNodeOperation::new(
            operation_id,
            site_hash,
            retained_site_name.clone(),
            action,
            now_unix,
        );
        operation.attempt_count = runtime_node_operations
            .get(&site_hash)
            .map(|existing| existing.attempt_count.saturating_add(1))
            .unwrap_or(1);
        let site_label = runtime_site_label(site_hash, operation.site_name.as_deref());
        let summary = format!(
            "Bakery allows only one top-level TreeGuard runtime operation in flight; site {} is still active, so TreeGuard {} for {} is deferred.",
            conflicting_site_hash,
            if virtualized {
                "virtualization"
            } else {
                "restore"
            },
            site_label
        );
        operation.update_status(
            RuntimeNodeOperationStatus::Deferred,
            now_unix,
            Some(summary.clone()),
            Some(retry_at),
        );
        runtime_node_operations.insert(site_hash, operation.clone());
        update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);
        push_bakery_event_with_site(
            "runtime_node_op_deferred",
            "warning",
            Some(site_hash),
            summary,
        );
        return operation.snapshot();
    }

    if active_runtime_node_operation_count(runtime_node_operations)
        >= RUNTIME_NODE_OPERATION_CAPACITY
    {
        let operation_id = runtime_node_operations
            .get(&site_hash)
            .map(|operation| operation.operation_id)
            .unwrap_or_else(|| {
                let next = *next_runtime_operation_id;
                *next_runtime_operation_id = next_runtime_operation_id.saturating_add(1);
                next
            });
        let retry_at = now_unix.saturating_add(RUNTIME_NODE_OPERATION_DEFERRED_RETRY_SECONDS);
        let mut operation = RuntimeNodeOperation::new(
            operation_id,
            site_hash,
            retained_site_name.clone(),
            action,
            now_unix,
        );
        operation.attempt_count = runtime_node_operations
            .get(&site_hash)
            .map(|existing| existing.attempt_count.saturating_add(1))
            .unwrap_or(1);
        let site_label = runtime_site_label(site_hash, operation.site_name.as_deref());
        let summary = format!(
            "Bakery runtime node operation capacity ({}) is saturated; deferring TreeGuard {} for {}.",
            RUNTIME_NODE_OPERATION_CAPACITY,
            if virtualized {
                "virtualization"
            } else {
                "restore"
            },
            site_label
        );
        operation.update_status(
            RuntimeNodeOperationStatus::Deferred,
            now_unix,
            Some(summary.clone()),
            Some(retry_at),
        );
        runtime_node_operations.insert(site_hash, operation.clone());
        update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);
        push_bakery_event_with_site(
            "runtime_node_op_deferred",
            "warning",
            Some(site_hash),
            summary,
        );
        return operation.snapshot();
    }

    let operation_id = *next_runtime_operation_id;
    *next_runtime_operation_id = next_runtime_operation_id.saturating_add(1);
    let runtime_ops_snapshot = runtime_node_operations.clone();
    let sites_snapshot = sites.clone();
    let circuits_snapshot = circuits.clone();
    let qdisc_handles_snapshot = qdisc_handles.clone();
    let migrations_snapshot = migrations.clone();
    let virtualized_sites_snapshot = virtualized_sites.clone();

    let mut operation = RuntimeNodeOperation::new(
        operation_id,
        site_hash,
        retained_site_name,
        action,
        now_unix,
    );
    operation.attempt_count = 1;
    operation.update_status(RuntimeNodeOperationStatus::Applying, now_unix, None, None);
    runtime_node_operations.insert(site_hash, operation.clone());
    update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);

    let Ok(config) = lqos_config::load_config() else {
        operation.update_status(
            RuntimeNodeOperationStatus::Failed,
            now_unix,
            Some("Failed to load configuration".to_string()),
            None,
        );
        runtime_node_operations.insert(site_hash, operation.clone());
        update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);
        return operation.snapshot();
    };
    let current_site_names = load_current_runtime_site_names(&config);
    if operation.site_name.is_none() {
        operation.site_name =
            resolve_runtime_site_name(site_hash, current_site_names.as_ref(), virtualized_sites);
    }
    let site_label = runtime_site_label(site_hash, operation.site_name.as_deref());
    if let Some(reason) = live_tree_mutation_blocker_for_config(&config) {
        let summary = format!(
            "TreeGuard runtime {} for {} is blocked because {}.",
            if virtualized {
                "virtualization"
            } else {
                "restore"
            },
            site_label,
            reason
        );
        operation.update_status(
            RuntimeNodeOperationStatus::Deferred,
            now_unix,
            Some(summary.clone()),
            None,
        );
        runtime_node_operations.insert(site_hash, operation.clone());
        update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);
        push_bakery_event_with_site("runtime_node_op_deferred", "info", Some(site_hash), summary);
        return operation.snapshot();
    }

    if !cfg!(test) && !virtualized {
        let stale_runtime_summary = virtualized_sites.get(&site_hash).and_then(|saved_state| {
            current_site_names.as_ref().and_then(|names| {
                stale_retained_runtime_branch_summary(site_hash, saved_state, names)
                    .map(|summary| (saved_state.site_name.clone(), summary))
            })
        });
        if let Some((saved_site_name, summary)) = stale_runtime_summary {
            operation.site_name = Some(saved_site_name.clone());
            let summary_for_status = summary.clone();
            virtualized_sites.remove(&site_hash);
            operation.update_status(
                RuntimeNodeOperationStatus::Completed,
                now_unix,
                Some(summary_for_status),
                None,
            );
            runtime_node_operations.insert(site_hash, operation.clone());
            push_bakery_event_with_site_name(
                "runtime_state_invalidated",
                "warning",
                Some(site_hash),
                Some(saved_site_name),
                summary,
            );
            update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);
            return operation.snapshot();
        }
    }

    let mut failure_reason = None;
    let result: Result<(), String> = (|| {
        if virtualized {
            if virtualized_sites.contains_key(&site_hash) {
                return Ok(());
            }

            let Some(target_site) = sites.get(&site_hash).cloned() else {
                return Err(format!("Unknown site {}", site_label));
            };

            if site_is_top_level(target_site.as_ref()) {
                if let Some(reason) = top_level_runtime_virtualization_eligibility_error(
                    site_hash,
                    target_site.as_ref(),
                    sites,
                    circuits,
                ) {
                    failure_reason = reason.failure_reason;
                    return Err(reason.message);
                }
                let plan = build_top_level_virtualization_plan(
                    Arc::clone(&target_site),
                    sites,
                    circuits,
                    Some(&config),
                    virtualized_sites,
                    site_stick_offset(target_site.as_ref()),
                )?;
                apply_site_command_update_stages(
                    &config,
                    sites,
                    &plan.active_sites,
                    &plan.site_stages,
                    "TreeGuard runtime top-level site reparent",
                    true,
                )?;
                apply_top_level_circuit_command_updates(
                    &config,
                    sites,
                    circuits,
                    &plan.active_circuits,
                    live_circuits,
                    mq_layout,
                    qdisc_handles,
                    migrations,
                    "TreeGuard runtime top-level circuit reparent",
                )?;
                sites.remove(&site_hash);
                let state = VirtualizedSiteState {
                    site_name: operation
                        .site_name
                        .clone()
                        .unwrap_or_else(|| site_hash.to_string()),
                    site: target_site,
                    saved_sites: plan.saved_sites.clone(),
                    saved_circuits: plan.saved_circuits.clone(),
                    active_sites: plan
                        .active_sites
                        .into_iter()
                        .map(|(hash, update)| (hash, update.command))
                        .collect(),
                    active_circuits: plan
                        .active_circuits
                        .into_iter()
                        .map(|(hash, update)| (hash, update.command))
                        .collect(),
                    prune_sites: HashMap::new(),
                    prune_circuits: HashMap::new(),
                    qdisc_handles: VirtualizedSiteQdiscHandles {
                        down: None,
                        up: None,
                    },
                    active_branch: RuntimeVirtualizedActiveBranch::Shadow,
                    lifecycle: RuntimeVirtualizedBranchLifecycle::CutoverPending,
                    pending_prune: true,
                    next_prune_attempt_unix: now_unix.saturating_add(RUNTIME_CUTOVER_RETRY_SECONDS),
                };
                debug!(
                    "Bakery: queued top-level runtime cutover for site {} with {} active sites and {} active circuits",
                    site_hash,
                    state.active_sites.len(),
                    state.active_circuits.len()
                );
                virtualized_sites.insert(site_hash, state);
                if let Some(inserted) = virtualized_sites.get(&site_hash) {
                    debug!(
                        "Bakery: inserted top-level runtime state for site {} lifecycle={} pending_prune={} next_prune_attempt_unix={} active_branch={:?}",
                        site_hash,
                        runtime_lifecycle_label(inserted.lifecycle),
                        inserted.pending_prune,
                        inserted.next_prune_attempt_unix,
                        inserted.active_branch
                    );
                }
                return Ok(());
            }

            if let Some(reason) =
                nested_runtime_shadow_branch_eligibility_error(target_site.as_ref())
            {
                failure_reason = reason.failure_reason;
                return Err(reason.message);
            }

            if let Some(reason) =
                site_runtime_virtualization_eligibility_error(target_site.as_ref())
            {
                return Err(reason);
            }

            let plan =
                build_non_top_level_virtualization_plan(Arc::clone(&target_site), sites, circuits)?;
            apply_site_command_update_stages(
                &config,
                sites,
                &plan.active_sites,
                &plan.site_stages,
                "TreeGuard runtime child-site shadow create",
                true,
            )?;
            apply_circuit_command_updates(
                &config,
                sites,
                circuits,
                &plan.active_circuits,
                live_circuits,
                mq_layout,
                qdisc_handles,
                migrations,
                "TreeGuard runtime circuit reparent",
            )?;
            sites.remove(&site_hash);
            virtualized_sites.insert(
                site_hash,
                VirtualizedSiteState {
                    site_name: operation
                        .site_name
                        .clone()
                        .unwrap_or_else(|| site_hash.to_string()),
                    site: target_site,
                    saved_sites: plan.saved_sites.clone(),
                    saved_circuits: plan.saved_circuits.clone(),
                    active_sites: plan
                        .active_sites
                        .into_iter()
                        .map(|(hash, update)| (hash, update.command))
                        .collect(),
                    active_circuits: plan
                        .active_circuits
                        .into_iter()
                        .map(|(hash, update)| (hash, update.command))
                        .collect(),
                    prune_sites: HashMap::new(),
                    prune_circuits: HashMap::new(),
                    qdisc_handles: VirtualizedSiteQdiscHandles {
                        down: None,
                        up: None,
                    },
                    active_branch: RuntimeVirtualizedActiveBranch::Shadow,
                    lifecycle: RuntimeVirtualizedBranchLifecycle::CutoverPending,
                    pending_prune: true,
                    next_prune_attempt_unix: now_unix.saturating_add(RUNTIME_CUTOVER_RETRY_SECONDS),
                },
            );
            return Ok(());
        }

        let Some(saved_state) = virtualized_sites.get(&site_hash).cloned() else {
            return Ok(());
        };
        operation.site_name = Some(saved_state.site_name.clone());

        if !site_is_top_level(saved_state.site.as_ref())
            && let Some(reason) =
                site_runtime_virtualization_eligibility_error(saved_state.site.as_ref())
        {
            return Err(reason);
        }

        let reversible_standby = saved_state.active_branch_hides_original_site();
        let live_restore_snapshots = if reversible_standby {
            Some((
                read_live_class_snapshot(&config.isp_interface())?,
                read_live_class_snapshot(&config.internet_interface())?,
            ))
        } else {
            None
        };

        if !saved_state.pending_prune
            && let Some(cmds) = saved_state
                .site
                .to_commands(&config, ExecutionMode::Builder)
        {
            let root_already_live =
                live_restore_snapshots
                    .as_ref()
                    .is_some_and(|(down_snapshot, up_snapshot)| {
                        let mut only_root = HashMap::new();
                        only_root.insert(site_hash, saved_state.site.clone());
                        site_commands_observed_live(&only_root, down_snapshot, up_snapshot)
                    });
            if !root_already_live {
                let result =
                    execute_and_record_live_change(&cmds, "TreeGuard runtime hidden site restore");
                if !result.ok {
                    return Err(summarize_apply_result(
                        "TreeGuard runtime hidden site restore",
                        &result,
                    ));
                }
            }
        }
        sites.insert(site_hash, saved_state.site.clone());

        let restore_sites: HashMap<i64, PlannedSiteUpdate> = saved_state
            .saved_sites
            .iter()
            .map(|(hash, command)| {
                (
                    *hash,
                    PlannedSiteUpdate {
                        queue: current_site_queue(command.as_ref()).unwrap_or(1),
                        parent_site: None,
                        stage_depth: 0,
                        command: Arc::clone(command),
                    },
                )
            })
            .collect();
        let originals_already_live =
            live_restore_snapshots
                .as_ref()
                .is_some_and(|(down_snapshot, up_snapshot)| {
                    site_commands_observed_live(
                        &saved_state.saved_sites,
                        down_snapshot,
                        up_snapshot,
                    )
                });
        if reversible_standby && originals_already_live {
            for (hash, command) in &saved_state.saved_sites {
                sites.insert(*hash, Arc::clone(command));
            }
        } else {
            apply_site_command_updates(
                &config,
                sites,
                &restore_sites,
                "TreeGuard runtime site restore",
            )?;
        }

        let restore_circuits: HashMap<i64, PlannedCircuitUpdate> =
            if saved_state.saved_circuits.is_empty() {
                HashMap::new()
            } else {
                let Some(layout) = mq_layout.as_ref() else {
                    return Err(
                        "Bakery runtime virtualization requires MQ layout to be available"
                            .to_string(),
                    );
                };
                let live_reserved_handles = snapshot_live_qdisc_handle_majors_or_empty(
                    &config,
                    "TreeGuard runtime circuit restore",
                );
                let updates = saved_state
                    .saved_circuits
                    .iter()
                    .map(|(hash, command)| {
                        let refreshed = assign_fresh_qdisc_handles_reserved(
                            command,
                            &config,
                            layout,
                            qdisc_handles,
                            &live_reserved_handles,
                        )?;
                        Ok((
                            *hash,
                            PlannedCircuitUpdate {
                                queue: current_circuit_queue(refreshed.as_ref()).unwrap_or(1),
                                parent_site: None,
                                command: refreshed,
                            },
                        ))
                    })
                    .collect::<Result<HashMap<i64, PlannedCircuitUpdate>, String>>()?;
                apply_circuit_command_updates(
                    &config,
                    sites,
                    circuits,
                    &updates,
                    live_circuits,
                    mq_layout,
                    qdisc_handles,
                    migrations,
                    "TreeGuard runtime circuit restore",
                )?;
                updates
            };

        let restore_active_sites: HashMap<i64, Arc<BakeryCommands>> = restore_sites
            .iter()
            .map(|(hash, update)| (*hash, Arc::clone(&update.command)))
            .collect();
        let restore_active_circuits: HashMap<i64, Arc<BakeryCommands>> = restore_circuits
            .iter()
            .map(|(hash, update)| (*hash, Arc::clone(&update.command)))
            .collect();
        if let Some(restored_state) = build_post_restore_virtualized_state(
            saved_state,
            restore_active_sites,
            restore_active_circuits,
            now_unix,
        ) {
            virtualized_sites.insert(site_hash, restored_state);
        } else {
            virtualized_sites.remove(&site_hash);
        }
        Ok(())
    })();

    if let Err(error) = result {
        *sites = sites_snapshot;
        *circuits = circuits_snapshot;
        *qdisc_handles = qdisc_handles_snapshot;
        *migrations = migrations_snapshot;
        *virtualized_sites = virtualized_sites_snapshot;
        *runtime_node_operations = runtime_ops_snapshot;
        operation.update_status_with_reason(
            RuntimeNodeOperationStatus::Failed,
            unix_now(),
            Some(error.clone()),
            failure_reason,
            None,
        );
        runtime_node_operations.insert(site_hash, operation.clone());
        if runtime_error_suggests_material_desync(&error) {
            mark_reload_required(format!(
                "Bakery detected material runtime drift while processing TreeGuard {} for {}: {}",
                if virtualized {
                    "virtualization"
                } else {
                    "restore"
                },
                runtime_site_label(site_hash, operation.site_name.as_deref()),
                error
            ));
        }
        update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);
        return operation.snapshot();
    }

    let finished_at = unix_now();
    let next_retry = virtualized_sites
        .get(&site_hash)
        .and_then(|state| state.pending_prune.then_some(state.next_prune_attempt_unix));
    let status = virtualized_sites
        .get(&site_hash)
        .map(runtime_status_for_virtualized_state)
        .unwrap_or_else(|| {
            if next_retry.is_some() {
                RuntimeNodeOperationStatus::AppliedAwaitingCleanup
            } else {
                RuntimeNodeOperationStatus::Completed
            }
        });
    if let Some(state) = virtualized_sites.get(&site_hash) {
        debug!(
            "Bakery: final runtime status for site {} action={:?} lifecycle={} pending_prune={} next_retry={:?} computed_status={:?}",
            site_hash,
            action,
            runtime_lifecycle_label(state.lifecycle),
            state.pending_prune,
            next_retry,
            status
        );
    } else {
        debug!(
            "Bakery: final runtime status for site {} action={:?} with no retained state next_retry={:?} computed_status={:?}",
            site_hash, action, next_retry, status
        );
    }
    operation.update_status(status, finished_at, None, next_retry);
    runtime_node_operations.insert(site_hash, operation.clone());
    update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);
    operation.snapshot()
}

#[allow(clippy::too_many_arguments)]
fn full_reload(
    batch: &mut Option<Vec<Arc<BakeryCommands>>>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
    mq_layout: &mut Option<MqDeviceLayout>,
    qdisc_handles: &mut QdiscHandleState,
    config: &Arc<Config>,
    new_batch: Vec<Arc<BakeryCommands>>,
    resolved_mq_layout: Option<MqDeviceLayout>,
    stormguard_overrides: &HashMap<StormguardOverrideKey, u64>,
    virtualized_sites: &mut HashMap<i64, VirtualizedSiteState>,
    runtime_node_operations: &mut HashMap<i64, RuntimeNodeOperation>,
    trigger_summary: String,
) {
    FULL_RELOAD_IN_PROGRESS.store(true, Ordering::Relaxed);
    mark_bakery_action_started(
        BakeryMode::ApplyingFullReload,
        "full_reload_started",
        trigger_summary,
    );
    let _reload_scope = FullReloadScope;
    let previous_sites = sites.clone();
    let previous_circuits = circuits.clone();
    let previous_live_circuits = live_circuits.clone();
    let previous_mq_layout = mq_layout.clone();
    let previous_mq_created = MQ_CREATED.load(Ordering::Relaxed);
    let previous_shaping_tree_active = SHAPING_TREE_ACTIVE.load(Ordering::Relaxed);

    if let Err(error) = prepare_root_mq_for_full_reload(config) {
        let summary = format!("Failed to prepare root mq state before full reload: {error}");
        error!("{summary}");
        mark_bakery_action_finished(BakeryApplyMetrics {
            apply_type: BakeryApplyType::FullReload,
            summary: &summary,
            build_duration_ms: 0,
            apply_duration_ms: 0,
            total_tc_commands: 0,
            class_commands: 0,
            qdisc_commands: 0,
            ok: false,
        });
        *batch = None;
        return;
    }
    invalidate_live_tc_snapshots();
    MQ_CREATED.store(true, Ordering::Relaxed);

    let live_reserved_handles = match snapshot_live_qdisc_handle_majors(config) {
        Ok(handles) => handles,
        Err(error) => {
            let summary =
                format!("Failed to snapshot live qdisc handles before full reload: {error}");
            error!("{summary}");
            mark_bakery_action_finished(BakeryApplyMetrics {
                apply_type: BakeryApplyType::FullReload,
                summary: &summary,
                build_duration_ms: 0,
                apply_duration_ms: 0,
                total_tc_commands: 0,
                class_commands: 0,
                qdisc_commands: 0,
                ok: false,
            });
            *batch = None;
            return;
        }
    };

    let mut working_sites = HashMap::new();
    let mut working_circuits = HashMap::new();
    let mut working_qdisc_handles = QdiscHandleState::default();
    let layout = resolved_mq_layout.clone().unwrap_or_default();
    if resolved_mq_layout.is_none() {
        warn!("Bakery: full reload skipped MQ layout restore because layout is unknown");
    }

    let result = process_batch(
        new_batch,
        config,
        &mut working_sites,
        &mut working_circuits,
        &layout,
        &mut working_qdisc_handles,
        &live_reserved_handles,
    );

    if result.ok {
        *sites = working_sites;
        *circuits = working_circuits;
        live_circuits.clear();
        *qdisc_handles = working_qdisc_handles;
        qdisc_handles.save(config);
        MQ_CREATED.store(true, Ordering::Relaxed);
        SHAPING_TREE_ACTIVE.store(desired_shaping_tree_active(config), Ordering::Relaxed);
        if resolved_mq_layout.is_some() {
            *mq_layout = resolved_mq_layout;
        }
        virtualized_sites.clear();
        runtime_node_operations.clear();
        update_desync_state_from_runtime_state(runtime_node_operations, virtualized_sites);
        clear_reload_required(
            "A successful Bakery full reload re-established baseline state; incremental topology mutations can resume.",
        );
        FIRST_COMMIT_APPLIED.store(true, Ordering::Relaxed);
        apply_stormguard_overrides(stormguard_overrides, config);
    } else {
        *sites = previous_sites;
        *circuits = previous_circuits;
        *live_circuits = previous_live_circuits;
        *mq_layout = previous_mq_layout;
        MQ_CREATED.store(previous_mq_created, Ordering::Relaxed);
        SHAPING_TREE_ACTIVE.store(previous_shaping_tree_active, Ordering::Relaxed);
    }
    if result.ok {
        refresh_live_capacity_snapshot(config, true);
    }
    update_queue_distribution_snapshot(sites, circuits);
    *batch = None;
}

fn process_batch(
    batch: Vec<Arc<BakeryCommands>>,
    config: &Arc<lqos_config::Config>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    mq_layout: &MqDeviceLayout,
    qdisc_handles: &mut QdiscHandleState,
    extra_reserved_handles: &HashMap<String, HashSet<u16>>,
) -> ExecuteResult {
    info!("Bakery: Processing batch of {} commands", batch.len());
    update_bakery_apply_progress(Some("Building tc command batch"), 0, 0, 0, 0);
    let build_started = std::time::Instant::now();
    let mut circuit_count = 0u64;
    let commands = batch
        .into_iter()
        .map(|b| {
            with_assigned_qdisc_handles_reserved(
                &b,
                config,
                mq_layout,
                qdisc_handles,
                extra_reserved_handles,
            )
        })
        .filter_map(|b| {
            // Ensure that our state map is up to date with the latest commands
            match b.as_ref() {
                BakeryCommands::AddSite { site_hash, .. } => {
                    sites.insert(*site_hash, Arc::clone(&b));
                }
                BakeryCommands::AddCircuit { circuit_hash, .. } => {
                    circuits.insert(*circuit_hash, Arc::clone(&b));
                    circuit_count += 1;
                }
                _ => {}
            }
            b.to_commands(config, ExecutionMode::Builder)
        })
        .flatten()
        .collect::<Vec<Vec<String>>>();

    let path = Path::new(&config.lqos_directory).join("linux_tc_rust.txt");
    write_command_file(&path, &commands);
    let build_duration_ms = build_started.elapsed().as_millis() as u64;
    let (total_tc_commands, class_commands, qdisc_commands) = count_tc_command_types(&commands);
    let total_chunks = if total_tc_commands == 0 {
        0
    } else {
        total_tc_commands.div_ceil(FULL_RELOAD_TC_CHUNK_SIZE)
    };
    update_bakery_apply_progress(
        Some("Applying tc command chunks"),
        total_tc_commands,
        0,
        total_chunks,
        0,
    );
    let result = execute_in_memory_chunked(
        &commands,
        "processing batch",
        FULL_RELOAD_TC_CHUNK_SIZE,
        Some(BAKERY_MEMORY_GUARD_MIN_AVAILABLE_BYTES),
        |completed_tc_commands, total_tc_commands, completed_chunks, total_chunks| {
            update_bakery_apply_progress(
                Some("Applying tc command chunks"),
                total_tc_commands,
                completed_tc_commands,
                total_chunks,
                completed_chunks,
            );
        },
    );
    let summary = summarize_apply_result("processing batch", &result);
    maybe_emit_memory_guard_urgent(&summary);
    mark_bakery_action_finished(BakeryApplyMetrics {
        apply_type: BakeryApplyType::FullReload,
        summary: &summary,
        build_duration_ms,
        apply_duration_ms: result.duration_ms,
        total_tc_commands,
        class_commands,
        qdisc_commands,
        ok: result.ok,
    });

    result
}

fn apply_stormguard_overrides(
    overrides: &HashMap<StormguardOverrideKey, u64>,
    config: &Arc<Config>,
) {
    if config.queues.queue_mode.is_observe() {
        push_bakery_event(
            "stormguard_override_replay_skipped",
            "info",
            "Skipping StormGuard HTB override replay because queue_mode is observe.".to_string(),
        );
        return;
    }
    if overrides.is_empty() {
        return;
    }
    let mut commands = Vec::new();
    for (key, rate) in overrides.iter() {
        commands.push(vec![
            "class".to_string(),
            "replace".to_string(),
            "dev".to_string(),
            key.interface.clone(),
            "classid".to_string(),
            key.class.as_tc_string(),
            "htb".to_string(),
            "rate".to_string(),
            format!("{}mbit", rate.saturating_sub(1)),
            "ceil".to_string(),
            format!("{}mbit", rate),
        ]);
    }
    let result = execute_in_memory(&commands, "replaying StormGuard overrides");
    if !result.ok {
        push_bakery_event(
            "stormguard_override_replay_failed",
            "error",
            summarize_apply_result("replaying StormGuard overrides", &result),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_config::Config;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn bakery_test_lock() -> &'static std::sync::Mutex<()> {
        crate::test_state_lock()
    }

    fn install_bakery_test_config() {
        static TEST_CONFIG_PATH: std::sync::OnceLock<std::path::PathBuf> =
            std::sync::OnceLock::new();

        let config_path = TEST_CONFIG_PATH.get_or_init(|| {
            let runtime_dir = std::env::temp_dir().join("lqos_bakery_test_runtime");
            std::fs::create_dir_all(&runtime_dir)
                .expect("create lqos_bakery test runtime directory");

            let config = lqos_config::Config {
                lqos_directory: runtime_dir.display().to_string(),
                bridge: Some(lqos_config::BridgeConfig {
                    use_xdp_bridge: false,
                    to_internet: "lo".to_string(),
                    to_network: "lo".to_string(),
                }),
                ..lqos_config::Config::default()
            };

            let config_path = std::env::temp_dir().join("lqos_bakery_test_lqos.conf");
            let raw = toml::to_string_pretty(&config).expect("serialize lqos_bakery test config");
            std::fs::write(&config_path, raw).expect("write lqos_bakery test config");
            config_path
        });

        // SAFETY: these unit tests call this helper only while holding the bakery test lock,
        // so the process environment is mutated in a serialized way within this test binary.
        unsafe {
            std::env::set_var("LQOS_CONFIG", config_path);
        }
        lqos_config::clear_cached_config();
    }

    fn reset_bakery_test_state() {
        *telemetry_state().write() = BakeryTelemetryState::default();
        MQ_CREATED.store(false, Ordering::Relaxed);
        SHAPING_TREE_ACTIVE.store(false, Ordering::Relaxed);
        FIRST_COMMIT_APPLIED.store(false, Ordering::Relaxed);
        FULL_RELOAD_IN_PROGRESS.store(false, Ordering::Relaxed);
        install_bakery_test_config();
    }

    fn mk_add_circuit(hash: i64, ip_addresses: &str) -> Arc<BakeryCommands> {
        Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: hash,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x1),
            up_parent_class_id: TcHandle::from_u32(0x2),
            class_minor: 0x10,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: ip_addresses.to_string(),
            sqm_override: None,
        })
    }

    fn mk_add_site(
        site_hash: i64,
        parent_class_id: u32,
        up_parent_class_id: u32,
        class_minor: u16,
    ) -> Arc<BakeryCommands> {
        Arc::new(BakeryCommands::AddSite {
            site_hash,
            parent_class_id: TcHandle::from_u32(parent_class_id),
            up_parent_class_id: TcHandle::from_u32(up_parent_class_id),
            class_minor,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
        })
    }

    fn test_config_with_runtime_dir(name: &str) -> Arc<Config> {
        let mut cfg = Config::default();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("lqos-bakery-{name}-{ts}"));
        std::fs::create_dir_all(&dir).expect("temp runtime dir");
        cfg.lqos_directory = dir.display().to_string();
        Arc::new(cfg)
    }

    fn mk_test_circuit(
        circuit_hash: i64,
        parent_class_id: u32,
        up_parent_class_id: u32,
        class_minor: u16,
        class_major: u16,
        up_class_major: u16,
        ip_addresses: &str,
    ) -> Arc<BakeryCommands> {
        Arc::new(BakeryCommands::AddCircuit {
            circuit_hash,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(parent_class_id),
            up_parent_class_id: TcHandle::from_u32(up_parent_class_id),
            class_minor,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major,
            up_class_major,
            down_qdisc_handle: Some(0x9000),
            up_qdisc_handle: Some(0x9001),
            ip_addresses: ip_addresses.to_string(),
            sqm_override: None,
        })
    }

    fn mk_runtime_operation(
        operation_id: u64,
        site_hash: i64,
        action: RuntimeNodeOperationAction,
        status: RuntimeNodeOperationStatus,
        attempt_count: u32,
        next_retry_at_unix: Option<u64>,
    ) -> RuntimeNodeOperation {
        let now = unix_now();
        let mut operation = RuntimeNodeOperation::new(operation_id, site_hash, None, action, now);
        operation.attempt_count = attempt_count;
        operation.update_status(status, now, None, next_retry_at_unix);
        operation
    }

    fn live_class_entry(class_id: u32, parent: Option<u32>) -> LiveTcClassEntry {
        LiveTcClassEntry {
            class_id: TcHandle::from_u32(class_id),
            parent: parent.map(TcHandle::from_u32),
            leaf_qdisc_major: None,
        }
    }

    fn live_qdisc_entry(
        kind: &str,
        handle: Option<u32>,
        parent: Option<u32>,
        is_root: bool,
    ) -> LiveTcQdiscEntry {
        LiveTcQdiscEntry {
            kind: kind.to_string(),
            handle: handle.map(TcHandle::from_u32),
            parent: parent.map(TcHandle::from_u32),
            is_root,
        }
    }

    fn has_hash(batch: &[Arc<BakeryCommands>], hash: i64) -> bool {
        batch.iter().any(|cmd| {
            matches!(
                cmd.as_ref(),
                BakeryCommands::AddCircuit {
                    circuit_hash,
                    ..
                } if *circuit_hash == hash
            )
        })
    }

    #[test]
    fn mapped_circuit_predicate_works() {
        let mapped = mk_add_circuit(1, "192.0.2.1/32");
        let unmapped = mk_add_circuit(2, "");
        assert!(is_mapped_add_circuit(mapped.as_ref()));
        assert!(!is_mapped_add_circuit(unmapped.as_ref()));
    }

    #[test]
    fn mapped_circuit_limit_preserves_existing_first() {
        let mut existing = HashMap::new();
        let mut batch = Vec::new();

        for h in 1..=1000 {
            let c = mk_add_circuit(h, &format!("10.0.{}.1/32", h % 255));
            existing.insert(h, Arc::clone(&c));
            batch.push(c);
        }
        batch.push(mk_add_circuit(1001, "10.0.0.200/32"));

        let (filtered, stats) = filter_batch_by_mapped_circuit_limit(
            batch,
            &existing,
            Some(DEFAULT_MAPPED_CIRCUITS_LIMIT),
        );
        assert_eq!(stats.enforced_limit, Some(DEFAULT_MAPPED_CIRCUITS_LIMIT));
        assert_eq!(stats.requested_mapped, 1001);
        assert_eq!(stats.allowed_mapped, 1000);
        assert_eq!(stats.dropped_mapped, 1);
        assert_eq!(filtered.len(), 1000);
        assert!(!has_hash(&filtered, 1001));
        assert!(has_hash(&filtered, 1));
        assert!(has_hash(&filtered, 1000));
    }

    #[test]
    fn unmapped_additions_are_not_limited() {
        let mut existing = HashMap::new();
        let mut batch = Vec::new();

        for h in 1..=1000 {
            let c = mk_add_circuit(h, &format!("10.1.{}.1/32", h % 255));
            existing.insert(h, c);
            batch.push(mk_add_circuit(h, &format!("10.1.{}.1/32", h % 255)));
        }
        batch.push(mk_add_circuit(9001, ""));

        let (filtered, stats) = filter_batch_by_mapped_circuit_limit(
            batch,
            &existing,
            Some(DEFAULT_MAPPED_CIRCUITS_LIMIT),
        );
        assert_eq!(stats.enforced_limit, Some(DEFAULT_MAPPED_CIRCUITS_LIMIT));
        assert_eq!(stats.requested_mapped, 1000);
        assert_eq!(stats.allowed_mapped, 1000);
        assert_eq!(stats.dropped_mapped, 0);
        assert!(has_hash(&filtered, 9001));
        assert_eq!(filtered.len(), 1001);
    }

    #[test]
    fn runtime_virtualization_overlay_hides_virtualized_site_and_reparents_children() {
        let parent_site = mk_add_site(10, 0x10001, 0x20001, 0x20);
        let virtualized_site = mk_add_site(20, 0x10020, 0x20020, 0x21);
        let child_site = mk_add_site(30, 0x10021, 0x20021, 0x22);
        let child_circuit = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 40,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10021),
            up_parent_class_id: TcHandle::from_u32(0x20021),
            class_minor: 0x30,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x110,
            up_class_major: 0x210,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.40/32".to_string(),
            sqm_override: None,
        });

        let mut virtualized_sites = HashMap::new();
        virtualized_sites.insert(
            20,
            VirtualizedSiteState {
                site_name: "test-site-20".to_string(),
                site: virtualized_site.clone(),
                saved_sites: HashMap::new(),
                saved_circuits: HashMap::new(),
                active_sites: HashMap::from([(
                    30,
                    Arc::new(BakeryCommands::AddSite {
                        site_hash: 30,
                        parent_class_id: TcHandle::from_u32(0x10020),
                        up_parent_class_id: TcHandle::from_u32(0x20020),
                        class_minor: 0x22,
                        download_bandwidth_min: 50.0,
                        upload_bandwidth_min: 50.0,
                        download_bandwidth_max: 500.0,
                        upload_bandwidth_max: 500.0,
                    }),
                )]),
                active_circuits: HashMap::from([(
                    40,
                    Arc::new(BakeryCommands::AddCircuit {
                        circuit_hash: 40,
                        circuit_name: None,
                        site_name: None,
                        parent_class_id: TcHandle::from_u32(0x10020),
                        up_parent_class_id: TcHandle::from_u32(0x20020),
                        class_minor: 0x30,
                        download_bandwidth_min: 10.0,
                        upload_bandwidth_min: 10.0,
                        download_bandwidth_max: 100.0,
                        upload_bandwidth_max: 100.0,
                        class_major: 0x110,
                        up_class_major: 0x210,
                        down_qdisc_handle: None,
                        up_qdisc_handle: None,
                        ip_addresses: "192.0.2.40/32".to_string(),
                        sqm_override: None,
                    }),
                )]),
                prune_sites: HashMap::new(),
                prune_circuits: HashMap::new(),
                qdisc_handles: VirtualizedSiteQdiscHandles {
                    down: None,
                    up: None,
                },
                active_branch: RuntimeVirtualizedActiveBranch::Shadow,
                lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
                pending_prune: false,
                next_prune_attempt_unix: 0,
            },
        );

        let overlaid = apply_runtime_virtualization_overlay(
            vec![parent_site, virtualized_site, child_site, child_circuit],
            &virtualized_sites,
        );

        assert_eq!(overlaid.len(), 3);
        assert!(!overlaid.iter().any(|cmd| matches!(
            cmd.as_ref(),
            BakeryCommands::AddSite { site_hash, .. } if *site_hash == 20
        )));

        let child_site = overlaid
            .iter()
            .find_map(|cmd| match cmd.as_ref() {
                BakeryCommands::AddSite {
                    site_hash,
                    parent_class_id,
                    up_parent_class_id,
                    ..
                } if *site_hash == 30 => Some((*parent_class_id, *up_parent_class_id)),
                _ => None,
            })
            .expect("child site should remain in batch");
        assert_eq!(child_site.0, TcHandle::from_u32(0x10020));
        assert_eq!(child_site.1, TcHandle::from_u32(0x20020));

        let child_circuit = overlaid
            .iter()
            .find_map(|cmd| match cmd.as_ref() {
                BakeryCommands::AddCircuit {
                    circuit_hash,
                    parent_class_id,
                    up_parent_class_id,
                    ..
                } if *circuit_hash == 40 => Some((*parent_class_id, *up_parent_class_id)),
                _ => None,
            })
            .expect("child circuit should remain in batch");
        assert_eq!(child_circuit.0, TcHandle::from_u32(0x10020));
        assert_eq!(child_circuit.1, TcHandle::from_u32(0x20020));
    }

    #[test]
    fn structural_baseline_reconstruction_ignores_hidden_runtime_virtualized_sites() {
        let parent_site = mk_add_site(10, 0x10001, 0x20001, 0x20);
        let virtualized_site = mk_add_site(20, 0x10020, 0x20020, 0x21);
        let child_site = mk_add_site(30, 0x10021, 0x20021, 0x22);
        let child_site_runtime = Arc::new(BakeryCommands::AddSite {
            site_hash: 30,
            parent_class_id: TcHandle::from_u32(0x10020),
            up_parent_class_id: TcHandle::from_u32(0x20020),
            class_minor: 0x22,
            download_bandwidth_min: 50.0,
            upload_bandwidth_min: 50.0,
            download_bandwidth_max: 500.0,
            upload_bandwidth_max: 500.0,
        });
        let child_circuit =
            mk_test_circuit(40, 0x10021, 0x20021, 0x30, 0x110, 0x210, "192.0.2.40/32");
        let child_circuit_runtime =
            mk_test_circuit(40, 0x10020, 0x20020, 0x30, 0x110, 0x210, "192.0.2.40/32");

        let effective_sites = HashMap::from([
            (10, Arc::clone(&parent_site)),
            (30, Arc::clone(&child_site_runtime)),
        ]);
        let effective_circuits = HashMap::from([(40, Arc::clone(&child_circuit_runtime))]);
        let virtualized_sites = HashMap::from([(
            20,
            VirtualizedSiteState {
                site_name: "test-site-20".to_string(),
                site: Arc::clone(&virtualized_site),
                saved_sites: HashMap::from([(30, Arc::clone(&child_site))]),
                saved_circuits: HashMap::from([(40, Arc::clone(&child_circuit))]),
                active_sites: HashMap::from([(30, Arc::clone(&child_site_runtime))]),
                active_circuits: HashMap::from([(40, Arc::clone(&child_circuit_runtime))]),
                prune_sites: HashMap::new(),
                prune_circuits: HashMap::new(),
                qdisc_handles: VirtualizedSiteQdiscHandles {
                    down: None,
                    up: None,
                },
                active_branch: RuntimeVirtualizedActiveBranch::Shadow,
                lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
                pending_prune: true,
                next_prune_attempt_unix: 0,
            },
        )]);

        let raw_batch = vec![
            Arc::clone(&parent_site),
            Arc::clone(&virtualized_site),
            Arc::clone(&child_site),
            Arc::clone(&child_circuit),
        ];

        let (baseline_sites, baseline_circuits) = reconstruct_structural_baseline_state(
            &effective_sites,
            &effective_circuits,
            &virtualized_sites,
        );

        assert!(matches!(
            diff_sites(&raw_batch, &effective_sites),
            SiteDiffResult::RebuildRequired { .. }
        ));
        assert!(matches!(
            diff_sites(&raw_batch, &baseline_sites),
            SiteDiffResult::NoChange
        ));
        assert!(matches!(
            baseline_circuits
                .get(&40)
                .expect("baseline circuit restored")
                .as_ref(),
            BakeryCommands::AddCircuit {
                circuit_hash,
                parent_class_id,
                up_parent_class_id,
                ..
            } if *circuit_hash == 40
                && *parent_class_id == TcHandle::from_u32(0x10021)
                && *up_parent_class_id == TcHandle::from_u32(0x20021)
        ));
    }

    #[test]
    fn runtime_virtualized_site_prune_ready_respects_backoff_and_migrations() {
        let state = VirtualizedSiteState {
            site_name: "test-site-20".to_string(),
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::from([(40, mk_add_circuit(40, "192.0.2.40/32"))]),
            prune_sites: HashMap::new(),
            prune_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
            active_branch: RuntimeVirtualizedActiveBranch::Shadow,
            lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
            pending_prune: true,
            next_prune_attempt_unix: 120,
        };

        let mut migrations = HashMap::new();
        migrations.insert(
            40,
            Migration {
                circuit_hash: 40,
                circuit_name: None,
                site_name: None,
                old_class_major: 0x100,
                old_up_class_major: 0x200,
                old_down_qdisc_handle: None,
                old_up_qdisc_handle: None,
                parent_class_id: TcHandle::from_u32(0x1),
                up_parent_class_id: TcHandle::from_u32(0x2),
                class_major: 0x100,
                up_class_major: 0x200,
                down_qdisc_handle: None,
                up_qdisc_handle: None,
                old_down_min: 1.0,
                old_down_max: 1.0,
                old_up_min: 1.0,
                old_up_max: 1.0,
                new_down_min: 1.0,
                new_down_max: 1.0,
                new_up_min: 1.0,
                new_up_max: 1.0,
                old_minor: 0x10,
                shadow_minor: 0x31,
                final_minor: 0x10,
                ips: vec!["192.0.2.40/32".to_string()],
                sqm_override: None,
                desired_cmd: mk_add_circuit(40, "192.0.2.40/32"),
                stage: MigrationStage::PrepareShadow,
                shadow_verify_attempts: 0,
                final_verify_attempts: 0,
            },
        );

        assert!(!runtime_virtualized_site_prune_ready(
            &state,
            &migrations,
            119
        ));
        assert!(!runtime_virtualized_site_prune_ready(
            &state,
            &migrations,
            120
        ));

        migrations.clear();
        assert!(!runtime_virtualized_site_prune_ready(
            &state,
            &migrations,
            119
        ));
        assert!(runtime_virtualized_site_prune_ready(
            &state,
            &migrations,
            120
        ));
    }

    #[test]
    fn ordered_prune_site_hashes_runs_children_before_parents() {
        let parent = mk_add_site(30, 0x10021, 0x20021, 0x22);
        let child = mk_add_site(31, 0x10022, 0x20022, 0x23);
        let ordered = ordered_prune_site_hashes(&HashMap::from([(30, parent), (31, child)]));
        assert_eq!(ordered, vec![31, 30]);
    }

    #[test]
    fn site_prune_commands_delete_qdisc_by_handle_before_class() {
        let config = Arc::new(lqos_config::Config::default());
        let state = VirtualizedSiteState {
            site_name: "test-site-20".to_string(),
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            prune_sites: HashMap::new(),
            prune_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: Some(0x9000),
                up: Some(0x9001),
            },
            active_branch: RuntimeVirtualizedActiveBranch::Shadow,
            lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
            pending_prune: true,
            next_prune_attempt_unix: 0,
        };
        let commands = site_prune_commands(&config, &state).expect("site prune commands");

        assert_eq!(commands.len(), 4);
        assert_eq!(commands[0][0], "qdisc");
        assert_eq!(commands[0][1], "del");
        assert_eq!(commands[0][4], "handle");
        assert_eq!(commands[0][5], "0x9000:");
        assert_eq!(commands[1][0], "qdisc");
        assert_eq!(commands[1][1], "del");
        assert_eq!(commands[1][4], "handle");
        assert_eq!(commands[1][5], "0x9001:");
        assert_eq!(commands[2][0], "class");
        assert_eq!(commands[2][1], "del");
        assert_eq!(commands[3][0], "class");
        assert_eq!(commands[3][1], "del");
    }

    #[test]
    fn site_prune_commands_skip_qdisc_delete_without_tracked_handles() {
        let config = Arc::new(lqos_config::Config::default());
        let state = VirtualizedSiteState {
            site_name: "test-site-20".to_string(),
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            prune_sites: HashMap::new(),
            prune_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
            active_branch: RuntimeVirtualizedActiveBranch::Shadow,
            lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
            pending_prune: true,
            next_prune_attempt_unix: 0,
        };

        let commands = site_prune_commands(&config, &state).expect("site prune commands");
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0][0], "class");
        assert_eq!(commands[1][0], "class");
    }

    #[test]
    fn observed_circuit_prune_commands_only_delete_existing_old_state() {
        let config = Arc::new(lqos_config::Config::default());
        let circuit = mk_test_circuit(9300, 0x10020, 0x20020, 0x21, 0x1, 0x2, "192.0.2.94/32");

        let mut down_snapshot = HashMap::new();
        down_snapshot.insert(
            TcHandle::from_u32(0x10021),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x10021),
                parent: Some(TcHandle::from_u32(0x10020)),
                leaf_qdisc_major: Some(0x9000),
            },
        );
        let up_snapshot = HashMap::new();

        let commands = observed_circuit_prune_commands(
            &config,
            circuit.as_ref(),
            Some(&down_snapshot),
            Some(&up_snapshot),
            &HashSet::new(),
            &HashSet::new(),
        )
        .expect("observed circuit prune commands");

        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0][0], "qdisc");
        assert_eq!(commands[0][3], config.isp_interface());
        assert_eq!(commands[1][0], "class");
        assert_eq!(commands[1][3], config.isp_interface());
    }

    #[test]
    fn observed_circuit_prune_commands_skip_missing_old_classes() {
        let config = Arc::new(lqos_config::Config::default());
        let circuit = mk_test_circuit(9301, 0x10020, 0x20020, 0x21, 0x1, 0x2, "192.0.2.95/32");
        let down_snapshot = HashMap::new();
        let up_snapshot = HashMap::new();

        let commands = observed_circuit_prune_commands(
            &config,
            circuit.as_ref(),
            Some(&down_snapshot),
            Some(&up_snapshot),
            &HashSet::new(),
            &HashSet::new(),
        )
        .expect("observed circuit prune commands");

        assert!(commands.is_empty());
    }

    #[test]
    fn observed_circuit_prune_commands_skip_handles_reserved_by_batch_targets() {
        let config = Arc::new(lqos_config::Config::default());
        let circuit = mk_test_circuit(9302, 0x10020, 0x20020, 0x21, 0x1, 0x2, "192.0.2.96/32");

        let mut down_snapshot = HashMap::new();
        down_snapshot.insert(
            TcHandle::from_u32(0x10021),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x10021),
                parent: Some(TcHandle::from_u32(0x10020)),
                leaf_qdisc_major: Some(0x9000),
            },
        );
        let mut up_snapshot = HashMap::new();
        up_snapshot.insert(
            TcHandle::from_u32(0x20021),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x20021),
                parent: Some(TcHandle::from_u32(0x20020)),
                leaf_qdisc_major: Some(0x9001),
            },
        );

        let protected_down = HashSet::from([TcHandle::from_u32(0x10021)]);
        let protected_up = HashSet::from([TcHandle::from_u32(0x20021)]);

        let commands = observed_circuit_prune_commands(
            &config,
            circuit.as_ref(),
            Some(&down_snapshot),
            Some(&up_snapshot),
            &protected_down,
            &protected_up,
        )
        .expect("observed circuit prune commands");

        assert!(commands.is_empty());
    }

    #[test]
    fn sync_runtime_virtualized_site_qdisc_handles_from_live_snapshot_recovers_leaf_handles() {
        let mut state = VirtualizedSiteState {
            site_name: "test-site-20".to_string(),
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            prune_sites: HashMap::new(),
            prune_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
            active_branch: RuntimeVirtualizedActiveBranch::Shadow,
            lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
            pending_prune: true,
            next_prune_attempt_unix: 0,
        };

        let mut down_snapshot = HashMap::new();
        down_snapshot.insert(
            TcHandle::from_u32(0x10021),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x10021),
                parent: Some(TcHandle::from_u32(0x10020)),
                leaf_qdisc_major: Some(0x9000),
            },
        );
        let mut up_snapshot = HashMap::new();
        up_snapshot.insert(
            TcHandle::from_u32(0x20021),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x20021),
                parent: Some(TcHandle::from_u32(0x20020)),
                leaf_qdisc_major: Some(0x9001),
            },
        );

        sync_runtime_virtualized_site_qdisc_handles_from_live_snapshot(
            &mut state,
            &down_snapshot,
            &up_snapshot,
        );

        assert_eq!(state.qdisc_handles.down, Some(0x9000));
        assert_eq!(state.qdisc_handles.up, Some(0x9001));
    }

    #[test]
    fn runtime_virtualized_site_observed_child_gate_blocks_prune() {
        let state = VirtualizedSiteState {
            site_name: "test-site-20".to_string(),
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            prune_sites: HashMap::new(),
            prune_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
            active_branch: RuntimeVirtualizedActiveBranch::Shadow,
            lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
            pending_prune: true,
            next_prune_attempt_unix: 0,
        };

        let mut down_snapshot = HashMap::new();
        down_snapshot.insert(
            TcHandle::from_u32(0x10022),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x10022),
                parent: Some(TcHandle::from_u32(0x10021)),
                leaf_qdisc_major: Some(0x9000),
            },
        );
        let mut up_snapshot = HashMap::new();
        up_snapshot.insert(
            TcHandle::from_u32(0x20022),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x20022),
                parent: Some(TcHandle::from_u32(0x20021)),
                leaf_qdisc_major: Some(0x9001),
            },
        );

        assert!(
            runtime_virtualized_site_has_remaining_observed_child_classes(
                &state,
                &down_snapshot,
                &up_snapshot
            )
        );
    }

    #[test]
    fn site_prune_class_commands_for_observed_state_only_emits_existing_directions() {
        let config = Arc::new(lqos_config::Config::default());
        let site = mk_add_site(20, 0x10020, 0x20020, 0x21);

        let down_only =
            site_prune_class_commands_for_observed_state(&config, site.as_ref(), true, false)
                .expect("down-only commands");
        assert_eq!(down_only.len(), 1);
        assert_eq!(down_only[0][0], "class");
        assert_eq!(down_only[0][3], config.isp_interface());

        let up_only =
            site_prune_class_commands_for_observed_state(&config, site.as_ref(), false, true)
                .expect("up-only commands");
        assert_eq!(up_only.len(), 1);
        assert_eq!(up_only[0][0], "class");
        assert_eq!(up_only[0][3], config.internet_interface());

        assert!(
            site_prune_class_commands_for_observed_state(&config, site.as_ref(), false, false)
                .is_none()
        );
    }

    #[test]
    fn cutover_pending_maps_to_applying_status() {
        let state = VirtualizedSiteState {
            site_name: "test-site-20".to_string(),
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            prune_sites: HashMap::new(),
            prune_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
            active_branch: RuntimeVirtualizedActiveBranch::Shadow,
            lifecycle: RuntimeVirtualizedBranchLifecycle::CutoverPending,
            pending_prune: true,
            next_prune_attempt_unix: 0,
        };

        assert_eq!(
            runtime_status_for_virtualized_state(&state),
            RuntimeNodeOperationStatus::Applying
        );
    }

    #[test]
    fn cutover_failure_counts_as_material_desync() {
        assert!(runtime_error_suggests_material_desync(
            "Top-level runtime cutover verification failed to converge: Observed live child classes still attached"
        ));
        assert!(runtime_error_suggests_material_desync(
            "Inactive branch cleanup encountered unexpected live child classes still attached: down [0x4:0xf]"
        ));
    }

    #[test]
    fn stale_retained_runtime_branch_summary_includes_site_name_for_missing_root() {
        let saved_state = VirtualizedSiteState {
            site_name: "7232 Rochester".to_string(),
            site: mk_add_site(20, 0x10000, 0x20000, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            prune_sites: HashMap::new(),
            prune_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
            active_branch: RuntimeVirtualizedActiveBranch::Shadow,
            lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
            pending_prune: false,
            next_prune_attempt_unix: 0,
        };

        let summary = stale_retained_runtime_branch_summary(20, &saved_state, &HashMap::new())
            .expect("stale root summary");

        assert!(summary.contains("7232 Rochester"));
        assert!(summary.contains("20"));
        assert!(summary.contains("no longer present in current topology"));
    }

    #[test]
    fn top_level_virtualization_plan_promotes_children_and_direct_circuits() {
        let target_site = mk_add_site(20, 0x10000, 0x20000, 0x21);
        let sibling_site = mk_add_site(10, 0x10000, 0x20000, 0x20);
        let child_site = mk_add_site(30, 0x10021, 0x20021, 0x22);
        let grandchild_site = mk_add_site(31, 0x10022, 0x20022, 0x23);
        let direct_circuit = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 40,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10021),
            up_parent_class_id: TcHandle::from_u32(0x20021),
            class_minor: 0x30,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x1,
            up_class_major: 0x2,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.40/32".to_string(),
            sqm_override: None,
        });

        let mut sites = HashMap::new();
        sites.insert(10, sibling_site);
        sites.insert(20, Arc::clone(&target_site));
        sites.insert(30, child_site);
        sites.insert(31, grandchild_site);

        let mut circuits = HashMap::new();
        circuits.insert(40, direct_circuit);

        let plan = build_top_level_virtualization_plan(
            Arc::clone(&target_site),
            &sites,
            &circuits,
            None,
            &HashMap::new(),
            1,
        )
        .expect("top-level plan should build");

        assert!(plan.saved_sites.contains_key(&30));
        assert!(plan.saved_sites.contains_key(&31));
        assert!(plan.saved_circuits.contains_key(&40));
        assert!(
            plan.active_sites
                .keys()
                .all(|hash| plan.saved_sites.contains_key(hash))
        );

        let promoted_site = plan
            .active_sites
            .get(&30)
            .expect("promoted child site should be rewritten");
        let BakeryCommands::AddSite {
            parent_class_id,
            up_parent_class_id,
            ..
        } = promoted_site.command.as_ref()
        else {
            panic!("expected AddSite");
        };
        assert_eq!(parent_class_id.get_major_minor().1, 0);
        assert_eq!(up_parent_class_id.get_major_minor().1, 0);

        let promoted_circuit = plan
            .active_circuits
            .get(&40)
            .expect("direct circuit should be rewritten");
        let BakeryCommands::AddCircuit {
            parent_class_id,
            up_parent_class_id,
            ..
        } = promoted_circuit.command.as_ref()
        else {
            panic!("expected AddCircuit");
        };
        assert_eq!(parent_class_id.get_major_minor().1, 0);
        assert_eq!(up_parent_class_id.get_major_minor().1, 0);
        assert_eq!(plan.site_stages, vec![vec![30], vec![31]]);
        assert_eq!(
            plan.active_sites.get(&30).expect("child site").stage_depth,
            0
        );
        assert_eq!(
            plan.active_sites
                .get(&31)
                .expect("grandchild site")
                .stage_depth,
            1
        );
    }

    #[test]
    fn non_top_level_virtualization_plan_uses_shadow_minor_unique_within_major_domain() {
        let parent_site = mk_add_site(10, 0x10003, 0x20003, 0x2002);
        let target_site = mk_add_site(20, 0x10003, 0x20003, 0x2003);
        let child_site = mk_add_site(30, 0x12003, 0x22003, 0x2000);

        let mut sites = HashMap::new();
        sites.insert(10, parent_site);
        sites.insert(20, Arc::clone(&target_site));
        sites.insert(30, Arc::clone(&child_site));

        let circuits = HashMap::new();

        let plan =
            build_non_top_level_virtualization_plan(Arc::clone(&target_site), &sites, &circuits)
                .expect("non-top-level plan should build");

        let rewritten_child = plan
            .active_sites
            .get(&30)
            .expect("child site should be shadowed");
        let BakeryCommands::AddSite {
            parent_class_id,
            up_parent_class_id,
            class_minor,
            ..
        } = rewritten_child.command.as_ref()
        else {
            panic!("expected AddSite");
        };

        assert_eq!(*parent_class_id, TcHandle::from_u32(0x10003));
        assert_eq!(*up_parent_class_id, TcHandle::from_u32(0x20003));
        assert_ne!(
            *class_minor, 0x2000,
            "shadow child site minor must not reuse an existing classid in the same major domain"
        );
        assert_eq!(plan.site_stages, vec![vec![30]]);
    }

    #[test]
    fn non_top_level_virtualization_plan_stages_shadow_sites_by_depth() {
        let parent_site = mk_add_site(10, 0x10003, 0x20003, 0x2002);
        let target_site = mk_add_site(20, 0x10003, 0x20003, 0x2003);
        let child_site = mk_add_site(30, 0x12003, 0x22003, 0x2000);
        let grandchild_site = mk_add_site(40, 0x12000, 0x22000, 0x2001);

        let mut sites = HashMap::new();
        sites.insert(10, parent_site);
        sites.insert(20, Arc::clone(&target_site));
        sites.insert(30, Arc::clone(&child_site));
        sites.insert(40, Arc::clone(&grandchild_site));

        let circuits = HashMap::new();

        let plan =
            build_non_top_level_virtualization_plan(Arc::clone(&target_site), &sites, &circuits)
                .expect("non-top-level plan should build");

        assert_eq!(plan.site_stages, vec![vec![30], vec![40]]);
        assert_eq!(
            plan.active_sites.get(&30).expect("child site").stage_depth,
            0
        );
        assert_eq!(
            plan.active_sites
                .get(&40)
                .expect("grandchild site")
                .stage_depth,
            1
        );
    }

    #[test]
    fn site_runtime_virtualization_rejects_top_level_sites() {
        let site = mk_add_site(99, 0x10000, 0x20000, 0x21);
        let reason = site_runtime_virtualization_eligibility_error(site.as_ref())
            .expect("top-level site should be rejected");
        assert!(reason.contains("top-level"));
    }

    #[test]
    fn site_runtime_virtualization_rejects_queue_root_child_sites_as_top_level() {
        let site = mk_add_site(99, 0x10003, 0x20003, 0x21);
        let reason = site_runtime_virtualization_eligibility_error(site.as_ref())
            .expect("queue-root child site should be rejected as top-level");
        assert!(reason.contains("top-level"));
    }

    #[test]
    fn site_runtime_virtualization_accepts_normal_same_queue_site() {
        let site = mk_add_site(77, 0x10020, 0x20020, 0x22);
        assert!(site_runtime_virtualization_eligibility_error(site.as_ref()).is_none());
    }

    #[test]
    fn nested_runtime_shadow_branch_rejects_non_top_level_virtualization() {
        let site = mk_add_site(77, 0x10003, 0x20003, 0x2001);
        let reason = nested_runtime_shadow_branch_eligibility_error(site.as_ref())
            .expect("runtime shadow site should be rejected");
        assert!(reason.message.contains("retained runtime shadow branch"));
        assert_eq!(
            reason.failure_reason,
            Some(RuntimeNodeOperationFailureReason::StructuralIneligibleNestedRuntimeBranch)
        );
    }

    #[test]
    fn site_runtime_virtualization_rejects_non_site_commands() {
        let circuit = mk_add_circuit(77, "192.0.2.77/32");
        let reason = site_runtime_virtualization_eligibility_error(circuit.as_ref())
            .expect("non-site commands should be rejected");
        assert!(reason.contains("AddSite command"));
    }

    #[test]
    fn top_level_runtime_virtualization_rejects_single_promotable_child() {
        let target_site = mk_add_site(20, 0x10000, 0x20000, 0x21);
        let only_child = mk_add_site(30, 0x10021, 0x20021, 0x22);

        let mut sites = HashMap::new();
        sites.insert(20, Arc::clone(&target_site));
        sites.insert(30, only_child);

        let reason = top_level_runtime_virtualization_eligibility_error(
            20,
            target_site.as_ref(),
            &sites,
            &HashMap::new(),
        )
        .expect("single-child top-level virtualization should be rejected");

        assert!(reason.message.contains("only one promotable direct child"));
        assert_eq!(
            reason.failure_reason,
            Some(RuntimeNodeOperationFailureReason::StructuralIneligibleSinglePromotableChild)
        );
    }

    #[test]
    fn runtime_node_capacity_returns_deferred_status() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let mut runtime_node_operations = HashMap::new();
        for index in 0..RUNTIME_NODE_OPERATION_CAPACITY {
            let site_hash = index as i64 + 1;
            let mut operation = RuntimeNodeOperation::new(
                site_hash as u64,
                site_hash,
                None,
                RuntimeNodeOperationAction::Virtualize,
                unix_now(),
            );
            operation.attempt_count = 1;
            operation.update_status(RuntimeNodeOperationStatus::Applying, unix_now(), None, None);
            runtime_node_operations.insert(site_hash, operation);
        }
        update_desync_state_from_runtime_state(&runtime_node_operations, &HashMap::new());

        let snapshot = handle_treeguard_set_node_virtual_live(
            999,
            true,
            &mut HashMap::new(),
            &mut HashMap::new(),
            &HashMap::new(),
            &None,
            &mut QdiscHandleState::default(),
            &mut HashMap::new(),
            &mut HashMap::new(),
            &mut runtime_node_operations,
            &mut 10_000,
        );

        assert_eq!(snapshot.status, RuntimeNodeOperationStatus::Deferred);
        assert!(snapshot.next_retry_at_unix.is_some());
        assert!(
            snapshot
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("capacity")
        );

        let telemetry = bakery_status_snapshot();
        assert_eq!(telemetry.runtime_operations.deferred_count, 1);
        assert!(!telemetry.reload_required);
    }

    #[test]
    fn top_level_runtime_operations_are_serialized() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let site_a = mk_add_site(20, 0x10000, 0x20000, 0x21);
        let site_b = mk_add_site(21, 0x110000, 0x120000, 0x22);
        let mut sites = HashMap::from([(20, site_a), (21, site_b)]);

        let mut runtime_node_operations = HashMap::new();
        let mut operation = RuntimeNodeOperation::new(
            1,
            20,
            None,
            RuntimeNodeOperationAction::Virtualize,
            unix_now(),
        );
        operation.attempt_count = 1;
        operation.update_status(RuntimeNodeOperationStatus::Applying, unix_now(), None, None);
        runtime_node_operations.insert(20, operation);
        update_desync_state_from_runtime_state(&runtime_node_operations, &HashMap::new());

        let snapshot = handle_treeguard_set_node_virtual_live(
            21,
            true,
            &mut sites,
            &mut HashMap::new(),
            &HashMap::new(),
            &None,
            &mut QdiscHandleState::default(),
            &mut HashMap::new(),
            &mut HashMap::new(),
            &mut runtime_node_operations,
            &mut 10_000,
        );

        assert_eq!(snapshot.status, RuntimeNodeOperationStatus::Deferred);
        assert!(snapshot.next_retry_at_unix.is_some());
        assert!(
            snapshot
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("one top-level TreeGuard runtime operation in flight")
        );
    }

    #[test]
    fn structural_runtime_failures_are_counted_as_blocked_not_failed() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let now = unix_now();
        let mut retryable = RuntimeNodeOperation::new(
            1,
            20,
            Some("retryable-site".to_string()),
            RuntimeNodeOperationAction::Virtualize,
            now,
        );
        retryable.update_status(
            RuntimeNodeOperationStatus::Failed,
            now,
            Some("transient runtime failure".to_string()),
            None,
        );

        let mut blocked = RuntimeNodeOperation::new(
            2,
            21,
            Some("blocked-site".to_string()),
            RuntimeNodeOperationAction::Virtualize,
            now.saturating_add(1),
        );
        blocked.update_status_with_reason(
            RuntimeNodeOperationStatus::Failed,
            now.saturating_add(1),
            Some("inside a retained runtime shadow branch".to_string()),
            Some(RuntimeNodeOperationFailureReason::StructuralIneligibleNestedRuntimeBranch),
            None,
        );

        let runtime_node_operations = HashMap::from([(20, retryable), (21, blocked)]);
        let snapshot = rebuild_runtime_operations_snapshot(&runtime_node_operations);

        assert_eq!(snapshot.failed_count, 1);
        assert_eq!(snapshot.blocked_count, 1);
        assert_eq!(
            snapshot
                .latest
                .as_ref()
                .and_then(|entry| entry.site_name.as_deref()),
            Some("blocked-site")
        );
    }

    #[test]
    fn non_top_level_runtime_virtualization_enters_cutover_pending_without_pruning_standby() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();
        SHAPING_TREE_ACTIVE.store(true, Ordering::Relaxed);

        let parent_site = mk_add_site(10, 0x10003, 0x20003, 0x20);
        let target_site = mk_add_site(20, 0x10020, 0x20020, 0x21);
        let mut sites = HashMap::from([(10, parent_site), (20, target_site)]);
        let mut circuits = HashMap::new();
        let mut qdisc_handles = QdiscHandleState::default();
        let mut migrations = HashMap::new();
        let mut virtualized_sites = HashMap::new();
        let mut runtime_node_operations = HashMap::new();
        let mut next_runtime_operation_id = 10_000;

        let snapshot = handle_treeguard_set_node_virtual_live(
            20,
            true,
            &mut sites,
            &mut circuits,
            &HashMap::new(),
            &None,
            &mut qdisc_handles,
            &mut migrations,
            &mut virtualized_sites,
            &mut runtime_node_operations,
            &mut next_runtime_operation_id,
        );

        assert_eq!(snapshot.status, RuntimeNodeOperationStatus::Applying);
        let state = virtualized_sites
            .get(&20)
            .expect("non-top-level virtualized site state");
        assert_eq!(
            state.lifecycle,
            RuntimeVirtualizedBranchLifecycle::CutoverPending
        );
        assert_eq!(state.active_branch, RuntimeVirtualizedActiveBranch::Shadow);
        assert!(state.pending_prune);
        assert!(state.prune_sites.is_empty());
        assert!(state.prune_circuits.is_empty());
    }

    #[test]
    fn restore_with_pending_cleanup_is_not_reported_completed() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();
        SHAPING_TREE_ACTIVE.store(true, Ordering::Relaxed);

        let site_hash = 20;
        let standby_site = mk_add_site(site_hash, 0x10000, 0x20000, 0x21);
        let shadow_child = mk_add_site(30, 0x10020, 0x20020, 0x22);
        let mut virtualized_sites = HashMap::from([(
            site_hash,
            VirtualizedSiteState {
                site_name: "test-site-20".to_string(),
                site: Arc::clone(&standby_site),
                saved_sites: HashMap::new(),
                saved_circuits: HashMap::new(),
                active_sites: HashMap::from([(30, shadow_child)]),
                active_circuits: HashMap::new(),
                prune_sites: HashMap::new(),
                prune_circuits: HashMap::new(),
                qdisc_handles: VirtualizedSiteQdiscHandles {
                    down: None,
                    up: None,
                },
                active_branch: RuntimeVirtualizedActiveBranch::Shadow,
                lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
                pending_prune: true,
                next_prune_attempt_unix: unix_now(),
            },
        )]);

        let snapshot = handle_treeguard_set_node_virtual_live(
            site_hash,
            false,
            &mut HashMap::new(),
            &mut HashMap::new(),
            &HashMap::new(),
            &None,
            &mut QdiscHandleState::default(),
            &mut HashMap::new(),
            &mut virtualized_sites,
            &mut HashMap::new(),
            &mut 10_000,
        );

        assert_eq!(
            snapshot.status,
            RuntimeNodeOperationStatus::AppliedAwaitingCleanup
        );
        let state = virtualized_sites
            .get(&site_hash)
            .expect("restored virtualized site state");
        assert_eq!(
            state.active_branch,
            RuntimeVirtualizedActiveBranch::Original
        );
        assert!(!state.active_branch_hides_original_site());
        assert!(state.pending_prune);
    }

    #[test]
    fn deferred_restore_cleanup_completes_and_removes_retained_state() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let config = Arc::new(Config::default());
        let site_hash = 20;
        let site = mk_add_site(site_hash, 0x10020, 0x20020, 0x21);
        let mut virtualized_sites = HashMap::from([(
            site_hash,
            VirtualizedSiteState {
                site_name: "test-site-20".to_string(),
                site,
                saved_sites: HashMap::new(),
                saved_circuits: HashMap::new(),
                active_sites: HashMap::new(),
                active_circuits: HashMap::new(),
                prune_sites: HashMap::new(),
                prune_circuits: HashMap::new(),
                qdisc_handles: VirtualizedSiteQdiscHandles {
                    down: None,
                    up: None,
                },
                active_branch: RuntimeVirtualizedActiveBranch::Original,
                lifecycle: RuntimeVirtualizedBranchLifecycle::PhysicalActiveCleanupPending,
                pending_prune: true,
                next_prune_attempt_unix: 0,
            },
        )]);
        let mut runtime_node_operations = HashMap::from([(
            site_hash,
            mk_runtime_operation(
                1,
                site_hash,
                RuntimeNodeOperationAction::Restore,
                RuntimeNodeOperationStatus::AppliedAwaitingCleanup,
                1,
                Some(0),
            ),
        )]);

        flush_deferred_runtime_site_prunes_with_snapshots(
            &config,
            &mut virtualized_sites,
            &HashMap::new(),
            &mut runtime_node_operations,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert!(!virtualized_sites.contains_key(&site_hash));
        let operation = runtime_node_operations
            .get(&site_hash)
            .expect("runtime operation");
        assert_eq!(operation.status, RuntimeNodeOperationStatus::Completed);
        assert_eq!(operation.next_retry_at_unix, None);

        let telemetry = bakery_status_snapshot();
        assert_eq!(telemetry.runtime_operations.awaiting_cleanup_count, 0);
        assert_eq!(
            bakery_runtime_node_operation_snapshot(site_hash)
                .expect("runtime op snapshot")
                .status,
            RuntimeNodeOperationStatus::Completed
        );
    }

    #[test]
    fn cutover_pending_converges_to_completed_when_shadow_is_verified_active() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let config = Arc::new(Config::default());
        let site_hash = 20;
        let active_site = mk_add_site(30, 0x10020, 0x20020, 0x22);
        let active_circuit =
            mk_test_circuit(40, 0x10020, 0x20020, 0x30, 0x110, 0x210, "192.0.2.40/32");
        let mut virtualized_sites = HashMap::from([(
            site_hash,
            VirtualizedSiteState {
                site_name: "test-site-20".to_string(),
                site: mk_add_site(site_hash, 0x10000, 0x20000, 0x21),
                saved_sites: HashMap::new(),
                saved_circuits: HashMap::new(),
                active_sites: HashMap::from([(30, Arc::clone(&active_site))]),
                active_circuits: HashMap::from([(40, Arc::clone(&active_circuit))]),
                prune_sites: HashMap::new(),
                prune_circuits: HashMap::new(),
                qdisc_handles: VirtualizedSiteQdiscHandles {
                    down: None,
                    up: None,
                },
                active_branch: RuntimeVirtualizedActiveBranch::Shadow,
                lifecycle: RuntimeVirtualizedBranchLifecycle::CutoverPending,
                pending_prune: true,
                next_prune_attempt_unix: 0,
            },
        )]);
        let mut runtime_node_operations = HashMap::from([(
            site_hash,
            mk_runtime_operation(
                2,
                site_hash,
                RuntimeNodeOperationAction::Virtualize,
                RuntimeNodeOperationStatus::Applying,
                1,
                Some(0),
            ),
        )]);
        let down_snapshot = HashMap::from([
            (
                TcHandle::from_u32(0x10022),
                live_class_entry(0x10022, Some(0x10020)),
            ),
            (
                tc_handle_from_major_minor(0x110, 0x30),
                LiveTcClassEntry {
                    class_id: tc_handle_from_major_minor(0x110, 0x30),
                    parent: Some(TcHandle::from_u32(0x10020)),
                    leaf_qdisc_major: Some(0x9000),
                },
            ),
        ]);
        let up_snapshot = HashMap::from([
            (
                TcHandle::from_u32(0x20022),
                live_class_entry(0x20022, Some(0x20020)),
            ),
            (
                tc_handle_from_major_minor(0x210, 0x30),
                LiveTcClassEntry {
                    class_id: tc_handle_from_major_minor(0x210, 0x30),
                    parent: Some(TcHandle::from_u32(0x20020)),
                    leaf_qdisc_major: Some(0x9001),
                },
            ),
        ]);

        flush_deferred_runtime_site_prunes_with_snapshots(
            &config,
            &mut virtualized_sites,
            &HashMap::new(),
            &mut runtime_node_operations,
            &down_snapshot,
            &up_snapshot,
        );

        let state = virtualized_sites
            .get(&site_hash)
            .expect("retained shadow runtime state");
        assert_eq!(
            state.lifecycle,
            RuntimeVirtualizedBranchLifecycle::FlattenedActive
        );
        assert!(!state.pending_prune);
        let operation = runtime_node_operations
            .get(&site_hash)
            .expect("runtime operation");
        assert_eq!(operation.status, RuntimeNodeOperationStatus::Completed);
    }

    #[test]
    fn restore_without_shadow_remainders_completes_immediately() {
        let saved_state = VirtualizedSiteState {
            site_name: "test-site-20".to_string(),
            site: mk_add_site(20, 0x10000, 0x20000, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            prune_sites: HashMap::new(),
            prune_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
            active_branch: RuntimeVirtualizedActiveBranch::Shadow,
            lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
            pending_prune: false,
            next_prune_attempt_unix: 0,
        };

        assert!(
            build_post_restore_virtualized_state(saved_state, HashMap::new(), HashMap::new(), 0)
                .is_none()
        );
    }

    #[test]
    fn restore_cleanup_retry_keeps_operation_in_applied_awaiting_cleanup() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let config = Arc::new(Config::default());
        let site_hash = 20;
        let prune_site = mk_add_site(site_hash, 0x10020, 0x20020, 0x21);
        let mut virtualized_sites = HashMap::from([(
            site_hash,
            VirtualizedSiteState {
                site_name: "test-site-20".to_string(),
                site: mk_add_site(site_hash, 0x10000, 0x20000, 0x21),
                saved_sites: HashMap::new(),
                saved_circuits: HashMap::new(),
                active_sites: HashMap::new(),
                active_circuits: HashMap::new(),
                prune_sites: HashMap::from([(site_hash, Arc::clone(&prune_site))]),
                prune_circuits: HashMap::new(),
                qdisc_handles: VirtualizedSiteQdiscHandles {
                    down: None,
                    up: None,
                },
                active_branch: RuntimeVirtualizedActiveBranch::Original,
                lifecycle: RuntimeVirtualizedBranchLifecycle::PhysicalActiveCleanupPending,
                pending_prune: true,
                next_prune_attempt_unix: 0,
            },
        )]);
        let mut runtime_node_operations = HashMap::from([(
            site_hash,
            mk_runtime_operation(
                3,
                site_hash,
                RuntimeNodeOperationAction::Restore,
                RuntimeNodeOperationStatus::AppliedAwaitingCleanup,
                1,
                Some(0),
            ),
        )]);
        let down_snapshot = HashMap::from([(
            TcHandle::from_u32(0x10022),
            live_class_entry(0x10022, Some(0x10021)),
        )]);
        let up_snapshot = HashMap::from([(
            TcHandle::from_u32(0x20022),
            live_class_entry(0x20022, Some(0x20021)),
        )]);

        flush_deferred_runtime_site_prunes_with_snapshots(
            &config,
            &mut virtualized_sites,
            &HashMap::new(),
            &mut runtime_node_operations,
            &down_snapshot,
            &up_snapshot,
        );

        let state = virtualized_sites
            .get(&site_hash)
            .expect("virtualized site state");
        assert!(state.pending_prune);
        let operation = runtime_node_operations
            .get(&site_hash)
            .expect("runtime operation");
        assert_eq!(
            operation.status,
            RuntimeNodeOperationStatus::AppliedAwaitingCleanup
        );
        assert!(operation.next_retry_at_unix.is_some());
        assert!(operation.last_error.is_some());
    }

    #[test]
    fn restore_cleanup_failure_eventually_marks_dirty() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let config = Arc::new(Config::default());
        let site_hash = 20;
        let prune_site = mk_add_site(site_hash, 0x10020, 0x20020, 0x21);
        let mut virtualized_sites = HashMap::from([(
            site_hash,
            VirtualizedSiteState {
                site_name: "test-site-20".to_string(),
                site: mk_add_site(site_hash, 0x10000, 0x20000, 0x21),
                saved_sites: HashMap::new(),
                saved_circuits: HashMap::new(),
                active_sites: HashMap::new(),
                active_circuits: HashMap::new(),
                prune_sites: HashMap::from([(site_hash, Arc::clone(&prune_site))]),
                prune_circuits: HashMap::new(),
                qdisc_handles: VirtualizedSiteQdiscHandles {
                    down: None,
                    up: None,
                },
                active_branch: RuntimeVirtualizedActiveBranch::Original,
                lifecycle: RuntimeVirtualizedBranchLifecycle::PhysicalActiveCleanupPending,
                pending_prune: true,
                next_prune_attempt_unix: 0,
            },
        )]);
        let mut runtime_node_operations = HashMap::from([(
            site_hash,
            mk_runtime_operation(
                4,
                site_hash,
                RuntimeNodeOperationAction::Restore,
                RuntimeNodeOperationStatus::AppliedAwaitingCleanup,
                RUNTIME_SITE_PRUNE_MAX_ATTEMPTS - 1,
                Some(0),
            ),
        )]);
        let down_snapshot = HashMap::from([(
            TcHandle::from_u32(0x10022),
            live_class_entry(0x10022, Some(0x10021)),
        )]);
        let up_snapshot = HashMap::from([(
            TcHandle::from_u32(0x20022),
            live_class_entry(0x20022, Some(0x20021)),
        )]);

        flush_deferred_runtime_site_prunes_with_snapshots(
            &config,
            &mut virtualized_sites,
            &HashMap::new(),
            &mut runtime_node_operations,
            &down_snapshot,
            &up_snapshot,
        );

        let state = virtualized_sites
            .get(&site_hash)
            .expect("virtualized site state");
        assert!(!state.pending_prune);
        assert_eq!(state.lifecycle, RuntimeVirtualizedBranchLifecycle::Failed);
        let operation = runtime_node_operations
            .get(&site_hash)
            .expect("runtime operation");
        assert_eq!(operation.status, RuntimeNodeOperationStatus::Dirty);
        assert!(operation.next_retry_at_unix.is_none());
    }

    #[test]
    fn runtime_branch_snapshot_tracks_restore_completion() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let config = Arc::new(Config::default());
        let site_hash = 20;
        let mut virtualized_sites = HashMap::from([(
            site_hash,
            VirtualizedSiteState {
                site_name: "test-site-20".to_string(),
                site: mk_add_site(site_hash, 0x10000, 0x20000, 0x21),
                saved_sites: HashMap::new(),
                saved_circuits: HashMap::new(),
                active_sites: HashMap::new(),
                active_circuits: HashMap::new(),
                prune_sites: HashMap::new(),
                prune_circuits: HashMap::new(),
                qdisc_handles: VirtualizedSiteQdiscHandles {
                    down: None,
                    up: None,
                },
                active_branch: RuntimeVirtualizedActiveBranch::Original,
                lifecycle: RuntimeVirtualizedBranchLifecycle::PhysicalActiveCleanupPending,
                pending_prune: true,
                next_prune_attempt_unix: 0,
            },
        )]);
        let mut runtime_node_operations = HashMap::from([(
            site_hash,
            mk_runtime_operation(
                5,
                site_hash,
                RuntimeNodeOperationAction::Restore,
                RuntimeNodeOperationStatus::AppliedAwaitingCleanup,
                1,
                Some(0),
            ),
        )]);

        update_desync_state_from_runtime_state(&runtime_node_operations, &virtualized_sites);
        let before = bakery_runtime_node_branch_snapshot(site_hash).expect("branch snapshot");
        assert_eq!(before.active_branch, "Original");
        assert!(before.pending_prune);

        flush_deferred_runtime_site_prunes_with_snapshots(
            &config,
            &mut virtualized_sites,
            &HashMap::new(),
            &mut runtime_node_operations,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert!(bakery_runtime_node_branch_snapshot(site_hash).is_none());
        assert_eq!(
            bakery_runtime_node_operation_snapshot(site_hash)
                .expect("runtime operation snapshot")
                .status,
            RuntimeNodeOperationStatus::Completed
        );
    }

    #[test]
    fn deferred_runtime_site_prune_advances_without_pending_migrations() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let cfg = Config {
            bridge: Some(lqos_config::BridgeConfig {
                use_xdp_bridge: false,
                to_internet: "__bakery-missing-wan__".to_string(),
                to_network: "__bakery-missing-lan__".to_string(),
            }),
            ..Config::default()
        };
        let config = Arc::new(cfg);

        let site_hash = 20;
        let site = mk_add_site(site_hash, 0x10020, 0x20020, 0x21);
        let mut virtualized_sites = HashMap::from([(
            site_hash,
            VirtualizedSiteState {
                site_name: "test-site-20".to_string(),
                site,
                saved_sites: HashMap::new(),
                saved_circuits: HashMap::new(),
                active_sites: HashMap::new(),
                active_circuits: HashMap::new(),
                prune_sites: HashMap::new(),
                prune_circuits: HashMap::new(),
                qdisc_handles: VirtualizedSiteQdiscHandles {
                    down: None,
                    up: None,
                },
                active_branch: RuntimeVirtualizedActiveBranch::Shadow,
                lifecycle: RuntimeVirtualizedBranchLifecycle::FlattenedActive,
                pending_prune: true,
                next_prune_attempt_unix: 0,
            },
        )]);
        let mut runtime_node_operations = HashMap::new();
        let submitted_at = unix_now().saturating_sub(5);
        let mut operation = RuntimeNodeOperation::new(
            1,
            site_hash,
            None,
            RuntimeNodeOperationAction::Virtualize,
            submitted_at,
        );
        operation.attempt_count = 1;
        operation.update_status(
            RuntimeNodeOperationStatus::AppliedAwaitingCleanup,
            submitted_at,
            None,
            Some(0),
        );
        runtime_node_operations.insert(site_hash, operation);

        flush_deferred_runtime_site_prunes(
            &config,
            &mut virtualized_sites,
            &HashMap::new(),
            &mut runtime_node_operations,
        );

        let state = virtualized_sites
            .get(&site_hash)
            .expect("virtualized site state");
        let operation = runtime_node_operations
            .get(&site_hash)
            .expect("runtime operation");

        assert!(state.pending_prune);
        assert_eq!(
            operation.status,
            RuntimeNodeOperationStatus::AppliedAwaitingCleanup
        );
        assert!(operation.attempt_count >= 1);
        assert!(operation.updated_at_unix >= submitted_at);
        assert_eq!(
            operation.next_retry_at_unix,
            Some(state.next_prune_attempt_unix)
        );
        assert!(operation.next_retry_at_unix.expect("retry time") >= operation.updated_at_unix);
        assert!(
            operation
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("Failed to snapshot live classes on __bakery-missing-lan__")
        );
    }

    #[test]
    fn inactive_branch_prune_fails_fast_on_unexpected_live_child_classes() {
        let config = Arc::new(lqos_config::Config::default());
        let prune_root = mk_add_site(20, 0x10020, 0x20020, 0x21);

        let mut state = VirtualizedSiteState {
            site_name: "test-site-20".to_string(),
            site: Arc::clone(&prune_root),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            prune_sites: HashMap::from([(20, Arc::clone(&prune_root))]),
            prune_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
            active_branch: RuntimeVirtualizedActiveBranch::Original,
            lifecycle: RuntimeVirtualizedBranchLifecycle::PhysicalActiveCleanupPending,
            pending_prune: true,
            next_prune_attempt_unix: 0,
        };

        let mut down_snapshot = HashMap::new();
        down_snapshot.insert(
            TcHandle::from_u32(0x10022),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x10022),
                parent: Some(TcHandle::from_u32(0x10021)),
                leaf_qdisc_major: None,
            },
        );
        let mut up_snapshot = HashMap::new();
        up_snapshot.insert(
            TcHandle::from_u32(0x20022),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x20022),
                parent: Some(TcHandle::from_u32(0x20021)),
                leaf_qdisc_major: None,
            },
        );

        let result = execute_runtime_virtualized_subtree_prune(
            &config,
            &mut state,
            &down_snapshot,
            &up_snapshot,
        );

        match result {
            RuntimePrunePassResult::Failed(summary) => {
                assert!(summary.contains("unexpected live child classes"));
            }
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn dirty_runtime_subtrees_below_threshold_do_not_require_full_reload() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let mut runtime_node_operations = HashMap::new();
        for index in 0..(RUNTIME_DIRTY_SUBTREE_RELOAD_THRESHOLD - 1) {
            let site_hash = index as i64 + 1;
            let mut operation = RuntimeNodeOperation::new(
                site_hash as u64,
                site_hash,
                None,
                RuntimeNodeOperationAction::Virtualize,
                unix_now(),
            );
            operation.attempt_count = 2;
            operation.update_status(
                RuntimeNodeOperationStatus::Dirty,
                unix_now(),
                Some("test dirty subtree".to_string()),
                None,
            );
            runtime_node_operations.insert(site_hash, operation);
        }

        update_desync_state_from_runtime_state(&runtime_node_operations, &HashMap::new());

        let telemetry = bakery_status_snapshot();
        assert_eq!(
            telemetry.dirty_subtree_count,
            RUNTIME_DIRTY_SUBTREE_RELOAD_THRESHOLD - 1
        );
        assert!(!telemetry.reload_required);
        assert!(bakery_reload_required_reason().is_none());
    }

    #[test]
    fn dirty_runtime_subtrees_at_threshold_require_full_reload() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let mut runtime_node_operations = HashMap::new();
        for index in 0..RUNTIME_DIRTY_SUBTREE_RELOAD_THRESHOLD {
            let site_hash = index as i64 + 1;
            let mut operation = RuntimeNodeOperation::new(
                site_hash as u64,
                site_hash,
                None,
                RuntimeNodeOperationAction::Virtualize,
                unix_now(),
            );
            operation.attempt_count = 3;
            operation.update_status(
                RuntimeNodeOperationStatus::Dirty,
                unix_now(),
                Some("test dirty subtree".to_string()),
                None,
            );
            runtime_node_operations.insert(site_hash, operation);
        }

        update_desync_state_from_runtime_state(&runtime_node_operations, &HashMap::new());

        let telemetry = bakery_status_snapshot();
        assert_eq!(
            telemetry.dirty_subtree_count,
            RUNTIME_DIRTY_SUBTREE_RELOAD_THRESHOLD
        );
        assert!(telemetry.reload_required);
        assert!(
            telemetry
                .reload_required_reason
                .as_deref()
                .unwrap_or_default()
                .contains("dirty runtime subtree operations")
        );
        assert!(
            bakery_reload_required_reason()
                .unwrap_or_default()
                .contains("dirty runtime subtree operations")
        );
    }

    #[test]
    fn mapped_circuit_limit_uses_custom_limit() {
        let existing = HashMap::new();
        let batch = vec![
            mk_add_circuit(1, "10.2.0.1/32"),
            mk_add_circuit(2, "10.2.0.2/32"),
            mk_add_circuit(3, "10.2.0.3/32"),
        ];

        let (filtered, stats) = filter_batch_by_mapped_circuit_limit(batch, &existing, Some(2));
        assert_eq!(stats.enforced_limit, Some(2));
        assert_eq!(stats.requested_mapped, 3);
        assert_eq!(stats.allowed_mapped, 2);
        assert_eq!(stats.dropped_mapped, 1);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn mapped_circuit_limit_none_is_unlimited() {
        let existing = HashMap::new();
        let batch = vec![
            mk_add_circuit(1, "10.3.0.1/32"),
            mk_add_circuit(2, "10.3.0.2/32"),
            mk_add_circuit(3, "10.3.0.3/32"),
        ];

        let (filtered, stats) = filter_batch_by_mapped_circuit_limit(batch, &existing, None);
        assert_eq!(stats.enforced_limit, None);
        assert_eq!(stats.requested_mapped, 3);
        assert_eq!(stats.allowed_mapped, 3);
        assert_eq!(stats.dropped_mapped, 0);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn qdisc_budget_estimate_counts_total_qdiscs_per_interface() {
        let _guard = bakery_test_lock().lock().expect("lock");
        reset_bakery_test_state();
        let config = Arc::new(Config::default());
        let queue = vec![
            BakeryCommands::StartBatch,
            BakeryCommands::MqSetup {
                queues_available: 2,
                stick_offset: 0,
            },
            BakeryCommands::AddSite {
                site_hash: 1,
                parent_class_id: TcHandle::from_u32(0x1),
                up_parent_class_id: TcHandle::from_u32(0x2),
                class_minor: 0x10,
                download_bandwidth_min: 50.0,
                upload_bandwidth_min: 50.0,
                download_bandwidth_max: 100.0,
                upload_bandwidth_max: 100.0,
            },
            BakeryCommands::AddCircuit {
                circuit_hash: 2,
                circuit_name: None,
                site_name: None,
                parent_class_id: TcHandle::from_u32(0x10001),
                up_parent_class_id: TcHandle::from_u32(0x20001),
                class_minor: 0x20,
                download_bandwidth_min: 10.0,
                upload_bandwidth_min: 10.0,
                download_bandwidth_max: 100.0,
                upload_bandwidth_max: 100.0,
                class_major: 0x100,
                up_class_major: 0x200,
                down_qdisc_handle: Some(0x9000),
                up_qdisc_handle: Some(0x9001),
                ip_addresses: "192.0.2.1/32".to_string(),
                sqm_override: None,
            },
            BakeryCommands::CommitBatch,
        ];

        let estimate = estimate_full_reload_auto_qdisc_budget(&config, &queue);
        assert_eq!(estimate.interfaces.get("eth1"), Some(&8));
        assert_eq!(estimate.interfaces.get("eth0"), Some(&8));
        assert_eq!(
            estimate
                .interface_details
                .get("eth0")
                .map(|d| d.cake_qdiscs),
            Some(1)
        );
        assert_eq!(
            estimate
                .interface_details
                .get("eth1")
                .map(|d| d.cake_qdiscs),
            Some(1)
        );
        assert_eq!(
            estimate
                .interface_details
                .get("eth0")
                .map(|d| d.infra_qdiscs),
            Some(7)
        );
        assert_eq!(
            estimate
                .interface_details
                .get("eth1")
                .map(|d| d.infra_qdiscs),
            Some(7)
        );
        assert!(estimate.ok());
    }

    #[test]
    fn qdisc_budget_estimate_counts_only_root_mq_in_observe_mode() {
        let _guard = bakery_test_lock().lock().expect("lock");
        reset_bakery_test_state();
        let mut cfg = Config::default();
        cfg.queues.set_queue_mode(lqos_config::QueueMode::Observe);
        let config = Arc::new(cfg);
        let queue = vec![
            BakeryCommands::MqSetup {
                queues_available: 1,
                stick_offset: 0,
            },
            BakeryCommands::AddCircuit {
                circuit_hash: 2,
                circuit_name: None,
                site_name: None,
                parent_class_id: TcHandle::from_u32(0x10001),
                up_parent_class_id: TcHandle::from_u32(0x20001),
                class_minor: 0x20,
                download_bandwidth_min: 10.0,
                upload_bandwidth_min: 10.0,
                download_bandwidth_max: 100.0,
                upload_bandwidth_max: 100.0,
                class_major: 0x100,
                up_class_major: 0x200,
                down_qdisc_handle: Some(0x9000),
                up_qdisc_handle: Some(0x9001),
                ip_addresses: "192.0.2.1/32".to_string(),
                sqm_override: None,
            },
        ];

        let estimate = estimate_full_reload_auto_qdisc_budget(&config, &queue);
        assert_eq!(estimate.interfaces.get("eth1"), Some(&1));
        assert_eq!(estimate.interfaces.get("eth0"), Some(&1));
        assert_eq!(
            estimate
                .interface_details
                .get("eth0")
                .map(|d| d.infra_qdiscs),
            Some(1)
        );
        assert_eq!(
            estimate
                .interface_details
                .get("eth1")
                .map(|d| d.infra_qdiscs),
            Some(1)
        );
        assert_eq!(
            estimate
                .interface_details
                .get("eth0")
                .map(|d| d.cake_qdiscs),
            Some(0)
        );
        assert_eq!(
            estimate
                .interface_details
                .get("eth1")
                .map(|d| d.cake_qdiscs),
            Some(0)
        );
    }

    #[test]
    fn live_tree_mutation_blocker_reports_observe_mode_and_inactive_tree() {
        let _guard = bakery_test_lock().lock().expect("lock");
        reset_bakery_test_state();

        let mut observe_cfg = Config::default();
        observe_cfg
            .queues
            .set_queue_mode(lqos_config::QueueMode::Observe);
        let observe_cfg = Arc::new(observe_cfg);
        assert_eq!(
            live_tree_mutation_blocker_for_config(&observe_cfg),
            Some(
                "queue_mode is observe; root MQ is retained but the shaping tree is not live"
                    .to_string()
            )
        );

        let active_cfg = Arc::new(Config::default());
        assert_eq!(
            live_tree_mutation_blocker_for_config(&active_cfg),
            Some("the shaping tree is not currently active".to_string())
        );

        SHAPING_TREE_ACTIVE.store(true, Ordering::Relaxed);
        assert_eq!(live_tree_mutation_blocker_for_config(&active_cfg), None);
    }

    #[test]
    fn live_tree_mutation_blocker_reports_full_reload_in_progress() {
        let _guard = bakery_test_lock().lock().expect("lock");
        reset_bakery_test_state();

        let active_cfg = Arc::new(Config::default());
        SHAPING_TREE_ACTIVE.store(true, Ordering::Relaxed);
        FULL_RELOAD_IN_PROGRESS.store(true, Ordering::Relaxed);

        assert_eq!(
            live_tree_mutation_blocker_for_config(&active_cfg),
            Some("a full reload is currently in progress".to_string())
        );
        reset_bakery_test_state();
    }

    #[test]
    fn verify_clean_root_child_tree_accepts_empty_retained_root() {
        let qdisc_snapshot = vec![live_qdisc_entry("mq", Some(0x7fff0000), None, true)];
        let class_snapshot = HashMap::new();
        assert!(verify_clean_root_child_tree(&qdisc_snapshot, &class_snapshot, "eth0").is_ok());
    }

    #[test]
    fn verify_clean_root_child_tree_accepts_kernel_default_root_children() {
        let qdisc_snapshot = vec![
            live_qdisc_entry("mq", Some(0x7fff0000), None, true),
            live_qdisc_entry("fq_codel", Some(0), Some(0x7fff0001), false),
        ];
        let class_snapshot = HashMap::new();
        assert!(verify_clean_root_child_tree(&qdisc_snapshot, &class_snapshot, "eth0").is_ok());
    }

    #[test]
    fn verify_clean_root_child_tree_accepts_kernel_default_mq_classes() {
        let qdisc_snapshot = vec![live_qdisc_entry("mq", Some(0x7fff0000), None, true)];
        let class_snapshot = HashMap::from([(
            TcHandle::from_u32(0x7fff0001),
            live_class_entry(0x7fff0001, None),
        )]);
        assert!(verify_clean_root_child_tree(&qdisc_snapshot, &class_snapshot, "eth0").is_ok());
    }

    #[test]
    fn verify_clean_root_child_tree_rejects_lingering_managed_root_children() {
        let qdisc_snapshot = vec![
            live_qdisc_entry("mq", Some(0x7fff0000), None, true),
            live_qdisc_entry("htb", Some(0x00010000), Some(0x7fff0001), false),
        ];
        let class_snapshot = HashMap::new();
        let err = verify_clean_root_child_tree(&qdisc_snapshot, &class_snapshot, "eth0")
            .expect_err("child qdisc should fail retained-root verification");
        assert!(err.contains("still has 1 managed child qdisc"));
    }

    #[test]
    fn verify_clean_root_child_tree_rejects_lingering_classes() {
        let qdisc_snapshot = vec![live_qdisc_entry("mq", Some(0x7fff0000), None, true)];
        let class_snapshot = HashMap::from([(
            TcHandle::from_u32(0x10001),
            live_class_entry(0x10001, Some(0x10000)),
        )]);
        let err = verify_clean_root_child_tree(&qdisc_snapshot, &class_snapshot, "eth0")
            .expect_err("lingering classes should fail retained-root verification");
        assert!(err.contains("still has 1 managed tc class"));
    }

    #[test]
    fn managed_root_child_parent_handles_ignore_kernel_default_qdiscs() {
        let qdisc_snapshot = vec![
            live_qdisc_entry("mq", Some(0x7fff0000), None, true),
            live_qdisc_entry("fq_codel", Some(0), Some(0x7fff0001), false),
        ];
        assert!(managed_root_child_parent_handles(&qdisc_snapshot).is_empty());

        let managed_snapshot = vec![
            live_qdisc_entry("mq", Some(0x7fff0000), None, true),
            live_qdisc_entry("htb", Some(0x00010000), Some(0x7fff0001), false),
        ];
        assert_eq!(
            managed_root_child_parent_handles(&managed_snapshot),
            HashSet::from([TcHandle::from_u32(0x7fff0001)])
        );
    }

    #[test]
    fn grouped_runtime_site_events_retain_human_site_name() {
        let _guard = bakery_test_lock().lock().expect("lock");
        reset_bakery_test_state();

        let site_hash = 3571466403324592518i64;
        let site_name = "JR_AP_TR_F".to_string();
        let mut grouped = GroupedBakeryEventLimiter::default();
        grouped.emit_with_site_name(
            "runtime_cutover_completed|completed",
            "runtime_cutover_completed",
            "info",
            Some((site_hash, Some(site_name.clone()))),
            format!(
                "Runtime cutover completed for site {}; shadow branch is active and original branch is standby.",
                runtime_site_display_name(site_hash, Some(site_name.as_str()))
            ),
            "runtime cutover completion events".to_string(),
        );
        grouped.flush();

        let activity = bakery_activity_snapshot();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].site_hash, Some(site_hash));
        assert_eq!(activity[0].site_name.as_deref(), Some(site_name.as_str()));
        assert!(activity[0].summary.contains(site_name.as_str()));
        assert!(!activity[0].summary.contains(&format!("site {site_hash}")));
    }

    #[test]
    fn qdisc_budget_estimate_counts_fq_codel_leaf_qdiscs_separately() {
        let config = Arc::new(Config::default());
        let queue = vec![
            BakeryCommands::MqSetup {
                queues_available: 1,
                stick_offset: 0,
            },
            BakeryCommands::AddCircuit {
                circuit_hash: 2,
                circuit_name: None,
                site_name: None,
                parent_class_id: TcHandle::from_u32(0x10001),
                up_parent_class_id: TcHandle::from_u32(0x20001),
                class_minor: 0x20,
                download_bandwidth_min: 10.0,
                upload_bandwidth_min: 10.0,
                download_bandwidth_max: 2_000.0,
                upload_bandwidth_max: 2_000.0,
                class_major: 0x100,
                up_class_major: 0x200,
                down_qdisc_handle: Some(0x9000),
                up_qdisc_handle: Some(0x9001),
                ip_addresses: "192.0.2.1/32".to_string(),
                sqm_override: Some("fq_codel".to_string()),
            },
        ];

        let estimate = estimate_full_reload_auto_qdisc_budget(&config, &queue);
        assert_eq!(
            estimate
                .interface_details
                .get("eth0")
                .map(|d| d.fq_codel_qdiscs),
            Some(1)
        );
        assert_eq!(
            estimate
                .interface_details
                .get("eth1")
                .map(|d| d.fq_codel_qdiscs),
            Some(1)
        );
        assert_eq!(
            estimate
                .interface_details
                .get("eth0")
                .map(|d| d.cake_qdiscs),
            Some(0)
        );
    }

    #[test]
    fn kind_switch_rotates_only_changed_direction_and_reuses_old_handle() {
        let config = test_config_with_runtime_dir("kind-switch");
        let layout = MqDeviceLayout::from_setup(&config, 2, 0);
        let mut handles = QdiscHandleState::default();

        let original = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 101,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.1/32".to_string(),
            sqm_override: None,
        });

        let original = with_assigned_qdisc_handles(&original, &config, &layout, &mut handles);
        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(original_down),
            up_qdisc_handle: Some(original_up),
            ..
        } = original.as_ref()
        else {
            panic!("expected assigned handles");
        };

        handles.save(&config);
        let mut reloaded = QdiscHandleState::load(&config);

        let switched = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 101,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.1/32".to_string(),
            sqm_override: Some("fq_codel/cake".to_string()),
        });

        let switched = with_assigned_qdisc_handles(&switched, &config, &layout, &mut reloaded);
        let rotated = rotate_changed_qdisc_handles(
            original.as_ref(),
            &switched,
            &config,
            &layout,
            &mut reloaded,
        );

        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(rotated_down),
            up_qdisc_handle: Some(rotated_up),
            ..
        } = rotated.as_ref()
        else {
            panic!("expected rotated handles");
        };

        assert_ne!(*rotated_down, *original_down);
        assert_eq!(*rotated_up, *original_up);

        reloaded.save(&config);
        let mut persisted = QdiscHandleState::load(&config);
        let new_circuit = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 202,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x21,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.2/32".to_string(),
            sqm_override: None,
        });
        let new_circuit =
            with_assigned_qdisc_handles(&new_circuit, &config, &layout, &mut persisted);

        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(new_down),
            ..
        } = new_circuit.as_ref()
        else {
            panic!("expected new circuit handle");
        };

        assert_eq!(*new_down, *original_down);
    }

    #[test]
    fn parent_change_rotates_only_changed_direction_and_reuses_old_handle() {
        let config = test_config_with_runtime_dir("parent-change");
        let layout = MqDeviceLayout::from_setup(&config, 2, 0);
        let mut handles = QdiscHandleState::default();

        let original = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 303,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.3/32".to_string(),
            sqm_override: None,
        });

        let original = with_assigned_qdisc_handles(&original, &config, &layout, &mut handles);
        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(original_down),
            up_qdisc_handle: Some(original_up),
            ..
        } = original.as_ref()
        else {
            panic!("expected assigned handles");
        };

        handles.save(&config);
        let mut reloaded = QdiscHandleState::load(&config);

        let moved = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 303,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10002),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x101,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.3/32".to_string(),
            sqm_override: None,
        });

        let moved = with_assigned_qdisc_handles(&moved, &config, &layout, &mut reloaded);
        let rotated = rotate_changed_qdisc_handles(
            original.as_ref(),
            &moved,
            &config,
            &layout,
            &mut reloaded,
        );

        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(rotated_down),
            up_qdisc_handle: Some(rotated_up),
            ..
        } = rotated.as_ref()
        else {
            panic!("expected rotated handles");
        };

        assert_ne!(*rotated_down, *original_down);
        assert_eq!(*rotated_up, *original_up);

        reloaded.save(&config);
        let mut persisted = QdiscHandleState::load(&config);
        let new_circuit = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 404,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x21,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.4/32".to_string(),
            sqm_override: None,
        });
        let new_circuit =
            with_assigned_qdisc_handles(&new_circuit, &config, &layout, &mut persisted);

        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(new_down),
            ..
        } = new_circuit.as_ref()
        else {
            panic!("expected new circuit handle");
        };
        assert_eq!(*new_down, *original_down);
    }

    #[test]
    fn parent_change_rotation_avoids_live_reserved_handles() {
        let config = test_config_with_runtime_dir("parent-change-live-reserved");
        let layout = MqDeviceLayout::from_setup(&config, 2, 0);
        let mut handles = QdiscHandleState::default();

        let original = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 313,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.13/32".to_string(),
            sqm_override: None,
        });

        let original = with_assigned_qdisc_handles(&original, &config, &layout, &mut handles);
        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(original_down),
            up_qdisc_handle: Some(original_up),
            ..
        } = original.as_ref()
        else {
            panic!("expected assigned handles");
        };

        handles.save(&config);
        let mut reloaded = QdiscHandleState::load(&config);

        let moved = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 313,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10002),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x101,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.13/32".to_string(),
            sqm_override: None,
        });

        let live_reserved = HashMap::from([
            (config.isp_interface(), HashSet::from([*original_down + 1])),
            (config.internet_interface(), HashSet::from([*original_up])),
        ]);
        let moved = with_assigned_qdisc_handles_reserved(
            &moved,
            &config,
            &layout,
            &mut reloaded,
            &live_reserved,
        );
        let rotated = rotate_changed_qdisc_handles_reserved(
            original.as_ref(),
            &moved,
            &config,
            &layout,
            &mut reloaded,
            &live_reserved,
        );

        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(rotated_down),
            ..
        } = rotated.as_ref()
        else {
            panic!("expected rotated handles");
        };

        assert_ne!(*rotated_down, *original_down);
        assert_ne!(*rotated_down, *original_down + 1);
    }

    #[test]
    fn restore_rotation_avoids_live_reserved_prefilled_handle_without_parent_change() {
        let config = test_config_with_runtime_dir("restore-live-reserved-prefilled-handle");
        let layout = MqDeviceLayout::from_setup(&config, 2, 0);
        let mut handles = QdiscHandleState::default();

        let original = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 414,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.14/32".to_string(),
            sqm_override: None,
        });

        let original = with_assigned_qdisc_handles(&original, &config, &layout, &mut handles);
        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(original_down),
            up_qdisc_handle: Some(original_up),
            ..
        } = original.as_ref()
        else {
            panic!("expected assigned handles");
        };

        handles.save(&config);
        let mut reloaded = QdiscHandleState::load(&config);
        let conflicting_down = *original_down + 1;
        let conflicting_up = *original_up + 1;

        let restore = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 414,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: Some(conflicting_down),
            up_qdisc_handle: Some(conflicting_up),
            ip_addresses: "192.0.2.14/32".to_string(),
            sqm_override: None,
        });

        let live_reserved = HashMap::from([
            (config.isp_interface(), HashSet::from([conflicting_down])),
            (config.internet_interface(), HashSet::from([conflicting_up])),
        ]);

        let rotated = rotate_changed_qdisc_handles_reserved(
            original.as_ref(),
            &restore,
            &config,
            &layout,
            &mut reloaded,
            &live_reserved,
        );

        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(rotated_down),
            up_qdisc_handle: Some(rotated_up),
            ..
        } = rotated.as_ref()
        else {
            panic!("expected rotated handles");
        };

        assert_eq!(
            *original_down,
            previous_down_qdisc_handle(original.as_ref()).expect("old down handle")
        );
        assert_eq!(
            *original_up,
            previous_up_qdisc_handle(original.as_ref()).expect("old up handle")
        );
        assert_ne!(*rotated_down, conflicting_down);
        assert_ne!(*rotated_up, conflicting_up);
    }

    #[test]
    fn restore_assigns_fresh_handles_distinct_from_saved_and_live_shadow_handles() {
        let config = test_config_with_runtime_dir("restore-fresh-cutover-handles");
        let layout = MqDeviceLayout::from_setup(&config, 2, 0);
        let mut handles = QdiscHandleState::default();

        let original = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 415,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.15/32".to_string(),
            sqm_override: None,
        });
        let original = with_assigned_qdisc_handles(&original, &config, &layout, &mut handles);
        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(saved_down),
            up_qdisc_handle: Some(saved_up),
            ..
        } = original.as_ref()
        else {
            panic!("expected saved/original handles");
        };

        let shadow_down = handles
            .rotate_circuit_handle(&config.isp_interface(), 415, &HashSet::new())
            .expect("shadow down handle");
        let shadow_up = handles
            .rotate_circuit_handle(&config.internet_interface(), 415, &HashSet::new())
            .expect("shadow up handle");
        let live_reserved = HashMap::from([
            (config.isp_interface(), HashSet::from([shadow_down])),
            (config.internet_interface(), HashSet::from([shadow_up])),
        ]);

        let restored = assign_fresh_qdisc_handles_reserved(
            &original,
            &config,
            &layout,
            &mut handles,
            &live_reserved,
        )
        .expect("fresh restore handles");

        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(restored_down),
            up_qdisc_handle: Some(restored_up),
            ..
        } = restored.as_ref()
        else {
            panic!("expected restored handles");
        };

        assert_ne!(*restored_down, *saved_down);
        assert_ne!(*restored_up, *saved_up);
        assert_ne!(*restored_down, shadow_down);
        assert_ne!(*restored_up, shadow_up);
    }

    #[test]
    fn queue_live_migration_succeeds_for_active_parent_move() {
        let old_cmd = mk_test_circuit(9001, 0x10020, 0x20020, 0x21, 0x1, 0x2, "192.0.2.90/32");
        let new_cmd = mk_test_circuit(9001, 0x10034, 0x20034, 0x35, 0x3, 0x4, "192.0.2.90/32");

        let mut circuits = HashMap::from([(9001, Arc::clone(&old_cmd))]);
        let sites = HashMap::new();
        let live_circuits = HashMap::from([(9001, 1u64)]);
        let mut migrations = HashMap::new();

        let queued = queue_live_migration(
            old_cmd.as_ref(),
            &new_cmd,
            &sites,
            &mut circuits,
            &live_circuits,
            &mut migrations,
        );

        assert!(queued);
        let migration = migrations.get(&9001).expect("migration should be queued");
        assert_eq!(migration.stage, MigrationStage::PrepareShadow);
        assert_eq!(migration.parent_class_id, TcHandle::from_u32(0x10034));
        assert_eq!(migration.final_minor, 0x35);
        assert_ne!(migration.shadow_minor, migration.old_minor);
        assert!(matches!(
            circuits.get(&9001),
            Some(cmd) if Arc::ptr_eq(cmd, &old_cmd)
        ));
        assert!(Arc::ptr_eq(&migration.desired_cmd, &new_cmd));
    }

    #[test]
    fn queue_live_migration_fails_when_target_parent_has_no_shadow_minor() {
        let old_cmd = mk_test_circuit(9100, 0x10020, 0x20020, 0x21, 0x1, 0x2, "192.0.2.91/32");
        let new_cmd = mk_test_circuit(9100, 0x10034, 0x20034, 0x35, 0x3, 0x4, "192.0.2.91/32");

        let mut circuits = HashMap::from([(9100, Arc::clone(&old_cmd))]);
        let sites = HashMap::new();
        for minor in 1..=0xFFFEu16 {
            circuits.insert(
                100_000 + i64::from(minor),
                mk_test_circuit(
                    100_000 + i64::from(minor),
                    0x10034,
                    0x20034,
                    minor,
                    0x3,
                    0x4,
                    &format!("198.51.100.{}/32", (minor % 250) + 1),
                ),
            );
        }

        let live_circuits = HashMap::from([(9100, 1u64)]);
        let mut migrations = HashMap::new();

        let queued = queue_live_migration(
            old_cmd.as_ref(),
            &new_cmd,
            &sites,
            &mut circuits,
            &live_circuits,
            &mut migrations,
        );

        assert!(!queued);
        assert!(!migrations.contains_key(&9100));
        assert!(matches!(
            circuits.get(&9100),
            Some(cmd) if Arc::ptr_eq(cmd, &old_cmd)
        ));
    }

    #[test]
    fn find_free_circuit_shadow_minor_respects_major_domain_occupancy() {
        let sites = HashMap::from([(7, mk_add_site(7, 0x10001, 0x20001, 0x2001))]);
        let circuits = HashMap::from([(
            42,
            mk_test_circuit(42, 0x10099, 0x20034, 0x2000, 0x1, 0x2, "192.0.2.142/32"),
        )]);

        let chosen = find_free_circuit_shadow_minor(&sites, &circuits, &HashMap::new(), 0x1, 0x2)
            .expect("free minor");

        assert_ne!(chosen, 0x2000);
        assert_ne!(chosen, 0x2001);
        assert_eq!(chosen, 0x2002);
    }

    #[test]
    fn queue_runtime_migration_reserves_shadow_minor_across_pending_migrations() {
        let old_cmd_a = mk_test_circuit(9300, 0x10020, 0x20020, 0x21, 0x1, 0x2, "192.0.2.94/32");
        let new_cmd_a = mk_test_circuit(9300, 0x80034, 0x80034, 0x35, 0x8, 0x8, "192.0.2.94/32");
        let old_cmd_b = mk_test_circuit(9301, 0x10022, 0x20022, 0x23, 0x1, 0x2, "192.0.2.95/32");
        let new_cmd_b = mk_test_circuit(9301, 0x80036, 0x80036, 0x37, 0x8, 0x8, "192.0.2.95/32");

        let mut circuits = HashMap::from([
            (9300, Arc::clone(&old_cmd_a)),
            (9301, Arc::clone(&old_cmd_b)),
        ]);
        let sites = HashMap::new();
        let live_circuits = HashMap::new();
        let mut migrations = HashMap::new();

        assert!(queue_top_level_runtime_migration(
            old_cmd_a.as_ref(),
            &new_cmd_a,
            &sites,
            &mut circuits,
            &live_circuits,
            &mut migrations,
        ));
        assert!(queue_top_level_runtime_migration(
            old_cmd_b.as_ref(),
            &new_cmd_b,
            &sites,
            &mut circuits,
            &live_circuits,
            &mut migrations,
        ));

        let migration_a = migrations.get(&9300).expect("first migration");
        let migration_b = migrations.get(&9301).expect("second migration");
        assert_eq!(migration_a.shadow_minor, 0x2000);
        assert_eq!(migration_b.shadow_minor, 0x2001);
    }

    #[test]
    fn pending_migration_overlay_uses_desired_state_for_diff_only() {
        let old_cmd = mk_test_circuit(9300, 0x10020, 0x20020, 0x21, 0x1, 0x2, "192.0.2.94/32");
        let new_cmd = mk_test_circuit(9300, 0x10034, 0x20034, 0x35, 0x3, 0x4, "192.0.2.94/32");

        let mut circuits = HashMap::from([(9300, Arc::clone(&old_cmd))]);
        let sites = HashMap::new();
        let live_circuits = HashMap::from([(9300, 1u64)]);
        let mut migrations = HashMap::new();

        assert!(queue_live_migration(
            old_cmd.as_ref(),
            &new_cmd,
            &sites,
            &mut circuits,
            &live_circuits,
            &mut migrations,
        ));

        assert!(matches!(
            circuits.get(&9300),
            Some(cmd) if Arc::ptr_eq(cmd, &old_cmd)
        ));

        let overlaid = circuits_with_pending_migration_targets(&circuits, &migrations);
        assert!(matches!(
            overlaid.get(&9300),
            Some(cmd) if Arc::ptr_eq(cmd, &new_cmd)
        ));
    }

    #[test]
    fn migration_stage_failure_marks_reload_required_and_stops_migration() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        clear_reload_required(
            "reset before migration_stage_failure_marks_reload_required_and_stops_migration",
        );
        let mut migration = Migration {
            circuit_hash: 9002,
            circuit_name: None,
            site_name: None,
            old_class_major: 0x1,
            old_up_class_major: 0x2,
            old_down_qdisc_handle: Some(0x9000),
            old_up_qdisc_handle: Some(0x9001),
            parent_class_id: TcHandle::from_u32(0x10034),
            up_parent_class_id: TcHandle::from_u32(0x20034),
            class_major: 0x1,
            up_class_major: 0x2,
            down_qdisc_handle: Some(0x9000),
            up_qdisc_handle: Some(0x9001),
            old_down_min: 1.0,
            old_down_max: 10.0,
            old_up_min: 1.0,
            old_up_max: 10.0,
            new_down_min: 1.0,
            new_down_max: 20.0,
            new_up_min: 1.0,
            new_up_max: 20.0,
            old_minor: 0x21,
            shadow_minor: 0x2000,
            final_minor: 0x35,
            ips: vec!["192.0.2.92/32".to_string()],
            sqm_override: None,
            desired_cmd: mk_test_circuit(9002, 0x10034, 0x20034, 0x35, 0x1, 0x2, "192.0.2.92/32"),
            stage: MigrationStage::BuildFinal,
            shadow_verify_attempts: 0,
            final_verify_attempts: 0,
        };
        let result = ExecuteResult {
            ok: false,
            duration_ms: 1,
            failure_summary: Some("RTNETLINK answers: Invalid argument".to_string()),
        };

        let advanced = migration_stage_apply_succeeded(
            &mut migration,
            "live-move: build final",
            &result,
            MigrationStage::VerifyFinalReady,
        );

        assert!(!advanced);
        assert_eq!(migration.stage, MigrationStage::Done);
        let reason = bakery_reload_required_reason().expect("reload required reason should be set");
        assert!(reason.contains("9002"));
        assert!(reason.contains("live-move: build final"));
        clear_reload_required(
            "reset after migration_stage_failure_marks_reload_required_and_stops_migration",
        );
    }

    #[test]
    fn migration_stage_success_advances_without_reload_required() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        clear_reload_required(
            "reset before migration_stage_success_advances_without_reload_required",
        );
        let mut migration = Migration {
            circuit_hash: 9003,
            circuit_name: None,
            site_name: None,
            old_class_major: 0x1,
            old_up_class_major: 0x2,
            old_down_qdisc_handle: Some(0x9000),
            old_up_qdisc_handle: Some(0x9001),
            parent_class_id: TcHandle::from_u32(0x10034),
            up_parent_class_id: TcHandle::from_u32(0x20034),
            class_major: 0x1,
            up_class_major: 0x2,
            down_qdisc_handle: Some(0x9000),
            up_qdisc_handle: Some(0x9001),
            old_down_min: 1.0,
            old_down_max: 10.0,
            old_up_min: 1.0,
            old_up_max: 10.0,
            new_down_min: 1.0,
            new_down_max: 20.0,
            new_up_min: 1.0,
            new_up_max: 20.0,
            old_minor: 0x21,
            shadow_minor: 0x2000,
            final_minor: 0x35,
            ips: vec!["192.0.2.93/32".to_string()],
            sqm_override: None,
            desired_cmd: mk_test_circuit(9003, 0x10034, 0x20034, 0x35, 0x1, 0x2, "192.0.2.93/32"),
            stage: MigrationStage::BuildFinal,
            shadow_verify_attempts: 0,
            final_verify_attempts: 0,
        };
        let result = ExecuteResult {
            ok: true,
            duration_ms: 1,
            failure_summary: None,
        };

        let advanced = migration_stage_apply_succeeded(
            &mut migration,
            "live-move: build final",
            &result,
            MigrationStage::VerifyFinalReady,
        );

        assert!(advanced);
        assert_eq!(migration.stage, MigrationStage::VerifyFinalReady);
        assert!(bakery_reload_required_reason().is_none());
    }

    fn sample_test_migration(hash: i64) -> Migration {
        Migration {
            circuit_hash: hash,
            circuit_name: Some(format!("circuit-{hash}")),
            site_name: Some(format!("site-{hash}")),
            old_class_major: 0x1,
            old_up_class_major: 0x2,
            old_down_qdisc_handle: Some(0x9000),
            old_up_qdisc_handle: Some(0x9001),
            parent_class_id: TcHandle::from_u32(0x10034),
            up_parent_class_id: TcHandle::from_u32(0x20034),
            class_major: 0x1,
            up_class_major: 0x2,
            down_qdisc_handle: Some(0x9000),
            up_qdisc_handle: Some(0x9001),
            old_down_min: 1.0,
            old_down_max: 10.0,
            old_up_min: 1.0,
            old_up_max: 10.0,
            new_down_min: 1.0,
            new_down_max: 20.0,
            new_up_min: 1.0,
            new_up_max: 20.0,
            old_minor: 0x21,
            shadow_minor: 0x2000,
            final_minor: 0x35,
            ips: vec![format!("192.0.2.{hash}/32")],
            sqm_override: None,
            desired_cmd: mk_test_circuit(
                hash,
                0x10034,
                0x20034,
                0x35,
                0x1,
                0x2,
                &format!("192.0.2.{hash}/32"),
            ),
            stage: MigrationStage::VerifyFinalReady,
            shadow_verify_attempts: 0,
            final_verify_attempts: 0,
        }
    }

    #[test]
    fn observe_transition_cancels_pending_live_migrations_and_clears_live_migration_reload_state() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();
        clear_reload_required(
            "reset before observe_transition_cancels_pending_live_migrations_and_clears_live_migration_reload_state",
        );

        let mut migrations = HashMap::from([(9007, sample_test_migration(9007))]);
        mark_reload_required(
            "Bakery live-move final verification did not find the expected final class/qdisc for circuit 9007: observed missing. A full reload is now required before further incremental topology mutations."
                .to_string(),
        );

        let canceled = cancel_pending_migrations_for_observe_mode(
            &mut migrations,
            "queue mode transitioned to observe; the shaping tree is no longer live.",
        );

        assert_eq!(canceled, 1);
        assert!(migrations.is_empty());
        assert!(bakery_reload_required_reason().is_none());
        assert!(bakery_activity_snapshot().iter().any(|entry| {
            entry.event == "live_migrations_canceled"
                && entry.summary.contains("pending live migration")
        }));
    }

    #[test]
    fn observe_transition_preserves_non_migration_reload_required_state() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();
        clear_reload_required(
            "reset before observe_transition_preserves_non_migration_reload_required_state",
        );

        let unrelated_reason = "Bakery full reload triggered because site speed live-change enqueue failed: synthetic failure.";
        let mut migrations = HashMap::from([(9008, sample_test_migration(9008))]);
        mark_reload_required(unrelated_reason.to_string());

        let canceled = cancel_pending_migrations_for_observe_mode(
            &mut migrations,
            "queue mode transitioned to observe; the shaping tree is no longer live.",
        );

        assert_eq!(canceled, 1);
        assert!(migrations.is_empty());
        assert_eq!(
            bakery_reload_required_reason().as_deref(),
            Some(unrelated_reason)
        );
        clear_reload_required(
            "reset after observe_transition_preserves_non_migration_reload_required_state",
        );
    }

    #[test]
    fn migration_target_label_includes_circuit_and_site_names_when_known() {
        let migration = Migration {
            circuit_hash: 9006,
            circuit_name: Some("Rochester-Roof-Switch".to_string()),
            site_name: Some("7232 Rochester".to_string()),
            old_class_major: 0x1,
            old_up_class_major: 0x2,
            old_down_qdisc_handle: Some(0x9000),
            old_up_qdisc_handle: Some(0x9001),
            parent_class_id: TcHandle::from_u32(0x10034),
            up_parent_class_id: TcHandle::from_u32(0x20034),
            class_major: 0x1,
            up_class_major: 0x2,
            down_qdisc_handle: Some(0x9000),
            up_qdisc_handle: Some(0x9001),
            old_down_min: 1.0,
            old_down_max: 10.0,
            old_up_min: 1.0,
            old_up_max: 10.0,
            new_down_min: 1.0,
            new_down_max: 20.0,
            new_up_min: 1.0,
            new_up_max: 20.0,
            old_minor: 0x21,
            shadow_minor: 0x2000,
            final_minor: 0x35,
            ips: vec!["192.0.2.96/32".to_string()],
            sqm_override: None,
            desired_cmd: mk_test_circuit(9006, 0x10034, 0x20034, 0x35, 0x1, 0x2, "192.0.2.96/32"),
            stage: MigrationStage::BuildFinal,
            shadow_verify_attempts: 0,
            final_verify_attempts: 0,
        };

        let label = migration_target_label(&migration);
        assert!(label.contains("Rochester-Roof-Switch"));
        assert!(label.contains("9006"));
        assert!(label.contains("7232 Rochester"));
    }

    #[test]
    fn migration_branch_verification_summary_describes_expected_and_observed_state() {
        let verification = MigrationBranchVerification {
            down: MigrationDirectionVerification {
                interface_name: "if-down".to_string(),
                expected_handle: TcHandle::from_u32(0x4002000),
                expected_parent: TcHandle::from_u32(0x4000015),
                observed_present: true,
                observed_parent: Some(TcHandle::from_u32(0x4000013)),
                observed_leaf_qdisc_major: None,
            },
            up: Some(MigrationDirectionVerification {
                interface_name: "if-up".to_string(),
                expected_handle: TcHandle::from_u32(0x5002000),
                expected_parent: TcHandle::from_u32(0x5000015),
                observed_present: false,
                observed_parent: None,
                observed_leaf_qdisc_major: None,
            }),
        };

        let summary = verification.summary();
        assert!(summary.contains("if-down"));
        assert!(summary.contains("expected class"));
        assert!(summary.contains("2000"));
        assert!(summary.contains("parent"));
        assert!(summary.contains("15"));
        assert!(summary.contains("observed parent"));
        assert!(summary.contains("13"));
        assert!(summary.contains("with no leaf qdisc"));
        assert!(summary.contains("if-up"));
        assert!(summary.contains("500"));
        assert!(summary.contains("observed missing"));
        assert!(!verification.ready());
    }

    #[test]
    fn migration_branch_verification_summary_reports_root_attached_class() {
        let verification = MigrationDirectionVerification {
            interface_name: "if-root".to_string(),
            expected_handle: TcHandle::from_u32(0x221f5),
            expected_parent: TcHandle::from_u32(0x2103a),
            observed_present: true,
            observed_parent: None,
            observed_leaf_qdisc_major: Some(0x96c8),
        };

        let summary = verification.summary("down");
        assert!(summary.contains("observed at root"));
        assert!(summary.contains("96c8"));
    }

    #[test]
    fn wrong_parent_prune_commands_delete_root_attached_target_class() {
        let snapshot = HashMap::from([(
            TcHandle::from_u32(0x221f5),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x221f5),
                parent: None,
                leaf_qdisc_major: Some(0x96c8),
            },
        )]);

        let commands = wrong_parent_prune_commands_for_direction(
            "if-root".to_string(),
            &snapshot,
            TcHandle::from_u32(0x221f5),
            TcHandle::from_u32(0x2103a),
        );

        assert_eq!(
            commands,
            vec![
                vec![
                    "qdisc".to_string(),
                    "del".to_string(),
                    "dev".to_string(),
                    "if-root".to_string(),
                    "parent".to_string(),
                    "0x2:0x21f5".to_string(),
                ],
                vec![
                    "class".to_string(),
                    "del".to_string(),
                    "dev".to_string(),
                    "if-root".to_string(),
                    "classid".to_string(),
                    "0x2:0x21f5".to_string(),
                ],
            ]
        );
    }

    #[test]
    fn wrong_parent_prune_commands_skip_target_when_parent_already_matches() {
        let snapshot = HashMap::from([(
            TcHandle::from_u32(0x221f5),
            LiveTcClassEntry {
                class_id: TcHandle::from_u32(0x221f5),
                parent: Some(TcHandle::from_u32(0x2103a)),
                leaf_qdisc_major: Some(0x96c8),
            },
        )]);

        let commands = wrong_parent_prune_commands_for_direction(
            "if-ok".to_string(),
            &snapshot,
            TcHandle::from_u32(0x221f5),
            TcHandle::from_u32(0x2103a),
        );

        assert!(commands.is_empty());
    }

    #[test]
    fn migration_invariant_rejects_stale_qdisc_handles_when_parent_changes() {
        let migration = Migration {
            circuit_hash: 9004,
            circuit_name: None,
            site_name: None,
            old_class_major: 0x1,
            old_up_class_major: 0x2,
            old_down_qdisc_handle: Some(0x9000),
            old_up_qdisc_handle: Some(0x9001),
            parent_class_id: TcHandle::from_u32(0x10034),
            up_parent_class_id: TcHandle::from_u32(0x20034),
            class_major: 0x1,
            up_class_major: 0x2,
            down_qdisc_handle: Some(0x9000),
            up_qdisc_handle: Some(0x9001),
            old_down_min: 1.0,
            old_down_max: 10.0,
            old_up_min: 1.0,
            old_up_max: 10.0,
            new_down_min: 1.0,
            new_down_max: 20.0,
            new_up_min: 1.0,
            new_up_max: 20.0,
            old_minor: 0x21,
            shadow_minor: 0x2000,
            final_minor: 0x35,
            ips: vec!["192.0.2.94/32".to_string()],
            sqm_override: None,
            desired_cmd: mk_test_circuit(9004, 0x10034, 0x20034, 0x35, 0x1, 0x2, "192.0.2.94/32"),
            stage: MigrationStage::BuildFinal,
            shadow_verify_attempts: 0,
            final_verify_attempts: 0,
        };

        let summary = migration_qdisc_handle_rotation_invariant_error(&migration)
            .expect("stale qdisc handles should be rejected");
        assert!(summary.contains("old handle"));

        let final_cmd = build_temp_add_cmd(
            migration.desired_cmd.as_ref(),
            migration.final_minor,
            migration.new_down_min,
            migration.new_down_max,
            migration.new_up_min,
            migration.new_up_max,
            true,
        )
        .expect("final command");
        let final_summary =
            build_final_qdisc_handle_rotation_invariant_error(&migration, &final_cmd)
                .expect("final command should also be rejected");
        assert!(final_summary.contains("old handle"));
    }

    #[test]
    fn migration_invariant_rejects_live_reserved_handle_collisions() {
        let migration = Migration {
            circuit_hash: 9005,
            circuit_name: None,
            site_name: None,
            old_class_major: 0x1,
            old_up_class_major: 0x2,
            old_down_qdisc_handle: Some(0x9000),
            old_up_qdisc_handle: Some(0x9001),
            parent_class_id: TcHandle::from_u32(0x10034),
            up_parent_class_id: TcHandle::from_u32(0x20034),
            class_major: 0x1,
            up_class_major: 0x2,
            down_qdisc_handle: Some(0x93c9),
            up_qdisc_handle: Some(0x93ca),
            old_down_min: 1.0,
            old_down_max: 10.0,
            old_up_min: 1.0,
            old_up_max: 10.0,
            new_down_min: 1.0,
            new_down_max: 20.0,
            new_up_min: 1.0,
            new_up_max: 20.0,
            old_minor: 0x21,
            shadow_minor: 0x2000,
            final_minor: 0x35,
            ips: vec!["192.0.2.95/32".to_string()],
            sqm_override: None,
            desired_cmd: Arc::new(BakeryCommands::AddCircuit {
                circuit_hash: 9005,
                circuit_name: None,
                site_name: None,
                parent_class_id: TcHandle::from_u32(0x10034),
                up_parent_class_id: TcHandle::from_u32(0x20034),
                class_minor: 0x35,
                download_bandwidth_min: 1.0,
                upload_bandwidth_min: 1.0,
                download_bandwidth_max: 20.0,
                upload_bandwidth_max: 20.0,
                class_major: 0x1,
                up_class_major: 0x2,
                down_qdisc_handle: Some(0x93c9),
                up_qdisc_handle: Some(0x93ca),
                ip_addresses: "192.0.2.95/32".to_string(),
                sqm_override: None,
            }),
            stage: MigrationStage::BuildFinal,
            shadow_verify_attempts: 0,
            final_verify_attempts: 0,
        };

        let final_cmd = build_temp_add_cmd(
            migration.desired_cmd.as_ref(),
            migration.final_minor,
            migration.new_down_min,
            migration.new_down_max,
            migration.new_up_min,
            migration.new_up_max,
            true,
        )
        .expect("final command");
        let down_reserved = HashSet::from([0x93c9]);
        let up_reserved = HashSet::from([0x93ca]);
        let final_summary =
            build_final_qdisc_handle_rotation_invariant_error_with_live_reservations(
                &migration,
                &final_cmd,
                Some(&down_reserved),
                Some(&up_reserved),
            )
            .expect("live reserved collision should be rejected");
        assert!(final_summary.contains("already live"));
    }

    #[test]
    fn test_fault_once_matches_selector_and_clears_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "lqos-bakery-fault-{}.txt",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::write(&path, "migrating circuits between parent nodes").expect("fault file");

        let summary = consume_test_fault_once_from_path(
            &path,
            "migrating circuits between parent nodes (fallback)",
        )
        .expect("fault should trigger");
        assert!(summary.contains("synthetic Bakery test fault"));
        assert!(!path.exists());
    }

    #[test]
    fn test_fault_once_ignores_non_matching_selector() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "lqos-bakery-fault-{}.txt",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::write(&path, "TreeGuard runtime circuit reparent").expect("fault file");

        let summary = consume_test_fault_once_from_path(
            &path,
            "migrating circuits between parent nodes (fallback)",
        );
        assert!(summary.is_none());
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn queue_top_level_runtime_migration_succeeds_for_inactive_parent_move() {
        let old_cmd = mk_test_circuit(9200, 0x10020, 0x20020, 0x21, 0x1, 0x2, "192.0.2.92/32");
        let new_cmd = mk_test_circuit(9200, 0x10034, 0x20034, 0x35, 0x3, 0x4, "192.0.2.92/32");

        let mut circuits = HashMap::from([(9200, Arc::clone(&old_cmd))]);
        let sites = HashMap::new();
        let live_circuits = HashMap::new();
        let mut migrations = HashMap::new();

        let queued = queue_top_level_runtime_migration(
            old_cmd.as_ref(),
            &new_cmd,
            &sites,
            &mut circuits,
            &live_circuits,
            &mut migrations,
        );

        assert!(queued);
        let migration = migrations.get(&9200).expect("migration should be queued");
        assert_eq!(migration.stage, MigrationStage::PrepareShadow);
        assert_eq!(migration.parent_class_id, TcHandle::from_u32(0x10034));
        assert_eq!(migration.final_minor, 0x35);
        assert!(migration.shadow_minor >= 0x2000);
        assert_ne!(migration.shadow_minor, migration.old_minor);
        assert!(matches!(
            circuits.get(&9200),
            Some(cmd) if Arc::ptr_eq(cmd, &old_cmd)
        ));
        assert!(Arc::ptr_eq(&migration.desired_cmd, &new_cmd));
    }

    #[test]
    fn build_shadow_add_cmd_uses_ephemeral_qdisc_handles_distinct_from_old_and_final() {
        let config = test_config_with_runtime_dir("shadow-create-handles");
        let layout = MqDeviceLayout::from_setup(&config, 2, 0);
        let mut handles = QdiscHandleState::default();

        let original = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 9400,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10020),
            up_parent_class_id: TcHandle::from_u32(0x20020),
            class_minor: 0x21,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x1,
            up_class_major: 0x2,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.96/32".to_string(),
            sqm_override: None,
        });
        let original = with_assigned_qdisc_handles(&original, &config, &layout, &mut handles);
        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(original_down),
            up_qdisc_handle: Some(original_up),
            ..
        } = original.as_ref()
        else {
            panic!("expected original handles");
        };

        handles.save(&config);
        let mut reloaded = QdiscHandleState::load(&config);
        let moved = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 9400,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10034),
            up_parent_class_id: TcHandle::from_u32(0x20034),
            class_minor: 0x35,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x3,
            up_class_major: 0x4,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.96/32".to_string(),
            sqm_override: None,
        });
        let moved = with_assigned_qdisc_handles(&moved, &config, &layout, &mut reloaded);
        let moved = rotate_changed_qdisc_handles(
            original.as_ref(),
            &moved,
            &config,
            &layout,
            &mut reloaded,
        );
        reloaded.save(&config);

        let mut circuits = HashMap::from([(9400, Arc::clone(&original))]);
        let sites = HashMap::new();
        let live_circuits = HashMap::from([(9400, 1u64)]);
        let mut migrations = HashMap::new();
        assert!(queue_live_migration(
            original.as_ref(),
            &moved,
            &sites,
            &mut circuits,
            &live_circuits,
            &mut migrations,
        ));
        let migration = migrations.get(&9400).expect("migration should exist");
        let persisted_handles = QdiscHandleState::load(&config);
        let live_reserved = HashMap::from([
            (config.isp_interface(), HashSet::from([*original_down])),
            (config.internet_interface(), HashSet::from([*original_up])),
        ]);
        let shadow = build_shadow_add_cmd(migration, &config, &persisted_handles, &live_reserved)
            .expect("shadow command");

        let BakeryCommands::AddCircuit {
            class_minor,
            down_qdisc_handle: Some(shadow_down),
            up_qdisc_handle: Some(shadow_up),
            ..
        } = shadow
        else {
            panic!("expected shadow qdisc handles");
        };
        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(final_down),
            up_qdisc_handle: Some(final_up),
            ..
        } = moved.as_ref()
        else {
            panic!("expected final qdisc handles");
        };

        assert_eq!(class_minor, migration.shadow_minor);
        assert_ne!(shadow_down, *original_down);
        assert_ne!(shadow_up, *original_up);
        assert_ne!(shadow_down, *final_down);
        assert_ne!(shadow_up, *final_up);
    }

    #[test]
    fn full_reload_allocation_uses_fresh_handles_reserved_from_live_tree() {
        let config = test_config_with_runtime_dir("fresh-full-reload");
        let layout = MqDeviceLayout::from_setup(&config, 2, 0);
        let mut persisted = QdiscHandleState::default();

        let original = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 505,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.5/32".to_string(),
            sqm_override: None,
        });

        let original = with_assigned_qdisc_handles(&original, &config, &layout, &mut persisted);
        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(original_down),
            up_qdisc_handle: Some(original_up),
            ..
        } = original.as_ref()
        else {
            panic!("expected assigned handles");
        };

        let mut full_reload_handles = QdiscHandleState::default();
        let live_reserved = HashMap::from([
            (config.isp_interface(), HashSet::from([*original_down])),
            (config.internet_interface(), HashSet::from([*original_up])),
        ]);
        let rebuilt_cmd = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 505,
            circuit_name: None,
            site_name: None,
            parent_class_id: TcHandle::from_u32(0x10001),
            up_parent_class_id: TcHandle::from_u32(0x20001),
            class_minor: 0x20,
            download_bandwidth_min: 10.0,
            upload_bandwidth_min: 10.0,
            download_bandwidth_max: 100.0,
            upload_bandwidth_max: 100.0,
            class_major: 0x100,
            up_class_major: 0x200,
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses: "192.0.2.5/32".to_string(),
            sqm_override: None,
        });
        let rebuilt = with_assigned_qdisc_handles_reserved(
            &rebuilt_cmd,
            &config,
            &layout,
            &mut full_reload_handles,
            &live_reserved,
        );

        let BakeryCommands::AddCircuit {
            down_qdisc_handle: Some(rebuilt_down),
            up_qdisc_handle: Some(rebuilt_up),
            ..
        } = rebuilt.as_ref()
        else {
            panic!("expected rebuilt handles");
        };

        assert_ne!(*rebuilt_down, *original_down);
        assert_ne!(*rebuilt_up, *original_up);
        assert_eq!(*rebuilt_down, *original_down + 1);
        assert_eq!(*rebuilt_up, *original_up + 1);
    }
}

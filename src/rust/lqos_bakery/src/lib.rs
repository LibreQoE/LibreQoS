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
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::{debug, error, info, warn};
use utils::current_timestamp;
pub(crate) const CHANNEL_CAPACITY: usize = 65536; // 64k capacity for Bakery commands
use crate::commands::{
    ExecutionMode, RuntimeNodeOperationAction, RuntimeNodeOperationSnapshot,
    RuntimeNodeOperationStatus,
};
use crate::diff::{CircuitDiffResult, SiteDiffResult, diff_circuits, diff_sites};
use crate::qdisc_handles::QdiscHandleState;
use crate::queue_math::{SqmKind, effective_sqm_kind, format_rate_for_tc_f32};
use crate::utils::{
    ExecuteResult, LiveTcClassEntry, MemorySnapshot, execute_in_memory, execute_in_memory_chunked,
    read_live_class_snapshot, read_live_qdisc_handle_majors, read_memory_snapshot,
    write_command_file,
};
pub use commands::{
    BakeryCommands, RuntimeNodeOperationAction as BakeryRuntimeNodeOperationAction,
    RuntimeNodeOperationSnapshot as BakeryRuntimeNodeOperationSnapshot,
    RuntimeNodeOperationStatus as BakeryRuntimeNodeOperationStatus,
};
use lqos_bus::{
    BusRequest, BusResponse, InsightLicenseSummary, LibreqosBusClient, TcHandle, UrgentSeverity,
    UrgentSource,
};
use lqos_config::{
    CircuitIdentityGroupInput, Config, LazyQueueMode, PlannerCircuitIdentityState,
    PlannerSiteIdentityState, SiteIdentityInput, TopLevelPlannerItem, TopLevelPlannerMode,
    TopLevelPlannerParams, plan_class_identities, plan_top_level_assignments,
};
use qdisc_handles::MqDeviceLayout;

const TEST_FAULT_ONCE_PATH: &str = "/tmp/lqos_bakery_fail_purpose_once.txt";
// ---------------------- Live-Move Types and Helpers (module scope) ----------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationStage {
    PrepareShadow,
    SwapToShadow,
    BuildFinal,
    SwapToFinal,
    TeardownShadow,
    Done,
}

#[derive(Clone, Debug)]
struct Migration {
    circuit_hash: i64,
    // Old parent handles and majors
    old_parent_class_id: TcHandle,
    old_up_parent_class_id: TcHandle,
    old_class_major: u16,
    old_up_class_major: u16,
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

#[derive(Clone, Debug)]
struct VirtualizedSiteState {
    site: Arc<BakeryCommands>,
    saved_sites: HashMap<i64, Arc<BakeryCommands>>,
    saved_circuits: HashMap<i64, Arc<BakeryCommands>>,
    active_sites: HashMap<i64, Arc<BakeryCommands>>,
    active_circuits: HashMap<i64, Arc<BakeryCommands>>,
    qdisc_handles: VirtualizedSiteQdiscHandles,
    pending_prune: bool,
    next_prune_attempt_unix: u64,
}

const RUNTIME_SITE_PRUNE_RETRY_SECONDS: u64 = 30;
const RUNTIME_SITE_PRUNE_MAX_ATTEMPTS: u32 = 5;
const BAKERY_BACKGROUND_INTERVAL_MS: u64 = 250;
const RUNTIME_DIRTY_SUBTREE_RELOAD_THRESHOLD: usize = 3;
const RUNTIME_NODE_OPERATION_CAPACITY: usize = 32;
const RUNTIME_NODE_OPERATION_DEFERRED_RETRY_SECONDS: u64 = 60;

#[derive(Clone, Debug)]
struct RuntimeNodeOperation {
    operation_id: u64,
    site_hash: i64,
    action: RuntimeNodeOperationAction,
    status: RuntimeNodeOperationStatus,
    attempt_count: u32,
    submitted_at_unix: u64,
    updated_at_unix: u64,
    next_retry_at_unix: Option<u64>,
    last_error: Option<String>,
}

impl RuntimeNodeOperation {
    fn new(
        operation_id: u64,
        site_hash: i64,
        action: RuntimeNodeOperationAction,
        now_unix: u64,
    ) -> Self {
        Self {
            operation_id,
            site_hash,
            action,
            status: RuntimeNodeOperationStatus::Submitted,
            attempt_count: 0,
            submitted_at_unix: now_unix,
            updated_at_unix: now_unix,
            next_retry_at_unix: None,
            last_error: None,
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
        }
    }

    fn update_status(
        &mut self,
        status: RuntimeNodeOperationStatus,
        now_unix: u64,
        last_error: Option<String>,
        next_retry_at_unix: Option<u64>,
    ) {
        self.status = status;
        self.updated_at_unix = now_unix;
        self.last_error = last_error;
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

fn used_minors_for_parent(
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    parent: &TcHandle,
) -> HashSet<u16> {
    let mut set = HashSet::new();
    for (_k, v) in circuits.iter() {
        if let BakeryCommands::AddCircuit {
            parent_class_id,
            class_minor,
            ..
        } = v.as_ref()
            && parent_class_id == parent
        {
            set.insert(*class_minor);
        }
    }
    set
}

fn find_free_minor(
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    down_parent: &TcHandle,
    up_parent: &TcHandle,
) -> Option<u16> {
    let used_down = used_minors_for_parent(circuits, down_parent);
    let used_up = used_minors_for_parent(circuits, up_parent);
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

fn queue_runtime_migration(
    old_cmd: &BakeryCommands,
    new_cmd: &Arc<BakeryCommands>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &HashMap<i64, u64>,
    migrations: &mut HashMap<i64, Migration>,
    require_live_circuit: bool,
) -> bool {
    let BakeryCommands::AddCircuit {
        circuit_hash,
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
        parent_class_id: old_parent_class_id,
        up_parent_class_id: old_up_parent_class_id,
        class_minor: old_minor,
        download_bandwidth_min: old_down_min,
        upload_bandwidth_min: old_up_min,
        download_bandwidth_max: old_down_max,
        upload_bandwidth_max: old_up_max,
        class_major: old_class_major,
        up_class_major: old_up_class_major,
        ..
    } = old_cmd
    else {
        return false;
    };

    let Some(shadow_minor) = find_free_minor(circuits, parent_class_id, up_parent_class_id) else {
        return false;
    };

    let mig = Migration {
        circuit_hash: *circuit_hash,
        old_parent_class_id: *old_parent_class_id,
        old_up_parent_class_id: *old_up_parent_class_id,
        old_class_major: *old_class_major,
        old_up_class_major: *old_up_class_major,
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
    };

    migrations.insert(*circuit_hash, mig);
    true
}

fn queue_live_migration(
    old_cmd: &BakeryCommands,
    new_cmd: &Arc<BakeryCommands>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &HashMap<i64, u64>,
    migrations: &mut HashMap<i64, Migration>,
) -> bool {
    queue_runtime_migration(old_cmd, new_cmd, circuits, live_circuits, migrations, true)
}

fn queue_top_level_runtime_migration(
    old_cmd: &BakeryCommands,
    new_cmd: &Arc<BakeryCommands>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &HashMap<i64, u64>,
    migrations: &mut HashMap<i64, Migration>,
) -> bool {
    queue_runtime_migration(old_cmd, new_cmd, circuits, live_circuits, migrations, false)
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
/// Hard kernel limit for auto-allocated qdisc handles on a single network interface.
pub const HARD_QDISC_HANDLE_LIMIT_PER_INTERFACE: usize = 65_534;
/// Conservative operational limit used to fail a full reload before qdisc handle exhaustion.
pub const SAFE_QDISC_BUDGET_PER_INTERFACE: usize = 65_000;
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
    /// Number of failed operations that may be retried.
    pub failed_count: usize,
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
    /// Current runtime node-operation summary.
    pub runtime_operations: BakeryRuntimeOperationsSnapshot,
    /// Current queue-root distribution summary.
    pub queue_distribution: Vec<BakeryQueueDistributionSnapshot>,
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
    preflight: Option<BakeryPreflightSnapshot>,
    reload_required: bool,
    reload_required_reason: Option<String>,
    dirty_subtree_count: usize,
    runtime_operations_by_site: HashMap<i64, RuntimeNodeOperationSnapshot>,
    activity: VecDeque<BakeryActivityEntry>,
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
                dirty_count: 0,
                latest: None,
            },
            queue_distribution: Vec::new(),
            preflight: None,
            reload_required: false,
            reload_required_reason: None,
            dirty_subtree_count: 0,
            runtime_operations_by_site: HashMap::new(),
            activity: VecDeque::with_capacity(BAKERY_EVENT_LIMIT),
        }
    }
}

static BAKERY_TELEMETRY: OnceLock<RwLock<BakeryTelemetryState>> = OnceLock::new();

/// Message Queue sender for the bakery
pub static BAKERY_SENDER: OnceLock<Sender<BakeryCommands>> = OnceLock::new();
static MQ_CREATED: AtomicBool = AtomicBool::new(false);
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
    let entry = BakeryActivityEntry {
        ts: current_timestamp(),
        event: event.to_string(),
        status: status.to_string(),
        summary,
    };
    let mut state = telemetry_state().write();
    state.activity.push_front(entry);
    while state.activity.len() > BAKERY_EVENT_LIMIT {
        state.activity.pop_back();
    }
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
        last_failure_unix: state.last_failure_unix,
        last_failure_summary: state.last_failure_summary,
        last_apply_type: state.last_apply_type,
        last_total_tc_commands: state.last_total_tc_commands,
        last_class_commands: state.last_class_commands,
        last_qdisc_commands: state.last_qdisc_commands,
        last_build_duration_ms: state.last_build_duration_ms,
        last_apply_duration_ms: state.last_apply_duration_ms,
        runtime_operations: state.runtime_operations,
        queue_distribution: state.queue_distribution,
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

    if config.queues.monitor_only {
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

    if config.queues.monitor_only {
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

    if config.queues.monitor_only {
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

fn rotate_changed_qdisc_handles(
    previous: &BakeryCommands,
    command: &Arc<BakeryCommands>,
    config: &Arc<Config>,
    mq_layout: &MqDeviceLayout,
    qdisc_handles: &mut QdiscHandleState,
) -> Arc<BakeryCommands> {
    let (old_down_kind, old_up_kind) = effective_directional_sqm_kinds(previous, config);
    let (new_down_kind, new_up_kind) = effective_directional_sqm_kinds(command.as_ref(), config);
    let (old_down_parent, old_up_parent) = effective_directional_qdisc_parents(previous, config);
    let (new_down_parent, new_up_parent) =
        effective_directional_qdisc_parents(command.as_ref(), config);

    let mut rotated = command.as_ref().clone();
    let isp_interface = config.isp_interface();
    let internet_interface = config.internet_interface();
    let isp_reserved = mq_layout.reserved_handles(&isp_interface);
    let up_reserved = mq_layout.reserved_handles(&internet_interface);

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
        if down_kind_changed || down_parent_changed {
            *down_qdisc_handle =
                qdisc_handles.rotate_circuit_handle(&isp_interface, *circuit_hash, &isp_reserved);
        }
        let up_kind_changed =
            old_up_kind.is_some() && new_up_kind.is_some() && old_up_kind != new_up_kind;
        let up_parent_changed =
            old_up_parent.is_some() && new_up_parent.is_some() && old_up_parent != new_up_parent;
        if up_kind_changed || up_parent_changed {
            *up_qdisc_handle = qdisc_handles.rotate_circuit_handle(
                &internet_interface,
                *circuit_hash,
                &up_reserved,
            );
        }
    }

    Arc::new(rotated)
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

    fn migration_final_state_ready(
        config: &Arc<Config>,
        migration: &Migration,
    ) -> Result<bool, String> {
        let down_snapshot = read_live_class_snapshot(&config.isp_interface())?;
        let down_handle = TcHandle::from_u32(
            (u32::from(migration.class_major) << 16) | u32::from(migration.final_minor),
        );
        let Some(down_entry) = down_snapshot.get(&down_handle) else {
            return Ok(false);
        };
        if down_entry.leaf_qdisc_major.is_none() {
            return Ok(false);
        }

        if config.on_a_stick_mode() {
            return Ok(true);
        }

        let up_snapshot = read_live_class_snapshot(&config.internet_interface())?;
        let up_handle = TcHandle::from_u32(
            (u32::from(migration.up_class_major) << 16) | u32::from(migration.final_minor),
        );
        let Some(up_entry) = up_snapshot.get(&up_handle) else {
            return Ok(false);
        };
        if up_entry.leaf_qdisc_major.is_none() {
            return Ok(false);
        }

        Ok(true)
    }

    fn process_pending_migrations(
        circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
        live_circuits: &mut HashMap<i64, u64>,
        sites: &HashMap<i64, Arc<BakeryCommands>>,
        migrations: &mut HashMap<i64, Migration>,
        mapping_current: &mut HashMap<MappingKey, MappingVal>,
        virtualized_sites: &mut HashMap<i64, VirtualizedSiteState>,
        runtime_node_operations: &mut HashMap<i64, RuntimeNodeOperation>,
    ) {
        if migrations.is_empty() {
            return;
        }

        let Ok(config) = lqos_config::load_config() else {
            error!("Failed to load configuration while processing pending migrations.");
            return;
        };

        let mut advanced = 0usize;
        let mut to_remove = Vec::new();
        let mut effective_state_changed = false;

        for (_hash, mig) in migrations.iter_mut() {
            if advanced >= MIGRATIONS_PER_TICK {
                break;
            }

            match mig.stage {
                MigrationStage::PrepareShadow => {
                    if let Some(temp) = build_temp_add_cmd(
                        &BakeryCommands::AddCircuit {
                            circuit_hash: mig.circuit_hash,
                            parent_class_id: mig.parent_class_id,
                            up_parent_class_id: mig.up_parent_class_id,
                            class_minor: mig.shadow_minor,
                            download_bandwidth_min: mig.old_down_min,
                            upload_bandwidth_min: mig.old_up_min,
                            download_bandwidth_max: mig.old_down_max,
                            upload_bandwidth_max: mig.old_up_max,
                            class_major: mig.class_major,
                            up_class_major: mig.up_class_major,
                            down_qdisc_handle: mig.down_qdisc_handle,
                            up_qdisc_handle: mig.up_qdisc_handle,
                            ip_addresses: "".to_string(),
                            sqm_override: mig.sqm_override.clone(),
                        },
                        mig.shadow_minor,
                        mig.old_down_min,
                        mig.old_down_max,
                        mig.old_up_min,
                        mig.old_up_max,
                        false,
                    ) {
                        let mut cmds = Vec::new();
                        match config.queues.lazy_queues.as_ref() {
                            None | Some(LazyQueueMode::No) => {
                                if let Some(c) =
                                    add_commands_for_circuit(&temp, &config, ExecutionMode::Builder)
                                {
                                    cmds.extend(c);
                                }
                            }
                            Some(LazyQueueMode::Htb) => {
                                if let Some(c) =
                                    add_commands_for_circuit(&temp, &config, ExecutionMode::Builder)
                                {
                                    cmds.extend(c);
                                }
                                if let Some(c) = add_commands_for_circuit(
                                    &temp,
                                    &config,
                                    ExecutionMode::LiveUpdate,
                                ) {
                                    cmds.extend(c);
                                }
                            }
                            Some(LazyQueueMode::Full) => {
                                if let Some(c) = add_commands_for_circuit(
                                    &temp,
                                    &config,
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
                                MigrationStage::SwapToShadow,
                            );
                        } else {
                            mig.stage = MigrationStage::SwapToShadow;
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
                MigrationStage::SwapToShadow => {
                    for ip in &mig.ips {
                        let (ip_s, prefix) = parse_ip_and_prefix(ip);
                        let key = MappingKey {
                            ip: ip_s.clone(),
                            prefix,
                        };
                        let cpu = mapping_current.get(&key).map(|v| v.cpu).unwrap_or(0);
                        let handle = tc_handle_from_major_minor(mig.class_major, mig.shadow_minor);
                        let _ = lqos_sys::add_ip_to_tc(&ip_s, handle, cpu, false, 0, 0);
                        mapping_current.insert(key, MappingVal { handle, cpu });
                    }
                    let _ = lqos_sys::clear_hot_cache();
                    mig.stage = MigrationStage::BuildFinal;
                    advanced += 1;
                }
                MigrationStage::BuildFinal => {
                    if let Some(old_cmd) = build_temp_add_cmd(
                        &BakeryCommands::AddCircuit {
                            circuit_hash: mig.circuit_hash,
                            parent_class_id: mig.old_parent_class_id,
                            up_parent_class_id: mig.old_up_parent_class_id,
                            class_minor: mig.old_minor,
                            download_bandwidth_min: mig.old_down_min,
                            upload_bandwidth_min: mig.old_up_min,
                            download_bandwidth_max: mig.old_down_max,
                            upload_bandwidth_max: mig.old_up_max,
                            class_major: mig.old_class_major,
                            up_class_major: mig.old_up_class_major,
                            down_qdisc_handle: None,
                            up_qdisc_handle: None,
                            ip_addresses: "".to_string(),
                            sqm_override: mig.sqm_override.clone(),
                        },
                        mig.old_minor,
                        mig.old_down_min,
                        mig.old_down_max,
                        mig.old_up_min,
                        mig.old_up_max,
                        true,
                    ) {
                        let mut cmds = Vec::new();
                        if let Some(prune) = old_cmd.to_prune(&config, true) {
                            cmds.extend(prune);
                        }
                        if let Some(final_cmd) = build_temp_add_cmd(
                            mig.desired_cmd.as_ref(),
                            mig.final_minor,
                            mig.new_down_min,
                            mig.new_down_max,
                            mig.new_up_min,
                            mig.new_up_max,
                            true,
                        ) {
                            match config.queues.lazy_queues.as_ref() {
                                None | Some(LazyQueueMode::No) => {
                                    if let Some(c) = add_commands_for_circuit(
                                        &final_cmd,
                                        &config,
                                        ExecutionMode::Builder,
                                    ) {
                                        cmds.extend(c);
                                    }
                                }
                                Some(LazyQueueMode::Htb) => {
                                    if let Some(c) = add_commands_for_circuit(
                                        &final_cmd,
                                        &config,
                                        ExecutionMode::Builder,
                                    ) {
                                        cmds.extend(c);
                                    }
                                    if let Some(c) = add_commands_for_circuit(
                                        &final_cmd,
                                        &config,
                                        ExecutionMode::LiveUpdate,
                                    ) {
                                        cmds.extend(c);
                                    }
                                }
                                Some(LazyQueueMode::Full) => {
                                    if let Some(c) = add_commands_for_circuit(
                                        &final_cmd,
                                        &config,
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
                                MigrationStage::SwapToFinal,
                            );
                        } else {
                            mig.stage = MigrationStage::SwapToFinal;
                        }
                        advanced += 1;
                    } else {
                        warn!(
                            "live-move: failed to build old prune cmd for {}",
                            mig.circuit_hash
                        );
                        mig.stage = MigrationStage::Done;
                        advanced += 1;
                    }
                }
                MigrationStage::SwapToFinal => {
                    let final_ready = match migration_final_state_ready(&config, mig) {
                        Ok(ready) => ready,
                        Err(e) => {
                            mark_reload_required(format!(
                                "Bakery live-move final verification failed for circuit {}: {}. A full reload is now required before further incremental topology mutations.",
                                mig.circuit_hash, e
                            ));
                            mig.stage = MigrationStage::Done;
                            advanced += 1;
                            continue;
                        }
                    };
                    if !final_ready {
                        mark_reload_required(format!(
                            "Bakery live-move final verification did not find the expected final class/qdisc for circuit {}. A full reload is now required before further incremental topology mutations.",
                            mig.circuit_hash
                        ));
                        mig.stage = MigrationStage::Done;
                        advanced += 1;
                        continue;
                    }

                    for ip in &mig.ips {
                        let (ip_s, prefix) = parse_ip_and_prefix(ip);
                        let key = MappingKey {
                            ip: ip_s.clone(),
                            prefix,
                        };
                        let cpu = mapping_current.get(&key).map(|v| v.cpu).unwrap_or(0);
                        let handle = tc_handle_from_major_minor(mig.class_major, mig.final_minor);
                        let _ = lqos_sys::add_ip_to_tc(&ip_s, handle, cpu, false, 0, 0);
                        mapping_current.insert(key, MappingVal { handle, cpu });
                    }
                    let _ = lqos_sys::clear_hot_cache();
                    circuits.insert(mig.circuit_hash, Arc::clone(&mig.desired_cmd));
                    effective_state_changed = true;
                    mig.stage = MigrationStage::TeardownShadow;
                    advanced += 1;
                }
                MigrationStage::TeardownShadow => {
                    if let Some(shadow_cmd) = build_temp_add_cmd(
                        &BakeryCommands::AddCircuit {
                            circuit_hash: mig.circuit_hash,
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
                    ) && let Some(prune) = shadow_cmd.to_prune(&config, true)
                    {
                        let result =
                            execute_and_record_live_change(&prune, "live-move: prune shadow");
                        let _ = migration_stage_apply_succeeded(
                            mig,
                            "live-move: prune shadow",
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
            &config,
            sites,
            circuits,
            virtualized_sites,
            migrations,
            runtime_node_operations,
        );
        if effective_state_changed {
            update_queue_distribution_snapshot(sites, circuits);
        }

        let _ = live_circuits;
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
                process_pending_migrations(
                    &mut circuits,
                    &mut live_circuits,
                    &sites,
                    &mut migrations,
                    &mut mapping_current,
                    &mut virtualized_sites,
                    &mut runtime_node_operations,
                );
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
                    &mut live_circuits,
                    &mut mq_layout,
                    &mut qdisc_handles,
                    &tx,
                    &mut migrations,
                    &stormguard_overrides,
                    &mut virtualized_sites,
                    &mut runtime_node_operations,
                );
                process_pending_migrations(
                    &mut circuits,
                    &mut live_circuits,
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
            BakeryCommands::OnCircuitActivity { circuit_ids } => {
                handle_circuit_activity(circuit_ids, &circuits, &mut live_circuits);
            }
            BakeryCommands::Tick => {
                // Reset per-cycle counters at the start of the tick
                handle_tick(&mut circuits, &mut live_circuits, &mut sites);
                process_pending_migrations(
                    &mut circuits,
                    &mut live_circuits,
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

    let Some(new_batch) = batch.take() else {
        debug!("CommitBatch received without a batch to commit.");
        return;
    };
    let new_batch = apply_runtime_virtualization_overlay(new_batch, virtualized_sites);
    let resolved_mq_layout = current_mq_layout(&new_batch, &config, mq_layout);

    let mapped_limit = resolve_mapped_circuit_limit();
    let effective_limit = mapped_limit.effective_limit;
    let limit_label = format_mapped_limit(effective_limit);

    if let Some(reason) = bakery_reload_required_reason() {
        let (new_batch, mapped_limit_stats) =
            filter_batch_by_mapped_circuit_limit(new_batch, circuits, effective_limit);
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
        warn!(
            "Bakery: full reload required before further incremental topology mutation: {}",
            reason
        );
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
        );
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    let has_mq_been_setup = MQ_CREATED.load(std::sync::atomic::Ordering::Relaxed);
    if !has_mq_been_setup {
        push_bakery_event(
            "baseline_rebuild_required",
            "warning",
            "Bakery runtime state was reset by restart/cold start; performing explicit baseline full reload.".to_string(),
        );
        let (new_batch, mapped_limit_stats) =
            filter_batch_by_mapped_circuit_limit(new_batch, circuits, effective_limit);
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
        info!("Bakery baseline rebuild after restart/cold start: performing explicit full reload.");
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
        );
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    let site_change_mode = diff_sites(&new_batch, sites);
    if matches!(site_change_mode, SiteDiffResult::RebuildRequired) {
        let (new_batch, mapped_limit_stats) =
            filter_batch_by_mapped_circuit_limit(new_batch, circuits, effective_limit);
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
        );
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    let circuits_for_diff = circuits_with_pending_migration_targets(circuits, migrations);
    let circuit_change_mode = diff_circuits(&new_batch, &circuits_for_diff);

    // If neither has changed, there's nothing to do.
    if matches!(site_change_mode, SiteDiffResult::NoChange)
        && matches!(circuit_change_mode, CircuitDiffResult::NoChange)
    {
        // No changes detected, skip processing
        info!("No changes detected in batch, skipping processing.");
        return;
    }

    // If any structural changes occurred, do a full reload
    if let CircuitDiffResult::Categorized(categories) = &circuit_change_mode
        && !categories.structural_changed.is_empty()
    {
        let (new_batch, mapped_limit_stats) =
            filter_batch_by_mapped_circuit_limit(new_batch.clone(), circuits, effective_limit);
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
        );
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
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
                full_reload(
                    batch,
                    sites,
                    circuits,
                    live_circuits,
                    mq_layout,
                    qdisc_handles,
                    &config,
                    new_batch.clone(),
                    resolved_mq_layout.clone(),
                    stormguard_overrides,
                    virtualized_sites,
                    runtime_node_operations,
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
            let Some(layout) = resolved_mq_layout.as_ref() else {
                warn!("Bakery: missing MQ layout during circuit speed updates");
                return;
            };
            let mut immediate_commands = Vec::new();
            for cmd in &categories.speed_changed {
                let mut enriched_cmd =
                    with_assigned_qdisc_handles(cmd, &config, layout, qdisc_handles);
                let old_cmd = if let BakeryCommands::AddCircuit { circuit_hash, .. } =
                    enriched_cmd.as_ref()
                {
                    circuits.get(circuit_hash).cloned()
                } else {
                    None
                };
                if let Some(old_cmd) = old_cmd.as_ref() {
                    enriched_cmd = rotate_changed_qdisc_handles(
                        old_cmd.as_ref(),
                        &enriched_cmd,
                        &config,
                        layout,
                        qdisc_handles,
                    );
                }
                if let Some(old_cmd) = old_cmd.as_ref()
                    && queue_live_migration(
                        old_cmd.as_ref(),
                        &enriched_cmd,
                        circuits,
                        live_circuits,
                        migrations,
                    )
                {
                    continue;
                }
                if let BakeryCommands::AddCircuit { circuit_hash, .. } = enriched_cmd.as_ref() {
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
            if !immediate_commands.is_empty() {
                execute_and_record_live_change(
                    &immediate_commands,
                    "updating circuit speeds (fallback)",
                );
            }
        }

        // 2b) Parent/class migrations
        if !categories.migrated.is_empty() {
            let Some(layout) = resolved_mq_layout.as_ref() else {
                warn!("Bakery: missing MQ layout during circuit migrations");
                return;
            };
            let down_live_snapshot = read_live_class_snapshot(&config.isp_interface()).ok();
            let up_live_snapshot = read_live_class_snapshot(&config.internet_interface()).ok();
            let mut immediate_commands = Vec::new();
            let mut migrated_updates = Vec::new();
            let mut prepared_migrations = Vec::new();
            let mut protected_down_classes = HashSet::new();
            let mut protected_up_classes = HashSet::new();
            for cmd in &categories.migrated {
                let mut enriched_cmd =
                    with_assigned_qdisc_handles(cmd, &config, layout, qdisc_handles);
                let Some(old_cmd) = (if let BakeryCommands::AddCircuit { circuit_hash, .. } =
                    enriched_cmd.as_ref()
                {
                    circuits.get(circuit_hash).cloned()
                } else {
                    None
                }) else {
                    continue;
                };

                enriched_cmd = rotate_changed_qdisc_handles(
                    old_cmd.as_ref(),
                    &enriched_cmd,
                    &config,
                    layout,
                    qdisc_handles,
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
                    let (new_batch, mapped_limit_stats) = filter_batch_by_mapped_circuit_limit(
                        new_batch.clone(),
                        circuits,
                        effective_limit,
                    );
                    log_mapped_limit_decision(
                        "circuit-migration rebuild",
                        mapped_limit,
                        mapped_limit_stats,
                    );
                    full_reload(
                        batch,
                        sites,
                        circuits,
                        live_circuits,
                        mq_layout,
                        qdisc_handles,
                        &config,
                        new_batch,
                        resolved_mq_layout.clone(),
                        stormguard_overrides,
                        virtualized_sites,
                        runtime_node_operations,
                    );
                    MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
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
            let enriched_additions: Vec<Arc<BakeryCommands>> = accepted_additions
                .iter()
                .map(|command| with_assigned_qdisc_handles(command, &config, layout, qdisc_handles))
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
            for command in categories.ip_changed {
                let enriched = with_assigned_qdisc_handles(command, &config, layout, qdisc_handles);
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
    parent_class_id.get_major_minor().1 == 0 && up_parent_class_id.get_major_minor().1 == 0
}

fn site_runtime_virtualization_eligibility_error(site: &BakeryCommands) -> Option<String> {
    let BakeryCommands::AddSite {
        site_hash,
        parent_class_id,
        up_parent_class_id,
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

fn reparent_site_command(
    site: &Arc<BakeryCommands>,
    parent_class_id: TcHandle,
    up_parent_class_id: TcHandle,
) -> Option<Arc<BakeryCommands>> {
    let BakeryCommands::AddSite {
        site_hash,
        class_minor,
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
        class_minor: *class_minor,
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

fn collect_direct_child_site_hashes(
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    target_down: TcHandle,
    target_up: TcHandle,
    target_hash: i64,
) -> Vec<i64> {
    let mut hashes: Vec<i64> = sites
        .iter()
        .filter_map(|(site_hash, site)| {
            let BakeryCommands::AddSite {
                parent_class_id,
                up_parent_class_id,
                ..
            } = site.as_ref()
            else {
                return None;
            };
            (*site_hash != target_hash
                && *parent_class_id == target_down
                && *up_parent_class_id == target_up)
                .then_some(*site_hash)
        })
        .collect();
    hashes.sort_unstable();
    hashes
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

#[allow(clippy::too_many_arguments)]
fn apply_direct_circuit_reparents(
    config: &Arc<Config>,
    circuit_hashes: &[i64],
    new_parent_class_id: TcHandle,
    new_up_parent_class_id: TcHandle,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &HashMap<i64, u64>,
    mq_layout: &Option<MqDeviceLayout>,
    qdisc_handles: &mut QdiscHandleState,
    migrations: &mut HashMap<i64, Migration>,
) -> Result<(), String> {
    if circuit_hashes.is_empty() {
        return Ok(());
    }
    let Some(layout) = mq_layout.as_ref() else {
        return Err("Bakery runtime virtualization requires MQ layout to be available".to_string());
    };

    for circuit_hash in circuit_hashes {
        let Some(old_cmd) = circuits.get(circuit_hash).cloned() else {
            continue;
        };
        let Some(candidate_cmd) =
            reparent_circuit_command(&old_cmd, new_parent_class_id, new_up_parent_class_id)
        else {
            continue;
        };
        if live_circuits.contains_key(circuit_hash)
            && let BakeryCommands::AddCircuit {
                parent_class_id,
                up_parent_class_id,
                ..
            } = candidate_cmd.as_ref()
            && find_free_minor(circuits, parent_class_id, up_parent_class_id).is_none()
        {
            return Err(format!(
                "Unable to queue live migration for active circuit {}: no shadow minor available",
                circuit_hash
            ));
        }
    }

    let mut immediate_commands = Vec::new();
    let mut updated_circuits: Vec<(i64, Arc<BakeryCommands>)> = Vec::new();

    for circuit_hash in circuit_hashes {
        let Some(old_cmd) = circuits.get(circuit_hash).cloned() else {
            continue;
        };
        let Some(candidate_cmd) =
            reparent_circuit_command(&old_cmd, new_parent_class_id, new_up_parent_class_id)
        else {
            continue;
        };
        let enriched_cmd = rotate_changed_qdisc_handles(
            old_cmd.as_ref(),
            &candidate_cmd,
            config,
            layout,
            qdisc_handles,
        );

        if queue_live_migration(
            old_cmd.as_ref(),
            &enriched_cmd,
            circuits,
            live_circuits,
            migrations,
        ) {
            continue;
        }

        let BakeryCommands::AddCircuit { circuit_hash, .. } = enriched_cmd.as_ref() else {
            continue;
        };
        let was_activated = live_circuits.contains_key(circuit_hash);
        if was_activated {
            return Err(format!(
                "Bakery runtime virtualization could not live-migrate active circuit {}",
                circuit_hash
            ));
        }

        match config.queues.lazy_queues.as_ref() {
            None | Some(LazyQueueMode::No) => {
                if let Some(prune) = old_cmd.to_prune(config, true) {
                    immediate_commands.extend(prune);
                }
                if let Some(add) = enriched_cmd.to_commands(config, ExecutionMode::Builder) {
                    immediate_commands.extend(add);
                }
            }
            Some(LazyQueueMode::Htb) => {
                if let Some(prune) = old_cmd.to_prune(config, true) {
                    immediate_commands.extend(prune);
                }
                if let Some(add_htb) = enriched_cmd.to_commands(config, ExecutionMode::Builder) {
                    immediate_commands.extend(add_htb);
                }
            }
            Some(LazyQueueMode::Full) => {}
        }
        updated_circuits.push((*circuit_hash, enriched_cmd));
    }

    if !immediate_commands.is_empty() {
        let result = execute_and_record_live_change(
            &immediate_commands,
            "TreeGuard runtime circuit reparent",
        );
        if !result.ok {
            return Err(summarize_apply_result(
                "TreeGuard runtime circuit reparent",
                &result,
            ));
        }
    }

    for (circuit_hash, command) in updated_circuits {
        circuits.insert(circuit_hash, command);
    }

    Ok(())
}

#[derive(Clone, Debug)]
struct PlannedSiteUpdate {
    queue: u32,
    parent_site: Option<i64>,
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
    let mut dirty_count = 0usize;

    let latest = runtime_node_operations
        .values()
        .inspect(|operation| match operation.status {
            RuntimeNodeOperationStatus::Submitted => submitted_count += 1,
            RuntimeNodeOperationStatus::Deferred => deferred_count += 1,
            RuntimeNodeOperationStatus::Applying => applying_count += 1,
            RuntimeNodeOperationStatus::AppliedAwaitingCleanup => awaiting_cleanup_count += 1,
            RuntimeNodeOperationStatus::Failed => failed_count += 1,
            RuntimeNodeOperationStatus::Dirty => dirty_count += 1,
            RuntimeNodeOperationStatus::Completed => {}
        })
        .max_by_key(|operation| operation.updated_at_unix)
        .map(|operation| BakeryRuntimeOperationHeadlineSnapshot {
            operation_id: operation.operation_id,
            site_hash: operation.site_hash,
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
        let id = planner_site_key(*site_hash);
        planner_items.push(TopLevelPlannerItem {
            id: id.clone(),
            weight: site_max_weight(site.as_ref()),
        });
        prev_assign.insert(id, top_level_bin_name(queue));
    }
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

    let mut future_top_level_roots = Vec::new();
    for site_hash in &top_level_sites {
        if *site_hash != target_site_hash {
            future_top_level_roots.push(*site_hash);
        }
    }
    future_top_level_roots.extend(promoted_site_roots.iter().copied());
    future_top_level_roots.sort_unstable();

    let mut future_top_level_queue = HashMap::new();
    for site_hash in &future_top_level_roots {
        let key = planner_site_key(*site_hash);
        let assigned = planner
            .assignment
            .get(&key)
            .or_else(|| prev_assign.get(&key))
            .ok_or_else(|| format!("Planner did not assign top-level site {}", site_hash))?;
        let queue = queue_from_bin_name(assigned)
            .ok_or_else(|| format!("Invalid planner queue assignment {}", assigned))?;
        future_top_level_queue.insert(*site_hash, queue);
    }

    let mut moved_top_level_roots = Vec::new();
    for site_hash in &future_top_level_roots {
        let Some(site) = sites.get(site_hash) else {
            continue;
        };
        let current_queue = current_site_queue(site.as_ref()).unwrap_or(1);
        let new_queue = future_top_level_queue
            .get(site_hash)
            .copied()
            .unwrap_or(current_queue);
        if *site_hash == target_site_hash
            || promoted_site_roots.contains(site_hash)
            || new_queue != current_queue
        {
            moved_top_level_roots.push(*site_hash);
        }
    }
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

    let (previous_sites, previous_circuits) = build_current_planner_state(sites, circuits);

    let mut future_parent_by_site: HashMap<i64, Option<i64>> = HashMap::new();
    for site_hash in &future_top_level_roots {
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
    let mut planner_site_hashes: Vec<i64> = sites
        .keys()
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
    for (circuit_hash, circuit) in circuits {
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

    let identity = plan_class_identities(
        &planner_site_inputs,
        &planner_circuit_groups,
        &previous_sites,
        &previous_circuits,
        stick_offset,
        0,
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

    let mut active_circuits = HashMap::new();
    for (circuit_hash, circuit) in circuits {
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
    active_circuits.retain(|hash, _| saved_circuits.contains_key(hash));

    Ok(TopLevelVirtualizationPlan {
        saved_sites,
        saved_circuits,
        active_sites,
        active_circuits,
    })
}

fn apply_site_command_updates(
    config: &Arc<Config>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    updates: &HashMap<i64, PlannedSiteUpdate>,
    action_label: &str,
) -> Result<(), String> {
    if updates.is_empty() {
        return Ok(());
    }
    let mut ordered: Vec<_> = updates.iter().collect();
    ordered.sort_by_key(|(_, update)| {
        (
            update.parent_site.is_some(),
            update.queue,
            update.parent_site.unwrap_or_default(),
            site_hash_from_command(update.command.as_ref()).unwrap_or_default(),
        )
    });
    let mut commands = Vec::new();
    for (_, update) in &ordered {
        if let Some(cmds) = update.command.to_commands(config, ExecutionMode::Builder) {
            commands.extend(cmds);
        }
    }
    if !commands.is_empty() {
        let result = execute_and_record_live_change(&commands, action_label);
        if !result.ok {
            return Err(summarize_apply_result(action_label, &result));
        }
    }
    for (site_hash, update) in ordered {
        sites.insert(*site_hash, Arc::clone(&update.command));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_circuit_command_updates(
    config: &Arc<Config>,
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
        let enriched_cmd = rotate_changed_qdisc_handles(
            old_cmd.as_ref(),
            &update.command,
            config,
            layout,
            qdisc_handles,
        );
        if live_circuits.contains_key(circuit_hash)
            && let BakeryCommands::AddCircuit {
                parent_class_id,
                up_parent_class_id,
                ..
            } = enriched_cmd.as_ref()
            && find_free_minor(circuits, parent_class_id, up_parent_class_id).is_none()
        {
            return Err(format!(
                "Unable to queue live migration for active circuit {}: no shadow minor available",
                circuit_hash
            ));
        }
        if queue_live_migration(
            old_cmd.as_ref(),
            &enriched_cmd,
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

#[allow(clippy::too_many_arguments)]
fn apply_top_level_circuit_command_updates(
    config: &Arc<Config>,
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
        let enriched_cmd = rotate_changed_qdisc_handles(
            old_cmd.as_ref(),
            &update.command,
            config,
            layout,
            qdisc_handles,
        );
        if circuit_qdisc_parent_changed(old_cmd.as_ref(), enriched_cmd.as_ref(), config)
            && let BakeryCommands::AddCircuit {
                parent_class_id,
                up_parent_class_id,
                ..
            } = enriched_cmd.as_ref()
            && find_free_minor(circuits, parent_class_id, up_parent_class_id).is_none()
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
        let enriched_cmd = rotate_changed_qdisc_handles(
            old_cmd.as_ref(),
            &update.command,
            config,
            layout,
            qdisc_handles,
        );

        if circuit_qdisc_parent_changed(old_cmd.as_ref(), enriched_cmd.as_ref(), config) {
            if !queue_top_level_runtime_migration(
                old_cmd.as_ref(),
                &enriched_cmd,
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
        hidden_site_hashes.insert(*site_hash);
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

fn runtime_virtualized_site_has_pending_migrations(
    state: &VirtualizedSiteState,
    migrations: &HashMap<i64, Migration>,
) -> bool {
    state
        .active_circuits
        .keys()
        .any(|circuit_hash| migrations.contains_key(circuit_hash))
}

fn runtime_virtualized_site_has_remaining_live_child_classes(
    state: &VirtualizedSiteState,
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
) -> bool {
    let Some((target_down_class, target_up_class)) = site_class_handles(state.site.as_ref()) else {
        return false;
    };

    let child_sites_remaining = sites.values().any(|site| {
        let BakeryCommands::AddSite {
            parent_class_id,
            up_parent_class_id,
            ..
        } = site.as_ref()
        else {
            return false;
        };
        *parent_class_id == target_down_class && *up_parent_class_id == target_up_class
    });
    if child_sites_remaining {
        return true;
    }

    circuits.values().any(|circuit| {
        let BakeryCommands::AddCircuit {
            parent_class_id,
            up_parent_class_id,
            ..
        } = circuit.as_ref()
        else {
            return false;
        };
        *parent_class_id == target_down_class && *up_parent_class_id == target_up_class
    })
}

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
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    migrations: &HashMap<i64, Migration>,
    now_unix: u64,
) -> bool {
    state.pending_prune
        && state.next_prune_attempt_unix <= now_unix
        && !runtime_virtualized_site_has_pending_migrations(state, migrations)
        && !runtime_virtualized_site_has_remaining_live_child_classes(state, sites, circuits)
}

fn runtime_site_prune_missing_qdisc_is_harmless(summary: &str) -> bool {
    summary
        .to_ascii_lowercase()
        .contains("cannot find specified qdisc on specified device")
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

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
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

fn runtime_error_suggests_material_desync(summary: &str) -> bool {
    let lower = summary.to_ascii_lowercase();
    lower.contains("rtnetlink")
        || lower.contains("specified class not found")
        || lower.contains("cannot find specified qdisc")
        || lower.contains("cannot move an existing qdisc")
        || lower.contains("device or resource busy")
}

fn update_desync_state_from_runtime_operations(
    runtime_node_operations: &HashMap<i64, RuntimeNodeOperation>,
) {
    let snapshot = rebuild_runtime_operations_snapshot(runtime_node_operations);
    let dirty_count = snapshot.dirty_count;
    let runtime_operations_by_site = runtime_node_operations
        .iter()
        .map(|(site_hash, operation)| (*site_hash, operation.snapshot()))
        .collect();
    {
        let mut state = telemetry_state().write();
        state.runtime_operations = snapshot;
        state.dirty_subtree_count = dirty_count;
        state.runtime_operations_by_site = runtime_operations_by_site;
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
    sites: &HashMap<i64, Arc<BakeryCommands>>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    virtualized_sites: &mut HashMap<i64, VirtualizedSiteState>,
    migrations: &HashMap<i64, Migration>,
    runtime_node_operations: &mut HashMap<i64, RuntimeNodeOperation>,
) {
    if bakery_reload_required_reason().is_some() {
        return;
    }
    let now_unix = unix_now();
    let pending_site_hashes: Vec<i64> = virtualized_sites
        .iter()
        .filter_map(|(site_hash, state)| {
            runtime_virtualized_site_prune_ready(state, sites, circuits, migrations, now_unix)
                .then_some(*site_hash)
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
                    let retry_at = now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS);
                    state.next_prune_attempt_unix = retry_at;
                    if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                        operation.attempt_count = operation.attempt_count.saturating_add(1);
                        if operation.attempt_count >= RUNTIME_SITE_PRUNE_MAX_ATTEMPTS {
                            state.pending_prune = false;
                            state.next_prune_attempt_unix = 0;
                            operation.update_status(
                                RuntimeNodeOperationStatus::Dirty,
                                now_unix,
                                Some(summary.clone()),
                                None,
                            );
                            push_bakery_event(
                                "runtime_site_prune_dirty",
                                "error",
                                format!(
                                    "Deferred runtime site prune for site {} marked Dirty after snapshot failure: {}",
                                    site_hash, summary
                                ),
                            );
                        } else {
                            operation.update_status(
                                RuntimeNodeOperationStatus::AppliedAwaitingCleanup,
                                now_unix,
                                Some(summary.clone()),
                                Some(retry_at),
                            );
                            push_bakery_event(
                                "runtime_site_prune_retry",
                                "warning",
                                format!(
                                    "Deferred runtime site prune retry {}/{} for site {} postponed: {}",
                                    operation.attempt_count,
                                    RUNTIME_SITE_PRUNE_MAX_ATTEMPTS,
                                    site_hash,
                                    summary
                                ),
                            );
                        }
                    }
                }
            }
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
                    let retry_at = now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS);
                    state.next_prune_attempt_unix = retry_at;
                    if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                        operation.attempt_count = operation.attempt_count.saturating_add(1);
                        if operation.attempt_count >= RUNTIME_SITE_PRUNE_MAX_ATTEMPTS {
                            state.pending_prune = false;
                            state.next_prune_attempt_unix = 0;
                            operation.update_status(
                                RuntimeNodeOperationStatus::Dirty,
                                now_unix,
                                Some(summary.clone()),
                                None,
                            );
                            push_bakery_event(
                                "runtime_site_prune_dirty",
                                "error",
                                format!(
                                    "Deferred runtime site prune for site {} marked Dirty after snapshot failure: {}",
                                    site_hash, summary
                                ),
                            );
                        } else {
                            operation.update_status(
                                RuntimeNodeOperationStatus::AppliedAwaitingCleanup,
                                now_unix,
                                Some(summary.clone()),
                                Some(retry_at),
                            );
                            push_bakery_event(
                                "runtime_site_prune_retry",
                                "warning",
                                format!(
                                    "Deferred runtime site prune retry {}/{} for site {} postponed: {}",
                                    operation.attempt_count,
                                    RUNTIME_SITE_PRUNE_MAX_ATTEMPTS,
                                    site_hash,
                                    summary
                                ),
                            );
                        }
                    }
                }
            }
            return;
        }
    };

    for site_hash in pending_site_hashes {
        let Some(state) = virtualized_sites.get_mut(&site_hash) else {
            continue;
        };
        sync_runtime_virtualized_site_qdisc_handles_from_live_snapshot(
            state,
            &down_snapshot,
            &up_snapshot,
        );
        if runtime_virtualized_site_has_remaining_observed_child_classes(
            state,
            &down_snapshot,
            &up_snapshot,
        ) {
            let retry_at = now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS);
            state.next_prune_attempt_unix = retry_at;
            if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                operation.update_status(
                    RuntimeNodeOperationStatus::AppliedAwaitingCleanup,
                    now_unix,
                    Some("Observed live child classes still attached".to_string()),
                    Some(retry_at),
                );
            }
            continue;
        }
        if site_prune_commands(config, state).is_none() {
            state.pending_prune = false;
            if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                operation.update_status(
                    RuntimeNodeOperationStatus::Completed,
                    now_unix,
                    None,
                    None,
                );
            }
            continue;
        }

        match execute_runtime_site_prune(config, state, &down_snapshot, &up_snapshot) {
            Ok(()) => {
                state.pending_prune = false;
                state.next_prune_attempt_unix = 0;
                if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                    operation.update_status(
                        RuntimeNodeOperationStatus::Completed,
                        now_unix,
                        None,
                        None,
                    );
                }
                push_bakery_event(
                    "runtime_site_prune_completed",
                    "info",
                    format!(
                        "Deferred runtime site prune completed for site {}.",
                        site_hash
                    ),
                );
            }
            Err(summary) => {
                if let Some(operation) = runtime_node_operations.get_mut(&site_hash) {
                    operation.attempt_count = operation.attempt_count.saturating_add(1);
                    if operation.attempt_count >= RUNTIME_SITE_PRUNE_MAX_ATTEMPTS {
                        state.pending_prune = false;
                        state.next_prune_attempt_unix = 0;
                        operation.update_status(
                            RuntimeNodeOperationStatus::Dirty,
                            now_unix,
                            Some(summary.clone()),
                            None,
                        );
                        push_bakery_event(
                            "runtime_site_prune_dirty",
                            "error",
                            format!(
                                "Deferred runtime site prune for site {} marked Dirty after {} attempts: {}",
                                site_hash, operation.attempt_count, summary
                            ),
                        );
                    } else {
                        let retry_at = now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS);
                        state.next_prune_attempt_unix = retry_at;
                        operation.update_status(
                            RuntimeNodeOperationStatus::AppliedAwaitingCleanup,
                            now_unix,
                            Some(summary.clone()),
                            Some(retry_at),
                        );
                        push_bakery_event(
                            "runtime_site_prune_retry",
                            "warning",
                            format!(
                                "Deferred runtime site prune retry {}/{} for site {} failed: {}",
                                operation.attempt_count,
                                RUNTIME_SITE_PRUNE_MAX_ATTEMPTS,
                                site_hash,
                                summary
                            ),
                        );
                    }
                } else {
                    state.next_prune_attempt_unix =
                        now_unix.saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS);
                    push_bakery_event(
                        "runtime_site_prune_retry",
                        "warning",
                        format!(
                            "Deferred runtime site prune for site {} failed outside operation tracking: {}",
                            site_hash, summary
                        ),
                    );
                }
                warn!(
                    "Bakery: deferred runtime site prune for {} failed again: {}",
                    site_hash, summary
                );
            }
        }
    }
    update_desync_state_from_runtime_operations(runtime_node_operations);
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
    if let Some(reason) = bakery_reload_required_reason() {
        let snapshot = if let Some(existing) = runtime_node_operations.get(&site_hash)
            && existing.action == action
        {
            existing.snapshot()
        } else {
            let operation_id = *next_runtime_operation_id;
            *next_runtime_operation_id = next_runtime_operation_id.saturating_add(1);
            let mut operation =
                RuntimeNodeOperation::new(operation_id, site_hash, action, now_unix);
            operation.attempt_count = 1;
            operation.update_status(
                RuntimeNodeOperationStatus::Dirty,
                now_unix,
                Some(reason.clone()),
                None,
            );
            runtime_node_operations.insert(site_hash, operation.clone());
            update_desync_state_from_runtime_operations(runtime_node_operations);
            operation.snapshot()
        };
        return snapshot;
    }
    if let Some(existing) = runtime_node_operations.get(&site_hash)
        && runtime_node_operation_is_active(existing.status)
    {
        return existing.snapshot();
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
        let mut operation = RuntimeNodeOperation::new(operation_id, site_hash, action, now_unix);
        operation.attempt_count = runtime_node_operations
            .get(&site_hash)
            .map(|existing| existing.attempt_count.saturating_add(1))
            .unwrap_or(1);
        let summary = format!(
            "Bakery runtime node operation capacity ({}) is saturated; deferring TreeGuard {} for site {}.",
            RUNTIME_NODE_OPERATION_CAPACITY,
            if virtualized {
                "virtualization"
            } else {
                "restore"
            },
            site_hash
        );
        operation.update_status(
            RuntimeNodeOperationStatus::Deferred,
            now_unix,
            Some(summary.clone()),
            Some(retry_at),
        );
        runtime_node_operations.insert(site_hash, operation.clone());
        update_desync_state_from_runtime_operations(runtime_node_operations);
        push_bakery_event("runtime_node_op_deferred", "warning", summary);
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

    let mut operation = RuntimeNodeOperation::new(operation_id, site_hash, action, now_unix);
    operation.attempt_count = 1;
    operation.update_status(RuntimeNodeOperationStatus::Applying, now_unix, None, None);
    runtime_node_operations.insert(site_hash, operation.clone());
    update_desync_state_from_runtime_operations(runtime_node_operations);

    let Ok(config) = lqos_config::load_config() else {
        operation.update_status(
            RuntimeNodeOperationStatus::Failed,
            now_unix,
            Some("Failed to load configuration".to_string()),
            None,
        );
        runtime_node_operations.insert(site_hash, operation.clone());
        update_desync_state_from_runtime_operations(runtime_node_operations);
        return operation.snapshot();
    };

    let result: Result<(), String> = (|| {
        if virtualized {
            if virtualized_sites.contains_key(&site_hash) {
                return Ok(());
            }

            let Some(target_site) = sites.get(&site_hash).cloned() else {
                return Err(format!("Unknown site hash {}", site_hash));
            };

            if site_is_top_level(target_site.as_ref()) {
                let plan = build_top_level_virtualization_plan(
                    Arc::clone(&target_site),
                    sites,
                    circuits,
                    site_stick_offset(target_site.as_ref()),
                )?;
                apply_site_command_updates(
                    &config,
                    sites,
                    &plan.active_sites,
                    "TreeGuard runtime top-level site reparent",
                )?;
                apply_top_level_circuit_command_updates(
                    &config,
                    circuits,
                    &plan.active_circuits,
                    live_circuits,
                    mq_layout,
                    qdisc_handles,
                    migrations,
                    "TreeGuard runtime top-level circuit reparent",
                )?;
                sites.remove(&site_hash);
                virtualized_sites.insert(
                    site_hash,
                    VirtualizedSiteState {
                        site: target_site,
                        saved_sites: plan.saved_sites,
                        saved_circuits: plan.saved_circuits,
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
                        qdisc_handles: VirtualizedSiteQdiscHandles {
                            down: None,
                            up: None,
                        },
                        pending_prune: true,
                        next_prune_attempt_unix: now_unix
                            .saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS),
                    },
                );
                return Ok(());
            }

            if let Some(reason) =
                site_runtime_virtualization_eligibility_error(target_site.as_ref())
            {
                return Err(reason);
            }

            let Some((target_down_class, target_up_class)) =
                site_class_handles(target_site.as_ref())
            else {
                return Err(format!("Site {} is not a valid AddSite command", site_hash));
            };
            let (parent_class_id, up_parent_class_id) = match target_site.as_ref() {
                BakeryCommands::AddSite {
                    parent_class_id,
                    up_parent_class_id,
                    ..
                } => (parent_class_id, up_parent_class_id),
                _ => unreachable!("site_class_handles succeeded for a non-site command"),
            };

            let child_sites = collect_direct_child_site_hashes(
                sites,
                target_down_class,
                target_up_class,
                site_hash,
            );
            let direct_circuits =
                collect_direct_circuit_hashes(circuits, target_down_class, target_up_class);
            let saved_sites: HashMap<i64, Arc<BakeryCommands>> = child_sites
                .iter()
                .filter_map(|hash| sites.get(hash).cloned().map(|cmd| (*hash, cmd)))
                .collect();
            let saved_circuits: HashMap<i64, Arc<BakeryCommands>> = direct_circuits
                .iter()
                .filter_map(|hash| circuits.get(hash).cloned().map(|cmd| (*hash, cmd)))
                .collect();

            let mut active_sites = HashMap::new();
            for child_hash in &child_sites {
                let Some(child_site) = sites.get(child_hash).cloned() else {
                    continue;
                };
                let Some(updated_site) =
                    reparent_site_command(&child_site, *parent_class_id, *up_parent_class_id)
                else {
                    continue;
                };
                active_sites.insert(
                    *child_hash,
                    PlannedSiteUpdate {
                        queue: current_site_queue(updated_site.as_ref()).unwrap_or(1),
                        parent_site: Some(site_hash),
                        command: updated_site,
                    },
                );
            }
            apply_site_command_updates(
                &config,
                sites,
                &active_sites,
                "TreeGuard runtime site reparent",
            )?;

            apply_direct_circuit_reparents(
                &config,
                &direct_circuits,
                *parent_class_id,
                *up_parent_class_id,
                circuits,
                live_circuits,
                mq_layout,
                qdisc_handles,
                migrations,
            )?;
            sites.remove(&site_hash);
            virtualized_sites.insert(
                site_hash,
                VirtualizedSiteState {
                    site: target_site,
                    saved_sites,
                    saved_circuits,
                    active_sites: active_sites
                        .into_iter()
                        .map(|(hash, update)| (hash, update.command))
                        .collect(),
                    active_circuits: direct_circuits
                        .iter()
                        .filter_map(|hash| circuits.get(hash).cloned().map(|cmd| (*hash, cmd)))
                        .collect(),
                    qdisc_handles: VirtualizedSiteQdiscHandles {
                        down: None,
                        up: None,
                    },
                    pending_prune: true,
                    next_prune_attempt_unix: now_unix
                        .saturating_add(RUNTIME_SITE_PRUNE_RETRY_SECONDS),
                },
            );
            return Ok(());
        }

        let Some(saved_state) = virtualized_sites.get(&site_hash).cloned() else {
            return Ok(());
        };

        if !site_is_top_level(saved_state.site.as_ref())
            && let Some(reason) =
                site_runtime_virtualization_eligibility_error(saved_state.site.as_ref())
        {
            return Err(reason);
        }

        if !saved_state.pending_prune
            && let Some(cmds) = saved_state
                .site
                .to_commands(&config, ExecutionMode::Builder)
        {
            let result =
                execute_and_record_live_change(&cmds, "TreeGuard runtime hidden site restore");
            if !result.ok {
                return Err(summarize_apply_result(
                    "TreeGuard runtime hidden site restore",
                    &result,
                ));
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
                        command: Arc::clone(command),
                    },
                )
            })
            .collect();
        apply_site_command_updates(
            &config,
            sites,
            &restore_sites,
            "TreeGuard runtime site restore",
        )?;

        let restore_circuits: HashMap<i64, PlannedCircuitUpdate> = saved_state
            .saved_circuits
            .iter()
            .map(|(hash, command)| {
                (
                    *hash,
                    PlannedCircuitUpdate {
                        queue: current_circuit_queue(command.as_ref()).unwrap_or(1),
                        parent_site: None,
                        command: Arc::clone(command),
                    },
                )
            })
            .collect();
        apply_circuit_command_updates(
            &config,
            circuits,
            &restore_circuits,
            live_circuits,
            mq_layout,
            qdisc_handles,
            migrations,
            "TreeGuard runtime circuit restore",
        )?;

        virtualized_sites.remove(&site_hash);
        Ok(())
    })();

    if let Err(error) = result {
        *sites = sites_snapshot;
        *circuits = circuits_snapshot;
        *qdisc_handles = qdisc_handles_snapshot;
        *migrations = migrations_snapshot;
        *virtualized_sites = virtualized_sites_snapshot;
        *runtime_node_operations = runtime_ops_snapshot;
        operation.update_status(
            RuntimeNodeOperationStatus::Failed,
            unix_now(),
            Some(error.clone()),
            None,
        );
        runtime_node_operations.insert(site_hash, operation.clone());
        if runtime_error_suggests_material_desync(&error) {
            mark_reload_required(format!(
                "Bakery detected material runtime drift while processing TreeGuard {} for site {}: {}",
                if virtualized {
                    "virtualization"
                } else {
                    "restore"
                },
                site_hash,
                error
            ));
        }
        update_desync_state_from_runtime_operations(runtime_node_operations);
        return operation.snapshot();
    }

    let finished_at = unix_now();
    let next_retry = virtualized_sites
        .get(&site_hash)
        .and_then(|state| state.pending_prune.then_some(state.next_prune_attempt_unix));
    let status = if virtualized && next_retry.is_some() {
        RuntimeNodeOperationStatus::AppliedAwaitingCleanup
    } else {
        RuntimeNodeOperationStatus::Completed
    };
    operation.update_status(status, finished_at, None, next_retry);
    runtime_node_operations.insert(site_hash, operation.clone());
    update_desync_state_from_runtime_operations(runtime_node_operations);
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
) {
    warn!("Bakery: Full reload triggered due to site or circuit changes.");
    FULL_RELOAD_IN_PROGRESS.store(true, Ordering::Relaxed);
    mark_bakery_action_started(
        BakeryMode::ApplyingFullReload,
        "full_reload_started",
        "Full reload triggered due to site or circuit changes.".to_string(),
    );
    let _reload_scope = FullReloadScope;
    let previous_sites = sites.clone();
    let previous_circuits = circuits.clone();
    let previous_live_circuits = live_circuits.clone();
    let previous_mq_layout = mq_layout.clone();

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
        if resolved_mq_layout.is_some() {
            *mq_layout = resolved_mq_layout;
        }
        virtualized_sites.clear();
        runtime_node_operations.clear();
        update_desync_state_from_runtime_operations(runtime_node_operations);
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
    let _ = config; // currently unused but kept for future interface-specific logic
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
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn bakery_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn reset_bakery_test_state() {
        *telemetry_state().write() = BakeryTelemetryState::default();
        MQ_CREATED.store(false, Ordering::Relaxed);
        FIRST_COMMIT_APPLIED.store(false, Ordering::Relaxed);
        FULL_RELOAD_IN_PROGRESS.store(false, Ordering::Relaxed);
    }

    fn mk_add_circuit(hash: i64, ip_addresses: &str) -> Arc<BakeryCommands> {
        Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: hash,
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
                qdisc_handles: VirtualizedSiteQdiscHandles {
                    down: None,
                    up: None,
                },
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
    fn runtime_virtualized_site_prune_ready_respects_backoff_and_migrations() {
        let state = VirtualizedSiteState {
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::from([(40, mk_add_circuit(40, "192.0.2.40/32"))]),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
            pending_prune: true,
            next_prune_attempt_unix: 120,
        };

        let mut migrations = HashMap::new();
        migrations.insert(
            40,
            Migration {
                circuit_hash: 40,
                old_parent_class_id: TcHandle::from_u32(0x1),
                old_up_parent_class_id: TcHandle::from_u32(0x2),
                old_class_major: 0x100,
                old_up_class_major: 0x200,
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
            },
        );

        assert!(!runtime_virtualized_site_prune_ready(
            &state,
            &HashMap::new(),
            &HashMap::new(),
            &migrations,
            119
        ));
        assert!(!runtime_virtualized_site_prune_ready(
            &state,
            &HashMap::new(),
            &HashMap::new(),
            &migrations,
            120
        ));

        migrations.clear();
        assert!(!runtime_virtualized_site_prune_ready(
            &state,
            &HashMap::new(),
            &HashMap::new(),
            &migrations,
            119
        ));
        assert!(runtime_virtualized_site_prune_ready(
            &state,
            &HashMap::new(),
            &HashMap::new(),
            &migrations,
            120
        ));
    }

    #[test]
    fn runtime_virtualized_site_prune_ready_requires_no_remaining_live_children() {
        let state = VirtualizedSiteState {
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
            pending_prune: true,
            next_prune_attempt_unix: 0,
        };

        let child_site = mk_add_site(30, 0x10021, 0x20021, 0x22);
        let mut sites = HashMap::new();
        sites.insert(30, child_site);
        assert!(!runtime_virtualized_site_prune_ready(
            &state,
            &sites,
            &HashMap::new(),
            &HashMap::new(),
            0
        ));

        sites.clear();
        let mut circuits = HashMap::new();
        circuits.insert(
            40,
            Arc::new(BakeryCommands::AddCircuit {
                circuit_hash: 40,
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
            }),
        );
        assert!(!runtime_virtualized_site_prune_ready(
            &state,
            &HashMap::new(),
            &circuits,
            &HashMap::new(),
            0
        ));

        circuits.clear();
        assert!(runtime_virtualized_site_prune_ready(
            &state,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            0
        ));
    }

    #[test]
    fn site_prune_commands_delete_qdisc_by_handle_before_class() {
        let config = Arc::new(lqos_config::Config::default());
        let state = VirtualizedSiteState {
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: Some(0x9000),
                up: Some(0x9001),
            },
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
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
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
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
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
            site: mk_add_site(20, 0x10020, 0x20020, 0x21),
            saved_sites: HashMap::new(),
            saved_circuits: HashMap::new(),
            active_sites: HashMap::new(),
            active_circuits: HashMap::new(),
            qdisc_handles: VirtualizedSiteQdiscHandles {
                down: None,
                up: None,
            },
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
    fn top_level_virtualization_plan_promotes_children_and_direct_circuits() {
        let target_site = mk_add_site(20, 0x10000, 0x20000, 0x21);
        let sibling_site = mk_add_site(10, 0x10000, 0x20000, 0x20);
        let child_site = mk_add_site(30, 0x10021, 0x20021, 0x22);
        let grandchild_site = mk_add_site(31, 0x10022, 0x20022, 0x23);
        let direct_circuit = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 40,
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

        let plan =
            build_top_level_virtualization_plan(Arc::clone(&target_site), &sites, &circuits, 1)
                .expect("top-level plan should build");

        assert!(plan.saved_sites.contains_key(&30));
        assert!(plan.saved_sites.contains_key(&31));
        assert!(plan.saved_circuits.contains_key(&40));

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
    }

    #[test]
    fn site_runtime_virtualization_rejects_top_level_sites() {
        let site = mk_add_site(99, 0x10000, 0x20000, 0x21);
        let reason = site_runtime_virtualization_eligibility_error(site.as_ref())
            .expect("top-level site should be rejected");
        assert!(reason.contains("top-level"));
    }

    #[test]
    fn site_runtime_virtualization_accepts_normal_same_queue_site() {
        let site = mk_add_site(77, 0x10020, 0x20020, 0x22);
        assert!(site_runtime_virtualization_eligibility_error(site.as_ref()).is_none());
    }

    #[test]
    fn site_runtime_virtualization_rejects_non_site_commands() {
        let circuit = mk_add_circuit(77, "192.0.2.77/32");
        let reason = site_runtime_virtualization_eligibility_error(circuit.as_ref())
            .expect("non-site commands should be rejected");
        assert!(reason.contains("AddSite command"));
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
                RuntimeNodeOperationAction::Virtualize,
                unix_now(),
            );
            operation.attempt_count = 1;
            operation.update_status(RuntimeNodeOperationStatus::Applying, unix_now(), None, None);
            runtime_node_operations.insert(site_hash, operation);
        }
        update_desync_state_from_runtime_operations(&runtime_node_operations);

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
    fn dirty_runtime_subtrees_below_threshold_do_not_require_full_reload() {
        let _guard = bakery_test_lock().lock().expect("test lock");
        reset_bakery_test_state();

        let mut runtime_node_operations = HashMap::new();
        for index in 0..(RUNTIME_DIRTY_SUBTREE_RELOAD_THRESHOLD - 1) {
            let site_hash = index as i64 + 1;
            let mut operation = RuntimeNodeOperation::new(
                site_hash as u64,
                site_hash,
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

        update_desync_state_from_runtime_operations(&runtime_node_operations);

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

        update_desync_state_from_runtime_operations(&runtime_node_operations);

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
    fn qdisc_budget_estimate_skips_circuit_leaf_qdiscs_in_monitor_only_mode() {
        let mut cfg = Config::default();
        cfg.queues.monitor_only = true;
        let config = Arc::new(cfg);
        let queue = vec![
            BakeryCommands::MqSetup {
                queues_available: 1,
                stick_offset: 0,
            },
            BakeryCommands::AddCircuit {
                circuit_hash: 2,
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
        assert_eq!(estimate.interfaces.get("eth1"), Some(&4));
        assert_eq!(estimate.interfaces.get("eth0"), Some(&4));
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
    fn qdisc_budget_estimate_counts_fq_codel_leaf_qdiscs_separately() {
        let config = Arc::new(Config::default());
        let queue = vec![
            BakeryCommands::MqSetup {
                queues_available: 1,
                stick_offset: 0,
            },
            BakeryCommands::AddCircuit {
                circuit_hash: 2,
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
    fn queue_live_migration_succeeds_for_active_parent_move() {
        let old_cmd = mk_test_circuit(9001, 0x10020, 0x20020, 0x21, 0x1, 0x2, "192.0.2.90/32");
        let new_cmd = mk_test_circuit(9001, 0x10034, 0x20034, 0x35, 0x3, 0x4, "192.0.2.90/32");

        let mut circuits = HashMap::from([(9001, Arc::clone(&old_cmd))]);
        let live_circuits = HashMap::from([(9001, 1u64)]);
        let mut migrations = HashMap::new();

        let queued = queue_live_migration(
            old_cmd.as_ref(),
            &new_cmd,
            &mut circuits,
            &live_circuits,
            &mut migrations,
        );

        assert!(queued);
        let migration = migrations.get(&9001).expect("migration should be queued");
        assert_eq!(migration.stage, MigrationStage::PrepareShadow);
        assert_eq!(migration.old_parent_class_id, TcHandle::from_u32(0x10020));
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
    fn pending_migration_overlay_uses_desired_state_for_diff_only() {
        let old_cmd = mk_test_circuit(9300, 0x10020, 0x20020, 0x21, 0x1, 0x2, "192.0.2.94/32");
        let new_cmd = mk_test_circuit(9300, 0x10034, 0x20034, 0x35, 0x3, 0x4, "192.0.2.94/32");

        let mut circuits = HashMap::from([(9300, Arc::clone(&old_cmd))]);
        let live_circuits = HashMap::from([(9300, 1u64)]);
        let mut migrations = HashMap::new();

        assert!(queue_live_migration(
            old_cmd.as_ref(),
            &new_cmd,
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
        clear_reload_required(
            "reset before migration_stage_failure_marks_reload_required_and_stops_migration",
        );
        let mut migration = Migration {
            circuit_hash: 9002,
            old_parent_class_id: TcHandle::from_u32(0x10020),
            old_up_parent_class_id: TcHandle::from_u32(0x20020),
            old_class_major: 0x1,
            old_up_class_major: 0x2,
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
            MigrationStage::SwapToFinal,
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
        clear_reload_required(
            "reset before migration_stage_success_advances_without_reload_required",
        );
        let mut migration = Migration {
            circuit_hash: 9003,
            old_parent_class_id: TcHandle::from_u32(0x10020),
            old_up_parent_class_id: TcHandle::from_u32(0x20020),
            old_class_major: 0x1,
            old_up_class_major: 0x2,
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
            MigrationStage::SwapToFinal,
        );

        assert!(advanced);
        assert_eq!(migration.stage, MigrationStage::SwapToFinal);
        assert!(bakery_reload_required_reason().is_none());
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
        let live_circuits = HashMap::new();
        let mut migrations = HashMap::new();

        let queued = queue_top_level_runtime_migration(
            old_cmd.as_ref(),
            &new_cmd,
            &mut circuits,
            &live_circuits,
            &mut migrations,
        );

        assert!(queued);
        let migration = migrations.get(&9200).expect("migration should be queued");
        assert_eq!(migration.stage, MigrationStage::PrepareShadow);
        assert_eq!(migration.old_parent_class_id, TcHandle::from_u32(0x10020));
        assert_eq!(migration.parent_class_id, TcHandle::from_u32(0x10034));
        assert_eq!(migration.final_minor, 0x35);
        assert_ne!(migration.shadow_minor, migration.old_minor);
        assert!(matches!(
            circuits.get(&9200),
            Some(cmd) if Arc::ptr_eq(cmd, &old_cmd)
        ));
        assert!(Arc::ptr_eq(&migration.desired_cmd, &new_cmd));
    }

    #[test]
    fn full_reload_allocation_uses_fresh_handles_reserved_from_live_tree() {
        let config = test_config_with_runtime_dir("fresh-full-reload");
        let layout = MqDeviceLayout::from_setup(&config, 2, 0);
        let mut persisted = QdiscHandleState::default();

        let original = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: 505,
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

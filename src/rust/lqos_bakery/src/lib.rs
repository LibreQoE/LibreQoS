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

use crossbeam_channel::{Receiver, Sender};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tracing::{debug, error, info, warn};
use utils::current_timestamp;
pub(crate) const CHANNEL_CAPACITY: usize = 65536; // 64k capacity for Bakery commands
use crate::commands::ExecutionMode;
use crate::diff::{CircuitDiffResult, SiteDiffResult, diff_circuits, diff_sites};
use crate::qdisc_handles::QdiscHandleState;
use crate::queue_math::{SqmKind, effective_sqm_kind, format_rate_for_tc_f32};
use crate::utils::{ExecuteResult, execute_in_memory, write_command_file};
pub use commands::BakeryCommands;
use lqos_bus::{
    BusRequest, BusResponse, InsightLicenseSummary, LibreqosBusClient, TcHandle, UrgentSeverity,
    UrgentSource,
};
use lqos_config::{Config, LazyQueueMode};
use qdisc_handles::MqDeviceLayout;
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
    // Parent handles and majors
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
    stage: MigrationStage,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StormguardOverrideKey {
    interface: String,
    class: TcHandle,
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

/// Count of Bakery-Managed circuits that are currently active.
pub static ACTIVE_CIRCUITS: AtomicUsize = AtomicUsize::new(0);
/// True while Bakery is applying a full reload batch to `tc`.
static FULL_RELOAD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
/// Hard kernel limit for auto-allocated qdisc handles on a single network interface.
pub const HARD_QDISC_HANDLE_LIMIT_PER_INTERFACE: usize = 65_534;
/// Conservative operational limit used to fail a full reload before qdisc handle exhaustion.
pub const SAFE_QDISC_BUDGET_PER_INTERFACE: usize = 65_000;
/// Maximum number of mapped circuits allowed without Insight.
const DEFAULT_MAPPED_CIRCUITS_LIMIT: usize = 1000;
/// Minimum interval between repeated mapped-circuit-limit urgent issues.
const CIRCUIT_LIMIT_URGENT_INTERVAL_SECONDS: u64 = 30 * 60;
/// Last timestamp at which we emitted a mapped-circuit-limit urgent issue.
static LAST_CIRCUIT_LIMIT_URGENT_TS: AtomicU64 = AtomicU64::new(0);
const BAKERY_EVENT_LIMIT: usize = 50;

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
    /// Last qdisc preflight summary known to Bakery.
    pub preflight: Option<BakeryPreflightSnapshot>,
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
    last_success_unix: Option<u64>,
    last_failure_unix: Option<u64>,
    last_failure_summary: Option<String>,
    last_apply_type: BakeryApplyType,
    last_total_tc_commands: usize,
    last_class_commands: usize,
    last_qdisc_commands: usize,
    last_build_duration_ms: u64,
    last_apply_duration_ms: u64,
    preflight: Option<BakeryPreflightSnapshot>,
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
            last_success_unix: None,
            last_failure_unix: None,
            last_failure_summary: None,
            last_apply_type: BakeryApplyType::None,
            last_total_tc_commands: 0,
            last_class_commands: 0,
            last_qdisc_commands: 0,
            last_build_duration_ms: 0,
            last_apply_duration_ms: 0,
            preflight: None,
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

fn mark_bakery_action_started(mode: BakeryMode, event: &str, summary: String) {
    let ts = current_timestamp();
    {
        let mut state = telemetry_state().write();
        state.mode = mode;
        state.current_action_started_unix = Some(ts);
    }
    push_bakery_event(event, "info", summary);
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
        preflight: state.preflight,
    }
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

fn execute_and_record_live_change(
    command_buffer: &Vec<Vec<String>>,
    purpose: &str,
) -> ExecuteResult {
    let (total, class_commands, qdisc_commands) = count_tc_command_types(command_buffer);
    mark_bakery_action_started(
        BakeryMode::ApplyingLiveChange,
        "live_change_started",
        format!("{purpose}: started"),
    );
    let result = execute_in_memory(command_buffer, purpose);
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
pub struct QdiscBudgetEstimate {
    /// Estimated total qdisc count grouped by network interface name.
    pub interfaces: BTreeMap<String, usize>,
    /// Conservative operational limit enforced before a full reload is committed.
    pub safe_budget: usize,
    /// Kernel hard limit for the per-device qdisc-handle namespace.
    pub hard_limit: usize,
}

impl QdiscBudgetEstimate {
    /// Returns `true` when all planned per-interface counts fit within the safe budget.
    pub fn ok(&self) -> bool {
        self.interfaces
            .values()
            .all(|count| *count <= self.safe_budget)
    }
}

fn find_arg_value<'a>(argv: &'a [String], key: &str) -> Option<&'a str> {
    argv.windows(2)
        .find_map(|pair| (pair[0] == key).then_some(pair[1].as_str()))
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
                *interfaces.entry(dev).or_insert(0) += 1;
            }
        }
    }

    QdiscBudgetEstimate {
        interfaces,
        safe_budget: SAFE_QDISC_BUDGET_PER_INTERFACE,
        hard_limit: HARD_QDISC_HANDLE_LIMIT_PER_INTERFACE,
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
    let BakeryCommands::AddCircuit { circuit_hash, .. } = command.as_ref() else {
        return Arc::clone(command);
    };

    if config.queues.monitor_only {
        return Arc::clone(command);
    }

    let mut enriched = command.as_ref().clone();
    let isp_interface = config.isp_interface();
    let internet_interface = config.internet_interface();
    let isp_reserved = mq_layout.reserved_handles(&isp_interface);
    let up_reserved = mq_layout.reserved_handles(&internet_interface);

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
    }

    while let Ok(command) = rx.recv() {
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

                // Advance live-move migrations
                let Ok(config) = lqos_config::load_config() else {
                    error!("Failed to load configuration, exiting Bakery thread.");
                    continue;
                };
                let mut advanced = 0usize;
                let mut to_remove = Vec::new();
                for (_hash, mig) in migrations.iter_mut() {
                    if advanced >= MIGRATIONS_PER_TICK {
                        break;
                    }
                    match mig.stage {
                        MigrationStage::PrepareShadow => {
                            // Create shadow HTB+CAKE with OLD rates
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
                                        if let Some(c) = add_commands_for_circuit(
                                            &temp,
                                            &config,
                                            ExecutionMode::Builder,
                                        ) {
                                            cmds.extend(c);
                                        }
                                    }
                                    Some(LazyQueueMode::Htb) => {
                                        if let Some(c) = add_commands_for_circuit(
                                            &temp,
                                            &config,
                                            ExecutionMode::Builder,
                                        ) {
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
                                    execute_and_record_live_change(
                                        &cmds,
                                        "live-move: create shadow",
                                    );
                                }
                                mig.stage = MigrationStage::SwapToShadow;
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
                            // Remap all IPs to shadow handles using existing CPU.
                            // Upload mapping is derived in the dataplane for on-a-stick mode.
                            for ip in &mig.ips {
                                let (ip_s, prefix) = parse_ip_and_prefix(ip);
                                let key = MappingKey {
                                    ip: ip_s.clone(),
                                    prefix,
                                };
                                let cpu = mapping_current.get(&key).map(|v| v.cpu).unwrap_or(0);
                                let handle =
                                    tc_handle_from_major_minor(mig.class_major, mig.shadow_minor);
                                let _ = lqos_sys::add_ip_to_tc(&ip_s, handle, cpu, false, 0, 0);
                                // Update local mapping view
                                mapping_current.insert(key, MappingVal { handle, cpu });
                            }
                            // Clear the hot cache directly
                            let _ = lqos_sys::clear_hot_cache();
                            mig.stage = MigrationStage::BuildFinal;
                            advanced += 1;
                        }
                        MigrationStage::BuildFinal => {
                            // Delete old classes/qdiscs and create final with NEW rates at original minor
                            if let Some(old_cmd) = build_temp_add_cmd(
                                &BakeryCommands::AddCircuit {
                                    circuit_hash: mig.circuit_hash,
                                    parent_class_id: mig.parent_class_id,
                                    up_parent_class_id: mig.up_parent_class_id,
                                    class_minor: mig.old_minor,
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
                                // Final add (new rates) at final_minor
                                if let Some(final_cmd) = build_temp_add_cmd(
                                    &old_cmd,
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
                                    execute_and_record_live_change(&cmds, "live-move: build final");
                                }
                                mig.stage = MigrationStage::SwapToFinal;
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
                            for ip in &mig.ips {
                                let (ip_s, prefix) = parse_ip_and_prefix(ip);
                                let key = MappingKey {
                                    ip: ip_s.clone(),
                                    prefix,
                                };
                                let cpu = mapping_current.get(&key).map(|v| v.cpu).unwrap_or(0);
                                let handle =
                                    tc_handle_from_major_minor(mig.class_major, mig.final_minor);
                                let _ = lqos_sys::add_ip_to_tc(&ip_s, handle, cpu, false, 0, 0);
                                mapping_current.insert(key, MappingVal { handle, cpu });
                            }
                            let _ = lqos_sys::clear_hot_cache();
                            mig.stage = MigrationStage::TeardownShadow;
                            advanced += 1;
                        }
                        MigrationStage::TeardownShadow => {
                            // Remove shadow classes
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
                                execute_and_record_live_change(&prune, "live-move: prune shadow");
                            }
                            mig.stage = MigrationStage::Done;
                            advanced += 1;
                        }
                        MigrationStage::Done => {
                            to_remove.push(mig.circuit_hash);
                        }
                    }
                }
                for h in to_remove {
                    migrations.remove(&h);
                }
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
    let resolved_mq_layout = current_mq_layout(&new_batch, &config, mq_layout);

    let mapped_limit = resolve_mapped_circuit_limit();
    let effective_limit = mapped_limit.effective_limit;
    let limit_label = format_mapped_limit(effective_limit);

    let has_mq_been_setup = MQ_CREATED.load(std::sync::atomic::Ordering::Relaxed);
    if !has_mq_been_setup {
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
        info!("MQ not created, performing full reload.");
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
        );
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    let circuit_change_mode = diff_circuits(&new_batch, circuits);

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
                );
                return; // Skip the rest of this CommitBatch processing
            }
        }
    }

    // Now we can process circuit changes incrementally
    if let CircuitDiffResult::Categorized(categories) = circuit_change_mode {
        // One-line summary of changes (info!)
        info!(
            "Bakery changes: sites_speed={}, circuits_added={}, removed={}, speed={}, ip={}",
            site_speed_change_count,
            categories.newly_added.len(),
            categories.removed_circuits.len(),
            categories.speed_changed.len(),
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
                if let BakeryCommands::AddCircuit { circuit_hash, .. } = enriched_cmd.as_ref()
                    && let Some(old_cmd) = circuits.get(circuit_hash)
                {
                    enriched_cmd = rotate_changed_qdisc_handles(
                        old_cmd.as_ref(),
                        &enriched_cmd,
                        &config,
                        layout,
                        qdisc_handles,
                    );
                }
                if let BakeryCommands::AddCircuit {
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
                    ip_addresses,
                    sqm_override,
                    ..
                } = enriched_cmd.as_ref()
                {
                    let was_activated = live_circuits.contains_key(circuit_hash);
                    if was_activated {
                        // Attempt live-move
                        if let Some(shadow_minor) =
                            find_free_minor(circuits, parent_class_id, up_parent_class_id)
                        {
                            // Find old command for old rates
                            if let Some(old_cmd) = circuits.get(circuit_hash)
                                && let BakeryCommands::AddCircuit {
                                    download_bandwidth_min: old_down_min,
                                    upload_bandwidth_min: old_up_min,
                                    download_bandwidth_max: old_down_max,
                                    upload_bandwidth_max: old_up_max,
                                    down_qdisc_handle,
                                    up_qdisc_handle,
                                    ..
                                } = old_cmd.as_ref()
                            {
                                let mig = Migration {
                                    circuit_hash: *circuit_hash,
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
                                    old_minor: *class_minor,
                                    shadow_minor,
                                    final_minor: *class_minor,
                                    ips: parse_ip_list(ip_addresses),
                                    sqm_override: sqm_override.clone(),
                                    stage: MigrationStage::PrepareShadow,
                                };
                                migrations.insert(*circuit_hash, mig);
                                // Update desired circuit definition now
                                circuits.insert(*circuit_hash, Arc::clone(&enriched_cmd));
                                continue; // skip immediate path
                            }
                        }
                    }
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
) {
    warn!("Bakery: Full reload triggered due to site or circuit changes.");
    FULL_RELOAD_IN_PROGRESS.store(true, Ordering::Relaxed);
    mark_bakery_action_started(
        BakeryMode::ApplyingFullReload,
        "full_reload_started",
        "Full reload triggered due to site or circuit changes.".to_string(),
    );
    let _reload_scope = FullReloadScope;
    sites.clear();
    let previous_circuits = std::mem::take(circuits);
    live_circuits.clear();
    if let Some(layout) = resolved_mq_layout.as_ref() {
        process_batch(
            new_batch,
            config,
            sites,
            circuits,
            &previous_circuits,
            layout,
            qdisc_handles,
        );
        let active_hashes = circuits.keys().copied().collect::<HashSet<_>>();
        qdisc_handles.retain_circuits(&config.isp_interface(), &active_hashes);
        if !config.on_a_stick_mode() {
            qdisc_handles.retain_circuits(&config.internet_interface(), &active_hashes);
        }
        qdisc_handles.save(config);
        *mq_layout = resolved_mq_layout;
    } else {
        warn!("Bakery: full reload skipped qdisc-handle assignment because MQ layout is unknown");
        process_batch(
            new_batch,
            config,
            sites,
            circuits,
            &previous_circuits,
            &MqDeviceLayout::default(),
            qdisc_handles,
        );
    }
    *batch = None;
    apply_stormguard_overrides(stormguard_overrides, config);
}

fn process_batch(
    batch: Vec<Arc<BakeryCommands>>,
    config: &Arc<lqos_config::Config>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    previous_circuits: &HashMap<i64, Arc<BakeryCommands>>,
    mq_layout: &MqDeviceLayout,
    qdisc_handles: &mut QdiscHandleState,
) {
    info!("Bakery: Processing batch of {} commands", batch.len());
    let build_started = std::time::Instant::now();
    let mut circuit_count = 0u64;
    let commands = batch
        .into_iter()
        .map(|b| {
            let enriched = with_assigned_qdisc_handles(&b, config, mq_layout, qdisc_handles);
            let BakeryCommands::AddCircuit { circuit_hash, .. } = enriched.as_ref() else {
                return enriched;
            };
            let Some(previous) = previous_circuits.get(circuit_hash) else {
                return enriched;
            };
            rotate_changed_qdisc_handles(
                previous.as_ref(),
                &enriched,
                config,
                mq_layout,
                qdisc_handles,
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
    let result = execute_in_memory(&commands, "processing batch");
    let summary = summarize_apply_result("processing batch", &result);
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

    // Mark that at least one batch has been applied, unblocking live activation.
    FIRST_COMMIT_APPLIED.store(true, Ordering::Relaxed);
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
    use std::time::{SystemTime, UNIX_EPOCH};

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
}

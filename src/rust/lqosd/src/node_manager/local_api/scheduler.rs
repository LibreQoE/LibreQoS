use serde::Serialize;

use lqos_bus::{IntegrationErrorClass, SchedulerProgressReport, SchedulerRun};

use crate::node_manager::runtime_onboarding::runtime_onboarding_state;
use crate::tool_status::{
    is_scheduler_available, scheduler_error_message, scheduler_integration_status_state,
    scheduler_last_seen_unix, scheduler_output_message, scheduler_progress_state,
};

// Remove ANSI escape sequences (basic CSI/OSC handling) for browser display
fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1B {
            // ESC sequence
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] as char {
                '[' => {
                    // CSI: ESC [ ... final byte 0x40..=0x7E
                    i += 1;
                    while i < bytes.len() {
                        let b = bytes[i];
                        if (0x40..=0x7E).contains(&b) {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                }
                ']' => {
                    // OSC: ESC ] ... BEL (0x07) or ESC \
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1B {
                            // ESC
                            if i + 1 < bytes.len() && bytes[i + 1] as char == '\\' {
                                i += 2; // ESC \
                                break;
                            }
                        }
                        i += 1;
                    }
                }
                _ => {
                    // Other ESC-seq: skip next char at least
                    i += 1;
                }
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Combined System Health severity: the worst of the shaping and integration
/// axes. Computed backend-side; the frontend only renders it.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemHealthSeverity {
    /// Both axes healthy.
    Healthy,
    /// Shaping active but NOT healthy on at least one axis: a failing import
    /// and/or a non-Fresh boot world (bootstrap trunk / pass-through). WHICH
    /// axis is named by `SchedulerStatus::summary_reason` (produced together
    /// with this value) — the enum itself does not imply the import axis.
    Degraded,
    /// Shaping stalled — the scheduler has not run recently.
    Down,
    /// An import is in progress.
    Running,
    /// No integration configured, or runtime setup is incomplete.
    Idle,
}

/// Provenance of the applied shaping topology.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopologySource {
    /// The applied topology reflects a successful import.
    Fresh,
    /// The applied topology is the last-known-good fallback (import failing).
    LastKnownGood,
}

/// AVAIL (CARD `v3_availability_hardening_card` §7 b4): the BOOT-WORLD tier
/// the bridge is shaping from — a DISTINCT axis from [`TopologySource`] (the
/// import axis). Reported from the bridge `HealthReport` via
/// `bridge_supervisor`; `None`/"unknown" when there is no report — it never
/// falls back to the import axis.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BootWorldTier {
    /// The configured world (transaction or artifact escape).
    Fresh,
    /// The persisted last-known-good world.
    LastKnownGood,
    /// A minimal single-trunk world from `lqos.conf` bandwidth.
    BootstrapTrunk,
    /// No verified world: the bridge runs unshaped rather than refusing.
    PassThrough,
}

/// Why the System Health summary is [`SystemHealthSeverity::Degraded`].
///
/// Single-source (CARD `system_health_degraded_reason_card.md` §4 F1): produced
/// by [`compute_summary_severity`] TOGETHER with the severity, so the two can
/// never diverge. The import axis and the boot-world axis are independent, so
/// the combined cases are named explicitly — a doubly-degraded box must never
/// render one axis silently.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryReason {
    /// The integration import axis is failing (the boot world is Fresh/LKG).
    ImportFailing,
    /// The bridge booted on the bootstrap trunk (no verified world at boot).
    BootWorldBootstrapTrunk,
    /// The bridge booted pass-through (no shaping world verified at boot).
    BootWorldPassThrough,
    /// Both axes: a failing import AND a bootstrap-trunk boot world.
    ImportFailingAndBootWorldBootstrapTrunk,
    /// Both axes: a failing import AND a pass-through boot world.
    ImportFailingAndBootWorldPassThrough,
}

/// Snapshot of scheduler availability for the node-manager widget.
#[derive(Serialize, Debug, Clone)]
pub struct SchedulerStatus {
    /// Whether the scheduler has reported in within the last five minutes.
    pub available: bool,
    /// The latest scheduler error, if one was reported.
    pub error: Option<String>,
    /// The scheduler's last reported progress snapshot, if any.
    pub progress: Option<SchedulerProgressReport>,
    /// The best-known Unix timestamp for this scheduler snapshot.
    pub updated_unix: Option<u64>,
    /// Whether this snapshot is older than the scheduler availability window.
    pub stale: bool,
    /// Whether initial runtime onboarding is blocking scheduler activity.
    pub setup_required: bool,
    /// Operator-facing onboarding guidance when `setup_required` is true.
    pub setup_message: Option<String>,
    /// The integration label that last ran, if any.
    pub integration_name: Option<String>,
    /// Whether the last import succeeded, if attested.
    pub last_import_ok: Option<bool>,
    /// Unix timestamp of the last import, if attested.
    pub last_import_at: Option<u64>,
    /// Duration of the last import in milliseconds, if known.
    pub last_import_ms: Option<u64>,
    /// Consecutive integration failures since the last success.
    pub consecutive_failures: u32,
    /// The classified cause of the last integration failure.
    pub integration_error_class: IntegrationErrorClass,
    /// Stripped, actionable detail from the last integration failure.
    pub integration_error_detail: Option<String>,
    /// The bounded recent-run ring (oldest first) for the timeline.
    pub recent_runs: Vec<SchedulerRun>,
    /// The applied dataplane generation id, if one is known.
    pub dataplane_generation: Option<String>,
    /// Unix timestamp of the last applied dataplane generation, if known.
    pub applied_at: Option<u64>,
    /// Provenance of the applied topology (import axis; fresh vs last-known-good).
    pub topology_source: Option<TopologySource>,
    /// AVAIL (CARD §7 b4): the bridge's boot-world tier (the world it is
    /// actually shaping), or None when no Supervisor report is available —
    /// a DISTINCT axis, never the import-axis fallback.
    pub boot_world_tier: Option<BootWorldTier>,
    /// AVAIL 0(c) (CARD §2): the reconciliation convergence terminal, when
    /// the broker's bounded owner has exhausted its attempts (None otherwise).
    pub convergence_terminal: Option<String>,
    /// HOLD(2) (§0a): total boots observed (the fallback-rate denominator).
    pub boots: u64,
    /// HOLD(2) (§0a): seconds-since-epoch of the last boot.
    pub last_boot_unix: u64,
    /// HOLD(2) (§0a): per-tier boot counts (the fallback FREQUENCY across
    /// boots), `TopologyTier::as_str()` -> count. Empty when no report.
    pub tier_counts: std::collections::BTreeMap<String, u64>,
    /// Number of configured/shaped leaves in the device catalog.
    pub leaf_count: usize,
    /// The combined System Health severity (worst of the two axes).
    pub summary_severity: SystemHealthSeverity,
    /// Why [`Self::summary_severity`] is `Degraded` (`None` = healthy);
    /// produced TOGETHER with the severity so the two cannot diverge.
    pub summary_reason: Option<SummaryReason>,
}

/// Detailed scheduler snapshot for the scheduler modal.
#[derive(Serialize, Debug, Clone)]
pub struct SchedulerDetails {
    /// Whether the scheduler has reported in within the last five minutes.
    pub available: bool,
    /// The latest scheduler error, if one was reported.
    pub error: Option<String>,
    /// The latest non-error scheduler output line, if any.
    pub output: Option<String>,
    /// The scheduler's last reported progress snapshot, if any.
    pub progress: Option<SchedulerProgressReport>,
    /// The best-known Unix timestamp for this scheduler snapshot.
    pub updated_unix: Option<u64>,
    /// Whether this snapshot is older than the scheduler availability window.
    pub stale: bool,
    /// Whether initial runtime onboarding is blocking scheduler activity.
    pub setup_required: bool,
    /// Operator-facing onboarding guidance when `setup_required` is true.
    pub setup_message: Option<String>,
    /// The integration label that last ran, if any.
    pub integration_name: Option<String>,
    /// Whether the last import succeeded, if attested.
    pub last_import_ok: Option<bool>,
    /// Unix timestamp of the last import, if attested.
    pub last_import_at: Option<u64>,
    /// Duration of the last import in milliseconds, if known.
    pub last_import_ms: Option<u64>,
    /// Consecutive integration failures since the last success.
    pub consecutive_failures: u32,
    /// The classified cause of the last integration failure.
    pub integration_error_class: IntegrationErrorClass,
    /// Stripped, actionable detail from the last integration failure.
    pub integration_error_detail: Option<String>,
    /// The bounded recent-run ring (oldest first) for the timeline.
    pub recent_runs: Vec<SchedulerRun>,
    /// The applied dataplane generation id, if one is known.
    pub dataplane_generation: Option<String>,
    /// Unix timestamp of the last applied dataplane generation, if known.
    pub applied_at: Option<u64>,
    /// Provenance of the applied topology (import axis; fresh vs last-known-good).
    pub topology_source: Option<TopologySource>,
    /// AVAIL (CARD §7 b4): the bridge's boot-world tier (the world it is
    /// actually shaping), or None when no Supervisor report is available —
    /// a DISTINCT axis, never the import-axis fallback.
    pub boot_world_tier: Option<BootWorldTier>,
    /// AVAIL 0(c) (CARD §2): the reconciliation convergence terminal, when
    /// the broker's bounded owner has exhausted its attempts (None otherwise).
    pub convergence_terminal: Option<String>,
    /// HOLD(2) (§0a): total boots observed (the fallback-rate denominator).
    pub boots: u64,
    /// HOLD(2) (§0a): seconds-since-epoch of the last boot.
    pub last_boot_unix: u64,
    /// HOLD(2) (§0a): per-tier boot counts (the fallback FREQUENCY across
    /// boots), `TopologyTier::as_str()` -> count. Empty when no report.
    pub tier_counts: std::collections::BTreeMap<String, u64>,
    /// Number of configured/shaped leaves in the device catalog.
    pub leaf_count: usize,
    /// The combined System Health severity (worst of the two axes).
    pub summary_severity: SystemHealthSeverity,
    /// Why [`Self::summary_severity`] is `Degraded` (`None` = healthy).
    pub summary_reason: Option<SummaryReason>,
    /// Human-readable raw details for the operator debug view.
    pub details: String,
}

/// Worst-of severity from the shaping axis + the integration axis.
///
/// `setup_required` and `available` are handled by the caller; this covers the
/// shaping-active/failing matrix. Integration `unconfigured` is NOT a fault, so
/// shaping-active + no integration returns `Healthy` (the false-amber guard).
fn compute_summary_severity(
    available: bool,
    active: bool,
    integration_failing: bool,
    setup_required: bool,
    boot_world_tier: Option<BootWorldTier>,
) -> (SystemHealthSeverity, Option<SummaryReason>) {
    if setup_required {
        return (SystemHealthSeverity::Idle, None);
    }
    if !available {
        return (SystemHealthSeverity::Down, None);
    }
    if active {
        return (SystemHealthSeverity::Running, None);
    }
    // AVAIL (CARD §7 b4): a non-Fresh boot world (bootstrap trunk /
    // pass-through) is degraded — the summary can never read Healthy while
    // the bridge is not shaping a fresh/LKG world. The boot axis needs the
    // TIER (not a bool) so the combined reasons can name trunk vs pass-through.
    let boot_degraded = matches!(
        boot_world_tier,
        Some(BootWorldTier::BootstrapTrunk | BootWorldTier::PassThrough)
    );
    let reason = match (integration_failing, boot_world_tier) {
        (true, Some(BootWorldTier::BootstrapTrunk)) => {
            Some(SummaryReason::ImportFailingAndBootWorldBootstrapTrunk)
        }
        (true, Some(BootWorldTier::PassThrough)) => {
            Some(SummaryReason::ImportFailingAndBootWorldPassThrough)
        }
        (true, _) => Some(SummaryReason::ImportFailing),
        (false, Some(BootWorldTier::BootstrapTrunk)) => {
            Some(SummaryReason::BootWorldBootstrapTrunk)
        }
        (false, Some(BootWorldTier::PassThrough)) => Some(SummaryReason::BootWorldPassThrough),
        (false, _) => None,
    };
    debug_assert_eq!(reason.is_some(), integration_failing || boot_degraded);
    if reason.is_some() {
        (SystemHealthSeverity::Degraded, reason)
    } else {
        (SystemHealthSeverity::Healthy, None)
    }
}

/// Derive the topology provenance from the integration axis.
///
/// The broker record carries no provenance, so this is a JOIN: a failing
/// import means the applied topology is the last-known-good fallback; a
/// successful import means it is fresh; no attested import means unknown.
fn derive_topology_source(last_import_ok: Option<bool>) -> Option<TopologySource> {
    match last_import_ok {
        Some(false) => Some(TopologySource::LastKnownGood),
        Some(true) => Some(TopologySource::Fresh),
        None => None,
    }
}

/// AVAIL (CARD `v3_availability_hardening_card` §7 b4): map the bridge's boot
/// topology-tier string onto [`BootWorldTier`]. `None` = unknown/empty — the
/// caller reports "unknown", NEVER an import-axis fallback.
fn boot_world_tier_from_str(tier: &str) -> Option<BootWorldTier> {
    match tier {
        "fresh" => Some(BootWorldTier::Fresh),
        "last_known_good" => Some(BootWorldTier::LastKnownGood),
        "bootstrap_trunk" => Some(BootWorldTier::BootstrapTrunk),
        "pass_through" => Some(BootWorldTier::PassThrough),
        _ => None,
    }
}

/// The bridge-reported boot tier, when the Supervisor has a successful probe
/// (the AVAIL `HealthReport` carries it). `None` when the Supervisor is
/// unconfigured or has no report yet.
fn bridge_boot_world_tier() -> Option<BootWorldTier> {
    let status = crate::bridge_supervisor::bridge_health_status();
    let report = (*status)
        .as_ref()
        .and_then(|status| status.report.as_ref())?;
    boot_world_tier_from_str(&report.topology_tier)
}

/// HOLD(2) (§0a): the bridge-reported per-tier boot record (boots total,
/// last-boot unix, per-tier counts), when the Supervisor has a probe.
fn bridge_tier_record() -> Option<(u64, u64, std::collections::BTreeMap<String, u64>)> {
    let status = crate::bridge_supervisor::bridge_health_status();
    let report = (*status)
        .as_ref()
        .and_then(|status| status.report.as_ref())?;
    Some((report.boots, report.last_boot_unix, report.tier_counts.clone()))
}

/// The most recent APPLIED dataplane generation + timestamp, if any.
fn applied_dataplane_record() -> Option<(String, u64)> {
    use crate::bridge_broker::outcomes::{PublishState, publish_outcomes};
    publish_outcomes()
        .into_iter()
        .find(|record| matches!(record.state, PublishState::Applied))
        .map(|record| (record.generation, record.updated_unix))
}

/// The integration axis fields, flattened from the tool-status snapshot.
struct IntegrationAxis {
    name: Option<String>,
    ok: Option<bool>,
    at: Option<u64>,
    ms: Option<u64>,
    failures: u32,
    class: IntegrationErrorClass,
    detail: Option<String>,
    recent_runs: Vec<SchedulerRun>,
}

fn integration_axis() -> IntegrationAxis {
    match scheduler_integration_status_state() {
        Some(status) => IntegrationAxis {
            name: status.integration_name,
            ok: status.last_import_ok,
            at: status.last_import_at,
            ms: status.last_import_ms,
            failures: status.consecutive_failures,
            class: status.error_class,
            detail: status.error_detail,
            recent_runs: status.recent_runs,
        },
        None => IntegrationAxis {
            name: None,
            ok: None,
            at: None,
            ms: None,
            failures: 0,
            class: IntegrationErrorClass::None,
            detail: None,
            recent_runs: Vec::new(),
        },
    }
}

fn scheduler_error() -> Option<String> {
    scheduler_error_message().and_then(|s| {
        let t = s.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(strip_ansi(&t))
        }
    })
}

fn scheduler_output() -> Option<String> {
    scheduler_output_message().and_then(|s| {
        let t = s.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(strip_ansi(&t))
        }
    })
}

fn scheduler_updated_unix(progress: Option<&SchedulerProgressReport>) -> Option<u64> {
    let progress_updated = progress.and_then(|report| report.updated_unix);
    match (progress_updated, scheduler_last_seen_unix()) {
        (Some(progress_updated), Some(last_seen_updated)) => {
            Some(progress_updated.max(last_seen_updated))
        }
        (Some(progress_updated), None) => Some(progress_updated),
        (None, Some(last_seen_updated)) => Some(last_seen_updated),
        (None, None) => None,
    }
}

/// Builds the lightweight scheduler status snapshot for node-manager consumers.
pub fn scheduler_status_data() -> SchedulerStatus {
    let onboarding = runtime_onboarding_state();
    if onboarding.required {
        return SchedulerStatus {
            available: false,
            error: None,
            progress: None,
            updated_unix: None,
            stale: false,
            setup_required: true,
            setup_message: Some(onboarding.summary),
            integration_name: None,
            last_import_ok: None,
            last_import_at: None,
            last_import_ms: None,
            consecutive_failures: 0,
            integration_error_class: IntegrationErrorClass::None,
            integration_error_detail: None,
            recent_runs: Vec::new(),
            dataplane_generation: None,
            applied_at: None,
            topology_source: None,
            boot_world_tier: None,
            convergence_terminal: None,
            boots: 0,
            last_boot_unix: 0,
            tier_counts: std::collections::BTreeMap::new(),
            leaf_count: 0,
            summary_severity: SystemHealthSeverity::Idle,
            summary_reason: None,
        };
    }

    let available = is_scheduler_available();
    let error = scheduler_error();
    let progress = scheduler_progress_state();
    let updated_unix = scheduler_updated_unix(progress.as_ref());
    let axis = integration_axis();
    let (dataplane_generation, applied_at) = match applied_dataplane_record() {
        Some((generation, at)) => (Some(generation), Some(at)),
        None => (None, None),
    };
    // The import-axis provenance is UNCHANGED (the (a) ruling; CARD §7 b4).
    let topology_source = derive_topology_source(axis.ok);
    // AVAIL (CARD §7 b4): a SEPARATE boot-world tier reported by the bridge;
    // None when there is no Supervisor report — never an import-axis fallback.
    let boot_world_tier = bridge_boot_world_tier();
    // HOLD(2) (§0a): the bridge-reported per-tier boot record.
    let (boots, last_boot_unix, tier_counts) = bridge_tier_record().unwrap_or_default();
    // WP5 (`docs/wp5_seam_state_surface.md`): the seam's last typed reject — the
    // SEAM_STATE reconciliation surface (replaces the retired AVAIL 0(c) terminal).
    let convergence_terminal = crate::bridge_broker::convergence_snapshot().last_reject;
    let leaf_count =
        crate::node_manager::local_api::circuit_count::circuit_count_data().configured_count;
    let active = progress.as_ref().is_some_and(|report| report.active);
    let (summary_severity, summary_reason) = compute_summary_severity(
        available,
        active,
        axis.ok == Some(false),
        false,
        boot_world_tier,
    );
    SchedulerStatus {
        available,
        error,
        progress,
        updated_unix,
        stale: updated_unix.is_some() && !available,
        setup_required: false,
        setup_message: None,
        integration_name: axis.name,
        last_import_ok: axis.ok,
        last_import_at: axis.at,
        last_import_ms: axis.ms,
        consecutive_failures: axis.failures,
        integration_error_class: axis.class,
        integration_error_detail: axis.detail,
        recent_runs: axis.recent_runs,
        dataplane_generation,
        applied_at,
        topology_source,
        boot_world_tier,
        convergence_terminal,
        boots,
        last_boot_unix,
        tier_counts,
        leaf_count,
        summary_severity,
        summary_reason,
    }
}

/// Builds the detailed scheduler snapshot for the scheduler modal.
pub fn scheduler_details_data() -> SchedulerDetails {
    let status = scheduler_status_data();
    let output = scheduler_output();
    let mut body = String::new();
    if status.setup_required {
        let message = status.setup_message.clone().unwrap_or_else(|| {
            "Choose a topology source in Complete Setup before expecting scheduler activity."
                .to_string()
        });
        body.push_str("Scheduler status: setup required\n\n");
        body.push_str("LibreQoS runtime onboarding is incomplete.\n");
        body.push_str(&message);
        body.push('\n');
        return SchedulerDetails {
            available: false,
            error: None,
            output: None,
            progress: None,
            updated_unix: None,
            stale: false,
            setup_required: true,
            setup_message: Some(message),
            integration_name: None,
            last_import_ok: None,
            last_import_at: None,
            last_import_ms: None,
            consecutive_failures: 0,
            integration_error_class: IntegrationErrorClass::None,
            integration_error_detail: None,
            recent_runs: Vec::new(),
            dataplane_generation: None,
            applied_at: None,
            topology_source: None,
            boot_world_tier: None,
            convergence_terminal: None,
            boots: 0,
            last_boot_unix: 0,
            tier_counts: std::collections::BTreeMap::new(),
            leaf_count: 0,
            summary_severity: SystemHealthSeverity::Idle,
            summary_reason: None,
            details: body,
        };
    }

    body.push_str(&format!("Scheduler available: {}\n\n", status.available));
    if let Some(updated_unix) = status.updated_unix {
        body.push_str(&format!("Last updated Unix: {updated_unix}\n"));
    } else {
        body.push_str("Last updated Unix: unavailable\n");
    }
    body.push_str(&format!("Snapshot stale: {}\n\n", status.stale));
    match status.progress.as_ref() {
        Some(progress) => {
            body.push_str("Current progress:\n");
            body.push_str(&format!(
                "- Active: {}\n- Phase: {}\n- Step: {}/{}\n- Percent: {}%\n",
                progress.active,
                progress.phase_label,
                progress.step_index,
                progress.step_count,
                progress.percent
            ));
            if let Some(updated_unix) = progress.updated_unix {
                body.push_str(&format!("- Updated Unix: {}\n", updated_unix));
            }
            body.push('\n');
        }
        None => {
            body.push_str("No scheduler progress reported.\n\n");
        }
    }
    match status.error.as_ref() {
        Some(err) => {
            body.push_str("Reported error:\n");
            body.push_str(err);
            body.push('\n');
        }
        None => {
            body.push_str("No scheduler error reported.\n");
        }
    }
    body.push('\n');
    match output.as_ref() {
        Some(text) => {
            body.push_str("Recent output:\n");
            body.push_str(text);
            body.push('\n');
        }
        None => {
            body.push_str("No recent scheduler output recorded.\n");
        }
    }
    body.push_str("\n(Additional scheduler diagnostics not available.)\n");

    SchedulerDetails {
        available: status.available,
        error: status.error,
        output,
        progress: status.progress,
        updated_unix: status.updated_unix,
        stale: status.stale,
        setup_required: false,
        setup_message: None,
        integration_name: status.integration_name,
        last_import_ok: status.last_import_ok,
        last_import_at: status.last_import_at,
        last_import_ms: status.last_import_ms,
        consecutive_failures: status.consecutive_failures,
        integration_error_class: status.integration_error_class,
        integration_error_detail: status.integration_error_detail,
        recent_runs: status.recent_runs,
        dataplane_generation: status.dataplane_generation,
        applied_at: status.applied_at,
        topology_source: status.topology_source,
        boot_world_tier: status.boot_world_tier,
        convergence_terminal: status.convergence_terminal,
        boots: status.boots,
        last_boot_unix: status.last_boot_unix,
        tier_counts: status.tier_counts,
        leaf_count: status.leaf_count,
        summary_severity: status.summary_severity,
        summary_reason: status.summary_reason,
        details: body,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BootWorldTier, SummaryReason, SystemHealthSeverity, TopologySource,
        boot_world_tier_from_str, compute_summary_severity, derive_topology_source,
        scheduler_details_data, scheduler_status_data,
    };
    use crate::test_support::runtime_config_test_lock;
    use crate::tool_status::{
        scheduler_error, scheduler_integration_status, scheduler_last_seen_unix, scheduler_output,
        scheduler_progress, scheduler_seen,
    };
    use lqos_bus::{
        IntegrationErrorClass, SchedulerIntegrationStatus, SchedulerProgressReport, SchedulerRun,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "libreqos-scheduler-status-{label}-{}-{unique}",
            std::process::id()
        ))
    }

    fn write_scheduler_test_config(runtime_dir: &Path) -> PathBuf {
        let config_path = runtime_dir.join("lqos.conf");
        let runtime_dir_display = runtime_dir.to_string_lossy();
        let state_dir_display = runtime_dir.join("state").to_string_lossy().into_owned();
        let raw = include_str!("../../../../lqos_config/src/etc/v15/example.toml")
            .replacen(
                "lqos_directory = \"/opt/libreqos/src\"",
                &format!("lqos_directory = {:?}", runtime_dir_display),
                1,
            )
            .replacen(
                "state_directory = \"/opt/libreqos/state\"",
                &format!("state_directory = {:?}", state_dir_display),
                1,
            )
            .replacen("enable_splynx = false", "enable_splynx = true", 1);
        fs::write(&config_path, raw).expect("write scheduler test config");
        config_path
    }

    struct SchedulerStatusTestContext {
        _guard: std::sync::MutexGuard<'static, ()>,
        old_lqos_config: Option<OsString>,
        old_lqos_directory: Option<OsString>,
        runtime_dir: PathBuf,
    }

    impl SchedulerStatusTestContext {
        fn new(label: &str) -> Self {
            let guard = runtime_config_test_lock()
                .lock()
                .expect("scheduler status test lock should not be poisoned");
            let runtime_dir = unique_test_dir(label);
            fs::create_dir_all(runtime_dir.join("state")).expect("create runtime state directory");
            let config_path = write_scheduler_test_config(&runtime_dir);
            let old_lqos_config = std::env::var_os("LQOS_CONFIG");
            let old_lqos_directory = std::env::var_os("LQOS_DIRECTORY");
            unsafe {
                std::env::set_var("LQOS_CONFIG", &config_path);
                std::env::set_var("LQOS_DIRECTORY", &runtime_dir);
            }
            lqos_config::clear_cached_config();
            Self {
                _guard: guard,
                old_lqos_config,
                old_lqos_directory,
                runtime_dir,
            }
        }
    }

    impl Drop for SchedulerStatusTestContext {
        fn drop(&mut self) {
            scheduler_error(None);
            scheduler_output(None);
            scheduler_progress(None);
            scheduler_integration_status(None);
            match &self.old_lqos_config {
                Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
                None => unsafe { std::env::remove_var("LQOS_CONFIG") },
            }
            match &self.old_lqos_directory {
                Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
                None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
            }
            lqos_config::clear_cached_config();
            let _ = fs::remove_dir_all(&self.runtime_dir);
        }
    }

    #[test]
    fn scheduler_status_surfaces_validation_failure_for_ui_contract() {
        let _context = SchedulerStatusTestContext::new("validation-failure");
        scheduler_seen();
        scheduler_error(Some(
            "Scheduled shaping refresh blocked by validation: duplicate IPv4".to_string(),
        ));
        scheduler_output(Some(
            "Scheduled shaping refresh blocked by validation: duplicate IPv4".to_string(),
        ));
        scheduler_progress(Some(SchedulerProgressReport {
            active: false,
            phase: "validation_failed".to_string(),
            phase_label: "Scheduler validation failed".to_string(),
            step_index: 5,
            step_count: 5,
            percent: 100,
            updated_unix: Some(1_234_567_890),
        }));

        let status = scheduler_status_data();
        let details = scheduler_details_data();
        let expected_updated_unix = scheduler_last_seen_unix()
            .map(|last_seen_unix| last_seen_unix.max(1_234_567_890))
            .expect("scheduler last seen should be set");

        assert!(status.available);
        assert_eq!(
            status.error.as_deref(),
            Some("Scheduled shaping refresh blocked by validation: duplicate IPv4")
        );
        assert_eq!(
            status
                .progress
                .as_ref()
                .map(|progress| progress.phase.as_str()),
            Some("validation_failed")
        );
        assert_eq!(status.updated_unix, Some(expected_updated_unix));
        assert!(!status.stale);
        assert!(!status.setup_required);

        assert!(details.available);
        assert_eq!(
            details.error.as_deref(),
            Some("Scheduled shaping refresh blocked by validation: duplicate IPv4")
        );
        assert_eq!(
            details.output.as_deref(),
            Some("Scheduled shaping refresh blocked by validation: duplicate IPv4")
        );
        assert_eq!(details.updated_unix, Some(expected_updated_unix));
        assert!(!details.stale);
        assert!(details.details.contains("Reported error:"));
        assert!(details.details.contains("Snapshot stale: false"));
        assert!(details.details.contains("Scheduler validation failed"));
    }

    #[test]
    fn scheduler_status_uses_last_seen_when_progress_timestamp_is_missing() {
        let _context = SchedulerStatusTestContext::new("last-seen-fallback");
        scheduler_seen();

        let status = scheduler_status_data();
        let details = scheduler_details_data();
        let last_seen_unix = scheduler_last_seen_unix().expect("scheduler last seen should be set");

        assert_eq!(status.updated_unix, Some(last_seen_unix));
        assert!(!status.stale);
        assert_eq!(details.updated_unix, Some(last_seen_unix));
        assert!(
            details
                .details
                .contains(&format!("Last updated Unix: {last_seen_unix}"))
        );
    }

    #[test]
    fn worst_of_total_order_and_unconfigured_neutral_guard() {
        use SystemHealthSeverity::{Degraded, Down, Healthy, Idle, Running};
        // Neutral guard: shaping active + no integration is NOT a fault.
        assert_eq!(
            compute_summary_severity(true, false, false, false, None),
            (Healthy, None)
        );
        // Shaping active + integration failing => Degraded (ImportFailing).
        assert_eq!(
            compute_summary_severity(true, false, true, false, None),
            (Degraded, Some(SummaryReason::ImportFailing))
        );
        // Shaping stalled => Down, and Down outranks a failing integration.
        assert_eq!(
            compute_summary_severity(false, false, false, false, None),
            (Down, None)
        );
        assert_eq!(
            compute_summary_severity(false, false, true, false, None),
            (Down, None)
        );
        // Running outranks Degraded.
        assert_eq!(
            compute_summary_severity(true, true, false, false, None),
            (Running, None)
        );
        assert_eq!(
            compute_summary_severity(true, true, true, false, None),
            (Running, None)
        );
        // setup_required outranks BOTH axes.
        assert_eq!(
            compute_summary_severity(true, false, true, true, None),
            (Idle, None)
        );
        assert_eq!(
            compute_summary_severity(false, true, true, true, None),
            (Idle, None)
        );
        // AVAIL (CARD §7 b4): a non-Fresh boot world (bootstrap trunk /
        // pass-through) folds to Degraded — the summary can never read
        // Healthy while the bridge is not shaping a fresh/LKG world.
        assert_eq!(
            compute_summary_severity(
                true,
                false,
                false,
                false,
                Some(BootWorldTier::BootstrapTrunk)
            ),
            (Degraded, Some(SummaryReason::BootWorldBootstrapTrunk))
        );
        assert_eq!(
            compute_summary_severity(true, false, false, false, Some(BootWorldTier::PassThrough)),
            (Degraded, Some(SummaryReason::BootWorldPassThrough))
        );
    }

    /// CARD `system_health_degraded_reason_card.md` §4 F1 / §7: the reason is
    /// produced TOGETHER with the severity (never a sibling that can diverge),
    /// over the FULL axis matrix; the combined rows name BOTH axes with the
    /// boot world FIRST; and a boot-world-only degrade is NEVER misattributed
    /// to the import (`ImportFailing`).
    #[test]
    fn summary_reason_truth_table_over_the_axis_matrix() {
        use SummaryReason::{
            BootWorldBootstrapTrunk, BootWorldPassThrough, ImportFailing,
            ImportFailingAndBootWorldBootstrapTrunk, ImportFailingAndBootWorldPassThrough,
        };
        use SystemHealthSeverity::{Degraded, Healthy};
        for tier in [
            None,
            Some(BootWorldTier::Fresh),
            Some(BootWorldTier::LastKnownGood),
            Some(BootWorldTier::BootstrapTrunk),
            Some(BootWorldTier::PassThrough),
        ] {
            for integration_failing in [false, true] {
                let (severity, reason) =
                    compute_summary_severity(true, false, integration_failing, false, tier);
                let expected = match (integration_failing, tier) {
                    (true, Some(BootWorldTier::BootstrapTrunk)) => {
                        Some(ImportFailingAndBootWorldBootstrapTrunk)
                    }
                    (true, Some(BootWorldTier::PassThrough)) => {
                        Some(ImportFailingAndBootWorldPassThrough)
                    }
                    (true, _) => Some(ImportFailing),
                    (false, Some(BootWorldTier::BootstrapTrunk)) => Some(BootWorldBootstrapTrunk),
                    (false, Some(BootWorldTier::PassThrough)) => Some(BootWorldPassThrough),
                    (false, _) => None,
                };
                assert_eq!(
                    reason, expected,
                    "integration_failing={integration_failing} tier={tier:?}"
                );
                assert_eq!(
                    severity,
                    if expected.is_some() {
                        Degraded
                    } else {
                        Healthy
                    },
                    "severity must pair with the reason (integration_failing={integration_failing} tier={tier:?})"
                );
                if !integration_failing {
                    assert!(
                        !matches!(
                            reason,
                            Some(ImportFailing)
                                | Some(ImportFailingAndBootWorldBootstrapTrunk)
                                | Some(ImportFailingAndBootWorldPassThrough)
                        ),
                        "boot-world-only degrade misattributed to the import: {reason:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn topology_source_is_a_join_of_the_import_axis() {
        assert_eq!(
            derive_topology_source(Some(false)),
            Some(TopologySource::LastKnownGood)
        );
        assert_eq!(
            derive_topology_source(Some(true)),
            Some(TopologySource::Fresh)
        );
        assert_eq!(derive_topology_source(None), None);
    }

    /// AVAIL (CARD `v3_availability_hardening_card` §7 b4): the bridge's boot
    /// tier strings map 1:1 onto the SEPARATE `BootWorldTier`; unknown/empty
    /// names return `None` ("unknown"), never an import-axis fallback.
    #[test]
    fn bridge_boot_world_tier_maps_1_to_1() {
        assert_eq!(
            boot_world_tier_from_str("fresh"),
            Some(BootWorldTier::Fresh)
        );
        assert_eq!(
            boot_world_tier_from_str("last_known_good"),
            Some(BootWorldTier::LastKnownGood)
        );
        assert_eq!(
            boot_world_tier_from_str("bootstrap_trunk"),
            Some(BootWorldTier::BootstrapTrunk)
        );
        assert_eq!(
            boot_world_tier_from_str("pass_through"),
            Some(BootWorldTier::PassThrough)
        );
        assert_eq!(boot_world_tier_from_str(""), None);
        assert_eq!(boot_world_tier_from_str("bogus"), None);
    }

    #[test]
    fn leaf_count_uses_the_configured_shaped_circuit_count() {
        let _context = SchedulerStatusTestContext::new("leaf-count-source");
        scheduler_seen();
        let status = scheduler_status_data();
        // configured_count is the stable shaped set (never the 5-minute
        // traffic-window `count`), matching `circuit_count_data`.
        let expected =
            crate::node_manager::local_api::circuit_count::circuit_count_data().configured_count;
        assert_eq!(status.leaf_count, expected);
    }

    #[test]
    fn integration_failure_is_carried_by_the_integration_axis_not_the_error_axis() {
        let _context = SchedulerStatusTestContext::new("integration-axis");
        scheduler_seen();
        scheduler_progress(Some(SchedulerProgressReport {
            active: false,
            phase: "ready".to_string(),
            phase_label: "Scheduler ready".to_string(),
            step_index: 5,
            step_count: 5,
            percent: 100,
            updated_unix: Some(1_234_567_890),
        }));
        scheduler_integration_status(Some(SchedulerIntegrationStatus {
            integration_name: Some("UISP integration".to_string()),
            last_import_ok: Some(false),
            last_import_at: Some(1_234_567_800),
            last_import_ms: Some(900),
            consecutive_failures: 6,
            error_class: IntegrationErrorClass::RedirectLoop,
            error_detail: Some("too many redirects".to_string()),
            recent_runs: vec![SchedulerRun {
                at: 1_234_567_800,
                ok: false,
                ms: Some(900),
            }],
        }));

        let status = scheduler_status_data();
        assert_eq!(status.summary_severity, SystemHealthSeverity::Degraded);
        assert_eq!(status.topology_source, Some(TopologySource::LastKnownGood));
        assert_eq!(status.consecutive_failures, 6);
        assert_eq!(
            status.integration_error_class,
            IntegrationErrorClass::RedirectLoop
        );
        assert_eq!(status.recent_runs.len(), 1);
        assert!(!status.recent_runs[0].ok);
        assert!(
            status.error.is_none(),
            "integration faults must not write the scheduler-error axis"
        );
    }
}

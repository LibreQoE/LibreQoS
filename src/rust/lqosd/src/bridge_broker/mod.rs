//! Bridge broker (E1, `docs` card `lqosd_broker_emission_card.md`):
//! the lqosd-side owner of the bridge control seam.
//!
//! Actor contract:
//! - Watches the compiler-published full desired-state artifact
//!   (`<state_dir>/shaping/dataplane_transaction.json`, stamped with
//!   the source-stable `shaping_generation` content hash) and emits
//!   the semantic delta vs the last APPLIED state (fresh-#12 G1/G3:
//!   parsed-content diffs, newest-wins after the current ack).
//!   F3-P (`docs/f3b_flip_and_f3p_work_order.md`): the diff is ONE
//!   coherent change-set (the F3-B shape); the bridge topo-sorts it
//!   against its own applied base. The old ≤1-op-per-tx SEQUENCE build
//!   and the new-site withholding are DELETED (the sequencing-driven
//!   quarantine is retired); the reject-driven hold + retry SURVIVES.
//! - Attach-retry: the bridge may be absent (boot ordering, crash) —
//!   re-attach on a 10s cadence; a boot_id change (G2) resets the
//!   baseline and re-emits the FULL desired state (idempotent
//!   Upserts).
//! - Ack discipline: single ack per publish, 30s timeout; on timeout
//!   a liveness check (G4) detects a replaced segment and re-attaches;
//!   `ATTEMPT_EXHAUSTED` is the only bounded-retry outcome (G5, ≤3
//!   with backoff); every other reject is terminal for its generation
//!   (no retry-storm; observe-mode rejections surface honestly).
//! - Inert when the product itself runs `queue_mode=observe` (one log
//!   line; nothing is emitted in that posture).

mod ack_map;
mod applied;
pub(crate) mod maintenance;
pub(crate) mod posture;
pub mod outcomes;
mod seam;
mod translate;

#[cfg(test)]
mod oracle1_live_tests;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use analytics_ipc::WorkerAffinityReport;
use lqos_config::topology_dataplane_transaction_path;
use lqos_bus::{UrgentSeverity, UrgentSource};
use lqos_dataplane_types::{
    DataplaneInnerOp, DataplaneLeafOp, DataplaneTransaction, LeafRow, compute_owner_sets,
    crossing_nodes,
};
use seam::{AttachError, SEGMENT_NAME, SeamSegment};
use tracing::{error, info, warn};

use crate::urgent;

const POLL_ARTIFACT: Duration = Duration::from_secs(2);
const RETRY_ATTACH_EVERY: u32 = 5; // polls (≈10s)
const ACK_TIMEOUT: Duration =
    Duration::from_millis(lqos_dataplane_types::BROKER_ACK_TIMEOUT_MS);
/// `lqos-74t.101`: four broker ACK waits (4 x 30 s = 120 s) leave room
/// for the bridge's 20 s convergence bound and a late-applied report.
const ACK_SILENCE_MAX_TIMEOUTS: u32 = 4;

#[cfg(test)]
thread_local! {
    static TEST_ACK_TIMEOUT_MS: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

fn ack_timeout() -> Duration {
    #[cfg(test)]
    if let Some(ms) = TEST_ACK_TIMEOUT_MS.with(std::cell::Cell::get) {
        return Duration::from_millis(ms);
    }
    ACK_TIMEOUT
}

/// `lqos-74t.102` §2.4(19)/(20): any terminal loss of the in-flight non-suffix
/// trigger (no seam, replaced seam, ack silence, attempt exhaustion) must keep
/// the gate and re-arm the replay. The gate must never be left waiting with no
/// trigger to come.
fn rearm_non_suffix_after_lost_pending(state: &mut BrokerState) {
    if state.pending_non_suffix.is_some() {
        state.non_suffix_replay_due = true;
    }
}
const ATTEMPT_EXHAUSTED_MAX_RETRIES: u32 = 3;
const ATTEMPT_BACKOFF: Duration = Duration::from_secs(5);
/// C3(b): consecutive terminal rejects (across generations) before the broker
/// arms a bounded backoff.
const REJECT_BACKOFF_THRESHOLD: u32 = 3;
/// C3(b): the backoff base (doubles per consecutive reject, capped).
const REJECT_BACKOFF_BASE: Duration = Duration::from_secs(2);
/// C3(b): the backoff cap.
const REJECT_BACKOFF_MAX: Duration = Duration::from_secs(30);
/// CARD (`docs/broker_single_structural_op_sequencing_card.md` §2): the bounded
/// auto-retry cadence for a HELD op — after this the held set is re-attempted
/// (dropped for ONE diff) so a transient cause self-clears without an operator
/// re-attach. Bounded (not a storm): a failed retry re-arms it.
const HELD_RETRY_BACKOFF: Duration = Duration::from_secs(30);
/// URGENT (jackrabbit stall): the MAX number of auto-retry attempts. A
/// GENUINELY-unappliable op (a span-changing generated move, a retire whose
/// applied subtree is non-empty) never clears by re-emitting — after this many
/// attempts the broker STOPS retrying (keeps the hold; the fence is a
/// bug-catcher, not a reject loop). Clears only on a structural change/re-attach.
const HELD_RETRY_MAX: u32 = 5;
/// §`lqos-74t.12`: how long a pure-site create may be DEFERRED (no circuit
/// rides it) before the operator gets the age-threshold alarm. A policy gap, not
/// a fault: the site is inert until its first circuit; the M-A route applies the
/// shell on demand.
const DEFERRED_SITE_ALARM_SECONDS: u64 = 900;
/// A3 cadence (`docs/broker_applied_single_source_card.md` §2 criterion 2): the
/// PERIODIC re-read of the applied-state report — the drift reconcile that keeps
/// the belief true WITHIN a session (the adopt alone is attach-only).
const REPORT_REREAD_PERIOD: Duration = Duration::from_secs(60);
/// A3 cadence: while there is NO base (the A1 HOLD), re-ask on this SHORTER
/// backoff so recovery is AUTOMATIC (the report returning clears the surface).
const REPORT_REREAD_RETRY: Duration = Duration::from_secs(10);
/// Persisted operator seam state (also the restart source for the non-suffix gate).
const SEAM_STATE_PATH: &str = "/run/libreqos/seam_state.json";
struct BrokerState {
    segment: Option<SeamSegment>,
    /// A3 (`docs/broker_applied_single_source_card.md` §2): the ONE owner of the
    /// broker's applied belief (the bridge's report = truth; the ack-fold =
    /// interim). `last_applied` / `last_applied_tx` are PROJECTIONS of it — the
    /// owner is the ONLY writer (single-writer by construction).
    applied: applied::AppliedState,
    /// In-flight publish: encoded frames + bookkeeping.
    pending: Option<Pending>,
    /// Generation that terminally failed — never re-emitted until the
    /// artifact's generation changes (the retry-storm fence).
    terminal_fence: Option<String>,
    /// WP5: the desired generation the broker last diffed (the reconciler's input).
    last_desired_generation: Option<String>,
    /// (a) LKG-as-cache (`docs/seam_lkg_restart_and_arms_work_order.md` item 2):
    /// the FULL desired world's content key (`content_key(&desired)`), advertised
    /// so the bridge's cache-match check needs no bridge-side fold. `None` until
    /// the first artifact read.
    last_desired_world_key: Option<String>,
    /// Card (`docs/duplicate_mapping_alerts_card.md`): the mapping IDENTITIES
    /// currently surfaced as a `DUPLICATE_MAPPING_IDENTITY` alert (bound to >1
    /// circuit). Tracked so `clear_by_identity` fires when one resolves — the
    /// issue TTL is 24h, so clear-on-resolve is load-bearing.
    duplicate_mapping_alerts: BTreeSet<String>,
    /// L2 (`docs/control_plane_structural_convergence_card.md` §3): the
    /// structurally HELD op identities. A structural reject QUARANTINES the
    /// offending op(s) instead of freezing the world; the appliable projection
    /// (`desired − held`) keeps publishing rates and unaffected structural work.
    /// This replaces the old `failed_structural` world-freeze key.
    held_ops: BTreeSet<translate::HeldOp>,
    /// The desired structural identity at hold time (L2-5). When the desired
    /// identity CHANGES the held set is cleared so the quarantined op is
    /// re-attempted (it may now be appliable); a re-attach also clears it.
    held_structural_key: Option<String>,
    /// CARD §2: the bounded auto-retry deadline for the held set — at/after this
    /// the held op is re-attempted (dropped for one diff). `None` = no retry due.
    held_retry_at: Option<Instant>,
    /// URGENT: auto-retry attempts used for the current hold (capped at
    /// `HELD_RETRY_MAX` so a genuinely-unappliable op is not re-emitted forever).
    held_retries: u32,
    /// `lqos-74t.54`: the last structural-remove capture — the FULL removed set,
    /// the bridge dense-table size, the suffix verdict, and the held non-suffix
    /// remainder. Surfaced by the ALWAYS-ON capture and persisted in SEAM_STATE.
    removal_capture: Option<RemovalCapture>,
    /// `lqos-74t.102`: the broker-owned non-suffix gate. It is deliberately NOT
    /// keyed to the structural identity: after a non-suffix remove the next
    /// structural ADD must not reach the partial slot table. While set, the
    /// broker holds all new incremental emission and waits for the bridge's R3
    /// report to match the full desired world before clearing this gate.
    pending_non_suffix: Option<RemovalCapture>,
    /// `lqos-74t.102` §2.4(9): a restored (or SEAM_LOST) non-suffix trigger
    /// must be re-emitted only after a real control-seam attach. This flag is
    /// deliberately separate from `pending`: the old restore set `pending`
    /// before the loop, and the first no-segment `resolve_pending` consumed it
    /// as SEAM_LOST, losing the only trigger. Keeping the intent here lets the
    /// broker hold the gate and replay after attach.
    non_suffix_replay_due: bool,
    /// §2.4(14): the desired-world key carried by the non-suffix trigger that
    /// is currently in `pending`. When the artifact advances (c4 -> c5) before
    /// the in-flight trigger resolves, the broker must re-arm with c5 rather
    /// than wait forever on the stale key.
    non_suffix_trigger_key: Option<String>,
    /// §`lqos-74t.12` (DRIVER wave-1): NEW sites DEFERRED because no circuit
    /// rides them (a pure-site create has zero dataplane effect). The value is
    /// the site's FIRST-SEEN unix second, so the operator surface can show how
    /// long the desired≠applied POLICY gap has aged (never silent).
    deferred_sites: BTreeMap<String, u64>,
    /// §`lqos-74t.13`/`.15`(d): applied SITES DEFERRED for retirement because
    /// the applied subtree still bears a circuit (the bridge's
    /// `RetireWithCircuits` gate runs pre-delta). The value is the site's
    /// FIRST-SEEN unix second (same surface as `deferred_sites`).
    deferred_retires: BTreeMap<String, u64>,
    /// R3c-2: the bridge's authoritative slot order (from the adopted
    /// report). `None` = not carried — semantic_diff falls back to the
    /// baseline's own order and the drain key warns.
    slot_order: Option<Vec<String>>,
    /// R3: the analytics socket path (the applied-state report source).
    /// `None` = the [dataplane_bridge] section is unconfigured — the
    /// adoption arm is inert and every attach falls back to full
    /// re-emit (exactly today's posture).
    socket_path: Option<std::path::PathBuf>,
    /// Stat identity of the last successfully parsed artifact
    /// (review fix 3: skip the re-read/re-parse while mtime+size are
    /// unchanged — a size-stable same-mtime rewrite is re-picked-up on
    /// the NEXT change; the poll is 2s and the artifact is
    /// compiler-published).
    artifact_cache: Option<(std::time::SystemTime, u64, DataplaneTransaction)>,
    /// The bad-layout attach warning was already surfaced (review fix
    /// 5: once, not every 10s). Reset on a successful attach.
    bad_layout_reported: bool,
    attach_polls: u32,
    /// Ghost-segment idle-liveness poll counter (fix shape (i)): advances
    /// only while attached AND idle; a replaced seam drops `segment` to
    /// None so the attach poll above recovers. Distinct from
    /// `attach_polls` (which only advances while absent).
    liveness_polls: u32,
    /// C3(b): consecutive terminal rejects (reset on an APPLIED ack). Bounds a
    /// reject/retry churn with a growing backoff instead of a per-tick storm.
    rejects_consecutive: u32,
    /// C3(b): while set in the future, the actor skips starting a new publish.
    backoff_until: Option<std::time::Instant>,
    /// A3 cadence (`docs/broker_applied_single_source_card.md` §2 criterion 2):
    /// the next time the applied-state REPORT is re-read (reconcile the interim
    /// ack-fold against the bridge's truth). `None` = not yet armed.
    report_reread_at: Option<Instant>,
    /// A3 cadence: a terminal reject (or the A1 no-base HOLD) marks a re-read
    /// DUE immediately — the minimum cadence (read after a reject).
    report_reread_due: bool,
    /// `lqos-74t.101`: while set, `reconcile_from_report` is being called by
    /// the ACK-silence late-adopt path. The report must match the current
    /// desired content key + generation, and a matching report is adopted
    /// without surfacing the interim ack-fold as a divergence.
    late_adopt: bool,
    /// `lqos-8h8`: the `[maintenance_window]` gate config.
    maintenance: lqos_config::MaintenanceWindowConfig,
    /// `lqos-8h8`: the RESOLVED site TZ (`None` = server-local).
    maintenance_tz: Option<chrono_tz::Tz>,
    /// `lqos-8h8`: the operator's `LIBREQOS_DPDK_BULK_GENERATION_THRESHOLD`
    /// override (the SAME value the producer uses — R-OVERRIDE-PARITY).
    maintenance_override_ops: Option<usize>,
    /// `lqos-8h8`: the currently-HELD pause-class publish (the pending surface +
    /// the force clock). `None` = nothing held (window open / pause-free).
    deferred_maintenance: Option<maintenance::DeferredMaintenance>,
    /// `lqos-8h8` (c): a one-shot operator APPLY-NOW (bypasses the window once
    /// for the currently-held change).
    apply_now: bool,
    /// `lqos-8h8` (c): the time-boxed GLOBAL break-glass expiry (unix seconds).
    break_glass_until_unix: Option<u64>,
    /// `lqos-8h8` (c): the operator OVERRIDE control file (polled each tick;
    /// written by the WS handler — the broker owns a single-threaded state, so
    /// the file is the seam, like the artifact).
    maintenance_control_path: Option<std::path::PathBuf>,
}

impl BrokerState {
    /// C3(b): record a terminal reject; once consecutive rejects reach the
    /// threshold, arm an exponentially-growing, capped backoff.
    fn record_reject(&mut self) {
        self.rejects_consecutive = self.rejects_consecutive.saturating_add(1);
        if self.rejects_consecutive >= REJECT_BACKOFF_THRESHOLD {
            let extra = self.rejects_consecutive - REJECT_BACKOFF_THRESHOLD;
            let factor = 1u32 << extra.min(16);
            let backoff = REJECT_BACKOFF_BASE
                .saturating_mul(factor)
                .min(REJECT_BACKOFF_MAX);
            self.backoff_until = Some(std::time::Instant::now() + backoff);
            warn!(
                rejects = self.rejects_consecutive,
                backoff_secs = backoff.as_secs(),
                "Bridge broker: consecutive-reject backoff armed (the control channel is \
                 not converging; the outage-containment churn alarm also fires)"
            );
        }
    }

    /// C3(b): an APPLIED ack is convergence — reset the reject backoff.
    fn record_applied_reset(&mut self) {
        self.rejects_consecutive = 0;
        self.backoff_until = None;
    }
}

#[derive(Clone)]
struct Pending {
    /// Frames of the IN-FLIGHT transaction (the sequence front).
    frames: Vec<Vec<u8>>,
    /// Frames already written to the ring. PUSH-ONCE (review fix 1):
    /// written frames are never re-pushed while waiting — a re-push
    /// would queue a duplicate publish and misattribute its stale ack
    /// to the NEXT transaction. Resumes mid-batch after a ring-full
    /// PushError; resets only on a fresh encode (next sequence tx) or
    /// an ATTEMPT_EXHAUSTED ack (which proves the bridge consumed the
    /// attempt).
    pushed: usize,
    /// True once every frame is in the ring: wait-only until an ack
    /// resolves (no re-push on ack silence — the bridge may still be
    /// processing).
    awaiting_ack: bool,
    generation: String,
    /// The desired world's structural identity at emit time; recorded as the
    /// held-structure key when this publish terminally rejects as structural.
    structural_key: String,
    /// L2-1 (segment bookkeeping): the IN-FLIGHT segment's transaction content,
    /// so a structural reject can name the offending op identity.
    current_tx: DataplaneTransaction,
    /// E3b bookkeeping: this transaction's 1-based position in the
    /// compliance sequence and the sequence length (for the operator
    /// progress surface).
    seq_pos: usize,
    seq_len: usize,
    /// Baseline the CURRENT in-flight transaction advances to on
    /// Applied.
    applied_after_current: DataplaneTransaction,
    /// `lqos-101`: the DESIRED world key carried on THIS transaction's frames
    /// (captured at encode from `state.last_desired_world_key`). The ack-fold
    /// uses it UNCONDITIONALLY, so a mid-flight artifact advance cannot relabel
    /// the applied world with the superseded/current-wrong key. `None` = a
    /// publish with no frame key (sequence/replay/tests) ⇒ folded fallback.
    desired_key: Option<String>,
    attempt_retries: u32,
    ack_timeouts: u32,
}

static STARTED: OnceLock<()> = OnceLock::new();

/// `lqos-74t.102`: the R3 trigger for a held non-suffix removal. The engine's
/// worker detector refuses the first non-suffix Remove BEFORE any mutation and
/// raises `RebuildDivergence::SlotMismatch`, so this transaction never reaches
/// the partial slot table. It carries NO structural ADD (and no rate work):
/// the full desired world is installed by the R3/re-adopt.
/// `lqos-74t.102`: an APPLIED ack for the remove-only trigger while the gate is
/// armed is a partial-world acceptance (the engine detector should have forced
/// R3). The broker must ignore that ack for belief advancement.
fn non_suffix_applied_is_suspect(pending_non_suffix: Option<&RemovalCapture>) -> bool {
    pending_non_suffix.is_some()
}

fn non_suffix_rebuild_trigger(
    desired: &DataplaneTransaction,
    capture: &RemovalCapture,
) -> Option<DataplaneTransaction> {
    if capture.non_suffix.is_empty() {
        return None;
    }
    Some(DataplaneTransaction {
        generation: desired.generation.clone(),
        inner_ops: Vec::new(),
        leaf_ops: capture
            .non_suffix
            .iter()
            .map(|circuit_id| DataplaneLeafOp::Remove {
                circuit_id: circuit_id.clone(),
            })
            .collect(),
        mapping_ops: Vec::new(),
        schema_version: desired.schema_version,
        steering_vlans: Vec::new(),
        lpm_delta: Vec::new(),
        apply_mode: Default::default(),
        circuit_capacity: None,
    })
}

/// `lqos-74t.102` §2.4(9): queue the restored/lost non-suffix trigger only
/// after a real control seam exists. The old restore path put this into
/// `state.pending` before the loop, so the first no-segment step consumed it
/// as SEAM_LOST and nothing was ever re-emitted.
fn queue_non_suffix_replay(state: &mut BrokerState, desired: &DataplaneTransaction) -> bool {
    let Some(capture) = state.pending_non_suffix.clone() else {
        state.non_suffix_replay_due = false;
        state.non_suffix_trigger_key = None;
        return false;
    };
    let Some(trigger) = non_suffix_rebuild_trigger(desired, &capture) else {
        state.pending_non_suffix = None;
        state.removal_capture = None;
        state.non_suffix_replay_due = false;
        state.non_suffix_trigger_key = None;
        state.terminal_fence = None;
        warn!(
            event = "NON_SUFFIX_GATE_DROPPED_NO_TRIGGER",
            "the held non-suffix set has no removable trigger — falling back to the \
             engine RemovedNotTail/SlotMismatch recovery"
        );
        return false;
    };
    let structural = translate::structural_identity(desired);
    match seam::encode_transaction(&trigger, state.last_desired_world_key.as_deref()) {
        Ok(frames) => {
            outcomes::record_emit(&trigger.generation, 1, 1);
            state.terminal_fence = Some(structural.clone());
            state.pending = Some(Pending {
                frames,
                pushed: 0,
                awaiting_ack: false,
                generation: trigger.generation.clone(),
                structural_key: structural,
                current_tx: trigger,
                seq_pos: 1,
                seq_len: 1,
                applied_after_current: state
                    .applied
                    .tx()
                    .cloned()
                    .unwrap_or_else(|| desired.clone()),
                desired_key: state.last_desired_world_key.clone(),
                attempt_retries: 0,
                ack_timeouts: 0,
            });
            state.non_suffix_replay_due = false;
            state.non_suffix_trigger_key = state.last_desired_world_key.clone();
            warn!(
                event = "NON_SUFFIX_TRIGGER_REPLAY_QUEUED",
                held = ?capture.non_suffix,
                "queued the non-suffix remove-only trigger after a real control-seam \
                 attach (no SEAM_LOST consumption before attach)"
            );
            true
        }
        Err(_) => {
            state.pending_non_suffix = None;
            state.removal_capture = None;
            state.non_suffix_replay_due = false;
            state.non_suffix_trigger_key = None;
            state.terminal_fence = None;
            warn!(
                event = "NON_SUFFIX_GATE_DROPPED_ENCODE",
                "could not encode the held non-suffix trigger — falling back to the \
                 engine RemovedNotTail/SlotMismatch recovery"
            );
            false
        }
    }
}

/// WP2 (`docs/wp2_wp3_fence_convergence_and_adopt_card.md`): the TERMINAL fence
/// decision. A NON-structural terminal failure (bad frame / malformed / version
/// / envelope) fences the **STRUCTURAL identity** — NEVER the shaping generation
/// and NEVER the rate-inclusive `content_key`. So a **RATE-ONLY churn does NOT
/// clear the fence** (no re-emit storm), while a legitimate **STRUCTURAL** change
/// does (the broker retries it exactly once).
fn terminal_fenced(fence: Option<&str>, desired_structural: &str) -> bool {
    fence.is_some_and(|f| f == desired_structural)
}

/// #4 (`docs/control_plane_simplification_card.md` §4c; the implementer's
/// `docs/resolved_posture_spec.md`): resolve + EMIT the RESOLVED semantic
/// posture at boot. OBSERVE BEFORE COLLAPSE — this lands + runs on `.12` before
/// any knob is collapsed. The field set is pinned by `posture::tests`.
fn log_resolved_posture(config: &lqos_config::Config) {
    let input = posture::PostureInput {
        queue_mode_observe: config.queues.queue_mode.is_observe(),
        dataplane_bridge_present: config.dataplane_bridge.is_some(),
    };
    posture::log_posture(&posture::resolve_posture(input));
}

/// Starts the bridge broker actor (idempotent; named thread per the
/// lqosd actor convention). Inert under `queue_mode=observe`.
/// (a) LKG-as-cache (`docs/seam_lkg_restart_and_arms_work_order.md` item 2): the
/// desired tx-file IS the boot manifest and the source the live R3 rebuild reads.
/// When it is ABSENT at lqosd startup (a fresh box), materialize it from the
/// current effective topology so a cold bridge + the armed R3 always have the
/// FULL desired. An EXISTING file is left to the topology thread's first round
/// (the bridge's structural-identity compare handles a stale world with a
/// rebuild); the build below validates and refuses an invalid world, so a
/// failure leaves no partial manifest.
fn ensure_desired_boot_manifest(config: &lqos_config::Config, artifact_path: &std::path::Path) {
    if artifact_path.exists() {
        return;
    }
    let canonical = lqos_topology::load_canonical_topology_state(config);
    let overrides = match lqos_overrides::TopologyOverridesFile::load() {
        Ok(overrides) => overrides,
        Err(error) => {
            warn!("bridge-broker: startup boot-manifest overrides load failed: {error:#}");
            return;
        }
    };
    let health = lqos_config::TopologyAttachmentHealthStateFile::default();
    match lqos_topology::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
        config,
        &canonical,
        &overrides,
        &health,
    ) {
        Ok(artifacts) => match lqos_topology::publish_effective_topology_artifacts(config, &artifacts)
        {
            Ok(()) => info!(
                path = %artifact_path.display(),
                "bridge-broker: materialized the desired boot manifest at lqosd startup"
            ),
            Err(error) => {
                warn!("bridge-broker: startup boot-manifest publish failed: {error:#}")
            }
        },
        Err(errors) => warn!(
            "bridge-broker: startup boot-manifest build refused: {}",
            errors.join(" | ")
        ),
    }
}

pub fn start_bridge_broker(config: &lqos_config::Config) {
    if STARTED.set(()).is_err() {
        return;
    }
    if config.queues.queue_mode.is_observe() {
        info!(
            "Bridge broker inert: queue_mode=observe — the product emits nothing \
             in the observe posture (no seam traffic)"
        );
        return;
    }
    let artifact_path = topology_dataplane_transaction_path(config);
    // (a) LKG-as-cache boot manifest (`docs/seam_lkg_restart_and_arms_work_order.md`
    // item 2): the broker starts BEFORE the topology thread, so guarantee the
    // desired artifact exists for a cold bridge boot + the live R3 rebuild.
    ensure_desired_boot_manifest(config, &artifact_path);
    // R3 (remove_lpm_completeness_card §R3): the analytics socket path —
    // the attach-baseline report rides the same socket the analytics
    // actor and the Supervisor already use (env wins, then config).
 // AUDIT FIX: the BROKER must be ENV-AWARE like the actor
    // (`bridge_analytics/mod.rs:621`) and the supervisor (`bridge_supervisor.rs:99`)
    // — `analytics_socket_path` reads LIBREQOS_ANALYTICS_SOCKET FIRST, so build
    // settings even when `[dataplane_bridge]` is absent (jackrabbit sets only
    // `[bridge]` + the env). Previously this was None ⇒ the adopt was skipped ⇒
    // the attach reset the base to None, the full world re-emitted, and the
    // bridge refused the live multi-add. None only when NEITHER the section NOR
    // the env is present (the adopt is inert anyway).
 // R1: use the SINGLE shared owner so the broker, the
    // analytics actor, and the supervisor gates cannot diverge again.
    let socket_path = crate::bridge_analytics::analytics_socket_path_if_active(config);
    // #4 (`docs/control_plane_simplification_card.md`): the RESOLVED semantic
    // posture — every semantic knob the broker reads logs its RESOLVED value at
    // boot (observed, never inferred), so "which drop-in won?" is answerable
    // from the log. OBSERVE BEFORE COLLAPSE.
    log_resolved_posture(config);
    let maintenance = config.maintenance_window.clone();
    // R-OVERRIDE-PARITY: the SAME producer override the shape predicates use.
    let maintenance_override_ops = std::env::var("LIBREQOS_DPDK_BULK_GENERATION_THRESHOLD")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok());
    std::thread::Builder::new()
        .name("bridge-broker".into())
        .spawn(move || actor(artifact_path, socket_path, maintenance, maintenance_override_ops))
        .map(|_| ())
        .unwrap_or_else(|err| {
            // Review fix 4: a dead broker must not be one log line and
            // silence — surface it as an operator-visible urgent.
            error!("Failed to spawn bridge broker: {err:?}");
            urgent::submit(
                UrgentSource::System,
                UrgentSeverity::Error,
                "BRIDGE_BROKER_NOT_SPAWNED".to_string(),
                format!(
                    "The bridge broker thread failed to spawn ({err:?}) — config \
                     changes will NOT reach the bridge until lqosd restarts."
                ),
                None,
                Some("bridge-broker:spawn".to_string()),
            );
        });
}

/// C2: cumulative `(applied, rejected)` publish-ack counters for the outage
/// monitor's churn tripwire. Cold-path; no per-packet cost.
pub(crate) fn churn_counters() -> (u64, u64) {
    ack_map::churn_counters()
}

/// L4 (`docs/control_plane_structural_convergence_card.md` §5): the operator
/// convergence snapshot the Bridge page consumes. Written by the broker actor
/// each tick (cold path); read by the WS `BridgeConvergence` request.
/// §`lqos-74t.12`: one deferred pure-site create — its id and how long the
/// deferral has aged (the operator-visible "my change didn't take yet").
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeferredSite {
    /// The site inner node id.
    pub site_id: String,
    /// Seconds since the site was first deferred.
    pub age_seconds: u64,
}

/// `lqos-8h8`: the pending-surface form of a HELD maintenance-window publish.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeferredMaintenanceStatus {
    /// The human label of the held pause class.
    pub class: String,
    /// Unix seconds the held change was FIRST deferred (the force clock).
    pub first_deferred_unix: u64,
    /// The op count of the currently-held COALESCED net delta.
    pub held_ops: usize,
    /// Whether the `force_apply_hours` bound has fired for this hold.
    pub forced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemovalCapture {
    /// The FULL desired removed-circuit set (sorted).
    #[serde(default)]
    pub removed: Vec<String>,
    /// The slot-table NAME count — the engine's `owners.len()-1` (the reserved
    /// best-effort slot 0 is not a removable target) — so a readout is not
    /// misread as the tail id. The removable tail id is the LAST entry of the
    /// composed order.
    #[serde(default)]
    pub slot_table_len: usize,
    /// True when a bridge slot table was available (the suffix verdict is
    /// authoritative only then; absent, the producer kept legacy emission).
    #[serde(default)]
    pub slot_order_available: bool,
    /// True when `removed` is a contiguous suffix of the adopted slot order
    /// (equivalently: nothing was held; vacuously true for an empty set).
    #[serde(default)]
    pub suffix: bool,
    /// The non-suffix remainder that was NOT emitted (held + fenced).
    #[serde(default)]
    pub non_suffix: Vec<String>,
}

impl Default for RemovalCapture {
    fn default() -> Self {
        Self {
            removed: Vec::new(),
            slot_table_len: 0,
            slot_order_available: false,
            suffix: true,
            non_suffix: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct BridgeConvergenceSnapshot {
    /// `healthy` | `converging` | `held`.
    pub state: String,
    /// The held op(s) in OPERATOR language (L2 quarantine), deduped.
    pub held: Vec<String>,
    /// Unix seconds the current hold latched (`None` when healthy).
    pub held_since_unix: Option<u64>,
    /// The last APPLIED world's content key.
    pub applied_generation: Option<String>,
    /// Unix seconds the applied key last changed.
    pub last_applied_unix: Option<u64>,
    /// Card (`docs/seam_state_truthfulness_card.md`): Unix seconds this snapshot
    /// was persisted. The file is only as fresh as the last broker tick, so a
    /// reader can see STALENESS. Additive (`#[serde(default)]`) so an older
    /// snapshot still parses; always `>= last_applied_unix`.
    #[serde(default)]
    pub written_at_unix: u64,
    /// A2 (`docs/broker_applied_single_source_card.md` §2): the PROVENANCE of
    /// the current applied belief — `BASE_ADOPT` (the report = truth) |
    /// `BASE_ACK_FOLD` (interim) | `BASE_RESET_NONE` (no belief) | `None`
    /// (never set). An operator can tell an ADOPTED base from a FOLDED one.
    #[serde(default)]
    pub applied_provenance: Option<String>,
    /// A2: the last report OVERRODE a diverging interim ack-fold (the report
    /// WINS; surfaced). Repeats mean the ack-fold is drifting.
    #[serde(default)]
    pub applied_diverged: bool,
    /// A2: the resolved CAPABILITY the broker advertises (what the dataplane can
    /// apply live) — the DRIVER's contractual set.
    #[serde(default)]
    pub capability: Option<applied::BridgeCapability>,
    /// §`lqos-74t.12`: the sites DEFERRED because no circuit rides them yet (the
    /// desired≠applied POLICY gap), with the site's age in seconds. First-class
    /// and never silent (the OBS-1 lesson). Additive (`serde(default)`).
    #[serde(default)]
    pub deferred_sites: Vec<DeferredSite>,
    /// §`lqos-74t.13`/`.15`(d): applied sites DEFERRED for retirement because
    /// their applied subtree still bears circuits, with the age in seconds.
    /// Additive (`serde(default)`).
    #[serde(default)]
    pub deferred_retires: Vec<DeferredSite>,
    /// `lqos-8h8`: the currently-HELD maintenance-window pause-class publish
    /// (the pending banner). `None` = nothing held. Additive (`serde(default)`).
    #[serde(default)]
    pub deferred_maintenance: Option<DeferredMaintenanceStatus>,
    /// WP5 (`docs/control_plane_seam_program.md`): the RECONCILIATION state — a
    /// COMPANION to `RESOLVED_POSTURE` (not the config posture). The desired
    /// generation (the reconciler's input).
    #[serde(default)]
    pub desired_generation: Option<String>,
    /// (a) LKG-as-cache: the FULL desired world's content key
    /// (`content_key(&desired)`) — the bridge's cache-match label, so the boot
    /// verify needs no bridge-side fold. `None` until the first artifact read.
    #[serde(default)]
    pub desired_world_content_key: Option<String>,
    /// WP5: the pending sequence cursor (generation + `seq_pos/seq_len`) — `None` when idle.
    #[serde(default)]
    pub pending: Option<String>,
    /// WP5: the last typed reject (the outcome reason + the generation).
    #[serde(default)]
    pub last_reject: Option<String>,
    /// `lqos-74t.54`: the last structural-remove capture — the FULL removed set,
    /// the bridge dense-table size, the suffix verdict, and the held non-suffix
    /// remainder. Additive (`serde(default)`); answers suffix-ness at the next
    /// occurrence without re-instrumenting.
    #[serde(default)]
    pub removal_capture: Option<RemovalCapture>,
    /// `lqos-74t.102`: the non-suffix R3 gate is armed (a next structural ADD is
    /// held until the bridge's R3 full world is adopted). Additive.
    #[serde(default)]
    pub pending_non_suffix: Option<RemovalCapture>,
}

static CONVERGENCE_SNAPSHOT: OnceLock<Mutex<BridgeConvergenceSnapshot>> = OnceLock::new();

fn convergence_slot() -> &'static Mutex<BridgeConvergenceSnapshot> {
    CONVERGENCE_SNAPSHOT.get_or_init(|| Mutex::new(BridgeConvergenceSnapshot::default()))
}

/// The current operator convergence snapshot (defaults before the first tick).
pub fn convergence_snapshot() -> BridgeConvergenceSnapshot {
    convergence_slot()
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// `lqos-74t.118.2`: the read-only CPU & Workers snapshot.
//
// lqosd owns the broker's applied world + the bridge's worker-affinity report;
// lqos_api owns CPU usage/topology and the OpenAPI contract. The bus carries
// this snapshot as a versioned JSON document (the `ConvergenceState`
// precedent), keeping `lqos_bus` free of analytics_ipc/broker types.
// ---------------------------------------------------------------------------

/// The applied owner view: the leaf rows, inner-parent chain and computed
/// descendant-owner sets for the broker's applied world.
struct OwnerView {
    generation: String,
    rows: Vec<LeafRow>,
    inner_parents: HashMap<String, String>,
    sets: BTreeMap<String, BTreeSet<u16>>,
}

/// Build the owner view from the applied world. `Err` carries the honest-empty
/// reason. A missing owner map is UNAVAILABLE, never an empty supported world.
fn owner_view(applied: &applied::AppliedState) -> Result<OwnerView, &'static str> {
    let Some(tx) = applied.tx() else {
        return Err("no_applied_world");
    };
    let owners = applied.owners();
    if !applied.owners_reported() {
        return Err("owner_map_unavailable");
    }
    let mut leaf_parents: BTreeMap<String, String> = BTreeMap::new();
    for op in &tx.leaf_ops {
        match op {
            DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } => {
                leaf_parents.insert(circuit_id.clone(), parent_node_id.clone());
            }
            DataplaneLeafOp::Move {
                circuit_id,
                new_parent_node_id,
            } => {
                if let Some(parent) = leaf_parents.get_mut(circuit_id) {
                    *parent = new_parent_node_id.clone();
                }
            }
            DataplaneLeafOp::Remove { circuit_id } => {
                leaf_parents.remove(circuit_id);
            }
        }
    }
    let mut inner_parents: HashMap<String, String> = HashMap::new();
    for op in &tx.inner_ops {
        match op {
            DataplaneInnerOp::Upsert {
                node_id,
                parent_node_id,
                ..
            } => {
                inner_parents.insert(node_id.clone(), parent_node_id.clone());
            }
            DataplaneInnerOp::Move {
                node_id,
                new_parent_node_id,
            } => {
                inner_parents.insert(node_id.clone(), new_parent_node_id.clone());
            }
            DataplaneInnerOp::Remove { node_id } => {
                inner_parents.remove(node_id);
            }
        }
    }
    let rows: Vec<LeafRow> = leaf_parents
        .iter()
        .map(|(circuit_id, parent)| {
            let owner = owners.get(circuit_id).copied().unwrap_or(u16::MAX);
            (circuit_id.clone(), parent.clone(), owner)
        })
        .collect();
    // `lqos-74t.118.2` pin: the `u16::MAX` sentinel is NOT a worker id. Exclude
    // unowned circuits from the node owner sets/crossings entirely; they are
    // counted separately (`unowned_circuits`) and never turn a node into a
    // crossing or appear in `worker_ids`.
    let owned_rows: Vec<LeafRow> = rows
        .iter()
        .filter(|(_, _, owner)| *owner != u16::MAX)
        .cloned()
        .collect();
    let sets = compute_owner_sets(&owned_rows, &inner_parents);
    Ok(OwnerView {
        generation: tx.generation.clone(),
        rows,
        inner_parents,
        sets,
    })
}

fn unavailable_ownership(reason: &str) -> serde_json::Value {
    serde_json::json!({
        "available": false,
        "reason": reason,
        "generation": null,
        "circuits": null,
        "nodes": null,
        "crossings": null,
        "unowned_circuits": null,
    })
}

/// `lqos-74t.118.3`: the operator-view display names, cached ONCE per report
/// (never looked up per row). An absent/empty name is omitted (the UI falls
/// back to the stable id); no empty string is fabricated.
fn display_name_map_from_view(
    view: &lqos_config::TopologyOperatorViewFile,
) -> HashMap<String, String> {
    view.nodes
        .iter()
        .filter_map(|node| {
            let name = node.name.trim();
            if name.is_empty() {
                None
            } else {
                Some((node.id.clone(), name.to_string()))
            }
        })
        .collect()
}

fn display_name_map() -> HashMap<String, String> {
    let Ok(config) = lqos_config::load_config() else {
        return HashMap::new();
    };
    let Ok(view) = lqos_config::TopologyOperatorViewFile::load(config.as_ref()) else {
        return HashMap::new();
    };
    display_name_map_from_view(&view)
}

fn ownership_json(
    applied: &applied::AppliedState,
    names: &HashMap<String, String>,
) -> serde_json::Value {
    let view = match owner_view(applied) {
        Ok(view) => view,
        Err(reason) => return unavailable_ownership(reason),
    };
    let unowned = view
        .rows
        .iter()
        .filter(|(_, _, owner)| *owner == u16::MAX)
        .count();
    let circuits: Vec<serde_json::Value> = view
        .rows
        .iter()
        .map(|(circuit_id, parent_node_id, owner)| {
            serde_json::json!({
                "circuit_id": circuit_id,
                "parent_node_id": parent_node_id,
                "name": names.get(circuit_id.as_str()),
                "worker_id": if *owner == u16::MAX {
                    serde_json::Value::Null
                } else {
                    serde_json::json!(owner)
                },
            })
        })
        .collect();
    let nodes: Vec<serde_json::Value> = view
        .sets
        .iter()
        .map(|(node_id, worker_ids)| {
            serde_json::json!({
                "node_id": node_id,
                "parent_node_id": view.inner_parents.get(node_id),
                "name": names.get(node_id.as_str()),
                "worker_ids": worker_ids.iter().copied().collect::<Vec<u16>>(),
                "crossing": worker_ids.len() > 1,
            })
        })
        .collect();
    let crossings: Vec<String> = crossing_nodes(&view.sets).into_iter().collect();
    serde_json::json!({
        "available": true,
        "reason": null,
        "generation": view.generation,
        "circuits": circuits,
        "nodes": nodes,
        "crossings": crossings,
        "unowned_circuits": unowned,
    })
}

fn unavailable_workers(reason: &str) -> serde_json::Value {
    serde_json::json!({
        "available": false,
        "reason": reason,
        "schema": null,
        "rx_queue_count": null,
        "tx_queue_count": null,
        "counter_support": null,
        "rows": null,
    })
}

fn workers_json(
    applied: &applied::AppliedState,
    report: Option<&WorkerAffinityReport>,
) -> serde_json::Value {
    let Some(report) = report else {
        return unavailable_workers("bridge_report_unavailable");
    };
    let view = owner_view(applied).ok();
    let rows: Vec<serde_json::Value> = report
        .workers
        .iter()
        .map(|worker| {
            let circuit_count = view.as_ref().map(|view| {
                view.rows
                    .iter()
                    .filter(|(_, _, owner)| {
                        *owner != u16::MAX && *owner == worker.worker_id
                    })
                    .count() as u32
            });
            // Cheap pin: count only nodes this worker SOLELY owns (not shared
            // with another worker); a crossing node is not "owned".
            let solely_owned_node_count = view.as_ref().map(|view| {
                view.sets
                    .values()
                    .filter(|owners| owners.len() == 1 && owners.contains(&worker.worker_id))
                    .count() as u32
            });
            serde_json::json!({
                "worker_id": worker.worker_id,
                "lcore_id": worker.lcore_id,
                "numa_node": worker.numa_node,
                "rx_queue_ids": worker.rx_queue_ids.clone(),
                "tx_queue_id": worker.tx_queue_id,
                "shared_tx_queue": worker.shared_tx_queue,
                "circuit_count": circuit_count,
                "solely_owned_node_count": solely_owned_node_count,
                "counters": worker.counters,
                "throughput": null,
                "throughput_support": "unsupported",
                "drops": null,
                "drops_support": "unsupported",
            })
        })
        .collect();
    serde_json::json!({
        "available": true,
        "reason": null,
        "schema": report.schema,
        "rx_queue_count": report.rx_queue_count,
        "tx_queue_count": report.tx_queue_count,
        "counter_support": report.counters,
        "rows": rows,
    })
}

fn moves_json() -> serde_json::Value {
    // v1 has no source: `move_journal` is a pre-move crash oracle cleared on
    // commit, and the bridge's move history is log-only. Honest-empty, never
    // synthesized from logs. The owner-map diff ring is a later optimization.
    serde_json::json!({
        "available": false,
        "reason": "no_history_source_v1",
        "source": null,
        "since_unix": null,
        "items": null,
    })
}

#[cfg(test)]
mod cpu_workers_name_tests {
    use super::*;

    #[test]
    fn display_name_map_keeps_names_and_drops_empty_ones() {
        let named: lqos_config::OperatorViewNode = serde_json::from_value(serde_json::json!({
            "id": "site:1",
            "name": "Site One",
            "type": "site"
        }))
        .expect("named node");
        let unnamed: lqos_config::OperatorViewNode = serde_json::from_value(serde_json::json!({
            "id": "site:2",
            "name": "   ",
            "type": "site"
        }))
        .expect("unnamed node");
        let view = lqos_config::TopologyOperatorViewFile {
            nodes: vec![named, unnamed],
            ..Default::default()
        };
        let names = display_name_map_from_view(&view);
        assert_eq!(names.get("site:1").map(String::as_str), Some("Site One"));
        assert!(
            !names.contains_key("site:2"),
            "an empty/whitespace name must fall back to the id, never emit an empty string"
        );
    }
}

fn bridge_json(convergence: &BridgeConvergenceSnapshot) -> serde_json::Value {
    let available = convergence.applied_generation.is_some();
    let configured = crate::node_manager::local_api::dpdk_status::configured_autobalance();
    serde_json::json!({
        "available": available,
        "reason": if available { serde_json::Value::Null } else { serde_json::json!("no_bridge_report") },
        "state": if convergence.state.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::json!(convergence.state.clone())
        },
        "applied_generation": convergence.applied_generation.clone(),
        "desired_generation": convergence.desired_generation.clone(),
        "applied_provenance": convergence.applied_provenance.clone(),
        "applied_diverged": if available {
            serde_json::json!(convergence.applied_diverged)
        } else {
            serde_json::Value::Null
        },
        "pending": convergence.pending.clone(),
        "last_reject": convergence.last_reject.clone(),
        "deferred_maintenance": serde_json::to_value(&convergence.deferred_maintenance)
            .unwrap_or(serde_json::Value::Null),
        "autobalance": {
            "configured": configured,
            "runtime": serde_json::Value::Null,
            "reason": "log_only",
        },
    })
}

fn cpu_workers_base() -> serde_json::Value {
    cpu_workers_slot()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(|| {
            serde_json::json!({
                "workers": unavailable_workers("no_bridge_report"),
                "ownership": unavailable_ownership("no_bridge_report"),
                "moves": moves_json(),
            })
        })
}

static CPU_WORKERS_SNAPSHOT: OnceLock<Mutex<Option<serde_json::Value>>> = OnceLock::new();

fn cpu_workers_slot() -> &'static Mutex<Option<serde_json::Value>> {
    CPU_WORKERS_SNAPSHOT.get_or_init(|| Mutex::new(None))
}

/// Replace the stored worker/ownership/moves snapshot after an adoption. The
/// bridge section is assembled per request from the convergence snapshot (it
/// changes independently of an adoption).
fn publish_cpu_workers_snapshot(
    applied: &applied::AppliedState,
    workers: Option<WorkerAffinityReport>,
) {
    let names = display_name_map();
    let doc = serde_json::json!({
        "workers": workers_json(applied, workers.as_ref()),
        "ownership": ownership_json(applied, &names),
        "moves": moves_json(),
    });
    if let Ok(mut slot) = cpu_workers_slot().lock() {
        *slot = Some(doc);
    }
}

/// Clear the stored snapshot on a re-attach/boot reset; the next adoption
/// repopulates it. Until then the endpoint is explicitly unavailable.
fn clear_cpu_workers_snapshot() {
    if let Ok(mut slot) = cpu_workers_slot().lock() {
        *slot = None;
    }
}

/// `lqos-74t.118.2`: the bus-facing CPU & Workers document. Versioned; the API
/// parses it into its OpenAPI DTOs. Never blocks the broker actor.
pub fn cpu_workers_json() -> String {
    let base = cpu_workers_base();
    let convergence = convergence_snapshot();
    let doc = serde_json::json!({
        "schema": 1,
        "generated_at_unix": now_unix(),
        "bridge": bridge_json(&convergence),
        "workers": base.get("workers").cloned().unwrap_or(serde_json::Value::Null),
        "ownership": base.get("ownership").cloned().unwrap_or(serde_json::Value::Null),
        "moves": base.get("moves").cloned().unwrap_or(serde_json::Value::Null),
    });
    serde_json::to_string(&doc).unwrap_or_else(|_| "{}".to_string())
}

/// `lqos-8h8`: should this emitted delta be HELD by the maintenance window?
///
/// Returns `true` (HOLD — never publish) iff the delta's DERIVED pause class
/// ([`maintenance::pause_class`]) is non-`None` AND the window is CLOSED AND the
/// `force_apply_hours` bound has not fired. A PAUSE-FREE delta CLEARS any held
/// change and returns `false` (never deferred — §14 pin 1). Side effects: the
/// held `deferred_maintenance` is refreshed (coalesced) + a one-shot WARN per
/// new hold / a force WARN.
fn maintenance_should_hold(
    state: &mut BrokerState,
    delta: &DataplaneTransaction,
    world: usize,
    desired_key: &str,
) -> bool {
    let class = maintenance::pause_class(
        delta,
        state.applied.tx(),
        world,
        state.maintenance_override_ops,
        state.applied.owners(),
    );
    let Some(class) = class else {
        // PAUSE-FREE: never held.
        state.deferred_maintenance = None;
        return false;
    };
    let now = now_unix();
    // Operator OVERRIDES of the gate only (Q9/Q11): a time-boxed GLOBAL
    // break-glass (while it holds) OR a per-change apply-now keyed on the held
    // content key. Both just bypass the window ONCE — the engine's classify
    // still governs the emitted generation.
    let break_glass = state
        .break_glass_until_unix
        .is_some_and(|until| now < until);
    let apply_now = state.apply_now;
    if apply_now {
        warn!(
            event = "MAINTENANCE_WINDOW_APPLY_NOW",
            key = %desired_key,
            "operator apply-now — emitting the held pause-class publish immediately (lqos-8h8)"
        );
        state.apply_now = false;
    }
    if break_glass || apply_now {
        state.deferred_maintenance = None;
        return false;
    }
    if state.break_glass_until_unix.is_some_and(|until| now >= until) {
        state.break_glass_until_unix = None; // expired (self-healing)
    }
    let force_due = state.deferred_maintenance.as_ref().is_some_and(|held| {
        now.saturating_sub(held.first_deferred_unix)
            >= state.maintenance.force_apply_hours.saturating_mul(3600)
    });
    if state
        .maintenance
        .is_open_at(maintenance::now_local_minutes(state.maintenance_tz))
    {
        state.deferred_maintenance = None;
        return false;
    }
    if force_due {
        warn!(
            event = "MAINTENANCE_WINDOW_FORCE",
            class = class.label(),
            "force-applying a held pause-class publish past force_apply_hours (lqos-8h8)"
        );
        state.deferred_maintenance = None;
        return false;
    }
    let first = state
        .deferred_maintenance
        .as_ref()
        .map(|held| held.first_deferred_unix)
        .unwrap_or(now);
    let held_ops = delta.inner_ops.len() + delta.leaf_ops.len() + delta.mapping_ops.len();
    let was_new = state.deferred_maintenance.is_none();
    state.deferred_maintenance = Some(maintenance::DeferredMaintenance {
        class,
        first_deferred_unix: first,
        held_ops,
        forced: false,
    });
    if was_new {
        warn!(
            event = "MAINTENANCE_WINDOW_HOLD",
            class = class.label(),
            held_ops,
            "maintenance window closed — holding a pause-class publish (lqos-8h8)"
        );
    }
    true
}

/// `lqos-8h8` (c): the operator OVERRIDE control-file path (the WS handler writes
/// it; the broker consumes it on its next tick). Sibling of the artifact.
pub fn maintenance_control_path(config: &lqos_config::Config) -> std::path::PathBuf {
    topology_dataplane_transaction_path(config)
        .parent()
        .map(|dir| dir.join("maintenance_control.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("maintenance_control.json"))
}

/// `lqos-8h8` (c): consume the operator OVERRIDE control file ONCE (apply-now /
/// break-glass), audited. The broker owns a single-threaded `BrokerState`, so
/// the WS handler cannot mutate it directly — the control file is the seam (like
/// the artifact). A malformed file is ignored + removed (never a panic / spin).
fn refresh_maintenance_control(state: &mut BrokerState) {
    let Some(path) = state.maintenance_control_path.clone() else {
        return;
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    let _ = std::fs::remove_file(&path); // consume once (audit is below)
    let Ok(control) = serde_json::from_str::<maintenance::MaintenanceControl>(&raw) else {
        warn!(
            event = "MAINTENANCE_CONTROL_MALFORMED",
            path = %path.display(),
            "maintenance control file is malformed — ignored (lqos-8h8)"
        );
        return;
    };
    let by = control.requested_by.as_deref().unwrap_or("unknown");
    if control.apply_now {
        warn!(
            event = "MAINTENANCE_CONTROL_APPLY_NOW",
            requested_by = %by,
            "operator APPLY-NOW accepted for the held change (audit) (lqos-8h8)"
        );
        state.apply_now = true;
    }
    if let Some(until) = control.break_glass_until_unix {
        warn!(
            event = "MAINTENANCE_CONTROL_BREAK_GLASS",
            until_unix = until,
            requested_by = %by,
            "operator BREAK-GLASS armed (time-boxed global, audited) (lqos-8h8)"
        );
        state.break_glass_until_unix = Some(until);
    }
}

/// L4 §5a: publish the operator snapshot from the live broker state (called at
/// the end of every `step`).
fn publish_convergence_snapshot(state: &BrokerState) {
    let now = now_unix();
    let Ok(mut slot) = convergence_slot().lock() else {
        return;
    };
    // Card (`docs/seam_state_truthfulness_card.md`): stamp the write so a reader
    // can see the snapshot's staleness (`now >= last_applied_unix`).
    slot.written_at_unix = now;
    let is_held = !state.held_ops.is_empty();
    slot.state = if is_held {
        "held"
    } else if state.pending.is_some() {
        "converging"
    } else {
        "healthy"
    }
    .to_string();
    if is_held {
        if slot.held_since_unix.is_none() {
            slot.held_since_unix = Some(now);
        }
    } else {
        slot.held_since_unix = None;
    }
    let mut held: Vec<String> = state
        .held_ops
        .iter()
        .map(|op| op.operator_text())
        .collect();
    held.sort();
    held.dedup();
    slot.held = held;
    if slot.applied_generation.as_deref() != state.applied.key() {
        slot.last_applied_unix = Some(now);
    }
    slot.applied_generation = state.applied.key().map(str::to_string);
    // A2: the provenance + the divergence flag (the operator's missing input).
    slot.applied_provenance = state.applied.provenance().map(|p| p.as_str().to_string());
    slot.applied_diverged = state.applied.diverged();
    // A2: the resolved capability (the contractual set). Resolved once per tick
    // (cold path) from the SAME shared readers the producer/bridge use.
    slot.capability = Some(applied::BridgeCapability::resolve());
    // §`lqos-74t.12`: surface the aged pure-site deferrals (never silent).
    slot.deferred_sites = state
        .deferred_sites
        .iter()
        .map(|(site, first_seen)| DeferredSite {
            site_id: site.clone(),
            age_seconds: now.saturating_sub(*first_seen),
        })
        .collect();
    slot.deferred_sites.sort_by(|a, b| a.site_id.cmp(&b.site_id));
    slot.deferred_maintenance = state
        .deferred_maintenance
        .as_ref()
        .map(|held| DeferredMaintenanceStatus {
            class: held.class.label().to_string(),
            first_deferred_unix: held.first_deferred_unix,
            held_ops: held.held_ops,
            forced: held.forced,
        });
    // §`lqos-74t.12`: an AGE-THRESHOLD alarm — a deferral that ages past the
    // bound is operator-visible (a POLICY gap, never a silent drop). Deduped by
    // identity, so a long deferral is ONE alert, not a storm.
    if let Some(oldest) = slot.deferred_sites.iter().max_by_key(|d| d.age_seconds)
        && oldest.age_seconds >= DEFERRED_SITE_ALARM_SECONDS
    {
        urgent::submit(
            UrgentSource::System,
            UrgentSeverity::Warning,
            "DEFERRED_SITE_CREATE_AGED".to_string(),
            format!(
                "A new site has been deferred for {}s because no circuit rides it yet ({}). \
                 The site is inert until its first circuit arrives; use the M-A route to \
                 apply the shell now.",
                oldest.age_seconds, oldest.site_id
            ),
            None,
            Some("bridge-broker:deferred-site-aged".to_string()),
        );
    }
    // §`lqos-74t.13`/`.15`(d): surface the aged RETIRE deferrals (never silent).
    slot.deferred_retires = state
        .deferred_retires
        .iter()
        .map(|(site, first_seen)| DeferredSite {
            site_id: site.clone(),
            age_seconds: now.saturating_sub(*first_seen),
        })
        .collect();
    slot.deferred_retires.sort_by(|a, b| a.site_id.cmp(&b.site_id));
    if let Some(oldest) = slot.deferred_retires.iter().max_by_key(|d| d.age_seconds)
        && oldest.age_seconds >= DEFERRED_SITE_ALARM_SECONDS
    {
        urgent::submit(
            UrgentSource::System,
            UrgentSeverity::Warning,
            "DEFERRED_SITE_RETIRE_AGED".to_string(),
            format!(
                "A retired site has been deferred for {}s because its applied subtree still \
                 bears circuits ({}). It retires one generation after the circuits drain and \
                 ack.",
                oldest.age_seconds, oldest.site_id
            ),
            None,
            Some("bridge-broker:deferred-retire-aged".to_string()),
        );
    }
    // WP5: the reconciliation state (the companion surface; NOT the config posture).
    slot.desired_generation = state.last_desired_generation.clone();
    slot.desired_world_content_key = state.last_desired_world_key.clone();
    slot.pending = state
        .pending
        .as_ref()
        .map(|p| format!("{} {}/{}", p.generation, p.seq_pos, p.seq_len));
    slot.last_reject = outcomes::publish_outcomes()
        .into_iter()
        .find_map(|r| match r.state {
            outcomes::PublishState::Rejected { reason } => Some(format!("{reason} @ {}", r.generation)),
            _ => None,
        });
    // `lqos-74t.54`: the ALWAYS-ON structural-remove capture (the full removed
    // set, the dense-table size, the suffix verdict) — the next occurrence
    // answers suffix-ness from this snapshot, level-independent.
    slot.removal_capture = state.removal_capture.clone();
    slot.pending_non_suffix = state.pending_non_suffix.clone();
    tracing::info!(
        event = "SEAM_STATE",
        schema = 1,
        desired_generation = ?slot.desired_generation,
        applied_generation = ?slot.applied_generation,
        applied_provenance = ?slot.applied_provenance,
        pending = ?slot.pending,
        last_reject = ?slot.last_reject,
        state = %slot.state,
        held = slot.held.len(),
        "reconciliation state (desired -> applied; the seam, NOT the config posture)"
    );
    // WP5 observability: lqosd defaults to WARN, so the `info!`
    // above never reaches the journal at the default. Mirror the
    // RESOLVED_POSTURE pattern — persist the snapshot to a well-known file so
    // `lqosd --print-seam-state` is level-independent + diffable.
    if let Ok(json) = serde_json::to_string_pretty(&*slot) {
        let path = std::path::Path::new(SEAM_STATE_PATH);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        write_snapshot_atomic(path, &json);
    }
}

/// Card (`docs/seam_state_truthfulness_card.md`): same-dir temp + rename, so a
/// concurrent reader never observes a partial snapshot. Extracted so the
/// concurrency oracle runs in a temp dir (the production path is `/run/libreqos`).
fn write_snapshot_atomic(path: &std::path::Path, json: &str) {
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn actor(
    artifact: PathBuf,
    socket_path: Option<std::path::PathBuf>,
    maintenance: lqos_config::MaintenanceWindowConfig,
    maintenance_override_ops: Option<usize>,
) {
    if maintenance.enabled && maintenance.window_minutes().is_none() {
        warn!(
            event = "MAINTENANCE_WINDOW_NORMALISED",
            start = %maintenance.start_local,
            end = %maintenance.end_local,
            "maintenance_window is disabled, malformed or zero-length — deferring NOTHING \
             (window always open) (lqos-8h8)"
        );
    }
    // (b) Q4: an explicit IANA `timezone` is DST-correct via `chrono-tz`; an
    // empty zone = server-local; an UNKNOWN name falls back to server-local with
    // a LOUD one-shot WARN (otherwise it is wrong-time SILENTLY).
    let maintenance_tz = match maintenance::resolve_timezone(&maintenance.timezone) {
        Ok(tz) => tz,
        Err(()) => {
            warn!(
                event = "MAINTENANCE_WINDOW_TZ_UNKNOWN",
                timezone = %maintenance.timezone,
                "maintenance_window.timezone is not a known IANA zone — falling back to the \
                 SERVER-LOCAL zone; the window may be at the WRONG TIME (lqos-8h8)"
            );
            None
        }
    };
    // Q9/Q10 gap (review pin): apply-now + break-glass are ABSENT
    // (no WS/bus). Surface it LOUD and pin a DATED GATE — the gate holds +
    // force_applies only until these land.
    if maintenance.enabled {
        // INFO (not WARN): this fires on EVERY default-enabled start, so a WARN
        // would be per-boot noise (review note). The dated removal gate stays.
        info!(
            event = "MAINTENANCE_WINDOW_OVERRIDES_UNAVAILABLE",
            "maintenance-window APPLY-NOW + BREAK-GLASS are NOT yet wired (no WS/bus) — the \
             gate holds + force_applies only; REMOVAL GATE: wire before 2026-12-01 / GA (lqos-8h8)"
        );
    }
    info!(
        event = "MAINTENANCE_WINDOW_RESOLVED",
        enabled = maintenance.enabled,
        start_local = %maintenance.start_local,
        end_local = %maintenance.end_local,
        max_pause_ms = maintenance.max_pause_ms,
        force_apply_hours = maintenance.force_apply_hours,
        "maintenance_window resolved (lqos-8h8)"
    );
    let mut state = BrokerState {
        segment: None,
        applied: applied::AppliedState::default(),
        pending: None,
        terminal_fence: None,
        last_desired_generation: None,
        last_desired_world_key: None,
        duplicate_mapping_alerts: BTreeSet::new(),
        held_ops: BTreeSet::new(),
        held_structural_key: None,
        held_retry_at: None,
        held_retries: 0,
        removal_capture: None,
        pending_non_suffix: None,
        non_suffix_replay_due: false,
        non_suffix_trigger_key: None,
        deferred_sites: BTreeMap::new(),
        deferred_retires: BTreeMap::new(),
        artifact_cache: None,
        bad_layout_reported: false,
        attach_polls: 0,
        liveness_polls: 0,
        slot_order: None,
        socket_path,
        rejects_consecutive: 0,
        backoff_until: None,
        report_reread_at: None,
        report_reread_due: false,
            late_adopt: false,
        maintenance,
        maintenance_tz,
        maintenance_override_ops,
        deferred_maintenance: None,
        apply_now: false,
        break_glass_until_unix: None,
        maintenance_control_path: artifact
            .parent()
            .map(|dir| dir.join("maintenance_control.json")),
    };
    // `lqos-74t.102`: a broker restart must not lose the non-suffix gate while
    // the bridge may still hold the partial table. Restore the persisted gate,
    // read the CURRENT artifact (before any report reconcile), and replay the
    // remove-only trigger. Missing artifact/trigger falls back to the engine
    // RemovedNotTail/SlotMismatch recovery.
    restore_non_suffix_gate_from(
        std::path::Path::new(SEAM_STATE_PATH),
        &artifact,
        &mut state,
    );
    loop {
        step(&artifact, &mut state);
        std::thread::sleep(POLL_ARTIFACT);
    }
}

/// R3: how the attach-baseline adoption resolved. `Report` = the
/// bridge's fence-committed applied state (adopt verbatim);
/// `Unavailable` carries the operator-facing reason (the fallback is
/// today's full re-emit posture + ONE named warning per attach).
enum AdoptedBaseline {
    Report {
        tx: DataplaneTransaction,
        /// WP3: the bridge's REPORTED content key (typed);
        /// empty from a pre-WP3 bridge => the broker derives it (back-compat).
        content_key: String,
        /// `lqos-101`: the DESIRED world key reported VERBATIM by the bridge
        /// (the frame key carried with the applied world). Empty from a
        /// pre-fix/boot report => the late guard falls back to `content_key`.
        desired_world_content_key: String,
        slot_order: Option<Vec<String>>,
        /// `lqos-8h8` (a): the per-slot PLACEMENT OWNER (`Vec<u16>`, parallel to
        /// `slot_order`). `None` = a pre-owner bridge (the reparent twin is
        /// inert).
        owners: Option<Vec<u16>>,
        /// `lqos-74t.118.1`/`.118.2`: the bridge's worker-affinity report.
        /// `None` = a pre-`.118.1` bridge or an unwired registry; the API must
        /// render it unavailable, never as zeros.
        workers: Option<WorkerAffinityReport>,
    },
    Unavailable(String),
}

fn adopt_bridge_baseline(socket: &std::path::Path) -> AdoptedBaseline {
    match analytics_ipc::connect_state(socket) {
        Ok(report) => {
            if report.report_schema != 1 {
                return AdoptedBaseline::Unavailable(format!(
                    "unknown report schema {}",
                    report.report_schema
                ));
            }
            let slot_order: Option<Vec<String>> = if report.slot_order.is_empty() {
                None
            } else {
                match ciborium::from_reader::<Vec<String>, _>(&report.slot_order[..]) {
                    Ok(order) => Some(order),
                    Err(error) => {
                        return AdoptedBaseline::Unavailable(format!(
                            "the slot_order bytes do not parse: {error:?}"
                        ));
                    }
                }
            };
            // `lqos-8h8` (a): the per-slot owner (parallel to `slot_order`).
            let owners: Option<Vec<u16>> = if report.owners.is_empty() {
                None
            } else {
                ciborium::from_reader::<Vec<u16>, _>(&report.owners[..]).ok()
            };
            match ciborium::from_reader::<DataplaneTransaction, _>(&report.transaction[..]) {
                Ok(tx) => {
                    // §8 (c): note the bridge's APPLIED capacity so the emitter's
                    // derived bound never shrinks across a bridge restart.
                    lqos_topology::note_applied_circuit_capacity(
                        report.applied_circuit_capacity,
                    );
                    AdoptedBaseline::Report {
                        tx,
                        content_key: report.content_key,
                        desired_world_content_key: report.desired_world_content_key,
                        slot_order,
                        owners,
                        workers: report.workers,
                    }
                }
                Err(error) => AdoptedBaseline::Unavailable(format!(
                    "the report's transaction bytes do not parse: {error:?}"
                )),
            }
        }
        Err(analytics_ipc::IpcError::StateUnavailable(reason)) => {
            AdoptedBaseline::Unavailable(format!("typed unavailable: {reason:?}"))
        }
        Err(error @ analytics_ipc::IpcError::MalformedStateFrame { .. }) => {
            // `lqos-74t.15.3` MalformedFrame localization (diagnostic only): the
            // broker's state-report read failed at a NAMED sub-arm + the request
            // kind — so the head-vs-P1 parse divergence is attributable.
            warn!(
                target: "lqos_state_report_diag",
                error = ?error,
                "MalformedFrame at the broker state-report read (named sub-arm + RequestKind)"
            );
            AdoptedBaseline::Unavailable(format!("{error:?}"))
        }
        Err(error) => AdoptedBaseline::Unavailable(format!("{error:?}")),
    }
}

/// R3c-3 (remove_lpm_completeness_card): the adoption-time classification
/// of the bridge's slot table against the report's applied fold. THE
/// MATRIX:
/// - `Proceed { paradox: false }` — fold order == slot order (the healthy
///   L1-invariant world).
/// - `Proceed { paradox: true }` — same set, REORDERED: the L1
///   belief-breaker, instrumented. PROCEED on slot_order for pops AND
///   views (every drawable position has content; the compile lands
///   exact) + the PARADOX urgent.
/// - `Stray` — MEMBERSHIP divergence: the slot table names circuits the
///   fold never saw (e.g. an install-completed/fence-lost crash window).
///   The walk has NO drawable arm — skipping a position shifts every
///   later survivor down one slot (the F1 harm inside the paradox
///   state). ABORT to the restart-class path; never drain through a
///   world the broker cannot draw.
/// - `Absent` — the report carried no slot table (old bridge): fold-order
///   fallback + the named degrade.
#[derive(Debug, PartialEq)]
enum SlotOrderClass {
    Proceed { paradox: bool },
    Stray(Vec<String>),
    Absent,
}

fn classify_slot_order(
    tx: &DataplaneTransaction,
    slot_order: &Option<Vec<String>>,
) -> SlotOrderClass {
    let Some(order) = slot_order else {
        return SlotOrderClass::Absent;
    };
 // the matrix classifies on the COMPOSED universe — the
    // same order source semantic_diff walks (table ++ baseline tail).
    // At adoption the baseline IS the report tx, so the tail contributes
    // nothing new; composition here keeps the classifier and the walker
    // ONE mechanism at two scopes.
    let composed = translate::compose_order(Some(order), Some(tx));
    let tx_names: std::collections::BTreeSet<&String> = tx
        .leaf_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneLeafOp::Upsert { circuit_id, .. } => Some(circuit_id),
            _ => None,
        })
        .collect();
    let strays: Vec<String> = composed
        .iter()
        .filter(|name| !tx_names.contains(*name))
        .cloned()
        .collect();
    if !strays.is_empty() {
        return SlotOrderClass::Stray(strays);
    }
    let fold_order: Vec<String> = tx
        .leaf_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneLeafOp::Upsert { circuit_id, .. } => Some(circuit_id.clone()),
            _ => None,
        })
        .collect();
    SlotOrderClass::Proceed {
        paradox: &fold_order != &composed,
    }
}

/// A3 (`docs/broker_applied_single_source_card.md` §2): reconcile the broker's
/// applied belief from the bridge's report — the TRUTH. Shared by the attach
/// path and the periodic/after-reject CADENCE. The report WINS over the interim
/// ack-fold; a divergence is surfaced. Returns `true` when a base was adopted.
///
/// `cadence` distinguishes the periodic re-read (quieter: it does not re-raise
/// the `BRIDGE_STATE_REPORT_UNAVAILABLE` urgent — the A1 surface already stands
/// while there is no base) from the attach read.
fn reconcile_from_report(state: &mut BrokerState, cadence: bool) -> bool {
    let Some(socket) = state.socket_path.clone() else {
        return false;
    };
    let (tx, content_key, desired_world_content_key, slot_order, owners, workers) =
        match adopt_bridge_baseline(&socket) {
        AdoptedBaseline::Report {
            tx,
            content_key,
            desired_world_content_key,
            slot_order,
            owners,
            workers,
        } => (tx, content_key, desired_world_content_key, slot_order, owners, workers),
            AdoptedBaseline::Unavailable(reason) => {
                // A1: a missing base is a HOLD + surface + RE-ASK (this cadence),
                // never a fabricated full-world (the last live non-incremental emit).
                warn!("BRIDGE_STATE_REPORT_UNAVAILABLE: {reason} — the broker holds and re-asks");
                if !cadence {
                    urgent::submit(
                        UrgentSource::System,
                        UrgentSeverity::Warning,
                        "BRIDGE_STATE_REPORT_UNAVAILABLE".to_string(),
                        format!(
                            "The bridge's applied-state report was unavailable ({reason}); the \
                             broker is HOLDING and re-asking (no full-world fabrication). The \
                             bridge keeps its own world; config changes apply automatically once \
                             the report returns."
                        ),
                        None,
                        Some("bridge-broker:state-report-unavailable".to_string()),
                    );
                }
                return false;
            }
        };
    // R3c-3: the paradox classification — the report's fold order vs the
    // bridge's slot table (the matrix, pure fn above).
    let slot_order = match classify_slot_order(&tx, &slot_order) {
        SlotOrderClass::Stray(strays) => {
            urgent::submit(
                UrgentSource::System,
                UrgentSeverity::Warning,
                "BRIDGE_SLOT_ORDER_STRAY".to_string(),
                format!(
                    "The bridge's slot table names circuits this broker never saw \
                     ({strays:?}) — the drain view cannot draw that world. Removals are \
                     RESTART-CLASS this generation; a bridge reload re-seeds the world."
                ),
                None,
                Some("bridge-broker:slot-stray".to_string()),
            );
            warn!("BRIDGE_SLOT_ORDER_STRAY: {strays:?} — baseline NOT adopted (HOLD)");
            return false;
        }
        SlotOrderClass::Proceed { paradox } => {
            if paradox {
                urgent::submit(
                    UrgentSource::System,
                    UrgentSeverity::Warning,
                    "BRIDGE_SLOT_ORDER_PARADOX".to_string(),
                    "The bridge's slot order diverges from the applied fold (the L1 belief \
                     breaker, now instrumented). Proceeding on the bridge's slot order for \
                     drains and views."
                        .to_string(),
                    None,
                    Some("bridge-broker:slot-paradox".to_string()),
                );
            }
            slot_order
        }
        SlotOrderClass::Absent => {
 // Absent-branch adopt MUST still carry the BASE_ADOPT marker (the implementer
 // REQUIRED): the named degrade is the slot order, not the base.
            warn!(
                "BRIDGE_SLOT_ORDER_ABSENT: the report carries no slot table — drains/views \
                 key on the fold order (one named degrade)"
            );
            None
        }
    };
    let generation = tx.generation.clone();
    let inners = tx.inner_ops.len();
    let leaves = tx.leaf_ops.len();
    let has_slot_order = slot_order.is_some();
    // WP3 (`docs/wp2_wp3_fence_convergence_and_adopt_card.md` §2): ADOPT via the
    // typed `AppliedGeneration` — the ONLY input an adopt may take (the bridge's
    // REPORTED applied; never assumed). A report that DISAGREES with the interim
    // ack-fold WINS and returns `true` (the divergence to surface).
    // WP3: adopt the bridge's REPORTED content key
    // VERBATIM (the typed field). A pre-WP3 bridge sends an EMPTY key => derive
    // it from the reported transaction (the back-compat arm). The shared
    // `lqos_dataplane_types::content_key` is the ONE home.
    let content_key = if content_key.is_empty() {
        translate::content_key(&tx)
    } else {
        content_key
    };
    // `lqos-101`: the report's DESIRED identity. The bridge carries the frame
    // key VERBATIM in the additive field; a pre-fix/boot report has an empty
    // field => fall back to the folded `content_key` (old behavior, including
    // the old late-adopt rejection). The applied belief below still uses the
    // folded `content_key` (cache/integrity/divergence unchanged).
    let desired_world_content_key = if desired_world_content_key.is_empty() {
        content_key.clone()
    } else {
        desired_world_content_key
    };
    // `lqos-74t.101`: the ACK-silence late-adopt path may only accept the
    // bridge's report when it proves the CURRENT desired world (DESIRED key +
    // generation). An old/partial report is not a late apply; returning false
    // keeps the pending attempt held without a false terminal reject.
    let late = state.late_adopt;
    if late
        && (state.last_desired_world_key.as_deref() != Some(desired_world_content_key.as_str())
            || state.last_desired_generation.as_deref() != Some(tx.generation.as_str()))
    {
        return false;
    }
    // `lqos-74t.102`: while a non-suffix R3 gate is armed, NEVER adopt an
    // intermediate/partial report. The ONLY report that may advance the applied
    // state (and clear the gate) is the R3 full desired world, identified by the
    // full desired content key. A stale/partial report keeps the old applied
    // world and the gate; the cadence re-asks on the short retry.
    let pending_non_suffix = state.pending_non_suffix.clone();
    if let Some(capture) = pending_non_suffix.as_ref() {
        let expected = state.last_desired_world_key.as_deref();
        if expected != Some(desired_world_content_key.as_str()) {
            warn!(
                event = "NON_SUFFIX_R3_WAIT",
                expected = ?expected,
                reported = %content_key,
                held = ?capture.non_suffix,
                "waiting for the post-R3 full desired world; keeping the old applied state \
                 and the non-suffix gate (no partial adoption) (lqos-74t.102)"
            );
            let _ = urgent::submit(
                UrgentSource::System,
                UrgentSeverity::Warning,
                "NON_SUFFIX_R3_WAIT".to_string(),
                format!(
                    "The broker is waiting for the R3/re-adopt full world after a non-suffix \
                     structural remove ({} held circuit(s)). The current applied report does \
                     not match the full desired world; the broker keeps the old applied state \
                     and the R3 gate, and will re-ask.",
                    capture.non_suffix.len()
                ),
                None,
                Some("bridge-broker:non-suffix-wait".to_string()),
            );
            state.report_reread_due = true;
            state.report_reread_at = Some(Instant::now() + REPORT_REREAD_RETRY);
            return false;
        }
        // `lqos-74t.102` full-slot oracle: a matching content key alone is NOT
        // proof that the R3/re-adopt committed a full slot table. Require the
        // raw report order to name EVERY desired Upsert leaf (no absent table
        // and no tail-completed short table) before clearing.
        let tx_names: BTreeSet<&str> = tx
            .leaf_ops
            .iter()
            .filter_map(|op| match op {
                DataplaneLeafOp::Upsert { circuit_id, .. } => Some(circuit_id.as_str()),
                _ => None,
            })
            .collect();
        let full_slot_table = slot_order.as_ref().is_some_and(|order| {
            // §2.4(11): set equality is the oracle. Length+contains allowed a
            // duplicate to stand in for a missing circuit; the BTreeSet form
            // rejects absent, short, and duplicate tables alike.
            let order_names: BTreeSet<&str> = order.iter().map(String::as_str).collect();
            order.len() == tx_names.len() && order_names == tx_names
        });
        if !full_slot_table {
            warn!(
                event = "NON_SUFFIX_R3_WAIT",
                reason = "absent_or_partial_slot_table",
                held = ?capture.non_suffix,
                "the matching report carries no full slot table — the R3/re-adopt \
                 commit is not proven; keeping the gate and the old applied state (lqos-74t.102)"
            );
            let _ = urgent::submit(
                UrgentSource::System,
                UrgentSeverity::Warning,
                "NON_SUFFIX_R3_WAIT".to_string(),
                "The broker is waiting for a full-slot R3/re-adopt report after a non-suffix \
                 structural remove; the current report matched the desired key but carried no \
                 full slot table. The old applied state and the gate are kept."
                    .to_string(),
                None,
                Some("bridge-broker:non-suffix-wait".to_string()),
            );
            state.report_reread_due = true;
            state.report_reread_at = Some(Instant::now() + REPORT_REREAD_RETRY);
            return false;
        }
    }
    let reported = applied::AppliedGeneration {
        generation: tx.generation.clone(),
        content_key,
        desired_world_content_key: Some(desired_world_content_key.clone()),
        slot_order: slot_order.clone(),
        owners: owners.clone(),
    };
    let diverged = if late {
        state.applied.adopt_late(reported, tx);
        false
    } else {
        state.applied.adopt(reported, tx)
    };
    // `lqos-74t.118.2`: publish the workers/ownership snapshot after every
    // adoption (the workers report may change only at boot, but ownership
    // changes with each applied world). Cold path; no per-packet work.
    publish_cpu_workers_snapshot(&state.applied, workers);
    state.slot_order = slot_order;
    // AUDIT GAP 3: the base SETTER marker — ADOPTED from the bridge's report.
    info!(
        event = "BASE_ADOPT",
        cadence,
        generation = %generation,
        inners,
        leaves,
        has_slot_order,
        "Bridge baseline ADOPTED from the applied-state report"
    );
    if pending_non_suffix.is_some() {
        // The R3 full world is now committed/applied and adopted; ONLY NOW may
        // the gate/fence/capture clear. Delivery resumes on this/next tick.
        state.pending_non_suffix = None;
        state.terminal_fence = None;
        state.removal_capture = None;
        state.non_suffix_replay_due = false;
        state.non_suffix_trigger_key = None;
        info!(
            event = "NON_SUFFIX_R3_COMPLETE",
            generation = %generation,
            inners,
            leaves,
            "post-R3 full world adopted — non-suffix gate/fence/capture cleared; delivery resumes"
        );
        let _ = urgent::clear_by_identity("NON_SUFFIX_R3_PENDING", "bridge-broker:non-suffix");
        let _ = urgent::clear_by_identity("NON_SUFFIX_R3_WAIT", "bridge-broker:non-suffix-wait");
    }
    if diverged {
        // A3 criterion 1/3: the report OVERRODE a diverging interim ack-fold.
        // The report is authoritative; the divergence is surfaced.
        warn!(
            event = "BASE_DIVERGENCE",
            "the applied-state report overrode a diverging interim ack-fold"
        );
        urgent::submit(
            UrgentSource::System,
            UrgentSeverity::Warning,
            "BRIDGE_APPLIED_DIVERGENCE".to_string(),
            "The bridge's applied-state report DISAGREED with the broker's interim \
             ack-fold; the report is authoritative and has been adopted (the belief \
             re-syncs on the next diff). Repeats mean the ack-fold is drifting."
                .to_string(),
            None,
            Some("bridge-broker:applied-divergence".to_string()),
        );
    }
    true
}

/// `lqos-74t.101`: before an ACK-silence attempt can be called terminal,
/// re-read the bridge's applied report. If it proves the CURRENT desired world
/// (content key + generation), adopt it as a LATE APPLY; no reject, no terminal
/// fence, and no `BRIDGE_APPLIED_DIVERGENCE` (the interim ack-fold was never the
/// truth). A non-matching/old/partial report returns false and leaves the
/// attempt fail-closed; only a bridge-reported fence may terminal it.
fn adopt_late_from_report(state: &mut BrokerState) -> bool {
    if state.socket_path.is_none() {
        return false;
    }
    state.late_adopt = true;
    let adopted = reconcile_from_report(state, true);
    state.late_adopt = false;
    if adopted {
        info!(
            event = "ACK_SILENCE_LATE_ADOPT",
            generation = ?state.last_desired_generation,
            "bridge applied report proves the in-flight desired world after ACK_SILENCE — \
             adopted as a late apply (no terminal reject, no divergence surface)"
        );
    }
    adopted
}

/// A3 cadence (`docs/broker_applied_single_source_card.md` §2 criterion 2): the
/// applied-state report RE-READ. The report is authoritative, so the broker
/// re-reads it (calls [`reconcile_from_report`]) — it never re-runs
/// `semantic_diff` with `base==None` (that is a permanent hold). Three triggers:
///   * `report_reread_due` — a terminal reject (the MINIMUM cadence);
///   * NO base (the A1 HOLD) — on the SHORTER retry so recovery is AUTOMATIC;
///   * otherwise — the periodic drift reconcile.
fn maybe_reread_report(state: &mut BrokerState) {
    if state.socket_path.is_none() {
        return;
    }
    let now = Instant::now();
    let deadline_due = state.report_reread_at.is_none_or(|deadline| now >= deadline);
    if !(state.report_reread_due || deadline_due) {
        return;
    }
    state.report_reread_due = false;
    let has_base = state.applied.has_base();
    state.report_reread_at = Some(now + if has_base {
        REPORT_REREAD_PERIOD
    } else {
        REPORT_REREAD_RETRY
    });
    reconcile_from_report(state, true);
}

/// `lqos-74t.102`: restore a broker-owned non-suffix gate from the persisted
/// SEAM_STATE across an lqosd restart. Returns the gate capture + the full
/// desired key that the post-R3 report must match.
fn read_restored_non_suffix(
    path: &std::path::Path,
) -> Option<(RemovalCapture, Option<String>)> {
    let raw = std::fs::read_to_string(path).ok()?;
    let snapshot: BridgeConvergenceSnapshot = serde_json::from_str(&raw).ok()?;
    let capture = snapshot.pending_non_suffix?;
    Some((capture, snapshot.desired_world_content_key))
}

/// `lqos-74t.102` restart ordering/replay: restore the persisted gate, then
/// read the CURRENT artifact **before** any report reconcile, and replay the
/// remove-only R3 trigger. A missing artifact/trigger falls back to the engine
/// RemovedNotTail/SlotMismatch path instead of holding silently forever.
fn restore_non_suffix_gate_from(
    snapshot_path: &std::path::Path,
    artifact: &PathBuf,
    state: &mut BrokerState,
) {
    let Some((capture, _persisted_key)) = read_restored_non_suffix(snapshot_path) else {
        return;
    };
    state.pending_non_suffix = Some(capture.clone());
    state.removal_capture = Some(capture.clone());

    let Some(desired) = read_artifact_cached(artifact, &mut state.artifact_cache) else {
        state.pending_non_suffix = None;
        state.removal_capture = None;
        state.non_suffix_replay_due = false;
        state.non_suffix_trigger_key = None;
        warn!(
            event = "NON_SUFFIX_GATE_DROPPED_NO_ARTIFACT",
            "a persisted non-suffix gate had no current artifact to rebuild from — \
             falling back to the engine RemovedNotTail/SlotMismatch recovery"
        );
        return;
    };
    state.last_desired_generation = Some(desired.generation.clone());
    state.last_desired_world_key = Some(translate::content_key(&desired));

    if non_suffix_rebuild_trigger(&desired, &capture).is_none() {
        state.pending_non_suffix = None;
        state.removal_capture = None;
        state.non_suffix_replay_due = false;
        state.non_suffix_trigger_key = None;
        warn!(
            event = "NON_SUFFIX_GATE_DROPPED_NO_TRIGGER",
            "a persisted non-suffix gate had no removable trigger — falling back to the \
             engine RemovedNotTail/SlotMismatch recovery"
        );
        return;
    }
    state.terminal_fence = Some(translate::structural_identity(&desired));
    // §2.4(9): do NOT put the trigger into `state.pending` here. The first
    // step after a restart has no segment yet, and resolving a pending then
    // records SEAM_LOST and destroys the only trigger. Hold the intent and
    // queue it only after the control seam attaches.
    state.non_suffix_replay_due = true;
    state.non_suffix_trigger_key = None;
    state.pending = None;
    warn!(
        event = "NON_SUFFIX_GATE_RESTORED_WAIT",
        "restored the non-suffix R3 gate from {snapshot_path:?}; current artifact \
         loaded; the remove-only trigger will be replayed after a real seam attach"
    );
}

fn step(artifact: &PathBuf, state: &mut BrokerState) {
    // `lqos-8h8` (c): consume any operator override (apply-now / break-glass).
    refresh_maintenance_control(state);
    // 1. Seam liveness/attach (10s cadence while absent).
    if state.segment.is_none() {
        state.attach_polls += 1;
        if state.attach_polls >= RETRY_ATTACH_EVERY {
            state.attach_polls = 0;
            match SeamSegment::attach(SEGMENT_NAME) {
                Ok(segment) => {
                    let boot = segment.boot_id;
                    state.segment = Some(segment);
                    // G2: a different bridge boot invalidated the
                    // baseline — RESET the belief (A3 owner) and re-reconcile.
                    state.applied.reset();
                    clear_cpu_workers_snapshot();
 // AUDIT GAP 3: the base SETTER marker — a
                    // re-attach RESET to None (the third setter; R2b covers the ack
                    // fold, the adopt is marker 2).
                    info!(
                        event = "BASE_RESET_NONE",
                        "broker base reset to None on re-attach (A3 owner)"
                    );
                    state.pending = None;
                    state.terminal_fence = None;
                    // L2-5: a re-attach clears the held set (the bridge re-ran
                    // its site layer or restarted — retry everything).
                    state.held_ops.clear();
                    state.held_structural_key = None;
                    state.held_retry_at = None;
                    state.held_retries = 0;
                    state.bad_layout_reported = false;
                    // U1/W5: a successful attach means the pair now agrees —
                    // clear the protocol-mismatch + bad-layout surfaces.
                    let _ = urgent::clear_by_identity(
                        "BRIDGE_PROTOCOL_MISMATCH",
                        "bridge-broker:protocol-mismatch",
                    );
                    info!(
                        "Bridge broker attached (boot_id {:02x?}); baseline reset — \
                         full desired state will be (re)emitted",
                        boot
                    );
                    // R3 + A3: ADOPT / RECONCILE the bridge's fence-committed
                    // applied state as the baseline (the report is the TRUTH).
                    // The shared cadence re-reads this periodically so a failed
                    // adopt RECOVERS automatically (A1 HOLD + re-ask).
                    reconcile_from_report(state, false);
                }
                Err(AttachError::Absent) => {}
                Err(AttachError::ProtocolMismatch { bridge, expected }) => {
                    if !state.bad_layout_reported {
                        // U1/W5 (the atomic-upgrade guard): the bridge's
                        // PROTOCOL_VERSION differs from this lqosd's. The attach
                        // is REFUSED (no silent ack-ring stride mismatch) and
                        // surfaced LOUDLY + actionably.
                        warn!(
                            "BRIDGE_PROTOCOL_MISMATCH: bridge protocol {bridge} != lqosd \
                             {expected} — attach refused (fail-closed)"
                        );
                        urgent::submit(
                            UrgentSource::System,
                            UrgentSeverity::Error,
                            "BRIDGE_PROTOCOL_MISMATCH".to_string(),
                            format!(
                                "The running bridge speaks control protocol {bridge}, but this \
                                 lqosd speaks {expected} — the control-plane and dataplane \
                                 packages are at DIFFERENT versions. Config changes will NOT \
                                 reach the bridge until the pair is upgraded TOGETHER (install \
                                 libreqos and libreqos-dataplane in the same step). This is \
                                 fail-closed: no silent misread, and no config change is applied."
                            ),
                            None,
                            Some("bridge-broker:protocol-mismatch".to_string()),
                        );
                        state.bad_layout_reported = true;
                    }
                }
                Err(AttachError::BadLayout(why)) => {
                    if !state.bad_layout_reported {
                        // Review fix 5: surface ONCE (warn + urgent),
                        // not every 10s cadence.
                        warn!("Bridge seam attach failed (bad layout): {why}");
                        urgent::submit(
                            UrgentSource::System,
                            UrgentSeverity::Warning,
                            "BRIDGE_SEAM_BAD_LAYOUT".to_string(),
                            format!(
                                "The bridge control seam attached with an unexpected \
                                 layout ({why}) — config changes are NOT reaching the \
                                 bridge. Check bridge/lqosd version agreement; this \
                                 warning will not repeat until a successful attach."
                            ),
                            None,
                            Some("bridge-broker:bad-layout".to_string()),
                        );
                        state.bad_layout_reported = true;
                    }
                }
            }
        }
    } else if state.pending.is_none() {
        // 1b. Idle-path liveness (ghost-segment card §3, fix shape (i)):
        // the attach poll above runs only while absent and the G4 arm only
        // with a pending, so a bridge-only restart in steady state is
        // otherwise invisible indefinitely. On the same ~10s cadence, poll
        // the seam's inode identity; a replaced (or absent) segment drops
        // to None so the existing attach/G2/re-emit machinery recovers.
        state.liveness_polls += 1;
        if state.liveness_polls >= RETRY_ATTACH_EVERY {
            state.liveness_polls = 0;
            if let Some(segment) = state.segment.as_ref()
                && segment.is_replaced(SEGMENT_NAME)
            {
                warn!("Bridge seam replaced while idle (bridge restart) — re-attaching");
                urgent::submit(
                    UrgentSource::System,
                    UrgentSeverity::Warning,
                    "BRIDGE_SEAM_REPLACED".to_string(),
                    "The bridge control seam was replaced while idle (bridge restart); \
                     the control plane was dead until this detection. Re-attaching and \
                     re-emitting the full desired state."
                        .to_string(),
                    None,
                    Some("bridge-broker:seam-replaced".to_string()),
                );
                state.segment = None;
                if state.pending_non_suffix.is_some() {
                    // §2.4(13): the idle replacement invalidated the seam while
                    // the gate is armed; 2c must queue the replay after the
                    // next attach instead of leaving the gate waiting forever.
                    state.non_suffix_replay_due = true;
                }
            }
        }
    }

    // 2a. `lqos-74t.102` §2.4(10): refresh the CURRENT artifact identity on
    // EVERY step, before the report reconcile. It is also before resolving an
    // in-flight publish so §2.4(14) can compare the trigger key with the c5
    // desired key on the same step the ack resolves.
    let desired_now = read_artifact_cached(artifact, &mut state.artifact_cache);
    if let Some(desired) = desired_now.as_ref() {
        state.last_desired_generation = Some(desired.generation.clone());
        state.last_desired_world_key = Some(translate::content_key(desired));
    }

    // 2. Resolve any in-flight publish first (G3: newest-wins only
    //    after the current ack resolves).
    if let Some(pending) = state.pending.take() {
        match resolve_pending(state, pending) {
            Resolve::Retained(pending) => state.pending = Some(*pending),
            Resolve::Cleared => {}
        }
    }

    // 2b. A3 cadence (`docs/broker_applied_single_source_card.md` §2 criterion
    //     2): re-read the applied-state report (the TRUTH) while idle, so a
    //     failed adopt recovers AUTOMATICALLY and the interim ack-fold cannot
    //     drift within a session. A pending sequence owns the base — do not
    //     re-base mid-flight. This runs AFTER 2a so the c5 key is live.
    if state.pending.is_none() {
        maybe_reread_report(state);
    }

    // 2c. `lqos-74t.102` §2.4(9): a restored/lost non-suffix trigger is held
    // as INTENT, never as a pending before a segment exists. Queue it only
    // after a real attach; if no seam is present, keep the gate and return
    // without consuming the trigger as SEAM_LOST.
    if state.pending_non_suffix.is_some() {
        // §2.4(14): the ack path arms replay_due when the in-flight trigger
        // carried an older desired key than the current artifact. A c4 trigger
        // resolving after the desired became c5 must not leave the gate
        // waiting on c4.
        if state.non_suffix_replay_due && state.pending.is_none() {
            if state.segment.is_none() {
                publish_convergence_snapshot(state);
                return;
            }
            if let Some(desired) = desired_now.as_ref() {
                let _ = queue_non_suffix_replay(state, desired);
            }
        }
        if state.pending_non_suffix.is_some() {
            publish_convergence_snapshot(state);
            return;
        }
    }

    // 3. Start a new publish when idle. C3(b): a consecutive-reject backoff
    // throttles the re-diff instead of a per-tick reject storm.
    let backoff_active = state
        .backoff_until
        .is_some_and(|deadline| std::time::Instant::now() < deadline);
    if state.pending.is_none()
        && !backoff_active
        && let Some(desired) = desired_now.as_ref()
    {
        // R3 rider (F8): the steady key must be FLOOR-CONSISTENT —
        // semantic_diff floors desired at entry, and post-adoption the
        // baseline holds PUBLISHED (already-floored) content. Keying the
        // RAW desired made any suspended (0/0) circuit in config force
        // an empty re-diff on every 2s poll, forever.
        let desired_structural = translate::structural_identity(desired);
        // Duplicate mapping identities (card `docs/duplicate_mapping_alerts_card.md`):
        // an identity bound to >1 circuit is silently misclassified by the LPM.
        // Warn once per identity; CLEAR when it resolves (the issue TTL is 24h, so
        // clear-on-resolve is load-bearing). Cold path; NO hold, NO reject, NO hot
        // path. Ignored subnets are filtered upstream (U4) ⇒ no 192.168.x false hit.
        let duplicates = translate::duplicate_mapping_identities(desired);
        let current: BTreeSet<String> = duplicates
            .iter()
            .map(|(identity, _)| identity.clone())
            .collect();
        for (identity, circuits) in &duplicates {
            urgent::submit(
                UrgentSource::System,
                UrgentSeverity::Warning,
                "DUPLICATE_MAPPING_IDENTITY".to_string(),
                format!(
                    "{identity} is bound to {} circuits ({}); the LPM can route only one",
                    circuits.len(),
                    circuits.join(", ")
                ),
                None,
                Some(identity.clone()),
            );
        }
        for identity in state.duplicate_mapping_alerts.difference(&current) {
            let _ = urgent::clear_by_identity("DUPLICATE_MAPPING_IDENTITY", identity);
        }
        state.duplicate_mapping_alerts = current;
        // L2-5: when the desired STRUCTURE changes, drop the held set so the
        // quarantined op is re-attempted (it may now be appliable). A re-attach
        // clears it too (above).
        if state
            .held_structural_key
            .as_deref()
            .is_some_and(|held| held != desired_structural.as_str())
        {
            state.held_ops.clear();
            state.held_structural_key = None;
            state.held_retry_at = None;
            state.held_retries = 0;
        }
        // CARD §2 + URGENT (jackrabbit stall): bounded AUTO-RETRY — past the retry
        // cadence, drop the hold for ONE attempt so a TRANSIENT cause self-clears
        // without a re-attach. CAPPED at `HELD_RETRY_MAX`: a GENUINELY-unappliable
        // op (a span-changing generated move, a retire whose applied subtree is
        // non-empty) never clears by re-emitting, so after the cap the broker
        // STOPS retrying (keeps the hold; the fence is a bug-catcher, not a
        // reject loop) — it clears only on a structural change or re-attach.
        if !state.held_ops.is_empty()
            && state
                .held_retry_at
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            state.held_retry_at = None;
            if state.held_retries < HELD_RETRY_MAX {
                state.held_retries += 1;
                state.held_ops.clear();
                state.held_structural_key = None;
            }
        }
        // LKG-as-cache (`docs/lkg_as_cache_card.md` §5): the empty-seed
        // CONTINGENCY is DELETED — under the cache model the bridge always
        // REBUILDS from the desired + reports, so the no-base HOLD resolves by
        // ADOPT; an empty-seed is a second-truth fallback (retired).
        // L2-2/L2-3: publish the APPLIABLE PROJECTION — desired minus the held
        // op(s) and their dependents — so rates and unaffected structural work
        // keep applying (Linux parity: never freeze the qdisc).
        // A1 (`docs/broker_applied_single_source_card.md`): a MISSING applied base
        // is SURFACED (and the producer holds + RE-ASKS) — never fabricate a
        // full-world. The A3 cadence (step 2b) re-reads the report so recovery is
        // automatic and the surface clears when a base returns.
        if !state.applied.has_base() {
            let _ = urgent::submit(
                UrgentSource::System,
                UrgentSeverity::Warning,
                "BRIDGE_APPLIED_UNAVAILABLE".to_string(),
                "The broker has NO bridge applied-state base (the report is unavailable) — it is \
                 HOLDING and re-asking (no full-world fabrication). The bridge keeps its own \
                 world; config changes apply automatically once the applied-state report returns."
                    .to_string(),
                None,
                Some("bridge-broker:applied-unavailable".to_string()),
            );
        } else {
            let _ = urgent::clear_by_identity(
                "BRIDGE_APPLIED_UNAVAILABLE",
                "bridge-broker:applied-unavailable",
            );
        }
        // P7/B3 retired (R3 card §2, DRIVER (b)): the projection is always
        // `desired − held` (the REJECT-driven hold) — never a full-desired bypass.
        let projected =
            translate::project_desired(desired, state.applied.tx(), &state.held_ops);
        let desired_key = translate::content_key(&translate::apply_suspend_floor(&projected));
        let already_applied = state.applied.key() == Some(desired_key.as_str());
        // WP2 (`docs/wp2_wp3_fence_convergence_and_adopt_card.md`): the TERMINAL
        // fence is keyed on the STRUCTURAL identity (never the shaping
        // generation, never the rate-inclusive content key). The M-A Baseline
        // fence-open hunk is DROPPED here — C-A deleted that arm.
        let failed_this_generation =
            terminal_fenced(state.terminal_fence.as_deref(), &desired_structural);
        // AVAIL 0(c) MINT seam: the identity is the artifact content key vs
        // the last-applied key; matching identities CONVERGE (reset).
        // L2: NO world-freeze — the projection is always attempted; a held op
        // simply does not appear in it.
        if !already_applied && !failed_this_generation {
            let outcome = translate::semantic_diff(
                &projected,
                state.applied.tx(),
                state.slot_order.as_deref(),
            );
            // lqos-74t.54: ALWAYS-ON structural-remove capture. The bridge's
            // dense table is LIFO (a remove is acceptable only at the tail), so
            // a non-suffix removed set is HELD + FENCED (never a doomed remove,
            // never a spin); the full set + the dense-table size + the verdict
            // are persisted in SEAM_STATE so the next occurrence answers
            // suffix-ness without re-instrumenting.
            if outcome.removal_capture.removed.is_empty() {
                state.removal_capture = None;
            } else {
                let capture = outcome.removal_capture.clone();
                if capture.non_suffix.is_empty() {
                    info!(
                        event = "STRUCTURAL_REMOVE_CAPTURE",
                        removed = ?capture.removed,
                        slot_table_len = capture.slot_table_len,
                        slot_order_available = capture.slot_order_available,
                        suffix = capture.suffix,
                        "structural remove set captured (lqos-74t.54)"
                    );
                } else {
                    warn!(
                        event = "NON_SUFFIX_STRUCTURAL_REMOVE",
                        removed = ?capture.removed,
                        slot_table_len = capture.slot_table_len,
                        suffix = capture.suffix,
                        held = ?capture.non_suffix,
                        "structural remove set is not a contiguous suffix of the bridge \
                         slot table — emitted only the removable tail run and FENCED the \
                         remainder (held, not spinning; clears on a structural change) \
                         (lqos-74t.54)"
                    );
                    // FENCE: never re-emit a remove the engine rejects. Keyed on
                    // the desired structural identity, so it clears when the
                    // structure changes (or on re-attach / the cold rebuild).
                    // NOTE: the shared `terminal_fence` FREEZES ALL work for this
                    // structural identity (not only the held remove) until the
                    // identity changes — a deliberate settled state, not a spin.
                    state.terminal_fence = Some(desired_structural.clone());
                    // `lqos-74t.102`: a NON-SUFFIX capture arms a broker-owned
                    // gate that is NOT keyed to the structural identity. The
                    // trigger below carries only the non-suffix Removes; the
                    // engine's typed SlotMismatch detector refuses them before
                    // any mutation and forces R3. No ADD is ever emitted while
                    // this gate is set. It clears only when the post-R3 report
                    // matches the full desired world.
                    state.pending_non_suffix = Some(capture.clone());
                    // The normal capture tick emits the remove-only trigger
                    // below; this is not a replay intent.
                    state.non_suffix_replay_due = false;
                    state.non_suffix_trigger_key = None;
                    warn!(
                        event = "NON_SUFFIX_R3_PENDING",
                        held = ?capture.non_suffix,
                        slot_table_len = capture.slot_table_len,
                        "non-suffix remove captured — holding ALL new incremental emission; \
                         routing the whole world to R3/re-adopt (no ADD against the partial \
                         slot table) (lqos-74t.102)"
                    );
                    let _ = urgent::submit(
                        UrgentSource::System,
                        UrgentSeverity::Warning,
                        "NON_SUFFIX_R3_PENDING".to_string(),
                        format!(
                            "A non-suffix structural remove was captured ({} circuit(s), slot \
                             table {}); the broker is holding new incremental changes and \
                             routing the full desired world through R3/re-adopt. No ADD is \
                             emitted against the partial slot table.",
                            capture.non_suffix.len(),
                            capture.slot_table_len
                        ),
                        None,
                        Some("bridge-broker:non-suffix".to_string()),
                    );
                }
                state.removal_capture = Some(capture);
            }
            // §`lqos-74t.12`: refresh the aged pure-site deferral set — stamp a
            // site's first-seen time, clear it once a circuit rides it, and emit
            // ONE loud event per newly-deferred site (never silent).
            let now = now_unix();
            let deferred_now: BTreeSet<&str> =
                outcome.deferred_sites.iter().map(String::as_str).collect();
            state
                .deferred_sites
                .retain(|site, _| deferred_now.contains(site.as_str()));
            for site in &outcome.deferred_sites {
                if !state.deferred_sites.contains_key(site) {
                    state.deferred_sites.insert(site.clone(), now);
                    warn!(
                        event = "DEFERRED_SITE_CREATE",
                        site = %site,
                        "new site has no circuit yet — deferring its create (zero dataplane \
                         effect); it emits with its first circuit add (lqos-74t.12)"
                    );
                }
            }
            // §`lqos-74t.13`/`.15`(d): track the RETIRE deferrals the same way —
            // a site whose applied subtree still bears circuits is deferred one
            // generation (surfaced, never silent).
            let deferred_retires_now: BTreeSet<&str> =
                outcome.deferred_retires.iter().map(String::as_str).collect();
            state
                .deferred_retires
                .retain(|site, _| deferred_retires_now.contains(site.as_str()));
            for site in &outcome.deferred_retires {
                if !state.deferred_retires.contains_key(site) {
                    state.deferred_retires.insert(site.clone(), now);
                    warn!(
                        event = "DEFERRED_SITE_RETIRE",
                        site = %site,
                        "retired site still bears applied circuits — deferring the site Remove \
                         one generation (lqos-74t.13/.15)"
                    );
                }
            }
            // F3-P: ONE coherent delta (or `None` = hold/steady). The broker no
            // longer drains a sequence; the bridge reconciles the change-set
            // against its own applied base.
            //
            // `lqos-74t.102`: in the capture tick the normal delta is REPLACED
            // by a remove-only R3 trigger. It carries the non-suffix circuits
            // (which the engine refuses before mutating) and NO structural ADD,
            // so the next ADD never reaches the partial slot table.
            let delta = match state.pending_non_suffix.as_ref() {
                Some(capture) => non_suffix_rebuild_trigger(&desired, capture),
                None => outcome.delta,
            };
            if let Some(delta) = delta {
                let baseline = outcome.baseline.unwrap_or_else(|| {
                    // A non-suffix-only capture may have no normal delta at all;
                    // the trigger is still emitted. The fold baseline is unused
                    // on a REBUILD_REQUIRED ack, but keep the old applied world
                    // as the honest interim.
                    state
                        .applied
                        .tx()
                        .cloned()
                        .unwrap_or_else(|| desired.clone())
                });
                match seam::encode_transaction(&delta, state.last_desired_world_key.as_deref()) {
                    Ok(frames) => {
                        // `lqos-8h8`: the maintenance-window GATE (§14). A
                        // PAUSE-class delta while the window is CLOSED (and not
                        // forced) is HELD — the desired stays UNAPPLIED and is
                        // re-evaluated each tick; `semantic_diff` coalesces the
                        // net change at window-open. A PAUSE-FREE delta (the
                        // daily churn, `lpm_delta`, same-owner reparent) is
                        // NEVER held (pin 1).
                        if maintenance_should_hold(
                            state,
                            &delta,
                            projected.leaf_ops.len(),
                            &desired_key,
                        ) {
                            let _ = frames;
                        } else {
                            info!(
                                "Bridge broker emitting {} op(s) for generation {} ({} frame(s))",
                                delta.inner_ops.len()
                                    + delta.leaf_ops.len()
                                    + delta.mapping_ops.len(),
                                delta.generation,
                                frames.len()
                            );
                            outcomes::record_emit(&delta.generation, 1, 1);
                            state.pending = Some(Pending {
                                frames,
                                pushed: 0,
                                awaiting_ack: false,
                                generation: delta.generation.clone(),
                                structural_key: desired_structural.clone(),
                                current_tx: delta,
                                seq_pos: 1,
                                seq_len: 1,
                                applied_after_current: baseline,
                                desired_key: state.last_desired_world_key.clone(),
                                attempt_retries: 0,
                                ack_timeouts: 0,
                            });
                            if state.pending_non_suffix.is_some() {
                                state.non_suffix_trigger_key =
                                    state.last_desired_world_key.clone();
                            }
                        }
                    }
                    Err(_) => {
                        warn!(
                            "Bridge broker: transaction too large for the seam (generation {}); \
                             skipped until config changes",
                            delta.generation
                        );
                        outcomes::record_rejected(&delta.generation, "TOO_LARGE_FOR_SEAM");
                        // TERMINAL fence: keyed on the STRUCTURAL identity (WP2 /
                        // review) so a RATE-ONLY churn does NOT clear it.
                        state.terminal_fence = Some(desired_structural.clone());
                    }
                }
            }
        }
    }

    // L4 §5a + WP5: publish the operator SEAM_STATE snapshot the Bridge page reads.
    publish_convergence_snapshot(state);
}

enum Resolve {
    Retained(Box<Pending>),
    Cleared,
}

/// E3b: the operator-facing reason string for a terminal non-applied
/// outcome (surfaced verbatim by the UI, never retried).
/// `lqos-zxc.6`: the ack's declared drop source/timing + count, when present
/// (`AckDropsKind::None`/absent ⇒ `None` — nothing to render).
fn ack_drops(
    ack: &lqos_dataplane_types::wire::AckFrame,
) -> Option<(lqos_dataplane_types::wire::AckDropsKind, u32)> {
    ack.drops_kind()
        .filter(|kind| !matches!(kind, lqos_dataplane_types::wire::AckDropsKind::None))
        .map(|kind| (kind, ack.dropped_packets()))
}

fn reject_reason(outcome: &ack_map::PublishOutcome) -> &'static str {
    match outcome {
        ack_map::PublishOutcome::Applied => "APPLIED",
        ack_map::PublishOutcome::AttemptExhausted => "ATTEMPT_EXHAUSTED",
        ack_map::PublishOutcome::ObserveMode => "REJECTED_OBSERVE_MODE",
        ack_map::PublishOutcome::RebuildRequired => "REBUILD_REQUIRED",
        ack_map::PublishOutcome::Rejected(name) => name,
    }
}

/// F3-B §3b (`docs/f3b_r3_runtime_rebuild_and_s3b_work_order.md` required 4): the
/// PURE routing disposition `resolve_pending` applies to a typed ack outcome.
/// A mechanics `REJECTED_STRUCTURAL_OP` quarantines + surfaces + retries;
/// `REBUILD_REQUIRED` does NEITHER (the bridge cold-rebuilds; the broker waits
/// for the post-rebuild report).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OutcomeDisposition {
    /// Quarantine the rejected ops (`held_ops_from_tx` + `HELD_RETRY`).
    quarantine: bool,
    /// Set the generation `terminal_fence`.
    fence: bool,
    /// Record a terminal reject / surface it as one.
    reject: bool,
}

fn outcome_disposition(outcome: ack_map::PublishOutcome) -> OutcomeDisposition {
    match outcome {
        ack_map::PublishOutcome::Applied
        | ack_map::PublishOutcome::AttemptExhausted
        | ack_map::PublishOutcome::ObserveMode
        | ack_map::PublishOutcome::RebuildRequired => OutcomeDisposition {
            quarantine: false,
            fence: false,
            reject: false,
        },
        ack_map::PublishOutcome::Rejected("REJECTED_STRUCTURAL_OP") => OutcomeDisposition {
            quarantine: true,
            fence: false,
            reject: true,
        },
        ack_map::PublishOutcome::Rejected(_) => OutcomeDisposition {
            quarantine: false,
            fence: true,
            reject: true,
        },
    }
}


fn resolve_pending(state: &mut BrokerState, mut pending: Pending) -> Resolve {
    let Some(segment) = state.segment.as_mut() else {
        // No segment: drop the in-flight frames; the artifact diff
        // regenerates them after the next successful attach. The
        // operator surface settles this attempt honestly — the
        // generation re-emits (and re-pends) after re-attach.
        outcomes::record_rejected(&pending.generation, "SEAM_LOST");
        // §2.4(9): losing the push before a real attach is not permission
        // to lose the trigger. Keep the gate and replay after attach.
        rearm_non_suffix_after_lost_pending(state);
        return Resolve::Cleared;
    };
    // Push-once (review fix 1): write only unwritten frames; once the
    // transaction is fully in the ring, WAIT ONLY — re-pushing on ack
    // silence would queue a duplicate publish and misattribute its
    // stale ack to the next transaction.
    while pending.pushed < pending.frames.len() {
        match segment.push_frame(&pending.frames[pending.pushed]) {
            Ok(()) => pending.pushed += 1,
            Err(_) => {
                warn!(
                    "Bridge seam ring full pushing generation {} (frame {}/{}); \
                     will resume where it left off",
                    pending.generation,
                    pending.pushed,
                    pending.frames.len()
                );
                return Resolve::Retained(Box::new(pending));
            }
        }
    }
    pending.awaiting_ack = true;
    let Some(ack) = segment.wait_ack(ack_timeout()) else {
        pending.ack_timeouts += 1;
        // G4: liveness — a replaced segment (bridge restart) means
        // these frames went nowhere; re-attach and re-baseline.
        if segment.is_replaced(SEGMENT_NAME) {
            info!("Bridge seam replaced mid-publish (ack silence) — re-attaching");
            outcomes::record_rejected(&pending.generation, "SEAM_LOST");
            rearm_non_suffix_after_lost_pending(state);
            state.segment = None;
            return Resolve::Cleared;
        }
        if pending.ack_timeouts >= ACK_SILENCE_MAX_TIMEOUTS {
            // `lqos-74t.101`: ACK silence is NOT a bridge-reported fence. The
            // bridge may have applied late (the 90 s convergence bound is
            // inside this 120 s ACK window), so re-read the applied report
            // BEFORE any terminal decision. A matching late report is adopted
            // as the truth; no reject/fence/divergence is raised.
            if adopt_late_from_report(state) {
                state.record_applied_reset();
                outcomes::record_applied(&pending.generation);
                state.report_reread_due = false;
                return Resolve::Cleared;
            }
            // No matching late apply: stay fail-closed. Keep the attempt
            // open (no new publish, no terminal fence, no BRIDGE_PUBLISH_
            // REJECTED) and keep re-reading. Only a bridge-reported fence may
            // terminal an ACK-silence attempt.
            state.report_reread_due = true;
            state.report_reread_at = Some(Instant::now() + REPORT_REREAD_RETRY);
            rearm_non_suffix_after_lost_pending(state);
            warn!(
                event = "ACK_SILENCE_LATE_WAIT",
                generation = %pending.generation,
                timeouts = pending.ack_timeouts,
                "no matching late-applied report in the broker ACK window; keeping the \
                 attempt fail-closed (no terminal reject / no fence) and re-reading"
            );
            pending.ack_timeouts = 0;
            return Resolve::Retained(Box::new(pending));
        }
        // Still waiting: NO re-push — the bridge may still be
        // processing the attempt in flight.
        return Resolve::Retained(Box::new(pending));
    };
    let outcome = ack_map::classify(ack.outcome_code());
    match outcome {
        ack_map::PublishOutcome::Applied => {
            if non_suffix_applied_is_suspect(state.pending_non_suffix.as_ref()) {
                // Defensive fallback (engine detector should have forced R3):
                // an APPLIED ack for the remove-only trigger means the bridge
                // accepted a partial world. NEVER advance the applied belief or
                // clear the gate here; keep the old applied state and wait for
                // the matching full R3 report. One surfaced item (bounded).
                warn!(
                    event = "NON_SUFFIX_UNEXPECTED_APPLIED",
                    generation = %pending.generation,
                    "the non-suffix trigger was APPLIED (the engine SlotMismatch fallback did \
                     not fire) — keeping the old applied state and the R3 gate; no partial \
                     applied belief (lqos-74t.102)"
                );
                let _ = urgent::submit(
                    UrgentSource::System,
                    UrgentSeverity::Warning,
                    "NON_SUFFIX_R3_WAIT".to_string(),
                    "A non-suffix trigger was unexpectedly applied by the bridge; the broker \
                     is keeping the previous applied state and the R3 gate until the full \
                     desired world is adopted."
                        .to_string(),
                    None,
                    Some("bridge-broker:non-suffix-wait".to_string()),
                );
                state.report_reread_due = true;
                // §2.4(17): recovery from the wait hazard — re-read the report
                // AND re-arm the trigger for the current desired key; state
                // retention alone would leave the gate with no next attempt.
                state.non_suffix_replay_due = true;
                return Resolve::Cleared;
            }
            ack_map::record_applied();
            state.record_applied_reset();
            // AVAIL 0(c): a successful apply is CONVERGENCE on the publish +
            // ack legs (the diff closes -> reset).
            info!(
                "Bridge applied generation {} (fence {})",
                ack.generation_str(),
                ack.fence_epoch()
            );
            // the sidebar UI-hygiene card: the seam-replaced red lives only while actually
            // degraded — the first generation applied after re-attach + full
            // re-emission clears it (the 24h TTL still retires genuinely
            // unresolved classes). Idempotent when nothing was raised.
            let _ = urgent::clear_by_identity("BRIDGE_SEAM_REPLACED", "bridge-broker:seam-replaced");
            // ROOT-2 heal: a structural reject is cleared once a world APPLIES
            // (the bridge re-ran the site layer, or an operator reloaded it).
            // L2: a projection apply does NOT clear a held op (the op is still
            // quarantined) — clear the structural urgent only once the hold is
            // gone (real convergence). A re-attach also clears the hold.
            if state.held_ops.is_empty() {
                let _ = urgent::clear_by_identity(
                    "BRIDGE_PUBLISH_REJECTED",
                    "bridge-broker:structural",
                );
                // The SAME real-convergence gate heals the rebuild alert: a world
                // APPLIED (the cold rebuild ran and the post-rebuild report landed)
                // ⇒ the request is served. Its stable identity means ONE item,
                // cleared here — it never accumulates per generation.
                let _ = urgent::clear_by_identity(
                    "BRIDGE_REBUILD_REQUIRED",
                    "bridge-broker:rebuild",
                );
            }
            // Review fix 2: fold THIS transaction's baseline in
            // immediately — the baseline tracks the applied prefix, so
            // a later failure re-diffs to only the remaining work.
            // A3: the fold is the INTERIM projection (the report reconciles it
            // on the next cadence; a divergence surfaces and the report wins).
            // `lqos-101`: use the DESIRED world key STORED ON THE PENDING at
            // encode time — UNCONDITIONALLY. A mid-flight artifact advance can
            // change `last_desired_world_key`; the ack must label the world it
            // actually applied (the frame key), not the newer desired world.
            // `None` (sequence/replay/pre-fix) folds to the applied key.
            state.applied.fold_ack_with_desired(
                pending.applied_after_current.clone(),
                pending.desired_key.clone(),
            );
            // AUDIT GAP 3: the base SETTER marker — folded from an APPLIED ack.
            info!(
                event = "BASE_ACK_FOLD",
                generation = %pending.generation,
                inners = pending.applied_after_current.inner_ops.len(),
                "broker base folded from an applied ack"
            );
            // F3-P: ONE coherent delta ⇒ the pending publish is COMPLETE on its
            // ack — there is no per-step sequence to drain. The final fold above
            // is the projection.
            outcomes::record_applied(&pending.generation);
            Resolve::Cleared
        }
        ack_map::PublishOutcome::AttemptExhausted => {
            ack_map::surface(
                outcome,
                &pending.generation,
                ack.fence_epoch(),
                ack.reason(),
                &ack.identity_str(),
                ack_drops(&ack),
            );
            // AVAIL 0(c) VALIDATE seam: the bridge's bounded retries are
            // exhausted (the transaction could not be applied).
            pending.attempt_retries += 1;
            if pending.attempt_retries >= ATTEMPT_EXHAUSTED_MAX_RETRIES {
                outcomes::record_rejected(&pending.generation, "ATTEMPT_EXHAUSTED");
                state.terminal_fence = Some(pending.structural_key.clone());
                state.record_reject();
                // A3 cadence: a terminal reject marks a report re-read DUE.
                state.report_reread_due = true;
                // §2.4(19): AttemptExhausted is the same wait hazard as
                // ACK_SILENCE while the non-suffix gate is armed.
                rearm_non_suffix_after_lost_pending(state);
                return Resolve::Cleared;
            }
            outcomes::record_retry(
                &pending.generation,
                pending.attempt_retries,
                ATTEMPT_EXHAUSTED_MAX_RETRIES,
            );
            std::thread::sleep(ATTEMPT_BACKOFF);
            // The ack proves the bridge consumed the attempt ("not
            // applied; retry") — a retry is a NEW delivery: reset the
            // push-once cursor and write again.
            pending.pushed = 0;
            pending.awaiting_ack = false;
            Resolve::Retained(Box::new(pending))
        }
        ack_map::PublishOutcome::RebuildRequired => {
            // F3-B §3b: the delta is not delta-expressible (a StructurallyInvalid
            // class) OR an owner-set/span mechanics violation — the bridge takes
            // its COLD, content-keyed REBUILD. EXPLICITLY do NOT quarantine
            // (`held_ops_from_tx`), arm `HELD_RETRY`, set `terminal_fence`, or
            // record a reject; surface the rebuild and wait for the post-rebuild
            // report (re-diff on the next tick).
            ack_map::surface(
                outcome,
                &pending.generation,
                ack.fence_epoch(),
                ack.reason(),
                &ack.identity_str(),
                ack_drops(&ack),
            );
            info!(
                event = "REBUILD_REQUIRED",
                generation = %pending.generation,
                "bridge reports REBUILD_REQUIRED — routing to the cold rebuild arm \
                 (no quarantine / no fence); awaiting the post-rebuild report"
            );
            // A3 cadence: re-read the report so the rebuilt key is observed.
            state.report_reread_due = true;
            // §2.4(14): the in-flight trigger carried the c4 key while the
            // current artifact is already c5. Re-arm the remove-only trigger
            // for c5 after this ack instead of waiting on the stale key.
            if state.pending_non_suffix.is_some()
                && state.non_suffix_trigger_key.as_deref()
                    != state.last_desired_world_key.as_deref()
            {
                state.non_suffix_replay_due = true;
            }
            Resolve::Cleared
        }
        other => {
            ack_map::surface(
                other,
                &pending.generation,
                ack.fence_epoch(),
                ack.reason(),
                &ack.identity_str(),
                ack_drops(&ack),
            );
            // AVAIL 0(c) VALIDATE seam: a typed bridge reject.
            outcomes::record_rejected(&pending.generation, reject_reason(&other));
            let disposition = outcome_disposition(other);
            if disposition.quarantine {
                // L2-1 (segment bookkeeping): QUARANTINE the rejected segment's
                // op(s) — do NOT freeze the world, and do NOT generation-fence
                // (the projection must publish on the NEXT tick for the SAME
                // generation). The held set clears when the desired structure
                // changes (L2-5) or on re-attach.
                state.held_ops.extend(translate::held_ops_from_tx(
                    &pending.current_tx,
                    state.applied.tx(),
                ));
                state.held_structural_key = Some(pending.structural_key.clone());
                // CARD §2: arm the bounded auto-retry so a transient cause clears
                // without an operator re-attach.
                state.held_retry_at = Some(Instant::now() + HELD_RETRY_BACKOFF);
            } else if disposition.fence {
                // A non-structural typed reject (bad frame/JSON, LPM prepare,
                // version mismatch, envelope) is not projection-fixable —
                // keep the generation fence.
                state.terminal_fence = Some(pending.structural_key.clone());
            }
            state.record_reject();
            // A3 cadence: a terminal reject marks a report re-read DUE.
            state.report_reread_due = true;
            Resolve::Cleared
        }
    }
}

/// Reads + parses the artifact unless its stat identity (mtime, size)
/// matches the last successful parse (review fix 3) — the poll is 2s
/// and a re-read/re-parse of an unchanged file every tick is waste.
fn read_artifact_cached(
    path: &PathBuf,
    cache: &mut Option<(std::time::SystemTime, u64, DataplaneTransaction)>,
) -> Option<DataplaneTransaction> {
    if let Ok(meta) = std::fs::metadata(path)
        && let Ok(mtime) = meta.modified()
        && let Some((cached_mtime, cached_size, cached_tx)) = cache.as_ref()
        && mtime == *cached_mtime
        && meta.len() == *cached_size
    {
        return Some(cached_tx.clone());
    }
    match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<DataplaneTransaction>(&raw) {
            Ok(tx) => {
                if let Ok(meta) = std::fs::metadata(path)
                    && let Ok(mtime) = meta.modified()
                {
                    *cache = Some((mtime, meta.len(), tx.clone()));
                }
                Some(tx)
            }
            Err(err) => {
                warn!(
                    "Bridge broker: artifact {} is unparseable ({err:?}); waiting \
                     for the next publish",
                    path.display()
                );
                None
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            warn!("Bridge broker: cannot read {}: {err:?}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod r3_tests {
    use super::*;
    use lqos_dataplane_types::{DataplaneLeafOp, DataplaneMappingOp};

    /// Serves a fixed StateReply (once per request, for up to two
    /// requests) over a real analytics socket, for the adoption tests.
    pub(super) fn state_socket(
        dir: &std::path::Path,
        reply: analytics_ipc::StateReply,
    ) -> (std::path::PathBuf, std::thread::JoinHandle<()>) {
        let socket = dir.join("state.sock");
        let (snapshot_requester, _snapshot_rx) =
            analytics_ipc::snapshot_channel(std::time::Duration::from_secs(1));
        let (health_requester, _health_rx) =
            analytics_ipc::health_channel(std::time::Duration::from_secs(1));
        let (state_requester, state_rx) =
            analytics_ipc::state_channel(std::time::Duration::from_secs(1));
        let service = analytics_ipc::SnapshotService::bind(
            &socket,
            snapshot_requester,
            health_requester,
            state_requester,
        )
        .expect("bind");
        std::fs::set_permissions(&socket, std::os::unix::fs::PermissionsExt::from_mode(0o666))
            .ok();
        // serve() loops forever; the test server thread is detached (the
        // process exits after the test).
        std::thread::spawn(move || {
            let _ = service.serve();
        });
        // Feed the collector side: one reply per request, twice.
        let feeder = std::thread::spawn(move || {
            if let Ok(request) = state_rx.recv() {
                let _ = request.reply.try_send(reply);
            }
        });
        (socket, feeder)
    }

    pub(super) fn report_tx_bytes(tx: &DataplaneTransaction) -> Vec<u8> {
        let mut bytes = Vec::new();
        ciborium::into_writer(tx, &mut bytes).unwrap();
        bytes
    }

    pub(super) fn slot_order_bytes(circuits: &[&str]) -> Vec<u8> {
        let order: Vec<String> = circuits.iter().map(|c| (*c).to_string()).collect();
        let mut bytes = Vec::new();
        ciborium::into_writer(&order, &mut bytes).unwrap();
        bytes
    }

    /// Serves a SEQUENCE of StateReplies — one per request — over a real
    /// analytics socket (the A3 cadence needs more than one read).
    fn state_socket_seq(
        dir: &std::path::Path,
        replies: Vec<analytics_ipc::StateReply>,
    ) -> (std::path::PathBuf, std::thread::JoinHandle<()>) {
        let socket = dir.join("state.sock");
        let (snapshot_requester, _snapshot_rx) =
            analytics_ipc::snapshot_channel(std::time::Duration::from_secs(1));
        let (health_requester, _health_rx) =
            analytics_ipc::health_channel(std::time::Duration::from_secs(1));
        let (state_requester, state_rx) =
            analytics_ipc::state_channel(std::time::Duration::from_secs(1));
        let service = analytics_ipc::SnapshotService::bind(
            &socket,
            snapshot_requester,
            health_requester,
            state_requester,
        )
        .expect("bind");
        std::fs::set_permissions(&socket, std::os::unix::fs::PermissionsExt::from_mode(0o666))
            .ok();
        std::thread::spawn(move || {
            let _ = service.serve();
        });
        let feeder = std::thread::spawn(move || {
            for reply in replies {
                match state_rx.recv() {
                    Ok(request) => {
                        let _ = request.reply.try_send(reply);
                    }
                    Err(_) => break,
                }
            }
        });
        (socket, feeder)
    }

    /// A bare broker state with the A3 owner + the given socket (the cadence
    /// tests do not need an artifact/segment).
    fn bare_state(socket: Option<std::path::PathBuf>) -> BrokerState {
        BrokerState {
            segment: None,
            applied: applied::AppliedState::default(),
            pending: None,
            terminal_fence: None,
            last_desired_generation: None,
        last_desired_world_key: None,
        duplicate_mapping_alerts: BTreeSet::new(),
            held_ops: BTreeSet::new(),
            held_structural_key: None,
            held_retry_at: None,
            held_retries: 0,
            removal_capture: None,
            pending_non_suffix: None,
            non_suffix_replay_due: false,
            non_suffix_trigger_key: None,
            deferred_sites: BTreeMap::new(),
            deferred_retires: BTreeMap::new(),
                slot_order: None,
            socket_path: socket,
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

    /// `lqos-74t.101`: a matching late-applied report after ACK_SILENCE adopts
    /// the desired world without a terminal fence, reject, or divergence.
    #[test]
    fn ack_silence_late_report_adopts_without_divergence() {
        let dir = std::env::temp_dir().join(format!(
            "lqosd-ack-late-adopt-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let world = base_tx();
        let key = translate::content_key(&world);
        let report = analytics_ipc::StateReport {
            generation: world.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&world),
            slot_order: Vec::new(),
            content_key: key.clone(),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) = state_socket(&dir, analytics_ipc::StateReply::Report(report));
        let mut state = bare_state(Some(socket));
        // An interim ack-fold of a DIFFERENT world would normally diverge; the
        // late-adopt path must not surface that false divergence.
        let mut interim = world.clone();
        interim.leaf_ops.truncate(1);
        interim.mapping_ops.truncate(1);
        state.applied.fold_ack(interim);
        state.last_desired_generation = Some(world.generation.clone());
        state.last_desired_world_key = Some(key.clone());

        assert!(
            adopt_late_from_report(&mut state),
            "matching key+generation must be adopted"
        );
        assert_eq!(state.applied.key(), Some(key.as_str()));
        assert!(
            !state.applied.diverged(),
            "a late apply is not an applied divergence"
        );
        assert!(
            state.terminal_fence.is_none(),
            "a late apply must not leave a terminal fence"
        );
        assert!(!state.late_adopt, "the late-adopt guard is one-shot");
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `lqos-101` gate-B/F1a equality: the report's `content_key` is the
    /// DESIRED world key carried VERBATIM by the bridge (the folded transaction
    /// it reports can have a DIFFERENT own key). The late adopt must accept it
    /// — this is the exact live mismatch the folded-key report hit.
    #[test]
    fn ack_silence_late_adopt_accepts_the_verbatim_desired_key_over_a_folded_tx() {
        let dir = std::env::temp_dir().join(format!(
            "lqosd-ack-late-desired-key-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let folded = base_tx();
        let mut desired = folded.clone();
        desired.leaf_ops.truncate(1);
        desired.mapping_ops.truncate(1);
        let desired_key = translate::content_key(&desired);
        let folded_key = translate::content_key(&folded);
        assert_ne!(desired_key, folded_key, "the fixture must fork the keys");
        let report = analytics_ipc::StateReport {
            generation: desired.generation.clone(),
            report_schema: 1,
            // The APPLIED/FOLDED world (its own key is `folded_key`)...
            transaction: report_tx_bytes(&folded),
            slot_order: Vec::new(),
            // ...the applied/cache identity stays folded...
            content_key: folded_key.clone(),
            // ...while the desired identity is the frame key, verbatim.
            desired_world_content_key: desired_key.clone(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) = state_socket(&dir, analytics_ipc::StateReply::Report(report));
        let mut state = bare_state(Some(socket));
        state.last_desired_generation = Some(desired.generation.clone());
        state.last_desired_world_key = Some(desired_key.clone());

        assert!(
            adopt_late_from_report(&mut state),
            "the verbatim desired key is the identity the late guard must accept"
        );
        assert_eq!(state.applied.key(), Some(folded_key.as_str()));
        assert_eq!(state.applied.desired_key(), Some(desired_key.as_str()));
        assert!(
            !state.applied.diverged(),
            "a late apply is never an applied divergence"
        );
        assert!(state.terminal_fence.is_none());
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `lqos-101` negative: an OLD/partial report whose `content_key` is the
    /// folded key (the pre-fix shape) must still be rejected when the broker's
    /// desired key differs. The key check is NOT dropped.
    #[test]
    fn ack_silence_late_adopt_rejects_the_folded_key_when_the_desired_key_differs() {
        let dir = std::env::temp_dir().join(format!(
            "lqosd-ack-late-folded-key-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let folded = base_tx();
        let mut desired = folded.clone();
        desired.leaf_ops.truncate(1);
        desired.mapping_ops.truncate(1);
        let desired_key = translate::content_key(&desired);
        let folded_key = translate::content_key(&folded);
        assert_ne!(desired_key, folded_key, "the fixture must fork the keys");
        let report = analytics_ipc::StateReport {
            generation: desired.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&folded),
            slot_order: Vec::new(),
            content_key: folded_key,
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) = state_socket(&dir, analytics_ipc::StateReply::Report(report));
        let mut state = bare_state(Some(socket));
        state.last_desired_generation = Some(desired.generation.clone());
        state.last_desired_world_key = Some(desired_key);

        assert!(
            !adopt_late_from_report(&mut state),
            "the folded/old report key must not satisfy the desired-key guard"
        );
        assert!(!state.applied.has_base(), "the old applied belief is kept");
        assert!(state.terminal_fence.is_none());
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `lqos-74t.101` pin: the PRODUCTION late-adopt path rejects a report
    /// that matches only one of content key / generation.
    #[test]
    fn ack_silence_late_adopt_rejects_wrong_key_or_generation() {
        let dir = std::env::temp_dir().join(format!(
            "lqosd-ack-late-mismatch-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let world = base_tx();
        let key = translate::content_key(&world);
        let mut wrong_generation = world.clone();
        wrong_generation.generation = "wrong-generation".to_string();
        let mut wrong_world = world.clone();
        wrong_world.leaf_ops.truncate(1);
        wrong_world.mapping_ops.truncate(1);
        let wrong_key = translate::content_key(&wrong_world);
        let key_match_bad_gen = analytics_ipc::StateReport {
            generation: wrong_generation.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&wrong_generation),
            slot_order: Vec::new(),
            content_key: key.clone(),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let gen_match_bad_key = analytics_ipc::StateReport {
            generation: world.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&wrong_world),
            slot_order: Vec::new(),
            content_key: wrong_key,
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) = state_socket_seq(
            &dir,
            vec![
                analytics_ipc::StateReply::Report(key_match_bad_gen),
                analytics_ipc::StateReply::Report(gen_match_bad_key),
            ],
        );
        let mut state = bare_state(Some(socket));
        state.last_desired_generation = Some(world.generation.clone());
        state.last_desired_world_key = Some(key);

        assert!(
            !adopt_late_from_report(&mut state),
            "matching content key but wrong generation must not late-adopt"
        );
        assert!(
            !adopt_late_from_report(&mut state),
            "matching generation but wrong content key must not late-adopt"
        );
        assert!(!state.applied.has_base(), "the old applied belief is kept");
        assert!(state.terminal_fence.is_none());
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `lqos-74t.101` pin: the broker ACK window is 4 x 30 s and strictly
    /// covers the bridge's derived 20 s fail-closed bound.
    #[test]
    fn broker_ack_window_covers_bridge_bound_without_terminal() {
        assert_eq!(ACK_SILENCE_MAX_TIMEOUTS, 4);
        assert!(
            ACK_TIMEOUT.as_millis() * u128::from(ACK_SILENCE_MAX_TIMEOUTS) > 20_000,
            "the broker ACK window must outlive the bridge 20 s convergence bound"
        );
    }

    fn base_tx() -> DataplaneTransaction {
        serde_json::from_str(
            r#"{
                "generation": "gen-adopt",
                "inner_ops": [
                    {"op": "upsert", "node_id": "s1", "parent_node_id": "root",
                     "download": {"commit_kbps": 100000, "ceil_kbps": 200000},
                     "upload": {"commit_kbps": 50000, "ceil_kbps": 100000}}
                ],
                "leaf_ops": [
                    {"op": "upsert", "circuit_id": "c1", "parent_node_id": "s1",
                     "download": {"commit_kbps": 10000, "ceil_kbps": 20000},
                     "upload": {"commit_kbps": 5000, "ceil_kbps": 10000},
                     "aqm": {"kind": "none"}, "admin_up": true},
                    {"op": "upsert", "circuit_id": "c2", "parent_node_id": "s1",
                     "download": {"commit_kbps": 10000, "ceil_kbps": 20000},
                     "upload": {"commit_kbps": 5000, "ceil_kbps": 10000},
                     "aqm": {"kind": "none"}, "admin_up": true}
                ],
                "mapping_ops": [
                    {"op": "map", "prefix_len": 32, "ip": "192.0.2.1", "circuit_id": "c1", "device_id": "d1"},
                    {"op": "map", "prefix_len": 32, "ip": "192.0.2.2", "circuit_id": "c2", "device_id": "d2"}
                ],
                "schema_version": 2
            }"#,
        )
        .unwrap()
    }


    /// THE gate: attach adopts the bridge-reported applied state, so a
    /// circuit removed from the config WHILE THE BROKER WAS DOWN diffs
    /// to Remove + Del naming the departed prefix (today it silently
    /// lingers forever — the G2 blindness, both axes).
    #[test]
    fn adoption_makes_removed_while_down_diff_to_remove_with_dels() {
        let dir = std::env::temp_dir().join(format!("r3-adopt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = base_tx();
        let report = analytics_ipc::StateReport {
            generation: "gen-adopt".to_string(),
            report_schema: 1,
            transaction: report_tx_bytes(&base),
            slot_order: Vec::new(),
            content_key: crate::bridge_broker::translate::content_key(&base),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) = state_socket(&dir, analytics_ipc::StateReply::Report(report));
        let adopted = match adopt_bridge_baseline(&socket) {
            AdoptedBaseline::Report { tx, .. } => tx,
            AdoptedBaseline::Unavailable(reason) => panic!("must adopt: {reason}"),
        };
        // The desired world AFTER the operator removed c2 while down.
        let mut desired = base_tx();
        desired.generation = "gen-desired".to_string();
        desired.leaf_ops.retain(|op| match op {
            DataplaneLeafOp::Upsert { circuit_id, .. } => circuit_id != "c2",
            _ => true,
        });
        desired
            .mapping_ops
            .retain(|op| match op {
                DataplaneMappingOp::Map { circuit_id, .. } => circuit_id != "c2",
                _ => true,
            });
        let outcome = translate::semantic_diff(&desired, Some(&adopted), None);
        assert_eq!(outcome.deltas().len(), 1, "one drain tx");
        let has_remove = outcome.deltas()[0]
            .leaf_ops
            .iter()
            .any(|op| matches!(op, DataplaneLeafOp::Remove { circuit_id } if circuit_id == "c2"));
        let has_del = outcome.deltas()[0]
            .mapping_ops
            .iter()
            .any(|op| matches!(op, DataplaneMappingOp::Del { ip, .. } if ip.to_string() == "192.0.2.2"));
        assert!(has_remove, "the departed circuit diff removes: {:?}", outcome.deltas()[0].leaf_ops);
        assert!(has_del, "the departed PREFIX diff deletes (F2: removes need the baseline's mappings)");
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `lqos-74t.102`: while the non-suffix gate is armed, a report that is NOT
    /// the full desired R3 world must not advance the applied state or clear the
    /// fence/capture. The next (matching) report adopts and clears the gate.
    #[test]
    fn non_suffix_gate_clears_only_after_the_r3_full_world_is_adopted() {
        let dir = std::env::temp_dir().join(format!("r3-non-suffix-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let desired = base_tx();
        let mut partial = base_tx();
        partial.leaf_ops.truncate(1);
        partial.mapping_ops.truncate(1);
        let partial_report = analytics_ipc::StateReport {
            generation: "gen-partial".to_string(),
            report_schema: 1,
            transaction: report_tx_bytes(&partial),
            slot_order: Vec::new(),
            content_key: crate::bridge_broker::translate::content_key(&partial),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let full_report = analytics_ipc::StateReport {
            generation: desired.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&desired),
            slot_order: slot_order_bytes(&["c1", "c2"]),
            content_key: crate::bridge_broker::translate::content_key(&desired),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) = state_socket_seq(
            &dir,
            vec![
                analytics_ipc::StateReply::Report(partial_report),
                analytics_ipc::StateReply::Report(full_report),
            ],
        );
        let mut state = bare_state(Some(socket));
        let capture = RemovalCapture {
            removed: vec!["c2".to_string()],
            slot_table_len: 3,
            slot_order_available: true,
            suffix: false,
            non_suffix: vec!["c2".to_string()],
        };
        state.pending_non_suffix = Some(capture.clone());
        state.terminal_fence = Some("non-suffix-fence".to_string());
        state.removal_capture = Some(capture);
        state.last_desired_world_key = Some(crate::bridge_broker::translate::content_key(&desired));

        assert!(
            !reconcile_from_report(&mut state, false),
            "a partial report must not adopt while the R3 gate is armed"
        );
        assert!(!state.applied.has_base(), "the old applied state is kept");
        assert_eq!(
            state
                .pending_non_suffix
                .as_ref()
                .map(|c| c.non_suffix.clone()),
            Some(vec!["c2".to_string()]),
            "the gate persists"
        );
        assert!(state.terminal_fence.is_some(), "the fence persists");
        assert!(state.removal_capture.is_some(), "the capture persists");

        assert!(
            reconcile_from_report(&mut state, true),
            "the full desired R3 report adopts"
        );
        assert!(state.applied.has_base());
        assert_eq!(
            state.applied.key(),
            Some(crate::bridge_broker::translate::content_key(&desired).as_str())
        );
        assert!(
            state.pending_non_suffix.is_none(),
            "the gate clears after R3"
        );
        assert!(state.terminal_fence.is_none(), "the fence clears after R3");
        assert!(
            state.removal_capture.is_none(),
            "the capture clears after R3"
        );
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `lqos-74t.102` full-slot oracle: a matching desired key with an ABSENT
    /// slot table is not proof of a committed R3/re-adopt. The gate stays until
    /// a report carries a full slot table.
    #[test]
    fn non_suffix_gate_rejects_absent_duplicate_and_short_slot_tables() {
        let dir = std::env::temp_dir().join(format!("r3-non-suffix-slot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let desired = base_tx();
        let key = crate::bridge_broker::translate::content_key(&desired);
        let absent = analytics_ipc::StateReport {
            generation: desired.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&desired),
            slot_order: Vec::new(),
            content_key: key.clone(),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let duplicate = analytics_ipc::StateReport {
            generation: desired.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&desired),
            slot_order: slot_order_bytes(&["c1", "c1"]),
            content_key: key.clone(),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let short = analytics_ipc::StateReport {
            generation: desired.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&desired),
            slot_order: slot_order_bytes(&["c1"]),
            content_key: key.clone(),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let duplicate_full = analytics_ipc::StateReport {
            generation: desired.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&desired),
            slot_order: slot_order_bytes(&["c1", "c2", "c1"]),
            content_key: key.clone(),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let full = analytics_ipc::StateReport {
            generation: desired.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&desired),
            slot_order: slot_order_bytes(&["c1", "c2"]),
            content_key: key,
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) = state_socket_seq(
            &dir,
            vec![
                analytics_ipc::StateReply::Report(absent),
                analytics_ipc::StateReply::Report(duplicate),
                analytics_ipc::StateReply::Report(short),
                analytics_ipc::StateReply::Report(duplicate_full),
                analytics_ipc::StateReply::Report(full),
            ],
        );
        let mut state = bare_state(Some(socket));
        let capture = RemovalCapture {
            removed: vec!["c2".to_string()],
            slot_table_len: 3,
            slot_order_available: true,
            suffix: false,
            non_suffix: vec!["c2".to_string()],
        };
        state.pending_non_suffix = Some(capture.clone());
        state.terminal_fence = Some("non-suffix-fence".to_string());
        state.removal_capture = Some(capture);
        state.last_desired_world_key =
            Some(crate::bridge_broker::translate::content_key(&desired));

        assert!(
            !reconcile_from_report(&mut state, false),
            "matching key without a slot table must not clear the gate"
        );
        assert!(state.pending_non_suffix.is_some(), "gate persists");
        assert!(
            !reconcile_from_report(&mut state, true),
            "a duplicate slot name must not stand in for the full set"
        );
        assert!(state.pending_non_suffix.is_some(), "gate persists on duplicate");
        assert!(
            !reconcile_from_report(&mut state, true),
            "a short slot table must not stand in for the full set"
        );
        assert!(state.pending_non_suffix.is_some(), "gate persists on short");
        assert!(
            !reconcile_from_report(&mut state, true),
            "a duplicate-plus-full slot table must not pass the bijection"
        );
        assert!(
            state.pending_non_suffix.is_some(),
            "gate persists on duplicate-plus-full"
        );
        assert!(
            reconcile_from_report(&mut state, true),
            "the full-slot report clears the gate"
        );
        assert!(state.pending_non_suffix.is_none());
        assert!(state.terminal_fence.is_none());
        assert!(state.removal_capture.is_none());
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `lqos-74t.102` §2.4(10): refresh the current artifact/key BEFORE the
    /// due report reconcile. With a stale persisted c4 key and a due c4 report,
    /// the old ordering would match c4 and clear the gate; the current c5
    /// artifact must be read first, keep the gate, and only the c5 report may
    /// clear it.
    #[test]
    fn non_suffix_gate_refreshes_current_desired_before_a_due_old_report() {
        let dir = std::env::temp_dir().join(format!("r3-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("dataplane_transaction.json");
        let old = base_tx();
        let mut current = base_tx();
        current.generation = "matrix-current-c5".to_string();
        current.leaf_ops.retain(|op| {
            !matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c2")
        });
        current.mapping_ops.retain(|op| {
            !matches!(op, DataplaneMappingOp::Map { circuit_id, .. } if circuit_id == "c2")
        });
        std::fs::write(&artifact, serde_json::to_vec(&current).unwrap()).unwrap();
        let old_report = analytics_ipc::StateReport {
            generation: old.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&old),
            slot_order: slot_order_bytes(&["c1", "c2"]),
            content_key: crate::bridge_broker::translate::content_key(&old),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let current_report = analytics_ipc::StateReport {
            generation: current.generation.clone(),
            report_schema: 1,
            transaction: report_tx_bytes(&current),
            slot_order: slot_order_bytes(&["c1"]),
            content_key: crate::bridge_broker::translate::content_key(&current),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) = state_socket_seq(
            &dir,
            vec![
                analytics_ipc::StateReply::Report(old_report),
                analytics_ipc::StateReply::Report(current_report),
            ],
        );
        let mut state = bare_state(Some(socket));
        let capture = RemovalCapture {
            removed: vec!["c2".to_string()],
            slot_table_len: 2,
            slot_order_available: true,
            suffix: false,
            non_suffix: vec!["c2".to_string()],
        };
        state.pending_non_suffix = Some(capture.clone());
        state.removal_capture = Some(capture);
        // Deliberately stale: the persisted trigger-time key is c4.
        state.last_desired_world_key =
            Some(crate::bridge_broker::translate::content_key(&old));
        state.report_reread_due = true;

        step(&artifact, &mut state);
        assert!(
            state.pending_non_suffix.is_some(),
            "a due old c4 report must not clear the c5 gate"
        );
        state.report_reread_due = true;
        state.report_reread_at = None;
        step(&artifact, &mut state);
        assert!(
            state.pending_non_suffix.is_none(),
            "the current c5 report clears after the gate held the old report"
        );
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The fallback: a typed unavailable (no report yet — a fresh bridge
    /// boot) degrades to baseline-None, i.e. exactly today's posture.
    #[test]
    fn adoption_degrades_on_typed_unavailable() {
        let dir = std::env::temp_dir().join(format!("r3-unavail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (socket, feeder) = state_socket(
            &dir,
            analytics_ipc::StateReply::Unavailable(
                analytics_ipc::StateUnavailableReason::NoReportYet,
            ),
        );
        match adopt_bridge_baseline(&socket) {
            AdoptedBaseline::Report { .. } => panic!("unavailable must not adopt"),
            AdoptedBaseline::Unavailable(reason) => {
                assert!(reason.contains("NoReportYet"), "{reason}");
            }
        }
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A3 criterion 2 (`docs/broker_applied_single_source_card.md` §2): the
    /// cadence MUST RE-READ THE REPORT. A failed adopt (typed `Unavailable` —
    /// the REAL seam, no test bypass) leaves NO base (A1: HOLD + surface; never
    /// fabricate a full-world); the cadence then re-reads and the base RETURNS —
    /// recovery is AUTOMATIC. (A re-run of `semantic_diff` with `base==None`
    /// would instead be a PERMANENT hold; this asserts the report re-read.)
    #[test]
    fn cadence_rereads_the_report_and_recovers_from_a_hold() {
        let dir = std::env::temp_dir().join(format!("a3-reread-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let recovered = base_tx();
        let report = analytics_ipc::StateReport {
            generation: "gen-recovered".to_string(),
            report_schema: 1,
            transaction: report_tx_bytes(&recovered),
            slot_order: Vec::new(),
            content_key: crate::bridge_broker::translate::content_key(&recovered),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) = state_socket_seq(
            &dir,
            vec![
                analytics_ipc::StateReply::Unavailable(
                    analytics_ipc::StateUnavailableReason::NoReportYet,
                ),
                analytics_ipc::StateReply::Report(report),
            ],
        );
        let mut state = bare_state(Some(socket));
        // First read: the real seam says unavailable — A1 HOLD, no fabrication.
        assert!(
            !reconcile_from_report(&mut state, false),
            "an unavailable report must not adopt"
        );
        assert!(
            !state.applied.has_base(),
            "no base after a failed adopt (A1 HOLD; never fabricate)"
        );
        // The cadence re-read (not a semantic_diff with base==None): the report
        // returned, so the base is adopted — automatic recovery.
        maybe_reread_report(&mut state);
        assert!(
            state.applied.has_base(),
            "the cadence re-read must adopt the returned report"
        );
        assert_eq!(
            state.applied.provenance().map(|p| p.as_str()),
            Some("BASE_ADOPT")
        );
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A3 criterion 1/3 (end-to-end through the real socket): a report that
    /// DISAGREES with the interim ack-fold WINS and the divergence is SURFACED.
    #[test]
    fn reconcile_surfaces_a_report_that_disagrees_with_the_folded_ack() {
        let dir = std::env::temp_dir().join(format!("a3-diverge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = bare_state(None);
        // The interim ack-fold claims the two-circuit world (c1, c2).
        state.applied.fold_ack(base_tx());
        // The report disagrees: a one-circuit world.
        let mut report_world = base_tx();
        report_world.leaf_ops.truncate(1);
        let report = analytics_ipc::StateReport {
            generation: "gen-report".to_string(),
            report_schema: 1,
            transaction: report_tx_bytes(&report_world),
            slot_order: Vec::new(),
            content_key: crate::bridge_broker::translate::content_key(&report_world),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) = state_socket(&dir, analytics_ipc::StateReply::Report(report));
        state.socket_path = Some(socket);
        assert!(
            reconcile_from_report(&mut state, false),
            "the diverging report must adopt (the report is authoritative)"
        );
        assert!(
            state.applied.diverged(),
            "the report-vs-fold divergence must be surfaced (report wins)"
        );
        assert_eq!(
            state.applied.provenance().map(|p| p.as_str()),
            Some("BASE_ADOPT")
        );
        feeder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F8 rider: the steady key is FLOOR-CONSISTENT — a suspended (0/0)
    /// circuit in config no longer forces an empty re-diff every poll.
    #[test]
    fn steady_key_is_floor_consistent() {
        let mut desired = base_tx();
        if let DataplaneLeafOp::Upsert {
            download, upload, ..
        } = &mut desired.leaf_ops[0]
        {
            download.commit_kbps = 0;
            download.ceil_kbps = 0;
            upload.commit_kbps = 0;
            upload.ceil_kbps = 0;
        }
        // The PUBLISHED baseline holds the FLOORED content (what the
        // bridge applied).
        let floored = translate::apply_suspend_floor(&desired);
        let baseline_key = translate::content_key(&floored);
        // The broker's poll keys the floored desired now — the keys MATCH.
        assert_eq!(
            translate::content_key(&translate::apply_suspend_floor(&desired)),
            baseline_key,
            "an already-applied suspended config must key steady"
        );
    }

    /// §`lqos-74t.30` (encode-once): the world-compile source now emits the
    /// FLOORED (1024) pair for a suspended circuit, so `apply_suspend_floor` is an
    /// IDEMPOTENT NO-OP — `content_key(artifact) ==
    /// content_key(apply_suspend_floor(artifact))`. This is the property that
    /// removes the RAW-vs-FLOORED fork: the report (bridge LKG), the R3 install,
    /// the advertised seam key, and the broker fold all key the SAME world.
    #[test]
    fn floored_artifact_keys_identically_to_its_suspend_floor() {
        let mut desired = base_tx();
        if let DataplaneLeafOp::Upsert {
            download, upload, ..
        } = &mut desired.leaf_ops[0]
        {
            // The FLOORED pair the compiler now emits (not 0/0).
            download.commit_kbps = 1024;
            download.ceil_kbps = 1024;
            upload.commit_kbps = 1024;
            upload.ceil_kbps = 1024;
        }
        assert_eq!(
            translate::content_key(&desired),
            translate::content_key(&translate::apply_suspend_floor(&desired)),
            "an already-floored artifact must key idempotently (no RAW/FLOORED fork)"
        );
    }

    /// `lqos-101` STALE-GENERATION pin: the ack-fold uses the key STORED on the
    /// Pending at encode, never the CURRENT `last_desired_world_key`. A config
    /// churn that advances the artifact mid-flight must not relabel the world
    /// this ack actually applied.
    #[test]
    fn stale_generation_fold_uses_the_pending_key_not_the_newer_desired() {
        let b = base_tx();
        let mut c = b.clone();
        c.leaf_ops.truncate(1);
        c.mapping_ops.truncate(1);
        let b_key = translate::content_key(&b);
        let c_key = translate::content_key(&c);
        assert_ne!(b_key, c_key, "the fixture must fork the desired keys");

        let mut state = bare_state(None);
        // The artifact advanced to C while B's ack was still in flight.
        state.last_desired_generation = Some("gen-c".to_string());
        state.last_desired_world_key = Some(c_key.clone());
        // The pending captured B at encode; the ack path passes THAT key.
        state
            .applied
            .fold_ack_with_desired(b.clone(), Some(b_key.clone()));

        assert_eq!(state.applied.desired_key(), Some(b_key.as_str()));
        assert_ne!(
            state.applied.desired_key(),
            Some(c_key.as_str()),
            "the newer desired key must NOT relabel the in-flight B ack"
        );
        assert_eq!(
            state.applied.key(),
            Some(translate::content_key(&b).as_str()),
            "the applied/cache key stays folded"
        );
    }
}

#[cfg(test)]
mod r3c_matrix_tests {
    use super::*;

    fn fold_tx(order: &[&str]) -> DataplaneTransaction {
        DataplaneTransaction {
            generation: "gen-matrix".to_string(),
            inner_ops: vec![],
            leaf_ops: order
                .iter()
                .map(|name| DataplaneLeafOp::Upsert {
                    circuit_id: (*name).to_string(),
                    parent_node_id: "s1".to_string(),
                    download: lqos_dataplane_types::DataplaneRatePair {
                        commit_kbps: 10_000,
                        ceil_kbps: 20_000,
                    },
                    upload: lqos_dataplane_types::DataplaneRatePair {
                        commit_kbps: 5_000,
                        ceil_kbps: 10_000,
                    },
                    aqm: lqos_dataplane_types::DataplaneLeafAqm::None,
                    admin_up: true,
                })
                .collect(),
            mapping_ops: vec![],
            schema_version: Some(2),
        
            steering_vlans: Vec::new(), lpm_delta: Vec::new(), apply_mode: Default::default(), circuit_capacity: None, }
    }

    /// §2.4 matrix — pure reorder (same set): PROCEED with the paradox
    /// flag — the L1 belief-breaker instrumented, not fatal.
    #[test]
    fn matrix_pure_reorder_proceeds_flagged() {
        let tx = fold_tx(&["c1", "c2", "c3"]);
        let order = Some(vec!["c3".to_string(), "c1".to_string(), "c2".to_string()]);
        assert_eq!(
            classify_slot_order(&tx, &order),
            SlotOrderClass::Proceed { paradox: true }
        );
    }

    /// §2.4 matrix — identity order: proceed, no paradox.
    #[test]
    fn matrix_identity_order_proceeds_clean() {
        let tx = fold_tx(&["c1", "c2"]);
        let order = Some(vec!["c1".to_string(), "c2".to_string()]);
        assert_eq!(
            classify_slot_order(&tx, &order),
            SlotOrderClass::Proceed { paradox: false }
        );
    }

    /// §2.4 matrix — a STRAY (the table names a circuit the fold never
    /// saw — e.g. an install-completed/fence-lost crash window): ABORT
    /// to restart-class; the walk has no drawable arm.
    #[test]
    fn matrix_stray_aborts_to_restart_class() {
        let tx = fold_tx(&["c1", "c2"]);
        let order = Some(vec![
            "c1".to_string(),
            "c2".to_string(),
            "cX".to_string(),
        ]);
        assert_eq!(
            classify_slot_order(&tx, &order),
            SlotOrderClass::Stray(vec!["cX".to_string()])
        );
    }

    /// §2.4 matrix — the ABSENT table (old bridge): the fold-order
    /// fallback degrade.
    #[test]
    fn matrix_absent_table_falls_back() {
        let tx = fold_tx(&["c1", "c2"]);
        assert_eq!(classify_slot_order(&tx, &None), SlotOrderClass::Absent);
    }
}

#[cfg(test)]
mod ghost_segment_tests {
    use super::seam::{AckFrame, create_full_test_segment, create_test_segment, unlink_test_segment};
    use super::*;
    use lqos_dataplane_types::DataplaneLeafOp;
    use lqos_dataplane_types::wire::AckOutcome;

    /// The idle arm checks `SEGMENT_NAME`, so these units must own the real
    /// segment name; serialize them (cargo runs tests in parallel).
    static GHOST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_state(segment: Option<SeamSegment>) -> BrokerState {
        // A1: a step with NO base now HOLDS, so the publish-path tests need a
        // base (the empty applied world: site-1, no circuits).
        let test_base = fence_tx(&[]);
        // A3: seed the one owner with the test base (a folded interim belief —
        // the tests exercise the publish path, not the adopt).
        let mut applied = applied::AppliedState::default();
        applied.fold_ack(test_base);
        BrokerState {
            segment,
            applied,
            pending: None,
            terminal_fence: None,
            last_desired_generation: None,
        last_desired_world_key: None,
        duplicate_mapping_alerts: BTreeSet::new(),
            held_ops: BTreeSet::new(),
            held_structural_key: None,
            held_retry_at: None,
            held_retries: 0,
            removal_capture: None,
            pending_non_suffix: None,
            non_suffix_replay_due: false,
            non_suffix_trigger_key: None,
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

    fn absent_artifact() -> PathBuf {
        std::env::temp_dir().join(format!("lqosd-ghost-absent-{}.json", std::process::id()))
    }

    /// C3(b): consecutive terminal rejects arm a growing (capped) backoff; an
    /// APPLIED ack resets it.
    #[test]
    fn consecutive_rejects_arm_a_bounded_backoff_and_applied_clears_it() {
        let mut state = test_state(None);
        for _ in 0..(REJECT_BACKOFF_THRESHOLD - 1) {
            state.record_reject();
        }
        assert!(
            state.backoff_until.is_none(),
            "below the threshold no backoff is armed"
        );
        state.record_reject();
        let first = state.backoff_until.expect("the threshold arms a backoff");
        state.record_reject();
        let second = state.backoff_until.expect("still armed");
        assert!(
            second > first,
            "the backoff grows with each consecutive reject"
        );

        state.record_applied_reset();
        assert!(state.backoff_until.is_none());
        assert_eq!(state.rejects_consecutive, 0);
    }

    fn fence_tx(circuits: &[&str]) -> DataplaneTransaction {
        use lqos_dataplane_types::{DataplaneInnerOp, DataplaneLeafAqm, DataplaneRatePair};
        let pair = DataplaneRatePair {
            commit_kbps: 1_000,
            ceil_kbps: 2_000,
        };
        DataplaneTransaction {
            generation: "gen-fence-test".to_string(),
            inner_ops: vec![DataplaneInnerOp::Upsert {
                node_id: "site-1".to_string(),
                parent_node_id: "root".to_string(),
                download: pair,
                upload: pair,
            }],
            leaf_ops: circuits
                .iter()
                .map(|c| DataplaneLeafOp::Upsert {
                    circuit_id: (*c).to_string(),
                    parent_node_id: "site-1".to_string(),
                    download: pair,
                    upload: pair,
                    aqm: DataplaneLeafAqm::None,
                    admin_up: true,
                })
                .collect(),
            mapping_ops: vec![],
            schema_version: Some(2),
            steering_vlans: Vec::new(),
            lpm_delta: Vec::new(),
            apply_mode: Default::default(),
            circuit_capacity: None,
        }
    }

    fn write_fence_artifact(path: &PathBuf, tx: &DataplaneTransaction) {
        std::fs::write(path, serde_json::to_string(tx).expect("serialize"))
            .expect("write fence artifact");
    }

    /// `lqos-101` AT-PUSH equality: the Pending STORES the desired key at
    /// encode, and it is the SAME value the frames carry (`last_desired_world_key`
    /// is the encode argument). This is the identity the ack-fold later uses.
    #[test]
    fn pending_stores_the_frame_desired_key_at_encode() {
        let _guard = GHOST_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!(
            "lqosd-pending-key-{}.json",
            std::process::id()
        ));
        create_test_segment(SEGMENT_NAME, [11u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        let desired = fence_tx(&["c1", "c2"]);
        write_fence_artifact(&path, &desired);
        step(&path, &mut state);
        let pending = state
            .pending
            .take()
            .expect("the structured delta must emit");
        assert_eq!(
            pending.desired_key, state.last_desired_world_key,
            "the Pending stores the SAME key the frames carry"
        );
        assert_eq!(
            pending.desired_key.as_deref(),
            Some(translate::content_key(&desired).as_str()),
            "the stored key is the desired world's, not a folded value"
        );
        let _ = std::fs::remove_file(&path);
        unlink_test_segment(SEGMENT_NAME);
    }



    /// L2 (`docs/control_plane_structural_convergence_card.md` §3): a structural
    /// reject QUARANTINES the offending op instead of freezing the world. With a
    /// held op the broker publishes the APPLIABLE PROJECTION (an unaffected op
    /// still applies); a structural change clears the hold (retry) and re-emits;
    /// a re-attach clears the hold too. (Replaces the old world-freeze test.)
    #[test]
    fn structural_hold_quarantines_the_op_and_never_freezes_the_world() {
        let _guard = GHOST_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!("lqosd-hold-{}.json", std::process::id()));
        create_test_segment(SEGMENT_NAME, [7u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));

        let tx1 = fence_tx(&["c1", "c2"]);
        write_fence_artifact(&path, &tx1);

        // L2: with a held op matching the desired structure, publish the
        // PROJECTION — c1 quarantined, c2 still applies (NO world-freeze).
        state.held_ops.insert(translate::HeldOp::Leaf {
            circuit_id: "c1".to_string(),
        });
        state.held_structural_key = Some(translate::structural_identity(&tx1));
        step(&path, &mut state);
        let pending = state
            .pending
            .take()
            .expect("the projection must still publish (no world-freeze)");
        assert!(
            !pending.current_tx.leaf_ops.iter().any(
                |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c1")
            ),
            "the held op is quarantined (not emitted)"
        );
        assert!(
            pending.current_tx.leaf_ops.iter().any(
                |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c2")
            ),
            "the unaffected op still applies in the projection"
        );

        // L2-5: a structural change clears the hold and re-emits the full world.
        let tx2 = fence_tx(&["c1", "c2", "c3"]);
        write_fence_artifact(&path, &tx2);
        state.held_ops.insert(translate::HeldOp::Leaf {
            circuit_id: "c1".to_string(),
        });
        state.held_structural_key = Some(translate::structural_identity(&tx1));
        step(&path, &mut state);
        assert!(
            state.held_ops.is_empty(),
            "a structural change clears the hold (retry)"
        );
        assert!(state.pending.is_some(), "and re-emits the full desired");

        // Re-attach clears the hold too.
        state.held_ops.insert(translate::HeldOp::Leaf {
            circuit_id: "c1".to_string(),
        });
        state.held_structural_key = Some(translate::structural_identity(&tx2));
        state.segment = None;
        state.attach_polls = RETRY_ATTACH_EVERY - 1;
        create_test_segment(SEGMENT_NAME, [8u8; 16]);
        step(&path, &mut state);
        assert!(state.held_ops.is_empty(), "a re-attach clears the hold");

        unlink_test_segment(SEGMENT_NAME);
        let _ = std::fs::remove_file(&path);
    }

    /// CARD §2 (`docs/broker_single_structural_op_sequencing_card.md`): the
    /// bounded AUTO-RETRY — a held op past its retry deadline is dropped for ONE
    /// attempt (so a transient cause self-clears without an operator re-attach).
    #[test]
    fn held_op_auto_retries_after_the_backoff_deadline() {
        let _guard = GHOST_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!("lqosd-retry-{}.json", std::process::id()));
        create_test_segment(SEGMENT_NAME, [9u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        let tx1 = fence_tx(&["c1", "c2"]);
        write_fence_artifact(&path, &tx1);
        state.held_ops.insert(translate::HeldOp::Leaf {
            circuit_id: "c1".to_string(),
        });
        state.held_structural_key = Some(translate::structural_identity(&tx1));
        // Past the deadline: the retry drops the hold for one attempt.
        state.held_retry_at = Some(std::time::Instant::now() - HELD_RETRY_BACKOFF);
        step(&path, &mut state);
        assert!(
            state.held_ops.is_empty(),
            "the held op is re-attempted past the deadline (no re-attach)"
        );
        assert!(state.pending.is_some(), "the full desired is attempted");

        unlink_test_segment(SEGMENT_NAME);
        let _ = std::fs::remove_file(&path);
    }

    /// URGENT (jackrabbit stall): the auto-retry is CAPPED — a
    /// genuinely-unappliable hold is NOT re-emitted forever.
    #[test]
    fn held_op_stops_retrying_after_the_cap() {
        let _guard = GHOST_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!("lqosd-retrycap-{}.json", std::process::id()));
        create_test_segment(SEGMENT_NAME, [10u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        let tx1 = fence_tx(&["c1", "c2"]);
        write_fence_artifact(&path, &tx1);
        state.held_ops.insert(translate::HeldOp::Leaf {
            circuit_id: "c1".to_string(),
        });
        state.held_structural_key = Some(translate::structural_identity(&tx1));
        // Past the deadline AND past the retry cap: the hold is KEPT (no re-emit).
        state.held_retries = HELD_RETRY_MAX;
        state.held_retry_at = Some(std::time::Instant::now() - HELD_RETRY_BACKOFF);
        step(&path, &mut state);
        assert!(
            !state.held_ops.is_empty(),
            "past the cap the genuinely-unappliable hold is KEPT, not re-emitted"
        );

        unlink_test_segment(SEGMENT_NAME);
        let _ = std::fs::remove_file(&path);
    }

    /// (1) idle-replacement: an attached, idle broker detects a replaced
    /// segment and drops it to None (the existing attach machinery recovers).
    #[test]
    fn idle_replacement_drops_segment_to_none() {
        let _guard = GHOST_LOCK.lock().unwrap();
        create_test_segment(SEGMENT_NAME, [1u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        state.liveness_polls = RETRY_ATTACH_EVERY - 1;
        // The bridge restart: unconditional unlink + recreate (new inode).
        create_test_segment(SEGMENT_NAME, [2u8; 16]);
        step(&absent_artifact(), &mut state);
        assert!(
            state.segment.is_none(),
            "a replaced idle seam must drop to None"
        );
        assert_eq!(state.liveness_polls, 0, "the counter resets on the poll");
        unlink_test_segment(SEGMENT_NAME);
    }

    /// (2) no churn: an intact segment is a no-op (retained, counter resets).
    #[test]
    fn idle_intact_segment_is_a_noop() {
        let _guard = GHOST_LOCK.lock().unwrap();
        create_test_segment(SEGMENT_NAME, [3u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        state.liveness_polls = RETRY_ATTACH_EVERY - 1;
        step(&absent_artifact(), &mut state);
        assert!(state.segment.is_some(), "an intact seam must be retained");
        assert_eq!(state.liveness_polls, 0);
        unlink_test_segment(SEGMENT_NAME);
    }

    /// (3) COLD/FIRST TICK (bootstrap rule): the poll is bounded by the ~10s
    /// cadence — no-op steps until the first cadence, then detection.
    #[test]
    fn idle_replacement_detected_on_the_first_cadence() {
        let _guard = GHOST_LOCK.lock().unwrap();
        create_test_segment(SEGMENT_NAME, [4u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        create_test_segment(SEGMENT_NAME, [5u8; 16]); // replaced before the first poll
        for _ in 0..(RETRY_ATTACH_EVERY - 1) {
            step(&absent_artifact(), &mut state);
            assert!(state.segment.is_some(), "not yet at the cadence");
        }
        step(&absent_artifact(), &mut state);
        assert!(state.segment.is_none(), "detected on the first cadence");
        unlink_test_segment(SEGMENT_NAME);
    }

    /// (4) absent-name (bridge down mid-idle): is_replaced is fail-true, so
    /// the idle arm also covers bridge death; the Absent retry arm holds.
    #[test]
    fn idle_absent_segment_drops_to_none() {
        let _guard = GHOST_LOCK.lock().unwrap();
        create_test_segment(SEGMENT_NAME, [6u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        state.liveness_polls = RETRY_ATTACH_EVERY - 1;
        unlink_test_segment(SEGMENT_NAME);
        step(&absent_artifact(), &mut state);
        assert!(state.segment.is_none(), "an absent seam must drop to None");
    }

    fn non_suffix_capture(circuits: &[&str]) -> RemovalCapture {
        RemovalCapture {
            removed: circuits.iter().map(|c| (*c).to_string()).collect(),
            slot_table_len: circuits.len() + 1,
            slot_order_available: true,
            suffix: false,
            non_suffix: circuits.iter().map(|c| (*c).to_string()).collect(),
        }
    }

    /// `lqos-74t.102`: the trigger itself carries ONLY the held non-suffix
    /// Removes — never an ADD — so the next structural ADD cannot reach the
    /// partial slot table.
    #[test]
    fn non_suffix_trigger_has_removes_and_no_add() {
        let desired = fence_tx(&["c1", "c3"]);
        let capture = non_suffix_capture(&["c2"]);
        let trigger = non_suffix_rebuild_trigger(&desired, &capture).expect("trigger");
        assert_eq!(trigger.generation, desired.generation);
        assert!(trigger.inner_ops.is_empty());
        assert!(trigger.mapping_ops.is_empty());
        assert_eq!(trigger.leaf_ops.len(), 1);
        assert!(matches!(
            &trigger.leaf_ops[0],
            DataplaneLeafOp::Remove { circuit_id } if circuit_id == "c2"
        ));
    }

    /// `lqos-74t.102` broker/publish-level regression: a non-suffix capture is
    /// replaced by the remove-only R3 trigger (the new ADD is suppressed); the
    /// gate persists across the next structural identity; after the R3 full
    /// world is represented in the applied base the gate clears and delivery
    /// resumes.
    #[test]
    fn non_suffix_capture_suppresses_the_next_add_until_r3_then_resumes() {
        let _guard = GHOST_LOCK.lock().unwrap();
        let path =
            std::env::temp_dir().join(format!("lqosd-non-suffix-{}.json", std::process::id()));
        create_test_segment(SEGMENT_NAME, [11u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));

        // The applied world and bridge slot order include the interior c2.
        let base = fence_tx(&["c1", "c2", "c3"]);
        let mut applied = applied::AppliedState::default();
        applied.fold_ack(base);
        state.applied = applied;
        state.slot_order = Some(vec!["c1".to_string(), "c2".to_string(), "c3".to_string()]);

        // Desired removes the interior c2 AND adds c4: exactly the incident's
        // non-suffix remove followed by a structural identity change.
        let desired = fence_tx(&["c1", "c3", "c4"]);
        write_fence_artifact(&path, &desired);
        step(&path, &mut state);

        let pending = state
            .pending
            .take()
            .expect("the capture tick must emit the R3 trigger");
        assert!(
            state.pending_non_suffix.is_some(),
            "the non-suffix gate must be armed"
        );
        assert!(
            pending
                .current_tx
                .leaf_ops
                .iter()
                .all(|op| !matches!(op, DataplaneLeafOp::Upsert { .. })),
            "the trigger must carry NO ADD: {:?}",
            pending.current_tx.leaf_ops
        );
        assert!(
            pending.current_tx.leaf_ops.iter().any(
                |op| matches!(op, DataplaneLeafOp::Remove { circuit_id } if circuit_id == "c2")
            ),
            "the trigger must carry the non-suffix Remove"
        );

        // The next structural ADD MUST NOT be emitted while the gate is set.
        let desired_next = fence_tx(&["c1", "c3", "c4", "c5"]);
        write_fence_artifact(&path, &desired_next);
        step(&path, &mut state);
        assert!(
            state.pending.is_none(),
            "the next structural ADD must be suppressed while the gate is set"
        );
        assert!(state.pending_non_suffix.is_some(), "the gate persists");

        // The post-R3 report is the FULL desired world. The broker clears the
        // gate by ADOPTING that report — no manual state clearing.
        let sock_dir =
            std::env::temp_dir().join(format!("lqosd-non-suffix-sock-{}", std::process::id()));
        std::fs::create_dir_all(&sock_dir).unwrap();
        let report = analytics_ipc::StateReport {
            generation: desired_next.generation.clone(),
            report_schema: 1,
            transaction: super::r3_tests::report_tx_bytes(&desired_next),
            slot_order: super::r3_tests::slot_order_bytes(&["c1", "c3", "c4", "c5"]),
            content_key: translate::content_key(&desired_next),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) =
            super::r3_tests::state_socket(&sock_dir, analytics_ipc::StateReply::Report(report));
        state.socket_path = Some(socket);
        assert!(
            reconcile_from_report(&mut state, true),
            "the post-R3 full-world report must adopt"
        );
        feeder.join().unwrap();
        assert!(state.pending_non_suffix.is_none(), "the gate clears on R3");
        assert!(state.terminal_fence.is_none(), "the fence clears on R3");
        assert!(state.removal_capture.is_none(), "the capture clears on R3");
        assert_eq!(
            state.applied.key(),
            Some(translate::content_key(&desired_next).as_str()),
            "the applied belief advances only to the R3 full world"
        );
        state.socket_path = None;
        let _ = std::fs::remove_dir_all(&sock_dir);

        // Delivery resumes with the NEXT structural ADD (c6).
        let desired_resume = fence_tx(&["c1", "c3", "c4", "c5", "c6"]);
        write_fence_artifact(&path, &desired_resume);
        step(&path, &mut state);
        let resumed = state
            .pending
            .take()
            .expect("delivery must resume after the R3 full world");
        assert!(
            resumed.current_tx.leaf_ops.iter().any(
                |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c6")
            ),
            "the new structural ADD resumes after R3"
        );
        assert!(
            resumed.current_tx.leaf_ops.iter().all(
                |op| !matches!(op, DataplaneLeafOp::Remove { circuit_id } if circuit_id == "c2")
            ),
            "the full-world applied base no longer removes c2"
        );

        unlink_test_segment(SEGMENT_NAME);
        let _ = std::fs::remove_file(&path);
    }

    /// `lqos-74t.102` stale expected key: while the gate is armed, the broker
    /// must still read the artifact. A desired change (c4 -> c5) updates the
    /// expected full-desired key; the post-R3 c5 report then matches and clears
    /// the gate (no unbounded `NON_SUFFIX_R3_WAIT`).
    #[test]
    fn non_suffix_expected_key_tracks_a_desired_change_while_armed() {
        let _guard = GHOST_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!(
            "lqosd-non-suffix-stale-{}.json",
            std::process::id()
        ));
        create_test_segment(SEGMENT_NAME, [12u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));

        let base = fence_tx(&["c1", "c2", "c3"]);
        let mut applied = applied::AppliedState::default();
        applied.fold_ack(base);
        state.applied = applied;
        state.slot_order = Some(vec!["c1".to_string(), "c2".to_string(), "c3".to_string()]);

        let desired = fence_tx(&["c1", "c3", "c4"]);
        write_fence_artifact(&path, &desired);
        step(&path, &mut state);
        state.pending.take().expect("capture trigger");
        assert!(state.pending_non_suffix.is_some());
        assert_eq!(
            state.last_desired_world_key.as_deref(),
            Some(translate::content_key(&desired).as_str())
        );

        // Desired changes while armed: the artifact must be re-read and the
        // expected key refreshed, with no emission.
        let desired_next = fence_tx(&["c1", "c3", "c4", "c5"]);
        write_fence_artifact(&path, &desired_next);
        step(&path, &mut state);
        assert!(state.pending.is_none(), "armed gate emits nothing");
        assert_eq!(
            state.last_desired_world_key.as_deref(),
            Some(translate::content_key(&desired_next).as_str()),
            "expected key must track the current desired"
        );

        // The R3 report is the NEW full world; it matches and clears.
        let sock_dir =
            std::env::temp_dir().join(format!("lqosd-non-suffix-stale-sock-{}", std::process::id()));
        std::fs::create_dir_all(&sock_dir).unwrap();
        let report = analytics_ipc::StateReport {
            generation: desired_next.generation.clone(),
            report_schema: 1,
            transaction: super::r3_tests::report_tx_bytes(&desired_next),
            slot_order: super::r3_tests::slot_order_bytes(&["c1", "c3", "c4", "c5"]),
            content_key: translate::content_key(&desired_next),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (socket, feeder) =
            super::r3_tests::state_socket(&sock_dir, analytics_ipc::StateReply::Report(report));
        state.socket_path = Some(socket);
        assert!(
            reconcile_from_report(&mut state, true),
            "the c5 R3 report must adopt"
        );
        feeder.join().unwrap();
        assert!(state.pending_non_suffix.is_none(), "gate clears on c5 R3");
        assert!(state.terminal_fence.is_none());
        assert!(state.removal_capture.is_none());
        assert_eq!(
            state.applied.key(),
            Some(translate::content_key(&desired_next).as_str())
        );
        state.socket_path = None;
        let _ = std::fs::remove_dir_all(&sock_dir);
        unlink_test_segment(SEGMENT_NAME);
        let _ = std::fs::remove_file(&path);
    }

    /// `lqos-74t.102`: an APPLIED ack for the remove-only trigger while armed
    /// is a partial-world acceptance and must not advance the belief.
    #[test]
    fn non_suffix_applied_ack_is_suspect_only_when_armed() {
        let capture = non_suffix_capture(&["c2"]);
        assert!(!non_suffix_applied_is_suspect(None));
        assert!(non_suffix_applied_is_suspect(Some(&capture)));
    }

    fn write_gate_snapshot(path: &std::path::Path, capture: &RemovalCapture, key: &str) {
        let snap = BridgeConvergenceSnapshot {
            pending_non_suffix: Some(capture.clone()),
            removal_capture: Some(capture.clone()),
            desired_world_content_key: Some(key.to_string()),
            ..Default::default()
        };
        std::fs::write(path, serde_json::to_vec(&snap).expect("serialize snapshot")).unwrap();
    }

    /// `lqos-74t.102` restart ordering: the restored gate reads the CURRENT
    /// artifact before any report reconcile. An OLD full report (c4) must not
    /// clear it; only the current c5 report does.
    #[test]
    fn restart_loads_current_desired_before_the_old_report() {
        let _guard = GHOST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "lqosd-non-suffix-restart-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("dataplane_transaction.json");
        let snapshot = dir.join("seam_state.json");
        let old = fence_tx(&["c1", "c3", "c4"]);
        let current = fence_tx(&["c1", "c3", "c4", "c5"]);
        write_fence_artifact(&artifact, &current);
        write_gate_snapshot(
            &snapshot,
            &non_suffix_capture(&["c2"]),
            &translate::content_key(&old),
        );

        let mut state = test_state(None);
        let old_report = analytics_ipc::StateReport {
            generation: old.generation.clone(),
            report_schema: 1,
            transaction: super::r3_tests::report_tx_bytes(&old),
            slot_order: super::r3_tests::slot_order_bytes(&["c1", "c3", "c4"]),
            content_key: translate::content_key(&old),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let (old_socket, old_feeder) = super::r3_tests::state_socket(
            &dir,
            analytics_ipc::StateReply::Report(old_report),
        );
        state.socket_path = Some(old_socket);
        restore_non_suffix_gate_from(&snapshot, &artifact, &mut state);

        assert!(state.pending_non_suffix.is_some(), "gate restored");
        assert_eq!(
            state.last_desired_world_key.as_deref(),
            Some(translate::content_key(&current).as_str()),
            "restart must read the current artifact before reconciling"
        );
        assert!(
            state.pending.is_none(),
            "restore holds intent only; no pending before a real attach"
        );
        assert!(state.non_suffix_replay_due, "the replay intent is held");

        // The old full report must not clear the gate.
        step(&artifact, &mut state);
        old_feeder.join().unwrap();
        assert!(
            state.pending_non_suffix.is_some(),
            "the old report must not clear the restored gate"
        );

        // The current c5 report (with a full slot table) clears.
        let current_report = analytics_ipc::StateReport {
            generation: current.generation.clone(),
            report_schema: 1,
            transaction: super::r3_tests::report_tx_bytes(&current),
            slot_order: super::r3_tests::slot_order_bytes(&["c1", "c3", "c4", "c5"]),
            content_key: translate::content_key(&current),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers: None,
        };
        let current_dir = dir.join("current");
        std::fs::create_dir_all(&current_dir).unwrap();
        let (current_socket, current_feeder) = super::r3_tests::state_socket(
            &current_dir,
            analytics_ipc::StateReply::Report(current_report),
        );
        state.socket_path = Some(current_socket);
        state.report_reread_due = true;
        state.report_reread_at = None;
        step(&artifact, &mut state);
        current_feeder.join().unwrap();
        assert!(state.pending_non_suffix.is_none(), "c5 report clears");
        assert!(state.terminal_fence.is_none());
        assert!(state.removal_capture.is_none());
        assert_eq!(
            state.applied.key(),
            Some(translate::content_key(&current).as_str())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `lqos-74t.102` lost-trigger fallback: a restored gate with no current
    /// artifact drops the broker gate (engine RemovedNotTail/SlotMismatch takes
    /// over) and the next structural ADD proceeds — no hang.
    #[test]
    fn restored_gate_without_artifact_falls_back_without_hanging() {
        let _guard = GHOST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "lqosd-non-suffix-lost-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("dataplane_transaction.json");
        let snapshot = dir.join("seam_state.json");
        write_gate_snapshot(
            &snapshot,
            &non_suffix_capture(&["c2"]),
            &translate::content_key(&fence_tx(&["c1", "c2", "c3"])),
        );

        let mut state = test_state(None);
        restore_non_suffix_gate_from(&snapshot, &artifact, &mut state);
        assert!(
            state.pending_non_suffix.is_none(),
            "no current artifact must drop the broker gate, not hang"
        );

        let desired = fence_tx(&["c1", "c3", "c4"]);
        write_fence_artifact(&artifact, &desired);
        step(&artifact, &mut state);
        assert!(
            state.pending.is_some(),
            "delivery resumes (engine fallback owns the recovery)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §2.4(13): an idle segment replacement while the gate is armed and no
    /// publish is pending must arm the 2c replay intent.
    #[test]
    fn idle_segment_replacement_arms_replay_when_gate_armed() {
        let _guard = GHOST_LOCK.lock().unwrap();
        create_test_segment(SEGMENT_NAME, [31u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        state.pending_non_suffix = Some(non_suffix_capture(&["c2"]));
        state.removal_capture = Some(non_suffix_capture(&["c2"]));
        state.liveness_polls = RETRY_ATTACH_EVERY - 1;

        // Bridge restart: same name, new inode; the idle liveness arm sees it.
        create_test_segment(SEGMENT_NAME, [32u8; 16]);
        step(&absent_artifact(), &mut state);
        assert!(state.segment.is_none(), "replacement drops the segment");
        assert!(
            state.non_suffix_replay_due,
            "§2.4(13) arms the replay intent"
        );
        assert!(state.pending_non_suffix.is_some(), "the gate remains");
        unlink_test_segment(SEGMENT_NAME);
    }

    /// §2.4(16): a queued trigger that loses its seam through `resolve_pending`
    /// must re-arm the replay, not leave the gate waiting forever.
    #[test]
    fn queued_lost_push_rearms_the_replay() {
        let _guard = GHOST_LOCK.lock().unwrap();
        create_full_test_segment(SEGMENT_NAME, [33u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        let desired = fence_tx(&["c1", "c3"]);
        let capture = non_suffix_capture(&["c2"]);
        state.pending_non_suffix = Some(capture.clone());
        state.removal_capture = Some(capture.clone());
        state.last_desired_world_key = Some(translate::content_key(&desired));
        let trigger = non_suffix_rebuild_trigger(&desired, &capture).expect("trigger");
        let frames = seam::encode_transaction(&trigger, state.last_desired_world_key.as_deref())
            .expect("encode");
        state.non_suffix_trigger_key = state.last_desired_world_key.clone();
        let pending = Pending {
            frames,
            pushed: 0,
            awaiting_ack: false,
            generation: trigger.generation.clone(),
            structural_key: translate::structural_identity(&desired),
            current_tx: trigger,
            seq_pos: 1,
            seq_len: 1,
            applied_after_current: desired,
            desired_key: None,
            attempt_retries: 0,
            ack_timeouts: 0,
        };
        state.segment = None;
        let resolved = resolve_pending(&mut state, pending);
        assert!(matches!(resolved, Resolve::Cleared));
        assert!(state.pending.is_none());
        assert!(state.pending_non_suffix.is_some(), "gate remains");
        assert!(
            state.non_suffix_replay_due,
            "the lost push re-arms the replay"
        );
        unlink_test_segment(SEGMENT_NAME);
    }

    /// §2.4(19)/(20): ACK_SILENCE after three real timeouts re-arms the gate.
    #[test]
    fn queued_lost_push_rearms_on_ack_silence() {
        let _guard = GHOST_LOCK.lock().unwrap();
        create_full_test_segment(SEGMENT_NAME, [35u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        let desired = fence_tx(&["c1", "c3"]);
        let capture = non_suffix_capture(&["c2"]);
        state.pending_non_suffix = Some(capture.clone());
        state.removal_capture = Some(capture.clone());
        state.last_desired_world_key = Some(translate::content_key(&desired));
        state.non_suffix_trigger_key = state.last_desired_world_key.clone();
        let trigger = non_suffix_rebuild_trigger(&desired, &capture).expect("trigger");
        let frames = seam::encode_transaction(&trigger, state.last_desired_world_key.as_deref())
            .expect("encode");
        let mut pending = Pending {
            frames,
            pushed: 0,
            awaiting_ack: false,
            generation: trigger.generation.clone(),
            structural_key: translate::structural_identity(&desired),
            current_tx: trigger,
            seq_pos: 1,
            seq_len: 1,
            applied_after_current: desired,
            desired_key: None,
            attempt_retries: 0,
            ack_timeouts: 0,
        };
        TEST_ACK_TIMEOUT_MS.with(|cell| cell.set(Some(1)));
        for _ in 0..(ACK_SILENCE_MAX_TIMEOUTS - 1) {
            match resolve_pending(&mut state, pending) {
                Resolve::Retained(next) => pending = *next,
                Resolve::Cleared => panic!("must retain before the ACK bound"),
            }
        }
        let resolved = resolve_pending(&mut state, pending);
        TEST_ACK_TIMEOUT_MS.with(|cell| cell.set(None));
        assert!(
            matches!(resolved, Resolve::Retained(_)),
            "ACK_SILENCE is not a bridge-reported fence: hold fail-closed, no terminal"
        );
        assert!(
            state.terminal_fence.is_none(),
            "ACK_SILENCE must not set a terminal fence"
        );
        assert!(state.pending_non_suffix.is_some(), "gate remains");
        assert!(
            state.non_suffix_replay_due,
            "ACK_SILENCE re-arms the replay"
        );
        unlink_test_segment(SEGMENT_NAME);
    }

    /// §2.4(20): a replaced segment detected on ack silence re-arms too.
    #[test]
    fn queued_lost_push_rearms_on_segment_replacement() {
        let _guard = GHOST_LOCK.lock().unwrap();
        create_full_test_segment(SEGMENT_NAME, [36u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        let desired = fence_tx(&["c1", "c3"]);
        let capture = non_suffix_capture(&["c2"]);
        state.pending_non_suffix = Some(capture.clone());
        state.removal_capture = Some(capture.clone());
        state.last_desired_world_key = Some(translate::content_key(&desired));
        state.non_suffix_trigger_key = state.last_desired_world_key.clone();
        let trigger = non_suffix_rebuild_trigger(&desired, &capture).expect("trigger");
        let frames = seam::encode_transaction(&trigger, state.last_desired_world_key.as_deref())
            .expect("encode");
        let pending = Pending {
            frames,
            pushed: 0,
            awaiting_ack: false,
            generation: trigger.generation.clone(),
            structural_key: translate::structural_identity(&desired),
            current_tx: trigger,
            seq_pos: 1,
            seq_len: 1,
            applied_after_current: desired,
            desired_key: None,
            attempt_retries: 0,
            ack_timeouts: 0,
        };
        // Bridge restart: same name, new inode.
        create_full_test_segment(SEGMENT_NAME, [37u8; 16]);
        TEST_ACK_TIMEOUT_MS.with(|cell| cell.set(Some(1)));
        let resolved = resolve_pending(&mut state, pending);
        TEST_ACK_TIMEOUT_MS.with(|cell| cell.set(None));
        assert!(matches!(resolved, Resolve::Cleared));
        assert!(state.segment.is_none(), "replacement drops the seam");
        assert!(state.pending_non_suffix.is_some(), "gate remains");
        assert!(
            state.non_suffix_replay_due,
            "segment replacement re-arms the replay"
        );
        unlink_test_segment(SEGMENT_NAME);
    }

    /// §2.4(19): terminal AttemptExhausted re-arms the gate as well.
    #[test]
    fn queued_lost_push_rearms_on_attempt_exhausted() {
        let _guard = GHOST_LOCK.lock().unwrap();
        create_full_test_segment(SEGMENT_NAME, [38u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        let desired = fence_tx(&["c1", "c3"]);
        let capture = non_suffix_capture(&["c2"]);
        state.pending_non_suffix = Some(capture.clone());
        state.removal_capture = Some(capture.clone());
        state.last_desired_world_key = Some(translate::content_key(&desired));
        state.non_suffix_trigger_key = state.last_desired_world_key.clone();
        let trigger = non_suffix_rebuild_trigger(&desired, &capture).expect("trigger");
        let frames = seam::encode_transaction(&trigger, state.last_desired_world_key.as_deref())
            .expect("encode");
        let pending = Pending {
            frames,
            pushed: 0,
            awaiting_ack: false,
            generation: trigger.generation.clone(),
            structural_key: translate::structural_identity(&desired),
            current_tx: trigger,
            seq_pos: 1,
            seq_len: 1,
            applied_after_current: desired,
            desired_key: None,
            attempt_retries: ATTEMPT_EXHAUSTED_MAX_RETRIES - 1,
            ack_timeouts: 0,
        };
        let ack = AckFrame::new(AckOutcome::AttemptExhausted, &pending.generation, 0);
        state
            .segment
            .as_mut()
            .expect("segment")
            .push_test_ack(&ack);
        let resolved = resolve_pending(&mut state, pending);
        assert!(matches!(resolved, Resolve::Cleared));
        assert!(state.pending_non_suffix.is_some(), "gate remains");
        assert!(
            state.non_suffix_replay_due,
            "AttemptExhausted re-arms the replay"
        );
        unlink_test_segment(SEGMENT_NAME);
    }

    /// §2.4(14): a c4 trigger already in flight when the desired becomes c5
    /// must re-arm with c5 after its ack; inspect the real queued frames via
    /// `as_ref`, never `take()` masking the in-flight state.
    #[test]
    fn inflight_c4_trigger_rearms_to_c5_after_ack() {
        let _guard = GHOST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("lqosd-c4-c5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("dataplane_transaction.json");
        let c4 = fence_tx(&["c1", "c2", "c3"]);
        let c5 = fence_tx(&["c1", "c3", "c4"]);
        let capture = non_suffix_capture(&["c2"]);
        write_fence_artifact(&artifact, &c5);

        create_full_test_segment(SEGMENT_NAME, [34u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        state.pending_non_suffix = Some(capture.clone());
        state.removal_capture = Some(capture.clone());
        state.last_desired_world_key = Some(translate::content_key(&c4));
        let trigger = non_suffix_rebuild_trigger(&c4, &capture).expect("trigger");
        let frames = seam::encode_transaction(&trigger, state.last_desired_world_key.as_deref())
            .expect("encode");
        state.non_suffix_trigger_key = state.last_desired_world_key.clone();
        state.pending = Some(Pending {
            frames,
            pushed: 0,
            awaiting_ack: false,
            generation: trigger.generation.clone(),
            structural_key: translate::structural_identity(&c4),
            current_tx: trigger.clone(),
            seq_pos: 1,
            seq_len: 1,
            applied_after_current: c4.clone(),
            desired_key: None,
            attempt_retries: 0,
            ack_timeouts: 0,
        });
        let ack = AckFrame::new(AckOutcome::RebuildRequired, &trigger.generation, 0);
        state
            .segment
            .as_mut()
            .expect("segment")
            .push_test_ack(&ack);

        step(&artifact, &mut state);
        let queued = state.pending.as_ref().expect("c5 replay queued");
        assert_eq!(
            queued.current_tx.generation,
            c5.generation,
            "the stale c4 trigger was replaced by a c5 trigger"
        );
        assert_eq!(
            state.non_suffix_trigger_key.as_deref(),
            Some(translate::content_key(&c5).as_str())
        );
        unlink_test_segment(SEGMENT_NAME);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §2.4(9): a restart restores INTENT, not a pending. The first no-segment
    /// step must not consume the trigger as SEAM_LOST; after a real control
    /// segment attaches, the remove-only trigger is replayed and its
    /// RebuildRequired ack follows the real `resolve_pending` path.
    #[test]
    fn restart_replays_only_after_a_real_attach_not_seam_lost() {
        let _guard = GHOST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "lqosd-restart-real-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("dataplane_transaction.json");
        let snapshot = dir.join("seam_state.json");
        let desired = fence_tx(&["c1", "c3"]);
        write_fence_artifact(&artifact, &desired);
        let capture = non_suffix_capture(&["c2"]);
        let snap = BridgeConvergenceSnapshot {
            pending_non_suffix: Some(capture.clone()),
            removal_capture: Some(capture.clone()),
            desired_world_content_key: Some(translate::content_key(&desired)),
            ..Default::default()
        };
        std::fs::write(&snapshot, serde_json::to_vec(&snap).unwrap()).unwrap();

        let mut state = test_state(None);
        restore_non_suffix_gate_from(&snapshot, &artifact, &mut state);
        assert!(state.pending_non_suffix.is_some(), "gate restored");
        assert!(state.pending.is_none(), "restore must not seed a pending");
        assert!(state.non_suffix_replay_due, "replay intent is held");

        // First step: no segment and attach_polls < RETRY_ATTACH_EVERY.
        step(&artifact, &mut state);
        assert!(state.pending.is_none(), "no no-segment pending was consumed");
        assert!(state.non_suffix_replay_due, "the trigger intent survives");
        assert!(state.pending_non_suffix.is_some(), "the gate survives");

        // A REAL control segment now appears. The next step attaches and
        // queues the replay; a real ack resolves it.
        create_full_test_segment(SEGMENT_NAME, [21u8; 16]);
        state.attach_polls = RETRY_ATTACH_EVERY - 1;
        step(&artifact, &mut state);
        assert!(state.segment.is_some(), "real segment attached");
        assert!(!state.non_suffix_replay_due, "replay intent consumed");
        let replayed = state.pending.as_ref().expect("replayed trigger queued");
        assert!(replayed.current_tx.leaf_ops.iter().any(
            |op| matches!(op, DataplaneLeafOp::Remove { circuit_id } if circuit_id == "c2")
        ));
        assert!(
            replayed
                .current_tx
                .leaf_ops
                .iter()
                .all(|op| !matches!(op, DataplaneLeafOp::Upsert { .. })),
            "replayed trigger carries no ADD"
        );
        let ack = AckFrame::new(
            AckOutcome::RebuildRequired,
            &replayed.generation,
            0,
        );
        state
            .segment
            .as_mut()
            .expect("segment")
            .push_test_ack(&ack);
        step(&artifact, &mut state);
        assert!(state.pending.is_none(), "ack resolved the replay");
        assert!(
            state.pending_non_suffix.is_some(),
            "RebuildRequired keeps the gate until the c5 report"
        );
        unlink_test_segment(SEGMENT_NAME);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §2.4(12): exercise the real ack/state path. An APPLIED ack for the
    /// remove-only trigger while the gate is armed is a partial-world
    /// acceptance: it must clear the pending but keep the gate and the old
    /// applied belief, and mark the report re-read due.
    #[test]
    fn unexpected_applied_ack_keeps_gate_and_old_applied_state() {
        let _guard = GHOST_LOCK.lock().unwrap();
        create_full_test_segment(SEGMENT_NAME, [22u8; 16]);
        let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach");
        let mut state = test_state(Some(segment));
        let desired = fence_tx(&["c1", "c3"]);
        let capture = non_suffix_capture(&["c2"]);
        state.pending_non_suffix = Some(capture.clone());
        state.removal_capture = Some(capture.clone());
        state.last_desired_world_key = Some(translate::content_key(&desired));
        let trigger = non_suffix_rebuild_trigger(&desired, &capture).expect("trigger");
        let frames = seam::encode_transaction(&trigger, state.last_desired_world_key.as_deref())
            .expect("encode");
        let before = state.applied.key().map(str::to_string);
        state.pending = Some(Pending {
            frames,
            pushed: 0,
            awaiting_ack: false,
            generation: trigger.generation.clone(),
            structural_key: translate::structural_identity(&desired),
            current_tx: trigger.clone(),
            seq_pos: 1,
            seq_len: 1,
            applied_after_current: desired.clone(),
            desired_key: None,
            attempt_retries: 0,
            ack_timeouts: 0,
        });
        let ack = AckFrame::new(AckOutcome::Applied, &trigger.generation, 0);
        state
            .segment
            .as_mut()
            .expect("segment")
            .push_test_ack(&ack);
        let pending = state.pending.take().expect("pending");
        let resolved = resolve_pending(&mut state, pending);
        assert!(matches!(resolved, Resolve::Cleared));
        assert!(state.pending.is_none(), "the suspect ack clears the pending");
        assert!(
            state.pending_non_suffix.is_some(),
            "APPLIED must not clear the R3 gate"
        );
        assert_eq!(
            state.applied.key().map(str::to_string),
            before,
            "APPLIED must not advance the applied belief"
        );
        assert!(state.report_reread_due, "the report re-read is armed");
        assert!(
            state.non_suffix_replay_due,
            "§2.4(17) re-arms the trigger, not only the report read"
        );
        // Prove the recovery actually queues a fresh attempt for the current
        // desired, rather than only retaining the gate.
        let dir = std::env::temp_dir().join(format!("lqosd-applied-rearm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("dataplane_transaction.json");
        write_fence_artifact(&artifact, &desired);
        step(&artifact, &mut state);
        assert!(
            state.pending.is_some(),
            "the re-armed trigger is queued after the APPLIED wait hazard"
        );
        assert!(
            state.non_suffix_trigger_key.as_deref()
                == Some(translate::content_key(&desired).as_str())
        );
        unlink_test_segment(SEGMENT_NAME);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod posture_surface_tests {
    use super::*;

 /// NON-VACUOUS (#4, `posture:log_posture` actually EMITS
    /// the `RESOLVED_POSTURE` event carrying the resolved fields — a
    /// commit-message claim is not a surface. A regression that drops the log
    /// FAILS here.
    #[test]
    fn resolved_posture_is_emitted_as_a_logged_event() {
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct CaptureWriter(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for CaptureWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
            type Writer = CaptureWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(CaptureWriter(buf.clone()))
            .with_ansi(false)
            .finish();
        let posture = posture::resolve_posture(posture::PostureInput {
            queue_mode_observe: false,
            dataplane_bridge_present: false,
        });
        tracing::subscriber::with_default(subscriber, || posture::log_posture(&posture));
        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap_or_default();
        assert!(
            out.contains("RESOLVED_POSTURE"),
            "the event must be emitted: {out}"
        );
        assert!(
            out.contains("baseline_max_circuits="),
            "the resolved fields must be emitted: {out}"
        );
        assert!(
            out.contains("side=\"lqosd\""),
            "the side must be emitted: {out}"
        );
    }
}

#[cfg(test)]
mod terminal_fence_tests {
    use super::*;

    fn world(circuits: &[&str], rate: u32) -> DataplaneTransaction {
        use lqos_dataplane_types::{DataplaneLeafAqm, DataplaneRatePair};
        DataplaneTransaction {
            generation: format!("gen-{rate}"),
            inner_ops: vec![],
            leaf_ops: circuits
                .iter()
                .map(|id| DataplaneLeafOp::Upsert {
                    circuit_id: (*id).to_string(),
                    parent_node_id: "root".to_string(),
                    download: DataplaneRatePair {
                        commit_kbps: rate,
                        ceil_kbps: rate.saturating_mul(2),
                    },
                    upload: DataplaneRatePair {
                        commit_kbps: rate,
                        ceil_kbps: rate.saturating_mul(2),
                    },
                    aqm: DataplaneLeafAqm::None,
                    admin_up: true,
                })
                .collect(),
            mapping_ops: vec![],
            lpm_delta: vec![],
            steering_vlans: vec![],
            schema_version: None,
            apply_mode: Default::default(),
            circuit_capacity: None,
        }
    }

    /// WP2 STORM TEST: the TERMINAL fence keys on the
    /// STRUCTURAL identity — the same world is HELD, a legitimate STRUCTURAL
    /// change is retried EXACTLY ONCE, and a RATE-ONLY churn is NOT retried
    /// (no storm). A regression to the shaping generation or the rate-inclusive
    /// content key FAILS here.
    #[test]
    fn the_terminal_fence_is_structural_and_does_not_storm() {
        let base = world(&["c1"], 10_000);
        let fence = translate::structural_identity(&base);

        // (i) the SAME world (even a new shape-generation) => HELD (no re-emit).
        assert!(
            terminal_fenced(Some(&fence), &translate::structural_identity(&base)),
            "the same world must stay fenced"
        );

        // (ii) a legitimate STRUCTURAL change (a new circuit) => NOT fenced
        //      (retried exactly once).
        let structural = world(&["c1", "c2"], 10_000);
        assert_ne!(
            translate::structural_identity(&structural),
            fence,
            "a new circuit changes the structural identity"
        );
        assert!(
            !terminal_fenced(Some(&fence), &translate::structural_identity(&structural)),
            "a structural change must not be fenced"
        );

        // (iii) a RATE-ONLY churn => STILL fenced (no storm).
        let rate_only = world(&["c1"], 99_999);
        assert_eq!(
            translate::structural_identity(&rate_only),
            fence,
            "the structural identity ignores rates"
        );
        assert!(
            terminal_fenced(Some(&fence), &translate::structural_identity(&rate_only)),
            "a rate-only churn must stay fenced (no storm)"
        );
    }
}

#[cfg(test)]
mod wp5_seam_state_tests {
    /// WP5: the reconciliation snapshot SERIALIZES the WP5 fields,
    /// so the persisted `seam_state.json` + `lqosd --print-seam-state` are real
    /// (level-independent; the `info!` emit is not journal-visible at the default).
    #[test]
    fn seam_state_snapshot_serializes_the_wp5_fields() {
        let snap = super::BridgeConvergenceSnapshot::default();
        let json = serde_json::to_string(&snap).expect("the snapshot serializes");
        for f in [
            "desired_generation",
            "pending",
            "last_reject",
            "applied_generation",
            "applied_provenance",
            "state",
            "written_at_unix",
            "desired_world_content_key",
            "removal_capture",
            "pending_non_suffix",
        ] {
            assert!(json.contains(f), "the snapshot JSON must carry `{f}` in: {json}");
        }
    }

    /// `lqos-74t.54` (b): the structural-remove capture round-trips through the
    /// snapshot so a later readout answers suffix-ness from `seam_state.json`.
    #[test]
    fn seam_state_carries_the_structural_remove_capture() {
        let snap = super::BridgeConvergenceSnapshot {
            removal_capture: Some(super::RemovalCapture {
                removed: vec!["c2".to_string()],
                slot_table_len: 3,
                slot_order_available: true,
                suffix: false,
                non_suffix: vec!["c2".to_string()],
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&snap).expect("serializes");
        let parsed: super::BridgeConvergenceSnapshot =
            serde_json::from_str(&json).expect("parses");
        let capture = parsed.removal_capture.expect("the capture survives");
        assert_eq!(capture.removed, vec!["c2".to_string()]);
        assert_eq!(capture.slot_table_len, 3);
        assert!(capture.slot_order_available);
        assert!(!capture.suffix);
        assert_eq!(capture.non_suffix, vec!["c2".to_string()]);
    }

    /// Card (`docs/seam_state_truthfulness_card.md`): `written_at_unix` is
    /// ADDITIVE — a snapshot persisted before the field existed still parses
    /// (defaults to 0).
    #[test]
    fn written_at_unix_is_additive() {
        let mut value =
            serde_json::to_value(super::BridgeConvergenceSnapshot::default()).expect("serializes");
        value
            .as_object_mut()
            .expect("object")
            .remove("written_at_unix");
        let parsed: super::BridgeConvergenceSnapshot =
            serde_json::from_value(value).expect("an older snapshot must parse");
        assert_eq!(parsed.written_at_unix, 0);
    }

    /// Card: the write is ATOMIC (temp + rename) — a concurrent reader NEVER
    /// observes a torn/partial snapshot.
    #[test]
    fn the_atomic_snapshot_write_never_exposes_partial_json() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = std::env::temp_dir().join(format!("seam-state-atomic-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("seam_state.json");
        let seed = super::BridgeConvergenceSnapshot::default();
        super::write_snapshot_atomic(&path, &serde_json::to_string(&seed).expect("serializes"));

        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let path = path.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let snap = super::BridgeConvergenceSnapshot {
                        written_at_unix: 1,
                        ..Default::default()
                    };
                    let json = serde_json::to_string(&snap).expect("serializes");
                    super::write_snapshot_atomic(&path, &json);
                    std::thread::yield_now();
                }
            })
        };
        for _ in 0..20_000 {
            let raw = std::fs::read_to_string(&path).expect("the snapshot is always readable");
            serde_json::from_str::<super::BridgeConvergenceSnapshot>(&raw)
                .expect("a reader must never see a torn/partial snapshot");
        }
        stop.store(true, Ordering::Relaxed);
        writer.join().expect("writer joins");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Card: `written_at_unix >= last_applied_unix` (the file is only as fresh as
    /// the last tick; it can never predate the applied world it describes).
    #[test]
    fn written_at_unix_is_at_least_last_applied_unix() {
        let now = super::now_unix();
        let snap = super::BridgeConvergenceSnapshot {
            last_applied_unix: Some(now.saturating_sub(5)),
            written_at_unix: now,
            ..Default::default()
        };
        assert!(
            snap.written_at_unix >= snap.last_applied_unix.expect("set"),
            "the write stamp must not predate the applied world"
        );
    }

}

#[cfg(test)]
mod r3s3b_routing_tests {
    use super::*;

    /// F3-B §3b (required 4): `REBUILD_REQUIRED` must NOT quarantine
    /// (`held_ops_from_tx` + `HELD_RETRY`) or set `terminal_fence` — the bridge
    /// cold-rebuilds and the broker waits for the post-rebuild report. A
    /// mechanics `REJECTED_STRUCTURAL_OP` still quarantines; every other typed
    /// reject fences. Also pins the wire code → `PublishOutcome` mapping.
    #[test]
    fn rebuild_required_does_not_quarantine_or_fence() {
        let rebuild = super::outcome_disposition(ack_map::PublishOutcome::RebuildRequired);
        assert!(!rebuild.quarantine, "REBUILD_REQUIRED must not quarantine");
        assert!(!rebuild.fence, "REBUILD_REQUIRED must not fence");
        assert!(!rebuild.reject, "REBUILD_REQUIRED is not a reject");

        let structural =
            super::outcome_disposition(ack_map::PublishOutcome::Rejected("REJECTED_STRUCTURAL_OP"));
        assert!(structural.quarantine, "a structural reject quarantines");
        assert!(!structural.fence, "a structural reject does not fence");
        assert!(structural.reject);

        let mechanics =
            super::outcome_disposition(ack_map::PublishOutcome::Rejected("REJECTED_LPM_PREPARE"));
        assert!(!mechanics.quarantine);
        assert!(mechanics.fence, "a mechanics reject fences");
        assert!(mechanics.reject);

        // The wire code (11) maps to the distinct broker outcome (never
        // UNKNOWN_OUTCOME).
        assert_eq!(
            ack_map::classify(lqos_dataplane_types::wire::AckOutcome::RebuildRequired.code()),
            ack_map::PublishOutcome::RebuildRequired
        );
    }
}

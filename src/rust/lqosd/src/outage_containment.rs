//! Circuit-outage containment alarms (WO C2): ONE cold-path surface that
//! catches a HELD circuit in seconds.
//!
//! The 2026-09-30 incident (a site re-parent reject) halted a customer's
//! traffic (`rx>0 / tx≈0`) for hours and was found by a complaint, not an
//! alarm. This monitor is the missing visibility:
//!
//! - **Black-hole tripwire:** a per-circuit `enqueued > 0 && sent == 0`
//!   sustained for [`BLACKHOLE_SECONDS`] raises an operator-visible Urgent
//!   Issue naming the circuit (cleared when the circuit sends again).
//! - **Churn tripwire:** the broker's control-plane events (applied/rejected
//!   generations) at or above [`CHURN_APPLIES_PER_WINDOW`] /
//!   [`CHURN_REJECTS_PER_WINDOW`] in [`CHURN_WINDOW`] raises an Urgent Issue.
//!
//! Both are COLD-PATH reads of existing counters/stats (no per-packet cost,
//! no new hot-path state). Named predecessor: the `CLASS-COVER` /
//! `flow_diag_tick` cold-cadence tripwire pattern; this adds no coordination
//! mechanism.
//!
//! Thresholds are the DRIVER's (§9 of the work order); the defaults here are
//! deliberately conservative.

use crate::bridge_analytics;
use crate::bridge_broker;
use crate::urgent;
use lqos_bus::{UrgentSeverity, UrgentSource};
use lqos_utils::units::DownUpOrder;
use tracing::{info, warn};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How often the cold-path alarm scans.
const SCAN_INTERVAL: Duration = Duration::from_secs(2);
/// A circuit must be enqueueing-but-not-sending for this long to alarm
/// (DRIVER-owned, §9).
const BLACKHOLE_SECONDS: u64 = 5;
/// D2 (driver ruling 2026-11-18, `lqos-74t.197`): delivery below this rate
/// does NOT count as resumed. It is in the same per-circuit rate unit as
/// [`BLACKHOLE_MIN_ENQUEUED_BPS`] (`shaped_bytes_per_second`, both directions
/// combined). The jackrabbit incident cleared 18 of its 29 alerts on a 41–236
/// trickle.
const BLACKHOLE_CLEAR_MIN_DELIVERED_BPS: u64 = 1_000;
/// D1 (driver ruling 2026-11-18): above-floor delivery must be CONTINUOUS
/// for this long before an alarm clears. A sub-floor scan restarts the run.
const BLACKHOLE_CLEAR_SUSTAIN: Duration = Duration::from_secs(30);
/// GUARD (card §4, the LIVE over-fire): if this many circuits are
/// enqueued/delivered==0 in ONE scan it is a GLOBAL sampling window, not N
/// holds — suppress (a true hold is isolated).
const BLACKHOLE_MAX_SIMULTANEOUS: usize = 3;
/// `lqos-74t.191` P0.3: a single circuit below this enqueued rate (bits/s,
/// both directions combined) is the known low-rate artifact class. The floor
/// applies ONLY to ISOLATED circuit alerts — a worker cluster is never
/// floor-filtered (the field 6544 alert was 729 bps).
const BLACKHOLE_MIN_ENQUEUED_BPS: u64 = 10_000;
/// `lqos-74t.191` P0.3: this many zero-delivery circuits on ONE applied owner
/// is a WORKER-level starvation, not a global sampling window.
const BLACKHOLE_WORKER_MIN: usize = 3;
/// `lqos-74t.191` MUST 3: owner-localization guard. A zero-delivery set that
/// spans this many DISTINCT owners is a fleet/global sampling window (a
/// partial startup or a shared cold window), not N worker-local holds; the
/// worker-cluster path must then stay quiet. Pre-data rule (v4 review §2
/// MUST 3); do not tune it after observing a run.
const BLACKHOLE_GLOBAL_OWNER_SPREAD: usize = 3;
/// The rolling window over which zero-delivery observations are CLUSTERED per
/// owner (card v4: 60 s). Staggered starts inside it form one episode; a
/// single 2 s scan is never enough on its own.
const BLACKHOLE_WORKER_WINDOW: Duration = Duration::from_secs(60);
/// `lqos-74t.191` P0.3: the analytics snapshot must be continuously HEALTHY
/// for this long before a worker alert fires (suppresses the 21:25 startup
/// window; the 7 s/25 s samples never qualify).
const HEALTH_HOLD: Duration = Duration::from_secs(30);
/// A snapshot whose `captured_at_millis` is older than this is not fresh.
const HEALTH_CAPTURE_FRESH: Duration = Duration::from_secs(10);
/// The churn measurement window (DRIVER-owned, §9).
const CHURN_WINDOW: Duration = Duration::from_secs(60);
/// Applied generations within the window that trip the churn alarm.
const CHURN_APPLIES_PER_WINDOW: u64 = 6;
/// Rejected publishes within the window that trip the churn alarm.
const CHURN_REJECTS_PER_WINDOW: u64 = 6;

static STARTED: OnceLock<()> = OnceLock::new();

/// Starts the outage-containment monitor (idempotent; named thread per the
/// lqosd actor convention). Reads existing cold-path stats only.
pub fn start_outage_containment() {
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("outage-containment".into())
        .spawn(monitor_loop)
        .map(|_| ())
        .unwrap_or_else(|err| {
            tracing::error!("Failed to spawn the outage-containment monitor: {err:?}");
            urgent::submit(
                UrgentSource::System,
                UrgentSeverity::Warning,
                "OUTAGE_CONTAINMENT_NOT_SPAWNED".to_string(),
                format!(
                    "The circuit-outage containment monitor failed to spawn ({err:?}) — a \
                     held circuit will not raise an alarm until lqosd restarts."
                ),
                None,
                Some("outage-containment:spawn".to_string()),
            );
        });
}

fn monitor_loop() {
    let mut state = ContainmentState::default();
    loop {
        let now = Instant::now();
        scan_blackholes(&mut state, now);
        scan_churn(&mut state, now);
        std::thread::sleep(SCAN_INTERVAL);
    }
}

#[derive(Default)]
struct ContainmentState {
    /// circuit id -> first scan where it was enqueueing-but-not-sending.
    blackhole_first_seen: HashMap<String, Instant>,
    /// circuit ids currently alarmed (for heal-clearing) -> their episode.
    blackhole_alarmed: BTreeMap<String, BlackholeEpisode>,
    /// `lqos-74t.191` P0.3: circuit id -> (applied owner slot, last scan it was
    /// observed zero-delivery). The rolling 60 s cluster input; only owned
    /// circuits are inserted. MUST 4 (v4 review): clustering additionally
    /// requires the circuit in the scan's CURRENT sustained set, so a
    /// recovered circuit stops contributing immediately (the 60 s window only
    /// bounds the episode for circuits that are still zero-delivery).
    zero_delivery_owner: HashMap<String, (u16, Instant)>,
    /// Owners with a live WORKER_STARVATION item (one per episode).
    worker_starvation_alarmed: BTreeSet<u16>,
    /// Start of the current continuously-healthy analytics period.
    healthy_since: Option<Instant>,
    /// `(sampled_at, applied_total, rejected_total)` window.
    churn_samples: VecDeque<(Instant, u64, u64)>,
    churn_alarmed: bool,
    /// GUARD (c) latch: a sustained global window must warn ONCE, not every scan.
    blackhole_suppressed: bool,
    /// GUARD (a) latch: QueueStats unavailable ⇒ guard (a) filters everything ⇒
    /// the detector is SILENTLY DISABLED. Warn ONCE so it is not mistaken for "no
    /// blackholes"; clears when QueueStats recovers.
    blackhole_queue_degraded: bool,
}

/// An alarmed circuit's episode: when it was raised, and the start of its
/// current CONTINUOUS above-floor delivery run (D1 + D2).
struct BlackholeEpisode {
    raised_at: Instant,
    raised_at_unix: u64,
    recovering_since: Option<Instant>,
}

/// D1 + D2 (driver ruling, `lqos-74t.197`): one scan of an alarmed circuit's
/// heal test.
///
/// - Delivery counts as resumed only at or above
///   [`BLACKHOLE_CLEAR_MIN_DELIVERED_BPS`]. A sub-floor trickle (the field's
///   41–236 bps) or a zero scan resets the run.
/// - The alarm clears once the run has lasted [`BLACKHOLE_CLEAR_SUSTAIN`].
///
/// Returns the new run start and whether to clear.
fn blackhole_recovery_step(
    recovering_since: Option<Instant>,
    delivered_bps: u64,
    now: Instant,
) -> (Option<Instant>, bool) {
    if delivered_bps < BLACKHOLE_CLEAR_MIN_DELIVERED_BPS {
        return (None, false);
    }
    let since = recovering_since.unwrap_or(now);
    (
        Some(since),
        now.saturating_duration_since(since) >= BLACKHOLE_CLEAR_SUSTAIN,
    )
}

/// Retention companion (`lqos-74t.197`): a healed blackhole does NOT vanish.
/// It is kept as a `CIRCUIT_BLACKHOLE_RECOVERED` record with its raise and
/// recovery times, so an operator who opens ALERTS after the heal still sees
/// it. It goes on acknowledge, or with the 24 h TTL.
fn record_blackhole_recovered(
    circuit_id: &str,
    raised_at_unix: u64,
    duration_s: u64,
    delivered_bps: u64,
) {
    let recovered_at_unix = lqos_utils::unix_time::unix_now().unwrap_or_default();
    urgent::mark_recovered_by_identity(
        "CIRCUIT_BLACKHOLE",
        &format!("circuit-blackhole:{circuit_id}"),
        format!(
            "RECOVERED: circuit '{circuit_id}' had traffic ENQUEUED but nothing delivered \
             (alarmed for {duration_s}s); delivery resumed at >= {BLACKHOLE_CLEAR_MIN_DELIVERED_BPS} \
             for {}s. Kept for review: check the bridge log around the raise time for a stuck \
             move/drain or a refused admission. Acknowledge to dismiss.",
            BLACKHOLE_CLEAR_SUSTAIN.as_secs()
        ),
        Some(format!(
            "raised_at_unix={raised_at_unix} recovered_at_unix={recovered_at_unix} \
             duration_s={duration_s} delivered_bps={delivered_bps}"
        )),
    );
}

/// One sustained zero-delivery observation (the shape ADMISSION/worker
/// starvation produces). Collected BEFORE the queue-depth gate: an empty own
/// queue is the field's `enqueued > 0, queue empty` symptom, not a reason to
/// hide the circuit.
struct ZeroDelivery {
    id: String,
    /// Applied placement worker (numeric slot -> owner); `None` when the
    /// broker has no consistent owner table yet.
    owner: Option<u16>,
    enqueued_bps: u64,
    /// `Some(true)` queue non-empty, `Some(false)` queue empty, `None` stats
    /// unavailable.
    queue_nonempty: Option<bool>,
}

/// TRUE when a circuit is enqueueing traffic but delivering none: the
/// incident's `rx>0 / tx≈0` shape (a held/parked circuit).
fn circuit_is_blackholed(enqueued: DownUpOrder<u64>, sent: DownUpOrder<u64>) -> bool {
    (enqueued.down > 0 || enqueued.up > 0) && sent.down == 0 && sent.up == 0
}

/// TRUE when an ISOLATED circuit alert is worth raising: the incident shape
/// plus enough traffic to clear the low-rate artifact floor. Worker-cluster
/// candidates are never passed through this floor.
fn circuit_is_reportable(enqueued: DownUpOrder<u64>, sent: DownUpOrder<u64>) -> bool {
    circuit_is_blackholed(enqueued, sent)
        && enqueued.down.saturating_add(enqueued.up) >= BLACKHOLE_MIN_ENQUEUED_BPS
}

/// TRUE when either churn arm is over its per-window bound.
fn churn_window_exceeds(applied_delta: u64, rejected_delta: u64) -> bool {
    rejected_delta >= CHURN_REJECTS_PER_WINDOW || applied_delta >= CHURN_APPLIES_PER_WINDOW
}

/// `lqos-74t.191` P0.3 health gate: is the analytics snapshot trustworthy
/// RIGHT NOW? (It must hold for [`HEALTH_HOLD`] before a worker alert.)
///
/// - not stale (the poll succeeded);
/// - the snapshot was RECEIVED recently on lqosd's monotonic clock
///   ([`bridge_analytics::BridgeStatsSnapshot::received_at`]);
/// - circuit coverage is non-empty (`by_circuit_id` carries rows);
/// - the global actual/shaped series is not all-zero (a pre-first-poll or
///   dead-class sample).
///
/// MUST 1 (v4 review): freshness MUST NOT compare the bridge's
/// `captured_at_millis` with wall-clock time — that field is boot-relative
/// raw-TSC milliseconds (`stats.rs`), so on a running box it is ~1.7e12 ms
/// behind Unix epoch and the gate could never pass. `received_at` is stamped
/// by lqosd at the successful-poll swap, one domain with `now`.
///
/// NOTE (driver call 2026-11-12): the v4 card listed
/// `last_good_circuit_ids` non-empty here, but that set means "rows CARRIED
/// from a previous poll after a counter reset/missing host row" and is
/// normally EMPTY on a healthy box. The driver approved `by_circuit_id`
/// non-empty as the coverage term; using the literal field would suppress
/// every worker alert. Recorded in the implementation report.
fn snapshot_is_healthy(
    snapshot: &bridge_analytics::BridgeStatsSnapshot,
    now: Instant,
) -> bool {
    let capture_fresh = snapshot
        .received_at
        .is_some_and(|received| now.saturating_duration_since(received) <= HEALTH_CAPTURE_FRESH);
    let coverage = !snapshot.by_circuit_id.is_empty();
    let global_nonzero = snapshot
        .bridge_actual_bytes_per_second
        .down
        .saturating_add(snapshot.bridge_actual_bytes_per_second.up)
        .saturating_add(snapshot.bridge_shaped_bytes_per_second.down)
        .saturating_add(snapshot.bridge_shaped_bytes_per_second.up)
        > 0;
    !snapshot.stale && capture_fresh && coverage && global_nonzero
}

/// MUST 3 (v4 review): TRUE when the CURRENT zero-delivery observations span
/// enough distinct applied owners to be a fleet-wide/global sampling window.
/// A genuine worker-local hold cannot implicate many workers at once; a
/// partial startup/global window can, and must not raise per-owner alerts.
fn zero_delivery_is_fleet_wide(current: &[ZeroDelivery]) -> bool {
    let owners: BTreeSet<u16> = current.iter().filter_map(|c| c.owner).collect();
    owners.len() >= BLACKHOLE_GLOBAL_OWNER_SPREAD
}

/// Groups the rolling zero-delivery observations by applied owner over
/// `window`, keeping only owners with at least [`BLACKHOLE_WORKER_MIN`]
/// DISTINCT circuits. Deterministic (sorted ids). Circuits are never
/// floor-filtered here.
///
/// MUST 4 (v4 review): only circuits in `current` (the scan's LIVE
/// zero-delivery set) contribute. A recovered circuit keeps its
/// `blackhole_first_seen` episode time but must stop counting as a cluster
/// candidate immediately — otherwise three recovered circuits can keep (or
/// raise) a worker alert for the rest of the 60 s window with stale ids.
fn worker_clusters(
    seen: &HashMap<String, (u16, Instant)>,
    current: &BTreeSet<String>,
    now: Instant,
    window: Duration,
) -> BTreeMap<u16, Vec<String>> {
    let mut by_owner: BTreeMap<u16, BTreeSet<String>> = BTreeMap::new();
    for (id, (owner, last_seen)) in seen {
        if current.contains(id) && now.saturating_duration_since(*last_seen) <= window {
            by_owner.entry(*owner).or_default().insert(id.clone());
        }
    }
    by_owner.retain(|_, ids| ids.len() >= BLACKHOLE_WORKER_MIN);
    by_owner
        .into_iter()
        .map(|(owner, ids)| (owner, ids.into_iter().collect()))
        .collect()
}

/// One alert per worker per episode: raise the owners with an ACTIVE cluster
/// that are not already alarmed (only while the analytics health gate holds);
/// clear the alarmed owners whose cluster is gone.
fn worker_alert_changes(
    clusters: &BTreeMap<u16, Vec<String>>,
    alarmed: &BTreeSet<u16>,
    health_ok: bool,
) -> (Vec<u16>, Vec<u16>) {
    let present: BTreeSet<u16> = clusters.keys().copied().collect();
    let raise: Vec<u16> = if health_ok {
        present.difference(alarmed).copied().collect()
    } else {
        Vec::new()
    };
    let clear: Vec<u16> = alarmed.difference(&present).copied().collect();
    (raise, clear)
}

fn scan_blackholes(state: &mut ContainmentState, now: Instant) {
    let snapshot = bridge_analytics::snapshot();
    scan_blackholes_with(state, now, &snapshot, |circuit_id| {
        bridge_analytics::queue_stats_for_circuit(circuit_id)
            .map(|q| q.download.queue_depth_pkts != 0 || q.upload.queue_depth_pkts != 0)
    });
}

/// [`scan_blackholes`] on an explicit snapshot and queue-depth source. It
/// exists so the archived-incident replay drives the REAL scan path.
/// `queue_nonempty_of` returns `None` when QueueStats is unavailable.
fn scan_blackholes_with(
    state: &mut ContainmentState,
    now: Instant,
    snapshot: &bridge_analytics::BridgeStatsSnapshot,
    queue_nonempty_of: impl Fn(&str) -> Option<bool>,
) {
    // P0.3 health gate (see `snapshot_is_healthy`): track a continuous
    // healthy period; any unhealthy sample resets it.
    if snapshot_is_healthy(snapshot, now) {
        state.healthy_since.get_or_insert(now);
    } else {
        state.healthy_since = None;
    }
    let health_ok = state
        .healthy_since
        .is_some_and(|since| now.saturating_duration_since(since) >= HEALTH_HOLD);

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut sustained: Vec<ZeroDelivery> = Vec::new();
    let mut queue_stats_available = false;
    let mut queue_stats_unavailable = false;
    for (circuit_id, row) in snapshot.by_circuit_id.iter() {
        if !circuit_is_blackholed(row.bytes_per_second, row.shaped_bytes_per_second) {
            continue;
        }
        seen.insert(circuit_id.clone());
        let first = state
            .blackhole_first_seen
            .entry(circuit_id.clone())
            .or_insert(now);
        if now.duration_since(*first) < Duration::from_secs(BLACKHOLE_SECONDS) {
            continue;
        }
        // P0.3: collect the SUSTAINED candidate BEFORE the queue-depth gate.
        // The field's 22:15 5033 case was enqueued>0 with an EMPTY queue; the
        // old gate dropped exactly the shape a worker alert exists for.
        let queue_nonempty = match queue_nonempty_of(circuit_id.as_str()) {
            Some(nonempty) => {
                queue_stats_available = true;
                Some(nonempty)
            }
            None => {
                // QueueStats unavailable (actor inactive / socket / connect):
                // do not raise an unverifiable ISOLATED alert, but the
                // sustained observation still feeds worker clustering.
                queue_stats_unavailable = true;
                None
            }
        };
        // P0.1 identity: `bridge_circuit_id` IS the numeric slot, and the
        // broker keeps the applied slot-indexed owner vector. Never a name
        // lookup, never `checked_owner_of(service_id)`.
        let owner = if row.bridge_circuit_id == 0 {
            None
        } else {
            bridge_broker::applied_owner_of_slot(row.bridge_circuit_id)
        };
        sustained.push(ZeroDelivery {
            id: circuit_id.clone(),
            owner,
            enqueued_bps: row
                .bytes_per_second
                .down
                .saturating_add(row.bytes_per_second.up),
            queue_nonempty,
        });
    }
    // DEGRADED latch (§4): if QueueStats is entirely unavailable, guard (a)
    // filters EVERY isolated candidate ⇒ the detector is SILENTLY DISABLED —
    // warn ONCE so it is not mistaken for "no blackholes"; clear when it
    // recovers. (Worker clusters do not depend on QueueStats.)
    if queue_stats_unavailable && !queue_stats_available {
        if !state.blackhole_queue_degraded {
            state.blackhole_queue_degraded = true;
            warn!(
                "CIRCUIT_BLACKHOLE detector DEGRADED: QueueStats unavailable — guard (a) cannot \
                 verify a circuit's own queue, so ISOLATED alerts are QUIET until it recovers \
                 (this is NOT 'no blackholes'); worker-starvation clustering is unaffected"
            );
        }
    } else {
        state.blackhole_queue_degraded = false;
    }

    // Roll the per-owner observation window and cluster (P0.3). MUST 3: the
    // CURRENT set's owner spread decides whether this is a fleet/global
    // window; MUST 4: only the CURRENT zero-delivery ids cluster.
    for candidate in &sustained {
        if let Some(owner) = candidate.owner {
            state
                .zero_delivery_owner
                .insert(candidate.id.clone(), (owner, now));
        }
    }
    state
        .zero_delivery_owner
        .retain(|_, (_, last_seen)| now.saturating_duration_since(*last_seen) <= BLACKHOLE_WORKER_WINDOW);
    let fleet_wide = zero_delivery_is_fleet_wide(&sustained);
    let sustained_ids: BTreeSet<String> = sustained.iter().map(|c| c.id.clone()).collect();
    let clusters = worker_clusters(
        &state.zero_delivery_owner,
        &sustained_ids,
        now,
        BLACKHOLE_WORKER_WINDOW,
    );

    // WORKER-LEVEL STARVATION: one item per owner per episode, health-gated
    // AND owner-localized (MUST 3). A fleet-wide window suppresses new raises
    // but does not clear a real item mid-episode; a genuine recovery still
    // clears it (the owner is absent from `clusters`).
    let (raise, clear) = worker_alert_changes(
        &clusters,
        &state.worker_starvation_alarmed,
        health_ok && !fleet_wide,
    );
    {
        let by_id: HashMap<&str, &ZeroDelivery> =
            sustained.iter().map(|c| (c.id.as_str(), c)).collect();
        for worker in raise {
            let Some(ids) = clusters.get(&worker) else {
                continue;
            };
            let circuits = ids.len();
            let total_enqueued: u64 = ids
                .iter()
                .filter_map(|id| by_id.get(id.as_str()))
                .map(|c| c.enqueued_bps)
                .sum();
            let queue_empty = ids
                .iter()
                .filter(|id| {
                    by_id
                        .get(id.as_str())
                        .is_some_and(|c| c.queue_nonempty == Some(false))
                })
                .count();
            let duration_s = ids
                .iter()
                .filter_map(|id| state.blackhole_first_seen.get(id))
                .min()
                .map(|first| now.saturating_duration_since(*first).as_secs())
                .unwrap_or(0);
            let top: Vec<&str> = ids.iter().take(5).map(String::as_str).collect();
            if state.worker_starvation_alarmed.insert(worker) {
                warn!(
                    worker,
                    circuits,
                    queue_empty,
                    enqueued_bps = total_enqueued,
                    duration_s,
                    top = ?top,
                    "WORKER_STARVATION raised: {circuits} circuits on worker {worker} have \
                     ENQUEUED > 0 but delivered nothing for >= {BLACKHOLE_SECONDS}s \
                     ({queue_empty} of {circuits} have an EMPTY own queue — enqueued > 0, queue \
                     empty). This is a WORKER-level starvation, not a global sampling window."
                );
                urgent::submit(
                    UrgentSource::System,
                    UrgentSeverity::Error,
                    "WORKER_STARVATION".to_string(),
                    format!(
                        "Worker {worker} has {circuits} circuits with traffic ENQUEUED but nothing \
                         delivered for at least {BLACKHOLE_SECONDS}s (symptom: enqueued > 0, queue \
                         empty on {queue_empty} of them; total enqueued {total_enqueued} bps). This \
                         is a WORKER-level starvation, not a global sampling window. Check the \
                         bridge log for a stuck move/drain or a refused admission on this worker. \
                         Top circuits: {top:?}."
                    ),
                    Some(format!(
                        "worker={worker} circuits={circuits} queue_empty={queue_empty} \
                         enqueued_bps={total_enqueued} sent_bps=0"
                    )),
                    Some(format!("worker-starvation:{worker}")),
                );
            }
        }
    }
    // Clearing is independent of the health hold-down: a recovered cluster
    // always clears its stale item.
    for worker in clear {
        info!(worker, "WORKER_STARVATION cleared: no zero-delivery cluster on this worker");
        urgent::clear_by_identity("WORKER_STARVATION", &format!("worker-starvation:{worker}"));
        state.worker_starvation_alarmed.remove(&worker);
    }

    // Isolated circuit alerts (the incumbent surface). Only an UNHEALTHY
    // period's clusters are ignored, so a startup window stays a global
    // sampling window rather than splitting into per-owner items.
    let active_clusters: BTreeMap<u16, Vec<String>> =
        if health_ok { clusters.clone() } else { BTreeMap::new() };
    let clustered: BTreeSet<&str> = active_clusters
        .values()
        .flatten()
        .map(String::as_str)
        .collect();
    let isolated: Vec<&ZeroDelivery> = sustained
        .iter()
        .filter(|c| !clustered.contains(c.id.as_str()))
        .filter(|c| {
            let Some(row) = snapshot.by_circuit_id.get(&c.id) else {
                return false;
            };
            circuit_is_reportable(row.bytes_per_second, row.shaped_bytes_per_second)
        })
        .filter(|c| c.queue_nonempty == Some(true))
        .collect();

    // GUARD (card §4, the LIVE over-fire): many circuits with delivered==0 in ONE
    // scan (the same generation, tiny enqueued rates, NO bridge-side event) is a
    // GLOBAL SAMPLING WINDOW, not N holds — suppress + log once. A true hold is
    // isolated (and the bridge log shows a stuck move/drain/refused admission).
    if isolated.len() >= BLACKHOLE_MAX_SIMULTANEOUS {
        // Warn ONCE per window (not every 2s scan) — the `blackhole_suppressed`
        // latch clears when the window ends.
        if !state.blackhole_suppressed {
            state.blackhole_suppressed = true;
            warn!(
                count = isolated.len(),
                "CIRCUIT_BLACKHOLE suppressed: {} circuits enqueued/delivered==0 in ONE scan — a \
                 global sampling window, not N holds (guard: many-circuits-same-generation)",
                isolated.len()
            );
        }
    } else {
        state.blackhole_suppressed = false;
        for candidate in isolated {
            let circuit_id = &candidate.id;
            let Some(row) = snapshot.by_circuit_id.get(circuit_id) else {
                continue;
            };
            let first = state.blackhole_first_seen.get(circuit_id).copied().unwrap_or(now);
            // CARD §1: the URGENT is the operator surface; the LOG is the
            // instrument. Log ONCE per circuit per window (the `blackhole_alarmed`
            // set gates it) — no log storm — and name the discriminator.
            if !state.blackhole_alarmed.contains_key(circuit_id) {
                state.blackhole_alarmed.insert(
                    circuit_id.clone(),
                    BlackholeEpisode {
                        raised_at: now,
                        raised_at_unix: lqos_utils::unix_time::unix_now().unwrap_or_default(),
                        recovering_since: None,
                    },
                );
                let applied_generation = bridge_broker::convergence_snapshot()
                    .applied_generation
                    .unwrap_or_default();
                warn!(
                    circuit_id = %circuit_id,
                    bridge_circuit_id = row.bridge_circuit_id,
                    enqueued_down_bps = row.bytes_per_second.down,
                    enqueued_up_bps = row.bytes_per_second.up,
                    delivered_down_bps = row.shaped_bytes_per_second.down,
                    delivered_up_bps = row.shaped_bytes_per_second.up,
                    duration_s = now.duration_since(first).as_secs(),
                    last_seen_nanos = row.last_seen_nanos,
                    applied_generation = %applied_generation,
                    "CIRCUIT_BLACKHOLE raised: enqueued>0 but delivered==0 for >= \
                     {BLACKHOLE_SECONDS}s — a TRUE HOLD if the bridge log shows a stuck \
                     move/drain/refused admission; an ARTIFACT if the delivered rate merely \
                     flickered on a low-rate circuit"
                );
            }
            // COPY (§4): no restart advice — restarting for a low-rate/idle
            // metric artifact is exactly the wrong advice.
            urgent::submit(
                UrgentSource::System,
                UrgentSeverity::Error,
                "CIRCUIT_BLACKHOLE".to_string(),
                format!(
                    "Circuit '{circuit_id}' has traffic ENQUEUED but nothing delivered for at \
                     least {BLACKHOLE_SECONDS}s — the bridge may be holding its packets. Check \
                     the bridge log for a stuck move/drain or a refused admission for this \
                     circuit."
                ),
                Some(format!(
                    "bridge_circuit_id={} enqueued_bps={} sent_bps=0",
                    row.bridge_circuit_id,
                    row.bytes_per_second.down.saturating_add(row.bytes_per_second.up)
                )),
                Some(format!("circuit-blackhole:{circuit_id}")),
            );
        }
    }

    // D1 + D2 (driver ruling, `lqos-74t.197`): this replaces the clear on the
    // first scan with delivered > 0. An alarm clears only after
    // `BLACKHOLE_CLEAR_SUSTAIN` of CONTINUOUS delivery at or above
    // `BLACKHOLE_CLEAR_MIN_DELIVERED_BPS`.
    // A circuit missing from the snapshot counts as delivered 0: an
    // empty/partial snapshot must never heal an alarm.
    // CARD (`docs/circuit_blackhole_observability_card.md` §1): INFO on clear,
    // with the episode duration and the delivered rate that cleared it.
    let mut cleared: Vec<(String, u64, u64, u64)> = Vec::new();
    for (id, episode) in state.blackhole_alarmed.iter_mut() {
        let delivered_bps = snapshot
            .by_circuit_id
            .get(id)
            .map(|r| {
                r.shaped_bytes_per_second
                    .down
                    .saturating_add(r.shaped_bytes_per_second.up)
            })
            .unwrap_or(0);
        let (since, clear) = blackhole_recovery_step(episode.recovering_since, delivered_bps, now);
        episode.recovering_since = since;
        if clear {
            cleared.push((
                id.clone(),
                now.saturating_duration_since(episode.raised_at).as_secs(),
                delivered_bps,
                episode.raised_at_unix,
            ));
        }
    }

    state
        .blackhole_first_seen
        .retain(|id, _| seen.contains(id));
    for (id, duration_s, delivered_bps, raised_at_unix) in cleared {
        info!(
            circuit_id = %id,
            duration_s,
            delivered_bps,
            "CIRCUIT_BLACKHOLE cleared: delivery resumed"
        );
        record_blackhole_recovered(&id, raised_at_unix, duration_s, delivered_bps);
        state.blackhole_alarmed.remove(&id);
    }
}

fn scan_churn(state: &mut ContainmentState, now: Instant) {
    let (applied, rejected) = bridge_broker::churn_counters();
    state.churn_samples.push_back((now, applied, rejected));
    while state
        .churn_samples
        .front()
        .is_some_and(|(t, _, _)| now.duration_since(*t) > CHURN_WINDOW)
    {
        state.churn_samples.pop_front();
    }
    let Some(&(_, base_applied, base_rejected)) = state.churn_samples.front() else {
        return;
    };
    let applied_delta = applied.saturating_sub(base_applied);
    let rejected_delta = rejected.saturating_sub(base_rejected);

    if churn_window_exceeds(applied_delta, rejected_delta) {
        urgent::submit(
            UrgentSource::System,
            UrgentSeverity::Warning,
            "CONTROL_PLANE_CHURN".to_string(),
            format!(
                "The bridge control plane emitted {} generation applies and {} rejects in the \
                 last {}s — a re-emit/reject loop is thrashing the control channel. Check the \
                 bridge log for structural rejects (a held circuit usually accompanies it).",
                applied_delta,
                rejected_delta,
                CHURN_WINDOW.as_secs()
            ),
            Some(format!(
                "applied_last_window={applied_delta} rejected_last_window={rejected_delta}"
            )),
            Some("outage-containment:churn".to_string()),
        );
        state.churn_alarmed = true;
    } else if state.churn_alarmed {
        urgent::clear_by_identity("CONTROL_PLANE_CHURN", "outage-containment:churn");
        state.churn_alarmed = false;
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn pair(down: u64, up: u64) -> DownUpOrder<u64> {
        DownUpOrder::new(down, up)
    }

    #[test]
    fn blackhole_detects_enqueued_without_sent() {
        assert!(circuit_is_blackholed(pair(1, 0), pair(0, 0)));
        assert!(circuit_is_blackholed(pair(0, 1), pair(0, 0)));
        assert!(circuit_is_blackholed(pair(5, 7), pair(0, 0)));
    }

    #[test]
    fn blackhole_ignores_idle_and_delivering_circuits() {
        assert!(
            !circuit_is_blackholed(pair(0, 0), pair(0, 0)),
            "an idle circuit is not a black hole"
        );
        assert!(
            !circuit_is_blackholed(pair(9, 9), pair(1, 0)),
            "any delivery clears it"
        );
        assert!(!circuit_is_blackholed(pair(9, 9), pair(0, 2)));
    }

    #[test]
    fn churn_thresholds_bind_on_either_arm() {
        assert!(!churn_window_exceeds(0, 0));
        assert!(!churn_window_exceeds(5, 5));
        assert!(churn_window_exceeds(CHURN_APPLIES_PER_WINDOW, 0));
        assert!(churn_window_exceeds(0, CHURN_REJECTS_PER_WINDOW));
        assert!(churn_window_exceeds(100, 100));
    }

    #[test]
    fn isolated_low_rate_is_floor_filtered_but_a_cluster_is_not() {
        assert!(
            !circuit_is_reportable(pair(999, 0), pair(0, 0)),
            "an isolated sub-floor circuit is the known artifact class"
        );
        assert!(circuit_is_reportable(
            pair(BLACKHOLE_MIN_ENQUEUED_BPS, 0),
            pair(0, 0)
        ));
        assert!(!circuit_is_reportable(pair(0, 0), pair(0, 0)));
        assert!(!circuit_is_reportable(pair(50_000, 0), pair(1, 0)));
        // A cluster built from observations is floor-agnostic: `worker_clusters`
        // never consults `circuit_is_reportable`.
        let now = Instant::now();
        let mut seen = HashMap::new();
        let mut current = BTreeSet::new();
        for id in ["a", "b", "c"] {
            seen.insert(id.to_string(), (5u16, now));
            current.insert(id.to_string());
        }
        let clusters = worker_clusters(&seen, &current, now, BLACKHOLE_WORKER_WINDOW);
        assert_eq!(clusters.get(&5).map(Vec::len), Some(3));
    }

    #[test]
    fn worker_cluster_needs_three_distinct_circuits_on_one_owner() {
        let now = Instant::now();
        let mut seen = HashMap::new();
        seen.insert("a".to_string(), (5u16, now));
        seen.insert("b".to_string(), (5u16, now));
        seen.insert("c".to_string(), (6u16, now));
        seen.insert("d".to_string(), (6u16, now));
        let current: BTreeSet<String> = seen.keys().cloned().collect();
        let clusters = worker_clusters(&seen, &current, now, BLACKHOLE_WORKER_WINDOW);
        assert!(
            !clusters.contains_key(&5),
            "two on one owner is not a worker cluster"
        );
        assert!(!clusters.contains_key(&6));
    }

    #[test]
    fn worker_cluster_uses_the_rolling_window_for_staggered_starts() {
        let now = Instant::now();
        let mut seen = HashMap::new();
        // Staggered, all inside the 60 s window.
        seen.insert("first".to_string(), (5u16, now - Duration::from_secs(50)));
        seen.insert("second".to_string(), (5u16, now - Duration::from_secs(20)));
        seen.insert("third".to_string(), (5u16, now - Duration::from_secs(1)));
        // Outside the window: not part of this episode.
        seen.insert("stale".to_string(), (6u16, now - Duration::from_secs(61)));
        seen.insert("other".to_string(), (6u16, now));
        seen.insert("other2".to_string(), (6u16, now));
        let current: BTreeSet<String> = seen.keys().cloned().collect();
        let clusters = worker_clusters(&seen, &current, now, BLACKHOLE_WORKER_WINDOW);
        assert_eq!(
            clusters.get(&5).map(Vec::len),
            Some(3),
            "staggered starts inside 60 s cluster"
        );
        assert!(!clusters.contains_key(&6), "the stale observation aged out");
    }

    #[test]
    fn worker_alert_is_one_per_episode_and_health_gated() {
        let mut clusters = BTreeMap::new();
        clusters.insert(5u16, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        let mut alarmed = BTreeSet::new();
        let (raise, clear) = worker_alert_changes(&clusters, &alarmed, true);
        assert_eq!(raise, vec![5], "first healthy scan raises ONE worker item");
        assert!(clear.is_empty());
        alarmed.extend(raise);
        let (raise, clear) = worker_alert_changes(&clusters, &alarmed, true);
        assert!(raise.is_empty(), "one alert per circuit/worker per episode");
        assert!(clear.is_empty());
        clusters.clear();
        let (raise, clear) = worker_alert_changes(&clusters, &alarmed, true);
        assert!(raise.is_empty());
        assert_eq!(clear, vec![5], "recovery clears the item");
        // Startup/global window: a cluster exists but health does not hold.
        let mut clusters = BTreeMap::new();
        clusters.insert(5u16, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        let (raise, clear) = worker_alert_changes(&clusters, &BTreeSet::new(), false);
        assert!(raise.is_empty(), "no worker alert during startup/global window");
        assert!(clear.is_empty());
    }

    /// MUST 1 (v4 review): the bridge's `captured_at_millis` is a
    /// BOOT-RELATIVE raw-TSC value (tiny vs Unix epoch), so freshness must
    /// come from lqosd's own `received_at` instant, never a wall-clock
    /// comparison against the bridge field.
    #[test]
    fn snapshot_health_uses_the_lqosd_receive_instant_not_the_bridge_clock() {
        let now = Instant::now();
        // A boot-relative captured value: ~27 minutes of uptime, nowhere near
        // Unix epoch. The old Unix comparison made this permanently stale.
        let boot_relative_millis = 1_600_000u64;
        let mut snap = bridge_analytics::BridgeStatsSnapshot {
            captured_at_millis: boot_relative_millis,
            received_at: Some(now),
            ..Default::default()
        };
        assert!(!snapshot_is_healthy(&snap, now), "no circuit coverage");
        snap.by_circuit_id.insert("c1".to_string(), Default::default());
        assert!(!snapshot_is_healthy(&snap, now), "all-zero global series");
        snap.bridge_actual_bytes_per_second = pair(1_000, 1_000);
        snap.bridge_shaped_bytes_per_second = pair(900, 900);
        assert!(
            snapshot_is_healthy(&snap, now),
            "a boot-relative capture with a fresh lqosd receipt is healthy"
        );
        // A fresh bridge capture cannot rescue an old lqosd receipt.
        snap.received_at = Some(now - HEALTH_CAPTURE_FRESH - Duration::from_secs(1));
        assert!(!snapshot_is_healthy(&snap, now), "old receipt is not fresh");
        snap.received_at = Some(now);
        snap.stale = true;
        assert!(!snapshot_is_healthy(&snap, now), "stale poll");
        snap.stale = false;
        // The 21:25 startup shape: a fresh poll but no circuit rows yet.
        snap.by_circuit_id.clear();
        assert!(!snapshot_is_healthy(&snap, now));
        // No receipt at all (pre-first-poll) is unhealthy.
        snap.by_circuit_id.insert("c1".to_string(), Default::default());
        snap.received_at = None;
        assert!(!snapshot_is_healthy(&snap, now));
    }

    fn zero_delivery(id: &str, owner: Option<u16>) -> ZeroDelivery {
        ZeroDelivery {
            id: id.to_string(),
            owner,
            enqueued_bps: 20_000,
            queue_nonempty: Some(false),
        }
    }

    /// MUST 3 (v4 review): a zero-delivery set that spans many owners is a
    /// fleet/global sampling window, so no per-owner WORKER_STARVATION may be
    /// raised even though each owner has >= 3 circuits.
    #[test]
    fn multi_owner_global_window_raises_no_worker_starvation() {
        // One owner's three circuits: localized => a cluster may raise.
        let local = vec![
            zero_delivery("a", Some(5)),
            zero_delivery("b", Some(5)),
            zero_delivery("c", Some(5)),
        ];
        assert!(!zero_delivery_is_fleet_wide(&local));

        // A partial startup/global window spread across the fleet: the same
        // per-owner shape on four owners. Not localized => no worker alert.
        let global = vec![
            zero_delivery("a", Some(5)),
            zero_delivery("b", Some(5)),
            zero_delivery("c", Some(5)),
            zero_delivery("d", Some(6)),
            zero_delivery("e", Some(6)),
            zero_delivery("f", Some(6)),
            zero_delivery("g", Some(7)),
            zero_delivery("h", Some(7)),
            zero_delivery("i", Some(7)),
            zero_delivery("j", Some(8)),
            zero_delivery("k", Some(8)),
            zero_delivery("l", Some(8)),
        ];
        assert!(zero_delivery_is_fleet_wide(&global));
        let mut clusters = BTreeMap::new();
        clusters.insert(5u16, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        let localized = !zero_delivery_is_fleet_wide(&global);
        let (raise, clear) = worker_alert_changes(&clusters, &BTreeSet::new(), localized);
        assert!(
            raise.is_empty() && clear.is_empty(),
            "a global window must neither raise nor clear worker items"
        );
    }

    /// MUST 4 (v4 review): recovered circuits must stop counting as cluster
    /// candidates immediately; `first_seen` keeps the episode time for the
    /// circuits that are still current.
    #[test]
    fn recovered_circuits_do_not_remain_worker_cluster_candidates() {
        let now = Instant::now();
        let mut seen = HashMap::new();
        for id in ["a", "b", "c"] {
            seen.insert(id.to_string(), (5u16, now - Duration::from_secs(10)));
        }
        // All three recovered: the map still holds them (within 60 s), but
        // the current zero-delivery set is empty.
        let current = BTreeSet::new();
        let clusters = worker_clusters(&seen, &current, now, BLACKHOLE_WORKER_WINDOW);
        assert!(
            clusters.is_empty(),
            "three recovered circuits must not keep a worker alert alive"
        );

        // Two recovered, two still current: two is below the worker minimum.
        let current: BTreeSet<String> = ["b".to_string(), "c".to_string()].into_iter().collect();
        let clusters = worker_clusters(&seen, &current, now, BLACKHOLE_WORKER_WINDOW);
        assert!(clusters.is_empty(), "fewer than 3 CURRENT circuits: no raise");

        // A NEW seventh circuit starts a fresh episode; three current
        // observations cluster again (the recovered ones stay excluded).
        seen.insert("d".to_string(), (5u16, now));
        let current: BTreeSet<String> = ["b".to_string(), "c".to_string(), "d".to_string()]
            .into_iter()
            .collect();
        let clusters = worker_clusters(&seen, &current, now, BLACKHOLE_WORKER_WINDOW);
        assert_eq!(clusters.get(&5).map(Vec::len), Some(3));
        assert!(!clusters[&5].contains(&"a".to_string()));
    }
    #[test]
    fn recovery_needs_sustained_above_floor_delivery() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // A sub-floor trickle never heals, however long it lasts.
        let mut since = None;
        for s in 0..120 {
            let (n, clear) = blackhole_recovery_step(since, 236, at(s));
            assert!(!clear && n.is_none());
            since = n;
        }
        // Above the floor: no clear before 30 s, a clear at 30 s.
        let (since, clear) = blackhole_recovery_step(None, 1_000, at(0));
        assert!(!clear);
        let (since, clear) = blackhole_recovery_step(since, 5_000, at(28));
        assert!(!clear);
        let (_, clear) = blackhole_recovery_step(since, 1_000, at(30));
        assert!(clear);
        // One sub-floor scan restarts the sustained window.
        let (since, _) = blackhole_recovery_step(None, 2_000, at(0));
        let (since, clear) = blackhole_recovery_step(since, 42, at(20));
        assert!(!clear && since.is_none());
        let (since, _) = blackhole_recovery_step(since, 2_000, at(22));
        let (since, clear) = blackhole_recovery_step(since, 2_000, at(50));
        assert!(!clear, "only 28 s since the restart");
        let (_, clear) = blackhole_recovery_step(since, 2_000, at(52));
        assert!(clear);
    }

    /// The ARCHIVED jackrabbit 2026-11-16 23:40:40–00:02:59Z CIRCUIT_BLACKHOLE
    /// raise/clear sequence (`results/jackrabbit_stall_20261117/`
    /// `jr_stall_20261117_full.txt` §4; `lqos-74t.197`), as rows of
    /// (ms since the first event, circuit, raised?, delivered at the clear,
    /// duration_s at the clear).
    const JR_IDS: [&str; 3] = [
        "24c34705-20c4-42e2-8d4a-1adcd629510c",
        "b1d9c3d2-de8e-4b60-a4dd-2c11cefd0710",
        "f6371262-17c7-4a0d-97f9-91828c53d857",
    ];
    const JR_END_MS: u64 = 1_339_713;
    const JR_EVENTS: &[(u64, usize, bool, u64, u64)] = &[
        (0, 0, false, 2896, 56), (126125, 0, true, 0, 8), (152659, 0, false, 95016, 34), (170898, 0, 
        true, 0, 6), (179008, 0, false, 43, 14), (187087, 0, true, 0, 6), (189103, 0, false, 42, 8), 
        (197299, 0, true, 0, 6), (199315, 0, false, 44, 8), (215523, 0, true, 0, 6), (260249, 0, false, 
        113418, 50), (300815, 1, true, 0, 30), (331496, 1, false, 43, 61), (335558, 0, true, 0, 34), 
        (364339, 0, false, 30777, 63), (384599, 0, true, 0, 10), (403105, 0, false, 43648, 28), 
        (419349, 0, true, 0, 6), (442093, 0, false, 10996, 28), (516754, 0, true, 0, 6), (525052, 0, 
        false, 3514, 14), (535260, 0, true, 0, 8), (584352, 0, false, 77650, 57), (590434, 1, true, 0, 
        8), (592449, 1, false, 43, 10), (596516, 0, true, 0, 8), (604631, 0, false, 3490, 16), (621215, 
        1, true, 0, 20), (623336, 1, false, 85, 22), (651944, 1, true, 0, 6), (668239, 1, false, 92, 
        22), (680417, 1, true, 0, 8), (682462, 0, true, 0, 10), (696694, 1, false, 43, 24), (708790, 0, 
        false, 160928, 36), (736971, 0, true, 0, 6), (773516, 1, true, 0, 20), (775533, 0, false, 
        117947, 44), (787617, 1, false, 77, 34), (817773, 1, true, 0, 8), (819773, 1, false, 42, 10), 
        (831831, 0, true, 0, 6), (835892, 1, true, 0, 14), (844149, 0, false, 42, 18), (852218, 0, 
        true, 0, 6), (874772, 0, false, 20947, 28), (874772, 1, false, 138, 52), (896891, 0, true, 0, 
        6), (896891, 1, true, 0, 14), (900942, 0, false, 86, 10), (902942, 1, false, 42, 20), (914965, 
        0, true, 0, 6), (929211, 0, false, 84, 20), (973626, 1, true, 0, 16), (979664, 1, false, 53, 
        22), (1050382, 1, true, 0, 8), (1054428, 1, false, 41, 12), (1313469, 2, true, 0, 18), 
        (1339713, 2, false, 236, 44)
    ];

    /// The delivered rate, reconstructed from the log:
    /// - 0 across each logged zero run `[clear - duration_s, clear)`;
    /// - after a clear, the logged clearing rate until the next zero run.
    fn jr_delivered(c: usize, t: u64) -> u64 {
        let mut val = 0;
        for &(at, ci, raised, d, dur) in JR_EVENTS {
            if ci != c || raised {
                continue;
            }
            if at.saturating_sub(dur * 1000) <= t && t < at {
                return 0;
            }
            if at <= t {
                val = d;
            }
        }
        val
    }

    /// The enqueued rate: idle (0) until the circuit's first logged zero
    /// run, then 20 kbps, above the 10 kbps raise floor (the field's
    /// raises were 10-30 kbps).
    fn jr_enqueued(c: usize, t: u64) -> u64 {
        let first_zero = JR_EVENTS
            .iter()
            .filter(|e| e.1 == c && !e.2)
            .map(|e| e.0.saturating_sub(e.4 * 1000))
            .min()
            .unwrap_or(u64::MAX);
        if t >= first_zero { 20_000 } else { 0 }
    }

    /// TRUE when `t` lies in the gap after one of `c`'s SUB-FLOOR archived
    /// clears and before its next zero run; returns that clear's row index.
    fn jr_trickle_gap(c: usize, t: u64) -> Option<usize> {
        let clears: Vec<(usize, &(u64, usize, bool, u64, u64))> = JR_EVENTS
            .iter()
            .enumerate()
            .filter(|(_, e)| e.1 == c && !e.2)
            .collect();
        let k = clears.iter().rposition(|(_, e)| e.0 <= t)?;
        let (idx, &(_, _, _, d, _)) = clears[k];
        let next_zero = clears
            .get(k + 1)
            .map(|(_, e)| e.0.saturating_sub(e.4 * 1000))
            .unwrap_or(u64::MAX);
        (d < BLACKHOLE_CLEAR_MIN_DELIVERED_BPS && t < next_zero).then_some(idx)
    }

    struct ReplayStats {
        coverage: f64,
        visible: f64,
        trickle_clears_held: BTreeSet<usize>,
        trickle_gap_scans_cleared: usize,
    }

    /// Replays the archived sequence at the detector's 2 s scan cadence.
    /// - Raises happen where the field raised (the raise side is unchanged).
    /// - The heal rule under test runs on the reconstructed delivered rate.
    /// - The urgent store is driven exactly as `scan_blackholes` drives it.
    fn jr_replay(
        step: fn(Option<Instant>, u64, Instant) -> (Option<Instant>, bool),
        retain: bool,
    ) -> ReplayStats {
        urgent::clear_all();
        let scan_ms = SCAN_INTERVAL.as_millis() as u64;
        let base = Instant::now();
        let mut alarmed: BTreeMap<usize, BlackholeEpisode> = BTreeMap::new();
        let mut raised_ever = [false; 3];
        let (mut scans, mut active_scans) = (0usize, 0usize);
        let (mut vis_obs, mut vis_hit) = (0usize, 0usize);
        let mut held = BTreeSet::new();
        let mut gap_cleared = 0;
        let mut t = 0;
        // One scan past the last event, so its trickle gap is observed;
        // coverage counts only the archived window.
        while t <= JR_END_MS + scan_ms {
            let now = base + Duration::from_millis(t);
            for &(at, c, raised, _, _) in JR_EVENTS {
                if raised && at <= t && at + scan_ms > t && !alarmed.contains_key(&c) {
                    alarmed.insert(
                        c,
                        BlackholeEpisode { raised_at: now, raised_at_unix: 1, recovering_since: None },
                    );
                    raised_ever[c] = true;
                    urgent::submit(
                        UrgentSource::System,
                        UrgentSeverity::Error,
                        "CIRCUIT_BLACKHOLE".to_string(),
                        "held".to_string(),
                        None,
                        Some(format!("circuit-blackhole:{}", JR_IDS[c])),
                    );
                }
            }
            let ids: Vec<usize> = alarmed.keys().copied().collect();
            for c in ids {
                let episode = alarmed.get_mut(&c).unwrap();
                let delivered = jr_delivered(c, t);
                let (since, clear) = step(episode.recovering_since, delivered, now);
                episode.recovering_since = since;
                if clear {
                    let raised_at_unix = episode.raised_at_unix;
                    alarmed.remove(&c);
                    if retain {
                        record_blackhole_recovered(JR_IDS[c], raised_at_unix, 0, delivered);
                    } else {
                        urgent::clear_by_identity(
                            "CIRCUIT_BLACKHOLE",
                            &format!("circuit-blackhole:{}", JR_IDS[c]),
                        );
                    }
                }
            }
            let items = urgent::list();
            let active_in_store = items.iter().filter(|i| i.code == "CIRCUIT_BLACKHOLE").count();
            assert_eq!(active_in_store, alarmed.len(), "store == detector at t={t}");
            for c in 0..3 {
                if raised_ever[c] {
                    let key = format!("circuit-blackhole:{}", JR_IDS[c]);
                    vis_obs += 1;
                    vis_hit +=
                        usize::from(items.iter().any(|i| i.dedupe_key.as_deref() == Some(key.as_str())));
                }
                if let Some(idx) = jr_trickle_gap(c, t) {
                    if raised_ever[c] {
                        if alarmed.contains_key(&c) {
                            held.insert(idx);
                        } else {
                            gap_cleared += 1;
                        }
                    }
                }
            }
            if t <= JR_END_MS {
                scans += 1;
                active_scans += usize::from(!alarmed.is_empty());
            }
            t += scan_ms;
        }
        urgent::clear_all();
        ReplayStats {
            coverage: active_scans as f64 / scans as f64,
            visible: vis_hit as f64 / vis_obs as f64,
            trickle_clears_held: held,
            trickle_gap_scans_cleared: gap_cleared,
        }
    }

    /// The REAL detector path on the archived sequence:
    /// - one `scan_blackholes_with` call per 2 s scan;
    /// - a synthetic snapshot carrying the three circuits' reconstructed
    ///   delivered rate, at 20 kbps enqueued (above the 10 kbps raise floor);
    /// - every queue non-empty, as the field raises required.
    /// Raises AND clears come from the detector itself.
    fn jr_real_replay() -> (ReplayStats, usize) {
        urgent::clear_all();
        let scan_ms = SCAN_INTERVAL.as_millis() as u64;
        let base = Instant::now();
        let mut state = ContainmentState::default();
        let mut raised_ever = [false; 3];
        let (mut scans, mut active_scans) = (0usize, 0usize);
        let (mut vis_obs, mut vis_hit) = (0usize, 0usize);
        let mut held = BTreeSet::new();
        let mut gap_cleared = 0;
        let mut recoveries = 0;
        let mut t = 0;
        while t <= JR_END_MS + scan_ms {
            let now = base + Duration::from_millis(t);
            let mut snap = bridge_analytics::BridgeStatsSnapshot::default();
            for (c, id) in JR_IDS.iter().enumerate() {
                snap.by_circuit_id.insert(
                    id.to_string(),
                    bridge_analytics::translate::CircuitLiveRow {
                        bytes_per_second: DownUpOrder::new(jr_enqueued(c, t), 0),
                        shaped_bytes_per_second: DownUpOrder::new(jr_delivered(c, t), 0),
                        ..Default::default()
                    },
                );
            }
            let before: BTreeSet<String> = state.blackhole_alarmed.keys().cloned().collect();
            scan_blackholes_with(&mut state, now, &snap, |_| Some(true));
            let items = urgent::list();
            for id in before.iter().filter(|id| !state.blackhole_alarmed.contains_key(*id)) {
                recoveries += 1;
                let key = format!("circuit-blackhole:{id}");
                assert!(
                    items.iter().any(|i| i.code == "CIRCUIT_BLACKHOLE_RECOVERED"
                        && i.dedupe_key.as_deref() == Some(key.as_str())
                        && i.context.as_deref().is_some_and(|c| c.contains("recovered_at_unix="))),
                    "a genuine recovery must leave a RECOVERED record (t={t})"
                );
            }
            let active_in_store = items.iter().filter(|i| i.code == "CIRCUIT_BLACKHOLE").count();
            assert_eq!(active_in_store, state.blackhole_alarmed.len(), "store == detector at t={t}");
            for (c, id) in JR_IDS.iter().enumerate() {
                let alarmed = state.blackhole_alarmed.contains_key(*id);
                raised_ever[c] |= alarmed;
                if raised_ever[c] {
                    let key = format!("circuit-blackhole:{id}");
                    vis_obs += 1;
                    vis_hit +=
                        usize::from(items.iter().any(|i| i.dedupe_key.as_deref() == Some(key.as_str())));
                    if let Some(idx) = jr_trickle_gap(c, t) {
                        if alarmed {
                            held.insert(idx);
                        } else {
                            gap_cleared += 1;
                        }
                    }
                }
            }
            if t <= JR_END_MS {
                scans += 1;
                active_scans += usize::from(!state.blackhole_alarmed.is_empty());
            }
            t += scan_ms;
        }
        urgent::clear_all();
        (
            ReplayStats {
                coverage: active_scans as f64 / scans as f64,
                visible: vis_hit as f64 / vis_obs as f64,
                trickle_clears_held: held,
                trickle_gap_scans_cleared: gap_cleared,
            },
            recoveries,
        )
    }

    /// The incumbent rule (clear on the first scan with delivered > 0), kept
    /// only as the differential baseline for the replay.
    fn incumbent_step(_: Option<Instant>, delivered: u64, _: Instant) -> (Option<Instant>, bool) {
        (None, delivered > 0)
    }

    #[test]
    fn jackrabbit_20261116_replay_alerts_hold_through_trickle_and_stay_visible() {
        let _guard = urgent::test_lock();
        let sub_floor_clears = JR_EVENTS
            .iter()
            .filter(|e| !e.2 && e.3 < BLACKHOLE_CLEAR_MIN_DELIVERED_BPS)
            .count();
        assert_eq!(sub_floor_clears, 18, "the archived 41-236 bps trickle clears");

        // The REAL scan path (the detector raises and clears by itself).
        let (new, recoveries) = jr_real_replay();
        // The same reconstruction through the step model, which must agree
        // with the real path.
        let model = jr_replay(blackhole_recovery_step, true);
        let old = jr_replay(incumbent_step, false);
        assert!(recoveries >= 1, "at least one genuine (>= floor, 30 s) recovery");
        assert!(
            (new.coverage - model.coverage).abs() < 0.03,
            "real {:.3} vs model {:.3}",
            new.coverage,
            model.coverage
        );
        eprintln!(
            "JR-REPLAY real: coverage={:.3} visible={:.3} trickle_held={} gap_cleared={} \
             recoveries={recoveries} model_coverage={:.3} | incumbent: coverage={:.3} \
             visible={:.3} gap_cleared={}",
            new.coverage,
            new.visible,
            new.trickle_clears_held.len(),
            new.trickle_gap_scans_cleared,
            model.coverage,
            old.coverage,
            old.visible,
            old.trickle_gap_scans_cleared
        );
        // D1 + D2: every one of the 18 trickle clears is held, and no alert
        // drops inside any trickle gap.
        assert_eq!(new.trickle_clears_held.len(), 18);
        assert_eq!(new.trickle_gap_scans_cleared, 0);
        assert!(new.coverage >= 0.86, "coverage {:.3} < 0.86", new.coverage);
        // Retention: once raised, every circuit's event is in the list at
        // every scan (no blind gap).
        assert!((new.visible - 1.0).abs() < f64::EPSILON, "visible {:.3}", new.visible);
        // Differential: the incumbent rule reproduces the field blindness.
        assert!(old.coverage < 0.5, "incumbent coverage {:.3}", old.coverage);
        assert!(old.visible < 0.5, "incumbent visible {:.3}", old.visible);
        assert!(old.trickle_gap_scans_cleared > 0);
    }
}

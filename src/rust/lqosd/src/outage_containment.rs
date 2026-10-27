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
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How often the cold-path alarm scans.
const SCAN_INTERVAL: Duration = Duration::from_secs(2);
/// A circuit must be enqueueing-but-not-sending for this long to alarm
/// (DRIVER-owned, §9).
const BLACKHOLE_SECONDS: u64 = 5;
/// GUARD (card §4, the LIVE over-fire): if this many circuits are
/// enqueued/delivered==0 in ONE scan it is a GLOBAL sampling window, not N
/// holds — suppress (a true hold is isolated).
const BLACKHOLE_MAX_SIMULTANEOUS: usize = 3;
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
    /// circuit ids currently alarmed (for heal-clearing).
    blackhole_alarmed: BTreeSet<String>,
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

/// TRUE when a circuit is enqueueing traffic but delivering none: the
/// incident's `rx>0 / tx≈0` shape (a held/parked circuit).
fn circuit_is_blackholed(enqueued: DownUpOrder<u64>, sent: DownUpOrder<u64>) -> bool {
    (enqueued.down > 0 || enqueued.up > 0) && sent.down == 0 && sent.up == 0
}

/// TRUE when either churn arm is over its per-window bound.
fn churn_window_exceeds(applied_delta: u64, rejected_delta: u64) -> bool {
    rejected_delta >= CHURN_REJECTS_PER_WINDOW || applied_delta >= CHURN_APPLIES_PER_WINDOW
}

fn scan_blackholes(state: &mut ContainmentState, now: Instant) {
    let snapshot = bridge_analytics::snapshot();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut candidates: Vec<String> = Vec::new();
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
        // GUARD (a) (§4, REQUIRED): the circuit's OWN bridge queue must be
        // non-empty — the bridge ALREADY publishes per-circuit `queue_depth_pkts`
        // (QueueStats `QdiscDetailRow` by slot). An RX-rate flicker with an EMPTY
        // bridge queue is NOT a hold. (Only queried for sustained candidates.)
        match bridge_analytics::queue_stats_for_circuit(circuit_id) {
            Some(q) => {
                queue_stats_available = true;
                if q.download.queue_depth_pkts == 0 && q.upload.queue_depth_pkts == 0 {
                    continue; // own queue empty ⇒ not a hold
                }
                candidates.push(circuit_id.clone());
            }
            None => {
                // QueueStats unavailable (actor inactive / socket / connect) —
                // track it; do NOT raise on an unverifiable candidate.
                queue_stats_unavailable = true;
            }
        }
    }
    // DEGRADED latch (§4): if QueueStats is entirely unavailable, guard (a)
    // filters EVERY candidate ⇒ the detector is SILENTLY DISABLED — warn ONCE so
    // it is not mistaken for "no blackholes"; clear when it recovers.
    if queue_stats_unavailable && !queue_stats_available {
        if !state.blackhole_queue_degraded {
            state.blackhole_queue_degraded = true;
            warn!(
                "CIRCUIT_BLACKHOLE detector DEGRADED: QueueStats unavailable — guard (a) cannot \
                 verify a circuit's own queue, so the detector is QUIET until it recovers (this \
                 is NOT 'no blackholes')"
            );
        }
    } else {
        state.blackhole_queue_degraded = false;
    }
    // GUARD (card §4, the LIVE over-fire): many circuits with delivered==0 in ONE
    // scan (the same generation, tiny enqueued rates, NO bridge-side event) is a
    // GLOBAL SAMPLING WINDOW, not N holds — suppress + log once. A true hold is
    // isolated (and the bridge log shows a stuck move/drain/refused admission).
    if candidates.len() >= BLACKHOLE_MAX_SIMULTANEOUS {
        // Warn ONCE per window (not every 2s scan) — the `blackhole_suppressed`
        // latch clears when the window ends.
        if !state.blackhole_suppressed {
            state.blackhole_suppressed = true;
            warn!(
                count = candidates.len(),
                "CIRCUIT_BLACKHOLE suppressed: {} circuits enqueued/delivered==0 in ONE scan — a \
                 global sampling window, not N holds (guard: many-circuits-same-generation)",
                candidates.len()
            );
        }
    } else {
        state.blackhole_suppressed = false;
        for circuit_id in &candidates {
            let Some(row) = snapshot.by_circuit_id.get(circuit_id) else {
                continue;
            };
            let first = state.blackhole_first_seen.get(circuit_id).copied().unwrap_or(now);
            // CARD §1: the URGENT is the operator surface; the LOG is the
            // instrument. Log ONCE per circuit per window (the `blackhole_alarmed`
            // set gates it) — no log storm — and name the discriminator.
            if state.blackhole_alarmed.insert(circuit_id.clone()) {
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

    // CARD (`docs/circuit_blackhole_observability_card.md` §1): capture the
    // cleared circuits' TOTAL duration + the delivered rate that cleared them
    // BEFORE the `first_seen` retain drops their start time. INFO on clear.
    let cleared: Vec<(String, u64, u64)> = state
        .blackhole_alarmed
        .iter()
        .filter(|id| !seen.contains(*id))
        .map(|id| {
            let duration_s = state
                .blackhole_first_seen
                .get(id)
                .map(|t| now.duration_since(*t).as_secs())
                .unwrap_or(0);
            let delivered_bps = snapshot
                .by_circuit_id
                .get(id)
                .map(|r| {
                    r.shaped_bytes_per_second
                        .down
                        .saturating_add(r.shaped_bytes_per_second.up)
                })
                .unwrap_or(0);
            (id.clone(), duration_s, delivered_bps)
        })
        .collect();

    state
        .blackhole_first_seen
        .retain(|id, _| seen.contains(id));
    for (id, duration_s, delivered_bps) in cleared {
        info!(
            circuit_id = %id,
            duration_s,
            delivered_bps,
            "CIRCUIT_BLACKHOLE cleared: delivery resumed"
        );
        urgent::clear_by_identity("CIRCUIT_BLACKHOLE", &format!("circuit-blackhole:{id}"));
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
}

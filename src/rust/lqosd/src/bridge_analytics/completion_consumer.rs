//! The flow-completion feed consumer (`lqos-74t.2.x`).
//!
//! A DEDICATED drain thread (not the 1 s snapshot tick) over the bridge's
//! completion ring (the additive `DrainCompletions` IPC), with a durable
//! `(epoch, seq)` cursor. The DRIVER ruled **option 1**: at-least-once, and a
//! bounded <=1-batch duplicate on an lqosd crash is accepted — no idempotency
//! key, **no Insight schema change**.
//!
//! Re-`#12` rework: Q2 health gate, Q3 retention bypass on the ring
//! path, Q4 SENT-packet mapping, Q5 health counters + `batch.dropped`, Q6 a
//! deterministic handover, Q7 a `next_seq` regression guard, Q8 the missing
//! tests. **Q1 (durable commit / at-most-once) is HELD for the DRIVER** — the
//! ack boundary here is the consumer ENQUEUE/handoff.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use analytics_ipc::{
    CompletionBatch, CompletionEvent, DirectionalCounters, FlowRow, FlowState,
    connect_drain_completions, connect_health,
};

/// The card §2 batch bound (8192 events).
pub(crate) const MAX_BATCH: usize = 8192;
/// The drain cadence while idle / after a failed batch.
const DRAIN_INTERVAL: Duration = Duration::from_millis(250);
/// The bounded retry (BOTH the one-sided and the both-failed paths):
/// exponential backoff capped at `MAX_BACKOFF`; after `PARTIAL_RETRY_ATTEMPTS`
/// the batch is ACKED as COUNTED loss and the feed latches DEGRADED.
/// 8 attempts / 2 s cap ≈ 12 s total — well inside the 45 s ring-cover margin.
const PARTIAL_RETRY_ATTEMPTS: u32 = 8;
const MAX_BACKOFF: Duration = Duration::from_secs(2);
/// The health re-probe cadence while the ring is not advertised (Q2: never
/// send `DrainCompletions` to an old bridge — no serde-error/reconnect churn).
const HEALTH_PROBE_INTERVAL: Duration = Duration::from_secs(2);

static RING_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Q6: latched on the FIRST acked batch; the snapshot export is then disabled
/// deterministically (never a polling race).
static HANDED_OVER: AtomicBool = AtomicBool::new(false);
static DRAINED: AtomicU64 = AtomicU64::new(0);
static EXPORTED: AtomicU64 = AtomicU64::new(0);
/// The bridge's cumulative `dropped` delta = events the RING overwrote
/// (bridge-side overflow). INFORMATIONAL — NOT a consumer loss and NOT a
/// degraded trigger (the pins).
static OVERWRITTEN: AtomicU64 = AtomicU64::new(0);
static REPLAY_LOST: AtomicU64 = AtomicU64::new(0);
static OVERLAP: AtomicU64 = AtomicU64::new(0);
/// The accepted loss when a side still fails after the BOUNDED retry.
static FAILED_SIDE_LOSS: AtomicU64 = AtomicU64::new(0);
static DEGRADED: AtomicBool = AtomicBool::new(false);

/// Q5: the lqosd-side drain health. `completion_loss` is the CONSUMER loss
/// (`replay_lost + failed_side_loss`); `overwritten` is the bridge-side ring
/// overflow (INFORMATIONAL — not loss, not degraded).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CompletionHealth {
    pub ring_active: bool,
    pub handed_over: bool,
    pub drained: u64,
    pub exported: u64,
    /// Bridge-side ring overwrite delta (informational).
    pub overwritten: u64,
    pub replay_lost: u64,
    /// The bounded pre-handover overlap (Q6): the first batch's events.
    pub overlap: u64,
    /// Q1 partial failure: events LOST on the failed side after the bounded
    /// retry was exhausted (accepted, disclosed — the batch is then acked).
    pub failed_side_loss: u64,
    /// The consumer-side LOSS = `replay_lost + failed_side_loss`.
    pub completion_loss: u64,
    pub degraded: bool,
}

pub(crate) fn health() -> CompletionHealth {
    let replay_lost = REPLAY_LOST.load(Ordering::Relaxed);
    let failed_side_loss = FAILED_SIDE_LOSS.load(Ordering::Relaxed);
    CompletionHealth {
        ring_active: ring_active(),
        handed_over: HANDED_OVER.load(Ordering::Relaxed),
        drained: DRAINED.load(Ordering::Relaxed),
        exported: EXPORTED.load(Ordering::Relaxed),
        overwritten: OVERWRITTEN.load(Ordering::Relaxed),
        replay_lost,
        overlap: OVERLAP.load(Ordering::Relaxed),
        failed_side_loss,
        completion_loss: replay_lost.saturating_add(failed_side_loss),
        degraded: DEGRADED.load(Ordering::Relaxed),
    }
}

/// Whether the completion-ring drain is live.
pub(crate) fn ring_active() -> bool {
    RING_ACTIVE.load(Ordering::Relaxed)
}

/// Q6: the snapshot export is the FALLBACK only — it is disabled as soon as the
/// ring hands over (deterministic, not a poll-to-poll race).
pub(crate) fn snapshot_export_enabled() -> bool {
    snapshot_export_enabled_for(HANDED_OVER.load(Ordering::Relaxed))
}

/// Pure predicate (unit-tested).
pub(crate) fn snapshot_export_enabled_for(handed_over: bool) -> bool {
    !handed_over
}

/// Q2: the bridge advertises the ring iff its health epoch is non-zero.
pub(crate) fn ring_advertised(health_epoch: u64) -> bool {
    health_epoch != 0
}

/// The durable `(epoch, seq)` cursor. `seq` is the once-only authority; an
/// epoch change (a bridge restart) re-bases it; a ring gap is counted as
/// `replay_lost`; the bridge's cumulative `dropped` is folded as a delta.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CompletionCursor {
    pub epoch: u64,
    pub seq: u64,
    pub replay_lost: u64,
    /// The bridge's cumulative `dropped` at the last ack (the delta folds into
    /// the health counter). ADDITIVE (serde default) for an old cursor file.
    #[serde(default)]
    pub dropped: u64,
}

impl CompletionCursor {
    pub(crate) fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&tmp, path)
    }

    /// Fold one batch: reset on an epoch change; count a gap as `replay_lost`
    /// (a cold start / fresh epoch begins at `oldest_seq` by design, so only a
    /// gap AFTER a position counts); advance to `next_seq` but **never
    /// regress** (Q7 — a backwards cursor would re-drain/duplicate).
    pub(crate) fn advance(&mut self, batch: &CompletionBatch) -> u64 {
        if batch.epoch != self.epoch {
            self.epoch = batch.epoch;
            self.seq = 0;
        }
        let expected = self.seq.saturating_add(1);
        if self.seq > 0 && batch.oldest_seq > expected {
            self.replay_lost = self
                .replay_lost
                .saturating_add(batch.oldest_seq.saturating_sub(expected));
        }
        self.seq = self.seq.max(batch.next_seq);
        batch.events.len() as u64
    }
}

/// Q4: map one compact completion record to the `FlowRow` the exporters read.
/// The consumers read `sent_pkts_*` (`lts2_flow_export` + `netflow`), so the
/// event's SENT packet counts go THERE; the 144 B summary carries no
/// TCP-specific packet split (`tcp_sent_packets*` = 0) and the shaped counters
/// are unconsumed (0). The Insight wire is UNCHANGED.
pub(crate) fn event_to_flow_row(event: &CompletionEvent) -> FlowRow {
    let state = match event.terminal_state {
        1 => FlowState::Reset,
        2 => FlowState::Idle,
        _ => FlowState::Fin,
    };
    let mut down = Vec::new();
    let mut up = Vec::new();
    if event.rtt_median_down_us > 0 {
        down.push(event.rtt_median_down_us);
    }
    if event.rtt_median_up_us > 0 {
        up.push(event.rtt_median_up_us);
    }
    let mut merged = down.clone();
    merged.extend_from_slice(&up);
    FlowRow {
        id: event.flow.clone(),
        actual: DirectionalCounters {
            download_bytes: event.actual_bytes_down,
            upload_bytes: event.actual_bytes_up,
        },
        shaped: DirectionalCounters::default(),
        rtt_samples_us: merged,
        tcp_retransmits: event
            .tcp_retransmits_down
            .saturating_add(event.tcp_retransmits_up),
        first_seen_millis: event.first_seen_millis,
        last_seen_millis: event.last_seen_millis,
        state,
        rtt_samples_down_us: down,
        rtt_samples_up_us: up,
        tcp_retransmits_down: event.tcp_retransmits_down,
        tcp_retransmits_up: event.tcp_retransmits_up,
        sent_pkts_down: event.actual_packets_down,
        sent_pkts_up: event.actual_packets_up,
        ..Default::default()
    }
}

/// Apply the do-not-track filter (`lqos-74t.31` W1) and convert a batch.
pub(crate) fn apply_batch(
    events: &[CompletionEvent],
    block: Option<&ip_network_table::IpNetworkTable<bool>>,
) -> Vec<FlowRow> {
    events
        .iter()
        .filter(|event| {
            block.is_none_or(|table| table.longest_match(event.flow.subscriber_ip).is_none())
        })
        .map(event_to_flow_row)
        .collect()
}

/// The result of handing one batch to the two bounded consumers. Both sides run
/// UNCONDITIONALLY (Q1); a one-sided failure is a PARTIAL that the BOUNDED
/// per-side retry resumes WITHOUT re-sending the succeeded side (no dup storm).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Handoff {
    pub netflow: bool,
    pub lts2: bool,
}

impl Handoff {
    pub(crate) fn ok() -> Self {
        Self {
            netflow: true,
            lts2: true,
        }
    }
    pub(crate) fn is_full(self) -> bool {
        self.netflow && self.lts2
    }
    pub(crate) fn is_partial(self) -> bool {
        self.netflow != self.lts2
    }
}

/// Which sides still need sending (SKIP the ones that already succeeded).
pub(crate) fn sides_to_retry(prior: Handoff) -> (bool, bool) {
    (!prior.netflow, !prior.lts2)
}

/// The bounded partial-retry budget: exponential backoff capped at
/// `MAX_BACKOFF`; `None` once spent (then ACK + count the loss + DEGRADED).
#[derive(Clone, Copy, Debug)]
pub(crate) struct RetryBudget {
    attempts_left: u32,
    backoff: Duration,
}

impl RetryBudget {
    pub(crate) fn new() -> Self {
        Self {
            attempts_left: PARTIAL_RETRY_ATTEMPTS,
            backoff: DRAIN_INTERVAL,
        }
    }
    /// The delay before the next retry, or `None` when the budget is spent.
    pub(crate) fn next_delay(&mut self) -> Option<Duration> {
        if self.attempts_left == 0 {
            return None;
        }
        self.attempts_left -= 1;
        let delay = self.backoff;
        self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
        Some(delay)
    }
}

/// The first attempt: BOTH consumers, unconditionally.
pub(crate) fn handoff(flows: &[FlowRow]) -> Handoff {
    let netflow = crate::netflow::queue_completed_flows_unretained(flows);
    let lts2 = super::lts2_flow_export::export_ring_batch(flows);
    Handoff { netflow, lts2 }
}

/// The retry: send ONLY the side(s) that failed (the succeeded side is
/// skipped ⇒ a persistent one-sided failure never re-sends it — no dup storm).
pub(crate) fn retry_failed_side(flows: &[FlowRow], prior: Handoff) -> Handoff {
    let (netflow, lts2) = sides_to_retry(prior);
    Handoff {
        netflow: if netflow {
            crate::netflow::queue_completed_flows_unretained(flows)
        } else {
            true
        },
        lts2: if lts2 {
            super::lts2_flow_export::export_ring_batch(flows)
        } else {
            true
        },
    }
}

/// Join a retry outcome with the prior (a side that already succeeded stays ok).
pub(crate) fn merge_retry(prior: Handoff, tried: Handoff) -> Handoff {
    Handoff {
        netflow: prior.netflow || tried.netflow,
        lts2: prior.lts2 || tried.lts2,
    }
}

#[derive(Debug)]
pub(crate) enum BatchOutcome {
    /// Fully handed off (the cursor advanced + persisted).
    Acked,
    /// A one-sided failure: the bounded per-side retry resumes the failed side.
    Partial { flows: Vec<FlowRow>, handoff: Handoff },
    /// BOTH sides failed: the SAME bounded retry resumes both, then ACKs +
    /// counts the loss + latches DEGRADED (never an unbounded 250 ms retry).
    Failed { flows: Vec<FlowRow> },
}

/// ACK one batch: the overwritten counter (INFORMATIONAL), Q6 (the first acked
/// batch is the deterministic handover + the counted overlap), Q7 (never
/// regress), then persist. Used by the normal AND the exhausted-retry path.
pub(crate) fn ack_batch(
    cursor: &mut CompletionCursor,
    cursor_path: &Path,
    batch: &CompletionBatch,
) {
    if !HANDED_OVER.swap(true, Ordering::Relaxed) {
        OVERLAP.fetch_add(batch.events.len() as u64, Ordering::Relaxed);
    }
    DRAINED.fetch_add(batch.events.len() as u64, Ordering::Relaxed);
    let dropped_delta = batch.dropped.saturating_sub(cursor.dropped);
    if dropped_delta > 0 {
        // INFORMATIONAL only: the ring OVERWROTE events (bridge-side overflow).
        // This is NOT a consumer loss and must NOT latch DEGRADED (the pins).
        OVERWRITTEN.fetch_add(dropped_delta, Ordering::Relaxed);
    }
    cursor.dropped = batch.dropped;
    let before = cursor.replay_lost;
    cursor.advance(batch);
    if cursor.replay_lost > before {
        REPLAY_LOST.store(cursor.replay_lost, Ordering::Relaxed);
        DEGRADED.store(true, Ordering::Relaxed);
    }
    let _ = cursor.save(cursor_path);
}

/// The bounded-retry terminal action (never false-healthy): ACK the batch,
/// count the failed side's loss, and latch DEGRADED.
pub(crate) fn exhaust_partial(
    cursor: &mut CompletionCursor,
    cursor_path: &Path,
    batch: &CompletionBatch,
    flows: &[FlowRow],
) {
    FAILED_SIDE_LOSS.fetch_add(flows.len() as u64, Ordering::Relaxed);
    DEGRADED.store(true, Ordering::Relaxed);
    tracing::warn!(
        "completion feed DEGRADED: a side failed after the bounded retry; acked + counted {} lost events",
        flows.len()
    );
    ack_batch(cursor, cursor_path, batch);
}

/// The first attempt for one drained batch (the handoff is injectable for tests).
pub(crate) fn process_batch_once(
    cursor: &mut CompletionCursor,
    cursor_path: &Path,
    batch: &CompletionBatch,
    attempt: impl FnOnce(&[FlowRow]) -> Handoff,
) -> BatchOutcome {
    // (a) An EPOCH CHANGE is a RE-BASE signal (flash_1 returns next_seq=0 on a
    // cursor/epoch mismatch): adopt the new epoch at seq 0 and IGNORE this
    // batch (its events + next_seq); the new epoch's FIRST real batch advances.
    if batch.epoch != cursor.epoch {
        cursor.epoch = batch.epoch;
        cursor.seq = 0;
        cursor.dropped = 0;
        let _ = cursor.save(cursor_path);
        return BatchOutcome::Acked;
    }
    let block = super::do_not_track_table();
    let flows = apply_batch(&batch.events, block.as_ref());
    if flows.is_empty() {
        ack_batch(cursor, cursor_path, batch);
        return BatchOutcome::Acked;
    }
    let outcome = attempt(&flows);
    if outcome.is_full() {
        EXPORTED.fetch_add(flows.len() as u64, Ordering::Relaxed);
        ack_batch(cursor, cursor_path, batch);
        BatchOutcome::Acked
    } else if outcome.is_partial() {
        BatchOutcome::Partial {
            flows,
            handoff: outcome,
        }
    } else {
        BatchOutcome::Failed { flows }
    }
}

/// A successful retry (partial or total): count the export and ACK.
pub(crate) fn retry_succeeded(
    cursor: &mut CompletionCursor,
    cursor_path: &Path,
    batch: &CompletionBatch,
    flows: &[FlowRow],
) {
    EXPORTED.fetch_add(flows.len() as u64, Ordering::Relaxed);
    ack_batch(cursor, cursor_path, batch);
}

/// Spawn the dedicated drain thread. Q2: probe the health epoch first and send
/// `DrainCompletions` only when advertised; a one-sided failure retries the
/// FAILED side only, with a bounded exponential backoff, then ACKs + counts the
/// loss + latches DEGRADED (no 250 ms dup storm, no false-healthy).
pub(crate) fn spawn(socket: PathBuf, cursor_path: PathBuf) {
    let _ = std::thread::Builder::new()
        .name("completion-feed".into())
        .spawn(move || {
            let mut cursor = CompletionCursor::load(&cursor_path);
            let mut advertised = false;
            let mut ticks: u64 = 0;
            let mut pending: Option<(CompletionBatch, Vec<FlowRow>, Handoff, RetryBudget)> = None;
            loop {
                if let Some((batch, flows, prior, mut budget)) = pending.take() {
                    match budget.next_delay() {
                        Some(delay) => {
                            std::thread::sleep(delay);
                            let retried = merge_retry(prior, retry_failed_side(&flows, prior));
                            if retried.is_full() {
                                retry_succeeded(&mut cursor, &cursor_path, &batch, &flows);
                            } else {
                                pending = Some((batch, flows, retried, budget));
                            }
                        }
                        None => exhaust_partial(&mut cursor, &cursor_path, &batch, &flows),
                    }
                    continue;
                }
                if !advertised {
                    advertised = matches!(
                        connect_health(&socket),
                        Ok(report) if ring_advertised(report.completion_ring_epoch)
                    );
                    if !advertised {
                        RING_ACTIVE.store(false, Ordering::Relaxed);
                        std::thread::sleep(HEALTH_PROBE_INTERVAL);
                        continue;
                    }
                }
                match connect_drain_completions(&socket, cursor.epoch, cursor.seq, MAX_BATCH) {
                    Ok(batch) => {
                        RING_ACTIVE.store(true, Ordering::Relaxed);
                        match process_batch_once(&mut cursor, &cursor_path, &batch, handoff) {
                            BatchOutcome::Acked => {
                                if batch.events.is_empty() {
                                    std::thread::sleep(DRAIN_INTERVAL);
                                }
                            }
                            BatchOutcome::Partial { flows, handoff } => {
                                pending = Some((batch, flows, handoff, RetryBudget::new()));
                            }
                            BatchOutcome::Failed { flows } => {
                                // (a) BOTH failed: the SAME bounded retry (both
                                // sides), then ACK + count + DEGRADED on
                                // exhaustion — never an unbounded 250 ms retry.
                                pending = Some((
                                    batch,
                                    flows,
                                    Handoff {
                                        netflow: false,
                                        lts2: false,
                                    },
                                    RetryBudget::new(),
                                ));
                            }
                        }
                        ticks = ticks.wrapping_add(1);
                        if ticks % 240 == 0 {
                            tracing::info!("completion feed health: {:?}", health());
                        }
                    }
                    Err(_) => {
                        advertised = false;
                        RING_ACTIVE.store(false, Ordering::Relaxed);
                        std::thread::sleep(DRAIN_INTERVAL);
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    // The health counters are process-global: serialize the tests that read
    // them and reset the statics so each test starts from a known state.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        RING_ACTIVE.store(false, Ordering::Relaxed);
        HANDED_OVER.store(false, Ordering::Relaxed);
        DRAINED.store(0, Ordering::Relaxed);
        EXPORTED.store(0, Ordering::Relaxed);
        OVERWRITTEN.store(0, Ordering::Relaxed);
        REPLAY_LOST.store(0, Ordering::Relaxed);
        OVERLAP.store(0, Ordering::Relaxed);
        FAILED_SIDE_LOSS.store(0, Ordering::Relaxed);
        DEGRADED.store(false, Ordering::Relaxed);
        guard
    }

    fn event(seq: u64, down_us: u32, up_us: u32) -> CompletionEvent {
        CompletionEvent {
            seq,
            actual_bytes_down: 1_000,
            actual_bytes_up: 500,
            actual_packets_down: 10,
            actual_packets_up: 5,
            tcp_retransmits_down: 2,
            tcp_retransmits_up: 1,
            rtt_median_down_us: down_us,
            rtt_median_up_us: up_us,
            first_seen_millis: 5_000,
            last_seen_millis: 9_999,
            terminal_state: 1,
            ..Default::default()
        }
    }

    fn tmp_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lqos-cursor-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("completion_cursor.json")
    }

    fn batch(epoch: u64, next_seq: u64, oldest_seq: u64, dropped: u64) -> CompletionBatch {
        CompletionBatch {
            epoch,
            events: vec![event(1, 10, 20)],
            next_seq,
            oldest_seq,
            dropped,
        }
    }

    #[test]
    fn health_gate_requires_the_ring_epoch() {
        let _g = serial();
        assert!(!ring_advertised(0), "an old bridge (epoch 0) is NOT drained");
        assert!(ring_advertised(7));
    }

    #[test]
    fn fallback_is_disabled_after_handover() {
        let _g = serial();
        assert!(snapshot_export_enabled_for(false));
        assert!(!snapshot_export_enabled_for(true));
    }

    /// Q7: a backwards `next_seq` must never regress the cursor.
    #[test]
    fn next_seq_regression_is_ignored() {
        let _g = serial();
        let mut c = CompletionCursor {
            epoch: 7,
            seq: 40,
            ..Default::default()
        };
        c.advance(&batch(7, 12, 12, 0));
        assert_eq!(c.seq, 40, "a regression must not re-drain/duplicate");
    }

    /// Q8 (ack boundary): a FAILED attempt does NOT advance/persist; a full
    /// handoff ACKs.
    #[test]
    fn ack_boundary_holds_on_export_failure() {
        let _g = serial();
        let path = tmp_path("ack");
        let mut c = CompletionCursor {
            epoch: 7,
            ..Default::default()
        };
        let b = batch(7, 2, 1, 0);
        let out = process_batch_once(&mut c, &path, &b, |_| Handoff {
            netflow: false,
            lts2: false,
        });
        assert!(matches!(out, BatchOutcome::Failed { .. }));
        assert_eq!((c.epoch, c.seq), (7, 0), "no ack on a failed handoff");
        assert!(!path.exists(), "no cursor persisted on a failure");
        let out = process_batch_once(&mut c, &path, &b, |_| Handoff::ok());
        assert!(matches!(out, BatchOutcome::Acked));
        assert_eq!((c.epoch, c.seq), (7, 2), "ack advances after the handoff");
        assert!(path.exists(), "the acked cursor is persisted");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// (a) An epoch change RE-BASES (epoch, seq=0) and does NOT export/advance.
    #[test]
    fn epoch_change_rebases_without_advancing() {
        let _g = serial();
        let path = tmp_path("epoch");
        let mut c = CompletionCursor {
            epoch: 3,
            seq: 40,
            dropped: 5,
            replay_lost: 0,
        };
        let out = process_batch_once(&mut c, &path, &batch(9, 0, 0, 0), |_| {
            panic!("must not export on a re-base")
        });
        assert!(matches!(out, BatchOutcome::Acked));
        assert_eq!((c.epoch, c.seq, c.dropped), (9, 0, 0));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// (a) A FOREIGN epoch is a re-base EVEN with a NON-zero `next_seq`.
    #[test]
    fn foreign_epoch_is_a_rebase_even_with_a_nonzero_next_seq() {
        let _g = serial();
        let path = tmp_path("foreign");
        let mut c = CompletionCursor {
            epoch: 3,
            seq: 40,
            dropped: 5,
            replay_lost: 0,
        };
        let out = process_batch_once(&mut c, &path, &batch(9, 50, 50, 0), |_| {
            panic!("must not export on a foreign epoch")
        });
        assert!(matches!(out, BatchOutcome::Acked));
        assert_eq!((c.epoch, c.seq, c.dropped), (9, 0, 0));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Q5: `batch.dropped` folds as a delta; a gap counts `replay_lost`.
    #[test]
    fn dropped_and_gaps_fold_into_the_cursor() {
        let _g = serial();
        let path = tmp_path("fold");
        let mut c = CompletionCursor {
            epoch: 7,
            ..Default::default()
        };
        let out = process_batch_once(&mut c, &path, &batch(7, 10, 10, 5), |_| Handoff::ok());
        assert!(matches!(out, BatchOutcome::Acked));
        assert_eq!(c.dropped, 5);
        let b2 = CompletionBatch {
            epoch: 7,
            events: vec![],
            next_seq: 25,
            oldest_seq: 20,
            dropped: 5,
        };
        let out = process_batch_once(&mut c, &path, &b2, |_| Handoff::ok());
        assert!(matches!(out, BatchOutcome::Acked));
        assert_eq!(c.replay_lost, 9);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Q1 (blocking): a one-sided failure is a PARTIAL and the retry SKIPS the
    /// succeeded side (no dup), and the budget is BOUNDED with backoff.
    #[test]
    fn partial_retry_skips_the_succeeded_side_and_is_bounded() {
        let _g = serial();
        let path = tmp_path("partial");
        let mut c = CompletionCursor {
            epoch: 7,
            ..Default::default()
        };
        let out = process_batch_once(&mut c, &path, &batch(7, 2, 1, 0), |_| Handoff {
            netflow: true,
            lts2: false,
        });
        let BatchOutcome::Partial { flows, handoff } = out else {
            panic!("a one-sided failure must be Partial");
        };
        assert_eq!(flows.len(), 1);
        // The retry targets ONLY the failed side.
        assert_eq!(sides_to_retry(handoff), (false, true));
        assert_eq!(sides_to_retry(Handoff {
            netflow: false,
            lts2: true
        }), (true, false));
        assert_eq!((c.epoch, c.seq), (7, 0), "a partial is NOT acked");
        // Bounded: exactly PARTIAL_RETRY_ATTEMPTS delays, backoff capped.
        let mut budget = RetryBudget::new();
        let mut delays = Vec::new();
        while let Some(d) = budget.next_delay() {
            delays.push(d);
        }
        assert_eq!(delays.len() as u32, PARTIAL_RETRY_ATTEMPTS);
        assert_eq!(delays[0], DRAIN_INTERVAL);
        assert!(delays.iter().all(|d| *d <= MAX_BACKOFF));
        assert_eq!(*delays.last().unwrap(), MAX_BACKOFF);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// `merge_retry` keeps a succeeded side true and only records the retried
    /// side (a failed retry leaves the prior success intact).
    #[test]
    fn merge_retry_keeps_the_succeeded_side() {
        let _g = serial();
        let prior = Handoff {
            netflow: true,
            lts2: false,
        };
        let merged = merge_retry(
            prior,
            Handoff {
                netflow: true,
                lts2: true,
            },
        );
        assert!(merged.is_full(), "the retried side recovered");
        let still = merge_retry(
            prior,
            Handoff {
                netflow: true,
                lts2: false,
            },
        );
        assert!(still.netflow && !still.lts2, "the retry failed again");
    }

    /// A successful retry counts the export and ACKs (the exported path).
    #[test]
    fn retry_success_increments_exported() {
        let _g = serial();
        let path = tmp_path("retry-ok");
        let mut c = CompletionCursor {
            epoch: 7,
            ..Default::default()
        };
        let b = batch(7, 2, 1, 0);
        let flows = vec![event_to_flow_row(&event(1, 10, 20))];
        let before = health().exported;
        retry_succeeded(&mut c, &path, &b, &flows);
        assert_eq!(health().exported, before + 1);
        assert_eq!((c.epoch, c.seq), (7, 2), "the retried batch is ACKED");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Q1 (blocking): after the bounded retry is spent, the batch is ACKED, the
    /// failed side's loss is COUNTED, and DEGRADED latches (never false-healthy).
    #[test]
    fn retry_exhaustion_acks_counts_and_degrades() {
        let _g = serial();
        let path = tmp_path("exhaust");
        let mut c = CompletionCursor {
            epoch: 7,
            ..Default::default()
        };
        let b = batch(7, 2, 1, 0);
        let before_loss = health().failed_side_loss;
        exhaust_partial(&mut c, &path, &b, &[event_to_flow_row(&event(1, 10, 20))]);
        assert_eq!((c.epoch, c.seq), (7, 2), "the exhausted batch is ACKED");
        assert_eq!(health().failed_side_loss, before_loss + 1);
        assert!(health().degraded, "the degraded latch is set");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Q4: the SENT packet counts go to `sent_pkts_*` (what BOTH consumers
    /// read); `tcp_sent_packets*` = 0; `shaped` = 0; a `0` RTT median is ABSENT.
    #[test]
    fn event_maps_sent_packets_for_the_consumers() {
        let _g = serial();
        let row = event_to_flow_row(&event(1, 40, 0));
        assert_eq!(row.sent_pkts_down, 10);
        assert_eq!(row.sent_pkts_up, 5);
        assert_eq!(row.tcp_sent_packets_down, 0);
        assert_eq!(row.tcp_sent_packets_up, 0);
        assert_eq!(row.shaped, DirectionalCounters::default());
        assert_eq!(row.actual.download_bytes, 1_000);
        assert_eq!(row.state, FlowState::Reset);
        assert_eq!(row.tcp_retransmits, 3);
        assert_eq!(row.rtt_samples_down_us, vec![40]);
        assert!(row.rtt_samples_up_us.is_empty(), "a 0 median is ABSENT");
    }

    #[test]
    fn cursor_persists_and_reloads() {
        let _g = serial();
        let path = tmp_path("persist");
        let cursor = CompletionCursor {
            epoch: 3,
            seq: 42,
            replay_lost: 2,
            dropped: 5,
        };
        cursor.save(&path).unwrap();
        assert_eq!(CompletionCursor::load(&path), cursor);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
    /// Pins: a full ring, caught up, no gap, no failure => NOT degraded and
    /// completion_loss == 0; the bridge-side overwrite is informational only.
    #[test]
    fn full_ring_caught_up_is_not_degraded() {
        let _g = serial();
        let path = tmp_path("healthy");
        let mut c = CompletionCursor {
            epoch: 7,
            ..Default::default()
        };
        let b = CompletionBatch {
            epoch: 7,
            events: vec![event(1, 10, 20)],
            next_seq: 2,
            oldest_seq: 1,
            dropped: 3,
        };
        assert!(matches!(
            process_batch_once(&mut c, &path, &b, |_| Handoff::ok()),
            BatchOutcome::Acked
        ));
        let h = health();
        assert!(!h.degraded, "a caught-up full ring is NOT degraded");
        assert_eq!(h.completion_loss, 0);
        assert_eq!(h.overwritten, 3, "the overwrite is informational");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Pins: a cursor GAP counts replay_lost, degrades, and folds into loss.
    #[test]
    fn cursor_gap_sets_replay_lost_and_degrades() {
        let _g = serial();
        let path = tmp_path("gap");
        let mut c = CompletionCursor {
            epoch: 7,
            seq: 4,
            ..Default::default()
        };
        let b = CompletionBatch {
            epoch: 7,
            events: vec![],
            next_seq: 20,
            oldest_seq: 15,
            dropped: 0,
        };
        assert!(matches!(
            process_batch_once(&mut c, &path, &b, |_| Handoff::ok()),
            BatchOutcome::Acked
        ));
        let h = health();
        assert_eq!(h.replay_lost, 10, "expected 5..14 lost");
        assert!(h.degraded);
        assert_eq!(h.completion_loss, h.replay_lost);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Pins: a failed side (exhausted) counts completion_loss, NOT overwritten.
    #[test]
    fn failed_side_exhaustion_counts_loss_not_overwritten() {
        let _g = serial();
        let path = tmp_path("side");
        let mut c = CompletionCursor {
            epoch: 7,
            ..Default::default()
        };
        let b = CompletionBatch {
            epoch: 7,
            events: vec![event(1, 10, 20)],
            next_seq: 2,
            oldest_seq: 1,
            dropped: 0,
        };
        exhaust_partial(&mut c, &path, &b, &[event_to_flow_row(&event(1, 10, 20))]);
        let h = health();
        assert_eq!(h.failed_side_loss, 1);
        assert_eq!(h.overwritten, 0);
        assert_eq!(h.completion_loss, 1);
        assert!(h.degraded);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Pins: an epoch RE-BASE is not a loss and does not degrade; the FIRST
    /// real new-epoch batch carries a pre-existing `dropped: 3`, which folds as
    /// INFORMATIONAL overwritten — not loss, not degraded.
    #[test]
    fn epoch_rebase_is_not_loss_or_degraded() {
        let _g = serial();
        let path = tmp_path("epoch-health");
        let mut c = CompletionCursor {
            epoch: 3,
            seq: 40,
            dropped: 5,
            replay_lost: 0,
        };
        // (1) the epoch-change signal (flash_1's next_seq=0 mismatch) ⇒ re-base.
        let rebase = CompletionBatch {
            epoch: 9,
            events: vec![event(1, 10, 20)],
            next_seq: 0,
            oldest_seq: 0,
            dropped: 0,
        };
        assert!(matches!(
            process_batch_once(&mut c, &path, &rebase, |_| Handoff::ok()),
            BatchOutcome::Acked
        ));
        assert_eq!((c.epoch, c.seq, c.dropped), (9, 0, 0));
        // (2) the FIRST real new-epoch batch carries a pre-existing overwritten
        // of 3 — it must fold INFORMATIONAL (not loss, not degraded).
        let first = CompletionBatch {
            epoch: 9,
            events: vec![event(1, 10, 20)],
            next_seq: 5,
            oldest_seq: 1,
            dropped: 3,
        };
        assert!(matches!(
            process_batch_once(&mut c, &path, &first, |_| Handoff::ok()),
            BatchOutcome::Acked
        ));
        let h = health();
        assert!(!h.degraded, "an informational overwrite must NOT degrade");
        assert_eq!(h.completion_loss, 0, "an overwrite is NOT loss");
        assert_eq!(
            h.overwritten, 3,
            "the pre-existing overwritten folds informational"
        );
        assert_eq!((c.epoch, c.seq, c.dropped), (9, 5, 3));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

}

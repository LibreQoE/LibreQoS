use crate::{
    AnalyticsSnapshot, CaptureBatch, CompletionBatch, HealthReport, MAX_QUEUE_STATS_IDS,
    MAX_WATCH_ENTRIES, QueueStatsReply, WatchAck,
};
use nix::cmsg_space;
use nix::sys::socket::{ControlMessage, ControlMessageOwned, MsgFlags, recvmsg, sendmsg};
use serde::{Deserialize, Serialize};
use shmem_ipc::mem::{mfd::Memfd, read_memfd, write_once};
use std::fs;
use std::io::{IoSlice, IoSliceMut, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

pub const DEFAULT_MAX_ROWS: usize = 10_000;
const MAX_REQUEST_BYTES: usize = 64;
/// Heimdall re-derivation §1.3b: `SetWatchList` cannot ride the 64-byte
/// request frame (a ≤64-entry list is ≥512 B), so the kind carries its OWN
/// request-frame bound. The reply envelope (MAX_SNAPSHOT_BYTES) is unchanged.
pub const MAX_WATCH_REQUEST_BYTES: usize = 1024;
/// K2 slice 2 §7: `QueueStats` carries a bounded slot-id list (≤64), so like
/// `SetWatchList` it gets its own 1 KiB-class request-frame bound.
pub const MAX_QUEUE_STATS_REQUEST_BYTES: usize = 1024;
/// The flow-completion drain kinds carry `epoch` + `after_seq` (two u64s) on
/// top of `limit`, so they get a slightly wider request-frame bound.
pub const MAX_COMPLETIONS_REQUEST_BYTES: usize = 256;
/// Directional-wire headroom raise: the fully-populated snapshot bound is
/// hosts + active + recent = 3 × [`DEFAULT_MAX_ROWS`] rows; the per-direction
/// RTT pair + six u64 counters add ~4.1 MiB at that bound. The pre-change
/// 8 MiB envelope already could not hold the theoretical maximum; 32 MiB
/// clears the measured 19.8 MiB worst case with margin. DISCLOSED
/// protocol-constant change (the memfd is sized to the actual reply, so
/// steady-state allocation is unaffected).
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;

fn default_limit() -> usize {
    1
}

/// `serde(skip_serializing_if)` helper: a zero cursor is omitted from the
/// frame (so the pre-existing kinds' frames do not grow and the 64-byte
/// request bound still holds).
fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("socket error: {0}")]
    Socket(#[from] nix::Error),
    #[error("CBOR error: {0}")]
    Cbor(#[from] serde_cbor::Error),
    #[error("shared-memory error: {0}")]
    SharedMemory(#[from] shmem_ipc::Error),
    #[error("snapshot row limit must be between 1 and {DEFAULT_MAX_ROWS}")]
    InvalidLimit,
    #[error("watch set must be 1..={MAX_WATCH_ENTRIES} entries and never contain slot 0")]
    InvalidWatchEntry,
    #[error("queue-stats slot list must be 0..={MAX_QUEUE_STATS_IDS} entries and never contain slot 0")]
    InvalidQueueStatsEntry,
    #[error("malformed IPC frame")]
    MalformedFrame,
    /// `lqos-74t.15.3` MalformedFrame localization: the applied-state read
    /// (`connect_state`) rejected the frame at a NAMED sub-arm — so the broker's
    /// log says WHICH of the checks failed and the request KIND, vs the generic
    /// [`Self::MalformedFrame`]. Diagnostic only; no behavior change.
    #[error("malformed applied-state frame at sub-arm `{arm}` (kind {kind:?})")]
    MalformedStateFrame {
        /// The named sub-check that rejected the frame.
        arm: &'static str,
        /// The request kind the frame answered.
        kind: RequestKind,
    },
    #[error("snapshot request queue is busy")]
    Busy,
    #[error("collector did not answer the snapshot request")]
    CollectorUnavailable,
    /// `.59` localization: the collector CHANNEL is alive but it did not reply
    /// within the timeout. The collector loop is WORK-limited (it drains the
    /// request queue every iteration; the only throttle is the 1 ms sleep), so
    /// a timeout means the collector had not REACHED the drain (a pre/post-drain
    /// phase consumed the whole timeout). Distinct from
    /// [`Self::CollectorSendGone`] / [`Self::CollectorReplyDropped`], so the
    /// capture names WHICH.
    #[error("collector alive but did not answer within the timeout (collector had not drained)")]
    CollectorTimeout,
    /// `.59` split — SEND path: `try_send` found the capacity-1 receiver gone,
    /// i.e. the collector thread had already exited (or never started) when the
    /// request was offered; the request never entered the queue. Named distinct
    /// from [`Self::CollectorReplyDropped`] (reply path) so the capture says
    /// WHICH side of the channel disconnected.
    #[error("collector receiver gone at send (collector thread not running)")]
    CollectorSendGone,
    /// `.59` split — REPLY path: the request WAS enqueued, but its reply sender
    /// dropped before answering — the collector dropped the queued request
    /// undrained (e.g. `run_collector` returned / the thread exited while the
    /// request sat in the slot). This is the "collector died with a request
    /// queued" signal, distinct from [`Self::CollectorSendGone`].
    #[error("collector dropped the queued request without replying (reply sender gone)")]
    CollectorReplyDropped,
    /// R3: the bridge answered the state request with a typed
    /// unavailable reason (no report yet / over the envelope bound).
    #[error("applied-state report unavailable: {0:?}")]
    StateUnavailable(StateUnavailableReason),
}

#[derive(Debug)]
pub struct SnapshotRequest {
    pub limit: usize,
    pub reply: SyncSender<AnalyticsSnapshot>,
}

#[derive(Clone, Debug)]
pub struct SnapshotRequester {
    tx: SyncSender<SnapshotRequest>,
    timeout: Duration,
}

impl SnapshotRequester {
    pub fn request(&self, limit: usize) -> Result<AnalyticsSnapshot, IpcError> {
        validate_limit(limit)?;
        let (reply, rx) = sync_channel(1);
        self.tx
            .try_send(SnapshotRequest { limit, reply })
            .map_err(|error| match error {
                TrySendError::Full(_) => IpcError::Busy,
                TrySendError::Disconnected(_) => IpcError::CollectorUnavailable,
            })?;
        rx.recv_timeout(self.timeout)
            .map_err(|_| IpcError::CollectorUnavailable)
    }
}

/// Creates capacity-one request plumbing. The collector calls `try_recv` and
/// `reply.try_send(snapshot)`; it never waits for IPC or the requester.
pub fn snapshot_channel(timeout: Duration) -> (SnapshotRequester, Receiver<SnapshotRequest>) {
    let (tx, rx) = sync_channel(1);
    (SnapshotRequester { tx, timeout }, rx)
}

/// One pending health request (supervision contract §2A). Same capacity-1
/// never-block-the-collector plumbing as snapshots.
#[derive(Debug)]
pub struct HealthRequest {
    pub reply: SyncSender<HealthReport>,
}

/// Health counterpart of [`SnapshotRequester`] (capacity-1, bounded wait).
#[derive(Clone, Debug)]
pub struct HealthRequester {
    tx: SyncSender<HealthRequest>,
    timeout: Duration,
}

impl HealthRequester {
    pub fn request(&self) -> Result<HealthReport, IpcError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .try_send(HealthRequest { reply })
            .map_err(|error| match error {
                TrySendError::Full(_) => IpcError::Busy,
                TrySendError::Disconnected(_) => IpcError::CollectorUnavailable,
            })?;
        rx.recv_timeout(self.timeout)
            .map_err(|_| IpcError::CollectorUnavailable)
    }
}

/// Creates the capacity-one health request plumbing (the collector polls
/// `try_recv` and `reply.try_send`; it never waits).
pub fn health_channel(timeout: Duration) -> (HealthRequester, Receiver<HealthRequest>) {
    let (tx, rx) = sync_channel(1);
    (HealthRequester { tx, timeout }, rx)
}

/// R3 (remove_lpm_completeness_card §R3): the bridge's fence-committed
/// applied-state report. The `transaction` bytes are OPAQUE here — CBOR
/// of the shared crate's `DataplaneTransaction` as serialized by the
/// bridge — because this crate deliberately carries NO dataplane-types
/// dependency; the broker parses with its own shared-crate types.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateReport {
    /// The bridge's last-ACKED generation (visibility/log alignment
    /// only — fence semantics live on the seam, never here).
    pub generation: String,
    /// The report-envelope version (1).
    pub report_schema: u32,
    /// CBOR bytes of the applied `DataplaneTransaction`.
    pub transaction: Vec<u8>,
    /// R3c (remove_lpm_completeness_card §R3c-2): CBOR of the live
    /// circuit NAMES in SLOT order (Vec<String>) — the bridge's
    /// authoritative slot table, the order source for the broker's
    /// drain pops AND view construction. Opaque bytes; serde default
    /// keeps pre-R3c brokers interoperable (they fall back to the
    /// report's transaction order + the named urgent).
    #[serde(default)]
    pub slot_order: Vec<u8>,
    /// WP3 (`docs/wp2_wp3_fence_convergence_and_adopt_card.md` §2): the
    /// semantic CONTENT KEY of the applied transaction
    /// (`lqos_dataplane_types::content_key`). Carried TYPED so the broker's
    /// adopt is a PURE function of the report (`generation`, `content_key`,
    /// `slot_order`) — it never has to re-derive the key from the opaque
    /// `transaction` bytes (and cannot drift from the bridge's).
    ///
    /// Additive + trailing + `#[serde(default)]`: a pre-WP3 bridge's report
    /// (with no `content_key`) still decodes; the broker falls back to
    /// deriving the key from `transaction`.
    #[serde(default)]
    pub content_key: String,
    /// `lqos-101` late-adopt identity: the DESIRED world's content key carried
    /// VERBATIM on the publish frame (`desired_world_content_key`). The
    /// broker's late-adopt guard compares its artifact key to THIS field; the
    /// folded `content_key` above remains the applied-world/cache identity
    /// (the identity-keyed fold can collapse a duplicate mapping op, so the
    /// two keys can legitimately differ). Additive + trailing + serde default:
    /// an old report decodes with an empty field, and the broker falls back to
    /// `content_key` (old behavior). No report_schema bump.
    #[serde(default)]
    pub desired_world_content_key: String,
    /// §8 (`docs/lqos_conf_single_source_card.md`): the bridge's APPLIED
    /// circuit-slot capacity (the derived value the world was built with).
    /// Additive + `#[serde(default)]`; `None` = a pre-§8 bridge. The EMITTER
    /// (lqosd) reads it so `max(next_power_of_two(n), 1000, applied)` never
    /// shrinks across a bridge restart (the emitter has no other source).
    #[serde(default)]
    pub applied_circuit_capacity: Option<u32>,
    /// `lqos-8h8` (a): the per-circuit PLACEMENT OWNER (worker id) in SLOT order
    /// — CBOR of a `Vec<u16>`, PARALLEL to `slot_order`. The control-plane
    /// broker reads it so it can call the SHARED owner-set predicates
    /// (`lqos_dataplane_types::compute_owner_sets` /
    /// `policy3_owner_set_violation`) on a reparent WITHOUT forking the
    /// placement. Additive + trailing + `#[serde(default)]`: a pre-owner
    /// bridge's report still decodes; empty = the broker's reparent twin is
    /// inert (no span classification).
    #[serde(default)]
    pub owners: Vec<u8>,
    /// `lqos-74t.118.1` bridge worker-affinity report. Additive + trailing +
    /// `#[serde(default)]`: an old bridge sends `None` (explicit unsupported),
    /// and an old consumer ignores the unknown field. The report is static
    /// boot data (worker id, lcore/NUMA, per-port queue indices); per-worker
    /// counters are explicitly `Unsupported` in v1.
    #[serde(default)]
    pub workers: Option<WorkerAffinityReport>,
}

/// `lqos-74t.118.1`: additive bridge worker-affinity report carried on
/// [`StateReport`]. All affinity fields are `Option`; `None` means the source
/// is unsupported/not yet registered, never a fabricated zero.
///
/// `rx_queue_ids`/`tx_queue_id` are DPDK per-port queue **indices**; the same
/// index is valid on both logical ports (no port is implied).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerAffinityReport {
    /// Report schema (1).
    pub schema: u32,
    /// Configured RX queue count (`None` = the producer could not report it).
    #[serde(default)]
    pub rx_queue_count: Option<u16>,
    /// Configured TX queue count (`None` = the producer could not report it).
    #[serde(default)]
    pub tx_queue_count: Option<u16>,
    /// Whether per-worker counters are carried. v1 always reports
    /// [`WorkerCounterSupport::Unsupported`]; the enum reserves `Available`
    /// for a future cumulative-packet producer (units: packets since worker
    /// start; an interval rate is derived by the consumer).
    pub counters: WorkerCounterSupport,
    /// One row per configured worker.
    pub workers: Vec<WorkerAffinity>,
}

/// Per-worker counter support level. `Unsupported` is the honest v1 value;
/// `Available` is reserved and must only be emitted with real cumulative
/// counters, never zeros.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerCounterSupport {
    Unsupported,
    Available,
}

/// One bridge worker's affinity row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerAffinity {
    /// DPDK worker/queue id (`0..worker_count`).
    pub worker_id: u16,
    /// DPDK lcore id; `None` = not yet registered / unsupported.
    #[serde(default)]
    pub lcore_id: Option<u32>,
    /// NUMA socket id; `None` = not yet registered / unsupported.
    #[serde(default)]
    pub numa_node: Option<i32>,
    /// Owned RX queue indices; `None` = unsupported, `Some([])` = supported
    /// with no RX queue assigned to this worker. Per-port on both logical
    /// ports.
    #[serde(default)]
    pub rx_queue_ids: Option<Vec<u16>>,
    /// TX queue index; `None` = unsupported. Per-port on both logical ports.
    #[serde(default)]
    pub tx_queue_id: Option<u16>,
    /// True when this worker shares its TX queue with another worker
    /// (`tx_queue_count < worker_count`); `None` = unknown.
    #[serde(default)]
    pub shared_tx_queue: Option<bool>,
    /// Per-worker cumulative counters; `None` = unsupported (v1).
    #[serde(default)]
    pub counters: Option<WorkerCounters>,
}

/// Cumulative per-worker counters. Units: packets since worker start.
/// Reserved for a future producer; v1 reports `counters: None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerCounters {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub drop_packets: u64,
}

/// Why no applied-state report is available. Instance-binding is the
/// design: the slot is empty until THIS instance's first fence commits
/// (a stale record from a previous instance is never served).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateUnavailableReason {
    /// This instance has not fence-committed any publish yet.
    NoReportYet,
    /// The report exceeds the reply-envelope protocol bound
    /// (MAX_SNAPSHOT_BYTES) — raising it means changing the protocol
    /// constant, not the transport.
    TooLarge,
}

/// The collector's answer to one state request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateReply {
    Report(StateReport),
    Unavailable(StateUnavailableReason),
}

/// `.15.3` diag (the MalformedFrame localization): the serialized State-report
/// length + a re-parse verdict, computed with the SAME codec `serve_one` uses
/// (`serde_cbor::to_vec`). `bytes = None` = the report exceeds the reply bound
/// (the typed `TooLarge`, never a truncated frame).
pub struct StateReportDiag {
    /// The CBOR length `serve_one` would place in the memfd, or `None` when it
    /// exceeds `MAX_SNAPSHOT_BYTES` (typed `TooLarge`).
    pub bytes: Option<usize>,
    /// Whether the just-serialized bytes re-parse as a `StateReport`.
    pub reparse_ok: bool,
}

/// Compute the [`StateReportDiag`] for a report (the diag entry point;
/// bridge_rs never needs a direct `serde_cbor` dependency). `bytes` is the
/// **BARE `StateReport`** length — the real memfd frame wraps it in
/// `StateReply::Report(report)` (a ~1-2 byte enum tag), so `bytes` is a close
/// lower bound; `reparse_ok` is unaffected.
pub fn state_report_diag(report: &StateReport) -> Result<StateReportDiag, IpcError> {
    let bytes = serde_cbor::to_vec(report)?;
    let over = bytes.len() > MAX_SNAPSHOT_BYTES;
    let reparse_ok = serde_cbor::from_slice::<StateReport>(&bytes).is_ok();
    Ok(StateReportDiag {
        bytes: if over { None } else { Some(bytes.len()) },
        reparse_ok,
    })
}

/// One pending applied-state request (R3). Same capacity-1
/// never-block-the-collector plumbing as snapshots/health.
#[derive(Debug)]
pub struct StateRequest {
    pub reply: SyncSender<StateReply>,
    /// `.59` collector-starvation instrument: the instant the requester
    /// ENQUEUED this request. The collector measures its `try_recv` against
    /// this to get the enqueue→drain latency (a capacity-1 queue drained at
    /// most once per collector iteration). Cold path; no protocol semantics.
    pub enqueued_at: Instant,
}

/// Applied-state counterpart of [`HealthRequester`] (capacity-1,
/// bounded wait).
#[derive(Clone, Debug)]
pub struct StateRequester {
    tx: SyncSender<StateRequest>,
    timeout: Duration,
}

impl StateRequester {
    pub fn request(&self) -> Result<StateReply, IpcError> {
        let (reply, rx) = sync_channel(1);
        let enqueued_at = Instant::now();
        self.tx
            .try_send(StateRequest { reply, enqueued_at })
            .map_err(|error| match error {
                TrySendError::Full(_) => IpcError::Busy,
                // `.59` SEND path: the receiver was already gone.
                TrySendError::Disconnected(_) => IpcError::CollectorSendGone,
            })?;
        // `.59` localization. TIMEOUT vs DISCONNECTED, and (new) which side of
        // the channel disconnected.
        rx.recv_timeout(self.timeout).map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => {
                // `.59` STALE-SLOT event: the request was enqueued (the
                // `try_send` above succeeded) but no reply arrived. If the
                // collector had drained it, `service_state_request` would have
                // `try_send`-ed a reply on an EMPTY, still-open capacity-1 reply
                // channel — which succeeds — so a timeout PROVES the collector
                // had not reached the drain by the deadline. `recv_timeout` does
                // NOT remove the request, so it REMAINS in the capacity-1 slot
                // and the next request reads `Busy` until the collector drains
                // it. This is the source behind the `Busy` burst.
                eprintln!(
                    "analytics State arm STALE SLOT: the request timed out after {:?} and \
                     REMAINS queued (the collector had not drained it); the next request reads \
                     Busy until it does",
                    self.timeout
                );
                IpcError::CollectorTimeout
            }
            // `.59` REPLY path: the request was enqueued but its reply sender
            // dropped before answering — the collector dropped it undrained
            // (thread exit with the request queued).
            std::sync::mpsc::RecvTimeoutError::Disconnected => IpcError::CollectorReplyDropped,
        })
    }
}

/// Creates the capacity-one state request plumbing (the collector polls
/// `try_recv` and `reply.try_send`; it never waits).
pub fn state_channel(timeout: Duration) -> (StateRequester, Receiver<StateRequest>) {
    let (tx, rx) = sync_channel(1);
    (StateRequester { tx, timeout }, rx)
}

/// One pending watch-set request (Heimdall re-derivation §1.3b). The
/// collector — the single writer of the shared watch region — applies the
/// set and replies with the accept/reject counts.
#[derive(Debug)]
pub struct WatchRequest {
    /// Leaf slot ids to watch (empty clears the set).
    pub watch: Vec<u64>,
    /// One-shot reply channel.
    pub reply: SyncSender<WatchAck>,
}

/// Watch-set counterpart of [`StateRequester`] (capacity-1, bounded wait).
#[derive(Clone, Debug)]
pub struct WatchRequester {
    tx: SyncSender<WatchRequest>,
    timeout: Duration,
}

impl WatchRequester {
    pub fn request(&self, watch: &[u64]) -> Result<WatchAck, IpcError> {
        validate_watch(watch)?;
        let (reply, rx) = sync_channel(1);
        self.tx
            .try_send(WatchRequest {
                watch: watch.to_vec(),
                reply,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => IpcError::Busy,
                TrySendError::Disconnected(_) => IpcError::CollectorUnavailable,
            })?;
        rx.recv_timeout(self.timeout)
            .map_err(|_| IpcError::CollectorUnavailable)
    }
}

/// Creates the capacity-one watch request plumbing (the collector polls
/// `try_recv` and `reply.try_send`; it never waits).
pub fn watch_channel(timeout: Duration) -> (WatchRequester, Receiver<WatchRequest>) {
    let (tx, rx) = sync_channel(1);
    (WatchRequester { tx, timeout }, rx)
}

/// One pending capture-drain request (Heimdall re-derivation §1.3a).
#[derive(Debug)]
pub struct DrainRequest {
    /// Maximum records to return in one reply.
    pub limit: usize,
    /// One-shot reply channel.
    pub reply: SyncSender<CaptureBatch>,
}

/// Capture-drain counterpart of [`StateRequester`] (capacity-1, bounded wait).
#[derive(Clone, Debug)]
pub struct DrainRequester {
    tx: SyncSender<DrainRequest>,
    timeout: Duration,
}

impl DrainRequester {
    pub fn request(&self, limit: usize) -> Result<CaptureBatch, IpcError> {
        validate_limit(limit)?;
        let (reply, rx) = sync_channel(1);
        self.tx
            .try_send(DrainRequest { limit, reply })
            .map_err(|error| match error {
                TrySendError::Full(_) => IpcError::Busy,
                TrySendError::Disconnected(_) => IpcError::CollectorUnavailable,
            })?;
        rx.recv_timeout(self.timeout)
            .map_err(|_| IpcError::CollectorUnavailable)
    }
}

/// Creates the capacity-one drain request plumbing (the collector polls
/// `try_recv` and `reply.try_send`; it never waits).
pub fn drain_channel(timeout: Duration) -> (DrainRequester, Receiver<DrainRequest>) {
    let (tx, rx) = sync_channel(1);
    (DrainRequester { tx, timeout }, rx)
}

/// One pending completion-ring drain request (the flow-completion feed).
#[derive(Debug)]
pub struct DrainCompletionsRequest {
    /// The epoch the caller's cursor is from (a change ⇒ the bridge restarted).
    pub epoch: u64,
    /// Drain events with `seq > after_seq`.
    pub after_seq: u64,
    /// Maximum events to return in one reply.
    pub limit: usize,
    /// One-shot reply channel.
    pub reply: SyncSender<CompletionBatch>,
}

/// Completion-drain counterpart of [`DrainRequester`] (capacity-1, bounded wait).
#[derive(Clone, Debug)]
pub struct DrainCompletionsRequester {
    tx: SyncSender<DrainCompletionsRequest>,
    timeout: Duration,
}

impl DrainCompletionsRequester {
    /// Drain the completion ring after `after_seq` (at most `limit` events).
    pub fn request(
        &self,
        epoch: u64,
        after_seq: u64,
        limit: usize,
    ) -> Result<CompletionBatch, IpcError> {
        validate_limit(limit)?;
        let (reply, rx) = sync_channel(1);
        self.tx
            .try_send(DrainCompletionsRequest {
                epoch,
                after_seq,
                limit,
                reply,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => IpcError::Busy,
                TrySendError::Disconnected(_) => IpcError::CollectorUnavailable,
            })?;
        rx.recv_timeout(self.timeout)
            .map_err(|_| IpcError::CollectorUnavailable)
    }
}

/// Creates the capacity-one completion-drain plumbing (the collector polls
/// `try_recv` and `reply.try_send`; it never waits).
pub fn drain_completions_channel(
    timeout: Duration,
) -> (
    DrainCompletionsRequester,
    Receiver<DrainCompletionsRequest>,
) {
    let (tx, rx) = sync_channel(1);
    (DrainCompletionsRequester { tx, timeout }, rx)
}

/// One pending qdisc-detail request (K2 slice 2 §7).
#[derive(Debug)]
pub struct QueueStatsRequest {
    /// The circuit slot ids to render (bounded by [`MAX_QUEUE_STATS_IDS`]).
    pub slots: Vec<u64>,
    /// One-shot reply channel.
    pub reply: SyncSender<QueueStatsReply>,
}

/// Qdisc-detail counterpart of [`DrainRequester`] (capacity-1, bounded wait).
#[derive(Clone, Debug)]
pub struct QueueStatsRequester {
    tx: SyncSender<QueueStatsRequest>,
    timeout: Duration,
}

impl QueueStatsRequester {
    pub fn request(&self, slots: &[u64]) -> Result<QueueStatsReply, IpcError> {
        validate_queue_stats(slots)?;
        let (reply, rx) = sync_channel(1);
        self.tx
            .try_send(QueueStatsRequest {
                slots: slots.to_vec(),
                reply,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => IpcError::Busy,
                TrySendError::Disconnected(_) => IpcError::CollectorUnavailable,
            })?;
        rx.recv_timeout(self.timeout)
            .map_err(|_| IpcError::CollectorUnavailable)
    }
}

/// Creates the capacity-one qdisc-detail request plumbing (the collector
/// polls `try_recv` and `reply.try_send`; it never waits).
pub fn queue_stats_channel(timeout: Duration) -> (QueueStatsRequester, Receiver<QueueStatsRequest>) {
    let (tx, rx) = sync_channel(1);
    (QueueStatsRequester { tx, timeout }, rx)
}

/// §1.1: an empty set is legal (it clears the watches); slot 0
/// (`CircuitId::best_effort()`) is reserved and never settable.
fn validate_watch(watch: &[u64]) -> Result<(), IpcError> {
    if watch.len() > MAX_WATCH_ENTRIES || watch.contains(&0) {
        return Err(IpcError::InvalidWatchEntry);
    }
    Ok(())
}

/// K2 slice 2 §7: the QueueStats slot list is bounded by
/// [`MAX_QUEUE_STATS_IDS`]; slot 0 (`CircuitId::best_effort()`) has no leaf
/// qdisc and is never requestable. An empty list is legal (empty reply).
fn validate_queue_stats(slots: &[u64]) -> Result<(), IpcError> {
    if slots.len() > MAX_QUEUE_STATS_IDS || slots.contains(&0) {
        return Err(IpcError::InvalidQueueStatsEntry);
    }
    Ok(())
}

/// Wire request kind (supervision contract §7 LOW-2: the tag field ships
/// TOGETHER with the existing fields; serde defaults keep pre-health
/// clients (`{"limit": N}`) and pre-health servers interoperable in both
/// directions — an old server simply ignores the tag and serves a
/// snapshot).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    #[default]
    Snapshot,
    Health,
    /// R3: the applied-state report request.
    State,
    /// Heimdall re-derivation §1.3b: replace the shared watch-set image.
    /// Carries its own request-frame bound ([`MAX_WATCH_REQUEST_BYTES`]).
    SetWatchList,
    /// Heimdall re-derivation §1.3a: drain the per-worker capture rings.
    DrainCapture,
    /// K2 slice 2 §7: per-circuit qdisc detail for a bounded slot-id list.
    /// Carries its own request-frame bound ([`MAX_QUEUE_STATS_REQUEST_BYTES`]).
    QueueStats,
    /// The flow-completion feed: drain the bridge's completion ring after the
    /// caller's `(epoch, after_seq)` cursor. ADDITIVE.
    DrainCompletions,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReplyKind {
    #[default]
    Snapshot,
    Health,
    State,
    SetWatchList,
    DrainCapture,
    QueueStats,
    DrainCompletions,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireRequest {
    #[serde(default)]
    kind: RequestKind,
    #[serde(default = "default_limit")]
    limit: usize,
    /// Heimdall re-derivation §1.3b: the watch set (leaf slot ids). Empty
    /// for non-watch kinds; serde-defaulted for wire compatibility.
    #[serde(default)]
    watch: Vec<u64>,
    /// K2 slice 2 §7: the QueueStats slot-id list. Empty for non-QueueStats
    /// kinds; serde-defaulted for wire compatibility.
    #[serde(default)]
    queue_slots: Vec<u64>,
    /// The completion-drain cursor (flow-completion feed). Serde-defaulted so
    /// an OLD lqosd request (no cursor) still deserializes on a NEW bridge.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    epoch: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    after_seq: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireReply {
    snapshot_len: usize,
    #[serde(default)]
    kind: ReplyKind,
}

pub struct SnapshotService {
    listener: UnixListener,
    socket_path: PathBuf,
    /// The inode of the socket file this service created — the ownership
    /// token for the self-heal + the hardened unlink (`bridge_x_selfheal_card`
    /// §2). A path whose inode differs is a foreign/recreated socket.
    bound_inode: u64,
    requester: SnapshotRequester,
    health_requester: HealthRequester,
    state_requester: StateRequester,
    watch_requester: WatchRequester,
    drain_requester: DrainRequester,
    queue_stats_requester: QueueStatsRequester,
    /// The flow-completion feed's ring requester (flow_completion_feed_card
    /// §2). `None` = no ring in this build ⇒ `DrainCompletions` reports
    /// `CollectorUnavailable` and the consumer falls back to the snapshot.
    completions_requester: Option<DrainCompletionsRequester>,
}

impl SnapshotService {
    /// Binds the analytics socket. HIGH-1 (supervision contract §7): under
    /// bridge-root/lqosd-libreqos, mode-only permissions leave the socket
    /// root:root 0600 and lqosd can NEVER connect (invisible in the
    /// all-root lab). The root bridge CHOWNS the socket to the `libreqos`
    /// user at bind — fail-visible degrade when the user is absent — and
    /// checks the parent-dir exec path (mirrors control_channel.rs's G5
    /// pattern exactly; NO new privilege grant).
    ///
    /// This legacy path serves snapshot/health/state only. Heimdall
    /// capture kinds answer `CollectorUnavailable` until the bridge
    /// switches to [`SnapshotService::bind_with_capture`].
    pub fn bind(
        path: impl AsRef<Path>,
        requester: SnapshotRequester,
        health_requester: HealthRequester,
        state_requester: StateRequester,
    ) -> Result<Self, IpcError> {
        let (watch_requester, _watch_rx) = watch_channel(Duration::from_secs(1));
        let (drain_requester, _drain_rx) = drain_channel(Duration::from_secs(1));
        let (queue_stats_requester, _queue_stats_rx) = queue_stats_channel(Duration::from_secs(1));
        Self::bind_with_capture(
            path,
            requester,
            health_requester,
            state_requester,
            watch_requester,
            drain_requester,
            queue_stats_requester,
        )
    }

    /// Binds the analytics socket with the Heimdall capture kinds wired
    /// (§1.3). The watch requester's collector owns the move-only watch
    /// region writer; the drain requester's collector owns the per-worker
    /// capture ring consumers; the queue-stats requester's collector renders
    /// the per-circuit qdisc detail (K2 slice 2 §7).
    pub fn bind_with_capture(
        path: impl AsRef<Path>,
        requester: SnapshotRequester,
        health_requester: HealthRequester,
        state_requester: StateRequester,
        watch_requester: WatchRequester,
        drain_requester: DrainRequester,
        queue_stats_requester: QueueStatsRequester,
    ) -> Result<Self, IpcError> {
        let path = path.as_ref();
        // F4/L2 (LKG-boot parity card §2): an unclean death leaves the
        // socket file behind and the next bind FATALs "Address already in
        // use" — a crash→restart loop. Unlink-first: the bridge is
        // single-instance, so a live peer socket is not a protected case
        // (clean shutdown removes the file in Drop).
        let _ = fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        adopt_for_libreqos(path);
        let bound_inode = path_inode(path).unwrap_or(0);
        Ok(Self {
            listener,
            socket_path: path.to_owned(),
            bound_inode,
            requester,
            health_requester,
            state_requester,
            watch_requester,
            drain_requester,
            queue_stats_requester,
            completions_requester: None,
        })
    }

    /// Wires the flow-completion ring (the `DrainCompletions` feed). Additive:
    /// a service without it keeps answering `CollectorUnavailable`.
    pub fn with_completions_requester(
        mut self,
        requester: DrainCompletionsRequester,
    ) -> Self {
        self.completions_requester = Some(requester);
        self
    }

    pub fn serve(mut self) -> Result<(), IpcError> {
        // Self-heal (`bridge_x_selfheal_card` §2): the socket may be unlinked
        // mid-life by an unrelated actor. Poll the accept path on a low-rate
        // cadence, verify the path still resolves to OUR inode, and re-bind
        // when it has vanished. A transient accept error must NOT terminate
        // the service (the legacy `?`-style return killed the collector on a
        // single EMFILE/ECONNABORTED, never to recover).
        self.listener
            .set_nonblocking(true)
            .map_err(IpcError::Io)?;
        let mut next_check = Instant::now() + SELF_HEAL_INTERVAL;
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    // `.59`: a DISCARDED `serve_one` error made a State-arm
                    // failure SILENT — the bridge sent NO reply (hence no
                    // SCM_RIGHTS) and the broker only saw
                    // `no_scm_rights_fd`/`peer_closed_no_bytes`. Log it, never
                    // discard (the serve loop must keep serving regardless).
                    if let Err(error) = serve_one(
                        &mut stream,
                        &self.requester,
                        &self.health_requester,
                        &self.state_requester,
                        &self.watch_requester,
                        &self.drain_requester,
                        &self.queue_stats_requester,
                        self.completions_requester.as_ref(),
                    ) {
                        eprintln!(
                            "analytics serve_one error (path={}): {error} ({error:?})",
                            self.socket_path.display()
                        );
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= next_check {
                        self.self_heal();
                        next_check = Instant::now() + SELF_HEAL_INTERVAL;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(_) => {
                    // Transient accept error: keep serving; retry next pass.
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }

    /// Re-binds the socket when its path has VANISHED. Ownership-checked: a
    /// path that exists with a foreign inode is never unlinked (another
    /// instance owns it); it is only reported.
    fn self_heal(&mut self) {
        let ino = path_inode(&self.socket_path);
        if ino == Some(self.bound_inode) {
            return; // still ours
        }
        if ino.is_some() {
            eprintln!(
                "WARN: analytics socket {} was replaced by a foreign inode {:?}; not touching it",
                self.socket_path.display(),
                ino
            );
            return;
        }
        // Vanished: re-bind (there is nothing of ours left on disk).
        match UnixListener::bind(&self.socket_path) {
            Ok(listener) => {
                let _ = listener.set_nonblocking(true);
                if fs::set_permissions(&self.socket_path, fs::Permissions::from_mode(0o600)).is_ok()
                {
                    adopt_for_libreqos(&self.socket_path);
                }
                self.listener = listener;
                self.bound_inode = path_inode(&self.socket_path).unwrap_or(0);
                eprintln!(
                    "WARN: analytics socket {} vanished; re-bound (inode {})",
                    self.socket_path.display(),
                    self.bound_inode
                );
            }
            Err(error) => {
                eprintln!(
                    "WARN: analytics socket self-heal failed to re-bind {}: {error:?}",
                    self.socket_path.display()
                );
            }
        }
    }

    pub fn local_addr_path(&self) -> &Path {
        &self.socket_path
    }
}

/// Returns the inode a socket path currently resolves to, when it exists.
fn path_inode(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|meta| meta.ino())
}

/// Self-heal poll cadence (`bridge_x_selfheal_card` §2).
const SELF_HEAL_INTERVAL: Duration = Duration::from_secs(1);

impl Drop for SnapshotService {
    fn drop(&mut self) {
        // Hardened unlink (`bridge_x_selfheal_card` §2.2): remove the path
        // ONLY when it still resolves to the inode this service created; a
        // foreign/recreated socket is never unlinked.
        if path_inode(&self.socket_path) == Some(self.bound_inode) {
            let _ = fs::remove_file(&self.socket_path);
        }
    }
}

// Pre-existing IPC loop (8 args / 7 threshold); targeted allow.
#[allow(clippy::too_many_arguments)]
fn serve_one(
    stream: &mut UnixStream,
    requester: &SnapshotRequester,
    health_requester: &HealthRequester,
    state_requester: &StateRequester,
    watch_requester: &WatchRequester,
    drain_requester: &DrainRequester,
    queue_stats_requester: &QueueStatsRequester,
    completions_requester: Option<&DrainCompletionsRequester>,
) -> Result<(), IpcError> {
    // §1.3b: the frame bound is per-kind, so read at the widest bound and
    // reject a frame that exceeds its own kind's bound.
    let (request, frame_len): (WireRequest, usize) =
        read_frame_with_len(stream, MAX_WATCH_REQUEST_BYTES)?;
    if frame_len > request_bound(request.kind) {
        return Err(IpcError::MalformedFrame);
    }
    let (bytes, kind) = match request.kind {
        RequestKind::Snapshot => {
            let mut snapshot = requester.request(request.limit)?;
            snapshot.sort_and_limit(request.limit);
            (serde_cbor::to_vec(&snapshot)?, ReplyKind::Snapshot)
        }
        // Supervision health arm (§2A): read-only aggregation, identical
        // memfd reply envelope.
        RequestKind::Health => {
            let report = health_requester.request()?;
            (serde_cbor::to_vec(&report)?, ReplyKind::Health)
        }
        // R3 applied-state arm: the fence-committed fold (opaque CBOR
        // transaction bytes). A report over the reply-envelope bound is
        // a Typed TooLarge — never a truncated or failed frame.
        RequestKind::State => {
            // `.59`: NAME the State-arm failure here (the serve loop also logs
            // it generically). A `CollectorUnavailable`/timeout returns from
            // `serve_one` with NO reply sent — no SCM_RIGHTS fd — so the
            // broker's only visible symptom is
            // `no_scm_rights_fd`/`peer_closed_no_bytes`.
            let state_result = state_requester.request();
            if let Err(error) = state_result.as_ref() {
                eprintln!("analytics State arm requester FAILED (kind=State): {error} ({error:?})");
            }
            let reply = match state_result? {
                StateReply::Unavailable(reason) => StateReply::Unavailable(reason),
                StateReply::Report(report) => {
                    let bytes = serde_cbor::to_vec(&report)?;
                    if bytes.len() > MAX_SNAPSHOT_BYTES {
                        StateReply::Unavailable(StateUnavailableReason::TooLarge)
                    } else {
                        StateReply::Report(report)
                    }
                }
            };
            (serde_cbor::to_vec(&reply)?, ReplyKind::State)
        }
        // §1.3b: replace the shared watch-set image (single writer = the
        // collector).
        RequestKind::SetWatchList => {
            let ack = watch_requester.request(&request.watch)?;
            (serde_cbor::to_vec(&ack)?, ReplyKind::SetWatchList)
        }
        // §1.3a: drain the per-worker capture rings.
        RequestKind::DrainCapture => {
            let batch = drain_requester.request(request.limit)?;
            (serde_cbor::to_vec(&batch)?, ReplyKind::DrainCapture)
        }
        // K2 slice 2 §7: render per-circuit qdisc detail for the bounded
        // requested slot list.
        RequestKind::QueueStats => {
            let reply = queue_stats_requester.request(&request.queue_slots)?;
            (serde_cbor::to_vec(&reply)?, ReplyKind::QueueStats)
        }
        // Flow-completion feed: the ring source is supplied by the collector
        // (bridge_rs). Without a requester the ring is unavailable (an older
        // bridge ⇒ the consumer falls back to the snapshot export).
        RequestKind::DrainCompletions => {
            let Some(requester) = completions_requester else {
                return Err(IpcError::CollectorUnavailable);
            };
            let batch = requester.request(request.epoch, request.after_seq, request.limit)?;
            (serde_cbor::to_vec(&batch)?, ReplyKind::DrainCompletions)
        }
    };
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(IpcError::MalformedFrame);
    }
    let memfd = write_once(bytes.len() as u64, "lqtop-snapshot", |memory| {
        memory.copy_from_slice(&bytes);
    })?;
    let reply = serde_cbor::to_vec(&WireReply {
        snapshot_len: bytes.len(),
        kind,
    })?;
    let length = (reply.len() as u32).to_be_bytes();
    let iov = [IoSlice::new(&length)];
    let fds = [memfd.as_file().as_raw_fd()];
    sendmsg::<()>(
        stream.as_raw_fd(),
        &iov,
        &[ControlMessage::ScmRights(&fds)],
        MsgFlags::empty(),
        None,
    )?;
    stream.write_all(&reply)?;
    Ok(())
}

/// HIGH-1 ownership adoption: chown the socket to the `libreqos` service
/// user (fail-visible warn on absence/failure — the bridge keeps serving
/// root-only) and warn when the parent dir is not traversable by that
/// user. Mirrors control_channel.rs's G5 pattern; no new privilege grant.
fn adopt_for_libreqos(path: &Path) {
    let name = b"libreqos\0";
    let pw = unsafe { libc::getpwnam(name.as_ptr() as *const libc::c_char) };
    if pw.is_null() {
        eprintln!(
            "WARN: libreqos user not found; analytics socket {} stays root-owned — lqosd cannot connect until provisioned",
            path.display()
        );
        return;
    }
    let (uid, gid) = unsafe { ((*pw).pw_uid, (*pw).pw_gid) };
    let c_path = match std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
        Ok(c) => c,
        Err(_) => return,
    };
    if unsafe { libc::chown(c_path.as_ptr(), uid, gid) } < 0 {
        eprintln!(
            "WARN: chown of analytics socket {} failed ({:?}); lqosd cannot connect until provisioned",
            path.display(),
            std::io::Error::last_os_error()
        );
    }
    if let Some(parent) = path.parent() {
        let traversable = fs::metadata(parent)
            .map(|meta| {
                let mode = meta.permissions().mode();
                (meta.uid() == uid && mode & 0o100 != 0)
                    || (meta.gid() == gid && mode & 0o010 != 0)
                    || mode & 0o001 != 0
            })
            .unwrap_or(false);
        if !traversable {
            eprintln!(
                "WARN: analytics socket parent dir {} is not traversable by the libreqos user — lqosd cannot connect",
                parent.display()
            );
        }
    }
}

/// Connects and requests the HEALTH report (supervision contract §2A) —
/// the lqosd Supervisor's L2/L4 probe reference implementation.
pub fn connect_health(path: impl AsRef<Path>) -> Result<HealthReport, IpcError> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write_frame(
        &mut stream,
        &WireRequest {
            kind: RequestKind::Health,
            limit: 1,
            watch: Vec::new(),
            queue_slots: Vec::new(),
            epoch: 0,
            after_seq: 0,
        },
    )?;
    let mut header = [0_u8; 4];
    let mut iov = [IoSliceMut::new(&mut header)];
    let mut control = cmsg_space!([RawFd; 1]);
    let (bytes, fd) = {
        let message = recvmsg::<()>(
            stream.as_raw_fd(),
            &mut iov,
            Some(&mut control),
            MsgFlags::empty(),
        )?;
        let mut received_fd = None;
        for control in message.cmsgs()? {
            if let ControlMessageOwned::ScmRights(fds) = control {
                received_fd = fds.first().copied();
            }
        }
        (message.bytes, received_fd.ok_or(IpcError::MalformedFrame)?)
    };
    if bytes != 4 {
        return Err(IpcError::MalformedFrame);
    }
    let frame_len = u32::from_be_bytes(header) as usize;
    if frame_len == 0 || frame_len > 1_024 {
        return Err(IpcError::MalformedFrame);
    }
    let mut reply_bytes = vec![0; frame_len];
    stream.read_exact(&mut reply_bytes)?;
    let reply: WireReply = serde_cbor::from_slice(&reply_bytes)?;
    if reply.kind != ReplyKind::Health
        || reply.snapshot_len == 0
        || reply.snapshot_len > MAX_SNAPSHOT_BYTES
    {
        return Err(IpcError::MalformedFrame);
    }
    // SAFETY: SCM_RIGHTS transferred ownership of this fresh descriptor.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let memfd = Memfd::try_from_file(file).map_err(|_| IpcError::MalformedFrame)?;
    let memory = read_memfd(&memfd)?;
    if memory.len() < reply.snapshot_len {
        return Err(IpcError::MalformedFrame);
    }
    Ok(serde_cbor::from_slice(&memory[..reply.snapshot_len])?)
}

pub fn connect_snapshot(
    path: impl AsRef<Path>,
    limit: usize,
) -> Result<AnalyticsSnapshot, IpcError> {
    validate_limit(limit)?;
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write_frame(
        &mut stream,
        &WireRequest {
            kind: RequestKind::Snapshot,
            limit,
            watch: Vec::new(),
            queue_slots: Vec::new(),
            epoch: 0,
            after_seq: 0,
        },
    )?;

    let mut header = [0_u8; 4];
    let mut iov = [IoSliceMut::new(&mut header)];
    let mut control = cmsg_space!([RawFd; 1]);
    let (bytes, fd) = {
        let message = recvmsg::<()>(
            stream.as_raw_fd(),
            &mut iov,
            Some(&mut control),
            MsgFlags::empty(),
        )?;
        let mut received_fd = None;
        for control in message.cmsgs()? {
            if let ControlMessageOwned::ScmRights(fds) = control {
                received_fd = fds.first().copied();
            }
        }
        (message.bytes, received_fd.ok_or(IpcError::MalformedFrame)?)
    };
    if bytes != 4 {
        return Err(IpcError::MalformedFrame);
    }
    let frame_len = u32::from_be_bytes(header) as usize;
    if frame_len == 0 || frame_len > 1_024 {
        return Err(IpcError::MalformedFrame);
    }
    let mut reply_bytes = vec![0; frame_len];
    stream.read_exact(&mut reply_bytes)?;
    let reply: WireReply = serde_cbor::from_slice(&reply_bytes)?;
    if reply.snapshot_len == 0 || reply.snapshot_len > MAX_SNAPSHOT_BYTES {
        return Err(IpcError::MalformedFrame);
    }
    // SAFETY: SCM_RIGHTS transferred ownership of this fresh descriptor.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let memfd = Memfd::try_from_file(file).map_err(|_| IpcError::MalformedFrame)?;
    let memory = read_memfd(&memfd)?;
    if memory.len() < reply.snapshot_len {
        return Err(IpcError::MalformedFrame);
    }
    Ok(serde_cbor::from_slice(&memory[..reply.snapshot_len])?)
}

fn validate_limit(limit: usize) -> Result<(), IpcError> {
    if !(1..=DEFAULT_MAX_ROWS).contains(&limit) {
        return Err(IpcError::InvalidLimit);
    }
    Ok(())
}

fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<(), IpcError> {
    let bytes = serde_cbor::to_vec(value)?;
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    Ok(())
}

/// The per-kind request-frame bound (§1.3b). `SetWatchList` carries the
/// watch set and gets its own wider bound; all other kinds keep 64 bytes.
fn request_bound(kind: RequestKind) -> usize {
    match kind {
        RequestKind::SetWatchList => MAX_WATCH_REQUEST_BYTES,
        RequestKind::QueueStats => MAX_QUEUE_STATS_REQUEST_BYTES,
        RequestKind::DrainCompletions => MAX_COMPLETIONS_REQUEST_BYTES,
        RequestKind::Snapshot
        | RequestKind::Health
        | RequestKind::State
        | RequestKind::DrainCapture => MAX_REQUEST_BYTES,
    }
}

#[cfg(test)]
fn read_frame<T: for<'de> Deserialize<'de>>(
    stream: &mut UnixStream,
    maximum: usize,
) -> Result<T, IpcError> {
    read_frame_with_len(stream, maximum).map(|(value, _length)| value)
}

fn read_frame_with_len<T: for<'de> Deserialize<'de>>(
    stream: &mut UnixStream,
    maximum: usize,
) -> Result<(T, usize), IpcError> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > maximum {
        return Err(IpcError::MalformedFrame);
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    Ok((serde_cbor::from_slice(&bytes)?, length))
}

/// R3: connects and requests the applied-state report — the broker's
/// attach-baseline reference client. A typed unavailable reason comes
/// back as [`IpcError::StateUnavailable`] (the broker's named
/// fallback); an OLD server (no State arm) answers with a mismatched
/// reply kind ⇒ MalformedFrame ⇒ the same fallback path.
pub fn connect_state(path: impl AsRef<Path>) -> Result<StateReport, IpcError> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write_frame(
        &mut stream,
        &WireRequest {
            kind: RequestKind::State,
            limit: 1,
            watch: Vec::new(),
            queue_slots: Vec::new(),
            epoch: 0,
            after_seq: 0,
        },
    )?;
    // `lqos-74t.15.3` MalformedFrame localization: name the sub-arm that rejects
    // the frame (diagnostic only — no behavior change vs the generic reject).
    let mf = |arm: &'static str| IpcError::MalformedStateFrame {
        arm,
        kind: RequestKind::State,
    };
    let mut header = [0_u8; 4];
    let mut iov = [IoSliceMut::new(&mut header)];
    let mut control = cmsg_space!([RawFd; 1]);
    let (bytes, fd) = {
        let message = recvmsg::<()>(
            stream.as_raw_fd(),
            &mut iov,
            Some(&mut control),
            MsgFlags::empty(),
        )?;
        let mut received_fd = None;
        for control in message.cmsgs()? {
            if let ControlMessageOwned::ScmRights(fds) = control {
                received_fd = fds.first().copied();
            }
        }
        let nbytes = message.bytes;
        let fd = match received_fd {
            Some(fd) => fd,
            // `.59` residual: a 0-byte FIRST message carrying no fd is an
            // orderly peer close / setup failure (the bridge's `serve_one`
            // returned an Err before its `sendmsg`, and the serve loop discards
            // that error) — NOT a framing divergence. Name it distinctly so the
            // broker-side surface is not mis-attributed as a header bug.
            None if nbytes == 0 => return Err(mf("peer_closed_no_bytes")),
            None => return Err(mf("no_scm_rights_fd")),
        };
        (nbytes, fd)
    };
    if bytes != 4 {
        return Err(mf("header_len_ne_4"));
    }
    let frame_len = u32::from_be_bytes(header) as usize;
    if frame_len == 0 || frame_len > 1_024 {
        return Err(mf("frame_len_out_of_range"));
    }
    let mut reply_bytes = vec![0; frame_len];
    stream.read_exact(&mut reply_bytes)?;
    let reply: WireReply =
        serde_cbor::from_slice(&reply_bytes).map_err(|_| mf("wire_reply_cbor"))?;
    if reply.kind != ReplyKind::State
        || reply.snapshot_len == 0
        || reply.snapshot_len > MAX_SNAPSHOT_BYTES
    {
        return Err(mf("reply_kind_or_snapshot_len"));
    }
    // SAFETY: SCM_RIGHTS transferred ownership of this fresh descriptor.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let memfd = Memfd::try_from_file(file).map_err(|_| mf("memfd_from_fd"))?;
    let memory = read_memfd(&memfd)?;
    if memory.len() < reply.snapshot_len {
        return Err(mf("memfd_short"));
    }
    match serde_cbor::from_slice::<StateReply>(&memory[..reply.snapshot_len])
        .map_err(|_| mf("reply_cbor"))?
    {
        StateReply::Report(report) => Ok(report),
        StateReply::Unavailable(reason) => Err(IpcError::StateUnavailable(reason)),
    }
}

/// Shared reply-envelope reader for the watch/drain clients: the 4-byte
/// length header, the SCM_RIGHTS memfd, and the small CBOR [`WireReply`].
fn recv_memfd_reply(stream: &mut UnixStream) -> Result<(WireReply, Vec<u8>), IpcError> {
    let mut header = [0_u8; 4];
    let mut iov = [IoSliceMut::new(&mut header)];
    let mut control = cmsg_space!([RawFd; 1]);
    let (bytes, fd) = {
        let message = recvmsg::<()>(
            stream.as_raw_fd(),
            &mut iov,
            Some(&mut control),
            MsgFlags::empty(),
        )?;
        let mut received_fd = None;
        for control in message.cmsgs()? {
            if let ControlMessageOwned::ScmRights(fds) = control {
                received_fd = fds.first().copied();
            }
        }
        (message.bytes, received_fd.ok_or(IpcError::MalformedFrame)?)
    };
    if bytes != 4 {
        return Err(IpcError::MalformedFrame);
    }
    let frame_len = u32::from_be_bytes(header) as usize;
    if frame_len == 0 || frame_len > 1_024 {
        return Err(IpcError::MalformedFrame);
    }
    let mut reply_bytes = vec![0; frame_len];
    stream.read_exact(&mut reply_bytes)?;
    let reply: WireReply = serde_cbor::from_slice(&reply_bytes)?;
    if reply.snapshot_len == 0 || reply.snapshot_len > MAX_SNAPSHOT_BYTES {
        return Err(IpcError::MalformedFrame);
    }
    // SAFETY: SCM_RIGHTS transferred ownership of this fresh descriptor.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let memfd = Memfd::try_from_file(file).map_err(|_| IpcError::MalformedFrame)?;
    let memory = read_memfd(&memfd)?;
    if memory.len() < reply.snapshot_len {
        return Err(IpcError::MalformedFrame);
    }
    let payload = memory[..reply.snapshot_len].to_vec();
    Ok((reply, payload))
}

/// Heimdall re-derivation §1.3b: replaces the bridge's watch set. An empty
/// slice clears the watches; slot 0 is rejected client-side.
pub fn connect_watch(path: impl AsRef<Path>, watch: &[u64]) -> Result<WatchAck, IpcError> {
    validate_watch(watch)?;
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write_frame(
        &mut stream,
        &WireRequest {
            kind: RequestKind::SetWatchList,
            limit: 1,
            watch: watch.to_vec(),
            queue_slots: Vec::new(),
            epoch: 0,
            after_seq: 0,
        },
    )?;
    let (reply, memory) = recv_memfd_reply(&mut stream)?;
    if reply.kind != ReplyKind::SetWatchList {
        return Err(IpcError::MalformedFrame);
    }
    Ok(serde_cbor::from_slice(&memory)?)
}

/// Heimdall re-derivation §1.3a: drains the bridge's per-worker capture
/// rings (drop-on-full; the batch carries the cumulative drop count).
pub fn connect_drain(path: impl AsRef<Path>, limit: usize) -> Result<CaptureBatch, IpcError> {
    validate_limit(limit)?;
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write_frame(
        &mut stream,
        &WireRequest {
            kind: RequestKind::DrainCapture,
            limit,
            watch: Vec::new(),
            queue_slots: Vec::new(),
            epoch: 0,
            after_seq: 0,
        },
    )?;
    let (reply, memory) = recv_memfd_reply(&mut stream)?;
    if reply.kind != ReplyKind::DrainCapture {
        return Err(IpcError::MalformedFrame);
    }
    Ok(serde_cbor::from_slice(&memory)?)
}

/// Flow-completion drain client (the lqosd consumer): drain the bridge's
/// completion ring after `after_seq` on `epoch`. An OLD bridge (no
/// `DrainCompletions` kind) fails the request ⇒ the caller falls back to the
/// snapshot export.
pub fn connect_drain_completions(
    path: impl AsRef<Path>,
    epoch: u64,
    after_seq: u64,
    limit: usize,
) -> Result<CompletionBatch, IpcError> {
    validate_limit(limit)?;
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write_frame(
        &mut stream,
        &WireRequest {
            kind: RequestKind::DrainCompletions,
            limit,
            watch: Vec::new(),
            queue_slots: Vec::new(),
            epoch,
            after_seq,
        },
    )?;
    let (reply, memory) = recv_memfd_reply(&mut stream)?;
    if reply.kind != ReplyKind::DrainCompletions {
        return Err(IpcError::MalformedFrame);
    }
    Ok(serde_cbor::from_slice(&memory)?)
}

/// K2 slice 2 §7: renders per-circuit qdisc detail for a bounded slot-id
/// list (≤[`MAX_QUEUE_STATS_IDS`]; slot 0 rejected client-side). The reply is
/// the memfd CBOR [`QueueStatsReply`] — one [`crate::QdiscDetailRow`] per
/// requested slot.
/// `.115`: test/diagnostic call counter for the QueueStats socket path. The
/// API `raw_queue_data` path must never increment this (cache-only read).
pub static QUEUE_STATS_SOCKET_CALLS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// `.115`: the number of `connect_queue_stats` socket calls in this process.
pub fn queue_stats_socket_calls() -> u64 {
    QUEUE_STATS_SOCKET_CALLS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn connect_queue_stats(
    path: impl AsRef<Path>,
    slots: &[u64],
) -> Result<QueueStatsReply, IpcError> {
    QUEUE_STATS_SOCKET_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    validate_queue_stats(slots)?;
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write_frame(
        &mut stream,
        &WireRequest {
            kind: RequestKind::QueueStats,
            limit: 1,
            watch: Vec::new(),
            queue_slots: slots.to_vec(),
            epoch: 0,
            after_seq: 0,
        },
    )?;
    let (reply, memory) = recv_memfd_reply(&mut stream)?;
    if reply.kind != ReplyKind::QueueStats {
        return Err(IpcError::MalformedFrame);
    }
    Ok(serde_cbor::from_slice(&memory)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::TryRecvError;
    use std::thread;

    #[test]
    fn capacity_one_requests_never_block_collector() {
        let (requester, rx) = snapshot_channel(Duration::from_secs(1));
        let (reply, _unused) = sync_channel(1);
        requester
            .tx
            .try_send(SnapshotRequest { limit: 1, reply })
            .unwrap();
        let (reply, _unused) = sync_channel(1);
        assert!(matches!(
            requester.tx.try_send(SnapshotRequest { limit: 1, reply }),
            Err(TrySendError::Full(_))
        ));
        assert!(!matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    /// `bridge_x_selfheal_card` §3: the hardened unlink must NEVER remove a
    /// foreign/recreated socket (a foreign inode at our path is left intact).
    #[test]
    fn hardened_unlink_refuses_a_foreign_inode() {
        let dir = std::env::temp_dir().join(format!("lq3-heal-owned-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("analytics.sock");
        let (requester, _rx) = snapshot_channel(Duration::from_secs(1));
        let (health_requester, _hrx) = health_channel(Duration::from_secs(1));
        let (state_requester, _srx) = state_channel(Duration::from_secs(1));
        let service =
            SnapshotService::bind(&path, requester, health_requester, state_requester).unwrap();
        // Externally remove our socket, then plant a FOREIGN one at the path.
        let _ = fs::remove_file(&path);
        let foreign = UnixListener::bind(&path).unwrap();
        drop(service); // must not unlink the foreign inode
        assert!(
            path.exists(),
            "the hardened Drop must not unlink a foreign inode"
        );
        drop(foreign);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    /// `bridge_x_selfheal_card` §3: a socket unlinked while `serve()` runs is
    /// re-bound within one check interval (a new inode appears at the path).
    #[test]
    fn self_heal_rebinds_a_vanished_socket() {
        let dir = std::env::temp_dir().join(format!("lq3-heal-vanish-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("analytics.sock");
        let (requester, _rx) = snapshot_channel(Duration::from_secs(1));
        let (health_requester, _hrx) = health_channel(Duration::from_secs(1));
        let (state_requester, _srx) = state_channel(Duration::from_secs(1));
        let service =
            SnapshotService::bind(&path, requester, health_requester, state_requester).unwrap();
        let original = path_inode(&path).expect("bound socket exists");
        let handle = thread::spawn(move || {
            let _ = service.serve();
        });
        assert!(path.exists());
        // The vanish the self-heal must recover.
        let _ = fs::remove_file(&path);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut rebound = None;
        while Instant::now() < deadline {
            if let Some(ino) = path_inode(&path)
                && ino != original
            {
                rebound = Some(ino);
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            rebound.is_some(),
            "serve() must re-bind a socket unlinked mid-life"
        );
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
        drop(handle);
    }

    #[test]
    fn invalid_limits_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            connect_snapshot(dir.path().join("missing"), 0),
            Err(IpcError::InvalidLimit)
        ));
        assert!(matches!(
            connect_snapshot(dir.path().join("missing"), DEFAULT_MAX_ROWS + 1),
            Err(IpcError::InvalidLimit)
        ));
    }

    #[test]
    fn socket_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("analytics.sock");
        let (requester, _rx) = snapshot_channel(Duration::from_secs(1));
        let (health_requester, _hrx) = health_channel(Duration::from_secs(1));
        let (state_requester, _state_rx) = state_channel(Duration::from_millis(100));
        let service =
            SnapshotService::bind(&socket, requester, health_requester, state_requester).unwrap();
        assert_eq!(
            fs::metadata(service.local_addr_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    /// L2 (LKG-boot parity card §2, F4): an unclean death (kill -9, crash)
    /// never runs Drop, so the socket file survives; the restart's bind
    /// must recover instead of FATALing "Address already in use" forever.
    #[test]
    fn bind_recovers_from_stale_socket_file() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("analytics.sock");
        let (requester, _rx) = snapshot_channel(Duration::from_secs(1));
        let (health_requester, _hrx) = health_channel(Duration::from_secs(1));
        let (state_requester, _state_rx) = state_channel(Duration::from_millis(100));
        let dead = SnapshotService::bind(
            &socket,
            requester.clone(),
            health_requester.clone(),
            state_requester,
        )
        .unwrap();
        // Simulate the unclean death: Drop never runs.
        std::mem::forget(dead);
        assert!(socket.exists());
        let (state_requester, _state_rx) = state_channel(Duration::from_millis(100));
        let service =
            SnapshotService::bind(&socket, requester, health_requester, state_requester).unwrap();
        assert!(socket.exists());
        drop(service);
        assert!(!socket.exists());
    }

    #[test]
    fn malformed_frame_is_rejected() {
        let (mut left, mut right) = UnixStream::pair().unwrap();
        left.write_all(&[0, 0, 1, 0]).unwrap();
        assert!(matches!(
            read_frame::<WireRequest>(&mut right, MAX_REQUEST_BYTES),
            Err(IpcError::MalformedFrame)
        ));
    }

    #[test]
    fn one_shot_clients_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("analytics.sock");
        let (requester, rx) = snapshot_channel(Duration::from_secs(1));
        let (health_requester, _hrx) = health_channel(Duration::from_secs(1));
        let (state_requester, _state_rx) = state_channel(Duration::from_millis(100));
        let service =
            SnapshotService::bind(&socket, requester, health_requester, state_requester).unwrap();
        let server = thread::spawn(move || {
            for stream in service.listener.incoming().take(2) {
                serve_one(
                    &mut stream.unwrap(),
                    &service.requester,
                    &service.health_requester,
                    &service.state_requester,
                    &service.watch_requester,
                    &service.drain_requester,
                    &service.queue_stats_requester,
                    None,
                )
                .unwrap();
            }
        });
        let collector = thread::spawn(move || {
            for captured_at_millis in [1, 2] {
                let request = rx.recv().unwrap();
                request
                    .reply
                    .try_send(AnalyticsSnapshot {
                        captured_at_millis,
                        ..Default::default()
                    })
                    .unwrap();
            }
        });
        assert_eq!(connect_snapshot(&socket, 10).unwrap().captured_at_millis, 1);
        assert_eq!(connect_snapshot(&socket, 10).unwrap().captured_at_millis, 2);
        server.join().unwrap();
        collector.join().unwrap();
    }
}

#[cfg(test)]
mod supervision_tests {
    use super::*;
    use std::thread;

    /// LOW-2: a pre-health client's frame (`{"limit": N}`, no tag) decodes
    /// as a Snapshot request — the tag ships together with defaults.
    #[test]
    fn legacy_request_frame_still_decodes_as_snapshot() {
        // Hand-encoded CBOR map {"limit": 10} — the exact pre-health
        // client frame (no serde_json dependency in this crate).
        let frame: &[u8] = &[0xA1, 0x65, b'l', b'i', b'm', b'i', b't', 0x0A];
        let request: WireRequest = serde_cbor::from_slice(frame).unwrap();
        assert_eq!(request.kind, RequestKind::Snapshot);
        assert_eq!(request.limit, 10);
    }

    /// The health request round-trips end-to-end through the real socket
    /// with the same memfd envelope; snapshot clients on the SAME socket
    /// keep working (LOW-2 both-directions compatibility).
    #[test]
    fn health_request_round_trips_alongside_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("analytics.sock");
        let (requester, rx) = snapshot_channel(Duration::from_secs(1));
        let (health_requester, health_rx) = health_channel(Duration::from_secs(1));
        let (state_requester, _state_rx) = state_channel(Duration::from_millis(100));
        let service =
            SnapshotService::bind(&socket, requester, health_requester, state_requester).unwrap();
        let server = thread::spawn(move || {
            for stream in service.listener.incoming().take(2) {
                let _ = serve_one(
                    &mut stream.unwrap(),
                    &service.requester,
                    &service.health_requester,
                    &service.state_requester,
                    &service.watch_requester,
                    &service.drain_requester,
                    &service.queue_stats_requester,
                    None,
                );
            }
        });
        let snapshot_collector = thread::spawn(move || {
            let request = rx.recv().unwrap();
            request
                .reply
                .try_send(AnalyticsSnapshot::default())
                .unwrap();
        });
        let health_collector = thread::spawn(move || {
            let request = health_rx.recv().unwrap();
            request
                .reply
                .try_send(crate::HealthReport {
                    status: "starting".to_string(),
                    version: "test".to_string(),
                    uptime_ms: 5,
                    completion_ring_epoch: 0,
                    completion_ring_emitted: 0,
                    completion_ring_dropped: 0,
                    completion_ring_overwritten: 0,
                    completion_ring_forced: 0,
                    completion_ring_depth: 0,
                    completion_ring_oldest_seq: 0,
                    completion_ring_next_seq: 0,
                    admission_refusals: 0,
                    tcp_cap_drops: 0,
                    host_cap_drops: 0,
                    recent_full_drops: 0,
                    pending_telemetry_drops: 0,
                    workers: vec![crate::WorkerHealth {
                        queue_id: 1,
                        rollup_age_ms: None,
                        ..Default::default()
                    }],
                    channel: crate::ChannelHealth::default(),
                    barrier: crate::BarrierHealth::default(),
                    classification_empty: false,
                    topology_tier: String::new(),
                    degraded_subsystems: Vec::new(),
                    boots: 0,
                    last_boot_unix: 0,
                    tier_counts: std::collections::BTreeMap::new(),
                })
                .unwrap();
        });
        let report = connect_health(&socket).unwrap();
        assert_eq!(report.status, "starting");
        assert_eq!(report.workers.len(), 1);
        assert_eq!(report.workers[0].rollup_age_ms, None);
        let snapshot = connect_snapshot(&socket, 10).unwrap();
        assert_eq!(snapshot.captured_at_millis, 0);
        server.join().unwrap();
        snapshot_collector.join().unwrap();
        health_collector.join().unwrap();
    }

    /// R3: the State arm end-to-end — a report round-trips byte-faithful
    /// and a no-report-yet slot answers the typed unavailable reason.
    #[test]
    fn state_request_round_trips_report_and_unavailable() {
        let socket_dir = std::env::temp_dir().join(format!(
            "lq3-state-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&socket_dir).unwrap();
        let socket = socket_dir.join("state.sock");
        let (requester, _rx) = snapshot_channel(Duration::from_secs(1));
        let (health_requester, _hrx) = health_channel(Duration::from_secs(1));
        let (state_requester, state_rx) = state_channel(Duration::from_secs(1));
        let service =
            SnapshotService::bind(&socket, requester, health_requester, state_requester).unwrap();
        let server = thread::spawn(move || {
            for stream in service.listener.incoming().take(2) {
                let _ = serve_one(
                    &mut stream.unwrap(),
                    &service.requester,
                    &service.health_requester,
                    &service.state_requester,
                    &service.watch_requester,
                    &service.drain_requester,
                    &service.queue_stats_requester,
                    None,
                );
            }
        });
        // The collector serves the two sequential requests: first the
        // typed NoReportYet, then a byte-faithful report (R3c: the
        // slot_order field rides the same reply envelope; the bytes are
        // OPAQUE here — serde_cbor for the fixture, the producers
        // encode with their own CBOR).
        let order: Vec<String> = vec!["c2".to_string(), "c1".to_string()];
        let report = StateReport {
            generation: "gen-r3".to_string(),
            report_schema: 1,
            transaction: vec![1, 2, 3, 4, 250],
            slot_order: serde_cbor::to_vec(&order).expect("order serializes"),
            content_key: "tx-test-vector".to_string(),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: serde_cbor::to_vec(&vec![0u16; order.len()]).expect("owners serialize"),
            workers: None,
        };
        let expected = report.clone();
        let collector = thread::spawn(move || {
            if let Ok(request) = state_rx.recv() {
                let _ = request
                    .reply
                    .try_send(StateReply::Unavailable(StateUnavailableReason::NoReportYet));
            }
            if let Ok(request) = state_rx.recv() {
                let _ = request.reply.try_send(StateReply::Report(expected));
            }
        });
        match connect_state(&socket) {
            Err(IpcError::StateUnavailable(StateUnavailableReason::NoReportYet)) => {}
            other => panic!("expected the typed NoReportYet error, got {other:?}"),
        }
        let got = connect_state(&socket).expect("the report round-trips");
        assert_eq!(got, report);
        server.join().unwrap();
        collector.join().unwrap();
        let _ = fs::remove_dir_all(&socket_dir);
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::{
        CAPTURE_PREFIX_BYTES, CaptureBatch, CaptureDirection, CapturePrefix, CaptureRecord,
    };
    use std::thread;

    fn temp_socket(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lq3-capture-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("capture.sock")
    }

    /// LOW-8 / §1.3b: `limit` gains a serde default, so a kind-only frame
    /// decodes instead of failing.
    #[test]
    fn wire_request_defaults_limit_and_watch_when_absent() {
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            serde_cbor::Value::Text("kind".to_string()),
            serde_cbor::Value::Text("health".to_string()),
        );
        let frame = serde_cbor::to_vec(&serde_cbor::Value::Map(map)).unwrap();
        let request: WireRequest = serde_cbor::from_slice(&frame).unwrap();
        assert_eq!(request.kind, RequestKind::Health);
        assert_eq!(request.limit, 1);
        assert!(request.watch.is_empty());
    }

    /// HIGH-1 / §1.3b: a max-size watch frame exceeds the 64-byte snapshot
    /// bound but round-trips through its own [`MAX_WATCH_REQUEST_BYTES`].
    #[test]
    fn max_size_watch_frame_round_trips_within_its_own_bound() {
        let watch: Vec<u64> = (1..=MAX_WATCH_ENTRIES as u64)
            .map(|i| i.wrapping_mul(0x0101_0101_0101_0101))
            .collect();
        let frame = serde_cbor::to_vec(&WireRequest {
            kind: RequestKind::SetWatchList,
            limit: 1,
            watch: watch.clone(),
            queue_slots: Vec::new(),
            epoch: 0,
            after_seq: 0,
        })
        .unwrap();
        assert!(
            frame.len() > MAX_REQUEST_BYTES,
            "watch frame ({} B) must exceed the 64-byte snapshot bound",
            frame.len()
        );
        assert!(
            frame.len() <= MAX_WATCH_REQUEST_BYTES,
            "watch frame ({} B) must fit its own bound",
            frame.len()
        );
        let (mut left, mut right) = UnixStream::pair().unwrap();
        write_frame(
            &mut left,
            &WireRequest {
                kind: RequestKind::SetWatchList,
                limit: 1,
                watch: watch.clone(),
                queue_slots: Vec::new(),
                epoch: 0,
                after_seq: 0,
            },
        )
        .unwrap();
        let (decoded, len) =
            read_frame_with_len::<WireRequest>(&mut right, MAX_WATCH_REQUEST_BYTES).unwrap();
        assert_eq!(decoded.watch, watch);
        assert_eq!(len, frame.len());
    }

    #[test]
    fn request_bounds_are_per_kind() {
        assert_eq!(
            request_bound(RequestKind::SetWatchList),
            MAX_WATCH_REQUEST_BYTES
        );
        assert_eq!(
            request_bound(RequestKind::QueueStats),
            MAX_QUEUE_STATS_REQUEST_BYTES
        );
        for kind in [
            RequestKind::Snapshot,
            RequestKind::Health,
            RequestKind::State,
            RequestKind::DrainCapture,
        ] {
            assert_eq!(request_bound(kind), MAX_REQUEST_BYTES);
        }
    }

    /// K2 slice 2 §7: a max-size QueueStats frame exceeds the 64-byte
    /// snapshot bound but round-trips through its own 1 KiB-class bound.
    #[test]
    fn max_size_queue_stats_frame_round_trips_within_its_own_bound() {
        let slots: Vec<u64> = (1..=MAX_QUEUE_STATS_IDS as u64)
            .map(|i| i.wrapping_mul(0x0101_0101_0101_0101))
            .collect();
        let frame = serde_cbor::to_vec(&WireRequest {
            kind: RequestKind::QueueStats,
            limit: 1,
            watch: Vec::new(),
            queue_slots: slots.clone(),
            epoch: 0,
            after_seq: 0,
        })
        .unwrap();
        assert!(
            frame.len() > MAX_REQUEST_BYTES,
            "queue-stats frame ({} B) must exceed the 64-byte snapshot bound",
            frame.len()
        );
        assert!(
            frame.len() <= MAX_QUEUE_STATS_REQUEST_BYTES,
            "queue-stats frame ({} B) must fit its own bound",
            frame.len()
        );
        let (mut left, mut right) = UnixStream::pair().unwrap();
        write_frame(
            &mut left,
            &WireRequest {
                kind: RequestKind::QueueStats,
                limit: 1,
                watch: Vec::new(),
                queue_slots: slots.clone(),
                epoch: 0,
                after_seq: 0,
            },
        )
        .unwrap();
        let (decoded, len) =
            read_frame_with_len::<WireRequest>(&mut right, MAX_QUEUE_STATS_REQUEST_BYTES).unwrap();
        assert_eq!(decoded.queue_slots, slots);
        assert_eq!(len, frame.len());
    }

    /// K2 slice 2 §7: slot 0 has no leaf qdisc; an empty list is a legal
    /// empty request; an over-length list is rejected.
    #[test]
    fn slot_zero_and_oversize_queue_stats_lists_are_rejected() {
        assert!(matches!(
            validate_queue_stats(&[0]),
            Err(IpcError::InvalidQueueStatsEntry)
        ));
        assert!(validate_queue_stats(&[]).is_ok());
        assert!(validate_queue_stats(&[1, 2, 3]).is_ok());
        let too_many: Vec<u64> = (1..=MAX_QUEUE_STATS_IDS as u64 + 1).collect();
        assert!(matches!(
            validate_queue_stats(&too_many),
            Err(IpcError::InvalidQueueStatsEntry)
        ));
        assert!(matches!(
            connect_queue_stats("/nonexistent/socket", &[0]),
            Err(IpcError::InvalidQueueStatsEntry)
        ));
    }

    /// §1.1: slot 0 (best-effort) is never settable; an empty set is a legal
    /// clear; an over-length set is rejected.
    #[test]
    fn slot_zero_and_oversize_watch_sets_are_rejected() {
        assert!(matches!(
            validate_watch(&[0]),
            Err(IpcError::InvalidWatchEntry)
        ));
        assert!(validate_watch(&[]).is_ok());
        assert!(validate_watch(&[1, 2, 3]).is_ok());
        let too_many: Vec<u64> = (1..=MAX_WATCH_ENTRIES as u64 + 1).collect();
        assert!(matches!(
            validate_watch(&too_many),
            Err(IpcError::InvalidWatchEntry)
        ));
        assert!(matches!(
            connect_watch("/nonexistent/socket", &[0]),
            Err(IpcError::InvalidWatchEntry)
        ));
    }

    /// The watch set and the drain batch round-trip end-to-end through the
    /// real socket + memfd envelope, including the 128-byte inner prefix.
    #[test]
    fn watch_and_drain_round_trip_through_the_socket() {
        let socket = temp_socket("watch-drain");
        let (requester, _rx) = snapshot_channel(Duration::from_secs(1));
        let (health_requester, _hrx) = health_channel(Duration::from_secs(1));
        let (state_requester, _srx) = state_channel(Duration::from_secs(1));
        let (watch_requester, watch_rx) = watch_channel(Duration::from_secs(1));
        let (drain_requester, drain_rx) = drain_channel(Duration::from_secs(1));
        let (queue_stats_requester, _queue_stats_rx) =
            queue_stats_channel(Duration::from_secs(1));
        let service = SnapshotService::bind_with_capture(
            &socket,
            requester,
            health_requester,
            state_requester,
            watch_requester,
            drain_requester,
            queue_stats_requester,
        )
        .unwrap();
        let server = thread::spawn(move || {
            for stream in service.listener.incoming().take(2) {
                let _ = serve_one(
                    &mut stream.unwrap(),
                    &service.requester,
                    &service.health_requester,
                    &service.state_requester,
                    &service.watch_requester,
                    &service.drain_requester,
                    &service.queue_stats_requester,
                    None,
                );
            }
        });
        let collector = thread::spawn(move || {
            let request = watch_rx.recv().unwrap();
            assert_eq!(request.watch, vec![3, 7, 11]);
            request
                .reply
                .try_send(WatchAck {
                    accepted: 3,
                    rejected: 0,
                })
                .unwrap();
            let request = drain_rx.recv().unwrap();
            assert_eq!(request.limit, 100);
            request
                .reply
                .try_send(CaptureBatch {
                    records: vec![CaptureRecord {
                        timestamp_ns: 42,
                        circuit_id: 3,
                        src: "192.0.2.1".parse().unwrap(),
                        dst: "198.51.100.2".parse().unwrap(),
                        src_port: 1234,
                        dst_port: 80,
                        ip_protocol: 6,
                        tos: 0,
                        size: 1500,
                        tcp_flags: 0x10,
                        tcp_window: 64240,
                        tcp_tsval: 7,
                        tcp_tsecr: 9,
                        direction: CaptureDirection::Download,
                        prefix: CapturePrefix([0xAB; CAPTURE_PREFIX_BYTES]),
                    }],
                    dropped: 5,
                })
                .unwrap();
        });
        let ack = connect_watch(&socket, &[3, 7, 11]).unwrap();
        assert_eq!(ack.accepted, 3);
        let batch = connect_drain(&socket, 100).unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].circuit_id, 3);
        assert_eq!(
            batch.records[0].prefix,
            CapturePrefix([0xAB; CAPTURE_PREFIX_BYTES])
        );
        assert_eq!(batch.dropped, 5);
        server.join().unwrap();
        collector.join().unwrap();
        let _ = fs::remove_dir_all(socket.parent().unwrap());
    }

    /// K2 slice 2 §7: the QueueStats kind round-trips through the socket +
    /// memfd envelope, covering the EMPTY state (no slots ⇒ no rows, no
    /// error) and a populated detail row (per-direction tins + mirrors + the
    /// 50-bucket RTT histogram).
    #[test]
    fn queue_stats_round_trips_empty_and_populated() {
        let socket = temp_socket("queue-stats");
        let (requester, _rx) = snapshot_channel(Duration::from_secs(1));
        let (health_requester, _hrx) = health_channel(Duration::from_secs(1));
        let (state_requester, _srx) = state_channel(Duration::from_secs(1));
        let (watch_requester, _wrx) = watch_channel(Duration::from_secs(1));
        let (drain_requester, _drx) = drain_channel(Duration::from_secs(1));
        let (queue_stats_requester, queue_stats_rx) = queue_stats_channel(Duration::from_secs(1));
        let service = SnapshotService::bind_with_capture(
            &socket,
            requester,
            health_requester,
            state_requester,
            watch_requester,
            drain_requester,
            queue_stats_requester,
        )
        .unwrap();
        let server = thread::spawn(move || {
            for stream in service.listener.incoming().take(2) {
                let _ = serve_one(
                    &mut stream.unwrap(),
                    &service.requester,
                    &service.health_requester,
                    &service.state_requester,
                    &service.watch_requester,
                    &service.drain_requester,
                    &service.queue_stats_requester,
                    None,
                );
            }
        });
        let collector = thread::spawn(move || {
            let request = queue_stats_rx.recv().unwrap();
            assert!(request.slots.is_empty());
            request.reply.try_send(QueueStatsReply::default()).unwrap();
            let request = queue_stats_rx.recv().unwrap();
            assert_eq!(request.slots, vec![7]);
            let mut histogram = [0_u64; crate::LATENCY_BUCKETS];
            histogram[0] = 11;
            histogram[49] = 2;
            request
                .reply
                .try_send(QueueStatsReply {
                    rows: vec![crate::QdiscDetailRow {
                        slot_id: 7,
                        download: crate::QdiscDirectionRow {
                            tins: vec![crate::QdiscTinRow {
                                sent_bytes: 1_500,
                                backlog_bytes: 9_000,
                                sojourn_p99_ns: 4_200_000,
                            }],
                            memory_used: 12_345,
                            queue_depth_pkts: 3,
                            drop_pkts: 4,
                            drop_pkts_queue_full: 1,
                            aqm_marks: 2,
                        },
                        upload: crate::QdiscDirectionRow::default(),
                        rtt_histogram: crate::RttHistogram(histogram),
                    }],
                    ..QueueStatsReply::default()
                })
                .unwrap();
        });
        let empty = connect_queue_stats(&socket, &[]).unwrap();
        assert!(empty.rows.is_empty());
        let reply = connect_queue_stats(&socket, &[7]).unwrap();
        assert_eq!(reply.rows.len(), 1);
        let row = &reply.rows[0];
        assert_eq!(row.slot_id, 7);
        assert_eq!(row.download.tins.len(), 1);
        assert_eq!(row.download.tins[0].sent_bytes, 1_500);
        assert_eq!(row.download.tins[0].backlog_bytes, 9_000);
        assert_eq!(row.download.tins[0].sojourn_p99_ns, 4_200_000);
        assert_eq!(row.download.memory_used, 12_345);
        assert_eq!(row.download.queue_depth_pkts, 3);
        assert_eq!(row.download.drop_pkts, 4);
        assert_eq!(row.download.drop_pkts_queue_full, 1);
        assert_eq!(row.download.aqm_marks, 2);
        assert_eq!(row.rtt_histogram.0[0], 11);
        assert_eq!(row.rtt_histogram.0[49], 2);
        server.join().unwrap();
        collector.join().unwrap();
        let _ = fs::remove_dir_all(socket.parent().unwrap());
    }
}

#[cfg(test)]
mod completion_contract_tests {
    use super::*;
    use crate::{CompletionBatch, CompletionEvent};

    fn cbor_map(pairs: &[(&str, serde_cbor::Value)]) -> Vec<u8> {
        let mut map = std::collections::BTreeMap::new();
        for (k, v) in pairs {
            map.insert(serde_cbor::Value::Text((*k).to_string()), v.clone());
        }
        serde_cbor::to_vec(&serde_cbor::Value::Map(map)).unwrap()
    }

    fn text(s: &str) -> serde_cbor::Value {
        serde_cbor::Value::Text(s.to_string())
    }
    fn int(n: i128) -> serde_cbor::Value {
        serde_cbor::Value::Integer(n)
    }

    /// OLD lqosd -> NEW bridge: a pre-completion request frame (no cursor)
    /// deserializes with the cursor defaulted to 0 (additive wire contract).
    #[test]
    fn old_request_decodes_on_a_new_bridge() {
        let old = cbor_map(&[("kind", text("snapshot")), ("limit", int(100))]);
        let parsed: WireRequest = serde_cbor::from_slice(&old).unwrap();
        assert_eq!(parsed.kind, RequestKind::Snapshot);
        assert_eq!(parsed.limit, 100);
        assert_eq!(parsed.epoch, 0);
        assert_eq!(parsed.after_seq, 0);
    }

    /// NEW lqosd -> OLD bridge: the drain kind is unknown to an old decoder, so
    /// its decode FAILS and the consumer falls back (the intended rollout).
    #[test]
    fn new_drain_request_is_unknown_to_an_old_bridge() {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        #[allow(dead_code)]
        enum OldRequestKind {
            Snapshot,
            Health,
            State,
            SetWatchList,
            DrainCapture,
            QueueStats,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct OldWireRequest {
            kind: OldRequestKind,
            #[serde(default = "default_limit")]
            limit: usize,
        }
        let new = cbor_map(&[
            ("kind", text("drain_completions")),
            ("limit", int(8192)),
            ("epoch", int(7)),
            ("after_seq", int(42)),
        ]);
        let parsed: WireRequest = serde_cbor::from_slice(&new).unwrap();
        assert_eq!(parsed.kind, RequestKind::DrainCompletions);
        assert_eq!(parsed.limit, 8192);
        assert_eq!(parsed.epoch, 7);
        assert_eq!(parsed.after_seq, 42);
        assert_eq!(
            request_bound(RequestKind::DrainCompletions),
            MAX_COMPLETIONS_REQUEST_BYTES
        );
        assert!(
            serde_cbor::from_slice::<OldWireRequest>(&new).is_err(),
            "an OLD bridge cannot know drain_completions => the client falls back"
        );
    }

    /// The additive record/batch types round-trip on the wire.
    #[test]
    fn completion_batch_round_trips() {
        let batch = CompletionBatch {
            epoch: 3,
            events: vec![CompletionEvent {
                seq: 9,
                ..Default::default()
            }],
            next_seq: 10,
            oldest_seq: 1,
            dropped: 2,
        };
        let bytes = serde_cbor::to_vec(&batch).unwrap();
        assert_eq!(serde_cbor::from_slice::<CompletionBatch>(&bytes).unwrap(), batch);
    }

    fn sample_worker_affinity() -> WorkerAffinity {
        WorkerAffinity {
            worker_id: 0,
            lcore_id: Some(2),
            numa_node: Some(0),
            rx_queue_ids: Some(vec![0, 1]),
            tx_queue_id: Some(0),
            shared_tx_queue: Some(false),
            counters: None,
        }
    }

    fn sample_state_with_workers(workers: Option<WorkerAffinityReport>) -> StateReport {
        StateReport {
            generation: "gen-affinity".to_string(),
            report_schema: 1,
            transaction: vec![1, 2, 3],
            slot_order: Vec::new(),
            content_key: "key".to_string(),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
            workers,
        }
    }

    /// Pins: `None` = unsupported, `Some(empty)` = supported/no queue;
    /// legacy consumers ignore the additive field; old data decodes as None.
    #[test]
    fn worker_affinity_report_is_additive_and_legacy_compatible() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct LegacyStateReport {
            generation: String,
            report_schema: u32,
            transaction: Vec<u8>,
            #[serde(default)]
            slot_order: Vec<u8>,
            #[serde(default)]
            content_key: String,
            #[serde(default)]
            desired_world_content_key: String,
            #[serde(default)]
            applied_circuit_capacity: Option<u32>,
            #[serde(default)]
            owners: Vec<u8>,
        }

        let report = WorkerAffinityReport {
            schema: 1,
            rx_queue_count: Some(2),
            tx_queue_count: Some(2),
            counters: WorkerCounterSupport::Unsupported,
            workers: vec![
                sample_worker_affinity(),
                WorkerAffinity {
                    worker_id: 1,
                    lcore_id: None,
                    numa_node: None,
                    rx_queue_ids: Some(Vec::new()),
                    tx_queue_id: None,
                    shared_tx_queue: None,
                    counters: None,
                },
            ],
        };
        assert_eq!(
            report.workers[0].rx_queue_ids,
            Some(vec![0, 1]),
            "Some(non-empty) is a supported queue list"
        );
        assert_eq!(
            report.workers[1].rx_queue_ids,
            Some(Vec::new()),
            "Some(empty) is supported/no queue, distinct from None"
        );
        assert_eq!(report.workers[1].tx_queue_id, None, "None is unsupported");
        assert_eq!(
            report.counters,
            WorkerCounterSupport::Unsupported,
            "v1 counters are explicitly unsupported"
        );

        let state = sample_state_with_workers(Some(report.clone()));
        let bytes = serde_cbor::to_vec(&state).expect("state serializes");
        let decoded: StateReport = serde_cbor::from_slice(&bytes).expect("new reader decodes");
        assert_eq!(decoded.workers, Some(report));

        // A LEGACY consumer (old struct without `workers`) must ignore the
        // additive field; this is the forward-compat direction.
        let legacy: LegacyStateReport =
            serde_cbor::from_slice(&bytes).expect("legacy decoder ignores workers");
        assert_eq!(legacy.generation, "gen-affinity");
        assert_eq!(legacy.report_schema, 1);

        // A new reader of OLD data (no workers field) must see None.
        let old = sample_state_with_workers(None);
        let old_bytes = serde_cbor::to_vec(&old).expect("old state serializes");
        let new_reader: StateReport =
            serde_cbor::from_slice(&old_bytes).expect("new reader decodes old data");
        assert_eq!(new_reader.workers, None, "absent report is None, not zeros");

        // A truly ABSENT-field decode: the legacy shape serializes NO
        // `workers` key at all (not `workers: null`), so this exercises
        // `#[serde(default)]` rather than explicit null.
        let legacy_absent = LegacyStateReport {
            generation: "gen-legacy".to_string(),
            report_schema: 1,
            transaction: vec![1, 2, 3],
            slot_order: Vec::new(),
            content_key: "key".to_string(),
            desired_world_content_key: String::new(),
            applied_circuit_capacity: None,
            owners: Vec::new(),
        };
        let absent_bytes = serde_cbor::to_vec(&legacy_absent).expect("legacy map serializes");
        let absent_reader: StateReport = serde_cbor::from_slice(&absent_bytes)
            .expect("serde(default) decodes a map with no workers key");
        assert_eq!(absent_reader.workers, None);
        assert_eq!(absent_reader.generation, "gen-legacy");
    }

    #[test]
    fn worker_counter_support_serde_is_snake_case() {
        let bytes = serde_cbor::to_vec(&WorkerCounterSupport::Unsupported).unwrap();
        assert!(
            bytes.windows(b"unsupported".len()).any(|w| w == &b"unsupported"[..]),
            "the enum wire spelling is snake_case: {bytes:?}"
        );
    }

}

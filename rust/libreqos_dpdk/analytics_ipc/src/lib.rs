//! Same-tree, unversioned analytics snapshot contract and local transport.

mod ipc;
#[cfg(feature = "tui")]
pub mod tui;

pub use ipc::{
    DEFAULT_MAX_ROWS, DrainCompletionsRequest, DrainCompletionsRequester, DrainRequest,
    DrainRequester, HealthRequest, HealthRequester, IpcError, MAX_QUEUE_STATS_REQUEST_BYTES,
    MAX_WATCH_REQUEST_BYTES, QUEUE_STATS_SOCKET_CALLS, QueueStatsRequest, QueueStatsRequester,
    RequestKind, SnapshotRequest, SnapshotRequester, SnapshotService, StateReply, StateReport,
    StateReportDiag, StateRequest, StateRequester, StateUnavailableReason, WatchRequest,
    WatchRequester, WorkerAffinity, WorkerAffinityReport, WorkerCounterSupport, WorkerCounters,
    connect_drain,
    connect_drain_completions, connect_health, connect_queue_stats, connect_snapshot, connect_state,
    connect_watch, drain_channel, drain_completions_channel, health_channel, queue_stats_channel,
    queue_stats_socket_calls,
    snapshot_channel, state_channel, state_report_diag, watch_channel,
};

use serde::{Deserialize, Serialize};
use std::net::IpAddr;

pub const LATENCY_BUCKETS: usize = 50;

/// Heimdall re-derivation (heimdall_bridge_rederivation.md §1.1): the
/// watch table is 64 direct-mapped slots, so a watch set carries at most
/// 64 leaf ids. Slot 0 (`CircuitId::best_effort()`) is reserved and never
/// settable.
pub const MAX_WATCH_ENTRIES: usize = 64;

/// §1.2: the captured frame prefix length (the kernel's PACKET_OCTET_SIZE).
pub const CAPTURE_PREFIX_BYTES: usize = 128;

/// §1.2: the captured frame prefix is the INNER Ethernet frame post-decap
/// (the shaper's view; link_type 1 stays honest). Fixed-size in memory;
/// serialized as a CBOR byte string (serde's array impls stop at 32).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CapturePrefix(pub [u8; CAPTURE_PREFIX_BYTES]);

impl serde::Serialize for CapturePrefix {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for CapturePrefix {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PrefixVisitor;
        impl<'de> serde::de::Visitor<'de> for PrefixVisitor {
            type Value = CapturePrefix;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(formatter, "a {CAPTURE_PREFIX_BYTES}-byte prefix")
            }
            fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
                if value.len() != CAPTURE_PREFIX_BYTES {
                    return Err(E::invalid_length(value.len(), &self));
                }
                let mut out = [0_u8; CAPTURE_PREFIX_BYTES];
                out.copy_from_slice(value);
                Ok(CapturePrefix(out))
            }
        }
        deserializer.deserialize_bytes(PrefixVisitor)
    }
}

/// The direction a captured packet was classified on (the bridge sees both
/// RX ports; the kernel XDP hook saw one physical direction).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureDirection {
    /// Traffic toward the subscriber.
    Download,
    /// Traffic from the subscriber.
    Upload,
}

/// One captured packet — the HeimdallEvent shape (§1.2) plus the bridge's
/// leaf key, direction, and boot-relative timestamp. The kernel's
/// `packet_data` prefix is the inner frame's first [`CAPTURE_PREFIX_BYTES`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRecord {
    /// Bridge boot-relative nanoseconds (TSC → ns; §1.6).
    pub timestamp_ns: u64,
    /// The watched leaf slot id this packet classified to.
    pub circuit_id: u64,
    /// Source IP address.
    pub src: IpAddr,
    /// Destination IP address.
    pub dst: IpAddr,
    /// Source port, or ICMP type.
    pub src_port: u16,
    /// Destination port.
    pub dst_port: u16,
    /// IP protocol number.
    pub ip_protocol: u8,
    /// IP header TOS value.
    pub tos: u8,
    /// Total packet size in bytes.
    pub size: u32,
    /// TCP flags.
    pub tcp_flags: u8,
    /// TCP window size.
    pub tcp_window: u16,
    /// TCP TSval (best-effort; options walk on capture hits only).
    pub tcp_tsval: u32,
    /// TCP TSecr (best-effort).
    pub tcp_tsecr: u32,
    /// Classification direction.
    pub direction: CaptureDirection,
    /// The inner Ethernet frame prefix (first [`CAPTURE_PREFIX_BYTES`]).
    pub prefix: CapturePrefix,
}

/// One drain reply: the records available since the last drain plus the
/// cumulative ring drop count (drop-on-full, never backpressure; §1.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureBatch {
    /// Captured records, oldest first.
    pub records: Vec<CaptureRecord>,
    /// Cumulative records dropped because a capture ring was full.
    pub dropped: u64,
}

/// The compact flow-completion summary the bridge's completion ring keeps
/// (the ~144 B record). ADDITIVE (the flow-completion feed): consumers that
/// predate it never receive one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionEvent {
    /// Monotonic per-epoch sequence (the once-only authority).
    pub seq: u64,
    /// The completed flow's identity.
    pub flow: FlowId,
    /// Boot-relative milliseconds (the `FlowRow` convention).
    pub first_seen_millis: u64,
    pub last_seen_millis: u64,
    /// ACTUAL (delivered) cumulative counters at completion.
    pub actual_bytes_down: u64,
    pub actual_bytes_up: u64,
    pub actual_packets_down: u64,
    pub actual_packets_up: u64,
    /// Per-direction TCP retransmit counts.
    pub tcp_retransmits_down: u64,
    pub tcp_retransmits_up: u64,
    /// Per-direction RTT medians (microseconds); `0` = absent (the `FlowRow`
    /// convention the exporter's `median_us` uses).
    pub rtt_median_down_us: u32,
    pub rtt_median_up_us: u32,
    /// The terminal state: 0 = Fin, 1 = Reset, 2 = Idle (the completion
    /// predicate the ring emits on).
    pub terminal_state: u8,
}

/// One drained batch of [`CompletionEvent`]s — the bridge ring's reply.
/// ADDITIVE (the flow-completion feed).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionBatch {
    /// The bridge's ring epoch; a bridge restart resets it (the consumer
    /// re-bases its `(epoch, seq)` cursor).
    pub epoch: u64,
    /// The events, oldest first.
    pub events: Vec<CompletionEvent>,
    /// The sequence the consumer should pass as `after_seq` next.
    pub next_seq: u64,
    /// The oldest sequence still resident in the ring (a gap = loss).
    pub oldest_seq: u64,
    /// Cumulative events dropped/overwritten in this epoch.
    pub dropped: u64,
}

/// Acknowledges a [`RequestKind::SetWatchList`] request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchAck {
    /// Number of watch entries accepted into the region image.
    pub accepted: u32,
    /// Number of entries rejected (slot 0 or over [`MAX_WATCH_ENTRIES`]).
    pub rejected: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectionalCounters {
    pub download_bytes: u64,
    pub upload_bytes: u64,
}

/// K2 slice 2 (k2_delete_bias_sweep_derivation.md §7): per-direction
/// CUMULATIVE packet counters. lqtop differences these into its directional
/// pps columns; the byte counters stay [`DirectionalCounters`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectionalPackets {
    pub download_packets: u64,
    pub upload_packets: u64,
}

/// K2 slice 2 §7: whole-bridge qdisc totals (summed once by the collector).
/// Carries the aggregate `totalCakeStats` dashlets under the per-request
/// 64-id QueueStats bound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QdiscTotals {
    pub drop_pkts: u64,
    pub aqm_marks: u64,
    pub backlog_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostics {
    pub worker_push_drops: u64,
    pub flow_untracked_packets: u64,
    pub host_capacity_drops: u64,
    pub flow_capacity_drops: u64,
    /// §16(B) (`docs/bulk_swap_free_pending_accounting_card.md`): residual
    /// backlog freed at the bulk-swap point of no return, ACCOUNTED AS DROPS.
    /// `0` in steady state (the pre-commit drain flushes to empty).
    #[serde(default)]
    pub bulk_swap_dropped_packets: u64,
    #[serde(default)]
    pub bulk_swap_dropped_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct HostId {
    pub circuit_id: u64,
    pub subscriber_ip: IpAddr,
}

impl Default for HostId {
    fn default() -> Self {
        Self {
            circuit_id: 0,
            subscriber_ip: IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct FlowId {
    pub circuit_id: u64,
    pub flow_hash: u64,
    pub subscriber_ip: IpAddr,
    pub remote_ip: IpAddr,
    pub subscriber_port: u16,
    pub remote_port: u16,
    pub protocol: u8,
}

impl Default for FlowId {
    fn default() -> Self {
        Self {
            circuit_id: 0,
            flow_hash: 0,
            subscriber_ip: IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            remote_ip: IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            subscriber_port: 0,
            remote_port: 0,
            protocol: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowState {
    #[default]
    Active,
    Fin,
    Reset,
    Idle,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HostRow {
    pub id: HostId,
    pub actual: DirectionalCounters,
    pub shaped: DirectionalCounters,
    /// The merged RTT sample view (the union across directions; every
    /// pre-directional consumer reads this unchanged).
    pub rtt_samples_us: Vec<u32>,
    pub dns_samples_us: Vec<u32>,
    pub tcp_sent_packets: u64,
    pub tcp_retransmits: u64,
    pub last_seen_millis: u64,
    /// K2 slice 2 §7: cumulative per-direction packet counters (the pps
    /// source; additive field — pre-slice-2 readers serde-default to zero).
    #[serde(default)]
    pub enqueued_packets: DirectionalPackets,
    /// K2 slice 2 §7: cumulative per-direction sent (dequeued) packets.
    #[serde(default)]
    pub sent_packets: DirectionalPackets,
    /// Per-direction RTT samples (the host's download/upload rings).
    #[serde(default)]
    pub rtt_samples_down_us: Vec<u32>,
    #[serde(default)]
    pub rtt_samples_up_us: Vec<u32>,
    /// Per-direction retransmit observation times (boot-relative nanoseconds;
    /// the bounded ring). The exporter converts to unix seconds and PADS the
    /// array to the counted `tcp_retransmits_down/up` (Insight reads only the
    /// array length). ADDITIVE (`serde(default)`).
    #[serde(default)]
    pub retransmit_times_down_ns: Vec<u64>,
    #[serde(default)]
    pub retransmit_times_up_ns: Vec<u64>,
    /// Per-direction TCP retransmits (the host accumulates per direction at
    /// the flow-landing and live-observation merge sites).
    #[serde(default)]
    pub tcp_retransmits_down: u64,
    #[serde(default)]
    pub tcp_retransmits_up: u64,
    /// Per-direction TCP sent packets (the retransmit-% denominator).
    #[serde(default)]
    pub tcp_sent_packets_down: u64,
    #[serde(default)]
    pub tcp_sent_packets_up: u64,
    /// Per-direction ALL-protocol sent packets (the packets columns;
    /// sourced from `FlowDirectionDelta.sent_pkts`).
    #[serde(default)]
    pub sent_pkts_down: u64,
    #[serde(default)]
    pub sent_pkts_up: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowRow {
    pub id: FlowId,
    pub actual: DirectionalCounters,
    pub shaped: DirectionalCounters,
    /// The merged RTT sample view (the union across directions; every
    /// pre-directional consumer reads this unchanged).
    pub rtt_samples_us: Vec<u32>,
    pub tcp_sent_packets: u64,
    pub tcp_retransmits: u64,
    pub first_seen_millis: u64,
    pub last_seen_millis: u64,
    pub state: FlowState,
    /// Per-direction RTT samples.
    #[serde(default)]
    pub rtt_samples_down_us: Vec<u32>,
    #[serde(default)]
    pub rtt_samples_up_us: Vec<u32>,
    /// Per-direction retransmit observation times (boot-relative nanoseconds;
    /// the bounded ring). The LTS2 exporter converts to unix seconds and PADS
    /// the array to the counted `tcp_retransmits_down/up` — Insight computes
    /// the retransmit % from the array LENGTH. ADDITIVE (`serde(default)`).
    #[serde(default)]
    pub retransmit_times_down_ns: Vec<u64>,
    #[serde(default)]
    pub retransmit_times_up_ns: Vec<u64>,
    #[serde(default)]
    pub tcp_retransmits_down: u64,
    #[serde(default)]
    pub tcp_retransmits_up: u64,
    #[serde(default)]
    pub tcp_sent_packets_down: u64,
    #[serde(default)]
    pub tcp_sent_packets_up: u64,
    /// Per-direction ALL-protocol sent packets (the packets columns;
    /// sourced from `FlowDirectionDelta.sent_pkts`).
    #[serde(default)]
    pub sent_pkts_down: u64,
    #[serde(default)]
    pub sent_pkts_up: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsSnapshot {
    pub captured_at_millis: u64,
    pub actual: Option<DirectionalCounters>,
    pub shaped: Option<DirectionalCounters>,
    /// `.66.1` (Path-A wire; `docs/lqos_74t_66_1_path_a_wire_card.md`): the
    /// WHOLE-BOX **mapped** class byte totals per direction — summed at the
    /// SOURCE (per enqueue) BEFORE the capped host maps' `truncate`/limit, so
    /// they never under-count. "mapped" = a circuit was resolved for the
    /// subscriber IP. ADDITIVE (`serde(default)`): an older reader sees `None`.
    /// Cumulative (like [`Self::actual`]/[`Self::shaped`]); `.66.2` differences
    /// them to rates.
    #[serde(default)]
    pub mapped: Option<DirectionalCounters>,
    /// `.66.1`: the whole-box **unmapped** (`CircuitId::best_effort`) class byte
    /// totals per direction. ADDITIVE (`serde(default)`); see [`Self::mapped`].
    #[serde(default)]
    pub unmapped: Option<DirectionalCounters>,
    pub hosts: Vec<HostRow>,
    /// `.49.1`: the SEPARATE best-effort (unmapped, `circuit_id == 0`) host
    /// rows. ADDITIVE (`serde(default)`), so its own budget can never evict the
    /// shaped `hosts` rows under an unmapped flood.
    #[serde(default)]
    pub best_effort_hosts: Vec<HostRow>,
    pub active_flows: Vec<FlowRow>,
    pub recent_flows: Vec<FlowRow>,
    pub diagnostics: Diagnostics,
    /// K2 slice 2 §7: whole-bridge qdisc totals (additive field).
    #[serde(default)]
    pub qdisc_totals: QdiscTotals,
}

impl AnalyticsSnapshot {
    pub fn sort_and_limit(&mut self, limit: usize) {
        self.hosts.sort_by(|a, b| {
            traffic(&b.actual)
                .cmp(&traffic(&a.actual))
                .then_with(|| a.id.cmp(&b.id))
        });
        self.active_flows.sort_by(flow_order);
        self.recent_flows.sort_by(flow_order);
        self.hosts.truncate(limit);
        // `.49.1`: the best-effort rows have their OWN budget so they can never
        // evict the shaped `hosts` above.
        self.best_effort_hosts.sort_by(|a, b| {
            traffic(&b.actual)
                .cmp(&traffic(&a.actual))
                .then_with(|| a.id.cmp(&b.id))
        });
        self.best_effort_hosts.truncate(limit);
        self.active_flows.truncate(limit);
        self.recent_flows.truncate(limit);
    }
}

fn flow_order(a: &FlowRow, b: &FlowRow) -> std::cmp::Ordering {
    traffic(&b.actual)
        .cmp(&traffic(&a.actual))
        .then_with(|| a.id.cmp(&b.id))
}

fn traffic(counters: &DirectionalCounters) -> u64 {
    counters
        .download_bytes
        .saturating_add(counters.upload_bytes)
}

/// Canonical latency bucket: 10 ms wide, with every value >=490 ms in bucket 49.
pub fn latency_bucket(latency_us: u32) -> usize {
    ((latency_us / 10_000) as usize).min(LATENCY_BUCKETS - 1)
}

pub fn latency_histogram(samples_us: &[u32]) -> [u32; LATENCY_BUCKETS] {
    let mut bins = [0; LATENCY_BUCKETS];
    for sample in samples_us {
        bins[latency_bucket(*sample)] += 1;
    }
    bins
}

/// K2 slice 2 §7: a `RequestKind::QueueStats` request carries at most this
/// many slot ids; the kind's own 1 KiB-class request-frame bound is enforced
/// post-decode.
pub const MAX_QUEUE_STATS_IDS: usize = 64;

/// K2 slice 2 §7: the per-tin block. `sent_bytes` is the tin's dequeued
/// (sent) bytes, `backlog_bytes` its instantaneous queue bytes, and
/// `sojourn_p99_ns` the tin's p99 queue sojourn (the ruling
/// replacing Linux `tc`'s `base_delay_us` — a measured sojourn, not a
/// pacing-derived baseline).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QdiscTinRow {
    pub sent_bytes: u64,
    pub backlog_bytes: u64,
    pub sojourn_p99_ns: u64,
}

/// K2 slice 2 §7: one direction's per-circuit qdisc detail. A circuit has two
/// independent leaf qdiscs (download/upload), so the detail row carries both.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QdiscDirectionRow {
    pub tins: Vec<QdiscTinRow>,
    pub memory_used: u64,
    pub queue_depth_pkts: u64,
    pub drop_pkts: u64,
    pub drop_pkts_queue_full: u64,
    pub aqm_marks: u64,
}

/// K2 slice 2 §7: one requested slot's qdisc detail. `rtt_histogram` is the
/// circuit's pooled per-host RTT histogram (the bridge keeps one pooled RTT
/// set per host, not per direction).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QdiscDetailRow {
    pub slot_id: u64,
    pub download: QdiscDirectionRow,
    pub upload: QdiscDirectionRow,
    pub rtt_histogram: RttHistogram,
}

/// K2 slice 2 §7: the `RequestKind::QueueStats` reply — one row per requested
/// slot id.
///
/// `.115` (`docs/74t115_design.md`): the reply also carries the **sample
/// provenance** of the collector fold the rows came from. All fields are
/// additive with `serde(default)`: an old bridge omits them (None/0), and an
/// old lqosd ignores them. `sampled_at_unix_ms` is the collector's wall-clock
/// sample time; `elapsed_ms` is the MONOTONIC fold-to-fold interval (None on
/// the first sample of an epoch); `sample_seq` is the GLOBAL actor sample
/// counter, scoped by `sample_epoch_unix_ms`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueStatsReply {
    pub rows: Vec<QdiscDetailRow>,
    /// `.115`: collector sample publication time (CLOCK_REALTIME, Unix ms).
    #[serde(default)]
    pub sampled_at_unix_ms: Option<u64>,
    /// `.115`: actor fold-to-fold interval (CLOCK_MONOTONIC, ms); None on the
    /// first sample of an epoch.
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    /// `.115`: nominal collector cadence (ms); 0 = unknown.
    #[serde(default)]
    pub cadence_ms: u64,
    /// `.115`: actor start marker (Unix ms); a change = a new actor epoch.
    #[serde(default)]
    pub sample_epoch_unix_ms: u64,
    /// `.115`: GLOBAL actor sample counter (+1 per collector sample).
    #[serde(default)]
    pub sample_seq: u64,
    /// `.115`: the actor missed a fold (`elapsed_ms > 2*cadence`) or a worker
    /// restarted since the previous sample.
    #[serde(default)]
    pub actor_gap: bool,
    /// `.115`: a worker restart/reset was observed since the previous sample.
    #[serde(default)]
    pub counter_reset: bool,
}

/// K2 slice 2 §7: the `[u64; LATENCY_BUCKETS]` per-circuit RTT histogram.
/// serde's array impls stop at 32, so this carries a hand-written seq impl
/// (the [`CapturePrefix`] precedent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RttHistogram(pub [u64; LATENCY_BUCKETS]);

impl Default for RttHistogram {
    fn default() -> Self {
        Self([0; LATENCY_BUCKETS])
    }
}

impl serde::Serialize for RttHistogram {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(LATENCY_BUCKETS))?;
        for bucket in self.0.iter() {
            seq.serialize_element(bucket)?;
        }
        seq.end()
    }
}

impl<'de> serde::Deserialize<'de> for RttHistogram {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct HistogramVisitor;
        impl<'de> serde::de::Visitor<'de> for HistogramVisitor {
            type Value = RttHistogram;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(formatter, "a {LATENCY_BUCKETS}-bucket RTT histogram")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut buckets = [0_u64; LATENCY_BUCKETS];
                for (index, bucket) in buckets.iter_mut().enumerate() {
                    *bucket = seq
                        .next_element()?
                        .ok_or_else(|| serde::de::Error::invalid_length(index, &self))?;
                }
                Ok(RttHistogram(buckets))
            }
        }
        deserializer.deserialize_seq(HistogramVisitor)
    }
}

// ---------------------------------------------------------------------------
// Supervision health report (docs/supervision_contract_card.md §2A / §7).
// Read-only L1-L4 ladder view served on the existing analytics socket; the
// lqosd Supervisor's probe policy (cadence, thresholds, MEDIUM-1's
// busy/timeout-never-restart-worthy distinction) is PRODUCT-side.

/// Top-level health view. `status` classification for the Supervisor:
/// "starting" (no worker rollups yet — a fresh boot), "healthy" (serving;
/// busy workers are HEALTHY — sustained silence is the only escalation
/// class per §7 MEDIUM-1), "degraded" (a worker reports deadline_aqm
/// degraded).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HealthReport {
    pub status: String,
    pub version: String,
    pub uptime_ms: u64,
    /// The completion-ring epoch (the flow-completion feed). `0` = the bridge
    /// predates the ring (the consumer falls back to the snapshot export).
    /// ADDITIVE (`serde(default)`): older readers ignore it; a new reader
    /// defaults it to 0 against an older bridge.
    #[serde(default)]
    pub completion_ring_epoch: u64,
    /// §2.6/§8.1 (flow-completion feed): the ring health + the SPLIT
    /// projection-drop counters. ADDITIVE (`serde(default)`): an older reader
    /// ignores them; the snapshot's `flow_capacity_drops` stays the aggregate.
    #[serde(default)]
    pub completion_ring_emitted: u64,
    #[serde(default)]
    pub completion_ring_dropped: u64,
    #[serde(default)]
    pub completion_ring_overwritten: u64,
    #[serde(default)]
    pub completion_ring_forced: u64,
    #[serde(default)]
    pub completion_ring_depth: u64,
    #[serde(default)]
    pub completion_ring_oldest_seq: u64,
    #[serde(default)]
    pub completion_ring_next_seq: u64,
    #[serde(default)]
    pub admission_refusals: u64,
    #[serde(default)]
    pub tcp_cap_drops: u64,
    #[serde(default)]
    pub host_cap_drops: u64,
    #[serde(default)]
    pub recent_full_drops: u64,
    #[serde(default)]
    pub pending_telemetry_drops: u64,
    pub workers: Vec<WorkerHealth>,
    pub channel: ChannelHealth,
    pub barrier: BarrierHealth,
    /// R1 (remove_lpm_completeness_card §R1): latched at boot when the
    /// classifier holds ZERO prefixes — all traffic forwards best-effort
    /// UNSHAPED. ADDITIVE field (serde default): older lqosd readers
    /// ignore it; the status string already folds it to "degraded".
    #[serde(default)]
    pub classification_empty: bool,
    /// AVAIL (CARD `v3_availability_hardening_card` §7 b4): the boot topology
    /// tier the bridge is shaping from — "fresh" | "last_known_good" |
    /// "bootstrap_trunk" | "pass_through"; empty = unknown. ADDITIVE
    /// (serde default): older readers ignore it; the read-only System Health
    /// join maps it to lqosd's ONE `TopologySource`.
    #[serde(default)]
    pub topology_tier: String,
    /// AVAIL (CARD §7 R1): named NON-DATA-PATH subsystems latched DEGRADED at
    /// boot (analytics_ipc / stats_collector / publish_driver /
    /// stats_summary). ADDITIVE (serde default); the status string folds it
    /// to "degraded".
    #[serde(default)]
    pub degraded_subsystems: Vec<String>,
    /// HOLD(2) (§0a): total boots observed — the DENOMINATOR of the fallback
    /// rate (`(boots - tiers["fresh"]) / boots`). ADDITIVE (serde default).
    #[serde(default)]
    pub boots: u64,
    /// HOLD(2) (§0a): seconds-since-epoch of the last boot (R1 time anchor).
    #[serde(default)]
    pub last_boot_unix: u64,
    /// HOLD(2) (§0a): per-tier boot counts (the fallback FREQUENCY across
    /// boots), `TopologyTier::as_str()` -> count. ADDITIVE (serde default).
    #[serde(default)]
    pub tier_counts: std::collections::BTreeMap<String, u64>,
}

/// Per-worker L3/L4 view (from the 1s utilization rollup stream).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkerHealth {
    pub queue_id: u16,
    /// Milliseconds since this worker's last utilization rollup merged;
    /// `None` = no rollups yet (the fresh-boot "starting" marker).
    pub rollup_age_ms: Option<u64>,
    pub laps: u64,
    pub busy_laps: u64,
    pub deadline_aqm_degraded: bool,
    pub unowned_admission_refusals: u64,
    pub parked_admission_refusals: u64,
}

/// Control-channel counters (L4; the driver thread mirrors them per tick).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelHealth {
    pub frames_in: u64,
    pub acks_out: u64,
    pub ack_drops: u64,
}

/// Barrier view (L4; the fence value is INFORMATIONAL per channel-card
/// G12 — never a two-generation identity).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BarrierHealth {
    pub armed_cmd: u64,
    pub fence_value: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_boundaries_are_canonical() {
        let bins = latency_histogram(&[0, 9_999, 10_000, 489_999, 490_000, u32::MAX]);
        assert_eq!(bins[0], 2);
        assert_eq!(bins[1], 1);
        assert_eq!(bins[48], 1);
        assert_eq!(bins[49], 2);
    }

    #[test]
    fn serialization_has_no_version_or_generation_field() {
        let value = serde_cbor::to_vec(&AnalyticsSnapshot::default()).unwrap();
        let text = String::from_utf8_lossy(&value);
        assert!(!text.contains("version"));
        assert!(!text.contains("generation"));
    }

    /// K2 slice 2 §7: the 50-bucket histogram round-trips through CBOR and a
    /// short sequence is rejected (the wire contract is exactly 50 buckets).
    #[test]
    fn rtt_histogram_round_trips_all_fifty_buckets() {
        let mut buckets = [0_u64; LATENCY_BUCKETS];
        for (index, bucket) in buckets.iter_mut().enumerate() {
            *bucket = index as u64 * 7 + 1;
        }
        let histogram = RttHistogram(buckets);
        let encoded = serde_cbor::to_vec(&histogram).unwrap();
        let decoded: RttHistogram = serde_cbor::from_slice(&encoded).unwrap();
        assert_eq!(decoded, histogram);
        let short = &buckets[..LATENCY_BUCKETS - 1];
        let encoded_short = serde_cbor::to_vec(&short).unwrap();
        assert!(serde_cbor::from_slice::<RttHistogram>(&encoded_short).is_err());
    }

    /// K2 slice 2 §7: the new snapshot totals block is additive — a
    /// pre-slice-2 frame deserializes to the zero default.
    #[test]
    fn pre_slice2_snapshot_frames_default_the_qdisc_totals() {
        let legacy = AnalyticsSnapshot::default();
        let encoded = serde_cbor::to_vec(&legacy).unwrap();
        let decoded: AnalyticsSnapshot = serde_cbor::from_slice(&encoded).unwrap();
        assert_eq!(decoded.qdisc_totals, QdiscTotals::default());
    }

    /// Directional wire §4 headroom re-check: the worst-case snapshot
    /// (every row at `DEFAULT_MAX_ROWS`, every sample vec at the 8-cap,
    /// every counter at `u64::MAX`) still fits the reply envelope.
    #[test]
    fn worst_case_snapshot_fits_the_reply_envelope() {
        let samples: Vec<u32> = (0..8).map(|index| u32::MAX - index).collect();
        // The flow merged RTT vec is the concatenation of two capped rings.
        let merged_samples: Vec<u32> = (0..16).map(|index| u32::MAX - index).collect();
        let max = u64::MAX;
        let host = HostRow {
            rtt_samples_us: samples.clone(),
            dns_samples_us: samples.clone(),
            rtt_samples_down_us: samples.clone(),
            rtt_samples_up_us: samples.clone(),
            tcp_sent_packets: max,
            tcp_retransmits: max,
            tcp_sent_packets_down: max,
            tcp_sent_packets_up: max,
            tcp_retransmits_down: max,
            tcp_retransmits_up: max,
            sent_pkts_down: max,
            sent_pkts_up: max,
            last_seen_millis: max,
            ..HostRow::default()
        };
        let flow = FlowRow {
            rtt_samples_us: merged_samples,
            rtt_samples_down_us: samples.clone(),
            rtt_samples_up_us: samples.clone(),
            tcp_sent_packets: max,
            tcp_retransmits: max,
            tcp_sent_packets_down: max,
            tcp_sent_packets_up: max,
            tcp_retransmits_down: max,
            tcp_retransmits_up: max,
            sent_pkts_down: max,
            sent_pkts_up: max,
            first_seen_millis: max,
            last_seen_millis: max,
            ..FlowRow::default()
        };
        let snapshot = AnalyticsSnapshot {
            hosts: vec![host; DEFAULT_MAX_ROWS],
            active_flows: vec![flow.clone(); DEFAULT_MAX_ROWS],
            recent_flows: vec![flow; DEFAULT_MAX_ROWS],
            ..AnalyticsSnapshot::default()
        };
        let encoded = serde_cbor::to_vec(&snapshot).unwrap();
        eprintln!(
            "worst-case snapshot bytes = {} (envelope {})",
            encoded.len(),
            crate::ipc::MAX_SNAPSHOT_BYTES
        );
        assert!(
            encoded.len() <= crate::ipc::MAX_SNAPSHOT_BYTES,
            "worst-case snapshot is {} B > envelope {} B",
            encoded.len(),
            crate::ipc::MAX_SNAPSHOT_BYTES
        );
    }

    /// Directional wire: the per-direction fields are additive — a frame
    /// produced before they existed decodes with them defaulted (the
    /// merged/scalar fields are untouched).
    #[test]
    fn pre_directional_frames_default_the_new_fields() {
        let fields = [
            "rtt_samples_down_us",
            "rtt_samples_up_us",
            "tcp_retransmits_down",
            "tcp_retransmits_up",
            "tcp_sent_packets_down",
            "tcp_sent_packets_up",
            "sent_pkts_down",
            "sent_pkts_up",
        ];
        let strip = |value: serde_cbor::Value| {
            let mut value = value;
            if let serde_cbor::Value::Map(map) = &mut value {
                map.retain(|key, _| match key {
                    serde_cbor::Value::Text(name) => !fields.contains(&name.as_str()),
                    _ => true,
                });
            }
            serde_cbor::to_vec(&value).unwrap()
        };

        let host_bytes = strip(serde_cbor::value::to_value(HostRow::default()).unwrap());
        let host: HostRow = serde_cbor::from_slice(&host_bytes).unwrap();
        assert!(host.rtt_samples_down_us.is_empty());
        assert!(host.rtt_samples_up_us.is_empty());
        assert_eq!(host.tcp_retransmits_down, 0);
        assert_eq!(host.tcp_sent_packets_up, 0);
        assert_eq!(host.sent_pkts_down, 0);

        let flow_bytes = strip(serde_cbor::value::to_value(FlowRow::default()).unwrap());
        let flow: FlowRow = serde_cbor::from_slice(&flow_bytes).unwrap();
        assert!(flow.rtt_samples_down_us.is_empty());
        assert_eq!(flow.tcp_retransmits_up, 0);
        assert_eq!(flow.sent_pkts_up, 0);
    }

    /// `.66.1` (Path-A wire): the whole-box class totals are ADDITIVE — a frame
    /// produced before they existed decodes with them `None` (no wire break).
    #[test]
    fn pre_path_a_frames_default_the_class_totals() {
        let fields = ["mapped", "unmapped"];
        let strip = |value: serde_cbor::Value| {
            let mut value = value;
            if let serde_cbor::Value::Map(map) = &mut value {
                map.retain(|key, _| match key {
                    serde_cbor::Value::Text(name) => !fields.contains(&name.as_str()),
                    _ => true,
                });
            }
            serde_cbor::to_vec(&value).unwrap()
        };
        let bytes = strip(serde_cbor::value::to_value(AnalyticsSnapshot::default()).unwrap());
        let snapshot: AnalyticsSnapshot = serde_cbor::from_slice(&bytes).unwrap();
        assert_eq!(snapshot.mapped, None);
        assert_eq!(snapshot.unmapped, None);
    }

    /// `.66.1`: the class totals round-trip through the wire.
    #[test]
    fn path_a_class_totals_round_trip() {
        let snapshot = AnalyticsSnapshot {
            mapped: Some(DirectionalCounters {
                download_bytes: 1_000,
                upload_bytes: 2_000,
            }),
            unmapped: Some(DirectionalCounters {
                download_bytes: 300,
                upload_bytes: 40,
            }),
            ..AnalyticsSnapshot::default()
        };
        let encoded = serde_cbor::to_vec(&snapshot).unwrap();
        let decoded: AnalyticsSnapshot = serde_cbor::from_slice(&encoded).unwrap();
        assert_eq!(decoded.mapped, snapshot.mapped);
        assert_eq!(decoded.unmapped, snapshot.unmapped);
    }
}

#[cfg(test)]
mod queue_stats_metadata_tests {
    use super::*;

    /// The pre-`.115` reply shape: rows only.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct OldReply {
        rows: Vec<QdiscDetailRow>,
    }

    #[test]
    fn queue_stats_metadata_decodes_both_ways() {
        // Old bridge -> new lqosd: the metadata defaults.
        let old = serde_cbor::to_vec(&OldReply { rows: Vec::new() }).unwrap();
        let decoded: QueueStatsReply = serde_cbor::from_slice(&old).unwrap();
        assert!(decoded.sampled_at_unix_ms.is_none());
        assert_eq!(decoded.sample_seq, 0);
        assert!(!decoded.actor_gap);

        // New bridge -> old lqosd: unknown fields are ignored.
        let new = QueueStatsReply {
            rows: Vec::new(),
            sampled_at_unix_ms: Some(1),
            elapsed_ms: Some(2),
            cadence_ms: 200,
            sample_epoch_unix_ms: 3,
            sample_seq: 4,
            actor_gap: true,
            counter_reset: true,
        };
        let bytes = serde_cbor::to_vec(&new).unwrap();
        let decoded: OldReply = serde_cbor::from_slice(&bytes).unwrap();
        assert!(decoded.rows.is_empty());

        // New -> new round-trips the metadata.
        let decoded: QueueStatsReply = serde_cbor::from_slice(&bytes).unwrap();
        assert_eq!(decoded, new);
    }

    #[test]
    fn worst_case_queue_stats_reply_fits_the_budget() {
        let direction = QdiscDirectionRow {
            tins: (0..8)
                .map(|index| QdiscTinRow {
                    sent_bytes: u64::MAX - index,
                    backlog_bytes: u64::MAX,
                    sojourn_p99_ns: u64::MAX,
                })
                .collect(),
            memory_used: u64::MAX,
            queue_depth_pkts: u64::MAX,
            drop_pkts: u64::MAX,
            drop_pkts_queue_full: u64::MAX,
            aqm_marks: u64::MAX,
        };
        let rows = (0..64)
            .map(|slot| QdiscDetailRow {
                slot_id: slot,
                download: direction.clone(),
                upload: direction.clone(),
                rtt_histogram: RttHistogram::default(),
            })
            .collect();
        let reply = QueueStatsReply {
            rows,
            sampled_at_unix_ms: Some(u64::MAX),
            elapsed_ms: Some(u64::MAX),
            cadence_ms: u64::MAX,
            sample_epoch_unix_ms: u64::MAX,
            sample_seq: u64::MAX,
            actor_gap: true,
            counter_reset: true,
        };
        let bytes = serde_cbor::to_vec(&reply).unwrap();
        assert!(
            bytes.len() <= 8 * 1024 * 1024,
            "worst-case QueueStats reply is {} B",
            bytes.len()
        );
    }
}

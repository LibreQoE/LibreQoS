//! Cluster-2 flow history: PART A (recent_flows retention) + PART B (the
//! aggregate history ring). Card: `docs/flow_history_ring_derivation.md`
//! §2 + the §8 #12 folds (binding).
//!
//! This module owns the single-writer state and the immutable published
//! snapshot. Everything runs on the existing bridge-analytics poll thread;
//! nothing here touches a dataplane hot path. The §4 oracles live in the
//! test module at the bottom.
//!
//! Increment 2 wires the actor (`bridge_analytics::mod`) and repoints the
//! flow-explorer endpoints (`node_manager::local_api::{flow_explorer,
//! flow_map}`) at this snapshot.

use analytics_ipc::{AnalyticsSnapshot, FlowId, FlowRow, FlowState};
use fxhash::{FxHashMap, FxHashSet};
use lqos_utils::units::DownUpOrder;
use std::collections::VecDeque;
use std::mem;
use std::net::IpAddr;
use std::sync::Arc;

use crate::throughput_tracker::flow_data::{AsnAggregate, FlowProtocol};

/// PART A bound: the freshest finished-flow summaries retained.
pub const RECENT_FLOW_CAP: usize = 10_000;
/// PART B: 24 h at 1-minute granularity.
pub const RING_ENTRIES: usize = 1_440;
/// PART B: per-entry per-dimension top-K before the `other` lump.
pub const TOP_K: usize = 256;
/// PART B: wall-clock rollup bucket.
pub const ROLLUP_MILLIS: u64 = 60_000;

/// One class's bytes for both directions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClassSlot {
    pub down: u64,
    pub up: u64,
}

impl ClassSlot {
    fn add(&mut self, other: ClassSlot) {
        self.down = self.down.saturating_add(other.down);
        self.up = self.up.saturating_add(other.up);
    }

    pub fn total(&self) -> u64 {
        self.down.saturating_add(self.up)
    }
}

/// `None` for a counter reset / first sight (the seed rebases, no rate).
fn delta(prev: DownUpOrder<u64>, cur: DownUpOrder<u64>) -> Option<ClassSlot> {
    let down = cur.down.checked_sub(prev.down)?;
    let up = cur.up.checked_sub(prev.up)?;
    Some(ClassSlot { down, up })
}

/// The bridge flow's protocol NAME, matching the kernel classifier's display
/// form: the remote port is the service-port analogue the kernel keyed on.
fn protocol_name(row: &FlowRow) -> String {
    FlowProtocol::from_parts(row.id.protocol, row.id.remote_port, row.id.subscriber_port).to_string()
}

/// The RTT median in nanoseconds over raw microsecond samples (the same
/// summary K2 slice 1 publishes for lqtop); 0 when there are no samples.
fn rtt_median_nanos_over(samples: &[u32]) -> u64 {
    let mut samples = samples.to_vec();
    samples.sort_unstable();
    samples
        .get(samples.len() / 2)
        .map(|us| u64::from(*us) * 1_000)
        .unwrap_or(0)
}

/// The MERGED RTT median in nanoseconds (the union-across-directions view).
fn rtt_median_nanos(row: &FlowRow) -> u64 {
    rtt_median_nanos_over(&row.rtt_samples_us)
}

/// The resolved class for one flow's remote IP (GeoTable join). An unknown
/// ASN is `None` and an unknown country is the empty ISO string — kept in
/// the maps so every dimension conserves the full tick total (§8 fold 3).
/// The ring keys the country dimension on the ISO code (`flag`) because the
/// flow-explorer `country_timeline_data` query key is the ISO code. The
/// display name + lat/lon ride PART A (the current views), not the ring.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedClass {
    pub asn: Option<u32>,
    pub country_iso: String,
    pub country_name: String,
    pub lat: f64,
    pub lon: f64,
}

/// One retained finished flow (PART A). The raw `FlowRow` summary + the
/// resolved class; no aggregation. Carries the fields the cold-path
/// endpoint builders need (the kernel `TimeBuffer` equivalent): the class,
/// the display country name, the geo point, the RTT medians (nanos) and the
/// TCP retransmit count. Per-flow `throughput` (bytes/sec) is the named
/// honest-empty limitation (no bridge time-series per flow); the cumulative
/// per-direction PACKETS are carried (`packets`).
///
/// The `ether_protocol_summary` EXACT reproduction (review
/// `docs/recent_flows_ether_summary_answer.md` §1) needs the per-direction
/// packet counts + the per-direction RTT medians + the address family, so
/// all three ride here rather than being derived/merged at the endpoint.
#[derive(Clone, Debug, PartialEq)]
pub struct RecentFlow {
    pub flow_id: FlowId,
    pub state: FlowState,
    pub first_seen_millis: u64,
    pub last_seen_millis: u64,
    pub bytes: DownUpOrder<u64>,
    /// The cumulative per-direction SENT packets (`FlowRow.sent_pkts_*`) —
    /// the exact analogue of the legacy `packets_sent` the ether summary sums.
    pub packets: DownUpOrder<u64>,
    pub asn: Option<u32>,
    pub country_iso: String,
    pub country_name: String,
    pub protocol: String,
    pub lat: f64,
    pub lon: f64,
    /// The MERGED RTT median (nanos) — the existing views' contract.
    pub rtt_nanos: u64,
    /// The per-direction RTT medians (nanos): the ether summary's
    /// `get_summary_rtt_as_nanos(Download/Upload)` analogue (0 when the
    /// direction has no samples).
    pub rtt_nanos_down: u64,
    pub rtt_nanos_up: u64,
    /// The subscriber address family (`key.local_ip.is_v4()` analogue): the
    /// ether summary splits v4/v6 on it.
    pub is_v4: bool,
    pub tcp_retransmits: u64,
}

/// One dimension's top-K + the byte-exact `other` tail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DimensionTop<K> {
    pub entries: Vec<(K, ClassSlot)>,
    pub other: ClassSlot,
}

impl<K: Ord + Clone> Default for DimensionTop<K> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            other: ClassSlot::default(),
        }
    }
}

impl<K: Ord + Clone> DimensionTop<K> {
    /// Top-K by total bytes (deterministic key tie-break); the tail lumps
    /// byte-exactly into `other` (misattribution impossible by arithmetic).
    fn build(map: FxHashMap<K, ClassSlot>) -> Self {
        let mut items: Vec<(K, ClassSlot)> = map.into_iter().collect();
        items.sort_by(|a, b| {
            b.1.total()
                .cmp(&a.1.total())
                .then_with(|| a.0.cmp(&b.0))
        });
        let mut entries = Vec::with_capacity(TOP_K.min(items.len()));
        let mut other = ClassSlot::default();
        for (key, slot) in items {
            if entries.len() < TOP_K {
                entries.push((key, slot));
            } else {
                other.add(slot);
            }
        }
        Self { entries, other }
    }

    /// The named class plus the `other` lump (the per-entry dimension total).
    pub fn total(&self) -> ClassSlot {
        let mut total = self.other;
        for (_, slot) in &self.entries {
            total.add(*slot);
        }
        total
    }

    /// The named slot only (`other` is exposed separately, never guessed).
    pub fn get(&self, key: &K) -> Option<ClassSlot> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, s)| *s)
    }
}

#[derive(Clone, Debug, Default)]
struct Accumulators {
    /// `None` = unknown ASN (conserved, not dropped).
    asn: FxHashMap<Option<u32>, ClassSlot>,
    /// `""` = unknown country (conserved, not dropped).
    country: FxHashMap<String, ClassSlot>,
    /// The protocol NAME (`FlowProtocol` display form) — the same string the
    /// protocol timeline query carries, so ring and query align.
    protocol: FxHashMap<String, ClassSlot>,
    /// The per-ASN heatmap aggregate for the ASN store: bytes + per-direction
    /// packets + retransmits, plus the WINDOW-SEMANTICS RTT count/sum/min/max
 /// folded from each flow's current latency-ring tail; RTT is
    /// NOT under the byte conservation identity).  Keyed by the KNOWN ASN only
    /// (`None`/`0` are dropped by the store), so the ASN conservation identity
    /// is over known-ASN flows (disclosed).
    asn_heat: FxHashMap<u32, AsnAggregate>,
    /// Per-tick RAW tail samples per known ASN per direction, collected only to
    /// compute the TRUE per-tick p50/p90 (card v3 pin A); drained and cleared
    /// every tick (never retained — no sample memory).
    asn_rtt: FxHashMap<u32, (Vec<u32>, Vec<u32>)>,
    new_flows: u64,
}

impl Accumulators {
    fn add(
        &mut self,
        class: &ResolvedClass,
        protocol: &str,
        slot: ClassSlot,
        packets: ClassSlot,
        retransmits: ClassSlot,
    ) {
        self.asn.entry(class.asn).or_default().add(slot);
        self.country
            .entry(class.country_iso.clone())
            .or_default()
            .add(slot);
        self.protocol
            .entry(protocol.to_string())
            .or_default()
            .add(slot);
        if let Some(asn) = class.asn.filter(|asn| *asn != 0) {
            let agg = self.asn_heat.entry(asn).or_default();
            agg.bytes.down = agg.bytes.down.saturating_add(slot.down);
            agg.bytes.up = agg.bytes.up.saturating_add(slot.up);
            agg.packets.down = agg.packets.down.saturating_add(packets.down);
            agg.packets.up = agg.packets.up.saturating_add(packets.up);
            agg.retransmits.down = agg.retransmits.down.saturating_add(retransmits.down);
            agg.retransmits.up = agg.retransmits.up.saturating_add(retransmits.up);
        }
    }

 /// Fold one flow's CURRENT RTT tail into the per-ASN window numerics (the deriver
 /// each tick adds the tail's count/sum/min/max per direction —
    /// NOT a delta (the source is a bounded 8-slot ring, not a cumulative
    /// counter).  Residence-weighted: a sample counts once per tick it stays in
    /// the ring (disclosed).  RTT is NOT under the byte conservation identity.
    fn add_rtt(&mut self, class: &ResolvedClass, down: &[u32], up: &[u32]) {
        let Some(asn) = class.asn.filter(|asn| *asn != 0) else {
            return;
        };
        let (down_count, down_sum, down_min, down_max) = rtt_tail_stats(down);
        let (up_count, up_sum, up_min, up_max) = rtt_tail_stats(up);
        if down_count == 0 && up_count == 0 {
            return;
        }
        let agg = self.asn_heat.entry(asn).or_default();
        agg.rtt_count.down = agg.rtt_count.down.saturating_add(down_count);
        agg.rtt_count.up = agg.rtt_count.up.saturating_add(up_count);
        agg.rtt_sum_us.down = agg.rtt_sum_us.down.saturating_add(down_sum);
        agg.rtt_sum_us.up = agg.rtt_sum_us.up.saturating_add(up_sum);
        fold_min(&mut agg.rtt_min_us.down, down_min);
        fold_min(&mut agg.rtt_min_us.up, up_min);
        fold_max(&mut agg.rtt_max_us.down, down_max);
        fold_max(&mut agg.rtt_max_us.up, up_max);
        // card v3 pin A: collect this tick's raw tail samples for the TRUE
        // p50/p90 (cleared every tick — never retained).
        let tails = self.asn_rtt.entry(asn).or_default();
        tails.0.extend_from_slice(down);
        tails.1.extend_from_slice(up);
    }

    /// Compute the per-tick TRUE p50/p90 (v2's exact arithmetic) over the
    /// tails collected this tick and attach them to the per-ASN aggregate,
    /// then drop the raw buffers.
    fn finalize_asn_rtt(&mut self) {
        for (asn, (mut down, mut up)) in mem::take(&mut self.asn_rtt) {
            let (p50_down, p90_down) = rtt_percentiles(&mut down);
            let (p50_up, p90_up) = rtt_percentiles(&mut up);
            if let Some(agg) = self.asn_heat.get_mut(&asn) {
                agg.rtt_p50_down = p50_down;
                agg.rtt_p50_up = p50_up;
                agg.rtt_p90_down = p90_down;
                agg.rtt_p90_up = p90_up;
            }
        }
    }

    /// Drain this tick's per-ASN aggregate (byte/packet/retransmit deltas +
    /// the TRUE p50/p90), leaving the accumulator empty for the next tick.
    fn drain_asn_heat(&mut self) -> FxHashMap<u32, AsnAggregate> {
        self.finalize_asn_rtt();
        mem::take(&mut self.asn_heat)
    }
}

/// v2's `p50_p90` arithmetic (commit 4cefb987): p50 = the INTERPOLATED median
/// (odd → the middle; even → the mean of the two middles); p90 = the ceil-rank
/// nearest-rank (`rank = ceil(0.90·len) − 1`, clamped).  Sorts in place.
/// `(None, None)` for an empty set (the honest-empty rule).
fn rtt_percentiles(samples: &mut [u32]) -> (Option<f32>, Option<f32>) {
    if samples.is_empty() {
        return (None, None);
    }
    samples.sort_unstable();
    let n = samples.len();
    let p50 = if n % 2 == 1 {
        samples[n / 2] as f32
    } else {
        (samples[n / 2 - 1] as f32 + samples[n / 2] as f32) / 2.0
    };
    let rank = ((0.90 * n as f64).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1);
    (Some(p50), Some(samples[rank] as f32))
}

/// `(count, sum µs, min µs, max µs)` of one direction's RTT ring tail; all
/// zero when empty (the ring is chronological, oldest→newest).
fn rtt_tail_stats(samples: &[u32]) -> (u64, u64, u64, u64) {
    if samples.is_empty() {
        return (0, 0, 0, 0);
    }
    let mut sum = 0u64;
    let mut min = u64::MAX;
    let mut max = 0u64;
    for &sample in samples {
        let value = u64::from(sample);
        sum = sum.saturating_add(value);
        min = min.min(value);
        max = max.max(value);
    }
    (samples.len() as u64, sum, min, max)
}

/// Associative min with `0` as the "no sample" sentinel.
fn fold_min(acc: &mut u64, value: u64) {
    if value != 0 {
        *acc = if *acc == 0 { value } else { (*acc).min(value) };
    }
}

/// Associative max with `0` as the "no sample" sentinel.
fn fold_max(acc: &mut u64, value: u64) {
    *acc = (*acc).max(value);
}

/// One committed wall-clock minute (PART B).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RingEntry {
    pub start_millis: u64,
    /// Contributing folds, ≤ 60; 0 = a real missed bucket (never a fake zero).
    pub sampled_intervals: u16,
    pub asn: DimensionTop<Option<u32>>,
    pub country: DimensionTop<String>,
    pub protocol: DimensionTop<String>,
    pub new_flows: u64,
}

/// One class's windowed bucket (the additive `buckets` endpoint field).
/// `bytes` is the requested class's slot for the minute; `flows` is the
/// bucket-level new-flow count (`sampled_intervals == 0` marks a real
/// missed minute).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryBucket {
    pub start_millis: u64,
    pub sampled_intervals: u16,
    pub bytes: ClassSlot,
    pub flows: u64,
}

#[derive(Clone, Debug, Default)]
struct FlowSeed {
    bytes: DownUpOrder<u64>,
    /// Per-direction sent-packet counter (cumulative; differenced like bytes).
    packets: DownUpOrder<u64>,
    /// Per-direction TCP retransmit counter (cumulative; differenced).
    retransmits: DownUpOrder<u64>,
    class: ResolvedClass,
    protocol: String,
}

/// The immutable published projection (reader side).
#[derive(Clone, Debug, Default)]
pub struct FlowHistorySnapshot {
    pub ring: Arc<[RingEntry]>,
    pub recent: Arc<[RecentFlow]>,
    pub evicted: u64,
    pub captured_at_millis: u64,
    pub bucket_start_millis: Option<u64>,
}

impl FlowHistorySnapshot {
    /// The last `window` committed buckets (oldest → newest) for one ASN.
    /// The named slot only; the `other` lump is read from `entry.asn.other`
    /// by callers that disclose it.
    pub fn asn_buckets(&self, asn: Option<u32>, window: usize) -> Vec<HistoryBucket> {
        self.ring
            .iter()
            .rev()
            .take(window)
            .rev()
            .map(|e| HistoryBucket {
                start_millis: e.start_millis,
                sampled_intervals: e.sampled_intervals,
                bytes: e.asn.get(&asn).unwrap_or_default(),
                flows: e.new_flows,
            })
            .collect()
    }

    pub fn country_buckets(&self, country_iso: &str, window: usize) -> Vec<HistoryBucket> {
        let key = country_iso.to_string();
        self.ring
            .iter()
            .rev()
            .take(window)
            .rev()
            .map(|e| HistoryBucket {
                start_millis: e.start_millis,
                sampled_intervals: e.sampled_intervals,
                bytes: e.country.get(&key).unwrap_or_default(),
                flows: e.new_flows,
            })
            .collect()
    }

    pub fn protocol_buckets(&self, protocol: &str, window: usize) -> Vec<HistoryBucket> {
        let key = protocol.to_string();
        self.ring
            .iter()
            .rev()
            .take(window)
            .rev()
            .map(|e| HistoryBucket {
                start_millis: e.start_millis,
                sampled_intervals: e.sampled_intervals,
                bytes: e.protocol.get(&key).unwrap_or_default(),
                flows: e.new_flows,
            })
            .collect()
    }

    /// The flow-history DATA home: the per-country summary over the retained
    /// finished flows — the legacy `country_summary` shape (name, bytes,
    /// [median RTT, 0], ISO), sorted by down bytes desc. Thin over `recent`
    /// (the bridge snapshot).
    pub fn country_summary(&self) -> Vec<(String, DownUpOrder<u64>, [f32; 2], String)> {
        let mut groups: std::collections::BTreeMap<String, (String, DownUpOrder<u64>, Vec<f32>)> =
            std::collections::BTreeMap::new();
        for flow in self.recent.iter() {
            let entry = groups
                .entry(flow.country_name.clone())
                .or_insert_with(|| (flow.country_iso.clone(), DownUpOrder::zeroed(), Vec::new()));
            entry.1.down = entry.1.down.saturating_add(flow.bytes.down);
            entry.1.up = entry.1.up.saturating_add(flow.bytes.up);
            if flow.rtt_nanos > 0 {
                entry.2.push(flow.rtt_nanos as f32);
            }
        }
        let mut summary: Vec<(String, DownUpOrder<u64>, [f32; 2], String)> = groups
            .into_iter()
            .map(|(name, (iso, bytes, rtt))| (name, bytes, [median_f32(&rtt), 0.0], iso))
            .collect();
        summary.sort_by_key(|row| std::cmp::Reverse(row.1.down));
        summary
    }

    /// The lat/lon endpoint points over `recent` (the
    /// legacy `lat_lon_endpoints` shape), sorted by lat, deduped.
    pub fn lat_lon_endpoints(&self) -> Vec<(f64, f64, String, u64, f32)> {
        let mut points: Vec<(f64, f64, String, u64, f32)> = self
            .recent
            .iter()
            .filter(|flow| flow.lat != 0.0 && flow.lon != 0.0)
            .map(|flow| {
                (
                    flow.lat,
                    flow.lon,
                    flow.country_name.clone(),
                    flow.bytes.down,
                    flow.rtt_nanos as f32,
                )
            })
            .collect();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        points.dedup();
        points
    }

    /// The per-IP-protocol byte summary over `recent`
    /// (the legacy `ip_protocol_summary` shape), sorted by down bytes desc,
    /// top-10.
    pub fn ip_protocol_summary(&self) -> Vec<(String, DownUpOrder<u64>)> {
        let mut results: FxHashMap<String, DownUpOrder<u64>> = FxHashMap::default();
        for flow in self.recent.iter() {
            let entry = results
                .entry(flow.protocol.clone())
                .or_insert(DownUpOrder::zeroed());
            entry.checked_add(flow.bytes);
        }
        let mut results: Vec<(String, DownUpOrder<u64>)> = results.into_iter().collect();
        results.sort_by_key(|row| std::cmp::Reverse(row.1.down));
        results.truncate(10);
        results
    }
}

fn median_f32(slice: &[f32]) -> f32 {
    if slice.is_empty() {
        return 0.0;
    }
    let mut slice = slice.to_vec();
    slice.sort_by(|a, b| a.total_cmp(b));
    let mid = slice.len() / 2;
    if slice.len().is_multiple_of(2) {
        (slice[mid] + slice[mid - 1]) / 2.0
    } else {
        slice[mid]
    }
}

/// The single-writer flow-history state.
pub struct FlowHistory {
    seeds: FxHashMap<FlowId, FlowSeed>,
    recent: VecDeque<RecentFlow>,
    retained: FxHashSet<FlowId>,
    evicted: u64,
    acc: Accumulators,
    bucket_start: Option<u64>,
    ring: Vec<RingEntry>,
    head: usize,
    count: usize,
    sampled: u16,
    captured_at_millis: u64,
    ring_dirty: bool,
    ring_view: Arc<[RingEntry]>,
    recent_view: Arc<[RecentFlow]>,
}

impl Default for FlowHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl FlowHistory {
    pub fn new() -> Self {
        Self {
            seeds: FxHashMap::default(),
            recent: VecDeque::with_capacity(RECENT_FLOW_CAP),
            retained: FxHashSet::default(),
            evicted: 0,
            acc: Accumulators::default(),
            bucket_start: None,
            ring: vec![RingEntry::default(); RING_ENTRIES],
            head: 0,
            count: 0,
            sampled: 0,
            captured_at_millis: 0,
            ring_dirty: false,
            ring_view: Arc::from(Vec::new()),
            recent_view: Arc::from(Vec::new()),
        }
    }

    /// One poll tick: seed-diff the snapshot's flows into the live bucket,
    /// retain newly finished flows, and advance the wall-clock buckets.
    /// `resolve` performs the GeoTable join (injected for testability).
    pub fn fold<F>(
        &mut self,
        snapshot: &AnalyticsSnapshot,
        captured_at_millis: u64,
        mut resolve: F,
    ) where
        F: FnMut(IpAddr) -> ResolvedClass,
    {
        self.captured_at_millis = captured_at_millis;
        let bucket = captured_at_millis.saturating_sub(captured_at_millis % ROLLUP_MILLIS);
        self.advance_buckets(bucket);

        // PART B: per-flow DELTAS over active ∪ recent (never cumulative),
        // seed keyed by FlowId; absent flows prune their seed (§8 folds 1+2).
        let mut fresh: FxHashMap<FlowId, FlowSeed> = FxHashMap::default();
        for row in snapshot.active_flows.iter().chain(snapshot.recent_flows.iter()) {
            let key = row.id.clone();
            // D3 (card v3): the active/recent partitions are not disjoint by
            // contract, so a FlowId present in BOTH must be folded ONCE — or
            // the per-dimension conservation identity double-counts.
            if fresh.contains_key(&key) {
                continue;
            }
 // 3b: the flow-history/ASN/timeline buckets difference
            // the DELIVERED (shaped) series (the per-ASN current rate the
            // window caught reading demand).
            let cur = DownUpOrder::new(row.shaped.download_bytes, row.shaped.upload_bytes);
            let cur_packets = DownUpOrder::new(row.sent_pkts_down, row.sent_pkts_up);
            let cur_retransmits =
                DownUpOrder::new(row.tcp_retransmits_down, row.tcp_retransmits_up);
            match self.seeds.get(&key) {
                Some(prev) => {
                    if let Some(slot) = delta(prev.bytes, cur) {
                        // The packet/retransmit counters rebase with the byte
                        // delta (a reset prunes every dimension together).
                        let packets = delta(prev.packets, cur_packets).unwrap_or_default();
                        let retransmits =
                            delta(prev.retransmits, cur_retransmits).unwrap_or_default();
                        self.acc
                            .add(&prev.class, &prev.protocol, slot, packets, retransmits);
                    }
                    // RTT window: folded from the current tail every tick.
                    self.acc.add_rtt(
                        &prev.class,
                        &row.rtt_samples_down_us,
                        &row.rtt_samples_up_us,
                    );
                    fresh.insert(
                        key,
                        FlowSeed {
                            bytes: cur,
                            packets: cur_packets,
                            retransmits: cur_retransmits,
                            class: prev.class.clone(),
                            protocol: prev.protocol.clone(),
                        },
                    );
                }
                None => {
                    let class = resolve(key.remote_ip);
                    // RTT window: a first-sight flow contributes its tail too.
                    self.acc
                        .add_rtt(&class, &row.rtt_samples_down_us, &row.rtt_samples_up_us);
                    fresh.insert(
                        key.clone(),
                        FlowSeed {
                            bytes: cur,
                            packets: cur_packets,
                            retransmits: cur_retransmits,
                            class,
                            protocol: protocol_name(row),
                        },
                    );
                    self.acc.new_flows = self.acc.new_flows.saturating_add(1);
                }
            }
        }
        self.seeds = fresh;

        // PART A: retain each newly finished flow once (freshest first-in).
        for row in &snapshot.recent_flows {
            let finished = matches!(
                row.state,
                FlowState::Fin | FlowState::Reset | FlowState::Idle
            );
            if !finished || self.retained.contains(&row.id) {
                continue;
            }
            // The seed loop above resolved this flow (active ∪ recent), so
            // the class + protocol are available without a second join.
            let Some(seed) = self.seeds.get(&row.id) else {
                continue;
            };
            self.recent.push_back(RecentFlow {
                flow_id: row.id.clone(),
                state: row.state,
                first_seen_millis: row.first_seen_millis,
                last_seen_millis: row.last_seen_millis,
 // 3b: the retained flow bytes are the DELIVERED
                // (shaped) series — the demand stays the labeled Enqueued KPI.
                bytes: DownUpOrder::new(row.shaped.download_bytes, row.shaped.upload_bytes),
                // The cumulative per-direction SENT packets (the ether summary's
                // `packets` source; the legacy `packets_sent` analogue).
                packets: DownUpOrder::new(row.sent_pkts_down, row.sent_pkts_up),
                asn: seed.class.asn,
                country_iso: seed.class.country_iso.clone(),
                country_name: seed.class.country_name.clone(),
                protocol: seed.protocol.clone(),
                lat: seed.class.lat,
                lon: seed.class.lon,
                rtt_nanos: rtt_median_nanos(row),
                rtt_nanos_down: rtt_median_nanos_over(&row.rtt_samples_down_us),
                rtt_nanos_up: rtt_median_nanos_over(&row.rtt_samples_up_us),
                is_v4: row.id.subscriber_ip.is_ipv4(),
                tcp_retransmits: row.tcp_retransmits,
            });
            self.retained.insert(row.id.clone());
            while self.recent.len() > RECENT_FLOW_CAP {
                if let Some(evicted) = self.recent.pop_front() {
                    self.retained.remove(&evicted.flow_id);
                    self.evicted = self.evicted.saturating_add(1);
                }
            }
        }

        self.sampled = self.sampled.saturating_add(1);
    }

    /// Commit the live bucket and insert real (sampled=0) entries for every
    /// wall-clock minute that received no successful fold (§8 fold 4).
    fn advance_buckets(&mut self, bucket: u64) {
        match self.bucket_start {
            None => {
                self.bucket_start = Some(bucket);
                self.acc = Accumulators::default();
                self.sampled = 0;
            }
            Some(current) if bucket <= current => {}
            Some(current) => {
                self.commit(current, self.sampled);
                let mut skipped = current.saturating_add(ROLLUP_MILLIS);
                while skipped < bucket {
                    self.commit(skipped, 0);
                    skipped = skipped.saturating_add(ROLLUP_MILLIS);
                }
                self.bucket_start = Some(bucket);
                self.acc = Accumulators::default();
                self.sampled = 0;
            }
        }
    }

    fn commit(&mut self, start_millis: u64, sampled: u16) {
        let entry = RingEntry {
            start_millis,
            sampled_intervals: sampled.min(60),
            asn: DimensionTop::build(mem::take(&mut self.acc.asn)),
            country: DimensionTop::build(mem::take(&mut self.acc.country)),
            protocol: DimensionTop::build(mem::take(&mut self.acc.protocol)),
            new_flows: mem::take(&mut self.acc.new_flows),
        };
        self.ring[self.head] = entry;
        self.head = (self.head + 1) % RING_ENTRIES;
        self.count = (self.count + 1).min(RING_ENTRIES);
        self.ring_dirty = true;
    }

    /// Drain this tick's per-ASN aggregate (card v3 pin A): the byte/packet/
    /// retransmit DELTAS accumulated since the last drain plus the TRUE per-tick
    /// p50/p90.  The caller (`bridge_analytics` actor) writes it into the ASN
    /// heatmap store on the TICK clock (seconds); the fold itself stays free of
    /// global state.
    pub fn drain_asn_heat(&mut self) -> FxHashMap<u32, AsnAggregate> {
        self.acc.drain_asn_heat()
    }

    /// Build the immutable published snapshot. The ring view is rebuilt only
    /// when a commit happened; the PART A view is rebuilt each call.
    pub fn publish(&mut self) -> FlowHistorySnapshot {
        if self.ring_dirty {
            self.ring_view = self.ordered_ring().into();
            self.ring_dirty = false;
        }
        self.recent_view = self
            .recent
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .into();
        FlowHistorySnapshot {
            ring: self.ring_view.clone(),
            recent: self.recent_view.clone(),
            evicted: self.evicted,
            captured_at_millis: self.captured_at_millis,
            bucket_start_millis: self.bucket_start,
        }
    }

    fn ordered_ring(&self) -> Vec<RingEntry> {
        if self.count < RING_ENTRIES {
            self.ring[..self.count].to_vec()
        } else {
            let mut out = Vec::with_capacity(RING_ENTRIES);
            out.extend_from_slice(&self.ring[self.head..]);
            out.extend_from_slice(&self.ring[..self.head]);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::{DirectionalCounters, FlowRow};
    use std::collections::HashMap;

    fn flow(id: u64, remote: &str, protocol: u8, down: u64, up: u64, state: FlowState) -> FlowRow {
        FlowRow {
            id: FlowId {
                circuit_id: 1,
                flow_hash: id,
                subscriber_ip: "192.0.2.1".parse().expect("test IP"),
                remote_ip: remote.parse().expect("test IP"),
                subscriber_port: 1000 + id as u16,
                remote_port: 80,
                protocol,
            },
 // 3b sweep-fixture guard: demand != delivered.
            actual: DirectionalCounters {
                download_bytes: down + 10_000,
                upload_bytes: up + 10_000,
            },
            shaped: DirectionalCounters {
                download_bytes: down,
                upload_bytes: up,
            },
            rtt_samples_us: Vec::new(),
            tcp_sent_packets: 0,
            tcp_retransmits: 0,
            first_seen_millis: 0,
            last_seen_millis: 1,
            state,
            ..FlowRow::default()
        }
    }

    fn snapshot(active: Vec<FlowRow>, recent: Vec<FlowRow>) -> AnalyticsSnapshot {
        AnalyticsSnapshot {
            active_flows: active,
            recent_flows: recent,
            ..AnalyticsSnapshot::default()
        }
    }

    /// Resolver over a fixed map (remote IP -> (asn, country)).
    fn resolver() -> impl FnMut(IpAddr) -> ResolvedClass {
        let mut map: HashMap<IpAddr, ResolvedClass> = HashMap::new();
        map.insert(
            "198.51.100.1".parse().expect("test IP"),
            ResolvedClass {
                asn: Some(64500),
                country_iso: "US".into(),
                country_name: "United States".into(),
                lat: 1.0,
                lon: 2.0,
            },
        );
        map.insert(
            "198.51.100.2".parse().expect("test IP"),
            ResolvedClass {
                asn: Some(64501),
                country_iso: "DE".into(),
                country_name: "Germany".into(),
                lat: 3.0,
                lon: 4.0,
            },
        );
        move |ip: IpAddr| map.get(&ip).cloned().unwrap_or_default()
    }

    /// §4.1: each dimension's entry total == the tick-window total bytes.
    #[test]
    fn conservation_is_exact_per_dimension() {
        let mut h = FlowHistory::new();
        let t0 = 1_000_000_000_000u64;
        // First tick seeds (no delta).
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 100, 10, FlowState::Active)], vec![]), t0, resolver());
        // Delta tick: 150/20 then roll the bucket.
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 250, 30, FlowState::Active)], vec![]), t0 + 1_000, resolver());
        let roll = t0 + ROLLUP_MILLIS;
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 260, 32, FlowState::Active)], vec![]), roll, resolver());

        let snap = h.publish();
        let entry = snap.ring.last().expect("one committed entry");
        let expected = ClassSlot { down: 150, up: 20 };
        assert_eq!(entry.asn.total(), expected, "ASN map must conserve");
        assert_eq!(entry.country.total(), expected, "country map must conserve");
        assert_eq!(entry.protocol.total(), expected, "protocol map must conserve");
        assert_eq!(entry.asn.get(&Some(64500)), Some(expected));
        // Two folds sampled the bucket before the roll (the seed + the delta).
        assert_eq!(entry.sampled_intervals, 2);
    }

    /// ASN heatmap (card v3 pin A): the store is written PER TICK, and the
    /// per-ASN byte/packet/retransmit deltas are exact over KNOWN-ASN flows
    /// (the disclosed conservation universe) — the summed per-tick drains equal
    /// the folded deltas.
    #[test]
    fn asn_heatmap_conserves_known_asn_flows() {
        let t0 = 1_000_000u64;
        let mut h = FlowHistory::new();
        let row = |id: u64, remote: &str, down: u64, pkts: u64, rex: u64| {
            let mut f = flow(id, remote, 6, down, 0, FlowState::Active);
            f.sent_pkts_down = pkts;
            f.tcp_retransmits_down = rex;
            f
        };
        let mut totals: FxHashMap<u32, AsnAggregate> = FxHashMap::default();
        let mut tick = |h: &mut FlowHistory, snap: AnalyticsSnapshot, t: u64| {
            h.fold(&snap, t, resolver());
            for (asn, agg) in h.drain_asn_heat() {
                let e = totals.entry(asn).or_default();
                e.bytes.down += agg.bytes.down;
                e.packets.down += agg.packets.down;
                e.retransmits.down += agg.retransmits.down;
            }
        };

        tick(&mut h, snapshot(vec![row(1, "198.51.100.1", 1_000, 100, 1)], vec![]), t0);
        tick(&mut h, snapshot(vec![row(1, "198.51.100.1", 1_500, 150, 2)], vec![]), t0 + 1_000);
        tick(&mut h, snapshot(vec![row(2, "198.51.100.2", 2_000, 200, 0)], vec![]), t0 + 2_000);
        tick(&mut h, snapshot(vec![row(2, "198.51.100.2", 2_300, 230, 1)], vec![]), t0 + 3_000);

        let a = totals.get(&64500).expect("ASN 64500 present");
        assert_eq!(a.bytes.down, 500);
        assert_eq!(a.packets.down, 50);
        assert_eq!(a.retransmits.down, 1);

        let b = totals.get(&64501).expect("ASN 64501 present");
        assert_eq!(b.bytes.down, 300);
        assert_eq!(b.packets.down, 30);
        assert_eq!(b.retransmits.down, 1);
    }

    /// card v3 pin A: v2's EXACT percentile arithmetic — interpolated median
    /// for p50, ceil-rank for p90 — including odd/even/empty and len-1 edges.
    #[test]
    fn rtt_percentiles_match_v2_arithmetic() {
        assert_eq!(rtt_percentiles(&mut []), (None, None));
        assert_eq!(rtt_percentiles(&mut [7]), (Some(7.0), Some(7.0)));
        // even: the median interpolates the two middles.
        assert_eq!(rtt_percentiles(&mut [20, 10]), (Some(15.0), Some(20.0)));
        // odd: the middle; p90 = ceil(0.9*5)-1 = 4 -> the max.
        assert_eq!(rtt_percentiles(&mut [5, 1, 3, 2, 4]), (Some(3.0), Some(5.0)));
        // p90 rank = ceil(0.9*10)-1 = 8 -> the 9th smallest.
        let mut v: Vec<u32> = (1..=10).collect();
        assert_eq!(rtt_percentiles(&mut v), (Some(5.5), Some(9.0)));
    }

    /// D3 (card v3): a FlowId in BOTH `active_flows` and `recent_flows` is
    /// folded ONCE — otherwise the ASN conservation oracle double-counts.
    #[test]
    fn asn_heatmap_folds_a_flow_in_both_partitions_once() {
        let t0 = 2_000_000u64;
        let mut h = FlowHistory::new();
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 1_000, 0, FlowState::Active)], vec![]), t0, resolver());
        let _ = h.drain_asn_heat(); // drop the seed tick (no delta)
        let delta = flow(1, "198.51.100.1", 6, 1_500, 0, FlowState::Active);
        // The SAME FlowId in both partitions, delta 500 — must count once.
        h.fold(&snapshot(vec![delta.clone()], vec![delta]), t0 + 1_000, resolver());
        let aggregates = h.drain_asn_heat();
        assert_eq!(
            aggregates
                .get(&64500)
                .expect("ASN 64500 present")
                .bytes
                .down,
            500,
            "the active∪recent double-visit must fold once"
        );
    }

    /// §4.2: over TOP_K classes, the tail sums exactly into `other`.
    #[test]
    fn top_k_lump_is_byte_exact() {
        let mut map: FxHashMap<Option<u32>, ClassSlot> = FxHashMap::default();
        let n = TOP_K + 10;
        let mut expected_other = 0u64;
        for i in 0..n {
            let total = (n - i) as u64;
            map.insert(Some(i as u32), ClassSlot { down: total, up: 0 });
            if i >= TOP_K {
                expected_other += total;
            }
        }
        let top = DimensionTop::build(map);
        assert_eq!(top.entries.len(), TOP_K);
        assert_eq!(top.other.down, expected_other);
    }

    /// §4.3: a flow spanning buckets contributes its per-tick delta once each;
    /// no loss, no double count.
    #[test]
    fn boundary_straddler_counts_once_per_tick() {
        let mut h = FlowHistory::new();
        let t0 = 5_000_000_000_000u64;
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 0, 0, FlowState::Active)], vec![]), t0, resolver());
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 100, 0, FlowState::Active)], vec![]), t0 + 30_000, resolver());
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 400, 0, FlowState::Active)], vec![]), t0 + 90_000, resolver());
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 500, 0, FlowState::Active)], vec![]), t0 + 130_000, resolver());

        let snap = h.publish();
        let total: u64 = snap.ring.iter().map(|e| e.asn.total().down).sum();
        // The final in-progress bucket is not committed; committed deltas are
        // 100 (bucket 0) + 300 (bucket 1) = 400; the 100 in bucket 2 is live.
        assert_eq!(total, 400);
        let last = snap.ring.last().expect("committed bucket");
        assert_eq!(last.asn.total().down, 300, "the straddler split at the boundary");
    }

    /// §8 fold 4: a stalled poll records real (sampled=0) missed buckets.
    #[test]
    fn stalled_window_records_true_sample_counts() {
        let mut h = FlowHistory::new();
        let t0 = 2_000_000_000_000u64;
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 0, 0, FlowState::Active)], vec![]), t0, resolver());
        // Jump three minutes: commit the first bucket, then two sampled=0 buckets.
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 10, 0, FlowState::Active)], vec![]), t0 + 3 * ROLLUP_MILLIS + 1_000, resolver());
        let snap = h.publish();
        assert_eq!(snap.ring.len(), 3);
        assert_eq!(snap.ring[0].sampled_intervals, 1);
        assert_eq!(snap.ring[1].sampled_intervals, 0);
        assert_eq!(snap.ring[2].sampled_intervals, 0);
    }

    /// §4.4: PART A caps + discloses evictions.
    #[test]
    fn part_a_cap_and_evicted_counter() {
        let mut h = FlowHistory::new();
        let t0 = 3_000_000_000_000u64;
        let flows: Vec<FlowRow> = (0..RECENT_FLOW_CAP + 5)
            .map(|i| flow(i as u64, "198.51.100.1", 6, 1, 0, FlowState::Fin))
            .collect();
        h.fold(&snapshot(vec![], flows), t0, resolver());
        let snap = h.publish();
        assert_eq!(snap.recent.len(), RECENT_FLOW_CAP);
        assert_eq!(snap.evicted, 5);
    }

    /// §8 fold 1: a cumulative reset yields no delta (no double count).
    #[test]
    fn counter_reset_does_not_double_count() {
        let mut h = FlowHistory::new();
        let t0 = 4_000_000_000_000u64;
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 500, 0, FlowState::Active)], vec![]), t0, resolver());
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 5, 0, FlowState::Active)], vec![]), t0 + 1_000, resolver());
        // The roll commits the seed/reset bucket (no delta) and folds 10 into
        // the next bucket; one more roll commits that 10.
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 15, 0, FlowState::Active)], vec![]), t0 + ROLLUP_MILLIS, resolver());
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 25, 0, FlowState::Active)], vec![]), t0 + 2 * ROLLUP_MILLIS, resolver());
        let snap = h.publish();
        // Only the post-reset 10 bytes count.
        let total: u64 = snap.ring.iter().map(|e| e.asn.total().down).sum();
        assert_eq!(total, 10);
    }

    /// §8 fold 6: a 5-tuple reuse resets both sides (no cross-contamination).
    #[test]
    fn flow_id_reuse_resets_both_sides() {
        let mut h = FlowHistory::new();
        let t0 = 6_000_000_000_000u64;
        h.fold(&snapshot(vec![flow(7, "198.51.100.2", 6, 100, 50, FlowState::Active)], vec![]), t0, resolver());
        // Same FlowId, smaller values => reset; no delta for either flow.
        h.fold(&snapshot(vec![flow(7, "198.51.100.2", 6, 1, 1, FlowState::Active)], vec![]), t0 + 1_000, resolver());
        h.fold(&snapshot(vec![flow(7, "198.51.100.2", 6, 6, 4, FlowState::Active)], vec![]), t0 + ROLLUP_MILLIS, resolver());
        h.fold(&snapshot(vec![flow(7, "198.51.100.2", 6, 10, 6, FlowState::Active)], vec![]), t0 + 2 * ROLLUP_MILLIS, resolver());
        let snap = h.publish();
        let total: u64 = snap.ring.iter().map(|e| e.asn.total().total()).sum();
        assert_eq!(total, 8, "only the new flow's 5 down + 3 up count");
    }

    /// §4.5: a published snapshot is immutable under later folds.
    #[test]
    fn published_snapshot_is_immutable() {
        let mut h = FlowHistory::new();
        let t0 = 7_000_000_000_000u64;
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 0, 0, FlowState::Active)], vec![]), t0, resolver());
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 10, 0, FlowState::Active)], vec![]), t0 + ROLLUP_MILLIS, resolver());
        let before = h.publish();
        let before_len = before.ring.len();
        // Further folding must not mutate the already-published Arc.
        h.fold(&snapshot(vec![flow(1, "198.51.100.1", 6, 20, 0, FlowState::Active)], vec![]), t0 + 2 * ROLLUP_MILLIS, resolver());
        assert_eq!(before.ring.len(), before_len);
    }

    /// The ether-summary EXACT fields (review
    /// `docs/recent_flows_ether_summary_answer.md` §1): the retained flow
    /// carries the cumulative per-direction packets, the PER-DIRECTION RTT
    /// medians, and the address family — so the ether summary reproduces the
    /// legacy shape without a wire change.
    #[test]
    fn retained_flow_carries_the_ether_summary_fields() {
        let mut h = FlowHistory::new();
        let t0 = 9_000_000_000_000u64;
        let mut finished = flow(1, "198.51.100.1", 6, 700, 90, FlowState::Fin);
        finished.sent_pkts_down = 120;
        finished.sent_pkts_up = 80;
        finished.rtt_samples_down_us = vec![3_000, 1_000, 2_000];
        finished.rtt_samples_up_us = vec![7_000, 5_000, 6_000];
        // The seed tick resolves the class; the finished row is RETAINED.
        h.fold(&snapshot(vec![], vec![finished]), t0, resolver());
        let snap = h.publish();
        let retained = snap.recent.last().expect("the finished flow is retained");
        assert_eq!(
            retained.packets,
            DownUpOrder::new(120, 80),
            "the cumulative per-direction packets ride the retained flow"
        );
        assert_eq!(retained.rtt_nanos_down, 2_000_000, "down median (2000us)");
        assert_eq!(retained.rtt_nanos_up, 6_000_000, "up median (6000us)");
        assert!(retained.is_v4, "the subscriber family rides the retained flow");
    }
}

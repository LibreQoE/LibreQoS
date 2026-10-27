//! 3a cluster-1: the circuit/site temporal-heatmap accumulator — the
//! bridge-native replacement for the deleted kernel `TemporalHeatmap` feed
//! (docs/circuit_heatmap_accumulator_derivation.md, amended §1-§3 +
//! §6). Rides the analytics poll thread; no hot-path work.
//!
//! Per poll tick the accumulator differences its OWN raw per-circuit
//! cumulative counters (NEVER `CircuitLiveRow.bytes_per_second` — a rate) and
//! folds the interval delta into WALL-CLOCK minute buckets carrying
//! `sampled_intervals`. Rows exist only for OBSERVED-ACTIVE circuits (a
//! non-zero byte delta); a row persists the full retention window after its
//! last observation (idle history decays by age, never mid-window eviction).
//!
//! Published block values use the consumer field's EXISTING unit, pinned from
//! the deleted kernel call site (`tracking_data.rs` `add_sample`): bytes and
//! RTT are percent-utilization and milliseconds, retransmit is percent. The
//! current per-second-MEDIAN semantics become exact per-minute accounting —
//! DISCLOSED in the PR body. `p90` RTT fields stay honest-empty (the bridge
//! row carries only p50); `qoq_blocks` is honest-empty until the QoO
//! port lands.
//!
//! Increment 1 (this module): the mechanism + oracles, DECLARED-NOT-WIRED.
//! The poll fold + the circuit/site/global consumer repoints ride increment 2
//! (the accepted cluster-2 increment boundary).

use analytics_ipc::HostRow;
use fxhash::FxHashMap;
use lqos_utils::temporal_heatmap::{HeatmapBlocks, executive_retransmit_percent};
use std::collections::VecDeque;
use std::net::IpAddr;

/// The UI's native `TemporalHeatmap` span: 15 one-minute blocks.
pub const RETENTION_MINUTES: usize = 15;

const MINUTE_MILLIS: u64 = 60_000;
const RETENTION_MILLIS: u64 = RETENTION_MINUTES as u64 * MINUTE_MILLIS;

/// One committed wall-clock minute (the internal exact accounting).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeatmapMinute {
    pub start_millis: u64,
    /// Contributing folds, ≤ 60; 0 = a real missed bucket (never a fake zero).
    pub sampled_intervals: u16,
    pub down_bytes: u64,
    pub up_bytes: u64,
    /// Median of the tick p50s folded into this minute (ms).
    pub rtt_p50_down_ms: Option<f32>,
    pub rtt_p50_up_ms: Option<f32>,
    /// Median of the tick p90s folded into this minute (ms).
    pub rtt_p90_down_ms: Option<f32>,
    pub rtt_p90_up_ms: Option<f32>,
    /// Executive retransmit percent per direction (the kernel threshold applies).
    pub retransmit_down_pct: Option<f32>,
    pub retransmit_up_pct: Option<f32>,
    /// §lqos-74t.40 pin 3: the aligned-minute raw SUMS so the site rollup can be
    /// `Σretransmits/Σpackets` (a ratio-of-sums), never median-of-ratios-then-max.
    pub retransmit_down_count: u64,
    pub retransmit_down_packets: u64,
}

/// One circuit's cumulative counters as read from the wire's host rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RawCounters {
    down: u64,
    up: u64,
    tcp_packets: u64,
    tcp_retransmits: u64,
}

impl RawCounters {
    fn checked_delta(self, previous: Self) -> Option<Self> {
        Some(Self {
            down: self.down.checked_sub(previous.down)?,
            up: self.up.checked_sub(previous.up)?,
            tcp_packets: self.tcp_packets.checked_sub(previous.tcp_packets)?,
            tcp_retransmits: self.tcp_retransmits.checked_sub(previous.tcp_retransmits)?,
        })
    }

    fn total_bytes(self) -> u64 {
        self.down.saturating_add(self.up)
    }
}

/// The open (in-progress) minute's accumulators.
#[derive(Clone, Debug, Default)]
struct MinuteAccum {
    sampled_intervals: u16,
    down_bytes: u64,
    up_bytes: u64,
    rtt_p50_down_ms: Vec<f32>,
    rtt_p50_up_ms: Vec<f32>,
    rtt_p90_down_ms: Vec<f32>,
    rtt_p90_up_ms: Vec<f32>,
    retransmit_down_count: u64,
    retransmit_down_packets: u64,
}

impl MinuteAccum {
    fn fold(&mut self, delta: RawCounters, rtt: TickRtt) {
        self.sampled_intervals = self.sampled_intervals.saturating_add(1);
        self.down_bytes = self.down_bytes.saturating_add(delta.down);
        self.up_bytes = self.up_bytes.saturating_add(delta.up);
        if let Some(value) = rtt.p50_down_ms {
            self.rtt_p50_down_ms.push(value);
        }
        if let Some(value) = rtt.p50_up_ms {
            self.rtt_p50_up_ms.push(value);
        }
        if let Some(value) = rtt.p90_down_ms {
            self.rtt_p90_down_ms.push(value);
        }
        if let Some(value) = rtt.p90_up_ms {
            self.rtt_p90_up_ms.push(value);
        }
        // §lqos-74t.40 pin 3: accumulate the RAW counters (the ratio is taken
        // per-minute and again at the site as Σ/Σ, never per-tick).
        self.retransmit_down_count = self
            .retransmit_down_count
            .saturating_add(delta.tcp_retransmits);
        self.retransmit_down_packets = self
            .retransmit_down_packets
            .saturating_add(delta.tcp_packets);
    }

    fn into_minute(self, start_millis: u64) -> HeatmapMinute {
        HeatmapMinute {
            start_millis,
            sampled_intervals: self.sampled_intervals,
            down_bytes: self.down_bytes,
            up_bytes: self.up_bytes,
            rtt_p50_down_ms: median_f32(&self.rtt_p50_down_ms),
            rtt_p50_up_ms: median_f32(&self.rtt_p50_up_ms),
            rtt_p90_down_ms: median_f32(&self.rtt_p90_down_ms),
            rtt_p90_up_ms: median_f32(&self.rtt_p90_up_ms),
            retransmit_down_pct: executive_retransmit_percent(
                self.retransmit_down_count,
                self.retransmit_down_packets,
            ),
            retransmit_up_pct: executive_retransmit_percent(
                self.retransmit_down_count,
                self.retransmit_down_packets,
            ),
            retransmit_down_count: self.retransmit_down_count,
            retransmit_down_packets: self.retransmit_down_packets,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct CircuitRow {
    last_seen_millis: u64,
    bucket_start: Option<u64>,
    acc: MinuteAccum,
    /// Committed minutes, oldest first, strictly before the open minute.
    minutes: VecDeque<HeatmapMinute>,
    /// §`lqos-74t.40.1`: the latest poll's pooled RTT samples for this circuit.
    samples: CircuitRttSamples,
}

impl CircuitRow {
    /// Commit the open minute when the wall clock advanced and insert real
    /// (sampled=0) buckets for every skipped minute (the not-silent recovery
    /// rule: the outage is VISIBLE in `sampled_intervals`).
    fn advance(&mut self, bucket: u64) {
        match self.bucket_start {
            None => {
                self.bucket_start = Some(bucket);
                self.acc = MinuteAccum::default();
            }
            Some(current) if bucket <= current => {}
            Some(current) => {
                self.commit(current);
                let mut skipped = current.saturating_add(MINUTE_MILLIS);
                while skipped < bucket {
                    self.minutes.push_back(HeatmapMinute {
                        start_millis: skipped,
                        ..HeatmapMinute::default()
                    });
                    skipped = skipped.saturating_add(MINUTE_MILLIS);
                }
                self.trim();
                self.bucket_start = Some(bucket);
                self.acc = MinuteAccum::default();
            }
        }
    }

    fn commit(&mut self, start_millis: u64) {
        let acc = std::mem::take(&mut self.acc);
        self.minutes.push_back(acc.into_minute(start_millis));
        self.trim();
    }

    fn trim(&mut self) {
        while self.minutes.len() >= RETENTION_MINUTES {
            self.minutes.pop_front();
        }
    }

    /// Committed minutes + the open minute, oldest first.
    fn ordered_minutes(&self) -> Vec<HeatmapMinute> {
        let mut minutes: Vec<HeatmapMinute> = self.minutes.iter().cloned().collect();
        if let Some(bucket) = self.bucket_start {
            minutes.push(self.acc.clone().into_minute(bucket));
        }
        minutes
    }
}

/// §`lqos-74t.40.1`: one circuit's pooled RTT samples for the CURRENT poll (the
/// bridge per-circuit distribution — the circuit-view source the site RTT fold
/// and the QoO pool both consume).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CircuitRttSamples {
    pub down: Vec<u32>,
    pub up: Vec<u32>,
}

/// The immutable published view consumed by the executive heatmap surfaces.
#[derive(Clone, Debug, Default)]
pub struct CircuitHeatmapSnapshot {
    pub by_circuit: FxHashMap<String, Vec<HeatmapMinute>>,
    pub captured_at_millis: u64,
    /// §`lqos-74t.40.1`: the per-circuit pooled RTT samples (the current
    /// window) — the bridge distribution the site RTT fold + the QoO repoint
    /// consume. Pooling these preserves the pooled-SAMPLE percentile (never a
    /// p50-scalar statistic change).
    pub rtt_samples_by_circuit: FxHashMap<String, CircuitRttSamples>,
}

impl CircuitHeatmapSnapshot {
    /// Exact per-minute byte totals across a set of member circuits (the
    /// site/global rollup input). Minutes are aligned by wall clock.
    pub fn rollup<'a, I>(&self, circuits: I) -> Vec<HeatmapMinute>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let slices = circuits
            .into_iter()
            .filter_map(|circuit_id| self.by_circuit.get(circuit_id).map(Vec::as_slice));
        rollup_minute_slices(slices)
    }

    pub fn all_circuits(&self) -> impl Iterator<Item = &str> {
        self.by_circuit.keys().map(String::as_str)
    }
}

/// The bridge-side per-circuit temporal accumulator. One instance lives on the
/// poll thread; readers take a published snapshot.
#[derive(Debug, Default)]
pub struct CircuitHeatmapAccumulator {
    seeds: FxHashMap<String, RawCounters>,
    rows: FxHashMap<String, CircuitRow>,
    last_captured_millis: Option<u64>,
}

impl CircuitHeatmapAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observed_circuits(&self) -> usize {
        self.rows.len()
    }

    /// Build the immutable snapshot readers consume (increment 2).
    pub fn publish(&self) -> CircuitHeatmapSnapshot {
        CircuitHeatmapSnapshot {
            by_circuit: self
                .rows
                .iter()
                .map(|(circuit_id, row)| (circuit_id.clone(), row.ordered_minutes()))
                .collect(),
            captured_at_millis: self.last_captured_millis.unwrap_or(0),
            rtt_samples_by_circuit: self
                .rows
                .iter()
                .filter(|(_, row)| !row.samples.down.is_empty() || !row.samples.up.is_empty())
                .map(|(circuit_id, row)| (circuit_id.clone(), row.samples.clone()))
                .collect(),
        }
    }

    /// One poll tick. `hosts` are the wire's CUMULATIVE host rows; `resolve`
    /// joins a subscriber IP to its product circuit id (the same catalog join
    /// the live readers use). Returns nothing: increment 2 publishes the view.
    pub fn fold_tick<F>(&mut self, captured_at_millis: u64, hosts: &[HostRow], resolve: F)
    where
        F: Fn(&IpAddr) -> Option<String>,
    {
        let elapsed_ok = self
            .last_captured_millis
            .is_some_and(|previous| captured_at_millis > previous);
        self.last_captured_millis = Some(captured_at_millis);
        let bucket = captured_at_millis.saturating_sub(captured_at_millis % MINUTE_MILLIS);

        let raw = aggregate_hosts(hosts, resolve);
        let mut fresh_seeds: FxHashMap<String, RawCounters> = FxHashMap::default();
        for (circuit_id, per_circuit) in raw {
            let PerCircuit {
                counters,
                rtt,
                samples,
            } = per_circuit;
            if elapsed_ok
                && let Some(previous) = self.seeds.get(&circuit_id)
                && let Some(delta) = counters.checked_delta(*previous)
            {
                if delta.total_bytes() > 0 {
                    let row = self.rows.entry(circuit_id.clone()).or_default();
                    row.advance(bucket);
                    row.acc.fold(delta, rtt);
                    // §`lqos-74t.40.1`: carry the current pooled samples forward.
                    row.samples = samples;
                    row.last_seen_millis = captured_at_millis;
                } else if let Some(row) = self.rows.get_mut(&circuit_id) {
                    // Observed but idle this tick: advance the clock so a
                    // returning burst sees its own recovery bucket.
                    row.advance(bucket);
                }
            }
            // First sight (or a reset): seed only; the interval is unsampled.
            fresh_seeds.insert(circuit_id, counters);
        }
        // Vanished circuits drop their seed (the 10k seed-prune rule): on
        // return the first interval re-seeds (no-rate), never a fabricated
        // baseline. The row itself persists for the full retention window.
        self.seeds = fresh_seeds;
        self.rows
            .retain(|_, row| captured_at_millis.saturating_sub(row.last_seen_millis) <= RETENTION_MILLIS);
    }

    /// The exact per-minute accounting for one circuit, oldest first.
    pub fn minute_buckets(&self, circuit_id: &str) -> Vec<HeatmapMinute> {
        self.rows
            .get(circuit_id)
            .map(CircuitRow::ordered_minutes)
            .unwrap_or_default()
    }

    /// Publish one circuit's row in the UI's native `HeatmapBlocks` shape, in
    /// the field's existing unit (percent utilization for bytes, ms for RTT,
    /// percent for retransmit). Newest minute lands in the last block.
    pub fn blocks_for(
        &self,
        circuit_id: &str,
        plan_down_mbps: f32,
        plan_up_mbps: f32,
    ) -> HeatmapBlocks {
        let minutes = self.minute_buckets(circuit_id);
        blocks_from_minutes(&minutes, plan_down_mbps, plan_up_mbps)
    }

    /// Exact per-minute byte totals across a set of member circuits (the
    /// site/global rollup input). Minutes are aligned by wall clock.
    pub fn rollup_minutes<'a, I>(&self, circuits: I) -> Vec<HeatmapMinute>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let slices: Vec<Vec<HeatmapMinute>> = circuits
            .into_iter()
            .map(|circuit_id| self.minute_buckets(circuit_id))
            .collect();
        rollup_minute_slices(slices.iter().map(Vec::as_slice))
    }
}

/// Sum aligned per-minute rows (byte-exact; sampled_intervals = the max
/// contributing count; RTT/retransmit = the max across members).
fn rollup_minute_slices<'a, I>(slices: I) -> Vec<HeatmapMinute>
where
    I: IntoIterator<Item = &'a [HeatmapMinute]>,
{
    #[derive(Default)]
    struct Accum {
        sampled_intervals: u16,
        down_bytes: u64,
        up_bytes: u64,
        retransmit_down_count: u64,
        retransmit_down_packets: u64,
        p50_down: Vec<(f32, u64)>,
        p50_up: Vec<(f32, u64)>,
        p90_down: Vec<(f32, u64)>,
        p90_up: Vec<(f32, u64)>,
    }
    let mut by_start: FxHashMap<u64, Accum> = FxHashMap::default();
    for minutes in slices {
        for minute in minutes {
            let entry = by_start.entry(minute.start_millis).or_default();
            entry.sampled_intervals = entry.sampled_intervals.max(minute.sampled_intervals);
            entry.down_bytes = entry.down_bytes.saturating_add(minute.down_bytes);
            entry.up_bytes = entry.up_bytes.saturating_add(minute.up_bytes);
            // §`lqos-74t.47.5`: the four RTT fields roll up as a **traffic-weighted
            // median across members** (weight = the member's per-minute bytes:
            // `down_bytes` for the down fields, `up_bytes` for the up fields) — the
            // TYPICAL circuit, not the worst (the retired `max_optional`).
            if let Some(value) = minute.rtt_p50_down_ms {
                entry.p50_down.push((value, minute.down_bytes));
            }
            if let Some(value) = minute.rtt_p50_up_ms {
                entry.p50_up.push((value, minute.up_bytes));
            }
            if let Some(value) = minute.rtt_p90_down_ms {
                entry.p90_down.push((value, minute.down_bytes));
            }
            if let Some(value) = minute.rtt_p90_up_ms {
                entry.p90_up.push((value, minute.up_bytes));
            }
            // §lqos-74t.40 pin 3: SUM the raw counters across members so the
            // site ratio is Σretransmits/Σpackets (ratio-of-sums), never a
            // median-of-ratios-then-max.
            entry.retransmit_down_count = entry
                .retransmit_down_count
                .saturating_add(minute.retransmit_down_count);
            entry.retransmit_down_packets = entry
                .retransmit_down_packets
                .saturating_add(minute.retransmit_down_packets);
        }
    }
    let mut minutes: Vec<HeatmapMinute> = by_start
        .into_iter()
        .map(|(start_millis, acc)| {
            // The wire is direction-agnostic ⇒ both directions carry the same
            // ratio (pin 3: `retransmit_up` is now pushed, previously absent).
            let pct = executive_retransmit_percent(
                acc.retransmit_down_count,
                acc.retransmit_down_packets,
            );
            HeatmapMinute {
                start_millis,
                sampled_intervals: acc.sampled_intervals,
                down_bytes: acc.down_bytes,
                up_bytes: acc.up_bytes,
                rtt_p50_down_ms: weighted_median_f32(acc.p50_down),
                rtt_p50_up_ms: weighted_median_f32(acc.p50_up),
                rtt_p90_down_ms: weighted_median_f32(acc.p90_down),
                rtt_p90_up_ms: weighted_median_f32(acc.p90_up),
                retransmit_down_pct: pct,
                retransmit_up_pct: pct,
                retransmit_down_count: acc.retransmit_down_count,
                retransmit_down_packets: acc.retransmit_down_packets,
            }
        })
        .collect();
    minutes.sort_by_key(|minute| minute.start_millis);
    minutes
}

/// §`lqos-74t.47.5` (the `.47.1.3` convention): the **lower** traffic-weighted
/// median — sort ascending by value, accumulate weight, take the first value
/// whose cumulative weight reaches `ceil(Σ/2)`, **no interpolation**. `None` for
/// an empty set or zero total weight.
fn weighted_median_f32(mut pairs: Vec<(f32, u64)>) -> Option<f32> {
    if pairs.is_empty() {
        return None;
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: u128 = pairs.iter().map(|(_, weight)| u128::from(*weight)).sum();
    if total == 0 {
        return None;
    }
    let half = total.div_ceil(2);
    let mut cumulative: u128 = 0;
    for (value, weight) in &pairs {
        cumulative += u128::from(*weight);
        if cumulative >= half {
            return Some(*value);
        }
    }
    pairs.last().map(|(value, _)| *value)
}

/// One tick's direction-pooled RTT percentiles (ms).
#[derive(Clone, Copy, Debug, Default)]
struct TickRtt {
    p50_down_ms: Option<f32>,
    p50_up_ms: Option<f32>,
    p90_down_ms: Option<f32>,
    p90_up_ms: Option<f32>,
}

struct PerCircuit {
    counters: RawCounters,
    rtt: TickRtt,
    samples: CircuitRttSamples,
}

fn aggregate_hosts<F>(hosts: &[HostRow], resolve: F) -> FxHashMap<String, PerCircuit>
where
    F: Fn(&IpAddr) -> Option<String>,
{
    let mut by_circuit: FxHashMap<String, RawCounters> = FxHashMap::default();
    let mut rtt_down: FxHashMap<String, Vec<u32>> = FxHashMap::default();
    let mut rtt_up: FxHashMap<String, Vec<u32>> = FxHashMap::default();
    for row in hosts {
        let Some(circuit_id) = resolve(&row.id.subscriber_ip) else {
            continue;
        };
        let entry = by_circuit.entry(circuit_id.clone()).or_default();
        entry.down = entry.down.saturating_add(row.actual.download_bytes);
        entry.up = entry.up.saturating_add(row.actual.upload_bytes);
        entry.tcp_packets = entry.tcp_packets.saturating_add(row.tcp_sent_packets);
        entry.tcp_retransmits = entry.tcp_retransmits.saturating_add(row.tcp_retransmits);
        if !row.rtt_samples_down_us.is_empty() {
            rtt_down
                .entry(circuit_id.clone())
                .or_default()
                .extend_from_slice(&row.rtt_samples_down_us);
        }
        if !row.rtt_samples_up_us.is_empty() {
            rtt_up
                .entry(circuit_id)
                .or_default()
                .extend_from_slice(&row.rtt_samples_up_us);
        }
    }
    by_circuit
        .into_iter()
        .map(|(circuit_id, counters)| {
            let samples = CircuitRttSamples {
                down: rtt_down.remove(&circuit_id).unwrap_or_default(),
                up: rtt_up.remove(&circuit_id).unwrap_or_default(),
            };
            let rtt = TickRtt {
                p50_down_ms: percentile_micros_to_ms(&samples.down, 50),
                p50_up_ms: percentile_micros_to_ms(&samples.up, 50),
                p90_down_ms: percentile_micros_to_ms(&samples.down, 90),
                p90_up_ms: percentile_micros_to_ms(&samples.up, 90),
            };
            (
                circuit_id,
                PerCircuit {
                    counters,
                    rtt,
                    samples,
                },
            )
        })
        .collect()
}

/// The `percentile`-th sample (nearest-rank) in milliseconds; the bridge wire
/// is direction-agnostic, so the caller mirrors it into both directions.
///
/// §`lqos-74t.47.5`: each sample is clamped to the DRIVER cap **`1_000_000 µs`
/// (1000 ms)** *before* ranking — a stalled/retransmitted flow's ACK (bounded
/// only by the bridge's 10 s `TCP_TIMESTAMP_EXPIRY_SECS`) can no longer pin the
/// p90 at ~10 s. Clamping per sample preserves sane values **exactly** (no
/// histogram quantization, and the heatmap stays decoupled from the QoQ source).
fn percentile_micros_to_ms(samples: &[u32], percentile: usize) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    const RTT_SAMPLE_CAP_US: u32 = 1_000_000; // 1000 ms (DRIVER)
    let mut sorted: Vec<u32> = samples
        .iter()
        .map(|&us| us.min(RTT_SAMPLE_CAP_US))
        .collect();
    sorted.sort_unstable();
    let rank = (percentile.min(100) * sorted.len()).div_ceil(100).max(1) - 1;
    Some(sorted[rank.min(sorted.len() - 1)] as f32 / 1_000.0)
}

pub fn blocks_from_minutes(
    minutes: &[HeatmapMinute],
    plan_down_mbps: f32,
    plan_up_mbps: f32,
) -> HeatmapBlocks {
    let mut blocks = HeatmapBlocks {
        download: [None; RETENTION_MINUTES],
        upload: [None; RETENTION_MINUTES],
        rtt: [None; RETENTION_MINUTES],
        rtt_p50_down: [None; RETENTION_MINUTES],
        rtt_p50_up: [None; RETENTION_MINUTES],
        rtt_p90_down: [None; RETENTION_MINUTES],
        rtt_p90_up: [None; RETENTION_MINUTES],
        retransmit: [None; RETENTION_MINUTES],
        retransmit_down: [None; RETENTION_MINUTES],
        retransmit_up: [None; RETENTION_MINUTES],
    };
    let newest = RETENTION_MINUTES.saturating_sub(1);
    for (offset, minute) in minutes.iter().rev().take(RETENTION_MINUTES).enumerate() {
        let index = newest - offset;
        blocks.download[index] = utilization_percent(minute.down_bytes, minute.sampled_intervals, plan_down_mbps);
        blocks.upload[index] = utilization_percent(minute.up_bytes, minute.sampled_intervals, plan_up_mbps);
        blocks.rtt_p50_down[index] = minute.rtt_p50_down_ms;
        blocks.rtt_p50_up[index] = minute.rtt_p50_up_ms;
        blocks.rtt_p90_down[index] = minute.rtt_p90_down_ms;
        blocks.rtt_p90_up[index] = minute.rtt_p90_up_ms;
        blocks.rtt[index] = combine_optional(minute.rtt_p50_down_ms, minute.rtt_p50_up_ms);
        blocks.retransmit_down[index] = minute.retransmit_down_pct;
        blocks.retransmit_up[index] = minute.retransmit_up_pct;
        blocks.retransmit[index] =
            combine_optional(minute.retransmit_down_pct, minute.retransmit_up_pct);
    }
    blocks
}

/// The minute's average utilization percent of the circuit plan over its
/// sampled seconds (the existing `TemporalHeatmap` byte field unit).
fn utilization_percent(bytes: u64, sampled_intervals: u16, plan_mbps: f32) -> Option<f32> {
    if sampled_intervals == 0 || !plan_mbps.is_finite() || plan_mbps <= 0.0 {
        return None;
    }
    let average_bytes = bytes as f64 / sampled_intervals as f64;
    let capacity_bps = plan_mbps as f64 * 1_000_000.0;
    Some(((average_bytes * 8.0 / capacity_bps) * 100.0) as f32)
}

fn combine_optional(left: Option<f32>, right: Option<f32>) -> Option<f32> {
    match (left, right) {
        (Some(a), Some(b)) => Some((a + b) / 2.0),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn median_f32(values: &[f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let midpoint = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        Some(sorted[midpoint])
    } else {
        Some((sorted[midpoint - 1] + sorted[midpoint]) / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::{DirectionalCounters, DirectionalPackets, HostId};

    const MINUTE: u64 = 60_000;

    fn host(ip: &str, down: u64, up: u64) -> HostRow {
        HostRow {
            id: HostId {
                circuit_id: 1,
                subscriber_ip: ip.parse().expect("test IP"),
            },
            actual: DirectionalCounters {
                download_bytes: down,
                upload_bytes: up,
            },
            shaped: DirectionalCounters::default(),
            rtt_samples_us: vec![10_000],
            dns_samples_us: Vec::new(),
            tcp_sent_packets: down,
            tcp_retransmits: 0,
            last_seen_millis: 0,
            enqueued_packets: DirectionalPackets::default(),
            sent_packets: DirectionalPackets::default(),
            rtt_samples_down_us: vec![10_000],
            rtt_samples_up_us: vec![10_000],
            retransmit_times_down_ns: Vec::new(),
            retransmit_times_up_ns: Vec::new(),
            tcp_retransmits_down: 0,
            tcp_retransmits_up: 0,
            tcp_sent_packets_down: 0,
            tcp_sent_packets_up: 0,
            sent_pkts_down: 0,
            sent_pkts_up: 0,
        }
    }

    fn resolve_one(_ip: &IpAddr) -> Option<String> {
        Some("c1".to_string())
    }

    /// Oracle 1: bucket conservation is internal and truncation-independent —
    /// Σ(bucket bytes) equals Σ(folded interval deltas).
    #[test]
    fn bucket_bytes_conserve_the_folded_deltas() {
        let mut acc = CircuitHeatmapAccumulator::new();
        acc.fold_tick(0, &[host("100.97.0.1", 1_000, 100)], resolve_one);
        acc.fold_tick(1_000, &[host("100.97.0.1", 1_500, 200)], resolve_one);
        acc.fold_tick(2_000, &[host("100.97.0.1", 2_400, 500)], resolve_one);

        let minutes = acc.minute_buckets("c1");
        let down: u64 = minutes.iter().map(|minute| minute.down_bytes).sum();
        let up: u64 = minutes.iter().map(|minute| minute.up_bytes).sum();
        assert_eq!(down, (1_500 - 1_000) + (2_400 - 1_500));
        assert_eq!(up, (200 - 100) + (500 - 200));
    }

    /// Oracle 3 (bound): a circuit that never moves bytes creates no row, and
    /// an idle row is evicted only after the full retention window.
    #[test]
    fn idle_rows_persist_until_retention_then_evict() {
        let mut acc = CircuitHeatmapAccumulator::new();
        // First sight seeds; no row yet (zero delta).
        acc.fold_tick(0, &[host("100.97.0.1", 0, 0)], resolve_one);
        assert_eq!(acc.observed_circuits(), 0, "zero-byte first sight has no row");
        // A real delta creates the row.
        acc.fold_tick(1_000, &[host("100.97.0.1", 5_000, 0)], resolve_one);
        assert_eq!(acc.observed_circuits(), 1);
        // Idle within retention: still present.
        acc.fold_tick(2_000, &[host("100.97.0.1", 5_000, 0)], resolve_one);
        assert_eq!(acc.observed_circuits(), 1);
        // Idle past retention: evicted.
        let past = 1_000 + RETENTION_MILLIS + MINUTE;
        acc.fold_tick(past, &[host("100.97.0.1", 5_000, 0)], resolve_one);
        assert_eq!(acc.observed_circuits(), 0, "idle > retention evicts");
    }

    /// Oracle 4: a poll outage is NOT silently coalesced — the skipped minutes
    /// record sampled=0 while the recovery minute carries the straddling delta.
    #[test]
    fn recovery_is_visible_in_sampled_intervals() {
        let mut acc = CircuitHeatmapAccumulator::new();
        acc.fold_tick(0, &[host("100.97.0.1", 1_000, 0)], resolve_one);
        // Minute 0 fold.
        acc.fold_tick(1_000, &[host("100.97.0.1", 2_000, 0)], resolve_one);
        // A long outage: the next successful fold is two minutes later.
        acc.fold_tick(2 * MINUTE + 1_000, &[host("100.97.0.1", 6_000, 0)], resolve_one);

        let minutes = acc.minute_buckets("c1");
        let outage: Vec<_> = minutes
            .iter()
            .filter(|minute| minute.sampled_intervals == 0)
            .collect();
        assert_eq!(outage.len(), 1, "the skipped minute is a real sampled=0 bucket");
        let recovery = minutes
            .iter()
            .find(|minute| minute.start_millis == 2 * MINUTE)
            .expect("recovery minute exists");
        assert_eq!(recovery.down_bytes, 4_000, "the straddling delta is preserved");
    }

    /// Oracle 6: a counter reset mid-window leaves that interval unsampled and
    /// re-seeds — never a negative or fabricated delta.
    #[test]
    fn counter_reset_leaves_the_interval_unsampled() {
        let mut acc = CircuitHeatmapAccumulator::new();
        acc.fold_tick(0, &[host("100.97.0.1", 5_000, 0)], resolve_one);
        acc.fold_tick(1_000, &[host("100.97.0.1", 9_000, 0)], resolve_one);
        // Reset: the counter drops.
        acc.fold_tick(2_000, &[host("100.97.0.1", 100, 0)], resolve_one);
        let minutes = acc.minute_buckets("c1");
        let folded: u64 = minutes.iter().map(|minute| minute.down_bytes).sum();
        assert_eq!(folded, 4_000, "only the valid interval is folded");
    }

    /// Oracle 7: a circuit that returns after falling out of the snapshot
    /// re-seeds (its first interval is no-rate), never a fabricated baseline.
    #[test]
    fn seed_miss_return_is_a_fresh_seed() {
        let mut acc = CircuitHeatmapAccumulator::new();
        acc.fold_tick(0, &[host("100.97.0.1", 1_000, 0)], resolve_one);
        acc.fold_tick(1_000, &[host("100.97.0.1", 2_000, 0)], resolve_one);
        // Vanished: seed pruned.
        acc.fold_tick(2_000, &[], resolve_one);
        // Returns with a large cumulative total: first interval re-seeds.
        acc.fold_tick(3_000, &[host("100.97.0.1", 50_000, 0)], resolve_one);
        let minutes = acc.minute_buckets("c1");
        let folded: u64 = minutes.iter().map(|minute| minute.down_bytes).sum();
        assert_eq!(folded, 1_000, "the return interval contributes no fabricated bytes");
    }

    /// Oracle (shape): publication maps the newest minute to the last block
    /// and expresses bytes in the existing percent-utilization unit.
    #[test]
    fn blocks_publish_in_the_percent_utilization_unit() {
        let mut acc = CircuitHeatmapAccumulator::new();
        acc.fold_tick(0, &[host("100.97.0.1", 0, 0)], resolve_one);
        // 100 Mbps plan; 12_500_000 bytes over one sampled second = 100%.
        acc.fold_tick(1_000, &[host("100.97.0.1", 12_500_000, 0)], resolve_one);
        let blocks = acc.blocks_for("c1", 100.0, 100.0);
        let newest = RETENTION_MINUTES - 1;
        assert_eq!(blocks.download[newest], Some(100.0));
        assert_eq!(blocks.upload[newest], Some(0.0));
        assert_eq!(blocks.rtt_p50_down[newest], Some(10.0), "10 ms p50");
        assert!(blocks.download[..newest].iter().all(Option::is_none));
    }

    /// The restored p90: the pooled tick samples yield a per-direction p90 in
    /// ms (nearest rank), published alongside the p50.
    #[test]
    fn blocks_publish_per_direction_p90_in_millis() {
        let mut acc = CircuitHeatmapAccumulator::new();
        acc.fold_tick(0, &[host("100.97.0.1", 0, 0)], resolve_one);
        let mut varied = host("100.97.0.1", 5_000_000, 0);
        varied.rtt_samples_us = vec![10_000, 20_000, 30_000, 40_000, 50_000];
        varied.rtt_samples_down_us = vec![10_000, 20_000, 30_000, 40_000, 50_000];
        varied.rtt_samples_up_us = vec![10_000, 20_000, 30_000, 40_000, 50_000];
        acc.fold_tick(1_000, &[varied], resolve_one);
        let blocks = acc.blocks_for("c1", 100.0, 100.0);
        let newest = RETENTION_MINUTES - 1;
        // Nearest-rank p90 over five samples = the 5th (50 ms); p50 = 30 ms.
        assert_eq!(blocks.rtt_p90_down[newest], Some(50.0));
        assert_eq!(blocks.rtt_p90_up[newest], Some(50.0));
        assert_eq!(blocks.rtt_p50_down[newest], Some(30.0));
    }

    /// Oracle 2 (rollup identity, byte-exact): the site/global rollup sums the
    /// member rows' exact byte totals per aligned minute.
    #[test]
    fn rollup_sums_member_rows_byte_exactly() {
        let mut acc = CircuitHeatmapAccumulator::new();
        acc.fold_tick(0, &[host("100.97.0.1", 1_000, 0)], resolve_one);
        acc.fold_tick(1_000, &[host("100.97.0.1", 3_000, 0)], resolve_one);
        let rollup = acc.rollup_minutes(["c1"]);
        let total: u64 = rollup.iter().map(|minute| minute.down_bytes).sum();
        assert_eq!(total, 2_000);
    }

    /// Increment 2: the published snapshot carries the per-circuit minutes and
    /// rolls up byte-exactly into the consumer shape.
    #[test]
    fn published_snapshot_rolls_up_member_rows() {
        let mut acc = CircuitHeatmapAccumulator::new();
        acc.fold_tick(0, &[host("100.97.0.1", 1_000, 0)], resolve_one);
        acc.fold_tick(1_000, &[host("100.97.0.1", 3_000, 0)], resolve_one);
        let snapshot = acc.publish();
        assert_eq!(snapshot.captured_at_millis, 1_000);
        assert!(snapshot.by_circuit.contains_key("c1"));
        let rollup = snapshot.rollup(["c1"]);
        let total: u64 = rollup.iter().map(|minute| minute.down_bytes).sum();
        assert_eq!(total, 2_000);
        let blocks = blocks_from_minutes(&rollup, 100.0, 100.0);
        assert!(blocks.download[RETENTION_MINUTES - 1].is_some());
    }

    /// §lqos-74t.40 pin 3: the site retransmit is Σretransmits/Σpackets over the
    /// members — NOT the worst member's ratio (the old `max`) and NOT a median of
    /// per-tick ratios.
    #[test]
    fn site_retransmit_is_ratio_of_sums_not_max() {
        // Member A: 3/1000 (0.30%); member B: 5/1000 (0.50%). Σ = 8/2000 = 0.40%;
        // the old max statistic would report 0.50%.
        let a = HeatmapMinute {
            start_millis: 0,
            retransmit_down_count: 3,
            retransmit_down_packets: 1_000,
            ..HeatmapMinute::default()
        };
        let b = HeatmapMinute {
            start_millis: 0,
            retransmit_down_count: 5,
            retransmit_down_packets: 1_000,
            ..HeatmapMinute::default()
        };
        let sa = [a];
        let sb = [b];
        let rolled = rollup_minute_slices([&sa[..], &sb[..]].into_iter());
        let pct = rolled[0].retransmit_down_pct.expect("floor met");
        let expected = (8.0_f32 / 2_000.0) * 100.0;
        assert!(
            (pct - expected).abs() < 1e-4,
            "expected Σ/Σ {expected}, got {pct}"
        );
        assert!(pct < 0.50, "must NOT be the worst member's 0.50%");
        assert_eq!(
            rolled[0].retransmit_up_pct,
            Some(pct),
            "retransmit_up is now pushed"
        );
    }

    /// §`lqos-74t.47.5`: the per-sample clamp (`min(sample, 1000 ms)`) bounds a
    /// stalled-flow tail while leaving sane values exact. Fixture: 100×20 ms +
    /// 15×9.996 s = 115 samples (nearest-rank p90 = rank 104 ⇒ the stall today).
    #[test]
    fn rtt_tail_is_clamped_at_one_second() {
        let mut samples = vec![20_000_u32; 100];
        samples.extend(vec![9_996_000_u32; 15]);
        // p50 unchanged (20 ms); p90 clamped to 1000 ms (was 9996 ms, red).
        assert_eq!(percentile_micros_to_ms(&samples, 50), Some(20.0));
        assert_eq!(percentile_micros_to_ms(&samples, 90), Some(1000.0));
        // No-regression: all-sane samples are byte-identical.
        let sane = vec![20_000_u32; 115];
        assert_eq!(percentile_micros_to_ms(&sane, 50), Some(20.0));
        assert_eq!(percentile_micros_to_ms(&sane, 90), Some(20.0));
    }

    /// §`lqos-74t.47.5`: the site/global RTT rollup is the **traffic-weighted
    /// median across members** (the `.47.1.3` lower-median convention), NOT the
    /// retired `max_optional` (worst-of-members). Pinned `(down, up, bytes)`:
    /// `(20,30,1 MB)+(25,40,1 MB)+(9996,9996,1 KB)` ⇒ down **25 ms** / up **40 ms**;
    /// the max analogue is **9996 ms** (fails today).
    #[test]
    fn rollup_rtt_is_traffic_weighted_median_not_max() {
        let member = |down: f32, up: f32, bytes: u64| HeatmapMinute {
            start_millis: 0,
            sampled_intervals: 60,
            down_bytes: bytes,
            up_bytes: bytes,
            rtt_p50_down_ms: Some(down),
            rtt_p50_up_ms: Some(up),
            rtt_p90_down_ms: Some(down),
            rtt_p90_up_ms: Some(up),
            ..HeatmapMinute::default()
        };
        let a = [member(20.0, 30.0, 1_000_000)];
        let b = [member(25.0, 40.0, 1_000_000)];
        let c = [member(9996.0, 9996.0, 1_000)];
        // The retired `max_optional` analogue: the worst member = 9996 ms.
        let max_analogue = [a[0].clone(), b[0].clone(), c[0].clone()]
            .iter()
            .fold(0.0_f32, |acc, minute| {
                acc.max(minute.rtt_p50_down_ms.expect("member p50"))
            });
        assert_eq!(max_analogue, 9996.0, "the old rollup = the worst member");
        let rolled = rollup_minute_slices([&a[..], &b[..], &c[..]].into_iter());
        assert_eq!(rolled.len(), 1);
        assert_eq!(rolled[0].rtt_p50_down_ms, Some(25.0), "down weighted median");
        assert_eq!(rolled[0].rtt_p50_up_ms, Some(40.0), "up weighted median");
        // Byte SUM is unchanged by the rollup change.
        assert_eq!(rolled[0].down_bytes, 2_001_000);
        assert_eq!(rolled[0].up_bytes, 2_001_000);
    }
}

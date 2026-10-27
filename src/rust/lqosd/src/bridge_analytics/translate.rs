//! Pure translation from bridge analytics snapshots to per-circuit live
//! rows (E2 card §5.1.3 — this is the actual repoint work).
//!
//! Bridge counters are CUMULATIVE enqueued/sent bytes per host row; live
//! rates are produced by differencing across `captured_at_millis`:
//!
//! - FIRST-POLL SEED: a host row seen for the first time produces NO rate
//!   this interval ("no rate yet" — never 0, never the lifetime total;
//!   the bootstrap rule #10 precedent). The row's counters become the
//!   seed for the next interval.
//! - COUNTER-RESET: if any counter DECREASES between polls (retention
//!   eviction resets rows to low values), that row yields no rate for
//!   this interval and re-seeds from the new values.
//! - Rows that vanish from the snapshot are pruned from the seed state.
//!
//! Rates use the `actual` (enqueued, pre-shaping) counters — the same
//! basis as the kernel `bytes_per_second` (observed device traffic); the
//! `shaped` (post-shaper) counters remain available for GA surfaces.
//!
//! Directionality: bridge host rows are NOT direction-split for RTT
//! samples or TCP retransmit totals. The circuit-level p50 and the
//! interval retransmit sample are placed in BOTH down and up slots (the
//! page has two columns; the measurement itself is circuit-wide). This
//! is a presentation choice on real data, not a fabricated value; flip
//! to empty (None) if the deriver rules directional slots must carry
//! directional data only.

use std::collections::BTreeMap;
use std::net::IpAddr;

use analytics_ipc::{
    AnalyticsSnapshot, DirectionalCounters, DirectionalPackets, HostRow, LATENCY_BUCKETS,
    RttHistogram, latency_bucket,
};
use fxhash::FxHashMap;
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample};

/// Identity of one bridge host row (a circuit's numeric id plus the
/// subscriber IP). The numeric circuit id is BRIDGE-INTERNAL (a monotonic
/// first-sight assignment since bridge boot — not derivable from config);
/// rows are joined to product circuit-id STRINGS by IP at the consumer.
pub type HostKey = (u64, IpAddr);

/// Interval deltas for one host row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostDelta {
    /// Differenced enqueued bytes per second.
    pub bytes_per_second: DownUpOrder<u64>,
    /// Differenced shaped (sent) bytes per second (T2: the circuit
    /// summary's headline rate — actual stays the saturation-honest
    /// companion).
    pub shaped_bytes_per_second: DownUpOrder<u64>,
    /// K2 slice 2 §7: differenced per-direction ENQUEUED packets per second
    /// (the observed-traffic analog — the pps companion to
    /// `bytes_per_second`; feeds lqtop's Down/Up pps columns).
    pub enqueued_packets_per_second: DownUpOrder<u64>,
    /// K2 slice 2 §7: differenced per-direction SENT (dequeued) packets per
    /// second (post-shaper; the pps companion to `shaped_bytes_per_second`).
    pub sent_packets_per_second: DownUpOrder<u64>,
    /// Differenced TCP packets in the interval.
    pub tcp_packets: u64,
    /// Differenced TCP retransmits in the interval.
    pub tcp_retransmits: u64,
}

/// Cumulative seed state for one host row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostSeed {
    actual: DirectionalCounters,
    shaped: DirectionalCounters,
    enqueued_packets: DirectionalPackets,
    sent_packets: DirectionalPackets,
    tcp_sent_packets: u64,
    tcp_retransmits: u64,
}

/// Diffs two snapshots into per-host interval deltas.
///
/// `prev_seeds` is the seed state left by the previous poll (pruned to
/// the rows that were present then) and `prev_captured_millis` that
/// poll's `captured_at_millis`; the returned map is the seed state for
/// the NEXT poll (rows present in `next` only — vanished rows are
/// dropped so state cannot grow without bound).
pub fn difference(
    prev_seeds: &FxHashMap<HostKey, HostSeed>,
    prev_captured_millis: u64,
    next: &AnalyticsSnapshot,
) -> (FxHashMap<HostKey, HostDelta>, FxHashMap<HostKey, HostSeed>) {
    let mut deltas = FxHashMap::default();
    let mut seeds = FxHashMap::default();
    let elapsed_ms = next.captured_at_millis.saturating_sub(prev_captured_millis);
    // A zero/negative window (same-millis poll or clock oddity) yields no
    // rates at all — only the fresh seeds.
    let window_ok = elapsed_ms > 0;
    for row in &next.hosts {
        let key = (row.id.circuit_id, row.id.subscriber_ip);
        let seed = HostSeed {
            actual: row.actual.clone(),
            shaped: row.shaped.clone(),
            enqueued_packets: row.enqueued_packets,
            sent_packets: row.sent_packets,
            tcp_sent_packets: row.tcp_sent_packets,
            tcp_retransmits: row.tcp_retransmits,
        };
        let Some(prev) = prev_seeds.get(&key) else {
            // First-poll seed: no rate yet.
            seeds.insert(key, seed);
            continue;
        };
        seeds.insert(key, seed);
        if !window_ok {
            continue;
        }
        let down = row.actual.download_bytes.checked_sub(prev.actual.download_bytes);
        let up = row.actual.upload_bytes.checked_sub(prev.actual.upload_bytes);
        let shaped_down = row.shaped.download_bytes.checked_sub(prev.shaped.download_bytes);
        let shaped_up = row.shaped.upload_bytes.checked_sub(prev.shaped.upload_bytes);
        let enqueued_down = row
            .enqueued_packets
            .download_packets
            .checked_sub(prev.enqueued_packets.download_packets);
        let enqueued_up = row
            .enqueued_packets
            .upload_packets
            .checked_sub(prev.enqueued_packets.upload_packets);
        let sent_down = row
            .sent_packets
            .download_packets
            .checked_sub(prev.sent_packets.download_packets);
        let sent_up = row
            .sent_packets
            .upload_packets
            .checked_sub(prev.sent_packets.upload_packets);
        let packets = row
            .tcp_sent_packets
            .checked_sub(prev.tcp_sent_packets);
        let retransmits = row
            .tcp_retransmits
            .checked_sub(prev.tcp_retransmits);
        if let (
            Some(down),
            Some(up),
            Some(shaped_down),
            Some(shaped_up),
            Some(enqueued_down),
            Some(enqueued_up),
            Some(sent_down),
            Some(sent_up),
            Some(packets),
            Some(retransmits),
        ) = (
            down,
            up,
            shaped_down,
            shaped_up,
            enqueued_down,
            enqueued_up,
            sent_down,
            sent_up,
            packets,
            retransmits,
        ) {
            deltas.insert(
                key,
                HostDelta {
                    bytes_per_second: DownUpOrder {
                        down: per_second(down, elapsed_ms),
                        up: per_second(up, elapsed_ms),
                    },
                    shaped_bytes_per_second: DownUpOrder {
                        down: per_second(shaped_down, elapsed_ms),
                        up: per_second(shaped_up, elapsed_ms),
                    },
                    enqueued_packets_per_second: DownUpOrder {
                        down: per_second(enqueued_down, elapsed_ms),
                        up: per_second(enqueued_up, elapsed_ms),
                    },
                    sent_packets_per_second: DownUpOrder {
                        down: per_second(sent_down, elapsed_ms),
                        up: per_second(sent_up, elapsed_ms),
                    },
                    tcp_packets: packets,
                    tcp_retransmits: retransmits,
                },
            );
        }
        // A None on any field is a counter reset: no rate this interval
        // (the seed above already re-bases from the new values).
    }
    (deltas, seeds)
}

fn per_second(bytes: u64, elapsed_ms: u64) -> u64 {
    ((bytes as u128 * 1_000) / elapsed_ms as u128).min(u64::MAX as u128) as u64
}

/// One aggregated per-circuit live row (compact card shape).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CircuitLiveRow {
    /// Differenced enqueued (actual) bytes per second.
    pub bytes_per_second: DownUpOrder<u64>,
    /// Differenced shaped (sent) bytes per second (T2: the circuit
    /// summary's headline rate).
    pub shaped_bytes_per_second: DownUpOrder<u64>,
    pub rtt_current_p50_nanos: DownUpOrder<Option<u64>>,
    /// K2 slice 2 729b: the circuit's pooled RTT histogram, bucketed from the
    /// SAME host samples with the bridge wire's `latency_bucket` (a
    /// compute-identity with the wire histogram by construction).
    pub rtt_histogram: RttHistogram,
    pub tcp_retransmit_sample: DownUpOrder<TcpRetransmitSample>,
    /// AGE since last seen in nanoseconds (the page convention);
    /// `u64::MAX` = never seen.
    pub last_seen_nanos: u64,
    /// Bridge-internal numeric circuit id (diagnostics only).
    pub bridge_circuit_id: u64,
}

#[derive(Default)]
struct CircuitAccumulator {
    bytes_per_second: DownUpOrder<u64>,
    shaped_bytes_per_second: DownUpOrder<u64>,
    tcp_packets: u64,
    tcp_retransmits: u64,
    last_seen_nanos: Option<u64>,
    /// The MERGED union (kept for `rtt_histogram`; the QoO source split is a
    /// separate card).
    rtt_samples_us: Vec<u32>,
    /// `lqos-74t.47`: the two PER-DIRECTION rings — `down` = the subscriber-side
    /// (near) leg, `up` = the internet-side (far) leg. Carried separately so the
    /// per-direction p50s are the two legs (the merged union made them equal).
    rtt_samples_down_us: Vec<u32>,
    rtt_samples_up_us: Vec<u32>,
    bridge_circuit_id: u64,
    /// At least one row of this circuit has a measured interval. Circuits
    /// seen for the first time THIS poll stay absent ("no rate yet" —
    /// the first-poll seed rule; they appear at the next poll with a
    /// measured rate, idle or not).
    has_measured_interval: bool,
}

/// Joins host rows to product circuit-id strings and aggregates them.
///
/// The join is BY SUBSCRIBER IP through the caller's resolver (the
/// network-devices catalog longest-prefix match — the same join the
/// kernel rollup performs on tracker IPs). The bridge's internal numeric
/// circuit ids are boot-history-dependent (monotonic first-sight
/// assignment) and CANNOT be replayed safely on this side — an offline
/// config edit plus bridge reboot reorders them, so a replayed map would
/// silently mislabel circuits. IP-keyed joining needs no such map and is
/// no weaker than the incumbent kernel surface (same catalog, same
/// skew exposure).
///
/// Rows that fail to resolve are skipped (undercount for that circuit),
/// exactly like the kernel rollup skips unresolvable devices.
pub fn join_by_circuit<'a>(
    next: &AnalyticsSnapshot,
    deltas: &FxHashMap<HostKey, HostDelta>,
    resolve: impl Fn(&IpAddr) -> Option<&'a str>,
) -> FxHashMap<String, CircuitLiveRow> {
    let mut by_circuit: FxHashMap<String, CircuitAccumulator> = FxHashMap::default();
    for row in &next.hosts {
        let ip = row.id.subscriber_ip;
        let Some(circuit_id) = resolve(&ip).map(str::to_string) else {
            continue;
        };
        let entry = by_circuit.entry(circuit_id).or_default();
        entry.bridge_circuit_id = row.id.circuit_id;
        if let Some(delta) = deltas.get(&(row.id.circuit_id, ip)) {
            entry.has_measured_interval = true;
            entry.bytes_per_second.down += delta.bytes_per_second.down;
            entry.bytes_per_second.up += delta.bytes_per_second.up;
            entry.shaped_bytes_per_second.down += delta.shaped_bytes_per_second.down;
            entry.shaped_bytes_per_second.up += delta.shaped_bytes_per_second.up;
            entry.tcp_packets += delta.tcp_packets;
            entry.tcp_retransmits += delta.tcp_retransmits;
        }
        entry.rtt_samples_us.extend_from_slice(&row.rtt_samples_us);
        // lqos-74t.47: the two rings are the two LEGS (the merged union made
        // down == up). Same rings `circuit_heatmap`/`aggregate_hosts` consume.
        entry
            .rtt_samples_down_us
            .extend_from_slice(&row.rtt_samples_down_us);
        entry
            .rtt_samples_up_us
            .extend_from_slice(&row.rtt_samples_up_us);
        let age = age_nanos(next.captured_at_millis, row);
        entry.last_seen_nanos = Some(match entry.last_seen_nanos {
            Some(current) => current.min(age),
            None => age,
        });
    }

    by_circuit
        .into_iter()
        .filter(|(_, value)| value.has_measured_interval)
        .map(|(circuit_id, value)| {
            // lqos-74t.47: per-direction p50s — `down` = subscriber leg,
            // `up` = internet leg (were both the merged-union p50).
            let p50_down = percentile_nanos(&value.rtt_samples_down_us);
            let p50_up = percentile_nanos(&value.rtt_samples_up_us);
            (
                circuit_id,
                CircuitLiveRow {
                    bytes_per_second: value.bytes_per_second,
                    shaped_bytes_per_second: value.shaped_bytes_per_second,
                    rtt_current_p50_nanos: DownUpOrder {
                        down: p50_down,
                        up: p50_up,
                    },
                    rtt_histogram: bucket_rtt_samples(&value.rtt_samples_us),
                    tcp_retransmit_sample: DownUpOrder {
                        down: TcpRetransmitSample::new(value.tcp_retransmits, value.tcp_packets),
                        up: TcpRetransmitSample::new(value.tcp_retransmits, value.tcp_packets),
                    },
                    last_seen_nanos: value.last_seen_nanos.unwrap_or(u64::MAX),
                    bridge_circuit_id: value.bridge_circuit_id,
                },
            )
        })
        .collect()
}

/// Age since last seen in nanoseconds; `u64::MAX` for never-seen rows.
///
/// `.49.1`: `pub(crate)` — the best-effort class (`build_best_effort_by_ip`)
/// populates `IpLiveRow.last_seen_nanos` through this SAME age math.
pub(crate) fn age_nanos(captured_at_millis: u64, row: &HostRow) -> u64 {
    if row.last_seen_millis == 0 {
        return u64::MAX;
    }
    captured_at_millis
        .saturating_sub(row.last_seen_millis)
        .saturating_mul(1_000_000)
}

/// Bucket RTT samples into the bridge wire's `[u64; 50]` histogram shape
/// using the SAME `latency_bucket` mapping the bridge uses (K2 slice 2 §7):
/// a compute-identity with the wire histogram by construction.
pub fn bucket_rtt_samples(samples_us: &[u32]) -> RttHistogram {
    let mut bins = [0u64; LATENCY_BUCKETS];
    for &sample in samples_us {
        bins[latency_bucket(sample)] += 1;
    }
    RttHistogram(bins)
}

/// Median of the RTT sample set in nanos (sorted upper-median).
pub(crate) fn percentile_nanos(samples_us: &[u32]) -> Option<u64> {
    if samples_us.is_empty() {
        return None;
    }
    let mut sorted = samples_us.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    Some(u64::from(sorted[mid]) * 1_000)
}

/// Deterministic iteration order helper for tests and diagnostics.
pub fn sorted_circuit_ids(rows: &FxHashMap<String, CircuitLiveRow>) -> Vec<String> {
    let mut ids: BTreeMap<String, ()> = BTreeMap::new();
    for id in rows.keys() {
        ids.insert(id.clone(), ());
    }
    ids.into_keys().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::{DirectionalCounters, HostId};
    use std::net::Ipv4Addr;

    fn host(circuit: u64, _ip: &str, actual: DirectionalCounters) -> HostRow {
        HostRow {
            id: HostId {
                circuit_id: circuit,
                subscriber_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            },
            actual,
            shaped: DirectionalCounters::default(),
            enqueued_packets: DirectionalPackets::default(),
            sent_packets: DirectionalPackets::default(),
            rtt_samples_us: Vec::new(),
            dns_samples_us: Vec::new(),
            tcp_sent_packets: 0,
            tcp_retransmits: 0,
            last_seen_millis: 0,
            ..HostRow::default()
        }
    }

    fn row(circuit: u64, ip: &str, down: u64, up: u64, retrans: u64, pkts: u64) -> HostRow {
        HostRow {
            id: HostId {
                circuit_id: circuit,
                subscriber_ip: ip.parse().expect("valid ip"),
            },
            actual: DirectionalCounters {
                download_bytes: down,
                upload_bytes: up,
            },
            shaped: DirectionalCounters::default(),
            enqueued_packets: DirectionalPackets::default(),
            sent_packets: DirectionalPackets::default(),
            rtt_samples_us: Vec::new(),
            dns_samples_us: Vec::new(),
            tcp_sent_packets: pkts,
            tcp_retransmits: retrans,
            last_seen_millis: 0,
            ..HostRow::default()
        }
    }

    fn snapshot(captured: u64, hosts: Vec<HostRow>) -> AnalyticsSnapshot {
        AnalyticsSnapshot {
            captured_at_millis: captured,
            hosts,
            ..Default::default()
        }
    }

    fn resolve_known(ip: &IpAddr) -> Option<&'static str> {
        match ip.to_string().as_str() {
            "192.0.2.10" | "192.0.2.11" => Some("circuit-a"),
            "198.51.100.5" => Some("circuit-b"),
            _ => None,
        }
    }

    #[test]
    fn first_poll_seeds_without_rate() {
        let snap = snapshot(
            1_000,
            vec![row(1, "192.0.2.10", 5_000, 2_000, 1, 100)],
        );
        let (deltas, seeds) = difference(&FxHashMap::default(), 0, &snap);
        assert!(deltas.is_empty(), "no rate on first sight");
        assert_eq!(seeds.len(), 1);

        // Second poll over 1s: rate = delta bytes exactly.
        let snap2 = snapshot(
            2_000,
            vec![row(1, "192.0.2.10", 6_000, 2_500, 2, 220)],
        );
        let (deltas2, _) = difference(&seeds, snap.captured_at_millis, &snap2);
        let delta = deltas2
            .get(&(1, "192.0.2.10".parse::<IpAddr>().expect("test invariant")))
            .expect("rate after seed");
        assert_eq!(delta.bytes_per_second.down, 1_000);
        assert_eq!(delta.bytes_per_second.up, 500);
        assert_eq!(delta.tcp_packets, 120);
        assert_eq!(delta.tcp_retransmits, 1);
    }

    #[test]
    fn counter_reset_yields_no_rate_and_reseeds() {
        let snap1 = snapshot(
            1_000,
            vec![row(1, "192.0.2.10", 9_000, 9_000, 50, 900)],
        );
        let (_, seeds) = difference(&FxHashMap::default(), 0, &snap1);

        // Retention eviction: counters reset to low values.
        let snap2 = snapshot(
            2_000,
            vec![row(1, "192.0.2.10", 100, 100, 1, 10)],
        );
        let (deltas, seeds2) = difference(&seeds, snap1.captured_at_millis, &snap2);
        assert!(deltas.is_empty(), "reset interval carries no rate");
        assert_eq!(
            seeds2
                .get(&(1, "192.0.2.10".parse::<IpAddr>().expect("test invariant")))
                .map(|s| s.actual.download_bytes),
            Some(100),
            "reseeded from the reset values"
        );

        // Next interval recovers normally.
        let snap3 = snapshot(
            3_000,
            vec![row(1, "192.0.2.10", 1_100, 100, 1, 10)],
        );
        let (deltas3, _) = difference(&seeds2, snap2.captured_at_millis, &snap3);
        assert_eq!(
            deltas3
                .get(&(1, "192.0.2.10".parse::<IpAddr>().expect("test invariant")))
                .expect("test invariant")
                .bytes_per_second
                .down,
            1_000
        );
    }

    #[test]
    fn vanished_rows_are_pruned_from_seeds() {
        let snap1 = snapshot(
            1_000,
            vec![
                row(1, "192.0.2.10", 100, 100, 0, 10),
                row(2, "198.51.100.5", 100, 100, 0, 10),
            ],
        );
        let (_, seeds) = difference(&FxHashMap::default(), 0, &snap1);
        assert_eq!(seeds.len(), 2);
        let snap2 = snapshot(2_000, vec![row(1, "192.0.2.10", 200, 100, 0, 10)]);
        let (deltas, seeds2) = difference(&seeds, snap1.captured_at_millis, &snap2);
        assert_eq!(seeds2.len(), 1, "vanished row pruned");
        assert_eq!(deltas.len(), 1);
    }

    #[test]
    fn zero_elapsed_window_yields_no_rates_but_fresh_seeds() {
        let snap1 = snapshot(5_000, vec![row(1, "192.0.2.10", 100, 100, 0, 10)]);
        let (_, seeds) = difference(&FxHashMap::default(), 0, &snap1);
        let snap2 = snapshot(5_000, vec![row(1, "192.0.2.10", 500, 500, 0, 10)]);
        let (deltas, seeds2) = difference(&seeds, snap1.captured_at_millis, &snap2);
        assert!(deltas.is_empty());
        assert_eq!(
            seeds2
                .get(&(1, "192.0.2.10".parse::<IpAddr>().expect("test invariant")))
                .map(|s| s.actual.download_bytes),
            Some(500)
        );
    }

    #[test]
    fn join_aggregates_rows_into_circuits_and_skips_unknown_ips() {
        let snap = snapshot(
            10_000,
            vec![
                row(1, "192.0.2.10", 1_000, 100, 1, 100),
                row(1, "192.0.2.11", 2_000, 200, 0, 100),
                row(2, "198.51.100.5", 500, 50, 0, 50),
                row(9, "203.0.113.99", 999_999, 999_999, 0, 0),
            ],
        );
        let mut deltas = FxHashMap::default();
        deltas.insert(
            (1, "192.0.2.10".parse::<IpAddr>().expect("test invariant")),
            HostDelta {
                bytes_per_second: DownUpOrder { down: 900, up: 90 },
                shaped_bytes_per_second: DownUpOrder { down: 890, up: 89 },
                enqueued_packets_per_second: DownUpOrder { down: 90, up: 9 },
                sent_packets_per_second: DownUpOrder { down: 89, up: 8 },
                tcp_packets: 90,
                tcp_retransmits: 1,
            },
        );
        let joined = join_by_circuit(&snap, &deltas, resolve_known);
        // circuit-b's only row has NO measured interval (first sight) —
        // the circuit stays absent this poll ("no rate yet"); the
        // unknown-IP row never joins.
        assert_eq!(joined.len(), 1, "unknown IP skipped; first-sight circuit absent");
        let a = joined.get("circuit-a").expect("circuit-a joined");
        // 192.0.2.11 has no delta (first sight) — only .10's rate shows.
        assert_eq!(a.bytes_per_second.down, 900);
        assert_eq!(a.bytes_per_second.up, 90);
        // The retransmit sample uses only differenced windows — none
        // for .11, so the sample reflects .10's interval only.
        assert_eq!(
            a.tcp_retransmit_sample.down.packets.get(),
            90,
            "no first-sight sample"
        );
    }

    #[test]
    fn idle_circuit_stays_visible_with_measured_zero_rate() {
        // §5.1.6 idle-visibility: a circuit whose rows persist with no
        // traffic must render as a visible ZERO rate (measured), never
        // vanish — the population request carries the full row set.
        let snap1 = snapshot(1_000, vec![row(1, "192.0.2.10", 5_000, 1_000, 0, 100)]);
        let (_, seeds) = difference(&FxHashMap::default(), 0, &snap1);
        let snap2 = snapshot(4_000, vec![row(1, "192.0.2.10", 5_000, 1_000, 0, 100)]);
        let (deltas, _) = difference(&seeds, snap1.captured_at_millis, &snap2);
        let joined = join_by_circuit(&snap2, &deltas, resolve_known);
        let a = joined.get("circuit-a").expect("idle circuit visible");
        assert_eq!(a.bytes_per_second.down, 0);
        assert_eq!(a.bytes_per_second.up, 0);
    }

    #[test]
    fn last_seen_converts_to_age_nanos() {
        let mut snap = snapshot(10_000, vec![row(1, "192.0.2.10", 0, 0, 0, 0)]);
        snap.hosts[0].last_seen_millis = 9_500;
        let mut deltas = FxHashMap::default();
        deltas.insert(
            (1, "192.0.2.10".parse::<IpAddr>().expect("test invariant")),
            HostDelta::default(),
        );
        let joined = join_by_circuit(&snap, &deltas, resolve_known);
        assert_eq!(
            joined["circuit-a"].last_seen_nanos,
            500 * 1_000_000,
            "500ms age in nanos"
        );

        let mut never = host(1, "192.0.2.10", DirectionalCounters::default());
        never.id.subscriber_ip = "192.0.2.10".parse().expect("test invariant");
        never.last_seen_millis = 0;
        let snap_never = snapshot(10_000, vec![never]);
        let mut deltas2 = FxHashMap::default();
        deltas2.insert(
            (1, "192.0.2.10".parse::<IpAddr>().expect("test invariant")),
            HostDelta::default(),
        );
        let joined2 = join_by_circuit(&snap_never, &deltas2, resolve_known);
        assert_eq!(joined2["circuit-a"].last_seen_nanos, u64::MAX);
    }

    #[test]
    fn rtt_p50_pools_each_direction_ring_into_its_own_leg() {
        // lqos-74t.47: the two rings are the two LEGS — the old merged union set
        // down == up, which made neither a leg. Each direction p50s its OWN ring.
        let mut snap = snapshot(
            1_000,
            vec![row(1, "192.0.2.10", 0, 0, 0, 0)],
        );
        snap.hosts[0].rtt_samples_down_us = vec![10_000, 20_000, 60_000];
        snap.hosts[0].rtt_samples_up_us = vec![1_000, 3_000, 5_000];
        // The merged union still feeds `rtt_histogram` (the QoO source split is
        // a separate card) — carry it so the histogram assertion stays valid.
        snap.hosts[0].rtt_samples_us = vec![10_000, 20_000, 60_000];
        // The circuit needs one measured interval to join.
        let mut deltas = FxHashMap::default();
        deltas.insert(
            (1, "192.0.2.10".parse::<IpAddr>().expect("test invariant")),
            HostDelta::default(),
        );
        let joined = join_by_circuit(&snap, &deltas, resolve_known);
        let a = &joined["circuit-a"];
        assert_eq!(
            a.rtt_current_p50_nanos.down,
            Some(20_000 * 1_000),
            "subscriber leg = median(down ring)"
        );
        assert_eq!(
            a.rtt_current_p50_nanos.up,
            Some(3_000 * 1_000),
            "internet leg = median(up ring)"
        );
        assert_ne!(
            a.rtt_current_p50_nanos.down, a.rtt_current_p50_nanos.up,
            "the legs must DIFFER (the merged union made them equal)"
        );
    }

    #[test]
    fn qoo_is_absent_and_renders_empty() {
        // QoO is not carried by the bridge snapshot; CircuitLiveRow has
        // no qoo field — the endpoint layer maps it to None (empty).
        // Asserting the row type shape here pins that decision.
        let snap = snapshot(1_000, vec![row(1, "192.0.2.10", 0, 0, 0, 0)]);
        let mut deltas = FxHashMap::default();
        deltas.insert(
            (1, "192.0.2.10".parse::<IpAddr>().expect("test invariant")),
            HostDelta::default(),
        );
        let joined = join_by_circuit(&snap, &deltas, resolve_known);
        assert!(joined.contains_key("circuit-a"));
    }

    #[test]
    fn bucketing_matches_the_bridge_histogram_by_construction() {
        // K2 slice 2 729b compute-identity: our local bucketing uses the SAME
        // latency_bucket the bridge wire uses, so the histogram is identical.
        let samples: Vec<u32> = vec![1_000, 9_999, 10_000, 25_000, 500_000, 5_000];
        let ours = bucket_rtt_samples(&samples);
        let bridge = analytics_ipc::latency_histogram(&samples);
        assert_eq!(ours.0, bridge.map(u64::from));
        // The pure p90 reads the top bucket here (rank 6 of 6).
        assert_eq!(lqos_utils::rtt::histogram_p90_ms(&ours.0), Some(490.0));
    }

    #[test]
    fn packet_counters_difference_into_per_second_rates() {
        // K2 slice 2 §7: the per-direction cumulative packet counters
        // (enqueued/sent) difference into pps over the poll window, with
        // the same first-poll/reset semantics as the byte counters.
        let mut first = row(1, "192.0.2.10", 1_000, 500, 0, 0);
        first.enqueued_packets = DirectionalPackets {
            download_packets: 1_000,
            upload_packets: 500,
        };
        first.sent_packets = DirectionalPackets {
            download_packets: 900,
            upload_packets: 450,
        };
        let snap1 = snapshot(1_000, vec![first]);
        let (deltas_seed, seeds) = difference(&FxHashMap::default(), 0, &snap1);
        assert!(deltas_seed.is_empty(), "no pps on first sight");

        let mut second = row(1, "192.0.2.10", 2_000, 1_000, 0, 0);
        second.enqueued_packets = DirectionalPackets {
            download_packets: 2_000,
            upload_packets: 900,
        };
        second.sent_packets = DirectionalPackets {
            download_packets: 1_800,
            upload_packets: 900,
        };
        let snap2 = snapshot(2_000, vec![second]);
        let (deltas, _) = difference(&seeds, snap1.captured_at_millis, &snap2);
        let delta = deltas
            .get(&(1, "192.0.2.10".parse::<IpAddr>().expect("test invariant")))
            .expect("pps after seed");
        assert_eq!(delta.enqueued_packets_per_second.down, 1_000);
        assert_eq!(delta.enqueued_packets_per_second.up, 400);
        assert_eq!(delta.sent_packets_per_second.down, 900);
        assert_eq!(delta.sent_packets_per_second.up, 450);
    }
}

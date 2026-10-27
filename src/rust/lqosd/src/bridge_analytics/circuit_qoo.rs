//! 3a group-1: the bridge QoO/QoQ port — the pin; §`lqos-74t.47.1.2`
//! repointed the latency source to a per-flow, packet-weighted pool.
//!
//! The deleted kernel `update_flow_qoo` scored per-flow over the kernel RTT
//! buffer + tracker TCP counters. The bridge wire carries the raw ingredients on
//! `FlowRow` (per-direction `rtt_samples_down_us`/`up_us`, the
//! `tcp_sent_packets`/`tcp_retransmits` scalars), and `fold_tick` builds each
//! circuit's latency pool directly from those rows: per TCP flow a plain
//! per-leg median RTT weighted by its sent packets, composed into the full path
//! with the `.47.1` median-shifted union. The preserved gates:
//! 1. `MIN_RTT_SAMPLES_FOR_QOO` (on the **weighted** sample sum, inside
//!    `compute_qoq_scores`);
//! 2. the retransmit-proxy scope (TCP-only + the `tcp_sent_packets` denominator).
//!
//! The un-wired `flow_qoo_scores`/`MIN_QOO_FLOW_BYTES` gate path and the
//! unweighted ring pool are retired.

use analytics_ipc::FlowRow;
use fxhash::{FxHashMap, FxHashSet};
use lqos_utils::qoo::{compute_qoq_scores, LossMeasurement, QooProfile};
use lqos_utils::qoq_heatmap::{QoqHeatmapBlocks, TemporalQoqHeatmap};
use lqos_utils::rtt::{FlowbeeEffectiveDirection, RttBuffer, RttData};
use std::net::IpAddr;

/// §`lqos-74t.47.7` (DRIVER): a circuit must move at least this many TCP packets
/// in the collation interval (both directions summed) to be scored; below it the
/// circuit records `UNKNOWN` (never a spurious 0). Additional to the
/// `MIN_RTT_SAMPLES_FOR_QOO` sample-sum gate.
///
/// **Decay (bounded):** the heatmap greys **immediately** (`current_values`), but
/// `top_qoo`'s `latest_values` shows the latest **non-`None`** block, so a live
/// circuit that previously scored keeps its old top-QoO rank until the ~15-minute
/// ring ages out. The unit fixture asserts the `fold_tick` `None` directly.
pub const MIN_QOO_INTERVAL_PACKETS: u64 = 100;

/// The TCP retransmit loss proxy (restored verbatim from the deleted kernel
/// caller): TCP retransmits are a weak loss proxy on a transparent bridge, so
/// the confidence is capped low even at large sample sizes.
fn tcp_retransmit_loss_proxy(retransmits: u64, packets: u64) -> Option<LossMeasurement> {
    if packets == 0 {
        return None;
    }
    let retransmit_fraction = (retransmits as f64 / packets as f64).clamp(0.0, 1.0);
    const TCP_RETRANSMIT_CONFIDENCE_MAX: f64 = 0.05;
    let confidence = (packets as f64 / 10_000.0).clamp(0.0, 1.0) * TCP_RETRANSMIT_CONFIDENCE_MAX;
    Some(LossMeasurement::TcpRetransmitProxy {
        retransmit_fraction,
        confidence,
    })
}

/// The median of a set of microsecond samples (the even-count midpoint); `None`
/// for an empty set. Used for the per-flow leg estimate and the circuit-level
/// shift median (§`lqos-74t.47.1.2`).
fn leg_median_us(samples: &[u32]) -> Option<u32> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        Some(sorted[mid])
    } else {
        Some(((u64::from(sorted[mid - 1]) + u64::from(sorted[mid])) / 2) as u32)
    }
}

/// §`lqos-74t.47.1.2`: one TCP flow's per-leg median RTT estimate (µs) plus its
/// per-direction packet weights (clamped to ≥ 1 at push time).
struct FlowLegs {
    lan_med_us: Option<u32>,
    wan_med_us: Option<u32>,
    weight_down: u32,
    weight_up: u32,
}

/// The immutable published QoO view.
#[derive(Clone, Debug, Default)]
pub struct CircuitQooSnapshot {
    pub by_circuit: FxHashMap<String, QoqHeatmapBlocks>,
    /// Per-node (site) traffic-weighted-median scores, keyed by node name.
    pub by_node: FxHashMap<String, QoqHeatmapBlocks>,
    /// The whole-bridge traffic-weighted-median score.
    pub global_qoq: QoqHeatmapBlocks,
}

/// The bridge-side QoO accumulator. Each tick pools the qualifying flows' RTT
/// samples + TCP counters per circuit (the kernel's per-circuit rollup) and
/// feeds one `compute_qoq_scores` sample into the circuit's rolling
/// `TemporalQoqHeatmap`. The site/global rollups take a **traffic-weighted
/// median of the member circuits' per-direction scores** (§`lqos-74t.47.1.3`) —
/// no longer a pooled-distribution p90 (and not a plain mean of scores).
#[derive(Debug, Default)]
pub struct CircuitQooAccumulator {
    by_circuit: FxHashMap<String, TemporalQoqHeatmap>,
    by_node: FxHashMap<String, TemporalQoqHeatmap>,
    global_qoq: TemporalQoqHeatmap,
}

#[derive(Default)]
struct CircuitPool {
    rtt: RttBuffer,
    tcp_packets_down: u64,
    tcp_packets_up: u64,
    tcp_retransmits_down: u64,
    tcp_retransmits_up: u64,
}

impl CircuitQooAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observed_circuits(&self) -> usize {
        self.by_circuit.len()
    }

    /// One poll tick over the wire's flow rows. `resolve_host` joins the
    /// subscriber IP to its product circuit id (the same catalog join the live
    /// readers use); `resolve_ancestors` yields the node names (self-inclusive)
    /// the circuit rolls into; `excluded_circuits` are the ORPHAN-umbrella
    /// circuits (non-entities); `profile` is the active QoO profile, if loaded.
    ///
    /// §`lqos-74t.47.1` + §`lqos-74t.47.1.2`: the QoO latency pool is built from
    /// the **wire flow rows** (not the pre-pooled circuit rings). Per TCP flow,
    /// the **plain median** of its `rtt_samples_down_us` (LAN) / `rtt_samples_up_us`
    /// (WAN) is the per-leg estimate, and the **packet weight** is
    /// `tcp_sent_packets_down`/`_up` (clamped ≥ 1). The **full path** is composed
    /// with the `.47.1` median-shifted union over those per-flow estimates —
    /// `{ lan_med_i + median(wan_meds) } ∪ { wan_med_j + median(lan_meds) }` —
    /// carrying the packet weight, and the **same** composed distribution feeds
    /// **both** directions. A circuit whose either leg has no contributing flow
    /// scores `UNKNOWN` in both directions. The pool scores iff its **weighted
    /// sample sum ≥ `MIN_RTT_SAMPLES_FOR_QOO`** (weighted units, not flow count).
    /// The TCP loss proxy stays **per-direction** from the wire counters.
    ///
    /// **Filters:** unresolved subscribers (`resolve_host` → `None`) and ORPHAN
    /// umbrella circuits are excluded (as `snapshot_bridge_rtt` does). The retired
    /// ring-source filters are **not** reapplied: the `< 1 MB` byte gate is
    /// intentionally dropped (no gate) and the `snapshot_bridge_rtt` window
    /// `< 2 samples` gate does not apply (the pool is per-flow). One further
    /// `snapshot_bridge_rtt` filter is also bypassed: a circuit with **no
    /// resolvable parent** (which the ring fold skipped) **now scores** into the
    /// per-circuit + global pools — it contributes to no node (its
    /// `resolve_ancestors` chain is empty).
    pub fn fold_tick<F, A>(
        &mut self,
        flows: &[FlowRow],
        resolve_host: F,
        resolve_ancestors: A,
        excluded_circuits: &FxHashSet<String>,
        profile: Option<&QooProfile>,
    ) where
        F: Fn(&IpAddr) -> Option<String>,
        A: Fn(&str) -> Vec<String>,
    {
        let Some(profile) = profile else {
            return;
        };
        let mut pools: FxHashMap<String, CircuitPool> = FxHashMap::default();
        let mut legs_by_circuit: FxHashMap<String, Vec<FlowLegs>> = FxHashMap::default();
        // Per-circuit TCP loss counters + per-flow leg estimates, from the wire rows.
        for flow in flows {
            if flow.id.protocol != 6 {
                continue;
            }
            let Some(circuit_id) = resolve_host(&flow.id.subscriber_ip) else {
                continue;
            };
            if excluded_circuits.contains(&circuit_id) {
                continue;
            }
            let pool = pools.entry(circuit_id.clone()).or_default();
            pool.tcp_packets_down = pool
                .tcp_packets_down
                .saturating_add(flow.tcp_sent_packets_down);
            pool.tcp_packets_up = pool.tcp_packets_up.saturating_add(flow.tcp_sent_packets_up);
            pool.tcp_retransmits_down = pool
                .tcp_retransmits_down
                .saturating_add(flow.tcp_retransmits_down);
            pool.tcp_retransmits_up = pool
                .tcp_retransmits_up
                .saturating_add(flow.tcp_retransmits_up);
            legs_by_circuit
                .entry(circuit_id)
                .or_default()
                .push(FlowLegs {
                    lan_med_us: leg_median_us(&flow.rtt_samples_down_us),
                    wan_med_us: leg_median_us(&flow.rtt_samples_up_us),
                    weight_down: u32::try_from(flow.tcp_sent_packets_down).unwrap_or(u32::MAX),
                    weight_up: u32::try_from(flow.tcp_sent_packets_up).unwrap_or(u32::MAX),
                });
        }
        // §`lqos-74t.47.1.2` §2a R2: the shift medians are the PLAIN (unweighted)
        // median of the per-flow leg estimates over the flows carrying that leg.
        // If either leg has no contributing flow the full path is not measurable.
        for (circuit_id, legs) in &legs_by_circuit {
            let lan_meds: Vec<u32> = legs.iter().filter_map(|l| l.lan_med_us).collect();
            let wan_meds: Vec<u32> = legs.iter().filter_map(|l| l.wan_med_us).collect();
            let (Some(lan_shift), Some(wan_shift)) =
                (leg_median_us(&lan_meds), leg_median_us(&wan_meds))
            else {
                continue;
            };
            let Some(pool) = pools.get_mut(circuit_id) else {
                continue;
            };
            for l in legs {
                if let Some(lan_med) = l.lan_med_us {
                    let full_us = lan_med.saturating_add(wan_shift);
                    let reading = RttData::from_nanos(u64::from(full_us) * 1_000);
                    pool.rtt.push_weighted(
                        reading,
                        FlowbeeEffectiveDirection::Download,
                        0,
                        l.weight_down,
                    );
                    pool.rtt.push_weighted(
                        reading,
                        FlowbeeEffectiveDirection::Upload,
                        0,
                        l.weight_down,
                    );
                }
            }
            for l in legs {
                if let Some(wan_med) = l.wan_med_us {
                    let full_us = wan_med.saturating_add(lan_shift);
                    let reading = RttData::from_nanos(u64::from(full_us) * 1_000);
                    pool.rtt.push_weighted(
                        reading,
                        FlowbeeEffectiveDirection::Download,
                        0,
                        l.weight_up,
                    );
                    pool.rtt.push_weighted(
                        reading,
                        FlowbeeEffectiveDirection::Upload,
                        0,
                        l.weight_up,
                    );
                }
            }
        }

        // §`lqos-74t.47.1.3`: per-circuit scoring is UNCHANGED; the node/global rollup
        // is now a **traffic-weighted median of the scored circuit scores** (replacing
        // the pooled-distribution p90, whose union tail a single bad circuit could
        // zero). EVERY node touched by ANY circuit is registered each tick, so an
        // all-`UNKNOWN` node records `UNKNOWN` (never a stale block).
        let mut node_members: FxHashMap<String, Vec<MemberScore>> = FxHashMap::default();
        let mut global_members: Vec<MemberScore> = Vec::with_capacity(pools.len());
        for (circuit_id, pool) in pools {
            let loss_down =
                tcp_retransmit_loss_proxy(pool.tcp_retransmits_down, pool.tcp_packets_down);
            let loss_up = tcp_retransmit_loss_proxy(pool.tcp_retransmits_up, pool.tcp_packets_up);
            let scores = compute_qoq_scores(profile, &pool.rtt, loss_down, loss_up);
            let interval_packets = pool.tcp_packets_down.saturating_add(pool.tcp_packets_up);
            // §`lqos-74t.47.7`: the DRIVER-pinned circuit traffic floor — a circuit
            // must move `MIN_QOO_INTERVAL_PACKETS` in the interval (both directions
            // summed), else BOTH directions record `UNKNOWN` (grey). This is
            // ADDITIONAL to the `MIN_RTT_SAMPLES_FOR_QOO` sample-sum gate; the `None`
            // auto-excludes the circuit from the `.47.1.3` member set (`filter_map`).
            let (down, up) = if interval_packets >= MIN_QOO_INTERVAL_PACKETS {
                (scores.download_total_f32(), scores.upload_total_f32())
            } else {
                (None, None)
            };
            self.by_circuit
                .entry(circuit_id.clone())
                .or_default()
                .add_sample(down, up);
            let member = MemberScore {
                down: down.map(|v| v as u8),
                up: up.map(|v| v as u8),
                weight: interval_packets,
            };
            for ancestor in resolve_ancestors(&circuit_id) {
                node_members.entry(ancestor).or_default().push(member);
            }
            global_members.push(member);
        }
        for (node, members) in node_members {
            let (down, up) = weighted_median_pair(&members);
            self.by_node
                .entry(node)
                .or_default()
                .add_sample(down.map(f32::from), up.map(f32::from));
        }
        let (down, up) = weighted_median_pair(&global_members);
        self.global_qoq
            .add_sample(down.map(f32::from), up.map(f32::from));
    }

    pub fn publish(&self) -> CircuitQooSnapshot {
        CircuitQooSnapshot {
            by_circuit: self
                .by_circuit
                .iter()
                .map(|(circuit_id, heatmap)| (circuit_id.clone(), heatmap.blocks()))
                .collect(),
            by_node: self
                .by_node
                .iter()
                .map(|(node, heatmap)| (node.clone(), heatmap.blocks()))
                .collect(),
            global_qoq: self.global_qoq.blocks(),
        }
    }
}

/// §`lqos-74t.47.1.3`: one circuit's contribution to a node/global rollup — its
/// per-direction score (`None` = `UNKNOWN`) and its TCP packet weight.
#[derive(Clone, Copy)]
struct MemberScore {
    down: Option<u8>,
    up: Option<u8>,
    weight: u64,
}

/// §`lqos-74t.47.1.3`: the **lower** traffic-weighted median — the smallest score
/// whose cumulative weight reaches `ceil(Σ weights / 2)`, **no interpolation**.
/// `None` when no member is scored (or the total weight is 0) so the caller records
/// `UNKNOWN`, never 0.
fn weighted_median_scored(pairs: impl Iterator<Item = (u8, u64)>) -> Option<u8> {
    let mut scored: Vec<(u8, u64)> = pairs.collect();
    if scored.is_empty() {
        return None;
    }
    scored.sort_by_key(|(score, _)| *score);
    let total: u128 = scored.iter().map(|(_, weight)| u128::from(*weight)).sum();
    if total == 0 {
        return None;
    }
    let half = total.div_ceil(2);
    let mut cumulative: u128 = 0;
    for (score, weight) in scored {
        cumulative += u128::from(weight);
        if cumulative >= half {
            return Some(score);
        }
    }
    None
}

/// The per-direction weighted medians over a node/global member set. The down and
/// up scored sets may differ (a circuit can score in one direction only).
fn weighted_median_pair(members: &[MemberScore]) -> (Option<u8>, Option<u8>) {
    let down = weighted_median_scored(members.iter().filter_map(|m| m.down.map(|s| (s, m.weight))));
    let up = weighted_median_scored(members.iter().filter_map(|m| m.up.map(|s| (s, m.weight))));
    (down, up)
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::{DirectionalCounters, FlowId, FlowState};
    use lqos_utils::qoo::{LatencyNormalization, LatencyReq, LossHandling, LowHigh};
    use lqos_utils::rtt::RttBucket;

    /// §lqos-74t.40 pin 2: the `web_browsing` shape — p90 at 150 ms/50 ms.
    fn profile_p90() -> QooProfile {
        QooProfile {
            name: "p90".to_string(),
            latency: vec![LatencyReq {
                percentile: 90,
                rtt_ms: LowHigh::lower_is_better(150.0, 50.0),
            }],
            loss_fraction: LowHigh::lower_is_better(0.05, 0.01),
            loss_handling: LossHandling::Strict,
            latency_normalization: LatencyNormalization::None,
        }
    }

    fn flow(protocol: u8, down: u64, up: u64, samples: Vec<u32>) -> FlowRow {
        FlowRow {
            id: FlowId {
                circuit_id: 1,
                flow_hash: 1,
                subscriber_ip: "100.97.0.1".parse().expect("subscriber IP"),
                remote_ip: "198.51.100.1".parse().expect("remote IP"),
                subscriber_port: 1234,
                remote_port: 443,
                protocol,
            },
            actual: DirectionalCounters {
                download_bytes: down,
                upload_bytes: up,
            },
            shaped: DirectionalCounters::default(),
            rtt_samples_us: samples.clone(),
            tcp_sent_packets: 10_000,
            tcp_retransmits: 0,
            first_seen_millis: 0,
            last_seen_millis: 1_000,
            state: FlowState::Active,
            rtt_samples_down_us: samples.clone(),
            rtt_samples_up_us: samples,
            retransmit_times_down_ns: Vec::new(),
            retransmit_times_up_ns: Vec::new(),
            tcp_retransmits_down: 0,
            tcp_retransmits_up: 0,
            tcp_sent_packets_down: 10_000,
            tcp_sent_packets_up: 10_000,
            sent_pkts_down: 0,
            sent_pkts_up: 0,
        }
    }

    /// A TCP flow with per-leg RTT samples + per-direction packet weights.
    fn flow_legs(down_us: &[u32], up_us: &[u32], pkts_down: u64, pkts_up: u64) -> FlowRow {
        let mut f = flow(6, 5_000_000, 5_000_000, vec![10_000; 8]);
        f.rtt_samples_down_us = down_us.to_vec();
        f.rtt_samples_up_us = up_us.to_vec();
        f.tcp_sent_packets_down = pkts_down;
        f.tcp_sent_packets_up = pkts_up;
        f
    }

    fn resolve_one(_ip: &IpAddr) -> Option<String> {
        Some("c1".to_string())
    }

    fn none_excluded() -> FxHashSet<String> {
        FxHashSet::default()
    }

    /// The retired ring-analogue pool: concat every flow's raw leg samples, compose
    /// via the `.47.1` median-shifted union, `push` **unweighted** to both
    /// directions. Used only to show the pinned fixture fails on the old code.
    fn raw_ring_analogue(flows: &[FlowRow]) -> RttBuffer {
        let mut lan = Vec::new();
        let mut wan = Vec::new();
        for f in flows {
            lan.extend_from_slice(&f.rtt_samples_down_us);
            wan.extend_from_slice(&f.rtt_samples_up_us);
        }
        let mut raw = RttBuffer::default();
        if let (Some(lm), Some(wm)) = (leg_median_us(&lan), leg_median_us(&wan)) {
            for &l in &lan {
                let r = RttData::from_nanos(u64::from(l.saturating_add(wm)) * 1_000);
                raw.push(r, FlowbeeEffectiveDirection::Download, 0);
                raw.push(r, FlowbeeEffectiveDirection::Upload, 0);
            }
            for &w in &wan {
                let r = RttData::from_nanos(u64::from(w.saturating_add(lm)) * 1_000);
                raw.push(r, FlowbeeEffectiveDirection::Download, 0);
                raw.push(r, FlowbeeEffectiveDirection::Upload, 0);
            }
        }
        raw
    }

    /// §`lqos-74t.47.1.2` R4 (non-vacuous): the pinned 3-flow fixture. F3 is
    /// few-packet/high-RTT. The retired ring pool scores 0 (p90 bucket 180 ms); the
    /// per-flow packet-weighted pool scores 100 (p90 bucket 45 ms) — fails on today.
    #[test]
    fn pinned_fixture_packet_weighted_beats_ring_analogue() {
        let flows = vec![
            flow_legs(&[20_000; 10], &[20_000; 10], 1_000, 1_000),
            flow_legs(&[22_000; 10], &[21_000; 10], 1_000, 1_000),
            flow_legs(&[150_000; 10], &[20_000; 10], 5, 5),
        ];
        // (a) ring-analogue (unweighted): p90 bucket 180 ms ⇒ QoO 0.
        let raw = raw_ring_analogue(&flows);
        let raw_p90_ms = raw
            .percentile(RttBucket::Total, FlowbeeEffectiveDirection::Download, 90)
            .expect("raw p90")
            .as_nanos()
            / 1_000_000;
        assert_eq!(raw_p90_ms, 180, "ring-analogue p90 bucket upper bound");
        let loss = Some(LossMeasurement::Exact { loss_fraction: 0.0 });
        let raw_scores = compute_qoq_scores(&profile_p90(), &raw, loss, loss);
        assert_eq!(raw_scores.download_total, 0, "ring-analogue fails (QoO 0)");

        // (b) per-flow packet-weighted pool: p90 45 ms ⇒ QoO 100 in both directions.
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &flows,
            resolve_one,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        let snapshot = acc.publish();
        let node = snapshot.by_node.get("site1").expect("node scored");
        let values = node.current_values();
        assert_eq!(values.down, Some(100.0), "packet-weighted pool scores 100");
        assert_eq!(
            values.up,
            Some(100.0),
            "both directions score the composed path"
        );
    }

    /// §`lqos-74t.47.1`: BOTH directions score the composed FULL path (the same
    /// distribution) — circuit, node and global agree; the LATENCY is symmetric.
    #[test]
    fn both_directions_score_from_the_composed_full_path() {
        let flows = vec![flow_legs(&[10_000; 5], &[100_000; 5], 1_000, 1_000)];
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &flows,
            resolve_one,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        let snapshot = acc.publish();
        let circuit = snapshot
            .by_circuit
            .get("c1")
            .expect("circuit scored")
            .current_values()
            .down
            .expect("circuit down");
        let node = snapshot
            .by_node
            .get("site1")
            .expect("node scored")
            .current_values();
        let global = snapshot
            .global_qoq
            .current_values()
            .down
            .expect("global down");
        assert_eq!(circuit, node.down.expect("node down"));
        assert_eq!(Some(global), node.down);
        assert_eq!(
            node.down, node.up,
            "both directions read the composed full path"
        );
        assert!(
            circuit < 50.0,
            "composed full path ~110 ms scores low: {circuit}"
        );
    }

    /// §`lqos-74t.47.1` §2a R2: a circuit whose either leg has no contributing flow
    /// is not measurable ⇒ both directions UNKNOWN.
    #[test]
    fn missing_leg_scores_unknown() {
        let flows = vec![flow_legs(&[10_000; 5], &[], 1_000, 1_000)];
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &flows,
            resolve_one,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        let snapshot = acc.publish();
        let node = snapshot.by_node.get("site1").expect("node scored");
        assert!(
            node.current_values().down.is_none(),
            "no WAN leg ⇒ down UNKNOWN"
        );
        assert!(
            node.current_values().up.is_none(),
            "no WAN leg ⇒ up UNKNOWN"
        );
    }

    /// §`lqos-74t.47.1` scope guard: the direction equality is scoped to LATENCY —
    /// the TCP loss proxy stays per-direction. A 100 % down retransmit fraction caps
    /// the down total while the upload stays green.
    #[test]
    fn loss_proxy_stays_per_direction() {
        let mut f = flow_legs(&[10_000; 5], &[20_000; 5], 10_000, 10_000);
        f.tcp_retransmits_down = 10_000;
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &[f],
            resolve_one,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        let snapshot = acc.publish();
        let node = snapshot.by_node.get("site1").expect("node scored");
        let values = node.current_values();
        assert!(values.up.expect("up") >= 99.0, "upload latency green");
        assert!(
            values.down.expect("down") < values.up.expect("up"),
            "the down loss proxy still caps the down total"
        );
    }

    /// §`lqos-74t.47.1.3`: the node/global rollup is the **traffic-weighted median
    /// of the per-circuit scores** (not a pooled distribution). c1 scores lower but
    /// carries little traffic; c2 scores 100 and dominates by weight ⇒ the node reads
    /// 100. This is a sanity test of the median, **not** the old/new discriminator —
    /// with c2's weight dominant the retired union p90 would also read 100; the
    /// non-vacuity is the pinned 12-pair fixture + the cited live pooled-0.
    #[test]
    fn node_rollup_is_traffic_weighted_median_of_scores() {
        let mut f1 = flow_legs(&[10_000; 5], &[50_000; 5], 100, 100); // c1: low weight
        f1.id.subscriber_ip = "100.97.0.1".parse().expect("ip1");
        let mut f2 = flow_legs(&[10_000; 5], &[5_000; 5], 5_000, 5_000); // c2: high weight
        f2.id.subscriber_ip = "100.97.0.2".parse().expect("ip2");
        let resolve = |ip: &IpAddr| match ip.to_string().as_str() {
            "100.97.0.1" => Some("c1".to_string()),
            "100.97.0.2" => Some("c2".to_string()),
            _ => None,
        };
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &[f1, f2],
            resolve,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        let snap = acc.publish();
        let c1 = snap
            .by_circuit
            .get("c1")
            .expect("c1")
            .current_values()
            .down
            .expect("c1 down");
        let c2 = snap
            .by_circuit
            .get("c2")
            .expect("c2")
            .current_values()
            .down
            .expect("c2 down");
        let node = snap
            .by_node
            .get("site1")
            .expect("node")
            .current_values()
            .down
            .expect("node down");
        let global = snap.global_qoq.current_values().down.expect("global down");
        assert!(c1 < 90.0, "c1's high WAN leg composes lower: {c1}");
        assert_eq!(c2, 100.0, "c2's low legs compose green");
        assert_eq!(
            node, 100.0,
            "the weighted median follows c2's dominant traffic"
        );
        assert_eq!(node, global, "node and global aggregate the same circuits");
    }

    /// §`lqos-74t.47.1.3` R3 (pinned): the 12 `(score, weight)` pairs (Σ = 298 798)
    /// ⇒ the **lower** traffic-weighted median is **50** (cumulative first reaches
    /// `ceil(Σ/2) = 149 399` in the 50 bucket). The live **pooled-p90** rollup on the
    /// same representative set instead scored **0** (cited:
    /// `docs/qoq_site_global_aggregation_report.md`; a pair list carries no
    /// distribution to assert that separately). An unscored set ⇒ `None`, not 0.
    #[test]
    fn weighted_median_pinned_fixture_returns_50() {
        let pairs: [(u8, u64); 12] = [
            (0, 140_288),
            (30, 2_477),
            (50, 8_999),
            (50, 1_036),
            (60, 3_284),
            (80, 30_611),
            (80, 67_373),
            (90, 10_713),
            (100, 11_478),
            (100, 6_574),
            (100, 14_708),
            (100, 1_257),
        ];
        assert_eq!(pairs.iter().map(|(_, w)| *w).sum::<u64>(), 298_798);
        assert_eq!(
            weighted_median_scored(pairs.iter().copied()),
            Some(50),
            "first cumulative >= ceil(298798/2) lands in the 50 bucket"
        );
        assert_eq!(
            weighted_median_scored(std::iter::empty::<(u8, u64)>()),
            None
        );
    }

    /// §`lqos-74t.47.1.3` R2: every node touched by ANY circuit is registered each
    /// tick, so an all-`UNKNOWN` node records `UNKNOWN` — never 0, never a stale block.
    #[test]
    fn all_unknown_node_records_unknown() {
        let flows = vec![flow_legs(&[], &[], 1_000, 1_000)]; // no legs ⇒ no score
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &flows,
            resolve_one,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        let snap = acc.publish();
        let node = snap.by_node.get("site1").expect("node registered");
        assert!(
            node.current_values().down.is_none(),
            "all-UNKNOWN node ⇒ down UNKNOWN"
        );
        assert!(
            node.current_values().up.is_none(),
            "all-UNKNOWN node ⇒ up UNKNOWN"
        );
        assert!(
            snap.global_qoq.current_values().down.is_none(),
            "all-UNKNOWN global ⇒ UNKNOWN"
        );
    }

    /// §`lqos-74t.47.1.2` §2a R1: the pool scores iff the **weighted sample sum**
    /// is ≥ `MIN_RTT_SAMPLES_FOR_QOO` (5, weighted units) — not the flow count.
    #[test]
    fn sample_sum_gate_still_applies_above_the_floor() {
        // §`.47.7`: ≥ 100 interval packets WITH RTT samples ⇒ the composed pool
        // clears `MIN_RTT_SAMPLES_FOR_QOO` and scores.
        let with_rtt = flow_legs(&[10_000; 5], &[20_000; 5], 100, 100);
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &[with_rtt],
            resolve_one,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        assert!(
            acc.publish()
                .by_node
                .get("site1")
                .expect("node")
                .current_values()
                .down
                .is_some(),
            "floor + RTT samples ⇒ scores"
        );

        // ≥ 100 interval packets but NO RTT samples ⇒ the sample-sum gate keeps it
        // UNKNOWN (the floor alone is not sufficient — both gates must pass).
        let no_rtt = flow_legs(&[], &[], 100, 100);
        let mut acc2 = CircuitQooAccumulator::new();
        acc2.fold_tick(
            &[no_rtt],
            resolve_one,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        assert!(
            acc2.publish()
                .by_node
                .get("site1")
                .expect("node")
                .current_values()
                .down
                .is_none(),
            "floor but no RTT samples ⇒ UNKNOWN (sample gate)"
        );
    }

    /// §`lqos-74t.47.1.2` §2a R3: the ORPHAN-umbrella circuit and the unresolved
    /// subscriber are excluded (as the retired ring fold did).
    #[test]
    fn orphan_and_unresolved_circuits_are_excluded() {
        let f = flow_legs(&[10_000; 5], &[20_000; 5], 1_000, 1_000);
        let excluded: FxHashSet<String> = ["orphan".to_string()].into_iter().collect();
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &[f.clone()],
            |_| Some("orphan".to_string()),
            |_| vec!["site1".to_string()],
            &excluded,
            Some(&profile_p90()),
        );
        assert_eq!(acc.observed_circuits(), 0, "ORPHAN umbrella excluded");

        let mut acc2 = CircuitQooAccumulator::new();
        acc2.fold_tick(
            &[f],
            |_| None,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        assert_eq!(
            acc2.observed_circuits(),
            0,
            "unresolved subscriber excluded"
        );
    }

    /// A missing profile folds nothing (the kernel's `Option` short-circuit).
    #[test]
    fn absent_profile_folds_nothing() {
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &[flow_legs(&[10_000; 5], &[10_000; 5], 1_000, 1_000)],
            resolve_one,
            |_| Vec::new(),
            &none_excluded(),
            None,
        );
        assert_eq!(acc.observed_circuits(), 0);
    }

    /// A non-TCP flow never contributes a QoO score.
    #[test]
    fn non_tcp_flows_are_ignored() {
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &[flow(17, 5_000_000, 5_000_000, vec![10_000; 8])],
            resolve_one,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        assert_eq!(acc.observed_circuits(), 0);
    }

    /// §`lqos-74t.47.7`: a circuit below the DRIVER interval-packet floor
    /// (`< 100`, both directions summed) records `UNKNOWN` (None) **per direction**,
    /// even with stale RTT samples that would otherwise compose to a 0 score. The
    /// floor is ADDITIONAL to the sample-sum gate; at/above the floor scores as today.
    #[test]
    fn sub_floor_interval_traffic_scores_unknown() {
        // 40 + 40 = 80 interval packets < 100 ⇒ UNKNOWN despite stale 150 ms samples.
        let low = flow_legs(&[150_000; 8], &[150_000; 8], 40, 40);
        let mut acc = CircuitQooAccumulator::new();
        acc.fold_tick(
            &[low],
            resolve_one,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        let snap = acc.publish();
        assert!(
            snap.by_circuit
                .get("c1")
                .expect("c1 registered")
                .current_values()
                .down
                .is_none(),
            "sub-floor ⇒ circuit down UNKNOWN"
        );
        let node = snap.by_node.get("site1").expect("node registered");
        assert!(
            node.current_values().down.is_none(),
            "sub-floor ⇒ node down UNKNOWN"
        );
        assert!(
            node.current_values().up.is_none(),
            "sub-floor ⇒ node up UNKNOWN"
        );

        // 60 + 60 = 120 ≥ 100 ⇒ scores exactly as today.
        let ok = flow_legs(&[150_000; 8], &[150_000; 8], 60, 60);
        let mut acc2 = CircuitQooAccumulator::new();
        acc2.fold_tick(
            &[ok],
            resolve_one,
            |_| vec!["site1".to_string()],
            &none_excluded(),
            Some(&profile_p90()),
        );
        assert!(
            acc2.publish()
                .by_node
                .get("site1")
                .expect("node registered")
                .current_values()
                .down
                .is_some(),
            "at/above the floor ⇒ scores"
        );
    }
}

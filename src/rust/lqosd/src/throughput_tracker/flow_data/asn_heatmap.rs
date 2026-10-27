#![allow(dead_code)] // K3-B carve-out: the ASN heatmap store and reader are retained; the kernel aggregate feed is retired.
use fxhash::FxHashMap;
use lqos_utils::{
    temporal_heatmap::{HeatmapBlocks, TemporalHeatmap, executive_retransmit_percent},
    units::DownUpOrder,
};
use once_cell::sync::Lazy;
use parking_lot::Mutex;

const MAX_ASN_HEATMAPS: usize = 1000;
const EXPIRE_CYCLES: u64 = 15 * 60; // 15 minutes

#[derive(Clone, Debug, Default)]
pub struct AsnAggregate {
    pub bytes: DownUpOrder<u64>,
    pub packets: DownUpOrder<u64>,
    pub retransmits: DownUpOrder<u64>,
 /// Additive RTT WINDOW numerics: count + sum (⇒ mean
    /// [sic]) + min/max per direction, folded from each flow's current 8-slot
    /// tail every tick (no differencing; residence-weighted per the heatmap
    /// precedent).  `count == 0` = no samples; `min == 0` = no sample.  RTT is
    /// NOT under the byte conservation identity.
    pub rtt_count: DownUpOrder<u64>,
    pub rtt_sum_us: DownUpOrder<u64>,
    pub rtt_min_us: DownUpOrder<u64>,
    pub rtt_max_us: DownUpOrder<u64>,
    /// TRUE per-tick RTT percentiles (µs) computed by the fold over the current
    /// latency tails (card v3, pin A): v2's exact arithmetic — interpolated
    /// median for p50, ceil-rank for p90; `None` when the tick had no samples.
    /// The store pools these into the minute cells; the additive window
    /// numerics above stay aggregate-internal.
    pub rtt_p50_down: Option<f32>,
    pub rtt_p50_up: Option<f32>,
    pub rtt_p90_down: Option<f32>,
    pub rtt_p90_up: Option<f32>,
}

struct AsnHeatmapEntry {
    heatmap: TemporalHeatmap,
    last_updated_cycle: u64,
}

#[derive(Default)]
pub struct AsnHeatmapStore {
    entries: FxHashMap<u32, AsnHeatmapEntry>,
}

impl AsnHeatmapStore {
    pub fn new() -> Self {
        Self {
            entries: FxHashMap::default(),
        }
    }

    pub fn update(
        &mut self,
        mut aggregates: FxHashMap<u32, AsnAggregate>,
        current_cycle: u64,
        enable: bool,
    ) {
        if !enable {
            self.entries.clear();
            return;
        }

        // Drop unknown ASN 0 and purge any existing entry.
        aggregates.retain(|asn, _| *asn != 0);
        self.entries.remove(&0);

        self.retain_recent(current_cycle);

        for (asn, aggregate) in aggregates.drain() {
            // v2 parity (card v3, pin A): the fold hands TRUE per-tick p50/p90
            // computed over the current latency tails; the store pools them
            // into the minute cells.  The additive window numerics stay
            // aggregate-internal (not rendered here).
            let retransmit_down =
                executive_retransmit_percent(aggregate.retransmits.down, aggregate.packets.down);
            let retransmit_up =
                executive_retransmit_percent(aggregate.retransmits.up, aggregate.packets.up);
            let entry = self.entries.entry(asn).or_insert_with(|| AsnHeatmapEntry {
                heatmap: TemporalHeatmap::new(),
                last_updated_cycle: current_cycle,
            });
            entry.last_updated_cycle = current_cycle;
 // UNIT CONVENTION: the store's rtt slots are
            // MILLISECONDS (v2's contract — tracking_data.rs combined rtt_ms).
            // The fold's true percentiles are in MICROSECONDS (the wire unit,
            // FlowRow.rtt_samples_*_us), so convert exactly ONCE here at the
            // boundary. Omitting this renders a real 10-30 ms RTT as
            // 10,000-30,000 (the 1000× bug).
            entry.heatmap.add_sample(
                bytes_to_mbps(aggregate.bytes.down),
                bytes_to_mbps(aggregate.bytes.up),
                aggregate.rtt_p50_down.map(|v| v / 1000.0),
                aggregate.rtt_p50_up.map(|v| v / 1000.0),
                aggregate.rtt_p90_down.map(|v| v / 1000.0),
                aggregate.rtt_p90_up.map(|v| v / 1000.0),
                retransmit_down,
                retransmit_up,
            );
        }

        self.enforce_limit();
    }

    fn retain_recent(&mut self, current_cycle: u64) {
        let cutoff = current_cycle.saturating_sub(EXPIRE_CYCLES);
        self.entries
            .retain(|_, entry| entry.last_updated_cycle >= cutoff);
    }

    fn enforce_limit(&mut self) {
        if self.entries.len() <= MAX_ASN_HEATMAPS {
            return;
        }
        let mut entries: Vec<(u32, u64)> = self
            .entries
            .iter()
            .map(|(asn, entry)| (*asn, entry.last_updated_cycle))
            .collect();
        entries.sort_by_key(|(_, last)| *last);
        let remove_count = self.entries.len().saturating_sub(MAX_ASN_HEATMAPS);
        for (asn, _) in entries.into_iter().take(remove_count) {
            self.entries.remove(&asn);
        }
    }
}

pub static ASN_HEATMAPS: Lazy<Mutex<AsnHeatmapStore>> =
    Lazy::new(|| Mutex::new(AsnHeatmapStore::new()));

pub fn update_asn_heatmaps(
    aggregates: FxHashMap<u32, AsnAggregate>,
    current_cycle: u64,
    enable: bool,
) {
    let mut store = ASN_HEATMAPS.lock();
    store.update(aggregates, current_cycle, enable);
}

/// Snapshot current ASN heatmap data for bus responses.
pub fn snapshot_asn_heatmaps() -> Vec<(u32, HeatmapBlocks)> {
    let store = ASN_HEATMAPS.lock();
    store
        .entries
        .iter()
        .filter(|(asn, _)| **asn != 0)
        .map(|(asn, entry)| (*asn, entry.heatmap.blocks()))
        .collect()
}

fn bytes_to_mbps(bytes: u64) -> f32 {
    (bytes as f64 * 8.0 / 1_000_000.0) as f32
}

#[cfg(test)]
mod tests {
    use super::{AsnAggregate, AsnHeatmapStore, bytes_to_mbps};
    use fxhash::FxHashMap;
    use lqos_utils::units::DownUpOrder;

    /// The rendered-heatmap smoke (card v3 §6.6): the fold's TRUE per-tick
    /// p50/p90 render through the store into the snapshot cells — bytes to the
    /// throughput slots, the percentiles to the p50/p90 slots, and the
    /// qualified retransmit to its slot.
    #[test]
    fn asn_heatmap_renders_true_rtt_percentiles() {
        let mut store = AsnHeatmapStore::new();
        let mut aggregates = FxHashMap::default();
        aggregates.insert(
            64514,
            AsnAggregate {
                bytes: DownUpOrder {
                    down: 12_500_000,
                    up: 1_250_000,
                },
                packets: DownUpOrder { down: 100, up: 10 },
                retransmits: DownUpOrder { down: 2, up: 0 },
                rtt_count: DownUpOrder { down: 2, up: 1 },
                rtt_sum_us: DownUpOrder { down: 2200, up: 400 },
                rtt_min_us: DownUpOrder { down: 700, up: 400 },
                rtt_max_us: DownUpOrder { down: 1500, up: 400 },
                // The fold's TRUE percentiles for this tick.
                rtt_p50_down: Some(1100.0),
                rtt_p50_up: Some(400.0),
                rtt_p90_down: Some(1500.0),
                rtt_p90_up: Some(400.0),
            },
        );

        store.update(aggregates, 1, true);

        let blocks = store
            .entries
            .get(&64514)
            .expect("ASN heatmap should exist")
            .heatmap
            .blocks();
        let last = blocks.download.len() - 1;
        assert_eq!(blocks.download[last], Some(bytes_to_mbps(12_500_000)));
        assert_eq!(blocks.upload[last], Some(bytes_to_mbps(1_250_000)));
        assert_eq!(blocks.rtt_p50_down[last], Some(1.1));
        assert_eq!(blocks.rtt_p90_down[last], Some(1.5));
        assert_eq!(blocks.rtt_p50_up[last], Some(0.4));
        assert_eq!(blocks.rtt_p90_up[last], Some(0.4));
        // Qualified retransmit (2/100) renders as a percentage.
        assert_eq!(blocks.retransmit_down[last], Some(2.0));
    }

    /// Retention regression (card v3 §6.5 — the v1 units bug): the store's
    /// cycle domain is SECONDS, so a 120 s gap must RETAIN the entry (the v1
    /// bug stamped millis against `EXPIRE_CYCLES = 900` and purged every
    /// update), while an entry older than 15 min is purged.
    #[test]
    fn asn_heatmap_retains_recent_cycles_and_purges_stale() {
        let mut store = AsnHeatmapStore::new();
        let one = |bytes: u64| {
            let mut m: FxHashMap<u32, AsnAggregate> = FxHashMap::default();
            m.insert(
                64515,
                AsnAggregate {
                    bytes: DownUpOrder { down: bytes, up: 0 },
                    ..Default::default()
                },
            );
            m
        };
        store.update(one(1_000_000), 1_000, true);
        store.update(one(2_000_000), 1_120, true);
        assert!(
            store.entries.contains_key(&64515),
            "a 120 s gap must retain the entry (the v1 bug purged every update)"
        );
        // No update for the ASN for >900 s: the next tick's retain purges it.
        store.update(FxHashMap::default(), 2_100, true);
        assert!(
            !store.entries.contains_key(&64515),
            "an entry older than EXPIRE_CYCLES purges"
        );
    }

    #[test]
    fn asn_heatmaps_suppress_low_confidence_retransmits() {
        let mut store = AsnHeatmapStore::new();
        let mut aggregates = FxHashMap::default();
        aggregates.insert(
            64512,
            AsnAggregate {
                bytes: DownUpOrder {
                    down: 12_500_000,
                    up: 0,
                },
                packets: DownUpOrder { down: 99, up: 0 },
                retransmits: DownUpOrder { down: 1, up: 0 },
                rtt_count: DownUpOrder { down: 2, up: 0 },
                rtt_sum_us: DownUpOrder { down: 22, up: 0 },
                rtt_min_us: DownUpOrder { down: 10, up: 0 },
                rtt_max_us: DownUpOrder { down: 12, up: 0 },
                rtt_p50_down: None,
                rtt_p50_up: None,
                rtt_p90_down: None,
                rtt_p90_up: None,
            },
        );

        store.update(aggregates, 1, true);

        let blocks = store
            .entries
            .get(&64512)
            .expect("ASN heatmap should exist")
            .heatmap
            .blocks();
        assert_eq!(blocks.retransmit_down.last().copied().flatten(), None);
    }

    #[test]
    fn asn_heatmaps_keep_qualified_retransmits() {
        let mut store = AsnHeatmapStore::new();
        let mut aggregates = FxHashMap::default();
        aggregates.insert(
            64513,
            AsnAggregate {
                bytes: DownUpOrder {
                    down: 12_500_000,
                    up: 0,
                },
                packets: DownUpOrder { down: 100, up: 0 },
                retransmits: DownUpOrder { down: 2, up: 0 },
                rtt_count: DownUpOrder { down: 2, up: 0 },
                rtt_sum_us: DownUpOrder { down: 22, up: 0 },
                rtt_min_us: DownUpOrder { down: 10, up: 0 },
                rtt_max_us: DownUpOrder { down: 12, up: 0 },
                rtt_p50_down: None,
                rtt_p50_up: None,
                rtt_p90_down: None,
                rtt_p90_up: None,
            },
        );

        store.update(aggregates, 1, true);

        let blocks = store
            .entries
            .get(&64513)
            .expect("ASN heatmap should exist")
            .heatmap
            .blocks();
        assert_eq!(blocks.retransmit_down.last().copied().flatten(), Some(2.0));
    }
}

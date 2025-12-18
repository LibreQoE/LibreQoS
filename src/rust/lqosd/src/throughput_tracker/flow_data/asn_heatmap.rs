use fxhash::FxHashMap;
use lqos_utils::{
    temporal_heatmap::{HeatmapBlocks, TemporalHeatmap},
    units::DownUpOrder,
};
use once_cell::sync::Lazy;
use parking_lot::Mutex;

const MAX_ASN_HEATMAPS: usize = 1000;
const EXPIRE_CYCLES: u64 = 15 * 60; // 15 minutes

#[derive(Default)]
pub struct AsnAggregate {
    pub bytes: DownUpOrder<u64>,
    pub packets: DownUpOrder<u64>,
    pub retransmits: DownUpOrder<u64>,
    pub rtts: Vec<f32>,
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

        self.retain_recent(current_cycle);

        for (asn, mut aggregate) in aggregates.drain() {
            let rtt = median(&mut aggregate.rtts);
            let retransmit_down =
                retransmit_percent(aggregate.retransmits.down, aggregate.packets.down);
            let retransmit_up =
                retransmit_percent(aggregate.retransmits.up, aggregate.packets.up);
            let entry = self
                .entries
                .entry(asn)
                .or_insert_with(|| AsnHeatmapEntry {
                    heatmap: TemporalHeatmap::new(),
                    last_updated_cycle: current_cycle,
                });
            entry.last_updated_cycle = current_cycle;
            entry.heatmap.add_sample(
                bytes_to_mbps(aggregate.bytes.down),
                bytes_to_mbps(aggregate.bytes.up),
                rtt,
                rtt,
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
        .map(|(asn, entry)| (*asn, entry.heatmap.blocks()))
        .collect()
}

fn bytes_to_mbps(bytes: u64) -> f32 {
    (bytes as f64 * 8.0 / 1_000_000.0) as f32
}

fn retransmit_percent(retransmits: u64, packets: u64) -> Option<f32> {
    if retransmits == 0 || packets == 0 {
        return None;
    }
    Some((retransmits as f32 / packets as f32) * 100.0)
}

fn median(values: &mut Vec<f32>) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[mid])
    } else {
        Some((values[mid - 1] + values[mid]) / 2.0)
    }
}

// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Type used for *displaying* the queue store data. It deliberately
/// doesn't include data that we aren't going to display in a GUI.
#[allow(missing_docs)]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, Allocative)]
pub struct QueueStoreTransit {
    pub history: Vec<(CakeDiffTransit, CakeDiffTransit)>,
    pub history_head: usize,
    //pub prev_download: Option<CakeTransit>,
    //pub prev_upload: Option<CakeTransit>,
    pub current_download: CakeTransit,
    pub current_upload: CakeTransit,
    /// Queue kind for downlink (e.g., "cake" or "fq_codel")
    pub kind_down: String,
    /// Queue kind for uplink (e.g., "cake" or "fq_codel")
    pub kind_up: String,
 /// Bridge mode (K2 slice 2 §7 /a): the bridge wire carries drops
    /// and marks per DIRECTION, not per tin. When true the marks/drops graphs
    /// render one down + one up aggregate series from
    /// [`CakeDiffTransit::drops`] / [`CakeDiffTransit::marks`] with the
    /// honest "per-direction" label, instead of the per-tin fields. Kernel
    /// payloads leave this false (per-tin data available).
    #[serde(default)]
    pub per_direction: bool,
    /// `.115` (`docs/74t115_design.md`): the bridge-analytics sample this
    /// response represents. `None` = no bridge sample (kernel/observe/old
    /// bridge/unavailable): clients must render unavailable, never synthesize.
    #[serde(default)]
    pub snapshot: Option<QueueSampleSnapshot>,
    /// `.115`: entries in `history` (0 on the snapshot path).
    #[serde(default)]
    pub history_len: usize,
    /// `.115`: no watch lease on the snapshot path (deferred v2 adapter).
    #[serde(default)]
    pub watch_expires_at_unix_ms: Option<u64>,
}

/// `.115`: one bridge collector sample of a circuit's qdisc state.
///
/// Cumulative counters vs point-in-time gauges are explicit; availability is
/// explicit (`counters_available`/`tins_available`) so a non-CAKE direction is
/// never presented as a zero rate. `sample_age_ms`/`cache_age_ms` are checked
/// (None on a backward clock step); `elapsed_ms` is the actor's MONOTONIC
/// fold-to-fold interval, not the client's poll window.
#[allow(missing_docs)]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, Allocative)]
pub struct QueueSampleSnapshot {
    pub sampled_at_unix_ms: u64,
    pub now_unix_ms: u64,
    pub sample_age_ms: Option<u64>,
    pub cache_age_ms: Option<u64>,
    pub clock_step: bool,
    pub invalid_window: bool,
    pub elapsed_ms: Option<u64>,
    pub cadence_ms: u64,
    pub stale_after_ms: u64,
    pub sample_epoch_unix_ms: u64,
    pub sample_seq: u64,
    pub actor_gap: bool,
    pub stale: bool,
    pub counter_reset: bool,
    pub download: QueueSampleDirection,
    pub upload: QueueSampleDirection,
}

/// `.115`: one direction's sample. `sent_bytes`/`drop_pkts`/`aqm_marks` are
/// cumulative and `None` when `counters_available=false` (non-CAKE/unavailable);
/// `backlog_bytes`/`queue_depth_pkts`/`memory_used` are gauges.
#[allow(missing_docs)]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, Allocative)]
pub struct QueueSampleDirection {
    pub kind: String,
    pub tins_available: bool,
    pub counters_available: bool,
    pub sent_bytes: Option<u64>,
    pub drop_pkts: Option<u64>,
    pub aqm_marks: Option<u64>,
    pub backlog_bytes: u64,
    pub queue_depth_pkts: u64,
    pub memory_used: u64,
    pub tins: Vec<QueueSampleTin>,
}

/// `.115`: one CAKE tin's cumulative sent bytes + gauges (only present when
/// `tins_available`).
#[allow(missing_docs)]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, Allocative)]
pub struct QueueSampleTin {
    pub sent_bytes: u64,
    pub backlog_bytes: u64,
    pub sojourn_p99_ns: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, Allocative)]
#[allow(missing_docs)]
pub struct CakeDiffTransit {
    pub bytes: u64,
    pub packets: u64,
    pub qlen: u64,
    pub tins: Vec<CakeDiffTinTransit>,
    /// Bridge mode: direction-aggregate drop count for this interval (the
    /// bridge wire has no per-tin drops). Kernel payloads leave it 0.
    #[serde(default)]
    pub drops: u64,
    /// Bridge mode: direction-aggregate ECN/aqm mark count for this interval.
    #[serde(default)]
    pub marks: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, Allocative)]
#[allow(missing_docs)]
pub struct CakeDiffTinTransit {
    pub sent_bytes: u64,
    pub backlog_bytes: u64,
    pub drops: u64,
    pub marks: u64,
    pub base_delay_us: u64,
    pub sent_packets: Option<u64>,
    pub peak_delay_us: Option<u64>,
    pub avg_delay_us: Option<u64>,
    pub way_indirect_hits: Option<u64>,
    pub way_misses: Option<u64>,
    pub way_collisions: Option<u64>,
    pub sparse_flows: Option<u64>,
    pub bulk_flows: Option<u64>,
    pub unresponsive_flows: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, Allocative)]
#[allow(missing_docs)]
pub struct CakeTransit {
    //pub handle: TcHandle,
    //pub parent: TcHandle,
    //pub bytes: u64,
    //pub packets: u64,
    //pub overlimits: u64,
    //pub requeues: u64,
    //pub backlog: u64,
    //pub qlen: u64,
    pub memory_used: u64,
    //pub memory_limit: u64,
    //pub capacity_estimate: u64,
    //pub min_network_size: u64,
    //pub max_network_size: u64,
    //pub min_adj_size: u64,
    //pub max_adj_size: u64,
    //pub avg_hdr_offset: u64,
    //pub tins: Vec<CakeTinTransit>,
    //pub drops: u64,
}

/*
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[allow(missing_docs)]
pub struct CakeOptionsTransit {
    pub rtt: u64,
    pub bandwidth: u8,
    pub diffserv: u8,
    pub flowmode: u8,
    pub ack_filter: u8,
    pub nat: bool,
    pub wash: bool,
    pub ingress: bool,
    pub split_gso: bool,
    pub raw: bool,
    pub overhead: u16,
    pub fwmark: TcHandle,
}


// Commented out data is collected but not used
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[allow(missing_docs)]
pub struct CakeTinTransit {
    //pub threshold_rate: u64,
    //pub sent_bytes: u64,
    //pub backlog_bytes: u64,
    //pub target_us: u64,
    //pub interval_us: u64,
    //pub peak_delay_us: u64,
    //pub avg_delay_us: u64,
    //pub base_delay_us: u64,
    //pub sent_packets: u64,
    //pub way_indirect_hits: u64,
    //pub way_misses: u64,
    //pub way_collisions: u64,
    //pub drops: u64,
    //pub ecn_marks: u64,
    //pub ack_drops: u64,
    //pub sparse_flows: u64,
    //pub bulk_flows: u64,
    //pub unresponsive_flows: u64,
    //pub max_pkt_len: u64,
    //pub flow_quantum: u64,
}
*/

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct LegacyCakeDiffTinTransit {
        sent_bytes: u64,
        backlog_bytes: u32,
        drops: u32,
        marks: u32,
        base_delay_us: u32,
    }

    #[test]
    fn cake_diff_tin_transit_accepts_legacy_payloads() {
        let legacy = LegacyCakeDiffTinTransit {
            sent_bytes: 100,
            backlog_bytes: 10,
            drops: 2,
            marks: 3,
            base_delay_us: 42,
        };
        let bytes = serde_cbor::to_vec(&legacy).expect("legacy tin should serialize");
        let decoded: CakeDiffTinTransit =
            serde_cbor::from_slice(&bytes).expect("legacy tin should deserialize");

        assert_eq!(decoded.sent_bytes, 100);
        assert_eq!(decoded.backlog_bytes, 10);
        assert_eq!(decoded.drops, 2);
        assert_eq!(decoded.marks, 3);
        assert_eq!(decoded.base_delay_us, 42);
        assert_eq!(decoded.sent_packets, None);
        assert_eq!(decoded.peak_delay_us, None);
        assert_eq!(decoded.avg_delay_us, None);
        assert_eq!(decoded.way_indirect_hits, None);
        assert_eq!(decoded.way_misses, None);
        assert_eq!(decoded.way_collisions, None);
        assert_eq!(decoded.sparse_flows, None);
        assert_eq!(decoded.bulk_flows, None);
        assert_eq!(decoded.unresponsive_flows, None);
    }

    #[test]
    fn cake_diff_tin_transit_round_trips_extended_payloads() {
        let tin = CakeDiffTinTransit {
            sent_bytes: 100,
            backlog_bytes: 10,
            drops: 2,
            marks: 3,
            base_delay_us: 42,
            sent_packets: Some(5_000_000_000),
            peak_delay_us: Some(101),
            avg_delay_us: Some(51),
            way_indirect_hits: Some(70_000),
            way_misses: Some(80_000),
            way_collisions: Some(90_000),
            sparse_flows: Some(11),
            bulk_flows: Some(12),
            unresponsive_flows: Some(13),
        };

        let bytes = serde_cbor::to_vec(&tin).expect("extended tin should serialize");
        let decoded: CakeDiffTinTransit =
            serde_cbor::from_slice(&bytes).expect("extended tin should deserialize");

        assert_eq!(decoded, tin);
    }
}

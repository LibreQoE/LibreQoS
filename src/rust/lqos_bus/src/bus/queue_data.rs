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
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, Allocative)]
#[allow(missing_docs)]
pub struct CakeDiffTransit {
    pub bytes: u64,
    pub packets: u64,
    pub qlen: u64,
    pub tins: Vec<CakeDiffTinTransit>,
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

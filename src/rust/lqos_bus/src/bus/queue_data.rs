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

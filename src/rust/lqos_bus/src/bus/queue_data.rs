use serde::{Deserialize, Serialize};

/// Type used for *displaying* the queue store data. It deliberately
/// doesn't include data that we aren't going to display in a GUI.
#[allow(missing_docs)]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct QueueStoreTransit {
    pub history: Vec<(CakeDiffTransit, CakeDiffTransit)>,
    pub history_head: usize,
    //pub prev_download: Option<CakeTransit>,
    //pub prev_upload: Option<CakeTransit>,
    pub current_download: CakeTransit,
    pub current_upload: CakeTransit,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[allow(missing_docs)]
pub struct CakeDiffTransit {
    pub bytes: u64,
    pub packets: u32,
    pub qlen: u32,
    pub tins: Vec<CakeDiffTinTransit>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[allow(missing_docs)]
pub struct CakeDiffTinTransit {
    pub sent_bytes: u64,
    pub backlog_bytes: u32,
    pub drops: u32,
    pub marks: u32,
    pub base_delay_us: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[allow(missing_docs)]
pub struct CakeTransit {
    //pub handle: TcHandle,
    //pub parent: TcHandle,
    //pub bytes: u64,
    //pub packets: u32,
    //pub overlimits: u32,
    //pub requeues: u32,
    //pub backlog: u32,
    //pub qlen: u32,
    pub memory_used: u32,
    //pub memory_limit: u32,
    //pub capacity_estimate: u32,
    //pub min_network_size: u16,
    //pub max_network_size: u16,
    //pub min_adj_size: u16,
    //pub max_adj_size: u16,
    //pub avg_hdr_offset: u16,
    //pub tins: Vec<CakeTinTransit>,
    //pub drops: u32,
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
    //pub backlog_bytes: u32,
    //pub target_us: u32,
    //pub interval_us: u32,
    //pub peak_delay_us: u32,
    //pub avg_delay_us: u32,
    //pub base_delay_us: u32,
    //pub sent_packets: u32,
    //pub way_indirect_hits: u16,
    //pub way_misses: u16,
    //pub way_collisions: u16,
    //pub drops: u32,
    //pub ecn_marks: u32,
    //pub ack_drops: u32,
    //pub sparse_flows: u16,
    //pub bulk_flows: u16,
    //pub unresponsive_flows: u16,
    //pub max_pkt_len: u16,
    //pub flow_quantum: u16,
}
*/

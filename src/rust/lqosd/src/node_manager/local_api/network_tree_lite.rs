use crate::shaped_devices_tracker::full_network_map_lite_snapshot;
use serde::{Deserialize, Serialize};

/// Minimal live tree payload for pages that do not need the full `NetworkJsonTransport`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkTreeLiteNode {
    /// Display name for the node.
    pub name: String,
    /// Optional stable node identifier from `network.json` metadata.
    #[serde(default)]
    pub id: Option<String>,
    /// True if the node is virtual/logical-only.
    #[serde(rename = "virtual", default)]
    pub is_virtual: bool,
    /// True if TreeGuard has runtime-virtualized this node in Bakery.
    #[serde(default)]
    pub runtime_virtualized: bool,
    /// Configured maximum throughput in Mbps.
    pub max_throughput: (f64, f64),
    /// Current throughput in bytes per second.
    pub current_throughput: (u64, u64),
    /// Current TCP packets.
    pub current_tcp_packets: (u64, u64),
    /// Current TCP retransmits.
    pub current_retransmits: (u64, u64),
    /// Approximate current RTT medians for down/up, in milliseconds.
    pub rtts: Vec<f32>,
    /// QoO score for download/upload directions.
    #[serde(default)]
    pub qoo: (Option<f32>, Option<f32>),
    /// Parent node indexes.
    pub parents: Vec<usize>,
    /// Immediate parent node index.
    pub immediate_parent: Option<usize>,
    /// Optional node type metadata from `network.json`.
    #[serde(rename = "type")]
    pub node_type: Option<String>,
    /// Optional geographic latitude.
    #[serde(default)]
    pub latitude: Option<f32>,
    /// Optional geographic longitude.
    #[serde(default)]
    pub longitude: Option<f32>,
}

/// Returns the current lightweight network tree snapshot for websocket/API consumers.
pub fn network_tree_lite_data() -> Vec<(usize, NetworkTreeLiteNode)> {
    full_network_map_lite_snapshot()
}

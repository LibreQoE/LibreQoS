use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// A "transport-friendly" version of `NetworkJsonNode`. Designed
/// to be quickly cloned from original nodes and efficiently
/// transmitted/received.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct NetworkJsonTransport {
    /// Display name
    pub name: String,
    /// Optional stable node identifier carried in network.json metadata.
    #[serde(default)]
    pub id: Option<String>,
    /// True if this node is a "virtual" (logical-only) node.
    #[serde(rename = "virtual", default)]
    pub is_virtual: bool,
    /// Max throughput for node (not clamped)
    pub max_throughput: (f64, f64),
    /// Configured max throughput from `network.json`.
    #[serde(default)]
    pub configured_max_throughput: (f64, f64),
    /// Effective max throughput after parent inheritance, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_max_throughput: Option<(f64, f64)>,
    /// Current node throughput
    pub current_throughput: (u64, u64),
    /// Current node packets
    pub current_packets: (u64, u64),
    /// Current TCP packets
    pub current_tcp_packets: (u64, u64),
    /// Current UDP packets
    pub current_udp_packets: (u64, u64),
    /// Current ICMP packets
    pub current_icmp_packets: (u64, u64),
    /// Current count of TCP retransmits
    pub current_retransmits: (u64, u64),
    /// Cake marks
    pub current_marks: (u64, u64),
    /// Cake drops
    pub current_drops: (u64, u64),
    /// Set of RTT data
    pub rtts: Vec<f32>,
    /// QoO (Quality of Outcome) score for download/upload directions (0..100).
    ///
    /// `None` means "insufficient data".
    #[serde(default)]
    pub qoo: (Option<f32>, Option<f32>),
    /// Node indices of parents
    pub parents: Vec<usize>,
    /// The immediate parent node in the tree
    pub immediate_parent: Option<usize>,
    /// The type of node (site, ap, etc.)
    #[serde(rename = "type")]
    pub node_type: Option<String>,
    /// Optional node latitude from network.json metadata.
    #[serde(default)]
    pub latitude: Option<f32>,
    /// Optional node longitude from network.json metadata.
    #[serde(default)]
    pub longitude: Option<f32>,
    /// Total number of descendant site-tree nodes below this node.
    ///
    /// This excludes the node itself. For the synthetic root node, this is the
    /// total number of nodes in the loaded site tree.
    #[serde(default)]
    pub subtree_site_count: u32,
    /// Total number of unique circuits attached to this node or any descendant node.
    #[serde(default)]
    pub subtree_circuit_count: u32,
    /// Total number of shaped devices attached to this node or any descendant node.
    #[serde(default)]
    pub subtree_device_count: u32,
}

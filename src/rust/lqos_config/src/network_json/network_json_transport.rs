use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// A "transport-friendly" version of `NetworkJsonNode`. Designed
/// to be quickly cloned from original nodes and efficiently
/// transmitted/received.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct NetworkJsonTransport {
    /// Display name
    pub name: String,
    /// True if this node is a "virtual" (logical-only) node.
    #[serde(rename = "virtual", default)]
    pub is_virtual: bool,
    /// Max throughput for node (not clamped)
    pub max_throughput: (u32, u32),
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
}

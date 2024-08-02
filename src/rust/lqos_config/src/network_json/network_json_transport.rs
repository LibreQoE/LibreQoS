use serde::{Deserialize, Serialize};

/// A "transport-friendly" version of `NetworkJsonNode`. Designed
/// to be quickly cloned from original nodes and efficiently
/// transmitted/received.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkJsonTransport {
    /// Display name
    pub name: String,
    /// Max throughput for node (not clamped)
    pub max_throughput: (u32, u32),
    /// Current node throughput
    pub current_throughput: (u64, u64),
    /// Current count of TCP retransmits
    pub current_retransmits: (u64, u64),
    /// Cake marks
    pub current_marks: (u64, u64),
    /// Cake drops
    pub current_drops: (u64, u64),
    /// Set of RTT data
    pub rtts: Vec<f32>,
    /// Node indices of parents
    pub parents: Vec<usize>,
    /// The immediate parent node in the tree
    pub immediate_parent: Option<usize>,
    /// The type of node (site, ap, etc.)
    #[serde(rename = "type")]
    pub node_type: Option<String>,
}
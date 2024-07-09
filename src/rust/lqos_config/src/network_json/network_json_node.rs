use dashmap::DashSet;
use lqos_utils::units::{AtomicDownUp, DownUpOrder};
use crate::NetworkJsonTransport;

/// Describes a node in the network map tree.
#[derive(Debug, Clone)]
pub struct NetworkJsonNode {
    /// The node name, as it appears in `network.json`
    pub name: String,

    /// The maximum throughput allowed per `network.json` for this node
    pub max_throughput: (u32, u32), // In mbps

    /// Current throughput (in bytes/second) at this node
    pub current_throughput: DownUpOrder<u64>, // In bytes

    /// Current TCP Retransmits
    pub current_tcp_retransmits: DownUpOrder<u64>, // In retries

    /// Current Cake Marks
    pub current_marks: DownUpOrder<u64>,

    /// Current Cake Drops
    pub current_drops: DownUpOrder<u64>,

    /// Approximate RTTs reported for this level of the tree.
    /// It's never going to be as statistically accurate as the actual
    /// numbers, being based on medians.
    pub rtts: DashSet<u16>,

    /// A list of indices in the `NetworkJson` vector of nodes
    /// linking to parent nodes
    pub parents: Vec<usize>,

    /// The immediate parent node
    pub immediate_parent: Option<usize>,

    /// The node type
    pub node_type: Option<String>,
}

impl NetworkJsonNode {
    /// Make a deep copy of a `NetworkJsonNode`, converting atomics
    /// into concrete values.
    pub fn clone_to_transit(&self) -> NetworkJsonTransport {
        NetworkJsonTransport {
            name: self.name.clone(),
            max_throughput: self.max_throughput,
            current_throughput: (
                self.current_throughput.get_down(),
                self.current_throughput.get_up(),
            ),
            current_retransmits: (
                self.current_tcp_retransmits.get_down(),
                self.current_tcp_retransmits.get_up(),
            ),
            current_marks: (
                self.current_marks.get_down(),
                self.current_marks.get_up(),
            ),
            current_drops: (
                self.current_drops.get_down(),
                self.current_drops.get_up(),
            ),
            rtts: self.rtts.iter().map(|n| *n as f32 / 100.0).collect(),
            parents: self.parents.clone(),
            immediate_parent: self.immediate_parent,
            node_type: self.node_type.clone(),
        }
    }
}
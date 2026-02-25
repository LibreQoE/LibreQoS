use crate::NetworkJsonTransport;
use allocative_derive::Allocative;
use lqos_utils::{
    qoq_heatmap::TemporalQoqHeatmap,
    rtt::{FlowbeeEffectiveDirection, RttBucket, RttBuffer},
    temporal_heatmap::TemporalHeatmap,
    units::DownUpOrder,
};

/// Describes a node in the network map tree.
#[derive(Debug, Clone, Allocative)]
pub struct NetworkJsonNode {
    /// The node name, as it appears in `network.json`
    pub name: String,

    /// If true, this node is "virtual" (logical only): it exists for monitoring/aggregation
    /// but should be omitted from the physical HTB tree.
    pub virtual_node: bool,

    /// The maximum throughput allowed per `network.json` for this node
    pub max_throughput: (f64, f64), // In mbps

    /// Current throughput (in bytes/second) at this node
    pub current_throughput: DownUpOrder<u64>, // In bytes

    /// Current Packets
    pub current_packets: DownUpOrder<u64>,

    /// Current TCP Packets
    pub current_tcp_packets: DownUpOrder<u64>,

    /// Current UDP Packets
    pub current_udp_packets: DownUpOrder<u64>,

    /// Current ICMP Packets
    pub current_icmp_packets: DownUpOrder<u64>,

    /// Current TCP Retransmits
    pub current_tcp_retransmits: DownUpOrder<u64>, // In retries

    /// Current Cake Marks
    pub current_marks: DownUpOrder<u64>,

    /// Current Cake Drops
    pub current_drops: DownUpOrder<u64>,

    /// Approximate RTTs reported for this level of the tree.
    /// It's never going to be as statistically accurate as the actual
    /// numbers, being based on medians.
    pub rtt_buffer: RttBuffer,

    /// A list of indices in the `NetworkJson` vector of nodes
    /// linking to parent nodes
    pub parents: Vec<usize>,

    /// The immediate parent node
    pub immediate_parent: Option<usize>,

    /// The node type
    pub node_type: Option<String>,

    /// Rolling per-site TemporalHeatmap (optional, allocated when enabled).
    pub heatmap: Option<TemporalHeatmap>,

    /// Rolling per-site QoO/QoQ TemporalQoqHeatmap (optional, allocated when enabled).
    pub qoq_heatmap: Option<TemporalQoqHeatmap>,
}

impl NetworkJsonNode {
    /// Make a deep copy of a `NetworkJsonNode`, converting atomics
    /// into concrete values.
    pub fn clone_to_transit(&self) -> NetworkJsonTransport {
        let download =
            self.rtt_buffer
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50);
        let upload =
            self.rtt_buffer
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50);

        let rtts = match (download, upload) {
            (None, None) => Vec::new(),
            (Some(d), None) => vec![d.as_millis() as f32; 2],
            (None, Some(u)) => vec![u.as_millis() as f32; 2],
            (Some(d), Some(u)) => vec![d.as_millis() as f32, u.as_millis() as f32],
        };

        let qoo = self
            .qoq_heatmap
            .as_ref()
            .map(|heatmap| {
                let blocks = heatmap.blocks();
                let latest = |values: &[Option<f32>]| values.iter().rev().find_map(|v| *v);
                (latest(&blocks.download_total), latest(&blocks.upload_total))
            })
            .unwrap_or((None, None));

        NetworkJsonTransport {
            name: self.name.clone(),
            is_virtual: self.virtual_node,
            max_throughput: self.max_throughput,
            current_throughput: (
                self.current_throughput.get_down(),
                self.current_throughput.get_up(),
            ),
            current_packets: (
                self.current_packets.get_down(),
                self.current_packets.get_up(),
            ),
            current_tcp_packets: (
                self.current_tcp_packets.get_down(),
                self.current_tcp_packets.get_up(),
            ),
            current_udp_packets: (
                self.current_udp_packets.get_down(),
                self.current_udp_packets.get_up(),
            ),
            current_icmp_packets: (
                self.current_icmp_packets.get_down(),
                self.current_icmp_packets.get_up(),
            ),
            current_retransmits: (
                self.current_tcp_retransmits.get_down(),
                self.current_tcp_retransmits.get_up(),
            ),
            current_marks: (self.current_marks.get_down(), self.current_marks.get_up()),
            current_drops: (self.current_drops.get_down(), self.current_drops.get_up()),
            rtts,
            qoo,
            parents: self.parents.clone(),
            immediate_parent: self.immediate_parent,
            node_type: self.node_type.clone(),
        }
    }
}

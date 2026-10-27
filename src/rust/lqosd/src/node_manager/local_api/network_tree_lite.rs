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
    /// Configured maximum throughput in Mbps.
    pub max_throughput: (f64, f64),
    /// Current throughput in bytes per second.
    pub current_throughput: (u64, u64),
    /// Current TCP packets.
    pub current_tcp_packets: (u64, u64),
    /// TCP packets paired with the current retransmit counts.
    #[serde(default)]
    pub current_tcp_retransmit_packets: (u64, u64),
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
    /// Total number of unique circuits attached to this node or any descendant
    /// node — the site's STRUCTURAL footprint (installed circuits, not live
    /// load). Attached from `build_network_tree_summaries`. Consumers are
    /// additive-only; `#[serde(default)]` lets a NEW reader accept an OLD/absent
    /// payload (→ 0) — lqosd has no `deny_unknown_fields`, so an OLD reader
    /// already tolerates the NEW field.
    #[serde(default)]
    pub subtree_circuit_count: u32,
}

/// Returns the current lightweight network tree snapshot for websocket/API consumers.
pub fn network_tree_lite_data() -> Vec<(usize, NetworkTreeLiteNode)> {
    full_network_map_lite_snapshot()
}

#[cfg(test)]
mod tests {
    use super::NetworkTreeLiteNode;

    // The fields without `#[serde(default)]`.
    const REQUIRED: &str =
        "\"name\":\"n\",\"max_throughput\":[0.0,0.0],\"current_throughput\":[0,0],\
         \"current_tcp_packets\":[0,0],\"current_retransmits\":[0,0],\"rtts\":[],\"parents\":[]";

    /// `.67.5` R2 + R3: an OLD/absent payload is tolerated by a NEW reader
    /// (`#[serde(default)]` => 0; lqosd has no `deny_unknown_fields`), and the
    /// field is genuinely READ in-crate (not merely written).
    #[test]
    fn subtree_circuit_count_defaults_when_absent_and_reads_when_present() {
        let old: NetworkTreeLiteNode = serde_json::from_str(&format!("{{{REQUIRED}}}"))
            .expect("an older payload without subtree_circuit_count should deserialize");
        assert_eq!(old.subtree_circuit_count, 0);

        let new: NetworkTreeLiteNode =
            serde_json::from_str(&format!("{{{REQUIRED},\"subtree_circuit_count\":42}}"))
                .expect("a payload with subtree_circuit_count should deserialize");
        assert_eq!(new.subtree_circuit_count, 42);
    }
}

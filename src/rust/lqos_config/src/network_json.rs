mod network_json_node;
mod network_json_transport;

use allocative_derive::Allocative;
use lqos_utils::{temporal_heatmap::TemporalHeatmap, units::DownUpOrder};
pub use network_json_node::NetworkJsonNode;
pub use network_json_transport::NetworkJsonTransport;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::{
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;
use tracing::{debug, error, warn};

/// Holder for the network.json representation.
/// This is condensed into a single level vector with index-based referencing
/// for easy use in funnel calculations.
#[derive(Debug, Allocative)]
pub struct NetworkJson {
    /// Nodes that make up the tree, flattened and referenced by index number.
    /// TODO: We should add a primary key to nodes in network.json.
    pub nodes: Vec<NetworkJsonNode>,
}

impl Default for NetworkJson {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkJson {
    /// Generates an empty network.json
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Returns the length and capacity of the nodes vector.
    pub fn len_and_capacity(&self) -> (usize, usize) {
        (self.nodes.len(), self.nodes.capacity())
    }

    /// The path to the current `network.json` file, determined
    /// by acquiring the prefix from the `/etc/lqos.conf` configuration
    /// file.
    pub fn path() -> Result<PathBuf, NetworkJsonError> {
        let cfg = crate::load_config().map_err(|_| NetworkJsonError::ConfigLoadError)?;
        let base_path = Path::new(&cfg.lqos_directory);
        let file_path = if cfg.long_term_stats.enable_insight_topology.unwrap_or(false) {
            let tmp_path = base_path.join("network.insight.json");
            if tmp_path.exists() {
                tmp_path
            } else {
                base_path.join("network.json")
            }
        } else {
            base_path.join("network.json")
        };
        Ok(file_path)
    }

    /// Does network.json exist?
    pub fn exists() -> bool {
        if let Ok(path) = Self::path() {
            path.exists()
        } else {
            false
        }
    }

    /// Attempt to load network.json from disk
    pub fn load() -> Result<Self, NetworkJsonError> {
        let mut nodes = vec![NetworkJsonNode {
            name: "Root".to_string(),
            max_throughput: (0, 0),
            current_throughput: DownUpOrder::zeroed(),
            current_packets: DownUpOrder::zeroed(),
            current_tcp_packets: DownUpOrder::zeroed(),
            current_udp_packets: DownUpOrder::zeroed(),
            current_icmp_packets: DownUpOrder::zeroed(),
            current_tcp_retransmits: DownUpOrder::zeroed(),
            current_drops: DownUpOrder::zeroed(),
            current_marks: DownUpOrder::zeroed(),
            parents: Vec::new(),
            immediate_parent: None,
            rtts: HashSet::new(),
            node_type: None,
            heatmap: None,
        }];
        if !Self::exists() {
            return Err(NetworkJsonError::FileNotFound);
        }
        let path = Self::path()?;
        let raw = fs::read_to_string(path).map_err(|_| NetworkJsonError::ConfigLoadError)?;
        let json: Value =
            serde_json::from_str(&raw).map_err(|_| NetworkJsonError::ConfigLoadError)?;

        // Start reading from the top. We are at the root node.
        let parents = vec![0];
        if let Value::Object(map) = &json {
            for (key, value) in map.iter() {
                if let Value::Object(inner_map) = value {
                    recurse_node(&mut nodes, key, inner_map, &parents, 0);
                }
            }
        }

        Ok(Self { nodes })
    }

    /// Find the index of a circuit_id
    pub fn get_index_for_name(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// Retrieve a cloned copy of a NetworkJsonNode entry, or None if there isn't
    /// an entry at that index.
    pub fn get_cloned_entry_by_index(&self, index: usize) -> Option<NetworkJsonTransport> {
        self.nodes.get(index).map(|n| n.clone_to_transit())
    }

    /// Retrieve a cloned copy of all children with a parent containing a specific
    /// node index.
    pub fn get_cloned_children(&self, index: usize) -> Vec<(usize, NetworkJsonTransport)> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_i, n)| n.immediate_parent == Some(index))
            .map(|(i, n)| (i, n.clone_to_transit()))
            .collect()
    }

    /// Find a circuit_id, and if it exists return its list of parent nodes
    /// as indices within the network_json layout.
    pub fn get_parents_for_circuit_id(&self, circuit_id: &str) -> Option<Vec<usize>> {
        //println!("Looking for parents of {circuit_id}");
        self.nodes
            .iter()
            .find(|n| n.name == circuit_id)
            .map(|node| node.parents.clone())
    }

    /// Obtains a reference to nodes once we're sure that
    /// doing so will provide valid data.
    pub fn get_nodes_when_ready(&self) -> &Vec<NetworkJsonNode> {
        &self.nodes
    }

    /// Sets all current throughput values to zero
    /// Note that due to interior mutability, this does not require mutable
    /// access.
    pub fn zero_throughput_and_rtt(&mut self) {
        //log::warn!("Locking network tree for throughput cycle");
        self.nodes.iter_mut().for_each(|n| {
            n.current_throughput.set_to_zero();
            n.current_packets.set_to_zero();
            n.current_tcp_packets.set_to_zero();
            n.current_udp_packets.set_to_zero();
            n.current_icmp_packets.set_to_zero();
            n.current_tcp_retransmits.set_to_zero();
            n.rtts.clear();
            n.current_drops.set_to_zero();
            n.current_marks.set_to_zero();
        });
    }

    /// Add throughput numbers to node entries. Note that this does *not* require
    /// mutable access due to atomics and interior mutability - so it is safe to use
    /// a read lock.
    pub fn add_throughput_cycle(
        &mut self,
        targets: &[usize],
        bytes: (u64, u64),
        packets: (u64, u64),
        tcp: (u64, u64),
        udp: (u64, u64),
        icmp: (u64, u64),
    ) {
        for idx in targets {
            // Safety first: use "get" to ensure that the node exists
            if let Some(node) = self.nodes.get_mut(*idx) {
                node.current_throughput.checked_add_tuple(bytes);
                node.current_packets.checked_add_tuple(packets);
                node.current_tcp_packets.checked_add_tuple(tcp);
                node.current_udp_packets.checked_add_tuple(udp);
                node.current_icmp_packets.checked_add_tuple(icmp);
            } else {
                warn!("No network tree entry for index {idx}");
            }
        }
    }

    /// Record RTT time in the tree. Note that due to interior mutability,
    /// this does not require mutable access.
    pub fn add_rtt_cycle(&mut self, targets: &[usize], rtt: f32) {
        for idx in targets {
            // Safety first: use "get" to ensure that the node exists
            if let Some(node) = self.nodes.get_mut(*idx) {
                node.rtts.insert((rtt * 100.0) as u16);
            } else {
                warn!("No network tree entry for index {idx}");
            }
        }
    }

    /// Record TCP Retransmits in the tree.
    pub fn add_retransmit_cycle(&mut self, targets: &[usize], tcp_retransmits: DownUpOrder<u64>) {
        for idx in targets {
            // Safety first; use "get" to ensure that the node exists
            if let Some(node) = self.nodes.get_mut(*idx) {
                node.current_tcp_retransmits.checked_add(tcp_retransmits);
            } else {
                warn!("No network tree entry for index {idx}");
            }
        }
    }

    /// Adds a series of CAKE marks and drops to the tree structure.
    pub fn add_queue_cycle(
        &mut self,
        targets: &[usize],
        marks: &DownUpOrder<u64>,
        drops: &DownUpOrder<u64>,
    ) {
        for idx in targets {
            // Safety first; use "get" to ensure that the node exists
            if let Some(node) = self.nodes.get_mut(*idx) {
                node.current_marks.checked_add(*marks);
                node.current_drops.checked_add(*drops);
            } else {
                warn!("No network tree entry for index {idx}");
            }
        }
    }

    /// Record a heatmap sample for each site based on the current per-cycle data.
    pub fn record_site_heatmaps(&mut self, enable: bool) {
        if !enable {
            self.nodes.iter_mut().for_each(|node| node.heatmap = None);
            return;
        }

        for node in self.nodes.iter_mut() {
            let download_util =
                utilization_percent_bytes(node.current_throughput.down, node.max_throughput.0)
                    .unwrap_or(0.0);
            let upload_util =
                utilization_percent_bytes(node.current_throughput.up, node.max_throughput.1)
                    .unwrap_or(0.0);
            let mut rtts: Vec<f32> = node.rtts.iter().map(|n| *n as f32 / 100.0).collect();
            let median_rtt = median_rtt(&mut rtts);
            let retransmit_down = retransmit_percent(
                node.current_tcp_retransmits.down,
                node.current_tcp_packets.down,
            );
            let retransmit_up =
                retransmit_percent(node.current_tcp_retransmits.up, node.current_tcp_packets.up);

            let heatmap = node.heatmap.get_or_insert_with(TemporalHeatmap::new);
            heatmap.add_sample(
                download_util,
                upload_util,
                median_rtt,
                median_rtt,
                retransmit_down,
                retransmit_up,
            );
        }
    }
}

fn json_to_u32(val: Option<&Value>) -> u32 {
    if let Some(val) = val {
        if let Some(n) = val.as_u64() {
            n as u32
        } else {
            0
        }
    } else {
        0
    }
}

fn recurse_node(
    nodes: &mut Vec<NetworkJsonNode>,
    name: &str,
    json: &Map<String, Value>,
    parents: &[usize],
    immediate_parent: usize,
) {
    debug!("Mapping {name} from network.json");
    let mut parents = parents.to_vec();
    let my_id = if name != "children" {
        parents.push(nodes.len());
        nodes.len()
    } else {
        nodes.len() - 1
    };
    let node = NetworkJsonNode {
        parents: parents.to_vec(),
        max_throughput: (
            json_to_u32(json.get("downloadBandwidthMbps")),
            json_to_u32(json.get("uploadBandwidthMbps")),
        ),
        current_throughput: DownUpOrder::zeroed(),
        current_packets: DownUpOrder::zeroed(),
        current_tcp_packets: DownUpOrder::zeroed(),
        current_udp_packets: DownUpOrder::zeroed(),
        current_icmp_packets: DownUpOrder::zeroed(),
        current_tcp_retransmits: DownUpOrder::zeroed(),
        current_drops: DownUpOrder::zeroed(),
        current_marks: DownUpOrder::zeroed(),
        name: name.to_string(),
        immediate_parent: Some(immediate_parent),
        rtts: HashSet::new(),
        node_type: json
            .get("type")
            .map(|v| v.as_str().unwrap_or_default().to_string()),
        heatmap: None,
    };

    if node.name != "children" {
        nodes.push(node);
    }

    // Recurse children
    for (key, value) in json.iter() {
        let key_str = key.as_str();
        if key_str != "uploadBandwidthMbps"
            && key_str != "downloadBandwidthMbps"
            && let Value::Object(value) = value
        {
            recurse_node(nodes, key, value, &parents, my_id);
        }
    }
}

fn utilization_percent_bytes(bytes: u64, max_mbps: u32) -> Option<f32> {
    if max_mbps == 0 {
        return None;
    }
    let bits_per_second = bytes.saturating_mul(8) as f64;
    let capacity_bps = max_mbps as f64 * 1_000_000.0;
    Some(((bits_per_second / capacity_bps) * 100.0) as f32)
}

fn retransmit_percent(retransmits: u64, packets: u64) -> Option<f32> {
    if retransmits == 0 || packets == 0 {
        return None;
    }
    Some((retransmits as f32 / packets as f32) * 100.0)
}

fn median_rtt(values: &mut Vec<f32>) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[mid])
    } else {
        Some((values[mid - 1] + values[mid]) / 2.0)
    }
}

#[derive(Error, Debug)]
pub enum NetworkJsonError {
    #[error("Unable to find or load network.json")]
    ConfigLoadError,
    #[error("network.json not found or does not exist")]
    FileNotFound,
}

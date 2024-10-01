mod network_json_node;
mod network_json_transport;
mod network_json_counting;

use dashmap::DashSet;
use tracing::{debug, error};
use serde_json::{Map, Value};
use std::{
    fs, path::{Path, PathBuf},
};
use thiserror::Error;
use lqos_utils::units::DownUpOrder;
pub use network_json_node::NetworkJsonNode;
pub use network_json_transport::NetworkJsonTransport;
pub use network_json_counting::NetworkJsonCounting;

/// Holder for the network.json representation.
/// This is condensed into a single level vector with index-based referencing
/// for easy use in funnel calculations.
#[derive(Debug)]
pub struct NetworkJson {
    /// Nodes that make up the tree, flattened and referenced by index number.
    /// TODO: We should add a primary key to nodes in network.json.
    nodes: Vec<NetworkJsonNode>,
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

    pub fn len_and_capacity(&self) -> (usize, usize) {
        (self.nodes.len(), self.nodes.capacity())
    }

    /// The path to the current `network.json` file, determined
    /// by acquiring the prefix from the `/etc/lqos.conf` configuration
    /// file.
    pub fn path() -> Result<PathBuf, NetworkJsonError> {
        let cfg =
            crate::load_config().map_err(|_| NetworkJsonError::ConfigLoadError)?;
        let base_path = Path::new(&cfg.lqos_directory);
        Ok(base_path.join("network.json"))
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
            current_tcp_retransmits: DownUpOrder::zeroed(),
            current_drops: DownUpOrder::zeroed(),
            current_marks: DownUpOrder::zeroed(),
            parents: Vec::new(),
            immediate_parent: None,
            rtts: DashSet::new(),
            node_type: None,
        }];
        if !Self::exists() {
            return Err(NetworkJsonError::FileNotFound);
        }
        let path = Self::path()?;
        let raw = fs::read_to_string(path)
            .map_err(|_| NetworkJsonError::ConfigLoadError)?;
        let json: Value = serde_json::from_str(&raw)
            .map_err(|_| NetworkJsonError::ConfigLoadError)?;

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
    pub fn get_cloned_entry_by_index(
        &self,
        index: usize,
    ) -> Option<NetworkJsonTransport> {
        self.nodes.get(index).map(|n| n.clone_to_transit())
    }

    /// Retrieve a cloned copy of all children with a parent containing a specific
    /// node index.
    pub fn get_cloned_children(
        &self,
        index: usize,
    ) -> Vec<(usize, NetworkJsonTransport)> {
        self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_i, n)| n.immediate_parent == Some(index))
            .map(|(i, n)| (i, n.clone_to_transit()))
            .collect()
    }

    /// Find a circuit_id, and if it exists return its list of parent nodes
    /// as indices within the network_json layout.
    pub fn get_parents_for_circuit_id(
        &self,
        circuit_id: &str,
    ) -> Option<Vec<usize>> {
        //println!("Looking for parents of {circuit_id}");
        self
            .nodes
            .iter()
            .find(|n| n.name == circuit_id)
            .map(|node| node.parents.clone())
    }

    /// Obtains a reference to nodes once we're sure that
    /// doing so will provide valid data.
    pub fn get_nodes_when_ready(&self) -> &Vec<NetworkJsonNode> {
        //log::warn!("Awaiting the network tree");
        //atomic_wait::wait(&self.busy, 1);
        //log::warn!("Acquired");
        &self.nodes
    }

    /// Starts an update cycle. This clones the nodes into
    /// another structure - work will be performed on the clone.
    pub fn begin_update_cycle(&self) -> NetworkJsonCounting {
        NetworkJsonCounting::begin_update_cycle(self.nodes.clone())
    }

    /// Finishes an update cycle. This is called after all updates
    /// have been made to the clone, and the clone is then copied back
    /// into the main structure.
    pub fn finish_update_cycle(&mut self, counting: NetworkJsonCounting) {
        if !counting.nodes.is_empty() {
            self.nodes = counting.nodes;
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
        current_tcp_retransmits: DownUpOrder::zeroed(),
        current_drops: DownUpOrder::zeroed(),
        current_marks: DownUpOrder::zeroed(),
        name: name.to_string(),
        immediate_parent: Some(immediate_parent),
        rtts: DashSet::new(),
        node_type: json.get("type").map(|v| v.as_str().unwrap().to_string()),
    };

    if node.name != "children" {
        nodes.push(node);
    }

    // Recurse children
    for (key, value) in json.iter() {
        let key_str = key.as_str();
        if key_str != "uploadBandwidthMbps" && key_str != "downloadBandwidthMbps" {
            if let Value::Object(value) = value {
                recurse_node(nodes, key, value, &parents, my_id);
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum NetworkJsonError {
    #[error("Unable to find or load network.json")]
    ConfigLoadError,
    #[error("network.json not found or does not exist")]
    FileNotFound,
}

use crate::etc;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
  fs,
  path::{Path, PathBuf},
};
use thiserror::Error;

/// Describes a node in the network map tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkJsonNode {
  /// The node name, as it appears in `network.json`
  pub name: String,

  /// The maximum throughput allowed per `network.json` for this node
  pub max_throughput: (u32, u32), // In mbps

  /// Current throughput (in bytes/second) at this node
  pub current_throughput: (u64, u64), // In bytes

  /// Approximate RTTs reported for this level of the tree.
  /// It's never going to be as statistically accurate as the actual
  /// numbers, being based on medians.
  pub rtts: Vec<f32>,

  /// A list of indices in the `NetworkJson` vector of nodes
  /// linking to parent nodes
  pub parents: Vec<usize>,

  /// The immediate parent node
  pub immediate_parent: Option<usize>,
}

/// Holder for the network.json representation.
/// This is condensed into a single level vector with index-based referencing
/// for easy use in funnel calculations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

  /// The path to the current `network.json` file, determined
  /// by acquiring the prefix from the `/etc/lqos.conf` configuration
  /// file.
  pub fn path() -> Result<PathBuf, NetworkJsonError> {
    let cfg =
      etc::EtcLqos::load().map_err(|_| NetworkJsonError::ConfigLoadError)?;
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
      current_throughput: (0, 0),
      parents: Vec::new(),
      immediate_parent: None,
      rtts: Vec::new(),
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
  ) -> Option<NetworkJsonNode> {
    self.nodes.get(index).cloned()
  }

  /// Retrieve a cloned copy of all children with a parent containing a specific
  /// node index.
  pub fn get_cloned_children(
    &self,
    index: usize,
  ) -> Vec<(usize, NetworkJsonNode)> {
    self
      .nodes
      .iter()
      .enumerate()
      .filter(|(_i, n)| n.immediate_parent == Some(index))
      .map(|(i, n)| (i, n.clone()))
      .collect()
  }

  /// Find a circuit_id, and if it exists return its list of parent nodes
  /// as indices within the network_json layout.
  pub fn get_parents_for_circuit_id(
    &self,
    circuit_id: &str,
  ) -> Option<Vec<usize>> {
    self
      .nodes
      .iter()
      .find(|n| n.name == circuit_id)
      .map(|node| node.parents.clone())
  }

  /// Sets all current throughput values to zero
  pub fn zero_throughput_and_rtt(&mut self) {
    self.nodes.iter_mut().for_each(|n| {
      n.current_throughput = (0, 0);
      n.rtts.clear();
    });
  }

  /// Add throughput numbers to node entries
  pub fn add_throughput_cycle(
    &mut self,
    targets: &[usize],
    bytes: (u64, u64),
  ) {
    for idx in targets {
      // Safety first: use "get" to ensure that the node exists
      if let Some(node) = self.nodes.get_mut(*idx) {
        node.current_throughput.0 += bytes.0;
        node.current_throughput.1 += bytes.1;
      } else {
        warn!("No network tree entry for index {idx}");
      }
    }
  }

  /// Record RTT time in the tree
  pub fn add_rtt_cycle(&mut self, targets: &[usize], rtt: f32) {
    for idx in targets {
      // Safety first: use "get" to ensure that the node exists
      if let Some(node) = self.nodes.get_mut(*idx) {
        node.rtts.push(rtt);
      } else {
        warn!("No network tree entry for index {idx}");
      }
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
  info!("Mapping {name} from network.json");
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
    current_throughput: (0, 0),
    name: name.to_string(),
    immediate_parent: Some(immediate_parent),
    rtts: Vec::new(),
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

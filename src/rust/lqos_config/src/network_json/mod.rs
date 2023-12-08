use dashmap::DashSet;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
  fs,
  path::{Path, PathBuf}, sync::atomic::AtomicU64,
};
use thiserror::Error;

/// Describes a node in the network map tree.
#[derive(Debug)]
pub struct NetworkJsonNode {
  /// The node name, as it appears in `network.json`
  pub name: String,

  /// The maximum throughput allowed per `network.json` for this node
  pub max_throughput: (u32, u32), // In mbps

  /// Current throughput (in bytes/second) at this node
  pub current_throughput: (AtomicU64, AtomicU64), // In bytes

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
        self.current_throughput.0.load(std::sync::atomic::Ordering::Relaxed),
        self.current_throughput.1.load(std::sync::atomic::Ordering::Relaxed),
      ),
      rtts: self.rtts.iter().map(|n| *n as f32 / 100.0).collect(),
      parents: self.parents.clone(),
      immediate_parent: self.immediate_parent,
      node_type: self.node_type.clone(),
    }
  }
}

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

/// Holder for the network.json representation.
/// This is condensed into a single level vector with index-based referencing
/// for easy use in funnel calculations.
#[derive(Debug)]
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
      current_throughput: (AtomicU64::new(0), AtomicU64::new(0)),
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

  /// Sets all current throughput values to zero
  /// Note that due to interior mutability, this does not require mutable
  /// access.
  pub fn zero_throughput_and_rtt(&self) {
    self.nodes.iter().for_each(|n| {
      n.current_throughput.0.store(0, std::sync::atomic::Ordering::Relaxed);
      n.current_throughput.1.store(0, std::sync::atomic::Ordering::Relaxed);
      n.rtts.clear();
    });
  }

  /// Add throughput numbers to node entries. Note that this does *not* require
  /// mutable access due to atomics and interior mutability - so it is safe to use
  /// a read lock.
  pub fn add_throughput_cycle(
    &self,
    targets: &[usize],
    bytes: (u64, u64),
  ) {
    for idx in targets {
      // Safety first: use "get" to ensure that the node exists
      if let Some(node) = self.nodes.get(*idx) {
        node.current_throughput.0.fetch_add(bytes.0, std::sync::atomic::Ordering::Relaxed);
        node.current_throughput.1.fetch_add(bytes.1, std::sync::atomic::Ordering::Relaxed);
      } else {
        warn!("No network tree entry for index {idx}");
      }
    }
  }

  /// Record RTT time in the tree. Note that due to interior mutability,
  /// this does not require mutable access.
  pub fn add_rtt_cycle(&self, targets: &[usize], rtt: f32) {
    for idx in targets {
      // Safety first: use "get" to ensure that the node exists
      if let Some(node) = self.nodes.get(*idx) {
        node.rtts.insert((rtt * 100.0) as u16);
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
    current_throughput: (AtomicU64::new(0), AtomicU64::new(0)),
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

use crate::Config;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Runtime filename carrying integration-provided topology parent candidates for UI editing.
pub const TOPOLOGY_PARENT_CANDIDATES_FILENAME: &str = "topology_parent_candidates.json";

/// Errors returned while reading or writing topology parent candidate snapshots.
#[derive(Debug, Error)]
pub enum TopologyParentCandidatesError {
    /// Reading or writing the snapshot file failed.
    #[error("Unable to access topology parent candidates file: {0}")]
    Io(#[from] std::io::Error),
    /// Serializing or deserializing the snapshot failed.
    #[error("Unable to parse topology parent candidates JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// One immediate upstream parent candidate for a site/AP node.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopologyParentCandidate {
    /// Stable node identifier matching `network.json` metadata.
    pub node_id: String,
    /// Display name for the candidate node.
    pub node_name: String,
}

/// Candidate-parent metadata for one topology node.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyParentCandidatesNode {
    /// Stable node identifier matching `network.json` metadata.
    pub node_id: String,
    /// Display name for the node.
    pub node_name: String,
    /// Currently selected immediate parent node ID, if one was resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_parent_node_id: Option<String>,
    /// Currently selected immediate parent node name, if one was resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_parent_node_name: Option<String>,
    /// Ordered immediate upstream candidates detected by the integration.
    #[serde(default)]
    pub candidate_parents: Vec<TopologyParentCandidate>,
}

/// Integration-generated runtime snapshot of node parent candidates.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyParentCandidatesFile {
    /// Human-readable source for the snapshot, such as `uisp/full`.
    #[serde(default)]
    pub source: String,
    /// Stable identity of the imported topology facts plus selected compile mode, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_identity: Option<String>,
    /// Candidate-parent metadata keyed by node.
    #[serde(default)]
    pub nodes: Vec<TopologyParentCandidatesNode>,
}

/// Returns the path of the topology parent candidate runtime file.
///
/// This function is pure: it has no side effects.
pub fn topology_parent_candidates_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_PARENT_CANDIDATES_FILENAME)
}

fn atomic_write_json<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), TopologyParentCandidatesError> {
    let raw = serde_json::to_string_pretty(value)?;
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

impl TopologyParentCandidatesFile {
    /// Loads the topology parent candidate snapshot if it exists.
    ///
    /// Side effects: reads `topology_parent_candidates.json` from `config.lqos_directory`.
    pub fn load(config: &Config) -> Result<Self, TopologyParentCandidatesError> {
        let path = topology_parent_candidates_path(config);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Saves the topology parent candidate snapshot atomically.
    ///
    /// Side effects: writes `topology_parent_candidates.json` into `config.lqos_directory`.
    pub fn save(&self, config: &Config) -> Result<(), TopologyParentCandidatesError> {
        atomic_write_json(&topology_parent_candidates_path(config), self)
    }

    /// Finds candidate-parent metadata for `node_id`.
    ///
    /// This function is pure: it has no side effects.
    pub fn find_node(&self, node_id: &str) -> Option<&TopologyParentCandidatesNode> {
        self.nodes.iter().find(|node| node.node_id == node_id)
    }
}

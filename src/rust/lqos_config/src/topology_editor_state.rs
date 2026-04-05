use crate::{
    Config, TOPOLOGY_PARENT_CANDIDATES_FILENAME, TopologyParentCandidatesError,
    TopologyParentCandidatesFile,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Runtime filename carrying integration-provided topology editor state for UI editing.
pub const TOPOLOGY_EDITOR_STATE_FILENAME: &str = "topology_editor_state.json";

/// Stable pseudo-ID for the dynamic attachment mode.
pub const TOPOLOGY_ATTACHMENT_AUTO_ID: &str = "auto";

/// Runtime health state for an attachment option.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopologyAttachmentHealthStatus {
    /// Attachment is eligible for selection.
    #[default]
    Healthy,
    /// Attachment is temporarily suppressed due to probe failure.
    Suppressed,
    /// Attachment cannot be probed because required endpoint metadata is unavailable.
    ProbeUnavailable,
    /// Attachment probing is disabled for this pair.
    Disabled,
}

/// Errors returned while reading or writing topology editor snapshots.
#[derive(Debug, Error)]
pub enum TopologyEditorStateError {
    /// Reading or writing the snapshot file failed.
    #[error("Unable to access topology editor state file: {0}")]
    Io(#[from] std::io::Error),
    /// Serializing or deserializing the snapshot failed.
    #[error("Unable to parse topology editor state JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Reading the legacy parent-candidate snapshot failed.
    #[error("Unable to load legacy topology parent candidates: {0}")]
    Legacy(#[from] TopologyParentCandidatesError),
}

/// One concrete attachment choice available under a selected parent.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopologyAttachmentOption {
    /// Stable attachment identifier. `auto` means LibreQoS will choose dynamically.
    pub attachment_id: String,
    /// Human-readable attachment label.
    pub attachment_name: String,
    /// Attachment kind such as `auto`, `site`, or `device`.
    #[serde(default)]
    pub attachment_kind: String,
    /// Stable pair identifier used for runtime probe policy and health state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair_id: Option<String>,
    /// Stable identifier of the far-side attachment/endpoint when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_attachment_id: Option<String>,
    /// Human-readable far-side attachment label when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_attachment_name: Option<String>,
    /// Capacity used when this attachment is effective.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_mbps: Option<u64>,
    /// Local management IP used for health probing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_probe_ip: Option<String>,
    /// Remote management IP used for health probing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_probe_ip: Option<String>,
    /// Whether health probing is enabled for this attachment pair.
    #[serde(default)]
    pub probe_enabled: bool,
    /// Whether this attachment pair is probeable with the current metadata.
    #[serde(default)]
    pub probeable: bool,
    /// Runtime health state of this attachment.
    #[serde(default)]
    pub health_status: TopologyAttachmentHealthStatus,
    /// Human-readable health/suppression reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_reason: Option<String>,
    /// Unix timestamp after which suppression may be cleared, when currently suppressed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppressed_until_unix: Option<u64>,
    /// Whether this attachment is currently effective in runtime topology.
    #[serde(default)]
    pub effective_selected: bool,
}

/// One valid parent target for a topology node, plus allowed attachments below that parent.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyAllowedParent {
    /// Stable node identifier for the allowed parent.
    pub parent_node_id: String,
    /// Human-readable parent label.
    pub parent_node_name: String,
    /// Ordered attachment choices valid for this `(child, parent)` pair.
    #[serde(default)]
    pub attachment_options: Vec<TopologyAttachmentOption>,
    /// Whether every explicit attachment under this parent is currently suppressed.
    #[serde(default)]
    pub all_attachments_suppressed: bool,
    /// Whether any explicit attachment under this parent is currently probe unavailable.
    #[serde(default)]
    pub has_probe_unavailable_attachments: bool,
}

/// Runtime topology-manager metadata for one movable node.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyEditorNode {
    /// Stable node identifier matching `network.json` metadata.
    pub node_id: String,
    /// Display name for the node.
    pub node_name: String,
    /// Currently resolved immediate parent node ID, if one was resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_parent_node_id: Option<String>,
    /// Currently resolved immediate parent node name, if one was resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_parent_node_name: Option<String>,
    /// Currently resolved concrete attachment identifier, if one was resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_attachment_id: Option<String>,
    /// Currently resolved concrete attachment label, if one was resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_attachment_name: Option<String>,
    /// Whether operators may move this node in the topology manager.
    #[serde(default)]
    pub can_move: bool,
    /// Ordered valid parent targets for this node.
    #[serde(default)]
    pub allowed_parents: Vec<TopologyAllowedParent>,
    /// Stable attachment identifier preferred by operator intent, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_attachment_id: Option<String>,
    /// Human-readable preferred attachment label, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_attachment_name: Option<String>,
    /// Stable attachment identifier currently effective for runtime shaping, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_attachment_id: Option<String>,
    /// Human-readable effective attachment label, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_attachment_name: Option<String>,
}

/// Integration-generated runtime snapshot consumed by the topology manager UI.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyEditorStateFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_topology_editor_schema_version")]
    pub schema_version: u32,
    /// Human-readable source for the snapshot, such as `uisp/full2`.
    #[serde(default)]
    pub source: String,
    /// Unix timestamp when the snapshot was generated, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_unix: Option<u64>,
    /// Runtime topology-manager metadata keyed by node.
    #[serde(default)]
    pub nodes: Vec<TopologyEditorNode>,
}

fn default_topology_editor_schema_version() -> u32 {
    1
}

/// Returns the path of the topology editor runtime file.
///
/// This function is pure: it has no side effects.
pub fn topology_editor_state_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_EDITOR_STATE_FILENAME)
}

impl TopologyEditorStateFile {
    /// Loads the topology editor snapshot if it exists.
    ///
    /// Side effects: reads `topology_editor_state.json` from `config.lqos_directory`.
    pub fn load(config: &Config) -> Result<Self, TopologyEditorStateError> {
        let path = topology_editor_state_path(config);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Loads the topology editor snapshot, falling back to the legacy UISP parent-candidate file.
    ///
    /// Side effects: reads `topology_editor_state.json` and, if missing, may read
    /// `topology_parent_candidates.json` from `config.lqos_directory`.
    pub fn load_with_legacy_fallback(config: &Config) -> Result<Self, TopologyEditorStateError> {
        let state = Self::load(config)?;
        if !state.nodes.is_empty() {
            return Ok(state);
        }

        let legacy = TopologyParentCandidatesFile::load(config)?;
        if legacy.nodes.is_empty() {
            return Ok(state);
        }
        Ok(Self::from_legacy_parent_candidates(&legacy))
    }

    /// Saves the topology editor snapshot.
    ///
    /// Side effects: writes `topology_editor_state.json` into `config.lqos_directory`.
    pub fn save(&self, config: &Config) -> Result<(), TopologyEditorStateError> {
        let path = topology_editor_state_path(config);
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(path, raw.as_bytes())?;
        Ok(())
    }

    /// Finds runtime editor metadata for `node_id`.
    ///
    /// This function is pure: it has no side effects.
    pub fn find_node(&self, node_id: &str) -> Option<&TopologyEditorNode> {
        self.nodes.iter().find(|node| node.node_id == node_id)
    }

    /// Converts the legacy parent-candidate snapshot into a minimal topology editor state.
    ///
    /// This function is pure: it has no side effects.
    pub fn from_legacy_parent_candidates(legacy: &TopologyParentCandidatesFile) -> Self {
        let mut nodes = Vec::with_capacity(legacy.nodes.len());
        for legacy_node in &legacy.nodes {
            let allowed_parents = legacy_node
                .candidate_parents
                .iter()
                .map(|candidate| TopologyAllowedParent {
                    parent_node_id: candidate.node_id.clone(),
                    parent_node_name: candidate.node_name.clone(),
                    attachment_options: vec![TopologyAttachmentOption {
                        attachment_id: TOPOLOGY_ATTACHMENT_AUTO_ID.to_string(),
                        attachment_name: "Auto".to_string(),
                        attachment_kind: "auto".to_string(),
                        pair_id: None,
                        peer_attachment_id: None,
                        peer_attachment_name: None,
                        capacity_mbps: None,
                        local_probe_ip: None,
                        remote_probe_ip: None,
                        probe_enabled: false,
                        probeable: false,
                        health_status: TopologyAttachmentHealthStatus::Disabled,
                        health_reason: None,
                        suppressed_until_unix: None,
                        effective_selected: false,
                    }],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                })
                .collect::<Vec<_>>();

            nodes.push(TopologyEditorNode {
                node_id: legacy_node.node_id.clone(),
                node_name: legacy_node.node_name.clone(),
                current_parent_node_id: legacy_node.current_parent_node_id.clone(),
                current_parent_node_name: legacy_node.current_parent_node_name.clone(),
                current_attachment_id: None,
                current_attachment_name: None,
                can_move: !allowed_parents.is_empty(),
                allowed_parents,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            });
        }

        Self {
            schema_version: default_topology_editor_schema_version(),
            source: if legacy.source.trim().is_empty() {
                format!("legacy:{TOPOLOGY_PARENT_CANDIDATES_FILENAME}")
            } else {
                format!("legacy:{}", legacy.source.trim())
            },
            generated_unix: None,
            nodes,
        }
    }
}

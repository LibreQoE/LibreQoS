use crate::{
    Config, TOPOLOGY_PARENT_CANDIDATES_FILENAME, TopologyCanonicalStateFile,
    TopologyParentCandidatesError, TopologyParentCandidatesFile,
    topology_canonical_state::{
        current_topology_ingress_identity, legacy_id_for_name, quarantine_stale_topology_state,
        topology_ingress_fingerprint_from_tokens,
    },
};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::warn;

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

/// Source category for attachment bandwidth values shown in Topology Manager.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopologyAttachmentRateSource {
    /// The integration did not classify the attachment rate source.
    #[default]
    Unknown,
    /// The attachment rate is static or operator-managed and may be overridden.
    Static,
    /// The attachment rate comes from dynamic integration telemetry and should not be overridden.
    DynamicIntegration,
    /// The attachment was defined manually by the operator.
    Manual,
}

/// Feed-role classification for one attachment option.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopologyAttachmentRole {
    /// The integration did not classify the attachment role.
    #[default]
    Unknown,
    /// A point-to-point backhaul path between sites.
    PtpBackhaul,
    /// A site fed as a client of an upstream PtMP AP.
    PtmpUplink,
    /// A wired handoff such as an ethernet or switch-based uplink.
    WiredUplink,
    /// A manual operator-defined attachment.
    Manual,
}

/// Baseline queue-visibility policy for a logical topology node.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopologyQueueVisibilityPolicy {
    /// The node remains visible in the baseline queue topology.
    #[default]
    QueueVisible,
    /// The node remains logical-only for queueing and its children are promoted one level.
    QueueHiddenPromoteChildren,
    /// LibreQoS decides queue visibility from node role and configured capacity.
    QueueAuto,
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
    /// Feed-role classification for this attachment.
    #[serde(default)]
    pub attachment_role: TopologyAttachmentRole,
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
    /// Effective download bandwidth for this attachment in Mbps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_bandwidth_mbps: Option<u64>,
    /// Effective upload bandwidth for this attachment in Mbps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_bandwidth_mbps: Option<u64>,
    /// Effective infrastructure transport cap applied to this attachment in Mbps, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_cap_mbps: Option<u64>,
    /// Human-readable explanation for the transport cap, when one was applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_cap_reason: Option<String>,
    /// Classification of where the attachment rates came from.
    #[serde(default)]
    pub rate_source: TopologyAttachmentRateSource,
    /// Whether operators may save a rate override for this attachment.
    #[serde(default)]
    pub can_override_rate: bool,
    /// Human-readable reason rate overrides are unavailable, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_override_disabled_reason: Option<String>,
    /// Whether an attachment-scoped rate override is currently active.
    #[serde(default)]
    pub has_rate_override: bool,
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct TopologyEditorNode {
    /// Stable node identifier matching `network.json` metadata.
    pub node_id: String,
    /// Display name for the node.
    pub node_name: String,
    /// Optional geographic latitude.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f32>,
    /// Optional geographic longitude.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f32>,
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
    /// Baseline queue-visibility policy for runtime-effective topology export.
    #[serde(default)]
    pub queue_visibility_policy: TopologyQueueVisibilityPolicy,
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
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
    /// Stable identity of the imported topology facts plus selected compile mode, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_identity: Option<String>,
    /// Runtime topology-manager metadata keyed by node.
    #[serde(default)]
    pub nodes: Vec<TopologyEditorNode>,
}

fn default_topology_editor_schema_version() -> u32 {
    1
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), TopologyEditorStateError> {
    let raw = serde_json::to_string_pretty(value)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

/// Returns the path of the topology editor runtime file.
///
/// This function is pure: it has no side effects.
pub fn topology_editor_state_path(config: &Config) -> PathBuf {
    config.topology_state_read_path(TOPOLOGY_EDITOR_STATE_FILENAME)
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
            let is_current = match state.matches_current_ingress(config) {
                Ok(is_current) => is_current,
                Err(err) => {
                    warn!(
                        "Unable to validate topology editor state against current ingress; preserving existing state: {err}"
                    );
                    true
                }
            };
            if is_current {
                return Ok(state);
            }
            quarantine_stale_topology_state(
                config,
                &format!(
                    "topology editor source '{}' does not match current topology ingress identity",
                    state.source
                ),
            )?;
        }

        let canonical = TopologyCanonicalStateFile::load_with_legacy_fallback(config)?;
        if !canonical.nodes.is_empty() {
            return Ok(canonical.to_editor_state());
        }

        let legacy = TopologyParentCandidatesFile::load(config)?;
        if legacy.nodes.is_empty() {
            return Ok(state);
        }
        Ok(Self::from_legacy_parent_candidates(&legacy))
    }

    /// Saves the topology editor snapshot atomically.
    ///
    /// Side effects: writes `topology_editor_state.json` into `config.lqos_directory`.
    pub fn save(&self, config: &Config) -> Result<(), TopologyEditorStateError> {
        atomic_write_json(
            &config.topology_state_file_path(TOPOLOGY_EDITOR_STATE_FILENAME),
            self,
        )
    }

    /// Returns a stable fingerprint of the topology ingress this editor state represents.
    ///
    /// This function is pure: it has no side effects.
    pub fn topology_ingress_fingerprint(&self) -> Option<String> {
        topology_ingress_fingerprint_from_tokens(self.nodes.iter().map(|node| {
            let node_id = node.node_id.trim();
            if node_id.is_empty() {
                legacy_id_for_name(&node.node_name)
            } else {
                node_id.to_string()
            }
        }))
    }

    /// Returns true if this editor state still matches the current topology ingress identity.
    ///
    /// Side effects: reads topology ingress inputs from `config.lqos_directory`.
    pub fn matches_current_ingress(
        &self,
        config: &Config,
    ) -> Result<bool, TopologyEditorStateError> {
        if let Some(saved_identity) = self
            .ingress_identity
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let Some(current_identity) = current_topology_ingress_identity(config)? else {
                return Ok(true);
            };
            return Ok(saved_identity == current_identity);
        }

        let Some(current_fingerprint) = current_topology_ingress_identity(config)? else {
            return Ok(true);
        };
        let Some(saved_fingerprint) = self.topology_ingress_fingerprint() else {
            return Ok(false);
        };
        Ok(saved_fingerprint == current_fingerprint)
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
                        attachment_role: TopologyAttachmentRole::Unknown,
                        pair_id: None,
                        peer_attachment_id: None,
                        peer_attachment_name: None,
                        capacity_mbps: None,
                        download_bandwidth_mbps: None,
                        upload_bandwidth_mbps: None,
                        transport_cap_mbps: None,
                        transport_cap_reason: None,
                        rate_source: TopologyAttachmentRateSource::Unknown,
                        can_override_rate: false,
                        rate_override_disabled_reason: None,
                        has_rate_override: false,
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
                latitude: None,
                longitude: None,
                current_parent_node_id: legacy_node.current_parent_node_id.clone(),
                current_parent_node_name: legacy_node.current_parent_node_name.clone(),
                current_attachment_id: None,
                current_attachment_name: None,
                can_move: !allowed_parents.is_empty(),
                allowed_parents,
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
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
            ingress_identity: legacy.ingress_identity.clone(),
            nodes,
        }
    }
}

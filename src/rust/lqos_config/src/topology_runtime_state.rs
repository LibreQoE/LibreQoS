use crate::{Config, TopologyAttachmentHealthStatus};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Runtime filename carrying transient attachment-health state.
pub const TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME: &str = "topology_attachment_health_state.json";

/// Runtime filename carrying effective attachment selection state.
pub const TOPOLOGY_EFFECTIVE_STATE_FILENAME: &str = "topology_effective_state.json";

/// Runtime filename carrying the effective network tree for shaping/export.
pub const TOPOLOGY_EFFECTIVE_NETWORK_FILENAME: &str = "network.effective.json";

/// Errors returned while reading or writing topology runtime snapshots.
#[derive(Debug, Error)]
pub enum TopologyRuntimeStateError {
    /// Reading or writing the snapshot file failed.
    #[error("Unable to access topology runtime state file: {0}")]
    Io(#[from] std::io::Error),
    /// Serializing or deserializing the snapshot failed.
    #[error("Unable to parse topology runtime state JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// One probe endpoint result inside a health-state entry.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopologyAttachmentEndpointStatus {
    /// Stable attachment identifier for this endpoint.
    pub attachment_id: String,
    /// Probe target IP address.
    pub ip: String,
    /// Whether the endpoint responded during the most recent round.
    pub reachable: bool,
}

/// One attachment pair's runtime health state.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyAttachmentHealthEntry {
    /// Stable attachment pair identifier.
    pub attachment_pair_id: String,
    /// Stable attachment identifier used by the runtime topology/editor state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    /// Display name of the attachment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_name: Option<String>,
    /// Stable child node identifier being shaped through this attachment pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_node_id: Option<String>,
    /// Display name of the child node being shaped through this attachment pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_node_name: Option<String>,
    /// Stable parent node identifier for this attachment group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_node_id: Option<String>,
    /// Display name of the parent node for this attachment group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_node_name: Option<String>,
    /// Local management IP used for the probe, when configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_probe_ip: Option<String>,
    /// Remote management IP used for the probe, when configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_probe_ip: Option<String>,
    /// Current runtime health status.
    #[serde(default)]
    pub status: TopologyAttachmentHealthStatus,
    /// Human-readable reason for the current status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Whether the pair can currently be probed.
    #[serde(default)]
    pub probeable: bool,
    /// Whether probing is enabled for this pair.
    #[serde(default)]
    pub enabled: bool,
    /// Consecutive failed probe rounds.
    #[serde(default)]
    pub consecutive_misses: u32,
    /// Consecutive successful probe rounds.
    #[serde(default)]
    pub consecutive_successes: u32,
    /// Unix timestamp until which suppression must be held.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppressed_until_unix: Option<u64>,
    /// Unix timestamp of the last successful round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_unix: Option<u64>,
    /// Unix timestamp of the last failed round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_unix: Option<u64>,
    /// Endpoint-by-endpoint status for the last probe round.
    #[serde(default)]
    pub endpoint_status: Vec<TopologyAttachmentEndpointStatus>,
}

/// Full transient attachment-health snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyAttachmentHealthStateFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_runtime_schema_version")]
    pub schema_version: u32,
    /// Unix timestamp when the file was generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_unix: Option<u64>,
    /// Runtime state for each known attachment pair.
    #[serde(default)]
    pub attachments: Vec<TopologyAttachmentHealthEntry>,
}

/// Effective runtime state for one attachment beneath a node.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyEffectiveAttachmentState {
    /// Stable attachment identifier.
    pub attachment_id: String,
    /// Current runtime health status.
    #[serde(default)]
    pub health_status: TopologyAttachmentHealthStatus,
    /// Human-readable health/suppression reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_reason: Option<String>,
    /// Unix timestamp after which suppression may clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppressed_until_unix: Option<u64>,
    /// Whether health probing is enabled for this attachment pair.
    #[serde(default)]
    pub probe_enabled: bool,
    /// Whether this attachment pair is probeable.
    #[serde(default)]
    pub probeable: bool,
    /// Whether this attachment is currently selected as effective.
    #[serde(default)]
    pub effective_selected: bool,
}

/// Effective runtime state for one topology node.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyEffectiveNodeState {
    /// Stable node identifier.
    pub node_id: String,
    /// Stable logical parent node identifier.
    pub logical_parent_node_id: String,
    /// Stable preferred attachment identifier, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_attachment_id: Option<String>,
    /// Stable effective attachment identifier, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_attachment_id: Option<String>,
    /// Explanation for emergency fallback behavior, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    /// Whether all explicit attachments for the logical parent are currently suppressed.
    #[serde(default)]
    pub all_attachments_suppressed: bool,
    /// Effective runtime attachment states for this node.
    #[serde(default)]
    pub attachments: Vec<TopologyEffectiveAttachmentState>,
}

/// Full effective runtime topology snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyEffectiveStateFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_runtime_schema_version")]
    pub schema_version: u32,
    /// Unix timestamp when the file was generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_unix: Option<u64>,
    /// Generation timestamp of the canonical editor state used as input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_generated_unix: Option<u64>,
    /// Generation timestamp of the health-state input used as input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_generated_unix: Option<u64>,
    /// Effective node-by-node runtime state.
    #[serde(default)]
    pub nodes: Vec<TopologyEffectiveNodeState>,
}

fn default_runtime_schema_version() -> u32 {
    1
}

fn atomic_write_json<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), TopologyRuntimeStateError> {
    let raw = serde_json::to_string_pretty(value)?;
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

/// Returns the path of the runtime attachment-health state file.
pub fn topology_attachment_health_state_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME)
}

/// Returns the path of the effective topology state file.
pub fn topology_effective_state_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_EFFECTIVE_STATE_FILENAME)
}

/// Returns the path of the effective runtime network tree file.
pub fn topology_effective_network_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_EFFECTIVE_NETWORK_FILENAME)
}

impl TopologyAttachmentHealthStateFile {
    /// Loads the transient attachment-health state file if it exists.
    pub fn load(config: &Config) -> Result<Self, TopologyRuntimeStateError> {
        let path = topology_attachment_health_state_path(config);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Saves the transient attachment-health state file atomically.
    pub fn save(&self, config: &Config) -> Result<(), TopologyRuntimeStateError> {
        atomic_write_json(&topology_attachment_health_state_path(config), self)
    }
}

impl TopologyEffectiveStateFile {
    /// Loads the effective topology state file if it exists.
    pub fn load(config: &Config) -> Result<Self, TopologyRuntimeStateError> {
        let path = topology_effective_state_path(config);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Saves the effective topology state file atomically.
    pub fn save(&self, config: &Config) -> Result<(), TopologyRuntimeStateError> {
        atomic_write_json(&topology_effective_state_path(config), self)
    }
}

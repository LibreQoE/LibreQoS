use crate::{
    CIRCUIT_ANCHORS_FILENAME, Config, TOPOLOGY_CANONICAL_STATE_FILENAME,
    TOPOLOGY_EDITOR_STATE_FILENAME, TopologyAttachmentHealthStatus, TopologyCanonicalStateFile,
    TopologyEditorStateFile,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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

/// Runtime filename carrying integration-import topology facts before mode projection.
pub const TOPOLOGY_IMPORT_FILENAME: &str = "topology_import.json";

/// Runtime filename carrying compiled shaping rows and anchors for integration ingress.
pub const TOPOLOGY_COMPILED_SHAPING_FILENAME: &str = "topology_compiled_shaping.json";

/// Runtime filename carrying shaping-ready circuit inputs resolved from topology runtime.
pub const TOPOLOGY_SHAPING_INPUTS_FILENAME: &str = "shaping_inputs.json";

/// Runtime filename carrying topology publication readiness for one source generation.
pub const TOPOLOGY_RUNTIME_STATUS_FILENAME: &str = "topology_runtime_status.json";

const OPERATOR_OVERRIDES_FILENAME: &str = "lqos_overrides.json";
const STORMGUARD_OVERRIDES_FILENAME: &str = "lqos_overrides.stormguard.json";
const TREEGUARD_OVERRIDES_FILENAME: &str = "lqos_overrides.treeguard.json";
const LEGACY_AUTOPILOT_OVERRIDES_FILENAME: &str = "lqos_overrides.autopilot.json";

/// Errors returned while reading or writing topology runtime snapshots.
#[derive(Debug, Error)]
pub enum TopologyRuntimeStateError {
    /// Reading or writing the snapshot file failed.
    #[error("Unable to access topology runtime state file: {0}")]
    Io(#[from] std::io::Error),
    /// Serializing or deserializing the snapshot failed.
    #[error("Unable to parse topology runtime state JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Computing the topology source generation failed.
    #[error("Unable to compute topology source generation: {0}")]
    SourceGeneration(String),
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

/// Source used to resolve one circuit's shaping parent.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopologyShapingResolutionSource {
    /// Runtime topology resolved the effective parent from `circuit_anchors.json`
    /// or a compatible anchor input.
    #[default]
    TopologyAnchor,
    /// Legacy `ParentNode`/`ParentNodeID` was used because no anchor was available.
    LegacyParent,
    /// Runtime could not resolve the circuit to an exported effective queue node, so
    /// shaping falls back to generated parent nodes during queue construction.
    RuntimeFallback,
    /// Flat topology mode assigned the circuit to an explicit generated parent bucket.
    FlatBucket,
}

/// One shaped device row carried into `shaping_inputs.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyShapingDeviceInput {
    /// Stable device identifier.
    pub device_id: String,
    /// Human-readable device name.
    pub device_name: String,
    /// Device MAC address when known.
    pub mac: String,
    /// All IPv4 addresses/subnets associated with the device.
    #[serde(default)]
    pub ipv4: Vec<String>,
    /// All IPv6 addresses/subnets associated with the device.
    #[serde(default)]
    pub ipv6: Vec<String>,
    /// Free-form operator/integration comment.
    #[serde(default)]
    pub comment: String,
}

/// One shaping-ready circuit compiled from `ShapedDevices.csv` plus effective topology.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct TopologyShapingCircuitInput {
    /// Stable circuit identifier.
    pub circuit_id: String,
    /// Human-readable circuit name.
    pub circuit_name: String,
    /// Stable topology node identifier the circuit attaches beneath, when provided
    /// by the integration/operator input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_node_id: Option<String>,
    /// Human-readable topology anchor node name, when resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_node_name: Option<String>,
    /// Legacy logical parent name from `ShapedDevices.csv`, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_parent_node_name: Option<String>,
    /// Legacy logical parent node identifier from `ShapedDevices.csv`, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_parent_node_id: Option<String>,
    /// Runtime-effective parent node name used for shaping.
    pub effective_parent_node_name: String,
    /// Runtime-effective parent node identifier used for shaping.
    pub effective_parent_node_id: String,
    /// Runtime-effective attachment identifier, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_attachment_id: Option<String>,
    /// Runtime-effective attachment label, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_attachment_name: Option<String>,
    /// How the effective parent was resolved.
    #[serde(default)]
    pub resolution_source: TopologyShapingResolutionSource,
    /// Guaranteed minimum download rate in Mbps.
    pub download_min_mbps: f32,
    /// Guaranteed minimum upload rate in Mbps.
    pub upload_min_mbps: f32,
    /// Maximum download rate in Mbps.
    pub download_max_mbps: f32,
    /// Maximum upload rate in Mbps.
    pub upload_max_mbps: f32,
    /// Free-form operator/integration comment.
    #[serde(default)]
    pub comment: String,
    /// Optional per-circuit SQM override token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqm_override: Option<String>,
    /// Device rows belonging to this circuit.
    #[serde(default)]
    pub devices: Vec<TopologyShapingDeviceInput>,
}

/// Full runtime shaping-input snapshot consumed by `LibreQoS.py`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct TopologyShapingInputsFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_runtime_schema_version")]
    pub schema_version: u32,
    /// Stable generation hash for shaping-relevant content in this file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shaping_generation: String,
    /// Unix timestamp when the file was generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_unix: Option<u64>,
    /// Generation timestamp of the canonical topology used as input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_generated_unix: Option<u64>,
    /// Generation timestamp of the effective topology used as input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_generated_unix: Option<u64>,
    /// Non-fatal generation warnings.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Shaping-ready circuits.
    #[serde(default)]
    pub circuits: Vec<TopologyShapingCircuitInput>,
}

/// Topology runtime readiness published for one exact source generation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TopologyRuntimeStatusFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_runtime_schema_version")]
    pub schema_version: u32,
    /// Stable generation hash of the source inputs topology used for this publish attempt.
    #[serde(default)]
    pub source_generation: String,
    /// Stable generation hash of the current shaping-relevant runtime output, when ready.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shaping_generation: String,
    /// Whether runtime outputs are ready for the source generation above.
    #[serde(default)]
    pub ready: bool,
    /// Unix timestamp when the status file was generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_unix: Option<u64>,
    /// Effective-state artifact path for operator inspection.
    #[serde(default)]
    pub effective_state_path: String,
    /// Effective-network artifact path for operator inspection.
    #[serde(default)]
    pub effective_network_path: String,
    /// Shaping-inputs artifact path for operator inspection.
    #[serde(default)]
    pub shaping_inputs_path: String,
    /// Error for this generation when runtime publication failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn default_runtime_schema_version() -> u32 {
    1
}

fn topology_import_ingress_enabled(config: &Config) -> bool {
    config.uisp_integration.enable_uisp
        || config.splynx_integration.enable_splynx
        || config
            .netzur_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_netzur)
        || config
            .visp_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_visp)
        || config.powercode_integration.enable_powercode
        || config.sonar_integration.enable_sonar
        || config
            .wispgate_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_wispgate)
}

fn operator_overrides_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(OPERATOR_OVERRIDES_FILENAME)
}

fn stormguard_overrides_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(STORMGUARD_OVERRIDES_FILENAME)
}

fn treeguard_overrides_path(config: &Config) -> PathBuf {
    let canonical = Path::new(&config.lqos_directory).join(TREEGUARD_OVERRIDES_FILENAME);
    if canonical.exists() {
        return canonical;
    }
    let legacy = Path::new(&config.lqos_directory).join(LEGACY_AUTOPILOT_OVERRIDES_FILENAME);
    if legacy.exists() { legacy } else { canonical }
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

/// Returns the path of the topology import artifact file.
pub fn topology_import_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_IMPORT_FILENAME)
}

/// Returns the path of the runtime shaping-input snapshot file.
pub fn topology_shaping_inputs_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_SHAPING_INPUTS_FILENAME)
}

/// Returns the path of the topology runtime readiness status file.
pub fn topology_runtime_status_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_RUNTIME_STATUS_FILENAME)
}

/// Returns the path of the integration compiled-shaping artifact.
pub fn topology_compiled_shaping_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_COMPILED_SHAPING_FILENAME)
}

fn file_exists_with_nonempty_nodes(path: &Path) -> Result<bool, TopologyRuntimeStateError> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = std::fs::read_to_string(path)?;
    let value = serde_json::from_str::<serde_json::Value>(&raw)?;
    Ok(value
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|nodes| !nodes.is_empty()))
}

fn hash_file_state(
    hasher: &mut Sha256,
    label: &str,
    path: &Path,
) -> Result<(), TopologyRuntimeStateError> {
    hasher.update(label.as_bytes());
    hasher.update([0]);
    match std::fs::read(path) {
        Ok(bytes) => {
            hasher.update(b"present");
            hasher.update([0]);
            hasher.update(bytes.len().to_le_bytes());
            hasher.update([0]);
            hasher.update(&bytes);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            hasher.update(b"missing");
        }
        Err(err) => return Err(err.into()),
    }
    hasher.update([0xff]);
    Ok(())
}

/// Computes the stable source generation for current topology runtime inputs.
///
/// This generation changes whenever shaping-relevant source inputs change.
pub fn compute_topology_source_generation(
    config: &Config,
) -> Result<String, TopologyRuntimeStateError> {
    let base = Path::new(&config.lqos_directory);
    let canonical_path = base.join(TOPOLOGY_CANONICAL_STATE_FILENAME);
    let editor_path = base.join(TOPOLOGY_EDITOR_STATE_FILENAME);
    let topology_import_path = base.join(TOPOLOGY_IMPORT_FILENAME);
    let topology_compiled_shaping_path = base.join(TOPOLOGY_COMPILED_SHAPING_FILENAME);
    let use_topology_import = topology_import_ingress_enabled(config);
    let network_path = base.join("network.json");
    let shaped_devices_path = base.join("ShapedDevices.csv");
    let circuit_anchors_path = base.join(CIRCUIT_ANCHORS_FILENAME);

    let canonical_active = if file_exists_with_nonempty_nodes(&canonical_path)? {
        let canonical = TopologyCanonicalStateFile::load(config)
            .map_err(|err| TopologyRuntimeStateError::SourceGeneration(err.to_string()))?;
        canonical
            .matches_current_ingress(config)
            .map_err(|err| TopologyRuntimeStateError::SourceGeneration(err.to_string()))?
    } else {
        false
    };
    let editor_active = if !canonical_active && file_exists_with_nonempty_nodes(&editor_path)? {
        let editor = TopologyEditorStateFile::load(config)
            .map_err(|err| TopologyRuntimeStateError::SourceGeneration(err.to_string()))?;
        editor
            .matches_current_ingress(config)
            .map_err(|err| TopologyRuntimeStateError::SourceGeneration(err.to_string()))?
    } else {
        false
    };

    let mut hasher = Sha256::new();
    hasher.update(b"topology-runtime-source-generation");
    hasher.update([0]);
    if canonical_active {
        hasher.update(b"topology_source=canonical");
    } else if editor_active {
        hasher.update(b"topology_source=editor");
    } else if use_topology_import {
        hasher.update(b"topology_source=integration_import");
    } else {
        hasher.update(b"topology_source=legacy_network");
    }
    hasher.update([0xff]);

    if use_topology_import {
        hash_file_state(&mut hasher, TOPOLOGY_IMPORT_FILENAME, &topology_import_path)?;
        hash_file_state(
            &mut hasher,
            TOPOLOGY_COMPILED_SHAPING_FILENAME,
            &topology_compiled_shaping_path,
        )?;
    } else {
        hash_file_state(&mut hasher, "network.json", &network_path)?;
        hash_file_state(&mut hasher, "ShapedDevices.csv", &shaped_devices_path)?;
        hash_file_state(&mut hasher, CIRCUIT_ANCHORS_FILENAME, &circuit_anchors_path)?;
    }
    if use_topology_import {
        hash_file_state(
            &mut hasher,
            "lqos_overrides.json",
            &operator_overrides_path(config),
        )?;

        if config
            .stormguard
            .as_ref()
            .is_some_and(|stormguard| stormguard.enabled && !stormguard.dry_run)
        {
            hash_file_state(
                &mut hasher,
                "lqos_overrides.stormguard.json",
                &stormguard_overrides_path(config),
            )?;
        }

        if config.treeguard.enabled {
            hash_file_state(
                &mut hasher,
                "lqos_overrides.treeguard.json",
                &treeguard_overrides_path(config),
            )?;
        }
    }
    if canonical_active {
        hash_file_state(
            &mut hasher,
            TOPOLOGY_CANONICAL_STATE_FILENAME,
            &canonical_path,
        )?;
    }
    if editor_active {
        hash_file_state(&mut hasher, TOPOLOGY_EDITOR_STATE_FILENAME, &editor_path)?;
    }

    Ok(format!("{:x}", hasher.finalize()))
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

impl TopologyShapingInputsFile {
    /// Loads the runtime shaping-input snapshot if it exists.
    pub fn load(config: &Config) -> Result<Self, TopologyRuntimeStateError> {
        let path = topology_shaping_inputs_path(config);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Saves the runtime shaping-input snapshot.
    pub fn save(&self, config: &Config) -> Result<(), TopologyRuntimeStateError> {
        atomic_write_json(&topology_shaping_inputs_path(config), self)
    }

    /// Returns a copy normalized for semantic comparisons and stable generation hashing.
    pub fn normalized_for_compare(&self) -> Self {
        let mut normalized = self.clone();
        normalized.shaping_generation.clear();
        normalized.generated_unix = None;
        normalized.canonical_generated_unix = None;
        normalized.effective_generated_unix = None;
        normalized
    }

    /// Returns true when two shaping-input payloads are semantically equal.
    pub fn semantic_equals(&self, other: &Self) -> bool {
        self.normalized_for_compare() == other.normalized_for_compare()
    }

    /// Computes the stable shaping generation for this payload.
    pub fn compute_shaping_generation(&self) -> Result<String, TopologyRuntimeStateError> {
        let normalized = self.normalized_for_compare();
        let payload = serde_json::to_vec(&normalized)?;
        let mut hasher = Sha256::new();
        hasher.update(b"topology-runtime-shaping-generation");
        hasher.update([0xff]);
        hasher.update(payload);
        Ok(format!("{:x}", hasher.finalize()))
    }
}

impl TopologyRuntimeStatusFile {
    /// Loads the topology runtime status file if it exists.
    pub fn load(config: &Config) -> Result<Self, TopologyRuntimeStateError> {
        let path = topology_runtime_status_path(config);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Saves the topology runtime status file atomically.
    pub fn save(&self, config: &Config) -> Result<(), TopologyRuntimeStateError> {
        atomic_write_json(&topology_runtime_status_path(config), self)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CIRCUIT_ANCHORS_FILENAME, Config, TOPOLOGY_CANONICAL_STATE_FILENAME,
        TOPOLOGY_COMPILED_SHAPING_FILENAME, TOPOLOGY_EDITOR_STATE_FILENAME,
        TOPOLOGY_IMPORT_FILENAME, TOPOLOGY_RUNTIME_STATUS_FILENAME, TopologyRuntimeStatusFile,
        TopologyShapingCircuitInput, TopologyShapingInputsFile, compute_topology_source_generation,
        topology_runtime_status_path,
    };
    use crate::{
        TopologyCanonicalStateFile, TopologyEditorNode, TopologyEditorStateFile,
        TopologyQueueVisibilityPolicy,
    };
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough for tests")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
        fs::create_dir_all(&path).expect("temp directory should be creatable");
        path
    }

    fn write_required_inputs(dir: &Path) {
        fs::write(dir.join("network.json"), "{\"root\":{}}\n").expect("network.json should write");
        fs::write(
            dir.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"c1\",\"Circuit 1\",\"d1\",\"Device 1\",\"Tower 1\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");
    }

    #[test]
    fn topology_source_generation_is_stable_for_identical_inputs() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-stable");
        write_required_inputs(&lqos_directory);
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;

        let first = compute_topology_source_generation(&config)
            .expect("generation should compute for stable inputs");
        let second = compute_topology_source_generation(&config)
            .expect("generation should recompute for stable inputs");

        assert_eq!(first, second);
    }

    #[test]
    fn topology_source_generation_changes_when_source_inputs_change() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-changes");
        write_required_inputs(&lqos_directory);
        fs::write(
            lqos_directory.join("network.json"),
            serde_json::to_string_pretty(&json!({
                "Tower 1": {
                    "children": {},
                    "id": "tower-1",
                    "type": "Site",
                    "downloadBandwidthMbps": 100,
                    "uploadBandwidthMbps": 100
                }
            }))
            .expect("network json should serialize"),
        )
        .expect("network json should write");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };

        let before = compute_topology_source_generation(&config)
            .expect("generation should compute before source change");

        fs::write(
            lqos_directory.join(CIRCUIT_ANCHORS_FILENAME),
            "{\"schema_version\":1,\"source\":\"test\",\"generated_unix\":1,\"anchors\":[]}\n",
        )
        .expect("circuit_anchors.json should write");
        let after_anchors = compute_topology_source_generation(&config)
            .expect("generation should compute after anchors change");
        assert_ne!(before, after_anchors);

        fs::write(
            lqos_directory.join(TOPOLOGY_CANONICAL_STATE_FILENAME),
            concat!(
                "{\"schema_version\":1,\"source\":\"test\",\"generated_unix\":1,",
                "\"ingress_kind\":\"native_integration\",",
                "\"compatibility_network_json\":{},",
                "\"nodes\":[{\"node_id\":\"tower-1\",\"node_name\":\"Tower 1\",",
                "\"node_kind\":\"Site\",\"is_virtual\":false,",
                "\"allowed_parents\":[],\"can_move\":false,",
                "\"rate_input\":{\"intrinsic_download_mbps\":100,",
                "\"intrinsic_upload_mbps\":100,",
                "\"source\":\"attachment_max\"}}]}\n"
            ),
        )
        .expect("topology_canonical_state.json should write");
        let after_canonical = compute_topology_source_generation(&config)
            .expect("generation should compute after canonical change");
        assert_ne!(after_anchors, after_canonical);
    }

    #[test]
    fn topology_source_generation_prefers_topology_import_over_network_json() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-import");
        write_required_inputs(&lqos_directory);
        fs::write(
            lqos_directory.join(TOPOLOGY_IMPORT_FILENAME),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "source": "uisp/full2",
                "compile_mode": "ap_site",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "imported": {
                    "source": "uisp/full2",
                    "generated_unix": 1,
                    "ingress_identity": "imported-1",
                    "compatibility_network_json": {},
                    "shaped_devices": [],
                    "circuit_anchors": {
                        "schema_version": 1,
                        "source": "uisp/ap_site",
                        "generated_unix": 1,
                        "anchors": []
                    },
                    "ethernet_advisories": []
                }
            }))
            .expect("topology import should serialize"),
        )
        .expect("topology_import.json should write");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;

        let before = compute_topology_source_generation(&config)
            .expect("generation should compute before compatibility export change");
        fs::write(
            lqos_directory.join("network.json"),
            serde_json::to_string_pretty(&json!({
                "Tower 2": {
                    "children": {},
                    "id": "tower-2",
                    "type": "Site",
                    "downloadBandwidthMbps": 200,
                    "uploadBandwidthMbps": 200
                }
            }))
            .expect("network json should serialize"),
        )
        .expect("network json should write");
        let after = compute_topology_source_generation(&config)
            .expect("generation should compute after compatibility export change");

        assert_eq!(before, after);
    }

    #[test]
    fn topology_source_generation_ignores_network_json_when_integration_ingress_is_enabled() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-no-network-fallback");
        write_required_inputs(&lqos_directory);
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;

        let before = compute_topology_source_generation(&config)
            .expect("generation should compute before compatibility export change");
        fs::write(
            lqos_directory.join("network.json"),
            serde_json::to_string_pretty(&json!({
                "Tower 2": {
                    "children": {},
                    "id": "tower-2",
                    "type": "Site",
                    "downloadBandwidthMbps": 200,
                    "uploadBandwidthMbps": 200
                }
            }))
            .expect("network json should serialize"),
        )
        .expect("network json should write");
        let after = compute_topology_source_generation(&config)
            .expect("generation should compute after compatibility export change");

        assert_eq!(before, after);
    }

    #[test]
    fn topology_source_generation_tracks_override_files_for_integration_ingress() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-override-files");
        write_required_inputs(&lqos_directory);
        fs::write(
            lqos_directory.join(TOPOLOGY_IMPORT_FILENAME),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "source": "python/splynx",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "imported": {
                    "source": "python/splynx",
                    "generated_unix": 1,
                    "ingress_identity": "imported-1",
                    "compatibility_network_json": {},
                    "shaped_devices": [],
                    "circuit_anchors": {
                        "schema_version": 1,
                        "source": "python/splynx",
                        "generated_unix": 1,
                        "anchors": []
                    },
                    "ethernet_advisories": []
                }
            }))
            .expect("topology import should serialize"),
        )
        .expect("topology import should write");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        config.splynx_integration.enable_splynx = true;

        let before = compute_topology_source_generation(&config)
            .expect("generation should compute before override change");
        fs::write(
            lqos_directory.join("lqos_overrides.json"),
            serde_json::to_string_pretty(&json!({
                "persistent_devices": [],
                "circuit_adjustments": [
                    {
                        "type": "device_adjust_speed",
                        "device_id": "device-1",
                        "max_download_bandwidth": 55.0
                    }
                ],
                "network_adjustments": []
            }))
            .expect("override json should serialize"),
        )
        .expect("override file should write");
        let after = compute_topology_source_generation(&config)
            .expect("generation should compute after override change");

        assert_ne!(before, after);
    }

    #[test]
    fn topology_source_generation_tracks_compiled_shaping_for_integration_ingress() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-compiled-shaping");
        write_required_inputs(&lqos_directory);
        fs::write(
            lqos_directory.join(TOPOLOGY_IMPORT_FILENAME),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "source": "uisp/full2",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "imported": {
                    "source": "uisp/full2",
                    "generated_unix": 1,
                    "ingress_identity": "imported-1",
                    "compatibility_network_json": {},
                    "shaped_devices": [],
                    "circuit_anchors": {
                        "schema_version": 1,
                        "source": "uisp/full",
                        "generated_unix": 1,
                        "anchors": []
                    },
                    "ethernet_advisories": []
                }
            }))
            .expect("topology import should serialize"),
        )
        .expect("topology import should write");
        fs::write(
            lqos_directory.join(TOPOLOGY_COMPILED_SHAPING_FILENAME),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "source": "uisp/full",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "shaped_devices": [],
                "circuit_anchors": {
                    "schema_version": 1,
                    "source": "uisp/full",
                    "generated_unix": 1,
                    "anchors": []
                }
            }))
            .expect("compiled shaping should serialize"),
        )
        .expect("compiled shaping should write");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;

        let before = compute_topology_source_generation(&config)
            .expect("generation should compute before compiled shaping change");
        fs::write(
            lqos_directory.join(TOPOLOGY_COMPILED_SHAPING_FILENAME),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "source": "uisp/full",
                "compile_mode": "full",
                "generated_unix": 2,
                "ingress_identity": "ingress-1",
                "shaped_devices": [
                    {
                        "circuit_id": "circuit-1",
                        "circuit_name": "Circuit 1",
                        "device_id": "device-1",
                        "device_name": "Device 1",
                        "parent_node": "Tower 1",
                        "parent_node_id": "tower-1",
                        "anchor_node_id": null,
                        "mac": "",
                        "ipv4": [],
                        "ipv6": [],
                        "download_min_mbps": 10.0,
                        "upload_min_mbps": 10.0,
                        "download_max_mbps": 100.0,
                        "upload_max_mbps": 100.0,
                        "comment": "",
                        "sqm_override": null
                    }
                ],
                "circuit_anchors": {
                    "schema_version": 1,
                    "source": "uisp/full",
                    "generated_unix": 2,
                    "anchors": []
                }
            }))
            .expect("compiled shaping should serialize"),
        )
        .expect("compiled shaping should rewrite");
        let after = compute_topology_source_generation(&config)
            .expect("generation should compute after compiled shaping change");

        assert_ne!(before, after);
    }

    #[test]
    fn topology_source_generation_ignores_shaped_devices_csv_for_integration_ingress() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-no-shaped-devices");
        write_required_inputs(&lqos_directory);
        fs::write(
            lqos_directory.join(TOPOLOGY_IMPORT_FILENAME),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "source": "uisp/full2",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "imported": {
                    "source": "uisp/full2",
                    "generated_unix": 1,
                    "ingress_identity": "imported-1",
                    "compatibility_network_json": {},
                    "shaped_devices": [],
                    "circuit_anchors": {
                        "schema_version": 1,
                        "source": "uisp/full",
                        "generated_unix": 1,
                        "anchors": []
                    },
                    "ethernet_advisories": []
                }
            }))
            .expect("topology import should serialize"),
        )
        .expect("topology import should write");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;

        let before = compute_topology_source_generation(&config)
            .expect("generation should compute before ShapedDevices change");
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"c2\",\"Circuit 2\",\"d2\",\"Device 2\",\"Tower 2\",\"aa:bb:cc:dd:ee:11\",\"192.0.2.11/32\",\"\",\"20\",\"20\",\"200\",\"200\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should rewrite");
        let after = compute_topology_source_generation(&config)
            .expect("generation should compute after ShapedDevices change");

        assert_eq!(before, after);
    }

    #[test]
    fn topology_source_generation_ignores_stale_canonical_and_editor_state() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-stale-state");
        write_required_inputs(&lqos_directory);
        fs::write(
            lqos_directory.join("network.json"),
            serde_json::to_string_pretty(&json!({
                "Current Tower": {
                    "children": {},
                    "id": "uisp:device:current-tower",
                    "type": "AP",
                    "downloadBandwidthMbps": 100,
                    "uploadBandwidthMbps": 100
                }
            }))
            .expect("network json should serialize"),
        )
        .expect("network json should write");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };

        let baseline = compute_topology_source_generation(&config)
            .expect("baseline generation should compute");

        let stale_canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({
            "Old Tower": {
                "children": {},
                "id": "uisp:device:old-tower",
                "type": "AP",
                "downloadBandwidthMbps": 50,
                "uploadBandwidthMbps": 50
            }
        }));
        fs::write(
            lqos_directory.join(TOPOLOGY_CANONICAL_STATE_FILENAME),
            serde_json::to_string_pretty(&stale_canonical)
                .expect("canonical json should serialize"),
        )
        .expect("canonical state should write");
        fs::write(
            lqos_directory.join(TOPOLOGY_EDITOR_STATE_FILENAME),
            serde_json::to_string_pretty(&TopologyEditorStateFile {
                schema_version: 1,
                source: "uisp/full2".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "uisp:device:old-tower".to_string(),
                    node_name: "Old Tower".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                }],
            })
            .expect("editor json should serialize"),
        )
        .expect("editor state should write");

        let with_stale = compute_topology_source_generation(&config)
            .expect("generation should ignore stale state");
        assert_eq!(baseline, with_stale);
    }

    #[test]
    fn topology_runtime_status_round_trips() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-status-roundtrip");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        let status = TopologyRuntimeStatusFile {
            schema_version: 1,
            source_generation: "generation-1".to_string(),
            shaping_generation: "shape-1".to_string(),
            ready: true,
            generated_unix: Some(123),
            effective_state_path: "/tmp/effective.json".to_string(),
            effective_network_path: "/tmp/network.effective.json".to_string(),
            shaping_inputs_path: "/tmp/shaping_inputs.json".to_string(),
            error: None,
        };

        status
            .save(&config)
            .expect("status file should save successfully");

        let loaded =
            TopologyRuntimeStatusFile::load(&config).expect("status file should load successfully");
        assert_eq!(loaded, status);
        assert_eq!(
            topology_runtime_status_path(&config),
            lqos_directory.join(TOPOLOGY_RUNTIME_STATUS_FILENAME)
        );
    }

    #[test]
    fn shaping_generation_ignores_timestamp_only_changes() {
        let first = TopologyShapingInputsFile {
            schema_version: 1,
            shaping_generation: String::new(),
            generated_unix: Some(1),
            canonical_generated_unix: Some(10),
            effective_generated_unix: Some(20),
            warnings: vec!["warn".to_string()],
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "circuit-1".to_string(),
                effective_parent_node_name: "Tower 1".to_string(),
                effective_parent_node_id: "tower-1".to_string(),
                ..TopologyShapingCircuitInput::default()
            }],
        };
        let second = TopologyShapingInputsFile {
            generated_unix: Some(2),
            canonical_generated_unix: Some(11),
            effective_generated_unix: Some(21),
            ..first.clone()
        };

        assert!(first.semantic_equals(&second));
        assert_eq!(
            first
                .compute_shaping_generation()
                .expect("generation should compute"),
            second
                .compute_shaping_generation()
                .expect("generation should recompute")
        );
    }
}

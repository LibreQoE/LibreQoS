use crate::{
    Config, TOPOLOGY_CANONICAL_STATE_FILENAME, TUNNEL_CIRCUITS_SNAPSHOT_FILENAME,
    TopologyAttachmentHealthStatus, TopologyCanonicalStateFile,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

/// Runtime filename carrying the compiled dataplane transaction for the DPDK bridge.
pub const TOPOLOGY_DATA_PLANE_TRANSACTION_FILENAME: &str = "dataplane_transaction.json";

/// Runtime filename carrying topology publication readiness for one source generation.
pub const TOPOLOGY_RUNTIME_STATUS_FILENAME: &str = "topology_runtime_status.json";

const OPERATOR_OVERRIDES_FILENAME: &str = "lqos_overrides.json";
const STORMGUARD_OVERRIDES_FILENAME: &str = "lqos_overrides.stormguard.json";

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

/// A stable tunnel classification identity (T-arc §2.8): the outer VXLAN VNI
/// plus the significant inner VLAN id (`u16::MAX` = untagged/absent). Populated
/// by the ingestion adapter (Herbert's unit); the compiler emits a `MapTunnel`
/// for a circuit carrying one.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TunnelIdentity {
    /// 24-bit VXLAN VNI.
    pub vni: u32,
    /// Inner VLAN id; `u16::MAX` = untagged/absent.
    pub vlan: u16,
}

/// Phase C S3: the resolved per-circuit match identity carried from the
/// standard integration format to the runtime circuit input. `ip` mode has no
/// identity (prefixes ride the device rows) and so contributes no entry.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TopologyCircuitMatchIdentity {
    /// `vxlan` / `vxlan+vlan`: the tunnel classification identity.
    Tunnel(TunnelIdentity),
    /// `vlan`: the service VLAN id (the first non-steering tag).
    Vlan(u16),
    /// `mpls`: the top-of-stack label.
    Mpls(u32),
}

/// Phase C S3: per-circuit match identities keyed by stable circuit id.
pub type TopologyCircuitMatchIndex =
    std::collections::BTreeMap<String, TopologyCircuitMatchIdentity>;

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
    /// Optional tunnel classification identity (T-arc §2.8): when present, the
    /// compiler emits a `MapTunnel` for this circuit alongside its IP maps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel_identity: Option<TunnelIdentity>,
    /// Phase C S2: the `vlan` match identity (the service VLAN id). When
    /// present, the compiler emits a `MapVlan` for this circuit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan_identity: Option<u16>,
    /// Phase C S2: the `mpls` match identity (the top-of-stack label). When
    /// present, the compiler emits a `MapMpls` for this circuit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpls_identity: Option<u32>,
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
    /// Content-only digest of the shaping artifact-of-record
    /// (`TopologyShapingInputsFile::compute_shaping_generation_with_export(None)`
    /// — no export). The verify recomputes it from the ON-DISK artifact to catch
    /// a content TAMPER on a churned world (a load->edit->save preserves the
    /// `shaping_generation` field, so it alone cannot). Backward-compatible
    /// (`serde(default)`); empty on older publishes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shaping_content_generation: String,
    /// Stable generation hash of the effective network export, when ready.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effective_generation: String,
    /// Whether runtime outputs are ready for the source generation above.
    #[serde(default)]
    pub ready: bool,
    /// Unix timestamp when the status file was generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_unix: Option<u64>,
    /// Effective-state artifact path for operator inspection.
    #[serde(default)]
    pub effective_state_path: String,
    /// Operator-view artifact path for operator inspection (the current name).
    #[serde(default)]
    pub operator_view_path: String,
    /// DEPRECATED: predecessor name of [`Self::operator_view_path`], written
    /// alongside it for one release so older readers keep working. Read through
    /// [`Self::operator_view_file_path`].
    #[serde(default)]
    pub effective_network_path: String,
    /// Shaping-inputs artifact path for operator inspection.
    #[serde(default)]
    pub shaping_inputs_path: String,
    /// Error for this generation when runtime publication failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Runtime status identity used by consumers that need to know whether the
/// active shaping payload or effective-network export changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TopologyRuntimeShapingPayloadIdentity {
    /// Whether runtime outputs are ready.
    pub ready: bool,
    /// Stable generation hash of the source topology inputs.
    pub source_generation: String,
    /// Stable generation hash of the shaping payload.
    pub shaping_generation: String,
    /// Stable generation hash of the effective network export.
    pub effective_generation: String,
    /// Path to the active shaping inputs payload.
    pub shaping_inputs_path: String,
}

fn default_runtime_schema_version() -> u32 {
    1
}

fn operator_overrides_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(OPERATOR_OVERRIDES_FILENAME)
}

fn stormguard_overrides_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(STORMGUARD_OVERRIDES_FILENAME)
}

fn atomic_write_json<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), TopologyRuntimeStateError> {
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

/// Returns the path of the runtime attachment-health state file.
pub fn topology_attachment_health_state_path(config: &Config) -> PathBuf {
    config.topology_state_read_path(TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME)
}

/// Returns the path of the effective topology state file.
pub fn topology_effective_state_path(config: &Config) -> PathBuf {
    config.topology_state_read_path(TOPOLOGY_EFFECTIVE_STATE_FILENAME)
}

/// Returns the path of the effective runtime network tree file.
pub fn topology_effective_network_path(config: &Config) -> PathBuf {
    config.topology_state_read_path(TOPOLOGY_EFFECTIVE_NETWORK_FILENAME)
}

/// Returns the path of the topology import artifact file.
pub fn topology_import_path(config: &Config) -> PathBuf {
    config.topology_state_read_path(TOPOLOGY_IMPORT_FILENAME)
}

/// Returns the path of the runtime shaping-input snapshot file.
pub fn topology_shaping_inputs_path(config: &Config) -> PathBuf {
    config.shaping_state_read_path(TOPOLOGY_SHAPING_INPUTS_FILENAME)
}

/// Returns the path of the compiled dataplane transaction file.
pub fn topology_dataplane_transaction_path(config: &Config) -> PathBuf {
    config.shaping_state_read_path(TOPOLOGY_DATA_PLANE_TRANSACTION_FILENAME)
}

/// Returns the runtime shaping-inputs path only after validating the status,
/// source generation, and effective-network generation.
///
/// K1b §4.8: the shaping-payload generation is content-addressed with the
/// effective export, which this layer cannot reproduce; that custody check
/// lives in `lqos_topology::verify_published_shaping_generation` (the
/// consumers gate on it).
///
/// Side effects: reads `topology_runtime_status.json` and the candidate
/// `shaping_inputs.json` and `network.effective.json`.
pub fn validated_runtime_shaping_inputs_path(
    config: &Config,
) -> Result<Option<PathBuf>, TopologyRuntimeStateError> {
    let current_generation = compute_topology_source_generation(config)?;
    let status = TopologyRuntimeStatusFile::load(config)?;
    Ok(
        load_active_runtime_shaping_inputs_for_status(&status, &current_generation)?
            .map(|(path, _)| path),
    )
}

/// Deprecated compatibility wrapper for older callers.
///
/// Side effects: validates the same files as [`validated_runtime_shaping_inputs_path`].
#[deprecated(
    note = "use validated_runtime_shaping_inputs_path; this function validates runtime artifacts before returning a path"
)]
pub fn active_runtime_shaping_inputs_path(
    config: &Config,
) -> Result<Option<PathBuf>, TopologyRuntimeStateError> {
    validated_runtime_shaping_inputs_path(config)
}

/// Deprecated compatibility wrapper for older callers that already loaded status.
///
/// This only checks the readiness metadata in `status`; it does not validate the
/// on-disk shaping or effective-network artifacts.
#[deprecated(
    note = "use validated_runtime_shaping_inputs_path or load_active_runtime_shaping_inputs for artifact validation"
)]
pub fn active_runtime_shaping_inputs_path_from_status(
    status: &TopologyRuntimeStatusFile,
    current_generation: &str,
) -> Option<PathBuf> {
    ready_runtime_shaping_inputs_path_from_status(status, current_generation)
}

fn ready_runtime_shaping_inputs_path_from_status(
    status: &TopologyRuntimeStatusFile,
    current_generation: &str,
) -> Option<PathBuf> {
    if !status.ready {
        return None;
    }
    if status.shaping_generation.trim().is_empty() {
        return None;
    }
    if status.source_generation.trim() != current_generation {
        return None;
    }
    let path = status.shaping_inputs_path.trim();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// Loads a runtime shaping-inputs snapshot from an explicit path.
///
/// Side effects: reads the supplied file.
pub fn load_shaping_inputs_at_path(
    path: &Path,
) -> Result<TopologyShapingInputsFile, TopologyRuntimeStateError> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

/// Computes the semantic effective-network generation for one runtime effective network file.
///
/// Side effects: reads the supplied effective-network file.
pub fn compute_effective_network_file_generation(
    path: &Path,
) -> Result<String, TopologyRuntimeStateError> {
    let raw = std::fs::read_to_string(path)?;
    let effective_network = serde_json::from_str::<Value>(&raw)?;
    compute_effective_network_generation(&effective_network)
}

fn effective_network_generation_matches_status(
    status: &TopologyRuntimeStatusFile,
) -> Result<bool, TopologyRuntimeStateError> {
    let path = status.operator_view_file_path();
    let expected_generation = status.effective_generation.trim();
    if path.is_empty() {
        return Ok(false);
    }
    let path = Path::new(path);
    if !path.exists() {
        return Ok(false);
    }
    if expected_generation.is_empty() {
        return Ok(false);
    }
    let computed_generation = compute_effective_network_file_generation(path)?;
    Ok(computed_generation == expected_generation)
}

fn load_active_runtime_shaping_inputs_for_status(
    status: &TopologyRuntimeStatusFile,
    current_generation: &str,
) -> Result<Option<(PathBuf, TopologyShapingInputsFile)>, TopologyRuntimeStateError> {
    let Some(path) = ready_runtime_shaping_inputs_path_from_status(status, current_generation)
    else {
        return Ok(None);
    };
    if !effective_network_generation_matches_status(status)? {
        return Ok(None);
    }
    let shaping_inputs = load_shaping_inputs_at_path(&path)?;
    // K1b §4.8: the published generation content-addresses the post-fold
    // effective export, which this layer cannot reproduce. The
    // generation-vs-status comparison therefore lives in the topology custody
    // check (`lqos_topology::verify_published_shaping_generation`); this loader
    // returns the published inputs and the consumers gate on that verify.
    Ok(Some((path, shaping_inputs)))
}

/// Loads the currently active runtime shaping-inputs file when the topology
/// runtime has published one.
pub fn load_active_runtime_shaping_inputs(
    config: &Config,
) -> Result<Option<TopologyShapingInputsFile>, TopologyRuntimeStateError> {
    let current_generation = compute_topology_source_generation(config)?;
    let status = TopologyRuntimeStatusFile::load(config)?;
    Ok(
        load_active_runtime_shaping_inputs_for_status(&status, &current_generation)?
            .map(|(_, shaping_inputs)| shaping_inputs),
    )
}

/// Loads the active runtime shaping-inputs file from an already-loaded status file.
pub fn load_active_runtime_shaping_inputs_from_status(
    config: &Config,
    status: &TopologyRuntimeStatusFile,
) -> Result<Option<TopologyShapingInputsFile>, TopologyRuntimeStateError> {
    let current_generation = compute_topology_source_generation(config)?;
    Ok(
        load_active_runtime_shaping_inputs_for_status(status, &current_generation)?
            .map(|(_, shaping_inputs)| shaping_inputs),
    )
}

/// Returns the path of the topology runtime readiness status file.
pub fn topology_runtime_status_path(config: &Config) -> PathBuf {
    config.topology_state_read_path(TOPOLOGY_RUNTIME_STATUS_FILENAME)
}

/// Returns the path of the integration compiled-shaping artifact.
pub fn topology_compiled_shaping_path(config: &Config) -> PathBuf {
    config.shaping_state_read_path(TOPOLOGY_COMPILED_SHAPING_FILENAME)
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
    let canonical_path = config.topology_state_read_path(TOPOLOGY_CANONICAL_STATE_FILENAME);
    let topology_import_path = config.topology_state_read_path(TOPOLOGY_IMPORT_FILENAME);
    let topology_compiled_shaping_path =
        config.shaping_state_read_path(TOPOLOGY_COMPILED_SHAPING_FILENAME);
    let tunnel_circuits_snapshot_path =
        config.topology_state_read_path(TUNNEL_CIRCUITS_SNAPSHOT_FILENAME);

    // Phase C S4: ONE ingest contract — the compiled import is the source for
    // every world (the producer materializes the manual pair into it), so the
    // legacy pair hashes retired with the mode.
    let canonical_active = if file_exists_with_nonempty_nodes(&canonical_path)? {
        let canonical = TopologyCanonicalStateFile::load(config)
            .map_err(|err| TopologyRuntimeStateError::SourceGeneration(err.to_string()))?;
        canonical
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
    } else {
        hasher.update(b"topology_source=integration_import");
    }
    hasher.update([0xff]);

    hash_file_state(&mut hasher, TOPOLOGY_IMPORT_FILENAME, &topology_import_path)?;
    hash_file_state(
        &mut hasher,
        TOPOLOGY_COMPILED_SHAPING_FILENAME,
        &topology_compiled_shaping_path,
    )?;
    // T-arc: the validated tunnel-circuits snapshot is a first-class shaping
    // input in BOTH ingress modes — a rate-only reprovision must move the
    // generation even when the canonical/site set is unchanged.
    hash_file_state(
        &mut hasher,
        TUNNEL_CIRCUITS_SNAPSHOT_FILENAME,
        &tunnel_circuits_snapshot_path,
    )?;
    // K1 §1.1 (the #12-HIGH trigger): the dynamic-circuit store is a
    // first-class publish-trigger input. A RADIUS upsert/expiry write must
    // move the generation so the runtime republishes, the fold reaches E1,
    // and the broker fires. Without this arm the K1 causal chain is dead.
    hash_file_state(
        &mut hasher,
        "dynamic_circuits.json",
        &config.legacy_runtime_file_path("dynamic_circuits.json"),
    )?;
    // Dynamic-ranges H1: the ranges CONFIG (lqos.toml) is a first-class
    // publish-trigger input. Without this arm a range add/rate change never
    // moves the generation, so the unconfirmed leaf + prefix Map never
    // republish. Hash the whole config section (deterministic serde).
    if let Some(dynamic_circuits) = config.dynamic_circuits.as_ref() {
        hasher.update(b"dynamic_circuits_config");
        hasher.update([0]);
        let payload = serde_json::to_vec(dynamic_circuits)
            .map_err(|err| TopologyRuntimeStateError::SourceGeneration(err.to_string()))?;
        hasher.update(payload);
        hasher.update([0xff]);
    }
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

    if canonical_active {
        hash_file_state(
            &mut hasher,
            TOPOLOGY_CANONICAL_STATE_FILENAME,
            &canonical_path,
        )?;
    }
    Ok(crate::hex_encoding::encode_hex_lower(hasher.finalize()))
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
        atomic_write_json(
            &config.topology_state_file_path(TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME),
            self,
        )
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
        atomic_write_json(
            &config.topology_state_file_path(TOPOLOGY_EFFECTIVE_STATE_FILENAME),
            self,
        )
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
        atomic_write_json(
            &config.shaping_state_file_path(TOPOLOGY_SHAPING_INPUTS_FILENAME),
            self,
        )
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

    /// K1b stormguard card §4.8: the publish generation content-addresses the
    /// post-fold effective export as well as the shaping inputs. A site-rate
    /// clamp changes the effective tree (inner ops) but not the shaping inputs,
    /// so without the export a clamp would mint the base generation and the
    /// bridge publish could not be correlated. Deterministic for deterministic
    /// inputs (the rung-1 re-emit gate).
    ///
    /// This is the ONE hashing door: the former plain
    /// `compute_shaping_generation()` was byte-identical to
    /// `with_export(None)` and is gone (gen_simple §1).
    pub fn compute_shaping_generation_with_export(
        &self,
        effective_export: Option<&Value>,
    ) -> Result<String, TopologyRuntimeStateError> {
        let normalized = self.normalized_for_compare();
        let payload = serde_json::to_vec(&normalized)?;
        let mut hasher = Sha256::new();
        hasher.update(b"topology-runtime-shaping-generation");
        hasher.update([0xff]);
        hasher.update(payload);
        if let Some(export) = effective_export {
            let normalized_export = normalized_json_value_for_generation(export);
            let export_payload = serde_json::to_vec(&normalized_export)?;
            hasher.update([0xfe]);
            hasher.update(export_payload);
        }
        Ok(crate::hex_encoding::encode_hex_lower(hasher.finalize()))
    }
}

fn normalized_json_value_for_generation(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(key, _)| *key);
            let mut normalized = serde_json::Map::with_capacity(entries.len());
            for (key, child) in entries {
                normalized.insert(key.clone(), normalized_json_value_for_generation(child));
            }
            Value::Object(normalized)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(normalized_json_value_for_generation)
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// Computes the stable effective-network generation for one runtime export payload.
pub fn compute_effective_network_generation(
    effective_network: &Value,
) -> Result<String, TopologyRuntimeStateError> {
    let normalized = normalized_json_value_for_generation(effective_network);
    let payload = serde_json::to_vec(&normalized)?;
    let mut hasher = Sha256::new();
    hasher.update(b"topology-runtime-effective-generation");
    hasher.update([0xff]);
    hasher.update(payload);
    Ok(crate::hex_encoding::encode_hex_lower(hasher.finalize()))
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
        atomic_write_json(
            &config.topology_state_file_path(TOPOLOGY_RUNTIME_STATUS_FILENAME),
            self,
        )
    }

    /// Returns the operator-view artifact path, preferring the current
    /// `operator_view_path` field and falling back to the deprecated
    /// `effective_network_path` so status files written before the rename keep
    /// resolving.
    pub fn operator_view_file_path(&self) -> &str {
        let current = self.operator_view_path.trim();
        if current.is_empty() {
            self.effective_network_path.trim()
        } else {
            current
        }
    }

    /// Returns true when runtime status is equal except for the generated timestamp.
    pub fn semantic_equals_for_publish(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.source_generation == other.source_generation
            && self.shaping_generation == other.shaping_generation
            && self.shaping_content_generation == other.shaping_content_generation
            && self.effective_generation == other.effective_generation
            && self.ready == other.ready
            && self.effective_state_path == other.effective_state_path
            && self.operator_view_path == other.operator_view_path
            && self.effective_network_path == other.effective_network_path
            && self.shaping_inputs_path == other.shaping_inputs_path
            && self.error == other.error
    }

    /// Returns the shaping payload identity used by cache consumers.
    pub fn shaping_payload_identity(&self) -> TopologyRuntimeShapingPayloadIdentity {
        TopologyRuntimeShapingPayloadIdentity {
            ready: self.ready,
            source_generation: self.source_generation.clone(),
            shaping_generation: self.shaping_generation.clone(),
            effective_generation: self.effective_generation.clone(),
            shaping_inputs_path: self.shaping_inputs_path.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Config, TOPOLOGY_CANONICAL_STATE_FILENAME, TOPOLOGY_COMPILED_SHAPING_FILENAME,
        TOPOLOGY_IMPORT_FILENAME,
        TOPOLOGY_RUNTIME_STATUS_FILENAME, TopologyRuntimeStatusFile,
        TopologyShapingCircuitInput, TopologyShapingInputsFile,
        compute_effective_network_file_generation, compute_effective_network_generation,
        compute_topology_source_generation,
        load_active_runtime_shaping_inputs, topology_runtime_status_path,
        validated_runtime_shaping_inputs_path,
    };
    use crate::{
        TOPOLOGY_EDITOR_STATE_FILENAME, TopologyCanonicalStateFile, TopologyEditorNode,
        TopologyEditorStateFile,
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

    fn write_effective_network(config: &Config, raw: &str) -> (PathBuf, String) {
        let path = super::topology_effective_network_path(config);
        fs::create_dir_all(
            path.parent()
                .expect("effective network path should have a parent"),
        )
        .expect("topology state dir should exist");
        fs::write(&path, raw).expect("effective network should write");
        let generation = compute_effective_network_file_generation(&path)
            .expect("effective generation should compute");
        (path, generation)
    }

    #[test]
    fn topology_source_generation_is_stable_for_identical_inputs() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-stable");
        write_required_inputs(&lqos_directory);
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
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
    fn source_generation_moves_when_dynamic_ranges_change() {
        // Dynamic-ranges H1: the ranges CONFIG is a first-class publish-trigger
        // input — without this arm a range add/rate change never republishes.
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-ranges");
        write_required_inputs(&lqos_directory);
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;
        let range = crate::DynamicCircuitRangeRule {
            name: "pool-1".to_string(),
            ip_range: "10.50.0.0/24".parse().expect("cidr"),
            download_min_mbps: 1.0,
            upload_min_mbps: 1.0,
            download_max_mbps: 5.0,
            upload_max_mbps: 5.0,
            attach_to: "site-a".to_string(),
        };
        config.dynamic_circuits = Some(crate::DynamicCircuitsConfig {
            enabled: true,
            enable_unknown_ip_promotion: true,
            ranges: vec![range],
            ..Default::default()
        });
        let before =
            compute_topology_source_generation(&config).expect("generation should compute");
        config
            .dynamic_circuits
            .as_mut()
            .expect("dynamic_circuits")
            .ranges[0]
            .download_max_mbps = 9.0;
        let after =
            compute_topology_source_generation(&config).expect("generation should compute");
        assert_ne!(
            before, after,
            "a ranges-config change must move the source generation"
        );
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
            state_directory: None,
            ..Config::default()
        };

        let before = compute_topology_source_generation(&config)
            .expect("generation should compute before source change");

        // Phase C S4: the compiled import is the ONE source — changing it
        // moves the generation (the legacy pair + anchors hashes retired).
        let import_path = config.topology_state_file_path(TOPOLOGY_IMPORT_FILENAME);
        fs::create_dir_all(import_path.parent().expect("import path should have a parent"))
            .expect("topology state dir should exist");
        fs::write(
            import_path,
            "{\"schema_version\":1,\"source\":\"test\",\"compile_mode\":\"full\",\"imported\":{\"source\":\"test\",\"compatibility_network_json\":{},\"shaped_devices\":[],\"circuit_anchors\":{\"schema_version\":1,\"source\":\"\",\"anchors\":[]},\"ethernet_advisories\":[],\"circuit_matches\":{}}}\n",
        )
        .expect("topology_import.json should write");
        let after_import = compute_topology_source_generation(&config)
            .expect("generation should compute after import change");
        assert_ne!(before, after_import);

        fs::write(
            config.topology_state_file_path(TOPOLOGY_CANONICAL_STATE_FILENAME),
            concat!(
                "{\"schema_version\":1,\"source\":\"test\",\"generated_unix\":1,",
                "\"ingress_kind\":\"native_integration\",",
                "\"compatibility_network_json\":{},",
                "\"nodes\":[{\"node_id\":\"tower-1\",\"node_name\":\"Tower 1\",",
                "\"node_kind\":\"Site\",",
                "\"allowed_parents\":[],\"can_move\":false,",
                "\"rate_input\":{\"intrinsic_download_mbps\":100,",
                "\"intrinsic_upload_mbps\":100,",
                "\"source\":\"attachment_max\"}}]}\n"
            ),
        )
        .expect("topology_canonical_state.json should write");
        let after_canonical = compute_topology_source_generation(&config)
            .expect("generation should compute after canonical change");
        // An ACTIVE canonical state is itself a hashed source.
        assert_ne!(after_import, after_canonical);
    }

    #[test]
    fn topology_source_generation_tracks_manual_override_files() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-manual-overrides");
        write_required_inputs(&lqos_directory);
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };

        let before = compute_topology_source_generation(&config)
            .expect("generation should compute before override change");
        fs::write(
            lqos_directory.join("lqos_overrides.json"),
            r#"{"schema_version":1,"network_adjustments":[{"node_id":"tower-1","download_mbps":100}]}"#,
        )
        .expect("operator overrides should write");
        let after = compute_topology_source_generation(&config)
            .expect("generation should compute after override change");

        assert_ne!(before, after);
    }

    #[test]
    fn topology_source_generation_tracks_dynamic_circuit_store_changes() {
        // K1 §1.1 #12-HIGH: a dynamic-store write must flip the publish
        // generation — the fold alone is inert without this trigger.
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-dynamic");
        write_required_inputs(&lqos_directory);
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };

        let before = compute_topology_source_generation(&config)
            .expect("generation should compute before store change");
        fs::write(
            lqos_directory.join("dynamic_circuits.json"),
            r#"{"schema_version":1,"circuits":[]}"#,
        )
        .expect("dynamic store should write");
        let after = compute_topology_source_generation(&config)
            .expect("generation should compute after store change");

        assert_ne!(
            before, after,
            "a dynamic-store write must move the source generation"
        );
    }

    #[test]
    fn topology_source_generation_prefers_topology_import_over_network_json() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-generation-import");
        write_required_inputs(&lqos_directory);
        let state_topology_dir = lqos_directory.join("state/topology");
        fs::create_dir_all(&state_topology_dir).expect("topology state dir should exist");
        fs::write(
            state_topology_dir.join(TOPOLOGY_IMPORT_FILENAME),
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
            state_directory: None,
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
            state_directory: None,
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
        let state_topology_dir = lqos_directory.join("state/topology");
        fs::create_dir_all(&state_topology_dir).expect("topology state dir should exist");
        fs::write(
            state_topology_dir.join(TOPOLOGY_IMPORT_FILENAME),
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
            state_directory: None,
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
        let state_topology_dir = lqos_directory.join("state/topology");
        let state_shaping_dir = lqos_directory.join("state/shaping");
        fs::create_dir_all(&state_topology_dir).expect("topology state dir should exist");
        fs::create_dir_all(&state_shaping_dir).expect("shaping state dir should exist");
        fs::write(
            state_topology_dir.join(TOPOLOGY_IMPORT_FILENAME),
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
            state_shaping_dir.join(TOPOLOGY_COMPILED_SHAPING_FILENAME),
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
            state_directory: None,
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;

        let before = compute_topology_source_generation(&config)
            .expect("generation should compute before compiled shaping change");
        fs::write(
            state_shaping_dir.join(TOPOLOGY_COMPILED_SHAPING_FILENAME),
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
        let state_topology_dir = lqos_directory.join("state/topology");
        fs::create_dir_all(&state_topology_dir).expect("topology state dir should exist");
        fs::write(
            state_topology_dir.join(TOPOLOGY_IMPORT_FILENAME),
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
            state_directory: None,
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
        // Phase C S4: the materialized import is the ONE source; a stale
        // canonical (ingress-identity mismatch) must not move the generation.
        let import_dir = lqos_directory.join("state/topology");
        fs::create_dir_all(&import_dir).expect("topology state dir should exist");
        fs::write(
            import_dir.join(TOPOLOGY_IMPORT_FILENAME),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "source": "uisp/full2",
                "compile_mode": "ap_site",
                "generated_unix": 1,
                "ingress_identity": "import-identity",
                "imported": {
                    "source": "uisp/full2",
                    "generated_unix": 1,
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
        .expect("topology import should write");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
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
        let state_topology_dir = lqos_directory.join("state/topology");
        fs::create_dir_all(&state_topology_dir).expect("topology state dir should exist");
        fs::write(
            state_topology_dir.join(TOPOLOGY_CANONICAL_STATE_FILENAME),
            serde_json::to_string_pretty(&stale_canonical)
                .expect("canonical json should serialize"),
        )
        .expect("canonical state should write");
        fs::write(
            state_topology_dir.join(TOPOLOGY_EDITOR_STATE_FILENAME),
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
            state_directory: None,
            ..Config::default()
        };
        let status = TopologyRuntimeStatusFile {
            schema_version: 1,
            source_generation: "generation-1".to_string(),
            shaping_generation: "shape-1".to_string(),
            shaping_content_generation: "content-1".to_string(),
            effective_generation: "effective-1".to_string(),
            ready: true,
            generated_unix: Some(123),
            effective_state_path: "/tmp/effective.json".to_string(),
            operator_view_path: "/tmp/network.effective.json".to_string(),
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
            lqos_directory
                .join("state")
                .join("topology")
                .join(TOPOLOGY_RUNTIME_STATUS_FILENAME)
        );
    }

    #[test]
    fn topology_runtime_status_publish_compare_ignores_generated_unix() {
        let first = TopologyRuntimeStatusFile {
            schema_version: 1,
            source_generation: "source-1".to_string(),
            shaping_generation: "shape-1".to_string(),
            shaping_content_generation: "content-1".to_string(),
            effective_generation: "effective-1".to_string(),
            ready: true,
            generated_unix: Some(1),
            effective_state_path: "/tmp/effective.json".to_string(),
            operator_view_path: "/tmp/network.effective.json".to_string(),
            effective_network_path: "/tmp/network.effective.json".to_string(),
            shaping_inputs_path: "/tmp/shaping_inputs.json".to_string(),
            error: None,
        };
        let second = TopologyRuntimeStatusFile {
            generated_unix: Some(2),
            ..first.clone()
        };
        let changed = TopologyRuntimeStatusFile {
            shaping_generation: "shape-2".to_string(),
            shaping_content_generation: "content-1".to_string(),
            ..first.clone()
        };
        let changed_effective = TopologyRuntimeStatusFile {
            effective_generation: "effective-2".to_string(),
            ..first.clone()
        };

        assert!(first.semantic_equals_for_publish(&second));
        assert!(!first.semantic_equals_for_publish(&changed));
        assert!(!first.semantic_equals_for_publish(&changed_effective));
        assert_eq!(
            first.shaping_payload_identity(),
            second.shaping_payload_identity()
        );
        assert_ne!(
            first.shaping_payload_identity(),
            changed.shaping_payload_identity()
        );
        assert_ne!(
            first.shaping_payload_identity(),
            changed_effective.shaping_payload_identity()
        );
    }

    #[test]
    fn active_shaping_inputs_custody_check_moved_to_topology_verify() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-shaping-generation-stale");
        write_required_inputs(&lqos_directory);
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        let source_generation =
            compute_topology_source_generation(&config).expect("source generation should compute");
        let mut shaping_inputs = TopologyShapingInputsFile {
            schema_version: 1,
            generated_unix: Some(1),
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "circuit-1".to_string(),
                effective_parent_node_name: "Tower 1".to_string(),
                effective_parent_node_id: "tower-1".to_string(),
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        };
        shaping_inputs.shaping_generation = shaping_inputs
            .compute_shaping_generation_with_export(None)
            .expect("shaping generation should compute");
        shaping_inputs
            .save(&config)
            .expect("shaping inputs should save");
        let shaping_path = super::topology_shaping_inputs_path(&config);
        let (effective_path, effective_generation) = write_effective_network(&config, "{}\n");
        TopologyRuntimeStatusFile {
            schema_version: 1,
            source_generation,
            shaping_generation: shaping_inputs.shaping_generation.clone(),
            effective_generation,
            ready: true,
            effective_network_path: effective_path.to_string_lossy().to_string(),
            shaping_inputs_path: shaping_path.to_string_lossy().to_string(),
            ..TopologyRuntimeStatusFile::default()
        }
        .save(&config)
        .expect("status should save");

        assert!(
            validated_runtime_shaping_inputs_path(&config)
                .expect("active path check should succeed")
                .is_some()
        );
        assert!(
            load_active_runtime_shaping_inputs(&config)
                .expect("active shaping inputs should load")
                .is_some()
        );

        // K1b §4.8 (card v2 i-a): the generation-vs-status comparison MOVED to
        // the topology custody verify
        // (`lqos_topology::verify_published_shaping_generation`), which also
        // checks the on-disk artifact against the fresh mint. This loader now
        // returns the published inputs; the payload-tamper teeth live in that
        // verify's oracle set (lqos_topology group_08). Assert the custody move.
        let mut changed = shaping_inputs;
        changed.circuits[0].effective_parent_node_name = "Tower 2".to_string();
        changed
            .save(&config)
            .expect("changed shaping inputs should save");

        assert!(
            validated_runtime_shaping_inputs_path(&config)
                .expect("active path check should succeed")
                .is_some(),
            "the loader no longer owns the generation custody check"
        );
        assert!(
            load_active_runtime_shaping_inputs(&config)
                .expect("active shaping inputs should load")
                .is_some(),
            "the loader no longer owns the generation custody check"
        );
    }

    #[test]
    fn active_shaping_inputs_rejects_status_when_effective_generation_changed() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-effective-generation-stale");
        write_required_inputs(&lqos_directory);
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        let source_generation =
            compute_topology_source_generation(&config).expect("source generation should compute");
        let mut shaping_inputs = TopologyShapingInputsFile {
            schema_version: 1,
            generated_unix: Some(1),
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "circuit-1".to_string(),
                effective_parent_node_name: "Tower 1".to_string(),
                effective_parent_node_id: "tower-1".to_string(),
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        };
        shaping_inputs.shaping_generation = shaping_inputs
            .compute_shaping_generation_with_export(None)
            .expect("shaping generation should compute");
        shaping_inputs
            .save(&config)
            .expect("shaping inputs should save");
        let (effective_path, effective_generation) = write_effective_network(&config, "{}\n");
        TopologyRuntimeStatusFile {
            schema_version: 1,
            source_generation,
            shaping_generation: shaping_inputs.shaping_generation.clone(),
            effective_generation,
            ready: true,
            effective_network_path: effective_path.to_string_lossy().to_string(),
            shaping_inputs_path: super::topology_shaping_inputs_path(&config)
                .to_string_lossy()
                .to_string(),
            ..TopologyRuntimeStatusFile::default()
        }
        .save(&config)
        .expect("status should save");

        assert!(
            load_active_runtime_shaping_inputs(&config)
                .expect("active shaping inputs should load")
                .is_some()
        );

        fs::write(&effective_path, "{\"Tower 2\":{\"children\":{}}}\n")
            .expect("effective network should rewrite");
        assert!(
            load_active_runtime_shaping_inputs(&config)
                .expect("active shaping inputs should check")
                .is_none()
        );
    }

    #[test]
    fn active_shaping_inputs_rejects_status_without_effective_network_metadata() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-effective-generation-missing");
        write_required_inputs(&lqos_directory);
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        let source_generation =
            compute_topology_source_generation(&config).expect("source generation should compute");
        let mut shaping_inputs = TopologyShapingInputsFile {
            schema_version: 1,
            generated_unix: Some(1),
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "circuit-1".to_string(),
                effective_parent_node_name: "Tower 1".to_string(),
                effective_parent_node_id: "tower-1".to_string(),
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        };
        shaping_inputs.shaping_generation = shaping_inputs
            .compute_shaping_generation_with_export(None)
            .expect("shaping generation should compute");
        shaping_inputs
            .save(&config)
            .expect("shaping inputs should save");
        TopologyRuntimeStatusFile {
            schema_version: 1,
            source_generation,
            shaping_generation: shaping_inputs.shaping_generation,
            ready: true,
            shaping_inputs_path: super::topology_shaping_inputs_path(&config)
                .to_string_lossy()
                .to_string(),
            ..TopologyRuntimeStatusFile::default()
        }
        .save(&config)
        .expect("status should save");

        assert!(
            load_active_runtime_shaping_inputs(&config)
                .expect("active shaping inputs should check")
                .is_none()
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
                .compute_shaping_generation_with_export(None)
                .expect("generation should compute"),
            second
                .compute_shaping_generation_with_export(None)
                .expect("generation should recompute")
        );
    }

    #[test]
    fn effective_generation_changes_when_effective_rates_change() {
        let first = serde_json::json!({
            "Root": {
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000,
                "children": {
                    "AP A": {
                        "id": "ap-a",
                        "downloadBandwidthMbps": 204,
                        "uploadBandwidthMbps": 58,
                        "children": {}
                    }
                }
            }
        });
        let second = serde_json::json!({
            "Root": {
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000,
                "children": {
                    "AP A": {
                        "id": "ap-a",
                        "downloadBandwidthMbps": 218,
                        "uploadBandwidthMbps": 57,
                        "children": {}
                    }
                }
            }
        });

        let first_generation =
            compute_effective_network_generation(&first).expect("generation should compute");
        let second_generation =
            compute_effective_network_generation(&second).expect("generation should compute");
        assert_ne!(first_generation, second_generation);
    }

    #[test]
    fn shaping_generation_content_addresses_the_effective_export() {
        // K1b stormguard card §4.8: a site-rate clamp changes the effective
        // export (inner ops) but NOT the shaping inputs, so the generation must
        // content-address the export or a clamp would mint the base generation
        // and the bridge publish could not be correlated.
        let inputs = TopologyShapingInputsFile {
            schema_version: 1,
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "circuit-1".to_string(),
                effective_parent_node_name: "Tower 1".to_string(),
                effective_parent_node_id: "tower-1".to_string(),
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        };
        let export_a = serde_json::json!({"Tower 1": {"downloadBandwidthMbps": 1000}});
        let export_b = serde_json::json!({"Tower 1": {"downloadBandwidthMbps": 880}});

        let base = inputs
            .compute_shaping_generation_with_export(None)
            .expect("base generation");
        let with_a = inputs
            .compute_shaping_generation_with_export(Some(&export_a))
            .expect("export-a generation");
        let with_b = inputs
            .compute_shaping_generation_with_export(Some(&export_b))
            .expect("export-b generation");
        let with_a_again = inputs
            .compute_shaping_generation_with_export(Some(&export_a))
            .expect("export-a generation again");

        assert_ne!(
            with_a, with_b,
            "a changed effective export must change the generation"
        );
        assert_ne!(base, with_a, "adding the export changes the generation");
        assert_eq!(with_a, with_a_again, "deterministic for identical inputs");
    }

    #[test]
    fn effective_generation_ignores_object_key_order_only_changes() {
        let first = serde_json::json!({
            "Root": {
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000,
                "children": {
                    "AP A": {
                        "id": "ap-a",
                        "downloadBandwidthMbps": 204,
                        "uploadBandwidthMbps": 58,
                        "children": {}
                    }
                }
            }
        });
        let second = serde_json::json!({
            "Root": {
                "children": {
                    "AP A": {
                        "children": {},
                        "uploadBandwidthMbps": 58,
                        "downloadBandwidthMbps": 204,
                        "id": "ap-a"
                    }
                },
                "uploadBandwidthMbps": 1000,
                "downloadBandwidthMbps": 1000
            }
        });

        assert_eq!(
            compute_effective_network_generation(&first).expect("generation should compute"),
            compute_effective_network_generation(&second).expect("generation should compute"),
        );
    }
}

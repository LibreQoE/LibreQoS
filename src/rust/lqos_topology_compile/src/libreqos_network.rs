//! Reader + strict validation for the standard custom-integration format
//! `libreqos_network.json` (Phase C S3; `docs/libreqos_network_format_card.md`).
//!
//! SINGLE-version fail-loud (the `tunnel_circuits` precedent): exactly one
//! `schema_version` is accepted; a breaking change ships a new version and a
//! one-shot migration tool — never a dual reader. Strict: unknown fields are
//! rejected, and every error NAMES its row (e.g. `circuits[7].plan`). No
//! tolerance rules (that contract belongs to the legacy CSV).
//!
//! This module is the READER only (S3a): it produces the validated typed
//! representation. Mapping onto the internal payload (the canonical nodes and
//! the shaping inputs) is S3b.

use crate::ImportedTopologyBundle;
use lqos_config::{
    CircuitAnchor, CircuitAnchorsFile, Config, ConfigShapedDevices, ShapedDevice,
    TopologyCanonicalIngressKind, TopologyCanonicalNode, TopologyCanonicalRateInput,
    TopologyCanonicalStateFile, TopologyCircuitMatchIdentity, TopologyCircuitMatchIndex,
    TunnelIdentity,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

/// The filename of the standard custom-integration format.
pub const LIBREQOS_NETWORK_FILENAME: &str = "libreqos_network.json";

/// Returns the watched-file path under the configured runtime directory.
pub fn libreqos_network_input_path(config: &Config) -> PathBuf {
    config.legacy_runtime_file_path(LIBREQOS_NETWORK_FILENAME)
}

/// The single supported `schema_version`.
pub const LIBREQOS_NETWORK_SCHEMA_VERSION: u32 = 2;

/// Upper bound on the raw file size (the `tunnel_circuits` 8 MiB precedent).
pub const LIBREQOS_NETWORK_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Maximum MPLS label value (20 bits).
pub const MPLS_LABEL_MAX: u32 = 1_048_575;

/// Maximum VXLAN VNI value (24 bits).
pub const VXLAN_VNI_MAX: u32 = 16_777_214;

/// Maximum 802.1Q VLAN id.
pub const VLAN_ID_MAX: u16 = 4094;

/// Why a `libreqos_network.json` was rejected.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LibreQosNetworkError {
    /// The file exceeds [`LIBREQOS_NETWORK_MAX_BYTES`].
    #[error("libreqos_network.json is {found} bytes, over the {max}-byte bound")]
    TooLarge {
        /// The observed size.
        found: usize,
        /// The bound.
        max: usize,
    },
    /// Malformed JSON (serde names the field/value + position).
    #[error("libreqos_network.json did not parse: {0}")]
    Parse(String),
    /// An unsupported `schema_version`.
    #[error(
        "libreqos_network.json schema_version {found} is not supported (this reader accepts {supported})"
    )]
    UnsupportedSchemaVersion {
        /// The version found.
        found: u32,
        /// The version supported.
        supported: u32,
    },
    /// A semantic validation failure; the path names the row.
    #[error("{path}: {reason}")]
    Validation {
        /// The JSON path of the offending row (e.g. `circuits[7].plan`).
        path: String,
        /// The reason.
        reason: String,
    },
}

/// A topology node's kind (the closed set).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NetworkNodeKind {
    /// A site.
    #[default]
    Site,
    /// An intermediate node.
    Node,
    /// An access point.
    Ap,
}

/// The closed match-mode enum (Phase C S2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum NetworkMatchMode {
    /// Per-circuit prefix lists.
    #[default]
    #[serde(rename = "ip")]
    Ip,
    /// A VNI per circuit (the untagged `u16::MAX` convention).
    #[serde(rename = "vxlan")]
    Vxlan,
    /// `(VNI, VLAN)` pairs (today's `MapTunnel`).
    #[serde(rename = "vxlan+vlan")]
    VxlanVlan,
    /// A service VLAN id per circuit (the first non-steering tag).
    #[serde(rename = "vlan")]
    Vlan,
    /// A top-of-stack MPLS label per circuit (v1).
    #[serde(rename = "mpls")]
    Mpls,
}

/// A commit/ceiling plan as written in the file: decimal megabits per second.
///
/// The wire contract is the parsed `f64` value. Conversion to bits per second
/// is a separate, checked step ([`NetworkRatePlan::to_bps`]); the reader is the
/// only bps surface.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NetworkRatePlan {
    /// Committed downstream megabits per second.
    pub commit_down_mbps: f64,
    /// Committed upstream megabits per second.
    pub commit_up_mbps: f64,
    /// Ceiling downstream megabits per second.
    pub ceil_down_mbps: f64,
    /// Ceiling upstream megabits per second.
    pub ceil_up_mbps: f64,
}

/// A checked commit/ceiling plan in raw bits per second.
///
/// Produced by [`NetworkRatePlan::to_bps`] after the finite/range/order checks;
/// [`LibreQosNetworkFile::into_imported_bundle`] consumes this type only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct NetworkRatePlanBps {
    /// Committed downstream bits per second.
    pub commit_down_bps: u64,
    /// Committed upstream bits per second.
    pub commit_up_bps: u64,
    /// Ceiling downstream bits per second.
    pub ceil_down_bps: u64,
    /// Ceiling upstream bits per second.
    pub ceil_up_bps: u64,
}

/// The maximum accepted value in any decimal-Mbps rate field (1 Tbps).
pub const NETWORK_RATE_MAX_MBPS: f64 = 1_000_000.0;

impl NetworkRatePlan {
    /// Converts the four decimal-Mbps fields to bits per second.
    ///
    /// `f64::round` rounds the **product** to nearest, ties away from zero.
    /// This is deliberately not exact-decimal arithmetic: `0.0001245 * 1e6`
    /// is `124.49999999999999` in f64 and therefore rounds to `124` bps.
    /// `path` names the row (e.g. `circuits[7].plan`) in any error.
    pub fn to_bps(&self, path: &str) -> Result<NetworkRatePlanBps, LibreQosNetworkError> {
        let commit_down_bps = checked_mbps_to_bps(self.commit_down_mbps, path, "commit_down_mbps")?;
        let commit_up_bps = checked_mbps_to_bps(self.commit_up_mbps, path, "commit_up_mbps")?;
        let ceil_down_bps = checked_mbps_to_bps(self.ceil_down_mbps, path, "ceil_down_mbps")?;
        let ceil_up_bps = checked_mbps_to_bps(self.ceil_up_mbps, path, "ceil_up_mbps")?;
        // The decimal order is the contract (a positive commit with ceil 0 is
        // never unlimited); the bps order is a post-conversion defense.
        if self.commit_down_mbps > self.ceil_down_mbps
            || self.commit_up_mbps > self.ceil_up_mbps
            || commit_down_bps > ceil_down_bps
            || commit_up_bps > ceil_up_bps
        {
            return Err(err(path, "commit must not exceed ceil"));
        }
        Ok(NetworkRatePlanBps {
            commit_down_bps,
            commit_up_bps,
            ceil_down_bps,
            ceil_up_bps,
        })
    }
}

fn checked_mbps_to_bps(
    value: f64,
    path: &str,
    field: &str,
) -> Result<u64, LibreQosNetworkError> {
    if !value.is_finite() {
        return Err(err(path, format!("{field} must be finite")));
    }
    if value < 0.0 {
        return Err(err(path, format!("{field} must be >= 0")));
    }
    if value > NETWORK_RATE_MAX_MBPS {
        return Err(err(
            path,
            format!("{field} {value} exceeds {} Mbps", NETWORK_RATE_MAX_MBPS),
        ));
    }
    let scaled = value * 1_000_000.0;
    // The bound above keeps this far inside u64, but keep the conversion
    // checked so a future bound change cannot silently wrap.
    if !scaled.is_finite() || scaled > u64::MAX as f64 {
        return Err(err(
            path,
            format!("{field} {value} Mbps overflows bits per second"),
        ));
    }
    Ok(scaled.round() as u64)
}

/// A topology node record.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkNode {
    /// Stable node id.
    pub id: String,
    /// The node kind.
    #[serde(rename = "type")]
    pub node_type: NetworkNodeKind,
    /// The parent node id (absent for a root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Human-readable name.
    pub name: String,
    /// Optional latitude (the site map's world-view fallback covers absence).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f32>,
    /// Optional longitude.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f32>,
    /// Optional node rate plan (commit/ceil; only where a node shapes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<NetworkRatePlan>,
}

/// A device record belonging to a circuit.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NetworkDevice {
    /// Stable device id.
    pub id: String,
    /// Human-readable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional MAC (operator-authored; the pair carries it forever; B2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    /// IPv4 addresses (bare or CIDR).
    #[serde(default)]
    pub ipv4: Vec<String>,
    /// IPv6 addresses (bare or CIDR).
    #[serde(default)]
    pub ipv6: Vec<String>,
    /// Free-form comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// The per-circuit mode-specific binding (validated against the global mode).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NetworkMatchBinding {
    /// `ip` mode: the circuit's prefixes.
    #[serde(default)]
    pub prefixes: Vec<String>,
    /// `vxlan` / `vxlan+vlan` mode: the VNI.
    #[serde(default)]
    pub vni: Option<u32>,
    /// `vxlan+vlan` / `vlan` mode: the VLAN id.
    #[serde(default)]
    pub vlan: Option<u16>,
    /// `mpls` mode: the top-of-stack label.
    #[serde(default)]
    pub label: Option<u32>,
}

/// A subscriber circuit record.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkCircuit {
    /// Stable circuit id.
    pub id: String,
    /// The topology node id this circuit attaches under (required; must name a
    /// topology node — the v2 expressiveness ruling).
    pub parent: String,
    /// Optional display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The commit/ceiling plan.
    pub plan: NetworkRatePlan,
    /// Devices (required for `ip` mode; optional otherwise).
    #[serde(default)]
    pub devices: Vec<NetworkDevice>,
    /// The mode-specific binding.
    #[serde(default)]
    pub r#match: NetworkMatchBinding,
}

/// The top-level `match` configuration (the mode is global; circuits carry
/// their per-mode binding).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NetworkMatchConfig {
    /// The global match mode.
    #[serde(default)]
    pub mode: NetworkMatchMode,
    /// The optional steering-VLAN declaration (advisory; C2).
    #[serde(default)]
    pub steering_vlans: Vec<u16>,
}

/// The parsed `libreqos_network.json` top level.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LibreQosNetworkFile {
    /// The schema version (must equal [`LIBREQOS_NETWORK_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// The global match configuration.
    #[serde(default)]
    pub r#match: NetworkMatchConfig,
    /// Topology nodes.
    #[serde(default)]
    pub topology: Vec<NetworkNode>,
    /// Subscriber circuits.
    #[serde(default)]
    pub circuits: Vec<NetworkCircuit>,
}

/// The lenient header used to reject an unsupported version **before** the
/// strict body parse. Unknown body fields are ignored here on purpose: a real
/// v1 file (`*_bps` fields) must reach the typed version reject, not a
/// `Parse(unknown field)` from `deny_unknown_fields`.
#[derive(Deserialize)]
struct LibreQosNetworkSchemaHeader {
    schema_version: u32,
}

/// Parses AND validates a `libreqos_network.json` payload.
///
/// Fail-loud: an over-size payload, malformed JSON, an unsupported version, or
/// any semantic error is a typed reject (never a partial accept). The version
/// is read through a lenient header first; only a supported version reaches
/// the strict (`deny_unknown_fields`) body parse.
pub fn parse_libreqos_network(raw: &str) -> Result<LibreQosNetworkFile, LibreQosNetworkError> {
    if raw.len() > LIBREQOS_NETWORK_MAX_BYTES {
        return Err(LibreQosNetworkError::TooLarge {
            found: raw.len(),
            max: LIBREQOS_NETWORK_MAX_BYTES,
        });
    }
    let header: LibreQosNetworkSchemaHeader = serde_json::from_str(raw)
        .map_err(|error| LibreQosNetworkError::Parse(error.to_string()))?;
    if header.schema_version != LIBREQOS_NETWORK_SCHEMA_VERSION {
        return Err(LibreQosNetworkError::UnsupportedSchemaVersion {
            found: header.schema_version,
            supported: LIBREQOS_NETWORK_SCHEMA_VERSION,
        });
    }
    let file: LibreQosNetworkFile = serde_json::from_str(raw)
        .map_err(|error| LibreQosNetworkError::Parse(error.to_string()))?;
    validate_libreqos_network(&file)?;
    Ok(file)
}

fn err(path: impl Into<String>, reason: impl Into<String>) -> LibreQosNetworkError {
    LibreQosNetworkError::Validation {
        path: path.into(),
        reason: reason.into(),
    }
}

/// Validates a parsed file (single version, topology references, plan order,
/// and the per-circuit mode binding).
pub fn validate_libreqos_network(file: &LibreQosNetworkFile) -> Result<(), LibreQosNetworkError> {
    if file.schema_version != LIBREQOS_NETWORK_SCHEMA_VERSION {
        return Err(LibreQosNetworkError::UnsupportedSchemaVersion {
            found: file.schema_version,
            supported: LIBREQOS_NETWORK_SCHEMA_VERSION,
        });
    }

    for vlan in &file.r#match.steering_vlans {
        if *vlan == 0 || *vlan > VLAN_ID_MAX {
            return Err(err(
                "match.steering_vlans",
                format!("vlan {vlan} is outside 1..={VLAN_ID_MAX}"),
            ));
        }
    }

    // Topology: unique ids; every parent names an existing node; empty ids banned.
    let mut node_ids = std::collections::BTreeSet::new();
    for (index, node) in file.topology.iter().enumerate() {
        if node.id.trim().is_empty() {
            return Err(err(format!("topology[{index}].id"), "must not be empty"));
        }
        if !node_ids.insert(node.id.as_str()) {
            return Err(err(
                format!("topology[{index}].id"),
                format!("duplicate node id '{}'", node.id),
            ));
        }
    }
    for (index, node) in file.topology.iter().enumerate() {
        if let Some(parent) = node.parent.as_deref()
            && !node_ids.contains(parent)
        {
            return Err(err(
                format!("topology[{index}].parent"),
                format!("parent '{parent}' does not name a node"),
            ));
        }
    }
    for (index, node) in file.topology.iter().enumerate() {
        if let Some(rate) = node.rate {
            rate.to_bps(&format!("topology[{index}].rate"))?;
        }
    }

    let mode = file.r#match.mode;
    let mut circuit_ids = std::collections::BTreeSet::new();
    for (index, circuit) in file.circuits.iter().enumerate() {
        if circuit.id.trim().is_empty() {
            return Err(err(format!("circuits[{index}].id"), "must not be empty"));
        }
        if !circuit_ids.insert(circuit.id.as_str()) {
            return Err(err(
                format!("circuits[{index}].id"),
                format!("duplicate circuit id '{}'", circuit.id),
            ));
        }
        if !node_ids.contains(circuit.parent.as_str()) {
            return Err(err(
                format!("circuits[{index}].parent"),
                format!("parent '{}' does not name a topology node", circuit.parent),
            ));
        }
        circuit.plan.to_bps(&format!("circuits[{index}].plan"))?;
        validate_binding(index, mode, circuit)?;
        for (device_index, device) in circuit.devices.iter().enumerate() {
            for entry in &device.ipv4 {
                if parse_ipv4_cidr(entry).is_none() {
                    return Err(err(
                        format!("circuits[{index}].devices[{device_index}].ipv4"),
                        format!("'{entry}' is not an IPv4 address or CIDR"),
                    ));
                }
            }
            for entry in &device.ipv6 {
                if parse_ipv6_cidr(entry).is_none() {
                    return Err(err(
                        format!("circuits[{index}].devices[{device_index}].ipv6"),
                        format!("'{entry}' is not an IPv6 address or CIDR"),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_binding(
    index: usize,
    mode: NetworkMatchMode,
    circuit: &NetworkCircuit,
) -> Result<(), LibreQosNetworkError> {
    let path = format!("circuits[{index}].match");
    let binding = &circuit.r#match;
    match mode {
        NetworkMatchMode::Ip => {
            if binding.prefixes.is_empty() {
                return Err(err(path, "ip mode requires a non-empty 'prefixes' list"));
            }
            if binding.vni.is_some() || binding.vlan.is_some() || binding.label.is_some() {
                return Err(err(path, "ip mode must not carry vni/vlan/label"));
            }
        }
        NetworkMatchMode::Vxlan => {
            let Some(vni) = binding.vni else {
                return Err(err(path, "vxlan mode requires 'vni'"));
            };
            if vni > VXLAN_VNI_MAX {
                return Err(err(path, format!("vni {vni} exceeds {VXLAN_VNI_MAX}")));
            }
            if binding.vlan.is_some() || binding.label.is_some() || !binding.prefixes.is_empty() {
                return Err(err(path, "vxlan mode carries only 'vni'"));
            }
        }
        NetworkMatchMode::VxlanVlan => {
            let Some(vni) = binding.vni else {
                return Err(err(path, "vxlan+vlan mode requires 'vni'"));
            };
            let Some(vlan) = binding.vlan else {
                return Err(err(path, "vxlan+vlan mode requires 'vlan'"));
            };
            if vni > VXLAN_VNI_MAX || vlan == 0 || vlan > VLAN_ID_MAX {
                return Err(err(path, "vni/vlan out of range"));
            }
            if binding.label.is_some() || !binding.prefixes.is_empty() {
                return Err(err(path, "vxlan+vlan mode carries only 'vni'+'vlan'"));
            }
        }
        NetworkMatchMode::Vlan => {
            let Some(vlan) = binding.vlan else {
                return Err(err(path, "vlan mode requires 'vlan'"));
            };
            if vlan == 0 || vlan > VLAN_ID_MAX {
                return Err(err(
                    path,
                    format!("vlan {vlan} is outside 1..={VLAN_ID_MAX}"),
                ));
            }
            if binding.vni.is_some() || binding.label.is_some() || !binding.prefixes.is_empty() {
                return Err(err(path, "vlan mode carries only 'vlan'"));
            }
        }
        NetworkMatchMode::Mpls => {
            let Some(label) = binding.label else {
                return Err(err(path, "mpls mode requires 'label'"));
            };
            if label > MPLS_LABEL_MAX {
                return Err(err(path, format!("label {label} exceeds {MPLS_LABEL_MAX}")));
            }
            if binding.vni.is_some() || binding.vlan.is_some() || !binding.prefixes.is_empty() {
                return Err(err(path, "mpls mode carries only 'label'"));
            }
        }
    }
    // Device-less circuits are legal ONLY with a non-`ip` match (E3).
    if mode == NetworkMatchMode::Ip && circuit.devices.is_empty() {
        return Err(err(
            format!("circuits[{index}].devices"),
            "ip mode requires at least one device",
        ));
    }
    Ok(())
}

impl LibreQosNetworkFile {
    /// Phase C S3: resolves the per-circuit match identities for the global
    /// mode, keyed by stable circuit id (the `ip` mode contributes no entries;
    /// its prefixes ride the device rows).
    ///
    /// This function is pure: it has no side effects.
    pub fn circuit_match_index(&self) -> TopologyCircuitMatchIndex {
        let mode = self.r#match.mode;
        let mut index = TopologyCircuitMatchIndex::new();
        for circuit in &self.circuits {
            let binding = &circuit.r#match;
            let identity = match mode {
                NetworkMatchMode::Ip => None,
                NetworkMatchMode::Vxlan | NetworkMatchMode::VxlanVlan => binding.vni.map(|vni| {
                    TopologyCircuitMatchIdentity::Tunnel(TunnelIdentity {
                        vni,
                        vlan: binding.vlan.unwrap_or(u16::MAX),
                    })
                }),
                NetworkMatchMode::Vlan => binding.vlan.map(TopologyCircuitMatchIdentity::Vlan),
                NetworkMatchMode::Mpls => binding.label.map(TopologyCircuitMatchIdentity::Mpls),
            };
            if let Some(identity) = identity {
                index.insert(circuit.id.clone(), identity);
            }
        }
        index
    }

    /// Phase C S3c: projects the validated format into an imported topology
    /// bundle targeting `native_canonical` (the flat-topology target). The
    /// compatibility `network.json` tree stays empty — that is the legacy
    /// door's business. The required circuit `parent` places each device
    /// (a device-less circuit becomes one synthetic, IP-less row so its match
    /// identity survives). Pure: no side effects.
    ///
    /// Every rate is converted through [`NetworkRatePlan::to_bps`] before it
    /// reaches the canonical/ShapedDevice mapping; a hand-built file that
    /// bypassed validation still cannot panic or silently wrap.
    pub fn into_imported_bundle(
        self,
        source: impl Into<String>,
    ) -> Result<ImportedTopologyBundle, LibreQosNetworkError> {
        let source = source.into();
        let node_names = self
            .topology
            .iter()
            .map(|node| (node.id.as_str(), node.name.as_str()))
            .collect::<BTreeMap<_, _>>();
        let mut nodes = Vec::with_capacity(self.topology.len());
        for (index, node) in self.topology.iter().enumerate() {
            let rate_bps = match node.rate {
                Some(rate) => Some(rate.to_bps(&format!("topology[{index}].rate"))?),
                None => None,
            };
            nodes.push(TopologyCanonicalNode {
                node_id: node.id.clone(),
                node_name: node.name.clone(),
                latitude: node.latitude,
                longitude: node.longitude,
                node_kind: node_kind_str(node.node_type).to_string(),
                current_parent_node_id: node.parent.clone(),
                current_parent_node_name: node
                    .parent
                    .as_deref()
                    .and_then(|parent| node_names.get(parent))
                    .map(|name| (*name).to_string()),
                rate_input: TopologyCanonicalRateInput {
                    intrinsic_download_mbps: rate_bps.map(|rate| bps_to_mbps_u64(rate.ceil_down_bps)),
                    intrinsic_upload_mbps: rate_bps.map(|rate| bps_to_mbps_u64(rate.ceil_up_bps)),
                    ..TopologyCanonicalRateInput::default()
                },
                ..TopologyCanonicalNode::default()
            });
        }

        let mut devices = Vec::new();
        let mut anchors = Vec::new();
        for (index, circuit) in self.circuits.iter().enumerate() {
            let plan_bps = circuit.plan.to_bps(&format!("circuits[{index}].plan"))?;
            let parent_name = node_names
                .get(circuit.parent.as_str())
                .map(|name| (*name).to_string())
                .unwrap_or_else(|| circuit.parent.clone());
            let circuit_name = circuit
                .display_name
                .clone()
                .unwrap_or_else(|| circuit.id.clone());
            anchors.push(CircuitAnchor {
                circuit_id: circuit.id.clone(),
                circuit_name: Some(circuit_name.clone()),
                anchor_node_id: circuit.parent.clone(),
                anchor_node_name: Some(parent_name.clone()),
            });
            if circuit.devices.is_empty() {
                devices.push(build_shaped_device(
                    circuit,
                    plan_bps,
                    &circuit.id,
                    &circuit_name,
                    &parent_name,
                    String::new(),
                    Vec::new(),
                    Vec::new(),
                ));
            } else {
                for device in &circuit.devices {
                    let device_name = device.name.clone().unwrap_or_else(|| device.id.clone());
                    devices.push(build_shaped_device(
                        circuit,
                        plan_bps,
                        &device.id,
                        &device_name,
                        &parent_name,
                        device.mac.clone().unwrap_or_default(),
                        device.ipv4.iter().filter_map(|ip| parse_ipv4_cidr(ip)).collect(),
                        device.ipv6.iter().filter_map(|ip| parse_ipv6_cidr(ip)).collect(),
                    ));
                }
            }
        }
        let mut shaped_devices = ConfigShapedDevices::default();
        shaped_devices.replace_with_new_data(devices);

        let native_canonical = (!nodes.is_empty()).then(|| TopologyCanonicalStateFile {
            schema_version: 1,
            source: source.clone(),
            generated_unix: None,
            ingress_identity: None,
            ingress_kind: TopologyCanonicalIngressKind::NativeIntegration,
            nodes,
            compatibility_network_json: serde_json::Value::Object(serde_json::Map::new()),
        });
        let circuit_matches = self.circuit_match_index();
        Ok(ImportedTopologyBundle {
            source: source.clone(),
            generated_unix: None,
            ingress_identity: None,
            native_canonical,
            parent_candidates: None,
            compatibility_network_json: serde_json::Value::Object(serde_json::Map::new()),
            shaped_devices,
            circuit_anchors: CircuitAnchorsFile {
                schema_version: 1,
                source,
                generated_unix: None,
                anchors,
            },
            ethernet_advisories: Vec::new(),
            circuit_matches,
        })
    }
}

fn node_kind_str(kind: NetworkNodeKind) -> &'static str {
    match kind {
        NetworkNodeKind::Site => "site",
        NetworkNodeKind::Node => "node",
        NetworkNodeKind::Ap => "ap",
    }
}

fn bps_to_mbps(bits_per_second: u64) -> f32 {
    (bits_per_second as f64 / 1_000_000.0) as f32
}

fn bps_to_mbps_u64(bits_per_second: u64) -> u64 {
    (bits_per_second as f64 / 1_000_000.0).round() as u64
}

fn build_shaped_device(
    circuit: &NetworkCircuit,
    plan: NetworkRatePlanBps,
    device_id: &str,
    device_name: &str,
    parent_name: &str,
    mac: String,
    ipv4: Vec<(Ipv4Addr, u32)>,
    ipv6: Vec<(Ipv6Addr, u32)>,
) -> ShapedDevice {
    let mut device = ShapedDevice {
        circuit_id: circuit.id.clone(),
        circuit_name: circuit
            .display_name
            .clone()
            .unwrap_or_else(|| circuit.id.clone()),
        device_id: device_id.to_string(),
        device_name: device_name.to_string(),
        parent_node: parent_name.to_string(),
        parent_node_id: Some(circuit.parent.clone()),
        anchor_node_id: None,
        mac,
        ipv4,
        ipv6,
        download_min_mbps: bps_to_mbps(plan.commit_down_bps),
        upload_min_mbps: bps_to_mbps(plan.commit_up_bps),
        download_max_mbps: bps_to_mbps(plan.ceil_down_bps),
        upload_max_mbps: bps_to_mbps(plan.ceil_up_bps),
        comment: String::new(),
        sqm_override: None,
        circuit_hash: 0,
        device_hash: 0,
        parent_hash: 0,
    };
    device.refresh_hashes();
    device
}

fn parse_ipv4_cidr(value: &str) -> Option<(Ipv4Addr, u32)> {
    let (address, prefix) = match value.split_once('/') {
        Some((address, prefix)) => (
            address,
            prefix.parse::<u32>().ok().filter(|prefix| *prefix <= 32)?,
        ),
        None => (value, 32),
    };
    Some((address.parse::<Ipv4Addr>().ok()?, prefix))
}

fn parse_ipv6_cidr(value: &str) -> Option<(Ipv6Addr, u32)> {
    let (address, prefix) = match value.split_once('/') {
        Some((address, prefix)) => (
            address,
            prefix.parse::<u32>().ok().filter(|prefix| *prefix <= 128)?,
        ),
        None => (value, 128),
    };
    Some((address.parse::<Ipv6Addr>().ok()?, prefix))
}


#[cfg(test)]
mod tests {
    use super::*;

    fn valid_ip() -> String {
        r#"{
            "schema_version": 2,
            "match": { "mode": "ip" },
            "topology": [
                { "id": "s1", "type": "site", "name": "Site 1" },
                { "id": "ap1", "type": "ap", "parent": "s1", "name": "AP 1", "latitude": 1.0, "longitude": 2.0 }
            ],
            "circuits": [
                { "id": "c1", "parent": "ap1", "display_name": "C1",
                  "plan": { "commit_down_mbps": 10.0, "commit_up_mbps": 5.0, "ceil_down_mbps": 100.0, "ceil_up_mbps": 50.0 },
                  "devices": [ { "id": "d1", "mac": "aa:bb:cc:dd:ee:ff", "ipv4": ["192.0.2.10/32"] } ],
                  "match": { "prefixes": ["192.0.2.10/32"] } }
            ]
        }"#
        .to_string()
    }

    fn plan(mbps: f64) -> NetworkRatePlan {
        NetworkRatePlan {
            commit_down_mbps: mbps,
            commit_up_mbps: mbps,
            ceil_down_mbps: mbps,
            ceil_up_mbps: mbps,
        }
    }

    fn validation_path(result: Result<LibreQosNetworkFile, LibreQosNetworkError>) -> String {
        match result {
            Err(LibreQosNetworkError::Validation { path, .. }) => path,
            other => panic!("expected a row-named validation error, got {other:?}"),
        }
    }

    #[test]
    fn valid_ip_file_parses() {
        let file = parse_libreqos_network(&valid_ip()).expect("valid ip file");
        assert_eq!(file.schema_version, 2);
        assert_eq!(file.r#match.mode, NetworkMatchMode::Ip);
        assert_eq!(file.topology.len(), 2);
        assert_eq!(file.circuits.len(), 1);
        assert!(file.topology[1].parent.as_deref() == Some("s1"));
    }

    #[test]
    fn decimal_mbps_to_bps_vectors() {
        assert_eq!(plan(115.0).to_bps("circuits[0].plan").unwrap().ceil_down_bps, 115_000_000);
        assert_eq!(plan(0.115).to_bps("circuits[0].plan").unwrap().ceil_down_bps, 115_000);
        assert_eq!(plan(250.5).to_bps("circuits[0].plan").unwrap().ceil_down_bps, 250_500_000);
        assert_eq!(plan(0.0000005).to_bps("circuits[0].plan").unwrap().ceil_down_bps, 1);
        assert_eq!(plan(0.0000004).to_bps("circuits[0].plan").unwrap().ceil_down_bps, 0);
        // f64 product semantics: 0.0001245 * 1e6 = 124.49999999999999 -> 124.
        assert_eq!(plan(0.0001245).to_bps("circuits[0].plan").unwrap().ceil_down_bps, 124);
    }

    #[test]
    fn node_and_circuit_use_the_same_decimal_conversion() {
        let raw = r#"{
            "schema_version": 2,
            "match": { "mode": "vlan" },
            "topology": [
                { "id": "s1", "type": "site", "name": "Site 1",
                  "rate": { "commit_down_mbps": 0.0, "commit_up_mbps": 0.0,
                            "ceil_down_mbps": 250.5, "ceil_up_mbps": 250.5 } }
            ],
            "circuits": [
                { "id": "c1", "parent": "s1",
                  "plan": { "commit_down_mbps": 0.115, "commit_up_mbps": 0.115,
                            "ceil_down_mbps": 250.5, "ceil_up_mbps": 250.5 },
                  "match": { "vlan": 10 } }
            ]
        }"#;
        let file = parse_libreqos_network(raw).expect("valid decimal file");
        let node_bps = file.topology[0]
            .rate
            .expect("node rate")
            .to_bps("topology[0].rate")
            .expect("node conversion");
        let circuit_bps = file.circuits[0]
            .plan
            .to_bps("circuits[0].plan")
            .expect("circuit conversion");
        assert_eq!(node_bps.ceil_down_bps, 250_500_000);
        assert_eq!(circuit_bps.ceil_down_bps, 250_500_000);
        assert_eq!(circuit_bps.commit_down_bps, 115_000);

        let bundle = file
            .into_imported_bundle("test/libreqos_network")
            .expect("valid bundle");
        let node = &bundle
            .native_canonical
            .expect("native canonical")
            .nodes[0];
        assert_eq!(node.rate_input.intrinsic_download_mbps, Some(251));
        let device = &bundle.shaped_devices.devices[0];
        assert_eq!(device.download_max_mbps, 250.5);
        assert_eq!(device.download_min_mbps, 0.115);
    }

    #[test]
    fn real_v1_fixture_rejects_with_typed_version() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/libreqos_network_v1_reject.json"
        ))
        .expect("v1 fixture readable");
        assert!(raw.contains("commit_down_bps"), "the fixture is a real v1 body");
        match parse_libreqos_network(&raw) {
            Err(LibreQosNetworkError::UnsupportedSchemaVersion { found, supported }) => {
                assert_eq!((found, supported), (1, 2));
            }
            other => panic!("expected a typed v1 version reject, got {other:?}"),
        }
    }

    #[test]
    fn version_probe_precedes_the_strict_body_parse() {
        // A v2-shaped body with only the version byte changed still reaches the
        // typed version check (the proxy case).
        let raw = valid_ip().replace("\"schema_version\": 2", "\"schema_version\": 1");
        assert!(matches!(
            parse_libreqos_network(&raw),
            Err(LibreQosNetworkError::UnsupportedSchemaVersion { found: 1, supported: 2 })
        ));
    }

    #[test]
    fn stray_bps_in_a_v2_body_is_a_parse_error() {
        let raw = valid_ip().replace("\"commit_down_mbps\"", "\"commit_down_bps\"");
        assert!(matches!(
            parse_libreqos_network(&raw),
            Err(LibreQosNetworkError::Parse(_))
        ));
    }

    #[test]
    fn unknown_field_rejects() {
        let raw = valid_ip().replace("\"id\": \"s1\"", "\"id\": \"s1\", \"bogus\": true");
        assert!(matches!(
            parse_libreqos_network(&raw),
            Err(LibreQosNetworkError::Parse(_))
        ));
    }

    #[test]
    fn commit_over_ceil_names_the_row() {
        let raw = valid_ip().replace("\"commit_down_mbps\": 10.0", "\"commit_down_mbps\": 1000.0");
        assert_eq!(validation_path(parse_libreqos_network(&raw)), "circuits[0].plan");
    }

    #[test]
    fn ceil_zero_with_positive_commit_rejects() {
        let raw = valid_ip().replace("\"ceil_down_mbps\": 100.0", "\"ceil_down_mbps\": 0.0");
        assert_eq!(validation_path(parse_libreqos_network(&raw)), "circuits[0].plan");
    }

    #[test]
    fn zero_commit_zero_ceil_is_unlimited() {
        let raw = valid_ip()
            .replace("\"commit_down_mbps\": 10.0", "\"commit_down_mbps\": 0.0")
            .replace("\"commit_up_mbps\": 5.0", "\"commit_up_mbps\": 0.0")
            .replace("\"ceil_down_mbps\": 100.0", "\"ceil_down_mbps\": 0.0")
            .replace("\"ceil_up_mbps\": 50.0", "\"ceil_up_mbps\": 0.0");
        let plan = parse_libreqos_network(&raw).expect("0/0 is unlimited").circuits[0].plan;
        let bps = plan.to_bps("circuits[0].plan").expect("0/0 conversion");
        assert_eq!(
            (bps.commit_down_bps, bps.commit_up_bps, bps.ceil_down_bps, bps.ceil_up_bps),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn bounds_reject_negative_non_finite_and_over_max() {
        assert!(matches!(
            plan(-1.0).to_bps("circuits[0].plan"),
            Err(LibreQosNetworkError::Validation { .. })
        ));
        assert!(matches!(
            plan(f64::NAN).to_bps("circuits[0].plan"),
            Err(LibreQosNetworkError::Validation { .. })
        ));
        assert!(matches!(
            plan(f64::INFINITY).to_bps("circuits[0].plan"),
            Err(LibreQosNetworkError::Validation { .. })
        ));
        assert!(matches!(
            plan(f64::MAX).to_bps("circuits[0].plan"),
            Err(LibreQosNetworkError::Validation { .. })
        ));
        assert!(matches!(
            plan(1_000_000.5).to_bps("circuits[0].plan"),
            Err(LibreQosNetworkError::Validation { .. })
        ));
        assert_eq!(
            plan(1_000_000.0).to_bps("circuits[0].plan").unwrap().ceil_down_bps,
            1_000_000_000_000
        );
    }

    #[test]
    fn ip_mode_without_devices_rejects() {
        let raw = valid_ip().replace(",\n                  \"devices\": [ { \"id\": \"d1\", \"mac\": \"aa:bb:cc:dd:ee:ff\", \"ipv4\": [\"192.0.2.10/32\"] } ]", "");
        assert_eq!(validation_path(parse_libreqos_network(&raw)), "circuits[0].devices");
    }

    #[test]
    fn missing_parent_rejects_with_row_name() {
        let raw = valid_ip().replace("\"parent\": \"ap1\"", "\"parent\": \"nope\"");
        assert_eq!(validation_path(parse_libreqos_network(&raw)), "circuits[0].parent");
    }

    #[test]
    fn vxlan_vlan_binding_round_trips_and_validates() {
        let raw = r#"{
            "schema_version": 2,
            "match": { "mode": "vxlan+vlan", "steering_vlans": [10, 20] },
            "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
            "circuits": [
                { "id": "c1",
                  "parent": "s1",
                  "plan": { "commit_down_mbps": 1.0, "commit_up_mbps": 1.0, "ceil_down_mbps": 2.0, "ceil_up_mbps": 2.0 },
                  "devices": [],
                  "match": { "vni": 100, "vlan": 64 } }
            ]
        }"#;
        let file = parse_libreqos_network(raw).expect("valid vxlan+vlan file");
        assert_eq!(file.circuits[0].r#match.vni, Some(100));
        assert_eq!(file.r#match.steering_vlans, vec![10, 20]);
    }

    #[test]
    fn vlan_binding_rejects_out_of_range() {
        let raw = r#"{
            "schema_version": 2,
            "match": { "mode": "vlan" },
            "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
            "circuits": [
                { "id": "c1",
                  "parent": "s1",
                  "plan": { "commit_down_mbps": 1.0, "commit_up_mbps": 1.0, "ceil_down_mbps": 2.0, "ceil_up_mbps": 2.0 },
                  "match": { "vlan": 4095 } }
            ]
        }"#;
        assert_eq!(validation_path(parse_libreqos_network(raw)), "circuits[0].match");
    }

    #[test]
    fn mpls_binding_accepts_top_label() {
        let raw = r#"{
            "schema_version": 2,
            "match": { "mode": "mpls" },
            "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
            "circuits": [
                { "id": "c1",
                  "parent": "s1",
                  "plan": { "commit_down_mbps": 1.0, "commit_up_mbps": 1.0, "ceil_down_mbps": 2.0, "ceil_up_mbps": 2.0 },
                  "match": { "label": 1048575 } }
            ]
        }"#;
        assert!(parse_libreqos_network(raw).is_ok());
    }

    #[test]
    fn circuit_match_index_resolves_each_mode() {
        let vxlan_vlan = parse_libreqos_network(
            r#"{
                "schema_version": 2,
                "match": { "mode": "vxlan+vlan" },
                "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
                "circuits": [
                    { "id": "c1",
                      "parent": "s1",
                      "plan": { "commit_down_mbps": 1.0, "commit_up_mbps": 1.0, "ceil_down_mbps": 2.0, "ceil_up_mbps": 2.0 },
                      "match": { "vni": 100, "vlan": 64 } }
                ]
            }"#,
        )
        .expect("valid vxlan+vlan file");
        assert_eq!(
            vxlan_vlan.circuit_match_index().get("c1"),
            Some(&TopologyCircuitMatchIdentity::Tunnel(TunnelIdentity {
                vni: 100,
                vlan: 64
            }))
        );

        let vxlan = parse_libreqos_network(
            r#"{
                "schema_version": 2,
                "match": { "mode": "vxlan" },
                "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
                "circuits": [
                    { "id": "c2",
                      "parent": "s1",
                      "plan": { "commit_down_mbps": 1.0, "commit_up_mbps": 1.0, "ceil_down_mbps": 2.0, "ceil_up_mbps": 2.0 },
                      "match": { "vni": 200 } }
                ]
            }"#,
        )
        .expect("valid vxlan file");
        assert_eq!(
            vxlan.circuit_match_index().get("c2"),
            Some(&TopologyCircuitMatchIdentity::Tunnel(TunnelIdentity {
                vni: 200,
                vlan: u16::MAX
            })),
            "an absent vlan resolves to the untagged sentinel"
        );

        let ip = parse_libreqos_network(&valid_ip()).expect("valid ip file");
        assert!(
            ip.circuit_match_index().is_empty(),
            "ip mode contributes no identity (prefixes ride the device rows)"
        );
    }

    #[test]
    fn into_imported_bundle_projects_topology_devices_and_anchors() {
        let file = parse_libreqos_network(&valid_ip()).expect("valid ip file");
        let bundle = file
            .into_imported_bundle("test/libreqos_network")
            .expect("valid bundle");
        let canonical = bundle.native_canonical.expect("native canonical");
        assert_eq!(canonical.nodes.len(), 2);
        assert_eq!(canonical.nodes[1].current_parent_node_id.as_deref(), Some("s1"));
        assert_eq!(
            canonical.nodes[1].current_parent_node_name.as_deref(),
            Some("Site 1")
        );
        assert_eq!(bundle.shaped_devices.devices.len(), 1);
        let device = &bundle.shaped_devices.devices[0];
        assert_eq!(device.circuit_id, "c1");
        assert_eq!(device.parent_node, "AP 1");
        assert_eq!(device.parent_node_id.as_deref(), Some("ap1"));
        assert_eq!(device.download_min_mbps, 10.0);
        assert_eq!(bundle.circuit_anchors.anchors.len(), 1);
        assert_eq!(bundle.circuit_anchors.anchors[0].anchor_node_id, "ap1");
        assert!(bundle.circuit_matches.is_empty(), "ip mode has no identity");
    }

    #[test]
    fn into_imported_bundle_synthesizes_device_less_match_circuit() {
        let file = parse_libreqos_network(
            r#"{
                "schema_version": 2,
                "match": { "mode": "vxlan+vlan" },
                "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
                "circuits": [
                    { "id": "c1",
                      "parent": "s1",
                      "plan": { "commit_down_mbps": 1.0, "commit_up_mbps": 1.0, "ceil_down_mbps": 2.0, "ceil_up_mbps": 2.0 },
                      "match": { "vni": 100, "vlan": 64 } }
                ]
            }"#,
        )
        .expect("valid device-less vxlan+vlan file");
        let bundle = file
            .into_imported_bundle("test/libreqos_network")
            .expect("valid bundle");
        assert_eq!(bundle.shaped_devices.devices.len(), 1);
        let device = &bundle.shaped_devices.devices[0];
        assert_eq!(device.circuit_id, "c1");
        assert!(device.ipv4.is_empty() && device.ipv6.is_empty());
        assert_eq!(
            bundle.circuit_matches.get("c1"),
            Some(&TopologyCircuitMatchIdentity::Tunnel(TunnelIdentity {
                vni: 100,
                vlan: 64
            }))
        );
    }
}

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
    CircuitAnchor, CircuitAnchorsFile, ConfigShapedDevices, ShapedDevice,
    TopologyCanonicalIngressKind, TopologyCanonicalNode, TopologyCanonicalRateInput,
    TopologyCanonicalStateFile, TopologyCircuitMatchIdentity, TopologyCircuitMatchIndex,
    TunnelIdentity,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};

/// The filename of the standard custom-integration format.
pub const LIBREQOS_NETWORK_FILENAME: &str = "libreqos_network.json";

/// The single supported `schema_version`.
pub const LIBREQOS_NETWORK_SCHEMA_VERSION: u32 = 1;

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

/// A commit/ceiling plan in raw bits per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NetworkRatePlan {
    /// Committed downstream bits per second.
    pub commit_down_bps: u64,
    /// Committed upstream bits per second.
    pub commit_up_bps: u64,
    /// Ceiling downstream bits per second.
    pub ceil_down_bps: u64,
    /// Ceiling upstream bits per second.
    pub ceil_up_bps: u64,
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

/// Parses AND validates a `libreqos_network.json` payload.
///
/// Fail-loud: an over-size payload, malformed JSON, an unsupported version, or
/// any semantic error is a typed reject (never a partial accept).
pub fn parse_libreqos_network(raw: &str) -> Result<LibreQosNetworkFile, LibreQosNetworkError> {
    if raw.len() > LIBREQOS_NETWORK_MAX_BYTES {
        return Err(LibreQosNetworkError::TooLarge {
            found: raw.len(),
            max: LIBREQOS_NETWORK_MAX_BYTES,
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
        if let Some(rate) = node.rate
            && (rate.commit_down_bps > rate.ceil_down_bps || rate.commit_up_bps > rate.ceil_up_bps)
        {
            return Err(err(
                format!("topology[{index}].rate"),
                "commit must not exceed ceil",
            ));
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
        let plan = circuit.plan;
        if plan.commit_down_bps > plan.ceil_down_bps || plan.commit_up_bps > plan.ceil_up_bps {
            return Err(err(
                format!("circuits[{index}].plan"),
                "commit must not exceed ceil",
            ));
        }
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
    pub fn into_imported_bundle(self, source: impl Into<String>) -> ImportedTopologyBundle {
        let source = source.into();
        let node_names = self
            .topology
            .iter()
            .map(|node| (node.id.as_str(), node.name.as_str()))
            .collect::<BTreeMap<_, _>>();
        let nodes = self
            .topology
            .iter()
            .map(|node| TopologyCanonicalNode {
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
                    intrinsic_download_mbps: node
                        .rate
                        .map(|rate| bps_to_mbps_u64(rate.ceil_down_bps)),
                    intrinsic_upload_mbps: node.rate.map(|rate| bps_to_mbps_u64(rate.ceil_up_bps)),
                    ..TopologyCanonicalRateInput::default()
                },
                ..TopologyCanonicalNode::default()
            })
            .collect::<Vec<_>>();

        let mut devices = Vec::new();
        let mut anchors = Vec::new();
        for circuit in &self.circuits {
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
        ImportedTopologyBundle {
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
        }
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
    device_id: &str,
    device_name: &str,
    parent_name: &str,
    mac: String,
    ipv4: Vec<(Ipv4Addr, u32)>,
    ipv6: Vec<(Ipv6Addr, u32)>,
) -> ShapedDevice {
    let plan = circuit.plan;
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
            "schema_version": 1,
            "match": { "mode": "ip" },
            "topology": [
                { "id": "s1", "type": "site", "name": "Site 1" },
                { "id": "ap1", "type": "ap", "parent": "s1", "name": "AP 1", "latitude": 1.0, "longitude": 2.0 }
            ],
            "circuits": [
                { "id": "c1", "parent": "ap1", "display_name": "C1",
                  "plan": { "commit_down_bps": 10, "commit_up_bps": 5, "ceil_down_bps": 100, "ceil_up_bps": 50 },
                  "devices": [ { "id": "d1", "mac": "aa:bb:cc:dd:ee:ff", "ipv4": ["192.0.2.10/32"] } ],
                  "match": { "prefixes": ["192.0.2.10/32"] } }
            ]
        }"#
        .to_string()
    }

    #[test]
    fn valid_ip_file_parses() {
        let file = parse_libreqos_network(&valid_ip()).expect("valid ip file");
        assert_eq!(file.r#match.mode, NetworkMatchMode::Ip);
        assert_eq!(file.topology.len(), 2);
        assert_eq!(file.circuits.len(), 1);
        assert!(file.topology[1].parent.as_deref() == Some("s1"));
    }

    #[test]
    fn unsupported_version_rejects() {
        let raw = valid_ip().replace("\"schema_version\": 1", "\"schema_version\": 2");
        assert!(matches!(
            parse_libreqos_network(&raw),
            Err(LibreQosNetworkError::UnsupportedSchemaVersion { found: 2, .. })
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
        let raw = valid_ip().replace("\"commit_down_bps\": 10", "\"commit_down_bps\": 1000");
        match parse_libreqos_network(&raw) {
            Err(LibreQosNetworkError::Validation { path, .. }) => {
                assert_eq!(path, "circuits[0].plan")
            }
            other => panic!("expected a row-named validation error, got {other:?}"),
        }
    }

    #[test]
    fn ip_mode_without_devices_rejects() {
        let raw = valid_ip().replace(",\n                  \"devices\": [ { \"id\": \"d1\", \"mac\": \"aa:bb:cc:dd:ee:ff\", \"ipv4\": [\"192.0.2.10/32\"] } ]", "");
        match parse_libreqos_network(&raw) {
            Err(LibreQosNetworkError::Validation { path, .. }) => {
                assert_eq!(path, "circuits[0].devices")
            }
            other => panic!("expected the devices row to be named, got {other:?}"),
        }
    }

    #[test]
    fn missing_parent_rejects_with_row_name() {
        let raw = valid_ip().replace("\"parent\": \"ap1\"", "\"parent\": \"nope\"");
        match parse_libreqos_network(&raw) {
            Err(LibreQosNetworkError::Validation { path, .. }) => {
                assert_eq!(path, "circuits[0].parent")
            }
            other => panic!("expected the parent row to be named, got {other:?}"),
        }
    }

    #[test]
    fn vxlan_vlan_binding_round_trips_and_validates() {
        let raw = r#"{
            "schema_version": 1,
            "match": { "mode": "vxlan+vlan", "steering_vlans": [10, 20] },
            "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
            "circuits": [
                { "id": "c1",
                  "parent": "s1",
                  "plan": { "commit_down_bps": 1, "commit_up_bps": 1, "ceil_down_bps": 2, "ceil_up_bps": 2 },
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
            "schema_version": 1,
            "match": { "mode": "vlan" },
            "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
            "circuits": [
                { "id": "c1",
                  "parent": "s1",
                  "plan": { "commit_down_bps": 1, "commit_up_bps": 1, "ceil_down_bps": 2, "ceil_up_bps": 2 },
                  "match": { "vlan": 4095 } }
            ]
        }"#;
        match parse_libreqos_network(raw) {
            Err(LibreQosNetworkError::Validation { path, .. }) => {
                assert_eq!(path, "circuits[0].match")
            }
            other => panic!("expected a vlan range error, got {other:?}"),
        }
    }

    #[test]
    fn mpls_binding_accepts_top_label() {
        let raw = r#"{
            "schema_version": 1,
            "match": { "mode": "mpls" },
            "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
            "circuits": [
                { "id": "c1",
                  "parent": "s1",
                  "plan": { "commit_down_bps": 1, "commit_up_bps": 1, "ceil_down_bps": 2, "ceil_up_bps": 2 },
                  "match": { "label": 1048575 } }
            ]
        }"#;
        assert!(parse_libreqos_network(raw).is_ok());
    }

    #[test]
    fn circuit_match_index_resolves_each_mode() {
        let vxlan_vlan = parse_libreqos_network(
            r#"{
                "schema_version": 1,
                "match": { "mode": "vxlan+vlan" },
                "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
                "circuits": [
                    { "id": "c1",
                      "parent": "s1",
                      "plan": { "commit_down_bps": 1, "commit_up_bps": 1, "ceil_down_bps": 2, "ceil_up_bps": 2 },
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
                "schema_version": 1,
                "match": { "mode": "vxlan" },
                "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
                "circuits": [
                    { "id": "c2",
                      "parent": "s1",
                      "plan": { "commit_down_bps": 1, "commit_up_bps": 1, "ceil_down_bps": 2, "ceil_up_bps": 2 },
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
        let bundle = file.into_imported_bundle("test/libreqos_network");
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
        assert_eq!(device.download_min_mbps, 0.00001);
        assert_eq!(bundle.circuit_anchors.anchors.len(), 1);
        assert_eq!(bundle.circuit_anchors.anchors[0].anchor_node_id, "ap1");
        assert!(bundle.circuit_matches.is_empty(), "ip mode has no identity");
    }

    #[test]
    fn into_imported_bundle_synthesizes_device_less_match_circuit() {
        let file = parse_libreqos_network(
            r#"{
                "schema_version": 1,
                "match": { "mode": "vxlan+vlan" },
                "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
                "circuits": [
                    { "id": "c1",
                      "parent": "s1",
                      "plan": { "commit_down_bps": 1, "commit_up_bps": 1, "ceil_down_bps": 2, "ceil_up_bps": 2 },
                      "match": { "vni": 100, "vlan": 64 } }
                ]
            }"#,
        )
        .expect("valid device-less vxlan+vlan file");
        let bundle = file.into_imported_bundle("test/libreqos_network");
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

//! `tunnel_circuits.json` ingestion (spec v1.1): the flat-file provisioning
//! contract for the MDU beta. The ISP writes one full-state snapshot; this
//! module owns the schema, the whole-file fail-loud validation, the
//! last-good snapshot persistence, and the derivation of the device-less
//! shaping circuits + their per-site canonical nodes.
//!
//! Classification is `(vni, vlan)` ONLY — no IP fields, ever (spec §2). The
//! derived circuits carry a [`TunnelIdentity`] and no devices; the compiler
//! emits a `MapTunnel` for each and the site nodes group them in E2.

use crate::{
    Config, TopologyCanonicalNode, TopologyShapingCircuitInput, TopologyShapingResolutionSource,
    TunnelIdentity,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use thiserror::Error;

/// Watched input filename, relative to `config.lqos_directory`.
pub const TUNNEL_CIRCUITS_FILENAME: &str = "tunnel_circuits.json";
/// Validated last-good snapshot filename, under the topology state directory.
pub const TUNNEL_CIRCUITS_SNAPSHOT_FILENAME: &str = "tunnel_circuits_snapshot.json";
/// Maximum accepted input size (spec §2 BOUNDS).
pub const TUNNEL_CIRCUITS_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Maximum accepted circuit count (spec §2 BOUNDS).
pub const TUNNEL_CIRCUITS_MAX_ENTRIES: usize = 8192;
/// The only accepted schema version (spec §2).
pub const TUNNEL_CIRCUITS_SCHEMA_VERSION: u32 = 1;
/// Namespace prefix for a derived site node.
pub const TUNNEL_SITE_ID_PREFIX: &str = "tunnel-circuits:site:";
/// Namespace prefix for a derived circuit id.
pub const TUNNEL_CIRCUIT_ID_PREFIX: &str = "tunnel-circuits:circuit:";
/// VXLAN VNI upper bound (24-bit space; `u32::MAX` is the classifier sentinel).
pub const TUNNEL_VNI_MAX: u32 = 16_777_214;
/// VLAN upper bound (a REAL VID; 0 and absent mean "no VLAN" to the classifier).
pub const TUNNEL_VLAN_MAX: u16 = 4094;
/// Plan-rate upper bound (spec §2).
pub const TUNNEL_PLAN_MBPS_MAX: u64 = 100_000;

/// Errors returned while reading, validating, or writing tunnel circuits.
#[derive(Debug, Error)]
pub enum TunnelCircuitsError {
    /// Reading or writing a file failed.
    #[error("Unable to access tunnel circuits file: {0}")]
    Io(#[from] std::io::Error),
    /// Parsing the JSON failed.
    #[error("Unable to parse tunnel circuits JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The whole file failed validation; the string names the first offender.
    #[error("tunnel circuits rejected: {0}")]
    Validation(String),
    /// The watched path is a symlink — rejected loud, never followed.
    #[error("tunnel circuits path is a symlink; rejected (regular file required)")]
    Symlink,
    /// The input exceeds the size bound.
    #[error("tunnel circuits file is {size} bytes, exceeding the {max}-byte bound")]
    TooLarge {
        /// Observed size in bytes.
        size: u64,
        /// Configured maximum.
        max: u64,
    },
    /// Updating the canonical site nodes failed.
    #[error("canonical topology state update failed: {0}")]
    Canonical(String),
}

/// One provisioning entry: `(vni, vlan) -> subscriber circuit`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TunnelCircuitEntry {
    /// 24-bit VXLAN VNI (1..=16,777,214).
    pub vni: u32,
    /// Inner VLAN id (1..=4094).
    pub vlan: u16,
    /// Stable subscriber identifier; becomes the circuit id's suffix.
    pub subscriber_id: String,
    /// Optional human label for E2/UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// MDU/site id slug; becomes the site node id's suffix (spec v1.2 charset).
    pub parent_site: String,
    /// Optional human-readable site label for E2/UI (spec v1.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_display_name: Option<String>,
    /// Plan download rate in Mbps (integer-only, 1..=100000).
    pub plan_down_mbps: u64,
    /// Plan upload rate in Mbps (integer-only, 1..=100000).
    pub plan_up_mbps: u64,
}

/// The full-state provisioning file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TunnelCircuitsFile {
    /// Schema version; must equal [`TUNNEL_CIRCUITS_SCHEMA_VERSION`].
    #[serde(default = "default_tunnel_circuits_schema_version")]
    pub schema_version: u32,
    /// Explicit opt-in for an intentional empty snapshot (spec §3, #12-2).
    #[serde(default)]
    pub allow_empty: bool,
    /// Optional informational RFC3339 timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    /// The complete desired set of tunnel circuits.
    #[serde(default)]
    pub circuits: Vec<TunnelCircuitEntry>,
}

fn default_tunnel_circuits_schema_version() -> u32 {
    TUNNEL_CIRCUITS_SCHEMA_VERSION
}

impl Default for TunnelCircuitsFile {
    fn default() -> Self {
        Self {
            schema_version: TUNNEL_CIRCUITS_SCHEMA_VERSION,
            allow_empty: false,
            generated_at: None,
            circuits: Vec::new(),
        }
    }
}

/// Returns the watched input path (`config.lqos_directory/tunnel_circuits.json`).
pub fn tunnel_circuits_input_path(config: &Config) -> PathBuf {
    config.legacy_runtime_file_path(TUNNEL_CIRCUITS_FILENAME)
}

/// Returns the validated last-good snapshot path.
pub fn tunnel_circuits_snapshot_path(config: &Config) -> PathBuf {
    config.topology_state_file_path(TUNNEL_CIRCUITS_SNAPSHOT_FILENAME)
}

/// Namespaced canonical node id for a parent-site name.
pub fn tunnel_site_node_id(parent_site: &str) -> String {
    format!("{TUNNEL_SITE_ID_PREFIX}{parent_site}")
}

/// Namespaced shaping circuit id for a subscriber identifier.
pub fn tunnel_circuit_id(subscriber_id: &str) -> String {
    format!("{TUNNEL_CIRCUIT_ID_PREFIX}{subscriber_id}")
}

/// Spec §2 CHARSET: `[A-Za-z0-9._-]{1,64}`, no leading/trailing `.` or `-`.
fn valid_namespaced_token(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return false;
    }
    !matches!(bytes[0], b'.' | b'-') && !matches!(bytes[bytes.len() - 1], b'.' | b'-')
}

impl TunnelCircuitsFile {
    /// Validates the whole file (spec §3). The first violation rejects the
    /// file and names the offending field; never partial, never silent.
    pub fn validate(&self) -> Result<(), TunnelCircuitsError> {
        if self.schema_version != TUNNEL_CIRCUITS_SCHEMA_VERSION {
            return Err(TunnelCircuitsError::Validation(format!(
                "schema_version {} is not supported (expected {})",
                self.schema_version, TUNNEL_CIRCUITS_SCHEMA_VERSION
            )));
        }
        if self.circuits.is_empty() && !self.allow_empty {
            return Err(TunnelCircuitsError::Validation(
                "circuits is empty; refusing to wipe all tunnel circuits unless \"allow_empty\": true is set"
                    .to_string(),
            ));
        }
        if self.circuits.len() > TUNNEL_CIRCUITS_MAX_ENTRIES {
            return Err(TunnelCircuitsError::Validation(format!(
                "circuits count {} exceeds the {TUNNEL_CIRCUITS_MAX_ENTRIES} bound",
                self.circuits.len()
            )));
        }
        let mut seen_pairs = HashSet::new();
        let mut seen_subscribers = HashSet::new();
        for (index, entry) in self.circuits.iter().enumerate() {
            if entry.vni == 0 || entry.vni > TUNNEL_VNI_MAX {
                return Err(TunnelCircuitsError::Validation(format!(
                    "circuits[{index}].vni {} is out of range 1..={TUNNEL_VNI_MAX}",
                    entry.vni
                )));
            }
            if entry.vlan == 0 || entry.vlan > TUNNEL_VLAN_MAX {
                return Err(TunnelCircuitsError::Validation(format!(
                    "circuits[{index}].vlan {} is out of range 1..={TUNNEL_VLAN_MAX}",
                    entry.vlan
                )));
            }
            if !valid_namespaced_token(&entry.subscriber_id) {
                return Err(TunnelCircuitsError::Validation(format!(
                    "circuits[{index}].subscriber_id {:?} must match [A-Za-z0-9._-]{{1,64}} with no leading/trailing '.' or '-'",
                    entry.subscriber_id
                )));
            }
            if !valid_namespaced_token(&entry.parent_site) {
                return Err(TunnelCircuitsError::Validation(format!(
                    "circuits[{index}].parent_site {:?} must match [A-Za-z0-9._-]{{1,64}} with no leading/trailing '.' or '-'",
                    entry.parent_site
                )));
            }
            if entry.plan_down_mbps < 1 || entry.plan_down_mbps > TUNNEL_PLAN_MBPS_MAX {
                return Err(TunnelCircuitsError::Validation(format!(
                    "circuits[{index}].plan_down_mbps {} is out of range 1..={TUNNEL_PLAN_MBPS_MAX}",
                    entry.plan_down_mbps
                )));
            }
            if entry.plan_up_mbps < 1 || entry.plan_up_mbps > TUNNEL_PLAN_MBPS_MAX {
                return Err(TunnelCircuitsError::Validation(format!(
                    "circuits[{index}].plan_up_mbps {} is out of range 1..={TUNNEL_PLAN_MBPS_MAX}",
                    entry.plan_up_mbps
                )));
            }
            if !seen_pairs.insert((entry.vni, entry.vlan)) {
                return Err(TunnelCircuitsError::Validation(format!(
                    "duplicate (vni, vlan) pair ({}, {}) in circuits[{index}]",
                    entry.vni, entry.vlan
                )));
            }
            if !seen_subscribers.insert(entry.subscriber_id.as_str()) {
                return Err(TunnelCircuitsError::Validation(format!(
                    "duplicate subscriber_id {:?} in circuits[{index}]",
                    entry.subscriber_id
                )));
            }
        }
        Ok(())
    }

    /// Parses and validates an in-memory payload (size checked by the caller).
    pub fn parse_and_validate(bytes: &[u8]) -> Result<Self, TunnelCircuitsError> {
        let file: Self = serde_json::from_slice(bytes)?;
        file.validate()?;
        Ok(file)
    }

    /// Loads the validated last-good snapshot, if present.
    ///
    /// Side effects: reads the snapshot file from the topology state directory.
    pub fn load_snapshot(config: &Config) -> Result<Option<Self>, TunnelCircuitsError> {
        let path = tunnel_circuits_snapshot_path(config);
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read(path)?;
        Ok(Some(Self::parse_and_validate(&raw)?))
    }

    /// Saves the validated snapshot atomically.
    ///
    /// Side effects: writes the snapshot file into the topology state directory.
    pub fn save_snapshot(&self, config: &Config) -> Result<(), TunnelCircuitsError> {
        let path = tunnel_circuits_snapshot_path(config);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        let temp_path = path.with_extension("tmp");
        std::fs::write(&temp_path, raw.as_bytes())?;
        std::fs::rename(&temp_path, &path)?;
        Ok(())
    }

    /// Distinct, sorted `parent_site` slugs referenced by the snapshot.
    pub fn site_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .circuits
            .iter()
            .map(|entry| entry.parent_site.clone())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Maps each distinct `parent_site` slug to its display label: the first
    /// non-empty `site_display_name` in file order, else the slug itself.
    pub fn site_display_labels(&self) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        for entry in &self.circuits {
            labels
                .entry(entry.parent_site.clone())
                .or_insert_with(|| {
                    entry
                        .site_display_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(entry.parent_site.as_str())
                        .to_string()
                });
        }
        labels
    }

    /// The canonical site nodes this snapshot owns (one per distinct `parent_site`).
    pub fn site_nodes(&self) -> Vec<TopologyCanonicalNode> {
        self.site_display_labels()
            .into_iter()
            .map(|(slug, label)| TopologyCanonicalNode {
                node_id: tunnel_site_node_id(&slug),
                node_name: label,
                node_kind: "site".to_string(),
                ..TopologyCanonicalNode::default()
            })
            .collect()
    }

    /// Derives the device-less shaping circuits (commit == ceil; symmetrical v1).
    pub fn to_shaping_circuits(&self) -> Vec<TopologyShapingCircuitInput> {
        let labels = self.site_display_labels();
        let mut circuits: Vec<TopologyShapingCircuitInput> = self
            .circuits
            .iter()
            .map(|entry| {
                let site_id = tunnel_site_node_id(&entry.parent_site);
                let site_label = labels
                    .get(&entry.parent_site)
                    .cloned()
                    .unwrap_or_else(|| entry.parent_site.clone());
                let down = entry.plan_down_mbps as f32;
                let up = entry.plan_up_mbps as f32;
                TopologyShapingCircuitInput {
                    circuit_id: tunnel_circuit_id(&entry.subscriber_id),
                    circuit_name: entry
                        .display_name
                        .clone()
                        .unwrap_or_else(|| entry.subscriber_id.clone()),
                    anchor_node_id: Some(site_id.clone()),
                    anchor_node_name: Some(site_label.clone()),
                    logical_parent_node_name: None,
                    logical_parent_node_id: None,
                    effective_parent_node_name: site_label,
                    effective_parent_node_id: site_id,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                    resolution_source: TopologyShapingResolutionSource::TopologyAnchor,
                    download_min_mbps: down,
                    upload_min_mbps: up,
                    download_max_mbps: down,
                    upload_max_mbps: up,
                    comment: String::new(),
                    sqm_override: None,
                    tunnel_identity: Some(TunnelIdentity {
                        vni: entry.vni,
                        vlan: entry.vlan,
                    }),
                    vlan_identity: None,
                    mpls_identity: None,
                    devices: Vec::new(),
                }
            })
            .collect();
        circuits.sort_unstable_by(|left, right| left.circuit_id.cmp(&right.circuit_id));
        circuits
    }

    /// Merges this snapshot's site nodes into `nodes`, replacing any prior
    /// `tunnel-circuits:site:*` nodes and preserving everything else.
    ///
    /// Returns `true` when the node list changed.
    pub fn merge_site_nodes(&self, nodes: &mut Vec<TopologyCanonicalNode>) -> bool {
        let existing_tunnel: Vec<&TopologyCanonicalNode> = nodes
            .iter()
            .filter(|node| node.node_id.starts_with(TUNNEL_SITE_ID_PREFIX))
            .collect();
        let wanted = self.site_nodes();
        let unchanged = existing_tunnel.len() == wanted.len()
            && wanted.iter().all(|node| {
                existing_tunnel.iter().any(|existing| {
                    existing.node_id == node.node_id && existing.node_name == node.node_name
                })
            });
        if unchanged {
            return false;
        }
        nodes.retain(|node| !node.node_id.starts_with(TUNNEL_SITE_ID_PREFIX));
        let mut wanted = wanted;
        nodes.append(&mut wanted);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(vni: u32, vlan: u16, subscriber: &str, site: &str) -> TunnelCircuitEntry {
        TunnelCircuitEntry {
            vni,
            vlan,
            subscriber_id: subscriber.to_string(),
            display_name: None,
            parent_site: site.to_string(),
            site_display_name: None,
            plan_down_mbps: 500,
            plan_up_mbps: 500,
        }
    }

    fn valid_file() -> TunnelCircuitsFile {
        TunnelCircuitsFile {
            schema_version: 1,
            allow_empty: false,
            generated_at: None,
            circuits: vec![
                entry(1010, 64, "SUB-1", "Riverside_MDU_7"),
                entry(1011, 65, "SUB-2", "Riverside_MDU_7"),
                entry(1012, 66, "SUB-3", "Lakeside_MDU_2"),
            ],
        }
    }

    #[test]
    fn valid_file_passes() {
        assert!(valid_file().validate().is_ok());
    }

    #[test]
    fn wrong_schema_version_rejects() {
        let mut file = valid_file();
        file.schema_version = 2;
        assert!(matches!(
            file.validate(),
            Err(TunnelCircuitsError::Validation(_))
        ));
    }

    #[test]
    fn empty_circuits_rejects_without_allow_empty() {
        let mut file = valid_file();
        file.circuits.clear();
        assert!(file.validate().is_err());
        file.allow_empty = true;
        assert!(file.validate().is_ok());
    }

    #[test]
    fn out_of_range_vni_and_vlan_reject() {
        let mut file = valid_file();
        file.circuits[0].vni = 0;
        assert!(file.validate().is_err());
        let mut file = valid_file();
        file.circuits[0].vni = TUNNEL_VNI_MAX + 1;
        assert!(file.validate().is_err());
        let mut file = valid_file();
        file.circuits[0].vlan = 0;
        assert!(file.validate().is_err());
        let mut file = valid_file();
        file.circuits[0].vlan = TUNNEL_VLAN_MAX + 1;
        assert!(file.validate().is_err());
    }

    #[test]
    fn duplicate_pair_and_subscriber_reject() {
        let mut file = valid_file();
        file.circuits[1].vni = file.circuits[0].vni;
        file.circuits[1].vlan = file.circuits[0].vlan;
        assert!(file.validate().is_err());

        let mut file = valid_file();
        file.circuits[1].subscriber_id = file.circuits[0].subscriber_id.clone();
        assert!(file.validate().is_err());
    }

    #[test]
    fn charset_violations_reject() {
        for bad in ["", " leading", "trailing ", ".leading", "trailing-", "a b", "a/b"] {
            let mut file = valid_file();
            file.circuits[0].subscriber_id = bad.to_string();
            assert!(
                file.validate().is_err(),
                "subscriber_id {bad:?} should reject"
            );
        }
        let mut file = valid_file();
        file.circuits[0].parent_site = "Riverside/MDU".to_string();
        assert!(file.validate().is_err());
    }

    #[test]
    fn plan_rate_bounds_reject() {
        let mut file = valid_file();
        file.circuits[0].plan_down_mbps = 0;
        assert!(file.validate().is_err());
        let mut file = valid_file();
        file.circuits[0].plan_up_mbps = TUNNEL_PLAN_MBPS_MAX + 1;
        assert!(file.validate().is_err());
    }

    #[test]
    fn non_integer_plan_rate_rejects_at_parse() {
        let raw = br#"{"schema_version":1,"circuits":[{"vni":1,"vlan":1,"subscriber_id":"S","parent_site":"P","plan_down_mbps":1.5,"plan_up_mbps":1}]}"#;
        assert!(matches!(
            TunnelCircuitsFile::parse_and_validate(raw),
            Err(TunnelCircuitsError::Json(_))
        ));
    }

    #[test]
    fn derived_circuits_are_device_less_and_anchored() {
        let circuits = valid_file().to_shaping_circuits();
        assert_eq!(circuits.len(), 3);
        let first = circuits
            .iter()
            .find(|circuit| circuit.circuit_id.ends_with("SUB-1"))
            .unwrap();
        assert_eq!(first.circuit_id, "tunnel-circuits:circuit:SUB-1");
        assert_eq!(
            first.effective_parent_node_id,
            "tunnel-circuits:site:Riverside_MDU_7"
        );
        assert!(first.devices.is_empty());
        assert_eq!(
            first.tunnel_identity,
            Some(TunnelIdentity { vni: 1010, vlan: 64 })
        );
        assert_eq!(first.download_min_mbps, 500.0);
        assert_eq!(first.download_max_mbps, 500.0);
    }

    #[test]
    fn site_display_name_labels_the_node_while_parent_site_stays_the_id() {
        let mut file = valid_file();
        for entry in file.circuits.iter_mut().take(2) {
            entry.parent_site = "riverside-mdu-7".to_string();
        }
        file.circuits[0].site_display_name = Some("Riverside MDU 7".to_string());

        let node = file
            .site_nodes()
            .into_iter()
            .find(|node| node.node_id == "tunnel-circuits:site:riverside-mdu-7")
            .expect("slug site node");
        assert_eq!(node.node_name, "Riverside MDU 7");

        let circuit = file
            .to_shaping_circuits()
            .into_iter()
            .find(|circuit| circuit.circuit_id.ends_with("SUB-2"))
            .unwrap();
        assert_eq!(
            circuit.effective_parent_node_id,
            "tunnel-circuits:site:riverside-mdu-7"
        );
        assert_eq!(circuit.effective_parent_node_name, "Riverside MDU 7");

        // A display-label change is a real change (merge is not a no-op).
        let mut nodes = Vec::new();
        assert!(file.merge_site_nodes(&mut nodes));
        assert!(!file.merge_site_nodes(&mut nodes));
        file.circuits[0].site_display_name = Some("Riverside MDU Seven".to_string());
        assert!(file.merge_site_nodes(&mut nodes));
    }

    #[test]
    fn site_nodes_are_distinct_and_merge_replaces_only_tunnel_sites() {
        let file = valid_file();
        let mut nodes = vec![
            TopologyCanonicalNode {
                node_id: "uisp:site:keep".to_string(),
                node_name: "Keep".to_string(),
                ..TopologyCanonicalNode::default()
            },
            TopologyCanonicalNode {
                node_id: "tunnel-circuits:site:Stale".to_string(),
                node_name: "Stale".to_string(),
                ..TopologyCanonicalNode::default()
            },
        ];
        assert!(file.merge_site_nodes(&mut nodes));
        assert!(nodes.iter().any(|node| node.node_id == "uisp:site:keep"));
        assert!(!nodes.iter().any(|node| node.node_name == "Stale"));
        assert_eq!(
            nodes
                .iter()
                .filter(|node| node.node_id.starts_with(TUNNEL_SITE_ID_PREFIX))
                .count(),
            2
        );
    }
}

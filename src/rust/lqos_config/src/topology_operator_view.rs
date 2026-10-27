//! T5 rung 1 (t5_topology_manager_consolidation_derivation.md): E2 —
//! the operator/tree view artifact. One of the manager's TWO exports:
//! E1 (the dataplane transaction) stays the frozen bridge wire; E2 is
//! the FULL operator catalog — a FLAT, id-sorted, self-describing node
//! list (ruling 2) that keeps circuit-less sites (the F-LM §8-4
//! asymmetry resolved as two honest projections) and carries devices
//! INLINE on circuit nodes so circuit identity renders without
//! ShapedDevices.csv (F10, the ~95% UISP-primary zero-CSV world).
//! Determinism is load-bearing: nodes are a Vec sorted by id and no
//! HashMap iteration order ever reaches the file — the same input
//! MUST re-emit byte-identically.

use crate::topology_runtime_state::{TopologyRuntimeStateError, TunnelIdentity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Runtime filename carrying the operator/tree view (E2).
pub const TOPOLOGY_OPERATOR_VIEW_FILENAME: &str = "topology_operator_view.json";

/// One device record inlined on a circuit node (ruling 2: the node
/// records ARE the resolution table — no separate join table).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OperatorViewDevice {
    /// The integration's stable device id.
    pub device_id: String,
    /// The human device name.
    #[serde(default)]
    pub device_name: String,
    /// IPv4 assignments (CIDR or bare).
    #[serde(default)]
    pub ipv4: Vec<String>,
    /// IPv6 assignments (CIDR or bare).
    #[serde(default)]
    pub ipv6: Vec<String>,
    /// The device MAC, when the integration supplies one.
    #[serde(default)]
    pub mac: String,
    /// Circuit download commit (Mbps).
    #[serde(default)]
    pub download_min_mbps: u64,
    /// Circuit upload commit (Mbps).
    #[serde(default)]
    pub upload_min_mbps: u64,
    /// Circuit download ceil (Mbps).
    #[serde(default)]
    pub download_max_mbps: u64,
    /// Circuit upload ceil (Mbps).
    #[serde(default)]
    pub upload_max_mbps: u64,
}

/// Rung 1.5 §1.5: where an E2 node's effective rates came from. The
/// suspend marker is DISPLAY-only — a suspended circuit renders
/// `SUSPENDED`, never the broker floor as a rate.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OperatorRateSource {
    /// No operator rate override applied.
    #[default]
    Base,
    /// An operator rate override applied.
    Override,
    /// The suspend sentinel — the circuit is suspended.
    SuspendSentinel,
    /// A dynamic (RADIUS/StormGuard) circuit; reserved for K1.
    Dynamic,
}

/// One node of the operator view. `id` is the source-native namespaced
/// stable id (ruling 1 / F9); `name` is the display name (dedupe
/// applied — names may collide, ids may not).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OperatorViewNode {
    /// The source-native namespaced stable id (ruling 1 / F9).
    pub id: String,
    /// The deduped human display name (T4's split, generalized).
    pub name: String,
    /// The parent node's stable id; `None`/empty = root-level.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// `site` | `ap` | `circuit` (lowercase store convention).
    #[serde(rename = "type")]
    pub node_type: String,
    /// Effective download ceil (Mbps), when known.
    #[serde(default)]
    pub download_mbps: Option<f64>,
    /// Effective upload ceil (Mbps), when known.
    #[serde(default)]
    pub upload_mbps: Option<f64>,
    /// Effective download commit (Mbps), when known.
    #[serde(default)]
    pub commit_download_mbps: Option<f64>,
    /// Effective upload commit (Mbps), when known.
    #[serde(default)]
    pub commit_upload_mbps: Option<f64>,
    /// Inline devices — circuit nodes only (ruling 2 / F10).
    #[serde(default)]
    pub devices: Vec<OperatorViewDevice>,
    /// Optional geographic latitude (site nodes), carried from the canonical
    /// model so the site map renders. Producer-agnostic: uisp + the python
    /// integration family + the legacy import all supply it upstream.
    ///
 /// (a) /: `f64`, NOT `f32` — an f32 in a
    /// serde_json `Value` does not round-trip through JSON (a 1-ULP loss), which
    /// made `to_value(view)` hash != the persisted file hash (the persistent
    /// `effective_mismatch`). f64 is JSON-exact; the builder widens losslessly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    /// Optional geographic longitude (site nodes), carried from the canonical
    /// model so the site map renders. `f64` for JSON round-trip fidelity — see
    /// [`Self::latitude`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    /// T-arc §2.8: the circuit's tunnel classification identity, when the
    /// ingestion adapter supplies one (circuit nodes only). Additive and
    /// skipped when absent, so a node without one re-emits byte-identically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel_identity: Option<TunnelIdentity>,
    /// Rung 1.5 §1.5: the origin of the effective rates (display marker).
    #[serde(default, skip_serializing_if = "operator_rate_source_is_base")]
    pub rate_source: OperatorRateSource,
}

/// Serde skip helper: the base marker is omitted for byte-stable re-emits.
fn operator_rate_source_is_base(source: &OperatorRateSource) -> bool {
    matches!(source, OperatorRateSource::Base)
}

impl OperatorViewNode {
    /// Circuit nodes carry the operator catalog inline (F10).
    pub fn is_circuit(&self) -> bool {
        self.node_type.eq_ignore_ascii_case("circuit")
    }
}

/// The E2 artifact payload.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TopologyOperatorViewFile {
    /// The E2 payload schema version.
    pub schema_version: u32,
    /// The topology source generation this view was published from.
    #[serde(default)]
    pub source_generation: String,
    /// Id-sorted flat node list (determinism pin — never a map).
    #[serde(default)]
    pub nodes: Vec<OperatorViewNode>,
    /// Raw base display names claimed by more than one node — a
    /// catalog reference to the bare name misses LOUDLY (T4 pin 2's
    /// loud-miss doctrine travels with the operator view).
    #[serde(default)]
    pub ambiguous_base_names: BTreeSet<String>,
}

impl TopologyOperatorViewFile {
    /// Loads the operator view if it exists; a missing file is the
    /// honest-empty posture (the tree store starts structure-less).
    pub fn load(config: &crate::Config) -> Result<Self, TopologyRuntimeStateError> {
        let path = topology_operator_view_path(config);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Saves the operator view atomically.
    pub fn save(&self, config: &crate::Config) -> Result<(), TopologyRuntimeStateError> {
        let path = topology_operator_view_path(config);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        let temp_path = path.with_extension("tmp");
        std::fs::write(&temp_path, raw)?;
        std::fs::rename(&temp_path, &path)?;
        Ok(())
    }

    /// Byte-stability helper for the dedup-write arm: true when the
    /// currently persisted view is semantically identical.
    pub fn persisted_equals(&self, config: &crate::Config) -> bool {
        // Compare the ON-DISK JSON VALUE to this struct's serialization, NOT a
        // re-deserialized struct: an out-of-band edit that ADDS/REMOVES a key
        // deserializes cleanly (serde ignores unknown fields) and would leave
        // the struct equal — so the publish would SKIP the write and the
        // tampered operator view would persist (the republish-then-verify
        // EffectiveMismatch). A value compare heals it.
        let Ok(raw) = std::fs::read_to_string(topology_operator_view_path(config)) else {
            return false;
        };
        let Ok(disk) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return false;
        };
        let Ok(mine) = serde_json::to_value(self) else {
            return false;
        };
        disk == mine
    }

    /// Id lookup for join consumers (flat list — id-sorted, binary
    /// searchable, but a linear scan is fine at operator scales).
    pub fn node_by_id(&self, id: &str) -> Option<&OperatorViewNode> {
        self.nodes.iter().find(|node| node.id == id)
    }
}

/// Returns the path of the operator view artifact.
pub fn topology_operator_view_path(config: &crate::Config) -> PathBuf {
    config.topology_state_read_path(TOPOLOGY_OPERATOR_VIEW_FILENAME)
}

#[cfg(test)]
mod operator_view_round_trip_tests {
    use super::*;
    use crate::topology_runtime_state::{
        TunnelIdentity, compute_effective_network_generation,
    };
    use serde_json::Value;

    /// First differing JSON path, or `None` when the two values are equal.
    fn first_diff(a: &Value, b: &Value, path: &str) -> Option<String> {
        match (a, b) {
            (Value::Object(am), Value::Object(bm)) => {
                for key in am.keys().chain(bm.keys()) {
                    let (pa, pb) = (am.get(key), bm.get(key));
                    if pa == pb {
                        continue;
                    }
                    return match (pa, pb) {
                        (Some(x), Some(y)) => {
                            first_diff(x, y, &format!("{path}.{key}"))
                                .or(Some(format!("{path}.{key}")))
                        }
                        _ => Some(format!("{path}.{key} (present on one side only)")),
                    };
                }
                None
            }
            (Value::Array(aa), Value::Array(ba)) => {
                if aa.len() != ba.len() {
                    return Some(format!("{path} (len {} vs {})", aa.len(), ba.len()));
                }
                for (index, (x, y)) in aa.iter().zip(ba.iter()).enumerate() {
                    if x != y {
                        return first_diff(x, y, &format!("{path}[{index}]"));
                    }
                }
                None
            }
            _ => (a != b).then(|| path.to_string()),
        }
    }

    fn representative_view() -> TopologyOperatorViewFile {
        TopologyOperatorViewFile {
            schema_version: 1,
            source_generation: "src-1".to_string(),
            nodes: vec![
                OperatorViewNode {
                    id: "site-1".to_string(),
                    name: "Site 1".to_string(),
                    parent_id: None,
                    node_type: "site".to_string(),
                    download_mbps: Some(1000.0),
                    upload_mbps: Some(1000.0),
                    commit_download_mbps: Some(500.5),
                    commit_upload_mbps: Some(500.5),
                    devices: Vec::new(),
                    // f64 (JSON-exact) — see the R-GUARD test for the ULP case.
                    latitude: Some(51.5074_f64),
                    longitude: Some(-0.1278_f64),
                    tunnel_identity: None,
                    rate_source: OperatorRateSource::Base,
                },
                OperatorViewNode {
                    id: "c-1".to_string(),
                    name: "Circuit 1".to_string(),
                    parent_id: Some("site-1".to_string()),
                    node_type: "circuit".to_string(),
                    download_mbps: Some(100.0),
                    upload_mbps: Some(100.0),
                    commit_download_mbps: Some(50.5),
                    commit_upload_mbps: Some(50.5),
                    devices: vec![OperatorViewDevice {
                        device_id: "dev-1".to_string(),
                        device_name: "Device 1".to_string(),
                        ipv4: vec!["192.0.2.1/32".to_string()],
                        ipv6: Vec::new(),
                        mac: "aa:bb:cc:dd:ee:ff".to_string(),
                        download_min_mbps: 50,
                        upload_min_mbps: 50,
                        download_max_mbps: 100,
                        upload_max_mbps: 100,
                    }],
                    latitude: None,
                    longitude: None,
                    tunnel_identity: Some(TunnelIdentity { vni: 100, vlan: 10 }),
                    rate_source: OperatorRateSource::Override,
                },
            ],
            ambiguous_base_names: std::collections::BTreeSet::new(),
        }
    }

 /// R3; the EXACT publish pipeline is
    /// `to_value(&view) -> to_string_pretty -> from_str::<Value>` then the
    /// content hash. A non-identity here NAMES the lossy `nodes` sub-field
    /// OFF-box, before any `.12` re-run. PASS => the file != operator_view_value
    /// (a 2nd writer / path mismatch) instead.
    #[test]
    fn operator_view_value_json_round_trip_is_identity() {
        let view = representative_view();
        let value = serde_json::to_value(&view).expect("to_value");
        let raw = serde_json::to_string_pretty(&value).expect("pretty");
        let parsed: Value = serde_json::from_str(&raw).expect("parse");
        assert_eq!(
            first_diff(&value, &parsed, "$"),
            None,
            "operator-view Value JSON round-trip must be identity"
        );
        assert_eq!(
            compute_effective_network_generation(&value).expect("hash value"),
            compute_effective_network_generation(&parsed).expect("hash parsed"),
            "equal Values must hash equal"
        );
    }

 /// R-GUARD (the implementer ``, binding): the persisted form's hash MUST
    /// equal the in-memory `to_value` hash — FAILS-CLOSED if a future lossy type
    /// (e.g. an `f32` field) enters. The view carries a ULP-FAILING f32
    /// magnitude widened to f64: as an `f32` field it is non-identity under
    /// `to_value -> pretty -> parse`; as `f64` it is JSON-exact.
    #[test]
    fn operator_view_generation_is_round_trip_stable_with_ulp_failing_magnitude() {
        let ulp_failing = f64::from(f32::from_bits(0x0001_0001));
        let mut view = representative_view();
        view.nodes[0].latitude = Some(ulp_failing);
        view.nodes[0].longitude = Some(-ulp_failing);
        let value = serde_json::to_value(&view).expect("to_value");
        let raw = serde_json::to_string_pretty(&value).expect("pretty");
        let parsed: Value = serde_json::from_str(&raw).expect("parse");
        assert_eq!(
            first_diff(&value, &parsed, "$"),
            None,
            "the persisted operator view must equal the in-memory Value (R-GUARD)"
        );
        assert_eq!(
            compute_effective_network_generation(&value).expect("hash value"),
            compute_effective_network_generation(&parsed).expect("hash parsed"),
            "the effective_generation must be round-trip-stable (R-GUARD)"
        );
    }
}

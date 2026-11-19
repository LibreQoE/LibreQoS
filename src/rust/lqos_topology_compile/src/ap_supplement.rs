//! AP/OLT supplement stage at the shared import choke point.
//!
//! The lqosd poller publishes a generation-stamped `ap_observations.json`
//! beside `dataplane_transaction.json`. This stage joins those observations to
//! the imported circuits by router MAC and applies observable placement and
//! capacity to the imported topology bundle before the compiler projects it.
//!
//! Every failure is fail-open: a missing, malformed, stale, or
//! generation-mismatched observation file is a no-op, and an unexpected error
//! is logged once while the unsupplemented document is compiled.

use crate::ImportedTopologyBundle;
use lqos_config::{CircuitAnchor, Config};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

/// Name of the generation-stamped observation file written by the poller.
pub const AP_OBSERVATIONS_FILENAME: &str = "ap_observations.json";
/// Name of the supplement report written beside the observations.
pub const AP_SUPPLEMENT_REPORT_FILENAME: &str = "ap_supplement_report.json";
/// Current observation-file schema version.
pub const AP_OBSERVATIONS_SCHEMA_VERSION: u32 = 1;

/// Observation record parent keys, in precedence order.
const PLACEMENT_PARENT_KEYS: [&str; 6] = [
    "parent_node_id",
    "onu_node_id",
    "onu",
    "pon_node_id",
    "pon",
    "placement_node_id",
];

/// The poller's `ap_observations.json` document (schema v1).
///
/// The writer owns the schema; this reader accepts the same JSON and treats
/// an unknown extra field as non-fatal.
#[derive(Clone, Debug, Deserialize)]
pub struct ApObservations {
    /// Schema version; only [`AP_OBSERVATIONS_SCHEMA_VERSION`] is accepted.
    pub schema_version: u32,
    /// Compiled-topology generation the sweep was taken against.
    pub topology_generation: String,
    /// Per-device observation records.
    pub devices: BTreeMap<String, Value>,
}

/// Operator pins. Presence of a placement pin suppresses the observed move;
/// presence of a capacity pin suppresses the polled capacity proposal.
#[derive(Clone, Debug, Default)]
pub struct SupplementPins {
    /// `circuit_id -> parent_node_id`.
    pub placement: HashMap<String, String>,
    /// Node ids with an operator capacity pin.
    pub capacity: BTreeSet<String>,
}

/// One applied circuit move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacementEdit {
    /// Circuit moved.
    pub circuit_id: String,
    /// Anchor before the move (empty when unknown).
    pub old_parent: String,
    /// New anchor node id.
    pub new_parent: String,
    /// New anchor display name.
    pub new_parent_name: String,
    /// Edit source; currently always `observed`.
    pub source: &'static str,
}

/// One capacity proposal and its decision.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CapacityEntry {
    /// Observation node id.
    pub node_id: String,
    /// Device model, when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Reported PHY rate in Mbps, or the poller's precomputed capacity when
    /// `capacity_source` is present.
    pub phy_mbps: f64,
    /// Resolved capacity factor, when one existed (legacy PHY path only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<f64>,
    /// `phy_mbps * factor` (legacy path) or the precomputed capacity
    /// (poller path), when computable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_mbps: Option<f64>,
    /// The poller's derivation source (`phy`, `per_sm_rate`, `crm`,
    /// `unknown`), when the observation carried one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity_source: Option<String>,
    /// `used`, `ignored_pin`, `ignored_crm_rate`, `ignored_no_factor`,
    /// `ignored_invalid_factor`, `ignored_invalid_capacity`, or
    /// `ignored_unknown_device`.
    pub decision: String,
}

/// One device's station inventory and its drift against the compiled
/// topology's pins. Display-only: nothing here changes placement.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StationInventoryEntry {
    /// Observation node id.
    pub node_id: String,
    /// Stations the AP reported this sweep.
    pub station_count: u64,
    /// Circuits the compiled topology currently pins under this node.
    pub crm_pinned_count: u64,
    /// `station_count - crm_pinned_count`; positive means the AP sees more
    /// stations than the CRM pins under it.
    pub delta: i64,
}

/// The pure planning result. Applying it to the bundle is infallible.
#[derive(Clone, Debug, Default)]
pub struct SupplementPlan {
    /// Number of observations that matched one circuit and one parent.
    pub matched: usize,
    /// Number of entries with no circuit match or no usable parent.
    pub unmatched: usize,
    /// Number of MACs/parents that could not be resolved uniquely.
    pub ambiguous: usize,
    /// Pinned circuits whose observed parent disagreed with the pin.
    pub pinned_overrides: usize,
    /// Pinned circuits whose observed parent differed from the pin.
    pub drift: usize,
    /// Moves to apply.
    pub edits: Vec<PlacementEdit>,
    /// One entry per device record that reported capacity.
    pub capacity: Vec<CapacityEntry>,
    /// Capacity entries applied.
    pub capacity_used: usize,
    /// Capacity entries left alone.
    pub capacity_ignored: usize,
    /// One entry per observed device with a station count. Display-only
    /// drift against the compiled topology's pins.
    pub station_inventory: Vec<StationInventoryEntry>,
}

#[derive(Serialize)]
struct ApSupplementReport {
    schema_version: u32,
    topology_generation: String,
    written_at: u64,
    matched: usize,
    unmatched: usize,
    ambiguous: usize,
    pinned_overrides: usize,
    drift: usize,
    applied_edits: Vec<AppliedEdit>,
    capacity_applied: Vec<AppliedCapacity>,
    capacity_ignored: Vec<CapacityEntry>,
    station_inventory: Vec<StationInventoryEntry>,
}

#[derive(Serialize)]
struct AppliedEdit {
    circuit_id: String,
    old_parent: String,
    new_parent: String,
    source: &'static str,
}

#[derive(Serialize)]
struct AppliedCapacity {
    node_id: String,
    proposed_mbps: f64,
}

#[derive(Clone, Debug)]
struct NodeInfo {
    name: String,
    in_tree: bool,
}

#[derive(Default)]
struct NodeIndex {
    nodes: HashMap<String, NodeInfo>,
}

impl NodeIndex {
    fn from_bundle(imported: &ImportedTopologyBundle) -> Self {
        let mut index = Self::default();
        if let Some(canonical) = imported.native_canonical.as_ref() {
            for node in &canonical.nodes {
                index
                    .nodes
                    .entry(node.node_id.clone())
                    .or_insert_with(|| NodeInfo {
                        name: node.node_name.clone(),
                        in_tree: false,
                    });
            }
        }
        if let Some(map) = imported.compatibility_network_json.as_object() {
            collect_node_names(map, &mut index.nodes);
        }
        index
    }
}

fn collect_node_names(map: &Map<String, Value>, out: &mut HashMap<String, NodeInfo>) {
    for (key, value) in map {
        let Some(node) = value.as_object() else {
            continue;
        };
        if let Some(node_id) = node.get("id").and_then(Value::as_str)
            && !node_id.trim().is_empty()
        {
            let name = node
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(key)
                .to_string();
            let entry = out.entry(node_id.to_string()).or_insert_with(|| NodeInfo {
                name: name.clone(),
                in_tree: false,
            });
            entry.in_tree = true;
            if entry.name.is_empty() {
                entry.name = name;
            }
        }
        if let Some(children) = node.get("children").and_then(Value::as_object) {
            collect_node_names(children, out);
        }
    }
}

/// Reads and validates the observation file.
///
/// Returns `None` for a disabled section, a missing/unreadable/malformed file,
/// a wrong schema version, an empty generation, or a generation mismatch
/// against the sibling `dataplane_transaction.json` generation (the value the
/// poller stamped). Never panics.
pub fn load_observations(config: &Config) -> Option<ApObservations> {
    if !config.ap_polling.enabled {
        return None;
    }
    let path = config.shaping_state_file_path(AP_OBSERVATIONS_FILENAME);
    let raw = std::fs::read_to_string(path).ok()?;
    let observations: ApObservations = serde_json::from_str(&raw).ok()?;
    if observations.schema_version != AP_OBSERVATIONS_SCHEMA_VERSION {
        return None;
    }
    if observations.topology_generation.trim().is_empty() {
        return None;
    }
    let expected = expected_generation(config)?;
    if observations.topology_generation != expected {
        return None;
    }
    Some(observations)
}

fn expected_generation(config: &Config) -> Option<String> {
    let path = lqos_config::topology_dataplane_transaction_path(config);
    let raw = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let generation = value.get("generation")?.as_str()?.trim();
    (!generation.is_empty()).then(|| generation.to_string())
}

/// Plans the supplement without mutating the bundle. Placement edits are
/// produced only when `[ap_polling].apply_placement` is true (default false:
/// v1 is capacity-only); capacity is always planned.
pub fn plan_supplement(
    imported: &ImportedTopologyBundle,
    observations: &ApObservations,
    pins: &SupplementPins,
    config: &Config,
) -> SupplementPlan {
    let index = NodeIndex::from_bundle(imported);
    let mut plan = SupplementPlan::default();

    // v1 is capacity-only: placement rewrites stay behind the default-off
    // switch, and the CRM `access_device` pin remains authoritative.
    if config.ap_polling.apply_placement {
        let circuit_by_mac = circuit_by_mac(imported);
        // Flatten station/ONU entries so duplicate MAC detection spans both lists
        // and all devices, matching the pre-5d planner.
        let mut observed: Vec<(String, Value, Value, String)> = Vec::new();
        for (node_id, record) in &observations.devices {
            if !record.is_object() {
                continue;
            }
            for list_name in ["stations", "onus"] {
                let Some(entries) = record.get(list_name).and_then(Value::as_array) else {
                    continue;
                };
                for entry in entries {
                    if !entry.is_object() {
                        continue;
                    }
                    let mac = entry_mac(entry);
                    if mac.is_empty() {
                        plan.unmatched += 1;
                        continue;
                    }
                    observed.push((node_id.clone(), record.clone(), entry.clone(), mac));
                }
            }
        }

        let mut by_mac: HashMap<&str, Vec<usize>> = HashMap::new();
        for (position, (_, _, _, mac)) in observed.iter().enumerate() {
            by_mac.entry(mac.as_str()).or_default().push(position);
        }
        let duplicate_macs: HashSet<&str> = by_mac
            .iter()
            .filter(|(_, positions)| positions.len() > 1)
            .map(|(mac, _)| *mac)
            .collect();
        plan.ambiguous += duplicate_macs.len();

        let mut matches: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (node_id, record, entry, mac) in &observed {
            if duplicate_macs.contains(mac.as_str()) {
                continue;
            }
            let Some(circuits) = circuit_by_mac.get(mac.as_str()) else {
                plan.unmatched += 1;
                continue;
            };
            if circuits.len() > 1 {
                plan.ambiguous += 1;
                continue;
            }
            let circuit_id = circuits.iter().next().expect("one circuit").clone();
            let Some(parent) = desired_parent(node_id, record, entry, &index) else {
                plan.unmatched += 1;
                continue;
            };
            matches.entry(circuit_id).or_default().push(parent);
        }

        for (circuit_id, parents) in matches {
            let desired: BTreeSet<&String> = parents.iter().collect();
            if desired.len() > 1 {
                plan.ambiguous += 1;
                continue;
            }
            let observed_parent = desired.iter().next().expect("one parent").to_string();
            plan.matched += parents.len();
            let old_parent = circuit_parent(imported, &circuit_id);
            if let Some(pin) = pins.placement.get(&circuit_id)
                && index.nodes.contains_key(pin)
            {
                if pin != &observed_parent || old_parent != pin.as_str() {
                    plan.pinned_overrides += 1;
                }
                if pin != &observed_parent {
                    plan.drift += 1;
                }
                // Pinned circuits are never moved by the supplement.
                continue;
            }
            if old_parent != observed_parent {
                let new_parent_name = index
                    .nodes
                    .get(&observed_parent)
                    .map(|node| node.name.clone())
                    .unwrap_or_else(|| observed_parent.clone());
                plan.edits.push(PlacementEdit {
                    circuit_id,
                    old_parent,
                    new_parent: observed_parent,
                    new_parent_name,
                    source: "observed",
                });
            }
        }
    }

    for (node_id, record) in &observations.devices {
        if !record.is_object() {
            continue;
        }
        let Some(proposal) = capacity_spec(record, config) else {
            continue;
        };
        let base_mbps = proposal.base_mbps;
        let factor = proposal.factor;
        let mut entry = CapacityEntry {
            node_id: node_id.clone(),
            model: proposal.model,
            phy_mbps: base_mbps,
            // Keep the report JSON serializable: a non-finite parsed factor is
            // still reported as the -1.0 invalid sentinel.
            factor: factor.map(|value| if value.is_finite() { value } else { -1.0 }),
            proposed_mbps: None,
            capacity_source: proposal.source,
            decision: String::new(),
        };
        let decision = if !base_mbps.is_finite() || base_mbps <= 0.0 {
            "ignored_invalid_capacity"
        } else if proposal.precomputed {
            // The poller already applied the 3c precedence and factors; this
            // stage only applies its own pin/CRM/unknown-device precedence.
            entry.proposed_mbps = Some(base_mbps);
            capacity_decision(imported, &index, pins, node_id)
        } else if factor.is_none() {
            "ignored_no_factor"
        } else if factor.is_some_and(|value| !value.is_finite() || value <= 0.0) {
            "ignored_invalid_factor"
        } else {
            // The proposal is computable before precedence decides whether it
            // is used; the report keeps it for pinned/CRM/unknown decisions.
            let factor = factor.expect("factor validated");
            entry.proposed_mbps = Some(base_mbps * factor);
            capacity_decision(imported, &index, pins, node_id)
        };
        entry.decision = decision.to_string();
        if decision == "used" {
            plan.capacity_used += 1;
        } else {
            plan.capacity_ignored += 1;
        }
        plan.capacity.push(entry);
    }

    let crm_pins = crm_pin_counts(imported);
    for (node_id, record) in &observations.devices {
        let Some(station_count) = record.get("station_count").and_then(Value::as_u64) else {
            continue;
        };
        let crm_pinned_count = crm_pins.get(node_id).copied().unwrap_or(0);
        plan.station_inventory.push(StationInventoryEntry {
            node_id: node_id.clone(),
            station_count,
            crm_pinned_count,
            delta: station_count as i64 - crm_pinned_count as i64,
        });
    }

    plan
}

/// Applies the supplement to an imported bundle.
///
/// Disabled, missing, malformed, stale, or generation-mismatched observations
/// return the bundle unchanged and no report. A report-write failure is logged
/// once; the applied edits stay applied and the import proceeds.
pub fn apply(
    config: &Config,
    mut imported: ImportedTopologyBundle,
    pins: &SupplementPins,
) -> (ImportedTopologyBundle, Option<SupplementPlan>) {
    if !config.ap_polling.enabled {
        return (imported, None);
    }
    let Some(observations) = load_observations(config) else {
        return (imported, None);
    };
    let plan = plan_supplement(&imported, &observations, pins, config);
    if config.ap_polling.apply_placement {
        apply_placement(&mut imported, &plan.edits);
    }
    apply_capacity(&mut imported, &plan);
    if let Err(error) = write_report(config, &observations, &plan) {
        warn!("AP supplement report write failed: {error}");
    }
    (imported, Some(plan))
}

fn circuit_by_mac(imported: &ImportedTopologyBundle) -> HashMap<String, BTreeSet<String>> {
    let mut by_mac: HashMap<String, BTreeSet<String>> = HashMap::new();
    for device in &imported.shaped_devices.devices {
        let mac = normalize_mac(&device.mac);
        if mac.is_empty() {
            continue;
        }
        by_mac
            .entry(mac)
            .or_default()
            .insert(device.circuit_id.clone());
    }
    by_mac
}

fn circuit_parent(imported: &ImportedTopologyBundle, circuit_id: &str) -> String {
    imported
        .circuit_anchors
        .anchors
        .iter()
        .find(|anchor| anchor.circuit_id == circuit_id)
        .map(|anchor| anchor.anchor_node_id.clone())
        .or_else(|| {
            imported
                .shaped_devices
                .devices
                .iter()
                .find(|device| device.circuit_id == circuit_id)
                .and_then(|device| device.parent_node_id.clone())
        })
        .unwrap_or_default()
}

fn desired_parent(
    node_id: &str,
    record: &Value,
    entry: &Value,
    index: &NodeIndex,
) -> Option<String> {
    for source in [entry, record] {
        for key in PLACEMENT_PARENT_KEYS {
            let Some(value) = source.get(key) else {
                continue;
            };
            if value.is_null() {
                continue;
            }
            let candidate = value_key(value);
            if index.nodes.contains_key(&candidate) {
                return Some(candidate);
            }
        }
    }
    index
        .nodes
        .contains_key(node_id)
        .then(|| node_id.to_string())
}

fn apply_placement(imported: &mut ImportedTopologyBundle, edits: &[PlacementEdit]) {
    for edit in edits {
        if let Some(anchor) = imported
            .circuit_anchors
            .anchors
            .iter_mut()
            .find(|anchor| anchor.circuit_id == edit.circuit_id)
        {
            anchor.anchor_node_id = edit.new_parent.clone();
            anchor.anchor_node_name = Some(edit.new_parent_name.clone());
        } else {
            let circuit_name = imported
                .shaped_devices
                .devices
                .iter()
                .find(|device| device.circuit_id == edit.circuit_id)
                .map(|device| device.circuit_name.clone());
            imported.circuit_anchors.anchors.push(CircuitAnchor {
                circuit_id: edit.circuit_id.clone(),
                circuit_name,
                anchor_node_id: edit.new_parent.clone(),
                anchor_node_name: Some(edit.new_parent_name.clone()),
            });
        }
        for device in &mut imported.shaped_devices.devices {
            if device.circuit_id == edit.circuit_id {
                device.parent_node = edit.new_parent_name.clone();
                device.parent_node_id = Some(edit.new_parent.clone());
            }
        }
    }
}

fn apply_capacity(imported: &mut ImportedTopologyBundle, plan: &SupplementPlan) {
    for entry in &plan.capacity {
        if entry.decision != "used" {
            continue;
        }
        let Some(proposed) = entry.proposed_mbps else {
            continue;
        };
        if !proposed.is_finite() {
            continue;
        }
        set_node_capacity(imported, &entry.node_id, proposed);
    }
}

fn set_node_capacity(imported: &mut ImportedTopologyBundle, node_id: &str, mbps: f64) {
    let rounded = mbps.round().max(0.0) as u64;
    if let Some(canonical) = imported.native_canonical.as_mut() {
        if let Some(node) = canonical
            .nodes
            .iter_mut()
            .find(|node| node.node_id == node_id)
        {
            node.rate_input.intrinsic_download_mbps = Some(rounded);
            node.rate_input.intrinsic_upload_mbps = Some(rounded);
        }
        set_tree_rates(&mut canonical.compatibility_network_json, node_id, mbps);
    }
    set_tree_rates(&mut imported.compatibility_network_json, node_id, mbps);
}

fn set_tree_rates(value: &mut Value, node_id: &str, mbps: f64) -> bool {
    let Some(map) = value.as_object_mut() else {
        return false;
    };
    set_tree_rates_map(map, node_id, mbps)
}

fn set_tree_rates_map(map: &mut Map<String, Value>, node_id: &str, mbps: f64) -> bool {
    let Some(number) = Number::from_f64(mbps) else {
        return false;
    };
    let mut changed = false;
    for value in map.values_mut() {
        let Some(node) = value.as_object_mut() else {
            continue;
        };
        if node.get("id").and_then(Value::as_str) == Some(node_id) {
            node.insert(
                "downloadBandwidthMbps".to_string(),
                Value::Number(number.clone()),
            );
            node.insert(
                "uploadBandwidthMbps".to_string(),
                Value::Number(number.clone()),
            );
            changed = true;
        }
        if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut) {
            changed |= set_tree_rates_map(children, node_id, mbps);
        }
    }
    changed
}

fn node_has_crm_rate(imported: &ImportedTopologyBundle, node_id: &str) -> bool {
    if let Some(canonical) = imported.native_canonical.as_ref()
        && let Some(node) = canonical.nodes.iter().find(|node| node.node_id == node_id)
    {
        let download = node
            .rate_input
            .intrinsic_download_mbps
            .or(node.rate_input.legacy_imported_download_mbps)
            .unwrap_or(0);
        let upload = node
            .rate_input
            .intrinsic_upload_mbps
            .or(node.rate_input.legacy_imported_upload_mbps)
            .unwrap_or(0);
        if download > 0 || upload > 0 {
            return true;
        }
    }
    tree_rates(&imported.compatibility_network_json, node_id)
        .is_some_and(|(download, upload)| download > 0 || upload > 0)
}

fn tree_rates(value: &Value, node_id: &str) -> Option<(u64, u64)> {
    let map = value.as_object()?;
    for value in map.values() {
        let Some(node) = value.as_object() else {
            continue;
        };
        if node.get("id").and_then(Value::as_str) == Some(node_id) {
            let download = read_u64(node.get("downloadBandwidthMbps")).unwrap_or(0);
            let upload = read_u64(node.get("uploadBandwidthMbps")).unwrap_or(0);
            return Some((download, upload));
        }
        if let Some(children) = node.get("children")
            && let Some(found) = tree_rates(children, node_id)
        {
            return Some(found);
        }
    }
    None
}

fn read_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_f64().map(|value| value.round().max(0.0) as u64)),
        _ => None,
    }
}

/// One capacity input: either a legacy reported PHY rate (factor applied
/// here) or the poller's precomputed capacity (factor already applied by the
/// 3c engine path).
struct CapacityProposal {
    base_mbps: f64,
    factor: Option<f64>,
    model: Option<String>,
    source: Option<String>,
    precomputed: bool,
}

fn capacity_spec(record: &Value, config: &Config) -> Option<CapacityProposal> {
    let capacity = record.get("capacity").and_then(Value::as_object);
    let model = capacity
        .and_then(|capacity| capacity.get("model"))
        .or_else(|| record.get("model"))
        .and_then(Value::as_str)
        .map(str::to_string);
    // The poller's shape: the 3c engine path already applied the factor and
    // precedence, so this stage must not multiply again.
    let precomputed = record
        .get("capacity_mbps")
        .or_else(|| capacity.and_then(|capacity| capacity.get("capacity_mbps")))
        .and_then(number_value);
    if let Some(mbps) = precomputed {
        let source = record
            .get("capacity_source")
            .or_else(|| capacity.and_then(|capacity| capacity.get("capacity_source")))
            .or_else(|| capacity.and_then(|capacity| capacity.get("source")))
            .and_then(Value::as_str)
            .map(str::to_string);
        return Some(CapacityProposal {
            base_mbps: if mbps.is_finite() { mbps } else { 0.0 },
            factor: None,
            model,
            source,
            precomputed: true,
        });
    }
    let phy = if let Some(capacity) = capacity {
        ["phy_mbps", "phy", "down_mbps"]
            .iter()
            .find_map(|key| capacity.get(*key))
    } else {
        ["phy_mbps", "observed_phy_mbps"]
            .iter()
            .find_map(|key| record.get(*key))
    }?;
    let phy_mbps = number_value(phy)?;
    let phy_mbps = if phy_mbps.is_finite() { phy_mbps } else { 0.0 };
    let explicit_factor = capacity
        .and_then(|capacity| capacity.get("factor"))
        .or_else(|| record.get("capacity_factor"))
        .or_else(|| record.get("factor"))
        .filter(|value| !value.is_null());
    let factor = if let Some(value) = explicit_factor {
        // -1.0 matches the Python planner's "invalid factor" sentinel and
        // keeps the report JSON serializable (NaN is not).
        Some(number_value(value).unwrap_or(-1.0))
    } else if let Some(model) = model.as_deref() {
        config
            .ap_polling
            .capacity_factor
            .get(model)
            .copied()
            .or_else(|| {
                let default = config.ap_polling.capacity_factor_default;
                (default.is_finite() && default > 0.0).then_some(default)
            })
    } else {
        let default = config.ap_polling.capacity_factor_default;
        (default.is_finite() && default > 0.0).then_some(default)
    };
    Some(CapacityProposal {
        base_mbps: phy_mbps,
        factor,
        model,
        source: None,
        precomputed: false,
    })
}

/// The supplement's own precedence over a computable proposal. The 3c engine
/// already ranked phy/per-SM/CRM inside the observation; this stage only
/// decides whether the polled value may replace the compiled topology.
fn capacity_decision(
    imported: &ImportedTopologyBundle,
    index: &NodeIndex,
    pins: &SupplementPins,
    node_id: &str,
) -> &'static str {
    if !index.nodes.contains_key(node_id) {
        "ignored_unknown_device"
    } else if pins.capacity.contains(node_id) {
        "ignored_pin"
    } else if node_has_crm_rate(imported, node_id) {
        "ignored_crm_rate"
    } else {
        "used"
    }
}

/// How many circuits the compiled topology currently pins under each node.
fn crm_pin_counts(imported: &ImportedTopologyBundle) -> HashMap<String, u64> {
    let mut counts: HashMap<String, u64> = HashMap::new();
    for anchor in &imported.circuit_anchors.anchors {
        *counts.entry(anchor.anchor_node_id.clone()).or_insert(0) += 1;
    }
    counts
}

fn number_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn entry_mac(entry: &Value) -> String {
    for key in ["router_mac", "mac", "cpe_mac", "onu_mac"] {
        if let Some(value) = entry.get(key) {
            let mac = first_mac(value);
            if !mac.is_empty() {
                return mac;
            }
        }
    }
    String::new()
}

fn first_mac(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Array(items) => items
            .iter()
            .find_map(|item| {
                let mac = first_mac(item);
                (!mac.is_empty()).then_some(mac)
            })
            .unwrap_or_default(),
        Value::String(text) => {
            for chunk in text.replace(';', ",").split(',') {
                for token in chunk.split_whitespace() {
                    let mac = normalize_mac(token);
                    if !mac.is_empty() {
                        return mac;
                    }
                }
            }
            String::new()
        }
        other => normalize_mac(&other.to_string()),
    }
}

fn normalize_mac(value: &str) -> String {
    value.trim().replace([':', '-'], "").to_uppercase()
}

fn value_key(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(boolean) => boolean.to_string(),
        other => other.to_string(),
    }
}

fn write_report(
    config: &Config,
    observations: &ApObservations,
    plan: &SupplementPlan,
) -> Result<(), std::io::Error> {
    let path = config.shaping_state_file_path(AP_SUPPLEMENT_REPORT_FILENAME);
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("ap_supplement_report.json has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let applied_edits = plan
        .edits
        .iter()
        .map(|edit| AppliedEdit {
            circuit_id: edit.circuit_id.clone(),
            old_parent: edit.old_parent.clone(),
            new_parent: edit.new_parent.clone(),
            source: edit.source,
        })
        .collect();
    let capacity_applied = plan
        .capacity
        .iter()
        .filter(|entry| entry.decision == "used")
        .filter_map(|entry| {
            entry.proposed_mbps.map(|proposed_mbps| AppliedCapacity {
                node_id: entry.node_id.clone(),
                proposed_mbps,
            })
        })
        .collect();
    let capacity_ignored = plan
        .capacity
        .iter()
        .filter(|entry| entry.decision != "used")
        .cloned()
        .collect();
    let report = ApSupplementReport {
        schema_version: AP_OBSERVATIONS_SCHEMA_VERSION,
        topology_generation: observations.topology_generation.clone(),
        written_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
        matched: plan.matched,
        unmatched: plan.unmatched,
        ambiguous: plan.ambiguous,
        pinned_overrides: plan.pinned_overrides,
        drift: plan.drift,
        applied_edits,
        capacity_applied,
        capacity_ignored,
        station_inventory: plan.station_inventory.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::normalize_mac;

    #[test]
    fn mac_normalization_strips_separators_and_uppercases() {
        assert_eq!(normalize_mac(" aa:bb:cc:dd:ee:ff "), "AABBCCDDEEFF");
        assert_eq!(normalize_mac("AA-BB-CC-DD-EE-FF"), "AABBCCDDEEFF");
        assert_eq!(normalize_mac(""), "");
    }
}

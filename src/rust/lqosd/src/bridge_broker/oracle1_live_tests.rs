//! ORACLE #1 — LIVE-CORPUS harness (producer half).
//!
//! The reviewer's #1 demand (`docs/oracle1_live_corpus_work_order.md`): prove
//! that the F3-B coherent-delta apply equals the per-op sequence **by driving
//! the producer's REAL diffs end-to-end**, not a hand-built op corpus.
//!
//! The corpus is anchored on the TOPOLOGY MANAGER's contract, not on import
//! files (import files are inputs; the topology is the manager's OUTPUT):
//!   1. the manager's INPUT is authored directly — `TopologyCanonicalStateFile`
//!      nodes/edges/ATTACHMENTS (plus the compiled shaping circuits) — and
//!      driven through `build_effective_topology_artifacts_from_canonical_with_
//!      runtime_queue_context` → `publish_effective_topology_artifacts` →
//!      `dataplane_transaction.json`, the REAL desired `DataplaneTransaction`;
//!   2. `translate::semantic_diff(desired, base, slot_order)` produces the REAL
//!      per-op sequence (`deltas`);
//!   3. the coherent delta `D*` is the union of the deltas' CHANGED ops.
//!
//! The corpus includes a plain tree, an AP served through a station (an
//! ATTACHMENT edge), and `libreqos:generated:*` AP/site nodes (including a
//! generated RE-PARENT — the LIVE `.12` class — and a generated add; both are
//! HELD by the producer). The legacy `network.json` + `ShapedDevices.csv` pair
//! is retained ONLY as a source-independence aside, never the primary path.
//!
//! No `DataplaneInnerOp::Upsert{..}`/`DataplaneLeafOp::Upsert{..}` literal is
//! authored here — the corpus builder authors canonical STATE and lets the
//! manager + compiler emit every dataplane op.
//!
//! The apply model is the shared `lqos_dataplane_types::AppliedFold` with
//! `content_key` + `structural_identity` (ONE home, both ends).
//!
//! The bridge apply half (`bridge_rs::dpdk_core::seam_contract_tests`) replays
//! the SAME corpus through the REAL `classify_transaction` + `AppliedState`
//! via the serialized golden this module also validates against fresh producer
//! output — see `oracle1_golden_matches_the_real_producer`.

#![cfg(test)]

use super::translate::semantic_diff;
use lqos_config::{
    CircuitAnchor, CircuitAnchorsFile, Config, ConfigShapedDevices, ShapedDevice,
    TopologyAttachmentHealthStateFile, TopologyCanonicalIngressKind, TopologyCanonicalNode,
    TopologyCanonicalRateInput, TopologyCanonicalStateFile, TopologyCircuitMatchIdentity,
    TopologyCircuitMatchIndex, TunnelIdentity,
};
use lqos_topology_compile::{ImportedTopologyBundle, TopologyCompiledShapingFile};
use lqos_dataplane_types::{
    AppliedFold, DataplaneInnerOp, DataplaneLeafOp, DataplaneMappingOp, DataplaneTransaction,
    content_key, structural_identity,
};
use lqos_overrides::TopologyOverridesFile;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// ─────────────────────────────────────────────────────────────────────────────
// Real compile plumbing
// ─────────────────────────────────────────────────────────────────────────────

const LEGACY_NETWORK: &str =
    include_str!("../../../lqos_topology_compile/tests/fixtures/legacy/network.json");
const LEGACY_DEVICES: &str =
    include_str!("../../../lqos_topology_compile/tests/fixtures/legacy/ShapedDevices.csv");
const RADIUS_NETWORK: &str =
    include_str!("../../../../../radius-harness/templates/network.json");

/// The committed golden the bridge apply half replays. Validated against fresh
/// producer output by [`oracle1_golden_matches_the_real_producer`].
fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../../../rust/libreqos_dpdk/bridge_rs/tests/fixtures/oracle1_live_corpus.json",
    )
}

/// A process-wide monotonic sequence. The temp-dir name must be UNIQUE even
/// when the parallel oracle tests allocate at the same instant — a wall-clock
/// stamp is NOT (two threads can read the same nanosecond and then race on the
/// same `network.json` / `ShapedDevices.csv`).
static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

fn unique_dir(label: &str) -> PathBuf {
    let seq = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
    let thread = std::thread::current().id();
    let dir = std::env::temp_dir().join(format!(
        "lqos-oracle1-{label}-{}-{seq}-{thread:?}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Runs the SAME production pipeline (`lqos_topology`) over the on-disk
/// `network.json` + `ShapedDevices.csv` pair in a FRESH temp dir and returns
/// the REAL compiled desired transaction (the artifact the bridge broker
/// consumes).
fn compile_desired(network_json: &str, shaped_devices_csv: &str) -> DataplaneTransaction {
    let dir = unique_dir("desired");
    let desired = compile_desired_in(&dir, network_json, shaped_devices_csv);
    let _ = std::fs::remove_dir_all(&dir);
    desired
}

fn compile_desired_in(
    dir: &Path,
    network_json: &str,
    shaped_devices_csv: &str,
) -> DataplaneTransaction {
    std::fs::write(dir.join("network.json"), network_json).expect("write network.json");
    std::fs::write(dir.join("ShapedDevices.csv"), shaped_devices_csv)
        .expect("write ShapedDevices.csv");
    let config = test_config(dir);
    // Phase C S4: the legacy pair is materialized into the ONE ingest
    // (`topology_import.json`) exactly as the scheduler's producer does.
    lqos_topology_compile::materialize_manual_topology(&config)
        .expect("legacy pair materializes into the one ingest");
    run_pipeline(&config)
}

fn test_config(dir: &Path) -> Config {
    Config {
        lqos_directory: dir.display().to_string(),
        state_directory: None,
        ..Config::default()
    }
}

/// Drives the TOPOLOGY MANAGER's contract directly: `canonical` + inputs →
/// `build_effective_topology_artifacts...` → `publish_effective_topology_
/// artifacts` → `dataplane_transaction.json`. The seeded half (legacy pair vs
/// authored canonical) is what the caller chooses.
fn run_pipeline(config: &Config) -> DataplaneTransaction {
    let canonical = lqos_topology::load_canonical_topology_state(config);
    let artifacts =
        lqos_topology::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        )
        .expect("effective topology artifacts build");
    lqos_topology::publish_effective_topology_artifacts(config, &artifacts)
        .expect("effective topology artifacts publish");
    let path = lqos_config::topology_dataplane_transaction_path(config);
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read dataplane tx {path:?}: {err}"));
    serde_json::from_str(&raw).expect("dataplane tx parses")
}

// ─────────────────────────────────────────────────────────────────────────────
// CANONICAL-AUTHORED worlds — the manager's INPUT model (nodes/edges/
// attachments), NOT import files. The manager + compiler emit every op.
// ─────────────────────────────────────────────────────────────────────────────

fn canonical_node(
    id: &str,
    name: &str,
    kind: &str,
    parent: Option<(&str, &str)>,
    attachment: Option<(&str, &str)>,
    down_mbps: u64,
    up_mbps: u64,
) -> TopologyCanonicalNode {
    TopologyCanonicalNode {
        node_id: id.to_string(),
        node_name: name.to_string(),
        node_kind: kind.to_string(),
        current_parent_node_id: parent.map(|(id, _)| id.to_string()),
        current_parent_node_name: parent.map(|(_, name)| name.to_string()),
        current_attachment_id: attachment.map(|(id, _)| id.to_string()),
        current_attachment_name: attachment.map(|(_, name)| name.to_string()),
        rate_input: TopologyCanonicalRateInput {
            intrinsic_download_mbps: Some(down_mbps),
            intrinsic_upload_mbps: Some(up_mbps),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn canonical_state(source: &str, nodes: Vec<TopologyCanonicalNode>) -> TopologyCanonicalStateFile {
    TopologyCanonicalStateFile {
        schema_version: 1,
        source: source.to_string(),
        generated_unix: Some(1),
        ingress_identity: None,
        ingress_kind: TopologyCanonicalIngressKind::NativeIntegration,
        nodes,
        compatibility_network_json: serde_json::json!({}),
    }
}

fn canonical_device(
    circuit_id: &str,
    parent_name: &str,
    parent_id: &str,
    ipv4: &str,
    rates: (f32, f32, f32, f32),
) -> ShapedDevice {
    ShapedDevice {
        circuit_id: circuit_id.to_string(),
        circuit_name: format!("Circuit {circuit_id}"),
        device_id: format!("device-{circuit_id}"),
        device_name: format!("Device {circuit_id}"),
        parent_node: parent_name.to_string(),
        parent_node_id: Some(parent_id.to_string()),
        anchor_node_id: None,
        mac: String::new(),
        ipv4: vec![(ipv4.parse().expect("device ipv4"), 32)],
        ipv6: Vec::new(),
        download_min_mbps: rates.0,
        upload_min_mbps: rates.1,
        download_max_mbps: rates.2,
        upload_max_mbps: rates.3,
        comment: String::new(),
        sqm_override: None,
        circuit_hash: 0,
        device_hash: 0,
        parent_hash: 0,
    }
}

fn canonical_anchor(circuit_id: &str, node_id: &str, node_name: &str) -> CircuitAnchor {
    CircuitAnchor {
        circuit_id: circuit_id.to_string(),
        circuit_name: Some(format!("Circuit {circuit_id}")),
        anchor_node_id: node_id.to_string(),
        anchor_node_name: Some(node_name.to_string()),
    }
}

fn shaped_from(devices: Vec<ShapedDevice>) -> ConfigShapedDevices {
    let mut shaped = ConfigShapedDevices::default();
    shaped.replace_with_new_data(devices);
    shaped
}

/// Seeds the manager's INPUT directly: authors `topology_canonical_state.json`
/// + the sibling compiled-shaping artifact. NO `network.json` /
/// `ShapedDevices.csv` / `topology_import.json` is written — the legacy pair is
/// deliberately absent so the canonical state IS the topology.
fn compile_from_canonical(
    canonical: &TopologyCanonicalStateFile,
    devices: Vec<ShapedDevice>,
    anchors: Vec<CircuitAnchor>,
    circuit_matches: TopologyCircuitMatchIndex,
) -> DataplaneTransaction {
    let dir = unique_dir("canonical");
    let config = test_config(&dir);
    assert!(
        !dir.join("network.json").exists() && !dir.join("ShapedDevices.csv").exists(),
        "the canonical-authored source must not carry a legacy pair"
    );
    canonical.save(&config).expect("topology_canonical_state.json saves");
    let imported = ImportedTopologyBundle {
        source: canonical.source.clone(),
        generated_unix: canonical.generated_unix,
        ingress_identity: None,
        native_canonical: Some(canonical.clone()),
        parent_candidates: None,
        compatibility_network_json: canonical.compatibility_network_json.clone(),
        shaped_devices: shaped_from(devices),
        circuit_anchors: CircuitAnchorsFile {
            schema_version: 1,
            source: canonical.source.clone(),
            generated_unix: canonical.generated_unix,
            anchors,
        },
        ethernet_advisories: Vec::new(),
        circuit_matches,
    };
    TopologyCompiledShapingFile::from_imported_manual(&imported, "canonical", None)
        .save(&config)
        .expect("topology_compiled_shaping.json saves");
    assert!(
        !dir.join("topology_import.json").exists(),
        "the canonical-authored source must not write an import artifact"
    );
    let desired = run_pipeline(&config);
    let _ = std::fs::remove_dir_all(&dir);
    desired
}

// ─── the canonical-authored world (the manager's INPUT) and its mutators ─────

/// A manager input triple: the canonical topology, plus the compiled shaping
/// circuits that ride it. Every transition mutates THIS (nodes/edges/
/// attachments), never a dataplane op.
#[derive(Clone)]
struct CanonicalWorld {
    canonical: TopologyCanonicalStateFile,
    devices: Vec<ShapedDevice>,
    anchors: Vec<CircuitAnchor>,
    /// T-arc §2.8 per-circuit match identities (tunnel / vlan / mpls). Empty in
    /// the base world; a transition may give a circuit a tunnel identity.
    circuit_matches: TopologyCircuitMatchIndex,
}

impl CanonicalWorld {
    fn compile(&self) -> DataplaneTransaction {
        compile_from_canonical(
            &self.canonical,
            self.devices.clone(),
            self.anchors.clone(),
            self.circuit_matches.clone(),
        )
    }
}

fn set_canonical_parent(world: &mut CanonicalWorld, id: &str, parent: (&str, &str)) {
    for node in &mut world.canonical.nodes {
        if node.node_id == id {
            node.current_parent_node_id = Some(parent.0.to_string());
            node.current_parent_node_name = Some(parent.1.to_string());
        }
    }
}

fn set_canonical_rates(world: &mut CanonicalWorld, id: &str, down: u64, up: u64) {
    for node in &mut world.canonical.nodes {
        if node.node_id == id {
            node.rate_input.intrinsic_download_mbps = Some(down);
            node.rate_input.intrinsic_upload_mbps = Some(up);
        }
    }
}

fn add_canonical_node(world: &mut CanonicalWorld, node: TopologyCanonicalNode) {
    world
        .canonical
        .nodes
        .retain(|existing| existing.node_id != node.node_id);
    world.canonical.nodes.push(node);
}

fn remove_canonical_node(world: &mut CanonicalWorld, id: &str) {
    world.canonical.nodes.retain(|node| node.node_id != id);
    world.devices.retain(|device| device.parent_node_id.as_deref() != Some(id));
    world.anchors.retain(|anchor| anchor.anchor_node_id != id);
}

fn add_circuit(
    world: &mut CanonicalWorld,
    circuit_id: &str,
    parent_name: &str,
    parent_id: &str,
    ipv4: &str,
    rates: (f32, f32, f32, f32),
) {
    world
        .devices
        .push(canonical_device(circuit_id, parent_name, parent_id, ipv4, rates));
    world
        .anchors
        .push(canonical_anchor(circuit_id, parent_id, parent_name));
}

/// D1: add a SECOND device to an EXISTING circuit (a distinct `device_id` +
/// `/32`), so the compiled circuit legitimately holds more than one mapping —
/// the live `.12` multi-device subscriber shape.
fn add_device(
    world: &mut CanonicalWorld,
    circuit_id: &str,
    parent_name: &str,
    parent_id: &str,
    suffix: &str,
    ipv4: &str,
    rates: (f32, f32, f32, f32),
) {
    let mut device = canonical_device(circuit_id, parent_name, parent_id, ipv4, rates);
    device.device_id = format!("device-{circuit_id}-{suffix}");
    device.device_name = format!("Device {circuit_id} {suffix}");
    world.devices.push(device);
}

fn remove_circuit(world: &mut CanonicalWorld, circuit_id: &str) {
    world.devices.retain(|device| device.circuit_id != circuit_id);
    world.anchors.retain(|anchor| anchor.circuit_id != circuit_id);
}

fn set_circuit_rates(world: &mut CanonicalWorld, circuit_id: &str, rates: (f32, f32, f32, f32)) {
    for device in &mut world.devices {
        if device.circuit_id == circuit_id {
            device.download_min_mbps = rates.0;
            device.upload_min_mbps = rates.1;
            device.download_max_mbps = rates.2;
            device.upload_max_mbps = rates.3;
        }
    }
}

/// U3: re-bind every device on a circuit to a single new IPv4 address. The old
/// prefix leaves `desired` ⇒ the structural view must emit an explicit `Del`.
fn set_circuit_ip(world: &mut CanonicalWorld, circuit_id: &str, ipv4: &str) {
    for device in &mut world.devices {
        if device.circuit_id == circuit_id {
            device.ipv4 = vec![(ipv4.parse().expect("device ipv4"), 32)];
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The LEGACY-PAIR world (source-independence aside only)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct World {
    network: Value,
    devices: String,
}

impl World {
    fn compile(&self) -> DataplaneTransaction {
        compile_desired(&self.network.to_string(), &self.devices)
    }

    fn from_fixtures() -> Self {
        Self {
            network: serde_json::from_str(LEGACY_NETWORK).expect("legacy network.json parses"),
            devices: LEGACY_DEVICES.to_string(),
        }
    }

    fn radius_fixture() -> Self {
        let mut devices = String::from(
            "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
        );
        devices.push_str(
            "\"r1\",\"Radius One\",\"rd1\",\"rd1\",\"PPPoE Access\",\"lab-pppoe-access\",\"\",\"aa:bb:cc:00:00:01\",\"192.0.2.21/32\",\"\",10,5,100,50,\"\"\n",
        );
        Self {
            network: serde_json::from_str(RADIUS_NETWORK).expect("radius network.json parses"),
            devices,
        }
    }
}

// ---- network.json mutation helpers (config-level; no dataplane ops) ----

fn flat_node(id: &str, name: &str, kind: &str, down: f64, up: f64) -> Value {
    json!({
        "children": {},
        "id": id,
        "name": name,
        "type": kind,
        "downloadBandwidthMbps": down,
        "uploadBandwidthMbps": up
    })
}

fn node_by_id_mut<'a>(value: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    if value.get("id").and_then(Value::as_str) == Some(id) {
        return Some(value);
    }
    if let Some(children) = value.get_mut("children").and_then(Value::as_object_mut) {
        for child in children.values_mut() {
            if let Some(found) = node_by_id_mut(child, id) {
                return Some(found);
            }
        }
    }
    None
}

fn node_by_id<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
    if value.get("id").and_then(Value::as_str) == Some(id) {
        return Some(value);
    }
    if let Some(children) = value.get("children").and_then(Value::as_object) {
        for child in children.values() {
            if let Some(found) = node_by_id(child, id) {
                return Some(found);
            }
        }
    }
    None
}

fn root_nodes_mut(network: &mut Value) -> &mut Map<String, Value> {
    network.as_object_mut().expect("network.json is an object")
}

fn find_root_node_mut<'a>(network: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    root_nodes_mut(network)
        .values_mut()
        .find(|node| node_by_id(node, id).is_some())
        .and_then(|node| node_by_id_mut(node, id))
}

fn insert_child(network: &mut Value, parent_id: &str, child: Value) {
    let parent = find_root_node_mut(network, parent_id).expect("parent exists");
    let children = parent
        .get_mut("children")
        .and_then(Value::as_object_mut)
        .expect("parent has children");
    let name = child
        .get("name")
        .and_then(Value::as_str)
        .expect("child has name")
        .to_string();
    children.insert(name, child);
}

fn remove_node(network: &mut Value, id: &str) -> Option<Value> {
    // Recursively remove from any children map; also handle the top level.
    fn recurse(children: &mut Map<String, Value>, id: &str) -> Option<Value> {
        let key = children
            .iter()
            .find(|(_, v)| v.get("id").and_then(Value::as_str) == Some(id))
            .map(|(k, _)| k.clone());
        if let Some(key) = key {
            return children.remove(&key);
        }
        for value in children.values_mut() {
            if let Some(grand) = value.get_mut("children").and_then(Value::as_object_mut) {
                if let Some(removed) = recurse(grand, id) {
                    return Some(removed);
                }
            }
        }
        None
    }
    let root = root_nodes_mut(network);
    if let Some(key) = root
        .iter()
        .find(|(_, v)| v.get("id").and_then(Value::as_str) == Some(id))
        .map(|(k, _)| k.clone())
    {
        return root.remove(&key);
    }
    for value in root.values_mut() {
        if let Some(children) = value.get_mut("children").and_then(Value::as_object_mut) {
            if let Some(removed) = recurse(children, id) {
                return Some(removed);
            }
        }
    }
    None
}

// ---- ShapedDevices.csv mutation helpers ----

const CIRCUIT_ID: usize = 0;
const PARENT_NODE: usize = 4;
const PARENT_NODE_ID: usize = 5;
const DOWN_MIN: usize = 10;
const UP_MIN: usize = 11;
const DOWN_MAX: usize = 12;
const UP_MAX: usize = 13;

fn csv_rows(csv: &str) -> Vec<Vec<String>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(csv.as_bytes());
    reader
        .records()
        .map(|row| {
            row.expect("csv row")
                .iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn csv_string(rows: &[Vec<String>]) -> String {
    let mut writer = csv::WriterBuilder::new().from_writer(Vec::new());
    for row in rows {
        writer.write_record(row).expect("csv record");
    }
    String::from_utf8(writer.into_inner().expect("csv bytes")).expect("csv utf8")
}

fn set_device_rates(devices: &mut String, circuit_id: &str, rates: (u32, u32, u32, u32)) {
    let mut rows = csv_rows(devices);
    for row in rows.iter_mut().skip(1) {
        if row[CIRCUIT_ID] == circuit_id {
            row[DOWN_MIN] = rates.0.to_string();
            row[UP_MIN] = rates.1.to_string();
            row[DOWN_MAX] = rates.2.to_string();
            row[UP_MAX] = rates.3.to_string();
        }
    }
    *devices = csv_string(&rows);
}

fn remove_device(devices: &mut String, circuit_id: &str) {
    let rows = csv_rows(devices);
    let kept: Vec<Vec<String>> = rows
        .into_iter()
        .filter(|row| row.get(CIRCUIT_ID).map(String::as_str) != Some(circuit_id))
        .collect();
    *devices = csv_string(&kept);
}

fn push_device(
    devices: &mut String,
    circuit_id: &str,
    device_id: &str,
    parent_name: &str,
    parent_id: &str,
    ipv4: &str,
    rates: (u32, u32, u32, u32),
) {
    let mut rows = csv_rows(devices);
    rows.push(vec![
        circuit_id.to_string(),
        format!("Circuit {circuit_id}"),
        device_id.to_string(),
        device_id.to_string(),
        parent_name.to_string(),
        parent_id.to_string(),
        String::new(),
        "aa:bb:cc:00:00:02".to_string(),
        ipv4.to_string(),
        String::new(),
        rates.0.to_string(),
        rates.1.to_string(),
        rates.2.to_string(),
        rates.3.to_string(),
        String::new(),
    ]);
    *devices = csv_string(&rows);
}

fn set_device_parent(devices: &mut String, circuit_id: &str, parent_name: &str, parent_id: &str) {
    let mut rows = csv_rows(devices);
    for row in rows.iter_mut().skip(1) {
        if row[CIRCUIT_ID] == circuit_id {
            row[PARENT_NODE] = parent_name.to_string();
            row[PARENT_NODE_ID] = parent_id.to_string();
        }
    }
    *devices = csv_string(&rows);
}

// ─────────────────────────────────────────────────────────────────────────────
// The coherent delta: the union of the deltas' CHANGED ops
// ─────────────────────────────────────────────────────────────────────────────

fn base_inner_map(base: &DataplaneTransaction) -> BTreeMap<String, DataplaneInnerOp> {
    base.inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => Some((node_id.clone(), op.clone())),
            _ => None,
        })
        .collect()
}

fn base_leaf_map(base: &DataplaneTransaction) -> BTreeMap<String, DataplaneLeafOp> {
    base.leaf_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneLeafOp::Upsert { circuit_id, .. } => Some((circuit_id.clone(), op.clone())),
            _ => None,
        })
        .collect()
}

fn map_identity(op: &DataplaneMappingOp) -> String {
    match op {
        DataplaneMappingOp::Map { prefix_len, ip, .. } => format!("prefix:{prefix_len}:{ip}"),
        DataplaneMappingOp::Del { prefix_len, ip } => format!("prefix:{prefix_len}:{ip}"),
        DataplaneMappingOp::MapTunnel { vni, vlan, .. } => format!("tunnel:{vni}:{vlan}"),
        DataplaneMappingOp::DelTunnel { vni, vlan } => format!("tunnel:{vni}:{vlan}"),
        DataplaneMappingOp::MapVlan { vlan, .. } => format!("vlan:{vlan}"),
        DataplaneMappingOp::DelVlan { vlan } => format!("vlan:{vlan}"),
        DataplaneMappingOp::MapMpls { label, .. } => format!("mpls:{label}"),
        DataplaneMappingOp::DelMpls { label } => format!("mpls:{label}"),
        DataplaneMappingOp::Clear => "clear".to_string(),
    }
}

fn base_map_map(base: &DataplaneTransaction) -> BTreeMap<String, DataplaneMappingOp> {
    base.mapping_ops
        .iter()
        .map(|op| (map_identity(op), op.clone()))
        .collect()
}

/// `D*` — the union of the deltas' **changed** ops (unchanged carry-alongs that
/// keep re-asserting the base content are dropped; per-identity last-write
/// wins). This is the F3-B coherent-delta shape of `docs/f3b_bridge_order_
/// independent_apply.md` §1.3 of the work order.
fn coherent_delta(base: &DataplaneTransaction, deltas: &[DataplaneTransaction]) -> DataplaneTransaction {
    let base_inners = base_inner_map(base);
    let base_leaves = base_leaf_map(base);
    let base_maps = base_map_map(base);

    let mut inner_order: Vec<String> = Vec::new();
    let mut inners: HashMap<String, DataplaneInnerOp> = HashMap::new();
    let mut leaf_order: Vec<String> = Vec::new();
    let mut leaves: HashMap<String, DataplaneLeafOp> = HashMap::new();
    let mut map_order: Vec<String> = Vec::new();
    let mut maps: HashMap<String, DataplaneMappingOp> = HashMap::new();

    for delta in deltas {
        for op in &delta.inner_ops {
            match op {
                DataplaneInnerOp::Upsert { node_id, .. } => {
                    let changed = base_inners.get(node_id) != Some(op);
                    if changed {
                        if !inners.contains_key(node_id) {
                            inner_order.push(node_id.clone());
                        }
                        inners.insert(node_id.clone(), op.clone());
                    }
                }
                DataplaneInnerOp::Remove { node_id } => {
                    if !inners.contains_key(node_id) {
                        inner_order.push(node_id.clone());
                    }
                    inners.insert(node_id.clone(), op.clone());
                }
                DataplaneInnerOp::Move { node_id, .. } => {
                    if !inners.contains_key(node_id) {
                        inner_order.push(node_id.clone());
                    }
                    inners.insert(node_id.clone(), op.clone());
                }
            }
        }
        for op in &delta.leaf_ops {
            match op {
                DataplaneLeafOp::Upsert { circuit_id, .. } => {
                    let changed = base_leaves.get(circuit_id) != Some(op);
                    if changed {
                        if !leaves.contains_key(circuit_id) {
                            leaf_order.push(circuit_id.clone());
                        }
                        leaves.insert(circuit_id.clone(), op.clone());
                    }
                }
                DataplaneLeafOp::Remove { circuit_id } => {
                    if !leaves.contains_key(circuit_id) {
                        leaf_order.push(circuit_id.clone());
                    }
                    leaves.insert(circuit_id.clone(), op.clone());
                }
                DataplaneLeafOp::Move { circuit_id, .. } => {
                    if !leaves.contains_key(circuit_id) {
                        leaf_order.push(circuit_id.clone());
                    }
                    leaves.insert(circuit_id.clone(), op.clone());
                }
            }
        }
        for op in &delta.mapping_ops {
            let identity = map_identity(op);
            let changed = base_maps.get(&identity) != Some(op);
            if changed {
                if !maps.contains_key(&identity) {
                    map_order.push(identity.clone());
                }
                maps.insert(identity, op.clone());
            }
        }
    }

    DataplaneTransaction {
        generation: base.generation.clone(),
        inner_ops: inner_order
            .into_iter()
            .filter_map(|id| inners.remove(&id))
            .collect(),
        leaf_ops: leaf_order
            .into_iter()
            .filter_map(|id| leaves.remove(&id))
            .collect(),
        mapping_ops: map_order
            .into_iter()
            .filter_map(|id| maps.remove(&id))
            .collect(),
        schema_version: None,
        steering_vlans: Vec::new(),
        lpm_delta: Vec::new(),
        apply_mode: Default::default(),
        circuit_capacity: None,
    }
}

/// The VERBATIM union of every op every delta carries (no change filter, no
/// dedup) — the literal "union of all the deltas' ops". For a multi-remove this
/// re-introduces the base-content carry-along `Upsert` alongside the explicit
/// `Remove` of the same identity: an R-A collision, i.e. NOT delta-expressible
/// (F3-B §1.4). Used to exercise the fallback arm.
fn all_ops_union(deltas: &[DataplaneTransaction]) -> DataplaneTransaction {
    let mut tx = DataplaneTransaction {
        generation: "all-ops".to_string(),
        inner_ops: Vec::new(),
        leaf_ops: Vec::new(),
        mapping_ops: Vec::new(),
        schema_version: None,
        steering_vlans: Vec::new(),
        lpm_delta: Vec::new(),
        apply_mode: Default::default(),
        circuit_capacity: None,
    };
    for delta in deltas {
        tx.inner_ops.extend(delta.inner_ops.iter().cloned());
        tx.leaf_ops.extend(delta.leaf_ops.iter().cloned());
        tx.mapping_ops.extend(delta.mapping_ops.iter().cloned());
    }
    tx
}


// ─────────────────────────────────────────────────────────────────────────────
// Fold helpers (the shared apply model)
// ─────────────────────────────────────────────────────────────────────────────

fn fold_seq(base: &DataplaneTransaction, seq: &[DataplaneTransaction]) -> DataplaneTransaction {
    let mut fold = AppliedFold::from_tx(base);
    for tx in seq {
        fold.fold(tx);
    }
    fold.to_transaction("oracle")
}

fn fold_delta(base: &DataplaneTransaction, delta: &DataplaneTransaction) -> DataplaneTransaction {
    let mut fold = AppliedFold::from_tx(base);
    fold.fold(delta);
    fold.to_transaction("oracle")
}

fn identity_set(tx: &DataplaneTransaction) -> (Vec<String>, Vec<String>, Vec<String>) {
    use std::collections::BTreeSet;
    let inners: BTreeSet<String> = tx
        .inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => Some(node_id.clone()),
            _ => None,
        })
        .collect();
    let leaves: BTreeSet<String> = tx
        .leaf_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneLeafOp::Upsert { circuit_id, .. } => Some(circuit_id.clone()),
            _ => None,
        })
        .collect();
    let maps: BTreeSet<String> = tx.mapping_ops.iter().map(map_identity).collect();
    (
        inners.into_iter().collect(),
        leaves.into_iter().collect(),
        maps.into_iter().collect(),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// The corpus
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
struct GoldenCase {
    name: String,
    /// The seeding layer this transition was authored at (coverage metadata).
    seed: String,
    /// `true` when the verbatim all-ops union is expected to be our R-A
    /// collision fallback case.
    collision: bool,
    /// `true` when the producer HOLDS the transition: a
    /// `libreqos:generated:*` subtree changed, so no live structural delta is
    /// emitted (the S3/bundle lane). `deltas` is then empty.
    held: bool,
    /// Coverage metadata: the authored world models an AP served through a
    /// station (an ATTACHMENT edge).
    ap_with_attachment: bool,
    /// §`lqos-74t.12`: the NEW sites DEFERRED by the producer because no circuit
    /// rides them (a POLICY gap). The convergence oracle compares the fold to the
    /// desired MINUS these sites.
    #[serde(default)]
    deferred_sites: Vec<String>,
    /// §`lqos-74t.13`/`.15`(d): applied sites DEFERRED for retirement while their
    /// applied subtree still bears circuits — the convergence oracle compares the
    /// fold to the desired PLUS these sites' base inners.
    #[serde(default)]
    deferred_retires: Vec<String>,
    base: DataplaneTransaction,
    desired: DataplaneTransaction,
    deltas: Vec<DataplaneTransaction>,
    coherent_delta: DataplaneTransaction,
    all_ops_delta: DataplaneTransaction,
}

fn normalized(mut tx: DataplaneTransaction, generation: &str) -> DataplaneTransaction {
    tx.generation = generation.to_string();
    tx
}

/// §`lqos-74t.12`: the producer DEFERS a NEW lone site (no circuit) BY POLICY —
/// the coherent delta folds to the desired MINUS those sites. A deferred site has
/// no circuits, so only its inner `Upsert` is absent.
fn desired_minus_deferred(
    tx: &DataplaneTransaction,
    deferred: &[String],
) -> DataplaneTransaction {
    let mut out = tx.clone();
    out.inner_ops.retain(|op| match op {
        DataplaneInnerOp::Upsert { node_id, .. } => !deferred.iter().any(|d| d == node_id),
        _ => true,
    });
    out
}

/// §`lqos-74t.13`/`.15`(d): the producer DEFERS an applied site's `Remove` while
/// its applied subtree still bears circuits, so the fold RETAINS it — the
/// expected world is `desired - deferred_sites` PLUS the deferred retire sites'
/// BASE inner `Upsert`s (their leaves are removed by the same delta).
fn expected_after_deferrals(
    desired: &DataplaneTransaction,
    base: &DataplaneTransaction,
    deferred_sites: &[String],
    deferred_retires: &[String],
) -> DataplaneTransaction {
    let mut out = desired_minus_deferred(desired, deferred_sites);
    for site in deferred_retires {
        let present = out
            .inner_ops
            .iter()
            .any(|op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == site));
        if !present
            && let Some(op) = base
                .inner_ops
                .iter()
                .find(|op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == site))
        {
            out.inner_ops.push(op.clone());
        }
    }
    out
}

/// Seeds a transition from two manager OUTPUTS (already compiled/published).
fn build_case_tx(
    name: &str,
    seed: &str,
    base_tx: DataplaneTransaction,
    desired_tx: DataplaneTransaction,
    collision: bool,
    _held: bool,
    ap_with_attachment: bool,
) -> GoldenCase {
    let outcome = semantic_diff(&desired_tx, Some(&base_tx), None);
    // P7/B3 retired: the producer no longer HOLDS a changed
    // `libreqos:generated:*` subtree — every real transition emits a coherent
    // delta (a steady/no-op transition is `None`). The caller's historical
    // `held` flag is ignored; `held` is derived from the outcome.
    let held = outcome.delta.is_none();
    let coherent = outcome.delta.clone().unwrap_or_else(|| DataplaneTransaction {
        generation: String::new(),
        inner_ops: Vec::new(),
        leaf_ops: Vec::new(),
        mapping_ops: Vec::new(),
        schema_version: None,
        steering_vlans: Vec::new(),
        lpm_delta: Vec::new(),
        apply_mode: Default::default(),
        circuit_capacity: None,
    });
    // The producer IS the coherent delta now; `all_ops_delta` is retained as an
    // alias for the (retired) fallback corpus.
    let all_ops = coherent.clone();
    let deferred_sites = outcome.deferred_sites.clone();
    let deferred_retires = outcome.deferred_retires.clone();
    GoldenCase {
        name: name.to_string(),
        seed: seed.to_string(),
        collision,
        held,
        ap_with_attachment,
        deferred_sites,
        deferred_retires,
        base: normalized(base_tx, &format!("{name}-base")),
        desired: normalized(desired_tx, &format!("{name}-desired")),
        deltas: outcome
            .delta
            .into_iter()
            .enumerate()
            .map(|(i, tx)| normalized(tx, &format!("{name}-delta-{i}")))
            .collect(),
        coherent_delta: normalized(coherent, &format!("{name}-coherent")),
        all_ops_delta: normalized(all_ops, &format!("{name}-all")),
    }
}

/// A legacy-pair-sourced transition (SOURCE-INDEPENDENCE ASIDE only — the
/// primary corpus is canonical-authored below).
fn build_case(name: &str, base: World, desired: World, collision: bool) -> GoldenCase {
    let base_tx = base.compile();
    let desired_tx = desired.compile();
    build_case_tx(
        name,
        "legacy-pair (aside)",
        base_tx,
        desired_tx,
        collision,
        false,
        false,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// THE PRIMARY CORPUS — authored at the MANAGER's contract layer
// ─────────────────────────────────────────────────────────────────────────────

/// The canonical-authored base world: a plain tree, an AP served through a
/// station (the ATTACHMENT edge), and `libreqos:generated:*` AP/site nodes.
///
/// Nodes: Root Site + Site Two; Station Alpha (the station the AP is served
/// through); AP Alpha (attachment = station-alpha); AP Beta; the generated
/// `libreqos:generated:ap_site:1` site and the generated
/// `libreqos:generated:graph:site:customer-alpha` site. Every circuit-bearing
/// node has a compiled shaping circuit.
fn canonical_base() -> CanonicalWorld {
    let nodes = vec![
        canonical_node("uisp:site:site-root", "Root Site", "Site", None, None, 1000, 1000),
        canonical_node("uisp:site:site-two", "Site Two", "Site", None, None, 500, 500),
        canonical_node(
            "uisp:device:station-alpha",
            "Station Alpha",
            "Switch",
            Some(("uisp:site:site-root", "Root Site")),
            None,
            100,
            100,
        ),
        canonical_node(
            "uisp:device:ap-alpha",
            "AP Alpha",
            "AP",
            Some(("uisp:site:site-root", "Root Site")),
            Some(("uisp:device:station-alpha", "Station Alpha")),
            100,
            100,
        ),
        canonical_node(
            "uisp:device:ap-beta",
            "AP Beta",
            "AP",
            Some(("uisp:site:site-root", "Root Site")),
            None,
            100,
            100,
        ),
        canonical_node(
            "libreqos:generated:ap_site:1",
            "Generated AP Site",
            "Site",
            Some(("uisp:site:site-root", "Root Site")),
            None,
            200,
            200,
        ),
        canonical_node(
            "libreqos:generated:graph:site:customer-alpha",
            "Customer Alpha",
            "Site",
            Some(("uisp:device:ap-alpha", "AP Alpha")),
            Some(("uisp:device:station-alpha", "Station Alpha")),
            100,
            100,
        ),
    ];
    let canonical = canonical_state("system:canonical-authoring", nodes);
    let devices = vec![
        canonical_device(
            "circuit-apsite",
            "Generated AP Site",
            "libreqos:generated:ap_site:1",
            "198.51.100.20",
            (10.0, 5.0, 100.0, 50.0),
        ),
        canonical_device(
            "circuit-graph",
            "Customer Alpha",
            "libreqos:generated:graph:site:customer-alpha",
            "198.51.100.10",
            (10.0, 5.0, 100.0, 50.0),
        ),
        canonical_device(
            "circuit-beta",
            "AP Beta",
            "uisp:device:ap-beta",
            "198.51.100.11",
            (20.0, 10.0, 200.0, 100.0),
        ),
    ];
    let anchors = vec![
        canonical_anchor(
            "circuit-apsite",
            "libreqos:generated:ap_site:1",
            "Generated AP Site",
        ),
        canonical_anchor(
            "circuit-graph",
            "libreqos:generated:graph:site:customer-alpha",
            "Customer Alpha",
        ),
        canonical_anchor("circuit-beta", "uisp:device:ap-beta", "AP Beta"),
    ];
    CanonicalWorld {
        canonical,
        devices,
        anchors,
        circuit_matches: TopologyCircuitMatchIndex::new(),
    }
}

/// T-arc §2.8: give a circuit a TUNNEL classification identity, so the compiler
/// emits a `MapTunnel` alongside its device prefix `Map`s (the cross-family
/// shape). The circuit must already exist in the world.
fn set_tunnel_identity(world: &mut CanonicalWorld, circuit_id: &str, vni: u32, vlan: u16) {
    world.circuit_matches.insert(
        circuit_id.to_string(),
        TopologyCircuitMatchIdentity::Tunnel(TunnelIdentity { vni, vlan }),
    );
}

const CANONICAL_SEED: &str = "system/canonical-authoring";

/// The corpus of REAL (base, desired) transitions. PRIMARY path: author the
/// manager's canonical INPUT and drive `build → publish → dataplane_
/// transaction.json`; the producer diffs the OUTPUT. The legacy pair is an
/// aside (source-independence only).
fn corpus() -> Vec<GoldenCase> {
    let mut cases = Vec::new();

    // 1. rate-only churn over the canonical world (generated subtrees unchanged
    //    ⇒ a real delta, NOT held).
    {
        let base = canonical_base();
        let mut desired = base.clone();
        set_circuit_rates(&mut desired, "circuit-graph", (30.0, 15.0, 300.0, 150.0));
        set_circuit_rates(&mut desired, "circuit-apsite", (25.0, 12.0, 250.0, 120.0));
        // An inner (node) rate churn rides the same manager output.
        set_canonical_rates(&mut desired, "uisp:device:ap-beta", 120, 120);
        cases.push(build_case_tx(
            "canonical_rate_only",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            false,
            true,
        ));
    }

    // 1b. §`lqos-74t.12`: a NEW LONE site with NO circuit is DEFERRED (zero
    //     dataplane effect); a rate change keeps the payload NON-shell (the
    //     `NewSiteNoAdd` class). The lone inner is authored directly: the
    //     topology compiler elides a circuit-less node, but the LIVE class is a
    //     producer tx that carries the new site inner with no add.
    {
        let base_tx = canonical_base().compile();
        let mut desired_tx = base_tx.clone();
        desired_tx.inner_ops.push(DataplaneInnerOp::Upsert {
            node_id: "uisp:site:site-lone".to_string(),
            parent_node_id: "uisp:site:site-root".to_string(),
            download: lqos_dataplane_types::DataplaneRatePair {
                commit_kbps: 100_000,
                ceil_kbps: 100_000,
            },
            upload: lqos_dataplane_types::DataplaneRatePair {
                commit_kbps: 100_000,
                ceil_kbps: 100_000,
            },
        });
        for op in &mut desired_tx.leaf_ops {
            if let DataplaneLeafOp::Upsert {
                circuit_id,
                download,
                ..
            } = op
                && circuit_id == "circuit-graph"
            {
                *download = lqos_dataplane_types::DataplaneRatePair {
                    commit_kbps: 31_000,
                    ceil_kbps: 310_000,
                };
            }
        }
        cases.push(build_case_tx(
            "canonical_lone_site_deferred",
            CANONICAL_SEED,
            base_tx,
            desired_tx,
            false,
            false,
            true,
        ));
    }

    // 1c. §`lqos-74t.19`: a NEW-TARGET re-parent — an existing circuit moves onto
    //     a NEWLY created site. The producer emits the target's inner Upsert +
    //     the moved leaf in ONE payload; the bridge classifies `MoveLeaf`.
    {
        let base_tx = canonical_base().compile();
        let mut desired_tx = base_tx.clone();
        desired_tx.inner_ops.push(DataplaneInnerOp::Upsert {
            node_id: "uisp:site:site-newtarget".to_string(),
            parent_node_id: "uisp:site:site-root".to_string(),
            download: lqos_dataplane_types::DataplaneRatePair {
                commit_kbps: 100_000,
                ceil_kbps: 100_000,
            },
            upload: lqos_dataplane_types::DataplaneRatePair {
                commit_kbps: 100_000,
                ceil_kbps: 100_000,
            },
        });
        for op in &mut desired_tx.leaf_ops {
            if let DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } = op
                && circuit_id == "circuit-graph"
            {
                *parent_node_id = "uisp:site:site-newtarget".to_string();
            }
        }
        cases.push(build_case_tx(
            "canonical_new_target_reparent",
            CANONICAL_SEED,
            base_tx,
            desired_tx,
            false,
            false,
            true,
        ));
    }

    // 2. add a NON-generated inner (AP Gamma) + its circuit.
    {
        let base = canonical_base();
        let mut desired = base.clone();
        add_canonical_node(
            &mut desired,
            canonical_node(
                "uisp:device:ap-gamma",
                "AP Gamma",
                "AP",
                Some(("uisp:site:site-root", "Root Site")),
                None,
                150,
                150,
            ),
        );
        add_circuit(
            &mut desired,
            "circuit-gamma",
            "AP Gamma",
            "uisp:device:ap-gamma",
            "198.51.100.30",
            (15.0, 7.0, 150.0, 75.0),
        );
        cases.push(build_case_tx(
            "canonical_add_inner",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            false,
            true,
        ));
    }

    // 3. remove a NON-generated inner (AP Beta) + its circuit.
    {
        let base = canonical_base();
        let mut desired = base.clone();
        remove_circuit(&mut desired, "circuit-beta");
        remove_canonical_node(&mut desired, "uisp:device:ap-beta");
        cases.push(build_case_tx(
            "canonical_remove_inner",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            false,
            true,
        ));
    }

    // 4. re-parent a NON-generated inner (AP Beta moves to Site Two).
    {
        let base = canonical_base();
        let mut desired = base.clone();
        set_canonical_parent(
            &mut desired,
            "uisp:device:ap-beta",
            ("uisp:site:site-two", "Site Two"),
        );
        cases.push(build_case_tx(
            "canonical_reparent_inner",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            false,
            true,
        ));
    }

    // 5. THE ATTACHMENT CASE: AP Alpha is served through Station Alpha (an
    //    ATTACHMENT edge) and its logical parent changes; the attachment stays.
    {
        let base = canonical_base();
        let mut desired = base.clone();
        set_canonical_parent(
            &mut desired,
            "uisp:device:ap-alpha",
            ("uisp:site:site-two", "Site Two"),
        );
        cases.push(build_case_tx(
            "canonical_attachment_ap_reparent",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            false,
            true,
        ));
    }

    // 6. rate churn under an UNCHANGED generated subtree (exercises generated
    //    nodes in a real emitted delta).
    {
        let base = canonical_base();
        let mut desired = base.clone();
        set_circuit_rates(&mut desired, "circuit-graph", (40.0, 20.0, 400.0, 200.0));
        cases.push(build_case_tx(
            "canonical_generated_rate_only",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            false,
            true,
        ));
    }

    // 7. GENERATED ADD: a new `libreqos:generated:graph:*` node + circuit.
    //    A changed generated subtree is HELD (no live structural delta).
    {
        let base = canonical_base();
        let mut desired = base.clone();
        add_canonical_node(
            &mut desired,
            canonical_node(
                "libreqos:generated:graph:site:customer-beta",
                "Customer Beta",
                "Site",
                Some(("uisp:device:ap-alpha", "AP Alpha")),
                Some(("uisp:device:station-alpha", "Station Alpha")),
                100,
                100,
            ),
        );
        add_circuit(
            &mut desired,
            "circuit-graph-beta",
            "Customer Beta",
            "libreqos:generated:graph:site:customer-beta",
            "198.51.100.40",
            (10.0, 5.0, 100.0, 50.0),
        );
        cases.push(build_case_tx(
            "canonical_generated_add",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            true,
            true,
        ));
    }

    // 8. GENERATED RE-PARENT (the LIVE `.12` class): the generated
    //    `graph:customer-alpha` node's parent changes from AP Alpha to Root
    //    Site. The produced tree HOLDS; no live generated move is emitted.
    {
        let base = canonical_base();
        let mut desired = base.clone();
        set_canonical_parent(
            &mut desired,
            "libreqos:generated:graph:site:customer-alpha",
            ("uisp:site:site-root", "Root Site"),
        );
        cases.push(build_case_tx(
            "canonical_generated_reparent",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            true,
            true,
        ));
    }

    // 9. MIXED: add a circuit + remove a circuit + rate churn, generated
    //    subtrees unchanged (so a real delta is emitted).
    {
        let base = canonical_base();
        let mut desired = base.clone();
        add_circuit(
            &mut desired,
            "circuit-gamma",
            "AP Beta",
            "uisp:device:ap-beta",
            "198.51.100.30",
            (15.0, 7.0, 150.0, 75.0),
        );
        remove_circuit(&mut desired, "circuit-graph");
        remove_circuit(&mut desired, "circuit-apsite");
        let mut desired = desired;
        // Keep the generated nodes circuit-bearing so they are not retired
        // (which would HOLD); re-anchor their circuits to new ids.
        add_circuit(
            &mut desired,
            "circuit-graph-2",
            "Customer Alpha",
            "libreqos:generated:graph:site:customer-alpha",
            "198.51.100.50",
            (10.0, 5.0, 100.0, 50.0),
        );
        add_circuit(
            &mut desired,
            "circuit-apsite-2",
            "Generated AP Site",
            "libreqos:generated:ap_site:1",
            "198.51.100.51",
            (10.0, 5.0, 100.0, 50.0),
        );
        set_circuit_rates(&mut desired, "circuit-beta", (25.0, 12.0, 250.0, 120.0));
        cases.push(build_case_tx(
            "canonical_mixed",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            false,
            true,
        ));
    }

    // 10. D1 MULTI-DEVICE circuit: an EXISTING circuit gains a SECOND device
    //     (a distinct `/32`). The compiler emits one Map per device IPv4, so the
    //     circuit legitimately holds >1 mapping — the live `.12` shape that the
    //     old per-circuit churn collapsed and `MultipleMappingsForCircuit`
    //     rejected. The falsifier is `oracle1_live_corpus_conservation`: without
    //     D1 `fold_seq` drops the second identity (`ref_maps != des_maps`); with
    //     D1 both are conserved.
    {
        let base = canonical_base();
        let mut desired = base.clone();
        add_device(
            &mut desired,
            "circuit-beta",
            "AP Beta",
            "uisp:device:ap-beta",
            "second",
            "198.51.100.12",
            (20.0, 10.0, 200.0, 100.0),
        );
        cases.push(build_case_tx(
            "canonical_multi_device_circuit",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            false,
            false,
        ));
    }

    // 11. PREFIX + TUNNEL on ONE circuit (the cross-family shape): circuit-beta
    //     gains a tunnel identity, so the compiler emits `MapTunnel(100,64)`
    //     ALONGSIDE its device prefix Map. Both families must survive the fold
    //     together and stay order-independent.
    {
        let base = canonical_base();
        let mut desired = base.clone();
        set_tunnel_identity(&mut desired, "circuit-beta", 100, 64);
        cases.push(build_case_tx(
            "canonical_prefix_and_tunnel",
            CANONICAL_SEED,
            base.compile(),
            desired.compile(),
            false,
            false,
            false,
        ));
    }

    cases.extend(legacy_corpus_aside());
    cases
}

/// SOURCE-INDEPENDENCE ASIDE: the legacy `network.json` + `ShapedDevices.csv`
/// pair recompiled through the SAME manager + producer. Kept to show the
/// corpus is not dependent on the seeding layer; NOT the primary path.
fn legacy_corpus_aside() -> Vec<GoldenCase> {
    let mut cases = Vec::new();

    // 1. rate-only churn — no structural change.
    {
        let base = World::from_fixtures();
        let mut desired = base.clone();
        set_device_rates(&mut desired.devices, "c1", (30, 3, 300, 30));
        set_device_rates(&mut desired.devices, "c2", (25, 12, 250, 120));
        cases.push(build_case("rate_only", base, desired, false));
    }

    // 2. add a circuit under the existing AP.
    {
        let base = World::from_fixtures();
        let mut desired = base.clone();
        push_device(
            &mut desired.devices,
            "c3",
            "d3",
            "AP One",
            "ap1",
            "192.0.2.12/32",
            (10, 5, 100, 50),
        );
        cases.push(build_case("add_circuit", base, desired, false));
    }

    // 3. add a NEW inner chain (AP Two) with two new circuits.
    {
        let base = World::from_fixtures();
        let mut desired = base.clone();
        insert_child(&mut desired.network, "s1", flat_node("ap2", "AP Two", "AP", 200.0, 200.0));
        push_device(
            &mut desired.devices,
            "c3",
            "d3",
            "AP Two",
            "ap2",
            "192.0.2.12/32",
            (10, 5, 100, 50),
        );
        push_device(
            &mut desired.devices,
            "c4",
            "d4",
            "AP Two",
            "ap2",
            "192.0.2.13/32",
            (15, 7, 150, 75),
        );
        cases.push(build_case("add_inners", base, desired, false));
    }

    // 4. remove a single circuit (drain only; no retire).
    {
        let base = World::from_fixtures();
        let mut desired = base.clone();
        remove_device(&mut desired.devices, "c2");
        cases.push(build_case("remove_circuit", base, desired, false));
    }

    // 5. remove a whole site (drain c1+c2, then retire ap1). The multi-remove
    //    drain's VERBATIM union carries a base Upsert + explicit Remove of the
    //    same circuit => the R-A collision fallback case.
    {
        let base = World::from_fixtures();
        let mut desired = base.clone();
        remove_node(&mut desired.network, "ap1");
        remove_device(&mut desired.devices, "c1");
        remove_device(&mut desired.devices, "c2");
        cases.push(build_case("remove_site", base, desired, true));
    }

    // 6. leaf re-parent (R-B: an Upsert with a changed parent, not a Move).
    {
        let base = World::from_fixtures();
        let mut base = base;
        insert_child(&mut base.network, "s1", flat_node("ap2", "AP Two", "AP", 200.0, 200.0));
        push_device(
            &mut base.devices,
            "c3",
            "d3",
            "AP Two",
            "ap2",
            "192.0.2.13/32",
            (10, 5, 100, 50),
        );
        let mut desired = base.clone();
        set_device_parent(&mut desired.devices, "c2", "AP Two", "ap2");
        cases.push(build_case("leaf_reparent", base, desired, false));
    }

    // 7. site re-parent (AP One moves from Site One to Site Two).
    {
        let base = World::from_fixtures();
        let mut base = base;
        insert_child(&mut base.network, "s1", flat_node("ap2", "AP Two", "AP", 200.0, 200.0));
        push_device(
            &mut base.devices,
            "c3",
            "d3",
            "AP Two",
            "ap2",
            "192.0.2.13/32",
            (10, 5, 100, 50),
        );
        // A second site that will own AP One after the move.
        let mut site_two = flat_node("s2", "Site Two", "Site", 500.0, 500.0);
        site_two["children"] = json!({});
        root_nodes_mut(&mut base.network).insert("Site Two".to_string(), site_two);

        let mut desired = base.clone();
        let ap1 = remove_node(&mut desired.network, "ap1").expect("ap1 exists");
        insert_child(&mut desired.network, "s2", ap1);
        cases.push(build_case("site_reparent", base, desired, false));
    }

    // 8. mixed: add a circuit + remove a circuit + rate churn, all at once.
    {
        let base = World::from_fixtures();
        let mut base = base;
        insert_child(&mut base.network, "s1", flat_node("ap2", "AP Two", "AP", 200.0, 200.0));
        push_device(
            &mut base.devices,
            "c3",
            "d3",
            "AP Two",
            "ap2",
            "192.0.2.13/32",
            (10, 5, 100, 50),
        );
        let mut desired = base.clone();
        push_device(
            &mut desired.devices,
            "c4",
            "d4",
            "AP Two",
            "ap2",
            "192.0.2.14/32",
            (20, 10, 200, 100),
        );
        remove_device(&mut desired.devices, "c2");
        set_device_rates(&mut desired.devices, "c1", (40, 20, 400, 200));
        cases.push(build_case("mixed", base, desired, false));
    }

    // 9. the radius-harness template fixture: rate churn.
    {
        let base = World::radius_fixture();
        let mut desired = base.clone();
        set_device_rates(&mut desired.devices, "r1", (30, 15, 300, 150));
        cases.push(build_case("radius_rate_only", base, desired, false));
    }

    // 10. the radius-harness template fixture: add a circuit.
    {
        let base = World::radius_fixture();
        let mut desired = base.clone();
        push_device(
            &mut desired.devices,
            "r2",
            "rd2",
            "PPPoE Access",
            "lab-pppoe-access",
            "192.0.2.22/32",
            (20, 10, 200, 100),
        );
        cases.push(build_case("radius_add", base, desired, false));
    }

    cases
}

// ─────────────────────────────────────────────────────────────────────────────
// (1) EQUIVALENCE, (2) ORDER-INDEPENDENCE, (5) CONSERVATION
// ─────────────────────────────────────────────────────────────────────────────

/// (1) EQUIVALENCE: the coherent delta folds to the SAME world as the real
/// per-op sequence — content_key + structural_identity + op vectors.
#[test]
fn oracle1_live_corpus_coherent_delta_converges_to_the_desired() {
    for case in corpus() {
        // A generated change is HELD (no delta) — covered by the dedicated
        // held/rebuild test.
        if case.held {
            continue;
        }
        // F3-P: the producer emits ONE coherent delta; folding it onto the base
        // must reach the COMPILED DESIRED world (the F3-B convergence property).
        let folded = fold_delta(&case.base, &case.coherent_delta);
        // §lqos-74t.12: the fold reaches the desired MINUS the deferred lone
        // sites (a POLICY gap, never silent).
        let expected = expected_after_deferrals(
            &case.desired,
            &case.base,
            &case.deferred_sites,
            &case.deferred_retires,
        );
        assert_eq!(
            content_key(&folded),
            content_key(&expected),
            "[{}] content_key(fold(base, delta)) != content_key(desired - deferred)",
            case.name
        );
        assert_eq!(
            structural_identity(&folded),
            structural_identity(&expected),
            "[{}] structural identity(fold(base, delta)) != desired - deferred",
            case.name
        );
        // NON-VACUOUS: the fold actually moved the world.
        assert_ne!(
            content_key(&folded),
            content_key(&case.base),
            "[{}] the transition is a no-op — the corpus is vacuous",
            case.name
        );
    }
}

/// §`lqos-74t.12`: the corpus MUST carry a lone-site case whose deferral is
/// REAL (non-vacuous coverage) — the convergence oracle is policy-aware
/// (`fold == desired - deferred_sites`).
#[test]
fn oracle1_live_corpus_covers_a_deferred_lone_site() {
    let case = corpus()
        .into_iter()
        .find(|c| c.name == "canonical_lone_site_deferred")
        .expect("the lone-site case is in the corpus");
    assert!(
        !case.deferred_sites.is_empty(),
        "the lone-site case must actually DEFER (a new inner with no circuit)"
    );
    assert!(
        case.coherent_delta.inner_ops.iter().all(|op| !matches!(
            op,
            DataplaneInnerOp::Upsert { node_id, .. } if case.deferred_sites.contains(node_id)
        )),
        "the deferred site must NOT appear in the emitted delta"
    );
}

/// (2) ORDER-INDEPENDENCE: the coherent delta's fold is invariant under ≥2
/// permutations (the reverse and a rotation), for every coherent case.
#[test]
fn oracle1_live_corpus_coherent_delta_is_order_independent() {
    let mut checked = 0usize;
    for case in corpus() {
        if case.held {
            continue;
        }
        // The CHANGED-op union `D*` is coherent even for the fallback case
        // (the collision lives in the VERBATIM all-ops union, tested below).
        let canonical = fold_delta(&case.base, &case.coherent_delta);

        let mut reverse = case.coherent_delta.clone();
        reverse.inner_ops.reverse();
        reverse.leaf_ops.reverse();
        reverse.mapping_ops.reverse();
        let reversed = fold_delta(&case.base, &reverse);
        assert_eq!(
            content_key(&reversed),
            content_key(&canonical),
            "[{}] reverse permutation changed the world",
            case.name
        );
        assert_eq!(
            structural_identity(&reversed),
            structural_identity(&canonical),
            "[{}] reverse permutation changed the structure",
            case.name
        );

        // A rotation: move the last op of each kind to the front.
        let mut rotated = case.coherent_delta.clone();
        if rotated.mapping_ops.len() > 1 {
            rotated.mapping_ops.rotate_right(1);
        }
        if rotated.leaf_ops.len() > 1 {
            rotated.leaf_ops.rotate_right(1);
        }
        if rotated.inner_ops.len() > 1 {
            rotated.inner_ops.rotate_right(1);
        }
        let rotated_world = fold_delta(&case.base, &rotated);
        assert_eq!(
            content_key(&rotated_world),
            content_key(&canonical),
            "[{}] rotation permutation changed the world",
            case.name
        );
        checked += 1;
    }
    assert!(checked >= 6, "the corpus must exercise ≥6 coherent cases");
}


/// (5) CONSERVATION: the final identity sets (inner/leaf/mapping) conserve
/// across the swap, and both arms equal the compiled desired world.
#[test]
fn oracle1_live_corpus_conservation() {
    for case in corpus() {
        if case.held {
            continue;
        }
        let reference = fold_seq(&case.base, &case.deltas);
        let via_delta = fold_delta(&case.base, &case.coherent_delta);
        let (ref_inners, ref_leaves, ref_maps) = identity_set(&reference);
        let (d_inners, d_leaves, d_maps) = identity_set(&via_delta);
        assert_eq!(ref_inners, d_inners, "[{}] inner identity set changed", case.name);
        assert_eq!(ref_leaves, d_leaves, "[{}] leaf identity set changed", case.name);
        assert_eq!(ref_maps, d_maps, "[{}] mapping identity set changed", case.name);

        // The sequence converges to the compiled desired world (identity
        // conserved from base through the swap to desired) — MINUS the §lqos-74t.12
        // deferred lone sites (a POLICY gap, never silent).
        let expected = expected_after_deferrals(
            &case.desired,
            &case.base,
            &case.deferred_sites,
            &case.deferred_retires,
        );
        let (des_inners, des_leaves, des_maps) = identity_set(&expected);
        assert_eq!(ref_inners, des_inners, "[{}] did not converge to desired inners", case.name);
        assert_eq!(ref_leaves, des_leaves, "[{}] did not converge to desired leaves", case.name);
        assert_eq!(ref_maps, des_maps, "[{}] did not converge to desired mappings", case.name);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The serialized golden (validated fresh here; replayed by bridge_rs)
// ─────────────────────────────────────────────────────────────────────────────

fn serialize_corpus(cases: &[GoldenCase]) -> String {
    let mut json = serde_json::to_string_pretty(cases).expect("corpus serializes");
    json.push('\n');
    json
}

/// PROOF the corpus is the real producer: the committed golden the bridge
/// replays must equal the corpus compiled + diffed FRESH here. A drift means
/// the golden is stale (or was hand-edited).
#[test]
fn oracle1_golden_matches_the_real_producer() {
    let fresh = serialize_corpus(&corpus());
    let path = golden_path();
    if std::env::var("ORACLE1_WRITE_GOLDEN").is_ok() {
        std::fs::create_dir_all(path.parent().expect("golden has a parent")).expect("golden dir");
        std::fs::write(&path, &fresh).expect("write golden");
        eprintln!("ORACLE1: wrote golden {}", path.display());
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "the oracle1 golden is missing at {} ({err}); regenerate with \
             ORACLE1_WRITE_GOLDEN=1 cargo test -p lqosd oracle1_golden_matches_the_real_producer",
            path.display()
        )
    });
    assert_eq!(
        committed, fresh,
        "the committed oracle1 golden != fresh real-producer output — regenerate it \
         (never hand-edit): ORACLE1_WRITE_GOLDEN=1 cargo test -p lqosd oracle1_golden_matches_the_real_producer"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// COVERAGE: no silent regression of the generated / attachment shapes
// ─────────────────────────────────────────────────────────────────────────────

/// D1: does any circuit hold MORE than one mapping identity? The live
/// multi-device shape the corpus must carry (else the conservation falsifier
/// is blind to the class the old per-circuit churn collapsed).
fn has_multi_mapping_circuit(tx: &DataplaneTransaction) -> bool {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for op in &tx.mapping_ops {
        let cid = match op {
            DataplaneMappingOp::Map { circuit_id, .. } => Some(circuit_id.clone()),
            DataplaneMappingOp::MapTunnel { circuit, .. } => Some(circuit.clone()),
            DataplaneMappingOp::MapVlan { circuit, .. } => Some(circuit.clone()),
            DataplaneMappingOp::MapMpls { circuit, .. } => Some(circuit.clone()),
            _ => None,
        };
        if let Some(cid) = cid {
            *counts.entry(cid).or_default() += 1;
        }
    }
    counts.values().any(|n| *n > 1)
}

/// R3 residual: does any circuit hold BOTH a prefix `Map` and a tunnel
/// `MapTunnel`? The cross-family shape the corpus must carry.
fn has_cross_family_mapping(tx: &DataplaneTransaction) -> bool {
    let mut prefix: BTreeSet<String> = BTreeSet::new();
    let mut tunnel: BTreeSet<String> = BTreeSet::new();
    for op in &tx.mapping_ops {
        match op {
            DataplaneMappingOp::Map { circuit_id, .. } => {
                prefix.insert(circuit_id.clone());
            }
            DataplaneMappingOp::MapTunnel { circuit, .. } => {
                tunnel.insert(circuit.clone());
            }
            _ => {}
        }
    }
    prefix.intersection(&tunnel).next().is_some()
}

fn has_generated(tx: &DataplaneTransaction) -> bool {
    tx.inner_ops.iter().any(|op| {
        matches!(
            op,
            DataplaneInnerOp::Upsert { node_id, .. }
                if node_id.starts_with("libreqos:generated:")
        )
    })
}

fn generated_edges(tx: &DataplaneTransaction) -> BTreeMap<String, String> {
    tx.inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert {
                node_id,
                parent_node_id,
                ..
            } if node_id.starts_with("libreqos:generated:") => {
                Some((node_id.clone(), parent_node_id.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The corpus MUST contain `libreqos:generated:*` nodes and at least one case
/// authored over an AP served through a station (the ATTACHMENT edge). This
/// pins coverage so it cannot silently regress to a plain tree.
#[test]
fn oracle1_live_corpus_covers_generated_shapes_and_attachments() {
    let cases = corpus();
    assert!(
        cases
            .iter()
            .any(|case| has_generated(&case.base) || has_generated(&case.desired)),
        "the corpus must contain libreqos:generated:* nodes"
    );
    assert!(
        cases.iter().any(|case| case.ap_with_attachment),
        "the corpus must include an AP served through a station (attachment) case"
    );
    // The authored manager INPUT itself models the attachment edge (an AP node
    // whose current_attachment_id names a station), not just a node.
    let base = canonical_base();
    assert!(
        base.canonical.nodes.iter().any(|node| {
            node.node_kind == "AP" && node.current_attachment_id.is_some()
        }),
        "the authored canonical base must model an AP with an attachment edge"
    );
    let distinct_seeds: BTreeSet<&str> = cases.iter().map(|case| case.seed.as_str()).collect();
    assert!(
        distinct_seeds.len() >= 2,
        "the corpus must span >= 2 seeding layers (got {distinct_seeds:?})"
    );
    // D1: the corpus must carry the multi-device shape that trips the old
    // per-circuit mapping churn / `MultipleMappingsForCircuit`.
    assert!(
        cases
            .iter()
            .any(|case| has_multi_mapping_circuit(&case.desired)),
        "the corpus must contain a circuit with >1 mapping identity (D1)"
    );
    // R3 residual: the corpus must also carry the CROSS-FAMILY shape (a prefix
    // Map + a MapTunnel on one circuit).
    assert!(
        cases
            .iter()
            .any(|case| has_cross_family_mapping(&case.desired)),
        "the corpus must contain a circuit with both a prefix and a tunnel mapping"
    );
}

/// (3b) GENERATED HOLDS: a changed `libreqos:generated:*` subtree is held by
/// the producer (no live generated move — the `.12` invariant). The held
/// transition is still a REAL change: the R3/bundle rebuild (the compiled
/// desired world) is the world the lane converges to.
#[test]
fn oracle1_live_corpus_generated_changes_now_flow() {
    // P7/B3 RETIRED (R3 card §2; DRIVER (b)): a changed `libreqos:generated:*`
    // subtree is NO LONGER held by the producer — it emits a coherent delta that
    // folds to the desired. A generated re-parent is R-B expressible (or the
    // bridge's re-parent owner-set belt -> RebuildRequired -> R3); a retire is a
    // normal Remove.
    let mut saw_generated_reparent = false;
    let mut saw_generated_add = false;
    let mut saw_held = 0usize;
    for case in corpus() {
        if !has_generated(&case.desired) && !has_generated(&case.base) {
            continue;
        }
        if case.held {
            saw_held += 1;
            continue;
        }
        let folded = fold_delta(&case.base, &case.coherent_delta);
        let expected = expected_after_deferrals(
            &case.desired,
            &case.base,
            &case.deferred_sites,
            &case.deferred_retires,
        );
        assert_eq!(
            content_key(&folded),
            content_key(&expected),
            "[{}] a generated change must FLOW (fold to the desired - deferred), not be held",
            case.name
        );
        let base_edges = generated_edges(&case.base);
        let desired_edges = generated_edges(&case.desired);
        for (id, parent) in &desired_edges {
            match base_edges.get(id) {
                Some(base_parent) if base_parent != parent => saw_generated_reparent = true,
                None => saw_generated_add = true,
                _ => {}
            }
        }
    }
    assert_eq!(saw_held, 0, "no generated change may be HELD after P7/B3");
    assert!(
        saw_generated_reparent,
        "the corpus must include a libreqos:generated:* RE-PARENT (the LIVE .12 class)"
    );
    assert!(
        saw_generated_add,
        "the corpus must include a libreqos:generated:* ADD"
    );
}

/// D1 mapping-model producer half (U3): a DRAWN DESIRED
/// circuit must carry an EXPLICIT `Del` for every base mapping identity that is
/// no longer desired — the per-identity fold (U1) no longer retires the old key
/// on a `Map` re-bind, so the view must say so (mirrors the rate path
/// `translate.rs:1211-1255`). Forces the STRUCTURAL path with a concurrent add.
#[test]
fn structural_view_emits_explicit_del_for_a_survivors_removed_mapping() {
    let base = canonical_base();
    let mut desired = base.clone();
    // circuit-beta base prefix 198.51.100.11 → desired 198.51.100.99.
    set_circuit_ip(&mut desired, "circuit-beta", "198.51.100.99");
    // Force the structural path (a concurrent new circuit).
    add_circuit(
        &mut desired,
        "circuit-delta",
        "AP Beta",
        "uisp:device:ap-beta",
        "198.51.100.77",
        (5.0, 2.0, 50.0, 20.0),
    );
    let outcome = semantic_diff(&desired.compile(), Some(&base.compile()), None);
    let delta = outcome
        .delta
        .expect("the transition must emit a coherent delta");

    let old_ip: std::net::IpAddr = "198.51.100.11".parse().unwrap();
    let new_ip: std::net::IpAddr = "198.51.100.99".parse().unwrap();
    let maps_new = delta
        .mapping_ops
        .iter()
        .any(|m| matches!(m, DataplaneMappingOp::Map { ip, .. } if ip == &new_ip));
    let has_old_del = delta
        .mapping_ops
        .iter()
        .any(|m| matches!(m, DataplaneMappingOp::Del { ip, .. } if ip == &old_ip));
    assert!(
        maps_new && has_old_del,
        "U3: a re-bound survivor's delta must carry an explicit Del(old) beside its Map(new)"
    );
}

/// U3 SCOPE: a PENDING (not-yet-drained) base circuit keeps
/// its base maps in the drain views. The view must NOT emit a `Del` for its map
/// unless that same delta is its explicit drain tx (carries its `Remove`) — a
/// global "absent from desired ⇒ Del" scan deletes live mappings before the drain.
#[test]
fn structural_view_does_not_del_a_pending_circuits_mapping_early() {
    let base = canonical_base();
    let mut desired = base.clone();
    remove_circuit(&mut desired, "circuit-beta");
    let outcome = semantic_diff(&desired.compile(), Some(&base.compile()), None);

    let delta = outcome
        .delta
        .expect("a removed circuit emits a coherent delta");

    let pending_ip: std::net::IpAddr = "198.51.100.11".parse().unwrap();
    let has_remove = delta
        .leaf_ops
        .iter()
        .any(|op| matches!(op, DataplaneLeafOp::Remove { circuit_id } if circuit_id.as_str() == "circuit-beta"));
    let has_del = delta
        .mapping_ops
        .iter()
        .any(|m| matches!(m, DataplaneMappingOp::Del { ip, .. } if ip == &pending_ip));
    assert!(has_remove, "the pending circuit must emit its drain Remove");
    assert!(
        !has_del || has_remove,
        "U3: a pending circuit's mapping is deleted only in its own drain delta"
    );
}

/// U4 (v2-parity Ignored Subnets) end-to-end: a shared CPE LAN address
/// (`192.168.1.1/32`, the LIVE `.12` duplicate-identity class) on two circuits
/// must be pruned at the topology choke, so the EMITTED desired has ZERO
/// duplicate mapping identities and `content_key(desired) == content_key(fold)`.
#[test]
fn oracle1_live_ignored_subnets_prune_the_duplicate_identity() {
    let mut world = canonical_base();
    add_circuit(
        &mut world,
        "circuit-dup-a",
        "AP Beta",
        "uisp:device:ap-beta",
        "100.125.0.1",
        (10.0, 5.0, 100.0, 50.0),
    );
    add_circuit(
        &mut world,
        "circuit-dup-b",
        "AP Beta",
        "uisp:device:ap-beta",
        "100.125.0.2",
        (10.0, 5.0, 100.0, 50.0),
    );
    for device in &mut world.devices {
        if device.circuit_id == "circuit-dup-a" || device.circuit_id == "circuit-dup-b" {
            let wan = device.ipv4[0].0;
            device.ipv4 = vec![(wan, 32), ("192.168.1.1".parse().unwrap(), 32)];
        }
    }

    let desired = world.compile();

    // (4) ZERO duplicate mapping identities in the emitted desired.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for op in &desired.mapping_ops {
        let identity = map_identity(op);
        assert!(
            seen.insert(identity.clone()),
            "duplicate mapping identity in the emitted desired: {identity}"
        );
    }
    // The ignored CPE address never reaches mapping_ops.
    assert!(
        !desired.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::Map { ip, .. } if ip.to_string() == "192.168.1.1"
        )),
        "the ignored 192.168.1.1 must be pruned before mapping_ops"
    );
    // The WAN /32s survive (the circuits are not over-pruned).
    for wan in ["100.125.0.1", "100.125.0.2"] {
        assert!(
            desired.mapping_ops.iter().any(|op| matches!(
                op,
                DataplaneMappingOp::Map { ip, .. } if ip.to_string() == wan
            )),
            "the surviving WAN {wan} must remain"
        );
    }

    // The acceptance invariant: the per-identity fold reproduces the emitted set.
    let folded = AppliedFold::from_tx(&desired).to_transaction("fold");
    assert_eq!(
        content_key(&folded),
        content_key(&desired),
        "U4: content_key(desired) must equal content_key(fold) (no duplicate identity)"
    );
}

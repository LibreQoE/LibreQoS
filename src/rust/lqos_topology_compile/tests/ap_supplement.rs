//! lqos-74t.196 step 5d: the AP/OLT supplement at the shared choke point.
//!
//! Ports the deleted Python planner's semantics into Rust and proves that an
//! absent/disabled/stale supplement leaves the four artifacts byte-identical.

use lqos_config::{
    CIRCUIT_ANCHORS_FILENAME, CircuitAnchor, CircuitAnchorsFile, Config, ConfigShapedDevices,
    ShapedDevice, TOPOLOGY_CANONICAL_STATE_FILENAME, TOPOLOGY_COMPILED_SHAPING_FILENAME,
    TOPOLOGY_IMPORT_FILENAME, TOPOLOGY_PARENT_CANDIDATES_FILENAME, TopologyCanonicalStateFile,
};
use lqos_topology_compile::ap_supplement::{
    AP_OBSERVATIONS_FILENAME, AP_SUPPLEMENT_REPORT_FILENAME, ApObservations, SupplementPins, apply,
    plan_supplement,
};
use lqos_topology_compile::{
    ImportedTopologyBundle, TopologyWriteOrigin, compile_and_write_imported,
};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

fn temp_config(tag: &str, enabled: bool) -> Config {
    let dir = std::env::temp_dir().join(format!(
        "lqos-5d-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut config = Config {
        lqos_directory: dir.display().to_string(),
        state_directory: Some(dir.join("state").display().to_string()),
        ..Config::default()
    };
    config.ap_polling.enabled = enabled;
    config.ap_polling.apply_placement = true;
    config
}

fn shaped(
    circuit_id: &str,
    circuit_name: &str,
    device_id: &str,
    mac: &str,
    parent_id: &str,
    parent_name: &str,
) -> ShapedDevice {
    ShapedDevice {
        circuit_id: circuit_id.to_string(),
        circuit_name: circuit_name.to_string(),
        device_id: device_id.to_string(),
        device_name: device_id.to_string(),
        parent_node: parent_name.to_string(),
        parent_node_id: Some(parent_id.to_string()),
        mac: mac.to_string(),
        download_min_mbps: 10.0,
        upload_min_mbps: 5.0,
        download_max_mbps: 100.0,
        upload_max_mbps: 50.0,
        ..Default::default()
    }
}

fn anchor(circuit_id: &str, parent_id: &str, parent_name: &str) -> CircuitAnchor {
    CircuitAnchor {
        circuit_id: circuit_id.to_string(),
        circuit_name: Some(format!("Circuit {circuit_id}")),
        anchor_node_id: parent_id.to_string(),
        anchor_node_name: Some(parent_name.to_string()),
    }
}

fn network_tree() -> Value {
    json!({
        "Site One": {
            "id": "s1",
            "name": "Site One",
            "type": "Site",
            "downloadBandwidthMbps": 1000,
            "uploadBandwidthMbps": 1000,
            "children": {
                "AP One": {
                    "id": "ap1",
                    "name": "AP One",
                    "type": "AP",
                    "downloadBandwidthMbps": 500,
                    "uploadBandwidthMbps": 500,
                    "children": {}
                },
                "AP Two": {
                    "id": "ap2",
                    "name": "AP Two",
                    "type": "AP",
                    "downloadBandwidthMbps": 0,
                    "uploadBandwidthMbps": 0,
                    "children": {}
                }
            }
        }
    })
}

fn fixture_bundle() -> ImportedTopologyBundle {
    let network = network_tree();
    let canonical = TopologyCanonicalStateFile::from_legacy_network_json(&network);
    let mut devices = ConfigShapedDevices::default();
    devices.replace_with_new_data(vec![
        shaped(
            "c1",
            "Circuit One",
            "d1",
            "aa:bb:cc:dd:ee:ff",
            "ap1",
            "AP One",
        ),
        shaped(
            "c2",
            "Circuit Two",
            "d2",
            "aa:bb:cc:dd:ee:02",
            "ap1",
            "AP One",
        ),
    ]);
    ImportedTopologyBundle {
        source: "test/supplement".to_string(),
        generated_unix: None,
        ingress_identity: None,
        native_canonical: Some(canonical),
        parent_candidates: Some(Default::default()),
        compatibility_network_json: network,
        shaped_devices: devices,
        circuit_anchors: CircuitAnchorsFile {
            schema_version: 1,
            source: "test/supplement".to_string(),
            generated_unix: None,
            anchors: vec![anchor("c1", "ap1", "AP One"), anchor("c2", "ap1", "AP One")],
        },
        ethernet_advisories: Vec::new(),
        circuit_matches: Default::default(),
    }
}

fn fixture_with_duplicate_mac() -> ImportedTopologyBundle {
    let mut bundle = fixture_bundle();
    for device in &mut bundle.shaped_devices.devices {
        if device.circuit_id == "c2" {
            device.mac = "aa:bb:cc:dd:ee:ff".to_string();
        }
    }
    bundle
}

fn write_transaction(config: &Config, generation: &str) {
    let path = lqos_config::topology_dataplane_transaction_path(config);
    std::fs::create_dir_all(path.parent().expect("transaction parent")).expect("transaction dir");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "generation": generation,
            "inner_ops": [],
            "leaf_ops": [],
            "mapping_ops": []
        }))
        .expect("transaction json"),
    )
    .expect("transaction write");
}

fn write_observations_raw(config: &Config, raw: &[u8]) {
    let path = config.shaping_state_file_path(AP_OBSERVATIONS_FILENAME);
    std::fs::create_dir_all(path.parent().expect("observations parent")).expect("observations dir");
    std::fs::write(path, raw).expect("observations write");
}

fn write_observations(config: &Config, generation: &str, devices: Value) {
    write_transaction(config, generation);
    write_observations_raw(
        config,
        &serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "topology_generation": generation,
            "written_at": 1,
            "devices": devices
        }))
        .expect("observations json"),
    );
}

fn observations(devices: Value) -> ApObservations {
    ApObservations {
        schema_version: 1,
        topology_generation: "gen-1".to_string(),
        devices: serde_json::from_value(devices).expect("device map"),
    }
}

fn report_path(config: &Config) -> PathBuf {
    config.shaping_state_file_path(AP_SUPPLEMENT_REPORT_FILENAME)
}

fn artifact_paths(config: &Config) -> [(&'static str, PathBuf); 4] {
    [
        (
            "topology_import.json",
            config.topology_state_file_path(TOPOLOGY_IMPORT_FILENAME),
        ),
        (
            "topology_compiled_shaping.json",
            config.shaping_state_file_path(TOPOLOGY_COMPILED_SHAPING_FILENAME),
        ),
        (
            "topology_parent_candidates.json",
            config.topology_state_file_path(TOPOLOGY_PARENT_CANDIDATES_FILENAME),
        ),
        (
            "topology_canonical_state.json",
            config.topology_state_file_path(TOPOLOGY_CANONICAL_STATE_FILENAME),
        ),
    ]
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn compile(config: &Config) -> Vec<Vec<u8>> {
    compile_and_write_imported(
        config,
        fixture_bundle(),
        "full",
        TopologyWriteOrigin::Api,
        None,
    )
    .expect("compile");
    artifact_paths(config)
        .iter()
        .map(|(_, path)| read(path))
        .collect()
}

fn devices_with(node_id: &str, record: Value) -> Value {
    let mut devices = serde_json::Map::new();
    devices.insert(node_id.to_string(), record);
    Value::Object(devices)
}

fn task_mac() -> &'static str {
    "aa:bb:cc:dd:ee:ff"
}

#[test]
fn station_match_reparents_the_circuit_and_writes_the_report() {
    let config = temp_config("match", true);
    write_observations(
        &config,
        "gen-1",
        json!({
            "ap2": {
                "sequence": 0,
                "observed_at": 1,
                "stations": [{"router_mac": task_mac()}]
            }
        }),
    );
    compile(&config);

    let compiled: Value = serde_json::from_slice(&read(
        &config.shaping_state_file_path(TOPOLOGY_COMPILED_SHAPING_FILENAME),
    ))
    .expect("compiled shaping json");
    let anchors = compiled["circuit_anchors"]["anchors"]
        .as_array()
        .expect("anchors");
    let c1 = anchors
        .iter()
        .find(|anchor| anchor["circuit_id"] == "c1")
        .expect("c1 anchor");
    assert_eq!(c1["anchor_node_id"], "ap2");
    assert_eq!(c1["anchor_node_name"], "AP Two");

    let report: Value = serde_json::from_slice(&read(&report_path(&config))).expect("report json");
    assert_eq!(report["matched"], 1);
    assert_eq!(report["unmatched"], 0);
    assert_eq!(report["ambiguous"], 0);
    assert_eq!(
        report["applied_edits"],
        json!([{
            "circuit_id": "c1",
            "old_parent": "ap1",
            "new_parent": "ap2",
            "source": "observed"
        }])
    );
}

#[test]
fn pinned_circuit_is_not_moved_and_reports_drift() {
    let config = temp_config("pin", true);
    write_observations(
        &config,
        "gen-1",
        json!({
            "ap2": {
                "sequence": 0,
                "observed_at": 1,
                "stations": [{"router_mac": task_mac()}]
            }
        }),
    );
    let pins = SupplementPins {
        placement: HashMap::from([("c1".to_string(), "ap1".to_string())]),
        capacity: BTreeSet::new(),
    };
    let (bundle, plan) = apply(&config, fixture_bundle(), &pins);
    let plan = plan.expect("valid observations");
    assert!(plan.edits.is_empty(), "pinned circuits are never moved");
    assert_eq!(plan.matched, 1);
    assert_eq!(plan.pinned_overrides, 1);
    assert_eq!(plan.drift, 1);
    let c1 = bundle
        .circuit_anchors
        .anchors
        .iter()
        .find(|anchor| anchor.circuit_id == "c1")
        .expect("c1 anchor");
    assert_eq!(c1.anchor_node_id, "ap1");
}

#[test]
fn unmatched_mac_is_counted_and_not_edited() {
    let config = temp_config("unmatched", true);
    let plan = plan_supplement(
        &fixture_bundle(),
        &observations(json!({
            "ap2": {
                "sequence": 0,
                "observed_at": 1,
                "stations": [{"router_mac": "11:22:33:44:55:66"}]
            }
        })),
        &SupplementPins::default(),
        &config,
    );
    assert!(plan.edits.is_empty());
    assert_eq!(plan.matched, 0);
    assert_eq!(plan.unmatched, 1);
    assert_eq!(plan.ambiguous, 0);
}

#[test]
fn two_circuits_claiming_one_mac_are_ambiguous() {
    let config = temp_config("ambiguous-circuit", true);
    let plan = plan_supplement(
        &fixture_with_duplicate_mac(),
        &observations(json!({
            "ap2": {
                "sequence": 0,
                "observed_at": 1,
                "stations": [{"router_mac": task_mac()}]
            }
        })),
        &SupplementPins::default(),
        &config,
    );
    assert!(plan.edits.is_empty());
    assert_eq!(plan.matched, 0);
    assert_eq!(plan.ambiguous, 1);
}

#[test]
fn two_stations_claiming_one_mac_are_ambiguous() {
    let config = temp_config("ambiguous-station", true);
    let plan = plan_supplement(
        &fixture_bundle(),
        &observations(json!({
            "ap1": {
                "sequence": 0,
                "observed_at": 1,
                "stations": [{"router_mac": task_mac()}]
            },
            "ap2": {
                "sequence": 0,
                "observed_at": 1,
                "stations": [{"router_mac": task_mac()}]
            }
        })),
        &SupplementPins::default(),
        &config,
    );
    assert!(plan.edits.is_empty());
    assert_eq!(plan.matched, 0);
    assert_eq!(plan.ambiguous, 1);
}

#[test]
fn fibre_onu_uses_the_observed_parent_node() {
    let config = temp_config("fibre", true);
    let mut bundle = fixture_bundle();
    let canonical = bundle.native_canonical.as_mut().expect("canonical");
    canonical.nodes.push(lqos_config::TopologyCanonicalNode {
        node_id: "onu-1".to_string(),
        node_name: "ONU One".to_string(),
        node_kind: "device".to_string(),
        current_parent_node_id: Some("ap2".to_string()),
        ..Default::default()
    });
    let plan = plan_supplement(
        &bundle,
        &observations(json!({
            "olt-1": {
                "sequence": 0,
                "observed_at": 1,
                "onus": [{
                    "router_mac": task_mac(),
                    "parent_node_id": "onu-1"
                }]
            }
        })),
        &SupplementPins::default(),
        &config,
    );
    assert_eq!(plan.matched, 1);
    assert_eq!(plan.edits.len(), 1);
    assert_eq!(plan.edits[0].circuit_id, "c1");
    assert_eq!(plan.edits[0].new_parent, "onu-1");
}

struct CapacityCase {
    name: &'static str,
    node: &'static str,
    phy: f64,
    factor: Option<f64>,
    default_factor: f64,
    capacity_pin: bool,
    expected: &'static str,
    expected_proposed: Option<f64>,
}

#[test]
fn capacity_precedence_matrix() {
    let cases = [
        CapacityCase {
            name: "used",
            node: "ap2",
            phy: 100.0,
            factor: Some(0.7),
            default_factor: 0.7,
            capacity_pin: false,
            expected: "used",
            expected_proposed: Some(70.0),
        },
        CapacityCase {
            name: "ignored_pin",
            node: "ap2",
            phy: 100.0,
            factor: Some(0.7),
            default_factor: 0.7,
            capacity_pin: true,
            expected: "ignored_pin",
            expected_proposed: Some(70.0),
        },
        CapacityCase {
            name: "ignored_crm_rate",
            node: "ap1",
            phy: 100.0,
            factor: Some(0.7),
            default_factor: 0.7,
            capacity_pin: false,
            expected: "ignored_crm_rate",
            expected_proposed: Some(70.0),
        },
        CapacityCase {
            name: "ignored_no_factor",
            node: "ap2",
            phy: 100.0,
            factor: None,
            default_factor: 0.0,
            capacity_pin: false,
            expected: "ignored_no_factor",
            expected_proposed: None,
        },
        CapacityCase {
            name: "ignored_invalid_factor",
            node: "ap2",
            phy: 100.0,
            factor: Some(0.0),
            default_factor: 0.7,
            capacity_pin: false,
            expected: "ignored_invalid_factor",
            expected_proposed: None,
        },
        CapacityCase {
            name: "ignored_invalid_capacity",
            node: "ap2",
            phy: 0.0,
            factor: Some(0.7),
            default_factor: 0.7,
            capacity_pin: false,
            expected: "ignored_invalid_capacity",
            expected_proposed: None,
        },
        CapacityCase {
            name: "ignored_unknown_device",
            node: "ghost",
            phy: 100.0,
            factor: Some(0.7),
            default_factor: 0.7,
            capacity_pin: false,
            expected: "ignored_unknown_device",
            expected_proposed: Some(70.0),
        },
        CapacityCase {
            name: "used_via_config_default",
            node: "ap2",
            phy: 100.0,
            factor: None,
            default_factor: 0.7,
            capacity_pin: false,
            expected: "used",
            expected_proposed: Some(70.0),
        },
    ];
    for case in cases {
        let mut config = temp_config(case.name, true);
        config.ap_polling.capacity_factor_default = case.default_factor;
        let capacity = match (case.phy, case.factor) {
            (phy, Some(factor)) => json!({"phy_mbps": phy, "factor": factor}),
            (phy, None) => json!({"phy_mbps": phy}),
        };
        let plan = plan_supplement(
            &fixture_bundle(),
            &observations(devices_with(
                case.node,
                json!({
                    "sequence": 0,
                    "observed_at": 1,
                    "stations": [],
                    "capacity": capacity
                }),
            )),
            &SupplementPins {
                placement: HashMap::new(),
                capacity: if case.capacity_pin {
                    BTreeSet::from(["ap2".to_string()])
                } else {
                    BTreeSet::new()
                },
            },
            &config,
        );
        assert_eq!(plan.capacity.len(), 1, "{}", case.name);
        let entry = &plan.capacity[0];
        assert_eq!(entry.node_id, case.node, "{}", case.name);
        assert_eq!(entry.decision, case.expected, "{}", case.name);
        assert_eq!(entry.proposed_mbps, case.expected_proposed, "{}", case.name);
        assert_eq!(
            plan.capacity_used,
            usize::from(case.expected == "used"),
            "{}",
            case.name
        );
        assert_eq!(
            plan.capacity_ignored,
            usize::from(case.expected != "used"),
            "{}",
            case.name
        );
    }
}

#[test]
fn used_capacity_updates_the_canonical_and_compatibility_rates() {
    let config = temp_config("capacity-applied", true);
    write_observations(
        &config,
        "gen-1",
        json!({
            "ap2": {
                "sequence": 0,
                "observed_at": 1,
                "stations": [],
                "capacity": {"phy_mbps": 100, "factor": 0.7}
            }
        }),
    );
    let (bundle, plan) = apply(&config, fixture_bundle(), &SupplementPins::default());
    let plan = plan.expect("valid observations");
    assert_eq!(plan.capacity_used, 1);
    let canonical = bundle.native_canonical.as_ref().expect("canonical");
    let ap2 = canonical
        .nodes
        .iter()
        .find(|node| node.node_id == "ap2")
        .expect("ap2");
    assert_eq!(ap2.rate_input.intrinsic_download_mbps, Some(70));
    assert_eq!(ap2.rate_input.intrinsic_upload_mbps, Some(70));
    let tree_ap2 = find_tree_node(&bundle.compatibility_network_json, "ap2").expect("ap2 tree");
    assert_eq!(tree_ap2["downloadBandwidthMbps"], 70.0);
    assert_eq!(tree_ap2["uploadBandwidthMbps"], 70.0);
}

#[test]
fn apply_placement_false_is_capacity_only_and_true_restores_reparenting() {
    let observation = json!({
        "ap2": {
            "sequence": 0,
            "observed_at": 1,
            "stations": [{"router_mac": task_mac()}],
            "capacity": {"phy_mbps": 100, "factor": 0.7}
        }
    });

    // v1 default: the station would reparent c1 to ap2, but placement is
    // off; capacity still applies and the report carries edits=[].
    let mut capacity_only = temp_config("capacity-only-false", true);
    capacity_only.ap_polling.apply_placement = false;
    write_observations(&capacity_only, "gen-1", observation.clone());
    let (bundle, plan) = apply(&capacity_only, fixture_bundle(), &SupplementPins::default());
    let plan = plan.expect("valid observations");
    assert!(plan.edits.is_empty(), "placement is off: no edits");
    assert_eq!(plan.matched, 0);
    assert_eq!(plan.capacity_used, 1);
    let c1 = bundle
        .circuit_anchors
        .anchors
        .iter()
        .find(|anchor| anchor.circuit_id == "c1")
        .expect("c1 anchor");
    assert_eq!(c1.anchor_node_id, "ap1", "no circuit parent may change");
    let report: Value =
        serde_json::from_slice(&read(&report_path(&capacity_only))).expect("report json");
    assert_eq!(report["applied_edits"], json!([]));

    // The switch restores the pre-v1 behaviour exactly.
    let mut placement = temp_config("capacity-only-true", true);
    placement.ap_polling.apply_placement = true;
    write_observations(&placement, "gen-1", observation);
    let (bundle, plan) = apply(&placement, fixture_bundle(), &SupplementPins::default());
    let plan = plan.expect("valid observations");
    assert_eq!(plan.edits.len(), 1);
    assert_eq!(plan.matched, 1);
    assert_eq!(plan.capacity_used, 1);
    let c1 = bundle
        .circuit_anchors
        .anchors
        .iter()
        .find(|anchor| anchor.circuit_id == "c1")
        .expect("c1 anchor");
    assert_eq!(c1.anchor_node_id, "ap2");
}

#[test]
fn polled_capacity_and_station_inventory_apply_without_placement() {
    let mut config = temp_config("polled-capacity-inventory", true);
    config.ap_polling.apply_placement = false;
    write_observations(
        &config,
        "gen-1",
        json!({
            "ap2": {
                "sequence": 0,
                "observed_at": 1,
                "station_count": 14,
                "stations": [],
                "capacity_mbps": 70.0,
                "capacity_source": "per_sm_rate"
            }
        }),
    );
    let mut bundle = fixture_bundle();
    for index in 0..11 {
        bundle
            .circuit_anchors
            .anchors
            .push(anchor(&format!("crm-{index}"), "ap2", "AP Two"));
    }
    let (bundle, plan) = apply(&config, bundle, &SupplementPins::default());
    let plan = plan.expect("valid observations");
    assert!(plan.edits.is_empty(), "placement is off");
    assert_eq!(plan.capacity_used, 1);
    assert_eq!(plan.capacity_ignored, 0);

    let canonical = bundle.native_canonical.as_ref().expect("canonical");
    let ap2 = canonical
        .nodes
        .iter()
        .find(|node| node.node_id == "ap2")
        .expect("ap2");
    assert_eq!(ap2.rate_input.intrinsic_download_mbps, Some(70));
    assert_eq!(ap2.rate_input.intrinsic_upload_mbps, Some(70));

    let report: Value = serde_json::from_slice(&read(&report_path(&config))).expect("report json");
    assert_eq!(report["applied_edits"], json!([]));
    assert_eq!(report["capacity_applied"][0]["node_id"], "ap2");
    assert_eq!(report["capacity_applied"][0]["proposed_mbps"], 70.0);
    assert_eq!(report["station_inventory"][0]["node_id"], "ap2");
    assert_eq!(report["station_inventory"][0]["station_count"], 14);
    assert_eq!(report["station_inventory"][0]["crm_pinned_count"], 11);
    assert_eq!(report["station_inventory"][0]["delta"], 3);
}

fn find_tree_node<'a>(value: &'a Value, node_id: &str) -> Option<&'a Value> {
    let map = value.as_object()?;
    for value in map.values() {
        let Some(node) = value.as_object() else {
            continue;
        };
        if node.get("id").and_then(Value::as_str) == Some(node_id) {
            return Some(value);
        }
        if let Some(children) = node.get("children")
            && let Some(found) = find_tree_node(children, node_id)
        {
            return Some(found);
        }
    }
    None
}

#[test]
fn missing_malformed_stale_or_disabled_is_a_byte_identical_noop() {
    let baseline_config = temp_config("noop-baseline", false);
    let baseline = compile(&baseline_config);

    // Missing observations.
    let missing = temp_config("noop-missing", true);
    assert_eq!(compile(&missing), baseline);
    assert!(!report_path(&missing).exists());

    // Malformed / truncated.
    let malformed = temp_config("noop-malformed", true);
    write_observations_raw(&malformed, b"{not json");
    assert_eq!(compile(&malformed), baseline);
    assert!(!report_path(&malformed).exists());

    // Wrong schema.
    let wrong_schema = temp_config("noop-schema", true);
    write_transaction(&wrong_schema, "gen-1");
    write_observations_raw(
        &wrong_schema,
        br#"{"schema_version":2,"topology_generation":"gen-1","written_at":1,"devices":{}}"#,
    );
    assert_eq!(compile(&wrong_schema), baseline);
    assert!(!report_path(&wrong_schema).exists());

    // Stale generation (observations vs the sibling transaction).
    let stale = temp_config("noop-stale", true);
    write_transaction(&stale, "gen-2");
    write_observations_raw(
        &stale,
        br#"{"schema_version":1,"topology_generation":"gen-1","written_at":1,"devices":{}}"#,
    );
    assert_eq!(compile(&stale), baseline);
    assert!(!report_path(&stale).exists());

    // Disabled with a valid file: must not read it or write a report.
    let disabled = temp_config("noop-disabled", false);
    write_observations(
        &disabled,
        "gen-1",
        json!({
            "ap2": {
                "sequence": 0,
                "observed_at": 1,
                "stations": [{"router_mac": task_mac()}]
            }
        }),
    );
    assert_eq!(compile(&disabled), baseline);
    assert!(!report_path(&disabled).exists());
}

#[test]
fn absent_observations_are_stable_across_two_compiles() {
    let baseline_config = temp_config("stable-baseline", false);
    let baseline = compile(&baseline_config);

    let first = temp_config("stable-first", true);
    let first_artifacts = compile(&first);
    let second = temp_config("stable-second", true);
    let second_artifacts = compile(&second);

    assert_eq!(first_artifacts, baseline);
    assert_eq!(second_artifacts, baseline);
    assert_eq!(first_artifacts, second_artifacts);
    assert!(!report_path(&first).exists());
    assert!(!report_path(&second).exists());
}

#[test]
fn stale_legacy_anchor_is_still_cleaned_with_the_supplement_present() {
    let config = temp_config("anchor", true);
    let anchors = config.legacy_runtime_file_path(CIRCUIT_ANCHORS_FILENAME);
    std::fs::create_dir_all(anchors.parent().expect("anchor parent")).expect("anchor dir");
    std::fs::write(&anchors, b"stale").expect("stale anchor");
    compile(&config);
    assert!(!anchors.exists());
}

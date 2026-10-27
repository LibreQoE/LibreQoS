use crate::{
    DynamicCircuit, ParentNodeLookup, ShapedDevicesCatalog, load_shaped_devices_for_config,
    load_source_shaped_devices_for_config, resolve_parent_node_reference,
    resolve_parent_node_reference_in_nodes, runtime_inputs, with_network_json_write,
};
use lqos_config::{
    CircuitAnchorsFile, Config, ConfigShapedDevices, NetworkJsonNode, ShapedDevice,
    TOPOLOGY_RUNTIME_STATUS_FILENAME, TopologyShapingCircuitInput, TopologyShapingDeviceInput,
    TopologyShapingInputsFile, TopologyShapingResolutionSource,
};
use lqos_topology_compile::{ImportedTopologyBundle, TopologyImportFile};
use lqos_utils::rtt::RttBuffer;
use lqos_utils::units::DownUpOrder;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::json;
use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Shared test lock for the crate's process-global state. The dynamic-circuits
/// snapshot is mutated by both this module's tests and `state::tests`; both
/// must take this one lock (two different locks left the snapshot racy).
pub(crate) static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn make_node(
    name: &str,
    id: Option<&str>,
    active_attachment_name: Option<&str>,
) -> NetworkJsonNode {
    NetworkJsonNode {
        name: name.to_string(),
        id: id.map(|value| value.to_string()),
        max_throughput: (0.0, 0.0),
        current_throughput: DownUpOrder::zeroed(),
        current_packets: DownUpOrder::zeroed(),
        current_tcp_packets: DownUpOrder::zeroed(),
        current_udp_packets: DownUpOrder::zeroed(),
        current_icmp_packets: DownUpOrder::zeroed(),
        current_tcp_retransmits: DownUpOrder::zeroed(),
        current_tcp_retransmit_packets: DownUpOrder::zeroed(),
        current_rtt_flows: DownUpOrder::zeroed(),
        current_marks: DownUpOrder::zeroed(),
        current_drops: DownUpOrder::zeroed(),
        rtt_buffer: RttBuffer::default(),
        parents: Vec::new(),
        immediate_parent: None,
        node_type: None,
        latitude: None,
        longitude: None,
        active_attachment_name: active_attachment_name.map(|value| value.to_string()),
        heatmap: None,
    }
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough for tests")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
    std::fs::create_dir_all(&path).expect("temp directory should be creatable");
    path
}

fn write_shaped_devices_csv(path: &std::path::Path, circuit_id: &str, ip: &str) {
    std::fs::write(
        path,
        format!(
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,",
                "Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"{}\",\"Circuit {}\",\"device-{}\",",
                "\"Device {}\",\"Tower 1\",\"aa:bb:cc:dd:ee:ff\",\"{}\",\"\",",
                "\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
            circuit_id, circuit_id, circuit_id, circuit_id, ip,
        ),
    )
    .expect("ShapedDevices.csv should write");
}

fn write_runtime_status(
    path: &std::path::Path,
    ready: bool,
    shaping_inputs_path: &std::path::Path,
    source_generation: &str,
) {
    let effective_network_path = path
        .parent()
        .expect("runtime status path should have a parent")
        .join("network.effective.json");
    std::fs::write(&effective_network_path, "{}\n").expect("effective network should write");
    let shaping_inputs = lqos_config::load_shaping_inputs_at_path(shaping_inputs_path)
        .expect("shaping inputs should load");
    let shaping_generation = shaping_inputs
        .compute_shaping_generation_with_export(None)
        .expect("shaping generation should compute");
    let effective_generation =
        lqos_config::compute_effective_network_file_generation(&effective_network_path)
            .expect("effective generation should compute");

    std::fs::write(
        path,
        serde_json::json!({
            "schema_version": 1,
            "ready": ready,
            "shaping_inputs_path": shaping_inputs_path,
            "effective_state_path": "",
            "operator_view_path": effective_network_path,
            "effective_network_path": effective_network_path,
            "source_generation": source_generation,
            "shaping_generation": shaping_generation,
            "effective_generation": effective_generation,
        })
        .to_string(),
    )
    .expect("status should write");
}

#[test]
fn catalog_device_by_hashes_prefers_device_hash() {
    let _guard = TEST_LOCK.lock();

    let a = ShapedDevice {
        circuit_id: "circuit-a".into(),
        device_id: "device-a".into(),
        ..Default::default()
    };

    let b = ShapedDevice {
        circuit_id: "circuit-b".into(),
        device_id: "device-b".into(),
        ..Default::default()
    };

    let mut shaped = ConfigShapedDevices::default();
    shaped.replace_with_new_data(vec![a.clone(), b.clone()]);

    let catalog = ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped));

    let a_device_hash = lqos_utils::hash_to_i64(&a.device_id);
    let b_circuit_hash = lqos_utils::hash_to_i64(&b.circuit_id);

    let resolved = catalog
        .device_by_hashes(Some(a_device_hash), Some(b_circuit_hash))
        .expect("Expected shaped device match");
    assert_eq!(resolved.device_id, a.device_id);

    let fallback = catalog
        .device_by_hashes(Some(999), Some(b_circuit_hash))
        .expect("Expected circuit-hash fallback match");
    assert_eq!(fallback.device_id, b.device_id);
}

#[test]
fn resolve_parent_node_reference_prefers_id_then_name_then_alias() {
    let _guard = TEST_LOCK.lock();

    with_network_json_write(|net_json| {
        net_json.nodes = vec![
            make_node("Root", Some("root"), None),
            make_node("Site A", Some("node-a"), None),
            make_node("Site B", Some("node-b"), Some("B-alias")),
        ];
    });

    let by_id = resolve_parent_node_reference("ignored", Some("node-a"))
        .expect("Expected node id lookup to resolve");
    assert_eq!(by_id.name, "Site A");

    let by_name = resolve_parent_node_reference("Site B", None)
        .expect("Expected node name lookup to resolve");
    assert_eq!(by_name.id.as_deref(), Some("node-b"));

    let by_alias = resolve_parent_node_reference("B-alias", None)
        .expect("Expected active attachment alias to resolve");
    assert_eq!(by_alias.name, "Site B");
}

#[test]
fn borrowed_parent_node_resolver_uses_the_same_precedence() {
    let nodes = vec![
        make_node("Root", Some("root"), None),
        make_node("Site A", Some("node-a"), Some("shared")),
        make_node("shared", Some("node-b"), None),
    ];

    let by_id = resolve_parent_node_reference_in_nodes(&nodes, "shared", Some("node-a"))
        .expect("node id should take precedence");
    assert_eq!(by_id.name, "Site A");

    let by_name = resolve_parent_node_reference_in_nodes(&nodes, "shared", None)
        .expect("canonical name should take precedence over an alias");
    assert_eq!(by_name.id.as_deref(), Some("node-b"));

    let lookup = ParentNodeLookup::from_nodes(&nodes);
    assert_eq!(lookup.resolve("shared", Some("node-a")), Some(by_id));
    assert_eq!(lookup.resolve("shared", None), Some(by_name));
}

#[test]
fn dynamic_circuit_last_seen_updates_for_seen_hashes() {
    let _guard = TEST_LOCK.lock();

    let original = crate::state::dynamic_circuits_snapshot();

    let shaped = ShapedDevice {
        circuit_id: "dynamic-circuit".into(),
        device_id: "dynamic-device".into(),
        circuit_hash: 10,
        device_hash: 100,
        ..Default::default()
    };

    crate::state::publish_dynamic_circuits_snapshot(vec![DynamicCircuit {
        shaped: shaped.clone(),
        last_seen_unix: 0,
    }]);

    let mut seen_device_hashes: HashSet<i64> = HashSet::new();
    seen_device_hashes.insert(shaped.device_hash);
    let seen_circuit_hashes: HashSet<i64> = HashSet::new();
    let now_unix = 1234;

    let changed = crate::state::refresh_dynamic_circuits_last_seen_for_hashes(
        &seen_device_hashes,
        &seen_circuit_hashes,
        now_unix,
    );
    assert!(changed);

    let updated = crate::state::dynamic_circuits_snapshot();
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].last_seen_unix, now_unix);

    crate::state::publish_dynamic_circuits_snapshot(original.as_ref().clone());
}

#[test]
fn dynamic_circuit_expiration_helper_respects_ttl_boundary() {
    let _guard = TEST_LOCK.lock();

    let now_unix = 1_000;
    let ttl_seconds = 300;

    let alive = ShapedDevice {
        circuit_id: "alive".into(),
        device_id: "device-alive".into(),
        ..Default::default()
    };

    let expired = ShapedDevice {
        circuit_id: "expired".into(),
        device_id: "device-expired".into(),
        ..Default::default()
    };

    let circuits = vec![
        DynamicCircuit {
            shaped: alive,
            // age == ttl => not expired
            last_seen_unix: now_unix - ttl_seconds,
        },
        DynamicCircuit {
            shaped: expired,
            // age > ttl => expired
            last_seen_unix: now_unix - ttl_seconds - 1,
        },
    ];

    let mut ids = crate::dynamic::expired_dynamic_circuit_ids(&circuits, now_unix, ttl_seconds);
    ids.sort();
    assert_eq!(ids, vec!["expired".to_string()]);
}

#[test]
fn dynamic_circuit_expiration_helper_expires_zero_last_seen() {
    let _guard = TEST_LOCK.lock();

    let now_unix = 1_000;
    let ttl_seconds = 300;

    let shaped = ShapedDevice {
        circuit_id: "zero".into(),
        device_id: "device-zero".into(),
        ..Default::default()
    };

    let circuits = vec![DynamicCircuit {
        shaped,
        last_seen_unix: 0,
    }];

    let ids = crate::dynamic::expired_dynamic_circuit_ids(&circuits, now_unix, ttl_seconds);
    assert_eq!(ids, vec!["zero".to_string()]);
}

#[test]
fn dynamic_circuit_is_superseded_when_shaped_devices_now_cover_it() {
    let _guard = TEST_LOCK.lock();

    let mut static_device = ShapedDevice {
        circuit_id: "static".into(),
        device_id: "static-device".into(),
        ..Default::default()
    };
    static_device.ipv4.push((Ipv4Addr::new(192, 0, 2, 42), 32));

    let mut shaped = ConfigShapedDevices::default();
    shaped.replace_with_new_data(vec![static_device]);
    let catalog = ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped));

    let mut dynamic_shaped = ShapedDevice {
        circuit_id: "[dyn] (test) 192.0.2.42".into(),
        device_id: "[dyn] (test) 192.0.2.42".into(),
        circuit_hash: 123,
        device_hash: 456,
        ..Default::default()
    };
    dynamic_shaped.ipv4.push((Ipv4Addr::new(192, 0, 2, 42), 32));

    let dyn_circuit = DynamicCircuit {
        shaped: dynamic_shaped,
        last_seen_unix: 0,
    };

    assert!(crate::dynamic::is_superseded_by_shaped_devices(
        &dyn_circuit,
        &catalog
    ));
}

#[test]
fn runtime_inputs_build_shaped_devices_from_effective_parent_data() {
    let _guard = TEST_LOCK.lock();

    let shaping_inputs = TopologyShapingInputsFile {
        circuits: vec![TopologyShapingCircuitInput {
            circuit_id: "circuit-1".to_string(),
            circuit_name: "Circuit Alpha".to_string(),
            anchor_node_id: Some("anchor-1".to_string()),
            effective_parent_node_name: "Parent-A".to_string(),
            effective_parent_node_id: "parent-id-1".to_string(),
            download_min_mbps: 10.0,
            upload_min_mbps: 5.0,
            download_max_mbps: 50.0,
            upload_max_mbps: 10.0,
            devices: vec![TopologyShapingDeviceInput {
                device_id: "device-1".to_string(),
                device_name: "Device Alpha".to_string(),
                mac: "aa:bb:cc:dd:ee:ff".to_string(),
                ipv4: vec!["192.168.1.10/32".to_string()],
                comment: "device-comment".to_string(),
                ..TopologyShapingDeviceInput::default()
            }],
            comment: "circuit-comment".to_string(),
            ..TopologyShapingCircuitInput::default()
        }],
        ..TopologyShapingInputsFile::default()
    };

    let shaped = runtime_inputs::shaped_devices_from_runtime_inputs(&shaping_inputs);
    assert_eq!(shaped.devices.len(), 1);
    assert_eq!(shaped.devices[0].parent_node, "Parent-A");
    assert_eq!(
        shaped.devices[0].parent_node_id.as_deref(),
        Some("parent-id-1")
    );
    assert_eq!(
        shaped.devices[0].anchor_node_id.as_deref(),
        Some("anchor-1")
    );
    assert_eq!(shaped.devices[0].comment, "device-comment");
    assert_eq!(shaped.devices[0].circuit_id, "circuit-1");
}

#[test]
fn runtime_inputs_leave_runtime_fallback_circuits_unparented() {
    let _guard = TEST_LOCK.lock();

    let shaping_inputs = TopologyShapingInputsFile {
        circuits: vec![TopologyShapingCircuitInput {
            circuit_id: "circuit-1".to_string(),
            circuit_name: "Circuit Alpha".to_string(),
            logical_parent_node_name: Some("LibreQoS Unattached [Site]".to_string()),
            logical_parent_node_id: Some("libreqos:generated:splynx:site:unattached".to_string()),
            resolution_source: TopologyShapingResolutionSource::RuntimeFallback,
            devices: vec![TopologyShapingDeviceInput {
                device_id: "device-1".to_string(),
                device_name: "Device Alpha".to_string(),
                ipv4: vec!["192.168.1.10/32".to_string()],
                ..TopologyShapingDeviceInput::default()
            }],
            ..TopologyShapingCircuitInput::default()
        }],
        ..TopologyShapingInputsFile::default()
    };

    let shaped = runtime_inputs::shaped_devices_from_runtime_inputs(&shaping_inputs);
    assert_eq!(shaped.devices.len(), 1);
    assert_eq!(shaped.devices[0].parent_node, "");
    assert_eq!(shaped.devices[0].parent_node_id, None);
    assert_eq!(shaped.devices[0].circuit_id, "circuit-1");
}

#[test]
fn source_loader_ignores_unverified_runtime_inputs() {
    let _guard = TEST_LOCK.lock();

    let lqos_directory = unique_temp_dir("lqos-network-devices-runtime-empty");
    let state_directory = lqos_directory.join("state");
    std::fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");
    std::fs::create_dir_all(state_directory.join("shaping")).expect("shaping dir should exist");

    let runtime_path = lqos_directory.join("runtime_shaping_inputs.json");
    let status_path = state_directory
        .join("topology")
        .join(TOPOLOGY_RUNTIME_STATUS_FILENAME);
    write_shaped_devices_csv(
        &lqos_directory.join("ShapedDevices.csv"),
        "csv-circuit",
        "192.0.2.10/32",
    );
    let runtime_inputs = TopologyShapingInputsFile {
        circuits: vec![TopologyShapingCircuitInput {
            circuit_id: "runtime-circuit".to_string(),
            circuit_name: "Runtime Circuit".to_string(),
            effective_parent_node_name: "Runtime Parent".to_string(),
            effective_parent_node_id: "runtime-parent".to_string(),
            devices: vec![TopologyShapingDeviceInput {
                device_id: "runtime-device".to_string(),
                ..TopologyShapingDeviceInput::default()
            }],
            ..TopologyShapingCircuitInput::default()
        }],
        shaping_generation: "shape-1".to_string(),
        ..TopologyShapingInputsFile::default()
    };
    std::fs::write(
        &runtime_path,
        serde_json::to_string_pretty(&runtime_inputs).expect("runtime shaping inputs should encode"),
    )
    .expect("runtime shaping inputs should write");
    let mut import_devices = ConfigShapedDevices::default();
    import_devices.replace_with_new_data(vec![ShapedDevice {
        circuit_id: "import-circuit".to_string(),
        circuit_name: "Import Circuit".to_string(),
        device_id: "device-import".to_string(),
        parent_node: "Tower Import".to_string(),
        ipv4: vec![(Ipv4Addr::new(203, 0, 113, 10), 32)],
        ..Default::default()
    }]);
    let imported = ImportedTopologyBundle {
        source: "test/import".to_string(),
        generated_unix: Some(123),
        ingress_identity: Some("import-base".to_string()),
        native_canonical: None,
        parent_candidates: None,
        compatibility_network_json: json!({}),
        shaped_devices: import_devices,
        circuit_anchors: CircuitAnchorsFile::default(),
        ethernet_advisories: Vec::new(),
        circuit_matches: Default::default(),
    };
    TopologyImportFile::from_imported_bundle(&imported, "full")
        .save(&Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: Some(state_directory.to_string_lossy().to_string()),
            ..Config::default()
        })
        .expect("topology import should save");

    let mut config = Config {
        lqos_directory: lqos_directory.to_string_lossy().to_string(),
        state_directory: Some(state_directory.to_string_lossy().to_string()),
        ..Config::default()
    };
    config.uisp_integration.enable_uisp = true;
    let source_generation = lqos_config::compute_topology_source_generation(&config)
        .expect("generation should compute");
    write_runtime_status(&status_path, true, &runtime_path, &source_generation);

    // K1b §4.8 custody: a hand-written runtime status/artifact is NOT verified
    // by the topology custody check, so the preferred source falls back to the
    // integration import. (The verified-runtime path is covered by the
    // lqos_topology verify oracle + the live deploy.)
    let loaded = load_shaped_devices_for_config(&config)
        .expect("preferred shaped-device source should load");
    assert_eq!(loaded.shaped.devices.len(), 1);
    assert_eq!(loaded.shaped.devices[0].circuit_id, "import-circuit");

    let source =
        load_source_shaped_devices_for_config(&config).expect("raw integration source should load");
    assert_eq!(source.devices.len(), 1);
    assert_eq!(source.devices[0].circuit_id, "import-circuit");
}

#[test]
fn load_shaped_devices_uses_topology_import_when_runtime_is_not_ready() {
    let _guard = TEST_LOCK.lock();

    let lqos_directory = unique_temp_dir("lqos-network-devices-topology-import-fallback");
    let state_directory = lqos_directory.join("state");
    std::fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");

    let runtime_path = lqos_directory.join("runtime_shaping_inputs.json");
    let status_path = state_directory
        .join("topology")
        .join(TOPOLOGY_RUNTIME_STATUS_FILENAME);
    std::fs::write(
        &runtime_path,
        serde_json::to_string_pretty(&TopologyShapingInputsFile::default())
            .expect("runtime shaping inputs should encode"),
    )
    .expect("runtime shaping inputs should write");
    write_runtime_status(&status_path, false, &runtime_path, "gen-1");

    let mut import_devices = ConfigShapedDevices::default();
    import_devices.replace_with_new_data(vec![ShapedDevice {
        circuit_id: "import-circuit".to_string(),
        circuit_name: "Import Circuit".to_string(),
        device_id: "device-import".to_string(),
        parent_node: "Tower Import".to_string(),
        ipv4: vec![(Ipv4Addr::new(203, 0, 113, 10), 32)],
        ..Default::default()
    }]);
    let imported = ImportedTopologyBundle {
        source: "test/import".to_string(),
        generated_unix: Some(123),
        ingress_identity: Some("import-base".to_string()),
        native_canonical: None,
        parent_candidates: None,
        compatibility_network_json: json!({}),
        shaped_devices: import_devices,
        circuit_anchors: CircuitAnchorsFile::default(),
        ethernet_advisories: Vec::new(),
        circuit_matches: Default::default(),
    };
    TopologyImportFile::from_imported_bundle(&imported, "full")
        .save(&Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: Some(state_directory.to_string_lossy().to_string()),
            ..Config::default()
        })
        .expect("topology import should save");

    let mut config = Config {
        lqos_directory: lqos_directory.to_string_lossy().to_string(),
        state_directory: Some(state_directory.to_string_lossy().to_string()),
        ..Config::default()
    };
    config.uisp_integration.enable_uisp = true;

    let loaded =
        load_shaped_devices_for_config(&config).expect("topology import fallback should load");
    assert_eq!(loaded.shaped.devices.len(), 1);
    assert_eq!(loaded.shaped.devices[0].circuit_id, "import-circuit");
}

#[test]
fn load_shaped_devices_stays_empty_when_topology_import_is_empty() {
    let _guard = TEST_LOCK.lock();

    let lqos_directory = unique_temp_dir("lqos-network-devices-topology-import-empty");
    let state_directory = lqos_directory.join("state");
    std::fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");

    let imported = ImportedTopologyBundle {
        source: "test/import".to_string(),
        generated_unix: Some(123),
        ingress_identity: Some("import-base".to_string()),
        native_canonical: None,
        parent_candidates: None,
        compatibility_network_json: json!({}),
        shaped_devices: ConfigShapedDevices::default(),
        circuit_anchors: CircuitAnchorsFile::default(),
        ethernet_advisories: Vec::new(),
        circuit_matches: Default::default(),
    };
    TopologyImportFile::from_imported_bundle(&imported, "full")
        .save(&Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: Some(state_directory.to_string_lossy().to_string()),
            ..Config::default()
        })
        .expect("topology import should save");

    let mut config = Config {
        lqos_directory: lqos_directory.to_string_lossy().to_string(),
        state_directory: Some(state_directory.to_string_lossy().to_string()),
        ..Config::default()
    };
    config.uisp_integration.enable_uisp = true;

    let loaded =
        load_shaped_devices_for_config(&config).expect("integration mode should stay empty");
    assert!(loaded.shaped.devices.is_empty());
}

#[test]
fn load_shaped_devices_ignores_stale_runtime_inputs_in_manual_mode() {
    let _guard = TEST_LOCK.lock();

    let lqos_directory = unique_temp_dir("lqos-network-devices-manual-mode");
    let state_directory = lqos_directory.join("state");
    std::fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");
    std::fs::create_dir_all(state_directory.join("shaping")).expect("shaping dir should exist");

    let runtime_path = lqos_directory.join("runtime_shaping_inputs.json");
    let status_path = state_directory
        .join("topology")
        .join(TOPOLOGY_RUNTIME_STATUS_FILENAME);
    std::fs::write(
        &runtime_path,
        serde_json::to_string_pretty(&TopologyShapingInputsFile {
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "runtime-circuit".to_string(),
                devices: vec![TopologyShapingDeviceInput {
                    device_id: "runtime-device".to_string(),
                    ipv4: vec!["198.51.100.10/32".to_string()],
                    ..TopologyShapingDeviceInput::default()
                }],
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        })
        .expect("runtime shaping inputs should encode"),
    )
    .expect("runtime shaping inputs should write");
    write_runtime_status(&status_path, true, &runtime_path, "stale-generation");
    write_shaped_devices_csv(
        &lqos_directory.join("ShapedDevices.csv"),
        "csv-circuit",
        "192.0.2.10/32",
    );

    let config = Config {
        lqos_directory: lqos_directory.to_string_lossy().to_string(),
        state_directory: Some(state_directory.to_string_lossy().to_string()),
        ..Config::default()
    };

    let loaded =
        load_shaped_devices_for_config(&config).expect("manual mode should use shaped devices csv");
    assert_eq!(loaded.shaped.devices.len(), 1);
    assert_eq!(loaded.shaped.devices[0].circuit_id, "csv-circuit");
}

#[test]
fn load_shaped_devices_stays_empty_when_runtime_status_is_malformed() {
    let _guard = TEST_LOCK.lock();

    let lqos_directory = unique_temp_dir("lqos-network-devices-runtime-status-malformed");
    let state_directory = lqos_directory.join("state");
    std::fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");
    std::fs::write(
        state_directory
            .join("topology")
            .join(TOPOLOGY_RUNTIME_STATUS_FILENAME),
        "{not-json\n",
    )
    .expect("malformed runtime status should write");

    let mut config = Config {
        lqos_directory: lqos_directory.to_string_lossy().to_string(),
        state_directory: Some(state_directory.to_string_lossy().to_string()),
        ..Config::default()
    };
    config.uisp_integration.enable_uisp = true;

    let loaded =
        load_shaped_devices_for_config(&config).expect("malformed status should stay empty");
    assert!(loaded.shaped.devices.is_empty());
}

fn range_promoted_row(circuit_id: &str, ip: &str) -> DynamicCircuit {
    DynamicCircuit {
        shaped: ShapedDevice {
            circuit_id: circuit_id.to_string(),
            device_id: circuit_id.to_string(),
            parent_node: "tunnel-circuits:site:site-1".to_string(),
            ipv4: vec![(ip.parse::<Ipv4Addr>().expect("ipv4"), 32)],
            download_min_mbps: 1.0,
            upload_min_mbps: 1.0,
            download_max_mbps: 10.0,
            upload_max_mbps: 10.0,
            ..Default::default()
        },
        last_seen_unix: 100,
    }
}

/// Dynamic-ranges H2: a RADIUS Start (session-derived id, the real plan) for an
/// IP the promotion scanner already promoted must replace the promoted row in
/// place — exactly ONE store row survives, carrying the RADIUS plan. Without
/// this, the store (and therefore the transaction) would hold two /32 Maps for
/// the same subscriber IP.
#[test]
fn replace_by_ip_upgrades_a_promoted_row_in_place() {
    let _guard = TEST_LOCK.lock();

    let promoted = range_promoted_row("dynamic-range:dr-pool:10.60.0.5", "10.60.0.5");
    let snapshot = vec![promoted.clone()];

    let radius = ShapedDevice {
        circuit_id: "radius:nas-1:user:abcd".to_string(),
        device_id: "radius:nas-1:user:abcd".to_string(),
        parent_node: "tunnel-circuits:site:site-1".to_string(),
        ipv4: vec![("10.60.0.5".parse::<Ipv4Addr>().expect("ipv4"), 32)],
        download_min_mbps: 100.0,
        upload_min_mbps: 50.0,
        download_max_mbps: 100.0,
        upload_max_mbps: 50.0,
        ..Default::default()
    };

    let updated =
        crate::actor::merge_dynamic_circuits_replacing_by_ip(&snapshot, radius.clone(), 200);

    assert_eq!(updated.len(), 1, "exactly one row must survive: {updated:?}");
    assert_eq!(updated[0].shaped.circuit_id, "radius:nas-1:user:abcd");
    assert_eq!(
        updated[0].shaped.download_max_mbps, 100.0,
        "the RADIUS plan must win"
    );
    assert_eq!(updated[0].shaped.ipv4.len(), 1);
    assert_eq!(updated[0].last_seen_unix, 200);
}

/// The replace must only drop rows that share an IP; unrelated rows survive.
#[test]
fn replace_by_ip_keeps_unrelated_rows() {
    let _guard = TEST_LOCK.lock();

    let other = range_promoted_row("dynamic-range:dr-pool:10.60.0.9", "10.60.0.9");
    let promoted = range_promoted_row("dynamic-range:dr-pool:10.60.0.5", "10.60.0.5");
    let snapshot = vec![promoted, other];

    let radius = ShapedDevice {
        circuit_id: "radius:nas-1:user:abcd".to_string(),
        device_id: "radius:nas-1:user:abcd".to_string(),
        parent_node: "tunnel-circuits:site:site-1".to_string(),
        ipv4: vec![("10.60.0.5".parse::<Ipv4Addr>().expect("ipv4"), 32)],
        ..Default::default()
    };

    let updated =
        crate::actor::merge_dynamic_circuits_replacing_by_ip(&snapshot, radius, 200);

    assert_eq!(updated.len(), 2, "the unrelated row survives: {updated:?}");
    assert!(
        updated
            .iter()
            .any(|c| c.shaped.circuit_id == "dynamic-range:dr-pool:10.60.0.9"),
        "the 10.60.0.9 row must be untouched"
    );
    assert!(
        updated
            .iter()
            .any(|c| c.shaped.circuit_id == "radius:nas-1:user:abcd"),
        "the RADIUS row must be present"
    );
}

/// A circuit with no IPs must not clobber the store (the empty-IP guard).
#[test]
fn replace_by_ip_with_no_ips_does_not_remove_other_rows() {
    let _guard = TEST_LOCK.lock();

    let promoted = range_promoted_row("dynamic-range:dr-pool:10.60.0.5", "10.60.0.5");
    let snapshot = vec![promoted];

    let no_ip = ShapedDevice {
        circuit_id: "radius:no-ip".to_string(),
        device_id: "radius:no-ip".to_string(),
        parent_node: "tunnel-circuits:site:site-1".to_string(),
        ..Default::default()
    };

    let updated = crate::actor::merge_dynamic_circuits_replacing_by_ip(&snapshot, no_ip, 200);
    assert_eq!(updated.len(), 2, "both rows survive: {updated:?}");
}

#[test]
fn e2_operator_view_catalog_arm_zero_csv_identity() {
    use lqos_config::{OperatorViewDevice, OperatorViewNode, TopologyOperatorViewFile};

    // T5 rung 1 / F10: circuit identity from the E2 operator view
    // ALONE — no ShapedDevices.csv anywhere in this world.
    let view = TopologyOperatorViewFile {
        schema_version: 1,
        source_generation: "t5-zero-csv".to_string(),
        nodes: vec![
            OperatorViewNode {
                id: "uisp:site:42".to_string(),
                name: "UISP Site Downtown".to_string(),
                parent_id: None,
                node_type: "site".to_string(),
                download_mbps: Some(10_000.0),
                upload_mbps: Some(10_000.0),
                commit_download_mbps: None,
                commit_upload_mbps: None,
                devices: Vec::new(),
                latitude: None,
                longitude: None,
                tunnel_identity: None,
                rate_source: lqos_config::OperatorRateSource::Base,
            },
            OperatorViewNode {
                id: "c1".to_string(),
                name: "Bench Client 1".to_string(),
                parent_id: Some("uisp:site:42".to_string()),
                node_type: "circuit".to_string(),
                download_mbps: Some(200.0),
                upload_mbps: Some(100.0),
                commit_download_mbps: Some(50.0),
                commit_upload_mbps: Some(25.0),
                devices: vec![OperatorViewDevice {
                    device_id: "dev1".to_string(),
                    device_name: "Bench Radio".to_string(),
                    ipv4: vec!["100.97.0.1".to_string()],
                    mac: "aa:bb:cc:dd:ee:ff".to_string(),
                    download_min_mbps: 50,
                    upload_min_mbps: 25,
                    download_max_mbps: 200,
                    upload_max_mbps: 100,
                    ipv6: Vec::new(),
                }],
                latitude: None,
                longitude: None,
                tunnel_identity: None,
                rate_source: lqos_config::OperatorRateSource::Base,
            },
        ],
        ambiguous_base_names: Default::default(),
    };

    let shaped = crate::runtime_inputs::shaped_devices_from_operator_view(&view);
    assert_eq!(shaped.devices.len(), 1, "identity from E2 alone");
    let device = &shaped.devices[0];
    assert_eq!(device.circuit_id, "c1");
    assert_eq!(device.circuit_name, "Bench Client 1");
    assert_eq!(device.parent_node, "UISP Site Downtown");
    assert_eq!(device.ipv4.len(), 1);
    assert_eq!(device.download_max_mbps, 200.0);
}

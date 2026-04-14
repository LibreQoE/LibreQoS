use crate::{
    DynamicCircuit, ShapedDevicesCatalog, load_shaped_devices_for_config,
    resolve_parent_node_reference, runtime_inputs, with_network_json_write,
};
use lqos_config::{
    CircuitAnchorsFile, Config, ConfigShapedDevices, NetworkJsonNode, ShapedDevice,
    TOPOLOGY_RUNTIME_STATUS_FILENAME, TopologyShapingCircuitInput, TopologyShapingDeviceInput,
    TopologyShapingInputsFile,
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

static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn make_node(
    name: &str,
    id: Option<&str>,
    active_attachment_name: Option<&str>,
) -> NetworkJsonNode {
    NetworkJsonNode {
        name: name.to_string(),
        id: id.map(|value| value.to_string()),
        virtual_node: false,
        max_throughput: (0.0, 0.0),
        current_throughput: DownUpOrder::zeroed(),
        current_packets: DownUpOrder::zeroed(),
        current_tcp_packets: DownUpOrder::zeroed(),
        current_udp_packets: DownUpOrder::zeroed(),
        current_icmp_packets: DownUpOrder::zeroed(),
        current_tcp_retransmits: DownUpOrder::zeroed(),
        current_tcp_retransmit_packets: DownUpOrder::zeroed(),
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
        qoq_heatmap: None,
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
    std::fs::write(
        path,
        serde_json::json!({
            "schema_version": 1,
            "ready": ready,
            "shaping_inputs_path": shaping_inputs_path,
            "effective_state_path": "",
            "effective_network_path": "",
            "source_generation": source_generation,
            "shaping_generation": "shape-1",
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
fn load_shaped_devices_uses_topology_import_when_runtime_inputs_are_empty() {
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
    std::fs::write(
        &runtime_path,
        serde_json::to_string_pretty(&TopologyShapingInputsFile::default())
            .expect("empty shaping inputs should encode"),
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
        native_editor: None,
        parent_candidates: None,
        compatibility_network_json: json!({}),
        shaped_devices: import_devices,
        circuit_anchors: CircuitAnchorsFile::default(),
        ethernet_advisories: Vec::new(),
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

    let loaded = load_shaped_devices_for_config(&config)
        .expect("preferred shaped-device source should load");
    assert_eq!(loaded.devices.len(), 1);
    assert_eq!(loaded.devices[0].circuit_id, "import-circuit");
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
        native_editor: None,
        parent_candidates: None,
        compatibility_network_json: json!({}),
        shaped_devices: import_devices,
        circuit_anchors: CircuitAnchorsFile::default(),
        ethernet_advisories: Vec::new(),
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
    assert_eq!(loaded.devices.len(), 1);
    assert_eq!(loaded.devices[0].circuit_id, "import-circuit");
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
        native_editor: None,
        parent_candidates: None,
        compatibility_network_json: json!({}),
        shaped_devices: ConfigShapedDevices::default(),
        circuit_anchors: CircuitAnchorsFile::default(),
        ethernet_advisories: Vec::new(),
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
    assert!(loaded.devices.is_empty());
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
    assert_eq!(loaded.devices.len(), 1);
    assert_eq!(loaded.devices[0].circuit_id, "csv-circuit");
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
    assert!(loaded.devices.is_empty());
}

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
use ip_network::IpNetwork;
use lqos_topology_compile::{ImportedTopologyBundle, TopologyImportFile};
use lqos_utils::rtt::RttBuffer;
use lqos_utils::units::DownUpOrder;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Serializes the tests that read or write the process-global dynamic-circuit
/// snapshot or the test environment. `state.rs`'s liveness tests take this same
/// lock, so one lock guards the global they share.
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

    let mut ids = crate::dynamic_engine::expired_dynamic_circuit_ids(&circuits, now_unix, ttl_seconds);
    ids.sort();
    assert_eq!(ids, vec!["expired".to_string()]);
}

/// REV-C1: TTL expiry needs liveness EVIDENCE. A stamp of 0 means "never
/// established" (a clock-not-ready boot, a degraded checkpoint the loader could
/// not restore), so it must never expire — expiring it deletes the whole overlay
/// the moment the clock returns. A genuinely stale row still expires.
#[test]
fn dynamic_circuit_expiration_helper_requires_liveness_evidence() {
    let _guard = TEST_LOCK.lock();

    let now_unix = 1_000;
    let ttl_seconds = 300;

    let unproven = DynamicCircuit {
        shaped: ShapedDevice {
            circuit_id: "zero".into(),
            device_id: "device-zero".into(),
            ..Default::default()
        },
        last_seen_unix: 0,
    };
    let stale = DynamicCircuit {
        shaped: ShapedDevice {
            circuit_id: "stale".into(),
            device_id: "device-stale".into(),
            ..Default::default()
        },
        last_seen_unix: now_unix - ttl_seconds - 1,
    };

    let ids = crate::dynamic_engine::expired_dynamic_circuit_ids(&[unproven, stale], now_unix, ttl_seconds);
    assert_eq!(
        ids,
        vec!["stale".to_string()],
        "an unproven row is never expired; a genuinely stale one still is"
    );
}

/// A shaped row covering one IPv4 prefix, with the derived hashes the catalog
/// keys on.
fn shaped_row(circuit_id: &str, device_id: &str, ip: Ipv4Addr, prefix: u32) -> ShapedDevice {
    let mut device = ShapedDevice {
        circuit_id: circuit_id.to_string(),
        device_id: device_id.to_string(),
        ..Default::default()
    };
    device.ipv4.push((ip, prefix));
    device
}

/// The provisioned half of a world made of `devices`, where `origin_rows` are
/// the `(circuit_id, device_id)` rows the fold synthesized.
fn provisioned_rows(
    devices: Vec<ShapedDevice>,
    origin_rows: &[(&str, &str)],
) -> crate::dynamic::ProvisionedShapedRows {
    let mut shaped = ConfigShapedDevices::default();
    shaped.replace_with_new_data(devices);
    let catalog = ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped));
    let mut origins = lqos_utils::DynamicOriginRows::new();
    for (circuit_id, device_id) in origin_rows {
        origins.insert_row(circuit_id, device_id);
    }
    crate::dynamic::ProvisionedShapedRows::new(&catalog, &Arc::new(origins))
}

/// The store row a RADIUS Start leaves behind: the fold recomputes its hashes
/// from the ids, and the catalog carries the resulting row.
fn store_row(circuit_id: &str, device_id: &str, ip: Ipv4Addr, prefix: u32) -> DynamicCircuit {
    let mut shaped = shaped_row(circuit_id, device_id, ip, prefix);
    shaped.circuit_hash = lqos_utils::hash_to_i64(circuit_id);
    shaped.device_hash = lqos_utils::hash_to_i64(device_id);
    DynamicCircuit {
        shaped,
        last_seen_unix: 0,
    }
}

#[test]
fn dynamic_circuit_is_superseded_when_shaped_devices_now_cover_it() {
    let _guard = TEST_LOCK.lock();

    let provisioned = provisioned_rows(
        vec![shaped_row("static", "static-device", Ipv4Addr::new(192, 0, 2, 42), 32)],
        &[],
    );
    let dyn_circuit = store_row(
        "[dyn] (test) 192.0.2.42",
        "[dyn] (test) 192.0.2.42",
        Ipv4Addr::new(192, 0, 2, 42),
        32,
    );

    assert!(provisioned.is_superseded_by(&dyn_circuit));
}

/// u3 regression gate: the fold publishes the dynamic circuit as an ordinary
/// shaped row, so the actor's own reload must not read that row back as a
/// provisioned device. Without the origin marker this circuit is pruned ~300 ms
/// after its first publish (the u2 finding) and RADIUS interims recreate it —
/// the churn loop this unit removes.
#[test]
fn published_dynamic_circuit_is_not_superseded_by_its_own_fold() {
    let _guard = TEST_LOCK.lock();

    let static_device = shaped_row("static", "static-device", Ipv4Addr::new(203, 0, 113, 10), 32);
    let folded_dynamic = shaped_row(
        "dyn-sub-1",
        "dyn-dev-1",
        Ipv4Addr::new(100, 64, 0, 5),
        32,
    );
    let provisioned = provisioned_rows(
        vec![static_device, folded_dynamic],
        &[("dyn-sub-1", "dyn-dev-1")],
    );
    let dyn_circuit = store_row(
        "dyn-sub-1",
        "dyn-dev-1",
        Ipv4Addr::new(100, 64, 0, 5),
        32,
    );

    assert!(
        !provisioned.is_superseded_by(&dyn_circuit),
        "a published dynamic circuit must survive its own fold"
    );
}

/// A provisioned prefix BROADER than the dynamic circuit's `/32` still counts as
/// coverage: the circuit shadows a real device by longest-prefix match but does
/// not replace it.
#[test]
fn broader_provisioned_prefix_supersedes_a_shadowing_dynamic_circuit() {
    let _guard = TEST_LOCK.lock();

    let static_subnet = shaped_row("static-site", "static-site-dev", Ipv4Addr::new(100, 64, 0, 0), 24);
    let folded_dynamic = shaped_row(
        "dyn-sub-1",
        "dyn-dev-1",
        Ipv4Addr::new(100, 64, 0, 5),
        32,
    );
    let provisioned = provisioned_rows(
        vec![static_subnet, folded_dynamic],
        &[("dyn-sub-1", "dyn-dev-1")],
    );
    let dyn_circuit = store_row(
        "dyn-sub-1",
        "dyn-dev-1",
        Ipv4Addr::new(100, 64, 0, 5),
        32,
    );

    assert!(provisioned.is_superseded_by(&dyn_circuit));
}

/// u5 A1 (the u3 interaction): the fold's whole-prefix range FALLBACK is a
/// published row like any other, so without the dynamic-origin marker it would
/// read as provisioning and supersede every promoted circuit inside its rule —
/// promotion would delete itself one publish later. With the marker the fallback
/// is not ownership, and the promoted `/32` survives.
#[test]
fn a_range_fallback_never_supersedes_a_promoted_circuit_inside_it() {
    let _guard = TEST_LOCK.lock();

    let catch_all_fallback = shaped_row(
        "dynamic-range:Default",
        "dynamic-range:Default",
        Ipv4Addr::UNSPECIFIED,
        0,
    );
    let promoted = shaped_row(
        "dynamic-range:Default:10.92.0.7",
        "dynamic-range:Default:10.92.0.7",
        Ipv4Addr::new(10, 92, 0, 7),
        32,
    );
    let promoted_store_row = store_row(
        "dynamic-range:Default:10.92.0.7",
        "dynamic-range:Default:10.92.0.7",
        Ipv4Addr::new(10, 92, 0, 7),
        32,
    );

    let marked = provisioned_rows(
        vec![catch_all_fallback.clone(), promoted.clone()],
        &[
            ("dynamic-range:Default", "dynamic-range:Default"),
            (
                "dynamic-range:Default:10.92.0.7",
                "dynamic-range:Default:10.92.0.7",
            ),
        ],
    );
    assert!(
        !marked.is_superseded_by(&promoted_store_row),
        "a synthesized fallback is not provisioned coverage"
    );

    // CONTROL RUN: the same world with the origin marker removed — the gate must
    // fail, which is what proves the marker is load-bearing rather than incidental.
    let unmarked = provisioned_rows(vec![catch_all_fallback, promoted], &[]);
    assert!(
        unmarked.is_superseded_by(&promoted_store_row),
        "control: without the marker the fallback reads as a provisioned /0"
    );
}

/// Another dynamic circuit's prefix is not provisioning: one RADIUS overlay row
/// must never evict another.
#[test]
fn another_dynamic_circuits_row_does_not_supersede() {
    let _guard = TEST_LOCK.lock();

    let other_dynamic = shaped_row("dyn-pool", "dyn-pool-dev", Ipv4Addr::new(100, 64, 0, 0), 24);
    let folded_dynamic = shaped_row(
        "dyn-sub-1",
        "dyn-dev-1",
        Ipv4Addr::new(100, 64, 0, 5),
        32,
    );
    let provisioned = provisioned_rows(
        vec![other_dynamic, folded_dynamic],
        &[("dyn-pool", "dyn-pool-dev"), ("dyn-sub-1", "dyn-dev-1")],
    );
    let dyn_circuit = store_row(
        "dyn-sub-1",
        "dyn-dev-1",
        Ipv4Addr::new(100, 64, 0, 5),
        32,
    );

    assert!(
        !provisioned.is_superseded_by(&dyn_circuit),
        "a dynamic row is never a provisioned device"
    );
}

/// REV-C1 regression gate: a PROVISIONED row on the circuit's exact `/32`
/// supersedes it even while the circuit's own fold row occupies the same prefix
/// in the catalog.
///
/// The provisioned prefix set used to come from the catalog's trie iteration,
/// which holds one entry per PREFIX: the fold row (appended after the import
/// rows) took the slot, the identical provisioned prefix never entered the set,
/// and the live circuit survived a real static device on its own address.
#[test]
fn a_provisioned_row_on_the_identical_prefix_supersedes_the_live_circuit() {
    let _guard = TEST_LOCK.lock();

    let circuit_ip = Ipv4Addr::new(100, 64, 0, 5);
    let folded_dynamic = shaped_row("dyn-sub-1", "dyn-dev-1", circuit_ip, 32);
    let provisioned_same_prefix = shaped_row("static-sub", "static-sub-dev", circuit_ip, 32);
    // The real order: the fold APPENDS its rows after the import rows, so on an
    // identical prefix the trie holds the fold row, not the provisioned one.
    let provisioned = provisioned_rows(
        vec![provisioned_same_prefix, folded_dynamic],
        &[("dyn-sub-1", "dyn-dev-1")],
    );

    assert!(
        provisioned.is_superseded_by(&store_row("dyn-sub-1", "dyn-dev-1", circuit_ip, 32)),
        "a real device on the subscriber's exact /32 must retire the dynamic circuit"
    );
}

/// REV-D1 regression gate (the network-devices half): the APPEND override shape.
///
/// A persistent override with the circuit's `circuit_id` but a different
/// `device_id` APPENDS a second row; the fold's own row stays dynamic-origin.
/// The circuit must not be retired by its own address, and the appended row's
/// address must not cover it either.
#[test]
fn an_appended_override_row_never_retires_the_fold_row_by_its_own_address() {
    let _guard = TEST_LOCK.lock();

    let circuit_ip = Ipv4Addr::new(100, 64, 0, 5);
    let folded_dynamic = shaped_row("dyn-1", "dyn-1-dev", circuit_ip, 32);
    let appended_override =
        shaped_row("dyn-1", "dyn-1-extra", Ipv4Addr::new(100, 64, 0, 6), 32);
    let provisioned = provisioned_rows(
        vec![folded_dynamic, appended_override],
        &[("dyn-1", "dyn-1-dev")],
    );

    assert!(
        !provisioned.is_superseded_by(&store_row("dyn-1", "dyn-1-dev", circuit_ip, 32)),
        "the fold row's own address is not provisioning"
    );
}

/// REV-T2: the own-fold predicate normalizes the circuit id at the case
/// boundary, so a mixed-case catalog row still matches its normalized origin.
#[test]
fn a_mixed_case_circuit_id_still_matches_its_origin_row() {
    let _guard = TEST_LOCK.lock();

    let circuit_ip = Ipv4Addr::new(100, 64, 0, 5);
    let folded_dynamic = shaped_row("DYN-Sub-1", "dyn-dev-1", circuit_ip, 32);
    let provisioned = provisioned_rows(vec![folded_dynamic], &[("dyn-sub-1", "dyn-dev-1")]);

    assert!(
        !provisioned.is_superseded_by(&store_row("dyn-sub-1", "dyn-dev-1", circuit_ip, 32)),
        "a case-different catalog row is still the fold's own row"
    );
}

/// REV-T3: the IPv6 arm of the own-fold predicate. A `/128` dynamic circuit must
/// survive its own folded row.
#[test]
fn an_ipv6_dynamic_circuit_survives_its_own_fold() {
    let _guard = TEST_LOCK.lock();

    let ip: std::net::Ipv6Addr = "2001:db8::5".parse().expect("ipv6");
    let mut folded_dynamic = shaped_row("dyn-v6", "dyn-v6-dev", Ipv4Addr::UNSPECIFIED, 0);
    folded_dynamic.ipv4.clear();
    folded_dynamic.ipv6.push((ip, 128));
    let provisioned = provisioned_rows(vec![folded_dynamic], &[("dyn-v6", "dyn-v6-dev")]);

    let mut store = shaped_row("dyn-v6", "dyn-v6-dev", Ipv4Addr::UNSPECIFIED, 0);
    store.ipv4.clear();
    store.ipv6.push((ip, 128));
    let circuit = DynamicCircuit {
        shaped: store,
        last_seen_unix: 0,
    };
    assert!(
        !provisioned.is_superseded_by(&circuit),
        "an IPv6 circuit must survive its own folded /128"
    );
}

/// REV-T4: the identity arm alone, isolated from coverage. The provisioned row
/// carries the circuit's hashes but its address does NOT cover the circuit's, so
/// only the identity arm can retire it.
#[test]
fn a_reprovisioned_identity_supersedes_even_when_its_addresses_moved() {
    let _guard = TEST_LOCK.lock();

    let moved_device = shaped_row(
        "dyn-sub-1",
        "dyn-dev-1",
        Ipv4Addr::new(203, 0, 113, 99),
        32,
    );
    let provisioned = provisioned_rows(vec![moved_device], &[]);

    assert!(
        provisioned.is_superseded_by(&store_row(
            "dyn-sub-1",
            "dyn-dev-1",
            Ipv4Addr::new(100, 64, 0, 5),
            32,
        )),
        "a re-provisioned device keeps its identity even when its addresses move"
    );
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

/// The origin marker survives the fold → artifact → catalog leg, keyed PER ROW:
/// a circuit holding both a fold-marked row and a row the operator provisioned
/// under the same `circuit_id` reports only the fold row as dynamic-origin, and
/// a static-only artifact reports none.
#[test]
fn runtime_inputs_report_only_marked_rows_as_dynamic_origin() {
    let _guard = TEST_LOCK.lock();

    let device = |device_id: &str, dynamic: bool| TopologyShapingDeviceInput {
        device_id: device_id.to_string(),
        device_name: device_id.to_string(),
        ipv4: vec!["100.64.0.5/32".to_string()],
        dynamic,
        ..TopologyShapingDeviceInput::default()
    };
    let shaping_inputs = TopologyShapingInputsFile {
        circuits: vec![
            TopologyShapingCircuitInput {
                circuit_id: "Static-Circuit".to_string(),
                circuit_name: "Static Circuit".to_string(),
                devices: vec![device("static-dev", false)],
                ..TopologyShapingCircuitInput::default()
            },
            TopologyShapingCircuitInput {
                circuit_id: "DYN-SUB-1".to_string(),
                circuit_name: "Dynamic Sub 1".to_string(),
                devices: vec![device("dyn-dev-1", true), device("dyn-dev-extra", false)],
                ..TopologyShapingCircuitInput::default()
            },
        ],
        ..TopologyShapingInputsFile::default()
    };

    let origins = runtime_inputs::dynamic_origin_rows(&shaping_inputs);
    assert_eq!(origins.len(), 1, "one row is marked: {origins:?}");
    assert!(
        origins.contains_row("dyn-sub-1", "dyn-dev-1"),
        "the marked row is keyed by normalized circuit id plus its own device id"
    );
    assert!(
        !origins.contains_row("dyn-sub-1", "dyn-dev-extra"),
        "a provisioned row under the same circuit id is not dynamic-origin"
    );
    assert!(!origins.contains_row("Static-Circuit", "static-dev"));

    let static_only = TopologyShapingInputsFile {
        circuits: vec![TopologyShapingCircuitInput {
            circuit_id: "Static-Circuit".to_string(),
            circuit_name: "Static Circuit".to_string(),
            devices: vec![device("static-dev", false)],
            ..TopologyShapingCircuitInput::default()
        }],
        ..TopologyShapingInputsFile::default()
    };
    assert!(
        runtime_inputs::dynamic_origin_rows(&static_only).is_empty(),
        "a static-only world has no dynamic-origin rows"
    );
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
/// IP the unshaped-host promotion already promoted must replace the promoted row in
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

/// `(inode, mtime, length)` — the rewrite probe for the two dynamic-circuit
/// files. Both writers go through temp-file + rename, which moves all three; an
/// in-place rewrite would at least move the mtime.
fn file_fingerprint(path: &std::path::Path) -> (u64, SystemTime, u64) {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).expect("file metadata");
    (
        metadata.ino(),
        metadata.modified().expect("modification time"),
        metadata.len(),
    )
}

/// One dynamic-circuit row for the store/checkpoint tests.
fn dynamic_row(circuit_id: &str, ipv4: Ipv4Addr) -> DynamicCircuit {
    DynamicCircuit {
        shaped: ShapedDevice {
            circuit_id: circuit_id.to_string(),
            circuit_name: circuit_id.to_string(),
            device_id: format!("{circuit_id}-device"),
            device_name: circuit_id.to_string(),
            parent_node: "tower-1".to_string(),
            ipv4: vec![(ipv4, 32)],
            ..Default::default()
        },
        last_seen_unix: 0,
    }
}

/// Writes the membership store (and only it) for the loader tests.
fn write_store(path: &std::path::Path, circuits: Vec<DynamicCircuit>) {
    let file = lqos_config::DynamicCircuitsFile {
        schema_version: 1,
        circuits,
    };
    std::fs::write(
        path,
        serde_json::to_string_pretty(&file).expect("store encodes"),
    )
    .expect("store writes");
}

/// Reads the liveness checkpoint document.
fn read_checkpoint(path: &std::path::Path) -> std::collections::BTreeMap<String, u64> {
    serde_json::from_str(&std::fs::read_to_string(path).expect("checkpoint exists"))
        .expect("checkpoint parses")
}

/// Hermetic `lqos_directory` for store/generation tests. Returns the config plus
/// the store and liveness-checkpoint paths it resolves to.
fn isolated_dynamic_store_config(prefix: &str) -> (Config, PathBuf, PathBuf) {
    let lqos_directory = unique_temp_dir(prefix);
    let state_directory = lqos_directory.join("state");
    std::fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");
    let config = Config {
        lqos_directory: lqos_directory.to_string_lossy().to_string(),
        state_directory: Some(state_directory.to_string_lossy().to_string()),
        ..Config::default()
    };
    let store_path = lqos_config::dynamic_circuits_path(&config);
    let checkpoint_path = lqos_config::dynamic_circuits_state_path(&config);
    (config, store_path, checkpoint_path)
}

fn radius_plan_for_test(ip: &str, download_max_mbps: f32) -> ShapedDevice {
    ShapedDevice {
        circuit_id: "radius:nas-1:user:abcd".to_string(),
        circuit_name: "RADIUS subscriber".to_string(),
        device_id: "radius:nas-1:user:abcd".to_string(),
        device_name: "RADIUS subscriber".to_string(),
        parent_node: "tunnel-circuits:site:site-1".to_string(),
        ipv4: vec![(ip.parse::<Ipv4Addr>().expect("ipv4"), 32)],
        download_min_mbps: 10.0,
        upload_min_mbps: 5.0,
        download_max_mbps,
        upload_max_mbps: 50.0,
        ..Default::default()
    }
}

/// u1-churn A4: the generation-stability gate.
///
/// RADIUS reaches the store through `replace_dynamic_circuit_by_ip` (the bus
/// command `lqosd::dynamic_circuits::create_dynamic_circuit` sends), so this
/// drives the same RMW. A Start/Interim that only renews liveness must leave
/// `compute_topology_source_generation` byte-identical AND leave the store file
/// byte-identical. A real membership/rate change must move the generation.
#[test]
fn radius_liveness_only_update_leaves_generation_and_store_bytes_identical() {
    use std::os::unix::fs::MetadataExt;

    let _guard = TEST_LOCK.lock();
    crate::state::publish_dynamic_circuits_snapshot(Vec::new());

    let (config, store_path, checkpoint_path) =
        isolated_dynamic_store_config("lqos-network-devices-churn-gate");
    let source_generation = |config: &Config| {
        lqos_config::compute_topology_source_generation(config)
            .expect("source generation should compute")
    };

    let generation_before_start = source_generation(&config);
    let plan = radius_plan_for_test("10.70.0.7", 100.0f32);

    // RADIUS Start: the membership appears, and with it the liveness checkpoint.
    crate::actor::replace_dynamic_circuit_by_ip_inner(&config, plan.clone())
        .expect("Start must reach the store");
    let store_after_start = std::fs::read(&store_path).expect("the store must exist after a Start");
    let store_text = String::from_utf8(store_after_start.clone()).expect("store is UTF-8");
    assert!(
        !store_text.contains("last_seen_unix"),
        "liveness must never be serialized into the hashed store: {store_text}"
    );
    let generation_after_start = source_generation(&config);
    assert_ne!(
        generation_after_start, generation_before_start,
        "a membership change must move the topology source generation"
    );
    let store_fingerprint_after_start = file_fingerprint(&store_path);
    let inode_after_start = std::fs::metadata(&store_path)
        .expect("store metadata")
        .ino();
    // REV-T1: the membership write is what checkpoints liveness, so this is the
    // last legitimate checkpoint content before the liveness-only Interim.
    let checkpoint_after_start = std::fs::read(&checkpoint_path)
        .expect("the Start must checkpoint the new membership");
    let checkpoint_fingerprint_after_start = file_fingerprint(&checkpoint_path);

    // REV-T3: make the premise deterministic. The wall clock is
    // second-resolution, so Start and Interim can carry the same stamp; age the
    // in-memory stamp first so the Interim's renewal MUST advance it.
    let aged_stamp = lqos_utils::unix_time::unix_now().expect("clock").saturating_sub(600);
    let aged: Vec<DynamicCircuit> = crate::state::dynamic_circuits_snapshot()
        .as_ref()
        .iter()
        .cloned()
        .map(|mut circuit| {
            circuit.last_seen_unix = aged_stamp;
            circuit
        })
        .collect();
    assert_eq!(aged.len(), 1, "the Start must leave exactly one row");
    crate::state::publish_dynamic_circuits_snapshot(aged);

    // RADIUS Interim: same plan, liveness only.
    crate::actor::replace_dynamic_circuit_by_ip_inner(&config, plan)
        .expect("Interim must reach the store");
    let after_interim = crate::state::dynamic_circuits_snapshot();
    assert_eq!(after_interim.len(), 1, "the Interim must keep the membership");
    assert!(
        after_interim[0].last_seen_unix > aged_stamp,
        "PREMISE: the Interim must renew liveness ({} must exceed {aged_stamp})",
        after_interim[0].last_seen_unix
    );
    assert_eq!(
        source_generation(&config),
        generation_after_start,
        "a liveness-only update must not move the topology source generation"
    );
    assert_eq!(
        std::fs::read(&store_path).expect("store must still exist"),
        store_after_start,
        "a liveness-only update must leave the store bytes identical"
    );
    assert_eq!(
        std::fs::metadata(&store_path)
            .expect("store metadata")
            .ino(),
        inode_after_start,
        "a liveness-only update must not rewrite the store at all (atomic rename would change the inode)"
    );
    assert_eq!(
        file_fingerprint(&store_path),
        store_fingerprint_after_start,
        "a liveness-only update must leave the store inode/mtime/length untouched"
    );
    // REV-T1: ... and it must not checkpoint either. Checkpointing per interim
    // is exactly what the bounded-interval rule forbids.
    assert_eq!(
        std::fs::read(&checkpoint_path).expect("checkpoint must still exist"),
        checkpoint_after_start,
        "a liveness-only update must leave the checkpoint bytes identical"
    );
    assert_eq!(
        file_fingerprint(&checkpoint_path),
        checkpoint_fingerprint_after_start,
        "a liveness-only update must not rewrite the checkpoint at all"
    );

    // REV-T1: the inverse property. An EXPLICIT checkpoint write must move the
    // checkpoint and must NOT move the generation — liveness lives outside every
    // hashed input.
    let generation_before_checkpoint = source_generation(&config);
    let mut bumped = after_interim.as_ref().clone();
    bumped[0].last_seen_unix += 1;
    crate::dynamic_store::persist_liveness_checkpoint(&config, &bumped)
        .expect("an explicit checkpoint write must succeed");
    assert_ne!(
        std::fs::read(&checkpoint_path).expect("checkpoint must exist"),
        checkpoint_after_start,
        "an explicit checkpoint write must change the checkpoint bytes"
    );
    assert_ne!(
        file_fingerprint(&checkpoint_path),
        checkpoint_fingerprint_after_start,
        "an explicit checkpoint write must rewrite the checkpoint file"
    );
    assert_eq!(
        source_generation(&config),
        generation_before_checkpoint,
        "checkpointed liveness must stay outside the hashed topology inputs"
    );
    crate::state::publish_dynamic_circuits_snapshot(bumped);

    // A plan upgrade is a real membership/rate change: it must move both.
    crate::actor::replace_dynamic_circuit_by_ip_inner(
        &config,
        radius_plan_for_test("10.70.0.7", 250.0f32),
    )
    .expect("plan upgrade must reach the store");
    assert_ne!(
        source_generation(&config),
        generation_after_start,
        "a rates change must move the topology source generation"
    );
    assert_ne!(
        std::fs::read(&store_path).expect("store must exist"),
        store_after_start,
        "a rates change must rewrite the store"
    );

    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
}

/// u1-churn A1/A2: a legacy store file that still carries `last_seen_unix`
/// imports it into the checkpoint once and is rewritten without the field; a
/// restart then restores liveness from the checkpoint and touches no file.
#[test]
fn legacy_last_seen_unix_migrates_into_the_checkpoint_once() {
    let _guard = TEST_LOCK.lock();

    let (config, store_path, checkpoint_path) =
        isolated_dynamic_store_config("lqos-network-devices-legacy-migration");

    let legacy = lqos_config::DynamicCircuitsFile {
        schema_version: 1,
        circuits: vec![DynamicCircuit {
            shaped: ShapedDevice {
                circuit_id: "Legacy Circuit".to_string(),
                device_id: "legacy-device".to_string(),
                parent_node: "tower-1".to_string(),
                ipv4: vec![(Ipv4Addr::new(10, 71, 0, 9), 32)],
                ..Default::default()
            },
            // In-memory only now; the legacy writer serialized it.
            last_seen_unix: 4_242,
        }],
    };
    let mut legacy_json =
        serde_json::to_value(&legacy).expect("legacy store should serialize as a value");
    legacy_json["circuits"][0]["last_seen_unix"] = json!(4_242);
    std::fs::write(
        &store_path,
        serde_json::to_string_pretty(&legacy_json).expect("legacy store should encode"),
    )
    .expect("legacy store should write");

    let loaded = crate::dynamic_store::load_dynamic_circuits_from_disk(&config)
        .expect("the dynamic store loads");
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].last_seen_unix, 4_242,
        "the legacy liveness value must survive the migration"
    );

    let store_text = std::fs::read_to_string(&store_path).expect("store must exist");
    assert!(
        !store_text.contains("last_seen_unix"),
        "the migrated store must not carry liveness: {store_text}"
    );
    let checkpoint = read_checkpoint(&checkpoint_path);
    assert_eq!(
        checkpoint.get("legacy circuit"),
        Some(&4_242),
        "the checkpoint is keyed by the normalized circuit id: {checkpoint:?}"
    );

    // Restart: the checkpoint restores liveness and neither file is rewritten.
    let store_bytes = std::fs::read(&store_path).expect("store must exist");
    let checkpoint_bytes = std::fs::read(&checkpoint_path).expect("checkpoint must exist");
    let reloaded = crate::dynamic_store::load_dynamic_circuits_from_disk(&config)
        .expect("the dynamic store loads");
    assert_eq!(reloaded[0].last_seen_unix, 4_242);
    assert_eq!(
        std::fs::read(&store_path).expect("store must exist"),
        store_bytes
    );
    assert_eq!(
        std::fs::read(&checkpoint_path).expect("checkpoint must exist"),
        checkpoint_bytes
    );
}

/// u1-churn A2: a lost/late checkpoint must not mass-expire the store on the
/// next maintenance tick — restored rows get one bounded startup grace and the
/// grace is checkpointed immediately.
#[test]
fn missing_checkpoint_grants_one_startup_grace() {
    let _guard = TEST_LOCK.lock();

    let (config, store_path, checkpoint_path) =
        isolated_dynamic_store_config("lqos-network-devices-checkpoint-grace");

    let store = lqos_config::DynamicCircuitsFile {
        schema_version: 1,
        circuits: vec![DynamicCircuit {
            shaped: ShapedDevice {
                circuit_id: "grace-circuit".to_string(),
                device_id: "grace-device".to_string(),
                ..Default::default()
            },
            last_seen_unix: 0,
        }],
    };
    std::fs::write(
        &store_path,
        serde_json::to_string_pretty(&store).expect("store should encode"),
    )
    .expect("store should write");

    let now_unix = lqos_utils::unix_time::unix_now().expect("clock");
    let loaded = crate::dynamic_store::load_dynamic_circuits_from_disk(&config)
        .expect("the dynamic store loads");
    assert_eq!(loaded.len(), 1);
    assert!(
        loaded[0].last_seen_unix >= now_unix,
        "a restored row without checkpoint evidence gets the startup grace, got {}",
        loaded[0].last_seen_unix
    );

    let checkpoint = read_checkpoint(&checkpoint_path);
    assert_eq!(
        checkpoint.get("grace-circuit"),
        Some(&loaded[0].last_seen_unix),
        "the grace value must be durable so the next start is not regranted"
    );
}

/// REV-C1(c): a parseable-but-EMPTY checkpoint carries no evidence. A degraded
/// path can write `{}` over a valid checkpoint; treating that as authoritative
/// would leave every row at `last_seen = 0` and the first maintenance tick would
/// delete the whole overlay.
#[test]
fn empty_checkpoint_carries_no_evidence_and_grants_the_startup_grace() {
    let _guard = TEST_LOCK.lock();

    let (config, store_path, checkpoint_path) =
        isolated_dynamic_store_config("lqos-network-devices-empty-checkpoint");
    write_store(
        &store_path,
        vec![
            dynamic_row("kept-a", Ipv4Addr::new(10, 72, 0, 1)),
            dynamic_row("kept-b", Ipv4Addr::new(10, 72, 0, 2)),
        ],
    );
    std::fs::write(&checkpoint_path, "{}").expect("empty checkpoint writes");

    let now_unix = lqos_utils::unix_time::unix_now().expect("clock");
    let loaded = crate::dynamic_store::load_dynamic_circuits_from_disk(&config)
        .expect("the dynamic store loads");
    assert_eq!(loaded.len(), 2);
    for circuit in &loaded {
        assert!(
            circuit.last_seen_unix >= now_unix,
            "an empty checkpoint must grant the grace to '{}', got {}",
            circuit.shaped.circuit_id,
            circuit.last_seen_unix
        );
    }
    assert!(
        crate::dynamic_engine::expired_dynamic_circuit_ids(&loaded, now_unix, 300).is_empty(),
        "an emptied checkpoint must not mass-expire the overlay on the next tick"
    );
    let checkpoint = read_checkpoint(&checkpoint_path);
    assert_eq!(
        checkpoint.len(),
        2,
        "the grace must be checkpointed for both rows: {checkpoint:?}"
    );
}

/// REV-C1(a): the maintenance tick checkpoints only a configured, non-empty
/// membership — never `{}` from an empty snapshot, never a stray file on
/// installs that do not run dynamic circuits.
#[test]
fn the_tick_checkpoints_only_a_configured_nonempty_membership() {
    let row = dynamic_row("configured", Ipv4Addr::new(10, 73, 0, 1));
    let mut config = Config {
        dynamic_circuits: None,
        ..Config::default()
    };
    assert!(
        !crate::dynamic_store::should_checkpoint_liveness(&config, std::slice::from_ref(&row)),
        "an install without a [dynamic_circuits] section must not grow a checkpoint"
    );
    config.dynamic_circuits = Some(lqos_config::DynamicCircuitsConfig {
        enabled: false,
        ..Default::default()
    });
    assert!(
        !crate::dynamic_store::should_checkpoint_liveness(&config, std::slice::from_ref(&row)),
        "a disabled feature must not checkpoint"
    );
    config.dynamic_circuits = Some(lqos_config::DynamicCircuitsConfig {
        enabled: true,
        ..Default::default()
    });
    assert!(
        !crate::dynamic_store::should_checkpoint_liveness(&config, &[]),
        "an empty snapshot is not evidence — a failed startup load must not overwrite the checkpoint"
    );
    assert!(
        crate::dynamic_store::should_checkpoint_liveness(&config, std::slice::from_ref(&row)),
        "a configured feature with a live membership checkpoints"
    );
}


/// REV-C2: a DEAD-CLOCK tick must not overwrite a valid liveness checkpoint.
///
/// The loader's clock guard (REV-C1(b)) leaves every row at stamp 0 when the
/// clock never became readable, and the maintenance tick still runs. Persisting
/// that snapshot used to write `{id: 0}` over real stamps; the map is then
/// non-empty but every value is filtered by the `> 0` guard, so no grace is
/// granted and the overlay is silently TTL-exempt with its evidence destroyed.
#[test]
fn a_zero_stamp_membership_never_overwrites_a_good_checkpoint() {
    let _guard = TEST_LOCK.lock();
    crate::state::publish_dynamic_circuits_snapshot(Vec::new());

    let (config, store_path, checkpoint_path) =
        isolated_dynamic_store_config("lqos-network-devices-zero-stamp-checkpoint");
    let zeroed = dynamic_row("zeroed", Ipv4Addr::new(10, 81, 0, 1));
    let stamped = dynamic_row("stamped", Ipv4Addr::new(10, 81, 0, 2));
    write_store(&store_path, vec![zeroed.clone(), stamped.clone()]);

    // A healthy boot's checkpoint.
    std::fs::write(&checkpoint_path, r#"{"zeroed": 1234, "stamped": 1234}"#)
        .expect("checkpoint writes");
    let good_bytes = std::fs::read(&checkpoint_path).expect("checkpoint reads");
    let good_fingerprint = file_fingerprint(&checkpoint_path);

    // The dead-clock tick shape: a non-empty membership whose stamps are all 0.
    assert!(
        !crate::dynamic_store::persist_liveness_checkpoint(
            &config,
            &[zeroed.clone(), stamped.clone()]
        )
        .expect("a refused write is not a failure"),
        "a membership with no stamp at all must write nothing"
    );
    assert_eq!(
        std::fs::read(&checkpoint_path).expect("checkpoint reads"),
        good_bytes,
        "the good checkpoint must be untouched"
    );
    assert_eq!(
        file_fingerprint(&checkpoint_path),
        good_fingerprint,
        "a refused write must not even rename the file"
    );

    // The evidence survives: the next load restores the checkpoint's own stamps
    // instead of granting (or losing) grace.
    let loaded = crate::dynamic_store::load_dynamic_circuits_from_disk(&config)
        .expect("the dynamic store loads");
    assert!(
        loaded.iter().all(|circuit| circuit.last_seen_unix == 1234),
        "the surviving stamps must come back: {loaded:?}"
    );

    // A MIXED snapshot checkpoints the rows that carry evidence and omits the
    // zero one: a zero is never persisted as a stamp.
    let mut mixed = vec![zeroed, stamped];
    mixed[1].last_seen_unix = 2000;
    assert!(
        crate::dynamic_store::persist_liveness_checkpoint(&config, &mixed)
            .expect("a stamped membership writes"),
        "rows with evidence still checkpoint"
    );
    let checkpoint = read_checkpoint(&checkpoint_path);
    assert_eq!(
        checkpoint.get("stamped").copied(),
        Some(2000),
        "the stamped row is checkpointed: {checkpoint:?}"
    );
    assert!(
        !checkpoint.contains_key("zeroed"),
        "a zero stamp is never persisted: {checkpoint:?}"
    );
}

/// REV-T4: a future checkpoint stamp is clamped to now, so a skewed or
/// hand-edited stamp cannot keep a circuit alive past its TTL.
#[test]
fn future_checkpoint_stamp_is_clamped_to_now() {
    let _guard = TEST_LOCK.lock();

    let (config, store_path, checkpoint_path) =
        isolated_dynamic_store_config("lqos-network-devices-future-stamp");
    write_store(
        &store_path,
        vec![dynamic_row("future-circuit", Ipv4Addr::new(10, 75, 0, 1))],
    );
    let now_unix = lqos_utils::unix_time::unix_now().expect("clock");
    std::fs::write(
        &checkpoint_path,
        serde_json::to_string(&serde_json::json!({"future-circuit": now_unix + 3_600}))
            .expect("checkpoint encodes"),
    )
    .expect("checkpoint writes");

    let loaded = crate::dynamic_store::load_dynamic_circuits_from_disk(&config)
        .expect("the dynamic store loads");
    assert_eq!(loaded.len(), 1);
    assert!(
        loaded[0].last_seen_unix <= now_unix,
        "a future stamp must be clamped to now, got {} (now {now_unix})",
        loaded[0].last_seen_unix
    );
    assert!(loaded[0].last_seen_unix > 0, "the row keeps a real stamp");
}

/// REV-T4: the checkpoint wins over a stale legacy stamp still present in the
/// store file (pre-checkpoint releases serialized it there).
#[test]
fn checkpoint_stamp_wins_over_a_stale_legacy_store_stamp() {
    let _guard = TEST_LOCK.lock();

    let (config, store_path, checkpoint_path) =
        isolated_dynamic_store_config("lqos-network-devices-checkpoint-precedence");
    let legacy = lqos_config::DynamicCircuitsFile {
        schema_version: 1,
        circuits: vec![dynamic_row("precedence-circuit", Ipv4Addr::new(10, 76, 0, 1))],
    };
    let mut legacy_json =
        serde_json::to_value(&legacy).expect("legacy store should serialize as a value");
    legacy_json["circuits"][0]["last_seen_unix"] = json!(100);
    std::fs::write(
        &store_path,
        serde_json::to_string_pretty(&legacy_json).expect("legacy store encodes"),
    )
    .expect("legacy store writes");
    std::fs::write(
        &checkpoint_path,
        serde_json::to_string(&serde_json::json!({"precedence-circuit": 999})).expect("encodes"),
    )
    .expect("checkpoint writes");

    let loaded = crate::dynamic_store::load_dynamic_circuits_from_disk(&config)
        .expect("the dynamic store loads");
    assert_eq!(
        loaded[0].last_seen_unix, 999,
        "the checkpoint must win over the stale legacy store value"
    );
}

/// REV-T5: a corrupt checkpoint grants exactly one startup grace and is
/// rewritten parseable; a later load restores the stamp instead of re-granting.
#[test]
fn corrupt_checkpoint_grants_one_grace_and_is_rewritten_parseable() {
    let _guard = TEST_LOCK.lock();

    let (config, store_path, checkpoint_path) =
        isolated_dynamic_store_config("lqos-network-devices-corrupt-checkpoint");
    write_store(
        &store_path,
        vec![dynamic_row("corrupt-circuit", Ipv4Addr::new(10, 77, 0, 1))],
    );
    std::fs::write(&checkpoint_path, "not json").expect("corrupt checkpoint writes");

    let now_unix = lqos_utils::unix_time::unix_now().expect("clock");
    let first = crate::dynamic_store::load_dynamic_circuits_from_disk(&config)
        .expect("the dynamic store loads");
    assert_eq!(first.len(), 1);
    assert!(
        first[0].last_seen_unix >= now_unix,
        "a corrupt checkpoint grants the grace, got {}",
        first[0].last_seen_unix
    );
    let rewritten = read_checkpoint(&checkpoint_path);
    assert_eq!(
        rewritten.get("corrupt-circuit"),
        Some(&first[0].last_seen_unix),
        "the grace must be rewritten parseable: {rewritten:?}"
    );

    // A second load restores that stamp rather than granting a NEW grace.
    let stamped = now_unix.saturating_sub(1_000);
    std::fs::write(
        &checkpoint_path,
        serde_json::to_string(&serde_json::json!({"corrupt-circuit": stamped})).expect("encodes"),
    )
    .expect("checkpoint writes");
    let second = crate::dynamic_store::load_dynamic_circuits_from_disk(&config)
        .expect("the dynamic store loads");
    assert_eq!(
        second[0].last_seen_unix, stamped,
        "a parseable checkpoint is restored verbatim, never re-granted"
    );
}

/// REV-T6: removal drops the circuit from the store AND from the checkpoint.
#[test]
fn removal_drops_the_circuit_from_the_store_and_the_checkpoint() {
    let _guard = TEST_LOCK.lock();
    crate::state::publish_dynamic_circuits_snapshot(Vec::new());

    let (config, _store_path, checkpoint_path) =
        isolated_dynamic_store_config("lqos-network-devices-removal-checkpoint");
    crate::actor::replace_dynamic_circuit_by_ip_inner(
        &config,
        radius_plan_for_test("10.78.0.1", 100.0f32),
    )
    .expect("first circuit must reach the store");
    let mut second = radius_plan_for_test("10.78.0.2", 100.0f32);
    second.circuit_id = "radius:nas-1:user:efgh".to_string();
    second.device_id = second.circuit_id.clone();
    crate::actor::replace_dynamic_circuit_by_ip_inner(&config, second)
        .expect("second circuit must reach the store");
    assert_eq!(
        read_checkpoint(&checkpoint_path).len(),
        2,
        "both circuits must be checkpointed"
    );

    let removed = crate::actor::remove_dynamic_circuit_inner(&config, "radius:nas-1:user:abcd")
        .expect("removal must reach the store");
    assert!(removed, "the removal must report a change");

    let store = lqos_config::DynamicCircuitsFile::load(&config);
    assert_eq!(store.circuits.len(), 1, "the store must shrink to one row");
    let checkpoint = read_checkpoint(&checkpoint_path);
    assert!(
        !checkpoint.contains_key("radius:nas-1:user:abcd"),
        "the removed id must leave the checkpoint: {checkpoint:?}"
    );
    assert_eq!(
        checkpoint.len(),
        1,
        "the surviving circuit keeps its checkpoint entry: {checkpoint:?}"
    );
    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
}

/// u4-engine A5: a corrupt store at startup must never be overwritten by the
/// rows the process happens to hold.
///
/// The failure mode: an unparseable `dynamic_circuits.json` used to read as an
/// empty membership, so the first mutation rewrote the file from that empty
/// snapshot and deleted every circuit the process never read. A failed load now
/// publishes nothing and refuses membership writes, naming the file, until a
/// load succeeds.
#[test]
fn a_failed_store_load_blocks_membership_writes_until_a_load_succeeds() {
    use crate::state::{DynamicStoreMembership, note_dynamic_store_membership};

    let _guard = TEST_LOCK.lock();
    note_dynamic_store_membership(DynamicStoreMembership::Unknown);

    let (config, store_path, _checkpoint_path) =
        isolated_dynamic_store_config("lqos-network-devices-corrupt-store");

    // A membership this process already published. A failed reload must not
    // replace it with an empty snapshot.
    crate::state::publish_dynamic_circuits_snapshot(vec![dynamic_row(
        "live-circuit",
        Ipv4Addr::new(10, 91, 0, 1),
    )]);

    let corrupt = "{ this is not the dynamic-circuit store";
    std::fs::write(&store_path, corrupt).expect("the corrupt store writes");

    let err = crate::actor::reload_dynamic_circuits_inner(&config, "test")
        .expect_err("an unparseable store must fail the load");
    assert!(
        format!("{err:#}").contains("dynamic circuits file"),
        "the load error must name the file: {err:#}"
    );
    assert_eq!(
        crate::state::dynamic_circuits_snapshot().len(),
        1,
        "a failed load must not publish an empty snapshot over the live one"
    );

    let refused = crate::actor::replace_dynamic_circuit_by_ip_inner(
        &config,
        radius_plan_for_test("10.91.0.7", 100.0f32),
    )
    .expect_err("a replace must be refused while the membership is unknown");
    assert!(
        format!("{refused:#}").contains("refusing to write dynamic-circuit membership"),
        "the refusal must say what it refuses: {refused:#}"
    );
    assert!(
        crate::actor::upsert_dynamic_circuit_inner(
            &config,
            radius_plan_for_test("10.91.0.8", 100.0f32)
        )
        .is_err(),
        "an upsert must be refused"
    );
    assert!(
        crate::actor::remove_dynamic_circuit_inner(&config, "live-circuit").is_err(),
        "a remove must be refused: an empty snapshot is not an absent circuit"
    );
    assert!(
        crate::actor::prune_expired_dynamic_circuits_inner(&config).is_err(),
        "a TTL prune must be refused"
    );

    assert_eq!(
        std::fs::read_to_string(&store_path).expect("the corrupt store still exists"),
        corrupt,
        "a refused write must leave the file exactly as it was"
    );
    assert_eq!(crate::state::dynamic_circuits_snapshot().len(), 1);

    // Repair + reload: the process now holds the membership and writes resume.
    write_store(
        &store_path,
        vec![dynamic_row("recovered", Ipv4Addr::new(10, 91, 0, 2))],
    );
    crate::actor::reload_dynamic_circuits_inner(&config, "test")
        .expect("a readable store loads");
    crate::actor::replace_dynamic_circuit_by_ip_inner(
        &config,
        radius_plan_for_test("10.91.0.7", 100.0f32),
    )
    .expect("a successful load re-enables membership writes");
    assert_eq!(
        lqos_config::DynamicCircuitsFile::load(&config).circuits.len(),
        2,
        "the recovered membership plus the new circuit"
    );

    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    note_dynamic_store_membership(DynamicStoreMembership::Unknown);
}

/// The promoted host of the integration test: unshaped, inside the catch-all.
const PROMOTED_HOST: Ipv4Addr = Ipv4Addr::new(10, 92, 0, 7);
/// The provisioned host of the integration test: inside the SAME catch-all.
const PROVIDED_HOST: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 10);

/// One legacy `network.json` site, the parent `attach_to` names.
fn promotion_network_json() -> &'static str {
    r#"{
  "Site One": {
    "children": {},
    "id": "s1",
    "name": "Site One",
    "type": "Site",
    "downloadBandwidthMbps": 1000,
    "uploadBandwidthMbps": 1000
  }
}"#
}

/// One PROVISIONED circuit under the site, covering `PROVIDED_HOST`.
fn promotion_shaped_devices_csv() -> String {
    format!(
        concat!(
            "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,",
            "Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,",
            "Download Max Mbps,Upload Max Mbps,Comment\n",
            "\"static-1\",\"Static 1\",\"static-dev-1\",\"Static 1\",\"Site One\",\"\",",
            "\"\",\"aa:bb:cc:dd:ee:00\",\"{}/32\",\"\",10,5,100,50,\"\"\n",
        ),
        PROVIDED_HOST
    )
}

/// The hermetic dynamic config for the promotion integration test: a `0.0.0.0/0`
/// catch-all, so eligibility alone admits any host and only the coverage catalog
/// can exclude the provisioned one.
fn promotion_config(prefix: &str) -> Config {
    let (mut config, _store, _checkpoint) = isolated_dynamic_store_config(prefix);
    config.dynamic_circuits = Some(lqos_config::DynamicCircuitsConfig {
        enabled: true,
        ttl_seconds: 300,
        enable_unknown_ip_promotion: true,
        ranges: vec![lqos_config::DynamicCircuitRangeRule {
            name: "Default".to_string(),
            ip_range: "0.0.0.0/0".parse().expect("the catch-all parses"),
            download_min_mbps: 10.0,
            upload_min_mbps: 5.0,
            download_max_mbps: 100.0,
            upload_max_mbps: 50.0,
            attach_to: "Site One".to_string(),
        }],
        promotion_min_bytes_per_second: 1024,
        promotion_window_seconds: 2,
        promotion_cap_per_range_per_minute: 10,
    });
    config
}

/// The bridge circuit id the range fallback carries in these tests. The actor
/// never resolves this number back to a circuit id — it is the COUNTER SOURCE
/// identity (u11), so any stable value models "the fallback's accumulator".
const FALLBACK_BRIDGE_CIRCUIT: u64 = 7;

/// One candidate observation carried by the range fallback's accumulator: the
/// u11 signal (a host the fallback already shapes).
fn served(ip: std::net::IpAddr, cumulative_bytes: u64) -> crate::HostActivity {
    crate::HostActivity {
        ip,
        cumulative_bytes,
        source: crate::ActivitySource::Circuit(FALLBACK_BRIDGE_CIRCUIT),
    }
}

/// One candidate observation from the bridge's unmapped class (an address no
/// prefix Map covers).
fn unmapped(ip: std::net::IpAddr, cumulative_bytes: u64) -> crate::HostActivity {
    crate::HostActivity {
        ip,
        cumulative_bytes,
        source: crate::ActivitySource::Unmapped,
    }
}

/// u11 A5 (the Oracle's control): an address `ip_ranges.ignore_subnets` names is
/// never promoted, even when no circuit shapes it. The ignore list is the
/// operator's management-exclusion contract and promotion is a way to mint a
/// shape, so it must be rechecked HERE, not trusted from the sender. The control
/// beside it proves the gate is the ignore list rather than the world: the same
/// series for a non-ignored host still promotes.
#[test]
fn an_ignored_host_is_never_promoted() {
    let _guard = TEST_LOCK.lock();
    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    crate::state::publish_shaped_devices(
        ConfigShapedDevices::default(),
        lqos_utils::DynamicOriginRows::default(),
    );
    with_network_json_write(|net_json| {
        net_json.nodes = vec![
            make_node("Root", Some("root"), None),
            make_node("Site One", Some("s1"), None),
        ];
    });

    let mut config = promotion_config("lqos-network-devices-promotion-ignored");
    config.ip_ranges.ignore_subnets = vec!["10.93.0.0/16".to_string()];
    let ignored: IpAddr = "10.93.0.5".parse().expect("the ignored host parses");
    let eligible: IpAddr = "10.94.0.5".parse().expect("the eligible host parses");
    let catalog = crate::network_devices_catalog();
    let mut engine = crate::dynamic_engine::PromotionEngine::default();
    let mut warned = std::collections::HashSet::new();
    // Identical series, one ignored host and one not. An ignored host is
    // UNMAPPED in the real world (the fallback's Map subtracts the ignores), so
    // the ignored sample carries the unmapped source and the other the
    // fallback's counter identity.
    for (at, bytes) in [(0u64, 0u64), (1_000, 4_096), (2_000, 8_192)] {
        crate::actor::promote_sustained_hosts(
            &mut engine,
            &mut warned,
            &config,
            &catalog,
            at,
            &[unmapped(ignored, bytes), served(eligible, bytes)],
        );
    }
    let store = lqos_config::DynamicCircuitsFile::load(&config);
    let ids: Vec<&str> = store
        .circuits
        .iter()
        .map(|circuit| circuit.shaped.circuit_id.as_str())
        .collect();
    assert!(
        !ids.iter().any(|id| id.ends_with(&ignored.to_string())),
        "an ignored host must never be promoted: {ids:?}"
    );
    assert_eq!(
        ids,
        vec![format!("dynamic-range:Default:{eligible}")],
        "the non-ignored host still promotes, so the gate is the ignore list"
    );

    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    with_network_json_write(|net_json| net_json.nodes = Vec::new());
}

/// u12: a network.json view with the PARSER'S shape — a synthetic sentinel at
/// index 0 (`Root`, no id) and the world's real top-level node hanging from it —
/// plus one child site. The hand-built `make_node` view has no parent indexes,
/// so the two root-detection cases need their own builders.
///
/// u13: the two real nodes carry IDS, because the legacy parser reports them and
/// they are NOT the exported effective tree's ids. A promotion gate on an
/// id-less view cannot see the defect: the id has to be present for "promotion
/// drops it" to mean anything.
fn parser_shaped_network_json() -> Vec<NetworkJsonNode> {
    let mut sentinel = make_node("Root", None, None);
    sentinel.parents = Vec::new();
    let mut root = make_node("u8 Testbed Root", Some("raw-view-root-id"), None);
    root.parents = vec![0, 1];
    root.immediate_parent = Some(0);
    let mut site = make_node("pop1", Some("raw-view-pop1-id"), None);
    site.parents = vec![0, 1, 2];
    site.immediate_parent = Some(1);
    vec![sentinel, root, site]
}

/// One promotion window of `passes` observations at 4096 B/s (the promotion
/// config's 2 s window / 1024 B/s floor).
fn drive_promotion_window(
    config: &Config,
    catalog: &crate::NetworkDevicesCatalog,
    engine: &mut crate::dynamic_engine::PromotionEngine,
    warned: &mut std::collections::HashSet<String>,
    host: IpAddr,
    start_millis: u64,
) {
    for step in 0..3u64 {
        crate::actor::promote_sustained_hosts(
            engine,
            warned,
            config,
            catalog,
            start_millis + step * 1_000,
            &[served(host, step * 4_096)],
        );
    }
}

/// Owns the three PROCESS-GLOBAL fixtures the promotion gates seed — the
/// dynamic-store snapshot, the shaped-devices snapshot and the `network.json`
/// view — and restores them on drop, INCLUDING while unwinding (u15 review
/// REV-T4). The pre-fix gates restored them in trailing statements, so a
/// panicking assert left every global seeded for the rest of the test binary and
/// one genuine failure could cascade into unrelated ones.
///
/// `TEST_LOCK` is a `parking_lot::Mutex`, which does not poison, so a failing
/// test cannot silently drop mutual exclusion for the later tests either.
struct PromotionGlobals {
    /// Held for the fixture's lifetime so the globals stay private to the test.
    _lock: parking_lot::MutexGuard<'static, ()>,
}

impl PromotionGlobals {
    /// Seeds both snapshots empty and the `network.json` view with `nodes`.
    fn seeded_with(nodes: Vec<NetworkJsonNode>) -> Self {
        let lock = TEST_LOCK.lock();
        crate::state::publish_dynamic_circuits_snapshot(Vec::new());
        crate::state::publish_shaped_devices(
            ConfigShapedDevices::default(),
            lqos_utils::DynamicOriginRows::default(),
        );
        with_network_json_write(|net_json| net_json.nodes = nodes);
        Self { _lock: lock }
    }
}

impl Drop for PromotionGlobals {
    fn drop(&mut self) {
        crate::state::publish_dynamic_circuits_snapshot(Vec::new());
        crate::state::publish_shaped_devices(
            ConfigShapedDevices::default(),
            lqos_utils::DynamicOriginRows::default(),
        );
        with_network_json_write(|net_json| net_json.nodes = Vec::new());
    }
}

/// u15 A4: a BLANK rule attachment writes the FOLD's OWN root convention and
/// nothing else — the sentinel name `Root`, no id — so promotion never depends on
/// a view of the loaded topology. The `network.json` view here is the legacy
/// parser's shape (synthetic sentinel, then the real nodes), which is exactly
/// what the old "first node after the sentinel" guess read; that guess answered
/// `u8 Testbed Root` (u12) and, against the LIVE store, `dynamic-range:range-24`.
/// Neither is written any more, and never a circuit id.
#[test]
fn a_blank_attach_writes_the_folds_root_sentinel() {
    let _globals = PromotionGlobals::seeded_with(parser_shaped_network_json());

    let mut config = promotion_config("lqos-network-devices-promotion-root");
    config
        .dynamic_circuits
        .as_mut()
        .expect("the promotion config carries the section")
        .ranges[0]
        .attach_to = String::new();
    let host: IpAddr = "10.94.9.1".parse().expect("the host parses");
    let catalog = crate::network_devices_catalog();
    let mut engine = crate::dynamic_engine::PromotionEngine::default();
    let mut warned = std::collections::HashSet::new();
    drive_promotion_window(&config, &catalog, &mut engine, &mut warned, host, 0);

    let store = lqos_config::DynamicCircuitsFile::load(&config);
    let row = store
        .circuits
        .first()
        .expect("the blank-attach rule must promote the host");
    let host_v4 = match host {
        IpAddr::V4(ip) => ip,
        IpAddr::V6(_) => panic!("the test host is v4"),
    };
    assert_eq!(
        row.shaped.ipv4,
        vec![(host_v4, 32)],
        "the promoted host is carried as its own /32"
    );
    assert_eq!(
        row.shaped.parent_node, "Root",
        "a blank attach writes the fold's OWN root sentinel, not a node guessed from the store view"
    );
    assert_eq!(
        row.shaped.parent_node_id, None,
        "the sentinel carries no id: the fold proves the parent against the export, and the view's \
         id is not that tree's id (the live u8 deferral)"
    );
    assert_eq!(
        row.shaped.anchor_node_id, None,
        "the anchor id stays None for the same reason"
    );
    assert!(
        !row.shaped.parent_node.contains("dynamic-range:"),
        "a circuit id is not a node and must never be a parent: {}",
        row.shaped.parent_node
    );
    assert!(warned.is_empty(), "the root convention warns nothing");
}

/// u15 A1, the explicit half: `attach_to = "Root"` IS the root convention. That
/// NAME is the fold's sentinel (the store's synthetic root node is named `Root`,
/// and the data plane reserves the id `root`), so it writes the sentinel instead
/// of looking the name up in the store view — where the only match would be the
/// legacy parser's synthetic index-0 entry.
///
/// Case and surrounding whitespace do not change that (u15 review REV-T6): the
/// predicate is `trim().eq_ignore_ascii_case`, so `"root"` and `" Root "` write
/// the same canonical value.
#[test]
fn an_explicit_root_attach_writes_the_folds_root_sentinel() {
    for (index, attach_to) in ["Root", "root", " Root "].into_iter().enumerate() {
        let _globals = PromotionGlobals::seeded_with(parser_shaped_network_json());

        let mut config =
            promotion_config(&format!("lqos-network-devices-promotion-root-named-{index}"));
        config
            .dynamic_circuits
            .as_mut()
            .expect("the promotion config carries the section")
            .ranges[0]
            .attach_to = attach_to.to_string();
        let host: IpAddr = "10.94.9.4".parse().expect("the host parses");
        let catalog = crate::network_devices_catalog();
        let mut engine = crate::dynamic_engine::PromotionEngine::default();
        let mut warned = std::collections::HashSet::new();
        drive_promotion_window(&config, &catalog, &mut engine, &mut warned, host, 0);

        let store = lqos_config::DynamicCircuitsFile::load(&config);
        let row = store
            .circuits
            .first()
            .unwrap_or_else(|| panic!("attach_to {attach_to:?} must promote the host"));
        assert_eq!(
            row.shaped.parent_node, "Root",
            "attach_to {attach_to:?} is the root convention, written canonically"
        );
        assert_eq!(row.shaped.parent_node_id, None);
        assert!(
            warned.is_empty(),
            "the root convention warns nothing ({attach_to:?})"
        );
    }
}

/// u12 A3(b): a NAMED rule attachment promotes under THAT node, by its canonical
/// name and id.
#[test]
fn a_named_attach_promotes_under_that_node() {
    let _guard = TEST_LOCK.lock();
    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    crate::state::publish_shaped_devices(
        ConfigShapedDevices::default(),
        lqos_utils::DynamicOriginRows::default(),
    );
    with_network_json_write(|net_json| net_json.nodes = parser_shaped_network_json());

    let mut config = promotion_config("lqos-network-devices-promotion-named");
    config
        .dynamic_circuits
        .as_mut()
        .expect("the promotion config carries the section")
        .ranges[0]
        .attach_to = "pop1".to_string();
    let host: IpAddr = "10.94.9.2".parse().expect("the host parses");
    let catalog = crate::network_devices_catalog();
    let mut engine = crate::dynamic_engine::PromotionEngine::default();
    let mut warned = std::collections::HashSet::new();
    drive_promotion_window(&config, &catalog, &mut engine, &mut warned, host, 0);

    let store = lqos_config::DynamicCircuitsFile::load(&config);
    let row = store
        .circuits
        .first()
        .expect("the named-attach rule must promote the host");
    assert_eq!(row.shaped.parent_node, "pop1", "the named node is the parent");
    assert_eq!(
        row.shaped.parent_node_id, None,
        "u13: the named node's view id is dropped too — only the name is provable"
    );
    assert!(warned.is_empty(), "a resolved parent warns nothing");

    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    with_network_json_write(|net_json| net_json.nodes = Vec::new());
}

/// u12 A3(c): an attachment that names no node writes NO row (an unshaped row
/// would defer on every publish) and warns ONCE per rule for the process
/// lifetime, not once per promotion window.
#[test]
fn an_unresolvable_attach_writes_no_row_and_warns_once() {
    let _guard = TEST_LOCK.lock();
    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    crate::state::publish_shaped_devices(
        ConfigShapedDevices::default(),
        lqos_utils::DynamicOriginRows::default(),
    );
    with_network_json_write(|net_json| net_json.nodes = parser_shaped_network_json());

    let mut config = promotion_config("lqos-network-devices-promotion-unresolvable");
    config
        .dynamic_circuits
        .as_mut()
        .expect("the promotion config carries the section")
        .ranges[0]
        .attach_to = "No Such Node".to_string();
    let host: IpAddr = "10.94.9.3".parse().expect("the host parses");
    let catalog = crate::network_devices_catalog();
    let mut engine = crate::dynamic_engine::PromotionEngine::default();
    let mut warned = std::collections::HashSet::new();
    // Two full windows: the warn-once set is the only thing that keeps the
    // second decision quiet.
    drive_promotion_window(&config, &catalog, &mut engine, &mut warned, host, 0);
    drive_promotion_window(&config, &catalog, &mut engine, &mut warned, host, 10_000);

    let store = lqos_config::DynamicCircuitsFile::load(&config);
    assert!(
        store.circuits.is_empty(),
        "no row may be written for an unresolvable parent: {:?}",
        store
            .circuits
            .iter()
            .map(|circuit| circuit.shaped.parent_node.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        warned.len(),
        1,
        "one warn per rule, not one per window: {warned:?}"
    );
    assert!(warned.contains("Default"));

    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    with_network_json_write(|net_json| net_json.nodes = Vec::new());
}

/// u4-engine A4/A7 + u11: scripted FALLBACK-carried samples promote exactly the
/// in-range host no ordinary circuit shapes, and the created circuit publishes as
/// an ordinary leaf plus a `/32` Map.
#[test]
fn fallback_carried_samples_promote_one_circuit_that_publishes_a_leaf_and_map() {
    let _guard = TEST_LOCK.lock();
    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    // The parent resolver reads the process-global `network.json` view; give it
    // the same single site the hermetic world carries.
    with_network_json_write(|net_json| {
        net_json.nodes = vec![
            make_node("Root", Some("root"), None),
            make_node("Site One", Some("s1"), None),
        ];
    });

    let config = promotion_config("lqos-network-devices-promotion");
    // REV-D2/REV-T1: the range gets exactly ONE promotion slot per minute, so
    // "the already-promoted host wrongly read as uncovered" becomes observable as
    // the fate of a genuinely unshaped host beside it (below). One slot is also
    // the production default's behaviour under pressure, so nothing exotic.
    let mut config = config;
    config
        .dynamic_circuits
        .as_mut()
        .expect("the promotion config carries the section")
        .promotion_cap_per_range_per_minute = 1;
    // u7 subtracts `ip_ranges.ignore_subnets` from a catch-all fallback, so the
    // DEFAULT list (`192.168.0.0/16`) would fragment the /0 and this test's stated
    // premise — "the fallback covers everything" — would stop holding for the
    // addresses it asserts. The ignore semantics have their own gates
    // (`catch_all_fallback_excludes_ignored_subnets_end_to_end`, group_13); here
    // the world is the all-covering one the promotion gate is about.
    config.ip_ranges.ignore_subnets = Vec::new();
    let directory = std::path::PathBuf::from(&config.lqos_directory);
    std::fs::write(directory.join("network.json"), promotion_network_json())
        .expect("network.json writes");
    std::fs::write(
        directory.join("ShapedDevices.csv"),
        promotion_shaped_devices_csv(),
    )
    .expect("ShapedDevices.csv writes");
    lqos_topology_compile::materialize_manual_topology(&config)
        .expect("the manual topology materializes");

    let shaped = crate::load_shaped_devices_for_config(&config)
        .expect("the shaped devices load")
        .shaped;
    assert_eq!(
        shaped.devices.len(),
        1,
        "PREMISE: the provisioned CSV carries exactly one device"
    );

    // Publish BEFORE promoting: the daemon's promotion input is the PUBLISHED
    // world, and u5 folds the catch-all fallback into it, so this is the world in
    // which "the fallback covers everything" must still let a host promote.
    let canonical = lqos_topology::load_canonical_topology_state(&config);
    let artifacts =
        lqos_topology::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &lqos_overrides::TopologyOverridesFile::default(),
            &lqos_config::TopologyAttachmentHealthStateFile::default(),
        )
        .expect("the effective topology artifacts build");
    lqos_topology::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("the effective topology artifacts publish");
    let published = lqos_config::load_shaping_inputs_at_path(
        &lqos_config::topology_shaping_inputs_path(&config),
    )
    .expect("the published shaping inputs load");
    let fallback = published
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:Default")
        .expect("u5: the catch-all rule is an ordinary published circuit");
    assert!(
        fallback.devices.iter().all(|device| device.dynamic),
        "the fallback is dynamic-origin, so it is not provisioned coverage"
    );
    // Publish the loaded world through the SAME state the daemon uses, so the
    // coverage test reads the origins from the artifact leg (never a hand-built
    // marker set that could pass while production fails).
    let loaded = crate::load_shaped_devices_for_config(&config)
        .expect("the published world loads");
    assert!(
        matches!(
            loaded.source,
            crate::LoadedShapedDevicesSource::RuntimeTopology { .. }
        ),
        "PREMISE: the runtime artifact is the loaded source"
    );
    assert!(
        loaded
            .dynamic_origin_rows
            .contains_row("dynamic-range:Default", "dynamic-range:Default"),
        "the marker must survive the fold → artifact → catalog leg"
    );
    crate::state::publish_shaped_devices(loaded.shaped, loaded.dynamic_origin_rows.clone());
    let catalog = crate::network_devices_catalog();

    // Scripted unshaped samples: baseline, one interval, then the 2 s window.
    let mut engine = crate::dynamic_engine::PromotionEngine::default();
    let mut warned = std::collections::HashSet::new();
    let host = std::net::IpAddr::V4(PROMOTED_HOST);
    let provisioned = std::net::IpAddr::V4(PROVIDED_HOST);
    crate::actor::promote_sustained_hosts(
        &mut engine,
        &mut warned,
        &config,
        &catalog,
        1_000,
        &[served(host, 0), served(provisioned, 0)],
    );
    crate::actor::promote_sustained_hosts(
        &mut engine,
        &mut warned,
        &config,
        &catalog,
        2_000,
        &[served(host, 4_096), served(provisioned, 4_096)],
    );
    assert!(
        crate::state::dynamic_circuits_snapshot().is_empty(),
        "one second of traffic is not a sustained window"
    );
    crate::actor::promote_sustained_hosts(
        &mut engine,
        &mut warned,
        &config,
        &catalog,
        3_000,
        &[served(host, 8_192), served(provisioned, 8_192)],
    );

    let store = lqos_config::DynamicCircuitsFile::load(&config);
    assert_eq!(
        store.circuits.len(),
        1,
        "exactly one circuit: the provisioned host inside the catch-all is covered"
    );
    let created = &store.circuits[0];
    let circuit_id = created.shaped.circuit_id.clone();
    assert_eq!(circuit_id, format!("dynamic-range:Default:{PROMOTED_HOST}"));
    assert_eq!(
        created.shaped.parent_node, "Site One",
        "the matched rule's attach_to is the parent"
    );
    assert_eq!(created.shaped.ipv4, vec![(PROMOTED_HOST, 32)]);
    assert_eq!(
        (
            created.shaped.download_min_mbps,
            created.shaped.upload_min_mbps,
            created.shaped.download_max_mbps,
            created.shaped.upload_max_mbps
        ),
        (10.0, 5.0, 100.0, 50.0),
        "the matched rule's rates are the plan"
    );
    assert!(
        created.last_seen_unix == 0,
        "liveness never belongs in the hashed store file"
    );
    let checkpoint = read_checkpoint(&lqos_config::dynamic_circuits_state_path(&config));
    assert!(
        checkpoint
            .get(&lqos_utils::normalize_circuit_id_key(&circuit_id))
            .is_some_and(|stamp| *stamp > 0),
        "a promotion is liveness evidence and must be checkpointed: {checkpoint:?}"
    );

    // Re-publish so the transaction reflects the promotion; the catch-all
    // fallback stays in it (both Maps must coexist: /0 and /32).
    lqos_topology::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("the effective topology artifacts republish");
    let transaction: lqos_dataplane_types::DataplaneTransaction = serde_json::from_str(
        &std::fs::read_to_string(lqos_config::topology_dataplane_transaction_path(&config))
            .expect("the published transaction reads"),
    )
    .expect("the published transaction parses");
    assert!(
        transaction.leaf_ops.iter().any(|op| matches!(
            op,
            lqos_dataplane_types::DataplaneLeafOp::Upsert { circuit_id: leaf, .. }
                if leaf == &circuit_id
        )),
        "the promoted circuit must be an ordinary leaf"
    );
    assert!(
        transaction.mapping_ops.iter().any(|op| matches!(
            op,
            lqos_dataplane_types::DataplaneMappingOp::Map { ip, prefix_len: 32, circuit_id: mapped, .. }
                if *ip == host && mapped == &circuit_id
        )),
        "the promoted circuit must carry its /32 Map"
    );
    assert!(
        transaction.mapping_ops.iter().any(|op| matches!(
            op,
            lqos_dataplane_types::DataplaneMappingOp::Map { prefix_len: 0, circuit_id: mapped, .. }
                if mapped == "dynamic-range:Default"
        )),
        "the /0 fallback keeps packet-one coverage alongside the promoted /32"
    );

    // ---- REV-D2/REV-T1: the SECOND coverage operand, exercised ----
    //
    // The store row is published while the SHAPED artifact is still stale (the real
    // daemon transient), so the promoted host is covered only by
    // `PublishedCoverage::promotion_covered`'s overlay arm — `provisioned_prefixes`
    // excludes the fold-synthesized fallback and does not have the promotion yet.
    //
    // `store.circuits.len()` alone cannot see this: re-promoting the same host
    // REPLACES its row by IP. So the gate also proves the cap slot: a restarted
    // engine (no in-memory memory of the promotion) sees a second, genuinely
    // unshaped host in the same window. With the overlay operand the promoted host
    // consumes nothing and the second host promotes (2 circuits); without it the
    // promoted host takes the ONLY slot and the second host is starved (1 circuit).
    const OTHER_HOST: Ipv4Addr = Ipv4Addr::new(10, 92, 0, 9);
    let fresh = crate::network_devices_catalog();
    assert!(
        fresh.dynamic_covers_ip(&lqos_utils::XdpIpAddress::from_ip(host)),
        "PREMISE: the promoted row is in the published overlay snapshot"
    );
    let mut restarted = crate::dynamic_engine::PromotionEngine::default();
    let mut warned = std::collections::HashSet::new();
    let other = std::net::IpAddr::V4(OTHER_HOST);
    for (at, bytes) in [(10_000u64, 0u64), (11_000, 4_096), (12_000, 8_192)] {
        crate::actor::promote_sustained_hosts(
            &mut restarted,
            &mut warned,
            &config,
            &fresh,
            at,
            &[served(host, bytes)],
        );
    }
    let after_restart = lqos_config::DynamicCircuitsFile::load(&config);
    assert_eq!(
        after_restart.circuits.len(),
        1,
        "an already-promoted host must not create a second circuit: {:?}",
        after_restart
            .circuits
            .iter()
            .map(|circuit| circuit.shaped.circuit_id.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        crate::state::dynamic_circuits_snapshot().len(),
        1,
        "the published snapshot agrees: the promotion is one row"
    );
    for (at, bytes) in [(20_000u64, 0u64), (21_000, 4_096), (22_000, 8_192)] {
        crate::actor::promote_sustained_hosts(
            &mut restarted,
            &mut warned,
            &config,
            &fresh,
            at,
            &[served(host, bytes), served(other, bytes)],
        );
    }
    let after_other = lqos_config::DynamicCircuitsFile::load(&config);
    assert!(
        after_other
            .circuits
            .iter()
            .any(|circuit| circuit.shaped.circuit_id
                == format!("dynamic-range:Default:{OTHER_HOST}")),
        "the range's one slot must still be free for a genuinely unshaped host — a \
         covered host that consumed it would starve this promotion permanently. \
         Circuits: {:?}",
        after_other
            .circuits
            .iter()
            .map(|circuit| circuit.shaped.circuit_id.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        after_other.circuits.len(),
        2,
        "exactly the promoted host and the neighbour"
    );

    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    // Leave the shared shaped world as empty as the test found it.
    crate::state::publish_shaped_devices(
        ConfigShapedDevices::default(),
        lqos_utils::DynamicOriginRows::default(),
    );
    with_network_json_write(|net_json| net_json.nodes = Vec::new());
}

/// The longest-prefix fallback answer over one family's prefixes, or `None` when
/// no fallback covers the address.
fn fallback_lpm(maps: &[(u8, IpAddr)], address: IpAddr) -> Option<(u8, IpAddr)> {
    maps.iter()
        .copied()
        .filter(|(prefix_len, network)| {
            IpNetwork::new(*network, *prefix_len)
                .map(|prefix| prefix.contains(address))
                .unwrap_or(false)
        })
        .max_by_key(|(prefix_len, _)| *prefix_len)
}

/// The emitted prefixes as the CIDR strings an operator reads.
fn cidr_set(maps: &[(u8, IpAddr)]) -> std::collections::BTreeSet<String> {
    maps.iter().map(|(len, ip)| format!("{ip}/{len}")).collect()
}

/// u7 A4 (second Oracle consult): a catch-all fallback subtracts
/// `ip_ranges.ignore_subnets`, so no catch-all ever classifies management traffic.
///
/// End to end through the production seam: the topology fold writes the fallback
/// rows, `publish_effective_topology_artifacts` compiles them with the NORMAL
/// compiler, and the emitted transaction is read back. One circuit and one device
/// per rule carry the whole complement at the rule's rates, the coverage is EXACTLY
/// rule-minus-ignores, and the fallback owns nothing for promotion.
#[test]
fn catch_all_fallback_excludes_ignored_subnets_end_to_end() {
    let _guard = TEST_LOCK.lock();
    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    with_network_json_write(|net_json| {
        net_json.nodes = vec![
            make_node("Root", Some("root"), None),
            make_node("Site One", Some("s1"), None),
        ];
    });

    let mut config = promotion_config("lqos-network-devices-catch-all-subtraction");
    config
        .dynamic_circuits
        .as_mut()
        .expect("the promotion config carries the section")
        .ranges
        .push(lqos_config::DynamicCircuitRangeRule {
            name: "Default-v6".to_string(),
            ip_range: "::/0".parse().expect("the v6 catch-all parses"),
            download_min_mbps: 10.0,
            upload_min_mbps: 5.0,
            download_max_mbps: 100.0,
            upload_max_mbps: 50.0,
            attach_to: "Site One".to_string(),
        });
    // The Oracle's two ignores plus duplicate, nested and overlapping entries:
    // redundant ignores cannot change the complement.
    config.ip_ranges.ignore_subnets = vec![
        "192.168.0.0/16".to_string(),
        "192.168.0.0/16".to_string(),
        "192.168.0.0/24".to_string(),
        "192.168.128.0/18".to_string(),
        "fd00::/8".to_string(),
        "fd00::/8".to_string(),
        "fd00::/16".to_string(),
        "fd12:3456::/32".to_string(),
    ];
    let directory = std::path::PathBuf::from(&config.lqos_directory);
    std::fs::write(directory.join("network.json"), promotion_network_json())
        .expect("network.json writes");
    std::fs::write(
        directory.join("ShapedDevices.csv"),
        promotion_shaped_devices_csv(),
    )
    .expect("ShapedDevices.csv writes");
    lqos_topology_compile::materialize_manual_topology(&config)
        .expect("the manual topology materializes");

    let canonical = lqos_topology::load_canonical_topology_state(&config);
    let artifacts =
        lqos_topology::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &lqos_overrides::TopologyOverridesFile::default(),
            &lqos_config::TopologyAttachmentHealthStateFile::default(),
        )
        .expect("the effective topology artifacts build");
    lqos_topology::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("the effective topology artifacts publish");
    let transaction: lqos_dataplane_types::DataplaneTransaction = serde_json::from_str(
        &std::fs::read_to_string(lqos_config::topology_dataplane_transaction_path(&config))
            .expect("the published transaction reads"),
    )
    .expect("the published transaction parses");

    // The fallback Maps exactly as the compiler emitted them, per family.
    let mut v4_maps: Vec<(u8, IpAddr)> = Vec::new();
    let mut v6_maps: Vec<(u8, IpAddr)> = Vec::new();
    for op in &transaction.mapping_ops {
        let lqos_dataplane_types::DataplaneMappingOp::Map {
            prefix_len,
            ip,
            circuit_id,
            ..
        } = op
        else {
            continue;
        };
        if !circuit_id.starts_with("dynamic-range:") {
            continue;
        }
        match ip {
            IpAddr::V4(_) => v4_maps.push((*prefix_len, *ip)),
            IpAddr::V6(_) => v6_maps.push((*prefix_len, *ip)),
        }
    }

    // 1) Every emitted fallback Map is disjoint from EVERY configured ignore.
    let ignored = config
        .ip_ranges
        .ignore_subnets
        .iter()
        .map(|cidr| {
            let (address, prefix_len) = cidr.split_once('/').expect("a test cidr carries a length");
            IpNetwork::new(
                address.parse::<IpAddr>().expect("the ignore address parses"),
                prefix_len.parse().expect("the ignore length parses"),
            )
            .expect("the ignore prefix is normalized")
        })
        .collect::<Vec<IpNetwork>>();
    for (prefix_len, ip) in v4_maps.iter().chain(v6_maps.iter()) {
        let mapped = IpNetwork::new(*ip, *prefix_len).expect("an emitted Map is a valid prefix");
        for ignore in &ignored {
            assert!(
                !mapped.contains(ignore.network_address())
                    && !ignore.contains(mapped.network_address()),
                "fallback Map {mapped} overlaps ignored subnet {ignore}"
            );
        }
    }

    // 2) The union is EXACTLY rule-minus-ignores, hand-derived per bisection level.
    let expected_v4: std::collections::BTreeSet<String> = [
        "0.0.0.0/1",
        "128.0.0.0/2",
        "192.0.0.0/9",
        "192.128.0.0/11",
        "192.160.0.0/13",
        "192.169.0.0/16",
        "192.170.0.0/15",
        "192.172.0.0/14",
        "192.176.0.0/12",
        "192.192.0.0/10",
        "193.0.0.0/8",
        "194.0.0.0/7",
        "196.0.0.0/6",
        "200.0.0.0/5",
        "208.0.0.0/4",
        "224.0.0.0/3",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(
        cidr_set(&v4_maps),
        expected_v4,
        "the v4 catch-all must become exactly 0.0.0.0/0 minus 192.168.0.0/16"
    );
    let expected_v6: std::collections::BTreeSet<String> = [
        "::/1",
        "8000::/2",
        "c000::/3",
        "e000::/4",
        "f000::/5",
        "f800::/6",
        "fc00::/8",
        "fe00::/7",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(
        cidr_set(&v6_maps),
        expected_v6,
        "the v6 catch-all must become exactly ::/0 minus fd00::/8"
    );

    // 3) The LPM answers: the ignored hosts are unmapped by every fallback, their
    //    neighbours inside the same rule are covered.
    for ignored_host in ["192.168.1.1", "fd00::1"] {
        let host: IpAddr = ignored_host.parse().expect("the probe parses");
        assert_eq!(
            fallback_lpm(&v4_maps, host),
            None,
            "no fallback may map the ignored v4 host {ignored_host}"
        );
        assert_eq!(
            fallback_lpm(&v6_maps, host),
            None,
            "no fallback may map the ignored v6 host {ignored_host}"
        );
    }
    for covered in ["192.167.255.255", "192.169.0.0", "2001:db8::1"] {
        let host: IpAddr = covered.parse().expect("the probe parses");
        assert!(
            fallback_lpm(&v4_maps, host).is_some() || fallback_lpm(&v6_maps, host).is_some(),
            "a permitted host keeps packet-one coverage: {covered}"
        );
    }

    // 4) ONE leaf per surviving rule, at the rule's configured rates. Rates are
    //    Mbps on the plan and kbps on the wire (x1000 at emit).
    for circuit_id in ["dynamic-range:Default", "dynamic-range:Default-v6"] {
        let leaves = transaction
            .leaf_ops
            .iter()
            .filter_map(|op| match op {
                lqos_dataplane_types::DataplaneLeafOp::Upsert {
                    circuit_id: leaf,
                    download,
                    upload,
                    ..
                } if leaf == circuit_id => Some((download, upload)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            leaves.len(),
            1,
            "exactly one leaf per surviving rule ('{circuit_id}'): {:?}",
            transaction.leaf_ops
        );
        let (download, upload) = leaves[0];
        assert_eq!(
            (
                download.commit_kbps,
                download.ceil_kbps,
                upload.commit_kbps,
                upload.ceil_kbps
            ),
            (10_000, 100_000, 5_000, 50_000),
            "the subtraction does not change the rule's plan"
        );
    }

    // 5) The fallback owns nothing for PROMOTION: the published world covers the
    //    permitted host (its Map is a fallback), the provisioned-coverage
    //    authority does not, and the origin marker is what carries the split.
    let loaded = crate::load_shaped_devices_for_config(&config).expect("the published world loads");
    assert!(
        loaded
            .dynamic_origin_rows
            .contains_row("dynamic-range:Default", "dynamic-range:Default"),
        "the v4 fallback row keeps the dynamic-origin marker"
    );
    assert!(
        loaded
            .dynamic_origin_rows
            .contains_circuit("dynamic-range:Default-v6"),
        "the v6 fallback row keeps the dynamic-origin marker"
    );
    for permitted in ["192.169.0.0", "2001:db8::1"] {
        let address = lqos_utils::XdpIpAddress::from_ip(permitted.parse().expect("the probe parses"));
        assert!(
            loaded
                .shaped
                .get_device_from_ip(&address)
                .is_some_and(|device| device.circuit_id.starts_with("dynamic-range:")),
            "PREMISE: the permitted host is covered by a fallback row ({permitted})"
        );
    }
    crate::state::publish_shaped_devices(loaded.shaped, loaded.dynamic_origin_rows.clone());
    let catalog = crate::network_devices_catalog();
    let coverage =
        crate::dynamic::PublishedCoverage::new(catalog.shaped_devices(), &loaded.dynamic_origin_rows);
    for permitted in ["192.169.0.0", "2001:db8::1"] {
        let address = lqos_utils::XdpIpAddress::from_ip(permitted.parse().expect("the probe parses"));
        assert!(
            !coverage.provisioned_covers(&address),
            "a fallback is never provisioned coverage ({permitted})"
        );
        assert!(
            !coverage.promotion_covered(&catalog, &address),
            "a fallback owns nothing for promotion ({permitted})"
        );
    }

    crate::state::publish_dynamic_circuits_snapshot(Vec::new());
    // Leave the shared shaped world as empty as the test found it.
    crate::state::publish_shaped_devices(
        ConfigShapedDevices::default(),
        lqos_utils::DynamicOriginRows::default(),
    );
    with_network_json_write(|net_json| net_json.nodes = Vec::new());
}

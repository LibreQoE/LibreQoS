use crate::{
    DynamicCircuit, ShapedDevicesCatalog, resolve_parent_node_reference, with_network_json_write,
};
use lqos_config::{ConfigShapedDevices, NetworkJsonNode, ShapedDevice};
use lqos_utils::rtt::RttBuffer;
use lqos_utils::units::DownUpOrder;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::sync::Arc;

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

#[test]
fn catalog_device_by_hashes_prefers_device_hash() {
    let _guard = TEST_LOCK.lock();

    let a = ShapedDevice {
        circuit_id: "circuit-a".into(),
        device_id: "device-a".into(),
        circuit_hash: 10,
        device_hash: 100,
        ..Default::default()
    };

    let b = ShapedDevice {
        circuit_id: "circuit-b".into(),
        device_id: "device-b".into(),
        circuit_hash: 20,
        device_hash: 200,
        ..Default::default()
    };

    let mut shaped = ConfigShapedDevices::default();
    shaped.replace_with_new_data(vec![a.clone(), b.clone()]);

    let catalog = ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped));

    let resolved = catalog
        .device_by_hashes(Some(a.device_hash), Some(b.circuit_hash))
        .expect("Expected shaped device match");
    assert_eq!(resolved.device_id, a.device_id);

    let fallback = catalog
        .device_by_hashes(Some(999), Some(b.circuit_hash))
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

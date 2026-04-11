use crate::{ShapedDevicesCatalog, resolve_parent_node_reference, with_network_json_write};
use lqos_config::{ConfigShapedDevices, NetworkJsonNode, ShapedDevice};
use lqos_utils::rtt::RttBuffer;
use lqos_utils::units::DownUpOrder;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
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

    let mut a = ShapedDevice::default();
    a.circuit_id = "circuit-a".into();
    a.device_id = "device-a".into();
    a.circuit_hash = 10;
    a.device_hash = 100;

    let mut b = ShapedDevice::default();
    b.circuit_id = "circuit-b".into();
    b.device_id = "device-b".into();
    b.circuit_hash = 20;
    b.device_hash = 200;

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

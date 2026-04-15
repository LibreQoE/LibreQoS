use crate::catalog::ShapedDevicesCatalog;
use crate::dynamic::DynamicCircuit;
use crate::hash_cache::ShapedDeviceHashCache;
use arc_swap::ArcSwap;
use lqos_config::{ConfigShapedDevices, NetworkJson};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
struct PublishedShapedState {
    shaped: Arc<ConfigShapedDevices>,
    cache: Arc<ShapedDeviceHashCache>,
    generation: u64,
}

static NEXT_SHAPED_GENERATION: AtomicU64 = AtomicU64::new(1);

static SHAPED_STATE: Lazy<ArcSwap<PublishedShapedState>> = Lazy::new(|| {
    ArcSwap::new(Arc::new(PublishedShapedState {
        shaped: Arc::new(ConfigShapedDevices::default()),
        cache: Arc::new(ShapedDeviceHashCache::default()),
        generation: 0,
    }))
});
static DYNAMIC_CIRCUITS: Lazy<ArcSwap<Vec<DynamicCircuit>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(Vec::new())));
static NETWORK_JSON: Lazy<RwLock<NetworkJson>> = Lazy::new(|| RwLock::new(NetworkJson::default()));

pub(crate) fn shaped_devices_snapshot() -> Arc<ConfigShapedDevices> {
    SHAPED_STATE.load_full().shaped.clone()
}

pub(crate) fn shaped_device_hash_cache_snapshot() -> Arc<ShapedDeviceHashCache> {
    SHAPED_STATE.load_full().cache.clone()
}

pub(crate) fn shaped_devices_catalog() -> ShapedDevicesCatalog {
    let state = SHAPED_STATE.load_full();
    ShapedDevicesCatalog::new(state.shaped.clone(), state.cache.clone(), state.generation)
}

pub(crate) fn dynamic_circuits_snapshot() -> Arc<Vec<DynamicCircuit>> {
    DYNAMIC_CIRCUITS.load_full()
}

pub(crate) fn with_network_json_read<R>(f: impl FnOnce(&NetworkJson) -> R) -> R {
    let reader = NETWORK_JSON.read();
    f(&reader)
}

pub(crate) fn with_network_json_write<R>(f: impl FnOnce(&mut NetworkJson) -> R) -> R {
    let mut writer = NETWORK_JSON.write();
    f(&mut writer)
}

pub(crate) fn publish_shaped_devices(new_file: ConfigShapedDevices) {
    let generation = NEXT_SHAPED_GENERATION.fetch_add(1, Ordering::Relaxed);
    let shaped = Arc::new(new_file);
    let cache = Arc::new(ShapedDeviceHashCache::from_devices(&shaped.devices));
    SHAPED_STATE.store(Arc::new(PublishedShapedState {
        shaped,
        cache,
        generation,
    }));
}

pub(crate) fn swap_shaped_devices_snapshot(
    new_snapshot: Arc<ConfigShapedDevices>,
) -> Arc<ConfigShapedDevices> {
    let generation = NEXT_SHAPED_GENERATION.fetch_add(1, Ordering::Relaxed);
    let cache = Arc::new(ShapedDeviceHashCache::from_devices(&new_snapshot.devices));
    let new_state = Arc::new(PublishedShapedState {
        shaped: new_snapshot,
        cache,
        generation,
    });
    let old = SHAPED_STATE.swap(new_state);
    old.shaped.clone()
}

fn network_json_with_carried_heatmaps(previous: &NetworkJson, mut next: NetworkJson) -> NetworkJson {
    next.carry_forward_heatmaps_from(previous);
    next
}

pub(crate) fn publish_network_json(new_file: NetworkJson) {
    let mut writer = NETWORK_JSON.write();
    *writer = network_json_with_carried_heatmaps(&writer, new_file);
}

pub(crate) fn publish_dynamic_circuits_snapshot(new_snapshot: Vec<DynamicCircuit>) {
    DYNAMIC_CIRCUITS.store(Arc::new(new_snapshot));
}

pub(crate) fn refresh_dynamic_circuits_last_seen_for_hashes(
    seen_device_hashes: &HashSet<i64>,
    seen_circuit_hashes: &HashSet<i64>,
    now_unix: u64,
) -> bool {
    let snapshot = dynamic_circuits_snapshot();
    if snapshot.is_empty() {
        return false;
    }

    let mut updated = snapshot.as_ref().clone();
    let mut changed = false;
    for circuit in updated.iter_mut() {
        let is_seen = seen_device_hashes.contains(&circuit.shaped.device_hash)
            || seen_circuit_hashes.contains(&circuit.shaped.circuit_hash);
        if is_seen && circuit.last_seen_unix != now_unix {
            circuit.last_seen_unix = now_unix;
            changed = true;
        }
    }

    if changed {
        publish_dynamic_circuits_snapshot(updated);
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::network_json_with_carried_heatmaps;
    use lqos_config::NetworkJsonNode;
    use lqos_utils::{
        qoq_heatmap::TemporalQoqHeatmap,
        rtt::RttBuffer,
        temporal_heatmap::TemporalHeatmap,
        units::DownUpOrder,
    };

    fn root_node() -> NetworkJsonNode {
        NetworkJsonNode {
            name: "Root".to_string(),
            id: None,
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
            active_attachment_name: None,
            heatmap: None,
            qoq_heatmap: None,
        }
    }

    fn site_node(name: &str, id: &str) -> NetworkJsonNode {
        NetworkJsonNode {
            name: name.to_string(),
            id: Some(id.to_string()),
            virtual_node: false,
            max_throughput: (100.0, 100.0),
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
            parents: vec![0],
            immediate_parent: Some(0),
            node_type: Some("site".to_string()),
            latitude: None,
            longitude: None,
            active_attachment_name: None,
            heatmap: None,
            qoq_heatmap: None,
        }
    }

    #[test]
    fn carried_heatmaps_survive_network_json_publish_replacement() {
        let mut previous = lqos_config::NetworkJson {
            nodes: vec![root_node(), site_node("Tower A", "tower-a")],
        };
        let site = previous
            .nodes
            .iter_mut()
            .find(|node| node.id.as_deref() == Some("tower-a"))
            .expect("previous site should exist");
        let heatmap = site.heatmap.get_or_insert_with(TemporalHeatmap::new);
        heatmap.add_sample(
            42.0,
            24.0,
            Some(10.0),
            Some(12.0),
            Some(15.0),
            Some(18.0),
            Some(1.5),
            Some(2.5),
        );
        let qoq_heatmap = site.qoq_heatmap.get_or_insert_with(TemporalQoqHeatmap::new);
        qoq_heatmap.add_sample(Some(91.0), Some(82.0));

        let next = lqos_config::NetworkJson {
            nodes: vec![root_node(), site_node("Tower A Renamed", "tower-a")],
        };

        let carried = network_json_with_carried_heatmaps(&previous, next);
        let site = carried
            .nodes
            .iter()
            .find(|node| node.id.as_deref() == Some("tower-a"))
            .expect("replacement site should exist");
        let blocks = site
            .heatmap
            .as_ref()
            .expect("site heatmap should carry forward")
            .blocks();
        let qoq_blocks = site
            .qoq_heatmap
            .as_ref()
            .expect("site qoq heatmap should carry forward")
            .blocks();

        assert_eq!(blocks.download[14], Some(42.0));
        assert_eq!(blocks.upload[14], Some(24.0));
        assert_eq!(blocks.retransmit_down[14], Some(1.5));
        assert_eq!(blocks.retransmit_up[14], Some(2.5));
        assert_eq!(qoq_blocks.download_total[14], Some(91.0));
        assert_eq!(qoq_blocks.upload_total[14], Some(82.0));
    }
}

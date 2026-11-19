use crate::catalog::ShapedDevicesCatalog;
use crate::dynamic::DynamicCircuit;
use crate::hash_cache::ShapedDeviceHashCache;
use arc_swap::ArcSwap;
use lqos_config::{ConfigShapedDevices, NetworkJson};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

#[derive(Clone)]
struct PublishedShapedState {
    shaped: Arc<ConfigShapedDevices>,
    cache: Arc<ShapedDeviceHashCache>,
    /// The device ROWS the published rows were synthesized for by the topology
    /// fold (design card §3 rule 2), keyed `(circuit id, device id)`. Empty when
    /// the snapshot came from a pre-fold source, which never carries dynamic
    /// rows.
    dynamic_origin_rows: Arc<lqos_utils::DynamicOriginRows>,
    generation: u64,
}

static NEXT_SHAPED_GENERATION: AtomicU64 = AtomicU64::new(1);

static SHAPED_STATE: Lazy<ArcSwap<PublishedShapedState>> = Lazy::new(|| {
    ArcSwap::new(Arc::new(PublishedShapedState {
        shaped: Arc::new(ConfigShapedDevices::default()),
        cache: Arc::new(ShapedDeviceHashCache::default()),
        dynamic_origin_rows: Arc::new(lqos_utils::DynamicOriginRows::default()),
        generation: 0,
    }))
});
static DYNAMIC_CIRCUITS: Lazy<ArcSwap<Vec<DynamicCircuit>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(Vec::new())));

/// Whether this process holds `dynamic_circuits.json`'s full membership.
///
/// A failed startup load must never be mistaken for an empty membership: the
/// first mutation would rewrite the store from the empty snapshot and delete
/// every circuit the load never read. `Failed` refuses membership writes until a
/// later load succeeds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DynamicStoreMembership {
    /// No load has run yet in this process (the actor loads at startup, before
    /// it processes commands).
    Unknown,
    /// The last load returned the store's membership (a missing file counts: it
    /// is an empty membership).
    Loaded,
    /// The last load failed; the in-memory snapshot is not the membership.
    Failed,
}

static DYNAMIC_STORE_MEMBERSHIP: AtomicU8 = AtomicU8::new(0);

pub(crate) fn dynamic_store_membership() -> DynamicStoreMembership {
    match DYNAMIC_STORE_MEMBERSHIP.load(Ordering::Relaxed) {
        1 => DynamicStoreMembership::Loaded,
        2 => DynamicStoreMembership::Failed,
        _ => DynamicStoreMembership::Unknown,
    }
}

/// Records the outcome of the last dynamic-store load.
pub(crate) fn note_dynamic_store_membership(membership: DynamicStoreMembership) {
    let value = match membership {
        DynamicStoreMembership::Unknown => 0,
        DynamicStoreMembership::Loaded => 1,
        DynamicStoreMembership::Failed => 2,
    };
    DYNAMIC_STORE_MEMBERSHIP.store(value, Ordering::Relaxed);
}
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

/// The device rows the published shaped rows were synthesized for by the
/// topology fold; empty when the snapshot is a pre-fold source.
pub(crate) fn dynamic_origin_rows() -> Arc<lqos_utils::DynamicOriginRows> {
    SHAPED_STATE.load().dynamic_origin_rows.clone()
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

pub(crate) fn publish_shaped_devices(
    new_file: ConfigShapedDevices,
    dynamic_origin_rows: lqos_utils::DynamicOriginRows,
) {
    let generation = NEXT_SHAPED_GENERATION.fetch_add(1, Ordering::Relaxed);
    let shaped = Arc::new(new_file);
    let cache = Arc::new(ShapedDeviceHashCache::from_devices(&shaped.devices));
    SHAPED_STATE.store(Arc::new(PublishedShapedState {
        shaped,
        cache,
        dynamic_origin_rows: Arc::new(dynamic_origin_rows),
        generation,
    }));
}

/// Replaces the published snapshot with an edited copy of the same world.
///
/// A swap is the operator/repair edit path: it modifies rows of the world it
/// replaces, so the row origins carry forward unchanged. Only a full reload
/// (which re-reads the fold) re-derives them.
pub(crate) fn swap_shaped_devices_snapshot(
    new_snapshot: Arc<ConfigShapedDevices>,
) -> Arc<ConfigShapedDevices> {
    let generation = NEXT_SHAPED_GENERATION.fetch_add(1, Ordering::Relaxed);
    let cache = Arc::new(ShapedDeviceHashCache::from_devices(&new_snapshot.devices));
    let dynamic_origin_rows = {
        let current = SHAPED_STATE.load();
        current.dynamic_origin_rows.clone()
    };
    let new_state = Arc::new(PublishedShapedState {
        shaped: new_snapshot,
        cache,
        dynamic_origin_rows,
        generation,
    });
    let old = SHAPED_STATE.swap(new_state);
    old.shaped.clone()
}

fn network_json_with_carried_runtime_state(
    previous: &NetworkJson,
    mut next: NetworkJson,
) -> NetworkJson {
    next.carry_forward_heatmaps_from(previous);
    next.carry_forward_live_counters_from(previous);
    next
}

pub(crate) fn publish_network_json(new_file: NetworkJson) {
    let mut writer = NETWORK_JSON.write();
    *writer = network_json_with_carried_runtime_state(&writer, new_file);
}

pub(crate) fn publish_dynamic_circuits_snapshot(new_snapshot: Vec<DynamicCircuit>) {
    DYNAMIC_CIRCUITS.store(Arc::new(new_snapshot));
}

/// How many refreshed circuit ids the liveness INFO line names before falling
/// back to a count. Bounds the line on a busy box (the feed runs on the
/// analytics poll path, one call per poll).
const LIVENESS_IDS_LOGGED: usize = 8;

/// K1 §1.4: the BRIDGE liveness feed. Touches `last_seen_unix` for dynamic
/// circuits whose normalized circuit id appears in the bridge's active set.
///
/// This is a memory-only touch; the actor's maintenance tick checkpoints the
/// result into the unhashed liveness file.
///
/// Side effects: publishes the updated snapshot and, when at least one stamp
/// moved, emits ONE bounded INFO line naming the refreshed circuit ids and the
/// stamp they received (first [`LIVENESS_IDS_LOGGED`] ids, then a count). That
/// line is the A7 instrument (u17): a row whose TTL clock moved because the
/// bridge still measured traffic for it is otherwise only inferable from the
/// checkpoint's value. Bounded on purpose — this runs on the analytics poll
/// path, so it must never become a per-id or per-packet log.
pub(crate) fn refresh_dynamic_circuits_last_seen_for_circuit_ids(
    seen_circuit_ids: &HashSet<String>,
    now_unix: u64,
) -> bool {
    let snapshot = dynamic_circuits_snapshot();
    if snapshot.is_empty() || seen_circuit_ids.is_empty() {
        return false;
    }

    let mut updated = snapshot.as_ref().clone();
    let mut changed = false;
    let mut refreshed = 0usize;
    let mut refreshed_ids: Vec<String> = Vec::new();
    for circuit in updated.iter_mut() {
        let key = lqos_utils::normalize_circuit_id_key(&circuit.shaped.circuit_id);
        if seen_circuit_ids.contains(&key) && circuit.last_seen_unix != now_unix {
            circuit.last_seen_unix = now_unix;
            changed = true;
            refreshed += 1;
            if refreshed_ids.len() < LIVENESS_IDS_LOGGED {
                refreshed_ids.push(circuit.shaped.circuit_id.clone());
            }
        }
    }

    if changed {
        let mut shown = refreshed_ids.join(", ");
        if refreshed > refreshed_ids.len() {
            shown.push_str(&format!(" (+{} more)", refreshed - refreshed_ids.len()));
        }
        tracing::info!(
            "Dynamic liveness: last_seen_unix={now_unix} refreshed {refreshed} circuit(s): {shown}"
        );
        publish_dynamic_circuits_snapshot(updated);
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::network_json_with_carried_runtime_state;
    use lqos_config::NetworkJsonNode;
    use lqos_utils::{rtt::RttBuffer, temporal_heatmap::TemporalHeatmap, units::DownUpOrder};

    fn root_node() -> NetworkJsonNode {
        NetworkJsonNode {
            name: "Root".to_string(),
            id: None,
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
            active_attachment_name: None,
            heatmap: None,
        }
    }

    fn site_node(name: &str, id: &str) -> NetworkJsonNode {
        NetworkJsonNode {
            name: name.to_string(),
            id: Some(id.to_string()),
            max_throughput: (100.0, 100.0),
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
            parents: vec![0],
            immediate_parent: Some(0),
            node_type: Some("site".to_string()),
            latitude: None,
            longitude: None,
            active_attachment_name: None,
            heatmap: None,
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
        site.current_throughput.down = 12_500_000;
        site.current_throughput.up = 6_250_000;
        site.current_tcp_retransmits.down = 15;
        site.current_tcp_retransmit_packets.down = 500;
        site.rtt_buffer = RttBuffer::new(
            lqos_utils::rtt::RttData::from_nanos(18_000_000),
            lqos_utils::rtt::FlowbeeEffectiveDirection::Download,
            1,
        );
        site.rtt_buffer.push(
            lqos_utils::rtt::RttData::from_nanos(18_000_000),
            lqos_utils::rtt::FlowbeeEffectiveDirection::Download,
            2,
        );
        site.rtt_buffer.push(
            lqos_utils::rtt::RttData::from_nanos(27_000_000),
            lqos_utils::rtt::FlowbeeEffectiveDirection::Upload,
            3,
        );
        site.rtt_buffer.push(
            lqos_utils::rtt::RttData::from_nanos(27_000_000),
            lqos_utils::rtt::FlowbeeEffectiveDirection::Upload,
            4,
        );

        let next = lqos_config::NetworkJson {
            nodes: vec![root_node(), site_node("Tower A Renamed", "tower-a")],
        };

        let carried = network_json_with_carried_runtime_state(&previous, next);
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
        assert_eq!(blocks.download[14], Some(42.0));
        assert_eq!(blocks.upload[14], Some(24.0));
        assert_eq!(blocks.retransmit_down[14], Some(1.5));
        assert_eq!(blocks.retransmit_up[14], Some(2.5));
        assert_eq!(site.current_throughput.down, 12_500_000);
        assert_eq!(site.current_throughput.up, 6_250_000);
        assert_eq!(site.current_tcp_retransmits.down, 15);
        assert_eq!(site.current_tcp_retransmit_packets.down, 500);
        assert_eq!(
            site.rtt_buffer
                .percentile(
                    lqos_utils::rtt::RttBucket::Current,
                    lqos_utils::rtt::FlowbeeEffectiveDirection::Download,
                    50,
                )
                .map(|rtt| rtt.as_millis()),
            Some(20.0)
        );
        assert_eq!(
            site.rtt_buffer
                .percentile(
                    lqos_utils::rtt::RttBucket::Current,
                    lqos_utils::rtt::FlowbeeEffectiveDirection::Upload,
                    50,
                )
                .map(|rtt| rtt.as_millis()),
            Some(30.0)
        );
    }

    /// K1 §1.4 #12-MED: the bridge liveness feed advances only for matching
    /// circuit ids.
    #[test]
    fn bridge_feed_advances_last_seen_only_for_matching_circuit_ids() {
        use std::collections::HashSet;
        let _guard = crate::tests::TEST_LOCK.lock();
        super::publish_dynamic_circuits_snapshot(Vec::new());
        let circuit = lqos_config::DynamicCircuit {
            shaped: lqos_config::ShapedDevice {
                circuit_id: "c1".to_string(),
                ..Default::default()
            },
            last_seen_unix: 100,
        };
        super::publish_dynamic_circuits_snapshot(vec![circuit]);

        // Bridge feed: advances for the matching circuit id.
        let ids: HashSet<String> = ["c1".to_string()].into_iter().collect();
        assert!(super::refresh_dynamic_circuits_last_seen_for_circuit_ids(
            &ids, 200
        ));
        assert_eq!(super::dynamic_circuits_snapshot()[0].last_seen_unix, 200);

        // A non-matching id does not advance.
        let other: HashSet<String> = ["c2".to_string()].into_iter().collect();
        assert!(!super::refresh_dynamic_circuits_last_seen_for_circuit_ids(
            &other, 300
        ));
        assert_eq!(super::dynamic_circuits_snapshot()[0].last_seen_unix, 200);
    }

    /// Stopping the liveness feed leaves `last_seen` at the last touch, so
    /// the store's TTL prune expires the circuit.
    #[test]
    fn stopping_both_feeds_expires_at_ttl() {
        let _guard = crate::tests::TEST_LOCK.lock();
        super::publish_dynamic_circuits_snapshot(Vec::new());
        let circuit = lqos_config::DynamicCircuit {
            shaped: lqos_config::ShapedDevice {
                circuit_id: "c-expire".to_string(),
                ..Default::default()
            },
            last_seen_unix: 1_000,
        };
        super::publish_dynamic_circuits_snapshot(vec![circuit]);
        // No feed touches last_seen; at ttl+1 the row is expired.
        let expired = crate::dynamic_engine::expired_dynamic_circuit_ids(
            &super::dynamic_circuits_snapshot(),
            1_000 + 60 + 1,
            60,
        );
        assert_eq!(expired, vec!["c-expire".to_string()]);
    }
}

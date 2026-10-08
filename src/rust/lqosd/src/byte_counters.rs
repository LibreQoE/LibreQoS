//! Optional cumulative byte counters for circuits and network nodes.
//!
//! When `[byte_counters] enabled = true` is set in `/etc/lqos.conf`, the
//! one-second throughput tracker feeds per-circuit byte deltas into this
//! module. Counters are cumulative since `lqosd` started and are not
//! persisted across restarts.
//!
//! Only per-circuit totals and their identity are stored. Node totals are
//! derived when a snapshot is requested by walking each circuit's parent node
//! through the runtime network tree, so node totals include descendants.
//! Configured circuits follow the current device catalog, and circuits removed
//! from configuration keep their last known identity, so their accumulated
//! bytes continue to contribute to their last known parent while `lqosd` runs.

use fxhash::FxHashMap;
use lqos_bus::{ByteCountersSnapshot, CircuitByteCounters, NodeByteCounters};
use lqos_config::{NetworkJson, ShapedDevice};
use lqos_network_devices::{NetworkDevicesCatalog, ParentNodeLookup};
use lqos_utils::units::DownUpOrder;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cumulative bytes and last-known identity keyed by circuit hash.
static CIRCUIT_BYTES: Lazy<RwLock<FxHashMap<i64, CircuitCounter>>> =
    Lazy::new(|| RwLock::new(FxHashMap::default()));

/// Whether byte counter collection is enabled.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Enables or disables byte counter collection.
///
/// Side effects: updates the global collection flag. Counters recorded before
/// a disable are kept until the process restarts.
pub(crate) fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Returns true when byte counter collection is enabled.
pub(crate) fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Adds one cycle of per-circuit byte deltas to the cumulative counters.
///
/// New circuits capture their identity from `catalog` on first sight.
///
/// Side effects: mutates the shared cumulative counter map. Callers may
/// already hold the network tree write lock, so this function must never
/// acquire a network tree lock.
pub(crate) fn add_batch(deltas: FxHashMap<i64, DownUpOrder<u64>>, catalog: &NetworkDevicesCatalog) {
    if deltas.is_empty() {
        return;
    }
    apply_deltas(&mut CIRCUIT_BYTES.write(), deltas, |circuit_hash| {
        describe_circuit(catalog, circuit_hash)
    });
}

/// Merges per-circuit deltas into a cumulative counter map, capturing identity
/// through `describe` when a circuit is first seen or has none captured yet.
fn apply_deltas(
    counters: &mut FxHashMap<i64, CircuitCounter>,
    deltas: FxHashMap<i64, DownUpOrder<u64>>,
    describe: impl Fn(i64) -> Option<CircuitDescriptor>,
) {
    for (circuit_hash, bytes) in deltas {
        let counter = counters.entry(circuit_hash).or_default();
        ensure_descriptor(counter, circuit_hash, &describe);
        counter.bytes.checked_add(bytes);
    }
}

/// Captures a counter's identity on first sight or when it could not be
/// resolved previously.
fn ensure_descriptor(
    counter: &mut CircuitCounter,
    circuit_hash: i64,
    describe: &impl Fn(i64) -> Option<CircuitDescriptor>,
) {
    if counter.descriptor.is_none() {
        counter.descriptor = describe(circuit_hash);
    }
}

/// Refreshes descriptors for circuits that still resolve in the device
/// catalog, so live circuits follow renames and re-parenting. Circuits that no
/// longer resolve keep their last-known descriptor and continue contributing
/// to their last known parent.
///
/// The catalog is indexed once per refresh so circuits missing from
/// `ShapedDevices.csv` cannot trigger per-circuit fallback scans.
///
/// Side effects: mutates the shared cumulative counter map.
fn refresh_descriptors() {
    let catalog = lqos_network_devices::network_devices_catalog();
    let mut devices_by_circuit_hash: FxHashMap<i64, &ShapedDevice> = FxHashMap::default();
    for device in catalog.iter_all_devices() {
        devices_by_circuit_hash
            .entry(device.circuit_hash)
            .or_insert(device);
    }

    let mut counters = CIRCUIT_BYTES.write();
    for (circuit_hash, counter) in counters.iter_mut() {
        if let Some(device) = devices_by_circuit_hash.get(circuit_hash) {
            counter.descriptor = Some(describe_device(device));
        }
    }
}

/// Resolves one circuit's identity fields from the combined device catalog.
///
/// This function must never acquire a network tree lock, because callers may
/// already hold the tree write lock.
fn describe_circuit(
    catalog: &NetworkDevicesCatalog,
    circuit_hash: i64,
) -> Option<CircuitDescriptor> {
    let device = catalog.device_by_hashes(None, Some(circuit_hash))?;
    Some(describe_device(device))
}

/// Builds identity fields for one device.
///
/// The runtime effective parent wins when one is published, so accounting
/// follows the same tree as the rest of the runtime. This function must never
/// acquire a network tree lock, because callers may already hold the tree
/// write lock.
fn describe_device(device: &ShapedDevice) -> CircuitDescriptor {
    let (parent_node, parent_node_id) =
        match crate::shaped_devices_tracker::effective_parent_for_circuit(&device.circuit_id) {
            Some(parent) => (parent.name, parent.id),
            None => (device.parent_node.clone(), device.parent_node_id.clone()),
        };
    CircuitDescriptor {
        circuit_id: device.circuit_id.clone(),
        circuit_name: device.circuit_name.clone(),
        parent_node,
        parent_node_id,
    }
}

/// Builds a cumulative byte counter snapshot for the bus.
///
/// Side effects: acquires the counter lock and the runtime network tree read
/// lock. Descriptors are refreshed and the counter lock is released before the
/// tree lock is taken, so the two locks are never held at the same time and
/// cannot invert the tracker's `NETWORK_JSON` then `CIRCUIT_BYTES` order.
pub(crate) fn snapshot() -> ByteCountersSnapshot {
    if !is_enabled() {
        return ByteCountersSnapshot {
            enabled: false,
            nodes: Vec::new(),
            circuits: Vec::new(),
        };
    }

    refresh_descriptors();

    let counters: Vec<CircuitCounter> = CIRCUIT_BYTES.read().values().cloned().collect();

    lqos_network_devices::with_network_json_read(|tree| rollup(tree, &counters))
}

/// Cumulative bytes plus the identity captured when the circuit was first seen.
#[derive(Clone, Default)]
struct CircuitCounter {
    bytes: DownUpOrder<u64>,
    descriptor: Option<CircuitDescriptor>,
}

/// Identity fields for one circuit, captured from the device catalog.
#[derive(Clone)]
struct CircuitDescriptor {
    circuit_id: String,
    circuit_name: String,
    parent_node: String,
    parent_node_id: Option<String>,
}

/// Rolls circuit totals up the network tree and formats a bus snapshot.
///
/// Node totals include every descendant circuit. Circuits keep a row even when
/// their parent node can no longer be resolved, but those bytes contribute to
/// no node total until the parent resolves again.
fn rollup(tree: &NetworkJson, counters: &[CircuitCounter]) -> ByteCountersSnapshot {
    let mut node_totals = vec![DownUpOrder::zeroed(); tree.nodes.len()];
    let mut node_by_name: FxHashMap<&str, usize> = FxHashMap::default();
    let mut node_by_id: FxHashMap<&str, usize> = FxHashMap::default();
    for (index, node) in tree.nodes.iter().enumerate() {
        node_by_name.entry(node.name.as_str()).or_insert(index);
        if let Some(id) = node.id.as_deref() {
            node_by_id.entry(id).or_insert(index);
        }
    }
    let parent_lookup = ParentNodeLookup::from_nodes(&tree.nodes);

    let mut circuit_rows = Vec::new();
    for counter in counters {
        let Some(descriptor) = counter.descriptor.as_ref() else {
            continue;
        };
        if let Some(parent_index) =
            resolve_parent_index(&parent_lookup, descriptor, &node_by_id, &node_by_name)
        {
            let parent_node = &tree.nodes[parent_index];
            if parent_node.parents.is_empty() {
                // The root node carries no parent list, so add its own traffic.
                if let Some(total) = node_totals.get_mut(parent_index) {
                    total.checked_add(counter.bytes);
                }
            } else {
                for &index in &parent_node.parents {
                    if let Some(total) = node_totals.get_mut(index) {
                        total.checked_add(counter.bytes);
                    }
                }
            }
        }
        circuit_rows.push(CircuitByteCounters {
            circuit_id: descriptor.circuit_id.clone(),
            circuit_name: descriptor.circuit_name.clone(),
            parent_node: descriptor.parent_node.clone(),
            bytes: counter.bytes,
        });
    }
    circuit_rows.sort_by(|a, b| a.circuit_id.cmp(&b.circuit_id));

    let nodes = tree
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| NodeByteCounters {
            node_id: node.id.clone(),
            name: node.name.clone(),
            bytes: node_totals[index],
        })
        .collect();

    ByteCountersSnapshot {
        enabled: true,
        nodes,
        circuits: circuit_rows,
    }
}

/// Resolves a circuit's parent node with the canonical id, name, and alias
/// precedence used by the rest of the runtime.
fn resolve_parent_index(
    parent_lookup: &ParentNodeLookup<'_>,
    descriptor: &CircuitDescriptor,
    node_by_id: &FxHashMap<&str, usize>,
    node_by_name: &FxHashMap<&str, usize>,
) -> Option<usize> {
    let parent = parent_lookup.resolve(
        &descriptor.parent_node,
        descriptor.parent_node_id.as_deref(),
    )?;
    parent
        .id
        .as_deref()
        .and_then(|id| node_by_id.get(id).copied())
        .or_else(|| node_by_name.get(parent.name.as_str()).copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_config::NetworkJsonNode;
    use lqos_utils::rtt::RttBuffer;

    fn node(
        name: &str,
        id: Option<&str>,
        parents: Vec<usize>,
        immediate_parent: Option<usize>,
    ) -> NetworkJsonNode {
        NetworkJsonNode {
            name: name.to_string(),
            id: id.map(str::to_string),
            virtual_node: false,
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
            parents,
            immediate_parent,
            node_type: None,
            latitude: None,
            longitude: None,
            active_attachment_name: None,
            heatmap: None,
            qoq_heatmap: None,
        }
    }

    fn descriptor(circuit_id: &str, parent_node: &str) -> CircuitDescriptor {
        CircuitDescriptor {
            circuit_id: circuit_id.to_string(),
            circuit_name: format!("{circuit_id} name"),
            parent_node: parent_node.to_string(),
            parent_node_id: None,
        }
    }

    fn counter(descriptor: CircuitDescriptor, bytes: DownUpOrder<u64>) -> CircuitCounter {
        CircuitCounter {
            bytes,
            descriptor: Some(descriptor),
        }
    }

    fn test_tree() -> NetworkJson {
        NetworkJson {
            nodes: vec![
                node("Root", None, vec![], None),
                node("Site A", Some("site-a"), vec![0, 1], Some(0)),
                node("Site A1", None, vec![0, 1, 2], Some(1)),
                node("Site B", None, vec![0, 3], Some(0)),
            ],
        }
    }

    #[test]
    fn apply_deltas_accumulates_repeated_circuit_deltas() {
        let mut counters = FxHashMap::default();
        apply_deltas(
            &mut counters,
            FxHashMap::from_iter([(7, DownUpOrder::new(100, 40))]),
            |_| Some(descriptor("circuit-7", "Site A")),
        );
        apply_deltas(
            &mut counters,
            FxHashMap::from_iter([(7, DownUpOrder::new(10, 5))]),
            |_| None,
        );

        let counter = counters.get(&7).expect("counter should exist");
        assert_eq!(counter.bytes, DownUpOrder::new(110, 45));
        assert_eq!(
            counter.descriptor.as_ref().map(|d| d.circuit_id.as_str()),
            Some("circuit-7")
        );
    }

    #[test]
    fn apply_deltas_fills_descriptor_when_first_lookup_missed() {
        let mut counters = FxHashMap::default();
        apply_deltas(
            &mut counters,
            FxHashMap::from_iter([(7, DownUpOrder::new(1, 0))]),
            |_| None,
        );
        apply_deltas(
            &mut counters,
            FxHashMap::from_iter([(7, DownUpOrder::new(2, 0))]),
            |_| Some(descriptor("circuit-7", "Site A")),
        );

        let counter = counters.get(&7).expect("counter should exist");
        assert_eq!(counter.bytes, DownUpOrder::new(3, 0));
        assert_eq!(
            counter.descriptor.as_ref().map(|d| d.circuit_id.as_str()),
            Some("circuit-7")
        );
    }

    #[test]
    fn rollup_includes_descendant_bytes_in_ancestor_totals() {
        let counters = vec![
            counter(descriptor("circuit-1", "Site A1"), DownUpOrder::new(100, 0)),
            counter(descriptor("circuit-2", "Site A"), DownUpOrder::new(0, 50)),
        ];
        let snapshot = rollup(&test_tree(), &counters);

        assert!(snapshot.enabled);
        assert_eq!(snapshot.nodes[0].bytes, DownUpOrder::new(100, 50));
        assert_eq!(snapshot.nodes[1].bytes, DownUpOrder::new(100, 50));
        assert_eq!(snapshot.nodes[2].bytes, DownUpOrder::new(100, 0));
        assert_eq!(snapshot.nodes[3].bytes, DownUpOrder::new(0, 0));
        assert_eq!(snapshot.nodes[1].node_id.as_deref(), Some("site-a"));
        assert_eq!(snapshot.circuits.len(), 2);
        assert_eq!(snapshot.circuits[0].circuit_id, "circuit-1");
        assert_eq!(snapshot.circuits[0].parent_node, "Site A1");
        assert_eq!(snapshot.circuits[1].bytes, DownUpOrder::new(0, 50));
    }

    #[test]
    fn rollup_keeps_circuit_rows_when_the_parent_node_is_missing() {
        let counters = vec![
            counter(descriptor("circuit-1", "Site B"), DownUpOrder::new(100, 0)),
            counter(
                descriptor("circuit-2", "Missing Node"),
                DownUpOrder::new(10, 10),
            ),
        ];
        let snapshot = rollup(&test_tree(), &counters);

        assert_eq!(snapshot.circuits.len(), 2);
        assert_eq!(snapshot.circuits[0].circuit_id, "circuit-1");
        assert_eq!(snapshot.circuits[1].circuit_id, "circuit-2");
        assert_eq!(snapshot.nodes[0].bytes, DownUpOrder::new(100, 0));
        assert_eq!(snapshot.nodes[3].bytes, DownUpOrder::new(100, 0));
    }

    #[test]
    fn rollup_skips_counters_without_identity() {
        let counters = vec![
            counter(descriptor("circuit-1", "Site B"), DownUpOrder::new(100, 0)),
            CircuitCounter::default(),
        ];
        let snapshot = rollup(&test_tree(), &counters);

        assert_eq!(snapshot.circuits.len(), 1);
        assert_eq!(snapshot.circuits[0].circuit_id, "circuit-1");
    }

    #[test]
    fn rollup_prefers_stable_node_ids_over_names() {
        let mut tree = test_tree();
        tree.nodes
            .push(node("Site B", Some("site-b-2"), vec![0, 4], Some(0)));
        let mut identified = descriptor("circuit-1", "Site B");
        identified.parent_node_id = Some("site-b-2".to_string());

        let snapshot = rollup(&tree, &[counter(identified, DownUpOrder::new(5, 5))]);

        assert_eq!(snapshot.nodes[0].bytes, DownUpOrder::new(5, 5));
        assert_eq!(snapshot.nodes[3].bytes, DownUpOrder::new(0, 0));
        assert_eq!(snapshot.nodes[4].bytes, DownUpOrder::new(5, 5));
    }

    #[test]
    fn rollup_resolves_parent_aliases() {
        let mut tree = test_tree();
        tree.nodes[2].active_attachment_name = Some("Site A1 Alias".to_string());
        let counters = vec![counter(
            descriptor("circuit-1", "Site A1 Alias"),
            DownUpOrder::new(4, 2),
        )];
        let snapshot = rollup(&tree, &counters);

        assert_eq!(snapshot.nodes[0].bytes, DownUpOrder::new(4, 2));
        assert_eq!(snapshot.nodes[1].bytes, DownUpOrder::new(4, 2));
        assert_eq!(snapshot.nodes[2].bytes, DownUpOrder::new(4, 2));
    }

    #[test]
    fn rollup_counts_circuits_attached_to_root() {
        let counters = vec![counter(
            descriptor("circuit-1", "Root"),
            DownUpOrder::new(7, 3),
        )];
        let snapshot = rollup(&test_tree(), &counters);

        assert_eq!(snapshot.nodes[0].bytes, DownUpOrder::new(7, 3));
    }

    #[test]
    fn rollup_sorts_circuit_rows_by_circuit_id() {
        let counters = vec![
            counter(descriptor("circuit-b", "Site A"), DownUpOrder::new(1, 0)),
            counter(descriptor("circuit-a", "Site A"), DownUpOrder::new(1, 0)),
        ];
        let snapshot = rollup(&test_tree(), &counters);

        let ids: Vec<&str> = snapshot
            .circuits
            .iter()
            .map(|row| row.circuit_id.as_str())
            .collect();
        assert_eq!(ids, vec!["circuit-a", "circuit-b"]);
    }

    #[test]
    fn disabled_snapshot_reports_empty_lists() {
        let previous = is_enabled();
        set_enabled(false);
        let snapshot = snapshot();
        set_enabled(previous);

        assert!(!snapshot.enabled);
        assert!(snapshot.nodes.is_empty());
        assert!(snapshot.circuits.is_empty());
    }
}

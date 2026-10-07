//! Optional cumulative byte counters for circuits and network nodes.
//!
//! When `[byte_counters] enabled = true` is set in `/etc/lqos.conf`, the
//! one-second throughput tracker feeds per-circuit byte deltas into this
//! module. Counters are cumulative since `lqosd` started and are not
//! persisted across restarts.
//!
//! Only per-circuit totals are stored. Node totals are derived when a
//! snapshot is requested by walking each circuit's effective parent node
//! through the runtime network tree, so node totals include descendants.

use fxhash::FxHashMap;
use lqos_bus::{ByteCountersSnapshot, CircuitByteCounters, NodeByteCounters};
use lqos_config::NetworkJson;
use lqos_utils::units::DownUpOrder;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cumulative bytes keyed by circuit hash.
static CIRCUIT_BYTES: Lazy<RwLock<FxHashMap<i64, DownUpOrder<u64>>>> =
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
/// Side effects: mutates the shared cumulative counter map.
pub(crate) fn add_batch(deltas: FxHashMap<i64, DownUpOrder<u64>>) {
    if deltas.is_empty() {
        return;
    }
    merge(&mut CIRCUIT_BYTES.write(), deltas);
}

/// Merges per-circuit deltas into a cumulative counter map.
fn merge(
    counters: &mut FxHashMap<i64, DownUpOrder<u64>>,
    deltas: FxHashMap<i64, DownUpOrder<u64>>,
) {
    for (circuit_hash, bytes) in deltas {
        counters.entry(circuit_hash).or_default().checked_add(bytes);
    }
}

/// Builds a cumulative byte counter snapshot for the bus.
///
/// Side effects: acquires the counter read lock and the runtime network tree
/// read lock.
pub(crate) fn snapshot() -> ByteCountersSnapshot {
    if !is_enabled() {
        return ByteCountersSnapshot {
            enabled: false,
            nodes: Vec::new(),
            circuits: Vec::new(),
        };
    }

    let circuit_bytes: Vec<(i64, DownUpOrder<u64>)> = CIRCUIT_BYTES
        .read()
        .iter()
        .map(|(circuit_hash, bytes)| (*circuit_hash, *bytes))
        .collect();

    let catalog = lqos_network_devices::network_devices_catalog();
    lqos_network_devices::with_network_json_read(|tree| {
        rollup(tree, &circuit_bytes, |circuit_hash| {
            let device = catalog.device_by_hashes(None, Some(circuit_hash))?;
            Some(CircuitDescriptor {
                circuit_id: device.circuit_id.clone(),
                circuit_name: device.circuit_name.clone(),
                parent_node: device.parent_node.clone(),
            })
        })
    })
}

/// Identity fields for one circuit, resolved from the shaped-devices catalog.
struct CircuitDescriptor {
    circuit_id: String,
    circuit_name: String,
    parent_node: String,
}

/// Rolls circuit totals up the network tree and formats a bus snapshot.
///
/// Node totals include every descendant circuit. Circuits whose device or
/// parent node cannot be resolved are omitted from both lists.
fn rollup(
    tree: &NetworkJson,
    circuits: &[(i64, DownUpOrder<u64>)],
    mut lookup: impl FnMut(i64) -> Option<CircuitDescriptor>,
) -> ByteCountersSnapshot {
    let mut node_totals = vec![DownUpOrder::zeroed(); tree.nodes.len()];
    let mut node_index: FxHashMap<&str, usize> = FxHashMap::default();
    for (index, node) in tree.nodes.iter().enumerate() {
        node_index.entry(node.name.as_str()).or_insert(index);
    }

    let mut circuit_rows = Vec::new();
    for (circuit_hash, bytes) in circuits {
        let Some(descriptor) = lookup(*circuit_hash) else {
            continue;
        };
        let Some(&parent_index) = node_index.get(descriptor.parent_node.as_str()) else {
            continue;
        };
        for &index in &tree.nodes[parent_index].parents {
            if let Some(total) = node_totals.get_mut(index) {
                total.checked_add(*bytes);
            }
        }
        circuit_rows.push(CircuitByteCounters {
            circuit_id: descriptor.circuit_id,
            circuit_name: descriptor.circuit_name,
            parent_node: descriptor.parent_node,
            bytes: *bytes,
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

    fn descriptor(circuit_id: &str, parent_node: &str) -> Option<CircuitDescriptor> {
        Some(CircuitDescriptor {
            circuit_id: circuit_id.to_string(),
            circuit_name: format!("{circuit_id} name"),
            parent_node: parent_node.to_string(),
        })
    }

    fn test_tree() -> NetworkJson {
        NetworkJson {
            nodes: vec![
                node("Root", None, vec![0], None),
                node("Site A", Some("site-a"), vec![0, 1], Some(0)),
                node("Site A1", None, vec![0, 1, 2], Some(1)),
                node("Site B", None, vec![0, 3], Some(0)),
            ],
        }
    }

    #[test]
    fn merge_accumulates_repeated_circuit_deltas() {
        let mut counters = FxHashMap::default();
        merge(
            &mut counters,
            FxHashMap::from_iter([(7, DownUpOrder::new(100, 40))]),
        );
        merge(
            &mut counters,
            FxHashMap::from_iter([(7, DownUpOrder::new(10, 5))]),
        );

        assert_eq!(counters.get(&7), Some(&DownUpOrder::new(110, 45)));
    }

    #[test]
    fn rollup_includes_descendant_bytes_in_ancestor_totals() {
        let circuits = vec![(1, DownUpOrder::new(100, 0)), (2, DownUpOrder::new(0, 50))];
        let snapshot = rollup(&test_tree(), &circuits, |circuit_hash| match circuit_hash {
            1 => descriptor("circuit-1", "Site A1"),
            2 => descriptor("circuit-2", "Site A"),
            _ => None,
        });

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
    fn rollup_skips_unresolved_circuits_and_parents() {
        let circuits = vec![
            (1, DownUpOrder::new(100, 0)),
            (2, DownUpOrder::new(10, 10)),
            (3, DownUpOrder::new(5, 5)),
        ];
        let snapshot = rollup(&test_tree(), &circuits, |circuit_hash| match circuit_hash {
            1 => descriptor("circuit-1", "Site B"),
            2 => None,
            _ => descriptor("circuit-3", "Missing Node"),
        });

        assert_eq!(snapshot.circuits.len(), 1);
        assert_eq!(snapshot.circuits[0].circuit_id, "circuit-1");
        assert_eq!(snapshot.nodes[0].bytes, DownUpOrder::new(100, 0));
        assert_eq!(snapshot.nodes[3].bytes, DownUpOrder::new(100, 0));
    }
}

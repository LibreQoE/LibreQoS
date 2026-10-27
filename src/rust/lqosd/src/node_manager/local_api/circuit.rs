use crate::node_manager::local_api::ethernet_caps::ethernet_advisory_for_circuit;
use crate::shaped_devices_tracker::effective_parent_for_circuit;
use lqos_config::{CircuitEthernetMetadata, ShapedDevice};
use lqos_utils::normalize_circuit_id_key;
use lqos_utils::units::DownUpOrder;
use serde::{Deserialize, Serialize};

/// Canonical circuit parent resolved from `network.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CircuitParentNode {
    /// Canonical node name from `network.json`.
    pub name: String,
    /// Optional stable node identifier from `network.json` metadata.
    pub id: Option<String>,
    /// Optional tree index of the parent node (the `parent` query the
    /// tree page needs to render the node directly). Present when the
    /// parent was resolved through the circuit's own tree node.
    #[serde(default)]
    pub tree_index: Option<usize>,
}

/// Circuit-page queue-stats mode.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CircuitQueueStatsMode {
    /// Standard per-circuit queue stats are expected to be available.
    Live,
}

/// Circuit-page payload containing shaped devices plus optional Ethernet advisory metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CircuitByIdData {
    /// Shaped-device rows for the requested circuit.
    pub devices: Vec<ShapedDevice>,
    /// Canonical circuit parent resolved from the shaped-device parent and `network.json`.
    pub parent_node: Option<CircuitParentNode>,
    /// Queue-stats behavior for the active topology mode.
    pub queue_stats_mode: CircuitQueueStatsMode,
    /// Optional negotiated-Ethernet advisory derived from integration metadata.
    pub ethernet_advisory: Option<CircuitEthernetMetadata>,
    /// Current programmed max rate for the circuit queue, in Mbps.
    pub effective_rate_mbps: Option<DownUpOrder<f32>>,
}

fn load_ethernet_advisory(
    circuit_id: &str,
    devices: &[ShapedDevice],
) -> Option<CircuitEthernetMetadata> {
    let device_ids: std::collections::HashSet<&str> = devices
        .iter()
        .map(|device| device.device_id.as_str())
        .collect();
    ethernet_advisory_for_circuit(circuit_id, &device_ids)
}

fn canonical_parent_node(devices: &mut [ShapedDevice]) -> Option<CircuitParentNode> {
    let mut resolved_parent = None;

    for device in devices.iter_mut() {
        let Some(resolved) = lqos_network_devices::resolve_parent_node_reference(
            &device.parent_node,
            device.parent_node_id.as_deref(),
        ) else {
            continue;
        };
        if resolved_parent.is_none() {
            resolved_parent = Some(CircuitParentNode {
                name: resolved.name.clone(),
                id: resolved.id.clone(),
                tree_index: None,
            });
        }
        device.parent_node = resolved.name;
        device.parent_node_id = resolved.id;
    }

    resolved_parent
}

/// Driver demo finding (2026-09-08): when the CSV parent name does not
/// resolve against the network json (the T1 tree names artifact-derived
/// nodes by their stable id, so a NAME lookup for the legacy CSV name
/// misses), fall back to the circuit's OWN tree node — the T1 tree
/// carries a circuit-type node named by the circuit id, and its
/// immediate parent IS the circuit's tree parent. The display name
/// stays the human CSV name; the id + tree index make the tree link
/// work.
fn circuit_node_parent(
    circuit_id: &str,
    human_parent_name: Option<&str>,
) -> Option<CircuitParentNode> {
    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        let circuit_node = nodes.iter().find(|node| {
            node.node_type.as_deref() == Some("circuit")
                && (node.name == circuit_id || node.id.as_deref() == Some(circuit_id))
        })?;
        let parent_index = circuit_node.immediate_parent?;
        let parent = nodes.get(parent_index)?;
        Some(CircuitParentNode {
            name: human_parent_name
                .map(str::to_string)
                .unwrap_or_else(|| parent.name.clone()),
            id: parent.id.clone(),
            tree_index: Some(parent_index),
        })
    })
}

fn circuit_parent_node(
    circuit_id: &str,
    devices: &mut [ShapedDevice],
) -> Option<CircuitParentNode> {
    let canonical_parent = canonical_parent_node(devices);
    let human_name = devices
        .first()
        .map(|device| device.parent_node.trim().to_string())
        .filter(|name| !name.is_empty());
    effective_parent_for_circuit(circuit_id)
        .map(|parent| CircuitParentNode {
            name: parent.name,
            id: parent.id,
            tree_index: None,
        })
        .or(canonical_parent)
        .or_else(|| circuit_node_parent(circuit_id, human_name.as_deref()))
}

fn queue_stats_mode() -> CircuitQueueStatsMode {
    CircuitQueueStatsMode::Live
}

fn effective_circuit_rate_mbps_for_key(circuit_key: &str) -> Option<DownUpOrder<f32>> {
    if circuit_key.is_empty() {
        return None;
    }
    // Rung 1.5 §1.5: E2 is the Rust-native effective-rate source.
    crate::e2_rates::circuit_rate(circuit_key).map(|(down, up)| DownUpOrder {
        down: down as f32,
        up: up as f32,
    })
}

pub fn circuit_by_id_data(id: &str) -> Option<CircuitByIdData> {
    let safe_id = normalize_circuit_id_key(id);
    let catalog = lqos_network_devices::shaped_devices_catalog();
    let mut devices: Vec<ShapedDevice> = catalog.devices_for_circuit_id(&safe_id);

    if devices.is_empty() {
        let catalog = lqos_network_devices::network_devices_catalog();
        if let Some(device) = catalog.dynamic_device_by_circuit_id(&safe_id) {
            devices.push(device.clone());
        }
    }

    if devices.is_empty() {
        None
    } else {
        let parent_node = circuit_parent_node(&safe_id, &mut devices);
        let queue_stats_mode = queue_stats_mode();
        let ethernet_advisory = load_ethernet_advisory(&safe_id, &devices);
        let effective_rate_mbps = effective_circuit_rate_mbps_for_key(&safe_id);
        Some(CircuitByIdData {
            devices,
            parent_node,
            queue_stats_mode,
            ethernet_advisory,
            effective_rate_mbps,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::runtime_config_test_lock;
    use lqos_config::ConfigShapedDevices;
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::{Arc, MutexGuard};

    static EFFECTIVE_RATE_TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    struct EffectiveRateSnapshot {
        previous_nodes: HashMap<String, (f64, f64)>,
        previous_circuits: HashMap<String, (f64, f64)>,
    }

    impl EffectiveRateSnapshot {
        fn replace(rates: HashMap<String, (f64, f64)>) -> Self {
            let (previous_nodes, previous_circuits) =
                crate::e2_rates::replace_for_test(HashMap::new(), rates);
            Self {
                previous_nodes,
                previous_circuits,
            }
        }
    }

    impl Drop for EffectiveRateSnapshot {
        fn drop(&mut self) {
            crate::e2_rates::restore_for_test(
                std::mem::take(&mut self.previous_nodes),
                std::mem::take(&mut self.previous_circuits),
            );
        }
    }

    struct ShapedDevicesSnapshot {
        _runtime_guard: MutexGuard<'static, ()>,
        previous_shaped: Arc<ConfigShapedDevices>,
    }

    impl ShapedDevicesSnapshot {
        fn replace(devices: Vec<ShapedDevice>) -> Self {
            let runtime_guard = runtime_config_test_lock()
                .lock()
                .expect("runtime config test lock should not be poisoned");
            let mut shaped = ConfigShapedDevices::default();
            shaped.replace_with_new_data(devices);
            let previous_shaped = lqos_network_devices::swap_shaped_devices_snapshot(
                "circuit-effective-rate-test",
                Arc::new(shaped),
            );
            Self {
                _runtime_guard: runtime_guard,
                previous_shaped,
            }
        }
    }

    impl Drop for ShapedDevicesSnapshot {
        fn drop(&mut self) {
            lqos_network_devices::swap_shaped_devices_snapshot(
                "circuit-effective-rate-test-restore",
                self.previous_shaped.clone(),
            );
        }
    }

    #[test]
    fn effective_circuit_rate_lookup_uses_normalized_circuit_key() {
        let _guard = EFFECTIVE_RATE_TEST_LOCK.lock();
        let mut rates = HashMap::new();
        rates.insert("circuit-42".to_string(), (115.0, 25.0));
        let _snapshot = EffectiveRateSnapshot::replace(rates);

        let rate = effective_circuit_rate_mbps_for_key("circuit-42");

        assert_eq!(
            rate,
            Some(DownUpOrder {
                down: 115.0,
                up: 25.0
            })
        );
    }

    #[test]
    fn circuit_by_id_data_includes_effective_circuit_rate() {
        let _guard = EFFECTIVE_RATE_TEST_LOCK.lock();
        let _shaped_snapshot = ShapedDevicesSnapshot::replace(vec![ShapedDevice {
            circuit_id: "Circuit-42".to_string(),
            circuit_name: "Circuit 42".to_string(),
            device_id: "device-42".to_string(),
            device_name: "Device 42".to_string(),
            download_max_mbps: 115.0,
            upload_max_mbps: 25.0,
            ..ShapedDevice::default()
        }]);
        let mut rates = HashMap::new();
        rates.insert("circuit-42".to_string(), (100.0, 20.0));
        let _rate_snapshot = EffectiveRateSnapshot::replace(rates);

        let data = circuit_by_id_data(" Circuit-42 ").expect("test circuit should resolve");

        assert_eq!(
            data.effective_rate_mbps,
            Some(DownUpOrder {
                down: 100.0,
                up: 20.0
            })
        );
    }
}

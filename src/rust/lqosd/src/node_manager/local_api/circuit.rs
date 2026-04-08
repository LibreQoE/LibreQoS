use crate::node_manager::local_api::ethernet_caps::ethernet_advisory_for_circuit;
use crate::shaped_devices_tracker::resolve_parent_node;
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use lqos_config::{CircuitEthernetMetadata, ShapedDevice};
use serde::{Deserialize, Serialize};

/// Canonical circuit parent resolved from `network.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CircuitParentNode {
    /// Canonical node name from `network.json`.
    pub name: String,
    /// Optional stable node identifier from `network.json` metadata.
    pub id: Option<String>,
}

/// Circuit-page payload containing shaped devices plus optional Ethernet advisory metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CircuitByIdData {
    /// Shaped-device rows for the requested circuit.
    pub devices: Vec<ShapedDevice>,
    /// Canonical circuit parent resolved from the shaped-device parent and `network.json`.
    pub parent_node: Option<CircuitParentNode>,
    /// Optional negotiated-Ethernet advisory derived from integration metadata.
    pub ethernet_advisory: Option<CircuitEthernetMetadata>,
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
        let Some(resolved) = resolve_parent_node(&device.parent_node) else {
            continue;
        };
        if resolved_parent.is_none() {
            resolved_parent = Some(CircuitParentNode {
                name: resolved.name.clone(),
                id: resolved.id.clone(),
            });
        }
        device.parent_node = resolved.name;
    }

    resolved_parent
}

pub fn circuit_by_id_data(id: &str) -> Option<CircuitByIdData> {
    let safe_id = id.to_lowercase().trim().to_string();
    let reader = SHAPED_DEVICES.load();
    let mut devices: Vec<ShapedDevice> = reader
        .devices
        .iter()
        .filter(|d| d.circuit_id.to_lowercase().trim() == safe_id)
        .cloned()
        .collect();

    if devices.is_empty() {
        None
    } else {
        let parent_node = canonical_parent_node(&mut devices);
        let ethernet_advisory = load_ethernet_advisory(&safe_id, &devices);
        Some(CircuitByIdData {
            devices,
            parent_node,
            ethernet_advisory,
        })
    }
}

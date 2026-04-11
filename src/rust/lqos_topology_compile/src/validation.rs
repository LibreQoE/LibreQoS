use crate::bundle::CompiledTopologyBundle;
use anyhow::{Result, bail};
use lqos_config::{ShapedDevice, TopologyCanonicalIngressKind};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
struct ExportedNode {
    name: String,
    parent_id: Option<String>,
}

fn exported_node_index(network_json: &Value) -> HashMap<String, ExportedNode> {
    let mut nodes = HashMap::new();
    if let Some(map) = network_json.as_object() {
        collect_nodes(map, None, &mut nodes);
    }
    nodes
}

fn collect_nodes(
    map: &Map<String, Value>,
    parent_id: Option<&str>,
    out: &mut HashMap<String, ExportedNode>,
) {
    for (key, value) in map {
        let Some(node) = value.as_object() else {
            continue;
        };
        let Some(node_id) = node
            .get("id")
            .and_then(Value::as_str)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let node_name = node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_string();
        out.insert(
            node_id.to_string(),
            ExportedNode {
                name: node_name,
                parent_id: parent_id.map(ToOwned::to_owned),
            },
        );
        if let Some(children) = node.get("children").and_then(Value::as_object) {
            collect_nodes(children, Some(node_id), out);
        }
    }
}

fn has_cycle(nodes: &HashMap<String, ExportedNode>) -> bool {
    for node_id in nodes.keys() {
        let mut seen = HashSet::new();
        let mut current = Some(node_id.as_str());
        while let Some(candidate) = current {
            if !seen.insert(candidate) {
                return true;
            }
            current = nodes
                .get(candidate)
                .and_then(|node| node.parent_id.as_deref());
        }
    }
    false
}

fn validate_shaped_device(
    device: &ShapedDevice,
    nodes: &HashMap<String, ExportedNode>,
) -> Result<()> {
    if let Some(parent_id) = device.parent_node_id.as_deref().map(str::trim)
        && !parent_id.is_empty()
    {
        let Some(parent) = nodes.get(parent_id) else {
            bail!(
                "Compiled shaped device '{}' references missing parent node '{}'",
                device.circuit_id,
                parent_id
            );
        };
        if !device.parent_node.trim().is_empty() && parent.name != device.parent_node {
            bail!(
                "Compiled shaped device '{}' parent name '{}' does not match exported node '{}'",
                device.circuit_id,
                device.parent_node,
                parent.name
            );
        }
    }
    if let Some(anchor_id) = device.anchor_node_id.as_deref().map(str::trim)
        && !anchor_id.is_empty()
        && !nodes.contains_key(anchor_id)
    {
        bail!(
            "Compiled shaped device '{}' references missing anchor node '{}'",
            device.circuit_id,
            anchor_id
        );
    }
    Ok(())
}

/// Validates the compiled bundle's exported node references.
pub fn validate_compiled_bundle(bundle: &CompiledTopologyBundle) -> Result<()> {
    let nodes = exported_node_index(&bundle.compatibility_network_json);
    if has_cycle(&nodes) {
        bail!("Compiled topology contains a parent cycle");
    }
    let native_integration =
        bundle.canonical.ingress_kind == TopologyCanonicalIngressKind::NativeIntegration;
    for anchor in &bundle.circuit_anchors.anchors {
        if !nodes.contains_key(anchor.anchor_node_id.as_str()) {
            bail!(
                "Compiled circuit anchor '{}' references missing node '{}'",
                anchor.circuit_id,
                anchor.anchor_node_id
            );
        }
    }
    for device in &bundle.shaped_devices.devices {
        validate_shaped_device(device, &nodes)?;
    }
    for node in &bundle.canonical.nodes {
        if !node.node_id.trim().is_empty()
            && !nodes.contains_key(node.node_id.as_str())
            && !bundle
                .editor
                .nodes
                .iter()
                .any(|editor| editor.node_id == node.node_id)
        {
            bail!("Compiled canonical node '{}' is not exported", node.node_id);
        }
    }
    for node in &bundle.editor.nodes {
        if !nodes.contains_key(node.node_id.as_str()) && !native_integration {
            bail!("Compiled editor node '{}' is not exported", node.node_id);
        }
        if let Some(parent_id) = node.current_parent_node_id.as_deref()
            && !parent_id.is_empty()
            && !nodes.contains_key(parent_id)
            && !native_integration
        {
            bail!(
                "Compiled editor node '{}' references missing parent '{}'",
                node.node_id,
                parent_id
            );
        }
    }
    Ok(())
}

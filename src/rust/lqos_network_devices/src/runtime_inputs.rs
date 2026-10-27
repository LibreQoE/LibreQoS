//! Shared helpers for resolving active runtime topology artifacts.
//!
//! This module centralizes how `lqos_network_devices` discovers the currently
//! active shaping inputs published by the topology runtime and converts them
//! into the shaped-device snapshot used by `lqosd`.

use anyhow::Result;
use lqos_config::{
    Config, ConfigShapedDevices, ShapedDevice, TopologyShapingInputsFile,
    TopologyShapingResolutionSource,
};
use std::net::{Ipv4Addr, Ipv6Addr};
use tracing::debug;

fn optional_trimmed_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_ipv4_entry(value: &str) -> Option<(Ipv4Addr, u32)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (ip, cidr) = if let Some((ip, cidr)) = trimmed.split_once('/') {
        (ip.trim(), cidr.trim().parse().ok()?)
    } else {
        (trimmed, 32)
    };
    Some((ip.parse().ok()?, cidr))
}

fn parse_ipv6_entry(value: &str) -> Option<(Ipv6Addr, u32)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (ip, cidr) = if let Some((ip, cidr)) = trimmed.split_once('/') {
        (ip.trim(), cidr.trim().parse().ok()?)
    } else {
        (trimmed, 128)
    };
    Some((ip.parse().ok()?, cidr))
}

fn parse_ipv4_list(values: &[String]) -> Vec<(Ipv4Addr, u32)> {
    values
        .iter()
        .filter_map(|value| parse_ipv4_entry(value))
        .collect()
}

fn parse_ipv6_list(values: &[String]) -> Vec<(Ipv6Addr, u32)> {
    values
        .iter()
        .filter_map(|value| parse_ipv6_entry(value))
        .collect()
}

fn runtime_parent_for_circuit(
    circuit: &lqos_config::TopologyShapingCircuitInput,
) -> (String, Option<String>) {
    if circuit.resolution_source == TopologyShapingResolutionSource::RuntimeFallback {
        return (String::new(), None);
    }

    let parent_node = optional_trimmed_string(&circuit.effective_parent_node_name)
        .or_else(|| circuit.logical_parent_node_name.clone())
        .unwrap_or_default();
    let parent_node_id = optional_trimmed_string(&circuit.effective_parent_node_id)
        .or_else(|| circuit.logical_parent_node_id.clone());

    (parent_node, parent_node_id)
}

/// Loads the currently active runtime shaping inputs when available.
pub(crate) fn load_ready_runtime_shaping_inputs(
    config: &Config,
) -> Result<Option<TopologyShapingInputsFile>> {
    // K1b §4.8 custody check: the published generation content-addresses the
    // effective export, which `lqos_config` cannot reproduce. Gate on the
    // read-only topology verify so a stale or hand-edited artifact is not
    // consumed.
    if !matches!(
        lqos_topology::verify_published_shaping_generation(config),
        Ok(lqos_topology::ShapingGenerationVerification {
            verified: Some(true),
            ..
        })
    ) {
        debug!("Topology custody verify failed; falling back from runtime shaping inputs");
        return Ok(None);
    }
    match lqos_config::load_active_runtime_shaping_inputs(config) {
        Ok(shaping_inputs) => Ok(shaping_inputs),
        Err(err) => {
            debug!(
                "Unable to load active runtime shaping inputs; falling back from runtime shaping inputs: {err}"
            );
            Ok(None)
        }
    }
}

/// Converts topology runtime shaping inputs into a `ConfigShapedDevices`
/// snapshot suitable for the shared in-memory catalog.
pub(crate) fn shaped_devices_from_runtime_inputs(
    shaping_inputs: &TopologyShapingInputsFile,
) -> ConfigShapedDevices {
    let mut devices = Vec::new();
    for circuit in &shaping_inputs.circuits {
        let (parent_node, parent_node_id) = runtime_parent_for_circuit(circuit);
        for device in &circuit.devices {
            devices.push(ShapedDevice {
                circuit_id: circuit.circuit_id.clone(),
                circuit_name: circuit.circuit_name.clone(),
                device_id: device.device_id.clone(),
                device_name: device.device_name.clone(),
                parent_node: parent_node.clone(),
                parent_node_id: parent_node_id.clone(),
                anchor_node_id: circuit.anchor_node_id.clone(),
                mac: device.mac.clone(),
                ipv4: parse_ipv4_list(&device.ipv4),
                ipv6: parse_ipv6_list(&device.ipv6),
                download_min_mbps: circuit.download_min_mbps,
                upload_min_mbps: circuit.upload_min_mbps,
                download_max_mbps: circuit.download_max_mbps,
                upload_max_mbps: circuit.upload_max_mbps,
                comment: if device.comment.trim().is_empty() {
                    circuit.comment.clone()
                } else {
                    device.comment.clone()
                },
                sqm_override: circuit.sqm_override.clone(),
                ..ShapedDevice::default()
            });
        }
    }

    let mut shaped = ConfigShapedDevices::default();
    shaped.replace_with_new_data(devices);
    shaped
}

/// T5 rung 1 / F10 (the zero-CSV identity gate): builds the catalog
/// snapshot from the E2 operator view ALONE — circuit nodes carry
/// their devices INLINE (ruling 2), so circuit identity renders with
/// no ShapedDevices.csv and no shaping-inputs readiness. Names are
/// the E2 display names (T4's dedupe applied; a parent name that
/// matches no node is honest — the tree's divergence report owns the
/// loud-miss).
pub(crate) fn shaped_devices_from_operator_view(
    view: &lqos_config::TopologyOperatorViewFile,
) -> ConfigShapedDevices {
    let mut devices = Vec::new();
    for circuit in view.nodes.iter().filter(|node| node.is_circuit()) {
        // The parent NODE's display name (the name-keyed join
        // convention — T4), resolved via the parent id.
        let parent_node = circuit
            .parent_id
            .as_deref()
            .and_then(|parent_id| view.node_by_id(parent_id))
            .map(|parent| parent.name.clone())
            .or_else(|| circuit.parent_id.clone())
            .unwrap_or_default();
        for device in &circuit.devices {
            devices.push(ShapedDevice {
                circuit_id: circuit.id.clone(),
                circuit_name: circuit.name.clone(),
                device_id: device.device_id.clone(),
                device_name: device.device_name.clone(),
                parent_node: parent_node.clone(),
                parent_node_id: circuit.parent_id.clone(),
                mac: device.mac.clone(),
                ipv4: parse_ipv4_list(&device.ipv4),
                ipv6: parse_ipv6_list(&device.ipv6),
                download_min_mbps: device.download_min_mbps as f32,
                upload_min_mbps: device.upload_min_mbps as f32,
                download_max_mbps: device.download_max_mbps as f32,
                upload_max_mbps: device.upload_max_mbps as f32,
                ..ShapedDevice::default()
            });
        }
    }

    let mut shaped = ConfigShapedDevices::default();
    shaped.replace_with_new_data(devices);
    shaped
}

/// Loads the E2 operator view when it exists (the catalog fallback
/// arm — F10).
pub(crate) fn load_operator_view(
    config: &Config,
) -> Result<Option<lqos_config::TopologyOperatorViewFile>> {
    let path = lqos_config::topology_operator_view_path(config);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    let view = serde_json::from_str(&raw)?;
    Ok(Some(view))
}

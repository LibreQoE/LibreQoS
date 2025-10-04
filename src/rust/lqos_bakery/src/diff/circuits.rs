use crate::BakeryCommands;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

pub(crate) enum CircuitDiffResult<'a> {
    NoChange,
    CircuitsChanged {
        newly_added: Vec<&'a Arc<BakeryCommands>>,
        removed_circuits: Vec<i64>,
        /// Circuits whose TC-relevant parameters changed (requires rebuild)
        updated_tc: Vec<&'a Arc<BakeryCommands>>,
        /// Circuits whose IP lists changed only (mapping diff only)
        updated_ip_only: Vec<&'a Arc<BakeryCommands>>,
    },
}

pub(crate) fn diff_circuits<'a>(
    batch: &'a [Arc<BakeryCommands>],
    old_circuits: &HashMap<i64, Arc<BakeryCommands>>,
) -> CircuitDiffResult<'a> {
    let new_circuits: HashMap<i64, &Arc<BakeryCommands>> = batch
        .iter()
        .filter_map(|cmd| {
            if let BakeryCommands::AddCircuit { circuit_hash, .. } = cmd.as_ref() {
                Some((*circuit_hash, cmd))
            } else {
                None
            }
        })
        .collect();

    // Find any circuits that have been added to `new_circuits` and not in `old_circuits`
    let mut newly_added = Vec::new();
    for (circuit_hash, new_cmd) in &new_circuits {
        if !old_circuits.contains_key(circuit_hash) {
            newly_added.push(*new_cmd);
        }
    }

    // Find any circuits that have been removed from `new_circuits`, but were in `old_circuits`
    let mut removed_circuits = Vec::new();
    for circuit_hash in old_circuits.keys() {
        if !new_circuits.contains_key(circuit_hash) {
            removed_circuits.push(*circuit_hash);
        }
    }

    // Find any circuits that have changed in `new_circuits` compared to `old_circuits`
    let mut updated_tc = Vec::new();
    let mut updated_ip_only = Vec::new();
    for (circuit_hash, old_cmd) in old_circuits {
        if let Some(new_cmd) = new_circuits.get(circuit_hash) {
            if has_circuit_changed_tc_params(old_cmd.as_ref(), new_cmd.as_ref()) {
                updated_tc.push(*new_cmd);
            } else if has_circuit_ip_only_changed(old_cmd.as_ref(), new_cmd.as_ref()) {
                updated_ip_only.push(*new_cmd);
            }
        }
    }

    // If there are any changes, return them
    if !newly_added.is_empty()
        || !removed_circuits.is_empty()
        || !updated_tc.is_empty()
        || !updated_ip_only.is_empty()
    {
        CircuitDiffResult::CircuitsChanged { newly_added, removed_circuits, updated_tc, updated_ip_only }
    } else {
        CircuitDiffResult::NoChange
    }
}

fn extract<'a>(
    circuit: &'a BakeryCommands,
) -> Option<(
    i64,
    &'a lqos_bus::TcHandle,
    &'a lqos_bus::TcHandle,
    u16,
    f32,
    f32,
    f32,
    f32,
    u16,
    u16,
    u32,
    u32,
    &'a String,
)> {
    if let BakeryCommands::AddCircuit {
        circuit_hash,
        parent_class_id,
        up_parent_class_id,
        class_minor,
        download_bandwidth_min,
        upload_bandwidth_min,
        download_bandwidth_max,
        upload_bandwidth_max,
        class_major,
        up_class_major,
        down_cpu,
        up_cpu,
        ip_addresses,
    } = circuit
    {
        Some((
            *circuit_hash,
            parent_class_id,
            up_parent_class_id,
            *class_minor,
            *download_bandwidth_min,
            *upload_bandwidth_min,
            *download_bandwidth_max,
            *upload_bandwidth_max,
            *class_major,
            *up_class_major,
            *down_cpu,
            *up_cpu,
            ip_addresses,
        ))
    } else {
        None
    }
}

/// Compare all TC-relevant parameters (everything except IP lists).
pub(crate) fn has_circuit_changed_tc_params(a: &BakeryCommands, b: &BakeryCommands) -> bool {
    let Some((hash_a, parent_class_id, up_parent_class_id, class_minor, download_bandwidth_min, upload_bandwidth_min, download_bandwidth_max, upload_bandwidth_max, class_major, up_class_major, down_cpu, up_cpu, _)) = extract(a) else { return false };
    let Some((hash_b, other_parent_class_id, other_up_parent_class_id, other_class_minor, other_download_bandwidth_min, other_upload_bandwidth_min, other_download_bandwidth_max, other_upload_bandwidth_max, other_class_major, other_up_class_major, other_down_cpu, other_up_cpu, _)) = extract(b) else { return false };
    if hash_a != hash_b {
        warn!("Circuit hashes do not match: {} != {}", hash_a, hash_b);
        return false;
    }
    parent_class_id != other_parent_class_id
        || up_parent_class_id != other_up_parent_class_id
        || class_minor != other_class_minor
        || download_bandwidth_min != other_download_bandwidth_min
        || upload_bandwidth_min != other_upload_bandwidth_min
        || download_bandwidth_max != other_download_bandwidth_max
        || upload_bandwidth_max != other_upload_bandwidth_max
        || class_major != other_class_major
        || up_class_major != other_up_class_major
        || down_cpu != other_down_cpu
        || up_cpu != other_up_cpu
}

/// Returns true if ONLY the IP list changed.
pub(crate) fn has_circuit_ip_only_changed(a: &BakeryCommands, b: &BakeryCommands) -> bool {
    let Some((hash_a, _, _, _, _, _, _, _, _, _, _, _, ip_addresses)) = extract(a) else { return false };
    let Some((hash_b, _, _, _, _, _, _, _, _, _, _, _, other_ip_addresses)) = extract(b) else { return false };
    if hash_a != hash_b {
        warn!("Circuit hashes do not match: {} != {}", hash_a, hash_b);
        return false;
    }
    if has_circuit_changed_tc_params(a, b) {
        return false;
    }
    ip_addresses != other_ip_addresses
}

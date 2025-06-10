use std::collections::HashMap;
use tracing::warn;
use crate::BakeryCommands;

pub(crate) enum CircuitDiffResult {
    NoChange,
    CircuitsChanged {
        newly_added: Vec<BakeryCommands>,
        removed_circuits: Vec<BakeryCommands>,
        updated_circuits: Vec<BakeryCommands>,
    }
}

pub(crate) fn diff_circuits(
    batch: &[BakeryCommands],
    old_circuits: &HashMap<i64, BakeryCommands>,
) -> CircuitDiffResult {
    let new_circuits: HashMap<i64, &BakeryCommands> = batch
        .iter()
        .filter_map(|cmd| {
            if let BakeryCommands::AddCircuit{ circuit_hash, .. } = cmd {
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
            newly_added.push((*new_cmd).clone());
        }
    }

    // Find any circuits that have been removed from `new_circuits`, but were in `old_circuits`
    let mut removed_circuits = Vec::new();
    for (circuit_hash, old_cmd) in old_circuits {
        if !new_circuits.contains_key(circuit_hash) {
            removed_circuits.push((*old_cmd).clone());
        }
    }

    // Find any circuits that have changed in `new_circuits` compared to `old_circuits`
    let mut updated_circuits = Vec::new();
    for (circuit_hash, old_cmd) in old_circuits {
        if let Some(new_cmd) = new_circuits.get(circuit_hash) {
            if has_circuit_changed(old_cmd, new_cmd) {
                updated_circuits.push((*new_cmd).clone());
            }
        }
    }

    // If there are any changes, return them
    if !newly_added.is_empty() || !removed_circuits.is_empty() || !updated_circuits.is_empty() {
        CircuitDiffResult::CircuitsChanged {
            newly_added,
            removed_circuits,
            updated_circuits,
        }
    } else {
        CircuitDiffResult::NoChange
    }
}

fn has_circuit_changed(
    circuit_a: &BakeryCommands,
    circuit_b: &BakeryCommands,
) -> bool {
    let BakeryCommands::AddCircuit { circuit_hash, parent_class_id, up_parent_class_id, class_minor, download_bandwidth_min, upload_bandwidth_min, download_bandwidth_max, upload_bandwidth_max, class_major, up_class_major, ip_addresses } = circuit_a else {
        warn!("circuit_changed called on non-circuit command: {:?}", circuit_a);
        return false; // Not a circuit command
    };
    let BakeryCommands::AddCircuit { circuit_hash: other_circuit_hash, parent_class_id: other_parent_class_id, up_parent_class_id: other_up_parent_class_id, class_minor: other_class_minor, download_bandwidth_min: other_download_bandwidth_min, upload_bandwidth_min: other_upload_bandwidth_min, download_bandwidth_max: other_download_bandwidth_max, upload_bandwidth_max: other_upload_bandwidth_max, class_major: other_class_major, up_class_major: other_up_class_major, ip_addresses: other_ip_addresses } = circuit_b else {
        warn!("circuit_changed called on non-circuit command: {:?}", circuit_b);
        return false; // Not a circuit command
    };
    if circuit_hash != other_circuit_hash {
        // This should never happen
        warn!("Circuit hashes do not match: {} != {}", circuit_hash, other_circuit_hash);
        return false; // Different circuit hashes
    }

    parent_class_id != other_parent_class_id ||
    up_parent_class_id != other_up_parent_class_id ||
    class_minor != other_class_minor ||
    download_bandwidth_min != other_download_bandwidth_min ||
    upload_bandwidth_min != other_upload_bandwidth_min ||
    download_bandwidth_max != other_download_bandwidth_max ||
    upload_bandwidth_max != other_upload_bandwidth_max ||
    class_major != other_class_major ||
    up_class_major != other_up_class_major ||
    ip_addresses != other_ip_addresses
}
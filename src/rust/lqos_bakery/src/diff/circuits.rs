use crate::BakeryCommands;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

pub(crate) struct CircuitDiffCategories<'a> {
    pub newly_added: Vec<&'a Arc<BakeryCommands>>,
    pub removed_circuits: Vec<i64>,
    pub speed_changed: Vec<&'a Arc<BakeryCommands>>,
    pub ip_changed: Vec<&'a Arc<BakeryCommands>>,
    pub structural_changed: Vec<&'a Arc<BakeryCommands>>,
}

pub(crate) enum CircuitDiffResult<'a> {
    NoChange,
    Categorized(CircuitDiffCategories<'a>),
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

    let mut newly_added = Vec::new();
    let mut removed_circuits = Vec::new();
    let mut speed_changed = Vec::new();
    let mut ip_changed = Vec::new();
    let mut structural_changed = Vec::new();

    for (circuit_hash, new_cmd) in &new_circuits {
        if !old_circuits.contains_key(circuit_hash) {
            newly_added.push(*new_cmd);
        }
    }

    for circuit_hash in old_circuits.keys() {
        if !new_circuits.contains_key(circuit_hash) {
            removed_circuits.push(*circuit_hash);
        }
    }

    for (circuit_hash, old_cmd) in old_circuits {
        if let Some(new_cmd) = new_circuits.get(circuit_hash) {
            match classify_circuit_change(old_cmd.as_ref(), new_cmd.as_ref()) {
                CircuitChange::None => {}
                CircuitChange::Structural => structural_changed.push(*new_cmd),
                CircuitChange::Speed => speed_changed.push(*new_cmd),
                CircuitChange::Ip => ip_changed.push(*new_cmd),
                CircuitChange::SpeedAndIp => {
                    speed_changed.push(*new_cmd);
                    ip_changed.push(*new_cmd);
                }
            }
        }
    }

    if !newly_added.is_empty()
        || !removed_circuits.is_empty()
        || !speed_changed.is_empty()
        || !ip_changed.is_empty()
        || !structural_changed.is_empty()
    {
        CircuitDiffResult::Categorized(CircuitDiffCategories {
            newly_added,
            removed_circuits,
            speed_changed,
            ip_changed,
            structural_changed,
        })
    } else {
        CircuitDiffResult::NoChange
    }
}

enum CircuitChange {
    None,
    Structural,
    Speed,
    Ip,
    SpeedAndIp,
}

fn classify_circuit_change(a: &BakeryCommands, b: &BakeryCommands) -> CircuitChange {
    let BakeryCommands::AddCircuit {
        parent_class_id,
        up_parent_class_id,
        class_minor,
        download_bandwidth_min,
        upload_bandwidth_min,
        download_bandwidth_max,
        upload_bandwidth_max,
        class_major,
        up_class_major,
        ip_addresses,
        sqm_override,
        ..
    } = a
    else {
        warn!(
            "classify_circuit_change called on non-circuit command: {:?}",
            a
        );
        return CircuitChange::None;
    };
    let BakeryCommands::AddCircuit {
        parent_class_id: other_parent_class_id,
        up_parent_class_id: other_up_parent_class_id,
        class_minor: other_class_minor,
        download_bandwidth_min: other_download_bandwidth_min,
        upload_bandwidth_min: other_upload_bandwidth_min,
        download_bandwidth_max: other_download_bandwidth_max,
        upload_bandwidth_max: other_upload_bandwidth_max,
        class_major: other_class_major,
        up_class_major: other_up_class_major,
        ip_addresses: other_ip_addresses,
        sqm_override: other_sqm_override,
        ..
    } = b
    else {
        warn!(
            "classify_circuit_change called on non-circuit command: {:?}",
            b
        );
        return CircuitChange::None;
    };
    // Structural change?
    let structural = parent_class_id != other_parent_class_id
        || up_parent_class_id != other_up_parent_class_id
        || class_minor != other_class_minor
        || class_major != other_class_major
        || up_class_major != other_up_class_major;

    if structural {
        return CircuitChange::Structural;
    }

    let speed = download_bandwidth_min != other_download_bandwidth_min
        || upload_bandwidth_min != other_upload_bandwidth_min
        || download_bandwidth_max != other_download_bandwidth_max
        || upload_bandwidth_max != other_upload_bandwidth_max
        || sqm_override != other_sqm_override; // treat SQM override changes as speed-level changes

    let ip = ip_addresses != other_ip_addresses;

    match (speed, ip) {
        (false, false) => CircuitChange::None,
        (true, false) => CircuitChange::Speed,
        (false, true) => CircuitChange::Ip,
        (true, true) => CircuitChange::SpeedAndIp,
    }
}

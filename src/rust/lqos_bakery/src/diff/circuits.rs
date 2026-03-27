use crate::BakeryCommands;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

pub(crate) struct CircuitDiffCategories<'a> {
    pub newly_added: Vec<&'a Arc<BakeryCommands>>,
    pub removed_circuits: Vec<i64>,
    pub speed_changed: Vec<&'a Arc<BakeryCommands>>,
    pub ip_changed: Vec<&'a Arc<BakeryCommands>>,
    pub migrated: Vec<&'a Arc<BakeryCommands>>,
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
    let mut migrated = Vec::new();
    let structural_changed = Vec::new();

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
                CircuitChange::Migration => migrated.push(*new_cmd),
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
        || !migrated.is_empty()
        || !structural_changed.is_empty()
    {
        CircuitDiffResult::Categorized(CircuitDiffCategories {
            newly_added,
            removed_circuits,
            speed_changed,
            ip_changed,
            migrated,
            structural_changed,
        })
    } else {
        CircuitDiffResult::NoChange
    }
}

enum CircuitChange {
    None,
    Migration,
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
        debug!(
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
        debug!(
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
        return CircuitChange::Migration;
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

#[cfg(test)]
mod tests {
    use super::{CircuitDiffResult, diff_circuits};
    use crate::BakeryCommands;
    use lqos_bus::TcHandle;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[allow(clippy::too_many_arguments)]
    fn add_circuit(
        circuit_hash: i64,
        parent_class_id: u32,
        up_parent_class_id: u32,
        class_minor: u16,
        class_major: u16,
        up_class_major: u16,
        dl_min: f32,
        ul_min: f32,
        dl_max: f32,
        ul_max: f32,
    ) -> Arc<BakeryCommands> {
        Arc::new(BakeryCommands::AddCircuit {
            circuit_hash,
            parent_class_id: TcHandle::from_u32(parent_class_id),
            up_parent_class_id: TcHandle::from_u32(up_parent_class_id),
            class_minor,
            download_bandwidth_min: dl_min,
            upload_bandwidth_min: ul_min,
            download_bandwidth_max: dl_max,
            upload_bandwidth_max: ul_max,
            class_major,
            up_class_major,
            down_qdisc_handle: Some(0x9000),
            up_qdisc_handle: Some(0x9001),
            ip_addresses: "192.0.2.1/32".to_string(),
            sqm_override: None,
        })
    }

    #[test]
    fn parent_move_is_categorized_as_migration() {
        let old = add_circuit(1, 0x10020, 0x20020, 0x21, 0x1, 0x2, 1.0, 1.0, 10.0, 10.0);
        let new = add_circuit(1, 0x10034, 0x20034, 0x35, 0x1, 0x2, 1.0, 1.0, 10.0, 10.0);
        let old_circuits = HashMap::from([(1, Arc::clone(&old))]);
        let batch = vec![new];

        let CircuitDiffResult::Categorized(categories) = diff_circuits(&batch, &old_circuits)
        else {
            panic!("expected categorized diff");
        };

        assert_eq!(categories.migrated.len(), 1);
        assert!(categories.structural_changed.is_empty());
        assert!(categories.speed_changed.is_empty());
    }

    #[test]
    fn rate_change_stays_speed_only() {
        let old = add_circuit(1, 0x10020, 0x20020, 0x21, 0x1, 0x2, 1.0, 1.0, 10.0, 10.0);
        let new = add_circuit(1, 0x10020, 0x20020, 0x21, 0x1, 0x2, 2.0, 1.5, 20.0, 15.0);
        let old_circuits = HashMap::from([(1, Arc::clone(&old))]);
        let batch = vec![new];

        let CircuitDiffResult::Categorized(categories) = diff_circuits(&batch, &old_circuits)
        else {
            panic!("expected categorized diff");
        };

        assert_eq!(categories.speed_changed.len(), 1);
        assert!(categories.migrated.is_empty());
        assert!(categories.structural_changed.is_empty());
    }

    #[test]
    fn ip_change_stays_ip_only() {
        let old = add_circuit(1, 0x10020, 0x20020, 0x21, 0x1, 0x2, 1.0, 1.0, 10.0, 10.0);
        let BakeryCommands::AddCircuit {
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
            down_qdisc_handle,
            up_qdisc_handle,
            sqm_override,
            ..
        } = old.as_ref()
        else {
            panic!("expected add circuit");
        };
        let new = Arc::new(BakeryCommands::AddCircuit {
            circuit_hash: *circuit_hash,
            parent_class_id: *parent_class_id,
            up_parent_class_id: *up_parent_class_id,
            class_minor: *class_minor,
            download_bandwidth_min: *download_bandwidth_min,
            upload_bandwidth_min: *upload_bandwidth_min,
            download_bandwidth_max: *download_bandwidth_max,
            upload_bandwidth_max: *upload_bandwidth_max,
            class_major: *class_major,
            up_class_major: *up_class_major,
            down_qdisc_handle: *down_qdisc_handle,
            up_qdisc_handle: *up_qdisc_handle,
            ip_addresses: "198.51.100.10/32".to_string(),
            sqm_override: sqm_override.clone(),
        });
        let old_circuits = HashMap::from([(1, Arc::clone(&old))]);
        let batch = vec![new];

        let CircuitDiffResult::Categorized(categories) = diff_circuits(&batch, &old_circuits)
        else {
            panic!("expected categorized diff");
        };

        assert_eq!(categories.ip_changed.len(), 1);
        assert!(categories.speed_changed.is_empty());
        assert!(categories.migrated.is_empty());
        assert!(categories.structural_changed.is_empty());
    }
}

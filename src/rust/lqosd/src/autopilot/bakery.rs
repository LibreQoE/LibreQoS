//! Bakery live-update helpers for Autopilot.
//!
//! This module implements live SQM switching via Bakery commands.

use crate::autopilot::AutopilotError;
use lqos_bakery::BakeryCommands;
use lqos_config::ShapedDevice;
use lqos_queue_tracker::QUEUE_STRUCTURE;
use lqos_utils::hash_to_i64;

/// Applies a per-circuit SQM override token live via Bakery.
///
/// This function has side effects: it reads the in-memory queue structure snapshot and sends a
/// `BakeryCommands::AddCircuit` update to the Bakery thread.
pub(crate) fn apply_circuit_sqm_override_live(
    circuit_id: &str,
    devices: &[ShapedDevice],
    sqm_override: &str,
) -> Result<(), AutopilotError> {
    let Some(sender) = lqos_bakery::BAKERY_SENDER.get() else {
        return Err(AutopilotError::BakeryNotReady);
    };

    let snapshot = QUEUE_STRUCTURE.load();
    let Some(queues) = snapshot.maybe_queues.as_ref() else {
        return Err(AutopilotError::QueueStructureUnavailable {
            details: "queueingStructure.json not loaded".to_string(),
        });
    };

    // Find the circuit node in the queue structure.
    let mut stack = Vec::new();
    for q in queues.iter() {
        stack.push(q);
    }

    let mut found = None;
    while let Some(node) = stack.pop() {
        if node.circuit_id.as_deref() == Some(circuit_id) && node.device_id.is_none() {
            found = Some(node);
            break;
        }

        for c in node.children.iter() {
            stack.push(c);
        }
        for c in node.circuits.iter() {
            stack.push(c);
        }
        for d in node.devices.iter() {
            stack.push(d);
        }
    }

    let Some(node) = found else {
        return Err(AutopilotError::CircuitNotFound {
            circuit_id: circuit_id.to_string(),
        });
    };

    let class_minor = u16::try_from(node.class_minor).map_err(|_| {
        AutopilotError::InvalidClassId {
            details: format!("class_minor too large: {}", node.class_minor),
        }
    })?;
    let class_major = u16::try_from(node.class_major).map_err(|_| {
        AutopilotError::InvalidClassId {
            details: format!("class_major too large: {}", node.class_major),
        }
    })?;
    let up_class_major = u16::try_from(node.up_class_major).map_err(|_| {
        AutopilotError::InvalidClassId {
            details: format!("up_class_major too large: {}", node.up_class_major),
        }
    })?;

    let circuit_hash = hash_to_i64(circuit_id);
    let ip_addresses = ip_list(devices);

    sender
        .send(BakeryCommands::AddCircuit {
            circuit_hash,
            parent_class_id: node.parent_class_id,
            up_parent_class_id: node.up_parent_class_id,
            class_minor,
            download_bandwidth_min: node.download_bandwidth_mbps_min as f32,
            upload_bandwidth_min: node.upload_bandwidth_mbps_min as f32,
            download_bandwidth_max: node.download_bandwidth_mbps as f32,
            upload_bandwidth_max: node.upload_bandwidth_mbps as f32,
            class_major,
            up_class_major,
            ip_addresses,
            sqm_override: Some(sqm_override.to_string()),
        })
        .map_err(|e| AutopilotError::BakerySend {
            details: e.to_string(),
        })?;

    Ok(())
}

/// Builds a comma-separated IP list string from a circuit's shaped devices.
///
/// This function is pure: it has no side effects.
fn ip_list(devices: &[ShapedDevice]) -> String {
    let mut ips = Vec::new();
    for dev in devices {
        for (ip, prefix) in dev.ipv4.iter() {
            ips.push(format!("{ip}/{prefix}"));
        }
        for (ip, prefix) in dev.ipv6.iter() {
            ips.push(format!("{ip}/{prefix}"));
        }
    }
    ips.sort();
    ips.dedup();
    ips.join(",")
}

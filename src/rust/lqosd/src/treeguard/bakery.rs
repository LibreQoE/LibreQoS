//! Bakery live-update helpers for TreeGuard.
//!
//! This module implements live SQM switching and runtime node virtualization via Bakery commands.

use crate::treeguard::TreeguardError;
use crossbeam_channel::Sender;
use lqos_bakery::BakeryCommands;
use lqos_config::ShapedDevice;
use lqos_queue_tracker::{QUEUE_STRUCTURE, QueueNode};
use lqos_utils::hash_to_i64;
use std::sync::mpsc;
use std::time::Duration;

/// Applies a per-circuit SQM override token live via Bakery.
///
/// This function has side effects: it reads the in-memory queue structure snapshot and sends a
/// `BakeryCommands::AddCircuit` update to the Bakery thread.
pub(crate) fn apply_circuit_sqm_override_live(
    circuit_id: &str,
    devices: &[ShapedDevice],
    sqm_override: &str,
) -> Result<(), TreeguardError> {
    let Some(sender) = lqos_bakery::BAKERY_SENDER.get() else {
        return Err(TreeguardError::BakeryNotReady);
    };

    let snapshot = QUEUE_STRUCTURE.load();
    let Some(queues) = snapshot.maybe_queues.as_ref() else {
        return Err(TreeguardError::QueueStructureUnavailable {
            details: "queueingStructure.json not loaded".to_string(),
        });
    };

    apply_circuit_sqm_override_live_with_sender_and_snapshot(
        circuit_id,
        devices,
        sqm_override,
        sender,
        queues,
    )
}

#[doc(hidden)]
pub(crate) fn apply_circuit_sqm_override_live_with_sender_and_snapshot(
    circuit_id: &str,
    devices: &[ShapedDevice],
    sqm_override: &str,
    sender: &Sender<BakeryCommands>,
    queues: &[QueueNode],
) -> Result<(), TreeguardError> {
    let node = find_circuit_queue_node(queues, circuit_id)?;
    send_live_sqm_override(node, devices, sqm_override, sender)
}

fn find_circuit_queue_node<'a>(
    queues: &'a [QueueNode],
    circuit_id: &str,
) -> Result<&'a QueueNode, TreeguardError> {
    // Find the circuit node in the queue structure.
    let mut stack = Vec::new();
    stack.extend(queues.iter());

    while let Some(node) = stack.pop() {
        if node.circuit_id.as_deref() == Some(circuit_id) && node.device_id.is_none() {
            return Ok(node);
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

    Err(TreeguardError::CircuitNotFound {
        circuit_id: circuit_id.to_string(),
    })
}

fn send_live_sqm_override(
    node: &QueueNode,
    devices: &[ShapedDevice],
    sqm_override: &str,
    sender: &Sender<BakeryCommands>,
) -> Result<(), TreeguardError> {
    let circuit_id = node
        .circuit_id
        .as_deref()
        .ok_or_else(|| TreeguardError::CircuitNotFound {
            circuit_id: "<missing>".to_string(),
        })?;
    let class_minor =
        u16::try_from(node.class_minor).map_err(|_| TreeguardError::InvalidClassId {
            details: format!("class_minor too large: {}", node.class_minor),
        })?;
    let class_major =
        u16::try_from(node.class_major).map_err(|_| TreeguardError::InvalidClassId {
            details: format!("class_major too large: {}", node.class_major),
        })?;
    let up_class_major =
        u16::try_from(node.up_class_major).map_err(|_| TreeguardError::InvalidClassId {
            details: format!("up_class_major too large: {}", node.up_class_major),
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
            down_qdisc_handle: None,
            up_qdisc_handle: None,
            ip_addresses,
            sqm_override: Some(sqm_override.to_string()),
        })
        .map_err(|e| TreeguardError::BakerySend {
            details: e.to_string(),
        })?;

    Ok(())
}

/// Requests Bakery to runtime-virtualize or restore a single node without a full reload.
///
/// This function has side effects: it sends a synchronous command to the Bakery thread and waits
/// briefly for an immediate success/failure result.
pub(crate) fn apply_node_virtualization_live(
    node_name: &str,
    virtualized: bool,
) -> Result<(), TreeguardError> {
    let Some(sender) = lqos_bakery::BAKERY_SENDER.get() else {
        return Err(TreeguardError::BakeryNotReady);
    };

    let site_hash = hash_to_i64(node_name);
    let (reply_tx, reply_rx) = mpsc::channel();

    sender
        .send(BakeryCommands::TreeGuardSetNodeVirtual {
            site_hash,
            virtualized,
            reply: Some(reply_tx),
        })
        .map_err(|e| TreeguardError::BakerySend {
            details: e.to_string(),
        })?;

    match reply_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(details)) => Err(TreeguardError::BakeryVirtualization { details }),
        Err(e) => Err(TreeguardError::BakeryVirtualization {
            details: format!("timed out waiting for Bakery runtime virtualization result: {e}"),
        }),
    }
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

#[cfg(test)]
mod tests {
    use super::apply_circuit_sqm_override_live_with_sender_and_snapshot;
    use crossbeam_channel::bounded;
    use lqos_bakery::BakeryCommands;
    use lqos_bus::TcHandle;
    use lqos_config::ShapedDevice;
    use lqos_queue_tracker::QueueNode;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn live_override_builds_bakery_update_from_snapshot() {
        let (tx, rx) = bounded(1);
        let queues = vec![QueueNode {
            circuit_id: Some("circuit-1".to_string()),
            class_minor: 0x14af,
            class_major: 0x0003,
            up_class_major: 0x0043,
            parent_class_id: TcHandle::from_string("3:20").expect("valid down parent"),
            up_parent_class_id: TcHandle::from_string("43:20").expect("valid up parent"),
            download_bandwidth_mbps_min: 50,
            upload_bandwidth_mbps_min: 10,
            download_bandwidth_mbps: 200,
            upload_bandwidth_mbps: 50,
            ..QueueNode::default()
        }];
        let devices = vec![ShapedDevice {
            circuit_id: "circuit-1".to_string(),
            device_id: "device-1".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 0, 2, 10), 32)],
            ipv6: vec![(Ipv6Addr::LOCALHOST, 128)],
            ..ShapedDevice::default()
        }];

        apply_circuit_sqm_override_live_with_sender_and_snapshot(
            "circuit-1",
            &devices,
            "cake/fq_codel",
            &tx,
            &queues,
        )
        .expect("live override should send bakery update");

        let command = rx.try_recv().expect("bakery command should be sent");
        let BakeryCommands::AddCircuit {
            circuit_hash,
            parent_class_id,
            up_parent_class_id,
            class_minor,
            class_major,
            up_class_major,
            ip_addresses,
            sqm_override,
            down_qdisc_handle,
            up_qdisc_handle,
            ..
        } = command
        else {
            panic!("expected AddCircuit");
        };

        assert_eq!(circuit_hash, lqos_utils::hash_to_i64("circuit-1"));
        assert_eq!(
            parent_class_id,
            TcHandle::from_string("3:20").expect("valid down parent")
        );
        assert_eq!(
            up_parent_class_id,
            TcHandle::from_string("43:20").expect("valid up parent")
        );
        assert_eq!(class_minor, 0x14af);
        assert_eq!(class_major, 0x0003);
        assert_eq!(up_class_major, 0x0043);
        assert_eq!(ip_addresses, "192.0.2.10/32,::1/128");
        assert_eq!(sqm_override, Some("cake/fq_codel".to_string()));
        assert_eq!(down_qdisc_handle, None);
        assert_eq!(up_qdisc_handle, None);
    }
}

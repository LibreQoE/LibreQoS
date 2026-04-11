use lqos_bus::BusResponse;
use lqos_config::ShapedDevice;
use std::sync::mpsc;
use std::time::Duration;
use tracing::{error, warn};

fn dynamic_circuits_enabled() -> bool {
    let Ok(config) = lqos_config::load_config() else {
        return false;
    };
    config
        .dynamic_circuits
        .as_ref()
        .is_some_and(|cfg| cfg.enabled)
}

pub(crate) fn create_dynamic_circuit(shaped_device: ShapedDevice) -> BusResponse {
    if !dynamic_circuits_enabled() {
        return BusResponse::Ack;
    }

    if shaped_device.circuit_id.trim().is_empty() {
        return BusResponse::Fail("dynamic circuit requires circuit_id".to_string());
    }
    if shaped_device.device_id.trim().is_empty() {
        return BusResponse::Fail("dynamic circuit requires device_id".to_string());
    }
    if shaped_device.parent_node.trim().is_empty() {
        return BusResponse::Fail("dynamic circuit requires parent_node".to_string());
    }

    if let Err(err) = lqos_network_devices::upsert_dynamic_circuit(shaped_device.clone()) {
        error!("Dynamic circuit persist failed: {err:?}");
        return BusResponse::Fail(format!("dynamic circuit persist failed: {err}"));
    }

    if let Err(err) = upsert_bakery_overlay(shaped_device.clone()) {
        warn!("Dynamic circuit bakery apply failed: {err}");
        // Best-effort rollback: keep network_devices and bakery in sync.
        let _ = lqos_network_devices::remove_dynamic_circuit(&shaped_device.circuit_id);
        return BusResponse::Fail(err);
    }

    BusResponse::Ack
}

pub(crate) fn remove_dynamic_circuit(circuit_id: &str) -> BusResponse {
    if !dynamic_circuits_enabled() {
        return BusResponse::Ack;
    }

    if circuit_id.trim().is_empty() {
        return BusResponse::Fail("dynamic circuit removal requires circuit_id".to_string());
    }

    if let Err(err) = remove_bakery_overlay(circuit_id) {
        warn!("Dynamic circuit bakery removal failed: {err}");
        return BusResponse::Fail(err);
    }

    if let Err(err) = lqos_network_devices::remove_dynamic_circuit(circuit_id) {
        error!("Dynamic circuit removal persist failed: {err:?}");
        return BusResponse::Fail(format!("dynamic circuit removal persist failed: {err}"));
    }

    BusResponse::Ack
}

fn upsert_bakery_overlay(shaped_device: ShapedDevice) -> Result<(), String> {
    let Some(sender) = lqos_bakery::BAKERY_SENDER.get() else {
        return Err("Bakery not initialized".to_string());
    };

    let (tx, rx) = mpsc::channel::<Result<Option<lqos_bus::TcHandle>, String>>();
    sender
        .send(lqos_bakery::BakeryCommands::UpsertDynamicCircuitOverlay {
            shaped_device: Box::new(shaped_device),
            reply: Some(tx),
        })
        .map_err(|e| format!("send to bakery failed: {e}"))?;

    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|e| format!("bakery reply timeout: {e}"))?
        .map(|_| ())
}

fn remove_bakery_overlay(circuit_id: &str) -> Result<(), String> {
    let Some(sender) = lqos_bakery::BAKERY_SENDER.get() else {
        return Err("Bakery not initialized".to_string());
    };

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    sender
        .send(lqos_bakery::BakeryCommands::RemoveDynamicCircuitOverlay {
            circuit_id: circuit_id.to_string(),
            reply: Some(tx),
        })
        .map_err(|e| format!("send to bakery failed: {e}"))?;

    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|e| format!("bakery reply timeout: {e}"))?
}

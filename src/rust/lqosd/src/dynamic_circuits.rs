use lqos_bus::BusResponse;
use lqos_config::ShapedDevice;
use tracing::error;

fn dynamic_circuits_enabled() -> bool {
    let Ok(config) = lqos_config::load_config() else {
        return false;
    };
    config
        .dynamic_circuits
        .as_ref()
        .is_some_and(|cfg| cfg.enabled)
}

/// The dynamic-circuit write path: every create/remove is a store write.
///
/// The topology fold turns the store into shaping rows, and the store's publish
/// trigger carries that fold to the bridge, so this module never talks to the
/// bakery or the bridge itself.
///
/// Dynamic-ranges: a RADIUS Start for an IP that the promotion scanner already
/// promoted must UPGRADE that row in place, not mint a second row with a
/// session-derived id (which would emit a duplicate /32 Map). Every dynamic
/// circuit write therefore goes through the replace-by-IP RMW.
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

    let circuit_id = shaped_device.circuit_id.clone();
    if let Err(err) = lqos_network_devices::replace_dynamic_circuit_by_ip(shaped_device) {
        error!("Dynamic circuit persist failed for circuit {circuit_id}: {err:?}");
        return BusResponse::Fail(format!("dynamic circuit persist failed: {err}"));
    }

    BusResponse::Ack
}

/// K1 §1.5: removal is a store write; the publish trigger + fold emit the
/// bridge Remove.
pub(crate) fn remove_dynamic_circuit(circuit_id: &str) -> BusResponse {
    if !dynamic_circuits_enabled() {
        return BusResponse::Ack;
    }

    if circuit_id.trim().is_empty() {
        return BusResponse::Fail("dynamic circuit removal requires circuit_id".to_string());
    }

    if let Err(err) = lqos_network_devices::remove_dynamic_circuit(circuit_id) {
        error!("Dynamic circuit removal persist failed for circuit {circuit_id}: {err:?}");
        return BusResponse::Fail(format!("dynamic circuit removal persist failed: {err}"));
    }

    BusResponse::Ack
}

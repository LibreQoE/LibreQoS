use crate::node_manager::local_api::unknown_ips::get_unknown_ips;
use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct DeviceCount {
    pub shaped_devices: usize,
    pub unknown_ips: usize,
    pub mapped_circuits: usize,
}

pub fn device_count() -> DeviceCount {
    let shaped_devices = lqos_network_devices::shaped_devices_catalog();

    DeviceCount {
        shaped_devices: shaped_devices.devices_len(),
        unknown_ips: get_unknown_ips().len(),
        mapped_circuits: lqos_network_devices::mapped_circuit_count(),
    }
}

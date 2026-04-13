use crate::node_manager::local_api::unknown_ips::get_unknown_ips;
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Serialize, Debug, Clone)]
pub struct DeviceCount {
    pub shaped_devices: usize,
    pub unknown_ips: usize,
    pub mapped_circuits: usize,
}

pub fn device_count() -> DeviceCount {
    let shaped_devices = SHAPED_DEVICES.load();
    let mapped_circuits = shaped_devices
        .devices
        .iter()
        .map(|device| device.circuit_hash)
        .collect::<BTreeSet<_>>()
        .len();

    DeviceCount {
        shaped_devices: shaped_devices.devices.len(),
        unknown_ips: get_unknown_ips().len(),
        mapped_circuits,
    }
}

use crate::node_manager::local_api::unknown_ips::get_unknown_ips;
use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct DeviceCount {
    pub shaped_devices: usize,
    pub unknown_ips: usize,
}

pub fn device_count() -> DeviceCount {
    DeviceCount {
        shaped_devices: lqos_network_devices::shaped_devices_catalog().devices_len(),
        unknown_ips: get_unknown_ips().len(),
    }
}

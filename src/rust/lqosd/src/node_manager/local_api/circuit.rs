use crate::shaped_devices_tracker::SHAPED_DEVICES;
use lqos_config::ShapedDevice;

pub fn circuit_by_id_data(id: &str) -> Option<Vec<ShapedDevice>> {
    let safe_id = id.to_lowercase().trim().to_string();
    let reader = SHAPED_DEVICES.load();
    let devices: Vec<ShapedDevice> = reader
        .devices
        .iter()
        .filter(|d| d.circuit_id.to_lowercase().trim() == safe_id)
        .cloned()
        .collect();

    if devices.is_empty() {
        None
    } else {
        Some(devices)
    }
}

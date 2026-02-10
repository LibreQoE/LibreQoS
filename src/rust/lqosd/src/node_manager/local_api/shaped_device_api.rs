use crate::shaped_devices_tracker::SHAPED_DEVICES;
use lqos_config::ShapedDevice;

pub fn all_shaped_devices_data() -> Vec<ShapedDevice> {
    SHAPED_DEVICES.load().devices.clone()
}

use lqos_config::ShapedDevice;

pub fn all_shaped_devices_data() -> Vec<ShapedDevice> {
    lqos_network_devices::shaped_devices_catalog().clone_all_devices()
}

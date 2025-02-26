use lqos_bus::BusResponse;
use lqos_config::ConfigShapedDevices;

pub fn validate_shaped_devices_csv() -> BusResponse {
    let result = ConfigShapedDevices::load();
    match result {
        Ok(..) => BusResponse::Ack,
        Err(e) => BusResponse::ShapedDevicesValidation(format!("{e:#?}")),
    }
}

use lqos_bus::BusResponse;

pub fn validate_shaped_devices_csv() -> BusResponse {
    let result = lqos_network_devices::load_shaped_devices();
    match result {
        Ok(..) => BusResponse::Ack,
        Err(e) => BusResponse::ShapedDevicesValidation(format!("{e:#?}")),
    }
}

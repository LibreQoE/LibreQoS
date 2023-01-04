use lqos_bus::BusResponse;

pub fn reload_libre_qos() -> BusResponse {
    let result = lqos_config::load_libreqos();
    match result {
        Ok(message) => BusResponse::ReloadLibreQoS(message),
        Err(..) => BusResponse::Fail("Unable to reload LibreQoS".to_string()),
    }
}
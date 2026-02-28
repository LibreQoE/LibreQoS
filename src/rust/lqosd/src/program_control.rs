use lqos_bus::BusResponse;

pub fn reload_libre_qos() -> BusResponse {
    match crate::reload_lock::try_reload_libreqos_locked() {
        crate::reload_lock::ReloadExecOutcome::Success(message) => {
            BusResponse::ReloadLibreQoS(message)
        }
        crate::reload_lock::ReloadExecOutcome::Busy => {
            BusResponse::Fail("Reload already in progress".to_string())
        }
        crate::reload_lock::ReloadExecOutcome::Failed(_) => {
            BusResponse::Fail("Unable to reload LibreQoS".to_string())
        }
    }
}

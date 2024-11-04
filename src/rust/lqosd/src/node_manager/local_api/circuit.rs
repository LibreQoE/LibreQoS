use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use lqos_config::ShapedDevice;
use crate::shaped_devices_tracker::SHAPED_DEVICES;

#[derive(Deserialize)]
pub struct CircuitId {
    id: String
}

pub async fn get_circuit_by_id(Json(id) : Json<CircuitId>) -> Result<Json<Vec<ShapedDevice>>, StatusCode> {
    let safe_id = id.id.to_lowercase().trim().to_string();
    let reader = SHAPED_DEVICES.load();
    let devices: Vec<ShapedDevice> = reader
        .devices
        .iter()
        .filter(|d| d.circuit_id.to_lowercase().trim() == safe_id)
        .cloned()
        .collect();

    if devices.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    } else {
        return Ok(Json(devices));
    }
}
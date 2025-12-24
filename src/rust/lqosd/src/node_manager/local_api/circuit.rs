use crate::shaped_devices_tracker::SHAPED_DEVICES;
use axum::Json;
use axum::http::StatusCode;
use lqos_config::ShapedDevice;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct CircuitId {
    id: String,
}

pub async fn get_circuit_by_id(
    Json(id): Json<CircuitId>,
) -> Result<Json<Vec<ShapedDevice>>, StatusCode> {
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

#[derive(Serialize)]
pub struct CircuitHash {
    pub hash: i64,
}

/// Compute the standard LibreQoS circuit hash (i64) from a circuit_id string.
pub async fn hash_circuit(Json(id): Json<CircuitId>) -> Json<CircuitHash> {
    let h = lqos_utils::hash_to_i64(&id.id);
    Json(CircuitHash { hash: h })
}

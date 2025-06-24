use crate::shaped_devices_tracker::SHAPED_DEVICES;
use axum::Json;
use serde::Serialize;
use std::collections::HashSet;

#[derive(Serialize)]
pub struct CircuitCount {
    pub count: usize,
}

pub async fn get_circuit_count() -> Json<CircuitCount> {
    let shaped_devices = SHAPED_DEVICES.load();
    
    // Collect unique circuit IDs
    let unique_circuits: HashSet<&str> = shaped_devices
        .devices
        .iter()
        .map(|device| device.circuit_id.as_str())
        .collect();
    
    Json(CircuitCount {
        count: unique_circuits.len(),
    })
}
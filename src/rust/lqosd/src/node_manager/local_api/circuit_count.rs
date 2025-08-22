use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
use axum::Json;
use lqos_utils::unix_time::time_since_boot;
use serde::Serialize;
use std::collections::HashSet;
use std::time::Duration;

#[derive(Serialize)]
pub struct CircuitCount {
    pub count: usize,
    pub configured_count: usize,
}

pub async fn get_circuit_count() -> Json<CircuitCount> {
    const FIVE_MINUTES_IN_NANOS: u64 = 5 * 60 * 1_000_000_000;
    
    let now = Duration::from(time_since_boot().unwrap()).as_nanos() as u64;
    
    // Collect unique circuit IDs from active traffic
    let active_circuits: HashSet<String> = THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        // Only include shaped devices (non-zero tc_handle)
        .filter(|(_k, d)| d.tc_handle.as_u32() != 0)
        // Only include recently seen devices (within 5 minutes)
        .filter(|(_k, d)| now.saturating_sub(d.last_seen) < FIVE_MINUTES_IN_NANOS)
        // Extract circuit IDs where they exist
        .filter_map(|(_k, d)| d.circuit_id.clone())
        .collect();
    
    // Get configured circuits from ShapedDevices
    let shaped_devices = SHAPED_DEVICES.load();
    let configured_circuits: HashSet<&str> = shaped_devices
        .devices
        .iter()
        .map(|device| device.circuit_id.as_str())
        .collect();
    
    // Use active count if > 0, otherwise fall back to configured count
    let count = if active_circuits.len() > 0 {
        active_circuits.len()
    } else {
        configured_circuits.len()
    };
    
    Json(CircuitCount {
        count,
        configured_count: configured_circuits.len(),
    })
}
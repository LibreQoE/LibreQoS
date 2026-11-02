use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::services::shaped_devices;
use crate::transit_types::CircuitTransit;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct CircuitCapacityData {
    /// The circuit ID
    pub circuit_id: String,

    /// The circuit name
    pub circuit_name: Option<String>,

    /// Current download throughput in bits per second
    pub current_download_bps: u64,

    /// Current upload throughput in bits per second
    pub current_upload_bps: u64,

    /// Maximum configured download capacity in bits per second
    pub max_download_bps: u64,

    /// Maximum configured upload capacity in bits per second
    pub max_upload_bps: u64,

    /// Download utilization percentage (0-100+)
    pub download_utilization_percent: f32,

    /// Upload utilization percentage (0-100+)
    pub upload_utilization_percent: f32,

    /// True if circuit is at/near capacity (>= threshold)
    pub at_capacity: bool,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct CircuitsAtCapacityResponse {
    /// List of all circuits with their capacity analysis
    pub circuits: Vec<CircuitCapacityData>,

    /// Number of circuits at or near capacity
    pub circuits_at_capacity_count: usize,

    /// Percentage threshold used to determine "at capacity"
    pub capacity_threshold_percent: f32,
}

#[utoipa::path(
    get,
    path = "/circuits_at_capacity",
    summary = "Analyze circuits for near-capacity throughput",
    description = "Analyze circuits for near-capacity throughput.",
    responses(
        (status = 200, description = "Return 200 OK and circuit capacity analysis data.", body = CircuitsAtCapacityResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn circuits_at_capacity() -> Result<Json<CircuitsAtCapacityResponse>, (StatusCode, String)>
{
    // Threshold for considering a circuit "at capacity" (default 85%)
    let capacity_threshold = 85.0;

    // Get current circuit throughput data
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetAllCircuits])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;

    let circuits = responses
        .into_iter()
        .find_map(|response| {
            if let BusResponse::CircuitData(circuits) = response {
                Some(
                    circuits
                        .into_iter()
                        .map(CircuitTransit::from)
                        .collect::<Vec<CircuitTransit>>(),
                )
            } else {
                None
            }
        })
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "No circuit data received from bus".to_string(),
        ))?;

    // Get shaped device limits
    let shaped_devices = shaped_devices::load_or_empty().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load shaped devices: {}", e),
        )
    })?;

    // Group devices by circuit and calculate max capacity per circuit
    let mut circuit_limits: HashMap<String, (f32, f32)> = HashMap::new();
    let mut circuit_names: HashMap<String, String> = HashMap::new();

    for device in &shaped_devices.devices {
        let circuit_id = &device.circuit_id;
        circuit_names.insert(circuit_id.clone(), device.circuit_name.clone());

        let entry = circuit_limits
            .entry(circuit_id.clone())
            .or_insert((0.0, 0.0));
        // Sum all device capacities in this circuit
        entry.0 += device.download_max_mbps;
        entry.1 += device.upload_max_mbps;
    }

    // Analyze each circuit
    let mut circuit_analysis: Vec<CircuitCapacityData> = Vec::new();

    for circuit in circuits {
        if let Some(circuit_id) = &circuit.circuit_id {
            let (max_download_mbps, max_upload_mbps) = circuit_limits
                .get(circuit_id)
                .copied()
                .unwrap_or((0.0, 0.0));

            // Convert Mbps to bits per second
            let max_download_bps = (max_download_mbps * 1_000_000.0) as u64;
            let max_upload_bps = (max_upload_mbps * 1_000_000.0) as u64;

            // Current throughput from circuit data (bytes_per_second * 8 = bits_per_second)
            let current_download_bps = circuit.bytes_per_second.down * 8;
            let current_upload_bps = circuit.bytes_per_second.up * 8;

            // Calculate utilization percentages
            let download_utilization_percent = if max_download_bps > 0 {
                (current_download_bps as f32 / max_download_bps as f32) * 100.0
            } else {
                0.0
            };

            let upload_utilization_percent = if max_upload_bps > 0 {
                (current_upload_bps as f32 / max_upload_bps as f32) * 100.0
            } else {
                0.0
            };

            let at_capacity = download_utilization_percent >= capacity_threshold
                || upload_utilization_percent >= capacity_threshold;

            circuit_analysis.push(CircuitCapacityData {
                circuit_id: circuit_id.clone(),
                circuit_name: circuit_names
                    .get(circuit_id)
                    .cloned()
                    .or(circuit.circuit_name.clone()),
                current_download_bps,
                current_upload_bps,
                max_download_bps,
                max_upload_bps,
                download_utilization_percent,
                upload_utilization_percent,
                at_capacity,
            });
        }
    }

    // Sort by highest utilization first
    circuit_analysis.sort_by(|a, b| {
        let a_max_util = a
            .download_utilization_percent
            .max(a.upload_utilization_percent);
        let b_max_util = b
            .download_utilization_percent
            .max(b.upload_utilization_percent);
        b_max_util
            .partial_cmp(&a_max_util)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let circuits_at_capacity_count = circuit_analysis.iter().filter(|c| c.at_capacity).count();

    Ok(Json(CircuitsAtCapacityResponse {
        circuits: circuit_analysis,
        circuits_at_capacity_count,
        capacity_threshold_percent: capacity_threshold,
    }))
}

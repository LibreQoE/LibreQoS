use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::transit_types::CircuitTransit;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct CircuitRetransmitData {
    /// The circuit ID
    pub circuit_id: Option<String>,

    /// The circuit name
    pub circuit_name: Option<String>,

    /// The device name associated with this circuit
    pub device_name: Option<String>,

    /// The IP address of the circuit
    pub ip: String,

    /// Current TCP retransmit count (download/upload)
    pub tcp_retransmits_count: (u64, u64),

    /// Current TCP packet count (download/upload)
    pub tcp_packets_count: (u64, u64),

    /// TCP retransmit percentage (download/upload)
    pub retransmit_percentage_download: f32,
    pub retransmit_percentage_upload: f32,

    /// Maximum retransmit percentage of download/upload
    pub max_retransmit_percentage: f32,

    /// True if circuit has high retransmits (>= threshold)
    pub high_retransmits: bool,

    /// Current throughput in bytes per second (for context)
    pub bytes_per_second: (u64, u64),
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct CircuitsHighRetransmitsResponse {
    /// List of all circuits with their retransmit analysis
    pub circuits: Vec<CircuitRetransmitData>,

    /// Number of circuits with high retransmits
    pub high_retransmit_count: usize,

    /// Percentage threshold used to determine "high retransmits"
    pub retransmit_threshold_percent: f32,
}

#[utoipa::path(
    get,
    path = "/circuits_high_retransmits",
    summary = "Analyze circuits for high TCP retransmit rates",
    description = "Analyze circuits for high TCP retransmit rates.",
    responses(
        (status = 200, description = "Return 200 OK and circuit retransmit analysis data.", body = CircuitsHighRetransmitsResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn circuits_high_retransmits()
-> Result<Json<CircuitsHighRetransmitsResponse>, (StatusCode, String)> {
    // Threshold for considering a circuit to have "high retransmits" (default 1%)
    let retransmit_threshold = 1.0;

    // Get current circuit data
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

    // Analyze each circuit for retransmit rates
    let mut circuit_analysis: Vec<CircuitRetransmitData> = Vec::new();

    for circuit in circuits {
        // Extract retransmit and packet counts
        let tcp_retransmits_count = (circuit.tcp_retransmits.down, circuit.tcp_retransmits.up);
        let tcp_packets_count = (circuit.tcp_packets.down, circuit.tcp_packets.up);

        // Calculate retransmit percentages
        let retransmit_percentage_download = if tcp_packets_count.0 > 0 {
            (tcp_retransmits_count.0 as f32 / tcp_packets_count.0 as f32) * 100.0
        } else {
            0.0
        };

        let retransmit_percentage_upload = if tcp_packets_count.1 > 0 {
            (tcp_retransmits_count.1 as f32 / tcp_packets_count.1 as f32) * 100.0
        } else {
            0.0
        };

        let max_retransmit_percentage =
            retransmit_percentage_download.max(retransmit_percentage_upload);

        let high_retransmits = max_retransmit_percentage >= retransmit_threshold;

        circuit_analysis.push(CircuitRetransmitData {
            circuit_id: circuit.circuit_id.clone(),
            circuit_name: circuit.circuit_name.clone(),
            device_name: circuit.device_name.clone(),
            ip: circuit.ip,
            tcp_retransmits_count,
            tcp_packets_count,
            retransmit_percentage_download,
            retransmit_percentage_upload,
            max_retransmit_percentage,
            high_retransmits,
            bytes_per_second: (circuit.bytes_per_second.down, circuit.bytes_per_second.up),
        });
    }

    // Sort by highest retransmit percentage first
    circuit_analysis.sort_by(|a, b| {
        b.max_retransmit_percentage
            .partial_cmp(&a.max_retransmit_percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let high_retransmit_count = circuit_analysis
        .iter()
        .filter(|c| c.high_retransmits)
        .count();

    Ok(Json(CircuitsHighRetransmitsResponse {
        circuits: circuit_analysis,
        high_retransmit_count,
        retransmit_threshold_percent: retransmit_threshold,
    }))
}

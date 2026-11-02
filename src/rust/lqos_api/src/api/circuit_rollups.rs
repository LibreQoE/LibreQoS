use axum::{Json, extract::Path, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};

use crate::transit_types::CircuitRollupTransit;

async fn fetch_circuit_rollups() -> Result<Vec<CircuitRollupTransit>, (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetAllCircuitRollups])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;

    for response in responses {
        if let BusResponse::CircuitRollups(rollups) = response {
            return Ok(rollups
                .into_iter()
                .map(CircuitRollupTransit::from)
                .collect());
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

async fn fetch_circuit_rollup(
    circuit_id: String,
) -> Result<CircuitRollupTransit, (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetCircuitRollupById {
            circuit_id: circuit_id.clone(),
        }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;

    for response in responses {
        if let BusResponse::CircuitRollup(rollup) = response {
            let Some(rollup) = rollup else {
                return Err((
                    StatusCode::NOT_FOUND,
                    format!("No combined circuit data found for '{}'", circuit_id),
                ));
            };
            return Ok(CircuitRollupTransit::from(rollup));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/all_circuits_combined",
    summary = "Combined live metrics for all active circuits",
    description = "Return one live rollup per active circuit ID. This endpoint aggregates active IP/device rows into circuit-level throughput and quality totals.",
    responses(
        (status = 200, description = "Combined circuit rollups.", body = Vec<CircuitRollupTransit>),
        (status = 401, description = "Return 401 Unauthorized if the license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the license is invalid or not checked yet.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available.", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "The license key to use for authentication.", example = "YOUR_INSIGHT_LICENSE_KEY_HERE"),
    ),
)]
pub async fn get_all_circuits_combined()
-> Result<(StatusCode, Json<Vec<CircuitRollupTransit>>), (StatusCode, String)> {
    let rollups = fetch_circuit_rollups().await?;
    Ok((StatusCode::OK, Json(rollups)))
}

#[utoipa::path(
    get,
    path = "/circuit_combined/{circuit_id}",
    summary = "Combined live metrics for one circuit",
    description = "Return one live rollup for a circuit ID. This endpoint aggregates active IP/device rows into circuit-level throughput and quality totals.",
    responses(
        (status = 200, description = "Combined circuit rollup.", body = CircuitRollupTransit),
        (status = 401, description = "Return 401 Unauthorized if the license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the license is invalid or not checked yet.", body = String),
        (status = 404, description = "Return 404 Not Found if the circuit ID has no active rollup data.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available.", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "The license key to use for authentication.", example = "YOUR_INSIGHT_LICENSE_KEY_HERE"),
        ("circuit_id" = String, Path, description = "Circuit ID to query.", example = "10001-10004"),
    ),
)]
pub async fn get_circuit_combined(
    Path(circuit_id): Path<String>,
) -> Result<(StatusCode, Json<CircuitRollupTransit>), (StatusCode, String)> {
    let rollup = fetch_circuit_rollup(circuit_id).await?;
    Ok((StatusCode::OK, Json(rollup)))
}

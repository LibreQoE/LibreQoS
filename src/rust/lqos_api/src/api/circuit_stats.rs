use std::net::IpAddr;

use axum::{Json, extract::Path, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use lqos_utils::XdpIpAddress;

use crate::services::shaped_devices;
use crate::transit_types::CircuitTransit;

async fn fetch_circuit_stats(
    circuit_id: &str,
) -> Result<Vec<CircuitTransit>, (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    // Two arms, one request: the device rows carry the per-device rates, the
    // rollup carries the circuit's live activity age and RTT. The device rows'
    // own age/RTT are hard zeros on a v3 bridge, so the rollup is the only
    // honest source for those two fields.
    let responses = client
        .request(vec![
            BusRequest::GetCircuitById {
                circuit_id: circuit_id.to_string(),
            },
            BusRequest::GetCircuitRollupById {
                circuit_id: circuit_id.to_string(),
            },
        ])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;

    let mut circuits: Option<Vec<CircuitTransit>> = None;
    let mut rollup: Option<lqos_bus::CircuitRollup> = None;
    for response in responses {
        match response {
            BusResponse::CircuitData(rows) => {
                if rows.is_empty() {
                    return Err((
                        StatusCode::NOT_FOUND,
                        format!("No circuit data found for '{}'", circuit_id),
                    ));
                }
                circuits = Some(rows.into_iter().map(CircuitTransit::from).collect());
            }
            BusResponse::CircuitRollup(row) => rollup = row,
            _ => {}
        }
    }

    let Some(circuits) = circuits else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response from bus.".to_string(),
        ));
    };

    Ok(match rollup {
        Some(rollup) => circuits
            .into_iter()
            .map(|row| {
                row.with_activity_age(Some(rollup.last_seen_nanos))
                    .with_rtt_current_p50(rollup.rtt_current_p50_nanos.into())
            })
            .collect(),
        // No live rollup for this circuit: the age and RTT stay `null`, never 0.
        None => circuits,
    })
}

#[utoipa::path(
    get,
    path = "/circuit/{circuit_id}",
    summary = "Current metrics for a circuit",
    description = "Return current throughput and related metrics for a single circuit ID.",
    responses(
        (status = 200, description = "Circuit metrics for the requested circuit ID.", body = Vec<CircuitTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 404, description = "Return 404 Not Found if the circuit ID has no data.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available.", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("circuit_id" = String, Path, description = "Circuit ID to query.", example = "circuit123"),
    ),
)]
pub async fn get_circuit_stats_by_id(
    Path(circuit_id): Path<String>,
) -> Result<(StatusCode, Json<Vec<CircuitTransit>>), (StatusCode, String)> {
    let circuits = fetch_circuit_stats(&circuit_id).await?;
    Ok((StatusCode::OK, Json(circuits)))
}

#[utoipa::path(
    get,
    path = "/circuit/by_ip/{ip_address}",
    summary = "Current metrics for a circuit by IP address",
    description = "Resolve an IP address to a circuit via LPM lookup and return current metrics.",
    responses(
        (status = 200, description = "Circuit metrics for the resolved circuit.", body = Vec<CircuitTransit>),
        (status = 400, description = "Return 400 Bad Request if the IP address is invalid.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 404, description = "Return 404 Not Found if the IP does not map to a circuit or has no data.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available.", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("ip_address" = String, Path, description = "The IP address to look up.", example = "192.168.1.1"),
    ),
)]
pub async fn get_circuit_stats_by_ip(
    Path(ip_address): Path<String>,
) -> Result<(StatusCode, Json<Vec<CircuitTransit>>), (StatusCode, String)> {
    let devices = shaped_devices::load_or_empty().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load shaped devices: {}", e),
        )
    })?;

    let ip: IpAddr = ip_address.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid IP address: {}", e),
        )
    })?;
    let xdp_ip = XdpIpAddress::from(ip);
    let Some((circuit_id, _name)) = devices.get_circuit_id_and_name_from_ip(&xdp_ip) else {
        return Err((
            StatusCode::NOT_FOUND,
            format!("No circuit found for IP address: {}", ip_address),
        ));
    };

    let circuits = fetch_circuit_stats(&circuit_id).await?;
    Ok((StatusCode::OK, Json(circuits)))
}

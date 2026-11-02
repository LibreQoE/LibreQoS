use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};

use crate::transit_types::IpstatsTransit;

#[utoipa::path(
    get,
    path = "/unknown_ips",
    summary = "Unknown IP addresses not mapped to circuits",
    description = "Return unknown IP addresses not mapped to circuits.",
    responses(
        (status = 200, description = "Return 200 OK and a JSON array of unknown IPs seen recently.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
        (status = 503, description = "Return 503 Service Unavailable if the bus reports it's not ready yet (e.g. immediately after reboot).", body = String),
    ),
    tag = "Network Map",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn unknown_ips() -> Result<(StatusCode, Json<Vec<IpstatsTransit>>), (StatusCode, String)>
{
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::AllUnknownIps])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        match response {
            BusResponse::AllUnknownIps(stats) => {
                let stats: Vec<IpstatsTransit> = stats.into_iter().map(Into::into).collect();
                return Ok((StatusCode::OK, Json(stats)));
            }
            BusResponse::Fail(msg) => return Err((StatusCode::INTERNAL_SERVER_ERROR, msg)),
            BusResponse::NotReadyYet => {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Bus not ready yet".to_string(),
                ));
            }
            _ => {}
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

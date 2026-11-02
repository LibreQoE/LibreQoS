use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use lqos_utils::units::DownUpOrder;

#[utoipa::path(
    get,
    path = "/host_counter",
    summary = "Host counter statistics from lqosd",
    description = "Return host counter statistics from lqosd.",
    responses(
        (status = 200, description = "Return 200 OK and a JSON array with all host traffic counters.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "User Activity",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_host_counter()
-> Result<(StatusCode, Json<Vec<(String, DownUpOrder<u64>)>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetHostCounter])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::HostCounters(counters) = response {
            return Ok((
                StatusCode::OK,
                Json(
                    counters
                        .into_iter()
                        .map(|(ip, data)| (ip.to_string(), data))
                        .collect(),
                ),
            ));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

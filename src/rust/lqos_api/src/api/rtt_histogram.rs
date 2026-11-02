use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct RttHistogramData {
    /// RTT histogram buckets containing the count of measurements in each bucket
    buckets: Vec<u32>,
}

#[utoipa::path(
    get,
    path = "/rtt_histogram",
    summary = "Histogram of round-trip time measurements",
    description = "Return a histogram of round-trip time measurements.",
    responses(
        (status = 200, description = "Return 200 OK and RTT histogram data for rendering.", body = RttHistogramData),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn rtt_histogram() -> Result<(StatusCode, Json<RttHistogramData>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::RttHistogram])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;

    for response in responses {
        if let BusResponse::RttHistogram(buckets) = response {
            let result = RttHistogramData { buckets };
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

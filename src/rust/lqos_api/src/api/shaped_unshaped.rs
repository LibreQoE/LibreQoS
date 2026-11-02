use crate::transit_types::DownUpTransit;
use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct ShapedUnshapedTraffic {
    /// Traffic that is shaped (under QoS control). On a v3 node this is the
    /// delivered rate: every packet the bridge forwards is shaped.
    shaped_bits_per_second: DownUpTransit,
    /// Traffic that is not shaped. v3 does not measure an unshaped class, so
    /// this is always `null` — never a fabricated 0.
    unshaped_bits_per_second: Option<DownUpTransit>,
    /// Total traffic (shaped + unshaped)
    total_bits_per_second: DownUpTransit,
    /// Percentage of traffic that is shaped. Always `null` on v3: with no
    /// unshaped measurement the split is not derivable, so the endpoint never
    /// reports a fabricated 100%.
    shaped_percentage: Option<DownUpTransit>,
}

#[utoipa::path(
    get,
    path = "/shaped_unshaped_traffic",
    summary = "Current shaped versus unshaped traffic totals",
    description = "Return the current delivered (shaped) traffic rate and total. A v3 node does not measure an unshaped class, so `unshaped_bits_per_second` and `shaped_percentage` are `null` rather than a derived 0 or 100%.",
    responses(
        (status = 200, description = "Return 200 OK and shaped vs unshaped traffic breakdown.", body = ShapedUnshapedTraffic),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "System Metrics",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn shaped_unshaped_traffic()
-> Result<(StatusCode, Json<ShapedUnshapedTraffic>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetCurrentThroughput])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;

    for response in responses {
        if let BusResponse::CurrentThroughput {
            bits_per_second,
            shaped_bits_per_second,
            ..
        } = response
        {
            let total = DownUpTransit::from(bits_per_second);
            let shaped = DownUpTransit::from(shaped_bits_per_second);

            let result = ShapedUnshapedTraffic {
                shaped_bits_per_second: shaped,
                // No unshaped class is measured on v3, so neither the
                // difference nor the percentage is observable.
                unshaped_bits_per_second: None,
                total_bits_per_second: total,
                shaped_percentage: None,
            };

            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

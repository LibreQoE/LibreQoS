use crate::transit_types::DownUpTransit;
use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct CurrentThroughput {
    /// Delivered rate: on a v3 node every forwarded packet is shaped, so this
    /// is the shaped series rather than a pre-shaping demand figure.
    bits_per_second: DownUpTransit,
    packets_per_second: DownUpTransit,
    tcp_packets_per_second: DownUpTransit,
    udp_packets_per_second: DownUpTransit,
    icmp_packets_per_second: DownUpTransit,
    shaped_bits_per_second: DownUpTransit,
}

#[utoipa::path(
    get,
    path = "/current_throughput",
    summary = "Current network throughput totals",
    description = "Return current network throughput totals. On a v3 (bridge) node the customer-facing rate is the DELIVERED rate, so `bits_per_second` and `shaped_bits_per_second` carry the same measured series by construction; there is no separate unshaped class.",
    responses(
        (status = 200, description = "Return 200 OK and a JSON object with current throughput metrics.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "System Metrics",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn current_throughput()
-> Result<(StatusCode, Json<CurrentThroughput>), (StatusCode, String)> {
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
            packets_per_second,
            tcp_packets_per_second,
            udp_packets_per_second,
            icmp_packets_per_second,
            shaped_bits_per_second,
        } = response
        {
            let result = CurrentThroughput {
                bits_per_second: DownUpTransit::from(bits_per_second),
                packets_per_second: DownUpTransit::from(packets_per_second),
                tcp_packets_per_second: DownUpTransit::from(tcp_packets_per_second),
                udp_packets_per_second: DownUpTransit::from(udp_packets_per_second),
                icmp_packets_per_second: DownUpTransit::from(icmp_packets_per_second),
                shaped_bits_per_second: DownUpTransit::from(shaped_bits_per_second),
            };
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use lqos_utils::units::DownUpOrder;

#[utoipa::path(
    get,
    path = "/ether_protocol_summary",
    summary = "Summary of ethernet protocol usage",
    description = "Return a summary of ethernet protocol usage.",
    responses(
        (status = 200, description = "Return 200 OK and ethernet protocol summary.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Protocol Metrics",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn ether_protocol_summary()
-> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::EtherProtocolSummary])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::EtherProtocols {
            v4_bytes,
            v6_bytes,
            v4_packets,
            v6_packets,
            v4_rtt,
            v6_rtt,
        } = response
        {
            let summary = serde_json::json!({
                "v4_bytes": v4_bytes,
                "v6_bytes": v6_bytes,
                "v4_packets": v4_packets,
                "v6_packets": v6_packets,
                "v4_rtt": v4_rtt,
                "v6_rtt": v6_rtt
            });
            return Ok((StatusCode::OK, Json(summary)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/ip_protocol_summary",
    summary = "Summary of IP protocol usage",
    description = "Return a summary of IP protocol usage.",
    responses(
        (status = 200, description = "Return 200 OK and IP protocol summary.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Protocol Metrics",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn ip_protocol_summary()
-> Result<(StatusCode, Json<Vec<(String, DownUpOrder<u64>)>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::IpProtocolSummary])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::IpProtocols(protocols) = response {
            return Ok((StatusCode::OK, Json(protocols)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

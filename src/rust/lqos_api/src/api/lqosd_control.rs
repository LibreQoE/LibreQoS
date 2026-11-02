use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};

#[utoipa::path(
    post,
    path = "/reload_libreqos",
    summary = "Run a LibreQoS reload and return the output from the reload process",
    description = "Run a LibreQoS reload and return the output from the reload process.",
    responses(
        (status = 200, description = "Return 200 OK and the reload operation results.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Control & Reload",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn reload_libreqos() -> Result<(StatusCode, String), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::ReloadLibreQoS])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::ReloadLibreQoS(result) = response {
            return Ok((StatusCode::OK, result));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/lqos_stats",
    summary = "Lqosd runtime statistics for monitoring and diagnostics",
    description = "Return lqosd runtime statistics for monitoring and diagnostics. `high_watermark` and `rtt_events_per_second` belonged to the retired kernel flowbee and are `null` on v3: no measurement produces them, so they are never reported as 0.",
    responses(
        (status = 200, description = "Return 200 OK and lqosd statistics.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "System Metrics",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_lqos_stats() -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)>
{
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetLqosStats])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::LqosdStats {
            bus_requests,
            time_to_poll_hosts,
            tracked_flows,
            ..
        } = response
        {
            // `high_watermark` and `rtt_events_per_second` were kernel-flowbee
            // counters; v3 has no producer for either, so they are `null` here
            // rather than the retired module's hard zeros.
            let stats = serde_json::json!({
                "bus_requests": bus_requests,
                "time_to_poll_hosts": time_to_poll_hosts,
                "high_watermark": serde_json::Value::Null,
                "tracked_flows": tracked_flows,
                "rtt_events_per_second": serde_json::Value::Null,
            });
            return Ok((StatusCode::OK, Json(stats)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/stormguard_stats",
    summary = "Stormguard statistics as raw tuples for monitoring",
    description = "Return Stormguard statistics as raw tuples for monitoring.",
    responses(
        (status = 200, description = "Return 200 OK and Stormguard statistics.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Stormguard",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_stormguard_stats()
-> Result<(StatusCode, Json<Vec<(String, u64, u64)>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetStormguardStats])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::StormguardStats(stats) = response {
            return Ok((StatusCode::OK, Json(stats)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

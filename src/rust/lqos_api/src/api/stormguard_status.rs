use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct StormguardSiteStatus {
    /// Site name or identifier
    site: String,
    /// Download bandwidth limit in bits per second
    download_limit: u64,
    /// Upload bandwidth limit in bits per second
    upload_limit: u64,
}

/// Array of StormGuard site statuses for dashboard graphing
pub type StormguardStatusData = Vec<StormguardSiteStatus>;

#[utoipa::path(
    get,
    path = "/stormguard_status",
    summary = "Stormguard site limits formatted for dashboard graphs",
    description = "Return Stormguard site limits formatted for dashboard graphs.",
    responses(
        (status = 200, description = "Return 200 OK and StormGuard status data formatted for dashboard bandwidth visualization.", body = Vec<StormguardSiteStatus>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Stormguard",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn stormguard_status()
-> Result<(StatusCode, Json<StormguardStatusData>), (StatusCode, String)> {
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
            let result: StormguardStatusData = stats
                .into_iter()
                .map(|(site, download, upload)| StormguardSiteStatus {
                    site,
                    download_limit: download,
                    upload_limit: upload,
                })
                .collect();
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

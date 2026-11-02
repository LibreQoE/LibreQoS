use crate::services;
use axum::{Json, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct HostCountsData {
    /// Total hosts detected
    pub total: u32,
    /// Hosts currently mapped/shaped
    pub shaped: u32,
    /// Hosts not mapped/shaped
    pub unknown: u32,
}

#[utoipa::path(
    get,
    path = "/host_counts",
    summary = "Host counts grouped by circuit",
    description = "Return host counts grouped by circuit.",
    responses(
        (status = 200, description = "Return 200 OK and host count summary.", body = HostCountsData),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "User Activity",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn host_counts() -> Result<(StatusCode, Json<HostCountsData>), (StatusCode, String)> {
    match services::host_counts().await {
        Ok((total, shaped, unknown)) => Ok((
            StatusCode::OK,
            Json(HostCountsData {
                total,
                shaped,
                unknown,
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get host counts: {}", e),
        )),
    }
}

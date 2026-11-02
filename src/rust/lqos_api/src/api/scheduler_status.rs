use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct SchedulerStatusData {
    /// Is the scheduler running
    pub running: bool,
    /// Any error message reported by integrations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[utoipa::path(
    get,
    path = "/scheduler_status",
    summary = "Report whether the scheduler is running and any error message",
    description = "Report whether the scheduler is running and any error message.",
    responses(
        (status = 200, description = "Scheduler running state and any error string.", body = SchedulerStatusData),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
        (status = 500, description = "Internal error communicating with bus.", body = String),
    ),
    tag = "Control & Reload",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn scheduler_status()
-> Result<(StatusCode, Json<SchedulerStatusData>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::CheckSchedulerStatus])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;

    for r in responses {
        if let BusResponse::SchedulerStatus { running, error, .. } = r {
            return Ok((StatusCode::OK, Json(SchedulerStatusData { running, error })));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

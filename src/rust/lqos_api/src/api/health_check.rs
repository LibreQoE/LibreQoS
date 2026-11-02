use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::Serialize;
use utoipa::ToSchema;

use crate::services;

#[derive(Serialize, ToSchema)]
pub struct HealthStatus {
    /// Service is up and responding to bus ping
    ok: bool,
    /// If known, whether the scheduler is running
    #[serde(skip_serializing_if = "Option::is_none")]
    scheduler_running: Option<bool>,
    /// Count of device rows with no live activity sample. The node retains a
    /// circuit's live row for ~30 s, so this is "not seen in the last 30 s" —
    /// a wider window is not answerable, and `null` means the rollup read failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    not_reporting_recently: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/health",
    summary = "Basic health status and scheduler visibility",
    description = "Return basic health status and scheduler visibility.",
    responses(
        (status = 200, description = "Return 200 OK if the service is running.", body = HealthStatus),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Health & Status",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn health_check() -> Result<(StatusCode, Json<HealthStatus>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client.request(vec![BusRequest::Ping]).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to communicate with bus: {}", e),
        )
    })?;
    if responses.len() != 1 || !matches!(responses[0], BusResponse::Ack) {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response from bus.".to_string(),
        ));
    }

    // Also report whether the scheduler is running (ignore status string)
    let mut scheduler_running: Option<bool> = None;
    if let Ok(responses) = client.request(vec![BusRequest::CheckSchedulerStatus]).await {
        for r in responses {
            if let BusResponse::SchedulerStatus { running, .. } = r {
                scheduler_running = Some(running);
                break;
            }
        }
    }

    // Additionally, provide a quick signal of customers not reporting recently.
    // The age can only come from the daemon's circuit-live rollups (the bus
    // rows' `last_seen_nanos` is a hard zero on a v3 bridge), and those rollups
    // retain a circuit for ~30 s — so the only window this can answer is that
    // horizon. Asking for 300 s would report every circuit idle 31 s as missing.
    // When the rollup arm does not answer, the field stays `null` rather than
    // claiming every circuit is healthy.
    let mut not_reporting_recently: Option<u32> = None;
    if let (Ok(ages), Ok(circuits)) = (
        services::circuit_activity_ages().await,
        services::known_circuits().await,
    ) {
        let window_ns: u64 = services::ACTIVITY_WINDOW_SECS.saturating_mul(1_000_000_000);
        let inactive = circuits
            .iter()
            .filter(|circuit| {
                circuit.circuit_id.is_some() && !services::seen_within(circuit, &ages, window_ns)
            })
            .count() as u32;
        not_reporting_recently = Some(inactive);
    }

    Ok((
        StatusCode::OK,
        Json(HealthStatus {
            ok: true,
            scheduler_running,
            not_reporting_recently,
        }),
    ))
}

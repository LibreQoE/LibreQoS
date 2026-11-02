use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

use crate::{api::ApiState, services};

#[derive(Serialize, ToSchema)]
pub struct StatusSnapshotTransit {
    ok: bool,
    throughput: Option<services::ThroughputSnapshot>,
    system: Option<services::SystemSnapshot>,
    users: Option<services::UserSnapshot>,
}

#[utoipa::path(
    get,
    path = "/status_snapshot",
    summary = "Combined snapshot of health, throughput, system, and user metrics",
    description = "Return a combined snapshot of health, throughput, system, and user metrics.",
    responses(
        (status = 200, description = "Combined health + key metrics snapshot", body = StatusSnapshotTransit),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
        (status = 500, description = "Internal error", body = String),
    ),
    tag = "Health & Status",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key."),
        ("window" = u64, Query, description = "Active user window (seconds). The node's activity ages come from a ~30 s retention, so the window is clamped to 30 s.", example = 30),
    ),
)]
pub async fn status_snapshot(
    State(state): State<Arc<ApiState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<(StatusCode, Json<StatusSnapshotTransit>), (StatusCode, String)> {
    let window_secs = params
        .get("window")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(300);

    let snap = services::status_snapshot(window_secs, &state).await;
    let result = StatusSnapshotTransit {
        ok: snap.ok,
        throughput: snap.throughput,
        system: snap.system,
        users: snap.users,
    };
    Ok((StatusCode::OK, Json(result)))
}

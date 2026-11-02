use crate::api::ApiState;
use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

/// CPU usage data for each core
pub type CpuUsageData = Vec<u32>;

/// One sample of the daemon's 1 Hz CPU/RAM monitor.
///
/// The mount reads this from the process that already samples the host; it has
/// no sampler of its own, so the API and the daemon's own pages can never
/// report two different CPU figures.
#[derive(Debug, Clone)]
pub struct SystemUsageSnapshot {
    /// Per-CPU usage, in the daemon's sampler order.
    pub cpu_usage: Vec<u32>,
    /// The kernel CPU id for each `cpu_usage` entry, when the join is proven.
    /// `u32::MAX` = unknown (never treat the vector index as a CPU id).
    pub cpu_ids: Vec<u32>,
    /// Number of CPUs the sampler counted.
    pub num_cpus: usize,
    /// RAM currently used (bytes).
    pub ram_used: u64,
    /// Total RAM installed (bytes).
    pub total_ram: u64,
}

#[derive(Serialize, ToSchema)]
pub struct RamUsageData {
    /// Total RAM installed (bytes)
    total: u64,
    /// RAM currently used (bytes)
    used: u64,
}

#[utoipa::path(
    get,
    path = "/cpu_usage",
    summary = "Current CPU usage information",
    description = "Return current CPU usage information from the daemon's 1 Hz monitor.",
    responses(
        (status = 200, description = "Return 200 OK and CPU usage percentage for each core.", body = Vec<u32>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the daemon's system monitor has not produced a sample yet.", body = String),
    ),
    tag = "System Metrics",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn cpu_usage(
    State(state): State<Arc<ApiState>>,
) -> Result<(StatusCode, Json<CpuUsageData>), (StatusCode, String)> {
    let stats = sample(&state)?;
    Ok((StatusCode::OK, Json(stats.cpu_usage)))
}

/// The daemon's current sample, or the honest "not sampled yet" refusal.
///
/// The daemon starts its sampler before the listener, but the first tick is a
/// second away: answering an empty or zero-filled sample would be a fabricated
/// measurement, so a cold start fails loudly instead.
fn sample(state: &ApiState) -> Result<SystemUsageSnapshot, (StatusCode, String)> {
    let stats = state.system_stats();
    if stats.num_cpus == 0 {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "System monitor has not produced a sample yet".to_string(),
        ));
    }
    Ok(stats)
}

#[utoipa::path(
    get,
    path = "/ram_usage",
    summary = "Current RAM usage information",
    description = "Return current RAM usage information from the daemon's 1 Hz monitor.",
    responses(
        (status = 200, description = "Return 200 OK and RAM usage information.", body = RamUsageData),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the daemon's system monitor has not produced a sample yet.", body = String),
    ),
    tag = "System Metrics",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn ram_usage(
    State(state): State<Arc<ApiState>>,
) -> Result<(StatusCode, Json<RamUsageData>), (StatusCode, String)> {
    let stats = sample(&state)?;
    let result = RamUsageData {
        total: stats.total_ram,
        used: stats.ram_used,
    };
    Ok((StatusCode::OK, Json(result)))
}

#[derive(Serialize, ToSchema)]
pub struct SystemStatsHealth {
    /// Number of CPU cores counted
    num_cpus: usize,
    /// Total RAM installed (bytes)
    total_ram: u64,
    /// RAM currently used (bytes)
    ram_used: u64,
}

#[utoipa::path(
    get,
    path = "/system_stats_health",
    summary = "Basic health status for system stats collection",
    description = "Return basic health status for the daemon's system stats collection.",
    responses(
        (status = 200, description = "Return 200 OK and basic system stats for health checks.", body = SystemStatsHealth),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the daemon's system monitor has not produced a sample yet.", body = String),
    ),
    tag = "System Metrics",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn system_stats_health(
    State(state): State<Arc<ApiState>>,
) -> Result<(StatusCode, Json<SystemStatsHealth>), (StatusCode, String)> {
    let stats = sample(&state)?;
    let result = SystemStatsHealth {
        num_cpus: stats.num_cpus,
        total_ram: stats.total_ram,
        ram_used: stats.ram_used,
    };
    Ok((StatusCode::OK, Json(result)))
}

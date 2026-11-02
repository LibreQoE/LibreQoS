use axum::{Json, http::StatusCode};

use crate::{services, transit_types::CircuitTransit};

fn seconds_to_window_nanos(seconds: u64) -> u64 {
    seconds.saturating_mul(1_000_000_000)
}

#[utoipa::path(
    get,
    path = "/missing_users/{seconds}",
    summary = "Circuits missing activity for longer than the window",
    description = "Return the device rows whose circuit the node has not seen inside the window, including circuits it has never seen. The age comes from the daemon's per-circuit live rollup, which retains a circuit for 30 s after its last activity: the window is clamped to that horizon, so this reports \"no live sample in the last ~30 s\" however wide `seconds` is asked.",
    responses(
        (status = 200, description = "Return 200 OK and a list of known circuits inactive for longer than the window.", body = Vec<CircuitTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "User Activity",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("seconds" = u64, Path, description = "Inactivity window in seconds.", example = 300),
    ),
)]
pub async fn missing_users(
    axum::extract::Path(seconds): axum::extract::Path<u64>,
) -> Result<(StatusCode, Json<Vec<CircuitTransit>>), (StatusCode, String)> {
    let ages = services::circuit_activity_ages().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to communicate with bus: {e}"),
        )
    })?;
    let circuits = services::known_circuits().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to communicate with bus: {e}"),
        )
    })?;

    let window_ns = seconds_to_window_nanos(services::activity_window_secs(seconds));
    let list: Vec<CircuitTransit> = circuits
        .into_iter()
        .filter(|circuit| {
            circuit.circuit_id.is_some() && !services::seen_within(circuit, &ages, window_ns)
        })
        .map(|circuit| {
            let age = services::activity_age(&circuit, &ages);
            CircuitTransit::from(circuit).with_activity_age(age)
        })
        .collect();
    Ok((StatusCode::OK, Json(list)))
}

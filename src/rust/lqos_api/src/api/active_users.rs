use axum::{Json, http::StatusCode};

use crate::{services, transit_types::CircuitTransit};

fn seconds_to_window_nanos(seconds: u64) -> u64 {
    // Convert seconds to nanoseconds with saturating multiplication
    seconds.saturating_mul(1_000_000_000)
}

/// The bus rows and their circuit-level activity ages, fetched together.
///
/// One helper for the three user-activity endpoints: an age can only come from
/// the circuit-live rollups, so a failed rollup read must fail the request
/// rather than answer from the bus rows' hard-zero `last_seen_nanos`.
async fn circuits_with_ages() -> Result<
    (
        Vec<lqos_bus::Circuit>,
        std::collections::HashMap<String, u64>,
    ),
    (StatusCode, String),
> {
    let ages = services::circuit_activity_ages().await.map_err(bus_error)?;
    let circuits = services::known_circuits().await.map_err(bus_error)?;
    Ok((circuits, ages))
}

/// The bus is the data source; a missing or misanswering bus is a 500.
fn bus_error(error: anyhow::Error) -> (StatusCode, String) {
    let text = error.to_string();
    if text.contains("Unexpected bus response") {
        return (StatusCode::INTERNAL_SERVER_ERROR, text);
    }
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to communicate with bus: {text}"),
    )
}

/// The circuits seen inside `window_secs`, one row per device, with each row
/// carrying its circuit's live activity age.
///
/// `window_secs` is the window the node can actually answer (see
/// [`services::activity_window_secs`]); the caller applies the clamp so the
/// response can report it.
fn active_rows(
    circuits: Vec<lqos_bus::Circuit>,
    ages: &std::collections::HashMap<String, u64>,
    window_secs: u64,
) -> Vec<CircuitTransit> {
    let window_ns = seconds_to_window_nanos(window_secs);
    circuits
        .into_iter()
        .filter(|circuit| services::seen_within(circuit, ages, window_ns))
        .map(|circuit| {
            let age = services::activity_age(&circuit, ages);
            CircuitTransit::from(circuit).with_activity_age(age)
        })
        .collect()
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct ActiveUserCountData {
    /// Number of device rows whose circuit the node saw inside the window
    pub active: u32,
    /// The window actually applied, in seconds. The node's activity ages come
    /// from a ~30 s retention, so a wider request is answered at
    /// `ACTIVITY_WINDOW_SECS` and reports that here.
    pub window_seconds: u64,
}

#[utoipa::path(
    get,
    path = "/active_user_count/{seconds}",
    summary = "Count of active circuits within a time window",
    description = "Return a count of device rows whose circuit the node saw inside the window. The age comes from the daemon's per-circuit live rollup, which retains a circuit for 30 s after its last activity: ages are exact up to that horizon, and a wider `seconds` is clamped to 30 and reported in `window_seconds`. A circuit with no live sample is not counted, and is never reported as active.",
    responses(
        (status = 200, description = "Return 200 OK and a count of active users within the window.", body = ActiveUserCountData),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "User Activity",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("seconds" = u64, Path, description = "Active window in seconds.", example = 300),
    ),
)]
pub async fn active_user_count(
    axum::extract::Path(seconds): axum::extract::Path<u64>,
) -> Result<(StatusCode, Json<ActiveUserCountData>), (StatusCode, String)> {
    let (circuits, ages) = circuits_with_ages().await?;
    let window_seconds = services::activity_window_secs(seconds);
    let active = active_rows(circuits, &ages, window_seconds).len() as u32;
    Ok((
        StatusCode::OK,
        Json(ActiveUserCountData {
            active,
            window_seconds,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/active_user_list/{seconds}",
    summary = "List of active circuits within a time window",
    description = "Return the device rows whose circuit the node saw inside the window. Each row's `last_seen_nanos` is its circuit's live age and `rtt_current_p50_nanos` its live RTT; both are null when the node has no live sample. The ages are exact up to the daemon's 30 s retention; a wider `seconds` is clamped to 30 s, so rows older than that are not listed (filter on each row's age if a narrower window is needed).",
    responses(
        (status = 200, description = "Return 200 OK and a list of active users within the window.", body = Vec<CircuitTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "User Activity",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("seconds" = u64, Path, description = "Active window in seconds.", example = 300),
    ),
)]
pub async fn active_user_list(
    axum::extract::Path(seconds): axum::extract::Path<u64>,
) -> Result<(StatusCode, Json<Vec<CircuitTransit>>), (StatusCode, String)> {
    let (circuits, ages) = circuits_with_ages().await?;
    let window_seconds = services::activity_window_secs(seconds);
    Ok((
        StatusCode::OK,
        Json(active_rows(circuits, &ages, window_seconds)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn circuit(id: &str) -> lqos_bus::Circuit {
        use lqos_utils::units::DownUpOrder;

        lqos_bus::Circuit {
            ip: "192.0.2.1".parse().expect("test ip"),
            bytes_per_second: DownUpOrder::zeroed(),
            actual_bytes_per_second: DownUpOrder::zeroed(),
            median_latency: None,
            rtt_current_p50_nanos: DownUpOrder::default(),
            rtt_current_p95_nanos: DownUpOrder::default(),
            rtt_total_p50_nanos: DownUpOrder::default(),
            rtt_total_p95_nanos: DownUpOrder::default(),
            qoo: DownUpOrder::default(),
            tcp_retransmit_sample: DownUpOrder::default(),
            circuit_id: Some(id.to_string()),
            device_id: Some(format!("device-{id}")),
            parent_node: Some("site".to_string()),
            circuit_name: Some(id.to_string()),
            device_name: Some(id.to_string()),
            plan: DownUpOrder {
                down: 100.0,
                up: 20.0,
            },
            last_seen_nanos: 0,
        }
    }

    /// The join decides the answer: a live age inside the window is active, an
    /// absent or older age is not, and the bus row's zero never counts.
    #[test]
    fn activity_is_decided_by_the_live_age_not_the_row_zero() {
        let ages = HashMap::from([
            ("fresh".to_string(), 5_000_000_000_u64),
            ("carried".to_string(), 100_000_000_000_u64),
            ("stale".to_string(), 400_000_000_000_u64),
        ]);
        let circuits = vec![
            circuit("fresh"),
            circuit("carried"),
            circuit("stale"),
            circuit("never-seen"),
        ];

        // A SHORT window is answered as asked: 5 s is inside 10 s, 100 s is not.
        let active = active_rows(circuits.clone(), &ages, services::activity_window_secs(10));
        assert_eq!(
            active.len(),
            1,
            "only the fresh circuit is active inside 10 s"
        );
        assert_eq!(active[0].circuit_id.as_deref(), Some("fresh"));
        assert_eq!(active[0].last_seen_nanos, Some(5_000_000_000));

        // The 100 s circuit sits BETWEEN the 30 s horizon and a 300 s request. It
        // is the fixture that makes the clamp load-bearing: the raw request would
        // count it, the clamped window must not.
        let requested = 300;
        let unclamped = active_rows(circuits.clone(), &ages, requested);
        assert_eq!(
            unclamped.len(),
            2,
            "without the clamp, a 300 s request would report the 100 s circuit"
        );

        let clamped = services::activity_window_secs(requested);
        assert_eq!(clamped, services::ACTIVITY_WINDOW_SECS);
        assert_ne!(
            unclamped.len(),
            active_rows(circuits.clone(), &ages, clamped).len(),
            "the clamp must change this answer, or the test cannot fail"
        );
        let active = active_rows(circuits, &ages, clamped);
        assert_eq!(
            active.len(),
            1,
            "the clamped window must exclude the 100 s circuit; if the clamp is removed this              assertion and the 2 above swap answers"
        );
    }

    /// The clamp is stated, not implied: it bounds both ends and leaves the
    /// answerable range alone.
    #[test]
    fn the_activity_window_is_clamped_to_what_the_node_can_answer() {
        assert_eq!(services::activity_window_secs(0), 1);
        assert_eq!(services::activity_window_secs(10), 10);
        assert_eq!(services::activity_window_secs(30), 30);
        assert_eq!(services::activity_window_secs(300), 30);
        assert_eq!(services::activity_window_secs(u64::MAX), 30);
    }

    /// The bus row's own `last_seen_nanos` is a hard zero on a v3 bridge; a
    /// filter that read it would call every configured circuit active.
    #[test]
    fn a_row_without_a_live_age_is_never_active() {
        let ages = HashMap::new();
        let rows = active_rows(
            vec![circuit("configured-but-idle")],
            &ages,
            services::activity_window_secs(300),
        );
        assert!(rows.is_empty());
    }
}

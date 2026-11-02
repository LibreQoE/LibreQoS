use anyhow::{Result, anyhow};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::api::ApiState;
pub mod shaped_devices;

/// The widest activity window the node can answer, in seconds.
///
/// The daemon's circuit-live rollups are the only source of an activity age, and
/// they retain a circuit's row for 30 s after its last fresh poll. A circuit idle
/// longer than that has no live sample at all, so the node cannot distinguish
/// "idle 31 s" from "idle 5 days": no answer exists for a wider window, and the
/// API clamps to this one instead of implying otherwise.
pub const ACTIVITY_WINDOW_SECS: u64 = 30;

/// The window actually applied to an activity age.
///
/// Ages are exact inside the retention horizon (the daemon recomputes a carried
/// row's age from its last fresh poll, so it grows with idleness), so a narrower
/// window is answered as asked. A wider one is clamped to
/// [`ACTIVITY_WINDOW_SECS`], and the endpoints report the window they applied.
pub fn activity_window_secs(requested: u64) -> u64 {
    requested.clamp(1, ACTIVITY_WINDOW_SECS)
}

/// Per-circuit activity ages, in nanoseconds since the circuit was last seen.
///
/// The daemon's circuit-live rollups are the only v3 source of an activity age:
/// the per-device `GetAllCircuits` rows carry a hard zero there (bridge mode has
/// no per-device age), and a zero must never be read as "seen just now".
pub async fn circuit_activity_ages() -> Result<HashMap<String, u64>> {
    let mut client = LibreqosBusClient::new().await?;
    let replies = client
        .request(vec![BusRequest::GetAllCircuitRollups])
        .await?;
    for resp in replies {
        if let BusResponse::CircuitRollups(rollups) = resp {
            return Ok(rollups
                .into_iter()
                .map(|rollup| (rollup.circuit_id, rollup.last_seen_nanos))
                .collect());
        }
    }
    Err(anyhow!("Unexpected bus response for GetAllCircuitRollups"))
}

/// Ping the LibreQoS bus to verify lqosd is responsive.
pub async fn bus_ping() -> Result<bool> {
    let mut client = LibreqosBusClient::new().await?;
    let replies = client.request(vec![BusRequest::Ping]).await?;
    Ok(replies.len() == 1 && matches!(replies[0], BusResponse::Ack))
}

/// Get host counts (total, shaped, unknown)
pub async fn host_counts() -> Result<(u32, u32, u32)> {
    let mut client = LibreqosBusClient::new().await?;
    let replies = client.request(vec![BusRequest::HostCounts]).await?;
    for resp in replies {
        if let BusResponse::HostCounts((total, shaped)) = resp {
            let unknown = total.saturating_sub(shaped);
            return Ok((total, shaped, unknown));
        }
    }
    Err(anyhow!("Unexpected bus response for HostCounts"))
}

/// Count active known circuits within a time window (seconds).
///
/// The count joins the daemon's circuit-live activity ages: a circuit with no
/// live sample is NOT active, and is never counted as one. The window is clamped
/// to [`ACTIVITY_WINDOW_SECS`] — the retention horizon is the widest window any
/// answer can cover.
pub async fn active_user_count(window_secs: u64) -> Result<u32> {
    let ages = circuit_activity_ages().await?;
    let window_ns = activity_window_secs(window_secs).saturating_mul(1_000_000_000);
    let circuits = known_circuits().await?;
    Ok(circuits
        .iter()
        .filter(|circuit| seen_within(circuit, &ages, window_ns))
        .count() as u32)
}

/// The per-device circuit rows the bus reports (`GetAllCircuits`).
pub async fn known_circuits() -> Result<Vec<lqos_bus::Circuit>> {
    let mut client = LibreqosBusClient::new().await?;
    let replies = client.request(vec![BusRequest::GetAllCircuits]).await?;
    for resp in replies {
        if let BusResponse::CircuitData(circuits) = resp {
            return Ok(circuits);
        }
    }
    Err(anyhow!("Unexpected bus response for GetAllCircuits"))
}

/// The circuit-level activity age for one bus row, when the daemon has a live
/// sample for its circuit. A row without a circuit id has no age.
pub fn activity_age(circuit: &lqos_bus::Circuit, ages: &HashMap<String, u64>) -> Option<u64> {
    circuit
        .circuit_id
        .as_deref()
        .and_then(|circuit_id| ages.get(circuit_id).copied())
}

/// `true` when the daemon last saw this circuit inside `window_ns`.
///
/// An absent age is "not seen in the window", never "seen just now": the bus
/// row's own `last_seen_nanos` is a hard zero on a v3 bridge, so reading it
/// would report every configured circuit as active forever.
pub fn seen_within(
    circuit: &lqos_bus::Circuit,
    ages: &HashMap<String, u64>,
    window_ns: u64,
) -> bool {
    circuit.circuit_id.is_some() && activity_age(circuit, ages).is_some_and(|age| age <= window_ns)
}

#[derive(Serialize, ToSchema, Clone)]
pub struct SimpleDownUp {
    pub down: u64,
    pub up: u64,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct ThroughputSnapshot {
    pub bits_per_second: SimpleDownUp,
    pub packets_per_second: SimpleDownUp,
    pub shaped_bits_per_second: SimpleDownUp,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct SystemSnapshot {
    pub num_cpus: usize,
    pub cpu_usage: Vec<u32>,
    pub total_ram: u64,
    pub ram_used: u64,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct UserSnapshot {
    pub active: u32,
    pub total: u32,
    pub shaped: u32,
    pub unknown: u32,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct StatusSnapshot {
    pub ok: bool,
    pub throughput: Option<ThroughputSnapshot>,
    pub system: Option<SystemSnapshot>,
    pub users: Option<UserSnapshot>,
}

pub async fn current_throughput() -> Result<ThroughputSnapshot> {
    let mut client = LibreqosBusClient::new().await?;
    let replies = client
        .request(vec![BusRequest::GetCurrentThroughput])
        .await?;
    for r in replies {
        if let BusResponse::CurrentThroughput {
            bits_per_second,
            packets_per_second,
            shaped_bits_per_second,
            ..
        } = r
        {
            return Ok(ThroughputSnapshot {
                bits_per_second: SimpleDownUp {
                    down: bits_per_second.down,
                    up: bits_per_second.up,
                },
                packets_per_second: SimpleDownUp {
                    down: packets_per_second.down,
                    up: packets_per_second.up,
                },
                shaped_bits_per_second: SimpleDownUp {
                    down: shaped_bits_per_second.down,
                    up: shaped_bits_per_second.up,
                },
            });
        }
    }
    Err(anyhow!("Unexpected bus response for GetCurrentThroughput"))
}

/// The daemon's current CPU/RAM sample, in this mount's snapshot shape.
///
/// `None` before the sampler's first tick: a cold start has no sample, and a
/// zero-filled one would read as a real measurement.
pub fn system_snapshot(state: &ApiState) -> Option<SystemSnapshot> {
    let stats = state.system_stats();
    if stats.num_cpus == 0 {
        return None;
    }
    Some(SystemSnapshot {
        num_cpus: stats.num_cpus,
        cpu_usage: stats.cpu_usage,
        total_ram: stats.total_ram,
        ram_used: stats.ram_used,
    })
}

pub async fn status_snapshot(window_secs: u64, state: &ApiState) -> StatusSnapshot {
    // ok flag
    let ok = bus_ping().await.unwrap_or(false);

    // Try gather throughput
    let throughput = current_throughput().await.ok();

    // System snapshot (the daemon's sampler, read synchronously)
    let system = system_snapshot(state);

    // Users
    let users = match (active_user_count(window_secs).await, host_counts().await) {
        (Ok(active), Ok((total, shaped, unknown))) => Some(UserSnapshot {
            active,
            total,
            shaped,
            unknown,
        }),
        _ => None,
    };

    StatusSnapshot {
        ok,
        throughput,
        system,
        users,
    }
}

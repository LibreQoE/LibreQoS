use std::net::IpAddr;
use std::time::Duration;

use axum::{Json, extract::Path, http::StatusCode};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::services::shaped_devices;

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct CircuitTraceHopNode {
    pub ip: String,
    pub rtts_ms: Vec<f32>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct CircuitTraceHop {
    pub ttl: u8,
    pub nodes: Vec<CircuitTraceHopNode>,
    pub loss_pct: f64,
    pub forward_loss_pct: f64,
    pub backward_loss_pct: f64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CircuitTracerouteStatus {
    Trace { hops: Vec<CircuitTraceHop> },
    Disabled,
    NotFound,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct TracerouteCircuitResponse {
    pub circuit_id: String,
    pub result: CircuitTracerouteStatus,
}

fn choose_host_ip_for_circuit(circuit_id: &str) -> Option<IpAddr> {
    if let Ok(Some(cfg)) = shaped_devices::load_optional() {
        for d in cfg.devices.iter().filter(|d| d.circuit_id == circuit_id) {
            if let Some((ip, _)) = d.ipv4.iter().find(|(_, p)| *p == 32) {
                return Some(IpAddr::V4(*ip));
            }
            if let Some((ip, _)) = d.ipv6.iter().find(|(_, p)| *p == 128) {
                return Some(IpAddr::V6(*ip));
            }
        }
    }
    None
}

#[utoipa::path(
    get,
    path = "/traceroute_circuit/{circuit_id}",
    summary = "Traceroute to a representative host for a circuit",
    description = "Traceroute to a representative host for a circuit.",
    responses(
        (status = 200, description = "Traceroute hops for a circuit target or disabled/not found", body = TracerouteCircuitResponse),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
        (status = 404, description = "Circuit has no eligible host IPs", body = String),
        (status = 500, description = "Internal error", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
        ("circuit_id" = String, Path, description = "Circuit ID to traceroute"),
    ),
)]
pub async fn traceroute_circuit(
    Path(circuit_id): Path<String>,
) -> Result<(StatusCode, Json<TracerouteCircuitResponse>), (StatusCode, String)> {
    // disabled? return Disabled
    if let Ok(cfg) = lqos_config::load_config()
        && cfg.disable_icmp_ping.unwrap_or(false)
    {
        let body = TracerouteCircuitResponse {
            circuit_id,
            result: CircuitTracerouteStatus::Disabled,
        };
        return Ok((StatusCode::OK, Json(body)));
    }

    let Some(addr) = choose_host_ip_for_circuit(&circuit_id) else {
        return Err((
            StatusCode::NOT_FOUND,
            format!(
                "Circuit '{}' not found or has no host IPs (/32 or /128)",
                circuit_id
            ),
        ));
    };

    let hops = trace_with_trippy(addr).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Traceroute error: {}", e),
        )
    })?;

    let body = TracerouteCircuitResponse {
        circuit_id,
        result: CircuitTracerouteStatus::Trace { hops },
    };
    Ok((StatusCode::OK, Json(body)))
}

async fn trace_with_trippy(addr: IpAddr) -> anyhow::Result<Vec<CircuitTraceHop>> {
    use trippy_core::{Builder, MultipathStrategy, PrivilegeMode, Protocol};

    let builder = Builder::new(addr)
        .protocol(Protocol::Icmp)
        .privilege_mode(PrivilegeMode::Privileged)
        .multipath_strategy(MultipathStrategy::Classic)
        .first_ttl(1)
        .max_ttl(30)
        .max_rounds(Some(3))
        .min_round_duration(Duration::from_millis(400))
        .max_round_duration(Duration::from_millis(400));

    let state = tokio::task::spawn_blocking(move || {
        let tracer = builder.build()?;
        tracer.run()?;
        let state = tracer.snapshot();
        anyhow::Ok(state)
    })
    .await
    .map_err(|e| anyhow::anyhow!("join error: {}", e))??;

    let hops_slice = state.hops();
    let mut out: Vec<CircuitTraceHop> = Vec::new();
    for hop in hops_slice {
        let ttl = hop.ttl();
        let nodes = hop
            .addrs()
            .map(|ip| CircuitTraceHopNode {
                ip: ip.to_string(),
                rtts_ms: hop
                    .samples()
                    .iter()
                    .map(|d| d.as_secs_f32() * 1000.0)
                    .collect(),
            })
            .collect::<Vec<_>>();
        out.push(CircuitTraceHop {
            ttl,
            nodes,
            loss_pct: hop.loss_pct(),
            forward_loss_pct: hop.forward_loss_pct(),
            backward_loss_pct: hop.backward_loss_pct(),
        });
    }
    Ok(out)
}

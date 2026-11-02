use std::net::IpAddr;
use std::time::Duration;

use axum::{Json, extract::Path, http::StatusCode};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct HopNodeTransit {
    /// IP address for this hop's responding node
    pub ip: String,
    /// Round-trip times in milliseconds for each probe that reached this node
    pub rtts_ms: Vec<f32>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct HopTransit {
    /// Time-to-live (hop number)
    pub ttl: u8,
    /// Responding nodes at this hop (if any); empty implies all probes timed out
    pub nodes: Vec<HopNodeTransit>,
    /// Overall loss percentage for this hop (0-100)
    pub loss_pct: f64,
    /// Forward loss percentage for this hop (0-100)
    pub forward_loss_pct: f64,
    /// Backward loss percentage for this hop (0-100)
    pub backward_loss_pct: f64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TracerouteStatus {
    /// Traceroute executed; contains hop-by-hop results
    Trace { hops: Vec<HopTransit> },
    /// ICMP/traceroute disabled in configuration
    Disabled,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct TracerouteIpResponse {
    /// Target IP address
    pub ip_address: String,
    /// Trace result or disabled status
    pub result: TracerouteStatus,
}

#[utoipa::path(
    get,
    path = "/traceroute_ip/{ip_address}",
    summary = "Traceroute to a specific IP address and return hop details",
    description = "Traceroute to a specific IP address and return hop details.",
    responses(
        (status = 200, description = "Traceroute hops for an IP or disabled status", body = TracerouteIpResponse),
        (status = 400, description = "Invalid IP address", body = String),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
        (status = 500, description = "Internal error", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
        ("ip_address" = String, Path, description = "IP address to traceroute (IPv4 or IPv6)"),
    ),
)]
pub async fn traceroute_ip(
    Path(ip_address): Path<String>,
) -> Result<(StatusCode, Json<TracerouteIpResponse>), (StatusCode, String)> {
    let addr: IpAddr = ip_address.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid IP address: {}", e),
        )
    })?;

    // Respect disable flag (same as ping)
    if let Ok(cfg) = lqos_config::load_config()
        && cfg.disable_icmp_ping.unwrap_or(false)
    {
        let body = TracerouteIpResponse {
            ip_address,
            result: TracerouteStatus::Disabled,
        };
        return Ok((StatusCode::OK, Json(body)));
    }

    let hops = trace_with_trippy(addr).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Traceroute error: {}", e),
        )
    })?;

    let body = TracerouteIpResponse {
        ip_address: addr.to_string(),
        result: TracerouteStatus::Trace { hops },
    };
    Ok((StatusCode::OK, Json(body)))
}

async fn trace_with_trippy(addr: IpAddr) -> anyhow::Result<Vec<HopTransit>> {
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

    // Run the blocking tracer on a blocking thread and capture snapshot
    let state = tokio::task::spawn_blocking(move || {
        let tracer = builder.build()?;
        tracer.run()?;
        let state = tracer.snapshot();
        anyhow::Ok(state)
    })
    .await
    .map_err(|e| anyhow::anyhow!("join error: {}", e))??;

    // Extract hops
    let hops_slice = state.hops();
    let mut out: Vec<HopTransit> = Vec::new();
    for hop in hops_slice {
        let ttl = hop.ttl();
        let nodes = hop
            .addrs()
            .map(|ip| HopNodeTransit {
                ip: ip.to_string(),
                rtts_ms: hop
                    .samples()
                    .iter()
                    .map(|d| d.as_secs_f32() * 1000.0)
                    .collect(),
            })
            .collect::<Vec<_>>();
        out.push(HopTransit {
            ttl,
            nodes,
            loss_pct: hop.loss_pct(),
            forward_loss_pct: hop.forward_loss_pct(),
            backward_loss_pct: hop.backward_loss_pct(),
        });
    }
    Ok(out)
}

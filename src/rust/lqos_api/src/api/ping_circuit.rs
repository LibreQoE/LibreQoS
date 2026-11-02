use axum::{Json, extract::Path, http::StatusCode};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::services::shaped_devices;

/// Result of attempting to ping a circuit's host IPs
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PingCircuitStatus {
    /// At least one host responded to ping
    Up {
        /// IP address that responded
        ip: String,
        /// Round trip time in milliseconds
        rtt_ms: f32,
        /// Optional label (device name) associated with the IP
        label: Option<String>,
    },
    /// None of the probed IPs responded within the timeout
    Down,
    /// ICMP ping disabled in configuration
    Disabled,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct PingCircuitResponse {
    /// The circuit ID that was probed
    pub circuit_id: String,
    /// Outcome of the ping attempt
    pub result: PingCircuitStatus,
}

fn collect_host_ips_for_circuit(circuit_id: &str) -> Vec<(std::net::IpAddr, Option<String>)> {
    // Load shaped devices and collect up to a few host IPs (/32 or /128) for the circuit
    let mut out: Vec<(std::net::IpAddr, Option<String>)> = Vec::new();
    if let Ok(Some(cfg)) = shaped_devices::load_optional() {
        for d in cfg
            .devices
            .into_iter()
            .filter(|d| d.circuit_id == circuit_id)
        {
            // prefer host IPs only
            for (ip, prefix) in d.ipv4.iter() {
                if *prefix == 32 {
                    out.push((std::net::IpAddr::V4(*ip), Some(d.device_name.clone())));
                }
            }
            for (ip, prefix) in d.ipv6.iter() {
                if *prefix == 128 {
                    out.push((std::net::IpAddr::V6(*ip), Some(d.device_name.clone())));
                }
            }
            if out.len() >= 3 {
                break;
            }
        }
    }
    out.truncate(3);
    out
}

async fn one_ping(ip: std::net::IpAddr) -> Option<std::time::Duration> {
    // Respect global configuration toggle
    if let Ok(cfg) = lqos_config::load_config()
        && cfg.disable_icmp_ping.unwrap_or(false)
    {
        return None;
    }

    use rand::random;
    use surge_ping::{Client, Config, ICMP, IcmpPacket, PingIdentifier, PingSequence};

    let client = match ip {
        std::net::IpAddr::V4(_) => Client::new(&Config::default()),
        std::net::IpAddr::V6(_) => Client::new(&Config::builder().kind(ICMP::V6).build()),
    };
    let client = match client {
        Ok(c) => c,
        Err(_) => return None,
    };

    let payload = [0; 32];
    let mut pinger = client.pinger(ip, PingIdentifier(random())).await;
    // Short timeout as requested (< 500ms)
    pinger.timeout(std::time::Duration::from_millis(400));
    match pinger.ping(PingSequence(0), &payload).await {
        Ok((IcmpPacket::V4(..), dur)) | Ok((IcmpPacket::V6(..), dur)) => Some(dur),
        _ => None,
    }
}

#[utoipa::path(
    get,
    path = "/ping_circuit/{circuit_id}",
    summary = "Ping representative host IPs for a circuit and return up/down status",
    description = "Ping representative host IPs for a circuit and return up/down status.",
    responses(
        (status = 200, description = "Ping result for a circuit (Up/Down/Disabled)", body = PingCircuitResponse),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
        (status = 404, description = "Circuit not found or has no host IPs", body = String),
        (status = 500, description = "Internal error", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
        ("circuit_id" = String, Path, description = "Circuit ID to ping"),
    ),
)]
pub async fn ping_circuit(
    Path(circuit_id): Path<String>,
) -> Result<(StatusCode, Json<PingCircuitResponse>), (StatusCode, String)> {
    let candidates = collect_host_ips_for_circuit(&circuit_id);
    if candidates.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            format!(
                "Circuit '{}' not found or has no host IPs (/32 or /128)",
                circuit_id
            ),
        ));
    }

    // Try up to 3 candidates; return Up on first success. If config disables ICMP, return Disabled.
    // Check ICMP disabled state explicitly so we can return Disabled without trying.
    if let Ok(cfg) = lqos_config::load_config()
        && cfg.disable_icmp_ping.unwrap_or(false)
    {
        let body = PingCircuitResponse {
            circuit_id,
            result: PingCircuitStatus::Disabled,
        };
        return Ok((StatusCode::OK, Json(body)));
    }

    for (ip, label) in candidates.into_iter() {
        if let Some(dur) = one_ping(ip).await {
            let body = PingCircuitResponse {
                circuit_id: circuit_id.clone(),
                result: PingCircuitStatus::Up {
                    ip: ip.to_string(),
                    rtt_ms: dur.as_secs_f32() * 1000.0,
                    label,
                },
            };
            return Ok((StatusCode::OK, Json(body)));
        }
    }

    let body = PingCircuitResponse {
        circuit_id,
        result: PingCircuitStatus::Down,
    };
    Ok((StatusCode::OK, Json(body)))
}

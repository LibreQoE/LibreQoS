use axum::{Json, extract::Path, http::StatusCode};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Result of attempting to ping a specific IP address
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PingIpStatus {
    /// Host responded to ping
    Up {
        /// IP address that responded
        ip: String,
        /// Round trip time in milliseconds
        rtt_ms: f32,
        /// Optional label (not used for direct IP)
        label: Option<String>,
    },
    /// No responses within timeout window
    Down,
    /// ICMP ping disabled in configuration
    Disabled,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct PingIpResponse {
    /// The IP address that was probed
    pub ip_address: String,
    /// Outcome of the ping attempt
    pub result: PingIpStatus,
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
    // Short timeout (<500ms)
    pinger.timeout(std::time::Duration::from_millis(400));
    match pinger.ping(PingSequence(0), &payload).await {
        Ok((IcmpPacket::V4(..), dur)) | Ok((IcmpPacket::V6(..), dur)) => Some(dur),
        _ => None,
    }
}

#[utoipa::path(
    get,
    path = "/ping_ip/{ip_address}",
    summary = "Ping a specific IP address and return up/down status",
    description = "Ping a specific IP address and return up/down status.",
    responses(
        (status = 200, description = "Ping result for an IP (Up/Down/Disabled)", body = PingIpResponse),
        (status = 400, description = "Invalid IP address", body = String),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
        (status = 500, description = "Internal error", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
        ("ip_address" = String, Path, description = "IP address to ping (IPv4 or IPv6)"),
    ),
)]
pub async fn ping_ip(
    Path(ip_address): Path<String>,
) -> Result<(StatusCode, Json<PingIpResponse>), (StatusCode, String)> {
    let ip: std::net::IpAddr = ip_address.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid IP address: {}", e),
        )
    })?;

    // Early Disabled signal
    if let Ok(cfg) = lqos_config::load_config()
        && cfg.disable_icmp_ping.unwrap_or(false)
    {
        let body = PingIpResponse {
            ip_address,
            result: PingIpStatus::Disabled,
        };
        return Ok((StatusCode::OK, Json(body)));
    }

    // Up to 3 attempts with short timeout
    for _ in 0..3 {
        if let Some(dur) = one_ping(ip).await {
            let body = PingIpResponse {
                ip_address: ip.to_string(),
                result: PingIpStatus::Up {
                    ip: ip.to_string(),
                    rtt_ms: dur.as_secs_f32() * 1000.0,
                    label: None,
                },
            };
            return Ok((StatusCode::OK, Json(body)));
        }
    }

    let body = PingIpResponse {
        ip_address: ip.to_string(),
        result: PingIpStatus::Down,
    };
    Ok((StatusCode::OK, Json(body)))
}

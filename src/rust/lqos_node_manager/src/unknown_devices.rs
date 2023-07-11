use std::net::IpAddr;

use crate::{
  auth_guard::AuthGuard, cache_control::NoCache, tracker::SHAPED_DEVICES
};
use lqos_bus::{IpStats, bus_request, BusRequest, BusResponse};
use rocket::serde::json::Json;

async fn unknown_devices() -> Vec<IpStats> {
  if let Ok(messages) = bus_request(vec![BusRequest::AllUnknownIps]).await {
    for msg in messages {
      if let BusResponse::AllUnknownIps(unknowns) = msg {
        let cfg = SHAPED_DEVICES.read().unwrap();
        let really_unknown: Vec<IpStats> = unknowns
          .iter()
          .filter(|ip| {
            if let Ok(ip) = ip.ip_address.parse::<IpAddr>() {
              let lookup = match ip {
                IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                IpAddr::V6(ip) => ip,
              };
              cfg.trie.longest_match(lookup).is_none()
            } else {
              false
            }
          })
          .cloned()
          .collect();
        return really_unknown;
      }
    }
  }

  Vec::new()
}

#[get("/api/all_unknown_devices")]
pub async fn all_unknown_devices(_auth: AuthGuard) -> NoCache<Json<Vec<IpStats>>> {
  NoCache::new(Json(unknown_devices().await))
}

#[get("/api/unknown_devices_count")]
pub async fn unknown_devices_count(_auth: AuthGuard) -> NoCache<Json<usize>> {
  NoCache::new(Json(unknown_devices().await.len()))
}

#[get("/api/unknown_devices_range/<start>/<end>")]
pub async fn unknown_devices_range(
  start: usize,
  end: usize,
  _auth: AuthGuard,
) -> NoCache<Json<Vec<IpStats>>> {
  let reader = unknown_devices().await;
  let result: Vec<IpStats> =
    reader.iter().skip(start).take(end).cloned().collect();
  NoCache::new(Json(result))
}

#[get("/api/unknown_devices_csv")]
pub async fn unknown_devices_csv(_auth: AuthGuard) -> NoCache<String> {
  let mut result = "IP Address,Download,Upload\n".to_string();
  let reader = unknown_devices().await;

  for unknown in reader.iter() {
    result += &format!("{},{},{}\n", unknown.ip_address, unknown.bits_per_second.0, unknown.bits_per_second.1);
  }
  NoCache::new(result)
}

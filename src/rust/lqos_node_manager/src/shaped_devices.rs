use std::sync::atomic::AtomicBool;

use crate::auth_guard::AuthGuard;
use crate::cache_control::NoCache;
use crate::tracker::SHAPED_DEVICES;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use lqos_config::ShapedDevice;
use rocket::serde::json::Json;

static RELOAD_REQUIRED: AtomicBool = AtomicBool::new(false);

#[get("/api/all_shaped_devices")]
pub fn all_shaped_devices(
  _auth: AuthGuard,
) -> NoCache<Json<Vec<ShapedDevice>>> {
  NoCache::new(Json(SHAPED_DEVICES.read().devices.clone()))
}

#[get("/api/shaped_devices_count")]
pub fn shaped_devices_count(_auth: AuthGuard) -> NoCache<Json<usize>> {
  NoCache::new(Json(SHAPED_DEVICES.read().devices.len()))
}

#[get("/api/shaped_devices_range/<start>/<end>")]
pub fn shaped_devices_range(
  start: usize,
  end: usize,
  _auth: AuthGuard,
) -> NoCache<Json<Vec<ShapedDevice>>> {
  let reader = SHAPED_DEVICES.read();
  let result: Vec<ShapedDevice> =
    reader.devices.iter().skip(start).take(end).cloned().collect();
  NoCache::new(Json(result))
}

#[get("/api/shaped_devices_search/<term>")]
pub fn shaped_devices_search(
  term: String,
  _auth: AuthGuard,
) -> NoCache<Json<Vec<ShapedDevice>>> {
  let term = term.trim().to_lowercase();
  let reader = SHAPED_DEVICES.read();
  let result: Vec<ShapedDevice> = reader
    .devices
    .iter()
    .filter(|s| {
      s.circuit_name.trim().to_lowercase().contains(&term)
        || s.device_name.trim().to_lowercase().contains(&term)
    })
    .cloned()
    .collect();
  NoCache::new(Json(result))
}

#[get("/api/reload_required")]
pub fn reload_required() -> NoCache<Json<bool>> {
  NoCache::new(Json(
    RELOAD_REQUIRED.load(std::sync::atomic::Ordering::Relaxed),
  ))
}

#[get("/api/reload_libreqos")]
pub async fn reload_libreqos(auth: AuthGuard) -> NoCache<Json<String>> {
  if auth != AuthGuard::Admin {
    return NoCache::new(Json("Not authorized".to_string()));
  }
  // Send request to lqosd
  let responses = bus_request(vec![BusRequest::ReloadLibreQoS]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::ReloadLibreQoS(msg) => msg.clone(),
    _ => "Unable to reload LibreQoS".to_string(),
  };

  RELOAD_REQUIRED.store(false, std::sync::atomic::Ordering::Relaxed);
  NoCache::new(Json(result))
}

use lqos_bus::IpStats;
use rocket::serde::json::Json;
use crate::{cache_control::NoCache, tracker::UNKNOWN_DEVICES, auth_guard::AuthGuard};

#[get("/api/all_unknown_devices")]
pub fn all_unknown_devices(_auth: AuthGuard) -> NoCache<Json<Vec<IpStats>>> {
    NoCache::new(Json(UNKNOWN_DEVICES.read().clone()))
}

#[get("/api/unknown_devices_count")]
pub fn unknown_devices_count(_auth: AuthGuard) -> NoCache<Json<usize>> {
    NoCache::new(Json(UNKNOWN_DEVICES.read().len()))
}

#[get("/api/unknown_devices_range/<start>/<end>")]
pub fn unknown_devices_range(start: usize, end: usize, _auth: AuthGuard) -> NoCache<Json<Vec<IpStats>>> {
    let reader = UNKNOWN_DEVICES.read();
    let result: Vec<IpStats> = reader.iter().skip(start).take(end).cloned().collect();
    NoCache::new(Json(result))
}

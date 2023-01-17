use crate::{auth_guard::AuthGuard, cache_control::NoCache, tracker::UNKNOWN_DEVICES};
use lqos_bus::IpStats;
use rocket::serde::json::Json;

#[get("/api/all_unknown_devices")]
pub fn all_unknown_devices(_auth: AuthGuard) -> NoCache<Json<Vec<IpStats>>> {
    NoCache::new(Json(UNKNOWN_DEVICES.read().clone()))
}

#[get("/api/unknown_devices_count")]
pub fn unknown_devices_count(_auth: AuthGuard) -> NoCache<Json<usize>> {
    NoCache::new(Json(UNKNOWN_DEVICES.read().len()))
}

#[get("/api/unknown_devices_range/<start>/<end>")]
pub fn unknown_devices_range(
    start: usize,
    end: usize,
    _auth: AuthGuard,
) -> NoCache<Json<Vec<IpStats>>> {
    let reader = UNKNOWN_DEVICES.read();
    let result: Vec<IpStats> = reader.iter().skip(start).take(end).cloned().collect();
    NoCache::new(Json(result))
}

#[get("/api/unknown_devices_csv")]
pub fn unknown_devices_csv(_auth: AuthGuard) -> NoCache<String> {
    let mut result = String::new();
    let reader = UNKNOWN_DEVICES.read();

    for unknown in reader.iter() {
        result += &format!("{}\n", unknown.ip_address);
    }
    NoCache::new(result)
}

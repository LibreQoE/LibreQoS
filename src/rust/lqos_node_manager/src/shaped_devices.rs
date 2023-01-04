use lqos_bus::{BusResponse, BUS_BIND_ADDRESS, BusSession, BusRequest, encode_request, decode_response};
use lqos_config::ShapedDevice;
use rocket::serde::json::Json;
use rocket::tokio::io::{AsyncWriteExt, AsyncReadExt};
use rocket::tokio::net::TcpStream;
use crate::cache_control::NoCache;
use crate::tracker::SHAPED_DEVICES;
use lazy_static::*;
use parking_lot::RwLock;

lazy_static! {
    static ref RELOAD_REQUIRED : RwLock<bool> = RwLock::new(false);
}

#[get("/api/all_shaped_devices")]
pub fn all_shaped_devices() -> NoCache<Json<Vec<ShapedDevice>>> {
    NoCache::new(Json(SHAPED_DEVICES.read().devices.clone()))
}

#[get("/api/shaped_devices_count")]
pub fn shaped_devices_count() -> NoCache<Json<usize>> {
    NoCache::new(Json(SHAPED_DEVICES.read().devices.len()))
}

#[get("/api/shaped_devices_range/<start>/<end>")]
pub fn shaped_devices_range(start: usize, end: usize) -> NoCache<Json<Vec<ShapedDevice>>> {
    let reader = SHAPED_DEVICES.read();
    let result: Vec<ShapedDevice> = reader.devices.iter().skip(start).take(end).cloned().collect();
    NoCache::new(Json(result))
}

#[get("/api/shaped_devices_search/<term>")]
pub fn shaped_devices_search(term: String) -> NoCache<Json<Vec<ShapedDevice>>> {
    let term = term.trim().to_lowercase();
    let reader = SHAPED_DEVICES.read();
    let result: Vec<ShapedDevice> = reader
        .devices
        .iter()
        .filter(|s| 
            s.circuit_name.trim().to_lowercase().contains(&term) ||
            s.device_name.trim().to_lowercase().contains(&term)
        )
        .cloned()
        .collect();
    NoCache::new(Json(result))
}

#[get("/api/reload_required")]
pub fn reload_required() -> NoCache<Json<bool>> {
    NoCache::new(Json(*RELOAD_REQUIRED.read()))
}

#[get("/api/reload_libreqos")]
pub async fn reload_libreqos() -> NoCache<Json<String>> {
    // Send request to lqosd
    let mut stream = TcpStream::connect(BUS_BIND_ADDRESS).await.unwrap();
    let test = BusSession {
        auth_cookie: 1234,
        requests: vec![
            BusRequest::ReloadLibreQoS,
        ],
    };
    let msg = encode_request(&test).unwrap();
    stream.write(&msg).await.unwrap();

    // Receive reply
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await.unwrap();
    let reply = decode_response(&buf).unwrap();

    let result = match &reply.responses[0] {
        BusResponse::ReloadLibreQoS(msg) => msg.clone(),
        _ => "Unable to reload LibreQoS".to_string(),
    };

    *RELOAD_REQUIRED.write() = false;
    NoCache::new(Json(result))
}
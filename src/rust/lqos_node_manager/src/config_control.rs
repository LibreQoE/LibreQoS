use crate::{auth_guard::AuthGuard, cache_control::NoCache};
use default_net::get_interfaces;
use lqos_bus::{BusRequest, bus_request};
use lqos_config::{EtcLqos, LibreQoSConfig, Tunables};
use rocket::{
    fs::NamedFile,
    serde::json::Json,
};

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/config")]
pub async fn config_page<'a>(_auth: AuthGuard) -> NoCache<Option<NamedFile>> {
    NoCache::new(NamedFile::open("static/config.html").await.ok())
}

#[get("/api/list_nics")]
pub async fn get_nic_list<'a>(_auth: AuthGuard) -> NoCache<Json<Vec<(String, String, String)>>> {
    let mut result = Vec::new();
    for eth in get_interfaces().iter() {
        let mac = if let Some(mac) = &eth.mac_addr {
            mac.to_string()
        } else {
            String::new()
        };
        result.push((eth.name.clone(), format!("{:?}", eth.if_type), mac));
    }
    NoCache::new(Json(result))
}

#[get("/api/python_config")]
pub async fn get_current_python_config(_auth: AuthGuard) -> NoCache<Json<LibreQoSConfig>> {
    let config = lqos_config::LibreQoSConfig::load().unwrap();
    println!("{:#?}", config);
    NoCache::new(Json(config))
}

#[get("/api/lqosd_config")]
pub async fn get_current_lqosd_config(_auth: AuthGuard) -> NoCache<Json<EtcLqos>> {
    let config = lqos_config::EtcLqos::load().unwrap();
    println!("{:#?}", config);
    NoCache::new(Json(config))
}

#[post("/api/python_config", data = "<config>")]
pub async fn update_python_config(_auth: AuthGuard, config: Json<LibreQoSConfig>) -> Json<String> {
    config.save().unwrap();
    Json("OK".to_string())
}

#[post("/api/lqos_tuning/<period>", data = "<tuning>")]
pub async fn update_lqos_tuning(
    auth: AuthGuard,
    period: u64,
    tuning: Json<Tunables>,
) -> Json<String> {
    if auth != AuthGuard::Admin {
        return Json("Error: Not authorized".to_string());
    }

    // Send the update to the server
    bus_request(vec![BusRequest::UpdateLqosDTuning(period, (*tuning).clone())]).await.unwrap();

    // For now, ignore the reply.

    Json("OK".to_string())
}

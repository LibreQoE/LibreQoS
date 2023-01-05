use default_net::get_interfaces;
use lqos_config::{LibreQoSConfig, EtcLqos};
use rocket::{fs::NamedFile, serde::json::Json};
use crate::cache_control::NoCache;

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/config")]
pub async fn config_page<'a>() -> NoCache<Option<NamedFile>> {
    NoCache::new(NamedFile::open("static/config.html").await.ok())
}

#[get("/api/list_nics")]
pub async fn get_nic_list<'a>() -> NoCache<Json<Vec<(String, String, String)>>> {
    let mut result = Vec::new();
    for eth in get_interfaces().iter() {
        let mac = if let Some(mac) = &eth.mac_addr {
            mac.to_string()
        } else {
            String::new()
        };
        result.push((
            eth.name.clone(),
            format!("{:?}", eth.if_type),
            mac,  
        ));
    }
    NoCache::new(Json(result))
}

#[get("/api/python_config")]
pub async fn get_current_python_config() -> NoCache<Json<LibreQoSConfig>> {
    let config = lqos_config::LibreQoSConfig::load().unwrap();
    println!("{:#?}", config);
    NoCache::new(Json(config))
}

#[get("/api/lqosd_config")]
pub async fn get_current_lqosd_config() -> NoCache<Json<EtcLqos>> {
    let config = lqos_config::EtcLqos::load().unwrap();
    println!("{:#?}", config);
    NoCache::new(Json(config))
}
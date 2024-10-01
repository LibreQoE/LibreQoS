use axum::{Extension, Json};
use axum::http::StatusCode;
use lqos_config::{Config, ShapedDevice};
use crate::node_manager::auth::LoginResult;
use default_net::get_interfaces;
use serde::Deserialize;
use serde_json::Value;
use lqos_bus::{bus_request, BusRequest};
use crate::shaped_devices_tracker::SHAPED_DEVICES;

pub async fn admin_check(
    Extension(login): Extension<LoginResult>
) -> Json<bool> {
    match login {
        LoginResult::Admin => Json(true),
        _ => Json(false),
    }
}

pub async fn get_config(
    Extension(login): Extension<LoginResult>
) -> Result<Json<Config>, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let config = lqos_config::load_config().unwrap();
    Ok(Json(config))
}

pub async fn list_nics(
    Extension(login): Extension<LoginResult>
) -> Result<Json<Vec<(String, String, String)>>, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let result = get_interfaces()
        .iter()
        .map(|eth| {
            let mac = if let Some(mac) = &eth.mac_addr {
                mac.to_string()
            } else {
                String::new()
            };
            (eth.name.clone(), format!("{:?}", eth.if_type), mac)
        })
        .collect();
    Ok(Json(result))
}

pub async fn network_json()-> Json<Value> {
    if let Ok(config) = lqos_config::load_config() {
        let path = std::path::Path::new(&config.lqos_directory).join("network.json");
        if path.exists() {
            let raw = std::fs::read_to_string(path).unwrap();
            let json: Value = serde_json::from_str(&raw).unwrap();
            return Json(json);
        }
    }

    Json(Value::String("Not done yet".to_string()))
}

pub async fn all_shaped_devices() -> Json<Vec<ShapedDevice>> {
    Json(SHAPED_DEVICES.read().unwrap().devices.clone())
}

pub async fn update_lqosd_config(
    Extension(login): Extension<LoginResult>,
    data: Json<Config>
) -> String {
    if login != LoginResult::Admin {
        return "Unauthorized".to_string();
    }
    let config: Config = (*data).clone();
    bus_request(vec![BusRequest::UpdateLqosdConfig(Box::new(config))])
        .await
        .unwrap();
    "Ok".to_string()
}

#[derive(Deserialize, Clone)]
pub struct NetworkAndDevices {
    shaped_devices: Vec<ShapedDevice>,
    network_json: Value,
}

pub async fn update_network_and_devices(
    Extension(login): Extension<LoginResult>,
    data: Json<NetworkAndDevices>
) -> String {
    if login != LoginResult::Admin {
        return "Unauthorized".to_string();
    }

    let config = lqos_config::load_config().unwrap();

    // Save network.json
    let serialized_string = serde_json::to_string_pretty(&data.network_json).unwrap();
    let net_json_path = std::path::Path::new(&config.lqos_directory).join("network.json");
    let net_json_backup_path = std::path::Path::new(&config.lqos_directory).join("network.json.backup");
    if net_json_path.exists() {
        // Make a backup
        std::fs::copy(&net_json_path, net_json_backup_path).unwrap();
    }
    std::fs::write(net_json_path, serialized_string).unwrap();

    // Save the Shaped Devices
    let sd_path = std::path::Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let sd_backup_path = std::path::Path::new(&config.lqos_directory).join("ShapedDevices.csv.backup");
    if sd_path.exists() {
        std::fs::copy(&sd_path, sd_backup_path).unwrap();
    }
    let mut lock = SHAPED_DEVICES.write().unwrap();
    lock.replace_with_new_data(data.shaped_devices.clone());
    //println!("{:?}", lock.devices);
    lock.write_csv(&format!("{}/ShapedDevices.csv", config.lqos_directory)).unwrap();

    "Ok".to_string()
}
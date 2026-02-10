use crate::node_manager::auth::LoginResult;
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use axum::http::StatusCode;
use default_net::get_interfaces;
use lqos_bus::{BusRequest, bus_request};
use lqos_config::{Config, ConfigShapedDevices, ShapedDevice, WebUser, WebUsers};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub fn admin_check_data(login: LoginResult) -> bool {
    matches!(login, LoginResult::Admin)
}

pub fn get_config_data(login: LoginResult) -> Result<Config, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    lqos_config::load_config()
        .map(|config| (*config).clone())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub fn list_nics_data(login: LoginResult) -> Result<Vec<(String, String, String)>, StatusCode> {
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
    Ok(result)
}

pub fn network_json_data() -> Value {
    if let Ok(config) = lqos_config::load_config() {
        let path = std::path::Path::new(&config.lqos_directory).join("network.json");
        if path.exists() {
            let raw = std::fs::read_to_string(path).expect("Unable to read network json");
            let json: Value = serde_json::from_str(&raw).expect("Unable to read network json");
            return json;
        }
    }

    Value::String("Not done yet".to_string())
}

pub fn all_shaped_devices_data() -> Vec<ShapedDevice> {
    SHAPED_DEVICES.load().devices.clone()
}

pub async fn update_lqosd_config_data(
    login: LoginResult,
    config: Config,
) -> Result<(), StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    bus_request(vec![BusRequest::UpdateLqosdConfig(Box::new(config))])
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}

pub fn update_network_and_devices_data(
    login: LoginResult,
    network_json: Value,
    shaped_devices: Vec<ShapedDevice>,
) -> Result<(), StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let config = lqos_config::load_config().expect("Unable to load LibreQoS config");

    // Save network.json
    let serialized_string = serde_json::to_string_pretty(&network_json)
        .expect("Unable to serialize network.json payload");
    let net_json_path = std::path::Path::new(&config.lqos_directory).join("network.json");
    let net_json_backup_path =
        std::path::Path::new(&config.lqos_directory).join("network.json.backup");
    if net_json_path.exists() {
        // Make a backup
        std::fs::copy(&net_json_path, net_json_backup_path)
            .expect("Unable to create network.json backup");
    }
    std::fs::write(net_json_path, serialized_string).expect("Unable to write network.json");

    // Save the Shaped Devices
    let sd_path = std::path::Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let sd_backup_path =
        std::path::Path::new(&config.lqos_directory).join("ShapedDevices.csv.backup");
    if sd_path.exists() {
        std::fs::copy(&sd_path, sd_backup_path).expect("Unable to create ShapedDevices.csv backup");
    }
    let mut copied = ConfigShapedDevices::default();
    copied.replace_with_new_data(shaped_devices);
    copied
        .write_csv(&format!("{}/ShapedDevices.csv", config.lqos_directory))
        .expect("Unable to write ShapedDevices.csv");
    SHAPED_DEVICES.store(Arc::new(copied));

    Ok(())
}

pub fn get_users_data(login: LoginResult) -> Result<Vec<WebUser>, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let users = WebUsers::load_or_create().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(users.get_users())
}

pub fn add_user_data(login: LoginResult, data: UserRequest) -> Result<String, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if data.username.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let password = match data.password.as_deref() {
        Some(p) if !p.is_empty() => p,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let mut users = WebUsers::load_or_create().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    users
        .add_or_update_user(&data.username.trim(), password, data.role.into())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(format!("User '{}' added", data.username))
}

pub fn update_user_data(login: LoginResult, data: UserRequest) -> Result<String, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut users = WebUsers::load_or_create().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let password = data.password.as_deref().filter(|p| !p.is_empty());
    users
        .update_user_with_optional_password(&data.username, password, data.role.into())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok("User updated".to_string())
}

pub fn delete_user_data(login: LoginResult, username: String) -> Result<String, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut users = WebUsers::load_or_create().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    users
        .remove_user(&username)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok("User deleted".to_string())
}

#[derive(Serialize, Deserialize)]
pub struct UserRequest {
    pub username: String,
    pub password: Option<String>,
    pub role: String,
}

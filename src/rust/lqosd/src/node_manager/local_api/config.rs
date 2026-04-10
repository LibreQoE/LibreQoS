use crate::node_manager::auth::LoginResult;
use axum::http::StatusCode;
use default_net::get_interfaces;
use lqos_bus::{BusRequest, bus_request};
use lqos_config::{Config, ConfigShapedDevices, ShapedDevice, UserRole, WebUser, WebUsers};
use lqos_utils::hash_to_i64;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};

type TopologySourceIntegration = (&'static str, fn(&Config) -> bool);

const TOPOLOGY_SOURCE_INTEGRATIONS: [TopologySourceIntegration; 7] = [
    ("UISP", |config| config.uisp_integration.enable_uisp),
    ("Splynx", |config| config.splynx_integration.enable_splynx),
    ("Powercode", |config| {
        config.powercode_integration.enable_powercode
    }),
    ("Sonar", |config| config.sonar_integration.enable_sonar),
    ("Netzur", |config| {
        config
            .netzur_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_netzur)
    }),
    ("VISP", |config| {
        config
            .visp_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_visp)
    }),
    ("WispGate", |config| {
        config
            .wispgate_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_wispgate)
    }),
];

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

    // Some systems can report the same interface more than once. The UI keys
    // off interface name, so dedupe by name here before returning results.
    let mut deduped: BTreeMap<String, (String, String)> = BTreeMap::new();
    for eth in get_interfaces() {
        let mac = eth
            .mac_addr
            .map(|m| m.to_string())
            .unwrap_or_else(String::new);
        let if_type = format!("{:?}", eth.if_type);

        deduped
            .entry(eth.name)
            .and_modify(|(existing_type, existing_mac)| {
                if existing_mac.is_empty() && !mac.is_empty() {
                    *existing_mac = mac.clone();
                }
                if existing_type == "Unknown" && if_type != "Unknown" {
                    *existing_type = if_type.clone();
                }
            })
            .or_insert((if_type, mac));
    }

    let result = deduped
        .into_iter()
        .map(|(name, (if_type, mac))| (name, if_type, mac))
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
    lqos_network_devices::shaped_devices_snapshot().devices.clone()
}

/// Returns the enabled integration names that act as the source of truth for
/// `network.json` and `ShapedDevices.csv`.
pub fn active_topology_source_integrations(config: &Config) -> Vec<&'static str> {
    TOPOLOGY_SOURCE_INTEGRATIONS
        .iter()
        .filter_map(|(name, enabled)| enabled(config).then_some(*name))
        .collect()
}

fn topology_editor_lock_message() -> Result<Option<String>, String> {
    let config =
        lqos_config::load_config().map_err(|e| format!("Unable to load LibreQoS config: {e}"))?;
    let integrations = active_topology_source_integrations(config.as_ref());
    if integrations.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        "Editing is disabled because these integrations are the source of truth: {}.",
        integrations.join(", ")
    )))
}

fn ensure_topology_editor_unlocked() -> Result<(), String> {
    if let Some(message) = topology_editor_lock_message()? {
        return Err(message);
    }
    Ok(())
}

fn validate_network_json(value: &Value) -> Result<(), String> {
    let Some(map) = value.as_object() else {
        return Err("network.json must be a JSON object".to_string());
    };

    let mut seen = HashSet::new();
    let mut duplicates = BTreeSet::new();

    fn walk(
        map: &serde_json::Map<String, Value>,
        seen: &mut HashSet<String>,
        duplicates: &mut BTreeSet<String>,
    ) {
        for (name, node) in map {
            if !seen.insert(name.clone()) {
                duplicates.insert(name.clone());
            }
            let Some(node_map) = node.as_object() else {
                continue;
            };
            let Some(children) = node_map.get("children").and_then(Value::as_object) else {
                continue;
            };
            walk(children, seen, duplicates);
        }
    }

    walk(map, &mut seen, &mut duplicates);

    if duplicates.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "network.json contains duplicate node names: {}. Node names must be globally unique.",
            duplicates.into_iter().collect::<Vec<_>>().join(", ")
        ))
    }
}

fn persist_network_json(network_json: &Value) -> Result<(), String> {
    validate_network_json(network_json)?;
    let config =
        lqos_config::load_config().map_err(|e| format!("Unable to load LibreQoS config: {e}"))?;
    let serialized_string = serde_json::to_string_pretty(network_json)
        .map_err(|e| format!("Unable to serialize network.json payload: {e}"))?;
    let net_json_path = std::path::Path::new(&config.lqos_directory).join("network.json");
    let net_json_backup_path =
        std::path::Path::new(&config.lqos_directory).join("network.json.backup");
    if net_json_path.exists() {
        std::fs::copy(&net_json_path, net_json_backup_path)
            .map_err(|e| format!("Unable to create network.json backup: {e}"))?;
    }
    std::fs::write(net_json_path, serialized_string)
        .map_err(|e| format!("Unable to write network.json: {e}"))?;
    Ok(())
}

fn normalize_sqm_override(raw: &Option<String>) -> Option<String> {
    let token = raw.as_deref().unwrap_or("").trim().to_lowercase();
    if token.is_empty() { None } else { Some(token) }
}

fn normalize_shaped_device(device: &mut ShapedDevice) {
    device.circuit_id = device.circuit_id.trim().to_string();
    device.circuit_name = device.circuit_name.trim().to_string();
    device.device_id = device.device_id.trim().to_string();
    device.device_name = device.device_name.trim().to_string();
    device.parent_node = device.parent_node.trim().to_string();
    device.mac = device.mac.trim().to_string();
    device.comment = device.comment.trim().to_string();
    device.sqm_override = normalize_sqm_override(&device.sqm_override);
}

fn populate_shaped_device_hashes(devices: &mut [ShapedDevice]) {
    for device in devices {
        device.circuit_hash = hash_to_i64(&device.circuit_id);
        device.device_hash = hash_to_i64(&device.device_id);
        device.parent_hash = hash_to_i64(&device.parent_node);
    }
}

fn collect_network_nodes(value: &Value, out: &mut HashSet<String>) {
    let Some(map) = value.as_object() else {
        return;
    };
    for (name, node) in map {
        out.insert(name.to_string());
        if let Some(children) = node.get("children") {
            collect_network_nodes(children, out);
        }
    }
}

fn validate_sqm_override(raw: &Option<String>) -> Result<(), String> {
    let Some(token) = normalize_sqm_override(raw) else {
        return Ok(());
    };
    let valid = |value: &str| matches!(value, "" | "cake" | "fq_codel" | "none");
    if token.contains('/') {
        let mut parts = token.splitn(2, '/');
        let down = parts.next().unwrap_or("").trim();
        let up = parts.next().unwrap_or("").trim();
        if valid(down) && valid(up) {
            return Ok(());
        }
    } else if valid(&token) {
        return Ok(());
    }
    Err(format!(
        "Invalid SQM override '{token}'. Allowed values: cake, fq_codel, none, or directional down/up tokens."
    ))
}

fn validate_shaped_devices(devices: &[ShapedDevice]) -> Result<(), String> {
    let network_json = network_json_data();
    let mut nodes = HashSet::new();
    collect_network_nodes(&network_json, &mut nodes);

    let mut device_ids = HashSet::new();
    let mut ipv4s = HashSet::new();
    let mut ipv6s = HashSet::new();

    for (index, device) in devices.iter().enumerate() {
        let label = if device.device_id.is_empty() {
            format!("row {}", index + 1)
        } else {
            format!("device '{}'", device.device_id)
        };

        if device.circuit_id.is_empty() {
            return Err(format!("{label}: Circuit ID is required"));
        }
        if device.circuit_name.is_empty() {
            return Err(format!("{label}: Circuit Name is required"));
        }
        if device.device_id.is_empty() {
            return Err(format!("{label}: Device ID is required"));
        }
        if device.device_name.is_empty() {
            return Err(format!("{label}: Device Name is required"));
        }
        if !device_ids.insert(device.device_id.clone()) {
            return Err(format!("Duplicate device ID '{}'", device.device_id));
        }
        if nodes.is_empty() {
            if !device.parent_node.is_empty() {
                return Err(format!(
                    "{label}: Parent node '{}' is invalid for a flat network",
                    device.parent_node
                ));
            }
        } else if !device.parent_node.is_empty() && !nodes.contains(&device.parent_node) {
            return Err(format!(
                "{label}: Parent node '{}' does not exist",
                device.parent_node
            ));
        }
        if device.ipv4.is_empty() && device.ipv6.is_empty() {
            return Err(format!(
                "{label}: At least one IPv4 or IPv6 address is required"
            ));
        }
        for (addr, prefix) in &device.ipv4 {
            let key = format!("{addr}/{prefix}");
            if !ipv4s.insert(key.clone()) {
                return Err(format!("Duplicate IPv4 entry '{key}'"));
            }
        }
        for (addr, prefix) in &device.ipv6 {
            let key = format!("{addr}/{prefix}");
            if !ipv6s.insert(key.clone()) {
                return Err(format!("Duplicate IPv6 entry '{key}'"));
            }
        }
        if !device.download_min_mbps.is_finite() || device.download_min_mbps < 0.1 {
            return Err(format!("{label}: Download min must be >= 0.1 Mbps"));
        }
        if !device.upload_min_mbps.is_finite() || device.upload_min_mbps < 0.1 {
            return Err(format!("{label}: Upload min must be >= 0.1 Mbps"));
        }
        if !device.download_max_mbps.is_finite() || device.download_max_mbps < 0.2 {
            return Err(format!("{label}: Download max must be >= 0.2 Mbps"));
        }
        if !device.upload_max_mbps.is_finite() || device.upload_max_mbps < 0.2 {
            return Err(format!("{label}: Upload max must be >= 0.2 Mbps"));
        }
        validate_sqm_override(&device.sqm_override)?;
    }

    Ok(())
}

fn persist_shaped_devices(mut devices: Vec<ShapedDevice>) -> Result<(), String> {
    for device in &mut devices {
        normalize_shaped_device(device);
    }
    validate_shaped_devices(&devices)?;
    populate_shaped_device_hashes(&mut devices);

    let config = lqos_config::load_config().expect("Unable to load LibreQoS config");
    let sd_path = std::path::Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let sd_backup_path =
        std::path::Path::new(&config.lqos_directory).join("ShapedDevices.csv.backup");
    if sd_path.exists() {
        std::fs::copy(&sd_path, sd_backup_path)
            .map_err(|e| format!("Unable to create ShapedDevices.csv backup: {e}"))?;
    }
    let mut copied = ConfigShapedDevices::default();
    copied.replace_with_new_data(devices);
    copied
        .write_csv("ShapedDevices.csv")
        .map_err(|e| format!("Unable to write ShapedDevices.csv: {e}"))?;
    lqos_network_devices::apply_shaped_devices_snapshot("node_manager:persist_shaped_devices", copied)
        .map_err(|e| format!("Unable to publish ShapedDevices.csv snapshot: {e}"))?;

    Ok(())
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

/// Persists both `network.json` and `ShapedDevices.csv` for administrative
/// edits.
///
/// Returns an error string when the caller is unauthorized, when integration-
/// managed topology editing is locked, or when validation/persistence fails.
pub fn update_network_and_devices_data(
    login: LoginResult,
    network_json: Value,
    shaped_devices: Vec<ShapedDevice>,
) -> Result<(), String> {
    if login != LoginResult::Admin {
        return Err("Unauthorized".to_string());
    }
    ensure_topology_editor_unlocked()?;
    persist_network_json(&network_json)?;
    persist_shaped_devices(shaped_devices)?;
    lqos_network_devices::request_reload_network_json("node_manager:update_network_and_devices")
        .map_err(|e| format!("Unable to reload network.json: {e}"))?;

    Ok(())
}

/// Persists `network.json` for administrative edits without modifying shaped
/// devices.
///
/// Returns an error string when the caller is unauthorized, when integration-
/// managed topology editing is locked, or when persistence fails.
pub fn update_network_json_only_data(
    login: LoginResult,
    network_json: Value,
) -> Result<(), String> {
    if login != LoginResult::Admin {
        return Err("Unauthorized".to_string());
    }
    ensure_topology_editor_unlocked()?;

    persist_network_json(&network_json)?;
    lqos_network_devices::request_reload_network_json("node_manager:update_network_json_only")
        .map_err(|e| format!("Unable to reload network.json: {e}"))?;

    Ok(())
}

/// Returns one shaped device row by device identifier for administrative
/// callers.
pub fn get_shaped_device_data(
    login: LoginResult,
    device_id: String,
) -> Result<Option<ShapedDevice>, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let wanted = device_id.trim();
    Ok(lqos_network_devices::shaped_devices_snapshot()
        .devices
        .iter()
        .find(|device| device.device_id == wanted)
        .cloned())
}

/// Creates one shaped device row for administrative callers.
///
/// Returns an error string when the caller is unauthorized, when integration-
/// managed topology editing is locked, or when validation/persistence fails.
pub fn create_shaped_device_data(
    login: LoginResult,
    device: ShapedDevice,
) -> Result<ShapedDevice, String> {
    if login != LoginResult::Admin {
        return Err("Unauthorized".to_string());
    }
    ensure_topology_editor_unlocked()?;
    let mut devices = lqos_network_devices::shaped_devices_snapshot().devices.clone();
    devices.push(device.clone());
    persist_shaped_devices(devices)?;
    let created = get_shaped_device_data(login, device.device_id.clone())
        .map_err(|_| "Unable to reload shaped device".to_string())?
        .ok_or_else(|| "Unable to reload shaped device".to_string())?;
    Ok(created)
}

/// Updates one shaped device row for administrative callers.
///
/// Returns an error string when the caller is unauthorized, when integration-
/// managed topology editing is locked, when the row is not found, or when
/// validation/persistence fails.
pub fn update_shaped_device_data(
    login: LoginResult,
    original_device_id: String,
    device: ShapedDevice,
) -> Result<ShapedDevice, String> {
    if login != LoginResult::Admin {
        return Err("Unauthorized".to_string());
    }
    ensure_topology_editor_unlocked()?;
    let mut devices = lqos_network_devices::shaped_devices_snapshot().devices.clone();
    let wanted = original_device_id.trim();
    let Some(index) = devices.iter().position(|row| row.device_id == wanted) else {
        return Err("Not found".to_string());
    };
    devices[index] = device.clone();
    persist_shaped_devices(devices)?;
    let updated = get_shaped_device_data(login, device.device_id.clone())
        .map_err(|_| "Unable to reload shaped device".to_string())?
        .ok_or_else(|| "Unable to reload shaped device".to_string())?;
    Ok(updated)
}

/// Deletes one shaped device row for administrative callers.
///
/// Returns an error string when the caller is unauthorized, when integration-
/// managed topology editing is locked, when the row is not found, or when
/// persistence fails.
pub fn delete_shaped_device_data(login: LoginResult, device_id: String) -> Result<(), String> {
    if login != LoginResult::Admin {
        return Err("Unauthorized".to_string());
    }
    ensure_topology_editor_unlocked()?;
    let wanted = device_id.trim();
    let mut devices = lqos_network_devices::shaped_devices_snapshot().devices.clone();
    let before = devices.len();
    devices.retain(|device| device.device_id != wanted);
    if devices.len() == before {
        return Err("Not found".to_string());
    }
    persist_shaped_devices(devices)?;
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
        .add_or_update_user(data.username.trim(), password, data.role.into())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(format!("User '{}' added", data.username))
}

pub fn update_user_data(login: LoginResult, data: UserRequest) -> Result<String, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut users = WebUsers::load_or_create().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let all_users = users.get_users();

    // Prevent turning the last administrator into a non-admin account.
    if let Some(existing_user) = all_users.iter().find(|u| u.username == data.username)
        && existing_user.role == UserRole::Admin
    {
        let admin_count = all_users
            .iter()
            .filter(|u| u.role == UserRole::Admin)
            .count();
        let requested_role: UserRole = data.role.clone().into();
        if admin_count <= 1 && requested_role != UserRole::Admin {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

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
    let all_users = users.get_users();

    // Prevent deleting the final administrator account.
    if let Some(existing_user) = all_users.iter().find(|u| u.username == username)
        && existing_user.role == UserRole::Admin
    {
        let admin_count = all_users
            .iter()
            .filter(|u| u.role == UserRole::Admin)
            .count();
        if admin_count <= 1 {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

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

#[cfg(test)]
mod tests {
    use super::validate_network_json;
    use serde_json::json;

    #[test]
    fn accepts_unique_node_names() {
        let network = json!({
            "Site A": {
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000,
                "children": {
                    "Site B": {
                        "downloadBandwidthMbps": 500,
                        "uploadBandwidthMbps": 500
                    }
                }
            },
            "Site C": {
                "downloadBandwidthMbps": 750,
                "uploadBandwidthMbps": 750
            }
        });

        assert!(validate_network_json(&network).is_ok());
    }

    #[test]
    fn rejects_duplicate_node_names_anywhere_in_tree() {
        let network = json!({
            "Site A": {
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000,
                "children": {
                    "Duplicate": {
                        "downloadBandwidthMbps": 500,
                        "uploadBandwidthMbps": 500
                    }
                }
            },
            "Site B": {
                "downloadBandwidthMbps": 750,
                "uploadBandwidthMbps": 750,
                "children": {
                    "Duplicate": {
                        "downloadBandwidthMbps": 300,
                        "uploadBandwidthMbps": 300
                    }
                }
            }
        });

        let err = validate_network_json(&network).expect_err("duplicate names must fail");
        assert!(err.contains("Duplicate"));
    }
}

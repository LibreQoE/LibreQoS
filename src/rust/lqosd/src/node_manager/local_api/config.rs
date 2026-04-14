use crate::node_manager::auth::LoginResult;
use crate::node_manager::local_api::network_mode::NetworkModeInspection;
use crate::node_manager::runtime_onboarding::RuntimeOnboardingState;
use crate::shaping_runtime::ShapingRuntimeStatus;
use axum::Extension;
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::http::header;
use default_net::get_interfaces;
use lqos_bus::{BusRequest, bus_request};
use lqos_config::{Config, ConfigShapedDevices, ShapedDevice, UserRole, WebUser, WebUsers};
use lqos_utils::hash_to_i64;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{Cursor, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const COBRAND_FILE_NAME: &str = "cobrand.png";
const COBRAND_DISPLAY_HEIGHT_PX: u64 = 48;
const COBRAND_MAX_DISPLAY_WIDTH_PX: u64 = 176;
const COBRAND_MAX_DECODE_BYTES: usize = 64 * 1024 * 1024;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CobrandUploadValidationError {
    UnsupportedMediaType,
    InvalidPng,
    TooLarge,
    TooWideForSidebar,
}

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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UispSecretState {
    #[serde(default)]
    pub token: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SplynxSecretState {
    #[serde(default)]
    pub api_key: bool,
    #[serde(default)]
    pub api_secret: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct VispSecretState {
    #[serde(default)]
    pub client_secret: bool,
    #[serde(default)]
    pub password: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SonarSecretState {
    #[serde(default)]
    pub sonar_api_key: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NetzurSecretState {
    #[serde(default)]
    pub api_key: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PowercodeSecretState {
    #[serde(default)]
    pub powercode_api_key: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ConfigSecretState {
    #[serde(default)]
    pub uisp_integration: UispSecretState,
    #[serde(default)]
    pub splynx_integration: SplynxSecretState,
    #[serde(default)]
    pub visp_integration: VispSecretState,
    #[serde(default)]
    pub sonar_integration: SonarSecretState,
    #[serde(default)]
    pub netzur_integration: NetzurSecretState,
    #[serde(default)]
    pub powercode_integration: PowercodeSecretState,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UispSecretClearRequest {
    #[serde(default)]
    pub token: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SplynxSecretClearRequest {
    #[serde(default)]
    pub api_key: bool,
    #[serde(default)]
    pub api_secret: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct VispSecretClearRequest {
    #[serde(default)]
    pub client_secret: bool,
    #[serde(default)]
    pub password: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SonarSecretClearRequest {
    #[serde(default)]
    pub sonar_api_key: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NetzurSecretClearRequest {
    #[serde(default)]
    pub api_key: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PowercodeSecretClearRequest {
    #[serde(default)]
    pub powercode_api_key: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ConfigSecretClearRequest {
    #[serde(default)]
    pub uisp_integration: UispSecretClearRequest,
    #[serde(default)]
    pub splynx_integration: SplynxSecretClearRequest,
    #[serde(default)]
    pub visp_integration: VispSecretClearRequest,
    #[serde(default)]
    pub sonar_integration: SonarSecretClearRequest,
    #[serde(default)]
    pub netzur_integration: NetzurSecretClearRequest,
    #[serde(default)]
    pub powercode_integration: PowercodeSecretClearRequest,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ConfigView {
    pub config: Config,
    #[serde(default)]
    pub secret_state: ConfigSecretState,
    #[serde(default)]
    pub shaping_status: ShapingRuntimeStatus,
    #[serde(default)]
    pub network_mode_inspection: NetworkModeInspection,
    #[serde(default)]
    pub runtime_onboarding: RuntimeOnboardingState,
}

pub fn admin_check_data(login: LoginResult) -> bool {
    matches!(login, LoginResult::Admin)
}

fn has_secret(value: &str) -> bool {
    !value.trim().is_empty()
}

fn redact_string_secret(value: &mut String) -> bool {
    let configured = has_secret(value);
    value.clear();
    configured
}

fn redact_config_secrets(config: &mut Config) -> ConfigSecretState {
    let mut secret_state = ConfigSecretState::default();

    secret_state.uisp_integration.token = redact_string_secret(&mut config.uisp_integration.token);
    secret_state.splynx_integration.api_key =
        redact_string_secret(&mut config.splynx_integration.api_key);
    secret_state.splynx_integration.api_secret =
        redact_string_secret(&mut config.splynx_integration.api_secret);
    secret_state.sonar_integration.sonar_api_key =
        redact_string_secret(&mut config.sonar_integration.sonar_api_key);
    secret_state.powercode_integration.powercode_api_key =
        redact_string_secret(&mut config.powercode_integration.powercode_api_key);

    if let Some(netzur) = config.netzur_integration.as_mut() {
        secret_state.netzur_integration.api_key = redact_string_secret(&mut netzur.api_key);
    }

    if let Some(visp) = config.visp_integration.as_mut() {
        secret_state.visp_integration.client_secret = redact_string_secret(&mut visp.client_secret);
        secret_state.visp_integration.password = redact_string_secret(&mut visp.password);
    }

    secret_state
}

fn merge_string_secret(incoming: &mut String, existing: &str, clear: bool) {
    if clear {
        incoming.clear();
    } else if incoming.trim().is_empty() {
        *incoming = existing.to_string();
    }
}

fn apply_secret_updates(
    existing: &Config,
    incoming: &mut Config,
    clear_secrets: &ConfigSecretClearRequest,
) {
    merge_string_secret(
        &mut incoming.uisp_integration.token,
        &existing.uisp_integration.token,
        clear_secrets.uisp_integration.token,
    );
    merge_string_secret(
        &mut incoming.splynx_integration.api_key,
        &existing.splynx_integration.api_key,
        clear_secrets.splynx_integration.api_key,
    );
    merge_string_secret(
        &mut incoming.splynx_integration.api_secret,
        &existing.splynx_integration.api_secret,
        clear_secrets.splynx_integration.api_secret,
    );
    merge_string_secret(
        &mut incoming.sonar_integration.sonar_api_key,
        &existing.sonar_integration.sonar_api_key,
        clear_secrets.sonar_integration.sonar_api_key,
    );
    merge_string_secret(
        &mut incoming.powercode_integration.powercode_api_key,
        &existing.powercode_integration.powercode_api_key,
        clear_secrets.powercode_integration.powercode_api_key,
    );

    match (
        existing.netzur_integration.as_ref(),
        incoming.netzur_integration.as_mut(),
    ) {
        (Some(existing_netzur), Some(incoming_netzur)) => {
            merge_string_secret(
                &mut incoming_netzur.api_key,
                &existing_netzur.api_key,
                clear_secrets.netzur_integration.api_key,
            );
        }
        (Some(existing_netzur), None) => {
            incoming.netzur_integration = Some(existing_netzur.clone());
            if let Some(incoming_netzur) = incoming.netzur_integration.as_mut() {
                merge_string_secret(
                    &mut incoming_netzur.api_key,
                    &existing_netzur.api_key,
                    clear_secrets.netzur_integration.api_key,
                );
            }
        }
        (None, Some(incoming_netzur)) => {
            if clear_secrets.netzur_integration.api_key {
                incoming_netzur.api_key.clear();
            }
        }
        (None, None) => {}
    }

    match (
        existing.visp_integration.as_ref(),
        incoming.visp_integration.as_mut(),
    ) {
        (Some(existing_visp), Some(incoming_visp)) => {
            merge_string_secret(
                &mut incoming_visp.client_secret,
                &existing_visp.client_secret,
                clear_secrets.visp_integration.client_secret,
            );
            merge_string_secret(
                &mut incoming_visp.password,
                &existing_visp.password,
                clear_secrets.visp_integration.password,
            );
        }
        (Some(existing_visp), None) => {
            incoming.visp_integration = Some(existing_visp.clone());
            if let Some(incoming_visp) = incoming.visp_integration.as_mut() {
                merge_string_secret(
                    &mut incoming_visp.client_secret,
                    &existing_visp.client_secret,
                    clear_secrets.visp_integration.client_secret,
                );
                merge_string_secret(
                    &mut incoming_visp.password,
                    &existing_visp.password,
                    clear_secrets.visp_integration.password,
                );
            }
        }
        (None, Some(incoming_visp)) => {
            if clear_secrets.visp_integration.client_secret {
                incoming_visp.client_secret.clear();
            }
            if clear_secrets.visp_integration.password {
                incoming_visp.password.clear();
            }
        }
        (None, None) => {}
    }
}

pub fn get_config_data(login: LoginResult) -> Result<ConfigView, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    lqos_config::load_config()
        .map(|config| {
            let mut config = (*config).clone();
            let secret_state = redact_config_secrets(&mut config);
            ConfigView {
                shaping_status: crate::shaping_runtime::get_status(),
                network_mode_inspection:
                    crate::node_manager::local_api::network_mode::inspect_network_mode(&config),
                runtime_onboarding:
                    crate::node_manager::runtime_onboarding::runtime_onboarding_state(),
                config,
                secret_state,
            }
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn cobrand_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2")
        .join(COBRAND_FILE_NAME)
}

fn validate_cobrand_upload(
    content_type: Option<&str>,
    body: &[u8],
) -> Result<(), CobrandUploadValidationError> {
    if content_type != Some("image/png") {
        return Err(CobrandUploadValidationError::UnsupportedMediaType);
    }
    inspect_png(body)?;
    Ok(())
}

fn inspect_png(body: &[u8]) -> Result<(u32, u32), CobrandUploadValidationError> {
    if body.len() < PNG_SIGNATURE.len() || &body[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err(CobrandUploadValidationError::InvalidPng);
    }

    let decoder = png::Decoder::new(Cursor::new(body));
    let Ok(mut reader) = decoder.read_info() else {
        return Err(CobrandUploadValidationError::InvalidPng);
    };
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    let output_buffer_size = reader.output_buffer_size();
    if output_buffer_size > COBRAND_MAX_DECODE_BYTES {
        return Err(CobrandUploadValidationError::TooLarge);
    }
    if rendered_cobrand_width_exceeds_sidebar(width, height) {
        return Err(CobrandUploadValidationError::TooWideForSidebar);
    }
    let mut output = vec![0; output_buffer_size];
    reader
        .next_frame(&mut output)
        .map_err(|_| CobrandUploadValidationError::InvalidPng)?;
    Ok((width, height))
}

fn rendered_cobrand_width_exceeds_sidebar(width: u32, height: u32) -> bool {
    u64::from(width) * COBRAND_DISPLAY_HEIGHT_PX > u64::from(height) * COBRAND_MAX_DISPLAY_WIDTH_PX
}

fn persist_cobrand_png(body: &[u8]) -> Result<(), StatusCode> {
    let config = lqos_config::load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let target_path = cobrand_path(config.as_ref());
    let Some(parent_dir) = target_path.parent() else {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    std::fs::create_dir_all(parent_dir).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let temp_root = parent_dir.parent().unwrap_or(parent_dir).join(".tmp");
    std::fs::create_dir_all(&temp_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut temp_path = None;
    for attempt in 0..32 {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let candidate = temp_root.join(format!(
            "cobrand-upload-{}-{unique_suffix}-{attempt}.png",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                if file.write_all(body).is_err() {
                    let _ = std::fs::remove_file(&candidate);
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
                temp_path = Some(candidate);
                break;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
    let Some(temp_path) = temp_path else {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    std::fs::rename(&temp_path, &target_path).map_err(|_| {
        let _ = std::fs::remove_file(&temp_path);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(())
}

pub async fn upload_cobrand(
    Extension(login): Extension<LoginResult>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, &'static str)> {
    if login != LoginResult::Admin {
        return Err((StatusCode::FORBIDDEN, "Administrator access is required."));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::trim);
    validate_cobrand_upload(content_type, &body).map_err(|error| match error {
        CobrandUploadValidationError::UnsupportedMediaType => (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Cobrand uploads must use Content-Type image/png.",
        ),
        CobrandUploadValidationError::InvalidPng => {
            (StatusCode::BAD_REQUEST, "Uploaded file is not a valid PNG.")
        }
        CobrandUploadValidationError::TooLarge => (
            StatusCode::BAD_REQUEST,
            "Cobrand image is too large to validate safely.",
        ),
        CobrandUploadValidationError::TooWideForSidebar => (
            StatusCode::BAD_REQUEST,
            "Cobrand image is too wide for the sidebar at 48px tall. Keep the rendered width at or below 176px.",
        ),
    })?;
    persist_cobrand_png(&body).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to save cobrand.png.",
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
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
    lqos_network_devices::shaped_devices_catalog().clone_all_devices()
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
    lqos_network_devices::apply_shaped_devices_snapshot(
        "node_manager:persist_shaped_devices",
        copied,
    )
    .map_err(|e| format!("Unable to publish ShapedDevices.csv snapshot: {e}"))?;

    Ok(())
}

pub async fn update_lqosd_config_data(
    login: LoginResult,
    mut config: Config,
    clear_secrets: ConfigSecretClearRequest,
) -> Result<(), StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let existing = lqos_config::load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    apply_secret_updates(existing.as_ref(), &mut config, &clear_secrets);
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
    Ok(lqos_network_devices::shaped_devices_catalog()
        .iter_devices()
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
    let mut devices = lqos_network_devices::shaped_devices_catalog().clone_all_devices();
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
    let mut devices = lqos_network_devices::shaped_devices_catalog().clone_all_devices();
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
    let mut devices = lqos_network_devices::shaped_devices_catalog().clone_all_devices();
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
    use super::{
        CobrandUploadValidationError, ConfigSecretClearRequest, apply_secret_updates, cobrand_path,
        persist_cobrand_png, redact_config_secrets, upload_cobrand, validate_cobrand_upload,
        validate_network_json,
    };
    use crate::node_manager::auth::LoginResult;
    use crate::test_support::runtime_config_test_lock;
    use axum::Extension;
    use axum::body::Bytes;
    use axum::http::HeaderMap;
    use lqos_config::Config;
    use serde_json::json;
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    const VALID_PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D',
        b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, b'I', b'D', b'A', b'T', 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, b'I',
        b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
    ];

    fn encode_test_png(width: u32, height: u32) -> Vec<u8> {
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("write png header");
            let pixel_count = usize::try_from(width)
                .expect("width fits usize")
                .saturating_mul(usize::try_from(height).expect("height fits usize"));
            let data = vec![0u8; pixel_count.saturating_mul(4)];
            writer.write_image_data(&data).expect("write png bytes");
        }
        png_bytes
    }

    fn cobrand_test_runtime_dir() -> PathBuf {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("lqosd-cobrand-test-{unique_suffix}"))
    }

    fn write_cobrand_test_config(runtime_dir: &std::path::Path) -> PathBuf {
        fs::create_dir_all(runtime_dir).expect("create runtime dir");
        let config_path = runtime_dir.join("lqos.conf");
        let runtime_dir_string = runtime_dir.display().to_string();
        let state_dir_string = runtime_dir.join("state").display().to_string();
        let raw = include_str!("../../../../lqos_config/src/etc/v15/example.toml")
            .replace("/opt/libreqos/src", &runtime_dir_string)
            .replace("/opt/libreqos/state", &state_dir_string)
            .replace("node_id = \"0000-0000-0000\"", "node_id = \"node\"");
        fs::write(&config_path, raw).expect("write config");
        config_path
    }

    struct CobrandTestContext {
        _guard: std::sync::MutexGuard<'static, ()>,
        old_lqos_config: Option<OsString>,
        old_lqos_directory: Option<OsString>,
        runtime_dir: PathBuf,
    }

    impl CobrandTestContext {
        fn new() -> Self {
            let guard = runtime_config_test_lock()
                .lock()
                .expect("cobrand test lock should not be poisoned");
            let runtime_dir = cobrand_test_runtime_dir();
            let config_path = write_cobrand_test_config(&runtime_dir);
            let old_lqos_config = std::env::var_os("LQOS_CONFIG");
            let old_lqos_directory = std::env::var_os("LQOS_DIRECTORY");
            unsafe {
                std::env::set_var("LQOS_CONFIG", &config_path);
                std::env::set_var("LQOS_DIRECTORY", &runtime_dir);
            }
            lqos_config::clear_cached_config();
            Self {
                _guard: guard,
                old_lqos_config,
                old_lqos_directory,
                runtime_dir,
            }
        }
    }

    impl Drop for CobrandTestContext {
        fn drop(&mut self) {
            match &self.old_lqos_config {
                Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
                None => unsafe { std::env::remove_var("LQOS_CONFIG") },
            }
            match &self.old_lqos_directory {
                Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
                None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
            }
            lqos_config::clear_cached_config();
            let _ = fs::remove_dir_all(&self.runtime_dir);
        }
    }

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

    #[test]
    fn redacts_secret_fields_and_marks_presence() {
        let mut config = Config::default();
        config.uisp_integration.token = "uisp-token".to_string();
        config.splynx_integration.api_key = "splynx-key".to_string();
        config.splynx_integration.api_secret = "splynx-secret".to_string();
        config.sonar_integration.sonar_api_key = "sonar-key".to_string();
        config.powercode_integration.powercode_api_key = "powercode-key".to_string();
        if let Some(visp) = config.visp_integration.as_mut() {
            visp.client_secret = "visp-secret".to_string();
            visp.password = "visp-password".to_string();
        }
        config.netzur_integration = Some(Default::default());
        config
            .netzur_integration
            .as_mut()
            .expect("netzur_integration should exist")
            .api_key = "netzur-key".to_string();

        let secret_state = redact_config_secrets(&mut config);

        assert!(secret_state.uisp_integration.token);
        assert!(secret_state.splynx_integration.api_key);
        assert!(secret_state.splynx_integration.api_secret);
        assert!(secret_state.visp_integration.client_secret);
        assert!(secret_state.visp_integration.password);
        assert!(secret_state.sonar_integration.sonar_api_key);
        assert!(secret_state.netzur_integration.api_key);
        assert!(secret_state.powercode_integration.powercode_api_key);
        assert!(config.uisp_integration.token.is_empty());
        assert!(config.splynx_integration.api_key.is_empty());
        assert!(config.splynx_integration.api_secret.is_empty());
        assert!(config.sonar_integration.sonar_api_key.is_empty());
        assert!(config.powercode_integration.powercode_api_key.is_empty());
        assert!(
            config
                .visp_integration
                .as_ref()
                .expect("visp_integration should exist")
                .client_secret
                .is_empty()
        );
        assert!(
            config
                .visp_integration
                .as_ref()
                .expect("visp_integration should exist")
                .password
                .is_empty()
        );
        assert!(
            config
                .netzur_integration
                .as_ref()
                .expect("netzur_integration should exist")
                .api_key
                .is_empty()
        );
    }

    #[test]
    fn applies_secret_updates_with_preserve_replace_and_clear() {
        let mut existing = Config::default();
        existing.uisp_integration.token = "old-uisp".to_string();
        existing.splynx_integration.api_key = "old-key".to_string();
        existing.splynx_integration.api_secret = "old-secret".to_string();
        existing.sonar_integration.sonar_api_key = "old-sonar".to_string();
        existing.powercode_integration.powercode_api_key = "old-powercode".to_string();
        if let Some(visp) = existing.visp_integration.as_mut() {
            visp.client_secret = "old-visp-secret".to_string();
            visp.password = "old-visp-password".to_string();
        }
        existing.netzur_integration = Some(Default::default());
        existing
            .netzur_integration
            .as_mut()
            .expect("netzur_integration should exist")
            .api_key = "old-netzur".to_string();

        let mut incoming = existing.clone();
        incoming.uisp_integration.token.clear();
        incoming.splynx_integration.api_key = "new-key".to_string();
        incoming.splynx_integration.api_secret.clear();
        incoming.sonar_integration.sonar_api_key.clear();
        incoming.powercode_integration.powercode_api_key.clear();
        if let Some(visp) = incoming.visp_integration.as_mut() {
            visp.client_secret.clear();
            visp.password = "new-visp-password".to_string();
        }
        incoming
            .netzur_integration
            .as_mut()
            .expect("netzur_integration should exist")
            .api_key
            .clear();

        let mut clear_secrets = ConfigSecretClearRequest::default();
        clear_secrets.splynx_integration.api_secret = true;
        clear_secrets.sonar_integration.sonar_api_key = true;
        clear_secrets.netzur_integration.api_key = true;

        apply_secret_updates(&existing, &mut incoming, &clear_secrets);

        assert_eq!(incoming.uisp_integration.token, "old-uisp");
        assert_eq!(incoming.splynx_integration.api_key, "new-key");
        assert!(incoming.splynx_integration.api_secret.is_empty());
        assert!(incoming.sonar_integration.sonar_api_key.is_empty());
        assert_eq!(
            incoming
                .visp_integration
                .as_ref()
                .expect("visp_integration should exist")
                .client_secret,
            "old-visp-secret"
        );
        assert_eq!(
            incoming
                .visp_integration
                .as_ref()
                .expect("visp_integration should exist")
                .password,
            "new-visp-password"
        );
        assert!(
            incoming
                .netzur_integration
                .as_ref()
                .expect("netzur_integration should exist")
                .api_key
                .is_empty()
        );
        assert_eq!(
            incoming.powercode_integration.powercode_api_key,
            "old-powercode"
        );
    }

    #[test]
    fn validate_cobrand_upload_requires_exact_png_content_type() {
        let body = VALID_PNG.to_vec();
        assert_eq!(
            validate_cobrand_upload(Some("image/jpeg"), &body),
            Err(CobrandUploadValidationError::UnsupportedMediaType)
        );
        assert_eq!(
            validate_cobrand_upload(Some("image/png"), b"not-a-png"),
            Err(CobrandUploadValidationError::InvalidPng)
        );
        let truncated_png = &VALID_PNG[..VALID_PNG.len() - 8];
        assert_eq!(
            validate_cobrand_upload(Some("image/png"), truncated_png),
            Err(CobrandUploadValidationError::InvalidPng)
        );
        assert_eq!(validate_cobrand_upload(Some("image/png"), &body), Ok(()));
    }

    #[test]
    fn validate_cobrand_upload_rejects_pngs_too_wide_for_sidebar() {
        let too_wide = encode_test_png(177, 48);
        assert_eq!(
            validate_cobrand_upload(Some("image/png"), &too_wide),
            Err(CobrandUploadValidationError::TooWideForSidebar)
        );
    }

    #[test]
    fn validate_cobrand_upload_rejects_pngs_with_huge_dimensions() {
        let too_large = encode_test_png(4097, 4096);
        assert_eq!(
            validate_cobrand_upload(Some("image/png"), &too_large),
            Err(CobrandUploadValidationError::TooLarge)
        );
    }

    #[test]
    fn validate_cobrand_upload_accepts_boundary_dimensions() {
        let max_width = encode_test_png(176, 48);
        let max_height = encode_test_png(1, 4096);
        assert_eq!(
            validate_cobrand_upload(Some("image/png"), &max_width),
            Ok(())
        );
        assert_eq!(
            validate_cobrand_upload(Some("image/png"), &max_height),
            Ok(())
        );
    }

    #[tokio::test]
    async fn upload_cobrand_requires_admin() {
        let result = upload_cobrand(
            Extension(LoginResult::ReadOnly),
            HeaderMap::new(),
            Bytes::from_static(VALID_PNG),
        )
        .await;

        assert_eq!(
            result,
            Err((
                axum::http::StatusCode::FORBIDDEN,
                "Administrator access is required.",
            ))
        );
    }

    #[tokio::test]
    async fn upload_cobrand_accepts_admin_png_and_persists_file() {
        let _context = CobrandTestContext::new();

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("image/png"),
        );
        let result = upload_cobrand(
            Extension(LoginResult::Admin),
            headers,
            Bytes::from_static(VALID_PNG),
        )
        .await;

        assert_eq!(result, Ok(axum::http::StatusCode::NO_CONTENT));
        let config = lqos_config::load_config().expect("load config");
        let path = cobrand_path(config.as_ref());
        assert_eq!(fs::read(path).expect("read cobrand"), VALID_PNG);
    }

    #[test]
    fn persist_cobrand_png_writes_runtime_static_file() {
        let _context = CobrandTestContext::new();

        let png_body = VALID_PNG.to_vec();
        persist_cobrand_png(&png_body).expect("write cobrand");

        let config = lqos_config::load_config().expect("load config");
        let path = cobrand_path(config.as_ref());
        assert_eq!(fs::read(path).expect("read cobrand"), png_body);
    }
}

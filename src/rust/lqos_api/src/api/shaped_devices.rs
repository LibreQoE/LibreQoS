use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use axum::{Json, extract::Path, extract::Query, http::StatusCode};
use csv::ReaderBuilder;
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use lqos_config::{ConfigShapedDevices, ShapedDevice};
use lqos_utils::XdpIpAddress;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::services::shaped_devices;

#[utoipa::path(
    get,
    path = "/validate_shaped_devices",
    summary = "Validate the current ShapedDevices.csv and return any warnings or errors",
    description = "Validate the current ShapedDevices.csv and return any warnings or errors.",
    responses(
        (status = 200, description = "Return 200 OK and validation results.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn validate_shaped_devices() -> Result<(StatusCode, String), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::ValidateShapedDevicesCsv])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::ShapedDevicesValidation(result) = response {
            return Ok((StatusCode::OK, result));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct ShapedDeviceTransit {
    // Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment
    /// The ID of the circuit to which the device belongs. Circuits are 1:many,
    /// multiple devices may be in a single circuit.
    pub circuit_id: String,

    /// The name of the circuit. Since we're in a flat file, circuit names
    /// must match.
    pub circuit_name: String,

    /// The device identification, typically drawn from a management tool.
    pub device_id: String,

    /// The display name of the device.
    pub device_name: String,

    /// The parent node of the device, derived from `network.json`
    pub parent_node: String,

    /// The device's MAC address. This isn't actually used, it exists for
    /// convenient mapping/seraching.
    pub mac: String,

    /// A list of all IPv4 addresses and CIDR subnets associated with the
    /// device. For example, ("192.168.1.0", 24) is equivalent to
    /// "192.168.1.0/24"
    pub ipv4: Vec<(String, u32)>,

    /// A list of all IPv4 addresses and CIDR subnets associated with the
    /// device.
    pub ipv6: Vec<(String, u32)>,

    /// Minimum download: this is the bandwidth level the shaper will try
    /// to ensure is always available.
    pub download_min_mbps: f32,

    /// Minimum upload: this is the bandwidth level the shaper will try to
    /// ensure is always available.
    pub upload_min_mbps: f32,

    /// Maximum download speed, when possible.
    pub download_max_mbps: f32,

    /// Maximum upload speed when possible.
    pub upload_max_mbps: f32,

    /// Generic comments field, does nothing.
    pub comment: String,

    /// Optional per-circuit SQM override token. Accepts "cake", "fq_codel",
    /// "none", or directional "down_sqm/up_sqm" values like "cake/none" or "/fq_codel".
    /// A single token applies to both directions; empty means use defaults.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqm_override: Option<String>,

    /// Hash of the circuit ID, used for internal lookups.
    pub circuit_hash: i64,

    /// Hash of the device ID, used for internal lookups.
    pub device_hash: i64,

    /// Hash of the parent node, used for internal lookups.
    pub parent_hash: i64,
}

impl From<ShapedDevice> for ShapedDeviceTransit {
    fn from(device: ShapedDevice) -> Self {
        ShapedDeviceTransit {
            circuit_id: device.circuit_id,
            circuit_name: device.circuit_name,
            device_id: device.device_id,
            device_name: device.device_name,
            parent_node: device.parent_node,
            mac: device.mac,
            ipv4: device
                .ipv4
                .into_iter()
                .map(|ip| (ip.0.to_string(), ip.1))
                .collect(),
            ipv6: device
                .ipv6
                .into_iter()
                .map(|ip| (ip.0.to_string(), ip.1))
                .collect(),
            download_min_mbps: device.download_min_mbps,
            upload_min_mbps: device.upload_min_mbps,
            download_max_mbps: device.download_max_mbps,
            upload_max_mbps: device.upload_max_mbps,
            comment: device.comment,
            sqm_override: device.sqm_override,
            circuit_hash: device.circuit_hash,
            device_hash: device.device_hash,
            parent_hash: device.parent_hash,
        }
    }
}

#[utoipa::path(
    get,
    path = "/shaped_devices",
    summary = "All shaped devices parsed from ShapedDevices.csv",
    description = "Return all shaped devices parsed from ShapedDevices.csv.",
    responses(
        (status = 200, description = "Return 200 OK and validation results.", body = Vec<ShapedDeviceTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_shaped_devices() -> Result<Json<Vec<ShapedDeviceTransit>>, (StatusCode, String)> {
    let devices = shaped_devices::load_or_empty().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load shaped devices: {}", e),
        )
    })?;

    let transit = devices
        .devices
        .into_iter()
        .map(ShapedDeviceTransit::from)
        .collect::<Vec<ShapedDeviceTransit>>();

    Ok(Json(transit))
}

#[utoipa::path(
    post,
    path = "/shaped_devices/update",
    summary = "Replace ShapedDevices.csv with the provided CSV payload",
    description = "Replace ShapedDevices.csv with the provided CSV payload.",
    responses(
        (status = 200, description = "ShapedDevices.csv saved."),
        (status = 400, description = "CSV validation failed.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Unable to persist ShapedDevices.csv.", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
    request_body = String,
)]
pub async fn write_shaped_devices(body: String) -> Result<StatusCode, (StatusCode, String)> {
    validate_shaped_devices_csv(&body).map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let path = ConfigShapedDevices::path().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to locate ShapedDevices.csv: {e}"),
        )
    })?;
    tokio::fs::write(path, body.as_bytes()).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to write ShapedDevices.csv: {e}"),
        )
    })?;

    Ok(StatusCode::OK)
}

#[utoipa::path(
    get,
    path = "/shaped_devices/{device_id}",
    summary = "Shaped device record matching a device ID",
    description = "Return the shaped device record matching a device ID.",
    responses(
        (status = 200, description = "Return 200 OK and validation results.", body = Vec<ShapedDeviceTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 404, description = "Return 404 Not Found if the device ID does not exist.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("device_id" = String, Path, description = "The device ID to look up.", example = "device123"),
    ),
)]
pub async fn get_shaped_device(
    Path(device_id): Path<String>,
) -> Result<Json<ShapedDeviceTransit>, (StatusCode, String)> {
    let devices = shaped_devices::load_or_empty().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load shaped devices: {}", e),
        )
    })?;

    let device = devices
        .devices
        .into_iter()
        .find(|d| d.device_id == device_id)
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("Device not found: {}", device_id),
        ))?;

    Ok(Json(ShapedDeviceTransit::from(device)))
}

#[utoipa::path(
    get,
    path = "/shaped_devices_by_circuit/{circuit_id}",
    summary = "All shaped devices for a circuit ID",
    description = "Return all shaped devices for a circuit ID.",
    responses(
        (status = 200, description = "Return 200 OK and validation results.", body = Vec<ShapedDeviceTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("circuit_id" = String, Path, description = "The circuit ID to look up.", example = "circuit123"),
    ),
)]
pub async fn shaped_devices_by_circuit_id(
    Path(circuit_id): Path<String>,
) -> Result<Json<Vec<ShapedDeviceTransit>>, (StatusCode, String)> {
    let devices = shaped_devices::load_or_empty().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load shaped devices: {}", e),
        )
    })?;

    let shaped_devices = devices
        .devices
        .into_iter()
        .filter(|d| d.circuit_id == circuit_id)
        .map(ShapedDeviceTransit::from)
        .collect::<Vec<ShapedDeviceTransit>>();

    Ok(Json(shaped_devices))
}

#[utoipa::path(
    get,
    path = "/identify_ip/{ip_address}",
    summary = "Look up an IP address and return matching shaped device records",
    description = "Look up an IP address and return matching shaped device records.",
    responses(
        (status = 200, description = "Return 200 OK and validation results.", body = Vec<ShapedDeviceTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 404, description = "Return 404 Not Found if the circuit ID is not found.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("ip_address" = String, Path, description = "The IP address to look up.", example = "192.168.1.1"),
    ),
)]
pub async fn identify_ip_address(
    Path(ip_address): Path<String>,
) -> Result<Json<Vec<ShapedDeviceTransit>>, (StatusCode, String)> {
    let devices = shaped_devices::load_or_empty().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load shaped devices: {}", e),
        )
    })?;

    let ip: IpAddr = ip_address.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid IP address: {}", e),
        )
    })?;
    let xdp_ip = XdpIpAddress::from(ip);
    let Some((circuit_id, _name)) = devices.get_circuit_id_and_name_from_ip(&xdp_ip) else {
        return Err((
            StatusCode::NOT_FOUND,
            format!("No circuit found for IP address: {}", ip_address),
        ));
    };

    let shaped_devices = devices
        .devices
        .into_iter()
        .filter(|d| d.circuit_id == circuit_id)
        .map(ShapedDeviceTransit::from)
        .collect::<Vec<ShapedDeviceTransit>>();

    Ok(Json(shaped_devices))
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct LiveUpsertCircuitRequest {
    pub circuit_id: String,
    pub circuit_name: String,
    pub device_id: String,
    pub device_name: String,
    pub parent_node: String,
    pub mac: String,
    /// IPv4 entries as strings. Either address (default /32) or CIDR.
    pub ipv4_list: Vec<String>,
    /// IPv6 entries as strings. Either address (default /128) or CIDR.
    pub ipv6_list: Vec<String>,
    pub download_min_mbps: f32,
    pub upload_min_mbps: f32,
    pub download_max_mbps: f32,
    pub upload_max_mbps: f32,
    #[serde(default)]
    pub comment: String,
    /// Optional per-circuit SQM override token. Accepts "cake", "fq_codel",
    /// "none", or directional "down_sqm/up_sqm" values like "cake/none" or "/fq_codel".
    /// A single token applies to both directions; empty means use defaults.
    #[serde(default)]
    pub sqm_override: Option<String>,
}

fn parse_ipv4(s: &str) -> anyhow::Result<(Ipv4Addr, u32)> {
    if let Some((ip, prefix)) = s.split_once('/') {
        Ok((ip.parse()?, prefix.parse()?))
    } else {
        Ok((s.parse()?, 32))
    }
}

fn parse_ipv6(s: &str) -> anyhow::Result<(Ipv6Addr, u32)> {
    if let Some((ip, prefix)) = s.split_once('/') {
        Ok((ip.parse()?, prefix.parse()?))
    } else {
        Ok((s.parse()?, 128))
    }
}

impl TryFrom<LiveUpsertCircuitRequest> for ShapedDevice {
    type Error = anyhow::Error;

    fn try_from(value: LiveUpsertCircuitRequest) -> Result<Self, Self::Error> {
        let ipv4 = value
            .ipv4_list
            .iter()
            .map(|s| parse_ipv4(s))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let ipv6 = value
            .ipv6_list
            .iter()
            .map(|s| parse_ipv6(s))
            .collect::<anyhow::Result<Vec<_>>>()?;

        if value.download_min_mbps <= 0.0
            || value.upload_min_mbps <= 0.0
            || value.download_max_mbps <= 0.0
            || value.upload_max_mbps <= 0.0
        {
            anyhow::bail!("Bandwidth values must be positive");
        }

        let sqm_override = match value.sqm_override.as_deref() {
            Some(raw) => normalize_sqm_override(raw).map_err(|e| anyhow::anyhow!(e))?,
            None => None,
        };

        let circuit_hash = lqos_utils::hash_to_i64(&value.circuit_id);
        let device_hash = lqos_utils::hash_to_i64(&value.device_id);
        let parent_hash = lqos_utils::hash_to_i64(&value.parent_node);

        Ok(ShapedDevice {
            circuit_id: value.circuit_id,
            circuit_name: value.circuit_name,
            device_id: value.device_id,
            device_name: value.device_name,
            parent_node: value.parent_node,
            parent_node_id: None,
            anchor_node_id: None,
            mac: value.mac,
            ipv4,
            ipv6,
            download_min_mbps: value.download_min_mbps,
            upload_min_mbps: value.upload_min_mbps,
            download_max_mbps: value.download_max_mbps,
            upload_max_mbps: value.upload_max_mbps,
            comment: value.comment,
            sqm_override,
            circuit_hash,
            device_hash,
            parent_hash,
        })
    }
}

#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
pub struct LiveReloadQuery {
    /// If true (default), run a LibreQoS reload after applying the change.
    pub reload: Option<bool>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct LiveUpdateResponse {
    pub message: String,
    pub changed: bool,
    pub reload_requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reload_output: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct LiveRemovedCountResponse {
    pub message: String,
    pub removed_count: usize,
    pub reload_requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reload_output: Option<String>,
}

fn ipv4_to_string(ip: &(Ipv4Addr, u32)) -> String {
    if ip.1 == 32 {
        format!("{}", ip.0)
    } else {
        format!("{}/{}", ip.0, ip.1)
    }
}

fn ipv4_list_to_string(ips: &[(Ipv4Addr, u32)]) -> String {
    if ips.is_empty() {
        return String::new();
    }
    if ips.len() == 1 {
        return ipv4_to_string(&ips[0]);
    }
    ips.iter()
        .map(ipv4_to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn ipv6_to_string(ip: &(Ipv6Addr, u32)) -> String {
    if ip.1 == 128 {
        format!("{}", ip.0)
    } else {
        format!("{}/{}", ip.0, ip.1)
    }
}

fn ipv6_list_to_string(ips: &[(Ipv6Addr, u32)]) -> String {
    if ips.is_empty() {
        return String::new();
    }
    if ips.len() == 1 {
        return ipv6_to_string(&ips[0]);
    }
    ips.iter()
        .map(ipv6_to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn shaped_devices_to_csv_string(devices: &[ShapedDevice]) -> anyhow::Result<String> {
    let mut writer = csv::WriterBuilder::new()
        .quote_style(csv::QuoteStyle::Necessary)
        .from_writer(vec![]);

    writer.write_record([
        "Circuit ID",
        "Circuit Name",
        "Device ID",
        "Device Name",
        "Parent Node",
        "MAC",
        "IPv4",
        "IPv6",
        "Download Min Mbps",
        "Upload Min Mbps",
        "Download Max Mbps",
        "Upload Max Mbps",
        "Comment",
        "sqm",
    ])?;

    for d in devices {
        writer.write_record([
            d.circuit_id.as_str(),
            d.circuit_name.as_str(),
            d.device_id.as_str(),
            d.device_name.as_str(),
            d.parent_node.as_str(),
            d.mac.as_str(),
            &ipv4_list_to_string(&d.ipv4),
            &ipv6_list_to_string(&d.ipv6),
            &d.download_min_mbps.to_string(),
            &d.upload_min_mbps.to_string(),
            &d.download_max_mbps.to_string(),
            &d.upload_max_mbps.to_string(),
            d.comment.as_str(),
            d.sqm_override.as_deref().unwrap_or(""),
        ])?;
    }

    let bytes = writer.into_inner()?;
    Ok(String::from_utf8(bytes)?)
}

async fn write_shaped_devices_struct(devices: &[ShapedDevice]) -> Result<(), (StatusCode, String)> {
    let csv = shaped_devices_to_csv_string(devices).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("CSV serialize failed: {e}"),
        )
    })?;
    validate_shaped_devices_csv(&csv).map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let path = ConfigShapedDevices::path().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to locate ShapedDevices.csv: {e}"),
        )
    })?;

    let tmp_path = path.with_extension("tmp");
    tokio::fs::write(&tmp_path, csv.as_bytes())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to write temp ShapedDevices.csv: {e}"),
            )
        })?;
    tokio::fs::rename(&tmp_path, &path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to replace ShapedDevices.csv: {e}"),
        )
    })?;
    Ok(())
}

#[utoipa::path(
    post,
    path = "/shaped_devices/circuit",
    summary = "Live upsert a circuit/device entry into ShapedDevices.csv",
    description = "Live upsert a circuit/device entry into ShapedDevices.csv. This updates the shaped devices file directly (non-persistent; does not touch overrides). Use the `reload` query parameter (default true) to trigger an immediate LibreQoS reload, or set it to false to batch multiple changes before reloading.",
    request_body = LiveUpsertCircuitRequest,
    responses(
        (status = 200, description = "Circuit/device entry upserted.", body = LiveUpdateResponse),
        (status = 400, description = "Invalid request body or resulting CSV validation failed.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 404, description = "Parent node not found in the topology operator view.", body = String),
        (status = 409, description = "Parent node name is ambiguous in the topology operator view.", body = String),
        (status = 500, description = "Unable to persist ShapedDevices.csv or reload LibreQoS.", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("reload" = Option<bool>, Query, description = "If true (default), reload LibreQoS after writing ShapedDevices.csv.", example = true),
    ),
)]
pub async fn live_upsert_circuit(
    Query(query): Query<LiveReloadQuery>,
    Json(req): Json<LiveUpsertCircuitRequest>,
) -> Result<Json<LiveUpdateResponse>, (StatusCode, String)> {
    let device = ShapedDevice::try_from(req)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid device: {e}")))?;

    if !super::network_json::published_network_has_node(&device.parent_node).await? {
        return Err((
            StatusCode::NOT_FOUND,
            format!(
                "Parent node not found in network.json: {}",
                device.parent_node
            ),
        ));
    }

    let mut existing = shaped_devices::load_or_empty().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load shaped devices: {e}"),
        )
    })?;

    if existing
        .devices
        .iter()
        .any(|d| d.circuit_id == device.circuit_id && d.circuit_name != device.circuit_name)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Circuit name mismatch for circuit_id {}: '{}' vs '{}'",
                device.circuit_id,
                existing
                    .devices
                    .iter()
                    .find(|d| d.circuit_id == device.circuit_id)
                    .map(|d| d.circuit_name.as_str())
                    .unwrap_or(""),
                device.circuit_name
            ),
        ));
    }

    let mut changed = false;
    if let Some(idx) = existing
        .devices
        .iter()
        .position(|d| d.device_id == device.device_id)
    {
        if existing.devices[idx] != device {
            existing.devices[idx] = device;
            changed = true;
        }
    } else {
        existing.devices.push(device);
        changed = true;
    }

    if changed {
        write_shaped_devices_struct(&existing.devices).await?;
    }

    let reload_requested = query.reload.unwrap_or(true);
    let reload_output = if reload_requested {
        match crate::api::reload_libreqos().await {
            Ok((_status, output)) => Some(output),
            Err((status, msg)) => return Err((status, msg)),
        }
    } else {
        None
    };

    Ok(Json(LiveUpdateResponse {
        message: if changed {
            "Circuit/device entry written to ShapedDevices.csv.".to_string()
        } else {
            "No change (existing entry already matches).".to_string()
        },
        changed,
        reload_requested,
        reload_output,
    }))
}

#[utoipa::path(
    delete,
    path = "/shaped_devices/circuit/{circuit_id}",
    summary = "Live remove a circuit from ShapedDevices.csv",
    description = "Live remove a circuit from ShapedDevices.csv by circuit ID (removes all devices matching the circuit). This updates the shaped devices file directly (non-persistent; does not touch overrides). Use the `reload` query parameter (default true) to trigger an immediate LibreQoS reload, or set it to false to batch multiple changes before reloading.",
    responses(
        (status = 200, description = "Circuit removed (or not present).", body = LiveRemovedCountResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Unable to persist ShapedDevices.csv or reload LibreQoS.", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("circuit_id" = String, Path, description = "Circuit ID to remove", example = "circuit123"),
        ("reload" = Option<bool>, Query, description = "If true (default), reload LibreQoS after writing ShapedDevices.csv.", example = true),
    ),
)]
pub async fn live_remove_circuit(
    Path(circuit_id): Path<String>,
    Query(query): Query<LiveReloadQuery>,
) -> Result<Json<LiveRemovedCountResponse>, (StatusCode, String)> {
    let mut existing = shaped_devices::load_or_empty().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load shaped devices: {e}"),
        )
    })?;

    let before = existing.devices.len();
    existing.devices.retain(|d| d.circuit_id != circuit_id);
    let removed = before.saturating_sub(existing.devices.len());

    if removed > 0 {
        write_shaped_devices_struct(&existing.devices).await?;
    }

    let reload_requested = query.reload.unwrap_or(true);
    let reload_output = if reload_requested {
        match crate::api::reload_libreqos().await {
            Ok((_status, output)) => Some(output),
            Err((status, msg)) => return Err((status, msg)),
        }
    } else {
        None
    };

    Ok(Json(LiveRemovedCountResponse {
        message: if removed > 0 {
            format!("Removed {removed} device row(s) for circuit {circuit_id}.")
        } else {
            format!("No device rows found for circuit {circuit_id}.")
        },
        removed_count: removed,
        reload_requested,
        reload_output,
    }))
}

fn validate_shaped_devices_csv(raw: &str) -> Result<(), String> {
    let mut reader = ReaderBuilder::new()
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .flexible(true)
        .has_headers(false)
        .from_reader(raw.as_bytes());

    let mut line_no = 0usize;
    let mut uses_extended_layout = false;
    for record in reader.records() {
        line_no += 1;
        let record = record.map_err(|e| format!("CSV parse error on line {line_no}: {e}"))?;
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        if is_header_record(&record) {
            uses_extended_layout = record.iter().any(|field| {
                let normalized = field.trim().to_ascii_lowercase().replace([' ', '_'], "");
                normalized == "parentnodeid" || normalized == "anchornodeid"
            });
            continue;
        }

        let field_count = record.len();
        if !(13..=16).contains(&field_count) {
            return Err(format!(
                "Line {line_no}: expected 13 to 16 columns, found {field_count}"
            ));
        }

        let bandwidth_start = if uses_extended_layout || field_count >= 15 {
            10
        } else {
            8
        };
        for (idx, label) in [
            (bandwidth_start, "download_min_mbps"),
            (bandwidth_start + 1, "upload_min_mbps"),
            (bandwidth_start + 2, "download_max_mbps"),
            (bandwidth_start + 3, "upload_max_mbps"),
        ] {
            let value = record.get(idx).unwrap_or("").trim();
            if value.parse::<f32>().is_err() {
                return Err(format!("Line {line_no}: invalid {label} value '{value}'"));
            }
        }

        let sqm_index = if bandwidth_start == 10 { 15 } else { 13 };
        if field_count > sqm_index {
            let raw = record.get(sqm_index).unwrap_or("");
            normalize_sqm_override(raw).map_err(|e| format!("Line {line_no}: {e}"))?;
        }
    }

    Ok(())
}

fn is_header_record(record: &csv::StringRecord) -> bool {
    let first = record.get(0).unwrap_or("").to_ascii_lowercase();
    let second = record.get(1).unwrap_or("").to_ascii_lowercase();
    first.contains("circuit id") && second.contains("circuit name")
}

fn normalize_sqm_override(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let token = trimmed.to_ascii_lowercase();
    if token.contains('/') {
        let mut parts = token.splitn(2, '/');
        let down = parts.next().unwrap_or("").trim();
        let up = parts.next().unwrap_or("").trim();
        let valid = |s: &str| matches!(s, "" | "cake" | "fq_codel" | "none");
        if !valid(down) || !valid(up) {
            return Err(format!(
                "invalid directional sqm override '{token}'. Allowed: 'cake', 'fq_codel', 'none', or 'down_sqm/up_sqm' (e.g. 'cake/fq_codel', '/none')"
            ));
        }
        return Ok(Some(format!("{down}/{up}")));
    }

    match token.as_str() {
        "cake" | "fq_codel" | "none" => Ok(Some(token)),
        _ => Err(format!(
            "invalid sqm override '{token}'. Allowed values: 'cake', 'fq_codel', 'none', or 'down_sqm/up_sqm' (e.g. 'cake/fq_codel', '/none')"
        )),
    }
}

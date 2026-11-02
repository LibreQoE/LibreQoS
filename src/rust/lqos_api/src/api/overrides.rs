use std::net::{Ipv4Addr, Ipv6Addr};

use axum::{Json, extract::Path, http::StatusCode};
use lqos_config::ShapedDevice;
use lqos_overrides::{
    CircuitAdjustment, NetworkAdjustment, OverrideFile, TopologyParentOverrideMode,
    UispRouteOverride,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ---------------
// Shared responses
// ---------------

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct OverrideActionResponse {
    /// Human-readable status message
    pub message: String,
    /// True if a change was made (where applicable)
    pub changed: Option<bool>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RemovedCountResponse {
    /// Number of items removed
    pub removed_count: usize,
}

// -----------------------------
// Persistent Devices management
// -----------------------------

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct AddPersistentDeviceRequest {
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

impl TryFrom<AddPersistentDeviceRequest> for ShapedDevice {
    type Error = anyhow::Error;

    fn try_from(value: AddPersistentDeviceRequest) -> Result<Self, Self::Error> {
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

        let sqm_override = normalize_sqm_override(value.sqm_override.as_deref())?;

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
            circuit_hash: 0,
            device_hash: 0,
            parent_hash: 0,
        })
    }
}

fn normalize_sqm_override(raw: Option<&str>) -> anyhow::Result<Option<String>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
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
            anyhow::bail!(
                "invalid directional sqm override '{token}'. Allowed: 'cake', 'fq_codel', 'none', or 'down_sqm/up_sqm' (e.g. 'cake/fq_codel', '/none')"
            );
        }
        return Ok(Some(format!("{down}/{up}")));
    }

    match token.as_str() {
        "cake" | "fq_codel" | "none" => Ok(Some(token)),
        _ => anyhow::bail!(
            "invalid sqm override '{token}'. Allowed values: 'cake', 'fq_codel', 'none', or 'down_sqm/up_sqm' (e.g. 'cake/fq_codel', '/none')"
        ),
    }
}

#[utoipa::path(
    get,
    path = "/overrides/persistent_devices",
    summary = "List persistent shaped devices stored in overrides",
    description = "List persistent shaped devices stored in overrides.",
    responses(
        (status = 200, description = "List persistent devices.", body = Vec<crate::api::ShapedDeviceTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error loading overrides.", body = String),
    ),
    tag = "Overrides",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn overrides_persistent_devices_list()
-> Result<Json<Vec<crate::api::ShapedDeviceTransit>>, (StatusCode, String)> {
    let overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let list = overrides
        .persistent_devices()
        .iter()
        .cloned()
        .map(crate::api::ShapedDeviceTransit::from)
        .collect::<Vec<_>>();
    Ok(Json(list))
}

#[utoipa::path(
    post,
    path = "/overrides/persistent_devices",
    summary = "Add or replace a persistent shaped device in overrides",
    description = "Add or replace a persistent shaped device in overrides.",
    request_body = AddPersistentDeviceRequest,
    responses(
        (status = 200, description = "Add or replace a persistent device.", body = OverrideActionResponse),
        (status = 400, description = "Invalid request body (e.g., malformed IP or bandwidth).", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn overrides_persistent_devices_add(
    Json(req): Json<AddPersistentDeviceRequest>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let device = ShapedDevice::try_from(req)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid device: {}", e)))?;

    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let changed = overrides.add_persistent_shaped_device_return_changed(device);
    if changed {
        overrides.save().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save overrides: {}", e),
            )
        })?;
    }

    Ok(Json(OverrideActionResponse {
        message: if changed {
            "Added/updated device; overrides saved.".to_string()
        } else {
            "No change (device already present).".to_string()
        },
        changed: Some(changed),
    }))
}

#[utoipa::path(
    delete,
    path = "/overrides/persistent_devices/by_circuit/{circuit_id}",
    summary = "Remove persistent devices by circuit ID",
    description = "Remove persistent devices by circuit ID.",
    responses(
        (status = 200, description = "Remove persistent devices by circuit ID.", body = RemovedCountResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
        ("circuit_id" = String, Path, description = "Circuit ID", example = "CIR-123"),
    ),
)]
pub async fn overrides_persistent_devices_delete_circuit(
    Path(circuit_id): Path<String>,
) -> Result<Json<RemovedCountResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let removed = overrides.remove_persistent_shaped_device_by_circuit_count(&circuit_id);
    if removed > 0 {
        overrides.save().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save overrides: {}", e),
            )
        })?;
    }
    Ok(Json(RemovedCountResponse {
        removed_count: removed,
    }))
}

#[utoipa::path(
    delete,
    path = "/overrides/persistent_devices/by_device/{device_id}",
    summary = "Remove persistent devices by device ID",
    description = "Remove persistent devices by device ID.",
    responses(
        (status = 200, description = "Remove persistent devices by device ID.", body = RemovedCountResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
        ("device_id" = String, Path, description = "Device ID", example = "DEV-123"),
    ),
)]
pub async fn overrides_persistent_devices_delete_device(
    Path(device_id): Path<String>,
) -> Result<Json<RemovedCountResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let removed = overrides.remove_persistent_shaped_device_by_device_count(&device_id);
    if removed > 0 {
        overrides.save().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save overrides: {}", e),
            )
        })?;
    }
    Ok(Json(RemovedCountResponse {
        removed_count: removed,
    }))
}

// -------------------------
// Circuit Adjustments (CSV)
// -------------------------

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CircuitAdjustmentTransit {
    CircuitAdjustSpeed {
        circuit_id: String,
        min_download_bandwidth: Option<f32>,
        max_download_bandwidth: Option<f32>,
        min_upload_bandwidth: Option<f32>,
        max_upload_bandwidth: Option<f32>,
    },
    DeviceAdjustSpeed {
        device_id: String,
        min_download_bandwidth: Option<f32>,
        max_download_bandwidth: Option<f32>,
        min_upload_bandwidth: Option<f32>,
        max_upload_bandwidth: Option<f32>,
    },
    DeviceAdjustSqm {
        device_id: String,
        sqm_override: Option<String>,
    },
    CircuitAdjustAqm {
        circuit_id: String,
        aqm_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at_unix: Option<u64>,
    },
    RemoveCircuit {
        circuit_id: String,
    },
    RemoveDevice {
        device_id: String,
    },
    ReparentCircuit {
        circuit_id: String,
        parent_node: String,
    },
}

impl From<CircuitAdjustment> for CircuitAdjustmentTransit {
    fn from(value: CircuitAdjustment) -> Self {
        match value {
            CircuitAdjustment::CircuitAdjustSpeed {
                circuit_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => Self::CircuitAdjustSpeed {
                circuit_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            },
            CircuitAdjustment::DeviceAdjustSpeed {
                device_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => Self::DeviceAdjustSpeed {
                device_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            },
            CircuitAdjustment::DeviceAdjustSqm {
                device_id,
                sqm_override,
            } => Self::DeviceAdjustSqm {
                device_id,
                sqm_override,
            },
            CircuitAdjustment::CircuitAdjustAqm {
                circuit_id,
                aqm_token,
                expires_at_unix,
            } => Self::CircuitAdjustAqm {
                circuit_id,
                aqm_token,
                expires_at_unix,
            },
            CircuitAdjustment::RemoveCircuit { circuit_id } => Self::RemoveCircuit { circuit_id },
            CircuitAdjustment::RemoveDevice { device_id } => Self::RemoveDevice { device_id },
            CircuitAdjustment::ReparentCircuit {
                circuit_id,
                parent_node,
            } => Self::ReparentCircuit {
                circuit_id,
                parent_node,
            },
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default)]
pub struct AddCircuitSpeedRequest {
    pub circuit_id: String,
    pub min_download_bandwidth: Option<f32>,
    pub max_download_bandwidth: Option<f32>,
    pub min_upload_bandwidth: Option<f32>,
    pub max_upload_bandwidth: Option<f32>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default)]
pub struct AddDeviceSpeedRequest {
    pub device_id: String,
    pub min_download_bandwidth: Option<f32>,
    pub max_download_bandwidth: Option<f32>,
    pub min_upload_bandwidth: Option<f32>,
    pub max_upload_bandwidth: Option<f32>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RemoveCircuitRequest {
    pub circuit_id: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RemoveDeviceRequest {
    pub device_id: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct ReparentCircuitRequest {
    pub circuit_id: String,
    pub parent_node: String,
}

#[utoipa::path(
    get,
    path = "/overrides/adjustments",
    summary = "List current circuit and device adjustments",
    description = "List current circuit and device adjustments.",
    responses(
        (status = 200, description = "List current circuit adjustments.", body = Vec<CircuitAdjustmentTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error loading overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_adjustments_list()
-> Result<Json<Vec<CircuitAdjustmentTransit>>, (StatusCode, String)> {
    let overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    Ok(Json(
        overrides
            .circuit_adjustments()
            .iter()
            .cloned()
            .map(CircuitAdjustmentTransit::from)
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/overrides/adjustments/circuit_speed",
    summary = "Add a circuit speed adjustment",
    description = "Add a circuit speed adjustment.",
    request_body = AddCircuitSpeedRequest,
    responses(
        (status = 200, description = "Add circuit speed adjustment.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_adjustments_add_circuit_speed(
    Json(req): Json<AddCircuitSpeedRequest>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let adj = CircuitAdjustment::CircuitAdjustSpeed {
        circuit_id: req.circuit_id,
        min_download_bandwidth: req.min_download_bandwidth,
        max_download_bandwidth: req.max_download_bandwidth,
        min_upload_bandwidth: req.min_upload_bandwidth,
        max_upload_bandwidth: req.max_upload_bandwidth,
    };
    overrides.add_circuit_adjustment(adj);
    overrides.save().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save overrides: {}", e),
        )
    })?;
    Ok(Json(OverrideActionResponse {
        message: "Added circuit speed adjustment; overrides saved.".to_string(),
        changed: Some(true),
    }))
}

#[utoipa::path(
    post,
    path = "/overrides/adjustments/device_speed",
    summary = "Add a device speed adjustment",
    description = "Add a device speed adjustment.",
    request_body = AddDeviceSpeedRequest,
    responses(
        (status = 200, description = "Add device speed adjustment.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_adjustments_add_device_speed(
    Json(req): Json<AddDeviceSpeedRequest>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let adj = CircuitAdjustment::DeviceAdjustSpeed {
        device_id: req.device_id,
        min_download_bandwidth: req.min_download_bandwidth,
        max_download_bandwidth: req.max_download_bandwidth,
        min_upload_bandwidth: req.min_upload_bandwidth,
        max_upload_bandwidth: req.max_upload_bandwidth,
    };
    overrides.add_circuit_adjustment(adj);
    overrides.save().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save overrides: {}", e),
        )
    })?;
    Ok(Json(OverrideActionResponse {
        message: "Added device speed adjustment; overrides saved.".to_string(),
        changed: Some(true),
    }))
}

#[utoipa::path(
    post,
    path = "/overrides/adjustments/remove_circuit",
    summary = "Add a remove-circuit adjustment",
    description = "Add a remove-circuit adjustment.",
    request_body = RemoveCircuitRequest,
    responses(
        (status = 200, description = "Add remove-circuit adjustment.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_adjustments_add_remove_circuit(
    Json(req): Json<RemoveCircuitRequest>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    overrides.add_circuit_adjustment(CircuitAdjustment::RemoveCircuit {
        circuit_id: req.circuit_id,
    });
    overrides.save().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save overrides: {}", e),
        )
    })?;
    Ok(Json(OverrideActionResponse {
        message: "Added remove-circuit adjustment; overrides saved.".to_string(),
        changed: Some(true),
    }))
}

#[utoipa::path(
    post,
    path = "/overrides/adjustments/remove_device",
    summary = "Add a remove-device adjustment",
    description = "Add a remove-device adjustment.",
    request_body = RemoveDeviceRequest,
    responses(
        (status = 200, description = "Add remove-device adjustment.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_adjustments_add_remove_device(
    Json(req): Json<RemoveDeviceRequest>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    overrides.add_circuit_adjustment(CircuitAdjustment::RemoveDevice {
        device_id: req.device_id,
    });
    overrides.save().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save overrides: {}", e),
        )
    })?;
    Ok(Json(OverrideActionResponse {
        message: "Added remove-device adjustment; overrides saved.".to_string(),
        changed: Some(true),
    }))
}

#[utoipa::path(
    post,
    path = "/overrides/adjustments/reparent_circuit",
    summary = "Add a reparent-circuit adjustment",
    description = "Add a reparent-circuit adjustment.",
    request_body = ReparentCircuitRequest,
    responses(
        (status = 200, description = "Add reparent-circuit adjustment.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_adjustments_add_reparent_circuit(
    Json(req): Json<ReparentCircuitRequest>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    overrides.add_circuit_adjustment(CircuitAdjustment::ReparentCircuit {
        circuit_id: req.circuit_id,
        parent_node: req.parent_node,
    });
    overrides.save().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save overrides: {}", e),
        )
    })?;
    Ok(Json(OverrideActionResponse {
        message: "Added reparent-circuit adjustment; overrides saved.".to_string(),
        changed: Some(true),
    }))
}

#[utoipa::path(
    delete,
    path = "/overrides/adjustments/{index}",
    summary = "Remove a circuit adjustment by index",
    description = "Remove a circuit adjustment by index.",
    responses(
        (status = 200, description = "Delete circuit adjustment by index.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
        ("index" = usize, Path, description = "Adjustment index", example = 0),
    ),
)]
pub async fn overrides_adjustments_delete_index(
    Path(index): Path<usize>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let removed = overrides.remove_circuit_adjustment_by_index(index);
    if removed {
        overrides.save().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save overrides: {}", e),
            )
        })?;
        Ok(Json(OverrideActionResponse {
            message: format!("Removed adjustment at index {index}; overrides saved."),
            changed: Some(true),
        }))
    } else {
        Ok(Json(OverrideActionResponse {
            message: format!("No adjustment at index {index}."),
            changed: Some(false),
        }))
    }
}

// --------------------
// Network Adjustments
// --------------------

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NetworkAdjustmentTransit {
    AdjustSiteSpeed {
        #[serde(skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
        site_name: String,
        download_bandwidth_mbps: Option<f32>,
        upload_bandwidth_mbps: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at_unix: Option<u64>,
    },
    TopologyParentOverride {
        node_id: String,
        node_name: String,
        mode: TopologyParentOverrideModeTransit,
        parent_node_ids: Vec<String>,
        parent_node_names: Vec<String>,
    },
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TopologyParentOverrideModeTransit {
    Pinned,
    PreferredOrder,
}

impl From<TopologyParentOverrideMode> for TopologyParentOverrideModeTransit {
    fn from(value: TopologyParentOverrideMode) -> Self {
        match value {
            TopologyParentOverrideMode::Pinned => Self::Pinned,
            TopologyParentOverrideMode::PreferredOrder => Self::PreferredOrder,
        }
    }
}

impl From<NetworkAdjustment> for NetworkAdjustmentTransit {
    fn from(value: NetworkAdjustment) -> Self {
        match value {
            NetworkAdjustment::AdjustSiteSpeed {
                node_id,
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
                expires_at_unix,
            } => Self::AdjustSiteSpeed {
                node_id,
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
                expires_at_unix,
            },
            NetworkAdjustment::TopologyParentOverride {
                node_id,
                node_name,
                mode,
                parent_node_ids,
                parent_node_names,
            } => Self::TopologyParentOverride {
                node_id,
                node_name,
                mode: mode.into(),
                parent_node_ids,
                parent_node_names,
            },
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default)]
/// Operator-authored site-speed write. `expires_at_unix` is intentionally not
/// accepted here: it is the machine-authored (StormGuard) dead-man switch and
/// is exposed on the response transit instead. Operator entries are immortal.
pub struct AddSiteSpeedRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub site_name: String,
    pub download_bandwidth_mbps: Option<f32>,
    pub upload_bandwidth_mbps: Option<f32>,
}

#[utoipa::path(
    get,
    path = "/overrides/network_adjustments",
    summary = "List current network/site adjustments",
    description = "List current network/site adjustments.",
    responses(
        (status = 200, description = "List network adjustments.", body = Vec<NetworkAdjustmentTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error loading overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_network_adjustments_list()
-> Result<Json<Vec<NetworkAdjustmentTransit>>, (StatusCode, String)> {
    let overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    Ok(Json(
        overrides
            .network_adjustments()
            .iter()
            .cloned()
            .map(NetworkAdjustmentTransit::from)
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/overrides/network_adjustments/site_speed",
    summary = "Add a site speed adjustment",
    description = "Add a site speed adjustment.",
    request_body = AddSiteSpeedRequest,
    responses(
        (status = 200, description = "Add site speed adjustment.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_network_adjustments_add_site_speed(
    Json(req): Json<AddSiteSpeedRequest>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    overrides.add_network_adjustment(NetworkAdjustment::AdjustSiteSpeed {
        node_id: req.node_id,
        site_name: req.site_name,
        download_bandwidth_mbps: req.download_bandwidth_mbps,
        upload_bandwidth_mbps: req.upload_bandwidth_mbps,
        expires_at_unix: None,
    });
    overrides.save().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save overrides: {}", e),
        )
    })?;
    Ok(Json(OverrideActionResponse {
        message: "Added site speed adjustment; overrides saved.".to_string(),
        changed: Some(true),
    }))
}

#[utoipa::path(
    delete,
    path = "/overrides/network_adjustments/{index}",
    summary = "Remove a network adjustment by index",
    description = "Remove a network adjustment by index.",
    responses(
        (status = 200, description = "Delete network adjustment by index.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
        ("index" = usize, Path, description = "Network adjustment index", example = 0),
    ),
)]
pub async fn overrides_network_adjustments_delete_index(
    Path(index): Path<usize>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let removed = overrides.remove_network_adjustment_by_index(index);
    if removed {
        overrides.save().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save overrides: {}", e),
            )
        })?;
        Ok(Json(OverrideActionResponse {
            message: format!("Removed network adjustment at index {index}; overrides saved."),
            changed: Some(true),
        }))
    } else {
        Ok(Json(OverrideActionResponse {
            message: format!("No network adjustment at index {index}."),
            changed: Some(false),
        }))
    }
}

// ---------------
// UISP overrides
// ---------------

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct UispBandwidthOverrideEntry {
    pub site_name: String,
    pub down: f32,
    pub up: f32,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct UispRouteOverrideTransit {
    pub from_site: String,
    pub to_site: String,
    pub cost: u32,
}

impl From<UispRouteOverride> for UispRouteOverrideTransit {
    fn from(value: UispRouteOverride) -> Self {
        Self {
            from_site: value.from_site,
            to_site: value.to_site,
            cost: value.cost,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct BandwidthSetRequest {
    pub site_name: String,
    pub down: f32,
    pub up: f32,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RouteAddRequest {
    pub from_site: String,
    pub to_site: String,
    pub cost: u32,
}

#[utoipa::path(
    get,
    path = "/overrides/uisp/bandwidth",
    summary = "List UISP bandwidth overrides",
    description = "List UISP bandwidth overrides.",
    responses(
        (status = 200, description = "List UISP bandwidth overrides.", body = Vec<UispBandwidthOverrideEntry>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error loading overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_uisp_bandwidth_list()
-> Result<Json<Vec<UispBandwidthOverrideEntry>>, (StatusCode, String)> {
    let overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let list = if let Some(uisp) = overrides.uisp() {
        uisp.bandwidth_overrides
            .iter()
            .map(|(site_name, (down, up))| UispBandwidthOverrideEntry {
                site_name: site_name.clone(),
                down: *down,
                up: *up,
            })
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    Ok(Json(list))
}

#[utoipa::path(
    put,
    path = "/overrides/uisp/bandwidth",
    summary = "Set a UISP bandwidth override for a site",
    description = "Set a UISP bandwidth override for a site.",
    request_body = BandwidthSetRequest,
    responses(
        (status = 200, description = "Set UISP bandwidth override.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_uisp_bandwidth_set(
    Json(req): Json<BandwidthSetRequest>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    overrides.set_uisp_bandwidth_override(req.site_name, req.down, req.up);
    overrides.save().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save overrides: {}", e),
        )
    })?;
    Ok(Json(OverrideActionResponse {
        message: "Set UISP bandwidth override; overrides saved.".to_string(),
        changed: Some(true),
    }))
}

#[utoipa::path(
    delete,
    path = "/overrides/uisp/bandwidth/{site_name}",
    summary = "Remove a UISP bandwidth override for a site",
    description = "Remove a UISP bandwidth override for a site.",
    responses(
        (status = 200, description = "Remove UISP bandwidth override.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
        ("site_name" = String, Path, description = "Site name", example = "example_site"),
    ),
)]
pub async fn overrides_uisp_bandwidth_remove(
    Path(site_name): Path<String>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let removed = overrides.remove_uisp_bandwidth_override(&site_name);
    if removed {
        overrides.save().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save overrides: {}", e),
            )
        })?;
        Ok(Json(OverrideActionResponse {
            message: format!("Removed UISP bandwidth override for {site_name}; overrides saved."),
            changed: Some(true),
        }))
    } else {
        Ok(Json(OverrideActionResponse {
            message: format!("No UISP bandwidth override for {site_name}."),
            changed: Some(false),
        }))
    }
}

#[utoipa::path(
    get,
    path = "/overrides/uisp/routes",
    summary = "List UISP route overrides",
    description = "List UISP route overrides.",
    responses(
        (status = 200, description = "List UISP route overrides.", body = Vec<UispRouteOverrideTransit>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error loading overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_uisp_route_list()
-> Result<Json<Vec<UispRouteOverrideTransit>>, (StatusCode, String)> {
    let overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let list = if let Some(uisp) = overrides.uisp() {
        uisp.route_overrides
            .iter()
            .cloned()
            .map(UispRouteOverrideTransit::from)
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    Ok(Json(list))
}

#[utoipa::path(
    post,
    path = "/overrides/uisp/routes",
    summary = "Add a UISP route override",
    description = "Add a UISP route override.",
    request_body = RouteAddRequest,
    responses(
        (status = 200, description = "Add UISP route override.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>")),
)]
pub async fn overrides_uisp_route_add(
    Json(req): Json<RouteAddRequest>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    overrides.add_uisp_route_override(req.from_site, req.to_site, req.cost);
    overrides.save().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save overrides: {}", e),
        )
    })?;
    Ok(Json(OverrideActionResponse {
        message: "Added UISP route override; overrides saved.".to_string(),
        changed: Some(true),
    }))
}

#[utoipa::path(
    delete,
    path = "/overrides/uisp/routes/{index}",
    summary = "Remove a UISP route override by index",
    description = "Remove a UISP route override by index.",
    responses(
        (status = 200, description = "Remove UISP route override by index.", body = OverrideActionResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Internal error saving overrides.", body = String),
    ),
    tag = "Overrides",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
        ("index" = usize, Path, description = "Route index", example = 0),
    ),
)]
pub async fn overrides_uisp_route_remove_index(
    Path(index): Path<usize>,
) -> Result<Json<OverrideActionResponse>, (StatusCode, String)> {
    let mut overrides = OverrideFile::load().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load overrides: {}", e),
        )
    })?;
    let removed = overrides.remove_uisp_route_by_index(index);
    if removed {
        overrides.save().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save overrides: {}", e),
            )
        })?;
        Ok(Json(OverrideActionResponse {
            message: format!("Removed UISP route override at index {index}; overrides saved."),
            changed: Some(true),
        }))
    } else {
        Ok(Json(OverrideActionResponse {
            message: format!("No UISP route override at index {index}."),
            changed: Some(false),
        }))
    }
}

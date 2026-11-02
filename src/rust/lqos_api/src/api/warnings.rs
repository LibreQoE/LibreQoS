use axum::{Json, http::StatusCode};
use lqos_bus::{
    BusRequest, BusResponse, LibreqosBusClient, UrgentIssue, WarningLevel as BusWarningLevel,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub enum WarningLevel {
    Info,
    Warning,
    Error,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct WarningEntry {
    pub level: WarningLevel,
    pub message: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct UrgentStatusData {
    pub has_urgent: bool,
    pub count: usize,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct UrgentItem {
    pub id: u64,
    pub ts: u64,
    pub source: String,
    pub severity: String,
    pub code: String,
    pub message: String,
    pub context: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct UrgentList {
    pub items: Vec<UrgentItem>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct UrgentClearResponse {
    pub ok: bool,
}

fn map_warning_level(level: BusWarningLevel) -> WarningLevel {
    match level {
        BusWarningLevel::Info => WarningLevel::Info,
        BusWarningLevel::Warning => WarningLevel::Warning,
        BusWarningLevel::Error => WarningLevel::Error,
    }
}

fn urgent_issue_to_item(issue: UrgentIssue) -> UrgentItem {
    UrgentItem {
        id: issue.id,
        ts: issue.ts,
        source: format!("{:?}", issue.source),
        severity: format!("{:?}", issue.severity),
        code: issue.code,
        message: issue.message,
        context: issue.context,
    }
}

#[utoipa::path(
    get,
    path = "/warnings",
    summary = "Global warnings list",
    description = "Return global warnings emitted by system checks.",
    responses(
        (status = 200, description = "Return 200 OK and a list of warnings.", body = Vec<WarningEntry>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "warnings",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn warnings() -> Result<(StatusCode, Json<Vec<WarningEntry>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetGlobalWarnings])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::GlobalWarnings(warnings) = response {
            let result = warnings
                .into_iter()
                .map(|(level, message)| WarningEntry {
                    level: map_warning_level(level),
                    message,
                })
                .collect();
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/urgent",
    summary = "Urgent issues list",
    description = "Return the current urgent issues list.",
    responses(
        (status = 200, description = "Return 200 OK and urgent issues list.", body = UrgentList),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "warnings",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn urgent_list() -> Result<(StatusCode, Json<UrgentList>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetUrgentIssues])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::UrgentIssues(list) = response {
            let items = list.into_iter().map(urgent_issue_to_item).collect();
            return Ok((StatusCode::OK, Json(UrgentList { items })));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/urgent/status",
    summary = "Urgent issues status summary",
    description = "Return whether urgent issues are present and how many exist.",
    responses(
        (status = 200, description = "Return 200 OK and urgent status summary.", body = UrgentStatusData),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "warnings",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn urgent_status() -> Result<(StatusCode, Json<UrgentStatusData>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetUrgentIssues])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::UrgentIssues(list) = response {
            let count = list.len();
            let data = UrgentStatusData {
                has_urgent: count > 0,
                count,
            };
            return Ok((StatusCode::OK, Json(data)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    delete,
    path = "/urgent/{id}",
    summary = "Clear an urgent issue by ID",
    description = "Clear a single urgent issue by ID.",
    responses(
        (status = 200, description = "Return 200 OK if the clear request was accepted.", body = UrgentClearResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "warnings",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("id" = u64, Path, description = "Urgent issue ID to clear."),
    ),
)]
pub async fn urgent_clear(
    axum::extract::Path(id): axum::extract::Path<u64>,
) -> Result<(StatusCode, Json<UrgentClearResponse>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::ClearUrgentIssue(id)])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if response == BusResponse::Ack {
            return Ok((StatusCode::OK, Json(UrgentClearResponse { ok: true })));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    post,
    path = "/urgent/clear_all",
    summary = "Clear all urgent issues",
    description = "Clear all urgent issues currently stored on the node.",
    responses(
        (status = 200, description = "Return 200 OK if the clear request was accepted.", body = UrgentClearResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "warnings",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn urgent_clear_all()
-> Result<(StatusCode, Json<UrgentClearResponse>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::ClearAllUrgentIssues])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if response == BusResponse::Ack {
            return Ok((StatusCode::OK, Json(UrgentClearResponse { ok: true })));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

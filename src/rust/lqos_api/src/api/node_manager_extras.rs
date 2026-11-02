use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::transit_types::{DownUpTransit, NetworkJsonTransit};

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct SchedulerDetailsData {
    pub available: bool,
    pub error: Option<String>,
    pub details: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct QueueStatsTotalData {
    pub marks: DownUpTransit,
    pub drops: DownUpTransit,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct CircuitCapacityRowData {
    pub circuit_id: String,
    pub circuit_name: String,
    pub capacity: [f64; 2],
    pub median_rtt: f32,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct NodeCapacityData {
    pub id: usize,
    pub name: String,
    pub down: f64,
    pub up: f64,
    pub max_down: f64,
    pub max_up: f64,
    pub median_rtt: f32,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RetransmitSummaryData {
    pub up: i32,
    pub down: i32,
    pub tcp_up: u64,
    pub tcp_down: u64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct TreeSummaryL2Child {
    pub id: usize,
    pub node: NetworkJsonTransit,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct TreeSummaryL2Entry {
    pub parent_id: usize,
    pub children: Vec<TreeSummaryL2Child>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchResultData {
    Circuit {
        id: String,
        name: String,
    },
    Device {
        circuit_id: String,
        name: String,
        circuit_name: String,
    },
    Site {
        idx: usize,
        name: String,
    },
}

#[utoipa::path(
    get,
    path = "/scheduler_details",
    summary = "Scheduler diagnostics details",
    description = "Return detailed scheduler diagnostics and availability.",
    responses(
        (status = 200, description = "Return 200 OK and scheduler details.", body = SchedulerDetailsData),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Control & Reload",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn scheduler_details()
-> Result<(StatusCode, Json<SchedulerDetailsData>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetSchedulerDetails])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::SchedulerDetails(details) = response {
            let result = SchedulerDetailsData {
                available: details.available,
                error: details.error,
                details: details.details,
            };
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
    path = "/queue_stats_total",
    summary = "Queue marks and drops totals",
    description = "Return total queue marks and drops for download and upload.",
    responses(
        (status = 200, description = "Return 200 OK and queue totals.", body = QueueStatsTotalData),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn queue_stats_total()
-> Result<(StatusCode, Json<QueueStatsTotalData>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetQueueStatsTotal])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::QueueStatsTotal(data) = response {
            let result = QueueStatsTotalData {
                marks: DownUpTransit::from(data.marks),
                drops: DownUpTransit::from(data.drops),
            };
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
    path = "/circuit_capacity",
    summary = "Circuit capacity utilization list",
    description = "Return circuit capacity utilization rows.",
    responses(
        (status = 200, description = "Return 200 OK and circuit capacity data.", body = Vec<CircuitCapacityRowData>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn circuit_capacity()
-> Result<(StatusCode, Json<Vec<CircuitCapacityRowData>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetCircuitCapacity])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::CircuitCapacity(rows) = response {
            let result = rows
                .into_iter()
                .map(|row| CircuitCapacityRowData {
                    circuit_id: row.circuit_id,
                    circuit_name: row.circuit_name,
                    capacity: row.capacity,
                    median_rtt: row.median_rtt,
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
    path = "/tree_capacity",
    summary = "Tree capacity utilization list",
    description = "Return node capacity utilization rows for the network tree.",
    responses(
        (status = 200, description = "Return 200 OK and tree capacity data.", body = Vec<NodeCapacityData>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn tree_capacity()
-> Result<(StatusCode, Json<Vec<NodeCapacityData>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetTreeCapacity])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::TreeCapacity(rows) = response {
            let result = rows
                .into_iter()
                .map(|row| NodeCapacityData {
                    id: row.id,
                    name: row.name,
                    down: row.down,
                    up: row.up,
                    max_down: row.max_down,
                    max_up: row.max_up,
                    median_rtt: row.median_rtt,
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
    path = "/retransmit_summary",
    summary = "Aggregate TCP retransmit summary",
    description = "Return aggregate TCP retransmit totals and TCP packet counts.",
    responses(
        (status = 200, description = "Return 200 OK and retransmit summary.", body = RetransmitSummaryData),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn retransmit_summary()
-> Result<(StatusCode, Json<RetransmitSummaryData>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetRetransmitSummary])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::RetransmitSummary(data) = response {
            let result = RetransmitSummaryData {
                up: data.up,
                down: data.down,
                tcp_up: data.tcp_up,
                tcp_down: data.tcp_down,
            };
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
    path = "/tree_summary_l2",
    summary = "Two-level tree summary",
    description = "Return curated two-level tree summary entries.",
    responses(
        (status = 200, description = "Return 200 OK and tree summary data.", body = Vec<TreeSummaryL2Entry>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Network Map",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn tree_summary_l2()
-> Result<(StatusCode, Json<Vec<TreeSummaryL2Entry>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetTreeSummaryL2])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::TreeSummaryL2(data) = response {
            let result = data
                .into_iter()
                .map(|(parent_id, children)| TreeSummaryL2Entry {
                    parent_id,
                    children: children
                        .into_iter()
                        .map(|(id, node)| TreeSummaryL2Child {
                            id,
                            node: NetworkJsonTransit::from(node),
                        })
                        .collect(),
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
    path = "/search",
    summary = "Search circuits, devices, and sites",
    description = "Search circuits, devices, and sites by term.",
    responses(
        (status = 200, description = "Return 200 OK and search results.", body = Vec<SearchResultData>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("term" = String, Query, description = "Search term (IP, CIDR, circuit/device/site name)."),
    ),
)]
pub async fn search(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<(StatusCode, Json<Vec<SearchResultData>>), (StatusCode, String)> {
    let Some(term) = params.get("term") else {
        return Err((StatusCode::BAD_REQUEST, "Missing term".to_string()));
    };

    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::Search { term: term.clone() }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::SearchResults(list) = response {
            let result = list
                .into_iter()
                .map(|entry| match entry {
                    lqos_bus::SearchResultEntry::Circuit { id, name } => {
                        SearchResultData::Circuit { id, name }
                    }
                    lqos_bus::SearchResultEntry::Device {
                        circuit_id,
                        name,
                        circuit_name,
                    } => SearchResultData::Device {
                        circuit_id,
                        name,
                        circuit_name,
                    },
                    lqos_bus::SearchResultEntry::Site { idx, name } => {
                        SearchResultData::Site { idx, name }
                    }
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

use axum::{Json, extract::State, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use std::sync::Arc;

use crate::{api::ApiState, transit_types::IpstatsTransit};

#[utoipa::path(
    get,
    path = "/top_n_downloaders/{start}/{end}",
    summary = "Top downloaders within the requested slice of results",
    description = "Return the top downloaders within the requested slice of results.",
    responses(
        (status = 200, description = "Return 200 OK and a JSON array with top N downloaders.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("start" = u32, Path, description = "The start index of the top N downloaders to return (0-based).", example = 0),
        ("end" = u32, Path, description = "The end index of the top N downloaders to return (exclusive).", example = 10),
    ),
)]
pub async fn top_n_downloaders(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path((start, end)): axum::extract::Path<(u32, u32)>,
) -> Result<(StatusCode, Json<Vec<IpstatsTransit>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetTopNDownloaders { start, end }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::TopDownloaders(stats) = response {
            let name_map = state.circuit_names();
            let transit_stats: Vec<IpstatsTransit> = stats
                .into_iter()
                .map(|s| {
                    let mut t = IpstatsTransit::from(s);
                    if let Some(name) = name_map.get(&t.circuit_id) {
                        t.circuit_name = Some(name.clone());
                    }
                    t
                })
                .collect();
            return Ok((StatusCode::OK, Json(transit_stats)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/top_n_uploaders/{start}/{end}",
    summary = "Top uploaders within the requested slice of results",
    description = "Return the top uploaders within the requested slice of results.",
    responses(
        (status = 200, description = "Return 200 OK and a JSON array with top N uploaders.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("start" = u32, Path, description = "The start index of the top N uploaders to return (0-based).", example = 0),
        ("end" = u32, Path, description = "The end index of the top N uploaders to return (exclusive).", example = 10),
    ),
)]
pub async fn top_n_uploaders(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path((start, end)): axum::extract::Path<(u32, u32)>,
) -> Result<(StatusCode, Json<Vec<IpstatsTransit>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetTopNUploaders { start, end }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::TopUploaders(stats) = response {
            let name_map = state.circuit_names();
            let transit_stats: Vec<IpstatsTransit> = stats
                .into_iter()
                .map(|s| {
                    let mut t = IpstatsTransit::from(s);
                    if let Some(name) = name_map.get(&t.circuit_id) {
                        t.circuit_name = Some(name.clone());
                    }
                    t
                })
                .collect();
            return Ok((StatusCode::OK, Json(transit_stats)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/worst_rtt/{start}/{end}",
    summary = "Hosts with the worst RTT within the requested slice of results",
    description = "Return hosts with the worst RTT within the requested slice of results.",
    responses(
        (status = 200, description = "Return 200 OK and a JSON array with hosts with worst RTT.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("start" = u32, Path, description = "The start index of the worst RTT hosts to return (0-based).", example = 0),
        ("end" = u32, Path, description = "The end index of the worst RTT hosts to return (exclusive).", example = 10),
    ),
)]
pub async fn worst_rtt(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path((start, end)): axum::extract::Path<(u32, u32)>,
) -> Result<(StatusCode, Json<Vec<IpstatsTransit>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetWorstRtt { start, end }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::WorstRtt(stats) = response {
            let name_map = state.circuit_names();
            let stats: Vec<IpstatsTransit> = stats
                .into_iter()
                .map(|s| {
                    let mut t = IpstatsTransit::from(s);
                    if let Some(name) = name_map.get(&t.circuit_id) {
                        t.circuit_name = Some(name.clone());
                    }
                    t
                })
                .collect();
            return Ok((StatusCode::OK, Json(stats)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/worst_retransmits/{start}/{end}",
    summary = "Hosts with the worst retransmit rates within the requested slice of results",
    description = "Return hosts with the worst retransmit rates within the requested slice of results.",
    responses(
        (status = 200, description = "Return 200 OK and a JSON array with hosts with worst retransmits.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("start" = u32, Path, description = "The start index of the worst retransmits hosts to return (0-based).", example = 0),
        ("end" = u32, Path, description = "The end index of the worst retransmits hosts to return (exclusive).", example = 10),
    ),
)]
pub async fn worst_retransmits(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path((start, end)): axum::extract::Path<(u32, u32)>,
) -> Result<(StatusCode, Json<Vec<IpstatsTransit>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetWorstRetransmits { start, end }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::WorstRetransmits(stats) = response {
            let name_map = state.circuit_names();
            let stats: Vec<IpstatsTransit> = stats
                .into_iter()
                .map(|s| {
                    let mut t = IpstatsTransit::from(s);
                    if let Some(name) = name_map.get(&t.circuit_id) {
                        t.circuit_name = Some(name.clone());
                    }
                    t
                })
                .collect();
            return Ok((StatusCode::OK, Json(stats)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/best_rtt/{start}/{end}",
    summary = "Hosts with the best RTT within the requested slice of results",
    description = "Return hosts with the best RTT within the requested slice of results.",
    responses(
        (status = 200, description = "Return 200 OK and a JSON array with hosts with best RTT.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("start" = u32, Path, description = "The start index of the best RTT hosts to return (0-based).", example = 0),
        ("end" = u32, Path, description = "The end index of the best RTT hosts to return (exclusive).", example = 10),
    ),
)]
pub async fn best_rtt(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path((start, end)): axum::extract::Path<(u32, u32)>,
) -> Result<(StatusCode, Json<Vec<IpstatsTransit>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetBestRtt { start, end }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::BestRtt(stats) = response {
            let name_map = state.circuit_names();
            let stats: Vec<IpstatsTransit> = stats
                .into_iter()
                .map(|s| {
                    let mut t = IpstatsTransit::from(s);
                    if let Some(name) = name_map.get(&t.circuit_id) {
                        t.circuit_name = Some(name.clone());
                    }
                    t
                })
                .collect();
            return Ok((StatusCode::OK, Json(stats)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

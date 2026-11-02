use crate::transit_types::NetworkJsonTransit;
use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct TreeSummaryData {
    /// Tree summary data as array of [id, network_data] tuples for dashboard consumption
    summary: Vec<(usize, NetworkJsonTransit)>,
}

#[utoipa::path(
    get,
    path = "/tree_summary",
    summary = "Summary of the network tree structure",
    description = "Return a summary of the network tree structure.",
    responses(
        (status = 200, description = "Return 200 OK and network tree summary data for Sankey diagram and tree table.", body = TreeSummaryData),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Network Map",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn tree_summary() -> Result<(StatusCode, Json<TreeSummaryData>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    // Get top-level network tree (parent = 0) as used by TreeSummary websocket
    let responses = client
        .request(vec![BusRequest::GetNetworkMap { parent: 0 }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;

    for response in responses {
        if let BusResponse::NetworkMap(data) = response {
            let summary = data
                .into_iter()
                .map(|q| (q.0, NetworkJsonTransit::from(q.1)))
                .collect();
            let result = TreeSummaryData { summary };
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

use axum::{Json, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::{
    circuits_at_capacity, circuits_high_retransmits, current_endpoint_latlon,
    current_endpoints_by_country, ether_protocol_summary, ip_protocol_summary,
    tree_nodes_at_capacity, tree_nodes_high_retransmits,
};

#[derive(Serialize, ToSchema)]
pub struct CapacityStatus {
    pub circuits: crate::api::circuits_at_capacity::CircuitsAtCapacityResponse,
    pub nodes: crate::api::tree_nodes_at_capacity::TreeNodesAtCapacityResponse,
}

#[utoipa::path(
    get,
    path = "/capacity_status",
    summary = "Combined capacity status for circuits and tree nodes",
    description = "Return combined capacity status for circuits and tree nodes.",
    responses(
        (status = 200, description = "Combined circuits and tree nodes capacity status.", body = CapacityStatus),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
        (status = 500, description = "Internal error", body = String),
    ),
    tag = "Circuit Analysis",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.")),
)]
pub async fn capacity_status() -> Result<(StatusCode, Json<CapacityStatus>), (StatusCode, String)> {
    let circuits = circuits_at_capacity().await?.0;
    let nodes = tree_nodes_at_capacity().await?.0;
    Ok((StatusCode::OK, Json(CapacityStatus { circuits, nodes })))
}

#[derive(Serialize, ToSchema)]
pub struct RetransmitStatus {
    pub circuits: crate::api::circuits_high_retransmits::CircuitsHighRetransmitsResponse,
    pub nodes: crate::api::tree_nodes_high_retransmits::TreeNodesHighRetransmitsResponse,
}

#[utoipa::path(
    get,
    path = "/retransmit_status",
    summary = "Combined retransmit status for circuits and tree nodes",
    description = "Return combined retransmit status for circuits and tree nodes.",
    responses(
        (status = 200, description = "Combined circuits and tree nodes retransmit status.", body = RetransmitStatus),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
        (status = 500, description = "Internal error", body = String),
    ),
    tag = "Circuit Analysis",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.")),
)]
pub async fn retransmit_status()
-> Result<(StatusCode, Json<RetransmitStatus>), (StatusCode, String)> {
    let circuits = circuits_high_retransmits().await?.0;
    let nodes = tree_nodes_high_retransmits().await?.0;
    Ok((StatusCode::OK, Json(RetransmitStatus { circuits, nodes })))
}

#[utoipa::path(
    get,
    path = "/protocol_status",
    summary = "Combined protocol summary for ethernet and IP protocols",
    description = "Return combined protocol summary for ethernet and IP protocols.",
    responses(
        (status = 200, description = "Ethernet and IP protocol summaries.", body = String),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
        (status = 500, description = "Internal error", body = String),
    ),
    tag = "Protocol Metrics",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.")),
)]
pub async fn protocol_status() -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)>
{
    let ether = ether_protocol_summary().await?.1.0; // unwrap Json to Value
    let ip = ip_protocol_summary().await?.1.0; // unwrap Json to inner Vec
    let out = serde_json::json!({ "ether": ether, "ip": ip });
    Ok((StatusCode::OK, Json(out)))
}

#[utoipa::path(
    get,
    path = "/endpoints_status",
    summary = "Endpoint distribution by country with a recent lat/lon sample",
    description = "Return endpoint distribution by country with a recent lat/lon sample.",
    responses(
        (status = 200, description = "Endpoints by country and recent lat/lon sample.", body = String),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
        (status = 500, description = "Internal error", body = String),
    ),
    tag = "Endpoint Metrics",
    params(("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.")),
)]
pub async fn endpoints_status()
-> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let by_country = current_endpoints_by_country().await?.1.0;
    let latlon = current_endpoint_latlon().await?.1.0;
    let out = serde_json::json!({ "by_country": by_country, "latlon": latlon });
    Ok((StatusCode::OK, Json(out)))
}

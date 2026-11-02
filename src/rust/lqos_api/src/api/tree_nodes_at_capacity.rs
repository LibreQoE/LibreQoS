use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::transit_types::NetworkJsonTransit;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct TreeNodeCapacityData {
    /// The node ID in the network tree
    pub node_id: usize,

    /// The node display name
    pub node_name: String,

    /// Current download throughput in bits per second
    pub current_download_bps: u64,

    /// Current upload throughput in bits per second
    pub current_upload_bps: u64,

    /// Maximum configured download capacity in bits per second
    pub max_download_bps: u64,

    /// Maximum configured upload capacity in bits per second
    pub max_upload_bps: u64,

    /// Download utilization percentage (0-100+)
    pub download_utilization_percent: f32,

    /// Upload utilization percentage (0-100+)
    pub upload_utilization_percent: f32,

    /// True if node is at/near capacity (>= threshold)
    pub at_capacity: bool,

    /// The immediate parent node in the tree (if any)
    pub immediate_parent: Option<usize>,

    /// The type of node (site, ap, etc.)
    pub node_type: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct TreeNodesAtCapacityResponse {
    /// List of all tree nodes with their capacity analysis
    pub nodes: Vec<TreeNodeCapacityData>,

    /// Number of nodes at or near capacity
    pub nodes_at_capacity_count: usize,

    /// Percentage threshold used to determine "at capacity"
    pub capacity_threshold_percent: f32,
}

#[utoipa::path(
    get,
    path = "/tree_nodes_at_capacity",
    summary = "Analyze tree nodes for near-capacity throughput",
    description = "Analyze tree nodes for near-capacity throughput.",
    responses(
        (status = 200, description = "Return 200 OK and tree node capacity analysis data.", body = TreeNodesAtCapacityResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn tree_nodes_at_capacity()
-> Result<Json<TreeNodesAtCapacityResponse>, (StatusCode, String)> {
    // Threshold for considering a node "at capacity" (default 85%)
    let capacity_threshold = 85.0;

    // Get full network tree data
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetFullNetworkMap])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;

    let network_nodes = responses
        .into_iter()
        .find_map(|response| {
            if let BusResponse::NetworkMap(nodes) = response {
                Some(
                    nodes
                        .into_iter()
                        .map(|(node_id, node)| (node_id, NetworkJsonTransit::from(node)))
                        .collect::<Vec<(usize, NetworkJsonTransit)>>(),
                )
            } else {
                None
            }
        })
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "No network map data received from bus".to_string(),
        ))?;

    // Analyze each tree node
    let mut node_analysis: Vec<TreeNodeCapacityData> = Vec::new();

    for (node_id, node) in network_nodes {
        // Convert Mbps to bits per second for max capacity
        let max_download_bps = (node.max_permitted_throughput_mbps.0.max(0.0) * 1_000_000.0) as u64;
        let max_upload_bps = (node.max_permitted_throughput_mbps.1.max(0.0) * 1_000_000.0) as u64;

        // Current throughput is already in bits per second
        let current_download_bps = node.current_throughput_bits_per_second.0;
        let current_upload_bps = node.current_throughput_bits_per_second.1;

        // Calculate utilization percentages
        let download_utilization_percent = if max_download_bps > 0 {
            (current_download_bps as f32 / max_download_bps as f32) * 100.0
        } else {
            0.0
        };

        let upload_utilization_percent = if max_upload_bps > 0 {
            (current_upload_bps as f32 / max_upload_bps as f32) * 100.0
        } else {
            0.0
        };

        let at_capacity = download_utilization_percent >= capacity_threshold
            || upload_utilization_percent >= capacity_threshold;

        node_analysis.push(TreeNodeCapacityData {
            node_id,
            node_name: node.name,
            current_download_bps,
            current_upload_bps,
            max_download_bps,
            max_upload_bps,
            download_utilization_percent,
            upload_utilization_percent,
            at_capacity,
            immediate_parent: node.immediate_parent,
            node_type: node.node_type,
        });
    }

    // Sort by highest utilization first
    node_analysis.sort_by(|a, b| {
        let a_max_util = a
            .download_utilization_percent
            .max(a.upload_utilization_percent);
        let b_max_util = b
            .download_utilization_percent
            .max(b.upload_utilization_percent);
        b_max_util
            .partial_cmp(&a_max_util)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let nodes_at_capacity_count = node_analysis.iter().filter(|n| n.at_capacity).count();

    Ok(Json(TreeNodesAtCapacityResponse {
        nodes: node_analysis,
        nodes_at_capacity_count,
        capacity_threshold_percent: capacity_threshold,
    }))
}

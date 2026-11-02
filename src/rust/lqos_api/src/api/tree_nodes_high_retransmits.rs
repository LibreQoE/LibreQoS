use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::transit_types::NetworkJsonTransit;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct TreeNodeRetransmitData {
    /// The node ID in the network tree
    pub node_id: usize,

    /// The node display name
    pub node_name: String,

    /// TCP retransmit percentage (download/upload) - None if no TCP traffic
    pub retransmit_percentage_download: Option<f64>,
    pub retransmit_percentage_upload: Option<f64>,

    /// Maximum retransmit percentage of download/upload (0.0 if no TCP traffic)
    pub max_retransmit_percentage: f64,

    /// True if node has high retransmits (>= threshold)
    pub high_retransmits: bool,

    /// Current throughput in bits per second (for context)
    pub current_throughput_bps: (u64, u64),

    /// Current TCP packets per second (for context)
    pub current_tcp_packets_per_second: (u64, u64),

    /// The immediate parent node in the tree (if any)
    pub immediate_parent: Option<usize>,

    /// The type of node (site, ap, etc.)
    pub node_type: Option<String>,

    /// Median RTT for this node
    pub tcp_rtt_median_ms: f32,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct TreeNodesHighRetransmitsResponse {
    /// List of all tree nodes with their retransmit analysis
    pub nodes: Vec<TreeNodeRetransmitData>,

    /// Number of nodes with high retransmits
    pub high_retransmit_count: usize,

    /// Percentage threshold used to determine "high retransmits"
    pub retransmit_threshold_percent: f64,
}

#[utoipa::path(
    get,
    path = "/tree_nodes_high_retransmits",
    summary = "Analyze tree nodes for high TCP retransmit rates",
    description = "Analyze tree nodes for high TCP retransmit rates.",
    responses(
        (status = 200, description = "Return 200 OK and tree node retransmit analysis data.", body = TreeNodesHighRetransmitsResponse),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Circuit Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn tree_nodes_high_retransmits()
-> Result<Json<TreeNodesHighRetransmitsResponse>, (StatusCode, String)> {
    // Threshold for considering a node to have "high retransmits" (default 1%)
    let retransmit_threshold = 1.0;

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

    // Analyze each tree node for retransmit rates
    let mut node_analysis: Vec<TreeNodeRetransmitData> = Vec::new();

    for (node_id, node) in network_nodes {
        // Extract retransmit percentages (already calculated as percentages)
        let (
            retransmit_percentage_download,
            retransmit_percentage_upload,
            max_retransmit_percentage,
        ) = if let Some((down_pct, up_pct)) = node.current_retransmits_percent {
            (Some(down_pct), Some(up_pct), down_pct.max(up_pct))
        } else {
            (None, None, 0.0)
        };

        let high_retransmits = max_retransmit_percentage >= retransmit_threshold;

        node_analysis.push(TreeNodeRetransmitData {
            node_id,
            node_name: node.name,
            retransmit_percentage_download,
            retransmit_percentage_upload,
            max_retransmit_percentage,
            high_retransmits,
            current_throughput_bps: node.current_throughput_bits_per_second,
            current_tcp_packets_per_second: node.current_tcp_packets_per_second,
            immediate_parent: node.immediate_parent,
            node_type: node.node_type,
            tcp_rtt_median_ms: node.tcp_round_trip_time_ms_median,
        });
    }

    // Sort by highest retransmit percentage first
    node_analysis.sort_by(|a, b| {
        b.max_retransmit_percentage
            .partial_cmp(&a.max_retransmit_percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let high_retransmit_count = node_analysis.iter().filter(|n| n.high_retransmits).count();

    Ok(Json(TreeNodesHighRetransmitsResponse {
        nodes: node_analysis,
        high_retransmit_count,
        retransmit_threshold_percent: retransmit_threshold,
    }))
}

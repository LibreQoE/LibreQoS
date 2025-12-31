use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::shaped_devices_tracker::NETWORK_JSON;
use lqos_config::NetworkJsonTransport;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Publishes a curated two-level summary of the network tree:
/// root -> first-level parents -> top-N second-level children across all parents.
/// N is currently fixed at 10. Only includes second-level items, grouped by parent id.
/// Data shape:
///   {
///     "event": "TreeSummaryL2",
///     "data": [ [ parent_id, [ [child_id, NetworkJsonTransport], ... ] ], ... ]
///   }
pub async fn tree_summary_l2(channels: Arc<PubSub>) {
    if !channels
        .is_channel_alive(PublishedChannels::TreeSummaryL2)
        .await
    {
        return;
    }

    // Build top-N second-level children across all first-level parents
    let grouped: Vec<(usize, Vec<(usize, NetworkJsonTransport)>)> = {
        let net_json = NETWORK_JSON.read();
        let nodes = net_json.get_nodes_when_ready();
        // Collect candidates as (parent_idx, child_idx, transport, total_bytes_per_sec)
        let mut candidates: Vec<(usize, usize, NetworkJsonTransport, u64)> = Vec::new();

        // Identify first-level parents (immediate_parent == Some(0))
        for (p_idx, p_node) in nodes.iter().enumerate() {
            if p_node.immediate_parent == Some(0) {
                // For each child-of-child under this parent
                for (c_idx, c_node) in nodes.iter().enumerate() {
                    if c_node.immediate_parent == Some(p_idx) {
                        let t = c_node.clone_to_transit();
                        let total = t.current_throughput.0 + t.current_throughput.1;
                        candidates.push((p_idx, c_idx, t, total));
                    }
                }
            }
        }

        // Sort by total throughput descending and cap to N
        candidates.sort_by(|a, b| b.3.cmp(&a.3));
        let n: usize = 10;
        if candidates.len() > n {
            candidates.truncate(n);
        }

        // Group by parent id
        let mut map: BTreeMap<usize, Vec<(usize, NetworkJsonTransport)>> = BTreeMap::new();
        for (p_idx, c_idx, t, _total) in candidates.into_iter() {
            map.entry(p_idx).or_default().push((c_idx, t));
        }
        map.into_iter().collect()
    };

    let message = WsResponse::TreeSummaryL2 { data: grouped };
    channels
        .send(PublishedChannels::TreeSummaryL2, message)
        .await;
}

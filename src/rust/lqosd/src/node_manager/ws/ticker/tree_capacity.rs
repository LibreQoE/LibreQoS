use crate::node_manager::ws::messages::{NodeCapacity, WsResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::shaped_devices_tracker::{NETWORK_JSON, node_to_transport};
use std::sync::Arc;

pub async fn tree_capacity(channels: Arc<PubSub>) {
    if !channels
        .is_channel_alive(PublishedChannels::TreeCapacity)
        .await
    {
        return;
    }
    let capacities: Vec<NodeCapacity> = {
        let net_json = NETWORK_JSON.read();
        net_json
            .get_nodes_when_ready()
            .iter()
            .enumerate()
            .map(|(id, node)| {
                let node = node_to_transport(node);
                let down = node.enqueue_throughput.0 as f64 * 8.0 / 1_000_000.0;
                let up = node.enqueue_throughput.1 as f64 * 8.0 / 1_000_000.0;
                let effective_max = node.effective_max_throughput.unwrap_or(node.max_throughput);
                let max_down = effective_max.0;
                let max_up = effective_max.1;
                let median_rtt = if node.rtts.is_empty() {
                    0.0
                } else {
                    let n = node.rtts.len() / 2;
                    if node.rtts.len().is_multiple_of(2) {
                        (node.rtts[n - 1] + node.rtts[n]) / 2.0
                    } else {
                        node.rtts[n]
                    }
                };

                NodeCapacity {
                    id,
                    name: node.name.clone(),
                    down,
                    up,
                    max_down,
                    max_up,
                    median_rtt,
                }
            })
            .collect()
    };

    let message = WsResponse::TreeCapacity { data: capacities };
    channels
        .send(PublishedChannels::TreeCapacity, message)
        .await;
}

use std::sync::Arc;
use serde_json::json;
use tokio::task::spawn_blocking;
use lqos_config::NetworkJsonTransport;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::shaped_devices_tracker::NETWORK_JSON;

pub async fn network_tree(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::NetworkTree).await {
        return;
    }

    let data: Vec<(usize, NetworkJsonTransport)> = spawn_blocking(|| {
        let net_json = NETWORK_JSON.read().unwrap();
        net_json
            .get_nodes_when_ready()
            .iter()
            .enumerate()
            .map(|(i, n) | (i, n.clone_to_transit()))
            .collect()
    }).await.unwrap();

    let message = json!(
        {
            "event": PublishedChannels::NetworkTree.to_string(),
            "data": data,
        }
    ).to_string();
    channels.send(PublishedChannels::NetworkTree, message).await;
}
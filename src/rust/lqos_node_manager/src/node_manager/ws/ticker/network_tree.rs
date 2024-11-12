use std::sync::Arc;
use serde_json::json;
use lqos_bus::{bus_request, BusRequest, BusResponse, Circuit};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn network_tree(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::NetworkTree).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::GetFullNetworkMap]).await else {
        return;
    };

    for reply in replies.into_iter() {
        if let BusResponse::NetworkMap(nodes) = reply {
            let message = json!(
                {
                    "event": PublishedChannels::NetworkTree.to_string(),
                    "data": nodes,
                }
            ).to_string();
            channels.send(PublishedChannels::NetworkTree, message).await;
        }
    }
}

pub async fn all_circuits() -> Vec<Circuit> {
    let Ok(replies) = bus_request(vec![BusRequest::GetAllCircuits]).await else {
        return Vec::new();
    };
    for reply in replies.into_iter() {
        if let BusResponse::CircuitData(circuits) = reply {
            return circuits;
        }
    }
    Vec::new()
}

pub async fn all_subscribers(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::NetworkTreeClients).await {
        return;
    }

    let devices = all_circuits().await;
    let message = json!(
        {
            "event": PublishedChannels::NetworkTreeClients.to_string(),
            "data": devices,
        }
        ).to_string();
    channels.send(PublishedChannels::NetworkTreeClients, message).await;
}
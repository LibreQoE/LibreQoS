use std::sync::Arc;
use serde_json::json;
use tokio::sync::mpsc::Sender;
use lqos_bus::{BusReply, BusRequest, BusResponse, Circuit};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn network_tree(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    if !channels.is_channel_alive(PublishedChannels::NetworkTree).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetFullNetworkMap;
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
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

pub async fn all_circuits(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) -> Vec<Circuit> {
    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetAllCircuits;
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::CircuitData(circuits) = reply {
            return circuits;
        }
    }
    Vec::new()
}

pub async fn all_subscribers(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels.is_channel_alive(PublishedChannels::NetworkTreeClients).await {
        return;
    }

    let devices = all_circuits(bus_tx).await;
    let message = json!(
        {
            "event": PublishedChannels::NetworkTreeClients.to_string(),
            "data": devices,
        }
        ).to_string();
    channels.send(PublishedChannels::NetworkTreeClients, message).await;
}
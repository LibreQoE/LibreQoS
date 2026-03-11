use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse, Circuit};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub async fn network_tree(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::NetworkTree)
        .await
    {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetFullNetworkMap;
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("NetworkTree: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "NetworkTree: failed to receive throughput from bus: {:?}",
                e
            );
            return;
        }
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::NetworkMap(nodes) = reply {
            let message = WsResponse::NetworkTree { data: nodes };
            channels.send(PublishedChannels::NetworkTree, message).await;
        }
    }
}

pub async fn all_circuits(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) -> Vec<Circuit> {
    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetAllCircuits;
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("AllCircuits: failed to send request to bus: {:?}", e);
        return Vec::new();
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "AllCircuits: failed to receive throughput from bus: {:?}",
                e
            );
            return Vec::new();
        }
    };
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
    if !channels
        .is_channel_alive(PublishedChannels::NetworkTreeClients)
        .await
    {
        return;
    }

    let devices = all_circuits(bus_tx).await;
    let message = WsResponse::NetworkTreeClients { data: devices };
    channels
        .send(PublishedChannels::NetworkTreeClients, message)
        .await;
}

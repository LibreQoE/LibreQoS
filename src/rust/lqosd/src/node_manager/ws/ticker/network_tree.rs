use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusRequest, BusResponse, Circuit};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

use super::request_internal_bus;

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

    let Some(replies) =
        request_internal_bus("NetworkTree", bus_tx, BusRequest::GetFullNetworkMap).await
    else {
        return;
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::NetworkMap(nodes) = reply {
            let message = WsResponse::NetworkTree { data: nodes };
            channels.send(PublishedChannels::NetworkTree, message).await;
        }
    }
}

async fn all_circuits_for_context(
    context: &'static str,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) -> Vec<Circuit> {
    let Some(replies) = request_internal_bus(context, bus_tx, BusRequest::GetAllCircuits).await
    else {
        return Vec::new();
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::CircuitData(circuits) = reply {
            return circuits;
        }
    }
    Vec::new()
}

pub async fn all_circuits(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) -> Vec<Circuit> {
    all_circuits_for_context("CircuitWatcherAllCircuits", bus_tx).await
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

    let devices = all_circuits_for_context("NetworkTreeClients", bus_tx).await;
    let message = WsResponse::NetworkTreeClients { data: devices };
    channels
        .send(PublishedChannels::NetworkTreeClients, message)
        .await;
}

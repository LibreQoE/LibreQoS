use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusRequest, BusResponse};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

use super::request_internal_bus;

pub async fn tree_summary(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::TreeSummary)
        .await
    {
        return;
    }

    let request = BusRequest::GetNetworkMap { parent: 0 };
    let Some(replies) = request_internal_bus("TreeSummary", bus_tx, request).await else {
        return;
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::NetworkMap(nodes) = reply {
            let message = WsResponse::TreeSummary { data: nodes };
            channels.send(PublishedChannels::TreeSummary, message).await;
        }
    }
}

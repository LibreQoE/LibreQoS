use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

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

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetNetworkMap { parent: 0 };
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("TreeSummary: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "TreeSummary: failed to receive throughput from bus: {:?}",
                e
            );
            return;
        }
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::NetworkMap(nodes) = reply {
            let message = WsResponse::TreeSummary { data: nodes };
            channels.send(PublishedChannels::TreeSummary, message).await;
        }
    }
}

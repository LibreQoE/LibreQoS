use std::sync::Arc;
use serde_json::json;
use tokio::sync::mpsc::Sender;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn tree_summary(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    if !channels.is_channel_alive(PublishedChannels::TreeSummary).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetNetworkMap { parent: 0 };
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::NetworkMap(nodes) = reply {
            let message = json!(
                {
                    "event": PublishedChannels::TreeSummary.to_string(),
                    "data": nodes,
                }
            ).to_string();
            channels.send(PublishedChannels::TreeSummary, message).await;
        }
    }
}
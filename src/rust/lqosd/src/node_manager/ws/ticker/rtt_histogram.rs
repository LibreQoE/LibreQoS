use std::sync::Arc;
use serde_json::json;
use tokio::sync::mpsc::Sender;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn rtt_histo(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    if !channels.is_channel_alive(PublishedChannels::RttHistogram).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::RttHistogram;
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::RttHistogram(data) = reply {
            let rtt_histo = json!(
                        {
                            "event": PublishedChannels::RttHistogram.to_string(),
                            "data": data,
                        }
                ).to_string();
            channels.send(PublishedChannels::RttHistogram, rtt_histo).await;
        }
    }
}
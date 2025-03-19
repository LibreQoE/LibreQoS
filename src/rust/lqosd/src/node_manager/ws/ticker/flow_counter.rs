use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub async fn flow_count(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::FlowCount)
        .await
    {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::CountActiveFlows;
    bus_tx
        .send((tx, request))
        .await
        .expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::CountActiveFlows(active) = reply {
            let active_flows = json!(
                {
                    "event": PublishedChannels::FlowCount.to_string(),
                    "active": active,
                    "recent": 0,
                }
            )
            .to_string();
            channels
                .send(PublishedChannels::FlowCount, active_flows)
                .await;
        }
    }
}

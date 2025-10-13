use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub async fn stormguard_ticker(
    pubsub: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    if !pubsub
        .is_channel_alive(PublishedChannels::StormguardStatus)
        .await
    {
        return;
    }

    // Request stats from bus
    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetStormguardStats;
    if let Ok(_) = bus_tx.send((tx, request)).await {
        if let Ok(replies) = rx.await {
            for response in replies.responses {
                if let BusResponse::StormguardStats(stats) = response {
                    // Format as JSON
                    let msg = json!({
                        "event": "StormguardStatus",
                        "data": stats,
                    });

                    pubsub
                        .send(PublishedChannels::StormguardStatus, msg.to_string())
                        .await;
                }
            }
        }
    }
}

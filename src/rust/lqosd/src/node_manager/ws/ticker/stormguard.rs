use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub async fn stormguard_ticker(
    pubsub: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    let status_alive = pubsub
        .is_channel_alive(PublishedChannels::StormguardStatus)
        .await;
    let debug_alive = pubsub
        .is_channel_alive(PublishedChannels::StormguardDebug)
        .await;

    if !status_alive && !debug_alive {
        return;
    }

    // Request stats from bus
    if status_alive {
        let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
        let request = BusRequest::GetStormguardStats;
        if let Ok(_) = bus_tx.send((tx, request)).await {
            if let Ok(replies) = rx.await {
                for response in replies.responses {
                    if let BusResponse::StormguardStats(stats) = response {
                        let msg = WsResponse::StormguardStatus { data: stats };
                        pubsub
                            .send(PublishedChannels::StormguardStatus, msg)
                            .await;
                    }
                }
            }
        }
    }

    if debug_alive {
        let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
        let request = BusRequest::GetStormguardDebug;
        if let Ok(_) = bus_tx.send((tx, request)).await {
            if let Ok(replies) = rx.await {
                for response in replies.responses {
                    if let BusResponse::StormguardDebug(stats) = response {
                        let msg = WsResponse::StormguardDebug { data: stats };

                        pubsub
                            .send(PublishedChannels::StormguardDebug, msg)
                            .await;
                    }
                }
            }
        }
    }
}

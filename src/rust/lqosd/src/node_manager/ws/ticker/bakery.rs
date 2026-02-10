use crate::node_manager::ws::messages::{BakeryStatusData, BakeryStatusState, WsResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub async fn bakery_ticker(
    pubsub: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    if !pubsub
        .is_channel_alive(PublishedChannels::BakeryStatus)
        .await
    {
        return;
    }

    // Request stats from bus
    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetBakeryStats;
    if let Ok(_) = bus_tx.send((tx, request)).await {
        if let Ok(replies) = rx.await {
            for response in replies.responses {
                if let BusResponse::BakeryActiveCircuits(stats) = response {
                    // Format as JSON
                    let msg = WsResponse::BakeryStatus {
                        data: BakeryStatusData {
                            current_state: BakeryStatusState {
                                active_circuits: stats,
                            },
                        },
                    };

                    pubsub
                        .send(PublishedChannels::BakeryStatus, msg)
                        .await;
                }
            }
        }
    }
}

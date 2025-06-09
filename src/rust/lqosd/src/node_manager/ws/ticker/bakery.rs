use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub async fn bakery_ticker(
    pubsub: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    if !pubsub.is_channel_alive(PublishedChannels::BakeryStatus).await {
        return;
    }

    // Request stats from bus
    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetBakeryStats;
    if let Ok(_) = bus_tx.send((tx, request)).await {
        if let Ok(replies) = rx.await {
            for response in replies.responses {
                if let BusResponse::BakeryStats(stats) = response {
                    // Format as JSON
                    let msg = json!({
                        "event": "BakeryStatus",
                        "data": {
                            "perCycle": {
                                "queuesCreated": stats.queues_created,
                                "queuesExpired": stats.queues_expired,
                                "lazyQueuesActivated": stats.lazy_queues_activated,
                                "tcCommandsExecuted": stats.tc_commands_executed,
                            },
                            "currentState": {
                                "totalSites": stats.total_sites,
                                "totalCircuits": stats.total_circuits,
                                "activeCircuits": stats.active_circuits,
                                "lazyCircuits": stats.lazy_circuits,
                            },
                            "performance": {
                                "lastBatchDurationMs": stats.last_batch_duration_ms,
                                "pendingCommands": stats.pending_commands,
                            }
                        }
                    });
                    
                    pubsub.send(PublishedChannels::BakeryStatus, msg.to_string()).await;
                }
            }
        }
    }
}
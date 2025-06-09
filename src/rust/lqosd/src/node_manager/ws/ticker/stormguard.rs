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
    if !pubsub.is_channel_alive(PublishedChannels::StormguardStatus).await {
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
                        "data": {
                            "perCycle": {
                                "adjustmentsUp": stats.adjustments_up,
                                "adjustmentsDown": stats.adjustments_down,
                                "sitesEvaluated": stats.sites_evaluated,
                            },
                            "currentState": {
                                "sitesInWarmup": stats.sites_in_warmup,
                                "sitesInCooldown": stats.sites_in_cooldown,
                                "sitesActive": stats.sites_active,
                                "totalSitesManaged": stats.total_sites_managed,
                            },
                            "performance": {
                                "lastCycleDurationMs": stats.last_cycle_duration_ms,
                                "recommendationsGenerated": stats.recommendations_generated,
                            }
                        }
                    });
                    
                    pubsub.send(PublishedChannels::StormguardStatus, msg.to_string()).await;
                }
            }
        }
    }
}
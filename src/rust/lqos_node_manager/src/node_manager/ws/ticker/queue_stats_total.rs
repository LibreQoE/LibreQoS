use std::sync::Arc;
use serde_json::json;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn queue_stats_totals(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::QueueStatsTotal).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::TotalCakeStats]).await else {
        return;
    };
    for reply in replies.into_iter() {
        if let BusResponse::TotalCakeStats { marks, drops } = reply {
            let message = json!(
        {
            "event": PublishedChannels::QueueStatsTotal.to_string(),
            "marks": {
                "down" : marks.get_down(),
                "up" : marks.get_up(),
            },
            "drops" : {
                "down" : drops.get_down(),
                "up" : drops.get_up(),
            },
        }
    ).to_string();
            channels.send(PublishedChannels::QueueStatsTotal, message).await;
        }
    }
}
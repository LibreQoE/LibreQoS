use lqos_queue_tracker::TOTAL_QUEUE_STATS;
use std::sync::Arc;
use serde_json::json;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn queue_stats_totals(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::QueueStatsTotal).await {
        return;
    }

    let message = json!(
        {
            "event": PublishedChannels::QueueStatsTotal.to_string(),
            "marks": {
                "down" : TOTAL_QUEUE_STATS.marks.get_down(),
                "up" : TOTAL_QUEUE_STATS.marks.get_up(),
            },
            "drops" : {
                "down" : TOTAL_QUEUE_STATS.drops.get_down(),
                "up" : TOTAL_QUEUE_STATS.drops.get_up(),
            },
        }
    ).to_string();
    channels.send(PublishedChannels::QueueStatsTotal, message).await;
}
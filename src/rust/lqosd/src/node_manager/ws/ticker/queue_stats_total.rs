use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_queue_tracker::TOTAL_QUEUE_STATS;
use lqos_utils::units::DownUpOrder;
use std::sync::Arc;

pub async fn queue_stats_totals(channels: Arc<PubSub>) {
    if !channels
        .is_channel_alive(PublishedChannels::QueueStatsTotal)
        .await
    {
        return;
    }

    let message = WsResponse::QueueStatsTotal {
        marks: DownUpOrder::new(
            TOTAL_QUEUE_STATS.marks.get_down(),
            TOTAL_QUEUE_STATS.marks.get_up(),
        ),
        drops: DownUpOrder::new(
            TOTAL_QUEUE_STATS.drops.get_down(),
            TOTAL_QUEUE_STATS.drops.get_up(),
        ),
    };
    channels
        .send(PublishedChannels::QueueStatsTotal, message)
        .await;
}

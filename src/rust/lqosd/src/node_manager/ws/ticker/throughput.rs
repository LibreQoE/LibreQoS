use std::sync::Arc;
use serde_json::json;
use lqos_config::load_config;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::throughput_tracker::THROUGHPUT_TRACKER;

pub async fn throughput(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::Throughput).await {
        return;
    }

    let (bits_per_second, packets_per_second, shaped_bits_per_second) = {
        (
            THROUGHPUT_TRACKER.bits_per_second(),
            THROUGHPUT_TRACKER.packets_per_second(),
            THROUGHPUT_TRACKER.shaped_bits_per_second(),
        )
    };
    let max = if let Ok(config) = load_config() {
        (
            config.queues.uplink_bandwidth_mbps,
            config.queues.downlink_bandwidth_mbps,
        )
    } else {
        (0,0)
    };
    let bps = json!(
        {
            "event" : "throughput",
            "data": {
                "bps": bits_per_second,
                "pps": packets_per_second,
                "shaped_bps": shaped_bits_per_second,
                "max": max,
            }
        }
        ).to_string();
    channels.send(PublishedChannels::Throughput, bps).await;
}
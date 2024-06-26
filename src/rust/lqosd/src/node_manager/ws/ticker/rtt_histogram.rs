use std::sync::Arc;
use serde_json::json;
use lqos_bus::BusResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::throughput_tracker::rtt_histogram;

pub async fn rtt_histo(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::RttHistogram).await {
        return;
    }

    let histo = rtt_histogram();
    if let BusResponse::RttHistogram(data) = &histo {
        let rtt_histo = json!(
                    {
                        "event": "rttHistogram",
                        "data": data,
                    }
            ).to_string();
        channels.send(PublishedChannels::RttHistogram, rtt_histo).await;
    }
}
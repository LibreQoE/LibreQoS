use std::sync::Arc;
use serde_json::json;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use lqos_config::load_config;
use lqos_utils::units::DownUpOrder;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn throughput(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::Throughput).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::GetCurrentThroughput]).await else {
        return;
    };
    for reply in replies.into_iter() {
        if let BusResponse::CurrentThroughput { bits_per_second, packets_per_second, shaped_bits_per_second } = reply {
            let max = if let Ok(config) = load_config() {
                DownUpOrder::new(
                    config.queues.uplink_bandwidth_mbps,
                    config.queues.downlink_bandwidth_mbps,
                )
            } else {
                DownUpOrder::zeroed()
            };

            let bps = json!(
            {
                "event" : PublishedChannels::Throughput.to_string(),
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
    }
}
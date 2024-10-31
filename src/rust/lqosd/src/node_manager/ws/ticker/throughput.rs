use std::sync::Arc;
use serde_json::json;
use tokio::sync::mpsc::Sender;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use lqos_config::load_config;
use lqos_utils::units::DownUpOrder;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn throughput(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    if !channels.is_channel_alive(PublishedChannels::Throughput).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetCurrentThroughput;
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
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
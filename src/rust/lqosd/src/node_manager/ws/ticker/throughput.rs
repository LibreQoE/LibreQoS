use crate::node_manager::ws::messages::{ThroughputData, WsResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use lqos_config::load_config;
use lqos_utils::units::DownUpOrder;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub async fn throughput(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::Throughput)
        .await
    {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetCurrentThroughput;
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("Throughput: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Throughput: failed to receive throughput from bus: {:?}", e);
            return;
        }
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::CurrentThroughput {
            bits_per_second,
            packets_per_second,
            tcp_packets_per_second,
            udp_packets_per_second,
            icmp_packets_per_second,
            shaped_bits_per_second,
        } = reply
        {
            let max = if let Ok(config) = load_config() {
                DownUpOrder::new(
                    config.queues.uplink_bandwidth_mbps,
                    config.queues.downlink_bandwidth_mbps,
                )
            } else {
                DownUpOrder::zeroed()
            };

            let bps = WsResponse::Throughput {
                data: ThroughputData {
                    bps: bits_per_second,
                    pps: packets_per_second,
                    tcp_pps: tcp_packets_per_second,
                    udp_pps: udp_packets_per_second,
                    icmp_pps: icmp_packets_per_second,
                    shaped_bps: shaped_bits_per_second,
                    max,
                },
            };
            channels.send(PublishedChannels::Throughput, bps).await;
        }
    }
}

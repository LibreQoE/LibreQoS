use std::sync::Arc;

use serde_json::json;
use tokio::sync::mpsc::Sender;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn endpoints_by_country(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    if !channels.is_channel_alive(PublishedChannels::EndpointsByCountry).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::CurrentEndpointsByCountry;
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::CurrentEndpointsByCountry(countries) = reply {
            let message = json!(
            {
                "event": PublishedChannels::EndpointsByCountry.to_string(),
                "data": countries,
            }
        ).to_string();
            channels.send(PublishedChannels::EndpointsByCountry, message).await;
        }
    }
}

pub async fn ether_protocols(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    if !channels.is_channel_alive(PublishedChannels::EtherProtocols).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::EtherProtocolSummary;
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::EtherProtocols { v4_bytes, v6_bytes, v4_packets, v6_packets, v4_rtt, v6_rtt } = reply {
            let message = json!(
            {
                "event": PublishedChannels::EtherProtocols.to_string(),
                "data": {
                    "v4_bytes": v4_bytes,
                    "v6_bytes": v6_bytes,
                    "v4_packets": v4_packets,
                    "v6_packets": v6_packets,
                    "v4_rtt": v4_rtt,
                    "v6_rtt": v6_rtt,
                },
            }
        ).to_string();
            channels.send(PublishedChannels::EtherProtocols, message).await;
        }
    }
}

pub async fn ip_protocols(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    if !channels.is_channel_alive(PublishedChannels::IpProtocols).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::IpProtocolSummary;
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::IpProtocols(ip_data) = reply {
            let message = json!(
            {
                "event": PublishedChannels::IpProtocols.to_string(),
                "data": ip_data,
            }
        ).to_string();
            channels.send(PublishedChannels::IpProtocols, message).await;
        }
    }
}

pub async fn flow_duration(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    if !channels.is_channel_alive(PublishedChannels::FlowDurations).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::IpProtocolSummary;
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::FlowDuration(flow_data) = reply {
            let message = json!(
            {
                "event": PublishedChannels::FlowDurations.to_string(),
                "data": flow_data,
            }
            ).to_string();
            channels.send(PublishedChannels::FlowDurations, message).await;
        }
    }

}
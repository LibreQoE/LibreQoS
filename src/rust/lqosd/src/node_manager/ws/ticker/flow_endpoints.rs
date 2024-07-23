use std::sync::Arc;

use serde_json::json;

use lqos_bus::BusResponse;

use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::throughput_tracker;

pub async fn endpoints_by_country(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::EndpointsByCountry).await {
        return;
    }

    if let BusResponse::CurrentEndpointsByCountry(countries) = throughput_tracker::current_endpoints_by_country() {
        let message = json!(
        {
            "event": PublishedChannels::EndpointsByCountry.to_string(),
            "data": countries,
        }
    ).to_string();
        channels.send(PublishedChannels::EndpointsByCountry, message).await;
    }
}

pub async fn ether_protocols(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::EtherProtocols).await {
        return;
    }

    if let BusResponse::EtherProtocols { v4_bytes, v6_bytes, v4_packets, v6_packets, v4_rtt, v6_rtt } = throughput_tracker::ether_protocol_summary() {
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

pub async fn ip_protocols(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::IpProtocols).await {
        return;
    }

    if let BusResponse::IpProtocols(ip_data) = throughput_tracker::ip_protocol_summary() {
        let message = json!(
        {
            "event": PublishedChannels::IpProtocols.to_string(),
            "data": ip_data,
        }
    ).to_string();
        channels.send(PublishedChannels::IpProtocols, message).await;
    }
}
use std::sync::Arc;

use crate::node_manager::ws::messages::{EtherProtocolsData, WsResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use tokio::sync::mpsc::Sender;

pub async fn endpoints_by_country(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::EndpointsByCountry)
        .await
    {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::CurrentEndpointsByCountry;
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("EndpointsByCountry: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "EndpointsByCountry: failed to receive throughput from bus: {:?}",
                e
            );
            return;
        }
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::CurrentEndpointsByCountry(countries) = reply {
            let message = WsResponse::EndpointsByCountry { data: countries };
            channels
                .send(PublishedChannels::EndpointsByCountry, message)
                .await;
        }
    }
}

pub async fn ether_protocols(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::EtherProtocols)
        .await
    {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::EtherProtocolSummary;
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("EtherProtocols: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "EtherProtocols: failed to receive throughput from bus: {:?}",
                e
            );
            return;
        }
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::EtherProtocols {
            v4_bytes,
            v6_bytes,
            v4_packets,
            v6_packets,
            v4_rtt,
            v6_rtt,
        } = reply
        {
            let message = WsResponse::EtherProtocols {
                data: EtherProtocolsData {
                    v4_bytes,
                    v6_bytes,
                    v4_packets,
                    v6_packets,
                    v4_rtt,
                    v6_rtt,
                },
            };
            channels
                .send(PublishedChannels::EtherProtocols, message)
                .await;
        }
    }
}

pub async fn ip_protocols(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::IpProtocols)
        .await
    {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::IpProtocolSummary;
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("IpProtocols: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "IpProtocols: failed to receive throughput from bus: {:?}",
                e
            );
            return;
        }
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::IpProtocols(ip_data) = reply {
            let message = WsResponse::IpProtocols { data: ip_data };
            channels.send(PublishedChannels::IpProtocols, message).await;
        }
    }
}

pub async fn flow_duration(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::FlowDurations)
        .await
    {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::FlowDuration;
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("FlowDurations: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "FlowDurations: failed to receive throughput from bus: {:?}",
                e
            );
            return;
        }
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::FlowDuration(flow_data) = reply {
            let message = WsResponse::FlowDurations { data: flow_data };
            channels
                .send(PublishedChannels::FlowDurations, message)
                .await;
        }
    }
}

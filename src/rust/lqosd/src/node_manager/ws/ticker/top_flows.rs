use std::sync::Arc;

use lqos_bus::{BusReply, BusRequest, BusResponse, TopFlowType};
use tokio::sync::mpsc::Sender;

use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn top_flows_bytes(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::TopFlowsBytes)
        .await
    {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::TopFlows {
        flow_type: TopFlowType::Bytes,
        n: 10,
    };
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("TopFlowsBytes: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "TopFlowsBytes: failed to receive throughput from bus: {:?}",
                e
            );
            return;
        }
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::TopFlows(flows) = reply {
            let message = WsResponse::TopFlowsBytes { data: flows };
            channels
                .send(PublishedChannels::TopFlowsBytes, message)
                .await;
        }
    }
}

pub async fn top_flows_rate(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::TopFlowsRate)
        .await
    {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::TopFlows {
        flow_type: TopFlowType::RateEstimate,
        n: 10,
    };
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("TopFlowsRate: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "TopFlowsRate: failed to receive throughput from bus: {:?}",
                e
            );
            return;
        }
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::TopFlows(flows) = reply {
            let message = WsResponse::TopFlowsRate { data: flows };
            channels
                .send(PublishedChannels::TopFlowsRate, message)
                .await;
        }
    }
}

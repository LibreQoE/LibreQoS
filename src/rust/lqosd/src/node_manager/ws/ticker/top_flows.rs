use std::sync::Arc;

use serde_json::json;
use tokio::sync::mpsc::Sender;
use lqos_bus::{BusReply, BusRequest, BusResponse, TopFlowType};

use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn top_flows_bytes(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>
) {
    if !channels.is_channel_alive(PublishedChannels::TopFlowsBytes).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::TopFlows { flow_type: TopFlowType::Bytes, n: 10 };
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::TopFlows(flows) = reply {
            let message = json!(
                {
                    "event": PublishedChannels::TopFlowsBytes.to_string(),
                    "data": flows,
                }
            ).to_string();
            channels.send(PublishedChannels::TopFlowsBytes, message).await;
        }
    }
}

pub async fn top_flows_rate(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>
) {
    if !channels.is_channel_alive(PublishedChannels::TopFlowsRate).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::TopFlows { flow_type: TopFlowType::RateEstimate, n: 10 };
    bus_tx.send((tx, request)).await.expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::TopFlows(flows) = reply {
            let message = json!(
                {
                    "event": PublishedChannels::TopFlowsRate.to_string(),
                    "data": flows,
                }
            ).to_string();
            channels.send(PublishedChannels::TopFlowsRate, message).await;
        }
    }
}
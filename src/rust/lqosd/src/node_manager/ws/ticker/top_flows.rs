use std::sync::Arc;

use lqos_bus::{BusReply, BusRequest, BusResponse, TopFlowType};
use tokio::sync::mpsc::Sender;

use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

use super::request_internal_bus;

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

    let request = BusRequest::TopFlows {
        flow_type: TopFlowType::Bytes,
        n: 10,
    };
    let Some(replies) = request_internal_bus("TopFlowsBytes", bus_tx, request).await else {
        return;
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

    let request = BusRequest::TopFlows {
        flow_type: TopFlowType::RateEstimate,
        n: 10,
    };
    let Some(replies) = request_internal_bus("TopFlowsRate", bus_tx, request).await else {
        return;
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

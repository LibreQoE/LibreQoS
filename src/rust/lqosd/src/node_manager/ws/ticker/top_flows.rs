use std::sync::Arc;

use serde_json::json;

use lqos_bus::{BusResponse, TopFlowType};

use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::throughput_tracker;

pub async fn top_flows_bytes(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::TopFlowsBytes).await {
        return;
    }

    if let BusResponse::TopFlows(flows) = throughput_tracker::top_flows(10, TopFlowType::Bytes) {
        let message = json!(
            {
                "event": PublishedChannels::TopFlowsBytes.to_string(),
                "data": flows,
            }
        ).to_string();
        channels.send(PublishedChannels::TopFlowsBytes, message).await;
    }
}

pub async fn top_flows_rate(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::TopFlowsRate).await {
        return;
    }

    if let BusResponse::TopFlows(flows) = throughput_tracker::top_flows(10, TopFlowType::RateEstimate) {
        let message = json!(
            {
                "event": PublishedChannels::TopFlowsRate.to_string(),
                "data": flows,
            }
        ).to_string();
        channels.send(PublishedChannels::TopFlowsRate, message).await;
    }
}
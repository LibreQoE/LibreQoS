use std::sync::Arc;

use serde_json::json;
use lqos_bus::{bus_request, BusRequest, BusResponse, TopFlowType};

use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn top_flows_bytes(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::TopFlowsBytes).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::TopFlows { flow_type: TopFlowType::Bytes, n: 10 }]).await else {
        return;
    };
    for reply in replies.into_iter() {
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
) {
    if !channels.is_channel_alive(PublishedChannels::TopFlowsRate).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::TopFlows { flow_type: TopFlowType::RateEstimate, n: 10 }]).await else {
        return;
    };
    for reply in replies.into_iter() {
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
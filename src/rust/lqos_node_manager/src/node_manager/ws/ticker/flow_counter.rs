use std::sync::Arc;
use serde_json::json;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn flow_count(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::FlowCount).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::CountActiveFlows]).await else {
        return;
    };
    for reply in replies.into_iter() {
        if let BusResponse::CountActiveFlows(active) = reply {
            let active_flows = json!(
            {
                "event": PublishedChannels::FlowCount.to_string(),
                "active": active,
                "recent": 0,
            }
        ).to_string();
            channels.send(PublishedChannels::FlowCount, active_flows).await;
        }
    }
}
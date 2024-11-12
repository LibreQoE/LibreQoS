use std::sync::Arc;
use serde_json::json;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn tree_summary(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::TreeSummary).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::GetNetworkMap { parent: 0 }]).await else {
        return;
    };
    for reply in replies.into_iter() {
        if let BusResponse::NetworkMap(nodes) = reply {
            let message = json!(
                {
                    "event": PublishedChannels::TreeSummary.to_string(),
                    "data": nodes,
                }
            ).to_string();
            channels.send(PublishedChannels::TreeSummary, message).await;
        }
    }
}

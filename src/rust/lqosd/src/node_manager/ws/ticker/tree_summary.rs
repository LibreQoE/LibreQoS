use std::sync::Arc;
use serde_json::json;
use lqos_bus::BusResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::shaped_devices_tracker;

pub async fn tree_summary(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::TreeSummary).await {
        return;
    }

    if let BusResponse::NetworkMap(nodes) = shaped_devices_tracker::get_top_n_root_queues(7) {

        let message = json!(
            {
                "event": PublishedChannels::TreeSummary.to_string(),
                "data": nodes,
            }
        ).to_string();
        channels.send(PublishedChannels::TreeSummary, message).await;
    }
}
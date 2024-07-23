use std::sync::Arc;
use serde_json::json;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn cadence(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::Cadence).await {
        return;
    }

    let message = json!(
        {
            "event": PublishedChannels::Cadence.to_string(),
        }
    ).to_string();
    channels.send(PublishedChannels::Cadence, message).await;
}
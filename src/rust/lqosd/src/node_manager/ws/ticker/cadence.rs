use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use std::sync::Arc;

pub async fn cadence(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::Cadence).await {
        return;
    }

    let message = WsResponse::Cadence;
    channels.send(PublishedChannels::Cadence, message).await;
}

use crate::node_manager::local_api::network_tree_lite::network_tree_lite_data;
use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use std::sync::Arc;

pub async fn network_tree_lite(channels: Arc<PubSub>) {
    if !channels
        .is_channel_alive(PublishedChannels::NetworkTreeLite)
        .await
    {
        return;
    }

    let message = WsResponse::NetworkTreeLite {
        data: network_tree_lite_data(),
    };
    channels.send(PublishedChannels::NetworkTreeLite, message).await;
}

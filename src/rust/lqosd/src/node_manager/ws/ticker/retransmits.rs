use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::throughput_tracker::min_max_median_tcp_retransmits;
use std::sync::Arc;

pub async fn tcp_retransmits(channels: Arc<PubSub>) {
    if !channels
        .is_channel_alive(PublishedChannels::Retransmits)
        .await
    {
        return;
    }

    let tcp_retransmits = min_max_median_tcp_retransmits();

    let message = WsResponse::Retransmits {
        data: tcp_retransmits,
    };
    channels.send(PublishedChannels::Retransmits, message).await;
}

use std::sync::Arc;

use crate::node_manager::local_api::flow_map;
use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn endpoint_latlon(channels: Arc<PubSub>) {
    if !channels
        .is_channel_alive(PublishedChannels::EndpointLatLon)
        .await
    {
        return;
    }

    let message = WsResponse::EndpointLatLon {
        data: flow_map::endpoint_latlon_data(),
    };
    channels
        .send(PublishedChannels::EndpointLatLon, message)
        .await;
}

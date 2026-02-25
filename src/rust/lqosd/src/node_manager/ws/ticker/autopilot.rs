use crate::autopilot::status::{autopilot_activity_snapshot, autopilot_status_snapshot};
use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use std::sync::Arc;

pub async fn autopilot_status(pubsub: Arc<PubSub>) {
    if !pubsub
        .is_channel_alive(PublishedChannels::AutopilotStatus)
        .await
    {
        return;
    }

    let data = autopilot_status_snapshot().await;
    let msg = WsResponse::AutopilotStatus { data };
    pubsub.send(PublishedChannels::AutopilotStatus, msg).await;
}

pub async fn autopilot_activity(pubsub: Arc<PubSub>) {
    if !pubsub
        .is_channel_alive(PublishedChannels::AutopilotActivity)
        .await
    {
        return;
    }

    let data = autopilot_activity_snapshot().await;
    let msg = WsResponse::AutopilotActivity { data };
    pubsub
        .send(PublishedChannels::AutopilotActivity, msg)
        .await;
}


use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::treeguard::status::{treeguard_activity_snapshot, treeguard_status_snapshot};
use std::sync::Arc;

pub async fn treeguard_status(pubsub: Arc<PubSub>) {
    if !pubsub
        .is_channel_alive(PublishedChannels::TreeGuardStatus)
        .await
    {
        return;
    }

    let data = treeguard_status_snapshot().await;
    let msg = WsResponse::TreeGuardStatus { data };
    pubsub.send(PublishedChannels::TreeGuardStatus, msg).await;
}

pub async fn treeguard_activity(pubsub: Arc<PubSub>) {
    if !pubsub
        .is_channel_alive(PublishedChannels::TreeGuardActivity)
        .await
    {
        return;
    }

    let data = treeguard_activity_snapshot().await;
    let msg = WsResponse::TreeGuardActivity { data };
    pubsub.send(PublishedChannels::TreeGuardActivity, msg).await;
}

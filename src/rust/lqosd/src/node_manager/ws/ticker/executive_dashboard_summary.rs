use std::sync::Arc;

use crate::node_manager::local_api::executive;
use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn executive_dashboard_summary(channels: Arc<PubSub>) {
    if !channels
        .is_channel_alive(PublishedChannels::ExecutiveDashboardSummary)
        .await
    {
        return;
    }

    let payload = WsResponse::ExecutiveDashboardSummary {
        data: executive::executive_dashboard_summary(),
    };
    channels
        .send(PublishedChannels::ExecutiveDashboardSummary, payload)
        .await;
}

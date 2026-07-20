use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusRequest, BusResponse};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

use super::request_internal_bus;

pub async fn flow_count(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::FlowCount)
        .await
    {
        return;
    }

    let request = BusRequest::CountActiveFlows;
    let Some(replies) = request_internal_bus("FlowCount", bus_tx, request).await else {
        return;
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::CountActiveFlows(active) = reply {
            let active_flows = WsResponse::FlowCount { active, recent: 0 };
            channels
                .send(PublishedChannels::FlowCount, active_flows)
                .await;
        }
    }
}

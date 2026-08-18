use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusRequest, BusResponse};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

use super::request_internal_bus;

pub async fn rtt_histo(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::RttHistogram)
        .await
    {
        return;
    }

    let request = BusRequest::RttHistogram;
    let Some(replies) = request_internal_bus("RttHistogram", bus_tx, request).await else {
        return;
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::RttHistogram(data) = reply {
            let rtt_histo = WsResponse::RttHistogram { data };
            channels
                .send(PublishedChannels::RttHistogram, rtt_histo)
                .await;
        }
    }
}

use std::sync::Arc;
use serde_json::json;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn rtt_histo(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::RttHistogram).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::RttHistogram]).await else {
        return;
    };
    for reply in replies.into_iter() {
        if let BusResponse::RttHistogram(data) = reply {
            let rtt_histo = json!(
                        {
                            "event": PublishedChannels::RttHistogram.to_string(),
                            "data": data,
                        }
                ).to_string();
            channels.send(PublishedChannels::RttHistogram, rtt_histo).await;
        }
    }
}
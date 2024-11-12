use std::sync::Arc;
use serde_json::json;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub async fn circuit_capacity(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::CircuitCapacity).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::CircuitCapacities]).await else {
        return;
    };
    for reply in replies {
        if let BusResponse::CircuitCapacities(capacities) = reply {
            let message = json!(
                {
                    "event": PublishedChannels::CircuitCapacity.to_string(),
                    "data": capacities,
                }
            ).to_string();
            channels.send(PublishedChannels::CircuitCapacity, message).await;
        }
    }
}
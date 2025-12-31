use std::sync::Arc;

use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use tokio::sync::mpsc::Sender;

pub async fn endpoint_latlon(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::EndpointLatLon)
        .await
    {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::CurrentEndpointLatLon;
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("EndpointLatLon: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "EndpointLatLon: failed to receive response from bus: {:?}",
                e
            );
            return;
        }
    };

    for reply in replies.responses.into_iter() {
        if let BusResponse::CurrentLatLon(points) = reply {
            let message = WsResponse::EndpointLatLon { data: points };
            channels
                .send(PublishedChannels::EndpointLatLon, message)
                .await;
        }
    }
}

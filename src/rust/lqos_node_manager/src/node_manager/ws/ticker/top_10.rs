use std::sync::Arc;
use serde_json::json;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::ipstats_conversion::IpStatsWithPlan;

pub async fn top_10_downloaders(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::TopDownloads).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::GetTopNDownloaders {  start: 0, end: 10 }]).await else {
        return;
    };
    for reply in replies.into_iter() {
        if let BusResponse::TopDownloaders(top) = reply {
            let result: Vec<IpStatsWithPlan> = top
                .iter()
                .map(|stat| stat.into())
                .collect();

            let message = json!(
            {
                "event": PublishedChannels::TopDownloads.to_string(),
                "data": result
            }
        ).to_string();
            channels.send(PublishedChannels::TopDownloads, message).await;
        }
    }
}

pub async fn worst_10_downloaders(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::WorstRTT).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::GetWorstRtt {  start: 0, end: 10 }]).await else {
        return;
    };
    for reply in replies.into_iter() {
        if let BusResponse::WorstRtt(top) = reply {
            let result: Vec<IpStatsWithPlan> = top
                .iter()
                .map(|stat| stat.into())
                .collect();

            let message = json!(
            {
                "event": PublishedChannels::WorstRTT.to_string(),
                "data": result
            }
        ).to_string();
            channels.send(PublishedChannels::WorstRTT, message).await;
        }
    }
}

pub async fn worst_10_retransmit(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::WorstRetransmits).await {
        return;
    }

    let Ok(replies) = bus_request(vec![BusRequest::GetWorstRetransmits {  start: 0, end: 10 }]).await else {
        return;
    };
    for reply in replies.into_iter() {
        if let BusResponse::WorstRetransmits(top) = reply {
            let result: Vec<IpStatsWithPlan> = top
                .iter()
                .map(|stat| stat.into())
                .collect();

            let message = json!(
            {
                "event": PublishedChannels::WorstRetransmits.to_string(),
                "data": result
            }
        ).to_string();
            channels.send(PublishedChannels::WorstRetransmits, message).await;
        }
    }
}
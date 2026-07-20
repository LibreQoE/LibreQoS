use crate::node_manager::ws::messages::WsResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::ipstats_conversion::IpStatsWithPlan;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

use super::request_internal_bus;

pub async fn top_10_downloaders(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::TopDownloads)
        .await
    {
        return;
    }

    let request = BusRequest::GetTopNDownloaders { start: 0, end: 10 };
    let Some(replies) = request_internal_bus("TopDownloads", bus_tx, request).await else {
        return;
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::TopDownloaders(top) = reply {
            let result: Vec<IpStatsWithPlan> = top.iter().map(|stat| stat.into()).collect();

            let message = WsResponse::TopDownloads { data: result };
            channels
                .send(PublishedChannels::TopDownloads, message)
                .await;
        }
    }
}

pub async fn top_10_uploaders(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::TopUploads)
        .await
    {
        return;
    }

    let request = BusRequest::GetTopNUploaders { start: 0, end: 10 };
    let Some(replies) = request_internal_bus("TopUploads", bus_tx, request).await else {
        return;
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::TopUploaders(top) = reply {
            let result: Vec<IpStatsWithPlan> = top.iter().map(|stat| stat.into()).collect();

            let message = WsResponse::TopUploads { data: result };
            channels.send(PublishedChannels::TopUploads, message).await;
        }
    }
}

pub async fn worst_10_downloaders(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    if !channels.is_channel_alive(PublishedChannels::WorstRTT).await {
        return;
    }

    let request = BusRequest::GetWorstRtt { start: 0, end: 10 };
    let Some(replies) = request_internal_bus("WorstRTT", bus_tx, request).await else {
        return;
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::WorstRtt(top) = reply {
            let result: Vec<IpStatsWithPlan> = top.iter().map(|stat| stat.into()).collect();

            let message = WsResponse::WorstRTT { data: result };
            channels.send(PublishedChannels::WorstRTT, message).await;
        }
    }
}

pub async fn worst_10_retransmit(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::WorstRetransmits)
        .await
    {
        return;
    }

    let request = BusRequest::GetWorstRetransmits { start: 0, end: 10 };
    let Some(replies) = request_internal_bus("WorstRetransmits", bus_tx, request).await else {
        return;
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::WorstRetransmits(top) = reply {
            let result: Vec<IpStatsWithPlan> = top.iter().map(|stat| stat.into()).collect();

            let message = WsResponse::WorstRetransmits { data: result };
            channels
                .send(PublishedChannels::WorstRetransmits, message)
                .await;
        }
    }
}

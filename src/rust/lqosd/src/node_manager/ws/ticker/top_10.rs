use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::ipstats_conversion::IpStatsWithPlan;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

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

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetTopNDownloaders { start: 0, end: 10 };
    bus_tx
        .send((tx, request))
        .await
        .expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::TopDownloaders(top) = reply {
            let result: Vec<IpStatsWithPlan> = top.iter().map(|stat| stat.into()).collect();

            let message = json!(
                {
                    "event": PublishedChannels::TopDownloads.to_string(),
                    "data": result
                }
            )
            .to_string();
            channels
                .send(PublishedChannels::TopDownloads, message)
                .await;
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

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetWorstRtt { start: 0, end: 10 };
    bus_tx
        .send((tx, request))
        .await
        .expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::WorstRtt(top) = reply {
            let result: Vec<IpStatsWithPlan> = top.iter().map(|stat| stat.into()).collect();

            let message = json!(
                {
                    "event": PublishedChannels::WorstRTT.to_string(),
                    "data": result
                }
            )
            .to_string();
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

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetWorstRetransmits { start: 0, end: 10 };
    bus_tx
        .send((tx, request))
        .await
        .expect("Failed to send request to bus");
    let replies = rx.await.expect("Failed to receive throughput from bus");
    for reply in replies.responses.into_iter() {
        if let BusResponse::WorstRetransmits(top) = reply {
            let result: Vec<IpStatsWithPlan> = top.iter().map(|stat| stat.into()).collect();

            let message = json!(
                {
                    "event": PublishedChannels::WorstRetransmits.to_string(),
                    "data": result
                }
            )
            .to_string();
            channels
                .send(PublishedChannels::WorstRetransmits, message)
                .await;
        }
    }
}

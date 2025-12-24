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
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("TopDownloads: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "TopDownloads: failed to receive throughput from bus: {:?}",
                e
            );
            return;
        }
    };
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

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetTopNUploaders { start: 0, end: 10 };
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("TopUploads: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("TopUploads: failed to receive throughput from bus: {:?}", e);
            return;
        }
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::TopUploaders(top) = reply {
            let result: Vec<IpStatsWithPlan> = top.iter().map(|stat| stat.into()).collect();

            let message = json!(
                {
                    "event": PublishedChannels::TopUploads.to_string(),
                    "data": result
                }
            )
            .to_string();
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

    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    let request = BusRequest::GetWorstRtt { start: 0, end: 10 };
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("WorstRTT: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("WorstRTT: failed to receive throughput from bus: {:?}", e);
            return;
        }
    };
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
    if let Err(e) = bus_tx.send((tx, request)).await {
        tracing::warn!("WorstRetransmits: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "WorstRetransmits: failed to receive throughput from bus: {:?}",
                e
            );
            return;
        }
    };
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

use std::sync::Arc;
use serde_json::json;
use lqos_bus::BusResponse;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::ipstats_conversion::IpStatsWithPlan;
use crate::throughput_tracker::{top_n, worst_n};

pub async fn top_10_downloaders(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::Top10Downloaders).await {
        return;
    }

    if let BusResponse::TopDownloaders(top) = top_n(0, 10) {
        let result: Vec<IpStatsWithPlan> = top
            .iter()
            .map(|stat| stat.into())
            .collect();

        let message = json!(
        {
            "event": "top10downloaders",
            "data": result
        }
    ).to_string();
        channels.send(PublishedChannels::Top10Downloaders, message).await;
    }
}

pub async fn worst_10_downloaders(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::Worst10Downloaders).await {
        return;
    }

    if let BusResponse::WorstRtt(top) = worst_n(0, 10) {
        let result: Vec<IpStatsWithPlan> = top
            .iter()
            .map(|stat| stat.into())
            .collect();

        let message = json!(
        {
            "event": "worst10downloaders",
            "data": result
        }
    ).to_string();
        channels.send(PublishedChannels::Worst10Downloaders, message).await;
    }
}
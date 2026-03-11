use crate::node_manager::ws::messages::{RamData, WsResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::system_stats::SystemStats;
use std::sync::Arc;

pub async fn cpu_info(
    channels: Arc<PubSub>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
    if !channels.is_channel_alive(PublishedChannels::Cpu).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Ok(_) = system_usage_tx.send(tx) {
        if let Ok(usage) = rx.await {
            let message = WsResponse::Cpu {
                data: usage.cpu_usage,
            };
            channels.send(PublishedChannels::Cpu, message).await;
        }
    }
}

pub async fn ram_info(
    channels: Arc<PubSub>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
    if !channels.is_channel_alive(PublishedChannels::Ram).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Ok(_) = system_usage_tx.send(tx) {
        if let Ok(usage) = rx.await {
            let message = WsResponse::Ram {
                data: RamData {
                    total: usage.total_ram,
                    used: usage.ram_used,
                },
            };
            channels.send(PublishedChannels::Ram, message).await;
        }
    }
}

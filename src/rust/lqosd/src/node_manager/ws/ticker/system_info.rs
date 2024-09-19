use std::sync::Arc;
use serde_json::json;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::system_stats::SystemStats;

pub async fn cpu_info(
    channels: Arc<PubSub>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>
) {
    if !channels.is_channel_alive(PublishedChannels::Cpu).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Ok(_) = system_usage_tx.send(tx)  {
        if let Ok(usage) = rx.await {
            let message = json!(
                {
                    "event": PublishedChannels::Cpu.to_string(),
                    "data": usage.cpu_usage,
                }
            ).to_string();
            channels.send(PublishedChannels::Cpu, message).await;
        }
    }
}

pub async fn ram_info(
    channels: Arc<PubSub>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>
) {
    if !channels.is_channel_alive(PublishedChannels::Ram).await {
        return;
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Ok(_) = system_usage_tx.send(tx)  {
        if let Ok(usage) = rx.await {
            let message = json!(
                {
                    "event": PublishedChannels::Ram.to_string(),
                    "data": {
                        "total" : usage.total_ram,
                        "used" : usage.ram_used,
                    },
                }
            ).to_string();
            channels.send(PublishedChannels::Ram, message).await;
        }
    }
}

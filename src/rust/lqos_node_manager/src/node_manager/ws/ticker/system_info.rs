use std::sync::Arc;
use serde_json::json;
use lqos_bus::{bus_request, BusClient, BusRequest, BusResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use anyhow::Result;

async fn get_cpu_ram() -> Result<(Vec<u32>, u64, u64)> {
    let Ok(replies) = bus_request(vec![BusRequest::SystemStatsCpuRam]).await else {
        return Err(anyhow::anyhow!("Failed to get CPU and RAM stats"));
    };
    for response in replies {
        match response {
            BusResponse::SystemStatsCpuRam { cpu_usage, ram_used, total_ram } => {
                return Ok((cpu_usage, ram_used, total_ram));
            }
            _ => continue,
        }
    }
    Err(anyhow::anyhow!("Failed to get CPU and RAM stats"))
}

pub async fn cpu_info(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::Cpu).await {
        return;
    }
    let Ok((cpu_usage, _, _)) = get_cpu_ram().await else {
        return;
    };

    let message = json!(
        {
            "event": PublishedChannels::Cpu.to_string(),
            "data": cpu_usage,
        }
    ).to_string();
    channels.send(PublishedChannels::Cpu, message).await;
}

pub async fn ram_info(
    channels: Arc<PubSub>,
) {
    if !channels.is_channel_alive(PublishedChannels::Ram).await {
        return;
    }

    let Ok((_, ram_used, total_ram)) = get_cpu_ram().await else {
        return;
    };

    let message = json!(
        {
            "event": PublishedChannels::Ram.to_string(),
            "data": {
                "total" : total_ram,
                "used" : ram_used,
            },
        }
    ).to_string();
    channels.send(PublishedChannels::Ram, message).await;
}

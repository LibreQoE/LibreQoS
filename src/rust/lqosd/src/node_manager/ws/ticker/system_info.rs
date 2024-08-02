use std::sync::Arc;
use serde_json::json;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::system_info::cache::{CPU_USAGE, NUM_CPUS, RAM_USED, TOTAL_RAM};

pub mod cache;

pub async fn cpu_info(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::Cpu).await {
        return;
    }

    let usage: Vec<u32> = CPU_USAGE
        .iter()
        .take(NUM_CPUS.load(std::sync::atomic::Ordering::Relaxed))
        .map(|cpu| cpu.load(std::sync::atomic::Ordering::Relaxed))
        .collect();

    let message = json!(
        {
            "event": PublishedChannels::Cpu.to_string(),
            "data": usage,
        }
    ).to_string();
    channels.send(PublishedChannels::Cpu, message).await;
}

pub async fn ram_info(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::Ram).await {
        return;
    }
    
    let ram_usage = RAM_USED.load(std::sync::atomic::Ordering::Relaxed);
    let total_ram = TOTAL_RAM.load(std::sync::atomic::Ordering::Relaxed);

    let message = json!(
        {
            "event": PublishedChannels::Ram.to_string(),
            "data": {
                "total" : total_ram,
                "used" : ram_usage,
            },
        }
    ).to_string();
    channels.send(PublishedChannels::Ram, message).await;
}
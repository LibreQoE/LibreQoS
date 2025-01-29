use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::error::Elapsed;
use tokio::time::timeout;
use tracing::{info, warn};
use crate::node_manager::local_api::lts::ThroughputData;
use crate::node_manager::shaper_queries_actor::{remote_insight, ShaperQueryCommand};
use crate::node_manager::shaper_queries_actor::caches::{CacheType, Caches};

pub async fn shaper_queries(mut rx: tokio::sync::mpsc::Receiver<ShaperQueryCommand>) {
    info!("Starting the shaper query actor.");

    // Initialize the cache system
    let (mut caches, mut broadcast_rx) = Caches::new();
    let mut remote_insight = remote_insight::RemoteInsight::new(caches.clone());

    while let Some(command) = rx.recv().await {
        caches.cleanup().await;

        match command {
            ShaperQueryCommand::ShaperThroughput { seconds, reply } => {
                // Check for cache hit
                if let Some(result) = caches.get::<ThroughputData>(seconds).await {
                    info!("Cache hit for {seconds} seconds throughput");
                    let _ = reply.send(result.clone());
                    return;
                }
                info!("Cache miss for {seconds} seconds throughput");

                info!("Requesting {seconds} seconds throughput from remote insight.");
                remote_insight.command(remote_insight::RemoteInsightCommand::ShaperThroughput { seconds }).await;
                // Tokio timer that ticks in 30 seconds
                let my_caches = caches.clone();
                let mut my_broadcast_rx = broadcast_rx.resubscribe();
                tokio::spawn(async move {
                    let mut timer = tokio::time::interval(Duration::from_secs(30));
                    let mut timer_count = 0;
                    loop {
                        tokio::select! {
                            _ = timer.tick() => {
                                // Timed out
                                timer_count += 1;
                                if timer_count > 1 {
                                    info!("Timeout for {seconds} seconds throughput");
                                    let _ = reply.send(vec![]);
                                    break;
                                }
                            }
                            Ok(CacheType::Throughput) = my_broadcast_rx.recv() => {
                                // Cache updated
                                info!("Cache updated for {seconds} seconds throughput");
                                if let Some(result) = my_caches.get(seconds).await {
                                    info!("Sending {seconds} seconds throughput");
                                    if let Err(e) = reply.send(result) {
                                        warn!("Failed to send {seconds} seconds throughput. Oneshot died?");
                                    }
                                    info!("Sent");
                                    break;
                                }
                            }
                        }
                    }
                });
            }
            _ => {}
            /*ShaperQueryCommand::ShaperThroughput { seconds, reply } => {
                if let Some(result) = caches.throughput.get(&seconds) {
                    info!("Cache hit for {seconds} seconds throughput");
                    let _ = reply.send(result.clone());
                } else {
                    // Get the data
                    let result = ws::get_remote_data(&mut caches, seconds);

                    // Return from the cache once more
                    if result.is_ok() {
                        let Some(result) = caches.throughput.get(&seconds) else {
                            warn!("Failed to get data for {seconds} seconds: {result:?}");
                            return;
                        };
                        let _ = reply.send(result.clone());
                    } else {
                        warn!("Failed to get data for {seconds} seconds: {result:?}");
                    }
                }
            }
            ShaperQueryCommand::ShaperPackets { seconds, reply } => {
                if let Some(result) = caches.packets.get(&seconds) {
                    info!("Cache hit for {seconds} seconds packets");
                    let _ = reply.send(result.clone());
                } else {
                    // Get the data
                    let result = ws::get_remote_data(&mut caches, seconds);

                    // Return from the cache once more
                    if result.is_ok() {
                        let Some(result) = caches.packets.get(&seconds) else {
                            warn!("Failed to get data for {seconds} seconds: {result:?}");
                            return;
                        };
                        let _ = reply.send(result.clone());
                    } else {
                        warn!("Failed to get data for {seconds} seconds: {result:?}");
                    }
                }
            }
            ShaperQueryCommand::ShaperPercent { seconds, reply } => {
                if let Some(result) = caches.percent_shaped.get(&seconds) {
                    let _ = reply.send(result.clone());
                } else {
                    // Get the data
                    let result = ws::get_remote_data(&mut caches, seconds);

                    // Return from the cache once more
                    if result.is_ok() {
                        let Some(result) = caches.percent_shaped.get(&seconds) else {
                            warn!("Failed to get data for {seconds} seconds: {result:?}");
                            return;
                        };
                        let _ = reply.send(result.clone());
                    } else {
                        warn!("Failed to get data for {seconds} seconds: {result:?}");
                    }
                }
            }
            ShaperQueryCommand::ShaperFlows { seconds, reply } => {
                if let Some(result) = caches.flows.get(&seconds) {
                    let _ = reply.send(result.clone());
                } else {
                    // Get the data
                    let result = ws::get_remote_data(&mut caches, seconds);

                    // Return from the cache once more
                    if result.is_ok() {
                        let Some(result) = caches.flows.get(&seconds) else {
                            warn!("Failed to get data for {seconds} seconds: {result:?}");
                            return;
                        };
                        let _ = reply.send(result.clone());
                    } else {
                        warn!("Failed to get data for {seconds} seconds: {result:?}");
                    }
                }
            }*/
        }
    }
    warn!("Shaper query actor closing.")
}
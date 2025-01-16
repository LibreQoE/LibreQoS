mod ws;
mod timed_cache;

use std::time::Duration;
use tracing::{info, warn};
use crate::node_manager::local_api::lts::{FullPacketData, ThroughputData};
use crate::node_manager::shaper_queries_actor::timed_cache::TimedCache;

pub enum ShaperQueryCommand {
    ShaperThroughput { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<ThroughputData>> },
    ShaperPackets { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<FullPacketData>> },
}

pub fn shaper_queries_actor() -> crossbeam_channel::Sender<ShaperQueryCommand> {
    let (tx, rx) = crossbeam_channel::bounded(128);
    let _ = std::thread::Builder::new().name("shaper_queries_actor".to_string()).spawn(move || {
        shaper_queries(rx);
    });
    tx
}

fn shaper_queries(rx: crossbeam_channel::Receiver<ShaperQueryCommand>) {
    info!("Starting the shaper query actor.");
    let mut caches = Caches::new();
    while let Ok(command) = rx.recv() {
        caches.cleanup();
        match command {
            ShaperQueryCommand::ShaperThroughput { seconds, reply } => {
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
        }
    }
    warn!("Shaper query actor closing.")
}

const CACHE_DURATION: Duration = Duration::from_secs(60 * 5);

struct Caches {
    throughput: TimedCache<i32, Vec<ThroughputData>>,
    packets: TimedCache<i32, Vec<FullPacketData>>,
}

impl Caches {
    fn new() -> Self {
        Self {
            throughput: TimedCache::new(CACHE_DURATION),
            packets: TimedCache::new(CACHE_DURATION),
        }
    }

    fn cleanup(&mut self) {
        self.throughput.cleanup();
    }
}
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;
use tracing::{info, warn};
use crate::node_manager::local_api::lts::{FlowCountViewWeb, FullPacketData, PercentShapedWeb, ShaperRttHistogramEntry, ThroughputData};
use crate::node_manager::shaper_queries_actor::timed_cache::TimedCache;

const CACHE_DURATION: Duration = Duration::from_secs(60 * 5);

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum CacheType {
    Throughput,
    Packets,
    PercentShaped,
    Flows,
    RttHistogram,
}

impl CacheType {
    fn from_str(tag: &str) -> Self {
        match tag {
            "throughput" => Self::Throughput,
            "packets" => Self::Packets,
            "percent" => Self::PercentShaped,
            "flows" => Self::Flows,
            "rtt_histogram" => Self::RttHistogram,
            _ => panic!("Unknown cache type: {}", tag),
        }
    }
}

pub struct Caches {
    on_update: tokio::sync::broadcast::Sender<CacheType>,
    cache: Mutex<HashMap<(CacheType, i32), (Instant, Vec<u8>)>>,
}

impl Caches {
    pub fn new() -> (Arc<Self>, tokio::sync::broadcast::Receiver<CacheType>) {
        let (tx, rx) = tokio::sync::broadcast::channel(32);
        (Arc::new(Self {
            on_update: tx,
            cache: Mutex::new(HashMap::new()),
        }), rx)
    }

    pub async fn cleanup(&self) {
        let now = Instant::now();
        let mut cache = self.cache.lock().await;
        cache.retain(|(_, _), (time, _)| now.duration_since(*time) < CACHE_DURATION);
    }

    pub async fn store(&self, tag: String, seconds: i32, data: Vec<u8>) {
        info!("Storing cache for {} seconds: {:?}", seconds, tag);
        let mut cache = self.cache.lock().await;
        let tag = CacheType::from_str(&tag);
        cache.insert((tag, seconds), (Instant::now(), data));
        drop(cache);
        let _ = self.on_update.send(tag);
        info!("Cache stored for {} seconds: {:?}", seconds, tag);
    }

    pub async fn get<T: Cacheable + DeserializeOwned>(&self, seconds: i32) -> Option<Vec<T>> {
        info!("Checking cache for {} seconds: {:?}", seconds, T::tag());
        let cache = self.cache.lock().await;
        let tag = T::tag();
        let Some((_, data)) = cache.get(&(tag, seconds)) else {
            drop(cache);
            info!("Cache miss for {} seconds: {:?}", seconds, tag);
            return None;
        };
        info!("Cache hit for {} seconds {:?}. Length: {}", seconds, tag, data.len());
        let deserialized = serde_cbor::from_slice(&data);
        drop(cache);
        if let Err(e) = deserialized {
            warn!("Failed to deserialize cache: {:?}", e);
            return None;
        }
        info!("Cache deserialized for {} seconds: {:?}", seconds, tag);
        Some(deserialized.unwrap())
    }
}

pub trait Cacheable {
    fn tag() -> CacheType;
}

impl Cacheable for ThroughputData {
    fn tag() -> CacheType {
        CacheType::Throughput
    }
}

impl Cacheable for FullPacketData {
    fn tag() -> CacheType {
        CacheType::Packets
    }
}

impl Cacheable for PercentShapedWeb {
    fn tag() -> CacheType {
        CacheType::PercentShaped
    }
}

impl Cacheable for FlowCountViewWeb {
    fn tag() -> CacheType {
        CacheType::Flows
    }
}

impl Cacheable for ShaperRttHistogramEntry {
    fn tag() -> CacheType {
        CacheType::RttHistogram
    }
}

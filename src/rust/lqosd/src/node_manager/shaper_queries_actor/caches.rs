use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;
use tracing::{info, warn};
use crate::node_manager::local_api::lts::{FlowCountViewWeb, FullPacketData, PercentShapedWeb, ThroughputData};
use crate::node_manager::shaper_queries_actor::timed_cache::TimedCache;

const CACHE_DURATION: Duration = Duration::from_secs(60 * 5);

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum CacheType {
    Throughput,
    Packets,
    PercentShaped,
    Flows,
}

impl CacheType {
    fn from_str(tag: &str) -> Self {
        match tag {
            "throughput" => Self::Throughput,
            "packets" => Self::Packets,
            "percent_shaped" => Self::PercentShaped,
            "flows" => Self::Flows,
            _ => panic!(),
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
        let mut cache = self.cache.lock().await;
        let tag = match tag.as_str() {
            "throughput" => CacheType::Throughput,
            "packets" => CacheType::Packets,
            "percent_shaped" => CacheType::PercentShaped,
            "flows" => CacheType::Flows,
            _ => return,
        };
        cache.insert((tag, seconds), (Instant::now(), data));
        let _ = self.on_update.send(tag);
    }

    pub async fn get<T: Cacheable + DeserializeOwned>(&self, seconds: i32) -> Option<Vec<T>> {
        let cache = self.cache.lock().await;
        let tag = T::tag();
        let (_, data) = cache.get(&(tag, seconds))?;
        info!("Cache hit for {} seconds {:?}. Length: {}", seconds, tag, data.len());
        let deserialized = serde_cbor::from_slice(&data);
        if let Err(e) = deserialized {
            warn!("Failed to deserialize cache: {:?}", e);
            return None;
        }
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
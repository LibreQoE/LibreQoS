use crate::node_manager::local_api::lts::{
    AsnFlowSizeWeb, FlowCountViewWeb, FullPacketData, PercentShapedWeb, RecentMedians,
    ShaperRttHistogramEntry, ThroughputData, Top10Circuit, Worst10RttCircuit, Worst10RxmitCircuit,
};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{info, warn};

const CACHE_DURATION: Duration = Duration::from_secs(60 * 5);

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum CacheType {
    Throughput,
    Packets,
    PercentShaped,
    Flows,
    RttHistogram,
    TopDownloaders,
    WorstRtt,
    WorstRxmit,
    TopFlows,
    RecentMedians,
}

impl CacheType {
    fn from_str(tag: &str) -> Self {
        match tag {
            "throughput" => Self::Throughput,
            "packets" => Self::Packets,
            "percent" => Self::PercentShaped,
            "flows" => Self::Flows,
            "rtt_histogram" => Self::RttHistogram,
            "top_downloaders" => Self::TopDownloaders,
            "worst_rtt" => Self::WorstRtt,
            "worst_rxmit" => Self::WorstRxmit,
            "top_flows" => Self::TopFlows,
            "recent_median" => Self::RecentMedians,
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
        (
            Arc::new(Self {
                on_update: tx,
                cache: Mutex::new(HashMap::new()),
            }),
            rx,
        )
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
        info!(
            "Cache hit for {} seconds {:?}. Length: {}",
            seconds,
            tag,
            data.len()
        );
        let deserialized = serde_cbor::from_slice(&data);
        drop(cache);
        if let Err(e) = deserialized {
            warn!("Failed to deserialize cache {tag:?}: {:?}", e);
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

impl Cacheable for Top10Circuit {
    fn tag() -> CacheType {
        CacheType::TopDownloaders
    }
}

impl Cacheable for Worst10RttCircuit {
    fn tag() -> CacheType {
        CacheType::WorstRtt
    }
}

impl Cacheable for Worst10RxmitCircuit {
    fn tag() -> CacheType {
        CacheType::WorstRxmit
    }
}

impl Cacheable for AsnFlowSizeWeb {
    fn tag() -> CacheType {
        CacheType::TopFlows
    }
}

impl Cacheable for RecentMedians {
    fn tag() -> CacheType {
        CacheType::RecentMedians
    }
}

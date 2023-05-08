use once_cell::sync::Lazy;
use tokio::sync::Mutex;
use crate::collector::{ThroughputSummary, network_tree::NetworkTreeEntry};

pub(crate) static SESSION_BUFFER: Lazy<Mutex<Vec<StatsSession>>> =
    Lazy::new(|| Mutex::new(Vec::new()));

pub(crate) struct StatsSession {
    pub(crate) throughput: ThroughputSummary,
    pub(crate) network_tree: Vec<(usize, NetworkTreeEntry)>,
}
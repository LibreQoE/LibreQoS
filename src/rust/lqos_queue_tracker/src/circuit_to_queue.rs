use crate::queue_store::QueueStore;
use dashmap::DashMap;
use once_cell::sync::Lazy;

pub(crate) static CIRCUIT_TO_QUEUE: Lazy<DashMap<String, QueueStore>> = Lazy::new(DashMap::new);

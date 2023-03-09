use dashmap::DashMap;
use once_cell::sync::Lazy;
use crate::queue_store::QueueStore;

pub(crate) static CIRCUIT_TO_QUEUE: Lazy<DashMap<String, QueueStore>> =
  Lazy::new(DashMap::new);

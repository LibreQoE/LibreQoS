use lazy_static::*;
use parking_lot::RwLock;
use std::collections::HashMap;
use crate::queue_store::QueueStore;

lazy_static! {
    pub(crate) static ref CIRCUIT_TO_QUEUE: RwLock<HashMap<String, QueueStore>> =
        RwLock::new(HashMap::new());
}
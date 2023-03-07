use once_cell::sync::Lazy;

use crate::queue_store::QueueStore;
use std::collections::HashMap;
use std::sync::RwLock;

pub(crate) static CIRCUIT_TO_QUEUE: Lazy<RwLock<HashMap<String, QueueStore>>> =
  Lazy::new(|| RwLock::new(HashMap::new()));

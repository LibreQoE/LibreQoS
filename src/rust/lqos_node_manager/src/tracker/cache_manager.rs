use tokio::time::{Duration, Instant};

use std::collections::HashMap;

use crate::AppState;
use crate::utils;

pub struct CacheManager {
    buffers: HashMap<String, utils::RedisRing>,
    connection: redis::Client
}

impl Default for CacheManager {
    fn default() -> Self {
        CacheManager {
            buffers: HashMap::new(),
        }
    }
}

impl CacheManager {
    fn update(&self) {
        self.buffers.insert()
    }
}
#![allow(dead_code)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct TimedCache<K, V> {
    cache: HashMap<K, (V, Instant)>,
    ttl: Duration,
}

impl<K, V> TimedCache<K, V>
where
    K: Eq + std::hash::Hash,
{
    pub fn new(ttl: Duration) -> Self {
        TimedCache {
            cache: HashMap::new(),
            ttl,
        }
    }

    pub fn insert(&mut self, key: K, value: V) {
        self.cache.insert(key, (value, Instant::now()));
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.cache.get(key).map(|(v, _)| v)
    }

    pub fn remove(&mut self, key: &K) {
        self.cache.remove(key);
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.cache.contains_key(key)
    }

    pub fn cleanup(&mut self) {
        let now = Instant::now();
        self.cache
            .retain(|_, (_, time)| now.duration_since(*time) < self.ttl);
    }
}

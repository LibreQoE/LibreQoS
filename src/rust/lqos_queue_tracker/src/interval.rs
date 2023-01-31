use lazy_static::*;
use std::sync::atomic::AtomicU64;

lazy_static! {
  pub(crate) static ref QUEUE_MONITOR_INTERVAL: AtomicU64 =
    AtomicU64::new(1000);
}

pub fn set_queue_refresh_interval(interval_ms: u64) {
  QUEUE_MONITOR_INTERVAL
    .store(interval_ms, std::sync::atomic::Ordering::Relaxed);
}

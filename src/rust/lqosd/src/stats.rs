use std::sync::atomic::AtomicU64;

pub static BUS_REQUESTS: AtomicU64 = AtomicU64::new(0);
pub static TIME_TO_POLL_HOSTS: AtomicU64 = AtomicU64::new(0);
pub static HIGH_WATERMARK_DOWN: AtomicU64 = AtomicU64::new(0);
pub static HIGH_WATERMARK_UP: AtomicU64 = AtomicU64::new(0);

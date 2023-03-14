//! Count statistics

use std::sync::atomic::AtomicU64;

/// Perf event counter
pub static COLLECTED_EVENTS: AtomicU64 = AtomicU64::new(0);

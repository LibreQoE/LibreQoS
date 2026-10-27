//! Aggregate CAKE queue accounting retained for the stats/tuning surfaces.
//!
//! K3-B retired the kernel `tc` queue monitor that fed this store; the
//! bridge queue-stats wire is the live per-circuit source. The aggregate
//! counters remain accessible (honest-empty) for the product surfaces that
//! still read them.

use std::sync::atomic::{AtomicBool, Ordering};

mod all_queue_data;
pub use all_queue_data::*;

static QUEUE_STATS_STALE: AtomicBool = AtomicBool::new(false);

/// Returns `true` when queue counts are intentionally held at their last-known values.
pub fn queue_stats_stale() -> bool {
    QUEUE_STATS_STALE.load(Ordering::Relaxed)
}

/// 3a A1: mark the queue-stats source stale. The bridge bulk qdisc read sets
/// this from its own success (a truncated/empty/failed read ⇒ `true`), so the
/// flag reflects the SOURCE rather than a hardwired `false`.
pub fn set_queue_stats_stale(stale: bool) {
    QUEUE_STATS_STALE.store(stale, Ordering::Relaxed);
}

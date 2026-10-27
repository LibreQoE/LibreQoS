//! Retrieves CAKE queue statistics, aggregate counters, and the queue-type
//! counts consumed by the product surfaces.
//!
//! K3-B: the kernel `tc` queue monitor, the per-circuit raw-queue feed, and
//! the `queuingStructure.json` reader are retired. The bridge queue-stats
//! wire is the live per-circuit source; this crate retains the aggregate
//! counters (now honest-empty until a bridge aggregate feed lands) and the
//! TC-tree parser used by the benchmarker.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]
mod queue_types;
mod tracking;

pub use queue_types::deserialize_tc_tree; // Exported for the benchmarker
pub use tracking::{
    ALL_QUEUE_SUMMARY, QueueCounts, TOTAL_QUEUE_STATS, queue_stats_stale, set_queue_stats_stale,
};

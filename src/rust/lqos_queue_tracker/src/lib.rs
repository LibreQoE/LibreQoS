//! Retrieves queue statistics from the Linux `tc` shaper, and stores
//! them in a `QueueStore` for later retrieval. The `QueueStore` is
//! thread-safe, and can be accessed from multiple threads. It is
//! updated periodically by a separate thread, and accumulates statistics
//! between polling periods.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]
mod bus;
mod circuit_to_queue;
mod interval;
mod queue_diff;
mod queue_store;
mod queue_structure;
mod queue_types;
mod tracking;

/// How many history items do we store?
const NUM_QUEUE_HISTORY: usize = 600;

pub use bus::get_raw_circuit_data;
pub use interval::set_queue_refresh_interval;
pub use queue_structure::{
    QUEUE_STRUCTURE, QUEUE_STRUCTURE_CHANGED_STORMGUARD, spawn_queue_structure_monitor,
};
pub use queue_types::deserialize_tc_tree; // Exported for the benchmarker
pub use tracking::spawn_queue_monitor;
pub use tracking::{ALL_QUEUE_SUMMARY, QueueCounts, TOTAL_QUEUE_STATS};
pub use tracking::{add_watched_queue, still_watching};

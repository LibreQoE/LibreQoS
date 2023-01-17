mod queue_store;
mod queue_types;
mod queue_diff;
mod circuit_to_queue;
mod queue_structure;
mod tracking;
mod interval;
mod bus;

/// How many history items do we store?
const NUM_QUEUE_HISTORY: usize = 600;

pub use queue_structure::spawn_queue_structure_monitor;
pub use tracking::spawn_queue_monitor;
pub use bus::get_raw_circuit_data;
pub use queue_types::deserialize_tc_tree; // Exported for the benchmarker
pub use interval::set_queue_refresh_interval;
pub use tracking::{add_watched_queue, still_watching};
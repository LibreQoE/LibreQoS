//! RTT (round-trip time) types and helpers.
//!
//! These types are shared between crates (e.g. `lqosd` and `lqos_config`) so
//! RTT aggregation can be performed consistently across the stack.

mod rtt_buffer;
mod rtt_data;

pub use rtt_buffer::{RttBucket, RttBuffer};
pub use rtt_data::RttData;

/// Effective direction of a flow/RTT sample from the shaped device perspective.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowbeeEffectiveDirection {
    /// Download direction (to the subscriber).
    Download = 0,
    /// Upload direction (from the subscriber).
    Upload = 1,
}


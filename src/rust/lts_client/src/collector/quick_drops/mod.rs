//! Provides a quick'n'dirty 10 second snapshot of the TC queue
//! status. This is used by the LTS system to provide a quick'n'dirty
//! summary of drops and marks for the last 10 seconds.

mod queue_structure;
mod retriever;
mod stats_diff;
pub(crate) use retriever::*;
use serde::{Serialize, Deserialize};
pub(crate) use stats_diff::*;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CakeStats {
    pub circuit_id: String,
    pub drops: u64,
    pub marks: u64,
}
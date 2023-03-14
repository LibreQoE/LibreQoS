//! Provides an interface to the Heimdall packet watching
//! system. Heimdall watches traffic flows, and is notified
//! about their contents via the eBPF Perf system.

mod config;
pub mod perf_interface;
pub mod stats;
pub use config::{HeimdallMode, HeimdalConfig};

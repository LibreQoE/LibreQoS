//! Collection of utility functions for LibreQoS

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]
mod commands;

/// Provides a Linux file-descriptor based timing service.
pub mod fdtimer;

/// Wrapper for watching when a file has changed.
pub mod file_watcher;

/// Utilities for handling strings in hex format
pub mod hex_string;

/// Utilities for scaling bits and packets to human-readable format
pub mod packet_scale;
mod string_table_enum;

/// Rolling heatmap data storage for executive summary views.
pub mod temporal_heatmap;
/// Re-export HeatmapBlocks for downstream crates.
pub use temporal_heatmap::HeatmapBlocks;
/// RTT histograms and strongly-typed RTT units.
pub mod rtt;
/// Helpers for units of measurement
pub mod units;
/// Utilities dealing with Unix Timestamps
pub mod unix_time;
/// Rolling QoQ (0..100) score heatmap storage.
pub mod qoq_heatmap;
mod xdp_ip_address;
/// Quality-of-Outcome (QoO) scoring utilities and profile loading.
pub mod qoo;

/// XDP compatible IP Address
pub use xdp_ip_address::XdpIpAddress;

/// Insight standard hasher for strings
pub fn hash_to_i64(text: &str) -> i64 {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    hasher.write(text.as_bytes());
    hasher.finish() as i64
}

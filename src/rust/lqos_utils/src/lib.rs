//! Collection of utility functions for LibreQoS

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]
mod commands;

/// Provides a Linux file-descriptor based timing service.
pub mod fdtimer;

/// Wrapper for watching when a file has changed.
pub mod file_watcher;

/// Watches a directory and coalesces filesystem events.
pub mod directory_watcher;

/// Utilities for handling strings in hex format
pub mod hex_string;

/// Shared mapped-circuit licensing definitions.
pub mod mapped_circuits;
/// Utilities for scaling bits and packets to human-readable format
pub mod packet_scale;
/// Cross-process PID-file locking helpers.
pub mod process_lock;
mod string_table_enum;

/// Rolling heatmap data storage for executive summary views.
pub mod temporal_heatmap;
pub use mapped_circuits::{
    DEFAULT_MAPPED_CIRCUIT_LIMIT, is_valid_ip_mapping_text, is_valid_ipv4_prefix,
    is_valid_ipv6_prefix, unique_mapped_circuit_hashes,
};
/// Re-export HeatmapBlocks for downstream crates.
pub use temporal_heatmap::HeatmapBlocks;
/// Quality-of-Outcome (QoO) scoring utilities and profile loading.
pub mod qoo;
/// Rolling QoQ (0..100) score heatmap storage.
pub mod qoq_heatmap;
/// RTT histograms and strongly-typed RTT units.
pub mod rtt;
/// Helpers for initializing the process-wide Rustls crypto provider.
pub mod rustls;
/// Helpers for units of measurement
pub mod units;
/// Utilities dealing with Unix Timestamps
pub mod unix_time;
mod xdp_ip_address;

/// XDP compatible IP Address
pub use xdp_ip_address::XdpIpAddress;

/// Normalizes a circuit identifier for case-insensitive catalog and overlay lookups.
pub fn normalize_circuit_id_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Insight standard hasher for strings
pub fn hash_to_i64(text: &str) -> i64 {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    hasher.write(text.as_bytes());
    hasher.finish() as i64
}

#[cfg(test)]
mod tests {
    use super::normalize_circuit_id_key;

    #[test]
    fn circuit_id_key_trims_and_ascii_lowercases() {
        assert_eq!(normalize_circuit_id_key(" Circuit-42 "), "circuit-42");
    }
}

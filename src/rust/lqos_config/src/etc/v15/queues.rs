//! Queue Generation definitions (originally from ispConfig.py)

use serde::{Serialize, Deserialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct QueueConfig {
    /// Which SQM to use by default
    pub default_sqm: String,

    /// Should we monitor only, and not shape traffic?
    pub monitor_only: bool,

    /// Upstream bandwidth total - download
    pub uplink_bandwidth_mbps: u32,

    /// Downstream bandwidth total - upload
    pub downlink_bandwidth_mbps: u32,

    /// Upstream bandwidth per interface queue
    pub generated_pn_download_mbps: u32,

    /// Downstream bandwidth per interface queue
    pub generated_pn_upload_mbps: u32,

    /// Should shell commands actually execute, or just be printed?
    pub dry_run: bool,

    /// Should `sudo` be prefixed on commands?
    pub sudo: bool,

    /// Should we override the number of available queues?
    pub override_available_queues: Option<u32>,

    /// Should we invoke the binpacking algorithm to optimize flat
    /// networks?
    pub use_binpacking: bool,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            default_sqm: "cake diffserv4".to_string(),
            monitor_only: false,
            uplink_bandwidth_mbps: 1_000,
            downlink_bandwidth_mbps: 1_000,
            generated_pn_download_mbps: 1_000,
            generated_pn_upload_mbps: 1_000,
            dry_run: false,
            sudo: false,
            override_available_queues: None,
            use_binpacking: false,
        }
    }
}
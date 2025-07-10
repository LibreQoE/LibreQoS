//! Queue Generation definitions (originally from ispConfig.py)

use allocative::Allocative;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct QueueConfig {
    /// Which SQM to use by default
    pub default_sqm: String,

    /// Should we monitor only, and not shape traffic?
    pub monitor_only: bool,

    /// Upstream bandwidth total - download
    pub uplink_bandwidth_mbps: u64,

    /// Downstream bandwidth total - upload
    pub downlink_bandwidth_mbps: u64,

    /// Upstream bandwidth per interface queue
    pub generated_pn_download_mbps: u64,

    /// Downstream bandwidth per interface queue
    pub generated_pn_upload_mbps: u64,

    /// Should shell commands actually execute, or just be printed?
    pub dry_run: bool,

    /// Should `sudo` be prefixed on commands?
    pub sudo: bool,

    /// Should we override the number of available queues?
    pub override_available_queues: Option<u32>,

    /// Should we invoke the binpacking algorithm to optimize flat
    /// networks?
    pub use_binpacking: bool,

    /// Enable lazy queue creation (only create circuit queues when traffic is detected)
    pub lazy_queues: Option<LazyQueueMode>,

    /// Expiration time in seconds for unused lazy queues (None = never expire)
    pub lazy_expire_seconds: Option<u64>,
}

/// Lazy queue creation modes
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default, Allocative)]
pub enum LazyQueueMode {
    /// No lazy queue creation
    #[default]
    No,
    /// HTB queues for circuits are created on build, but CAKE classes are created on demand
    Htb,
    /// Full lazy queue creation, both HTB queues and CAKE classes are created on demand.
    Full,
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
            lazy_queues: None, // Default to disabled for backward compatibility
            lazy_expire_seconds: Some(600), // 10 minutes default
        }
    }
}

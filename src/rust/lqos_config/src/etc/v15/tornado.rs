//! Tornado definitions (originally from ispConfig.py)

use serde::{Deserialize, Serialize};

/// Configuration for the Tornado module (auto-rate)
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct TornadoConfig {
    /// Whether Tornado is enabled or not
    pub enabled: bool,
    /// List of targets (site names) to monitor
    pub targets: Vec<TornadoSite>,
    /// Whether to run in dry run mode (no actual changes)
    pub dry_run: bool,
    /// Optional log file path - emits a CSV of site and rates
    pub log_file: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct TornadoSite {
    /// The site name as it appears in network.json
    pub name: String,
    /// Maximum bandwidth [download, upload] in Mbps
    pub max_mbps: [u64; 2],
    /// Minimum bandwidth [download, upload] in Mbps
    pub min_mbps: [u64; 2],
    /// Step Size [download, upload] in Mbps. Changes occur increments of this size
    pub step_mbps: [u64; 2],
}

impl Default for TornadoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            targets: Vec::new(),
            dry_run: true,
            log_file: None,
        }
    }
}

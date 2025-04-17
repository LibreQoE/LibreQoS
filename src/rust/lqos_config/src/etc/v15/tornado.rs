//! Tornado definitions (originally from ispConfig.py)

use serde::{Deserialize, Serialize};

/// Configuration for the Tornado module (auto-rate)
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct TornadoConfig {
    /// Whether Tornado is enabled or not
    pub enabled: bool,
    /// List of targets (site names) to monitor
    pub targets: Vec<String>,
    /// Whether to run in dry run mode (no actual changes)
    pub dry_run: bool,
    /// Optional log file path - emits a CSV of site and rates
    pub log_file: Option<String>,
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

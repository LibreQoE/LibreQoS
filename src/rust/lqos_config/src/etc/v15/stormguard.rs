//! StormGuard definitions (originally from ispConfig.py)

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Configuration for the Tornado module (auto-rate)
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct StormguardConfig {
    /// Whether Tornado is enabled or not
    pub enabled: bool,
    /// List of targets (site names) to monitor
    pub targets: Vec<String>,
    /// Whether to run in dry run mode (no actual changes)
    pub dry_run: bool,
    /// Optional log file path - emits a CSV of site and rates
    pub log_file: Option<String>,
    /// Minimum Percentage (e.g. 0.5 for 50%) Download
    pub minimum_download_percentage: f32,
    /// Minimum Percentage (e.g. 0.5 for 50%) Upload
    pub minimum_upload_percentage: f32,
}


impl Default for StormguardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            targets: Vec::new(),
            dry_run: true,
            log_file: None,
            minimum_download_percentage: 0.5,
            minimum_upload_percentage: 0.5,
        }
    }
}

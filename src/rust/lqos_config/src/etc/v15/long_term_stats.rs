//! Defines configuration for the LTS project

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LongTermStats {
    /// Should we store long-term stats at all?
    pub gather_stats: bool,

    /// How frequently should stats be accumulated into a long-term
    /// min/max/avg format per tick?
    pub collation_period_seconds: u32,

    /// The license key for submitting stats to a LibreQoS hosted
    /// statistics server
    pub license_key: Option<String>,

    /// UISP reporting period (in seconds). UISP queries can be slow,
    /// so hitting it every second or 10 seconds is going to cause problems
    /// for some people. A good default may be 5 minutes. Not specifying this
    /// disabled UISP integration.
    pub uisp_reporting_interval_seconds: Option<u64>,

    /// If set, then this URL will be used for connecting to a self-hosted or
    /// development LTS server. It will be decorated with https:// and :443
    pub lts_url: Option<String>,

    /// If enabled, Insight (LTS2) will be used in addition to the normal
    /// LTS system. This system is in alpha and is invite only for now.
    pub use_insight: Option<bool>,
}

impl Default for LongTermStats {
    fn default() -> Self {
        Self {
            gather_stats: true,
            collation_period_seconds: 60,
            license_key: None,
            uisp_reporting_interval_seconds: Some(300),
            lts_url: None,
            use_insight: None,
        }
    }
}

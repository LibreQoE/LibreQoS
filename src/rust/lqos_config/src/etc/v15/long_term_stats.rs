//! Defines configuration for the LTS project

use allocative::Allocative;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub struct LongTermStats {
    /// Should we store long-term stats at all?
    pub gather_stats: bool,

    /// How frequently should stats be accumulated into a long-term
    /// min/max/avg format per tick?
    pub collation_period_seconds: u32,

    /// The license key for submitting stats to a LibreQoS hosted
    /// statistics server
    pub license_key: Option<String>,

    /// If set, then this URL will be used for connecting to a self-hosted or
    /// development LTS server. It will be decorated with https:// and :443
    pub lts_url: Option<String>,

    /// If enabled, the Insight topology will be used to determine the
    /// topology of the network. This is in alpha and may not work
    /// as expected. It is disabled by default. NONE and FALSE are
    /// equivalent.
    pub enable_insight_topology: Option<bool>,

    /// If set, this will be the role of the node in the Insight topology.
    /// "Primary", "None" are the only valid values for now. This is in alpha
    /// and may not work as expected. It is disabled by default.
    pub insight_topology_role: Option<String>,
}

impl Default for LongTermStats {
    fn default() -> Self {
        Self {
            gather_stats: true,
            collation_period_seconds: 60,
            license_key: None,
            lts_url: None,
            enable_insight_topology: None,
            insight_topology_role: None,
        }
    }
}

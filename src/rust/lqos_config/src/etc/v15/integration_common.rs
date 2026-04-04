//! Common integration variables, shared between integrations

use allocative::Allocative;
use serde::{Deserialize, Serialize};

fn default_ethernet_port_limits_enabled() -> bool {
    true
}

/// Shared integration defaults used by CRM/NMS integrations.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct IntegrationConfig {
    /// Replace names with addresses?
    pub circuit_name_as_address: bool,

    /// Always overwrite network.json?
    pub always_overwrite_network_json: bool,

    /// Queue refresh interval in minutes
    pub queue_refresh_interval_mins: u32,

    /// Enable Mikrotik IPv6 enrichment for non-UISP integrations
    #[serde(default)]
    pub use_mikrotik_ipv6: bool,

    /// Root node promotion
    pub promote_to_root: Option<Vec<String>>,

    /// Client bandwidth multiplier
    pub client_bandwidth_multiplier: Option<f32>,

    /// Enable circuit Ethernet-port based shaping caps for integrations that can detect negotiated port speed.
    #[serde(default = "default_ethernet_port_limits_enabled")]
    pub ethernet_port_limits_enabled: bool,

    /// Optional operator override for Ethernet port headroom multiplier.
    ///
    /// When unset, LibreQoS defaults to `0.94`.
    #[serde(default)]
    pub ethernet_port_limit_multiplier: Option<f32>,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            circuit_name_as_address: false,
            always_overwrite_network_json: true,
            queue_refresh_interval_mins: 30,
            use_mikrotik_ipv6: false,
            promote_to_root: None,
            client_bandwidth_multiplier: None,
            ethernet_port_limits_enabled: true,
            ethernet_port_limit_multiplier: None,
        }
    }
}

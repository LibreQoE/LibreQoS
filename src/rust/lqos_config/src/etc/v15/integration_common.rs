//! Common integration variables, shared between integrations

use allocative::Allocative;
use serde::{Deserialize, Serialize};

fn default_ethernet_port_limits_enabled() -> bool {
    true
}

fn default_attachment_health_enabled() -> bool {
    true
}

fn default_attachment_probe_interval_seconds() -> u64 {
    1
}

fn default_attachment_fail_after_missed() -> u32 {
    5
}

fn default_attachment_hold_down_seconds() -> u64 {
    30
}

fn default_attachment_clear_after_successes() -> u32 {
    3
}

fn default_attachment_refresh_debounce_seconds() -> u64 {
    3
}

/// Shared runtime defaults for Topology Manager attachment health probing.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct TopologyAttachmentHealthConfig {
    /// Master switch for the attachment-health runtime.
    #[serde(default = "default_attachment_health_enabled")]
    pub enabled: bool,

    /// Seconds between probe rounds.
    #[serde(default = "default_attachment_probe_interval_seconds")]
    pub probe_interval_seconds: u64,

    /// Consecutive failed rounds required before an attachment pair is suppressed.
    #[serde(default = "default_attachment_fail_after_missed")]
    pub fail_after_missed: u32,

    /// Minimum seconds a suppressed pair remains suppressed before recovery is allowed.
    #[serde(default = "default_attachment_hold_down_seconds")]
    pub hold_down_seconds: u64,

    /// Consecutive successful rounds required to clear suppression after hold-down.
    #[serde(default = "default_attachment_clear_after_successes")]
    pub clear_after_successes: u32,

    /// Debounce window used before triggering topology/shaping refresh.
    #[serde(default = "default_attachment_refresh_debounce_seconds")]
    pub refresh_debounce_seconds: u64,
}

impl Default for TopologyAttachmentHealthConfig {
    fn default() -> Self {
        Self {
            enabled: default_attachment_health_enabled(),
            probe_interval_seconds: default_attachment_probe_interval_seconds(),
            fail_after_missed: default_attachment_fail_after_missed(),
            hold_down_seconds: default_attachment_hold_down_seconds(),
            clear_after_successes: default_attachment_clear_after_successes(),
            refresh_debounce_seconds: default_attachment_refresh_debounce_seconds(),
        }
    }
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

    /// Shared defaults for attachment-health probing and runtime topology suppression.
    #[serde(default)]
    pub topology_attachment_health: TopologyAttachmentHealthConfig,
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
            topology_attachment_health: TopologyAttachmentHealthConfig::default(),
        }
    }
}

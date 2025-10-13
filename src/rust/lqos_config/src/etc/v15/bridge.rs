//! Defines a two-interface bridge configuration.
//! A config file must contain EITHER this, or a `single_interface`
//! section, but not both.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Represents a two-interface bridge configuration.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct BridgeConfig {
    /// Use the XDP-accelerated bridge?
    pub use_xdp_bridge: bool,

    /// The name of the first interface, facing the Internet
    pub to_internet: String,

    /// The name of the second interface, facing the LAN
    pub to_network: String,

    /// The sandwich mode, if any. Sandwich mode enables a veth bridge pair,
    /// both for compatibility (e.g. if one interface doesn't support XDP),
    /// and for attaching an absolute rate limiter to the bridge.
    pub sandwich: Option<SandwichMode>,
}

/// The sandwich mode to use, if any.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub enum SandwichMode {
    /// No sandwich mode - direct interfaces
    None,
    /// Use a veth pair as the LibreQoS interface set, and attach a bridge
    /// on each end to the physical interfaces.
    Full {
        /// Whether to attach an absolute rate limiter to the bridge
        with_rate_limiter: SandwichRateLimiter,
        /// Normally, the rate limiter is set to the bandwidth of the
        /// connection. This allows overriding that for download traffic.
        rate_override_mbps_down: Option<u64>,
        /// Normally, the rate limiter is set to the bandwidth of the
        /// connection. This allows overriding that for upload traffic.
        rate_override_mbps_up: Option<u64>,
        /// Number of TX queues to use on the veth interfaces
        /// (Defaults to the available CPU cores)
        queue_override: Option<usize>,
        /// Attach an fq_codel child qdisc under the HTB class for better queueing behavior
        #[serde(default)]
        use_fq_codel: bool,
    },
}

/// The type of rate limiting to apply in sandwich mode
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub enum SandwichRateLimiter {
    /// No rate limiter
    None,
    /// Rate limit only download traffic
    Download,
    /// Rate limit only upload traffic
    Upload,
    /// Rate limit both download and upload traffic
    Both,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            use_xdp_bridge: true,
            to_internet: "eth0".to_string(),
            to_network: "eth1".to_string(),
            sandwich: None,
        }
    }
}

/// Represents a single-interface bridge
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct SingleInterfaceConfig {
    /// The name of the interface
    pub interface: String,

    /// The VLAN ID facing the Internet
    pub internet_vlan: u32,

    /// The VLAN ID facing the LAN
    pub network_vlan: u32,
}

impl Default for SingleInterfaceConfig {
    fn default() -> Self {
        Self {
            interface: "eth0".to_string(),
            internet_vlan: 2,
            network_vlan: 3,
        }
    }
}

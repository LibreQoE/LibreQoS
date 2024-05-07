//! Defines a two-interface bridge configuration.
//! A config file must contain EITHER this, or a `single_interface`
//! section, but not both.

use serde::{Deserialize, Serialize};

/// Represents a two-interface bridge configuration.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct BridgeConfig {
    /// Use the XDP-accelerated bridge?
    pub use_xdp_bridge: bool,

    /// The name of the first interface, facing the Internet
    pub to_internet: String,

    /// The name of the second interface, facing the LAN
    pub to_network: String,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            use_xdp_bridge: true,
            to_internet: "eth0".to_string(),
            to_network: "eth1".to_string(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
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
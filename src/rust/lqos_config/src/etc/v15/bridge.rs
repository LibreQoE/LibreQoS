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

    /// Optional MTU for LibreQoS-managed Linux bridge interfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u32>,

    /// Route traffic through LibreQoS-managed veth devices when the selected
    /// physical interfaces cannot host the XDP programs directly.
    #[serde(default, skip_serializing_if = "bool_is_false")]
    pub compatibility_shim: bool,
}

fn bool_is_false(value: &bool) -> bool {
    !*value
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            use_xdp_bridge: true,
            to_internet: "eth0".to_string(),
            to_network: "eth1".to_string(),
            mtu: None,
            compatibility_shim: false,
        }
    }
}

impl BridgeConfig {
    /// Returns whether the veth interface compatibility shim is enabled.
    pub fn compatibility_shim_enabled(&self) -> bool {
        self.compatibility_shim
    }

    /// Checks invariants required by the veth interface compatibility shim.
    pub fn validate_compatibility_shim(&self) -> Result<(), &'static str> {
        if self.compatibility_shim_enabled() && !self.use_xdp_bridge {
            return Err("bridge.compatibility_shim requires bridge.use_xdp_bridge = true");
        }
        Ok(())
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

    /// Optional MTU for the LibreQoS-managed trunk interface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u32>,
}

impl Default for SingleInterfaceConfig {
    fn default() -> Self {
        Self {
            interface: "eth0".to_string(),
            internet_vlan: 2,
            network_vlan: 3,
            mtu: None,
        }
    }
}

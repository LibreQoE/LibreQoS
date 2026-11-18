//! Defines a two-interface bridge configuration.
//! A config file must contain EITHER this, or a `single_interface`
//! section, but not both.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Returns a compatibility issue when Linux bonding settings cannot use native XDP.
///
/// An omitted mode uses the bonding driver's native-XDP-compatible default. Member NIC driver
/// support cannot be determined from configuration alone and is validated when XDP attaches.
pub fn native_xdp_bond_issue(
    mode: Option<&str>,
    transmit_hash_policy: Option<&str>,
) -> Option<String> {
    if let Some(mode) = mode
        && !matches!(
            mode.trim()
                .trim_matches(['[', ']'])
                .to_ascii_lowercase()
                .as_str(),
            "balance-rr" | "active-backup" | "balance-xor" | "802.3ad" | "0" | "1" | "2" | "4"
        )
    {
        return Some(format!("Bond mode {mode} does not support native XDP."));
    }
    if transmit_hash_policy.is_some_and(|policy| {
        policy
            .trim()
            .trim_matches(['[', ']'])
            .eq_ignore_ascii_case("vlan+srcmac")
    }) {
        return Some(
            "Bond transmit hash policy vlan+srcmac does not support native XDP.".to_string(),
        );
    }
    None
}

/// Represents a two-interface bridge configuration.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct BridgeConfig {
    /// Use the XDP-accelerated bridge?
    pub use_xdp_bridge: bool,

    /// The name of the first interface, facing the Internet. XDP mode also
    /// accepts a bond master when the kernel and member drivers support native XDP.
    pub to_internet: String,

    /// The name of the second interface, facing the LAN. XDP mode also accepts
    /// a bond master when the kernel and member drivers support native XDP.
    pub to_network: String,

    /// Optional MTU for LibreQoS-managed Linux bridge interfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u32>,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            use_xdp_bridge: true,
            to_internet: "eth0".to_string(),
            to_network: "eth1".to_string(),
            mtu: None,
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

#[cfg(test)]
mod tests {
    use super::native_xdp_bond_issue;

    #[test]
    fn native_xdp_bond_settings_match_linux_support() {
        for mode in [
            None,
            Some("balance-rr"),
            Some("active-backup"),
            Some("balance-xor"),
            Some("802.3ad"),
            Some("0"),
            Some("1"),
            Some("2"),
            Some("4"),
        ] {
            assert_eq!(native_xdp_bond_issue(mode, Some("layer3+4")), None);
        }
        assert!(
            native_xdp_bond_issue(Some("balance-alb"), None)
                .is_some_and(|issue| issue.contains("balance-alb"))
        );
        assert!(
            native_xdp_bond_issue(Some("802.3ad"), Some("vlan+srcmac"))
                .is_some_and(|issue| issue.contains("vlan+srcmac"))
        );
    }
}

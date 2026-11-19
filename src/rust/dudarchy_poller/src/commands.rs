//! The closed RouterOS read-only command vocabulary: the names a document
//! may declare, the reply row shape, and the detail slot each command
//! shapes.
//!
//! This module is a leaf: it knows nothing about the document schema, the
//! engine, or the transport, so both sides import it instead of each other.
//! The protocol path a name runs, its row cap, and the login stay with the
//! transport that owns them.

use std::collections::BTreeMap;

use serde::Deserialize;

/// One named-command reply row: the `!re` sentence's key/value pairs, with
/// omitted and empty values absent.
pub type CommandRow = BTreeMap<String, String>;

/// The closed list of core-registered read-only RouterOS operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandName {
    /// `/system/identity/print`.
    SystemIdentity,
    /// `/system/routerboard/print`.
    SystemRouterboard,
    /// `/ip/neighbor/print`.
    IpNeighbor,
    /// `/interface/print`.
    Interfaces,
    /// `/ip/address/print`.
    IpAddress,
    /// `/ip/arp/print`.
    IpArp,
    /// `/interface/bridge/port/print`.
    BridgePort,
    /// `/interface/bridge/vlan/print`.
    BridgeVlan,
    /// `/interface/bridge/host/print`.
    BridgeHost,
    /// `/system/resource/print`.
    SystemResource,
    /// `/system/health/print`.
    SystemHealth,
    /// `/ip/firewall/nat/print`.
    IpFirewallNat,
    /// `/ip/dhcp-server/lease/print`.
    IpDhcpServerLease,
}

/// The device-detail slot a named command naturally shapes, when it shapes
/// one.
///
/// A surface-filtered run fills a slot with the command whose name shapes it
/// even when `maps.details` names a probe from another surface, and a
/// document may only name a command for the slot it shapes.
pub fn detail_slot(name: CommandName) -> Option<&'static str> {
    match name {
        CommandName::Interfaces => Some("interfaces"),
        CommandName::SystemResource => Some("system"),
        CommandName::SystemHealth => Some("temperatures"),
        CommandName::IpArp => Some("arp"),
        CommandName::BridgePort => Some("bridge_ports"),
        CommandName::BridgeVlan => Some("bridge_vlans"),
        CommandName::BridgeHost => Some("bridge_hosts"),
        CommandName::IpFirewallNat => Some("nat"),
        CommandName::IpDhcpServerLease => Some("dhcp_leases"),
        CommandName::SystemIdentity
        | CommandName::SystemRouterboard
        | CommandName::IpNeighbor
        | CommandName::IpAddress => None,
    }
}

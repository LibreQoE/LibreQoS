//! Shared stored-inventory DTOs: the server's persisted host/link projection
//! and the evidence tables it carries. This module is unconditional so the
//! poller library (ports, walk) can use the same types without the server.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::identity::{Device, DeviceDetails};
use crate::walk::WalkNode;

/// One known host: the server-owned projection over [`Device`] — identity
/// claims plus provenance and timestamps, and nothing else. This is the
/// unit the dump persists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredHost {
    /// The merge key, normalized MAC text.
    pub mac: String,
    /// The best device name, when any claim named one.
    pub name: Option<String>,
    /// The best address, when any claim gave one.
    pub address: Option<String>,
    /// Which feed family or families reported the device.
    pub sources: String,
    /// Every identity claim the feeds made about the host.
    pub claims: Vec<StoredClaim>,
    /// When the host first entered the store.
    pub first_seen: SystemTime,
    /// When the host was last refreshed.
    pub last_seen: SystemTime,
    /// The latest successful details snapshot, when any query answered;
    /// a failed query never removes an earlier one.
    pub details: Option<StoredDetails>,
}

/// One stored interface: the device's own view of an interface, as
/// display text plus the queries that reported it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredInterface {
    /// The interface name.
    pub name: Option<String>,
    /// The normalized kind.
    pub kind: String,
    /// The interface's own MAC, when reported.
    pub mac: Option<String>,
    /// The bound addresses, in device order.
    pub addresses: Vec<String>,
    /// The administrative state, when reported.
    pub admin: Option<String>,
    /// The operational state, when reported.
    pub oper: Option<String>,
    /// The VLAN id, when carried.
    pub vlan_id: Option<u32>,
    /// The parent interface, for VLAN sub-interfaces.
    pub parent: Option<String>,
    /// The queries that contributed, as display text.
    pub sources: Vec<String>,
}

/// The device's self-reported system facts, as display text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredSystem {
    /// RouterOS version, when reported.
    pub version: Option<String>,
    /// Board name, when reported.
    pub board: Option<String>,
    /// Platform, when reported.
    pub platform: Option<String>,
    /// Uptime, when reported.
    pub uptime: Option<String>,
    /// The queries that produced the facts.
    pub sources: Vec<String>,
}

/// One stored temperature reading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredTemperature {
    /// The sensor name.
    pub sensor: String,
    /// The reported value in Celsius.
    pub celsius: f64,
    /// The queries that produced the reading.
    pub sources: Vec<String>,
}

/// One stored wireless station, from the access point's association
/// table: the latest successful snapshot, never unioned.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredStation {
    /// The station's wireless MAC.
    pub mac: String,
    /// The station's hostname, when the AP reports one.
    pub hostname: Option<String>,
    /// Signal strength, dBm.
    pub signal_dbm: Option<i64>,
    /// The station's last-seen IPv4 address.
    pub last_ip: Option<String>,
}

/// One stored ARP binding, as the device reported it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredArp {
    /// The bound IP address.
    pub address: Option<String>,
    /// The learned MAC, normalized; absent while the entry is
    /// incomplete.
    pub mac: Option<String>,
    /// The interface the binding sits on.
    pub interface: Option<String>,
    /// The `dynamic` flag, when reported.
    pub dynamic: Option<bool>,
    /// The `complete` flag, when reported.
    pub complete: Option<bool>,
    /// The `disabled` flag, when reported.
    pub disabled: Option<bool>,
}

/// One stored bridge-port membership.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredBridgePort {
    /// The bridge the port belongs to.
    pub bridge: Option<String>,
    /// The member interface's name.
    pub interface: Option<String>,
    /// The port's VLAN id (the untagged VLAN), when reported.
    pub pvid: Option<u32>,
    /// The `frame-types` setting, as the router spells it.
    pub frame_types: Option<String>,
    /// The router's raw port status, when it reports one.
    pub status: Option<String>,
    /// The `inactive` flag, when reported.
    pub inactive: Option<bool>,
    /// The `disabled` flag, when reported.
    pub disabled: Option<bool>,
    /// The `dynamic` flag, when reported.
    pub dynamic: Option<bool>,
}

/// One stored bridge VLAN configuration row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredBridgeVlan {
    /// The bridge the row configures.
    pub bridge: Option<String>,
    /// The configured VLAN ids, in router order.
    pub vlan_ids: Vec<u32>,
    /// The configured tagged interfaces.
    pub tagged: Vec<String>,
    /// The configured untagged interfaces.
    pub untagged: Vec<String>,
    /// The tagged interfaces currently in use.
    pub current_tagged: Vec<String>,
    /// The untagged interfaces currently in use.
    pub current_untagged: Vec<String>,
}

/// One stored bridge host: a MAC the bridge has learned, and where.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredBridgeHost {
    /// The bridge the host was learned on.
    pub bridge: Option<String>,
    /// The interface the host was seen on (the port).
    pub on_interface: Option<String>,
    /// The learned MAC, normalized.
    pub mac: Option<String>,
    /// How long since the entry was last seen, when the router reports
    /// it.
    pub age: Option<String>,
    /// The `dynamic` flag, when reported.
    pub dynamic: Option<bool>,
    /// The `local` flag, when reported.
    pub local: Option<bool>,
    /// The `external` flag, when reported.
    pub external: Option<bool>,
}

/// One stored firewall NAT rule: raw configuration evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredNatRule {
    /// The `/ip/firewall/nat` chain the rule belongs to.
    pub chain: Option<String>,
    /// The rule's action (`masquerade`, `src-nat`, `dst-nat`, …).
    pub action: Option<String>,
    /// The interface traffic must arrive on.
    pub in_interface: Option<String>,
    /// The interface traffic must leave through.
    pub out_interface: Option<String>,
    /// The source address, prefix, list, or range as configured.
    pub src_address: Option<String>,
    /// The destination address, prefix, list, or range as configured.
    pub dst_address: Option<String>,
    /// The translated address or range as configured.
    pub to_addresses: Option<String>,
    /// The `disabled` flag: the rule is configured but not active.
    pub disabled: Option<bool>,
    /// The rule's comment, verbatim.
    pub comment: Option<String>,
}

/// One stored DHCP server lease: the binding a device's DHCP server
/// handed a client. Address-binding evidence at the observer — a lease
/// row is evidence, not itself a claim; the store derives one
/// name/address claim per field it fills. It is never a node, a probe
/// target, or an adjacency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredDhcpLease {
    /// The lease address, gated and canonicalized by the walk.
    pub address: Option<String>,
    /// The client's MAC, normalized when it parses.
    pub mac: Option<String>,
    /// The client hostname the lease carries, verbatim.
    pub hostname: Option<String>,
    /// The router's raw lease status (`bound`, `waiting`, …).
    pub status: Option<String>,
    /// The `dynamic` flag: the lease was handed out, not configured.
    pub dynamic: Option<bool>,
    /// The `disabled` flag: the lease is configured but not active.
    pub disabled: Option<bool>,
}

/// One stored evidence table: the rows one query returned, the query,
/// and when it was observed. An empty `rows` is a successful empty
/// answer — distinct from the detail field being `None`, which is a
/// failed or unattempted query.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredSnapshot<T> {
    /// The rows, in device order.
    pub rows: Vec<T>,
    /// The queries that produced the table.
    pub sources: Vec<String>,
    /// When the live source observed the table.
    pub observed_at: SystemTime,
}

/// One details snapshot for a host: the device's own view of its
/// interfaces, system facts, temperatures, associated stations, and the
/// ARP/bridge evidence tables — credential-free, with the queries that
/// reported each part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredDetails {
    /// The interfaces, in the device's own row order.
    pub interfaces: Vec<StoredInterface>,
    /// The system facts, when the resource print answered.
    pub system: Option<StoredSystem>,
    /// Every temperature the board reports.
    pub temperatures: Vec<StoredTemperature>,
    /// The associated wireless stations.
    pub stations: Vec<StoredStation>,
    /// The ARP bindings, when the ARP query answered — `None` is a
    /// failed or unattempted query, an empty snapshot a successful
    /// empty table. Defaulted so an older dump without the field
    /// loads.
    #[serde(default)]
    pub arp: Option<StoredSnapshot<StoredArp>>,
    /// The bridge-port memberships, when the query was attempted and
    /// answered.
    #[serde(default)]
    pub bridge_ports: Option<StoredSnapshot<StoredBridgePort>>,
    /// The bridge VLAN configuration, when the query was attempted and
    /// answered.
    #[serde(default)]
    pub bridge_vlans: Option<StoredSnapshot<StoredBridgeVlan>>,
    /// The bridge's learned hosts, when the query was attempted and
    /// answered.
    #[serde(default)]
    pub bridge_hosts: Option<StoredSnapshot<StoredBridgeHost>>,
    /// The firewall NAT rules, when the query was attempted and answered.
    /// Defaulted so an older dump without the field loads.
    #[serde(default)]
    pub nat: Option<StoredSnapshot<StoredNatRule>>,
    /// The DHCP server leases, when the query was attempted and
    /// answered. Defaulted so an older dump without the field loads.
    #[serde(default)]
    pub dhcp_leases: Option<StoredSnapshot<StoredDhcpLease>>,
    /// When this snapshot first entered the store.
    pub first_seen: SystemTime,
    /// When this snapshot was last refreshed.
    pub last_seen: SystemTime,
}

/// One stored directional link: the observer evidence the walk recorded,
/// keyed by every evidence field — different ports stay separate
/// observations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredLink {
    /// The observer-side endpoint: a MAC when the walker knew one, else
    /// the endpoint's address; `None` when it was not identified.
    pub from: Option<String>,
    /// Whether `from` carries a MAC.
    pub from_is_mac: bool,
    /// The neighbour's normalized MAC.
    pub to: String,
    /// The observer-side port, when this probe attributed one.
    pub local_port: Option<String>,
    /// The feed that named the observer-side port.
    pub local_port_source: Option<String>,
    /// The neighbour's own port id, when reported.
    pub remote_port: Option<String>,
    /// Which feed(s) reported the neighbour in this probe.
    pub sources: String,
    /// Whether this is a wireless association observation.
    pub wireless: bool,
    /// When the link first entered the store.
    pub first_seen: SystemTime,
    /// When the link was last refreshed.
    pub last_seen: SystemTime,
    /// The discovery job that last refreshed the link.
    pub job: u64,
}

/// One stored identity claim: the claim's kind, value, and provenance as
/// display text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredClaim {
    /// The kind of fact ("serial", "address", "name", ...).
    pub kind: String,
    /// The stated value.
    pub value: String,
    /// The query that stated it ("snmp-lldp", "routeros-identity", ...).
    pub source: String,
}

impl StoredHost {
    /// Projects a discovered device into the store, stamped at `seen`.
    pub fn from_device(device: &Device, seen: SystemTime) -> Self {
        Self {
            mac: device.mac.as_str().to_owned(),
            name: device.name(),
            address: device.address(),
            sources: device.sources().to_string(),
            claims: device
                .claims()
                .iter()
                .map(|claim| StoredClaim {
                    kind: claim.kind.to_string(),
                    value: claim.value.clone(),
                    source: claim.source.to_string(),
                })
                .collect(),
            first_seen: seen,
            last_seen: seen,
            details: None,
        }
    }

    /// Projects a walk node into the store, stamped at `seen`. Nodes the
    /// walk has not tied to a MAC yet (the seed before any neighbour
    /// reports it) cannot be joined across runs and yield `None`. The
    /// node's details snapshot, when the probe answered, rides along.
    pub fn from_node(node: &WalkNode, seen: SystemTime) -> Option<Self> {
        Some(Self {
            mac: node.mac.clone()?,
            name: node.name.clone(),
            address: node.address.clone(),
            sources: node.source.to_string(),
            claims: node
                .claims
                .iter()
                .map(|claim| StoredClaim {
                    kind: claim.kind.to_string(),
                    value: claim.value.clone(),
                    source: claim.source.to_string(),
                })
                .collect(),
            first_seen: seen,
            last_seen: seen,
            details: node
                .details
                .as_ref()
                .map(|details| Self::project_details(details, seen)),
        })
    }

    /// Projects the device's collected details, stamped at `seen`.
    pub(crate) fn project_details(details: &DeviceDetails, seen: SystemTime) -> StoredDetails {
        StoredDetails {
            interfaces: details
                .interfaces
                .iter()
                .map(|row| StoredInterface {
                    name: row.name.clone(),
                    kind: row.kind.to_string(),
                    mac: row.mac.as_ref().map(|mac| mac.to_string()),
                    addresses: row.addresses.clone(),
                    admin: row.admin_state.map(|state| state.to_string()),
                    oper: row.oper_state.map(|state| state.to_string()),
                    vlan_id: row.vlan_id,
                    parent: row.parent_interface.clone(),
                    sources: row
                        .provenance
                        .iter()
                        .map(|source| source.to_string())
                        .collect(),
                })
                .collect(),
            system: details.system.as_ref().map(|system| StoredSystem {
                version: system.version.clone(),
                board: system.board_name.clone(),
                platform: system.platform.clone(),
                uptime: system.uptime.clone(),
                sources: system
                    .provenance
                    .iter()
                    .map(|source| source.to_string())
                    .collect(),
            }),
            temperatures: details
                .temperatures
                .iter()
                .map(|reading| StoredTemperature {
                    sensor: reading.sensor_name.clone(),
                    celsius: reading.celsius,
                    sources: reading
                        .provenance
                        .iter()
                        .map(|source| source.to_string())
                        .collect(),
                })
                .collect(),
            stations: details
                .stations
                .iter()
                .map(|station| StoredStation {
                    mac: station.mac.to_string(),
                    hostname: station.hostname.clone(),
                    signal_dbm: station.signal_dbm,
                    last_ip: station.last_ip.clone(),
                })
                .collect(),
            arp: project_snapshot(&details.arp, |row| StoredArp {
                address: row.address.clone(),
                mac: row.mac_address.as_ref().map(|mac| mac.to_string()),
                interface: row.interface.clone(),
                dynamic: row.dynamic,
                complete: row.complete,
                disabled: row.disabled,
            }),
            bridge_ports: project_snapshot(&details.bridge_ports, |row| StoredBridgePort {
                bridge: row.bridge.clone(),
                interface: row.interface.clone(),
                pvid: row.pvid,
                frame_types: row.frame_types.clone(),
                status: row.status.clone(),
                inactive: row.inactive,
                disabled: row.disabled,
                dynamic: row.dynamic,
            }),
            bridge_vlans: project_snapshot(&details.bridge_vlans, |row| StoredBridgeVlan {
                bridge: row.bridge.clone(),
                vlan_ids: row.vlan_ids.clone(),
                tagged: row.tagged.clone(),
                untagged: row.untagged.clone(),
                current_tagged: row.current_tagged.clone(),
                current_untagged: row.current_untagged.clone(),
            }),
            bridge_hosts: project_snapshot(&details.bridge_hosts, |row| StoredBridgeHost {
                bridge: row.bridge.clone(),
                on_interface: row.on_interface.clone(),
                mac: row.mac_address.as_ref().map(|mac| mac.to_string()),
                age: row.age.clone(),
                dynamic: row.dynamic,
                local: row.local,
                external: row.external,
            }),
            nat: project_snapshot(&details.nat, |row| StoredNatRule {
                chain: row.chain.clone(),
                action: row.action.clone(),
                in_interface: row.in_interface.clone(),
                out_interface: row.out_interface.clone(),
                src_address: row.src_address.clone(),
                dst_address: row.dst_address.clone(),
                to_addresses: row.to_addresses.clone(),
                disabled: row.disabled,
                comment: row.comment.clone(),
            }),
            dhcp_leases: project_snapshot(&details.dhcp_leases, |row| StoredDhcpLease {
                address: row.address.clone(),
                mac: row.mac.as_ref().map(|mac| mac.to_string()),
                hostname: row.hostname.clone(),
                status: row.status.clone(),
                dynamic: row.dynamic,
                disabled: row.disabled,
            }),
            first_seen: seen,
            last_seen: seen,
        }
    }
}

/// Projects one evidence snapshot for the dump: the rows through
/// `convert`, the query names as display text, and the observation time
/// unchanged. `None` stays `None` — unavailability survives the
/// projection.
fn project_snapshot<R, S>(
    snapshot: &Option<crate::identity::TableSnapshot<R>>,
    convert: impl Fn(&R) -> S,
) -> Option<StoredSnapshot<S>> {
    snapshot.as_ref().map(|table| StoredSnapshot {
        rows: table.rows.iter().map(convert).collect(),
        sources: table
            .provenance
            .iter()
            .map(|source| source.to_string())
            .collect(),
        observed_at: table.observed_at,
    })
}

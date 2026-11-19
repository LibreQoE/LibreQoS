//! Identity model for discovery: every neighbour feed's statement about a
//! device is a provenance-tagged [`Claim`] (kind, value, source), and
//! [`resolve`] groups both feeds' rows into per-MAC [`Device`]s.
//!
//! One grouping implementation lives here: [`resolve`] is the only grouping,
//! and no merge view derives from it. Devices keep every distinct claim —
//! identical claims (kind, value, and source all equal) deduplicate, the
//! same value from both feeds stays twice so the provenance survives.
//! Claims keep feed-then-row order (the API feed first, rows in feed order,
//! kinds in declaration order within a row), which is what pins
//! [`Device::name`] and [`Device::address`] to the historical preferences:
//! the first API claim wins, the first LLDP claim fills in.
//! [`Device::conflicts`] surfaces the singleton identity kinds — serial, name,
//! and sysdescr — whose claims disagree so nothing is silently picked.
//! Addresses, peer port ids, observer-side sighting context, and the derived
//! vendor hint are legitimately multi-valued, so their disagreement says
//! nothing about identity. [`resolve`] then appends one derived vendor hint
//! claim (kind [`ClaimKind::Vendor`], source [`Source::OuiRegistry`]) after
//! every feed claim: weakest in the strength order, so it can never change a
//! device's name, address, or grouping. Pure and deterministic: no I/O, no
//! clock, devices sorted by MAC.

use std::collections::BTreeMap;
use std::fmt;
use std::time::SystemTime;

use serde::Serialize;

use crate::mikrotik::{ApiNeighbour, RouterOsAddress, RouterOsInterface};
use crate::snmp::{LldpNeighbour, SnmpFdb, SnmpInterface};

/// A normalized MAC address: lowercase, colon-separated hex pairs.
///
/// Parsing accepts the common separators (`:`, `-`, `.`) and any case, and
/// requires exactly 12 hex digits once they are stripped; anything else is
/// not a MAC. `Display` reproduces the normalized form, so parsing and
/// formatting round-trip.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mac {
    text: String,
    /// The first octet, kept for U/L classification. Equal texts imply
    /// equal octets, so the derived comparisons stay effectively keyed on
    /// the text.
    first_octet: u8,
}

impl Mac {
    /// Parses MAC text in any common shape; `None` when the text is not a
    /// MAC.
    pub fn parse(text: &str) -> Option<Self> {
        let hex: String = text
            .chars()
            .filter(|ch| !matches!(ch, ':' | '-' | '.'))
            .map(|ch| ch.to_ascii_lowercase())
            .collect();
        if hex.len() != 12 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        // Cannot fail: the prefix is validated hex.
        let first_octet = u8::from_str_radix(&hex[0..2], 16).unwrap_or_default();
        Some(Self {
            text: (0..12)
                .step_by(2)
                .map(|at| &hex[at..at + 2])
                .collect::<Vec<_>>()
                .join(":"),
            first_octet,
        })
    }

    /// The MAC a raw LLDP chassis id advertises: six bytes are the raw MAC
    /// and format directly; a text chassis id (RouterOS sends the MAC
    /// pre-formatted) parses as MAC text; any other shape has none.
    pub fn from_chassis(bytes: &[u8]) -> Option<Self> {
        if let [a, b, c, d, e, f] = bytes {
            return Some(Self {
                text: format!("{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{f:02x}"),
                first_octet: *a,
            });
        }
        Self::parse(&String::from_utf8_lossy(bytes))
    }

    /// The normalized form.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// How this MAC classifies per the U/L bit of its first octet:
    /// universally administered (an allocated manufacturer block) or
    /// locally administered (software-set, virtualized, or randomized).
    pub fn class(&self) -> MacClass {
        if self.first_octet & 0x02 == 0 {
            MacClass::Universal
        } else {
            MacClass::Local
        }
    }

    /// Whether the MAC can name an interface owner: not the zero MAC, not
    /// broadcast, and not multicast (low bit of the first octet set).
    pub fn is_unicast(&self) -> bool {
        self.first_octet & 0x01 == 0 && self.text != "00:00:00:00:00:00"
    }

    /// Whether the address names a group rather than one interface: the
    /// I/G bit (the low bit of the first octet) is set. A group address is
    /// never a vendor assignment.
    pub fn is_group(&self) -> bool {
        self.first_octet & 0x01 != 0
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// How the U/L bit classifies a MAC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MacClass {
    /// Universally administered: an allocated manufacturer (OUI) block.
    Universal,
    /// Locally administered: software-set, virtualized, or randomized.
    Local,
}

impl fmt::Display for MacClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Universal => "universal",
            Self::Local => "local",
        })
    }
}

/// Which discovery feed reported a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sources {
    /// Only the LLDP table reported the MAC.
    Lldp,
    /// Only the RouterOS neighbour table reported the MAC.
    Api,
    /// Both feeds reported the MAC.
    Both,
}

impl fmt::Display for Sources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Sources::Lldp => "lldp",
            Sources::Api => "api",
            Sources::Both => "both",
        })
    }
}

/// The concrete management query that produced a claim or detail row:
/// one variant per query, no aliases, so provenance always names the
/// surface that answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// The LLDP remote-systems table, over SNMP.
    SnmpLldp,
    /// The SNMP system group (sysName, sysDescr).
    SnmpSystem,
    /// The RouterOS `/system/identity/print` query.
    RouterOsIdentity,
    /// The RouterOS `/system/routerboard/print` query.
    RouterOsSerial,
    /// The RouterOS `/ip/neighbor/print` table, over the binary API.
    RouterOsNeighbors,
    /// The RouterOS `/interface/print` query.
    RouterOsInterfaces,
    /// The RouterOS `/ip/address/print` query.
    RouterOsAddresses,
    /// The RouterOS `/interface/vlan/print` query.
    RouterOsVlans,
    /// The RouterOS `/ip/arp/print` query.
    RouterOsArp,
    /// The RouterOS `/interface/bridge/port/print` query.
    RouterOsBridgePorts,
    /// The RouterOS `/interface/bridge/vlan/print` query.
    RouterOsBridgeVlans,
    /// The RouterOS `/interface/bridge/host/print` query.
    RouterOsBridgeHosts,
    /// The RouterOS `/system/resource/print` query.
    RouterOsSystem,
    /// The RouterOS `/system/health/print` query.
    RouterOsHealth,
    /// The RouterOS `/ip/firewall/nat/print` query.
    RouterOsFirewallNat,
    /// The RouterOS `/ip/dhcp-server/lease/print` query.
    RouterOsDhcpLease,
    /// The SNMP `ifTable`/`ifXTable` interface columns.
    SnmpInterfaces,
    /// The airOS associated-station table (wireless clients).
    AirOsStations,
    /// A seed's ARP tables (IP-to-MAC bindings).
    SnmpArp,
    /// The SNMP `ipAddrTable` (IPv4 addresses bound to interfaces).
    SnmpIpAddresses,
    /// The SNMP bridge forwarding database: the MACs the bridge has
    /// learned, and on which port.
    SnmpFdb,
    /// A device's vendor-tree SNMP probes: system facts and sensor
    /// readings a document reads from an enterprise MIB rather than from
    /// MIB-II.
    SnmpVendor,
    /// The compiled-in IEEE OUI registry table: the derived vendor hint
    /// for a MAC, never a statement the device made.
    OuiRegistry,
    /// The operator's manual-device record: a human's statement about a
    /// device, never a statement the network made.
    Manual,
}

impl Source {
    /// The neighbour feed family a source belongs to, the classification
    /// [`Sources`] tracks per device: the SNMP feeds are the LLDP side, the
    /// RouterOS queries the API side. Sources that are not a neighbour feed —
    /// the derived vendor hint and the DHCP lease table, which is
    /// address-binding evidence — belong to neither.
    fn feed(self) -> Option<Sources> {
        match self {
            Self::SnmpLldp
            | Self::SnmpSystem
            | Self::SnmpInterfaces
            | Self::AirOsStations
            | Self::SnmpArp
            | Self::SnmpIpAddresses
            | Self::SnmpFdb
            | Self::SnmpVendor => Some(Sources::Lldp),
            Self::RouterOsIdentity
            | Self::RouterOsSerial
            | Self::RouterOsNeighbors
            | Self::RouterOsInterfaces
            | Self::RouterOsAddresses
            | Self::RouterOsVlans
            | Self::RouterOsArp
            | Self::RouterOsBridgePorts
            | Self::RouterOsBridgeVlans
            | Self::RouterOsBridgeHosts
            | Self::RouterOsSystem
            | Self::RouterOsHealth
            | Self::RouterOsFirewallNat => Some(Sources::Api),
            Self::RouterOsDhcpLease | Self::OuiRegistry | Self::Manual => None,
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SnmpLldp => "snmp-lldp",
            Self::SnmpSystem => "snmp-system",
            Self::RouterOsIdentity => "routeros-identity",
            Self::RouterOsSerial => "routeros-serial",
            Self::RouterOsNeighbors => "routeros-neighbors",
            Self::RouterOsInterfaces => "routeros-interfaces",
            Self::RouterOsAddresses => "routeros-addresses",
            Self::RouterOsVlans => "routeros-vlans",
            Self::RouterOsArp => "routeros-arp",
            Self::RouterOsBridgePorts => "routeros-bridge-ports",
            Self::RouterOsBridgeVlans => "routeros-bridge-vlans",
            Self::RouterOsBridgeHosts => "routeros-bridge-hosts",
            Self::RouterOsSystem => "routeros-system",
            Self::RouterOsHealth => "routeros-health",
            Self::RouterOsFirewallNat => "routeros-firewall-nat",
            Self::RouterOsDhcpLease => "routeros-dhcp-lease",
            Self::SnmpInterfaces => "snmp-interfaces",
            Self::AirOsStations => "airos-stations",
            Self::SnmpArp => "snmp-arp",
            Self::SnmpIpAddresses => "snmp-ip-addresses",
            Self::SnmpFdb => "snmp-fdb",
            Self::SnmpVendor => "snmp-vendor",
            Self::OuiRegistry => "ieee-oui",
            Self::Manual => "manual",
        })
    }
}

/// The identity form of a reported address: the address without its
/// prefix length (`192.168.88.1/24` → `192.168.88.1`), so the address
/// claims and the host list spell one address the same way however the
/// device wrote it. `None` when the row states no usable text — a claim
/// value is never empty.
pub fn bare_address(text: &str) -> Option<&str> {
    let address = match text.split_once('/') {
        Some((address, _prefix)) => address,
        None => text,
    };
    (!address.is_empty()).then_some(address)
}

/// What kind of fact a claim states. Declaration order is strength order:
/// merge proposals rank shared serials strongest, and the derived vendor
/// hint — appended by [`resolve`] after every feed claim — is weakest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClaimKind {
    /// The RouterOS routerboard serial number, self-reported over the API.
    Serial,
    /// An IP address: the API `address` or the LLDP management address.
    Address,
    /// A device name: the RouterOS identity or the LLDP sysName.
    Name,
    /// The LLDP system description.
    SysDescr,
    /// The LLDP remote port id.
    PortId,
    /// The LLDP observer-local port (`lldpRemLocalPortNum`), resolved to a
    /// name when the reporting device's own local-port table allowed.
    LocalPort,
    /// The local interface an API neighbour was seen on.
    Interface,
    /// The IEEE OUI registry hint for the device's MAC. Derived, never
    /// reported by a device; it names a vendor block, not a product.
    Vendor,
}

impl fmt::Display for ClaimKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Serial => "serial",
            Self::Address => "address",
            Self::Name => "name",
            Self::SysDescr => "sysdescr",
            Self::PortId => "portid",
            Self::LocalPort => "localport",
            Self::Interface => "interface",
            Self::Vendor => "vendor",
        })
    }
}

impl ClaimKind {
    /// Whether a differing value of this kind is an identity disagreement:
    /// only serial, name, and sysdescr are facts a device reports once.
    /// Addresses and peer port ids are multi-valued, localport and interface
    /// are observer-side sighting context, and the vendor hint is derived
    /// from the MAC — none of them contradict a device's identity.
    pub fn is_singleton_identity(self) -> bool {
        matches!(self, Self::Serial | Self::Name | Self::SysDescr)
    }

    /// The kind a stored claim-kind token names — the spellings [`Display`]
    /// writes, so a stored claim round-trips. `None` for any other token: an
    /// unknown stored kind gains no identity semantics by accident.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "serial" => Self::Serial,
            "address" => Self::Address,
            "name" => Self::Name,
            "sysdescr" => Self::SysDescr,
            "portid" => Self::PortId,
            "localport" => Self::LocalPort,
            "interface" => Self::Interface,
            "vendor" => Self::Vendor,
            _ => return None,
        })
    }
}

/// One feed's statement about a device: what kind of fact, the value, and
/// the concrete query that stated it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    /// The kind of fact.
    pub kind: ClaimKind,
    /// The stated value, never empty.
    pub value: String,
    /// The query that stated it.
    pub source: Source,
}

/// One device: a MAC with every distinct claim the feeds made about it.
#[derive(Debug)]
pub struct Device {
    /// The merge key both feeds share.
    pub mac: Mac,
    claims: Vec<Claim>,
    sources: Sources,
}

impl Device {
    /// The device's claims, deduplicated, in feed-then-row order: API-feed
    /// claims first (rows in feed order), then LLDP-feed claims; within a
    /// row, kinds in declaration order.
    pub fn claims(&self) -> &[Claim] {
        &self.claims
    }

    /// Which feed(s) reported this device.
    pub fn sources(&self) -> Sources {
        self.sources
    }

    /// Best name, exactly the historical preference: the first API
    /// identity claim, else the first LLDP sysName claim.
    pub fn name(&self) -> Option<String> {
        self.first(ClaimKind::Name).map(|claim| claim.value.clone())
    }

    /// Best address, exactly the historical preference: the first API
    /// address claim, else the first LLDP management-address claim.
    pub fn address(&self) -> Option<String> {
        self.first(ClaimKind::Address)
            .map(|claim| claim.value.clone())
    }

    /// Singleton identity kinds ([`ClaimKind::is_singleton_identity`]) whose
    /// claims carry more than one distinct value, in declaration order, each
    /// conflict listing all its claims — values and the feed that reported
    /// each — in claim order. Multi-valued facts never conflict here.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut kinds: Vec<ClaimKind> = self
            .claims
            .iter()
            .map(|claim| claim.kind)
            .filter(|kind| kind.is_singleton_identity())
            .collect();
        kinds.sort_unstable();
        kinds.dedup();
        kinds
            .into_iter()
            .filter_map(|kind| {
                let claims: Vec<Claim> = self
                    .claims
                    .iter()
                    .filter(|claim| claim.kind == kind)
                    .cloned()
                    .collect();
                let mut distinct: Vec<&str> = Vec::new();
                for claim in &claims {
                    if !distinct.contains(&claim.value.as_str()) {
                        distinct.push(&claim.value);
                    }
                }
                (distinct.len() > 1).then_some(Conflict { kind, claims })
            })
            .collect()
    }

    /// The first claim of `kind` in claim order; since the API feed's
    /// claims are recorded first, this is the preference rule.
    fn first(&self, kind: ClaimKind) -> Option<&Claim> {
        self.claims.iter().find(|claim| claim.kind == kind)
    }

    /// Records one row's non-empty fields as claims of `source`, skipping
    /// values the row left absent or empty, deduplicating identical
    /// claims, and folding `source` into the reported feeds.
    fn record(&mut self, source: Source, claims: &[(ClaimKind, Option<&str>)]) {
        if let Some(family) = source.feed() {
            self.sources = match (self.sources, family) {
                (Sources::Lldp, Sources::Api) | (Sources::Api, Sources::Lldp) => Sources::Both,
                (current, _) => current,
            };
        }
        for (kind, value) in claims {
            let Some(value) = value.filter(|value| !value.is_empty()) else {
                continue;
            };
            let claim = Claim {
                kind: *kind,
                value: value.to_owned(),
                source,
            };
            if !self.claims.contains(&claim) {
                self.claims.push(claim);
            }
        }
    }

    /// Appends the vendor hint derived from the device's own MAC: the
    /// weakest claim, after every feed claim, so name and address selection
    /// and conflict detection see the feeds first.
    fn record_vendor_hint(&mut self) {
        let hint = crate::oui::lookup(&self.mac);
        self.claims.push(Claim {
            kind: ClaimKind::Vendor,
            value: hint.claim_value(),
            source: Source::OuiRegistry,
        });
    }

    /// Starts a device reported first by `source`.
    fn new(mac: Mac, sources: Sources) -> Self {
        Self {
            mac,
            claims: Vec::new(),
            sources,
        }
    }

    /// Builds a device from its MAC and known claims — the injected-device
    /// entry point. Sources derive from the claims' own provenance (a
    /// claim from each feed marks the device `Both`); identical claims
    /// deduplicate; the given order is kept. The derived vendor hint is not
    /// appended here: the caller owns the claim list, and [`resolve`] adds
    /// the hint to the feed grouping. A device built with no claims carries
    /// none, and its sources value is then unused.
    pub fn from_claims(mac: Mac, claims: Vec<Claim>) -> Self {
        let mut sources: Option<Sources> = None;
        let mut deduped: Vec<Claim> = Vec::new();
        for claim in claims {
            if let Some(family) = claim.source.feed() {
                sources = Some(match sources {
                    None => family,
                    Some(current) if current != family => Sources::Both,
                    Some(current) => current,
                });
            }
            if !deduped.contains(&claim) {
                deduped.push(claim);
            }
        }
        Self {
            mac,
            claims: deduped,
            sources: sources.unwrap_or(Sources::Api),
        }
    }
}

/// One device block: the MAC header line, then one line per claim —
/// `kind value source`, indented — with ` (conflict)` on every claim whose
/// kind [`Device::conflicts`] lists. Claims appear in claim order; the
/// block ends with a newline.
impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.mac)?;
        let conflicted: Vec<ClaimKind> = self.conflicts().iter().map(|c| c.kind).collect();
        for claim in &self.claims {
            write!(f, "  {} {} {}", claim.kind, claim.value, claim.source)?;
            if conflicted.contains(&claim.kind) {
                write!(f, " (conflict)")?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

/// One identity kind whose claims disagree: every distinct value with the
/// feed that reported it.
#[derive(Debug)]
pub struct Conflict {
    /// The disagreeing kind.
    pub kind: ClaimKind,
    /// All claims of that kind, in claim order.
    pub claims: Vec<Claim>,
}

/// The normalized kind of an interface, derived from the device's raw
/// type string; [`InterfaceKind::Other`] covers every type the model does
/// not name, whose detail stays in [`Interface::raw_type`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceKind {
    /// A physical Ethernet port.
    Ethernet,
    /// A software bridge.
    Bridge,
    /// A VLAN sub-interface.
    Vlan,
    /// A loopback interface.
    Loopback,
    /// An overlay/tunnel transport (EoIP, VPLS, VXLAN, WireGuard, GRE,
    /// L2TP, PPPoE, ...): a link observed through it crosses a tunnel
    /// and is never a physical edge.
    Tunnel,
    /// Anything else (tunnels the model does not name, bonds, pppoe, …)
    /// — unknown, never silently physical.
    Other,
}

/// RouterOS interface `type` strings that mark overlay/tunnel
/// transports: the field-observed families and their documented
/// endpoint variants.
const OVERLAY_TYPES: [&str; 20] = [
    "eoip",
    "vpls",
    "vxlan",
    "wg",
    "gre",
    "gre6",
    "ipip",
    "ipipv6",
    "l2tp-out",
    "l2tp-in",
    "pptp-out",
    "pptp-in",
    "ovpn-out",
    "ovpn-in",
    "sstp-out",
    "sstp-in",
    "pppoe-out",
    "pppoe-in",
    "6to4",
    "sit",
];

/// Interface NAME prefixes that suggest an overlay, the fallback used
/// only when an interface's kind is unknown; case-insensitive.
const OVERLAY_NAME_PREFIXES: [&str; 11] = [
    "eoip", "vpls", "vxlan", "wg", "gre", "ipip", "l2tp", "pptp", "ovpn", "sstp", "pppoe",
];

impl InterfaceKind {
    /// Classifies a device's raw type string, case-sensitively: the
    /// strings come from machine queries, not humans.
    fn of(raw_type: &str) -> Self {
        match raw_type {
            "ether" => Self::Ethernet,
            "bridge" => Self::Bridge,
            "vlan" => Self::Vlan,
            "loopback" => Self::Loopback,
            raw_type if OVERLAY_TYPES.contains(&raw_type) => Self::Tunnel,
            _ => Self::Other,
        }
    }

    /// Classifies an SNMP `ifType` code: the kinds the model names map to
    /// their standard codes, everything else stays [`InterfaceKind::Other`]
    /// with the code carried in [`Interface::raw_type`].
    fn from_if_type(code: Option<u32>) -> Self {
        match code {
            Some(6) => Self::Ethernet,  // ethernetCsmacd
            Some(24) => Self::Loopback, // softwareLoopback
            Some(131) => Self::Tunnel,  // tunnel
            Some(135) => Self::Vlan,    // l2vlan
            Some(209) => Self::Bridge,  // bridge
            _ => Self::Other,
        }
    }

    /// Whether the kind marks an overlay/tunnel transport: a link
    /// observed through it is never a physical edge.
    pub fn is_overlay(self) -> bool {
        matches!(self, Self::Tunnel)
    }

    /// The kind a stored interface-kind token names — the spellings
    /// [`Display`] writes, so a stored row round-trips. `None` for any other
    /// token: an unknown stored kind gains no semantics by accident, and only
    /// the name fallback can still say something about it.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "ethernet" => Self::Ethernet,
            "bridge" => Self::Bridge,
            "vlan" => Self::Vlan,
            "loopback" => Self::Loopback,
            "tunnel" => Self::Tunnel,
            "other" => Self::Other,
            _ => return None,
        })
    }

    /// Whether an interface NAME suggests an overlay — the fallback for
    /// ports whose kind is unknown: the case-insensitive family prefixes,
    /// plus the `tunnel` substring (HEipv6-Tunnel).
    pub fn name_suggests_overlay(name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        lower.contains("tunnel")
            || OVERLAY_NAME_PREFIXES
                .iter()
                .any(|prefix| lower.starts_with(prefix))
    }
}

impl fmt::Display for InterfaceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ethernet => "ethernet",
            Self::Bridge => "bridge",
            Self::Vlan => "vlan",
            Self::Loopback => "loopback",
            Self::Tunnel => "tunnel",
            Self::Other => "other",
        })
    }
}

/// An interface's state, as the device reports each side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfState {
    /// Administratively enabled, or operationally running.
    Up,
    /// Administratively disabled, or operationally down.
    Down,
}

impl fmt::Display for IfState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Up => "up",
            Self::Down => "down",
        })
    }
}

/// One interface a device reports about itself. Fields the device left
/// out — or a query that failed — stay absent; nothing is guessed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    /// The device's row id for the interface.
    pub id: String,
    /// The interface name, when the device gave one.
    pub name: Option<String>,
    /// The interface's own description, when the device gave one and it
    /// is a different column than the name.
    pub description: Option<String>,
    /// A free-form label, when the device carries one (the RouterOS
    /// comment, the SNMP `ifAlias`).
    pub alias: Option<String>,
    /// The normalized kind.
    pub kind: InterfaceKind,
    /// The device's own type string, when the kind is not enough.
    pub raw_type: Option<String>,
    /// The interface's own MAC, when the device reports one.
    pub mac: Option<Mac>,
    /// Addresses bound to the interface, in device order.
    pub addresses: Vec<String>,
    /// Administrative state (enabled/disabled), when reported.
    pub admin_state: Option<IfState>,
    /// Operational state (running/down), when reported.
    pub oper_state: Option<IfState>,
    /// VLAN id, when the device carries VLAN detail for the interface.
    pub vlan_id: Option<u32>,
    /// The parent interface's name, for VLAN sub-interfaces.
    pub parent_interface: Option<String>,
    /// The concrete queries that contributed to this row, in query order.
    pub provenance: Vec<Source>,
}

/// A device's self-reported system facts, from the resource print.
/// Fields the board left out stay absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemDetails {
    /// RouterOS version string, when reported.
    pub version: Option<String>,
    /// Board name, when reported.
    pub board_name: Option<String>,
    /// Platform, when reported.
    pub platform: Option<String>,
    /// Uptime as the router formats it, when reported.
    pub uptime: Option<String>,
    /// The concrete query that produced these facts.
    pub provenance: Vec<Source>,
}

impl SystemDetails {
    /// The facts one RouterOS `/system/resource/print` row reports. A field
    /// the router left out stays absent, and the provenance is the resource
    /// query.
    pub fn from_router_os(
        version: Option<String>,
        board_name: Option<String>,
        platform: Option<String>,
        uptime: Option<String>,
    ) -> Self {
        Self {
            version,
            board_name,
            platform,
            uptime,
            provenance: vec![Source::RouterOsSystem],
        }
    }
}

/// A device's self-reported radio operating parameters, from a scalars
/// probe. Fields the device left out stay absent; `phy_mbps` feeds the
/// capacity derivation and `model` keys the per-model trust factor.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioDetails {
    /// The device model the AP reports, when the family exposes one. The
    /// capacity trust factor is keyed on this string.
    pub model: Option<String>,
    /// The radio's centre frequency, MHz.
    pub frequency_mhz: Option<u32>,
    /// The configured channel width, MHz (the document decodes the
    /// vendor enum through the `channel_width_mhz` parser).
    pub channel_width_mhz: Option<u32>,
    /// Connected stations, as the AP counts them.
    pub sm_count: Option<u32>,
    /// Transmit power, dBm.
    pub tx_power_dbm: Option<i64>,
    /// The radio's LAN link speed, Mbps.
    pub lan_speed_mbps: Option<u64>,
    /// The radio's reported PHY rate, Mbps, when the family exposes one.
    /// Capacity is derived from it, never supplied as a document field.
    pub phy_mbps: Option<f64>,
    /// The concrete query that produced these facts.
    pub provenance: Vec<Source>,
}

/// One derived device capacity and the input that produced it. The
/// document never supplies `capacity_mbps`; the engine derives it from a
/// reported PHY rate, the station table's downlink rates, or a CRM rate,
/// in that precedence order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapacityDerivation {
    /// The derived capacity, when one could be computed.
    pub mbps: Option<f64>,
    /// Which input produced it.
    pub source: CapacitySource,
}

/// The input a derived capacity came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapacitySource {
    /// The radio's reported PHY rate times the capacity factor.
    Phy,
    /// The highest per-station downlink rate times the capacity factor.
    PerSmRate,
    /// The CRM's own rate, used unchanged.
    Crm,
    /// No input produced a capacity.
    Unknown,
}

impl fmt::Display for CapacitySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Phy => "phy",
            Self::PerSmRate => "per_sm_rate",
            Self::Crm => "crm",
            Self::Unknown => "unknown",
        })
    }
}

/// One temperature sensor a board reports. `observed_at` is stamped by
/// the live collector at query time and carried through unchanged — the
/// pure model never reads a clock.
#[derive(Clone, Debug, PartialEq)]
pub struct TemperatureReading {
    /// The sensor name, as the board spells the field.
    pub sensor_name: String,
    /// The reported value in Celsius.
    pub celsius: f64,
    /// The concrete query that produced the reading.
    pub provenance: Vec<Source>,
    /// When the live source observed the board.
    pub observed_at: SystemTime,
}

impl TemperatureReading {
    /// One RouterOS `/system/health/print` temperature row: the sensor label
    /// the document maps, its reading, and the observation stamp the live
    /// source took.
    pub fn from_router_os(sensor_name: String, celsius: f64, observed_at: SystemTime) -> Self {
        Self {
            sensor_name,
            celsius,
            provenance: vec![Source::RouterOsHealth],
            observed_at,
        }
    }
}

/// One wireless station associated with an access point, as the AP
/// reports it at collection time — an association snapshot, not a
/// continuous proof.
///
/// Converted from the collector's raw row; identity fields the AP did
/// not report stay absent.
#[derive(Clone, Debug, PartialEq)]
pub struct WirelessStation {
    /// The station's wireless MAC.
    pub mac: Mac,
    /// The WLAN interface index the AP's table carries; semantics
    /// unconfirmed (single-radio sectors answer 1).
    pub wlan_if: u32,
    /// The station's own hostname, when the AP reports one.
    pub hostname: Option<String>,
    /// Signal strength, dBm.
    pub signal_dbm: Option<i64>,
    /// Noise floor, dBm.
    pub noise_dbm: Option<i64>,
    /// The station's last-seen IPv4 address as text.
    pub last_ip: Option<String>,
    /// Transmit rate, bps.
    pub tx_bps: Option<u64>,
    /// Receive rate, bps.
    pub rx_bps: Option<u64>,
    /// Bytes the AP received from the station since association
    /// (Counter64).
    pub rx_bytes: Option<u64>,
    /// Bytes the AP transmitted to the station since association
    /// (Counter64).
    pub tx_bytes: Option<u64>,
    /// Association uptime (TimeTicks, hundredths of a second).
    pub uptime: Option<u64>,
    /// Station-table column 5 (distance), preserved raw; the document
    /// layer does not read it.
    pub distance_ccq: Option<i64>,
    /// Station-table column 6 (CCQ), preserved raw; the document layer
    /// does not read it.
    pub quality: Option<i64>,
    /// The station's radio channel, when the AP reports one.
    pub channel: Option<u32>,
    /// Uplink RSSI, dBm.
    pub ul_rssi: Option<i64>,
    /// Downlink RSSI, dBm.
    pub dl_rssi: Option<i64>,
    /// Uplink SNR, dB.
    pub ul_snr: Option<i64>,
    /// Downlink SNR, dB.
    pub dl_snr: Option<i64>,
    /// The AP's reported transmit capacity for the station (vendor units);
    /// the AP->station direction, so the downlink capacity.
    pub tx_capacity: Option<u64>,
    /// The AP's reported receive capacity for the station (vendor units);
    /// the station->AP direction, so the uplink capacity.
    pub rx_capacity: Option<u64>,
    /// The AP's reported transmit quality for the station (vendor units).
    pub tx_quality: Option<u64>,
    /// The station's downlink rate, Mbps, as the AP reports it.
    pub dl_rate_mbps: Option<u64>,
    /// The station's model string, when the AP reports one.
    pub model: Option<String>,
    /// The station's software version, when the AP reports one.
    pub software_version: Option<String>,
    /// The concrete query that produced the row.
    pub provenance: Vec<Source>,
}

impl WirelessStation {
    /// Converts a collector row into a model station; the MAC must be
    /// exactly six octets (the collector guarantees it).
    pub fn from_air_station(station: crate::snmp::AirStation, provenance: Vec<Source>) -> Self {
        Self {
            mac: Mac::from_chassis(&station.mac).expect("collector MAC is six octets"),
            wlan_if: station.wlan_if,
            hostname: station.hostname,
            signal_dbm: station.signal_dbm,
            noise_dbm: station.noise_dbm,
            last_ip: station.last_ip,
            tx_bps: station.tx_bps,
            rx_bps: station.rx_bps,
            rx_bytes: station.rx_bytes,
            tx_bytes: station.tx_bytes,
            uptime: station.uptime,
            distance_ccq: station.distance_ccq,
            quality: station.quality,
            channel: station.channel,
            ul_rssi: station.ul_rssi,
            dl_rssi: station.dl_rssi,
            ul_snr: station.ul_snr,
            dl_snr: station.dl_snr,
            tx_capacity: station.tx_capacity,
            rx_capacity: station.rx_capacity,
            tx_quality: station.tx_quality,
            dl_rate_mbps: station.dl_rate_mbps,
            model: station.model,
            software_version: station.software_version,
            provenance,
        }
    }
}

/// One ARP binding a device reports about itself (`/ip/arp/print`):
/// raw evidence rows, never identity claims — a binding is a dataplane
/// observation, not a statement about what a device is. Fields the row
/// left out stay absent; an incomplete entry (still resolving) carries
/// no MAC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArpEntry {
    /// The bound IP address.
    pub address: Option<String>,
    /// The learned MAC, normalized; absent while the entry is
    /// incomplete.
    pub mac_address: Option<Mac>,
    /// The interface the binding sits on.
    pub interface: Option<String>,
    /// The `dynamic` flag: learned by the dataplane, not configured.
    pub dynamic: Option<bool>,
    /// The `complete` flag: the router has resolved and still holds the
    /// MAC.
    pub complete: Option<bool>,
    /// The `disabled` flag: a statically configured, disabled binding.
    pub disabled: Option<bool>,
}

impl ArpEntry {
    /// Converts a collector row; a MAC the row carries but that does not
    /// parse stays absent — malformed MAC text is no evidence.
    pub fn from_router_os(row: &crate::mikrotik::RouterOsArp) -> Self {
        Self {
            address: row.address.clone(),
            mac_address: row.mac_address.as_deref().and_then(Mac::parse),
            interface: row.interface.clone(),
            dynamic: row.dynamic,
            complete: row.complete,
            disabled: row.disabled,
        }
    }
}

/// One bridge-port membership a device reports about itself
/// (`/interface/bridge/port/print`): which interface belongs to which
/// software bridge, with the port's settings as the router states them.
/// The flags stay separate — `disabled` is configuration, `inactive`
/// is the dataplane's verdict, and a `pvid` alone is no proof of VLAN
/// membership while the bridge does no VLAN filtering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgePort {
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
    /// The `inactive` flag: the port is not currently forwarding.
    pub inactive: Option<bool>,
    /// The `disabled` flag: administratively disabled.
    pub disabled: Option<bool>,
    /// The `dynamic` flag: created by the dataplane, not configured.
    pub dynamic: Option<bool>,
}

impl BridgePort {
    /// Converts a collector row.
    pub fn from_router_os(row: &crate::mikrotik::RouterOsBridgePort) -> Self {
        Self {
            bridge: row.bridge.clone(),
            interface: row.interface.clone(),
            pvid: row.pvid,
            frame_types: row.frame_types.clone(),
            status: row.status.clone(),
            inactive: row.inactive,
            disabled: row.disabled,
            dynamic: row.dynamic,
        }
    }
}

/// One bridge's VLAN configuration a device reports about itself
/// (`/interface/bridge/vlan/print`). The configured columns and the
/// `current-*` columns — what the bridge is actually using — are kept
/// side by side; the interface lists stay in router order with no
/// resolution attempted against the interface inventory here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeVlan {
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

impl BridgeVlan {
    /// Converts a collector row; a VLAN id the router sent but that
    /// does not parse is dropped, like the collector drops it.
    pub fn from_router_os(row: &crate::mikrotik::RouterOsBridgeVlan) -> Self {
        Self {
            bridge: row.bridge.clone(),
            vlan_ids: row.vlan_ids.clone(),
            tagged: row.tagged.clone(),
            untagged: row.untagged.clone(),
            current_tagged: row.current_tagged.clone(),
            current_untagged: row.current_untagged.clone(),
        }
    }
}

/// One learned host a device's bridge reports (`/interface/bridge/host
/// /print`): a MAC the bridge has seen, and where. Raw evidence — never
/// an adjacency edge and never an identity claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeHost {
    /// The bridge the host was learned on.
    pub bridge: Option<String>,
    /// The interface the host was seen on (the port).
    pub on_interface: Option<String>,
    /// The learned MAC, normalized.
    pub mac_address: Option<Mac>,
    /// How long since the entry was last seen, when the router reports
    /// it.
    pub age: Option<String>,
    /// The `dynamic` flag: learned by the dataplane, not configured.
    pub dynamic: Option<bool>,
    /// The `local` flag: one of the bridge's own MACs.
    pub local: Option<bool>,
    /// The `external` flag: learned from an external FDB source.
    pub external: Option<bool>,
}

impl BridgeHost {
    /// Converts a collector row; a MAC that does not parse stays
    /// absent.
    pub fn from_router_os(row: &crate::mikrotik::RouterOsBridgeHost) -> Self {
        Self {
            bridge: row.bridge.clone(),
            on_interface: row.on_interface.clone(),
            mac_address: row.mac_address.as_deref().and_then(Mac::parse),
            age: row.age.clone(),
            dynamic: row.dynamic,
            local: row.local,
            external: row.external,
        }
    }
}

/// One firewall NAT rule a device reports about itself
/// (`/ip/firewall/nat/print`): raw configuration evidence — never an
/// identity claim, an adjacency, or a probe target. Addresses, prefixes,
/// address-list names, ranges, and ports stay exactly as the router
/// spells them; this model parses no coverage from them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NatRule {
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

impl NatRule {
    /// Converts a collector row.
    pub fn from_router_os(row: &crate::mikrotik::RouterOsNatRule) -> Self {
        Self {
            chain: row.chain.clone(),
            action: row.action.clone(),
            in_interface: row.in_interface.clone(),
            out_interface: row.out_interface.clone(),
            src_address: row.src_address.clone(),
            dst_address: row.dst_address.clone(),
            to_addresses: row.to_addresses.clone(),
            disabled: row.disabled,
            comment: row.comment.clone(),
        }
    }
}

/// One DHCP server lease a device reports about itself
/// (`/ip/dhcp-server/lease/print`): the binding a DHCP server handed a
/// client. This is address-binding evidence at the observing device — a
/// lease row is evidence, not itself a claim; the store derives one
/// name/address claim per field it fills. It is never a node, a probe
/// target, or an adjacency. The address is the walk's gated, canonical
/// text; the hostname stays as the router spells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DhcpLease {
    /// The lease address, gated and canonicalized by the walk.
    pub address: Option<String>,
    /// The client's MAC, normalized when it parses.
    pub mac: Option<Mac>,
    /// The client hostname the lease carries, verbatim.
    pub hostname: Option<String>,
    /// The router's raw lease status (`bound`, `waiting`, …).
    pub status: Option<String>,
    /// The `dynamic` flag: the lease was handed out, not configured.
    pub dynamic: Option<bool>,
    /// The `disabled` flag: the lease is configured but not active.
    pub disabled: Option<bool>,
}

/// One table a device reported about itself in a single query: the rows
/// in device order, the query that answered, and when the live source
/// observed the table. `DeviceDetails` holds these as `Option`s —
/// `None` is a failed or unattempted query, `Some` with empty rows a
/// successful empty answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSnapshot<T> {
    /// The rows, in device order.
    pub rows: Vec<T>,
    /// The concrete query that produced the table.
    pub provenance: Vec<Source>,
    /// When the live source observed the table.
    pub observed_at: SystemTime,
}

/// A device's self-reported details: one row per interface the device
/// names, with addresses and VLAN detail folded in, plus the system
/// facts, the temperature sensors, and — on wireless access points — the
/// associated stations the device reports. Absent details mean the
/// queries did not answer — never an empty stand-in.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceDetails {
    /// The interfaces, in the device's own row order.
    pub interfaces: Vec<Interface>,
    /// The system facts, when the resource print answered.
    pub system: Option<SystemDetails>,
    /// Every temperature the board reports, in device field order.
    pub temperatures: Vec<TemperatureReading>,
    /// The associated wireless stations, in MAC order, when the device
    /// is an access point whose station table answered.
    pub stations: Vec<WirelessStation>,
    /// The radio's operating parameters, when a scalars probe answered.
    pub radio: Option<RadioDetails>,
    /// The ARP bindings, when the ARP query answered — `None` is a
    /// failed or unattempted query, an empty snapshot a successful
    /// empty table.
    pub arp: Option<TableSnapshot<ArpEntry>>,
    /// The bridge-port memberships, when the query was attempted and
    /// answered.
    pub bridge_ports: Option<TableSnapshot<BridgePort>>,
    /// The bridge VLAN configuration, when the query was attempted and
    /// answered.
    pub bridge_vlans: Option<TableSnapshot<BridgeVlan>>,
    /// The bridge's learned hosts, when the query was attempted and
    /// answered.
    pub bridge_hosts: Option<TableSnapshot<BridgeHost>>,
    /// The firewall NAT rules, when the query was attempted and
    /// answered — configuration evidence only, never a probe target.
    pub nat: Option<TableSnapshot<NatRule>>,
    /// The DHCP server leases, when the query was attempted and
    /// answered — address-binding evidence only, never a node, probe
    /// target, or adjacency.
    pub dhcp_leases: Option<TableSnapshot<DhcpLease>>,
}

impl DeviceDetails {
    /// Assembles details from the RouterOS `/interface/print` and
    /// `/ip/address/print` rows: the interface rows are the spine, and
    /// address rows attach by interface name. Rows naming an interface the
    /// spine does not have are dropped — the spine is what the device says
    /// about its interfaces, and an attachment without an interface has no
    /// home. The system facts and the temperature readings are separate
    /// queries, each with its own per-fact constructor
    /// ([`SystemDetails::from_router_os`], [`TemperatureReading::from_router_os`]),
    /// so this assembler leaves those slots empty.
    pub fn from_router_os(interfaces: &[RouterOsInterface], addresses: &[RouterOsAddress]) -> Self {
        let mut rows: Vec<Interface> =
            interfaces
                .iter()
                .map(|row| Interface {
                    id: row.id.clone(),
                    name: row.name.clone(),
                    description: None,
                    alias: None,
                    kind: row
                        .raw_type
                        .as_deref()
                        .map(InterfaceKind::of)
                        .unwrap_or(InterfaceKind::Other),
                    raw_type: row.raw_type.clone(),
                    mac: row.mac_address.as_deref().and_then(Mac::parse),
                    addresses: Vec::new(),
                    admin_state: row.disabled.map(|disabled| {
                        if disabled { IfState::Down } else { IfState::Up }
                    }),
                    oper_state: row.running.map(
                        |running| {
                            if running { IfState::Up } else { IfState::Down }
                        },
                    ),
                    vlan_id: None,
                    parent_interface: None,
                    provenance: vec![Source::RouterOsInterfaces],
                })
                .collect();
        for address in addresses {
            let Some(value) = address.address.as_deref() else {
                continue;
            };
            let Some(row) = rows
                .iter_mut()
                .find(|row| row.name.as_deref() == address.interface.as_deref())
            else {
                continue;
            };
            row.addresses.push(value.to_owned());
            row.provenance.push(Source::RouterOsAddresses);
        }
        // Same-source pushes are adjacent by construction, so adjacent
        // dedup is exact: provenance lists each contributing query once.
        for row in &mut rows {
            row.provenance.dedup();
        }
        Self {
            interfaces: rows,
            system: None,
            temperatures: Vec::new(),
            stations: Vec::new(),
            radio: None,
            arp: None,
            bridge_ports: None,
            bridge_vlans: None,
            bridge_hosts: None,
            nat: None,
            dhcp_leases: None,
        }
    }

    /// Assembles details from the SNMP interface columns: one row per
    /// ifIndex, with `ifName` preferred over `ifDescr` as the identifier,
    /// `ifDescr` kept verbatim as the description, and `ifAlias` as the
    /// alias label (recon d6j). The raw `ifType` code stays in
    /// `raw_type`; statuses outside the up/down codes stay unknown.
    pub fn from_snmp_interfaces(rows: &[SnmpInterface]) -> Self {
        Self {
            interfaces: rows
                .iter()
                .map(|row| Interface {
                    id: row.index.to_string(),
                    name: row.name.clone().or_else(|| row.descr.clone()),
                    description: row.descr.clone(),
                    alias: row.alias.clone(),
                    kind: InterfaceKind::from_if_type(row.raw_type),
                    raw_type: row.raw_type.map(|code| code.to_string()),
                    mac: row.phys_address.as_deref().and_then(Mac::from_chassis),
                    addresses: Vec::new(),
                    admin_state: row.admin_status.and_then(if_state),
                    oper_state: row.oper_status.and_then(if_state),
                    vlan_id: None,
                    parent_interface: None,
                    provenance: vec![Source::SnmpInterfaces],
                })
                .collect(),
            system: None,
            temperatures: Vec::new(),
            stations: Vec::new(),
            radio: None,
            arp: None,
            bridge_ports: None,
            bridge_vlans: None,
            bridge_hosts: None,
            nat: None,
            dhcp_leases: None,
        }
    }

    /// Attaches a bridge forwarding database as this device's learned-host
    /// evidence: each row's bridge port joins `fdb.port_if_index` to an
    /// `ifIndex`, and that index joins these interfaces by id to the
    /// interface name. A row with no port, no join entry, or no named
    /// interface is unjoinable and is skipped, never remapped onto a
    /// guessed port. The two statuses that are not forwarded evidence —
    /// `self` (the bridge's own MACs) and `invalid` — are skipped too;
    /// `learned` marks the row dynamic, and the other statuses leave the
    /// flags unknown so the row renders as static. A MAC that is not
    /// unicast (zero, broadcast, multicast) is no evidence either, the same
    /// rule [`DeviceDetails::mac_ownerships`] applies. `dot1dTpFdbStatus`
    /// carries no age and no bridge name, so both stay absent.
    ///
    /// A document declares one FDB source, so the BRIDGE half answers when
    /// it exists and the Q-BRIDGE half otherwise; both are the same
    /// forwarding evidence to this model, which has one learned-host slot.
    /// A Q-BRIDGE row's VLAN stays in the observation — `BridgeHost` has no
    /// VLAN field — and two VLANs learning one MAC stay two rows. An
    /// answered-but-empty source attaches an empty snapshot, while an
    /// observation with neither half leaves the slot unknown.
    pub fn attach_snmp_fdb(&mut self, fdb: &SnmpFdb, observed_at: SystemTime) {
        // One row shape for both dialects: MAC, bridge port, status.
        let entries: Vec<([u8; 6], Option<u32>, Option<u32>)> = match (&fdb.bridge, &fdb.qbridge) {
            (Some(bridge), _) => bridge
                .iter()
                .map(|row| (row.mac, row.port, row.status))
                .collect(),
            (None, Some(qbridge)) => qbridge
                .iter()
                .map(|row| (row.mac, row.port, row.status))
                .collect(),
            (None, None) => return,
        };
        let names: BTreeMap<u32, &str> = self
            .interfaces
            .iter()
            .filter_map(|row| Some((row.id.parse().ok()?, row.name.as_deref()?)))
            .collect();
        let join: BTreeMap<u32, u32> = fdb.port_if_index.iter().copied().collect();
        let rows: Vec<BridgeHost> = entries
            .into_iter()
            .filter(|(_, _, status)| !matches!(*status, Some(2 | 4)))
            .filter_map(|(mac, port, status)| {
                let mac = Mac::from_chassis(&mac).filter(Mac::is_unicast)?;
                let on_interface = names.get(join.get(&port?)?)?;
                Some(BridgeHost {
                    bridge: None,
                    on_interface: Some((*on_interface).to_owned()),
                    mac_address: Some(mac),
                    age: None,
                    dynamic: (status == Some(3)).then_some(true),
                    local: None,
                    external: None,
                })
            })
            .collect();
        self.bridge_hosts = Some(TableSnapshot {
            rows,
            provenance: vec![Source::SnmpFdb],
            observed_at,
        });
    }

    /// The MAC-ownership evidence these interfaces carry, attributed to
    /// `owner` (the reporting device's MAC, else its address). Zero,
    /// broadcast, and multicast MACs are not ownership evidence; several
    /// interfaces carrying one MAC are each recorded as-is — validity is
    /// the merge's call, not the collector's.
    pub fn mac_ownerships(&self, owner: &str) -> Vec<MacOwnership> {
        self.interfaces
            .iter()
            .filter_map(|row| {
                let mac = row.mac.as_ref()?;
                (mac.is_unicast()).then(|| MacOwnership {
                    owner: owner.to_owned(),
                    interface_id: row.id.clone(),
                    mac: mac.clone(),
                    provenance: row.provenance.clone(),
                })
            })
            .collect()
    }
}

/// The `ifAdminStatus`/`ifOperStatus` code as a state: 1 is up, 2 is
/// down; the other codes (testing, dormant, …) carry no model state.
fn if_state(code: u32) -> Option<IfState> {
    match code {
        1 => Some(IfState::Up),
        2 => Some(IfState::Down),
        _ => None,
    }
}

/// One device interface owning a MAC: explicit merge evidence that the
/// reporting device claims this MAC on this interface — never a claim
/// that another MAC-bearing device is the same machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacOwnership {
    /// The reporting device: its MAC, else its address.
    pub owner: String,
    /// The owning interface's device row id.
    pub interface_id: String,
    /// The MAC the interface carries.
    pub mac: Mac,
    /// The concrete queries that contributed, from the interface row.
    pub provenance: Vec<Source>,
}

/// Groups the two neighbour feeds into per-MAC devices, sorted by MAC.
/// Rows without a usable MAC (an API row with no `mac-address`, an LLDP
/// chassis id of another subtype) are skipped. The API feed is recorded
/// first, so a device's claims — and with them [`Device::name`] and
/// [`Device::address`] — keep the historical API-first preference. Each
/// device then gets the derived vendor hint as its last claim; grouping is
/// by MAC alone, so the hint can never split or merge a device.
pub fn resolve(lldp: &[LldpNeighbour], api: &[ApiNeighbour]) -> Vec<Device> {
    let mut devices: BTreeMap<Mac, Device> = BTreeMap::new();
    for row in api {
        let Some(mac) = row.mac_address.as_deref().and_then(Mac::parse) else {
            continue;
        };
        let device = devices
            .entry(mac.clone())
            .or_insert_with(|| Device::new(mac, Sources::Api));
        device.record(
            Source::RouterOsNeighbors,
            &[
                (ClaimKind::Address, row.address.as_deref()),
                (ClaimKind::Name, row.identity.as_deref()),
                (ClaimKind::Interface, row.interface.as_deref()),
            ],
        );
    }
    for row in lldp {
        let Some(mac) = Mac::from_chassis(&row.chassis_id) else {
            continue;
        };
        let device = devices
            .entry(mac.clone())
            .or_insert_with(|| Device::new(mac, Sources::Lldp));
        device.record(
            Source::SnmpLldp,
            &[
                (ClaimKind::Address, row.address.as_deref()),
                (ClaimKind::Name, row.sys_name.as_deref()),
                (ClaimKind::SysDescr, row.sys_descr.as_deref()),
                (ClaimKind::PortId, row.remote_port_id.as_deref()),
                (ClaimKind::LocalPort, row.local_port.as_deref()),
            ],
        );
    }
    let mut devices: Vec<Device> = devices.into_values().collect();
    for device in &mut devices {
        device.record_vendor_hint();
    }
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An API row fixture: text MAC with optional address and identity.
    fn api_row(mac: &str, address: Option<&str>, identity: Option<&str>) -> ApiNeighbour {
        ApiNeighbour {
            address: address.map(str::to_owned),
            mac_address: Some(mac.to_owned()),
            identity: identity.map(str::to_owned),
            interface: None,
        }
    }

    /// An LLDP row fixture: raw 6-byte chassis MAC with optional address
    /// and sysName.
    fn lldp_row(mac: &str, address: Option<&str>, sys_name: Option<&str>) -> LldpNeighbour {
        LldpNeighbour {
            chassis_id: mac
                .split(':')
                .map(|pair| u8::from_str_radix(pair, 16).expect("fixture mac byte"))
                .collect(),
            address: address.map(str::to_owned),
            remote_port_id: None,
            sys_name: sys_name.map(str::to_owned),
            sys_descr: None,
            local_port: None,
        }
    }

    #[test]
    fn the_block_marks_disagreed_kinds_and_pins_the_line_format() {
        let lldp = [lldp_row(
            "00:1f:5e:aa:bb:cc",
            Some("10.0.0.20"),
            Some("FSW-AP-1"),
        )];
        let api = [api_row(
            "00:1f:5e:aa:bb:cc",
            Some("10.0.0.2"),
            Some("ap-one"),
        )];
        let device = &resolve(&lldp, &api)[0];

        let expected = "00:1f:5e:aa:bb:cc\n  address 10.0.0.2 routeros-neighbors\n  name ap-one routeros-neighbors (conflict)\n  address 10.0.0.20 snmp-lldp\n  name FSW-AP-1 snmp-lldp (conflict)\n  vendor Dyna Technology Co.,Ltd. (ma-l 001F5E) ieee-oui\n";
        assert_eq!(device.to_string(), expected);
    }

    #[test]
    fn agreeing_claims_are_not_marked_and_an_empty_device_is_just_a_header() {
        let lldp = [lldp_row(
            "00:1f:5e:aa:bb:cc",
            Some("10.0.0.2"),
            Some("ap-one"),
        )];
        let api = [api_row(
            "00:1f:5e:aa:bb:cc",
            Some("10.0.0.2"),
            Some("ap-one"),
        )];
        let agreed = &resolve(&lldp, &api)[0];
        let expected = "00:1f:5e:aa:bb:cc\n  address 10.0.0.2 routeros-neighbors\n  name ap-one routeros-neighbors\n  address 10.0.0.2 snmp-lldp\n  name ap-one snmp-lldp\n  vendor Dyna Technology Co.,Ltd. (ma-l 001F5E) ieee-oui\n";
        assert_eq!(agreed.to_string(), expected);

        let silent = [api_row("6c:3b:6b:70:96:3f", None, None)];
        let header_only = &resolve(&[], &silent)[0];
        assert_eq!(
            header_only.to_string(),
            "6c:3b:6b:70:96:3f\n  vendor Routerboard.com (ma-l 6C3B6B) ieee-oui\n"
        );
    }

    /// Every claim kind, in declaration order.
    const KINDS: [ClaimKind; 8] = [
        ClaimKind::Serial,
        ClaimKind::Address,
        ClaimKind::Name,
        ClaimKind::SysDescr,
        ClaimKind::PortId,
        ClaimKind::LocalPort,
        ClaimKind::Interface,
        ClaimKind::Vendor,
    ];

    /// Every stored interface kind, in declaration order.
    const INTERFACE_KINDS: [InterfaceKind; 6] = [
        InterfaceKind::Ethernet,
        InterfaceKind::Bridge,
        InterfaceKind::Vlan,
        InterfaceKind::Loopback,
        InterfaceKind::Tunnel,
        InterfaceKind::Other,
    ];

    /// One claim fixture.
    fn claim(kind: ClaimKind, value: &str, source: Source) -> Claim {
        Claim {
            kind,
            value: value.to_owned(),
            source,
        }
    }

    /// The device the claim-level fixtures describe.
    fn claim_device(claims: Vec<Claim>) -> Device {
        Device::from_claims(
            Mac::parse("00:1f:5e:aa:bb:cc").expect("fixture mac"),
            claims,
        )
    }

    #[test]
    fn only_serial_name_and_sysdescr_are_singleton_identity_facts() {
        let singleton: Vec<ClaimKind> = KINDS
            .into_iter()
            .filter(|kind| kind.is_singleton_identity())
            .collect();

        assert_eq!(
            singleton,
            [ClaimKind::Serial, ClaimKind::Name, ClaimKind::SysDescr]
        );
    }

    #[test]
    fn stored_kind_tokens_round_trip_and_unknown_tokens_are_not_identity_facts() {
        for kind in KINDS {
            assert_eq!(ClaimKind::from_token(&kind.to_string()), Some(kind));
        }

        let identity = |token: &str| {
            ClaimKind::from_token(token).is_some_and(ClaimKind::is_singleton_identity)
        };
        assert!(identity("serial"));
        assert!(!identity("address"));
        assert!(!identity("interface"));
        assert!(
            !identity("serial-number"),
            "unknown tokens default to false"
        );
    }

    #[test]
    fn stored_interface_kind_tokens_round_trip_and_an_unknown_token_gains_no_kind() {
        for kind in INTERFACE_KINDS {
            assert_eq!(InterfaceKind::from_token(&kind.to_string()), Some(kind));
        }

        assert_eq!(
            InterfaceKind::from_token("bond"),
            None,
            "only a Display spelling names a kind"
        );
    }

    #[test]
    fn multi_valued_claims_never_conflict_and_a_repeated_value_is_not_a_conflict() {
        let device = claim_device(vec![
            claim(ClaimKind::Address, "10.0.0.2", Source::RouterOsNeighbors),
            claim(ClaimKind::Address, "10.0.0.20", Source::SnmpLldp),
            claim(ClaimKind::Interface, "ether1", Source::RouterOsNeighbors),
            claim(ClaimKind::Interface, "ether2", Source::RouterOsNeighbors),
            claim(ClaimKind::LocalPort, "7", Source::SnmpLldp),
            claim(ClaimKind::LocalPort, "8", Source::SnmpLldp),
            claim(ClaimKind::PortId, "ether3", Source::SnmpLldp),
            claim(ClaimKind::PortId, "ether4", Source::SnmpLldp),
            claim(ClaimKind::Vendor, "vendor-a", Source::OuiRegistry),
            claim(ClaimKind::Vendor, "vendor-b", Source::OuiRegistry),
            claim(ClaimKind::Name, "ap-one", Source::RouterOsNeighbors),
            claim(ClaimKind::Name, "ap-one", Source::SnmpLldp),
        ]);

        assert!(
            device.conflicts().is_empty(),
            "multiplicity is not an identity disagreement"
        );
    }

    #[test]
    fn singleton_disagreements_list_every_value_and_source() {
        let device = claim_device(vec![
            claim(ClaimKind::SysDescr, "v1", Source::SnmpLldp),
            claim(ClaimKind::Serial, "SER-A", Source::RouterOsNeighbors),
            claim(ClaimKind::SysDescr, "v2", Source::SnmpLldp),
            claim(ClaimKind::Serial, "SER-B", Source::RouterOsNeighbors),
        ]);

        let conflicts = device.conflicts();
        assert_eq!(
            conflicts.iter().map(|c| c.kind).collect::<Vec<_>>(),
            [ClaimKind::Serial, ClaimKind::SysDescr],
            "declaration order, not claim order"
        );
        assert_eq!(
            claims_of(&conflicts[0]),
            [
                ("SER-A", Source::RouterOsNeighbors),
                ("SER-B", Source::RouterOsNeighbors),
            ]
        );
        assert_eq!(
            claims_of(&conflicts[1]),
            [("v1", Source::SnmpLldp), ("v2", Source::SnmpLldp)]
        );
    }

    /// A conflict's claims as (value, source) tuples.
    fn claims_of(conflict: &Conflict) -> Vec<(&str, Source)> {
        conflict
            .claims
            .iter()
            .map(|claim| (claim.value.as_str(), claim.source))
            .collect()
    }

    #[test]
    fn the_local_port_is_a_claim_on_the_neighbour_device() {
        let mut row = lldp_row("00:1f:5e:aa:bb:cc", Some("10.0.0.20"), Some("FSW-AP-1"));
        row.remote_port_id = Some("ether2".to_owned());
        row.local_port = Some("7".to_owned());
        let device = &resolve(&[row], &[])[0];

        let claims: Vec<_> = device
            .claims()
            .iter()
            .filter(|claim| matches!(claim.kind, ClaimKind::PortId | ClaimKind::LocalPort))
            .collect();
        assert_eq!(
            claims,
            [
                &Claim {
                    kind: ClaimKind::PortId,
                    value: "ether2".to_owned(),
                    source: Source::SnmpLldp,
                },
                &Claim {
                    kind: ClaimKind::LocalPort,
                    value: "7".to_owned(),
                    source: Source::SnmpLldp,
                },
            ],
            "the local port rides on the neighbour like the remote port id, after it"
        );

        // An absent local port contributes no claim.
        let silent = lldp_row("6c:3b:6b:70:96:3f", None, None);
        assert!(
            resolve(&[silent], &[])[0]
                .claims()
                .iter()
                .all(|claim| claim.kind != ClaimKind::LocalPort)
        );
    }

    #[test]
    fn mac_class_splits_universal_from_locally_administered() {
        assert_eq!(
            Mac::parse("00:1f:5e:aa:bb:cc").unwrap().class(),
            MacClass::Universal
        );
        assert_eq!(
            Mac::parse("001f.5eaabbcc").unwrap().class(),
            MacClass::Universal
        );
        assert_eq!(
            Mac::parse("02:1f:5e:aa:bb:cc").unwrap().class(),
            MacClass::Local
        );
        assert_eq!(
            Mac::parse("0a-1b-2c-3d-4e-5f").unwrap().class(),
            MacClass::Local
        );
    }

    /// An SNMP interface-row fixture over the raw column codes.
    fn snmp_row(index: u32, if_type: Option<u32>, oper: Option<u32>) -> SnmpInterface {
        SnmpInterface {
            index,
            name: None,
            descr: None,
            alias: None,
            raw_type: if_type,
            admin_status: None,
            oper_status: oper,
            phys_address: None,
        }
    }

    #[test]
    fn snmp_interfaces_map_the_standard_if_type_codes() {
        let details = DeviceDetails::from_snmp_interfaces(&[
            snmp_row(1, Some(6), Some(1)),
            snmp_row(2, Some(24), Some(2)),
            snmp_row(3, Some(135), Some(3)),
            snmp_row(4, Some(209), None),
            snmp_row(5, Some(1), None), // other: a code the model does not name
        ]);
        let kinds: Vec<_> = details
            .interfaces
            .iter()
            .map(|row| (row.kind, row.raw_type.as_deref()))
            .collect();
        assert_eq!(
            kinds,
            [
                (InterfaceKind::Ethernet, Some("6")),
                (InterfaceKind::Loopback, Some("24")),
                (InterfaceKind::Vlan, Some("135")),
                (InterfaceKind::Bridge, Some("209")),
                (InterfaceKind::Other, Some("1")),
            ]
        );
        assert!(
            details
                .interfaces
                .iter()
                .all(|row| row.provenance == [Source::SnmpInterfaces])
        );
    }

    #[test]
    fn snmp_statuses_map_up_down_and_leave_other_codes_unknown() {
        let details = DeviceDetails::from_snmp_interfaces(&[
            snmp_row(1, None, Some(1)),
            snmp_row(2, None, Some(2)),
            snmp_row(3, None, Some(5)), // dormant: no model state
        ]);
        let states: Vec<_> = details
            .interfaces
            .iter()
            .map(|row| (row.admin_state, row.oper_state))
            .collect();
        assert_eq!(
            states,
            [
                (None, Some(IfState::Up)),
                (None, Some(IfState::Down)),
                (None, None),
            ]
        );
    }

    #[test]
    fn snmp_names_prefer_ifname_and_keep_descr_and_alias_separate() {
        // Rows echoing the SwitchOS capture in
        // host/tests/fixtures/switch-fdb/swos-ports-sample.txt: ifName and
        // ifDescr carry the same configured label and ifAlias is empty.
        let mut labeled = snmp_row(1, Some(6), Some(1));
        labeled.name = Some("Management".to_owned());
        labeled.descr = Some("Management".to_owned());
        // A row without ifName falls back to ifDescr for the identifier;
        // a ProCurve-style numeric name and a comment alias both survive.
        let mut numeric = snmp_row(2, Some(6), Some(2));
        numeric.descr = Some("2".to_owned());
        numeric.alias = Some("PAQUIN-MGMT".to_owned());
        let details = DeviceDetails::from_snmp_interfaces(&[labeled, numeric]);

        let first = &details.interfaces[0];
        assert_eq!(first.name.as_deref(), Some("Management"));
        assert_eq!(first.description.as_deref(), Some("Management"));
        assert_eq!(first.alias, None, "an empty alias is no label");

        let second = &details.interfaces[1];
        assert_eq!(second.name.as_deref(), Some("2"), "ifDescr fills in");
        assert_eq!(second.description.as_deref(), Some("2"));
        assert_eq!(second.alias.as_deref(), Some("PAQUIN-MGMT"));
    }

    #[test]
    fn snmp_physical_addresses_parse_as_macs() {
        let mut row = snmp_row(1, None, None);
        row.phys_address = Some(vec![0x64, 0x4d, 0x86, 0x11, 0x22, 0x33]);
        let details = DeviceDetails::from_snmp_interfaces(&[row]);
        assert_eq!(
            details.interfaces[0].mac.as_ref().map(Mac::as_str),
            Some("64:4d:86:11:22:33")
        );

        let mut short = snmp_row(2, None, None);
        short.phys_address = Some(vec![0x00, 0x00, 0x00]);
        let details = DeviceDetails::from_snmp_interfaces(&[short]);
        assert_eq!(details.interfaces[0].mac, None, "not a MAC, not guessed");
    }

    #[test]
    fn snmp_fdb_rows_join_interfaces_and_skip_unjoinable_rows() {
        let mut management = snmp_row(1, Some(6), Some(1));
        management.name = Some("Management".to_owned());
        // Interface 2 exists but names nothing: a row cannot be attached.
        let unnamed = snmp_row(2, None, None);
        let mut details = DeviceDetails::from_snmp_interfaces(&[management, unnamed]);
        let observed_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(7);
        details.attach_snmp_fdb(
            &crate::snmp::SnmpFdb {
                qbridge: None,
                bridge: Some(vec![
                    // Learned on the management port: the one row that joins.
                    crate::snmp::SnmpBridgeRow {
                        mac: [0x00, 0x05, 0xb7, 0x00, 0x00, 0x01],
                        port: Some(1),
                        status: Some(3),
                    },
                    // `self`: the bridge's own MAC, never a learned host.
                    crate::snmp::SnmpBridgeRow {
                        mac: [0x02, 0x1f, 0x5e, 0x00, 0x00, 0x02],
                        port: Some(1),
                        status: Some(4),
                    },
                    // `invalid`: not forwarding evidence either.
                    crate::snmp::SnmpBridgeRow {
                        mac: [0x02, 0x1f, 0x5e, 0x00, 0x00, 0x03],
                        port: Some(1),
                        status: Some(2),
                    },
                    // No join entry for bridge port 9.
                    crate::snmp::SnmpBridgeRow {
                        mac: [0x02, 0x1f, 0x5e, 0x00, 0x00, 0x04],
                        port: Some(9),
                        status: Some(3),
                    },
                    // Port 2 resolves to an interface with no name.
                    crate::snmp::SnmpBridgeRow {
                        mac: [0x02, 0x1f, 0x5e, 0x00, 0x00, 0x05],
                        port: Some(2),
                        status: Some(3),
                    },
                    // No port column: nothing to join.
                    crate::snmp::SnmpBridgeRow {
                        mac: [0x02, 0x1f, 0x5e, 0x00, 0x00, 0x06],
                        port: None,
                        status: Some(3),
                    },
                    // A multicast MAC is not unicast forwarding evidence.
                    crate::snmp::SnmpBridgeRow {
                        mac: [0x01, 0x00, 0x5e, 0x00, 0x00, 0x01],
                        port: Some(1),
                        status: Some(3),
                    },
                    // Neither is the zero MAC.
                    crate::snmp::SnmpBridgeRow {
                        mac: [0x00; 6],
                        port: Some(1),
                        status: Some(3),
                    },
                ]),
                port_if_index: vec![(1, 1), (2, 2), (9, 9)],
            },
            observed_at,
        );

        let hosts = details.bridge_hosts.as_ref().expect("the FDB answered");
        assert_eq!(hosts.provenance, [Source::SnmpFdb]);
        assert_eq!(hosts.observed_at, observed_at);
        assert_eq!(
            hosts.rows.len(),
            1,
            "only the joinable learned unicast row: {:?}",
            hosts.rows
        );
        let row = &hosts.rows[0];
        assert_eq!(
            row.mac_address.as_ref().map(Mac::as_str),
            Some("00:05:b7:00:00:01")
        );
        assert_eq!(row.on_interface.as_deref(), Some("Management"));
        assert_eq!(row.dynamic, Some(true), "status 3 is learned");
        assert_eq!(row.age, None);
        assert_eq!(row.bridge, None);
        assert_eq!((row.local, row.external), (None, None));

        // A multicast and a zero MAC are never learned hosts.
        for not_evidence in ["01:00:5e:00:00:01", "00:00:00:00:00:00"] {
            assert!(
                hosts
                    .rows
                    .iter()
                    .all(|row| row.mac_address.as_ref().map(Mac::as_str) != Some(not_evidence)),
                "`{not_evidence}` is not unicast forwarding evidence"
            );
        }

        // Another status keeps the flags unknown: the row renders static.
        let static_row = crate::snmp::SnmpBridgeRow {
            mac: [0x00, 0x05, 0xb7, 0x00, 0x00, 0x0a],
            port: Some(1),
            status: Some(5),
        };
        details.attach_snmp_fdb(
            &crate::snmp::SnmpFdb {
                qbridge: None,
                bridge: Some(vec![static_row]),
                port_if_index: vec![(1, 1)],
            },
            observed_at,
        );
        assert_eq!(
            details.bridge_hosts.as_ref().expect("answered").rows[0].dynamic,
            None
        );

        // A Q-BRIDGE-only observation maps through the same join: the
        // device's `dot1dBasePortIfIndex` names ifIndex 16 for bridge port
        // 1, while interface 1 (the identity guess) is "Management".
        let mut management_again = snmp_row(1, Some(6), Some(1));
        management_again.name = Some("Management".to_owned());
        let mut sfp = snmp_row(16, Some(6), Some(1));
        sfp.name = Some("sfp-sfpplus1".to_owned());
        let mut qbridge_details = DeviceDetails::from_snmp_interfaces(&[management_again, sfp]);
        qbridge_details.attach_snmp_fdb(
            &crate::snmp::SnmpFdb {
                qbridge: Some(vec![
                    crate::snmp::SnmpQBridgeRow {
                        vlan: 0,
                        mac: [0x00, 0x05, 0xb7, 0x00, 0x00, 0x01],
                        port: Some(1),
                        status: Some(3),
                    },
                    // One MAC learned in a second VLAN is a second row: the
                    // model has no VLAN field to fold them together.
                    crate::snmp::SnmpQBridgeRow {
                        vlan: 92,
                        mac: [0x00, 0x05, 0xb7, 0x00, 0x00, 0x01],
                        port: Some(1),
                        status: Some(3),
                    },
                    // `self`: the bridge's own MACs are not learned hosts.
                    crate::snmp::SnmpQBridgeRow {
                        vlan: 0,
                        mac: [0x00, 0x05, 0xb7, 0x00, 0x00, 0x09],
                        port: Some(1),
                        status: Some(4),
                    },
                    // A multicast MAC is not unicast evidence.
                    crate::snmp::SnmpQBridgeRow {
                        vlan: 0,
                        mac: [0x01, 0x00, 0x5e, 0x00, 0x00, 0x0a],
                        port: Some(1),
                        status: Some(3),
                    },
                    // Bridge port 2 has no join entry.
                    crate::snmp::SnmpQBridgeRow {
                        vlan: 0,
                        mac: [0x00, 0x05, 0xb7, 0x00, 0x00, 0x0b],
                        port: Some(2),
                        status: Some(3),
                    },
                ]),
                bridge: None,
                port_if_index: vec![(1, 16)],
            },
            observed_at,
        );
        let hosts = qbridge_details
            .bridge_hosts
            .as_ref()
            .expect("the Q-BRIDGE answered");
        assert_eq!(hosts.provenance, [Source::SnmpFdb]);
        assert_eq!(
            hosts.rows.len(),
            2,
            "the self, multicast and unjoined rows are dropped: {:?}",
            hosts.rows
        );
        for row in &hosts.rows {
            assert_eq!(
                row.mac_address.as_ref().map(Mac::as_str),
                Some("00:05:b7:00:00:01")
            );
            assert_eq!(
                row.on_interface.as_deref(),
                Some("sfp-sfpplus1"),
                "the device's join names ifIndex 16, never the identity port"
            );
            assert_eq!(row.dynamic, Some(true), "status 3 is learned");
            assert_eq!(row.bridge, None, "the VLAN is not a bridge name");
            assert_eq!(row.age, None);
            assert_eq!((row.local, row.external), (None, None));
        }

        // Both halves populated: a document declares one FDB source, so the
        // BRIDGE half wins and no Q-BRIDGE row surfaces. The rows use
        // distinct MACs so a flipped preference is visible.
        let mut mgmt = snmp_row(1, Some(6), Some(1));
        mgmt.name = Some("Management".to_owned());
        let mut uplink = snmp_row(16, Some(6), Some(1));
        uplink.name = Some("sfp-sfpplus1".to_owned());
        let mut both_halves = DeviceDetails::from_snmp_interfaces(&[mgmt, uplink]);
        both_halves.attach_snmp_fdb(
            &crate::snmp::SnmpFdb {
                qbridge: Some(vec![crate::snmp::SnmpQBridgeRow {
                    vlan: 0,
                    mac: [0x02, 0x00, 0x00, 0x00, 0x00, 0x21],
                    port: Some(1),
                    status: Some(3),
                }]),
                bridge: Some(vec![crate::snmp::SnmpBridgeRow {
                    mac: [0x00, 0x05, 0xb7, 0x00, 0x00, 0x02],
                    port: Some(1),
                    status: Some(3),
                }]),
                port_if_index: vec![(1, 16)],
            },
            observed_at,
        );
        let hosts = both_halves.bridge_hosts.as_ref().expect("answered");
        assert_eq!(
            hosts.rows.len(),
            1,
            "the BRIDGE half wins over the Q-BRIDGE half: {:?}",
            hosts.rows
        );
        assert_eq!(
            hosts.rows[0].mac_address.as_ref().map(Mac::as_str),
            Some("00:05:b7:00:00:02")
        );
        assert!(
            hosts
                .rows
                .iter()
                .all(|row| row.mac_address.as_ref().map(Mac::as_str) != Some("02:00:00:00:00:21")),
            "no Q-BRIDGE row leaks through the BRIDGE preference"
        );

        // An answered-but-empty Q-BRIDGE table is an empty snapshot, not an
        // unknown one.
        qbridge_details.attach_snmp_fdb(
            &crate::snmp::SnmpFdb {
                qbridge: Some(Vec::new()),
                bridge: None,
                port_if_index: Vec::new(),
            },
            observed_at,
        );
        assert_eq!(
            qbridge_details
                .bridge_hosts
                .as_ref()
                .expect("answered empty")
                .rows
                .len(),
            0
        );

        // An observation with neither half leaves the slot unknown.
        let mut neither = DeviceDetails::from_snmp_interfaces(&[]);
        neither.attach_snmp_fdb(
            &crate::snmp::SnmpFdb {
                qbridge: None,
                bridge: None,
                port_if_index: Vec::new(),
            },
            observed_at,
        );
        assert!(neither.bridge_hosts.is_none());
    }

    /// A full ARP collector row.
    fn arp_row(
        address: &str,
        mac: Option<&str>,
        complete: Option<bool>,
    ) -> crate::mikrotik::RouterOsArp {
        crate::mikrotik::RouterOsArp {
            address: Some(address.to_owned()),
            mac_address: mac.map(str::to_owned),
            interface: Some("bridge-local".to_owned()),
            dynamic: Some(true),
            complete,
            disabled: Some(false),
        }
    }

    #[test]
    fn evidence_starts_unavailable_and_arp_rows_normalize_macs() {
        // Neither constructor invents evidence: a snapshot absent from
        // the queries stays None, never an empty stand-in.
        let api = DeviceDetails::from_router_os(&[], &[]);
        assert!(api.arp.is_none() && api.bridge_ports.is_none());
        assert!(api.bridge_vlans.is_none() && api.bridge_hosts.is_none());
        let snmp = DeviceDetails::from_snmp_interfaces(&[]);
        assert!(snmp.arp.is_none() && snmp.bridge_hosts.is_none());

        let entry = ArpEntry::from_router_os(&arp_row(
            "192.168.100.29",
            Some("C8:1F:66:33:AD:6F"),
            Some(true),
        ));
        assert_eq!(entry.address.as_deref(), Some("192.168.100.29"));
        assert_eq!(
            entry.mac_address.as_ref().map(Mac::as_str),
            Some("c8:1f:66:33:ad:6f"),
            "the MAC normalizes"
        );
        assert_eq!(entry.complete, Some(true));

        // An incomplete entry carries no MAC and none is guessed.
        let resolving = ArpEntry::from_router_os(&arp_row("192.168.100.30", None, Some(false)));
        assert_eq!(resolving.mac_address, None);
        assert_eq!(resolving.complete, Some(false));
    }

    #[test]
    fn bridge_port_rows_keep_the_router_status_and_the_flags_separate() {
        let row = crate::mikrotik::RouterOsBridgePort {
            bridge: Some("br_92".to_owned()),
            interface: Some("e1-tower".to_owned()),
            pvid: Some(92),
            frame_types: Some("admit all".to_owned()),
            status: None,
            inactive: Some(true),
            disabled: Some(false),
            dynamic: Some(false),
        };
        let port = BridgePort::from_router_os(&row);
        assert_eq!(port.bridge.as_deref(), Some("br_92"));
        assert_eq!(port.interface.as_deref(), Some("e1-tower"));
        assert_eq!(port.pvid, Some(92));
        assert_eq!(port.status, None, "6.49 sends no status column");
        assert_eq!(port.inactive, Some(true));
        assert_eq!(port.disabled, Some(false));

        // A router that does report a status keeps it verbatim.
        let mut modern = row;
        modern.status = Some("in-bridge".to_owned());
        assert_eq!(
            BridgePort::from_router_os(&modern).status.as_deref(),
            Some("in-bridge")
        );
    }

    #[test]
    fn bridge_vlan_rows_split_lists_in_router_order() {
        let row = crate::mikrotik::RouterOsBridgeVlan {
            bridge: Some("br_666".to_owned()),
            vlan_ids: vec![666, 667],
            tagged: vec!["e2-fsw".to_owned(), "eoip-izones-office666".to_owned()],
            untagged: vec![],
            current_tagged: vec!["e2-fsw".to_owned()],
            current_untagged: vec![],
        };
        let vlan = BridgeVlan::from_router_os(&row);
        assert_eq!(vlan.vlan_ids, [666, 667]);
        assert_eq!(
            vlan.tagged,
            ["e2-fsw".to_owned(), "eoip-izones-office666".to_owned()]
        );
        assert!(vlan.untagged.is_empty(), "an absent list stays empty");
        assert_eq!(vlan.current_tagged, ["e2-fsw".to_owned()]);
    }

    #[test]
    fn bridge_host_rows_normalize_macs_and_keep_flags() {
        let row = crate::mikrotik::RouterOsBridgeHost {
            bridge: Some("br_92".to_owned()),
            on_interface: Some("e1-vlan92".to_owned()),
            mac_address: Some("4C:5E:0C:67:35:12".to_owned()),
            age: Some("3m33s".to_owned()),
            dynamic: Some(true),
            local: Some(false),
            external: None,
        };
        let host = BridgeHost::from_router_os(&row);
        assert_eq!(
            host.mac_address.as_ref().map(Mac::as_str),
            Some("4c:5e:0c:67:35:12")
        );
        assert_eq!(host.age.as_deref(), Some("3m33s"));
        assert_eq!(host.local, Some(false));
        assert_eq!(host.external, None, "an absent flag stays absent");
    }

    #[test]
    fn the_identity_form_of_an_address_drops_the_prefix_and_rejects_empty_text() {
        assert_eq!(bare_address("192.168.88.1/24"), Some("192.168.88.1"));
        assert_eq!(
            bare_address("10.0.0.1"),
            Some("10.0.0.1"),
            "an address without a prefix is already bare"
        );
        assert_eq!(
            bare_address("2001:db8::1/64"),
            Some("2001:db8::1"),
            "the rule is text, not IPv4-only"
        );
        // No claim value and no host-list entry may be empty: a row that
        // states nothing yields nothing.
        assert_eq!(bare_address(""), None);
        assert_eq!(bare_address("/24"), None);
    }
}

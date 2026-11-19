//! SNMP table assembly: the walked `(OID, value)` bindings decoded into
//! typed rows — Q-BRIDGE forwarding-database rows, the interface table,
//! LLDP neighbours joined across three tables, airOS station rows, and
//! ARP/neighbour rows. The BRIDGE-MIB FDB and the airOS station table are
//! decoded by the document engine from a document's table; their row
//! types live here.
//!
//! Pure and synchronous: walked bindings in, rows out, in a deterministic
//! order. No socket, timeout, or walk policy lives here; the walks run in
//! [`crate::snmp`] over the transport in [`crate::snmp_transport`].

use std::collections::BTreeMap;
use std::fmt;

/// One Q-BRIDGE FDB row, decoded from the walk: the VLAN id and MAC from
/// the row index (ProCurve dialects carry the MAC nowhere else), the port
/// and status from their columns. The index ends with the VLAN and the six
/// MAC octets; any leading context arc(s) — ProCurve's filtering-database
/// id, for example — are ignored.
#[derive(Clone, Debug, PartialEq)]
pub struct SnmpQBridgeRow {
    /// The VLAN the entry was learned in.
    pub vlan: u32,
    /// The learned MAC.
    pub mac: [u8; 6],
    /// The bridge-port number, when the column answered.
    pub port: Option<u32>,
    /// The raw status code, when the column answered.
    pub status: Option<u32>,
}

/// One BRIDGE-MIB FDB row: the MAC from the row index or from a column
/// value, the port and status from their columns.
#[derive(Clone, Debug, PartialEq)]
pub struct SnmpBridgeRow {
    /// The learned MAC.
    pub mac: [u8; 6],
    /// The bridge-port number, when the column answered.
    pub port: Option<u32>,
    /// The raw status code, when the column answered.
    pub status: Option<u32>,
}

/// The `dot1dBasePortIfIndex` table: `<bridge-port>` → `ifIndex`.
pub(crate) const DOT1D_BASE_PORT_IF_INDEX: [u32; 11] = [1, 3, 6, 1, 2, 1, 17, 1, 4, 1, 2];

/// The OID tail octets as a 6-byte MAC; values above 255 are not octets.
pub(crate) fn mac_octets(tail: &[u32]) -> Option<[u8; 6]> {
    let bytes: Vec<u8> = tail
        .iter()
        .map(|o| u8::try_from(*o).ok())
        .collect::<Option<_>>()?;
    <[u8; 6]>::try_from(bytes).ok()
}

/// A 6-octet MAC from an octet-string value; every other shape yields
/// `None`, including a value of another length.
pub(crate) fn mac_value(value: &WalkValue) -> Option<[u8; 6]> {
    let WalkValue::Bytes(bytes) = value else {
        return None;
    };
    <[u8; 6]>::try_from(bytes.as_slice()).ok()
}

/// A 6-octet MAC from a DisplayString spelling: exactly six octets in
/// `xx:xx:xx:xx:xx:xx` or `xx-xx-xx-xx-xx-xx` form, ASCII hex and
/// case-insensitive. Mixed separators, a wrong length, a non-hex pair, or
/// a non-string value yield `None` (an absent field).
pub(crate) fn display_mac_value(value: &WalkValue) -> Option<[u8; 6]> {
    let WalkValue::Bytes(bytes) = value else {
        return None;
    };
    if bytes.len() != 17 {
        return None;
    }
    let separator = match bytes[2] {
        b':' => b':',
        b'-' => b'-',
        _ => return None,
    };
    let mut mac = [0u8; 6];
    for (index, octet) in mac.iter_mut().enumerate() {
        let start = index * 3;
        if index < 5 && bytes[start + 2] != separator {
            return None;
        }
        let pair = &bytes[start..start + 2];
        if !pair.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let text = std::str::from_utf8(pair).ok()?;
        *octet = u8::from_str_radix(text, 16).ok()?;
    }
    Some(mac)
}

/// A Q-BRIDGE instance's VLAN and MAC: the MAC is the last six arcs and the
/// VLAN the arc immediately before them, so a dialect's leading context
/// arc(s) — RouterOS writes none (`<vlan>.<mac>`), ProCurve writes its
/// filtering-database id (`<fdb-id>.<vlan>.<mac>`) — are ignored. A tail
/// shorter than seven arcs has no VLAN ahead of a MAC and yields `None`.
pub(crate) fn vlan_and_mac(tail: &[u32]) -> Option<(u32, [u8; 6])> {
    let mac_start = tail.len().checked_sub(6)?;
    let vlan = *tail.get(mac_start.checked_sub(1)?)?;
    Some((vlan, mac_octets(&tail[mac_start..])?))
}

/// One `ipNetToMediaTable` row from its instance tail
/// `<ifIndex>.<a>.<b>.<c>.<d>` and the MAC its column carried. A row with
/// no usable MAC is not evidence.
pub(crate) fn arp_v4_row(instance: &[u32], mac: Option<[u8; 6]>) -> Option<ArpRow> {
    let [if_index, a, b, c, d] = instance else {
        return None;
    };
    let mac = mac.filter(|mac| *mac != [0; 6])?;
    Some(ArpRow {
        if_index: *if_index,
        mac,
        ip: format!("{}.{}.{}.{}", a, b, c, d),
        physical: false,
    })
}

/// One `ipNetToPhysicalTable` row from its instance tail
/// `<ifIndex>.<address-type>.<address octets…>` and the MAC its column
/// carried. Addresses of 4 (IPv4) and 16 (IPv6) octets decode; anything
/// else is not a row.
pub(crate) fn arp_typed_row(instance: &[u32], mac: Option<[u8; 6]>) -> Option<ArpRow> {
    let [if_index, _address_type, address @ ..] = instance else {
        return None;
    };
    let ip = match address.len() {
        4 => format!(
            "{}.{}.{}.{}",
            address[0], address[1], address[2], address[3]
        ),
        16 => {
            let octets: Vec<u8> = address.iter().map(|octet| *octet as u8).collect();
            std::net::Ipv6Addr::from(<[u8; 16]>::try_from(octets).expect("length checked above"))
                .to_string()
        }
        _ => return None,
    };
    let mac = mac.filter(|mac| *mac != [0; 6])?;
    Some(ArpRow {
        if_index: *if_index,
        mac,
        ip,
        physical: true,
    })
}

/// One `ipAddrTable` row: an IPv4 address bound to an interface. The
/// address is the table's index; the netmask column becomes a prefix
/// length when it is a contiguous mask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpAddressRow {
    /// The interface index the address is bound to.
    pub if_index: u32,
    /// The address as dotted text.
    pub address: String,
    /// The netmask as a prefix length, when the mask was present,
    /// well-formed, and contiguous.
    pub prefix_len: Option<u8>,
}

/// One `ipAddrTable` row from its instance tail `<a>.<b>.<c>.<d>`, the
/// `ifIndex` its column carried, and the netmask its column carried.
pub(crate) fn ip_address_v4_row(
    instance: &[u32],
    if_index: Option<u32>,
    netmask: Option<&str>,
) -> Option<IpAddressRow> {
    let [a, b, c, d] = instance else {
        return None;
    };
    let octets = [a, b, c, d].map(|arc| u8::try_from(*arc).ok());
    let [Some(a), Some(b), Some(c), Some(d)] = octets else {
        return None;
    };
    Some(IpAddressRow {
        if_index: if_index?,
        address: format!("{a}.{b}.{c}.{d}"),
        prefix_len: netmask.and_then(netmask_prefix),
    })
}

/// A dotted IPv4 netmask as its prefix length. The mask must have
/// contiguous leading ones; a malformed or non-contiguous mask yields
/// `None` rather than a guessed prefix.
pub(crate) fn netmask_prefix(netmask: &str) -> Option<u8> {
    let octets: Vec<u8> = netmask
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [a, b, c, d] = octets.as_slice() else {
        return None;
    };
    let mask = u32::from_be_bytes([*a, *b, *c, *d]);
    let inverse = !mask;
    if inverse & inverse.wrapping_add(1) != 0 {
        return None;
    }
    u8::try_from(mask.leading_ones()).ok()
}

/// The `dot1dBasePortIfIndex` join: each walked `<bridge-port>` instance
/// and its integer `ifIndex`, sorted by bridge port.
pub(crate) fn base_port_if_index_rows(
    rows: &[(Vec<u32>, WalkValue)],
    root: &[u32],
) -> Vec<(u32, u32)> {
    let mut join: Vec<(u32, u32)> = rows
        .iter()
        .filter_map(|(oid, value)| {
            let [port] = oid.strip_prefix(root)? else {
                return None;
            };
            let if_index = match value {
                WalkValue::Integer(if_index) => u32::try_from(*if_index).ok()?,
                _ => return None,
            };
            Some((*port, if_index))
        })
        .collect();
    join.sort_unstable();
    join
}

/// One interface row collected over SNMP, keyed by ifIndex: the raw
/// column values the device answered with. A column the device skipped
/// stays absent; name/descr/alias are UTF-8 text (empty values count as
/// absent), the statuses and type are the raw integer codes, and the
/// physical address is the raw octets.
#[derive(Clone, Debug)]
pub struct SnmpInterface {
    /// The table index (`ifIndex`), the device's stable handle.
    pub index: u32,
    /// `ifName`, when the device carries the ifXTable column.
    pub name: Option<String>,
    /// `ifDescr`, when present.
    pub descr: Option<String>,
    /// `ifAlias`, when present and non-empty.
    pub alias: Option<String>,
    /// `ifType` as the raw integer code, when present.
    pub raw_type: Option<u32>,
    /// `ifAdminStatus` as the raw integer code, when present.
    pub admin_status: Option<u32>,
    /// `ifOperStatus` as the raw integer code, when present.
    pub oper_status: Option<u32>,
    /// `ifPhysAddress` as raw octets, when present.
    pub phys_address: Option<Vec<u8>>,
}

/// The walked value as text; empty strings count as absent.
pub(crate) fn utf8_value(value: &WalkValue) -> Option<String> {
    match value {
        WalkValue::Bytes(bytes) => {
            let text = String::from_utf8_lossy(bytes).into_owned();
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

/// The walked value as a small status/type code; out-of-range integers
/// and non-integers yield `None`.
pub(crate) fn code_value(value: &WalkValue) -> Option<u32> {
    match value {
        WalkValue::Integer(code) => u32::try_from(*code).ok(),
        _ => None,
    }
}

/// The ePMP channel-width enum as MHz. The MIB enumerates
/// `1=20, 2=40, 3=10, 4=5, 5=80, 6=160, 7=320`; any other code, and every
/// non-integer value, yields `None` (an absent field).
pub(crate) fn channel_width_mhz(value: &WalkValue) -> Option<u32> {
    match value {
        WalkValue::Integer(1) => Some(20),
        WalkValue::Integer(2) => Some(40),
        WalkValue::Integer(3) => Some(10),
        WalkValue::Integer(4) => Some(5),
        WalkValue::Integer(5) => Some(80),
        WalkValue::Integer(6) => Some(160),
        WalkValue::Integer(7) => Some(320),
        _ => None,
    }
}

/// A DisplayString rate with an `M` suffix, e.g. `103M`, as whole Mbps.
/// A bare number, another suffix, or non-UTF-8 text yields `None`; a zero
/// rate is absent (it contributes no capacity).
pub(crate) fn rate_suffix(value: &WalkValue) -> Option<u64> {
    let WalkValue::Bytes(bytes) = value else {
        return None;
    };
    let text = std::str::from_utf8(bytes).ok()?.trim();
    let number = text.strip_suffix('M').or_else(|| text.strip_suffix('m'))?;
    number.parse::<u64>().ok().filter(|rate| *rate > 0)
}

/// LLDP remote-systems table (`lldpRemTable`), the subtree the document
/// engine's `lldp_rem` layout walks.
pub(crate) const LLDP_REM_TABLE: [u32; 9] = [1, 0, 8802, 1, 1, 2, 1, 4, 1];

/// First component under each LLDP table: the row sequence (`lldpRemEntry`,
/// `lldpRemManAddrEntry`) every column instance lives in.
const LLDP_ENTRY: u32 = 1;

/// LLDP management-address table (`lldpRemManAddrTable`), the second
/// subtree the `lldp_rem` layout walks; its rows join the remote-systems
/// rows by the same index tuple.
pub(crate) const LLDP_REM_MAN_ADDR_TABLE: [u32; 9] = [1, 0, 8802, 1, 1, 2, 1, 4, 2];

/// LLDP local-port table (`lldpLocPortTable`), the third subtree the
/// `lldp_rem` layout walks: the device's own view of its ports, indexed by
/// the local port number the remote-systems rows carry.
pub(crate) const LLDP_LOC_PORT_TABLE: [u32; 9] = [1, 0, 8802, 1, 1, 2, 1, 3, 7];

/// `lldpLocPortEntry` column carrying the port id subtype
/// (`lldpLocPortIdSubtype`).
const LLDP_LOC_PORT_ID_SUBTYPE_COLUMN: u32 = 2;

/// `lldpLocPortEntry` column carrying the port id (`lldpLocPortId`).
const LLDP_LOC_PORT_ID_COLUMN: u32 = 3;

/// `lldpLocPortEntry` column carrying the port description
/// (`lldpLocPortDesc`).
const LLDP_LOC_PORT_DESC_COLUMN: u32 = 4;

/// `lldpLocPortIdSubtype` code `ifName(5)`: the port id is the interface
/// table's name for the port.
const LOC_PORT_SUBTYPE_IF_NAME: u32 = 5;

/// `lldpLocPortIdSubtype` code `local(7)`: the port id is an
/// agent-local string naming the port.
const LOC_PORT_SUBTYPE_LOCAL: u32 = 7;

/// `lldpRemManAddrEntry` column carrying the management address string.
const LLDP_MAN_ADDR_COLUMN: u32 = 2;

/// RouterOS reports this placeholder management address when a row carries
/// no usable address.
const ZERO_ADDRESS: &str = "0.0.0.0";

/// An owned SNMP value from a walk: the subset of BER values the
/// interface and UBNT tables carry. Exceptions and unhandled tags
/// collapse to [`WalkValue::Empty`], the walked-row-exists-but-no-value
/// case.
#[derive(Clone, Debug, PartialEq)]
pub enum WalkValue {
    /// An OCTET STRING's bytes.
    Bytes(Vec<u8>),
    /// An INTEGER's value.
    Integer(i64),
    /// A Counter64 (tag 0x46) — wide byte/rate counters v1 agents emit.
    Counter64(u64),
    /// A TimeTicks (tag 0x43) — upticks and ages in hundredths of a
    /// second.
    TimeTicks(u64),
    /// An IpAddress (tag 0x40) as dotted text.
    IpAddress(String),
    /// An OBJECT IDENTIFIER as dotted text.
    Oid(String),
    /// NULL, a v2c exception, or another unhandled tag.
    Empty,
}

/// The walked value as octet-string bytes; every other value yields
/// `None` — the same drop-a-non-string rule the LLDP walk always had.
pub(crate) fn octet_bytes(value: WalkValue) -> Option<Vec<u8>> {
    match value {
        WalkValue::Bytes(bytes) => Some(bytes),
        _ => None,
    }
}

/// A neighbour row learned from the LLDP remote-systems table, joined with
/// its management-address table row by index.
#[derive(Clone, Debug)]
pub struct LldpNeighbour {
    /// Chassis id the remote system advertises, typically its MAC address.
    pub chassis_id: Vec<u8>,
    /// Management address the remote system reports, when present; the
    /// empty string and RouterOS's `0.0.0.0` placeholder count as absent.
    pub address: Option<String>,
    /// Port id the remote system reports for the link, when present.
    pub remote_port_id: Option<String>,
    /// System name the remote system reports, when present.
    pub sys_name: Option<String>,
    /// System description the remote system reports, when present.
    pub sys_descr: Option<String>,
    /// The observer-local port: the interface name the device's own
    /// local-port table gives the port — the id for the `ifName`/`local`
    /// id subtypes, else the port description — else the raw number as
    /// decimal text. The number is the column-2 value when the agent
    /// answers it with a nonzero number, else the middle component of the
    /// row index for agents that decline the column (RouterOS >= 7.7);
    /// absent when neither carries one (RouterOS 6.49 zeroes the index).
    pub local_port: Option<String>,
}

impl fmt::Display for LldpNeighbour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Standard agents send a raw 6-byte MAC; RouterOS sends the MAC
        // pre-formatted as text. Six bytes mean raw and print as hex
        // pairs; anything else is already text.
        let mac = if self.chassis_id.len() == 6 {
            self.chassis_id
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(":")
        } else {
            String::from_utf8_lossy(&self.chassis_id).into_owned()
        };
        write!(
            f,
            "{mac} {} {} {} {} {}",
            self.address.as_deref().unwrap_or("-"),
            self.remote_port_id.as_deref().unwrap_or("-"),
            self.sys_name.as_deref().unwrap_or("-"),
            self.sys_descr.as_deref().unwrap_or("-"),
            self.local_port.as_deref().unwrap_or("-")
        )
    }
}

/// The remote-systems column numbers a document maps, so one assembler
/// serves every column layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LldpColumns {
    /// `lldpRemLocalPortNum`.
    pub local_port: u32,
    /// `lldpRemChassisId`.
    pub chassis_id: u32,
    /// `lldpRemPortId`.
    pub port_id: u32,
    /// `lldpRemSysName`.
    pub sys_name: u32,
    /// `lldpRemSysDescr`.
    pub sys_descr: u32,
}

/// Assembles walked bindings into neighbour rows keyed by the row-index
/// suffix of each instance OID, then joins the management addresses
/// collected from their table by the same index, and resolves each row's
/// local-port number against the local-port table. The column numbers come
/// from the caller so a document's remap and the built-in layout share one
/// assembler.
pub(crate) fn assemble_neighbours(
    rem: &[(Vec<u32>, WalkValue)],
    man_addrs: &[(Vec<u32>, WalkValue)],
    loc_ports: &BTreeMap<u32, LldpLocPort>,
    columns: LldpColumns,
) -> Vec<LldpNeighbour> {
    let addresses = man_addr_rows(man_addrs);
    let mut rows: BTreeMap<&[u32], LldpRow> = BTreeMap::new();
    for (oid, value) in rem {
        let Some(suffix) = oid.strip_prefix(LLDP_REM_TABLE.as_slice()) else {
            continue;
        };
        let [entry, column, indices @ ..] = suffix else {
            continue;
        };
        if *entry != LLDP_ENTRY {
            continue;
        }
        let row = rows.entry(indices).or_default();
        match *column {
            column if column == columns.local_port => row.local_port_num = code_value(value),
            column if column == columns.chassis_id => row.chassis_id = octet_bytes(value.clone()),
            column if column == columns.port_id => row.remote_port_id = utf8_value(value),
            column if column == columns.sys_name => row.sys_name = utf8_value(value),
            column if column == columns.sys_descr => row.sys_descr = utf8_value(value),
            _ => {}
        }
    }
    rows.into_iter()
        .filter_map(|(indices, row)| {
            // A row without a chassis id cannot identify a neighbour; the
            // all-zero MAC marks a junk row an agent reports with no data.
            let chassis_id = row.chassis_id?;
            if is_zero_mac(&chassis_id) {
                return None;
            }
            Some(LldpNeighbour {
                address: addresses.get(indices).cloned().flatten(),
                chassis_id,
                remote_port_id: row.remote_port_id,
                sys_name: row.sys_name,
                sys_descr: row.sys_descr,
                local_port: local_port_text(
                    local_port_number(row.local_port_num, indices),
                    loc_ports,
                ),
            })
        })
        .collect()
}

/// Management addresses from the man-addr walk, keyed by their row index;
/// the value is the address as text when the row carries a usable one.
fn man_addr_rows(walked: &[(Vec<u32>, WalkValue)]) -> BTreeMap<&[u32], Option<String>> {
    walked
        .iter()
        .filter_map(|(oid, value)| {
            let suffix = oid.strip_prefix(LLDP_REM_MAN_ADDR_TABLE.as_slice())?;
            let [entry, column, indices @ ..] = suffix else {
                return None;
            };
            if *entry != LLDP_ENTRY || *column != LLDP_MAN_ADDR_COLUMN {
                return None;
            }
            Some((indices, man_address(value)))
        })
        .collect()
}

/// Local-port rows from the loc-port walk, keyed by the port number (the
/// table's single index octet).
pub(crate) fn loc_port_rows(walked: &[(Vec<u32>, WalkValue)]) -> BTreeMap<u32, LldpLocPort> {
    let mut rows: BTreeMap<u32, LldpLocPort> = BTreeMap::new();
    for (oid, value) in walked {
        let Some([entry, column, num]) = oid.strip_prefix(LLDP_LOC_PORT_TABLE.as_slice()) else {
            continue;
        };
        if *entry != LLDP_ENTRY {
            continue;
        }
        let row = rows.entry(*num).or_default();
        match *column {
            LLDP_LOC_PORT_ID_SUBTYPE_COLUMN => row.subtype = code_value(value),
            LLDP_LOC_PORT_ID_COLUMN => row.id = utf8_value(value),
            LLDP_LOC_PORT_DESC_COLUMN => row.descr = utf8_value(value),
            _ => {}
        }
    }
    rows
}

/// One `lldpLocPortEntry` row under assembly.
#[derive(Default)]
pub(crate) struct LldpLocPort {
    /// The raw `lldpLocPortIdSubtype` code, when the column answered.
    subtype: Option<u32>,
    /// `lldpLocPortId` as text, when the column answered non-empty.
    id: Option<String>,
    /// `lldpLocPortDesc` as text, when the column answered non-empty.
    descr: Option<String>,
}

/// The observer-local port as display text: the device's own name for the
/// port — the local-port id for the `ifName`/`local` id subtypes, else the
/// local-port description — and the raw number where the local-port table
/// names nothing. The number is never read as an ifIndex or a bridge-port
/// number here; the walk layer may still join it against the interface
/// rows it collected.
fn local_port_text(num: Option<u32>, loc_ports: &BTreeMap<u32, LldpLocPort>) -> Option<String> {
    let num = num?;
    let named = loc_ports.get(&num).and_then(|row| match row.subtype {
        Some(LOC_PORT_SUBTYPE_IF_NAME) | Some(LOC_PORT_SUBTYPE_LOCAL) => row.id.clone(),
        _ => None,
    });
    Some(
        named
            .or_else(|| loc_ports.get(&num).and_then(|row| row.descr.clone()))
            .unwrap_or_else(|| num.to_string()),
    )
}

/// The row's observer-local port number: the column-2 value when the agent
/// answers it with a nonzero number, else the middle component of a row
/// index with exactly the RFC's three components ({timeMark, localPortNum,
/// remIndex}) when nonzero, else nothing. The MIB requires the column
/// value and the index to agree, so the column keeps priority wherever an
/// agent answers it; the index carries the number for agents that decline
/// the column (RouterOS >= 7.7 answers `noSuchObject` for columns 1-3),
/// and a zero in either place means the agent does not track the port
/// (RouterOS 6.49 zeroes every index middle) — zero is never a port.
fn local_port_number(column: Option<u32>, indices: &[u32]) -> Option<u32> {
    if let Some(num) = column.filter(|num| *num != 0) {
        return Some(num);
    }
    match indices {
        [_time_mark, local_port_num, _rem_index] => {
            (*local_port_num != 0).then_some(*local_port_num)
        }
        _ => None,
    }
}

/// One `lldpRemEntry` row under assembly.
#[derive(Default)]
struct LldpRow {
    local_port_num: Option<u32>,
    chassis_id: Option<Vec<u8>>,
    remote_port_id: Option<String>,
    sys_name: Option<String>,
    sys_descr: Option<String>,
}

/// The management address as text; the RouterOS `0.0.0.0` placeholder and
/// the empty string count as absent.
fn man_address(value: &WalkValue) -> Option<String> {
    let text = utf8_value(value)?;
    (text != ZERO_ADDRESS).then_some(text)
}

/// True when a chassis id is the all-zero MAC junk rows advertise, either
/// as six zero bytes (standard agents) or as the text form RouterOS sends
/// (`00:00:00:00:00:00`).
fn is_zero_mac(chassis: &[u8]) -> bool {
    chassis.iter().all(|&byte| byte == 0)
        || std::str::from_utf8(chassis).is_ok_and(|text| {
            text.split(':')
                .all(|group| !group.is_empty() && group.bytes().all(|byte| byte == b'0'))
        })
}

/// One station associated with an airOS access point, as the AP reports
/// it at collection time. Station-table columns the document does not
/// read keep raw slots (`distance_ccq`, `quality`); the MIB names them.
#[derive(Clone, Debug, PartialEq)]
pub struct AirStation {
    /// The station's wireless MAC, as the AP saw it.
    pub mac: [u8; 6],
    /// The WLAN interface index the row's index carries; semantics
    /// unconfirmed (single-radio sectors answer `1`).
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
}

/// One ARP/neighbour row from a seed's tables: an IP-to-MAC binding on
/// `if_index`. Rows without a 6-octet MAC (incomplete entries) are
/// dropped by the collector.
#[derive(Clone, Debug, PartialEq)]
pub struct ArpRow {
    /// The interface index the neighbour was learned on.
    pub if_index: u32,
    /// The neighbour MAC.
    pub mac: [u8; 6],
    /// The neighbour IP as text (dotted quad, or IPv6 text for rows from
    /// the physical table).
    pub ip: String,
    /// Whether the row came from `ipNetToPhysicalTable` (else
    /// `ipNetToMediaTable`).
    pub physical: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The LLDP-MIB's standard remote-systems layout, which agents answer:
    /// chassis 5, port id 7, sysName 9, sysDescr 10, local port 2.
    fn lldp_columns() -> LldpColumns {
        LldpColumns {
            local_port: 2,
            chassis_id: 5,
            port_id: 7,
            sys_name: 9,
            sys_descr: 10,
        }
    }

    #[test]
    fn qbridge_instances_decode_both_index_dialects() {
        let mac = [0x00, 0x0c, 0x42, 0xaa, 0xbb, 0xcc];
        // RouterOS: `<vlan>.<mac>` — seven arcs, no context ahead of the
        // VLAN.
        assert_eq!(
            vlan_and_mac(&[0, 0, 0x0c, 0x42, 0xaa, 0xbb, 0xcc]),
            Some((0, mac))
        );
        assert_eq!(
            vlan_and_mac(&[34, 0x00, 0x0c, 0x42, 0xaa, 0xbb, 0xcc]),
            Some((34, mac))
        );
        // ProCurve: `<fdb-id>.<vlan>.<mac>` — the leading filtering-database
        // id is context, the VLAN is the arc before the MAC.
        assert_eq!(
            vlan_and_mac(&[1, 34, 0x00, 0x0c, 0x42, 0xaa, 0xbb, 0xcc]),
            Some((34, mac))
        );
        assert_eq!(
            vlan_and_mac(&[0, 34, 0x00, 0x0c, 0x42, 0xaa, 0xbb, 0xcc]),
            Some((34, mac)),
            "the leading arc's value is irrelevant"
        );
        assert_eq!(
            vlan_and_mac(&[7, 1, 34, 0x00, 0x0c, 0x42, 0xaa, 0xbb, 0xcc]),
            Some((34, mac)),
            "any number of leading context arcs is ignored: the rule reads the tail"
        );
        // A tail with no VLAN ahead of the six MAC octets is no row.
        assert_eq!(vlan_and_mac(&[0x00, 0x0c, 0x42, 0xaa, 0xbb, 0xcc]), None);
        assert_eq!(vlan_and_mac(&[]), None);
        // The MAC must still be octets.
        assert_eq!(vlan_and_mac(&[1, 34, 0x00, 0x0c, 0x42, 0xaa, 0x1bc]), None);
    }

    #[test]
    fn neighbour_rows_assemble_across_routeros_columns() {
        let at = |table: &[u32], column: u32, row: &[u32]| -> Vec<u32> {
            [table, &[LLDP_ENTRY, column], row].concat()
        };
        let walked = vec![
            // Row 0.1.1: the integer subtype columns flank the identity
            // columns and carry no values; column 2 is the observer-local
            // port number.
            (at(&LLDP_REM_TABLE, 2, &[0, 1, 1]), WalkValue::Integer(2)),
            (at(&LLDP_REM_TABLE, 4, &[0, 1, 1]), WalkValue::Empty),
            (
                at(&LLDP_REM_TABLE, 5, &[0, 1, 1]),
                WalkValue::Bytes(vec![0x00, 0x1f, 0x5e, 0xaa, 0xbb, 0xcc]),
            ),
            (at(&LLDP_REM_TABLE, 6, &[0, 1, 1]), WalkValue::Empty),
            (
                at(&LLDP_REM_TABLE, 7, &[0, 1, 1]),
                WalkValue::Bytes(b"ether2".to_vec()),
            ),
            (
                at(&LLDP_REM_TABLE, 9, &[0, 1, 1]),
                WalkValue::Bytes(b"FSW-AP-1".to_vec()),
            ),
            (
                at(&LLDP_REM_TABLE, 10, &[0, 1, 1]),
                WalkValue::Bytes(b"RouterOS 7.16.2".to_vec()),
            ),
            // Row 0.0.2: sysName, sysDescr, and the local-port number
            // absent from this instance, and a column outside the observed
            // set that must be ignored. The index middle is zero — the
            // RouterOS 6.49 shape: the agent does not track the port.
            (
                at(&LLDP_REM_TABLE, 5, &[0, 0, 2]),
                WalkValue::Bytes(vec![0x00, 0x1f, 0x5e, 0xdd, 0xee, 0xff]),
            ),
            (
                at(&LLDP_REM_TABLE, 7, &[0, 0, 2]),
                WalkValue::Bytes(b"ether3".to_vec()),
            ),
            (
                at(&LLDP_REM_TABLE, 8, &[0, 0, 2]),
                WalkValue::Bytes(b"port description".to_vec()),
            ),
            (at(&LLDP_REM_TABLE, 9, &[0, 0, 2]), WalkValue::Empty),
            // A row without a chassis id cannot identify a neighbour.
            (
                at(&LLDP_REM_TABLE, 7, &[0, 1, 3]),
                WalkValue::Bytes(b"ether4".to_vec()),
            ),
        ];
        let man_addrs = vec![
            (
                at(&LLDP_REM_MAN_ADDR_TABLE, 2, &[0, 1, 1]),
                WalkValue::Bytes(b"10.0.0.5".to_vec()),
            ),
            // RouterOS's placeholder: reported, but counts as absent.
            (
                at(&LLDP_REM_MAN_ADDR_TABLE, 2, &[0, 0, 2]),
                WalkValue::Bytes(b"0.0.0.0".to_vec()),
            ),
            // An address row without a remote-systems row joins nothing.
            (
                at(&LLDP_REM_MAN_ADDR_TABLE, 2, &[0, 1, 9]),
                WalkValue::Bytes(b"10.0.0.9".to_vec()),
            ),
        ];
        // The local-port table names port 2 `ether2`; port 9 has no row.
        let loc_walked = vec![
            (
                at(&LLDP_LOC_PORT_TABLE, LLDP_LOC_PORT_ID_SUBTYPE_COLUMN, &[2]),
                WalkValue::Integer(5),
            ),
            (
                at(&LLDP_LOC_PORT_TABLE, LLDP_LOC_PORT_ID_COLUMN, &[2]),
                WalkValue::Bytes(b"ether2".to_vec()),
            ),
            (
                at(&LLDP_LOC_PORT_TABLE, LLDP_LOC_PORT_DESC_COLUMN, &[2]),
                WalkValue::Bytes(b"to the AP".to_vec()),
            ),
            (
                at(&LLDP_LOC_PORT_TABLE, LLDP_LOC_PORT_ID_SUBTYPE_COLUMN, &[9]),
                WalkValue::Integer(3),
            ),
        ];
        let neighbours = assemble_neighbours(
            &walked,
            &man_addrs,
            &loc_port_rows(&loc_walked),
            lldp_columns(),
        );
        assert_eq!(neighbours.len(), 2);
        // The rows assemble in index order: 0.0.2 (the zero-middle row)
        // sorts before 0.1.1.
        assert_eq!(
            neighbours[0].chassis_id,
            vec![0x00, 0x1f, 0x5e, 0xdd, 0xee, 0xff]
        );
        assert_eq!(neighbours[0].address, None);
        assert_eq!(neighbours[0].remote_port_id.as_deref(), Some("ether3"));
        assert_eq!(neighbours[0].sys_name, None);
        assert_eq!(neighbours[0].sys_descr, None);
        assert_eq!(
            neighbours[0].local_port, None,
            "the zero index middle means the agent does not track the port"
        );
        assert_eq!(
            neighbours[1].chassis_id,
            vec![0x00, 0x1f, 0x5e, 0xaa, 0xbb, 0xcc]
        );
        assert_eq!(neighbours[1].address.as_deref(), Some("10.0.0.5"));
        assert_eq!(neighbours[1].remote_port_id.as_deref(), Some("ether2"));
        assert_eq!(neighbours[1].sys_name.as_deref(), Some("FSW-AP-1"));
        assert_eq!(neighbours[1].sys_descr.as_deref(), Some("RouterOS 7.16.2"));
        assert_eq!(
            neighbours[1].local_port.as_deref(),
            Some("ether2"),
            "the local-port table's ifName id resolves the number"
        );
    }

    #[test]
    fn declined_column_falls_back_to_the_row_index() {
        // The live RouterOS 7.7 shape: the agent declines the identity
        // columns' low numbers, and the row index {timeMark, localPortNum,
        // remIndex} carries the local port number in its middle component.
        // The local-port table names port 5 with a MAC-valued id, so the
        // description names the port — and the row's other fields still
        // assemble.
        let at = |table: &[u32], column: u32, row: &[u32]| -> Vec<u32> {
            [table, &[LLDP_ENTRY, column], row].concat()
        };
        let walked = vec![
            (
                at(&LLDP_REM_TABLE, 5, &[0, 5, 25]),
                WalkValue::Bytes(vec![0x24, 0x5a, 0x4c, 0x3f, 0xeb, 0xa8]),
            ),
            (
                at(&LLDP_REM_TABLE, 7, &[0, 5, 25]),
                WalkValue::Bytes(b"eth0".to_vec()),
            ),
            (
                at(&LLDP_REM_TABLE, 9, &[0, 5, 25]),
                WalkValue::Bytes(b"ZRG-Columbia MX84".to_vec()),
            ),
        ];
        let loc_walked = vec![
            (
                at(&LLDP_LOC_PORT_TABLE, LLDP_LOC_PORT_ID_SUBTYPE_COLUMN, &[5]),
                WalkValue::Integer(3),
            ),
            (
                at(&LLDP_LOC_PORT_TABLE, LLDP_LOC_PORT_ID_COLUMN, &[5]),
                WalkValue::Bytes(vec![0x48, 0xa9, 0x8a, 0x17, 0xd6, 0x45]),
            ),
            (
                at(&LLDP_LOC_PORT_TABLE, LLDP_LOC_PORT_DESC_COLUMN, &[5]),
                WalkValue::Bytes(b"ether5".to_vec()),
            ),
        ];
        let neighbours =
            assemble_neighbours(&walked, &[], &loc_port_rows(&loc_walked), lldp_columns());
        assert_eq!(neighbours.len(), 1);
        assert_eq!(
            neighbours[0].chassis_id,
            vec![0x24, 0x5a, 0x4c, 0x3f, 0xeb, 0xa8]
        );
        assert_eq!(neighbours[0].remote_port_id.as_deref(), Some("eth0"));
        assert_eq!(neighbours[0].sys_name.as_deref(), Some("ZRG-Columbia MX84"));
        assert_eq!(
            neighbours[0].local_port.as_deref(),
            Some("ether5"),
            "the index middle carries the number; the description names it"
        );
    }

    #[test]
    fn zero_means_the_port_is_not_tracked() {
        // A column answered with zero, and an index whose middle is zero
        // to match: the agent does not track the port, and neither zero
        // may surface as a port.
        let at = |table: &[u32], column: u32, row: &[u32]| -> Vec<u32> {
            [table, &[LLDP_ENTRY, column], row].concat()
        };
        let walked = vec![
            (at(&LLDP_REM_TABLE, 2, &[0, 0, 7]), WalkValue::Integer(0)),
            (
                at(&LLDP_REM_TABLE, 5, &[0, 0, 7]),
                WalkValue::Bytes(vec![0x00, 0x1f, 0x5e, 0x11, 0x22, 0x33]),
            ),
        ];
        let neighbours = assemble_neighbours(&walked, &[], &BTreeMap::new(), lldp_columns());
        assert_eq!(neighbours.len(), 1);
        assert_eq!(
            neighbours[0].local_port, None,
            "a zero column value is absence, never a port"
        );
    }

    #[test]
    fn an_index_that_is_not_three_components_is_ignored() {
        // No column value and row indexes that are not the RFC's three
        // components: there is no local-port number to find, and none may
        // be guessed — even though the local-port table names port 5.
        let at = |table: &[u32], column: u32, row: &[u32]| -> Vec<u32> {
            [table, &[LLDP_ENTRY, column], row].concat()
        };
        let walked = vec![
            (
                at(&LLDP_REM_TABLE, 5, &[0, 5]),
                WalkValue::Bytes(vec![0x00, 0x1f, 0x5e, 0xaa, 0xbb, 0xcc]),
            ),
            (
                at(&LLDP_REM_TABLE, 5, &[3, 5, 25, 9]),
                WalkValue::Bytes(vec![0x00, 0x1f, 0x5e, 0xdd, 0xee, 0xff]),
            ),
        ];
        let loc_walked = vec![(
            at(&LLDP_LOC_PORT_TABLE, LLDP_LOC_PORT_DESC_COLUMN, &[5]),
            WalkValue::Bytes(b"ether5".to_vec()),
        )];
        let neighbours =
            assemble_neighbours(&walked, &[], &loc_port_rows(&loc_walked), lldp_columns());
        assert_eq!(neighbours.len(), 2);
        assert!(
            neighbours.iter().all(|row| row.local_port.is_none()),
            "no port may be guessed from a non-RFC index"
        );
    }

    #[test]
    fn local_port_text_respects_the_id_subtype() {
        let at = |column: u32, num: u32| -> Vec<u32> {
            [LLDP_LOC_PORT_TABLE.as_slice(), &[LLDP_ENTRY, column, num]].concat()
        };
        let walked = vec![
            // Port 2: ifName(5), the id is the interface name.
            (at(2, 2), WalkValue::Integer(5)),
            (at(3, 2), WalkValue::Bytes(b"ether2".to_vec())),
            // Port 4: macAddress(3), the id names nothing — the
            // description is the device's own label.
            (at(2, 4), WalkValue::Integer(3)),
            (at(4, 4), WalkValue::Bytes(b"port 4".to_vec())),
            // Port 6: local(7), the agent-local string names the port.
            (at(2, 6), WalkValue::Integer(7)),
            (at(3, 6), WalkValue::Bytes(b"sw7".to_vec())),
        ];
        let loc_ports = loc_port_rows(&walked);
        assert_eq!(
            local_port_text(Some(2), &loc_ports).as_deref(),
            Some("ether2"),
            "an ifName id is the port's name"
        );
        assert_eq!(
            local_port_text(Some(4), &loc_ports).as_deref(),
            Some("port 4"),
            "a non-name id falls to the description"
        );
        assert_eq!(
            local_port_text(Some(6), &loc_ports).as_deref(),
            Some("sw7"),
            "a local id names the port"
        );
        assert_eq!(
            local_port_text(Some(9), &loc_ports).as_deref(),
            Some("9"),
            "no local-port row: the raw number"
        );
        assert_eq!(local_port_text(None, &loc_ports), None);
    }

    #[test]
    fn zero_mac_rows_are_junk_and_dropped() {
        let at = |column: u32, row: &[u32]| -> Vec<u32> {
            [LLDP_REM_TABLE.as_slice(), &[LLDP_ENTRY, column], row].concat()
        };
        let walked = vec![
            // Raw zero bytes, yet carrying data: dropped for the MAC.
            (at(5, &[0, 1, 1]), WalkValue::Bytes(vec![0; 6])),
            (at(7, &[0, 1, 1]), WalkValue::Bytes(b"ether2".to_vec())),
            // RouterOS's text form of the same all-zero MAC: also junk.
            (
                at(5, &[0, 1, 2]),
                WalkValue::Bytes(b"00:00:00:00:00:00".to_vec()),
            ),
            // A real chassis id survives.
            (
                at(5, &[0, 1, 3]),
                WalkValue::Bytes(vec![0x00, 0x1f, 0x5e, 0xaa, 0xbb, 0xcc]),
            ),
        ];
        let neighbours = assemble_neighbours(&walked, &[], &BTreeMap::new(), lldp_columns());
        assert_eq!(neighbours.len(), 1);
        assert_eq!(
            neighbours[0].chassis_id,
            vec![0x00, 0x1f, 0x5e, 0xaa, 0xbb, 0xcc]
        );
    }

    #[test]
    fn neighbour_display_prints_mac_and_placeholders() {
        let full = LldpNeighbour {
            chassis_id: vec![0x00, 0x1f, 0x5e, 0x01, 0x02, 0x03],
            address: Some("10.0.0.5".to_owned()),
            remote_port_id: Some("ether2".to_owned()),
            sys_name: Some("FSW-AP-1".to_owned()),
            sys_descr: Some("RouterOS 7.16.2".to_owned()),
            local_port: Some("7".to_owned()),
        };
        assert_eq!(
            full.to_string(),
            "00:1f:5e:01:02:03 10.0.0.5 ether2 FSW-AP-1 RouterOS 7.16.2 7"
        );
        // RouterOS pre-formats the chassis MAC as text; it prints as-is.
        // Every absent field prints as `-`.
        let text = LldpNeighbour {
            chassis_id: b"6C:3B:6B:70:96:3F".to_vec(),
            address: None,
            remote_port_id: None,
            sys_name: None,
            sys_descr: None,
            local_port: None,
        };
        assert_eq!(text.to_string(), "6C:3B:6B:70:96:3F - - - - -");
    }

    #[test]
    fn netmask_prefix_accepts_only_contiguous_masks() {
        assert_eq!(netmask_prefix("255.255.255.0"), Some(24));
        assert_eq!(netmask_prefix("255.255.255.252"), Some(30));
        assert_eq!(netmask_prefix("255.255.255.255"), Some(32));
        assert_eq!(netmask_prefix("0.0.0.0"), Some(0));
        assert_eq!(netmask_prefix("255.0.255.0"), None, "a hole is no mask");
        assert_eq!(netmask_prefix("255.255.255"), None, "three octets");
        assert_eq!(netmask_prefix("255.255.255.256"), None, "out of range");
        assert_eq!(netmask_prefix(""), None);
    }

    #[test]
    fn ip_address_rows_decode_the_index_and_netmask() {
        assert_eq!(
            ip_address_v4_row(&[10, 0, 0, 1], Some(5), Some("255.255.255.0")),
            Some(IpAddressRow {
                if_index: 5,
                address: "10.0.0.1".to_owned(),
                prefix_len: Some(24),
            })
        );
        assert_eq!(
            ip_address_v4_row(&[10, 0, 0, 2], Some(3), None),
            Some(IpAddressRow {
                if_index: 3,
                address: "10.0.0.2".to_owned(),
                prefix_len: None,
            })
        );
        assert_eq!(
            ip_address_v4_row(&[10, 0, 0, 3], Some(3), Some("255.0.255.0")),
            Some(IpAddressRow {
                if_index: 3,
                address: "10.0.0.3".to_owned(),
                prefix_len: None,
            }),
            "a non-contiguous mask leaves the address unprefixed"
        );
        assert_eq!(ip_address_v4_row(&[10, 0, 0], Some(3), None), None);
        assert_eq!(ip_address_v4_row(&[10, 0, 0, 256], Some(3), None), None);
        assert_eq!(
            ip_address_v4_row(&[10, 0, 0, 1], None, None),
            None,
            "no ifIndex is no attachment"
        );
    }
}

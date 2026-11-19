//! Adapter document schema v2: the data-only description of how a device
//! family is detected, which read-only surfaces it exposes, and how the
//! answers map into the identity model.
//!
//! A document is JSON, validated whole before use. It carries no executable
//! behaviour: literals, direct field references, and closed enums only. The
//! transports, parsers, joins, and observation construction named here are
//! core primitives; a document chooses and names them.
//!
//! The field guide is `docs/adapters/schema-v2.md`. The loader (B2) reads
//! exactly `<vault-directory>/adapters/`, validates the whole registry, and
//! activates it atomically; this module owns the schema and its strict
//! validator only.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Deserialize;

use crate::commands::detail_slot;

pub mod engine;
pub mod live;
pub mod reader;
pub mod registry;

/// The RouterOS command vocabulary a document's command probes declare.
/// It lives in a leaf module so the transport and the schema both import it
/// without a cycle.
pub use crate::commands::CommandName;

/// The only document schema version this build understands.
pub const SCHEMA_VERSION: u32 = 2;

/// The most requests one document's probes may budget for.
pub const MAX_REQUESTS_CEILING: u32 = 10_000;
/// The most rows one document's tables may budget for.
pub const MAX_ROWS_CEILING: u32 = 4_096;
/// The most bytes one document's probes may budget for.
pub const MAX_BYTES_CEILING: u64 = 1 << 20;
/// The longest deadline one document may budget for.
pub const MAX_DEADLINE_MS_CEILING: u64 = 30_000;

/// One adapter document: a device family's detection hints, read-only
/// surfaces, probes, field maps, and budget.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// Must be [`SCHEMA_VERSION`]; any other value is rejected.
    pub schema_version: u32,
    /// The adapter's identifier, unique within the registry.
    pub id: String,
    /// How a device is recognised as belonging to this family.
    #[serde(default)]
    pub detect: Detect,
    /// The management surfaces the probes read through.
    pub surfaces: Vec<Surface>,
    /// The independent read-only probes.
    pub probes: Vec<Probe>,
    /// How probe fields become claims and details.
    #[serde(default)]
    pub maps: Maps,
    /// Request, row, byte, and time limits; they may only tighten core limits.
    #[serde(default)]
    pub budget: Budget,
}

/// Detection hints: a device matches when any prefix matches its
/// `sysObjectID` or any substring appears in its `sysDescr`. A document with
/// no hints may instead be the registry's single default, which applies when
/// no hint matches.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detect {
    /// Exact `sysObjectID.0` values, compared by OID arcs and evaluated
    /// before every prefix hint. A family that reports one fixed OID can
    /// use this so a broader prefix elsewhere cannot capture it.
    #[serde(default)]
    pub sys_object_id_exact: Vec<String>,
    /// Dotted OID prefixes matched against the agent's `sysObjectID.0`.
    #[serde(default)]
    pub sys_object_id_prefixes: Vec<String>,
    /// Literal substrings matched against the agent's `sysDescr.0`.
    #[serde(default)]
    pub sys_descr_contains: Vec<String>,
    /// A document reachable only through a per-device pin. It declares no
    /// hints and is never the default; automatic selection skips it.
    #[serde(default)]
    pub pin_only: bool,
    /// Whether this is the registry's generic base document: it declares no
    /// hints and applies only when no other document matches.
    #[serde(default)]
    pub default: bool,
}

/// One management surface: a transport and the credential kind it resolves.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    /// The surface's identifier, referenced by probes.
    pub id: String,
    /// The transport and its SNMP version, when it is SNMP.
    pub transport: Transport,
    /// The credential kind the core resolves from the vault.
    pub credential_ref: CredentialRef,
}

/// The closed set of transports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// SNMP v1: `noSuchName` is the absent-table answer.
    SnmpV1,
    /// SNMP v2c: exception values are the absent-table answer.
    SnmpV2c,
    /// The RouterOS binary API, read-only print commands only.
    #[serde(rename = "routeros_api")]
    RouterOsApi,
}

/// The closed set of credential kinds a surface may reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialRef {
    /// An SNMP community from the vault's `credential` reference.
    Snmp,
    /// A RouterOS API user/password from the vault's `api_credential`.
    Api,
}

/// One independent read: scalars, one table, or one named command.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    /// The probe's identifier, unique within the document.
    pub id: String,
    /// The surface id this probe reads through.
    pub surface: String,
    /// The read itself.
    pub read: Read,
    /// Whether the read must answer for the device to be supported.
    #[serde(default)]
    pub on_absent: OnAbsent,
}

/// What a probe's absent answer means for the device.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnAbsent {
    /// The read must answer; an absent, timed-out, errored, or cancelled
    /// answer leaves the device unsupported.
    #[default]
    Required,
    /// The read may be absent without leaving the device unsupported.
    Optional,
}

/// The closed set of reads. Each probe is one independent read: no steps,
/// no chaining, no conditional follow-ups.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Read {
    /// Scalar OIDs, each decoded by its parser.
    Scalars {
        /// The scalars to read, at least one.
        scalars: Vec<Scalar>,
    },
    /// One walked table.
    Table {
        /// The table definition.
        table: Table,
    },
    /// One core-registered read-only RouterOS command with a field map.
    Command {
        /// The command definition.
        command: Command,
    },
}

/// One scalar OID and the parser that decodes its value.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scalar {
    /// The field name other maps reference.
    pub name: String,
    /// The scalar's full dotted OID, e.g. `1.3.6.1.2.1.1.1.0`.
    pub oid: String,
    /// How the value is decoded.
    pub parser: Parser,
}

/// One walked table: columns, the index layout, and the bridge-port
/// resolution the table's port values need.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// The table's columns, at least one, each named uniquely.
    pub columns: Vec<Column>,
    /// How a row's instance OID is read.
    pub index: Index,
    /// How a bridge-port value becomes an `ifIndex`; required for FDB
    /// layouts, rejected elsewhere.
    #[serde(default)]
    pub bridge_port_resolution: Option<BridgePortResolution>,
}

/// One table column and the parser that decodes its values.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    /// The field name other maps reference.
    pub name: String,
    /// The column's full dotted OID, e.g. `1.3.6.1.2.1.2.2.1.2`.
    pub oid: String,
    /// How the value is decoded.
    pub parser: Parser,
}

/// The index layout of a table: a closed set of instance shapes the core
/// knows how to read.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    /// The layout kind.
    pub kind: IndexKind,
}

/// The closed set of instance layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexKind {
    /// `<ifIndex>` or any single unsigned index.
    Single,
    /// LLDP remote-systems: `<entry>.<local-port>`; the core also reads the
    /// management-address and local-port tables.
    LldpRem,
    /// `ipNetToMediaTable`: `<ifIndex>.<a>.<b>.<c>.<d>`.
    ArpV4,
    /// `ipAddrTable`: `<a>.<b>.<c>.<d>`; the index is the IPv4 address.
    IpAddressV4,
    /// `ipNetToPhysicalTable`: `<ifIndex>.<address-type>.<address octets…>`.
    ArpTyped,
    /// BRIDGE `dot1dTpFdbTable`: `<m1>…<m6>`.
    Dot1dMac,
    /// BRIDGE with a row-id index: `<row-id>`, MAC in a column.
    Dot1dRowId,
    /// Q-BRIDGE `dot1qTpFdbTable`: `<vlan>.<m1>…<m6>`.
    Dot1qVlanMac,
    /// airOS stations: `<wlan-if>.<m1>…<m6>` in decimal octets.
    UbntStation,
    /// Vendor AP stations: `<index>` (one unsigned integer, e.g. an ePMP
    /// association id); the MAC is the required `mac` column.
    Station,
    /// Vendor AP stations indexed by their six-octet MAC
    /// (`<m1>…<m6>`); a trailing discriminator such as the Wave family's
    /// `<m1>…<m6>.<slot>` is allowed and ignored. The MAC is also the
    /// required `mac` column, which must agree with the index.
    StationMac,
}

/// How an FDB port value becomes an `ifIndex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgePortResolution {
    /// The bridge port number is the `ifIndex`.
    Identity,
    /// The bridge port joins `dot1dBasePortIfIndex`; never substituted.
    Dot1dBasePortIfindex,
}

/// One core-registered read-only RouterOS command and its field map.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    /// The core operation, from a closed list.
    pub name: CommandName,
    /// The fields to keep, at least one, each named uniquely.
    pub fields: Vec<CommandField>,
}

/// One command field: the document's name and the RouterOS reply key it
/// reads.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandField {
    /// The field name other maps reference.
    pub name: String,
    /// The RouterOS reply key, e.g. `mac-address`.
    pub from: String,
}

/// How probe fields become claims and details, plus the documented
/// residuals a family carries.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Maps {
    /// Claims, in document order.
    #[serde(default)]
    pub claims: Vec<ClaimMap>,
    /// Device-detail slots and the probe that feeds each.
    #[serde(default)]
    pub details: Details,
    /// Documented leftovers: named, with a source note, and never executed.
    #[serde(default)]
    pub residuals: Vec<Residual>,
}

/// One claim mapping: a claim kind and the fields that state it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimMap {
    /// The claim kind.
    pub kind: ClaimKind,
    /// Field references, each `"<probe-id>.<field>"`; at least one.
    pub fields: Vec<String>,
}

/// The closed set of claim kinds a document may map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    /// A routerboard serial number.
    Serial,
    /// An IP address.
    Address,
    /// A device name.
    Name,
    /// The system description.
    Sysdescr,
    /// A neighbour's own port id.
    Portid,
    /// The observer-local port.
    Localport,
    /// The local interface a neighbour was seen on.
    Interface,
}

impl Transport {
    /// The document spelling of the transport.
    fn name(self) -> &'static str {
        match self {
            Self::SnmpV1 => "snmp_v1",
            Self::SnmpV2c => "snmp_v2c",
            Self::RouterOsApi => "routeros_api",
        }
    }
}

impl CredentialRef {
    /// The document spelling of the credential kind.
    fn name(self) -> &'static str {
        match self {
            Self::Snmp => "snmp",
            Self::Api => "api",
        }
    }
}

impl IndexKind {
    /// The document spelling of the layout.
    fn name(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::LldpRem => "lldp_rem",
            Self::ArpV4 => "arp_v4",
            Self::IpAddressV4 => "ip_address_v4",
            Self::ArpTyped => "arp_typed",
            Self::Dot1dMac => "dot1d_mac",
            Self::Dot1dRowId => "dot1d_row_id",
            Self::Dot1qVlanMac => "dot1q_vlan_mac",
            Self::UbntStation => "ubnt_station",
            Self::Station => "station",
            Self::StationMac => "station_mac",
        }
    }
}

impl ClaimKind {
    /// The model kind this document kind maps to.
    pub fn model(self) -> crate::identity::ClaimKind {
        match self {
            Self::Serial => crate::identity::ClaimKind::Serial,
            Self::Address => crate::identity::ClaimKind::Address,
            Self::Name => crate::identity::ClaimKind::Name,
            Self::Sysdescr => crate::identity::ClaimKind::SysDescr,
            Self::Portid => crate::identity::ClaimKind::PortId,
            Self::Localport => crate::identity::ClaimKind::LocalPort,
            Self::Interface => crate::identity::ClaimKind::Interface,
        }
    }
}

/// Device-detail slots: each names the probe whose observation feeds it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Details {
    /// Interfaces, from a table or command probe.
    #[serde(default)]
    pub interfaces: Option<String>,
    /// System facts, from a scalars or command probe.
    #[serde(default)]
    pub system: Option<String>,
    /// Temperature sensors, from a table or command probe.
    #[serde(default)]
    pub temperatures: Option<String>,
    /// Associated stations, from a table with the `ubnt_station`,
    /// `station`, or `station_mac` layout.
    #[serde(default)]
    pub stations: Option<String>,
    /// The radio's operating parameters, from a scalars probe.
    #[serde(default)]
    pub radio: Option<String>,
    /// ARP bindings, from a table or command probe.
    #[serde(default)]
    pub arp: Option<String>,
    /// Bridge-port memberships, from a table or command probe.
    #[serde(default)]
    pub bridge_ports: Option<String>,
    /// Bridge VLAN configuration, from a table or command probe.
    #[serde(default)]
    pub bridge_vlans: Option<String>,
    /// Bridge learned hosts, from a table or command probe.
    #[serde(default)]
    pub bridge_hosts: Option<String>,
    /// Firewall NAT rules, from a command probe.
    #[serde(default)]
    pub nat: Option<String>,
    /// DHCP server leases, from a command probe.
    #[serde(default)]
    pub dhcp_leases: Option<String>,
}

/// One documented residual: a shape the schema does not express, named with
/// where it came from. Data only; nothing reads it at runtime.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Residual {
    /// A short identifier, unique within the document.
    pub name: String,
    /// Where the residual is described (recon file, fixture, or issue).
    pub source: String,
}

/// A document's limits. A missing value means the core default; a present
/// value must be at least one and no more than the core ceiling.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Total requests across the document's probes.
    #[serde(default)]
    pub max_requests: Option<u32>,
    /// Total rows across the document's tables.
    #[serde(default)]
    pub max_rows: Option<u32>,
    /// Total response bytes across the document's probes.
    #[serde(default)]
    pub max_bytes: Option<u64>,
    /// Wall-clock deadline for one probe pass, in milliseconds.
    #[serde(default)]
    pub deadline_ms: Option<u64>,
}

/// One validation failure: the document it came from, the adapter and probe
/// it belongs to when known, the JSON path (or line/column for a parse
/// error), and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// The document path or name the caller supplied.
    pub document: String,
    /// The adapter id, when the document parsed far enough to have one.
    pub adapter: Option<String>,
    /// The probe id, when the failure belongs to one.
    pub probe: Option<String>,
    /// The JSON path inside the document, or `line L, column C` for a parse
    /// failure.
    pub path: String,
    /// What is wrong, naming the offending value.
    pub reason: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.document)?;
        if let Some(adapter) = &self.adapter {
            write!(f, " (adapter `{adapter}`)")?;
        }
        if let Some(probe) = &self.probe {
            write!(f, " (probe `{probe}`)")?;
        }
        write!(f, " at {}: {}", self.path, self.reason)
    }
}

impl std::error::Error for ValidationError {}

/// Parses and validates one document. `document` is the path or name the
/// error messages carry.
///
/// A JSON or type error stops at the parse; a well-formed document is then
/// checked against every structural rule and all failures are returned.
pub fn parse(document: &str, json: &str) -> Result<Document, Vec<ValidationError>> {
    let parsed = parse_document(document, json).map_err(|error| vec![error])?;
    validate(document, &parsed)?;
    Ok(parsed)
}

/// Parses one document's JSON without validating it. `document` labels a
/// parse error; the registry loader parses every file first so one report
/// can name them all.
fn parse_document(document: &str, json: &str) -> Result<Document, ValidationError> {
    serde_json::from_str(json).map_err(|error| ValidationError {
        document: document.to_owned(),
        adapter: None,
        probe: None,
        path: format!("line {}, column {}", error.line(), error.column()),
        reason: error.to_string(),
    })
}

/// Validates one parsed document, returning every failure it finds.
pub fn validate(document: &str, doc: &Document) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    let mut fail = |probe: Option<&str>, path: String, reason: String| {
        errors.push(ValidationError {
            document: document.to_owned(),
            adapter: Some(doc.id.clone()),
            probe: probe.map(str::to_owned),
            path,
            reason,
        });
    };

    if doc.schema_version != SCHEMA_VERSION {
        fail(
            None,
            "schema_version".to_owned(),
            format!(
                "unsupported schema version {}; this build reads {SCHEMA_VERSION}",
                doc.schema_version
            ),
        );
    }
    if !is_slug(&doc.id) {
        fail(
            None,
            "id".to_owned(),
            format!(
                "adapter id `{}` must be non-empty ASCII letters, digits, `-`, or `_`",
                doc.id
            ),
        );
    }

    // A document states either hints or `default: true`, never both: hints
    // next to a default are ambiguous intent.
    let has_hints = !doc.detect.sys_object_id_exact.is_empty()
        || !doc.detect.sys_object_id_prefixes.is_empty()
        || !doc.detect.sys_descr_contains.is_empty();
    if doc.detect.pin_only {
        if doc.detect.default {
            fail(
                None,
                "detect.pin_only".to_owned(),
                "a pin-only document is never the default".to_owned(),
            );
        }
        if has_hints {
            fail(
                None,
                "detect.pin_only".to_owned(),
                "a pin-only document declares no detection hints".to_owned(),
            );
        }
    } else {
        if doc.detect.default && has_hints {
            fail(
                None,
                "detect.default".to_owned(),
                "`detect.default` is true; the default document must declare no detection hints"
                    .to_owned(),
            );
        }
        if !doc.detect.default && !has_hints {
            fail(
                None,
                "detect".to_owned(),
                "at least one sys_object_id_exact, sys_object_id_prefixes or sys_descr_contains \
                 hint, `pin_only: true`, or `default: true`, is required"
                    .to_owned(),
            );
        }
    }
    for (index, exact) in doc.detect.sys_object_id_exact.iter().enumerate() {
        if !is_oid(exact) {
            fail(
                None,
                format!("detect.sys_object_id_exact[{index}]"),
                format!("`{exact}` is not a dotted OID"),
            );
        }
    }
    for (index, prefix) in doc.detect.sys_object_id_prefixes.iter().enumerate() {
        if !is_oid(prefix) {
            fail(
                None,
                format!("detect.sys_object_id_prefixes[{index}]"),
                format!("`{prefix}` is not a dotted OID"),
            );
        }
    }
    for (index, text) in doc.detect.sys_descr_contains.iter().enumerate() {
        if text.is_empty() || text.chars().any(char::is_control) {
            fail(
                None,
                format!("detect.sys_descr_contains[{index}]"),
                "the substring must be non-empty and contain no control characters".to_owned(),
            );
        }
    }

    let mut surface_ids = BTreeSet::new();
    for (index, surface) in doc.surfaces.iter().enumerate() {
        if !is_slug(&surface.id) {
            fail(
                None,
                format!("surfaces[{index}].id"),
                format!("surface id `{}` is not a slug", surface.id),
            );
        }
        if !surface_ids.insert(surface.id.clone()) {
            fail(
                None,
                format!("surfaces[{index}].id"),
                format!("duplicate surface id `{}`", surface.id),
            );
        }
        let expected = match surface.transport {
            Transport::SnmpV1 | Transport::SnmpV2c => CredentialRef::Snmp,
            Transport::RouterOsApi => CredentialRef::Api,
        };
        if surface.credential_ref != expected {
            fail(
                None,
                format!("surfaces[{index}].credential_ref"),
                format!(
                    "transport {} requires credential_ref {}",
                    surface.transport.name(),
                    expected.name()
                ),
            );
        }
    }
    if doc.surfaces.is_empty() {
        fail(
            None,
            "surfaces".to_owned(),
            "at least one surface is required".to_owned(),
        );
    }

    if doc.probes.is_empty() {
        fail(
            None,
            "probes".to_owned(),
            "at least one probe is required".to_owned(),
        );
    }
    let mut probe_ids = BTreeSet::new();
    let mut fields_by_probe: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut reads_by_probe: BTreeMap<&str, &Read> = BTreeMap::new();
    for (index, probe) in doc.probes.iter().enumerate() {
        let path = format!("probes[{index}]");
        if !is_slug(&probe.id) {
            fail(
                Some(&probe.id),
                format!("{path}.id"),
                format!("probe id `{}` is not a slug", probe.id),
            );
        }
        if !probe_ids.insert(probe.id.clone()) {
            fail(
                Some(&probe.id),
                format!("{path}.id"),
                format!("duplicate probe id `{}`", probe.id),
            );
        }
        if !surface_ids.contains(&probe.surface) {
            fail(
                Some(&probe.id),
                format!("{path}.surface"),
                format!("unknown surface `{}`", probe.surface),
            );
        }
        let mut fields = BTreeSet::new();
        match &probe.read {
            Read::Scalars { scalars } => {
                if scalars.is_empty() {
                    fail(
                        Some(&probe.id),
                        format!("{path}.read.scalars"),
                        "at least one scalar is required".to_owned(),
                    );
                }
                for (column, scalar) in scalars.iter().enumerate() {
                    check_field(
                        &mut fail,
                        &probe.id,
                        &format!("{path}.read.scalars[{column}]"),
                        &scalar.name,
                        &scalar.oid,
                        &mut fields,
                    );
                    if scalar.parser == Parser::DisplayMac {
                        fail(
                            Some(&probe.id),
                            format!("{path}.read.scalars[{column}].parser"),
                            "the display_mac parser is only valid on a `mac` table column"
                                .to_owned(),
                        );
                    }
                }
            }
            Read::Table { table } => {
                check_table(&mut fail, probe, &path, table, &mut fields);
            }
            Read::Command { command } => {
                if command.fields.is_empty() {
                    fail(
                        Some(&probe.id),
                        format!("{path}.read.command.fields"),
                        "at least one command field is required".to_owned(),
                    );
                }
                for (field, mapped) in command.fields.iter().enumerate() {
                    if !is_slug(&mapped.name) {
                        fail(
                            Some(&probe.id),
                            format!("{path}.read.command.fields[{field}].name"),
                            format!("field name `{}` is not a slug", mapped.name),
                        );
                    }
                    if mapped.from.is_empty() {
                        fail(
                            Some(&probe.id),
                            format!("{path}.read.command.fields[{field}].from"),
                            "the RouterOS reply key must not be empty".to_owned(),
                        );
                    }
                    if !fields.insert(mapped.name.as_str()) {
                        fail(
                            Some(&probe.id),
                            format!("{path}.read.command.fields[{field}].name"),
                            format!("duplicate field name `{}`", mapped.name),
                        );
                    }
                }
            }
        }
        fields_by_probe.insert(probe.id.as_str(), fields);
        reads_by_probe.insert(probe.id.as_str(), &probe.read);
    }

    for (index, claim) in doc.maps.claims.iter().enumerate() {
        if claim.fields.is_empty() {
            fail(
                None,
                format!("maps.claims[{index}].fields"),
                "at least one field reference is required".to_owned(),
            );
        }
        for (field, reference) in claim.fields.iter().enumerate() {
            let path = format!("maps.claims[{index}].fields[{field}]");
            let Some((probe, name)) = reference.split_once('.') else {
                fail(
                    None,
                    path,
                    format!("`{reference}` is not a `<probe>.<field>` reference"),
                );
                continue;
            };
            match fields_by_probe.get(probe) {
                None => fail(None, path, format!("unknown probe `{probe}`")),
                Some(fields) if !fields.contains(name) => fail(
                    None,
                    path,
                    format!("probe `{probe}` declares no field `{name}`"),
                ),
                Some(_) => {}
            }
        }
    }

    let mut residual_names = BTreeSet::new();
    for (index, residual) in doc.maps.residuals.iter().enumerate() {
        if !is_slug(&residual.name) {
            fail(
                None,
                format!("maps.residuals[{index}].name"),
                format!("residual name `{}` is not a slug", residual.name),
            );
        }
        if !residual_names.insert(residual.name.as_str()) {
            fail(
                None,
                format!("maps.residuals[{index}].name"),
                format!("duplicate residual name `{}`", residual.name),
            );
        }
        if residual.source.is_empty() {
            fail(
                None,
                format!("maps.residuals[{index}].source"),
                "the source note must not be empty".to_owned(),
            );
        }
    }

    for (slot, reference) in detail_slots(&doc.maps.details) {
        let path = format!("maps.details.{slot}");
        let Some(read) = reads_by_probe.get(reference.as_str()) else {
            fail(None, path, format!("unknown probe `{reference}`"));
            continue;
        };
        if let Err(reason) = detail_compatible(slot, read) {
            fail(None, path, reason);
            continue;
        }
        // A command probe feeds exactly the one slot its command shapes; any
        // other reference would map nothing at runtime.
        if let Read::Command { command } = read {
            match detail_slot(command.name) {
                Some(shaped) if shaped == slot => {}
                Some(shaped) => fail(
                    None,
                    path,
                    format!("command probe `{reference}` feeds `{shaped}`, not `{slot}`"),
                ),
                None => fail(
                    None,
                    path,
                    format!("command probe `{reference}` feeds no detail slot"),
                ),
            }
        }
    }

    check_budget(&mut fail, &doc.budget);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validates a registry: every document must be valid on its own, the
/// adapter ids must be unique across the registry, and at most one document
/// may be the default.
pub fn validate_registry(documents: &[(String, Document)]) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    let mut ids: BTreeMap<&str, &str> = BTreeMap::new();
    let mut default_document: Option<&str> = None;
    for (document, doc) in documents {
        if let Err(mut document_errors) = validate(document, doc) {
            errors.append(&mut document_errors);
        }
        if let Some(first) = ids.insert(doc.id.as_str(), document.as_str()) {
            errors.push(ValidationError {
                document: document.clone(),
                adapter: Some(doc.id.clone()),
                probe: None,
                path: "id".to_owned(),
                reason: format!(
                    "duplicate adapter id `{}` (first declared in {first})",
                    doc.id
                ),
            });
        }
        if doc.detect.default {
            match default_document {
                Some(first) => errors.push(ValidationError {
                    document: document.clone(),
                    adapter: Some(doc.id.clone()),
                    probe: None,
                    path: "detect.default".to_owned(),
                    reason: format!(
                        "a second default document; {first} already declares one, and a registry \
                         may have only one"
                    ),
                }),
                None => default_document = Some(document.as_str()),
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Checks one scalar or column: a slug name, a dotted OID, and no duplicate
/// name within the probe.
fn check_field<'a>(
    fail: &mut impl FnMut(Option<&str>, String, String),
    probe: &str,
    path: &str,
    name: &'a str,
    oid: &str,
    fields: &mut BTreeSet<&'a str>,
) {
    if !is_slug(name) {
        fail(
            Some(probe),
            format!("{path}.name"),
            format!("field name `{name}` is not a slug"),
        );
    }
    if !is_oid(oid) {
        fail(
            Some(probe),
            format!("{path}.oid"),
            format!("`{oid}` is not a dotted OID"),
        );
    }
    if !fields.insert(name) {
        fail(
            Some(probe),
            format!("{path}.name"),
            format!("duplicate field name `{name}`"),
        );
    }
}

/// Checks a table: columns, layout, and the column a layout requires.
fn check_table<'a>(
    fail: &mut impl FnMut(Option<&str>, String, String),
    probe: &Probe,
    path: &str,
    table: &'a Table,
    fields: &mut BTreeSet<&'a str>,
) {
    if table.columns.is_empty() {
        fail(
            Some(&probe.id),
            format!("{path}.read.table.columns"),
            "at least one column is required".to_owned(),
        );
    }
    let mut parsers: BTreeMap<&'a str, Parser> = BTreeMap::new();
    for (index, column) in table.columns.iter().enumerate() {
        let column_path = format!("{path}.read.table.columns[{index}]");
        check_field(
            fail,
            &probe.id,
            &column_path,
            &column.name,
            &column.oid,
            fields,
        );
        parsers.entry(column.name.as_str()).or_insert(column.parser);
    }
    let kind = table.index.kind;
    let fdb = matches!(
        kind,
        IndexKind::Dot1dMac | IndexKind::Dot1dRowId | IndexKind::Dot1qVlanMac
    );
    match (fdb, table.bridge_port_resolution) {
        (true, None) => fail(
            Some(&probe.id),
            format!("{path}.read.table.bridge_port_resolution"),
            format!(
                "the {} layout requires a bridge_port_resolution",
                kind.name()
            ),
        ),
        (false, Some(_)) => fail(
            Some(&probe.id),
            format!("{path}.read.table.bridge_port_resolution"),
            format!("the {} layout has no bridge ports to resolve", kind.name()),
        ),
        _ => {}
    }
    // Each layout owns a closed column vocabulary: a name the layout cannot
    // read is an error, not an ignored column.
    let (allowed, required): (&[&str], &[&str]) = match kind {
        IndexKind::Single => (&[], &[]),
        IndexKind::LldpRem => (
            &[
                "chassis_id",
                "port_id",
                "sys_name",
                "sys_descr",
                "local_port",
            ],
            &["chassis_id"],
        ),
        IndexKind::ArpV4 | IndexKind::ArpTyped => (&["mac"], &["mac"]),
        IndexKind::IpAddressV4 => (&["if_index", "netmask"], &["if_index"]),
        IndexKind::Dot1dMac | IndexKind::Dot1qVlanMac => (&["mac", "port", "status"], &["port"]),
        IndexKind::Dot1dRowId => (&["mac", "port", "status"], &["mac", "port"]),
        IndexKind::UbntStation => (
            &[
                "hostname",
                "signal_dbm",
                "noise_dbm",
                "last_ip",
                "tx_bps",
                "rx_bps",
                "rx_bytes",
                "tx_bytes",
                "uptime",
                "distance_ccq",
                "quality",
            ],
            &[],
        ),
        IndexKind::Station | IndexKind::StationMac => (
            &[
                "mac",
                "ip",
                "channel",
                "ul_rssi",
                "dl_rssi",
                "ul_snr",
                "dl_snr",
                "tx_capacity",
                "rx_capacity",
                "tx_quality",
                "dl_rate_mbps",
                "model",
                "software_version",
                "hostname",
            ],
            &["mac"],
        ),
    };
    for (name, parser) in &parsers {
        if *parser == Parser::DisplayMac && *name != "mac" {
            fail(
                Some(&probe.id),
                format!("{path}.read.table.columns"),
                format!("the display_mac parser is only valid on a `mac` column, not `{name}`"),
            );
        }
    }
    for name in parsers.keys() {
        if !allowed.is_empty() && !allowed.contains(name) {
            fail(
                Some(&probe.id),
                format!("{path}.read.table.columns"),
                format!("column `{name}` is not part of the {} layout", kind.name()),
            );
        }
    }
    for name in required {
        if !parsers.contains_key(name) {
            fail(
                Some(&probe.id),
                format!("{path}.read.table.columns"),
                format!("the {} layout requires a `{name}` column", kind.name()),
            );
        }
    }
    if kind == IndexKind::LldpRem && parsers.get("chassis_id") != Some(&Parser::OctetMac) {
        fail(
            Some(&probe.id),
            format!("{path}.read.table.columns"),
            "the lldp_rem layout requires a `chassis_id` column with parser `octet_mac`".to_owned(),
        );
    }
    if matches!(
        kind,
        IndexKind::ArpV4 | IndexKind::ArpTyped | IndexKind::Dot1dRowId
    ) && parsers.get("mac") != Some(&Parser::OctetMac)
    {
        fail(
            Some(&probe.id),
            format!("{path}.read.table.columns"),
            format!(
                "the {} layout requires a `mac` column with parser `octet_mac`",
                kind.name()
            ),
        );
    }
    if kind == IndexKind::Station && parsers.get("mac") != Some(&Parser::DisplayMac) {
        fail(
            Some(&probe.id),
            format!("{path}.read.table.columns"),
            "the station layout requires a `mac` column with parser `display_mac`".to_owned(),
        );
    }
    if kind == IndexKind::StationMac && parsers.get("mac") != Some(&Parser::OctetMac) {
        fail(
            Some(&probe.id),
            format!("{path}.read.table.columns"),
            "the station_mac layout requires a `mac` column with parser `octet_mac`".to_owned(),
        );
    }
    if kind == IndexKind::IpAddressV4 {
        if parsers.get("if_index") != Some(&Parser::U32) {
            fail(
                Some(&probe.id),
                format!("{path}.read.table.columns"),
                "the ip_address_v4 layout requires an `if_index` column with parser `u32`"
                    .to_owned(),
            );
        }
        if parsers
            .get("netmask")
            .is_some_and(|parser| *parser != Parser::IpAddress)
        {
            fail(
                Some(&probe.id),
                format!("{path}.read.table.columns"),
                "the ip_address_v4 layout requires a `netmask` column with parser `ip_address`"
                    .to_owned(),
            );
        }
    }
}

/// The closed set of value parsers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Parser {
    /// An OCTET STRING decoded as UTF-8 text.
    Utf8,
    /// An unsigned integer (INTEGER, Unsigned32, or Gauge32).
    U32,
    /// A signed integer (INTEGER).
    I64,
    /// An unsigned 64-bit counter (Counter64 or TimeTicks).
    U64,
    /// An OBJECT IDENTIFIER as dotted text.
    Oid,
    /// A six-octet OCTET STRING as a MAC.
    OctetMac,
    /// A DisplayString MAC in `xx:xx:…` or `xx-xx-…` form (ASCII hex,
    /// case-insensitive); only valid on a `mac` table column.
    DisplayMac,
    /// The ePMP channel-width enum, decoded to MHz:
    /// `1=20, 2=40, 3=10, 4=5, 5=80, 6=160, 7=320`.
    ChannelWidthMhz,
    /// A capacity in Kbps, decoded to whole Mbps (nearest, half up).
    KbpsToMbps,
    /// A DisplayString rate with an `M` suffix, e.g. `103M`, decoded to
    /// whole Mbps; any other text is absent.
    RateSuffix,
    /// An IpAddress as dotted text.
    IpAddress,
}

/// Every device-detail slot with its probe reference.
fn detail_slots(details: &Details) -> Vec<(&'static str, &String)> {
    let mut slots = Vec::new();
    if let Some(reference) = &details.interfaces {
        slots.push(("interfaces", reference));
    }
    if let Some(reference) = &details.system {
        slots.push(("system", reference));
    }
    if let Some(reference) = &details.temperatures {
        slots.push(("temperatures", reference));
    }
    if let Some(reference) = &details.stations {
        slots.push(("stations", reference));
    }
    if let Some(reference) = &details.radio {
        slots.push(("radio", reference));
    }
    if let Some(reference) = &details.arp {
        slots.push(("arp", reference));
    }
    if let Some(reference) = &details.bridge_ports {
        slots.push(("bridge_ports", reference));
    }
    if let Some(reference) = &details.bridge_vlans {
        slots.push(("bridge_vlans", reference));
    }
    if let Some(reference) = &details.bridge_hosts {
        slots.push(("bridge_hosts", reference));
    }
    if let Some(reference) = &details.nat {
        slots.push(("nat", reference));
    }
    if let Some(reference) = &details.dhcp_leases {
        slots.push(("dhcp_leases", reference));
    }
    slots
}

/// Whether a probe's read can feed a detail slot.
fn detail_compatible(slot: &str, read: &Read) -> Result<(), String> {
    let ok = match slot {
        "system" => matches!(read, Read::Scalars { .. } | Read::Command { .. }),
        "stations" => matches!(
            read,
            Read::Table { table }
                if matches!(
                    table.index.kind,
                    IndexKind::UbntStation | IndexKind::Station | IndexKind::StationMac
                )
        ),
        "radio" => matches!(read, Read::Scalars { .. }),
        "arp" => {
            matches!(
                read,
                Read::Table { table }
                    if matches!(table.index.kind, IndexKind::ArpV4 | IndexKind::ArpTyped)
            ) || matches!(read, Read::Command { .. })
        }
        "nat" => matches!(
            read,
            Read::Command { command } if command.name == CommandName::IpFirewallNat
        ),
        "dhcp_leases" => matches!(
            read,
            Read::Command { command } if command.name == CommandName::IpDhcpServerLease
        ),
        _ => matches!(read, Read::Table { .. } | Read::Command { .. }),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("`{slot}` cannot be fed by this read kind"))
    }
}

/// Checks the budget's floor and ceiling for every present limit.
fn check_budget(fail: &mut impl FnMut(Option<&str>, String, String), budget: &Budget) {
    check_limit(
        fail,
        "max_requests",
        budget.max_requests.map(u64::from),
        1,
        u64::from(MAX_REQUESTS_CEILING),
    );
    check_limit(
        fail,
        "max_rows",
        budget.max_rows.map(u64::from),
        1,
        u64::from(MAX_ROWS_CEILING),
    );
    check_limit(fail, "max_bytes", budget.max_bytes, 1, MAX_BYTES_CEILING);
    check_limit(
        fail,
        "deadline_ms",
        budget.deadline_ms,
        1,
        MAX_DEADLINE_MS_CEILING,
    );
}

/// One budget limit: at least `floor`, at most `ceiling`.
fn check_limit(
    fail: &mut impl FnMut(Option<&str>, String, String),
    name: &str,
    value: Option<u64>,
    floor: u64,
    ceiling: u64,
) {
    if let Some(value) = value
        && !(floor..=ceiling).contains(&value)
    {
        fail(
            None,
            format!("budget.{name}"),
            format!("{value} is outside the allowed range {floor}..={ceiling}"),
        );
    }
}

/// Whether `text` is a document slug: ASCII letters, digits, `-`, `_`, and
/// not empty.
fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

/// Whether `text` is a dotted OID: an optional leading dot, then at least
/// two arcs of ASCII digits that fit a `u32`. A single arc is not an SNMP
/// object identifier, and a sign or any other character is not an arc
/// (`u32::from_str` would accept a leading `+`).
fn is_oid(text: &str) -> bool {
    let text = text.strip_prefix('.').unwrap_or(text);
    let arcs: Vec<&str> = text.split('.').collect();
    arcs.len() >= 2
        && arcs.iter().all(|arc| {
            !arc.is_empty()
                && arc.bytes().all(|byte| byte.is_ascii_digit())
                && arc.parse::<u32>().is_ok()
        })
}

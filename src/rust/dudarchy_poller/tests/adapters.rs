//! Adapter schema v2 corpus: every valid document validates, and every
//! invalid one fails with the named path and reason. The corpus is the
//! schema's executable specification; B3 grows it into replay fixtures.

use std::path::{Path, PathBuf};

use dudarchy_poller::adapters;
use dudarchy_poller::adapters::registry::{Registry, RegistryError, RegistryState, directory_for};

/// The adapter fixture tree.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adapters")
}

/// Reads one fixture document.
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Parses and validates one fixture, naming the document by its file name.
fn parse_file(path: &Path) -> Result<adapters::Document, Vec<adapters::ValidationError>> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("fixture file names are UTF-8");
    adapters::parse(name, &read(path))
}

/// Every error, one per line, for assertion messages.
fn messages(errors: &[adapters::ValidationError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// One invalid fixture and the path and reason its rejection must name.
const INVALID: &[(&str, &str, &str)] = &[
    ("unknown-field.json", "line ", "unknown field `detectt`"),
    (
        "unknown-transport.json",
        "line ",
        "unknown variant `snmp_v3`",
    ),
    ("unknown-parser.json", "line ", "unknown variant `regex`"),
    (
        "unknown-detail-slot.json",
        "line ",
        "unknown field `firmware`",
    ),
    ("read-extra-field.json", "line ", "unknown field `extra`"),
    ("wrong-type.json", "line ", "invalid type"),
    (
        "duplicate-probe.json",
        "probes[1].id",
        "duplicate probe id `system`",
    ),
    (
        "duplicate-surface.json",
        "surfaces[1].id",
        "duplicate surface id `snmp`",
    ),
    (
        "unknown-surface-ref.json",
        "probes[0].surface",
        "unknown surface `smtp`",
    ),
    (
        "dangling-claim.json",
        "maps.claims[0].fields[0]",
        "probe `system` declares no field `nope`",
    ),
    (
        "dangling-detail.json",
        "maps.details.interfaces",
        "unknown probe `if_tabel`",
    ),
    (
        "incompatible-credential.json",
        "surfaces[0].credential_ref",
        "requires credential_ref",
    ),
    (
        "resolution-on-single.json",
        "probes[0].read.table.bridge_port_resolution",
        "has no bridge ports to resolve",
    ),
    (
        "fdb-without-resolution.json",
        "probes[0].read.table.bridge_port_resolution",
        "requires a bridge_port_resolution",
    ),
    (
        "rowid-without-mac.json",
        "probes[0].read.table.columns",
        "requires a `mac` column with parser `octet_mac`",
    ),
    (
        "budget-over-ceiling.json",
        "budget.max_rows",
        "outside the allowed range",
    ),
    (
        "unknown-schema-version.json",
        "schema_version",
        "unsupported schema version 3",
    ),
    ("no-detect.json", "detect", "at least one"),
    (
        "default-with-hints.json",
        "detect.default",
        "must declare no detection hints",
    ),
    (
        "bad-oid.json",
        "probes[0].read.scalars[0].oid",
        "is not a dotted OID",
    ),
    (
        "short-oid.json",
        "probes[0].read.scalars[0].oid",
        "is not a dotted OID",
    ),
    (
        "stations-wrong-layout.json",
        "maps.details.stations",
        "cannot be fed by this read kind",
    ),
    (
        "station-without-mac.json",
        "probes[0].read.table.columns",
        "the station layout requires a `mac` column",
    ),
    (
        "station-mac-parser.json",
        "probes[0].read.table.columns",
        "requires a `mac` column with parser `display_mac`",
    ),
    (
        "station-mac-octet-mac.json",
        "probes[0].read.table.columns",
        "the station_mac layout requires a `mac` column with parser `octet_mac`",
    ),
    (
        "pin-only-with-hints.json",
        "detect.pin_only",
        "a pin-only document declares no detection hints",
    ),
    (
        "display-mac-on-non-mac.json",
        "probes[0].read.table.columns",
        "the display_mac parser is only valid on a `mac` column",
    ),
    (
        "display-mac-scalar.json",
        "probes[0].read.scalars[0].parser",
        "the display_mac parser is only valid on a `mac` table column",
    ),
    (
        "radio-wrong-read.json",
        "maps.details.radio",
        "cannot be fed by this read kind",
    ),
    (
        "empty-scalars.json",
        "probes[0].read.scalars",
        "at least one scalar",
    ),
    (
        "unknown-layout-column.json",
        "probes[0].read.table.columns",
        "column `vendor_flag` is not part of the lldp_rem layout",
    ),
    (
        "arp-wrong-layout.json",
        "maps.details.arp",
        "cannot be fed by this read kind",
    ),
    (
        "ip-address-without-if-index.json",
        "probes[0].read.table.columns",
        "requires an `if_index` column",
    ),
    (
        "ip-address-wrong-parsers.json",
        "probes[0].read.table.columns",
        "requires an `if_index` column with parser `u32`",
    ),
    (
        "ip-address-wrong-netmask-parser.json",
        "probes[0].read.table.columns",
        "requires a `netmask` column with parser `ip_address`",
    ),
    (
        "command-empty-fields.json",
        "probes[0].read.command.fields",
        "at least one command field is required",
    ),
    (
        "command-duplicate-field.json",
        "probes[0].read.command.fields[1].name",
        "duplicate field name `name`",
    ),
    (
        "command-empty-from.json",
        "probes[0].read.command.fields[0].from",
        "must not be empty",
    ),
    (
        "command-unknown-name.json",
        "line ",
        "unknown variant `format_disk`",
    ),
    (
        "command-interface-vlan.json",
        "line ",
        "unknown variant `interface_vlan`",
    ),
    (
        "detail-command-wrong-slot.json",
        "maps.details.system",
        "feeds no detail slot",
    ),
    (
        "detail-nat-table.json",
        "maps.details.nat",
        "cannot be fed by this read kind",
    ),
    (
        "detail-dhcp-table.json",
        "maps.details.dhcp_leases",
        "cannot be fed by this read kind",
    ),
    (
        "required-column-missing.json",
        "probes[0].read.table.columns",
        "requires a `port` column",
    ),
    (
        "lldp-chassis-parser.json",
        "probes[0].read.table.columns",
        "requires a `chassis_id` column with parser `octet_mac`",
    ),
    (
        "lldp-missing-chassis.json",
        "probes[0].read.table.columns",
        "requires a `chassis_id` column",
    ),
    (
        "arp-mac-parser.json",
        "probes[0].read.table.columns",
        "requires a `mac` column with parser `octet_mac`",
    ),
    (
        "duplicate-column.json",
        "probes[0].read.table.columns[1].name",
        "duplicate field name `name`",
    ),
    (
        "duplicate-scalar.json",
        "probes[0].read.scalars[1].name",
        "duplicate field name `sys_name`",
    ),
    (
        "duplicate-residual.json",
        "maps.residuals[1].name",
        "duplicate residual name `a`",
    ),
    (
        "empty-residual-source.json",
        "maps.residuals[0].source",
        "must not be empty",
    ),
    (
        "bad-residual-name.json",
        "maps.residuals[0].name",
        "is not a slug",
    ),
    (
        "claim-ref-shape.json",
        "maps.claims[0].fields[0]",
        "is not a `<probe>.<field>` reference",
    ),
    (
        "claim-unknown-probe.json",
        "maps.claims[0].fields[0]",
        "unknown probe `nope`",
    ),
    (
        "empty-columns.json",
        "probes[0].read.table.columns",
        "at least one column is required",
    ),
    (
        "empty-surfaces.json",
        "surfaces",
        "at least one surface is required",
    ),
    (
        "empty-probes.json",
        "probes",
        "at least one probe is required",
    ),
    (
        "bad-adapter-id.json",
        "id",
        "must be non-empty ASCII letters",
    ),
    (
        "bad-detect-oid.json",
        "detect.sys_object_id_prefixes[0]",
        "is not a dotted OID",
    ),
    (
        "bad-exact-oid.json",
        "detect.sys_object_id_exact[0]",
        "is not a dotted OID",
    ),
    (
        "empty-detect-string.json",
        "detect.sys_descr_contains[0]",
        "must be non-empty",
    ),
    (
        "system-fed-by-table.json",
        "maps.details.system",
        "cannot be fed by this read kind",
    ),
    (
        "budget-unknown-field.json",
        "line ",
        "unknown field `max_hosts`",
    ),
    (
        "column-unknown-field.json",
        "line ",
        "unknown field `format`",
    ),
    (
        "plus-oid.json",
        "probes[0].read.scalars[0].oid",
        "is not a dotted OID",
    ),
];

/// One valid fixture and the shape it must keep proving: every probe with
/// its read kind/layout, and the claim kinds its maps declare.
struct Shape {
    document: &'static str,
    probes: &'static [(&'static str, &'static str)],
    claims: &'static [&'static str],
}

/// The executable shape-coverage proof: deleting or reshaping a
/// distinguishing probe fails the valid-corpus test.
const VALID: &[Shape] = &[
    Shape {
        document: "airos-ap.json",
        probes: &[
            ("system", "scalars"),
            ("lldp", "table:lldp_rem"),
            ("stations", "table:ubnt_station"),
        ],
        claims: &["name", "sysdescr", "portid", "localport"],
    },
    Shape {
        document: "edgeos-router.json",
        probes: &[
            ("system", "scalars"),
            ("if_table", "table:single"),
            ("ip_addresses", "table:ip_address_v4"),
            ("arp", "table:arp_v4"),
        ],
        claims: &["name", "sysdescr"],
    },
    Shape {
        document: "generic-base.json",
        probes: &[
            ("system", "scalars"),
            ("lldp", "table:lldp_rem"),
            ("ip_addresses", "table:ip_address_v4"),
        ],
        claims: &["name", "sysdescr"],
    },
    Shape {
        document: "netonix-switch.json",
        probes: &[
            ("vendor", "scalars"),
            ("temperatures", "table:single"),
            ("fdb", "table:dot1d_row_id"),
            ("arp", "table:arp_typed"),
        ],
        claims: &[],
    },
    Shape {
        document: "swos-switch.json",
        probes: &[("if_table", "table:single"), ("fdb", "table:dot1d_mac")],
        claims: &[],
    },
    Shape {
        document: "routeros-switch.json",
        probes: &[
            ("if_table", "table:single"),
            ("fdb", "table:dot1q_vlan_mac"),
        ],
        claims: &[],
    },
    Shape {
        document: "procurve-switch.json",
        probes: &[("fdb", "table:dot1q_vlan_mac")],
        claims: &[],
    },
    Shape {
        document: "routeros-router.json",
        probes: &[
            ("identity", "command:system_identity"),
            ("routerboard", "command:system_routerboard"),
            ("neighbors", "command:ip_neighbor"),
            ("interfaces", "command:interfaces"),
            ("addresses", "command:ip_address"),
            ("arp", "command:ip_arp"),
            ("bridge_ports", "command:bridge_port"),
            ("bridge_vlans", "command:bridge_vlan"),
            ("bridge_hosts", "command:bridge_host"),
            ("resource", "command:system_resource"),
            ("health", "command:system_health"),
        ],
        claims: &["serial", "name", "address", "interface"],
    },
];

/// The document spelling of an index layout.
fn layout_name(kind: adapters::IndexKind) -> &'static str {
    match kind {
        adapters::IndexKind::Single => "single",
        adapters::IndexKind::LldpRem => "lldp_rem",
        adapters::IndexKind::ArpV4 => "arp_v4",
        adapters::IndexKind::IpAddressV4 => "ip_address_v4",
        adapters::IndexKind::ArpTyped => "arp_typed",
        adapters::IndexKind::Dot1dMac => "dot1d_mac",
        adapters::IndexKind::Dot1dRowId => "dot1d_row_id",
        adapters::IndexKind::Dot1qVlanMac => "dot1q_vlan_mac",
        adapters::IndexKind::UbntStation => "ubnt_station",
        adapters::IndexKind::Station => "station",
        adapters::IndexKind::StationMac => "station_mac",
    }
}

/// The document spelling of a command.
fn command_name(name: adapters::CommandName) -> &'static str {
    match name {
        adapters::CommandName::SystemIdentity => "system_identity",
        adapters::CommandName::SystemRouterboard => "system_routerboard",
        adapters::CommandName::IpNeighbor => "ip_neighbor",
        adapters::CommandName::Interfaces => "interfaces",
        adapters::CommandName::IpAddress => "ip_address",
        adapters::CommandName::IpArp => "ip_arp",
        adapters::CommandName::BridgePort => "bridge_port",
        adapters::CommandName::BridgeVlan => "bridge_vlan",
        adapters::CommandName::BridgeHost => "bridge_host",
        adapters::CommandName::SystemResource => "system_resource",
        adapters::CommandName::SystemHealth => "system_health",
        adapters::CommandName::IpFirewallNat => "ip_firewall_nat",
        adapters::CommandName::IpDhcpServerLease => "ip_dhcp_server_lease",
    }
}

/// A probe's read as the shape table spells it.
fn describe_read(read: &adapters::Read) -> String {
    match read {
        adapters::Read::Scalars { .. } => "scalars".to_owned(),
        adapters::Read::Table { table } => format!("table:{}", layout_name(table.index.kind)),
        adapters::Read::Command { command } => format!("command:{}", command_name(command.name)),
    }
}

/// The claim kinds a document maps, in declaration order.
fn claim_kinds(document: &adapters::Document) -> Vec<&'static str> {
    document
        .maps
        .claims
        .iter()
        .map(|claim| match claim.kind {
            adapters::ClaimKind::Serial => "serial",
            adapters::ClaimKind::Address => "address",
            adapters::ClaimKind::Name => "name",
            adapters::ClaimKind::Sysdescr => "sysdescr",
            adapters::ClaimKind::Portid => "portid",
            adapters::ClaimKind::Localport => "localport",
            adapters::ClaimKind::Interface => "interface",
        })
        .collect()
}

#[test]
fn every_valid_document_validates() {
    let dir = fixtures().join("valid");
    let mut ids = Vec::new();
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).expect("the valid corpus exists") {
        let path = entry.expect("a readable fixture entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("fixture file names are UTF-8");
        let document = parse_file(&path).unwrap_or_else(|errors| {
            panic!("{} should validate:\n{}", path.display(), messages(&errors))
        });
        assert_eq!(document.schema_version, adapters::SCHEMA_VERSION);
        let shape = VALID
            .iter()
            .find(|shape| shape.document == name)
            .unwrap_or_else(|| panic!("{name} has a shape expectation"));
        let probes: Vec<(&str, String)> = document
            .probes
            .iter()
            .map(|probe| (probe.id.as_str(), describe_read(&probe.read)))
            .collect();
        let expected: Vec<(&str, String)> = shape
            .probes
            .iter()
            .map(|(id, kind)| (*id, (*kind).to_owned()))
            .collect();
        assert_eq!(probes, expected, "{name}: the shape it must keep proving");
        assert_eq!(
            claim_kinds(&document),
            shape.claims,
            "{name}: the claim kinds it maps"
        );
        ids.push(document.id.clone());
        count += 1;
    }
    assert_eq!(count, 8, "one valid document per shape the corpus proves");
    let mut shapes: Vec<&str> = VALID.iter().map(|shape| shape.document).collect();
    shapes.sort_unstable();
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("the valid corpus exists")
        .map(|entry| {
            entry
                .expect("a readable fixture entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".json"))
        .collect();
    files.sort_unstable();
    assert_eq!(
        files, shapes,
        "every valid fixture has a shape expectation and vice versa"
    );
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count, "adapter ids are unique in the corpus");
}

#[test]
fn the_station_layout_display_mac_and_radio_slot_validate_together() {
    // The ePMP shape: one unsigned association index, a DisplayString MAC
    // column, and a scalars probe for the radio block. Additive to schema v2.
    let json = r#"
{
  "schema_version": 2,
  "id": "cambium_epmp",
  "detect": { "sys_object_id_prefixes": ["1.3.6.1.4.1.17713.21"] },
  "surfaces": [{ "id": "snmp", "transport": "snmp_v2c", "credential_ref": "snmp" }],
  "probes": [
    {
      "id": "radio",
      "surface": "snmp",
      "read": {
        "kind": "scalars",
        "scalars": [
          { "name": "frequency_mhz", "oid": "1.3.6.1.4.1.17713.21.1.2.1.0", "parser": "u32" },
          { "name": "channel_width_mhz", "oid": "1.3.6.1.4.1.17713.21.1.2.2.0", "parser": "channel_width_mhz" },
          { "name": "sm_count", "oid": "1.3.6.1.4.1.17713.21.1.2.10.0", "parser": "u32" },
          { "name": "tx_power_dbm", "oid": "1.3.6.1.4.1.17713.21.1.2.5.0", "parser": "i64" },
          { "name": "lan_speed_mbps", "oid": "1.3.6.1.4.1.17713.21.1.4.12.0", "parser": "u32" }
        ]
      }
    },
    {
      "id": "stations",
      "surface": "snmp",
      "read": {
        "kind": "table",
        "table": {
          "columns": [
            { "name": "mac", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.1", "parser": "display_mac" },
            { "name": "ip", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.10", "parser": "utf8" },
            { "name": "channel", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.3", "parser": "u32" },
            { "name": "ul_rssi", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.4", "parser": "i64" },
            { "name": "dl_rssi", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.5", "parser": "i64" },
            { "name": "ul_snr", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.6", "parser": "i64" },
            { "name": "dl_snr", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.7", "parser": "i64" },
            { "name": "tx_capacity", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.19", "parser": "u32" },
            { "name": "tx_quality", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.20", "parser": "u32" },
            { "name": "dl_rate_mbps", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.28", "parser": "u32" },
            { "name": "model", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.38", "parser": "utf8" },
            { "name": "software_version", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.43", "parser": "utf8" }
          ],
          "index": { "kind": "station" }
        }
      }
    }
  ],
  "maps": { "details": { "radio": "radio", "stations": "stations" } }
}
"#;
    let document = adapters::parse("cambium_epmp.json", json)
        .unwrap_or_else(|errors| panic!("the ePMP shape should validate:\n{}", messages(&errors)));
    assert_eq!(document.id, "cambium_epmp");
    assert_eq!(document.maps.details.radio.as_deref(), Some("radio"));
    let table = document
        .probes
        .iter()
        .find_map(|probe| match &probe.read {
            adapters::Read::Table { table } => Some(table),
            _ => None,
        })
        .expect("the station table probe");
    assert_eq!(table.index.kind, adapters::IndexKind::Station);
    assert_eq!(
        table
            .columns
            .iter()
            .find(|column| column.name == "mac")
            .map(|column| column.parser),
        Some(adapters::Parser::DisplayMac)
    );
    assert!(
        document
            .probes
            .iter()
            .filter_map(|probe| match &probe.read {
                adapters::Read::Scalars { scalars } => Some(scalars),
                _ => None,
            })
            .flatten()
            .any(|scalar| scalar.parser == adapters::Parser::ChannelWidthMhz)
    );
}

#[test]
fn the_station_mac_layout_requires_an_octet_mac_column_and_allows_a_slot_suffix() {
    // The Wave/LTU shape: the index is the station MAC (optionally followed
    // by a slot), and the required `mac` column repeats it as an OCTET
    // STRING. The Kbps capacity column decodes to whole Mbps.
    let json = r#"
{
  "schema_version": 2,
  "id": "wave_ap",
  "detect": { "sys_object_id_exact": ["1.3.6.1.4.1.41112"] },
  "surfaces": [{ "id": "snmp", "transport": "snmp_v2c", "credential_ref": "snmp" }],
  "probes": [{
    "id": "stations",
    "surface": "snmp",
    "read": {
      "kind": "table",
      "table": {
        "columns": [
          { "name": "mac", "oid": "1.3.6.1.4.1.41112.1.11.2.3.1.1", "parser": "octet_mac" },
          { "name": "hostname", "oid": "1.3.6.1.4.1.41112.1.11.2.3.1.17", "parser": "utf8" },
          { "name": "tx_capacity", "oid": "1.3.6.1.4.1.41112.1.11.2.3.1.9", "parser": "u32" },
          { "name": "rx_capacity", "oid": "1.3.6.1.4.1.41112.1.11.2.3.1.10", "parser": "u32" },
          { "name": "dl_rate_mbps", "oid": "1.3.6.1.4.1.41112.1.11.2.3.1.9", "parser": "kbps_to_mbps" }
        ],
        "index": { "kind": "station_mac" }
      }
    }
  }],
  "maps": { "details": { "stations": "stations" } }
}
"#;
    let document = adapters::parse("wave_ap.json", json).unwrap_or_else(|errors| {
        panic!(
            "the station_mac shape should validate:\n{}",
            messages(&errors)
        )
    });
    let table = document
        .probes
        .iter()
        .find_map(|probe| match &probe.read {
            adapters::Read::Table { table } => Some(table),
            _ => None,
        })
        .expect("the station table probe");
    assert_eq!(table.index.kind, adapters::IndexKind::StationMac);
    assert_eq!(
        table
            .columns
            .iter()
            .find(|column| column.name == "dl_rate_mbps")
            .map(|column| column.parser),
        Some(adapters::Parser::KbpsToMbps)
    );
}

#[test]
fn every_invalid_document_fails_with_the_named_path_and_reason() {
    for (file, path, reason) in INVALID {
        let document = fixtures().join("invalid").join(file);
        let errors = parse_file(&document)
            .err()
            .unwrap_or_else(|| panic!("{file} should be rejected"));
        assert!(
            errors
                .iter()
                .any(|error| error.path.contains(path) && error.reason.contains(reason)),
            "{file} should name path `{path}` and reason `{reason}`:\n{}",
            messages(&errors)
        );
    }
}

#[test]
fn the_invalid_corpus_and_its_cases_match() {
    let dir = fixtures().join("invalid");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("the invalid corpus exists")
        .map(|entry| {
            entry
                .expect("a readable fixture entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".json"))
        .collect();
    files.sort_unstable();
    let mut cases: Vec<String> = INVALID
        .iter()
        .map(|(file, _, _)| (*file).to_owned())
        .collect();
    cases.sort_unstable();
    assert_eq!(
        files, cases,
        "every invalid fixture has a case and vice versa"
    );
}

#[test]
fn a_command_probe_must_shape_the_detail_slot_that_names_it() {
    let mut document = parse_file(&fixtures().join("valid/routeros-router.json")).expect("valid");
    let interfaces = document
        .probes
        .iter()
        .find(|probe| probe.id == "interfaces")
        .expect("the corpus document declares an `interfaces` command probe")
        .id
        .clone();
    document.maps.details.system = Some(interfaces);
    let errors = adapters::validate("valid/routeros-router.json", &document)
        .expect_err("an interfaces command cannot feed the system slot");
    assert!(
        errors
            .iter()
            .any(|error| error.path == "maps.details.system"
                && error
                    .reason
                    .contains("command probe `interfaces` feeds `interfaces`, not `system`")),
        "{}",
        messages(&errors)
    );
}

#[test]
fn the_nat_slot_accepts_only_the_firewall_nat_command() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/routeros.json");
    let mut document = parse_file(&path).expect("the shipped RouterOS document is valid");
    let arp = document
        .probes
        .iter()
        .find(|probe| probe.id == "arp")
        .expect("the shipped document declares the arp probe")
        .id
        .clone();
    document.maps.details.nat = Some(arp);
    let errors = adapters::validate("adapters/routeros.json", &document)
        .expect_err("a command that shapes another slot cannot feed nat");
    assert!(
        errors.iter().any(|error| error.path == "maps.details.nat"
            && error.reason.contains("cannot be fed by this read kind")),
        "{}",
        messages(&errors)
    );
}

#[test]
fn the_dhcp_leases_slot_accepts_only_the_dhcp_lease_command() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/routeros.json");
    let mut document = parse_file(&path).expect("the shipped RouterOS document is valid");
    let arp = document
        .probes
        .iter()
        .find(|probe| probe.id == "arp")
        .expect("the shipped document declares the arp probe")
        .id
        .clone();
    document.maps.details.dhcp_leases = Some(arp);
    let errors = adapters::validate("adapters/routeros.json", &document)
        .expect_err("a command that shapes another slot cannot feed dhcp_leases");
    assert!(
        errors
            .iter()
            .any(|error| error.path == "maps.details.dhcp_leases"
                && error.reason.contains("cannot be fed by this read kind")),
        "{}",
        messages(&errors)
    );
}

#[test]
fn a_duplicate_adapter_id_is_rejected_across_the_registry() {
    let first = parse_file(&fixtures().join("valid/airos-ap.json")).expect("valid");
    let mut second = parse_file(&fixtures().join("valid/netonix-switch.json")).expect("valid");
    second.id = first.id.clone();
    let registry = vec![
        ("valid/airos-ap.json".to_owned(), first),
        ("valid/netonix-switch.json".to_owned(), second),
    ];
    let errors = adapters::validate_registry(&registry).expect_err("duplicate ids are rejected");
    assert!(
        errors.iter().any(|error| error.path == "id"
            && error.reason.contains("duplicate adapter id")
            && error.document == "valid/netonix-switch.json"),
        "{}",
        messages(&errors)
    );
}

#[test]
fn a_clean_registry_is_ok_and_failures_accumulate_into_one_report() {
    // The unmutated corpus registry validates, with one default document.
    let clean = vec![
        (
            "valid/airos-ap.json".to_owned(),
            parse_file(&fixtures().join("valid/airos-ap.json")).expect("valid"),
        ),
        (
            "valid/edgeos-router.json".to_owned(),
            parse_file(&fixtures().join("valid/edgeos-router.json")).expect("valid"),
        ),
        (
            "valid/generic-base.json".to_owned(),
            parse_file(&fixtures().join("valid/generic-base.json")).expect("valid"),
        ),
        (
            "valid/netonix-switch.json".to_owned(),
            parse_file(&fixtures().join("valid/netonix-switch.json")).expect("valid"),
        ),
    ];
    adapters::validate_registry(&clean).expect("the corpus registry validates");

    // One document fails its own checks and another repeats an adapter id:
    // both appear in the same report.
    let mut broken = parse_file(&fixtures().join("valid/edgeos-router.json")).expect("valid");
    broken.schema_version = 3;
    let mut duplicate = parse_file(&fixtures().join("valid/netonix-switch.json")).expect("valid");
    duplicate.id = "airos-ap".to_owned();
    let registry = vec![
        (
            "valid/airos-ap.json".to_owned(),
            parse_file(&fixtures().join("valid/airos-ap.json")).expect("valid"),
        ),
        ("valid/broken.json".to_owned(), broken),
        ("valid/netonix-switch.json".to_owned(), duplicate),
    ];
    let errors = adapters::validate_registry(&registry).expect_err("both failures are reported");
    assert!(
        errors
            .iter()
            .any(|error| error.document == "valid/broken.json"
                && error.path == "schema_version"
                && error.reason.contains("unsupported schema version 3")),
        "{}",
        messages(&errors)
    );
    assert!(
        errors
            .iter()
            .any(|error| error.document == "valid/netonix-switch.json"
                && error.path == "id"
                && error.reason.contains("duplicate adapter id `airos-ap`")),
        "{}",
        messages(&errors)
    );
}

#[test]
fn a_second_default_document_is_rejected_across_the_registry() {
    let mut second = parse_file(&fixtures().join("valid/edgeos-router.json")).expect("valid");
    second.detect = adapters::Detect {
        default: true,
        ..adapters::Detect::default()
    };
    let registry = vec![
        (
            "valid/generic-base.json".to_owned(),
            parse_file(&fixtures().join("valid/generic-base.json")).expect("valid"),
        ),
        ("valid/edgeos-router.json".to_owned(), second),
    ];
    let errors =
        adapters::validate_registry(&registry).expect_err("a registry has one default at most");
    assert!(
        errors
            .iter()
            .any(|error| error.document == "valid/edgeos-router.json"
                && error.path == "detect.default"
                && error.reason.contains("a second default document")
                && error.reason.contains("valid/generic-base.json")),
        "{}",
        messages(&errors)
    );

    // The same document is legal when nothing else is the default.
    let sole_default = vec![
        (
            "valid/generic-base.json".to_owned(),
            parse_file(&fixtures().join("valid/generic-base.json")).expect("valid"),
        ),
        (
            "valid/edgeos-router.json".to_owned(),
            parse_file(&fixtures().join("valid/edgeos-router.json")).expect("valid"),
        ),
    ];
    adapters::validate_registry(&sole_default).expect("one default document validates");
}

#[test]
fn the_error_display_names_the_document_adapter_probe_and_path() {
    let errors = parse_file(&fixtures().join("invalid/rowid-without-mac.json"))
        .expect_err("the row-id layout needs a MAC column");
    let error = errors.first().expect("one error");
    let rendered = error.to_string();
    assert!(rendered.contains("rowid-without-mac.json"), "{rendered}");
    assert!(
        rendered.contains("adapter `rowid-without-mac`"),
        "{rendered}"
    );
    assert!(rendered.contains("probe `fdb`"), "{rendered}");
    assert!(
        rendered.contains("probes[0].read.table.columns"),
        "{rendered}"
    );
}

// --- the shipped registry and its loader -----------------------------------

/// The repository's shipped documents.
fn shipped() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters")
}

/// A fresh scratch directory for one registry test.
fn scratch(label: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("dudarchy-adapters-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir should create");
    dir
}

/// Copies one fixture document into `dir` under `name`.
fn copy_fixture(dir: &Path, fixture: &str, name: &str) {
    std::fs::copy(fixtures().join(fixture), dir.join(name)).expect("fixture should copy");
}

/// One table probe's columns and layout, by probe id.
fn table_columns<'a>(
    document: &'a adapters::Document,
    probe: &str,
) -> (
    Vec<(&'a str, &'a str, adapters::Parser)>,
    adapters::IndexKind,
) {
    let candidate = document
        .probes
        .iter()
        .find(|candidate| candidate.id == probe)
        .unwrap_or_else(|| panic!("the document declares a `{probe}` probe"));
    let adapters::Read::Table { table } = &candidate.read else {
        panic!("`{probe}` is a table probe");
    };
    (
        table
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.oid.as_str(), column.parser))
            .collect(),
        table.index.kind,
    )
}

#[test]
fn the_wave_and_ltu_adapters_map_ap_to_station_capacity_to_downlink() {
    // Direction: the downlink is always the AP->station direction. Wave's
    // AF60 MIB names it TxCapacity ("bytes transmitted to remote endpoint");
    // the LTU MIB names the same direction from the station side, so its
    // RxCapacity is the AP's downlink (the LTU UI labels it "Downlink
    // Capacity"). Dx is duplex, not a direction, and is deliberately unread.
    for (file, tx_oid, rx_oid) in [
        (
            "ltu_rocket.json",
            "1.3.6.1.4.1.41112.1.10.1.4.1.4",
            "1.3.6.1.4.1.41112.1.10.1.4.1.3",
        ),
        (
            "wave_ap.json",
            "1.3.6.1.4.1.41112.1.11.2.3.1.9",
            "1.3.6.1.4.1.41112.1.11.2.3.1.10",
        ),
    ] {
        let document = parse_file(&shipped().join(file)).expect("the adapter validates");
        let table = document
            .probes
            .iter()
            .find_map(|probe| match &probe.read {
                adapters::Read::Table { table } => Some(table),
                _ => None,
            })
            .expect("a station table");
        let column = |name: &str| {
            table
                .columns
                .iter()
                .find(|column| column.name == name)
                .unwrap_or_else(|| panic!("{file} declares `{name}`"))
        };
        assert_eq!(column("tx_capacity").oid, tx_oid, "{file} tx capacity");
        assert_eq!(column("rx_capacity").oid, rx_oid, "{file} uplink capacity");
        assert_eq!(column("dl_rate_mbps").oid, tx_oid, "{file} downlink rate");
        assert_eq!(
            column("dl_rate_mbps").parser,
            adapters::Parser::KbpsToMbps,
            "{file} downlink parser"
        );
        assert!(
            table
                .columns
                .iter()
                .all(|column| column.oid != "1.3.6.1.4.1.41112.1.11.2.3.1.11"),
            "{file} must not read the AF60 duplex column as a direction"
        );
    }

    let wave = parse_file(&shipped().join("wave_ap.json")).expect("the adapter validates");
    assert_eq!(
        wave.detect.sys_object_id_exact,
        ["1.3.6.1.4.1.41112"],
        "Wave detects by its exact OID"
    );
    assert!(
        wave.detect.sys_object_id_prefixes.is_empty(),
        "a prefix would capture airOS/EdgeOS descendants"
    );
}

#[test]
fn the_legacy_wave_adapter_is_pin_only_and_uses_the_20_column_table() {
    let document =
        parse_file(&shipped().join("wave_ap_legacy.json")).expect("the legacy adapter validates");
    assert!(document.detect.pin_only, "the legacy variant is pin-only");
    assert!(!document.detect.default);
    assert!(document.detect.sys_object_id_exact.is_empty());
    assert!(document.detect.sys_object_id_prefixes.is_empty());
    assert!(document.detect.sys_descr_contains.is_empty());

    let table = document
        .probes
        .iter()
        .find_map(|probe| match &probe.read {
            adapters::Read::Table { table } => Some(table),
            _ => None,
        })
        .expect("a station table");
    let column = |name: &str| {
        table
            .columns
            .iter()
            .find(|column| column.name == name)
            .unwrap_or_else(|| panic!("the legacy adapter declares `{name}`"))
    };
    assert_eq!(column("mac").oid, "1.3.6.1.4.1.41112.1.11.1.3.1.1");
    assert_eq!(column("dl_rate_mbps").oid, "1.3.6.1.4.1.41112.1.11.1.3.1.7");
    assert_eq!(column("rx_capacity").oid, "1.3.6.1.4.1.41112.1.11.1.3.1.8");
    assert_eq!(column("tx_capacity").oid, column("dl_rate_mbps").oid);
    assert_eq!(column("dl_rate_mbps").parser, adapters::Parser::KbpsToMbps);
}

#[test]
fn the_shipped_registry_loads_every_document_in_file_order() {
    let mut registry = Registry::default();
    registry
        .reload(&shipped())
        .expect("the shipped adapters validate as a registry");
    assert!(registry.is_configured());
    assert!(registry.last_failure().is_none());
    assert!(matches!(registry.state(), RegistryState::Loaded(_)));
    let ids: Vec<&str> = registry
        .documents()
        .iter()
        .map(|document| document.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "airos",
            "cambium_epmp",
            "edgeos",
            "generic-base",
            "ltu_rocket",
            "netonix",
            "procurve",
            "routeros",
            "swos",
            "wave_ap",
            "wave_ap_legacy"
        ],
        "the documents load in file-name order"
    );
    let defaults: Vec<&str> = registry
        .documents()
        .iter()
        .filter(|document| document.detect.default)
        .map(|document| document.id.as_str())
        .collect();
    assert_eq!(defaults, ["generic-base"], "one default document");

    // Every document declares exactly the transports its family's agent
    // speaks: airOS is the v1-only agent, RouterOS adds the binary API to
    // its v2c surface, and every other shipped family is v2c only.
    for document in registry.documents() {
        let expected: &[adapters::Transport] = match document.id.as_str() {
            "airos" => &[adapters::Transport::SnmpV1],
            "routeros" => &[
                adapters::Transport::SnmpV2c,
                adapters::Transport::RouterOsApi,
            ],
            _ => &[adapters::Transport::SnmpV2c],
        };
        let transports: Vec<adapters::Transport> = document
            .surfaces
            .iter()
            .map(|surface| surface.transport)
            .collect();
        assert_eq!(
            transports, expected,
            "{}: the declared surface transports",
            document.id
        );
    }

    // The recon notes for surfaces a family cannot express are document data,
    // not engine behaviour, so the pins live here.
    for (id, expected) in [
        ("airos", &["station-counters"][..]),
        (
            "edgeos",
            &[
                "lldp-unavailable",
                "fdb-unavailable",
                "ipv6-addresses-unread",
                "lag-membership-unread",
            ][..],
        ),
        (
            "routeros",
            &[
                "interface-vlan-enrichment",
                "health-multi-sensor",
                "bridge-table-conditional-read",
                "firewall-address-lists",
                "ipv6-firewall-nat",
            ][..],
        ),
        ("swos", &["lag-membership-unread"][..]),
    ] {
        let document = registry
            .documents()
            .iter()
            .find(|document| document.id == id)
            .unwrap_or_else(|| panic!("the shipped registry carries `{id}`"));
        let names: Vec<&str> = document
            .maps
            .residuals
            .iter()
            .map(|residual| residual.name.as_str())
            .collect();
        assert_eq!(names, expected, "{id}: the document-declared residuals");
    }
}

#[test]
fn the_shipped_generic_base_pins_every_probe_oid_and_mapping() {
    let doc = parse_file(&shipped().join("generic-base.json")).expect("the shipped doc is valid");

    // Detection: the registry's one default, with no hints.
    assert!(doc.detect.default);
    assert!(doc.detect.sys_object_id_prefixes.is_empty());
    assert!(doc.detect.sys_descr_contains.is_empty());

    let surfaces: Vec<(&str, adapters::Transport, adapters::CredentialRef)> = doc
        .surfaces
        .iter()
        .map(|surface| {
            (
                surface.id.as_str(),
                surface.transport,
                surface.credential_ref,
            )
        })
        .collect();
    assert_eq!(
        surfaces,
        [(
            "snmp",
            adapters::Transport::SnmpV2c,
            adapters::CredentialRef::Snmp
        )]
    );

    // One probe per surface, in document order, with the required ones
    // marked: only the system scalars gate support, so an unknown responder
    // that answers the system group still yields its neighbours.
    let probes: Vec<(&str, bool)> = doc
        .probes
        .iter()
        .map(|probe| {
            (
                probe.id.as_str(),
                probe.on_absent == adapters::OnAbsent::Required,
            )
        })
        .collect();
    assert_eq!(
        probes,
        [
            ("system", true),
            ("if_table", false),
            ("ip_addresses", false),
            ("lldp", false),
            ("arp_media", false),
            ("arp_physical", false),
        ]
    );

    let adapters::Read::Scalars { scalars } = &doc.probes[0].read else {
        panic!("system is a scalars probe");
    };
    let scalars: Vec<(&str, &str, adapters::Parser)> = scalars
        .iter()
        .map(|scalar| (scalar.name.as_str(), scalar.oid.as_str(), scalar.parser))
        .collect();
    assert_eq!(
        scalars,
        [
            ("sys_descr", "1.3.6.1.2.1.1.1.0", adapters::Parser::Utf8),
            ("sys_name", "1.3.6.1.2.1.1.5.0", adapters::Parser::Utf8),
            ("sys_object_id", "1.3.6.1.2.1.1.2.0", adapters::Parser::Oid),
        ]
    );

    assert_eq!(
        table_columns(&doc, "if_table"),
        (
            vec![
                ("description", "1.3.6.1.2.1.2.2.1.2", adapters::Parser::Utf8),
                ("raw_type", "1.3.6.1.2.1.2.2.1.3", adapters::Parser::U32),
                ("mac", "1.3.6.1.2.1.2.2.1.6", adapters::Parser::OctetMac),
                ("admin_state", "1.3.6.1.2.1.2.2.1.7", adapters::Parser::U32),
                ("oper_state", "1.3.6.1.2.1.2.2.1.8", adapters::Parser::U32),
                ("name", "1.3.6.1.2.1.31.1.1.1.1", adapters::Parser::Utf8),
                ("alias", "1.3.6.1.2.1.31.1.1.1.18", adapters::Parser::Utf8),
            ],
            adapters::IndexKind::Single
        )
    );

    assert_eq!(
        table_columns(&doc, "ip_addresses"),
        (
            vec![
                ("if_index", "1.3.6.1.2.1.4.20.1.2", adapters::Parser::U32),
                (
                    "netmask",
                    "1.3.6.1.2.1.4.20.1.3",
                    adapters::Parser::IpAddress
                ),
            ],
            adapters::IndexKind::IpAddressV4
        ),
        "the standard IPv4 ipAddrTable, so an unknown responder's own addresses attach"
    );

    assert_eq!(
        table_columns(&doc, "lldp"),
        (
            vec![
                (
                    "local_port",
                    "1.0.8802.1.1.2.1.4.1.1.2",
                    adapters::Parser::U32
                ),
                (
                    "chassis_id",
                    "1.0.8802.1.1.2.1.4.1.1.5",
                    adapters::Parser::OctetMac
                ),
                (
                    "port_id",
                    "1.0.8802.1.1.2.1.4.1.1.7",
                    adapters::Parser::Utf8
                ),
                (
                    "sys_name",
                    "1.0.8802.1.1.2.1.4.1.1.9",
                    adapters::Parser::Utf8
                ),
                (
                    "sys_descr",
                    "1.0.8802.1.1.2.1.4.1.1.10",
                    adapters::Parser::Utf8
                ),
            ],
            adapters::IndexKind::LldpRem
        ),
        "the LLDP-MIB value columns, not the subtype columns beside them"
    );

    assert_eq!(
        table_columns(&doc, "arp_media"),
        (
            vec![("mac", "1.3.6.1.2.1.4.22.1.2", adapters::Parser::OctetMac)],
            adapters::IndexKind::ArpV4
        )
    );
    assert_eq!(
        table_columns(&doc, "arp_physical"),
        (
            vec![("mac", "1.3.6.1.2.1.4.35.1.4", adapters::Parser::OctetMac)],
            adapters::IndexKind::ArpTyped
        )
    );

    // The mappings those probes feed.
    let claims: Vec<(adapters::ClaimKind, Vec<&str>)> = doc
        .maps
        .claims
        .iter()
        .map(|claim| {
            (
                claim.kind,
                claim.fields.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        claims,
        [
            (
                adapters::ClaimKind::Name,
                vec!["system.sys_name", "lldp.sys_name"]
            ),
            (
                adapters::ClaimKind::Sysdescr,
                vec!["system.sys_descr", "lldp.sys_descr"]
            ),
            (adapters::ClaimKind::Portid, vec!["lldp.port_id"]),
            (adapters::ClaimKind::Localport, vec!["lldp.local_port"]),
        ]
    );
    let details = &doc.maps.details;
    assert_eq!(details.interfaces.as_deref(), Some("if_table"));
    assert_eq!(details.arp.as_deref(), Some("arp_media"));
    assert!(
        details.system.is_none()
            && details.temperatures.is_none()
            && details.stations.is_none()
            && details.bridge_ports.is_none()
            && details.bridge_vlans.is_none()
            && details.bridge_hosts.is_none()
    );

    // The corpus fixture keeps the shipped LLDP columns, so the replay case
    // proves the document the product ships.
    let corpus = parse_file(&fixtures().join("valid/generic-base.json")).expect("valid");
    assert_eq!(table_columns(&corpus, "lldp"), table_columns(&doc, "lldp"));
}

#[test]
fn a_registry_loads_the_valid_documents_in_file_order() {
    let dir = scratch("valid");
    copy_fixture(&dir, "valid/netonix-switch.json", "b-netonix.json");
    copy_fixture(&dir, "valid/airos-ap.json", "a-airos.json");
    std::fs::write(dir.join("notes.txt"), "not a document").expect("ignore file should write");

    let mut registry = Registry::default();
    registry.reload(&dir).expect("the directory validates");
    let ids: Vec<&str> = registry
        .documents()
        .iter()
        .map(|document| document.id.as_str())
        .collect();
    assert_eq!(ids, ["airos-ap", "netonix-switch"]);
    assert!(registry.last_failure().is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_invalid_document_blocks_the_whole_registry_and_names_it() {
    let dir = scratch("invalid");
    copy_fixture(&dir, "valid/airos-ap.json", "airos-ap.json");
    copy_fixture(&dir, "invalid/unknown-parser.json", "broken.json");

    let mut registry = Registry::default();
    let error = registry
        .reload(&dir)
        .expect_err("one invalid document blocks the load");
    assert!(!registry.is_configured(), "nothing is activated");
    assert!(registry.documents().is_empty());
    assert_eq!(registry.last_failure(), Some(&error));
    let rendered = error.to_string();
    assert!(
        rendered.contains(&dir.join("broken.json").display().to_string()),
        "{rendered}"
    );
    assert!(rendered.contains("unknown variant `regex`"), "{rendered}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_reload_keeps_the_last_valid_registry() {
    let dir = scratch("reload");
    copy_fixture(&dir, "valid/edgeos-router.json", "edgeos-router.json");
    let mut registry = Registry::default();
    registry.reload(&dir).expect("the first load validates");

    copy_fixture(&dir, "invalid/unknown-field.json", "broken.json");
    let failure = registry
        .reload(&dir)
        .expect_err("the broken document fails the reload");
    assert_eq!(
        registry.documents().len(),
        1,
        "the last valid registry stays active"
    );
    assert_eq!(registry.documents()[0].id, "edgeos-router");
    assert_eq!(
        registry.last_failure().map(ToString::to_string),
        Some(failure.to_string())
    );

    std::fs::remove_file(dir.join("broken.json")).expect("broken document should remove");
    registry
        .reload(&dir)
        .expect("the repaired directory validates");
    assert!(registry.last_failure().is_none());
    assert_eq!(registry.documents()[0].id, "edgeos-router");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_or_empty_directory_is_the_named_unconfigured_state() {
    let parent = scratch("missing");
    let missing = parent.join("adapters");
    let mut registry = Registry::default();
    registry
        .reload(&missing)
        .expect("a missing directory is not an error");
    assert!(matches!(registry.state(), RegistryState::Unconfigured));
    assert!(!registry.is_configured());
    assert!(registry.documents().is_empty());
    assert!(registry.last_failure().is_none());

    let empty = scratch("empty");
    registry
        .reload(&empty)
        .expect("an empty directory is not an error");
    assert!(matches!(registry.state(), RegistryState::Unconfigured));

    let _ = std::fs::remove_dir_all(&parent);
    let _ = std::fs::remove_dir_all(&empty);
}

#[test]
fn the_adapters_directory_derives_from_the_vault_file() {
    assert_eq!(directory_for(None), PathBuf::from("adapters"));
    assert_eq!(
        directory_for(Some(Path::new("vault.json"))),
        PathBuf::from("adapters")
    );
    assert_eq!(
        directory_for(Some(Path::new("config/vault.json"))),
        PathBuf::from("config/adapters")
    );
    assert_eq!(
        directory_for(Some(Path::new("/etc/dudarchy/vault.json"))),
        PathBuf::from("/etc/dudarchy/adapters")
    );
}

#[test]
fn a_registry_level_failure_blocks_the_load() {
    let dir = scratch("duplicate");
    copy_fixture(&dir, "valid/airos-ap.json", "one.json");
    copy_fixture(&dir, "valid/airos-ap.json", "two.json");

    let mut registry = Registry::default();
    let error = registry
        .reload(&dir)
        .expect_err("duplicate adapter ids are rejected across documents");
    assert!(
        error
            .to_string()
            .contains("duplicate adapter id `airos-ap`"),
        "{error}"
    );
    assert!(!registry.is_configured(), "nothing is activated");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_json_directory_is_named_and_blocks_the_load() {
    let dir = scratch("json-directory");
    copy_fixture(&dir, "valid/airos-ap.json", "airos-ap.json");
    std::fs::create_dir(dir.join("nested.json")).expect("directory entry should create");

    let mut registry = Registry::default();
    let error = registry
        .reload(&dir)
        .expect_err("a directory named like a document blocks the load");
    let rendered = error.to_string();
    assert!(
        rendered.contains(&dir.join("nested.json").display().to_string()),
        "{rendered}"
    );
    assert!(rendered.contains("not a regular file"), "{rendered}");
    assert!(!registry.is_configured(), "nothing is activated");

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_broken_json_symlink_is_named_and_blocks_the_load() {
    let dir = scratch("json-symlink");
    copy_fixture(&dir, "valid/airos-ap.json", "airos-ap.json");
    std::os::unix::fs::symlink(dir.join("gone.json"), dir.join("broken.json"))
        .expect("broken symlink should create");

    let mut registry = Registry::default();
    let error = registry
        .reload(&dir)
        .expect_err("a broken symlink named like a document blocks the load");
    let rendered = error.to_string();
    assert!(
        rendered.contains(&dir.join("broken.json").display().to_string()),
        "{rendered}"
    );
    assert!(rendered.contains("cannot read document"), "{rendered}");
    assert!(!registry.is_configured(), "nothing is activated");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_a_lowercase_json_name_is_a_document() {
    let dir = scratch("case");
    std::fs::copy(
        fixtures().join("valid/airos-ap.json"),
        dir.join("airos-ap.JSON"),
    )
    .expect("uppercase document should copy");

    let mut registry = Registry::default();
    registry.reload(&dir).expect("no document is present");
    assert!(matches!(registry.state(), RegistryState::Unconfigured));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_reload_names_every_broken_document() {
    let dir = scratch("reload-broken");
    copy_fixture(&dir, "valid/edgeos-router.json", "edgeos-router.json");
    let mut registry = Registry::default();
    registry.reload(&dir).expect("the first load validates");

    copy_fixture(&dir, "invalid/unknown-parser.json", "one.json");
    copy_fixture(&dir, "invalid/unknown-field.json", "two.json");
    let error = registry
        .reload(&dir)
        .expect_err("both broken documents fail the reload");
    let rendered = error.to_string();
    assert!(
        rendered.contains(&dir.join("one.json").display().to_string()),
        "{rendered}"
    );
    assert!(rendered.contains("unknown variant `regex`"), "{rendered}");
    assert!(
        rendered.contains(&dir.join("two.json").display().to_string()),
        "{rendered}"
    );
    assert!(rendered.contains("unknown field `detectt`"), "{rendered}");
    assert_eq!(
        registry.documents().len(),
        1,
        "the last valid registry stays active"
    );
    assert_eq!(registry.documents()[0].id, "edgeos-router");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_path_that_is_not_a_directory_is_a_named_failure() {
    let parent = scratch("not-a-directory");
    let path = parent.join("adapters");
    std::fs::write(&path, "{}").expect("regular file should write");

    let mut registry = Registry::default();
    let error = registry
        .reload(&path)
        .expect_err("a regular file is not an adapters directory");
    assert!(matches!(&error, RegistryError::Directory { .. }), "{error}");
    assert!(
        error.to_string().contains(&path.display().to_string()),
        "{error}"
    );
    assert!(!registry.is_configured());

    let _ = std::fs::remove_dir_all(&parent);
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_is_a_named_failure() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("denied");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000))
        .expect("the scratch dir should chmod");
    // A privileged runner can still list it; the unreadable case only means
    // something to an unprivileged process.
    let denied = std::fs::read_dir(&dir).is_err();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
        .expect("the scratch dir should chmod back");
    if denied {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000))
            .expect("the scratch dir should chmod");
        let mut registry = Registry::default();
        let error = registry
            .reload(&dir)
            .expect_err("an unreadable directory is a failure");
        assert!(matches!(&error, RegistryError::Directory { .. }), "{error}");
        assert!(
            error.to_string().contains(&dir.display().to_string()),
            "{error}"
        );
        assert!(!registry.is_configured());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("the scratch dir should chmod back");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn an_unreadable_json_file_is_named() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("denied-file");
    copy_fixture(&dir, "valid/airos-ap.json", "airos-ap.json");
    let locked = dir.join("locked.json");
    copy_fixture(&dir, "valid/edgeos-router.json", "locked.json");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
        .expect("the document should chmod");
    let denied = std::fs::read_to_string(&locked).is_err();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644))
        .expect("the document should chmod back");
    if denied {
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
            .expect("the document should chmod");
        let mut registry = Registry::default();
        let error = registry
            .reload(&dir)
            .expect_err("an unreadable document blocks the load");
        assert!(
            error.to_string().contains(&locked.display().to_string()),
            "{error}"
        );
        assert!(
            error.to_string().contains("cannot read document"),
            "{error}"
        );
        assert!(!registry.is_configured(), "nothing is activated");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644))
            .expect("the document should chmod back");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

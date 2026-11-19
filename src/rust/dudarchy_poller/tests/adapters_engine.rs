//! Engine tests: documents replayed through a scripted Reader. Each test
//! feeds the varbinds or command rows a device would answer and asserts the
//! decoded rows, mapped claims/details, residuals, and typed outcomes.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use dudarchy_poller::WalkValue;
use dudarchy_poller::adapters::engine::{self, ApiSelection, Observation, Selection};
use dudarchy_poller::adapters::reader::{CommandRow, Outcome, OutcomeKind, Reader, Varbind};
use dudarchy_poller::adapters::{self, CommandName, Document};

/// One OID as arcs.
fn oid(text: &str) -> Vec<u32> {
    text.split('.')
        .map(|arc| arc.parse().expect("fixture OID arc"))
        .collect()
}

/// A column row OID under an entry root: `<entry>.<column>.<instance…>`.
fn at(entry: &[u32], column: u32, instance: &[u32]) -> Vec<u32> {
    [entry, &[column], instance].concat()
}

/// A walked varbind.
fn vb(oid: Vec<u32>, value: WalkValue) -> Varbind {
    Varbind { oid, value }
}

/// An octet-string value.
fn bytes(text: &str) -> WalkValue {
    WalkValue::Bytes(text.as_bytes().to_vec())
}

/// A MAC as the six raw octets an agent sends.
fn mac(text: &str) -> WalkValue {
    WalkValue::Bytes(
        text.split(':')
            .map(|pair| u8::from_str_radix(pair, 16).expect("fixture MAC byte"))
            .collect(),
    )
}

/// An integer value.
fn integer(value: i64) -> WalkValue {
    WalkValue::Integer(value)
}

/// A present read.
fn present(varbinds: Vec<Varbind>) -> Outcome<Vec<Varbind>> {
    Outcome::Present(varbinds)
}

/// One command row from key/value pairs.
fn command_row(fields: &[(&str, &str)]) -> CommandRow {
    fields
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// A scripted reader: every OID and command maps to a fixed outcome, and an
/// unscripted read answers `Absent`.
struct Scripted {
    gets: Vec<(Vec<u32>, Outcome<Vec<Varbind>>)>,
    walks: Vec<(Vec<u32>, Outcome<Vec<Varbind>>)>,
    commands: Vec<(CommandName, Outcome<Vec<CommandRow>>)>,
    cancel: Option<(Arc<AtomicBool>, usize)>,
    calls: usize,
    /// Every walk root the engine requested, in order.
    walks_requested: Vec<Vec<u32>>,
}

impl Scripted {
    fn new() -> Scripted {
        Scripted {
            gets: Vec::new(),
            walks: Vec::new(),
            commands: Vec::new(),
            cancel: None,
            calls: 0,
            walks_requested: Vec::new(),
        }
    }

    fn get(mut self, oid: &[u32], outcome: Outcome<Vec<Varbind>>) -> Scripted {
        self.gets.push((oid.to_vec(), outcome));
        self
    }

    fn walk(mut self, root: &[u32], outcome: Outcome<Vec<Varbind>>) -> Scripted {
        self.walks.push((root.to_vec(), outcome));
        self
    }

    fn command(mut self, name: CommandName, outcome: Outcome<Vec<CommandRow>>) -> Scripted {
        self.commands.push((name, outcome));
        self
    }

    fn cancelled_after(mut self, calls: usize, flag: Arc<AtomicBool>) -> Scripted {
        self.cancel = Some((flag, calls));
        self
    }

    fn tick(&mut self) {
        self.calls += 1;
        if let Some((flag, after)) = &self.cancel
            && self.calls >= *after
        {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

impl Reader for Scripted {
    fn get<'a>(
        &'a mut self,
        oid: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        Box::pin(async move {
            self.tick();
            self.gets
                .iter()
                .find(|(key, _)| key.as_slice() == oid)
                .map(|(_, outcome)| outcome.clone())
                .unwrap_or(Outcome::Absent)
        })
    }

    fn walk<'a>(
        &'a mut self,
        root: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        Box::pin(async move {
            self.tick();
            self.walks_requested.push(root.to_vec());
            self.walks
                .iter()
                .find(|(key, _)| key.as_slice() == root)
                .map(|(_, outcome)| outcome.clone())
                .unwrap_or(Outcome::Absent)
        })
    }

    fn command<'a>(
        &'a mut self,
        name: CommandName,
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<CommandRow>>> + Send + 'a>> {
        Box::pin(async move {
            self.tick();
            self.commands
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, outcome)| outcome.clone())
                .unwrap_or(Outcome::Absent)
        })
    }
}

/// The fixture tree.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adapters")
}

/// Loads and validates one corpus document.
fn document(name: &str) -> Document {
    let path = fixtures().join("valid").join(name);
    let text = std::fs::read_to_string(&path).expect("the corpus document reads");
    adapters::parse(name, &text).unwrap_or_else(|errors| {
        panic!(
            "{name} should validate: {}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    })
}

/// Runs one document with no cancellation.
async fn run(document: &Document, reader: &mut Scripted) -> Observation {
    engine::probe(document, reader, SystemTime::UNIX_EPOCH, &|| false).await
}

/// Runs one surface of a document with no cancellation.
async fn run_surface(document: &Document, surface: &str, reader: &mut Scripted) -> Observation {
    engine::probe_surface(document, surface, reader, SystemTime::UNIX_EPOCH, &|| false).await
}

/// One probe report by id.
fn report<'a>(observation: &'a Observation, probe: &str) -> &'a engine::ProbeReport {
    observation
        .probes
        .iter()
        .find(|report| report.probe == probe)
        .unwrap_or_else(|| panic!("{probe} has a report"))
}

// The entry OIDs the engine walks: each is the parent of the document's
// column OIDs, so a column row is `<entry>.<column>.<instance…>`.
const LLDP_REM_TABLE: [u32; 9] = [1, 0, 8802, 1, 1, 2, 1, 4, 1];
const LLDP_REM_ENTRY: [u32; 10] = [1, 0, 8802, 1, 1, 2, 1, 4, 1, 1];
const LLDP_MAN_TABLE: [u32; 9] = [1, 0, 8802, 1, 1, 2, 1, 4, 2];
const LLDP_LOC_TABLE: [u32; 9] = [1, 0, 8802, 1, 1, 2, 1, 3, 7];
const IF_TABLE: [u32; 9] = [1, 3, 6, 1, 2, 1, 2, 2, 1];
const IF_X_TABLE: [u32; 10] = [1, 3, 6, 1, 2, 1, 31, 1, 1, 1];
const ARP_V4: [u32; 9] = [1, 3, 6, 1, 2, 1, 4, 22, 1];
const IP_ADDR: [u32; 9] = [1, 3, 6, 1, 2, 1, 4, 20, 1];
const BRIDGE_ENTRY: [u32; 10] = [1, 3, 6, 1, 2, 1, 17, 4, 3, 1];
const NETONIX_TEMPS: [u32; 9] = [1, 3, 6, 1, 4, 1, 46242, 3, 1];
const ARP_TYPED: [u32; 9] = [1, 3, 6, 1, 2, 1, 4, 35, 1];
const QBRIDGE_ENTRY: [u32; 12] = [1, 3, 6, 1, 2, 1, 17, 7, 1, 2, 2, 1];
const BASE_PORT: [u32; 11] = [1, 3, 6, 1, 2, 1, 17, 1, 4, 1, 2];

#[tokio::test]
async fn edgeos_document_yields_interfaces_arp_and_details() {
    let document = document("edgeos-router.json");
    let mut reader = Scripted::new()
        .get(
            &oid("1.3.6.1.2.1.1.1.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.1.0"), bytes("EdgeOS v2"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.5.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.5.0"), bytes("edge"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.2.0"),
            present(vec![vb(
                oid("1.3.6.1.2.1.1.2.0"),
                WalkValue::Oid("1.3.6.1.4.1.41112.1.5".to_owned()),
            )]),
        )
        .walk(
            &IF_TABLE,
            present(vec![
                vb(at(&IF_TABLE, 2, &[1]), bytes("WAN")),
                vb(at(&IF_TABLE, 3, &[1]), integer(6)),
                vb(at(&IF_TABLE, 6, &[1]), mac("6c:3b:6b:00:00:01")),
                vb(at(&IF_TABLE, 7, &[1]), integer(1)),
                vb(at(&IF_TABLE, 8, &[1]), integer(1)),
                vb(at(&IF_TABLE, 2, &[2]), bytes("LAN")),
                vb(at(&IF_TABLE, 3, &[2]), integer(6)),
                vb(at(&IF_TABLE, 7, &[2]), integer(1)),
                vb(at(&IF_TABLE, 8, &[2]), integer(2)),
            ]),
        )
        .walk(
            &IF_X_TABLE,
            present(vec![
                vb(at(&IF_X_TABLE, 1, &[1]), bytes("eth0")),
                vb(at(&IF_X_TABLE, 18, &[1]), bytes("uplink")),
                vb(at(&IF_X_TABLE, 1, &[2]), bytes("eth1")),
            ]),
        )
        .walk(
            &ARP_V4,
            present(vec![vb(
                at(&ARP_V4, 2, &[2, 10, 0, 0, 5]),
                mac("02:00:00:aa:00:05"),
            )]),
        );
    let observation = run(&document, &mut reader).await;

    assert!(observation.supported());
    assert_eq!(observation.interfaces.len(), 2);
    let interfaces = &observation.interfaces;
    assert_eq!(interfaces[0].index, 1);
    assert_eq!(interfaces[0].name.as_deref(), Some("eth0"));
    assert_eq!(interfaces[0].descr.as_deref(), Some("WAN"));
    assert_eq!(interfaces[0].alias.as_deref(), Some("uplink"));
    assert_eq!(interfaces[0].raw_type, Some(6));
    assert_eq!(
        interfaces[0].phys_address.as_deref(),
        Some([0x6c, 0x3b, 0x6b, 0x00, 0x00, 0x01].as_slice())
    );
    assert_eq!(interfaces[1].index, 2);
    assert_eq!(interfaces[1].name.as_deref(), Some("eth1"));

    let details = observation.details.as_ref().expect("details map");
    assert_eq!(details.interfaces.len(), 2);
    assert_eq!(
        details.interfaces[1].kind,
        dudarchy_poller::InterfaceKind::Ethernet
    );
    assert_eq!(
        details.interfaces[1].oper_state,
        Some(dudarchy_poller::IfState::Down)
    );

    assert_eq!(observation.arp.len(), 1);
    assert_eq!(observation.arp[0].if_index, 2);
    assert_eq!(observation.arp[0].ip, "10.0.0.5");
    assert_eq!(observation.arp[0].mac, [0x02, 0x00, 0x00, 0xaa, 0x00, 0x05]);

    let arp = details.arp.as_ref().expect("ARP feeds details");
    assert_eq!(arp.rows.len(), 1);
    assert_eq!(arp.rows[0].address.as_deref(), Some("10.0.0.5"));
    assert_eq!(arp.rows[0].interface.as_deref(), Some("eth1"));
    assert_eq!(
        arp.rows[0].mac_address.as_ref().map(|mac| mac.as_str()),
        Some("02:00:00:aa:00:05")
    );
}

#[tokio::test]
async fn edgeos_ip_addresses_attach_to_the_interfaces_that_own_them() {
    let document = document("edgeos-router.json");
    let mut reader = Scripted::new()
        .get(
            &oid("1.3.6.1.2.1.1.1.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.1.0"), bytes("EdgeOS v2"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.5.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.5.0"), bytes("edge"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.2.0"),
            present(vec![vb(
                oid("1.3.6.1.2.1.1.2.0"),
                WalkValue::Oid("1.3.6.1.4.1.41112.1.5".to_owned()),
            )]),
        )
        .walk(
            &IF_TABLE,
            present(vec![
                vb(at(&IF_TABLE, 2, &[2]), bytes("LAN")),
                vb(at(&IF_TABLE, 3, &[2]), integer(6)),
                vb(at(&IF_TABLE, 7, &[2]), integer(1)),
                vb(at(&IF_TABLE, 8, &[2]), integer(1)),
                vb(at(&IF_TABLE, 2, &[3]), bytes("WAN")),
                vb(at(&IF_TABLE, 3, &[3]), integer(6)),
                vb(at(&IF_TABLE, 7, &[3]), integer(1)),
                vb(at(&IF_TABLE, 8, &[3]), integer(1)),
            ]),
        )
        .walk(
            &IF_X_TABLE,
            present(vec![
                vb(at(&IF_X_TABLE, 1, &[2]), bytes("eth0")),
                vb(at(&IF_X_TABLE, 1, &[3]), bytes("eth1")),
            ]),
        )
        .walk(
            &IP_ADDR,
            present(vec![
                vb(at(&IP_ADDR, 2, &[10, 0, 0, 1]), integer(2)),
                vb(
                    at(&IP_ADDR, 3, &[10, 0, 0, 1]),
                    WalkValue::IpAddress("255.255.255.0".to_owned()),
                ),
                vb(at(&IP_ADDR, 2, &[10, 0, 0, 2]), integer(3)),
                vb(
                    at(&IP_ADDR, 3, &[10, 0, 0, 2]),
                    WalkValue::IpAddress("255.255.255.252".to_owned()),
                ),
                // No netmask: the address stays bare.
                vb(at(&IP_ADDR, 2, &[10, 0, 0, 3]), integer(2)),
                // An address on an interface the ifTable does not have.
                vb(at(&IP_ADDR, 2, &[10, 0, 0, 9]), integer(9)),
                vb(
                    at(&IP_ADDR, 3, &[10, 0, 0, 9]),
                    WalkValue::IpAddress("255.255.255.0".to_owned()),
                ),
            ]),
        );
    let observation = run(&document, &mut reader).await;

    assert!(observation.supported());
    assert_eq!(
        report(&observation, "ip_addresses").outcome,
        OutcomeKind::Present
    );
    assert_eq!(report(&observation, "ip_addresses").rows, 4);
    assert_eq!(observation.ip_addresses.len(), 4);
    assert_eq!(observation.ip_addresses[0].if_index, 2);
    assert_eq!(observation.ip_addresses[0].address, "10.0.0.1");
    assert_eq!(observation.ip_addresses[0].prefix_len, Some(24));
    assert_eq!(observation.ip_addresses[1].prefix_len, Some(30));
    assert_eq!(observation.ip_addresses[2].prefix_len, None);

    let details = observation.details.as_ref().expect("details map");
    assert_eq!(details.interfaces.len(), 2);
    assert_eq!(details.interfaces[0].id, "2");
    assert_eq!(
        details.interfaces[0].addresses,
        ["10.0.0.1/24", "10.0.0.3"],
        "the ifIndex-2 rows attach; the netmask-less row stays bare"
    );
    assert_eq!(
        details.interfaces[0].provenance,
        [
            dudarchy_poller::Source::SnmpInterfaces,
            dudarchy_poller::Source::SnmpIpAddresses
        ],
        "one IP query is listed once however many addresses it named"
    );
    assert_eq!(details.interfaces[1].id, "3");
    assert_eq!(details.interfaces[1].addresses, ["10.0.0.2/30"]);
    assert!(
        details
            .interfaces
            .iter()
            .all(|interface| interface.id != "9"),
        "an address without an interface invents none"
    );
}

#[tokio::test]
async fn the_routeros_api_surface_maps_commands_claims_details_and_neighbours() {
    let document = shipped("routeros.json");
    let mut reader = Scripted::new()
        .command(
            CommandName::SystemIdentity,
            Outcome::Present(vec![command_row(&[("name", "office-core")])]),
        )
        .command(
            CommandName::SystemRouterboard,
            Outcome::Present(vec![command_row(&[("serial-number", "HGC123")])]),
        )
        .command(
            CommandName::IpNeighbor,
            Outcome::Present(vec![
                command_row(&[
                    ("mac-address", "6C:3B:6B:70:96:3F"),
                    ("address", "192.168.100.13"),
                    ("identity", "FSW-West"),
                    ("interface", "ether2"),
                ]),
                command_row(&[
                    ("mac-address", "02:00:00:00:00:01"),
                    ("address", "192.168.100.14"),
                ]),
            ]),
        )
        .command(
            CommandName::Interfaces,
            Outcome::Present(vec![
                command_row(&[
                    (".id", "*1"),
                    ("name", "bridge-local"),
                    ("type", "bridge"),
                    ("running", "true"),
                    ("disabled", "false"),
                ]),
                command_row(&[
                    (".id", "*2"),
                    ("name", "ether1"),
                    ("type", "ether"),
                    ("mac-address", "6C:3B:6B:70:96:3F"),
                    ("running", "true"),
                    ("disabled", "false"),
                ]),
                // An idless row has no stable handle and is dropped.
                command_row(&[("name", "ghost"), ("type", "ether")]),
            ]),
        )
        .command(
            CommandName::IpAddress,
            Outcome::Present(vec![command_row(&[
                ("address", "192.168.88.1/24"),
                ("interface", "bridge-local"),
            ])]),
        )
        .command(
            CommandName::IpArp,
            Outcome::Present(vec![command_row(&[
                ("address", "192.168.88.9"),
                ("mac-address", "00:11:22:33:44:55"),
                ("interface", "bridge-local"),
                ("dynamic", "true"),
                ("complete", "true"),
                ("disabled", "false"),
            ])]),
        )
        .command(
            CommandName::BridgePort,
            Outcome::Present(vec![command_row(&[
                ("bridge", "bridge-local"),
                ("interface", "ether1"),
                ("pvid", "92"),
                ("frame-types", "admit-only-untagged-and-priority-tagged"),
                ("status", "in-bridge"),
                ("inactive", "false"),
                ("disabled", "false"),
                ("dynamic", "false"),
            ])]),
        )
        .command(
            CommandName::BridgeVlan,
            Outcome::Present(vec![command_row(&[
                ("bridge", "bridge-local"),
                ("vlan-ids", "92,100"),
                ("tagged", "ether1"),
                ("untagged", "ether2"),
                ("current-tagged", "ether1"),
                ("current-untagged", "ether2"),
            ])]),
        )
        .command(
            CommandName::BridgeHost,
            Outcome::Present(vec![command_row(&[
                ("bridge", "bridge-local"),
                ("on-interface", "ether1"),
                ("mac-address", "4C:5E:0C:67:35:12"),
                ("age", "3m"),
                ("dynamic", "true"),
                ("local", "false"),
                ("external", "false"),
            ])]),
        )
        .command(
            CommandName::SystemResource,
            Outcome::Present(vec![command_row(&[
                ("version", "6.49.18"),
                ("board-name", "CCR1009-8G-1S"),
                ("platform", "routeros"),
                ("uptime", "1w2d"),
            ])]),
        )
        .command(
            CommandName::SystemHealth,
            Outcome::Present(vec![command_row(&[("temperature", "41")])]),
        )
        .command(
            CommandName::IpFirewallNat,
            Outcome::Present(vec![
                command_row(&[
                    ("chain", "srcnat"),
                    ("action", "masquerade"),
                    ("out-interface", "ether1"),
                    ("src-address", "192.168.100.0/24"),
                    ("dst-address-list", "unmapped"),
                    ("disabled", "false"),
                    ("comment", "NOC NAT"),
                ]),
                // Only the disabled flag is not a rule.
                command_row(&[("disabled", "true")]),
                // Whitespace-only text is no content either.
                command_row(&[("chain", "   "), ("disabled", "false")]),
            ]),
        )
        .command(
            CommandName::IpDhcpServerLease,
            Outcome::Present(vec![
                command_row(&[
                    ("address", "192.168.88.20"),
                    ("mac-address", "AA:BB:CC:DD:EE:20"),
                    ("host-name", "printer"),
                    ("status", "bound"),
                    ("dynamic", "true"),
                    ("disabled", "false"),
                ]),
                // A disabled lease is retained as evidence; consumers
                // ignore it.
                command_row(&[
                    ("address", "192.168.88.22"),
                    ("mac-address", "AA:BB:CC:DD:EE:22"),
                    ("host-name", "stale"),
                    ("status", "bound"),
                    ("dynamic", "false"),
                    ("disabled", "true"),
                ]),
                // A hostname-only lease states a client without an
                // address.
                command_row(&[
                    ("mac-address", "AA:BB:CC:DD:EE:23"),
                    ("host-name", "reserved"),
                    ("status", "bound"),
                ]),
                // Whitespace-only text is absent: the MAC still names a
                // client, so the row stays with no address or hostname.
                command_row(&[
                    ("address", "   "),
                    ("mac-address", "AA:BB:CC:DD:EE:24"),
                    ("host-name", "  "),
                    ("status", " "),
                ]),
                // Flags alone name no client.
                command_row(&[("dynamic", "true"), ("disabled", "false")]),
                // Whitespace-only text with no MAC names no client
                // either, so the row is dropped.
                command_row(&[("address", "   "), ("host-name", " ")]),
            ]),
        );
    let observation = run_surface(&document, "api", &mut reader).await;

    assert!(observation.supported());
    let probes: Vec<&str> = observation
        .probes
        .iter()
        .map(|probe| probe.probe.as_str())
        .collect();
    assert_eq!(
        probes,
        [
            "identity",
            "routerboard",
            "neighbors",
            "interfaces",
            "addresses",
            "arp",
            "bridge_ports",
            "bridge_vlans",
            "bridge_hosts",
            "resource",
            "health",
            "nat",
            "dhcp_leases",
        ],
        "the API surface runs its probes and nothing from the SNMP surface"
    );

    // The neighbour feed is a typed observation, not a claim on the node.
    assert_eq!(observation.neighbours.len(), 2);
    assert_eq!(
        observation.neighbours[0].identity.as_deref(),
        Some("FSW-West")
    );
    assert_eq!(
        observation.neighbours[0].interface.as_deref(),
        Some("ether2")
    );

    let claims: Vec<(String, String)> = observation
        .claims
        .iter()
        .map(|observed| {
            (
                observed.claim.value.clone(),
                observed.claim.kind.to_string(),
            )
        })
        .collect();
    assert_eq!(
        claims,
        [
            ("office-core".to_owned(), "name".to_owned()),
            ("HGC123".to_owned(), "serial".to_owned()),
            ("FSW-West".to_owned(), "name".to_owned()),
            ("192.168.100.13".to_owned(), "address".to_owned()),
            ("192.168.100.14".to_owned(), "address".to_owned()),
            ("192.168.88.1/24".to_owned(), "address".to_owned()),
            ("ether2".to_owned(), "interface".to_owned()),
        ],
        "the document's claim order, row for row"
    );

    let details = observation.details.as_ref().expect("the API slots mapped");
    assert_eq!(details.interfaces.len(), 2, "the idless row is dropped");
    assert_eq!(details.interfaces[1].name.as_deref(), Some("ether1"));
    assert_eq!(
        details.interfaces[0].addresses,
        ["192.168.88.1/24"],
        "the address probe enriches the named interface row"
    );
    assert_eq!(
        details.interfaces[0].provenance,
        [
            dudarchy_poller::Source::RouterOsInterfaces,
            dudarchy_poller::Source::RouterOsAddresses
        ]
    );
    assert!(
        details.interfaces[1].addresses.is_empty(),
        "an interface the address probe does not name stays unenriched"
    );
    let system = details.system.as_ref().expect("resource feeds system");
    assert_eq!(system.version.as_deref(), Some("6.49.18"));
    assert_eq!(system.provenance, [dudarchy_poller::Source::RouterOsSystem]);
    assert_eq!(details.temperatures.len(), 1);
    assert_eq!(details.temperatures[0].sensor_name, "temperature");
    assert_eq!(details.temperatures[0].celsius, 41.0);
    assert_eq!(details.temperatures[0].observed_at, SystemTime::UNIX_EPOCH);
    let arp = details.arp.as_ref().expect("ip_arp feeds arp");
    assert_eq!(arp.rows[0].address.as_deref(), Some("192.168.88.9"));
    assert_eq!(arp.rows[0].complete, Some(true));
    assert_eq!(
        details
            .bridge_ports
            .as_ref()
            .expect("bridge_port feeds bridge_ports")
            .rows[0]
            .pvid,
        Some(92)
    );
    assert_eq!(
        details
            .bridge_vlans
            .as_ref()
            .expect("bridge_vlan feeds bridge_vlans")
            .rows[0]
            .vlan_ids,
        [92, 100]
    );
    assert_eq!(
        details
            .bridge_hosts
            .as_ref()
            .expect("bridge_host feeds bridge_hosts")
            .rows[0]
            .age
            .as_deref(),
        Some("3m")
    );
    let nat = details.nat.as_ref().expect("ip_firewall_nat feeds nat");
    assert_eq!(
        nat.rows.len(),
        1,
        "a row with only `disabled`, or only whitespace, is not a rule"
    );
    let rule = &nat.rows[0];
    assert_eq!(rule.chain.as_deref(), Some("srcnat"));
    assert_eq!(rule.action.as_deref(), Some("masquerade"));
    assert_eq!(rule.out_interface.as_deref(), Some("ether1"));
    assert_eq!(rule.src_address.as_deref(), Some("192.168.100.0/24"));
    assert_eq!(rule.dst_address, None, "the unmapped key stays out");
    assert_eq!(rule.disabled, Some(false));
    assert_eq!(rule.comment.as_deref(), Some("NOC NAT"));
    assert_eq!(
        nat.provenance,
        [dudarchy_poller::Source::RouterOsFirewallNat],
        "the snapshot names the query that answered"
    );
    let leases = details
        .dhcp_leases
        .as_ref()
        .expect("ip_dhcp_server_lease feeds dhcp_leases");
    assert_eq!(
        leases.rows.len(),
        4,
        "the flags-only row and the all-whitespace row name no client and are dropped"
    );
    assert_eq!(leases.rows[0].address.as_deref(), Some("192.168.88.20"));
    assert_eq!(
        leases.rows[0]
            .mac
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("aa:bb:cc:dd:ee:20"),
        "the client MAC is normalized"
    );
    assert_eq!(leases.rows[0].hostname.as_deref(), Some("printer"));
    assert_eq!(leases.rows[0].status.as_deref(), Some("bound"));
    assert_eq!(leases.rows[0].dynamic, Some(true));
    assert_eq!(leases.rows[0].disabled, Some(false));
    assert_eq!(
        leases.rows[1].disabled,
        Some(true),
        "a disabled lease stays as evidence"
    );
    assert_eq!(leases.rows[2].address, None, "the lease states no address");
    assert_eq!(leases.rows[2].hostname.as_deref(), Some("reserved"));
    assert_eq!(
        leases.rows[3].address, None,
        "whitespace-only address text is absent"
    );
    assert_eq!(
        leases.rows[3].hostname, None,
        "whitespace-only hostname text is absent"
    );
    assert_eq!(
        leases.rows[3].status, None,
        "whitespace-only status text is absent"
    );
    assert_eq!(
        leases.rows[3]
            .mac
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("aa:bb:cc:dd:ee:24"),
        "the MAC still names the client"
    );
    assert_eq!(
        leases.provenance,
        [dudarchy_poller::Source::RouterOsDhcpLease],
        "the snapshot names the lease query"
    );
    assert!(
        observation.residuals.is_empty(),
        "every API probe has a model destination"
    );
}

#[tokio::test]
async fn a_routeros_lease_probe_absence_or_error_keeps_the_observation_supported() {
    let document = shipped("routeros.json");
    for outcome in [
        Outcome::Absent,
        Outcome::Empty,
        Outcome::Error {
            reason: "no such command".to_owned(),
        },
    ] {
        let mut reader = Scripted::new()
            .command(
                CommandName::SystemIdentity,
                Outcome::Present(vec![command_row(&[("name", "cr1")])]),
            )
            .command(CommandName::IpNeighbor, Outcome::Empty)
            .command(
                CommandName::Interfaces,
                Outcome::Present(vec![command_row(&[
                    (".id", "*1"),
                    ("name", "ether1"),
                    ("type", "ether"),
                    ("disabled", "false"),
                ])]),
            )
            .command(
                CommandName::SystemResource,
                Outcome::Present(vec![command_row(&[("version", "6.49.18")])]),
            )
            .command(CommandName::IpDhcpServerLease, outcome.clone());
        let observation = run_surface(&document, "api", &mut reader).await;

        assert!(
            observation.supported(),
            "a router without lease rows stays supported"
        );
        let details = observation
            .details
            .as_ref()
            .expect("the present slots still map their details");
        assert!(
            details.dhcp_leases.is_none(),
            "an absent, empty, or errored lease query states no lease table"
        );
        assert_eq!(
            report(&observation, "dhcp_leases").outcome,
            outcome.kind(),
            "the lease probe reports its own non-present outcome"
        );
    }
}

#[tokio::test]
async fn the_routeros_snmp_surface_keeps_the_interface_and_arp_fallback() {
    let document = shipped("routeros.json");
    let mut reader = Scripted::new()
        .get(
            &oid("1.3.6.1.2.1.1.1.0"),
            present(vec![vb(
                oid("1.3.6.1.2.1.1.1.0"),
                bytes("RouterOS 6.49.18"),
            )]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.5.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.5.0"), bytes("cr1"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.2.0"),
            present(vec![vb(
                oid("1.3.6.1.2.1.1.2.0"),
                WalkValue::Oid("1.3.6.1.4.1.14988.1".to_owned()),
            )]),
        )
        .walk(
            &IF_TABLE,
            present(vec![
                vb(at(&IF_TABLE, 2, &[16]), bytes("sfp-sfpplus1")),
                vb(at(&IF_TABLE, 3, &[16]), integer(6)),
                vb(at(&IF_TABLE, 6, &[16]), mac("00:0c:42:00:00:10")),
                vb(at(&IF_TABLE, 7, &[16]), integer(1)),
                vb(at(&IF_TABLE, 8, &[16]), integer(1)),
            ]),
        )
        .walk(
            &IF_X_TABLE,
            present(vec![vb(at(&IF_X_TABLE, 1, &[16]), bytes("sfp-sfpplus1"))]),
        )
        .walk(
            &ARP_V4,
            present(vec![vb(
                at(&ARP_V4, 2, &[16, 192, 168, 100, 9]),
                mac("00:11:22:33:44:55"),
            )]),
        );
    let observation = run_surface(&document, "snmp", &mut reader).await;

    let probes: Vec<&str> = observation
        .probes
        .iter()
        .map(|probe| probe.probe.as_str())
        .collect();
    assert_eq!(
        probes,
        [
            "system",
            "routerboard_serial",
            "if_table",
            "lldp",
            "arp_media",
            "arp_physical",
            "fdb"
        ],
        "the SNMP surface runs its probes only"
    );
    let details = observation
        .details
        .as_ref()
        .expect("the interface table feeds the fallback details");
    assert_eq!(details.interfaces.len(), 1);
    assert_eq!(details.interfaces[0].id, "16");
    assert_eq!(details.interfaces[0].name.as_deref(), Some("sfp-sfpplus1"));
    let arp = details
        .arp
        .as_ref()
        .expect("arp_media feeds the SNMP ARP detail");
    assert_eq!(arp.rows[0].address.as_deref(), Some("192.168.100.9"));
    assert_eq!(arp.rows[0].interface.as_deref(), Some("sfp-sfpplus1"));
    assert!(details.system.is_none(), "no API resource probe ran");
    assert!(details.bridge_ports.is_none(), "no API bridge probe ran");
    assert!(
        observation.neighbours.is_empty(),
        "no API neighbour probe ran"
    );
    assert!(
        observation
            .claims
            .iter()
            .any(|claim| claim.claim.value == "cr1"
                && claim.claim.source == dudarchy_poller::Source::SnmpSystem)
    );
}

#[test]
fn api_selection_prefers_the_snmp_document_and_falls_back_to_the_one_api_document() {
    let routeros = shipped("routeros.json");
    let airos = shipped("airos.json");

    // The SNMP-selected document serves its own API surface.
    match engine::select_api(&[routeros.clone(), airos.clone()], Some(&routeros)) {
        ApiSelection::Selected(document) => assert_eq!(document.id, "routeros"),
        other => panic!("expected the routeros document, got {other:?}"),
    }
    // A document without an API surface means no API work.
    assert!(matches!(
        engine::select_api(&[routeros.clone(), airos.clone()], Some(&airos)),
        ApiSelection::Unsupported
    ));
    // Without an SNMP selection, the one API-capable document wins.
    match engine::select_api(&[routeros.clone(), airos.clone()], None) {
        ApiSelection::Selected(document) => assert_eq!(document.id, "routeros"),
        other => panic!("expected the routeros document, got {other:?}"),
    }
    // The registry default matched no family hint, so it does not veto the
    // API surface an authenticated credential proves.
    let generic = shipped("generic-base.json");
    match engine::select_api(&[routeros.clone(), generic], None) {
        ApiSelection::Selected(document) => assert_eq!(document.id, "routeros"),
        other => panic!("expected the routeros document, got {other:?}"),
    }
    // No API-capable document is no API work.
    assert!(matches!(
        engine::select_api(std::slice::from_ref(&airos), None),
        ApiSelection::Unsupported
    ));
    // Two API-capable documents are ambiguous, never first-wins.
    let mut copy = routeros.clone();
    copy.id = "routeros-copy".to_owned();
    match engine::select_api(&[routeros, copy], None) {
        ApiSelection::Ambiguous { adapters } => {
            assert_eq!(adapters, ["routeros", "routeros-copy"]);
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }
}

#[tokio::test]
async fn netonix_vendor_reads_map_system_and_temperatures() {
    // The corpus Netonix document declares the vendor probes as detail
    // sources; the engine's generic arms turn the firmware string into the
    // system facts and the sensor table into temperature readings, so none
    // of them stays a residual observation.
    let document = document("netonix-switch.json");
    let mut reader = Scripted::new()
        .get(
            &oid("1.3.6.1.4.1.46242.1.0"),
            present(vec![vb(oid("1.3.6.1.4.1.46242.1.0"), bytes("1.5.14"))]),
        )
        .get(
            &oid("1.3.6.1.4.1.46242.6.0"),
            present(vec![vb(oid("1.3.6.1.4.1.46242.6.0"), integer(320))]),
        )
        .walk(
            &NETONIX_TEMPS,
            present(vec![
                vb(at(&NETONIX_TEMPS, 2, &[1]), bytes("board")),
                vb(at(&NETONIX_TEMPS, 3, &[1]), integer(45)),
            ]),
        )
        .walk(
            &BRIDGE_ENTRY,
            present(vec![
                vb(at(&BRIDGE_ENTRY, 1, &[7]), mac("02:00:00:00:00:07")),
                vb(at(&BRIDGE_ENTRY, 2, &[7]), integer(3)),
                vb(at(&BRIDGE_ENTRY, 3, &[7]), integer(3)),
            ]),
        )
        .walk(
            &ARP_TYPED,
            present(vec![vb(
                at(&ARP_TYPED, 4, &[2, 1, 10, 0, 0, 5]),
                mac("02:00:00:aa:00:05"),
            )]),
        );
    let observation = run(&document, &mut reader).await;

    assert!(observation.supported());
    assert_eq!(report(&observation, "arp").outcome, OutcomeKind::Present);
    assert_eq!(observation.arp.len(), 1, "the typed ARP row decodes");
    assert_eq!(observation.arp[0].if_index, 2);
    assert_eq!(observation.arp[0].ip, "10.0.0.5");
    assert!(observation.arp[0].physical);
    let fdb = observation.fdb.as_ref().expect("the bridge row decodes");
    assert_eq!(fdb.bridge.as_ref().expect("bridge rows").len(), 1);
    assert_eq!(fdb.port_if_index, [(3, 3)]);
    assert!(
        observation.residuals.is_empty(),
        "the vendor reads now have a model destination: {:?}",
        observation.residuals
    );
    let details = observation
        .details
        .as_ref()
        .expect("the detail reads have a model destination");
    assert_eq!(details.arp.as_ref().expect("arp details").rows.len(), 1);
    let system = details.system.as_ref().expect("the firmware read maps");
    assert_eq!(system.version.as_deref(), Some("1.5.14"));
    assert_eq!(system.provenance, [dudarchy_poller::Source::SnmpVendor]);
    assert_eq!(details.temperatures.len(), 1);
    assert_eq!(details.temperatures[0].sensor_name, "board");
    assert_eq!(details.temperatures[0].celsius, 45.0);
    assert_eq!(
        details.temperatures[0].provenance,
        [dudarchy_poller::Source::SnmpVendor]
    );
}

#[tokio::test]
async fn typed_outcomes_stay_distinct() {
    let document = document("edgeos-router.json");
    let mut reader = Scripted::new()
        .get(&oid("1.3.6.1.2.1.1.1.0"), Outcome::Absent)
        .get(&oid("1.3.6.1.2.1.1.5.0"), Outcome::Absent)
        .get(&oid("1.3.6.1.2.1.1.2.0"), Outcome::Absent)
        .walk(&IF_TABLE, Outcome::Timeout)
        .walk(&IF_X_TABLE, Outcome::Timeout)
        .walk(
            &ARP_V4,
            Outcome::Error {
                reason: "decode failed".to_owned(),
            },
        );
    let observation = run(&document, &mut reader).await;

    assert_eq!(report(&observation, "system").outcome, OutcomeKind::Absent);
    assert_eq!(
        report(&observation, "if_table").outcome,
        OutcomeKind::Timeout
    );
    let arp = report(&observation, "arp");
    assert_eq!(arp.outcome, OutcomeKind::Error);
    assert_eq!(arp.reason.as_deref(), Some("decode failed"));
    assert!(!observation.supported(), "a required read did not answer");

    // An empty table is a successful answer, not an absent one.
    let mut reader = Scripted::new()
        .get(
            &oid("1.3.6.1.2.1.1.1.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.1.0"), bytes("EdgeOS"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.5.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.5.0"), bytes("edge"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.2.0"),
            present(vec![vb(
                oid("1.3.6.1.2.1.1.2.0"),
                WalkValue::Oid("1.3.6.1.4.1.41112.1.5".to_owned()),
            )]),
        )
        .walk(&IF_TABLE, Outcome::Empty)
        .walk(&IF_X_TABLE, Outcome::Empty)
        .walk(&ARP_V4, Outcome::Absent);
    let observation = run(&document, &mut reader).await;
    assert_eq!(report(&observation, "if_table").outcome, OutcomeKind::Empty);
    assert_eq!(report(&observation, "arp").outcome, OutcomeKind::Absent);
    assert!(observation.supported(), "empty is a successful answer");
}

#[tokio::test]
async fn cancellation_reaches_the_reads() {
    let document = document("edgeos-router.json");
    let flag = Arc::new(AtomicBool::new(false));
    let mut reader = Scripted::new()
        .get(
            &oid("1.3.6.1.2.1.1.1.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.1.0"), bytes("EdgeOS"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.5.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.5.0"), bytes("edge"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.2.0"),
            present(vec![vb(
                oid("1.3.6.1.2.1.1.2.0"),
                WalkValue::Oid("1.3.6.1.4.1.41112.1.5".to_owned()),
            )]),
        )
        .cancelled_after(3, Arc::clone(&flag));
    let observation = engine::probe(&document, &mut reader, SystemTime::UNIX_EPOCH, &|| {
        flag.load(Ordering::SeqCst)
    })
    .await;

    assert_eq!(report(&observation, "system").outcome, OutcomeKind::Present);
    assert_eq!(
        report(&observation, "if_table").outcome,
        OutcomeKind::Cancelled
    );
    assert_eq!(report(&observation, "arp").outcome, OutcomeKind::Cancelled);
    assert!(!observation.supported());
}

#[tokio::test]
async fn budgets_stop_the_run_with_a_named_reason() {
    let mut doc = document("edgeos-router.json");
    doc.budget.max_requests = Some(1);
    let mut reader = Scripted::new().get(
        &oid("1.3.6.1.2.1.1.1.0"),
        present(vec![vb(oid("1.3.6.1.2.1.1.1.0"), bytes("EdgeOS"))]),
    );
    let observation = run(&doc, &mut reader).await;
    let system = report(&observation, "system");
    assert_eq!(system.outcome, OutcomeKind::Error);
    assert_eq!(system.reason.as_deref(), Some("request budget exhausted"));
    assert_eq!(report(&observation, "if_table").outcome, OutcomeKind::Error);
    assert!(!observation.supported());

    let mut doc = document("edgeos-router.json");
    doc.budget.max_rows = Some(1);
    let mut reader = Scripted::new()
        .get(
            &oid("1.3.6.1.2.1.1.1.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.1.0"), bytes("EdgeOS"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.5.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.5.0"), bytes("edge"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.2.0"),
            present(vec![vb(
                oid("1.3.6.1.2.1.1.2.0"),
                WalkValue::Oid("1.3.6.1.4.1.41112.1.5".to_owned()),
            )]),
        )
        .walk(
            &IF_TABLE,
            present(vec![
                vb(at(&IF_TABLE, 2, &[1]), bytes("WAN")),
                vb(at(&IF_TABLE, 2, &[2]), bytes("LAN")),
            ]),
        )
        .walk(&IF_X_TABLE, Outcome::Empty);
    let observation = run(&doc, &mut reader).await;
    assert_eq!(
        report(&observation, "if_table").outcome,
        OutcomeKind::Present
    );
    let arp = report(&observation, "arp");
    assert_eq!(arp.outcome, OutcomeKind::Error);
    assert_eq!(arp.reason.as_deref(), Some("row budget exceeded"));

    // A one-byte budget stops the first probe after its first response.
    let mut doc = document("edgeos-router.json");
    doc.budget.max_bytes = Some(1);
    let mut reader = Scripted::new().get(
        &oid("1.3.6.1.2.1.1.1.0"),
        present(vec![vb(oid("1.3.6.1.2.1.1.1.0"), bytes("EdgeOS"))]),
    );
    let observation = run(&doc, &mut reader).await;
    let system = report(&observation, "system");
    assert_eq!(system.outcome, OutcomeKind::Error);
    assert_eq!(system.reason.as_deref(), Some("byte budget exceeded"));

    // A zero deadline is reached before the first read.
    let mut doc = document("edgeos-router.json");
    doc.budget.deadline_ms = Some(0);
    let mut reader = Scripted::new();
    let observation = run(&doc, &mut reader).await;
    let system = report(&observation, "system");
    assert_eq!(system.outcome, OutcomeKind::Error);
    assert_eq!(system.reason.as_deref(), Some("deadline exceeded"));
}

#[test]
fn selection_is_selected_unsupported_or_ambiguous() {
    let airos = document("airos-ap.json");
    let edgeos = document("edgeos-router.json");
    let routeros_switch = document("routeros-switch.json");
    let routeros_router = document("routeros-router.json");
    let documents = vec![airos, edgeos, routeros_switch, routeros_router];

    let selected = engine::select(&documents, Some(".1.3.6.1.4.1.41112.1.5.2"), None);
    match selected {
        Selection::Selected(document) => assert_eq!(document.id, "edgeos-router"),
        other => panic!("expected a selection, got {other:?}"),
    }

    assert!(matches!(
        engine::select(&documents, Some("1.3.6.1.4.1.99999"), Some("unknown")),
        Selection::Unsupported
    ));

    match engine::select(&documents, None, Some("RouterOS 6.49")) {
        Selection::Ambiguous { adapters } => {
            assert_eq!(adapters, ["routeros-switch", "routeros-router"]);
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }

    // OID prefixes compare by arcs: 1.3.6.1.4.1.10002 must not match
    // 1.3.6.1.4.1.100021.
    assert!(matches!(
        engine::select(&documents, Some("1.3.6.1.4.1.100021"), None),
        Selection::Unsupported
    ));
    match engine::select(&documents, Some("1.3.6.1.4.1.10002.1"), None) {
        Selection::Selected(document) => assert_eq!(document.id, "airos-ap"),
        other => panic!("expected the airOS document, got {other:?}"),
    }
}

#[test]
fn an_exact_sys_object_id_beats_a_broader_prefix() {
    let mut broad = document("edgeos-router.json");
    broad.id = "broad".to_owned();
    broad.detect.sys_descr_contains.clear();
    broad.detect.sys_object_id_prefixes = vec!["1.3.6.1.4.1.41112".to_owned()];

    let mut exact = document("edgeos-router.json");
    exact.id = "exact".to_owned();
    exact.detect.sys_descr_contains.clear();
    exact.detect.sys_object_id_prefixes.clear();
    exact.detect.sys_object_id_exact = vec!["1.3.6.1.4.1.41112".to_owned()];

    let documents = vec![broad, exact];
    match engine::select(&documents, Some("1.3.6.1.4.1.41112"), None) {
        Selection::Selected(document) => assert_eq!(document.id, "exact"),
        other => panic!("expected the exact document, got {other:?}"),
    }

    // A descendant OID does not equal the exact value; the broad prefix takes it.
    match engine::select(&documents, Some("1.3.6.1.4.1.41112.1.5"), None) {
        Selection::Selected(document) => assert_eq!(document.id, "broad"),
        other => panic!("expected the prefix document, got {other:?}"),
    }

    // Two documents claiming one exact OID are refused, not first-wins.
    let mut duplicate = document("edgeos-router.json");
    duplicate.id = "exact-copy".to_owned();
    duplicate.detect.sys_descr_contains.clear();
    duplicate.detect.sys_object_id_prefixes.clear();
    duplicate.detect.sys_object_id_exact = vec!["1.3.6.1.4.1.41112".to_owned()];
    let mut exact = document("edgeos-router.json");
    exact.id = "exact".to_owned();
    exact.detect.sys_descr_contains.clear();
    exact.detect.sys_object_id_prefixes.clear();
    exact.detect.sys_object_id_exact = vec!["1.3.6.1.4.1.41112".to_owned()];
    match engine::select(&[exact, duplicate], Some("1.3.6.1.4.1.41112"), None) {
        Selection::Ambiguous { adapters } => assert_eq!(adapters, ["exact", "exact-copy"]),
        other => panic!("expected ambiguity between exact claims, got {other:?}"),
    }
}

#[test]
fn an_unknown_oid_matching_two_prefixes_is_ambiguous_and_refused() {
    let mut first = document("edgeos-router.json");
    first.id = "first".to_owned();
    first.detect.sys_descr_contains.clear();
    first.detect.sys_object_id_prefixes = vec!["1.3.6.1.4.1.41112.1".to_owned()];
    let mut second = document("edgeos-router.json");
    second.id = "second".to_owned();
    second.detect.sys_descr_contains.clear();
    second.detect.sys_object_id_prefixes = vec!["1.3.6.1.4.1.41112.1".to_owned()];

    match engine::select(&[first, second], Some("1.3.6.1.4.1.41112.1.5"), None) {
        Selection::Ambiguous { adapters } => assert_eq!(adapters, ["first", "second"]),
        other => panic!("expected ambiguity, got {other:?}"),
    }
}

#[test]
fn a_pin_only_document_is_never_auto_selected() {
    let wave = shipped("wave_ap.json");
    let legacy = shipped("wave_ap_legacy.json");
    assert!(legacy.detect.pin_only, "the legacy variant is pin-only");
    let documents = vec![wave, legacy];

    match engine::select(&documents, Some("1.3.6.1.4.1.41112"), None) {
        Selection::Selected(document) => assert_eq!(document.id, "wave_ap"),
        other => panic!("expected wave_ap, got {other:?}"),
    }
    // The pin-only document is not a fallback either.
    assert!(matches!(
        engine::select(&documents, Some("1.3.6.1.4.1.41112.9"), None),
        Selection::Unsupported
    ));
}

#[test]
fn selection_prefers_a_specific_match_and_falls_back_to_the_default() {
    let generic = document("generic-base.json");
    assert!(
        generic.detect.default,
        "the fixture is the default document"
    );
    let mut copy = generic.clone();
    copy.id = "generic-base-copy".to_owned();
    let one_default = vec![document("airos-ap.json"), generic.clone()];
    let two_defaults = vec![document("airos-ap.json"), generic, copy];

    // A specific hint match wins over the default.
    match engine::select(&one_default, Some("1.3.6.1.4.1.10002.1"), None) {
        Selection::Selected(document) => assert_eq!(document.id, "airos-ap"),
        other => panic!("expected airOS over the default, got {other:?}"),
    }

    // With nothing specific answering, the default applies. A responder
    // that answers no sysObjectID and no known sysDescr is the case this
    // exists for: an unknown device still gets LLDP discovery.
    match engine::select(&one_default, Some("1.3.6.1.4.1.99999"), Some("AcmeOS")) {
        Selection::Selected(document) => assert_eq!(document.id, "generic-base"),
        other => panic!("expected the default document, got {other:?}"),
    }

    // A specific match still wins when the registry also has two defaults.
    match engine::select(&two_defaults, Some("1.3.6.1.4.1.10002.1"), None) {
        Selection::Selected(document) => assert_eq!(document.id, "airos-ap"),
        other => panic!("expected airOS over two defaults, got {other:?}"),
    }

    // Two defaults and no specific match is ambiguous, not first-wins.
    match engine::select(&two_defaults, Some("1.3.6.1.4.1.99999"), Some("AcmeOS")) {
        Selection::Ambiguous { adapters } => {
            assert_eq!(adapters, ["generic-base", "generic-base-copy"]);
        }
        other => panic!("expected ambiguity between defaults, got {other:?}"),
    }
}

/// One of the repository's shipped documents, parsed and validated.
fn shipped(name: &str) -> Document {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("adapters")
        .join(name);
    let text = std::fs::read_to_string(&path).expect("the shipped document reads");
    adapters::parse(name, &text).unwrap_or_else(|errors| {
        panic!(
            "{name} should validate: {}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    })
}

#[test]
fn every_responder_selects_the_shipped_generic_base() {
    let documents = vec![shipped("generic-base.json")];

    // RouterOS answers the standard LLDP columns, so it needs no separate
    // document: the one default serves it like any other responder.
    match engine::select(
        &documents,
        Some("1.3.6.1.4.1.14988.1"),
        Some("RouterOS 7.16.2"),
    ) {
        Selection::Selected(document) => assert_eq!(document.id, "generic-base"),
        other => panic!("expected the generic base, got {other:?}"),
    }

    // An unknown sysObjectID whose agent answers the standard LLDP table
    // selects the same document: selection sees identity only, so table
    // contents and vendor columns never steer it.
    match engine::select(&documents, Some("1.3.6.1.4.1.99999"), Some("AcmeOS")) {
        Selection::Selected(document) => assert_eq!(document.id, "generic-base"),
        other => panic!("expected the generic base, got {other:?}"),
    }
}

#[tokio::test]
async fn an_answered_but_empty_fdb_is_a_known_empty_snapshot() {
    let swos = shipped("swos.json");

    // The walk answered the table with no rows: a known-empty FDB.
    let mut reader = Scripted::new().walk(&BRIDGE_ENTRY, Outcome::Empty);
    let observation = run(&swos, &mut reader).await;
    assert_eq!(report(&observation, "fdb").outcome, OutcomeKind::Empty);
    let fdb = observation
        .fdb
        .as_ref()
        .expect("an empty answer is a snapshot");
    assert_eq!(fdb.bridge, Some(Vec::new()), "empty is not missing");
    assert_eq!(fdb.qbridge, None);
    assert!(fdb.port_if_index.is_empty());

    // The row-id BRIDGE layout (Netonix) arms the same rule.
    let netonix = shipped("netonix.json");
    let mut reader = Scripted::new().walk(&BRIDGE_ENTRY, Outcome::Empty);
    let observation = run(&netonix, &mut reader).await;
    assert_eq!(report(&observation, "fdb").outcome, OutcomeKind::Empty);
    let fdb = observation
        .fdb
        .as_ref()
        .expect("an empty answer is a snapshot");
    assert_eq!(fdb.bridge, Some(Vec::new()), "empty is not missing");
    assert_eq!(fdb.qbridge, None);

    // And so does the Q-BRIDGE layout (the corpus RouterOS switch).
    let routeros_switch = document("routeros-switch.json");
    let mut reader = Scripted::new().walk(&QBRIDGE_ENTRY, Outcome::Empty);
    let observation = run(&routeros_switch, &mut reader).await;
    assert_eq!(report(&observation, "fdb").outcome, OutcomeKind::Empty);
    let fdb = observation
        .fdb
        .as_ref()
        .expect("an empty answer is a snapshot");
    assert_eq!(fdb.qbridge, Some(Vec::new()), "empty is not missing");
    assert_eq!(fdb.bridge, None);

    // NoSuchObject, a timeout, and an error all leave the FDB unknown.
    for outcome in [
        Outcome::Absent,
        Outcome::Timeout,
        Outcome::Error {
            reason: "walk failed".to_owned(),
        },
    ] {
        let outcome_name = outcome.clone();
        let mut reader = Scripted::new().walk(&BRIDGE_ENTRY, outcome);
        let observation = run(&swos, &mut reader).await;
        assert!(
            observation.fdb.is_none(),
            "no snapshot for {outcome_name:?}"
        );
    }

    // A cancellation after the FDB walk answered commits no rows.
    let flag = Arc::new(AtomicBool::new(false));
    let mut reader = Scripted::new()
        .walk(
            &BRIDGE_ENTRY,
            present(vec![
                vb(
                    at(&BRIDGE_ENTRY, 1, &[0x00, 0x05, 0xb7, 0, 0, 1]),
                    mac("00:05:b7:00:00:01"),
                ),
                vb(
                    at(&BRIDGE_ENTRY, 2, &[0x00, 0x05, 0xb7, 0, 0, 1]),
                    integer(1),
                ),
                vb(
                    at(&BRIDGE_ENTRY, 3, &[0x00, 0x05, 0xb7, 0, 0, 1]),
                    integer(3),
                ),
            ]),
        )
        .cancelled_after(9, Arc::clone(&flag));
    let observation = engine::probe(&swos, &mut reader, SystemTime::UNIX_EPOCH, &|| {
        flag.load(Ordering::SeqCst)
    })
    .await;
    assert_eq!(report(&observation, "fdb").outcome, OutcomeKind::Cancelled);
    assert!(
        observation.fdb.is_none(),
        "a cancelled decode commits no partial rows"
    );
}

/// The ePMP-shaped radio document: scalars for the radio block, and an
/// optional PHY rate column for the capacity tests.
fn radio_document() -> Document {
    adapters::parse(
        "epmp-radio.json",
        r#"{
  "schema_version": 2,
  "id": "epmp_radio",
  "detect": { "sys_object_id_prefixes": ["1.3.6.1.4.1.17713.21"] },
  "surfaces": [{ "id": "snmp", "transport": "snmp_v2c", "credential_ref": "snmp" }],
  "probes": [{
    "id": "radio",
    "surface": "snmp",
    "read": { "kind": "scalars", "scalars": [
      { "name": "model", "oid": "1.3.6.1.4.1.17713.21.1.4.14.0", "parser": "utf8" },
      { "name": "frequency_mhz", "oid": "1.3.6.1.4.1.17713.21.1.2.1.0", "parser": "u32" },
      { "name": "channel_width_mhz", "oid": "1.3.6.1.4.1.17713.21.1.2.2.0", "parser": "channel_width_mhz" },
      { "name": "sm_count", "oid": "1.3.6.1.4.1.17713.21.1.2.10.0", "parser": "u32" },
      { "name": "tx_power_dbm", "oid": "1.3.6.1.4.1.17713.21.1.2.5.0", "parser": "i64" },
      { "name": "lan_speed_mbps", "oid": "1.3.6.1.4.1.17713.21.1.4.12.0", "parser": "u32" },
      { "name": "phy_mbps", "oid": "1.3.6.1.4.1.17713.21.1.4.13.0", "parser": "u64" }
    ] }
  }],
  "maps": { "details": { "radio": "radio" } }
}"#,
    )
    .expect("the radio fixture validates")
}

/// The ePMP-shaped station document: one association-id index, a
/// DisplayString MAC column, and the downlink rate the capacity derivation
/// reads.
fn station_document() -> Document {
    adapters::parse(
        "epmp-stations.json",
        r#"{
  "schema_version": 2,
  "id": "epmp_stations",
  "detect": { "sys_object_id_prefixes": ["1.3.6.1.4.1.17713.21"] },
  "surfaces": [{ "id": "snmp", "transport": "snmp_v2c", "credential_ref": "snmp" }],
  "probes": [{
    "id": "stations",
    "surface": "snmp",
    "read": { "kind": "table", "table": {
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
    } }
  }],
  "maps": { "details": { "stations": "stations" } }
}"#,
    )
    .expect("the station fixture validates")
}

/// The combined AP document: radio scalars and the station table.
fn ap_document() -> Document {
    adapters::parse(
        "epmp-ap.json",
        r#"{
  "schema_version": 2,
  "id": "epmp_ap",
  "detect": { "sys_object_id_prefixes": ["1.3.6.1.4.1.17713.21"] },
  "surfaces": [{ "id": "snmp", "transport": "snmp_v2c", "credential_ref": "snmp" }],
  "probes": [
    {
      "id": "radio",
      "surface": "snmp",
      "read": { "kind": "scalars", "scalars": [
        { "name": "frequency_mhz", "oid": "1.3.6.1.4.1.17713.21.1.2.1.0", "parser": "u32" },
        { "name": "channel_width_mhz", "oid": "1.3.6.1.4.1.17713.21.1.2.2.0", "parser": "channel_width_mhz" },
        { "name": "phy_mbps", "oid": "1.3.6.1.4.1.17713.21.1.4.13.0", "parser": "u64" }
      ] }
    },
    {
      "id": "stations",
      "surface": "snmp",
      "read": { "kind": "table", "table": {
        "columns": [
          { "name": "mac", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.1", "parser": "display_mac" },
          { "name": "dl_rate_mbps", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.28", "parser": "u32" }
        ],
        "index": { "kind": "station" }
      } }
    }
  ],
  "maps": { "details": { "radio": "radio", "stations": "stations" } }
}"#,
    )
    .expect("the AP fixture validates")
}

const EPMP_STATION_ROOT: [u32; 12] = [1, 3, 6, 1, 4, 1, 17713, 21, 1, 2, 30, 1];

fn station_row(aid: u32, mac: WalkValue, dl_rate: Option<i64>) -> Vec<Varbind> {
    let mut row = vec![vb(at(&EPMP_STATION_ROOT, 1, &[aid]), mac)];
    if let Some(rate) = dl_rate {
        row.push(vb(at(&EPMP_STATION_ROOT, 28, &[aid]), integer(rate)));
    }
    row
}

/// Runs the station document over one walk of rows.
async fn run_stations(rows: Vec<Varbind>) -> Observation {
    run_stations_with_capacity(rows, &engine::CapacityPolicy::default()).await
}

/// Runs the station document over one walk of rows with an explicit policy.
async fn run_stations_with_capacity(
    rows: Vec<Varbind>,
    policy: &engine::CapacityPolicy,
) -> Observation {
    let document = station_document();
    let mut reader = Scripted::new().walk(&EPMP_STATION_ROOT, present(rows));
    engine::probe_with_capacity(
        &document,
        &mut reader,
        SystemTime::UNIX_EPOCH,
        &|| false,
        policy,
    )
    .await
}

/// Runs the radio document over one set of scalar answers.
async fn run_radio(answers: Vec<(&str, WalkValue)>) -> Observation {
    let document = radio_document();
    let mut reader = Scripted::new();
    for (text, value) in answers {
        let key = oid(text);
        let arcs = key.clone();
        reader = reader.get(&key, present(vec![vb(arcs, value)]));
    }
    engine::probe(&document, &mut reader, SystemTime::UNIX_EPOCH, &|| false).await
}

#[tokio::test]
async fn display_mac_accepts_colon_and_dash_forms_and_drops_garbage() {
    // Two separators, mixed case: every valid form maps the same MAC. A
    // wrong length, mixed separators, non-hex text, and junk are absent
    // fields (the station row is dropped, not guessed).
    let observation = run_stations(
        [
            (1, "bc:e6:7c:90:6a:2d"),
            (2, "BC-E6-7C-90-6A-2D"),
            (3, "Bc:E6:7c:90:6A:2d"),
        ]
        .into_iter()
        .flat_map(|(aid, mac)| station_row(aid, bytes(mac), Some(103)))
        .collect(),
    )
    .await;
    let stations = observation
        .details
        .as_ref()
        .expect("the station table maps")
        .stations
        .clone();
    assert_eq!(stations.len(), 3);
    assert_eq!(stations[0].mac.as_str(), "bc:e6:7c:90:6a:2d");

    for garbage in [
        "bc:e6:7c:90:6a",
        "bc:e6:7c:90:6a:2d:00",
        "bc:e6-7c:90:6a:2d",
        "zz:e6:7c:90:6a:2d",
        "garbage",
        "bc e6 7c 90 6a 2d",
    ] {
        let observation = run_stations(station_row(9, bytes(garbage), Some(103))).await;
        assert!(
            observation.stations.is_empty(),
            "`{garbage}` is not a six-octet DisplayString MAC"
        );
    }

    // A non-string value is absent too.
    let observation = run_stations(station_row(9, integer(42), Some(103))).await;
    assert!(observation.stations.is_empty());
}

#[tokio::test]
async fn station_rows_carry_the_extended_columns() {
    let observation = run_stations(vec![
        vb(at(&EPMP_STATION_ROOT, 1, &[7]), bytes("bc:e6:7c:90:6a:2d")),
        vb(at(&EPMP_STATION_ROOT, 10, &[7]), bytes("172.30.1.98")),
        vb(at(&EPMP_STATION_ROOT, 3, &[7]), integer(40)),
        vb(at(&EPMP_STATION_ROOT, 4, &[7]), integer(-70)),
        vb(at(&EPMP_STATION_ROOT, 5, &[7]), integer(-75)),
        vb(at(&EPMP_STATION_ROOT, 6, &[7]), integer(25)),
        vb(at(&EPMP_STATION_ROOT, 7, &[7]), integer(21)),
        vb(at(&EPMP_STATION_ROOT, 19, &[7]), integer(36)),
        vb(at(&EPMP_STATION_ROOT, 20, &[7]), integer(95)),
        vb(at(&EPMP_STATION_ROOT, 28, &[7]), integer(103)),
        vb(at(&EPMP_STATION_ROOT, 38, &[7]), bytes("5 GHz Force 4525")),
        vb(at(&EPMP_STATION_ROOT, 43, &[7]), bytes("5.10.3")),
    ])
    .await;
    let station = &observation
        .details
        .as_ref()
        .expect("the station table maps")
        .stations[0];
    assert_eq!(station.mac.as_str(), "bc:e6:7c:90:6a:2d");
    assert_eq!(station.wlan_if, 0, "the station index is an association id");
    assert_eq!(station.last_ip.as_deref(), Some("172.30.1.98"));
    assert_eq!(station.channel, Some(40));
    assert_eq!(station.ul_rssi, Some(-70));
    assert_eq!(station.dl_rssi, Some(-75));
    assert_eq!(station.ul_snr, Some(25));
    assert_eq!(station.dl_snr, Some(21));
    assert_eq!(station.tx_capacity, Some(36));
    assert_eq!(station.tx_quality, Some(95));
    assert_eq!(station.dl_rate_mbps, Some(103));
    assert_eq!(station.model.as_deref(), Some("5 GHz Force 4525"));
    assert_eq!(station.software_version.as_deref(), Some("5.10.3"));
    assert_eq!(station.provenance, [dudarchy_poller::Source::SnmpVendor]);
}

#[tokio::test]
async fn mac_indexed_station_rows_decode_from_the_index_and_derive_per_sm_capacity() {
    // The Wave/LTU shape: the six-octet MAC is the index (Wave appends a
    // slot), the `mac` column must agree, and a Kbps capacity column feeds
    // the Mbps downlink rate the capacity derivation reads.
    let json = r#"
{
  "schema_version": 2,
  "id": "mac_stations",
  "detect": { "sys_object_id_prefixes": ["1.3.6.1.4.1.41112.1.11"] },
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
    let document = adapters::parse("mac_stations.json", json)
        .unwrap_or_else(|errors| panic!("the station_mac shape should validate: {errors:?}"));
    const ROOT: [u32; 12] = [1, 3, 6, 1, 4, 1, 41112, 1, 11, 2, 3, 1];
    let row = |index: [u32; 7], mac_text: &str, tx: i64, rx: i64| {
        vec![
            vb(at(&ROOT, 1, &index), mac(mac_text)),
            vb(at(&ROOT, 17, &index), bytes("station")),
            vb(at(&ROOT, 9, &index), integer(tx)),
            vb(at(&ROOT, 10, &index), integer(rx)),
        ]
    };
    let rows: Vec<Varbind> = [
        row(
            [0, 17, 34, 51, 68, 85, 1],
            "00:11:22:33:44:55",
            2_000_000,
            1_000_000,
        ),
        row(
            [0, 17, 34, 51, 68, 102, 1],
            "00:11:22:33:44:66",
            1_000_000,
            1_000_000,
        ),
        // The column disagreeing with the index drops the row.
        row(
            [0, 17, 34, 51, 68, 119, 1],
            "00:11:22:33:44:88",
            1_000_000,
            1_000_000,
        ),
        // A short index is not a MAC and drops the row.
        vec![
            vb(at(&ROOT, 1, &[0, 17, 34, 51, 68]), mac("00:11:22:33:44:99")),
            vb(at(&ROOT, 9, &[0, 17, 34, 51, 68]), integer(1_000_000)),
        ],
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut reader = Scripted::new().walk(&ROOT, present(rows));
    let observation =
        engine::probe(&document, &mut reader, SystemTime::UNIX_EPOCH, &|| false).await;
    let stations = &observation
        .details
        .as_ref()
        .expect("the station table maps")
        .stations;
    assert_eq!(stations.len(), 2);
    assert_eq!(stations[0].mac.as_str(), "00:11:22:33:44:55");
    assert_eq!(stations[0].wlan_if, 0, "the MAC index is not an interface");
    assert_eq!(stations[0].hostname.as_deref(), Some("station"));
    assert_eq!(stations[0].tx_capacity, Some(2_000_000));
    assert_eq!(stations[0].rx_capacity, Some(1_000_000));
    assert_eq!(stations[0].dl_rate_mbps, Some(2000));
    assert_eq!(stations[1].dl_rate_mbps, Some(1000));
    assert_eq!(observation.capacity.mbps, Some(2000.0));
    assert_eq!(
        observation.capacity.source,
        dudarchy_poller::CapacitySource::PerSmRate
    );
}

#[tokio::test]
async fn rate_suffix_decodes_m_suffix_and_rejects_other_text() {
    let json = r#"
{
  "schema_version": 2,
  "id": "rate_suffix",
  "detect": { "sys_object_id_prefixes": ["1.3.6.1.4.1.17713.21"] },
  "surfaces": [{ "id": "snmp", "transport": "snmp_v2c", "credential_ref": "snmp" }],
  "probes": [{
    "id": "stations",
    "surface": "snmp",
    "read": {
      "kind": "table",
      "table": {
        "columns": [
          { "name": "mac", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.1", "parser": "display_mac" },
          { "name": "dl_rate_mbps", "oid": "1.3.6.1.4.1.17713.21.1.2.30.1.28", "parser": "rate_suffix" }
        ],
        "index": { "kind": "station" }
      }
    }
  }],
  "maps": { "details": { "stations": "stations" } }
}
"#;
    let document = adapters::parse("rate_suffix.json", json)
        .unwrap_or_else(|errors| panic!("the rate_suffix shape should validate: {errors:?}"));
    const ROOT: [u32; 12] = [1, 3, 6, 1, 4, 1, 17713, 21, 1, 2, 30, 1];
    let mut rows = Vec::new();
    for (aid, mac, rate) in [
        (1, "00:11:22:33:44:55", WalkValue::Bytes(b"103M".to_vec())),
        (2, "00:11:22:33:44:66", WalkValue::Bytes(b"275M".to_vec())),
        (3, "00:11:22:33:44:77", WalkValue::Bytes(b"junk".to_vec())),
        (4, "00:11:22:33:44:88", WalkValue::Bytes(b"103".to_vec())),
        (5, "00:11:22:33:44:99", WalkValue::Bytes(b"1G".to_vec())),
        (6, "00:11:22:33:44:aa", integer(103)),
        (7, "00:11:22:33:44:bb", WalkValue::Bytes(b"0M".to_vec())),
    ] {
        rows.push(vb(at(&ROOT, 1, &[aid]), bytes(mac)));
        rows.push(vb(at(&ROOT, 28, &[aid]), rate));
    }
    let mut reader = Scripted::new().walk(&ROOT, present(rows));
    let observation =
        engine::probe(&document, &mut reader, SystemTime::UNIX_EPOCH, &|| false).await;
    let stations = &observation
        .details
        .as_ref()
        .expect("the station table maps")
        .stations;
    assert_eq!(
        stations.len(),
        7,
        "invalid rates are absent fields, not dropped rows"
    );
    assert_eq!(stations[0].dl_rate_mbps, Some(103));
    assert_eq!(stations[1].dl_rate_mbps, Some(275));
    for station in &stations[2..] {
        assert_eq!(station.dl_rate_mbps, None);
    }
    assert_eq!(observation.capacity.mbps, Some(275.0));
    assert_eq!(
        observation.capacity.source,
        dudarchy_poller::CapacitySource::PerSmRate
    );
}

#[tokio::test]
async fn channel_width_enum_decodes_and_invalid_codes_stay_absent() {
    for (code, mhz) in [
        (1, 20),
        (2, 40),
        (3, 10),
        (4, 5),
        (5, 80),
        (6, 160),
        (7, 320),
    ] {
        let observation = run_radio(vec![
            ("1.3.6.1.4.1.17713.21.1.2.1.0", integer(5875)),
            ("1.3.6.1.4.1.17713.21.1.2.2.0", integer(code)),
        ])
        .await;
        let radio = observation
            .details
            .as_ref()
            .expect("the scalar answers map")
            .radio
            .as_ref()
            .expect("the radio slot maps");
        assert_eq!(radio.frequency_mhz, Some(5875));
        assert_eq!(
            radio.channel_width_mhz,
            Some(mhz),
            "enum {code} is {mhz} MHz"
        );
    }
    for code in [0, 8] {
        let observation = run_radio(vec![
            ("1.3.6.1.4.1.17713.21.1.2.1.0", integer(5875)),
            ("1.3.6.1.4.1.17713.21.1.2.2.0", integer(code)),
        ])
        .await;
        let radio = observation
            .details
            .as_ref()
            .unwrap()
            .radio
            .as_ref()
            .unwrap();
        assert_eq!(radio.channel_width_mhz, None, "enum {code} is undefined");
    }
}

#[tokio::test]
async fn capacity_derivation_matrix_uses_the_model_trust_factor_uniformly() {
    let policy = engine::CapacityPolicy::default();

    // PHY 1000 with the default (unknown-model) factor 1.0.
    let observation = {
        let document = radio_document();
        let mut reader = Scripted::new().get(
            &oid("1.3.6.1.4.1.17713.21.1.4.13.0"),
            present(vec![vb(
                oid("1.3.6.1.4.1.17713.21.1.4.13.0"),
                WalkValue::Counter64(1000),
            )]),
        );
        engine::probe_with_capacity(
            &document,
            &mut reader,
            SystemTime::UNIX_EPOCH,
            &|| false,
            &policy,
        )
        .await
    };
    assert_eq!(observation.capacity.mbps, Some(1000.0));
    assert_eq!(
        observation.capacity.source,
        dudarchy_poller::CapacitySource::Phy
    );

    // A per-SM rate of 1000 with the default factor 1.0 is used verbatim.
    let observation = run_stations(station_row(1, bytes("bc:e6:7c:90:6a:2d"), Some(1000))).await;
    assert_eq!(observation.capacity.mbps, Some(1000.0));
    assert_eq!(
        observation.capacity.source,
        dudarchy_poller::CapacitySource::PerSmRate
    );

    // The same factor applies to the per-SM source: 1000 * 0.85 = 850.
    let overridden = engine::CapacityPolicy {
        by_model: [("ePMP 4500L".to_owned(), 0.85)].into(),
        model: Some("ePMP 4500L".to_owned()),
        ..policy.clone()
    };
    let observation = run_stations_with_capacity(
        station_row(1, bytes("bc:e6:7c:90:6a:2d"), Some(1000)),
        &overridden,
    )
    .await;
    assert_eq!(observation.capacity.mbps, Some(850.0));
    assert_eq!(
        observation.capacity.source,
        dudarchy_poller::CapacitySource::PerSmRate
    );

    // A reported model keys the factor even when the CRM model is absent.
    let observation = {
        let document = radio_document();
        let mut reader = Scripted::new()
            .get(
                &oid("1.3.6.1.4.1.17713.21.1.4.14.0"),
                present(vec![vb(
                    oid("1.3.6.1.4.1.17713.21.1.4.14.0"),
                    bytes("ePMP 4500L"),
                )]),
            )
            .get(
                &oid("1.3.6.1.4.1.17713.21.1.4.13.0"),
                present(vec![vb(
                    oid("1.3.6.1.4.1.17713.21.1.4.13.0"),
                    WalkValue::Counter64(1000),
                )]),
            );
        let policy = engine::CapacityPolicy {
            by_model: [("ePMP 4500L".to_owned(), 0.85)].into(),
            ..policy.clone()
        };
        engine::probe_with_capacity(
            &document,
            &mut reader,
            SystemTime::UNIX_EPOCH,
            &|| false,
            &policy,
        )
        .await
    };
    assert_eq!(observation.capacity.mbps, Some(850.0));
    assert_eq!(
        observation.capacity.source,
        dudarchy_poller::CapacitySource::Phy
    );

    // An unknown model falls back to the 1.0 default.
    let unknown_model = engine::CapacityPolicy {
        by_model: [("ePMP 4500L".to_owned(), 0.85)].into(),
        model: Some("Other Model".to_owned()),
        ..policy.clone()
    };
    let observation = run_stations_with_capacity(
        station_row(1, bytes("bc:e6:7c:90:6a:2d"), Some(1000)),
        &unknown_model,
    )
    .await;
    assert_eq!(observation.capacity.mbps, Some(1000.0));
    assert_eq!(
        observation.capacity.source,
        dudarchy_poller::CapacitySource::PerSmRate
    );

    // The per-SM rate beats the CRM rate; the CRM rate stands alone when
    // nothing was polled.
    let with_crm = engine::CapacityPolicy {
        crm_rate_mbps: Some(250.0),
        ..policy.clone()
    };
    let observation = run_stations_with_capacity(
        station_row(1, bytes("bc:e6:7c:90:6a:2d"), Some(1000)),
        &with_crm,
    )
    .await;
    assert_eq!(observation.capacity.mbps, Some(1000.0));
    assert_eq!(
        observation.capacity.source,
        dudarchy_poller::CapacitySource::PerSmRate
    );

    let observation = {
        let document = radio_document();
        let mut reader = Scripted::new().get(
            &oid("1.3.6.1.4.1.17713.21.1.2.1.0"),
            present(vec![vb(oid("1.3.6.1.4.1.17713.21.1.2.1.0"), integer(5875))]),
        );
        engine::probe_with_capacity(
            &document,
            &mut reader,
            SystemTime::UNIX_EPOCH,
            &|| false,
            &with_crm,
        )
        .await
    };
    assert_eq!(observation.capacity.mbps, Some(250.0));
    assert_eq!(
        observation.capacity.source,
        dudarchy_poller::CapacitySource::Crm
    );

    // Nothing usable is unknown, and never a document value.
    let observation = {
        let document = radio_document();
        let mut reader = Scripted::new().get(
            &oid("1.3.6.1.4.1.17713.21.1.2.1.0"),
            present(vec![vb(oid("1.3.6.1.4.1.17713.21.1.2.1.0"), integer(5875))]),
        );
        engine::probe(&document, &mut reader, SystemTime::UNIX_EPOCH, &|| false).await
    };
    assert_eq!(observation.capacity.mbps, None);
    assert_eq!(
        observation.capacity.source,
        dudarchy_poller::CapacitySource::Unknown
    );
}

#[tokio::test]
async fn airos_stations_feed_the_station_detail() {
    // The shipped airOS document's station probe: the table is the read, and
    // the engine's existing `ubnt_station` arm turns its rows into the
    // station detail with airOS provenance.
    let airos = shipped("airos.json");
    let station_root = [1, 3, 6, 1, 4, 1, 41112, 1, 4, 7, 1];
    let station = [1, 0x00, 0x27, 0x22, 0x1a, 0xf9, 0x4e]; // wlan-if, then the MAC octets
    let mut reader = Scripted::new().walk(
        &station_root,
        present(vec![
            vb(at(&station_root, 2, &station), bytes("station-01")),
            vb(at(&station_root, 3, &station), integer(-47)),
            vb(at(&station_root, 4, &station), integer(-92)),
            vb(
                at(&station_root, 10, &station),
                WalkValue::IpAddress("172.30.1.98".to_owned()),
            ),
            vb(at(&station_root, 11, &station), integer(117000000)),
            vb(at(&station_root, 12, &station), integer(78000000)),
            vb(at(&station_root, 13, &station), WalkValue::Counter64(4096)),
            vb(at(&station_root, 14, &station), WalkValue::Counter64(2048)),
        ]),
    );
    let observation = run(&airos, &mut reader).await;

    assert_eq!(
        report(&observation, "stations").outcome,
        OutcomeKind::Present
    );
    let details = observation
        .details
        .as_ref()
        .expect("the station answer feeds the details");
    assert_eq!(details.stations.len(), 1);
    let station = &details.stations[0];
    assert_eq!(station.mac.as_str(), "00:27:22:1a:f9:4e");
    assert_eq!(station.wlan_if, 1);
    assert_eq!(station.hostname.as_deref(), Some("station-01"));
    assert_eq!(station.signal_dbm, Some(-47));
    assert_eq!(station.noise_dbm, Some(-92));
    assert_eq!(station.last_ip.as_deref(), Some("172.30.1.98"));
    assert_eq!(
        station.tx_bps,
        Some(117000000),
        "column 11 is ubntStaTxRate, an Integer32 the document decodes as i64"
    );
    assert_eq!(
        station.rx_bps,
        Some(78000000),
        "column 12 is ubntStaRxRate, an Integer32 the document decodes as i64"
    );
    assert_eq!(
        station.tx_bytes,
        Some(4096),
        "column 13 is ubntStaTxBytes, the AP's transmit counter"
    );
    assert_eq!(
        station.rx_bytes,
        Some(2048),
        "column 14 is ubntStaRxBytes, the AP's receive counter"
    );
    assert_eq!(station.provenance, [dudarchy_poller::Source::AirOsStations]);

    // An answered-but-empty table and a declined one stay distinct answers.
    let mut reader = Scripted::new().walk(&station_root, Outcome::Empty);
    let observation = run(&airos, &mut reader).await;
    assert_eq!(report(&observation, "stations").outcome, OutcomeKind::Empty);
    assert!(observation.stations.is_empty());

    let mut reader = Scripted::new().walk(&station_root, Outcome::Absent);
    let observation = run(&airos, &mut reader).await;
    assert_eq!(
        report(&observation, "stations").outcome,
        OutcomeKind::Absent
    );
    assert!(observation.stations.is_empty());
}

#[tokio::test]
async fn netonix_vendor_probes_feed_system_and_temperatures() {
    // The shipped Netonix document maps its enterprise tree onto the system
    // facts and the temperature list; the fields with no detail surface stay
    // named residuals instead of leaking into the model.
    let netonix = shipped("netonix.json");
    let mut reader = Scripted::new()
        .get(
            &oid("1.3.6.1.4.1.46242.1.0"),
            present(vec![vb(oid("1.3.6.1.4.1.46242.1.0"), bytes("1.5.14"))]),
        )
        .walk(
            &NETONIX_TEMPS,
            present(vec![
                vb(at(&NETONIX_TEMPS, 2, &[1]), bytes("Board Temp")),
                vb(at(&NETONIX_TEMPS, 2, &[2]), bytes("CPU Temp")),
                vb(at(&NETONIX_TEMPS, 3, &[1]), integer(64)),
                vb(at(&NETONIX_TEMPS, 3, &[2]), integer(94)),
            ]),
        );
    let observation = run(&netonix, &mut reader).await;

    assert_eq!(report(&observation, "vendor").outcome, OutcomeKind::Present);
    assert_eq!(
        report(&observation, "temperatures").outcome,
        OutcomeKind::Present
    );
    let details = observation.details.as_ref().expect("the vendor reads map");
    let system = details
        .system
        .as_ref()
        .expect("the firmware string feeds the system facts");
    assert_eq!(system.version.as_deref(), Some("1.5.14"));
    assert_eq!(system.board_name, None, "no model scalar is carried");
    assert_eq!(system.provenance, [dudarchy_poller::Source::SnmpVendor]);

    assert_eq!(details.temperatures.len(), 2);
    assert_eq!(details.temperatures[0].sensor_name, "Board Temp");
    assert_eq!(details.temperatures[0].celsius, 64.0);
    assert_eq!(
        details.temperatures[0].provenance,
        [dudarchy_poller::Source::SnmpVendor]
    );
    assert_eq!(details.temperatures[0].observed_at, SystemTime::UNIX_EPOCH);
    assert_eq!(details.temperatures[1].sensor_name, "CPU Temp");
    assert_eq!(details.temperatures[1].celsius, 94.0);

    // Nothing the vendor tree cannot express reaches the model: those reads
    // are documented residuals on the document, not engine residuals.
    assert!(
        observation.residuals.is_empty(),
        "the mapped vendor probes are not leftovers: {:?}",
        observation.residuals
    );
    let residual_names: Vec<&str> = netonix
        .maps
        .residuals
        .iter()
        .map(|residual| residual.name.as_str())
        .collect();
    assert_eq!(
        residual_names,
        ["total-draw", "psu-rails", "poe-mode", "port-uptime"]
    );
}

#[test]
fn the_shipped_documents_select_by_pen_and_sysdescr_without_ambiguity() {
    let documents: Vec<Document> = [
        "airos.json",
        "edgeos.json",
        "generic-base.json",
        "netonix.json",
        "procurve.json",
        "routeros.json",
        "swos.json",
    ]
    .iter()
    .map(|name| shipped(name))
    .collect();

    match engine::select(
        &documents,
        Some("1.3.6.1.4.1.10002.1"),
        Some("Linux 2.6.32 airOS"),
    ) {
        Selection::Selected(document) => assert_eq!(document.id, "airos"),
        other => panic!("expected the airOS document, got {other:?}"),
    }
    // The station table's own enterprise arc selects the same document.
    match engine::select(&documents, Some("1.3.6.1.4.1.41112.1.4"), None) {
        Selection::Selected(document) => assert_eq!(document.id, "airos"),
        other => panic!("expected the airOS document, got {other:?}"),
    }
    match engine::select(
        &documents,
        Some("1.3.6.1.4.1.41112.1.5"),
        Some("EdgeOS v2.0.9"),
    ) {
        Selection::Selected(document) => assert_eq!(document.id, "edgeos"),
        other => panic!("expected the EdgeOS document, got {other:?}"),
    }
    match engine::select(&documents, Some("1.3.6.1.4.1.14988.2"), Some("SwOS 2.13")) {
        Selection::Selected(document) => assert_eq!(document.id, "swos"),
        other => panic!("expected the SwOS document, got {other:?}"),
    }
    match engine::select(&documents, Some("1.3.6.1.4.1.46242"), Some("Netonix")) {
        Selection::Selected(document) => assert_eq!(document.id, "netonix"),
        other => panic!("expected the Netonix document, got {other:?}"),
    }
    match engine::select(
        &documents,
        Some("1.3.6.1.4.1.14988.1"),
        Some("RouterOS 6.49"),
    ) {
        Selection::Selected(document) => assert_eq!(document.id, "routeros"),
        other => panic!("expected the RouterOS document, got {other:?}"),
    }
    match engine::select(
        &documents,
        Some("1.3.6.1.4.1.11.2.3.7.11"),
        Some("ProCurve J9263A"),
    ) {
        Selection::Selected(document) => assert_eq!(document.id, "procurve"),
        other => panic!("expected the ProCurve document, got {other:?}"),
    }
    // A responder no hint claims still falls to the one default.
    match engine::select(&documents, Some("1.3.6.1.4.1.99999"), Some("AcmeOS")) {
        Selection::Selected(document) => assert_eq!(document.id, "generic-base"),
        other => panic!("expected the generic base, got {other:?}"),
    }
}

#[test]
fn limits_resolve_against_the_core_defaults() {
    let mut budget = adapters::Budget::default();
    let limits = engine::Limits::resolve(&budget);
    assert_eq!(limits.max_requests, engine::DEFAULT_MAX_REQUESTS);
    assert_eq!(limits.max_rows, engine::DEFAULT_MAX_ROWS);
    assert_eq!(limits.max_bytes, engine::DEFAULT_MAX_BYTES);

    budget.max_requests = Some(8);
    budget.deadline_ms = Some(5);
    let limits = engine::Limits::resolve(&budget);
    assert_eq!(limits.max_requests, 8);
    assert_eq!(limits.deadline, std::time::Duration::from_millis(5));
}

#[tokio::test]
async fn qbridge_documents_decode_vlan_mac_and_the_base_port_join() {
    // RouterOS: Q-BRIDGE with a real bridge-port join.
    let routeros_switch = document("routeros-switch.json");
    let mut reader = Scripted::new()
        .walk(
            &IF_TABLE,
            present(vec![
                vb(at(&IF_TABLE, 3, &[3]), integer(6)),
                vb(at(&IF_TABLE, 8, &[3]), integer(1)),
            ]),
        )
        .walk(
            &IF_X_TABLE,
            present(vec![vb(at(&IF_X_TABLE, 1, &[3]), bytes("port3"))]),
        )
        .walk(
            &QBRIDGE_ENTRY,
            present(vec![
                vb(at(&QBRIDGE_ENTRY, 2, &[0, 2, 0, 0, 0, 0, 2]), integer(5)),
                vb(at(&QBRIDGE_ENTRY, 3, &[0, 2, 0, 0, 0, 0, 2]), integer(3)),
                vb(at(&QBRIDGE_ENTRY, 2, &[0, 2, 0, 0, 0, 0, 1]), integer(3)),
                vb(at(&QBRIDGE_ENTRY, 3, &[0, 2, 0, 0, 0, 0, 1]), integer(3)),
            ]),
        )
        .walk(
            &BASE_PORT,
            present(vec![
                vb([BASE_PORT.as_slice(), &[3]].concat(), integer(16)),
                vb([BASE_PORT.as_slice(), &[5]].concat(), integer(17)),
            ]),
        );
    let observation = run(&routeros_switch, &mut reader).await;

    assert!(observation.supported());
    let fdb = observation.fdb.as_ref().expect("Q-BRIDGE rows");
    assert!(fdb.bridge.is_none(), "a Q-BRIDGE document");
    let rows = fdb.qbridge.as_ref().expect("qbridge rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].vlan, 0);
    assert_eq!(rows[0].mac, [0x02, 0, 0, 0, 0, 0x01]);
    assert_eq!(rows[0].port, Some(3));
    assert_eq!(rows[0].status, Some(3));
    assert_eq!(rows[1].mac, [0x02, 0, 0, 0, 0, 0x02]);
    assert_eq!(rows[1].port, Some(5));
    assert_eq!(
        fdb.port_if_index,
        [(3, 16), (5, 17)],
        "the join comes from dot1dBasePortIfIndex, not identity"
    );

    // ProCurve: the MAC decodes from the OID index; the document has no
    // MAC column at all.
    let procurve = document("procurve-switch.json");
    let mut reader = Scripted::new().walk(
        &QBRIDGE_ENTRY,
        present(vec![
            vb(
                at(&QBRIDGE_ENTRY, 2, &[100, 0, 12, 66, 170, 187, 204]),
                integer(290),
            ),
            vb(
                at(&QBRIDGE_ENTRY, 3, &[100, 0, 12, 66, 170, 187, 204]),
                integer(3),
            ),
            vb(at(&QBRIDGE_ENTRY, 2, &[50, 0, 12, 66, 0, 0, 1]), integer(1)),
            vb(at(&QBRIDGE_ENTRY, 3, &[50, 0, 12, 66, 0, 0, 1]), integer(3)),
        ]),
    );
    let observation = run(&procurve, &mut reader).await;

    assert!(observation.supported());
    let fdb = observation.fdb.as_ref().expect("Q-BRIDGE rows");
    let rows = fdb.qbridge.as_ref().expect("qbridge rows");
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].vlan, rows[0].port), (50, Some(1)));
    assert_eq!(rows[0].mac, [0x00, 0x0c, 0x42, 0x00, 0x00, 0x01]);
    assert_eq!((rows[1].vlan, rows[1].port), (100, Some(290)));
    assert_eq!(rows[1].mac, [0x00, 0x0c, 0x42, 0xaa, 0xbb, 0xcc]);
    assert_eq!(
        fdb.port_if_index,
        [(1, 1), (290, 290)],
        "an identity document"
    );
}

#[tokio::test]
async fn zero_mac_chassis_rows_are_dropped_in_both_encodings() {
    // The same junk row reaches the engine two ways: six zero octets (a raw
    // MAC) or the text `00:00:00:00:00:00` some agents pre-format. Neither
    // may become a neighbour; the valid row beside them must survive.
    let document = document("airos-ap.json");
    let valid = [0x00, 0x1f, 0x5e, 0xaa, 0xbb, 0xcc];
    let mut reader = Scripted::new()
        .walk(
            &LLDP_REM_TABLE,
            present(vec![
                vb(
                    at(&LLDP_REM_ENTRY, 5, &[0, 1, 1]),
                    WalkValue::Bytes(vec![0; 6]),
                ),
                vb(
                    at(&LLDP_REM_ENTRY, 5, &[0, 1, 2]),
                    bytes("00:00:00:00:00:00"),
                ),
                vb(at(&LLDP_REM_ENTRY, 5, &[0, 1, 3]), mac("00:1f:5e:aa:bb:cc")),
            ]),
        )
        .walk(&LLDP_MAN_TABLE, Outcome::Empty)
        .walk(&LLDP_LOC_TABLE, Outcome::Empty);
    let observation = run(&document, &mut reader).await;

    assert_eq!(report(&observation, "lldp").outcome, OutcomeKind::Present);
    assert_eq!(
        observation.lldp.len(),
        1,
        "both all-zero chassis rows are dropped: {:?}",
        observation.lldp
    );
    assert_eq!(observation.lldp[0].chassis_id, valid);
}

#[tokio::test]
async fn a_declined_lldp_table_is_reported_as_answered() {
    // An agent that declines the remote-systems table with `noSuchObject`
    // answered the request; the feed is known-empty, not unknown. The
    // engine reports that distinction for its callers.
    let document = document("airos-ap.json");
    let mut reader = Scripted::new().walk(&LLDP_REM_TABLE, Outcome::Absent);
    let observation = run(&document, &mut reader).await;

    assert_eq!(report(&observation, "lldp").outcome, OutcomeKind::Absent);
    assert!(observation.lldp_answered, "a declined table is an answer");
    assert!(observation.lldp.is_empty());
}

#[tokio::test]
async fn cancellation_between_inner_walks_stops_the_run() {
    let document = document("airos-ap.json");
    let flag = Arc::new(AtomicBool::new(false));
    let mut reader = Scripted::new()
        .get(
            &oid("1.3.6.1.2.1.1.1.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.1.0"), bytes("airOS"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.5.0"),
            present(vec![vb(oid("1.3.6.1.2.1.1.5.0"), bytes("AP-1"))]),
        )
        .get(
            &oid("1.3.6.1.2.1.1.2.0"),
            present(vec![vb(
                oid("1.3.6.1.2.1.1.2.0"),
                WalkValue::Oid("1.3.6.1.4.1.10002.1".to_owned()),
            )]),
        )
        .walk(
            &LLDP_REM_TABLE,
            present(vec![vb(
                at(&LLDP_REM_ENTRY, 5, &[0, 1, 1]),
                mac("00:1f:5e:aa:bb:cc"),
            )]),
        )
        // The fourth read (the remote-systems walk) raises the flag; the
        // management-address walk must see it before it starts.
        .cancelled_after(4, Arc::clone(&flag));
    let observation = engine::probe(&document, &mut reader, SystemTime::UNIX_EPOCH, &|| {
        flag.load(Ordering::SeqCst)
    })
    .await;

    assert_eq!(report(&observation, "lldp").outcome, OutcomeKind::Cancelled);
    assert_eq!(
        report(&observation, "stations").outcome,
        OutcomeKind::Cancelled,
        "the run stopped instead of reading on"
    );
    assert!(
        observation.supported(),
        "cancellation is not device unsupport; only a required read failing is"
    );
    assert!(
        observation.lldp.is_empty(),
        "an unassembled probe adds no rows"
    );
    assert!(
        reader.walks_requested.contains(&LLDP_REM_TABLE.to_vec()),
        "the remote-systems walk ran before the cancel point"
    );
    assert!(
        !reader.walks_requested.contains(&LLDP_MAN_TABLE.to_vec()),
        "the management-address walk must never be issued after the cancel point"
    );
    assert!(
        !reader.walks_requested.contains(&LLDP_LOC_TABLE.to_vec()),
        "the local-port walk must never be issued after the cancel point"
    );
}

#[tokio::test]
async fn cancellation_before_the_fdb_join_emits_no_partial_rows() {
    let routeros_switch = document("routeros-switch.json");
    let flag = Arc::new(AtomicBool::new(false));
    let mut reader = Scripted::new()
        .walk(
            &IF_TABLE,
            present(vec![
                vb(at(&IF_TABLE, 3, &[3]), integer(6)),
                vb(at(&IF_TABLE, 8, &[3]), integer(1)),
            ]),
        )
        .walk(
            &IF_X_TABLE,
            present(vec![vb(at(&IF_X_TABLE, 1, &[3]), bytes("port3"))]),
        )
        .walk(
            &QBRIDGE_ENTRY,
            present(vec![
                vb(at(&QBRIDGE_ENTRY, 2, &[0, 2, 0, 0, 0, 0, 1]), integer(3)),
                vb(at(&QBRIDGE_ENTRY, 3, &[0, 2, 0, 0, 0, 0, 1]), integer(3)),
            ]),
        )
        // The third read (the Q-BRIDGE walk) raises the flag, so the
        // base-port join must never start.
        .cancelled_after(3, Arc::clone(&flag));
    let observation = engine::probe(
        &routeros_switch,
        &mut reader,
        SystemTime::UNIX_EPOCH,
        &|| flag.load(Ordering::SeqCst),
    )
    .await;

    assert_eq!(report(&observation, "fdb").outcome, OutcomeKind::Cancelled);
    assert!(
        reader.walks_requested.contains(&QBRIDGE_ENTRY.to_vec()),
        "the table walk ran before the cancel point"
    );
    assert!(
        !reader.walks_requested.contains(&BASE_PORT.to_vec()),
        "the join walk must never be issued after the cancel point"
    );
    assert!(
        observation.fdb.is_none(),
        "a cancelled decode commits no partial rows"
    );
    assert!(
        !observation.supported(),
        "the required FDB probe did not answer"
    );
}

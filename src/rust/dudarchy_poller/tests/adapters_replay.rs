//! The adapter record/replay harness: one manifest of cases, each pairing an
//! adapter document with a recorded transcript and the observation it must
//! produce. Failures name the case, the document, the step or probe, and the
//! first divergent value.
//!
//! Offline by construction: the transcript implements the engine's `Reader`
//! seam, so no socket, vault, wasm target, or environment is involved.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::{Value, json};

use dudarchy_poller::adapters::engine::{self, Observation};
use dudarchy_poller::adapters::reader::{CommandRow, Outcome, OutcomeKind, Reader, Varbind};
use dudarchy_poller::adapters::{self, CommandName, Document};
use dudarchy_poller::{AirStation, ArpRow, LldpNeighbour, Mac, SnmpFdb, WalkValue};

/// The replay fixture root.
fn replay_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adapters/replay")
}

// --- manifest -------------------------------------------------------------

/// The case list. Every case names its document, transcript, and expectation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    cases: Vec<Case>,
}

/// One replay case.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    document: String,
    /// The document surface the engine runs; every probe when absent.
    #[serde(default)]
    surface: Option<String>,
    transcript: String,
    expect: String,
}

// --- transcript -----------------------------------------------------------

/// One recorded transcript: the reads the engine issues, in order, with the
/// answer each one gave.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transcript {
    schema_version: u32,
    steps: Vec<Step>,
}

/// One recorded read, before validation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    op: String,
    #[serde(default)]
    oid: Option<String>,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    command: Option<String>,
    outcome: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    varbinds: Vec<VarbindSpec>,
    #[serde(default)]
    rows: Vec<BTreeMap<String, String>>,
}

/// One recorded varbind.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VarbindSpec {
    oid: String,
    value: ValueSpec,
}

/// One recorded value, typed by the document parser vocabulary.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValueSpec {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    hex: Option<String>,
    #[serde(default)]
    value: Option<Value>,
}

/// A validated step: the request it answers and the typed response.
struct PreparedStep {
    request: String,
    response: Response,
}

/// A step's typed response.
enum Response {
    Snmp(Outcome<Vec<Varbind>>),
    Command(Outcome<Vec<CommandRow>>),
}

/// Validates the transcript into prepared steps, naming the step on failure.
fn prepare(transcript: Transcript, document: &str) -> Result<Vec<PreparedStep>, String> {
    if transcript.schema_version != 1 {
        return Err(format!(
            "transcript schema_version {} is not 1",
            transcript.schema_version
        ));
    }
    let mut prepared = Vec::new();
    for (index, step) in transcript.steps.into_iter().enumerate() {
        let number = index + 1;
        let fail = |reason: String| format!("{document}: transcript step {number}: {reason}");
        match step.op.as_str() {
            "get" | "walk" => {
                if step.command.is_some() {
                    return Err(fail("`command` is not valid on a get/walk step".to_owned()));
                }
                if !step.rows.is_empty() {
                    return Err(fail("`rows` is not valid on a get/walk step".to_owned()));
                }
                let key = if step.op == "get" {
                    if step.root.is_some() {
                        return Err(fail("`root` is not valid on a get step".to_owned()));
                    }
                    step.oid
                        .clone()
                        .ok_or_else(|| fail("`get` needs its `oid`".to_owned()))?
                } else {
                    if step.oid.is_some() {
                        return Err(fail("`oid` is not valid on a walk step".to_owned()));
                    }
                    step.root
                        .clone()
                        .ok_or_else(|| fail("`walk` needs its `root`".to_owned()))?
                };
                let varbinds = step
                    .varbinds
                    .iter()
                    .map(|spec| varbind(spec).map_err(fail))
                    .collect::<Result<Vec<_>, _>>()?;
                let outcome = match step.outcome.as_str() {
                    "present" => Outcome::Present(varbinds),
                    "empty" => Outcome::Empty,
                    "absent" => Outcome::Absent,
                    "timeout" => Outcome::Timeout,
                    "error" => Outcome::Error {
                        reason: step.reason.clone().ok_or_else(|| {
                            fail("an `error` outcome needs a `reason`".to_owned())
                        })?,
                    },
                    "cancelled" => Outcome::Cancelled,
                    other => return Err(fail(format!("unknown outcome `{other}`"))),
                };
                if !matches!(outcome, Outcome::Present(_) | Outcome::Error { .. })
                    && !step.varbinds.is_empty()
                {
                    return Err(fail("only a present outcome carries `varbinds`".to_owned()));
                }
                // A get names one OID, so its answer is one varbind or none at
                // all: an empty table is a walk outcome, never a get's.
                if step.op == "get" {
                    if matches!(outcome, Outcome::Empty) {
                        return Err(fail("a `get` cannot answer `empty`".to_owned()));
                    }
                    if matches!(&outcome, Outcome::Present(varbinds) if varbinds.len() != 1) {
                        return Err(fail(
                            "a `present` get must carry exactly one varbind".to_owned(),
                        ));
                    }
                }
                prepared.push(PreparedStep {
                    request: format!("{} {}", step.op, key),
                    response: Response::Snmp(outcome),
                });
            }
            "command" => {
                let raw = step
                    .command
                    .clone()
                    .ok_or_else(|| fail("`command` needs its `command`".to_owned()))?;
                let name =
                    command_name(&raw).ok_or_else(|| fail(format!("unknown command `{raw}`")))?;
                if step.oid.is_some() || step.root.is_some() {
                    return Err(fail(
                        "`oid`/`root` are not valid on a command step".to_owned(),
                    ));
                }
                if !step.varbinds.is_empty() {
                    return Err(fail("`varbinds` is not valid on a command step".to_owned()));
                }
                if step.outcome != "present" && !step.rows.is_empty() {
                    return Err(fail("only a present outcome carries `rows`".to_owned()));
                }
                let outcome = match step.outcome.as_str() {
                    "present" => Outcome::Present(step.rows),
                    "empty" => Outcome::Empty,
                    "absent" => Outcome::Absent,
                    "timeout" => Outcome::Timeout,
                    "error" => Outcome::Error {
                        reason: step.reason.clone().ok_or_else(|| {
                            fail("an `error` outcome needs a `reason`".to_owned())
                        })?,
                    },
                    "cancelled" => Outcome::Cancelled,
                    other => return Err(fail(format!("unknown outcome `{other}`"))),
                };
                prepared.push(PreparedStep {
                    request: format!("command {}", command_spelling(name)),
                    response: Response::Command(outcome),
                });
            }
            other => return Err(fail(format!("unknown op `{other}`"))),
        }
    }
    Ok(prepared)
}

/// One recorded value as the engine's walk value.
fn varbind(spec: &VarbindSpec) -> Result<Varbind, String> {
    let value = match spec.value.kind.as_str() {
        "octet_string" => match (&spec.value.text, &spec.value.hex) {
            (Some(text), None) => WalkValue::Bytes(text.as_bytes().to_vec()),
            (None, Some(hex)) => WalkValue::Bytes(parse_hex(hex)?),
            _ => {
                return Err("an octet_string needs exactly one of `text` or `hex`".to_owned());
            }
        },
        "integer" => WalkValue::Integer(number(&spec.value.value, "integer")?),
        "counter64" => WalkValue::Counter64(unsigned(&spec.value.value, "counter64")?),
        "time_ticks" => WalkValue::TimeTicks(unsigned(&spec.value.value, "time_ticks")?),
        "ip_address" => WalkValue::IpAddress(string(&spec.value.value, "ip_address")?),
        "oid" => WalkValue::Oid(string(&spec.value.value, "oid")?),
        "empty" => WalkValue::Empty,
        other => return Err(format!("unknown value type `{other}`")),
    };
    Ok(Varbind {
        oid: dotted(&spec.oid)?,
        value,
    })
}

/// A dotted OID string as arcs.
fn dotted(text: &str) -> Result<Vec<u32>, String> {
    text.strip_prefix('.')
        .unwrap_or(text)
        .split('.')
        .map(|arc| {
            arc.parse()
                .map_err(|_| format!("`{text}` is not a dotted OID"))
        })
        .collect()
}

/// Colon-separated hex octets.
fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    text.split(':')
        .map(|pair| u8::from_str_radix(pair, 16).map_err(|_| format!("`{text}` is not hex octets")))
        .collect()
}

/// A JSON number as a signed integer.
fn number(value: &Option<Value>, kind: &str) -> Result<i64, String> {
    value
        .as_ref()
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{kind} needs an integer `value`"))
}

/// A JSON number as an unsigned integer.
fn unsigned(value: &Option<Value>, kind: &str) -> Result<u64, String> {
    value
        .as_ref()
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{kind} needs an unsigned `value`"))
}

/// A JSON string value.
fn string(value: &Option<Value>, kind: &str) -> Result<String, String> {
    value
        .as_ref()
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{kind} needs a string `value`"))
}

/// The document spelling of a command.
fn command_name(name: &str) -> Option<CommandName> {
    Some(match name {
        "system_identity" => CommandName::SystemIdentity,
        "system_routerboard" => CommandName::SystemRouterboard,
        "ip_neighbor" => CommandName::IpNeighbor,
        "interfaces" => CommandName::Interfaces,
        "ip_address" => CommandName::IpAddress,
        "ip_arp" => CommandName::IpArp,
        "bridge_port" => CommandName::BridgePort,
        "bridge_vlan" => CommandName::BridgeVlan,
        "bridge_host" => CommandName::BridgeHost,
        "system_resource" => CommandName::SystemResource,
        "system_health" => CommandName::SystemHealth,
        "ip_firewall_nat" => CommandName::IpFirewallNat,
        "ip_dhcp_server_lease" => CommandName::IpDhcpServerLease,
        _ => return None,
    })
}

/// The reader that answers from a prepared transcript, in order.
struct TranscriptReader<'a> {
    steps: &'a [PreparedStep],
    next: usize,
    mismatch: Option<String>,
}

impl<'a> TranscriptReader<'a> {
    fn new(steps: &'a [PreparedStep]) -> TranscriptReader<'a> {
        TranscriptReader {
            steps,
            next: 0,
            mismatch: None,
        }
    }

    /// The next recorded step, when it answers `request` exactly.
    fn take(&mut self, request: &str) -> Option<&'a PreparedStep> {
        let Some(step) = self.steps.get(self.next) else {
            self.mismatch.get_or_insert_with(|| {
                format!(
                    "engine requested `{request}` after the transcript's last step ({})",
                    self.steps.len()
                )
            });
            return None;
        };
        if step.request != request {
            self.mismatch.get_or_insert_with(|| {
                format!(
                    "step {}: recorded `{}`, engine requested `{request}`",
                    self.next + 1,
                    step.request
                )
            });
            return None;
        }
        self.next += 1;
        Some(step)
    }

    /// The first divergence from the transcript, if any.
    fn fault(&self) -> Option<String> {
        if let Some(mismatch) = &self.mismatch {
            return Some(mismatch.clone());
        }
        if self.next < self.steps.len() {
            return Some(format!(
                "step {}: recorded `{}` was never requested (the engine stopped early)",
                self.next + 1,
                self.steps[self.next].request
            ));
        }
        None
    }
}

impl Reader for TranscriptReader<'_> {
    fn get<'a>(
        &'a mut self,
        oid: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        Box::pin(async move {
            let request = format!("get {}", arcs(oid));
            match self.take(&request).map(|step| &step.response) {
                Some(Response::Snmp(outcome)) => outcome.clone(),
                _ => Outcome::Absent,
            }
        })
    }

    fn walk<'a>(
        &'a mut self,
        root: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        Box::pin(async move {
            let request = format!("walk {}", arcs(root));
            match self.take(&request).map(|step| &step.response) {
                Some(Response::Snmp(outcome)) => outcome.clone(),
                _ => Outcome::Absent,
            }
        })
    }

    fn command<'a>(
        &'a mut self,
        name: CommandName,
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<CommandRow>>> + Send + 'a>> {
        Box::pin(async move {
            let request = format!("command {}", command_spelling(name));
            match self.take(&request).map(|step| &step.response) {
                Some(Response::Command(outcome)) => outcome.clone(),
                _ => Outcome::Absent,
            }
        })
    }
}

/// An OID's arcs as dotted text.
fn arcs(oid: &[u32]) -> String {
    oid.iter().map(u32::to_string).collect::<Vec<_>>().join(".")
}

/// The document spelling of a command.
fn command_spelling(name: CommandName) -> &'static str {
    match name {
        CommandName::SystemIdentity => "system_identity",
        CommandName::SystemRouterboard => "system_routerboard",
        CommandName::IpNeighbor => "ip_neighbor",
        CommandName::Interfaces => "interfaces",
        CommandName::IpAddress => "ip_address",
        CommandName::IpArp => "ip_arp",
        CommandName::BridgePort => "bridge_port",
        CommandName::BridgeVlan => "bridge_vlan",
        CommandName::BridgeHost => "bridge_host",
        CommandName::SystemResource => "system_resource",
        CommandName::SystemHealth => "system_health",
        CommandName::IpFirewallNat => "ip_firewall_nat",
        CommandName::IpDhcpServerLease => "ip_dhcp_server_lease",
    }
}

// --- the canonical observation -------------------------------------------

/// The observation as the expectation document: stable order, no secrets.
fn observation_json(observation: &Observation) -> Value {
    let mut value = json!({
        "supported": observation.supported,
        "probes": observation.probes.iter().map(|probe| json!({
            "probe": probe.probe,
            "outcome": outcome_name(probe.outcome),
            "reason": probe.reason,
            "rows": probe.rows,
        })).collect::<Vec<_>>(),
        "lldp": observation.lldp.iter().map(lldp_json).collect::<Vec<_>>(),
        "arp": observation.arp.iter().map(arp_json).collect::<Vec<_>>(),
        "fdb": observation.fdb.as_ref().map(fdb_json),
        "stations": observation.stations.iter().map(station_json).collect::<Vec<_>>(),
        "interfaces": observation.interfaces.iter().map(interface_json).collect::<Vec<_>>(),
        "claims": observation.claims.iter().map(|claim| json!({
            "probe": claim.probe,
            "row": claim.row,
            "kind": claim.claim.kind.to_string(),
            "value": claim.claim.value,
            "source": claim.claim.source.to_string(),
        })).collect::<Vec<_>>(),
        "details": observation.details.as_ref().map(details_json),
        "residuals": observation.residuals.iter().map(|residual| json!({
            "probe": residual.probe,
            "reason": residual_reason(residual.reason),
            "rows": residual.rows.iter().map(|row| {
                row.iter().map(|(field, value)| (field.clone(), Value::String(value.text()))).collect::<serde_json::Map<_, _>>()
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    if let Some(mbps) = observation.capacity.mbps {
        value
            .as_object_mut()
            .expect("observation is an object")
            .insert(
                "capacity".to_owned(),
                json!({
                    "mbps": mbps,
                    "source": observation.capacity.source.to_string(),
                }),
            );
    }
    value
}

/// The document spelling of a probe outcome.
fn outcome_name(outcome: OutcomeKind) -> &'static str {
    match outcome {
        OutcomeKind::Present => "present",
        OutcomeKind::Empty => "empty",
        OutcomeKind::Absent => "absent",
        OutcomeKind::Timeout => "timeout",
        OutcomeKind::Error => "error",
        OutcomeKind::Cancelled => "cancelled",
    }
}

/// The document spelling of a residual reason.
fn residual_reason(reason: engine::ResidualReason) -> &'static str {
    match reason {
        engine::ResidualReason::NoCoreSource => "no_core_source",
    }
}

/// One neighbour.
fn lldp_json(neighbour: &LldpNeighbour) -> Value {
    json!({
        "chassis_id": chassis(&neighbour.chassis_id),
        "address": neighbour.address,
        "port_id": neighbour.remote_port_id,
        "sys_name": neighbour.sys_name,
        "sys_descr": neighbour.sys_descr,
        "local_port": neighbour.local_port,
    })
}

/// One ARP row.
fn arp_json(row: &ArpRow) -> Value {
    json!({
        "if_index": row.if_index,
        "mac": hex(&row.mac),
        "ip": row.ip,
        "physical": row.physical,
    })
}

/// One station row.
fn station_json(station: &AirStation) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("mac".to_owned(), json!(hex(&station.mac)));
    map.insert("wlan_if".to_owned(), json!(station.wlan_if));
    map.insert("hostname".to_owned(), json!(station.hostname));
    map.insert("signal_dbm".to_owned(), json!(station.signal_dbm));
    map.insert("noise_dbm".to_owned(), json!(station.noise_dbm));
    map.insert("last_ip".to_owned(), json!(station.last_ip));
    map.insert("tx_bps".to_owned(), json!(station.tx_bps));
    map.insert("rx_bps".to_owned(), json!(station.rx_bps));
    map.insert("rx_bytes".to_owned(), json!(station.rx_bytes));
    map.insert("tx_bytes".to_owned(), json!(station.tx_bytes));
    map.insert("uptime".to_owned(), json!(station.uptime));
    map.insert("distance_ccq".to_owned(), json!(station.distance_ccq));
    map.insert("quality".to_owned(), json!(station.quality));
    put_optional(&mut map, "channel", station.channel);
    put_optional(&mut map, "ul_rssi", station.ul_rssi);
    put_optional(&mut map, "dl_rssi", station.dl_rssi);
    put_optional(&mut map, "ul_snr", station.ul_snr);
    put_optional(&mut map, "dl_snr", station.dl_snr);
    put_optional(&mut map, "tx_capacity", station.tx_capacity);
    put_optional(&mut map, "rx_capacity", station.rx_capacity);
    put_optional(&mut map, "tx_quality", station.tx_quality);
    put_optional(&mut map, "dl_rate_mbps", station.dl_rate_mbps);
    put_optional(&mut map, "model", station.model.clone());
    put_optional(
        &mut map,
        "software_version",
        station.software_version.clone(),
    );
    Value::Object(map)
}

/// Inserts an optional value only when present, so an expectation recorded
/// before the field existed still matches.
fn put_optional<T: Into<Value>>(
    map: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<T>,
) {
    if let Some(value) = value {
        map.insert(key.to_owned(), value.into());
    }
}

/// One model station (`details.stations`), with the extended fields
/// serialized only when present.
fn wireless_station_json(station: &dudarchy_poller::WirelessStation) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("mac".to_owned(), json!(station.mac.as_str()));
    map.insert("wlan_if".to_owned(), json!(station.wlan_if));
    map.insert("hostname".to_owned(), json!(station.hostname));
    map.insert("signal_dbm".to_owned(), json!(station.signal_dbm));
    map.insert("noise_dbm".to_owned(), json!(station.noise_dbm));
    map.insert("last_ip".to_owned(), json!(station.last_ip));
    map.insert("tx_bps".to_owned(), json!(station.tx_bps));
    map.insert("rx_bps".to_owned(), json!(station.rx_bps));
    map.insert("rx_bytes".to_owned(), json!(station.rx_bytes));
    map.insert("tx_bytes".to_owned(), json!(station.tx_bytes));
    map.insert("uptime".to_owned(), json!(station.uptime));
    map.insert("distance_ccq".to_owned(), json!(station.distance_ccq));
    map.insert("quality".to_owned(), json!(station.quality));
    put_optional(&mut map, "channel", station.channel);
    put_optional(&mut map, "ul_rssi", station.ul_rssi);
    put_optional(&mut map, "dl_rssi", station.dl_rssi);
    put_optional(&mut map, "ul_snr", station.ul_snr);
    put_optional(&mut map, "dl_snr", station.dl_snr);
    put_optional(&mut map, "tx_capacity", station.tx_capacity);
    put_optional(&mut map, "rx_capacity", station.rx_capacity);
    put_optional(&mut map, "tx_quality", station.tx_quality);
    put_optional(&mut map, "dl_rate_mbps", station.dl_rate_mbps);
    put_optional(&mut map, "model", station.model.clone());
    put_optional(
        &mut map,
        "software_version",
        station.software_version.clone(),
    );
    map.insert(
        "provenance".to_owned(),
        json!(
            station
                .provenance
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        ),
    );
    Value::Object(map)
}

/// One SNMP interface row.
fn interface_json(row: &dudarchy_poller::SnmpInterface) -> Value {
    json!({
        "index": row.index,
        "name": row.name,
        "descr": row.descr,
        "alias": row.alias,
        "raw_type": row.raw_type,
        "phys_address": row.phys_address.as_deref().map(hex),
        "admin_status": row.admin_status,
        "oper_status": row.oper_status,
    })
}

/// The FDB observation.
fn fdb_json(fdb: &SnmpFdb) -> Value {
    json!({
        "qbridge": fdb.qbridge.as_ref().map(|rows| rows.iter().map(|row| json!({
            "vlan": row.vlan,
            "mac": hex(&row.mac),
            "port": row.port,
            "status": row.status,
        })).collect::<Vec<_>>()),
        "bridge": fdb.bridge.as_ref().map(|rows| rows.iter().map(|row| json!({
            "mac": hex(&row.mac),
            "port": row.port,
            "status": row.status,
        })).collect::<Vec<_>>()),
        "port_if_index": fdb.port_if_index.iter().map(|(port, index)| json!([port, index])).collect::<Vec<_>>(),
    })
}

/// The device details.
fn details_json(details: &dudarchy_poller::DeviceDetails) -> Value {
    let mut value = json!({
        "interfaces": details.interfaces.iter().map(|row| json!({
            "id": row.id,
            "name": row.name,
            "description": row.description,
            "alias": row.alias,
            "kind": row.kind.to_string(),
            "raw_type": row.raw_type,
            "mac": row.mac.as_ref().map(Mac::as_str),
            "addresses": row.addresses,
            "admin_state": row.admin_state.map(|state| state.to_string()),
            "oper_state": row.oper_state.map(|state| state.to_string()),
            "vlan_id": row.vlan_id,
            "parent_interface": row.parent_interface,
            "provenance": row.provenance.iter().map(ToString::to_string).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "system": details.system.as_ref().map(|system| json!({
            "version": system.version,
            "board_name": system.board_name,
            "platform": system.platform,
            "uptime": system.uptime,
            "provenance": system.provenance.iter().map(ToString::to_string).collect::<Vec<_>>(),
        })),
        "temperatures": details.temperatures.iter().map(|reading| json!({
            "sensor_name": reading.sensor_name,
            "celsius": reading.celsius,
            "provenance": reading.provenance.iter().map(ToString::to_string).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "stations": details.stations.iter().map(wireless_station_json).collect::<Vec<_>>(),
        "arp": details.arp.as_ref().map(|snapshot| json!({
            "provenance": snapshot.provenance.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "rows": snapshot.rows.iter().map(|row| json!({
                "address": row.address,
                "mac_address": row.mac_address.as_ref().map(Mac::as_str),
                "interface": row.interface,
                "dynamic": row.dynamic,
                "complete": row.complete,
                "disabled": row.disabled,
            })).collect::<Vec<_>>(),
        })),
        "bridge_ports": details.bridge_ports.as_ref().map(|snapshot| json!({
            "provenance": snapshot.provenance.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "rows": snapshot.rows.iter().map(|row| json!({
                "bridge": row.bridge,
                "interface": row.interface,
                "pvid": row.pvid,
                "frame_types": row.frame_types,
                "status": row.status,
                "inactive": row.inactive,
                "disabled": row.disabled,
                "dynamic": row.dynamic,
            })).collect::<Vec<_>>(),
        })),
        "bridge_vlans": details.bridge_vlans.as_ref().map(|snapshot| json!({
            "provenance": snapshot.provenance.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "rows": snapshot.rows.iter().map(|row| json!({
                "bridge": row.bridge,
                "vlan_ids": row.vlan_ids,
                "tagged": row.tagged,
                "untagged": row.untagged,
                "current_tagged": row.current_tagged,
                "current_untagged": row.current_untagged,
            })).collect::<Vec<_>>(),
        })),
        "bridge_hosts": details.bridge_hosts.as_ref().map(|snapshot| json!({
            "provenance": snapshot.provenance.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "rows": snapshot.rows.iter().map(|row| json!({
                "bridge": row.bridge,
                "on_interface": row.on_interface,
                "mac_address": row.mac_address.as_ref().map(Mac::as_str),
                "age": row.age,
                "dynamic": row.dynamic,
                "local": row.local,
                "external": row.external,
            })).collect::<Vec<_>>(),
        })),
        "nat": details.nat.as_ref().map(|snapshot| json!({
            "provenance": snapshot.provenance.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "rows": snapshot.rows.iter().map(|row| json!({
                "chain": row.chain,
                "action": row.action,
                "in_interface": row.in_interface,
                "out_interface": row.out_interface,
                "src_address": row.src_address,
                "dst_address": row.dst_address,
                "to_addresses": row.to_addresses,
                "disabled": row.disabled,
                "comment": row.comment,
            })).collect::<Vec<_>>(),
        })),
        "dhcp_leases": details.dhcp_leases.as_ref().map(|snapshot| json!({
            "provenance": snapshot.provenance.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "rows": snapshot.rows.iter().map(|row| json!({
                "address": row.address,
                "mac": row.mac.as_ref().map(Mac::as_str),
                "hostname": row.hostname,
                "status": row.status,
                "dynamic": row.dynamic,
                "disabled": row.disabled,
            })).collect::<Vec<_>>(),
        })),
    });
    if let Some(radio) = &details.radio {
        value.as_object_mut().expect("details are an object").insert(
            "radio".to_owned(),
            json!({
                "model": radio.model,
                "frequency_mhz": radio.frequency_mhz,
                "channel_width_mhz": radio.channel_width_mhz,
                "sm_count": radio.sm_count,
                "tx_power_dbm": radio.tx_power_dbm,
                "lan_speed_mbps": radio.lan_speed_mbps,
                "phy_mbps": radio.phy_mbps,
                "provenance": radio.provenance.iter().map(ToString::to_string).collect::<Vec<_>>(),
            }),
        );
    }
    value
}

/// An octet string as display text: six bytes are a MAC, anything else is
/// UTF-8 text.
fn chassis(bytes: &[u8]) -> String {
    if bytes.len() == 6 {
        hex(bytes)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Octets as colon-separated hex.
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

// --- comparison -----------------------------------------------------------

/// The first divergent value between an expectation and an observation.
fn divergence(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let mut keys: Vec<&String> = expected.keys().chain(actual.keys()).collect();
            keys.sort_unstable();
            keys.dedup();
            for key in keys {
                let child = format!("{path}.{key}");
                match (expected.get(key), actual.get(key)) {
                    (Some(expected), Some(actual)) => {
                        if let Some(found) = divergence(expected, actual, &child) {
                            return Some(found);
                        }
                    }
                    (Some(expected), None) => {
                        return Some(format!("{child}: expected {expected}, actual <missing>"));
                    }
                    (None, Some(actual)) => {
                        return Some(format!("{child}: expected <missing>, actual {actual}"));
                    }
                    (None, None) => {}
                }
            }
            None
        }
        (Value::Array(expected), Value::Array(actual)) => {
            for (index, (expected, actual)) in expected.iter().zip(actual.iter()).enumerate() {
                if let Some(found) = divergence(expected, actual, &format!("{path}[{index}]")) {
                    return Some(found);
                }
            }
            (expected.len() != actual.len()).then(|| {
                format!(
                    "{path}: expected {} entries, actual {}",
                    expected.len(),
                    actual.len()
                )
            })
        }
        (expected, actual) if expected == actual => None,
        (expected, actual) => Some(format!("{path}: expected {expected}, actual {actual}")),
    }
}

// --- the harness ----------------------------------------------------------

/// Loads and validates the manifest, naming the path on failure.
fn load_manifest(root: &Path) -> Manifest {
    let path = root.join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing replay manifest {}: {error}", path.display()));
    let manifest: Manifest = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("invalid replay manifest {}: {error}", path.display()));
    assert_eq!(manifest.schema_version, 1, "manifest schema_version");
    manifest
}

/// Runs one case, returning a failure report naming the case, document, and
/// the first divergence.
async fn run_case(root: &Path, case: &Case) -> Result<(), String> {
    let document_path = root.join(&case.document);
    let document_text = std::fs::read_to_string(&document_path)
        .map_err(|error| format!("missing document {}: {error}", document_path.display()))?;
    let document: Document = adapters::parse(&case.document, &document_text).map_err(|errors| {
        format!(
            "invalid document {}: {}",
            case.document,
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;

    let transcript_path = root.join(&case.transcript);
    let transcript_text = std::fs::read_to_string(&transcript_path)
        .map_err(|error| format!("missing transcript {}: {error}", transcript_path.display()))?;
    let transcript: Transcript = serde_json::from_str(&transcript_text)
        .map_err(|error| format!("invalid transcript {}: {error}", transcript_path.display()))?;
    let steps = prepare(transcript, &case.document)?;

    let expect_path = root.join(&case.expect);
    let expect_text = std::fs::read_to_string(&expect_path)
        .map_err(|error| format!("missing expectation {}: {error}", expect_path.display()))?;
    let expected: Value = serde_json::from_str(&expect_text)
        .map_err(|error| format!("invalid expectation {}: {error}", expect_path.display()))?;

    let mut reader = TranscriptReader::new(&steps);
    let observation = match &case.surface {
        Some(surface) => {
            engine::probe_surface(
                &document,
                surface,
                &mut reader,
                SystemTime::UNIX_EPOCH,
                &|| false,
            )
            .await
        }
        None => engine::probe(&document, &mut reader, SystemTime::UNIX_EPOCH, &|| false).await,
    };
    if let Some(fault) = reader.fault() {
        return Err(format!("{}: {fault}", case.document));
    }

    let actual = observation_json(&observation);
    match divergence(&expected, &actual, "$") {
        None => Ok(()),
        Some(found) => {
            let hint = probe_hint(&actual, &found);
            let probes = observation
                .probes
                .iter()
                .map(|probe| format!("{}={}", probe.probe, outcome_name(probe.outcome)))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "{}: first divergence at {found}{hint} (probes: {probes})",
                case.document
            ))
        }
    }
}

/// The probe a divergent path belongs to, when the entry names one.
fn probe_hint(actual: &Value, divergence: &str) -> String {
    let path = divergence.split(':').next().unwrap_or_default();
    let path = path.strip_prefix("$.").unwrap_or(path);
    let section = path.split('[').next().unwrap_or_default();
    let Some(index) = path
        .split_once('[')
        .and_then(|(_, rest)| rest.split(']').next())
        .and_then(|index| index.parse::<usize>().ok())
    else {
        return String::new();
    };
    let probe = actual
        .get(section)
        .and_then(|value| value.get(index))
        .and_then(|entry| entry.get("probe"))
        .and_then(Value::as_str);
    probe
        .map(|probe| format!(" (probe `{probe}`)"))
        .unwrap_or_default()
}

// --- the harness's own failure paths --------------------------------------
//
// The harness is the gate future adapter units rely on, so its diagnostics
// are tested directly: a mismatched, exhausted, or unfinished transcript, an
// impossible reader shape, a divergent key, and the command vocabulary's two
// directions.

/// The OID and root the self-tests use.
const GET_OID: &str = "1.3.6.1.2.1.1.1.0";
const WALK_ROOT: &str = "1.3.6.1.2.1.2.2.1";

/// A transcript step for the self-tests: only the keys a case sets.
fn recorded(op: &str, outcome: &str) -> Step {
    Step {
        op: op.to_owned(),
        oid: None,
        root: None,
        command: None,
        outcome: outcome.to_owned(),
        reason: None,
        varbinds: Vec::new(),
        rows: Vec::new(),
    }
}

/// A transcript of the given steps.
fn transcript(steps: Vec<Step>) -> Transcript {
    Transcript {
        schema_version: 1,
        steps,
    }
}

/// One recorded varbind with no payload: enough to make a present get legal.
fn recorded_varbind() -> VarbindSpec {
    VarbindSpec {
        oid: GET_OID.to_owned(),
        value: ValueSpec {
            kind: "empty".to_owned(),
            text: None,
            hex: None,
            value: None,
        },
    }
}

/// A prepared step that answers nothing, for the reader tests.
fn prepared(request: &str) -> PreparedStep {
    PreparedStep {
        request: request.to_owned(),
        response: Response::Snmp(Outcome::Absent),
    }
}

#[test]
fn a_request_out_of_order_names_the_step_and_both_requests() {
    let steps = vec![
        prepared("get 1.3.6.1.2.1.1.1.0"),
        prepared("walk 1.3.6.1.2.1.2.2.1"),
    ];
    let mut reader = TranscriptReader::new(&steps);

    assert!(reader.take("walk 1.3.6.1.2.1.2.2.1").is_none());
    let fault = reader.fault().expect("a mismatched request is a fault");
    assert!(fault.contains("step 1"), "{fault}");
    assert!(
        fault.contains("recorded `get 1.3.6.1.2.1.1.1.0`"),
        "{fault}"
    );
    assert!(
        fault.contains("engine requested `walk 1.3.6.1.2.1.2.2.1`"),
        "{fault}"
    );
}

#[test]
fn a_request_after_the_last_step_names_the_exhausted_transcript() {
    let steps = vec![prepared("get 1.3.6.1.2.1.1.1.0")];
    let mut reader = TranscriptReader::new(&steps);

    assert!(reader.take("get 1.3.6.1.2.1.1.1.0").is_some());
    assert!(reader.take("get 1.3.6.1.2.1.1.5.0").is_none());
    let fault = reader.fault().expect("a request past the end is a fault");
    assert!(
        fault.contains("after the transcript's last step (1)"),
        "{fault}"
    );
    assert!(fault.contains("get 1.3.6.1.2.1.1.5.0"), "{fault}");
}

#[test]
fn a_leftover_step_is_a_fault_when_the_run_finishes() {
    let steps = vec![
        prepared("get 1.3.6.1.2.1.1.1.0"),
        prepared("get 1.3.6.1.2.1.1.5.0"),
    ];
    let mut reader = TranscriptReader::new(&steps);

    assert!(reader.take("get 1.3.6.1.2.1.1.1.0").is_some());
    let fault = reader.fault().expect("an unrequested step is a fault");
    assert!(fault.contains("step 2"), "{fault}");
    assert!(fault.contains("was never requested"), "{fault}");
    assert!(fault.contains("get 1.3.6.1.2.1.1.5.0"), "{fault}");
}

#[test]
fn prepare_rejects_an_unknown_op_outcome_and_value_type() {
    let mut step = recorded("poll", "present");
    step.oid = Some(GET_OID.to_owned());
    let error = prepare(transcript(vec![step]), "doc.json")
        .err()
        .expect("an unknown op is rejected");
    assert!(error.contains("transcript step 1"), "{error}");
    assert!(error.contains("unknown op `poll`"), "{error}");

    let mut step = recorded("get", "probably");
    step.oid = Some(GET_OID.to_owned());
    let error = prepare(transcript(vec![step]), "doc.json")
        .err()
        .expect("an unknown outcome is rejected");
    assert!(error.contains("unknown outcome `probably`"), "{error}");

    let mut spec = recorded_varbind();
    spec.value.kind = "bcd".to_owned();
    let mut step = recorded("get", "present");
    step.oid = Some(GET_OID.to_owned());
    step.varbinds = vec![spec];
    let error = prepare(transcript(vec![step]), "doc.json")
        .err()
        .expect("an unknown value type is rejected");
    assert!(error.contains("unknown value type `bcd`"), "{error}");
}

#[test]
fn prepare_rejects_impossible_get_and_walk_shapes() {
    let reason = |step: Step| {
        prepare(transcript(vec![step]), "doc.json")
            .err()
            .expect("the shape is impossible")
    };

    let mut empty_get = recorded("get", "empty");
    empty_get.oid = Some(GET_OID.to_owned());
    let error = reason(empty_get);
    assert!(error.contains("transcript step 1"), "{error}");
    assert!(error.contains("a `get` cannot answer `empty`"), "{error}");

    let mut bare_get = recorded("get", "present");
    bare_get.oid = Some(GET_OID.to_owned());
    let error = reason(bare_get);
    assert!(error.contains("must carry exactly one varbind"), "{error}");

    let mut crowded_get = recorded("get", "present");
    crowded_get.oid = Some(GET_OID.to_owned());
    crowded_get.varbinds = vec![recorded_varbind(), recorded_varbind()];
    let error = reason(crowded_get);
    assert!(error.contains("must carry exactly one varbind"), "{error}");

    let mut rooted_get = recorded("get", "present");
    rooted_get.oid = Some(GET_OID.to_owned());
    rooted_get.root = Some(WALK_ROOT.to_owned());
    let error = reason(rooted_get);
    assert!(
        error.contains("`root` is not valid on a get step"),
        "{error}"
    );

    let mut oid_walk = recorded("walk", "present");
    oid_walk.oid = Some(GET_OID.to_owned());
    oid_walk.root = Some(WALK_ROOT.to_owned());
    let error = reason(oid_walk);
    assert!(
        error.contains("`oid` is not valid on a walk step"),
        "{error}"
    );

    // The legal neighbours of those shapes still prepare: a one-varbind get
    // and a present walk that answered an empty table.
    let mut get = recorded("get", "present");
    get.oid = Some(GET_OID.to_owned());
    get.varbinds = vec![recorded_varbind()];
    let mut walk = recorded("walk", "present");
    walk.root = Some(WALK_ROOT.to_owned());
    let steps = prepare(transcript(vec![get, walk]), "doc.json")
        .expect("a one-varbind get and an empty present walk are legal");
    assert_eq!(steps.len(), 2);
}

#[test]
fn divergence_names_a_missing_and_an_extra_key() {
    let expected = json!({ "supported": true, "probes": [] });
    let extra = json!({ "supported": true, "probes": [], "residuals": [] });

    assert_eq!(
        divergence(&expected, &extra, "$").as_deref(),
        Some("$.residuals: expected <missing>, actual []")
    );
    assert_eq!(
        divergence(&extra, &expected, "$").as_deref(),
        Some("$.residuals: expected [], actual <missing>")
    );
}

#[test]
fn every_command_name_round_trips_through_its_document_spelling() {
    let names = [
        CommandName::SystemIdentity,
        CommandName::SystemRouterboard,
        CommandName::IpNeighbor,
        CommandName::Interfaces,
        CommandName::IpAddress,
        CommandName::IpArp,
        CommandName::BridgePort,
        CommandName::BridgeVlan,
        CommandName::BridgeHost,
        CommandName::SystemResource,
        CommandName::SystemHealth,
        CommandName::IpFirewallNat,
        CommandName::IpDhcpServerLease,
    ];
    let mut spellings = Vec::new();
    for name in names {
        let spelling = command_spelling(name);
        assert_eq!(
            command_name(spelling),
            Some(name),
            "`{spelling}` must map back to its variant"
        );
        spellings.push(spelling);
    }
    spellings.sort_unstable();
    let count = spellings.len();
    spellings.dedup();
    assert_eq!(spellings.len(), count, "the spellings are distinct");
    assert_eq!(command_name("format_disk"), None);
}

#[tokio::test]
async fn every_replay_case_matches_its_expectation() {
    let root = replay_root();
    let manifest = load_manifest(&root);
    let filter = std::env::var("ADAPTER_REPLAY_CASE")
        .ok()
        .filter(|name| !name.is_empty());
    let cases: Vec<&Case> = manifest
        .cases
        .iter()
        .filter(|case| filter.as_ref().is_none_or(|name| case.name == *name))
        .collect();
    if let Some(name) = &filter
        && cases.is_empty()
    {
        panic!(
            "unknown replay case `{name}`; the manifest has: {}",
            manifest
                .cases
                .iter()
                .map(|case| case.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let mut failures = Vec::new();
    for case in cases {
        if let Err(failure) = run_case(&root, case).await {
            failures.push(format!("case `{}`: {failure}", case.name));
        }
    }
    assert!(
        failures.is_empty(),
        "{} replay case(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

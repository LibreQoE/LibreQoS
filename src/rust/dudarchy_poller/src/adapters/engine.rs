//! The document engine: selection, execution, decode, and mapping.
//!
//! One engine runs every adapter document. It owns source selection (the
//! declared probe, table, or command), field mapping, budgets, cancellation,
//! and deterministic ordering. It has no vendor branch: every decision comes
//! from the document's closed vocabulary, and every transport is behind the
//! [`Reader`] seam.
//!
//! Reads that map onto an existing model destination become [`Claim`]s and
//! [`DeviceDetails`]; reads whose decoded shape has no core provenance
//! become [`ResidualObservation`]s carrying the adapter, probe, and decoded
//! fields for the unit that converts that family.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime};

use crate::commands::detail_slot;
use crate::identity::{
    ArpEntry, BridgeHost, BridgePort, BridgeVlan, CapacityDerivation, CapacitySource, Claim,
    DeviceDetails, DhcpLease, Interface, Mac, NatRule, RadioDetails, Source, SystemDetails,
    TableSnapshot, TemperatureReading, WirelessStation,
};
use crate::mikrotik::{
    ApiNeighbour, RouterOsAddress, RouterOsArp, RouterOsBridgeHost, RouterOsBridgePort,
    RouterOsBridgeVlan, RouterOsInterface, RouterOsNatRule,
};
use crate::snmp::{ArpRow, IpAddressRow, LldpNeighbour, SnmpFdb, SnmpInterface};
use crate::snmp_tables::{
    AirStation, DOT1D_BASE_PORT_IF_INDEX, LldpColumns, SnmpBridgeRow, SnmpQBridgeRow, WalkValue,
    arp_typed_row, arp_v4_row, assemble_neighbours, base_port_if_index_rows, channel_width_mhz,
    code_value, display_mac_value, ip_address_v4_row, loc_port_rows, mac_octets, mac_value,
    rate_suffix, vlan_and_mac,
};

use super::reader::{CommandRow, Outcome, OutcomeKind, Reader, Varbind};
use super::{
    Budget, CommandName, Details, Document, IndexKind, MAX_BYTES_CEILING, MAX_DEADLINE_MS_CEILING,
    MAX_REQUESTS_CEILING, MAX_ROWS_CEILING, Parser, Probe, Read, Scalar, Surface, Table, Transport,
};

/// The default request budget when a document sets none.
pub const DEFAULT_MAX_REQUESTS: u32 = 1024;
/// The default row budget when a document sets none.
pub const DEFAULT_MAX_ROWS: u32 = 4096;
/// The default byte budget when a document sets none.
pub const DEFAULT_MAX_BYTES: u64 = 1 << 20;
/// The default deadline when a document sets none.
pub const DEFAULT_DEADLINE_MS: u64 = 30_000;

/// The result of matching detection hints against one device.
#[derive(Debug)]
pub enum Selection<'a> {
    /// Exactly one document matched, or no document matched and the
    /// registry's single default was selected instead.
    Selected(&'a Document),
    /// No document matched and the registry has no default.
    Unsupported,
    /// More than one document matched, or no document matched and more than
    /// one default exists; nothing is executed.
    Ambiguous {
        /// The matching or default adapter ids, in document order.
        adapters: Vec<&'a str>,
    },
}

/// Selects a document by literal detection hints, falling back to the
/// registry's default document when no hint matches. An exact
/// `sysObjectID` match is evaluated first and beats every prefix or
/// sysDescr hint; a prefix/sysDescr-specific match then wins over the
/// default. Pin-only documents are never auto-selected. Zero matches and
/// no default is `Unsupported`; several matches at one level, or several
/// defaults, is `Ambiguous`; the engine never picks a closest match.
pub fn select<'a>(
    documents: &'a [Document],
    sys_object_id: Option<&str>,
    sys_descr: Option<&str>,
) -> Selection<'a> {
    let object_id = sys_object_id.and_then(oid_arcs);
    // An exact sysObjectID is not ambiguous with a broader prefix: a
    // family that reports one fixed OID owns that OID outright. Two
    // documents claiming the same exact OID are still refused.
    if let Some(id) = object_id.as_ref() {
        let mut exact: Vec<&Document> = documents
            .iter()
            .filter(|doc| {
                !doc.detect.pin_only
                    && doc.detect.sys_object_id_exact.iter().any(|exact| {
                        oid_arcs(exact).is_some_and(|exact| exact.as_slice() == id.as_slice())
                    })
            })
            .collect();
        match exact.len() {
            0 => {}
            1 => return Selection::Selected(exact.remove(0)),
            _ => {
                return Selection::Ambiguous {
                    adapters: exact.iter().map(|doc| doc.id.as_str()).collect(),
                };
            }
        }
    }
    let mut matches: Vec<&Document> = documents
        .iter()
        .filter(|doc| {
            if doc.detect.pin_only {
                return false;
            }
            let by_object_id = doc.detect.sys_object_id_prefixes.iter().any(|prefix| {
                let prefix = oid_arcs(prefix);
                match (&object_id, &prefix) {
                    (Some(id), Some(prefix)) => id.starts_with(prefix),
                    _ => false,
                }
            });
            let by_descr = sys_descr.is_some_and(|descr| {
                doc.detect
                    .sys_descr_contains
                    .iter()
                    .any(|needle| descr.contains(needle))
            });
            by_object_id || by_descr
        })
        .collect();
    match matches.len() {
        0 => {
            let mut defaults: Vec<&Document> = documents
                .iter()
                .filter(|doc| doc.detect.default && !doc.detect.pin_only)
                .collect();
            match defaults.len() {
                0 => Selection::Unsupported,
                1 => Selection::Selected(defaults.remove(0)),
                _ => Selection::Ambiguous {
                    adapters: defaults.iter().map(|doc| doc.id.as_str()).collect(),
                },
            }
        }
        1 => Selection::Selected(matches.remove(0)),
        _ => Selection::Ambiguous {
            adapters: matches.iter().map(|doc| doc.id.as_str()).collect(),
        },
    }
}

/// The result of choosing which document's API surface to run for a node.
#[derive(Debug)]
pub enum ApiSelection<'a> {
    /// Exactly one document's API surface runs.
    Selected(&'a Document),
    /// No document offers an API surface, or the SNMP-selected document
    /// declares none: no API work.
    Unsupported,
    /// More than one document offers an API surface and no SNMP selection
    /// narrowed it; nothing is executed.
    Ambiguous {
        /// The candidate adapter ids, in document order.
        adapters: Vec<&'a str>,
    },
}

/// The document's RouterOS API surface, when it declares one.
pub fn api_surface(document: &Document) -> Option<&Surface> {
    document
        .surfaces
        .iter()
        .find(|surface| surface.transport == Transport::RouterOsApi)
}

/// Chooses the document whose API surface runs for this node.
///
/// A document the SNMP system read selected on a detection hint wins when
/// it declares an API surface; a family document without one means no API
/// work. The registry's default document is not a family claim — it matched
/// no hint — so a default selection falls through to the API-only case:
/// the registry's documents that declare an API surface are the candidates.
/// Exactly one is selected, none means no API work, and several is
/// `Ambiguous` so nothing runs.
pub fn select_api<'a>(documents: &'a [Document], snmp: Option<&'a Document>) -> ApiSelection<'a> {
    if let Some(document) = snmp
        && !document.detect.default
    {
        return match api_surface(document) {
            Some(_) => ApiSelection::Selected(document),
            None => ApiSelection::Unsupported,
        };
    }
    let mut candidates: Vec<&Document> = documents
        .iter()
        .filter(|document| api_surface(document).is_some())
        .collect();
    match candidates.len() {
        0 => ApiSelection::Unsupported,
        1 => ApiSelection::Selected(candidates.remove(0)),
        _ => ApiSelection::Ambiguous {
            adapters: candidates
                .iter()
                .map(|document| document.id.as_str())
                .collect(),
        },
    }
}

/// The resolved limits for one run: the document's value when it set one,
/// otherwise the core default, never above the core ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Total reads across the document's probes.
    pub max_requests: u32,
    /// Total decoded rows across the document's tables.
    pub max_rows: u32,
    /// Total response bytes across the document's reads.
    pub max_bytes: u64,
    /// Wall-clock deadline for the run.
    pub deadline: Duration,
}

impl Limits {
    /// Resolves a document budget against the core defaults and ceilings.
    pub fn resolve(budget: &Budget) -> Limits {
        Limits {
            max_requests: budget
                .max_requests
                .unwrap_or(DEFAULT_MAX_REQUESTS)
                .min(MAX_REQUESTS_CEILING),
            max_rows: budget
                .max_rows
                .unwrap_or(DEFAULT_MAX_ROWS)
                .min(MAX_ROWS_CEILING),
            max_bytes: budget
                .max_bytes
                .unwrap_or(DEFAULT_MAX_BYTES)
                .min(MAX_BYTES_CEILING),
            deadline: Duration::from_millis(
                budget
                    .deadline_ms
                    .unwrap_or(DEFAULT_DEADLINE_MS)
                    .min(MAX_DEADLINE_MS_CEILING),
            ),
        }
    }
}

/// The capacity trust factor the `[ap_polling]` section defaults to: 1.0,
/// meaning a reported capacity is used as-is. Operators lower it per model
/// when a family's reported rates overstate usable throughput.
pub const DEFAULT_CAPACITY_FACTOR: f64 = 1.0;

/// The capacity inputs the application layer resolves from `[ap_polling]`
/// and the CRM. The document never states any of them.
#[derive(Clone, Debug, PartialEq)]
pub struct CapacityPolicy {
    /// `[ap_polling].capacity_factor_default` (1.0 by default): the trust
    /// factor for a model with no explicit entry, including an unknown or
    /// absent model.
    pub default_factor: f64,
    /// `[ap_polling.capacity_factor]`, keyed by the model string the device
    /// reports; an absent key falls back to `default_factor` (1.0).
    pub by_model: BTreeMap<String, f64>,
    /// The device's model from the CRM, used when the polled observation
    /// reports none; selects the per-model factor.
    pub model: Option<String>,
    /// The CRM's rate for this device, when it supplied one.
    pub crm_rate_mbps: Option<f64>,
}

impl Default for CapacityPolicy {
    fn default() -> Self {
        Self {
            default_factor: DEFAULT_CAPACITY_FACTOR,
            by_model: BTreeMap::new(),
            model: None,
            crm_rate_mbps: None,
        }
    }
}

impl CapacityPolicy {
    /// The trust factor for this device: the per-model entry when one
    /// exists, else the default (1.0 when unconfigured).
    pub fn factor_for(&self, model: Option<&str>) -> f64 {
        model
            .and_then(|model| self.by_model.get(model))
            .copied()
            .unwrap_or(self.default_factor)
    }
}

/// Derives a device capacity, never a document value. A reported PHY rate
/// wins; otherwise the highest per-station downlink rate wins (an explicit
/// `dl_rate_mbps`, or airOS's AP TX bps converted to Mbps); otherwise the
/// CRM's rate stands as-is; otherwise the capacity is unknown. The winning
/// PHY or per-station value is multiplied by the same per-model trust factor
/// (default 1.0, used verbatim). A non-finite or non-positive factor or rate
/// contributes nothing and the next input is tried.
pub fn derive_capacity(
    radio: Option<&RadioDetails>,
    stations: &[WirelessStation],
    policy: &CapacityPolicy,
) -> CapacityDerivation {
    let model = radio
        .and_then(|radio| radio.model.as_deref())
        .or(policy.model.as_deref());
    let factor = policy.factor_for(model);
    let usable_factor = factor.is_finite() && factor > 0.0;
    if usable_factor
        && let Some(phy) = radio
            .and_then(|radio| radio.phy_mbps)
            .filter(|value| value.is_finite() && *value > 0.0)
    {
        return CapacityDerivation {
            mbps: Some(phy * factor),
            source: CapacitySource::Phy,
        };
    }
    let max_downlink = stations
        .iter()
        .filter_map(|station| {
            station
                .dl_rate_mbps
                .or_else(|| station.tx_bps.map(|bps| bps / 1_000_000))
        })
        .filter(|rate| *rate > 0)
        .max();
    if usable_factor && let Some(rate) = max_downlink {
        return CapacityDerivation {
            mbps: Some(rate as f64 * factor),
            source: CapacitySource::PerSmRate,
        };
    }
    if let Some(rate) = policy
        .crm_rate_mbps
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        return CapacityDerivation {
            mbps: Some(rate),
            source: CapacitySource::Crm,
        };
    }
    CapacityDerivation {
        mbps: None,
        source: CapacitySource::Unknown,
    }
}

/// One decoded field value, in the type its parser produced.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    /// Text (`utf8`).
    Text(String),
    /// An unsigned integer (`u32`).
    U32(u32),
    /// A signed integer (`i64`).
    I64(i64),
    /// A 64-bit unsigned integer (`u64`).
    U64(u64),
    /// A dotted OID (`oid`).
    Oid(String),
    /// A six-octet MAC (`octet_mac`).
    Mac([u8; 6]),
    /// An address as text (`ip_address`).
    Ip(String),
}

impl FieldValue {
    /// The value as text, for claims and residual observations.
    pub fn text(&self) -> String {
        match self {
            Self::Text(text) | Self::Oid(text) | Self::Ip(text) => text.clone(),
            Self::U32(value) => value.to_string(),
            Self::I64(value) => value.to_string(),
            Self::U64(value) => value.to_string(),
            Self::Mac(bytes) => Mac::from_chassis(bytes)
                .expect("six bytes are a MAC")
                .as_str()
                .to_owned(),
        }
    }

    /// The value as an unsigned integer, when it carries one.
    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Self::U32(value) => Some(*value),
            Self::U64(value) => u32::try_from(*value).ok(),
            Self::I64(value) => u32::try_from(*value).ok(),
            Self::Text(text) => text.parse().ok(),
            _ => None,
        }
    }

    /// The value as a signed integer, when it carries one.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I64(value) => Some(*value),
            Self::U32(value) => Some(i64::from(*value)),
            Self::U64(value) => i64::try_from(*value).ok(),
            Self::Text(text) => text.parse().ok(),
            _ => None,
        }
    }

    /// The value as a 64-bit unsigned integer, when it carries one.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::U64(value) => Some(*value),
            Self::U32(value) => Some(u64::from(*value)),
            Self::I64(value) => u64::try_from(*value).ok(),
            Self::Text(text) => text.parse().ok(),
            _ => None,
        }
    }

    /// The value as a MAC, when it carries one.
    pub fn as_mac(&self) -> Option<[u8; 6]> {
        match self {
            Self::Mac(bytes) => Some(*bytes),
            _ => None,
        }
    }

    /// The value as floating point, when it carries a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::I64(value) => Some(*value as f64),
            Self::U32(value) => Some(f64::from(*value)),
            Self::U64(value) => Some(*value as f64),
            Self::Text(text) => text.parse().ok(),
            _ => None,
        }
    }
}

/// One probe's report: its typed outcome and how many rows it decoded.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeReport {
    /// The probe id.
    pub probe: String,
    /// The outcome.
    pub outcome: OutcomeKind,
    /// The failure reason, when the outcome is an error.
    pub reason: Option<String>,
    /// Whether the document marked the probe required.
    pub required: bool,
    /// Decoded rows, when the outcome is present.
    pub rows: usize,
}

/// One claim a document's map produced, with the probe and row it came
/// from so the caller can attach it to the right device.
#[derive(Clone, Debug, PartialEq)]
pub struct ObservedClaim {
    /// The probe id that stated it.
    pub probe: String,
    /// The decoded row, when the probe reads rows.
    pub row: Option<usize>,
    /// The model claim.
    pub claim: Claim,
}

/// Why a decoded read stayed out of the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResidualReason {
    /// The decoded read has no core `Source` to attribute a claim or detail
    /// row to.
    NoCoreSource,
}

/// A decoded read with no model destination: the document declared it, the
/// engine decoded it, and the model has nowhere to put it yet.
#[derive(Clone, Debug, PartialEq)]
pub struct ResidualObservation {
    /// The adapter id.
    pub adapter: String,
    /// The probe id.
    pub probe: String,
    /// Why it stayed out of the model.
    pub reason: ResidualReason,
    /// The decoded fields, one entry per row (one entry for scalars).
    pub rows: Vec<BTreeMap<String, FieldValue>>,
}

/// Everything one document run produced.
#[derive(Debug)]
pub struct Observation {
    /// The adapter id.
    pub adapter: String,
    /// One report per probe, in document order.
    pub probes: Vec<ProbeReport>,
    /// Whether the document's LLDP feed (`lldp_rem` layout) answered at all:
    /// present, empty, or absent. `lldp` carries the rows either way; a
    /// timeout, error, or cancellation leaves this false.
    pub lldp_answered: bool,
    /// Whether the probe feeding `details.interfaces` answered at all. An
    /// answered feed with no rows is an empty inventory, not an unknown one.
    pub interfaces_answered: bool,
    /// LLDP neighbours assembled from `lldp_rem` probes.
    pub lldp: Vec<LldpNeighbour>,
    /// ARP bindings from `arp_v4`/`arp_typed` probes, or from an `ip_arp`
    /// command probe.
    pub arp: Vec<ArpRow>,
    /// IPv4 addresses from `ip_address_v4` probes, keyed by ifIndex.
    pub ip_addresses: Vec<IpAddressRow>,
    /// Bridge FDB evidence, when a document declares an FDB table.
    pub fdb: Option<SnmpFdb>,
    /// The associated stations from `ubnt_station` probes.
    pub stations: Vec<AirStation>,
    /// The device capacity the engine derived from the radio, the station
    /// table, or the CRM rate (in that precedence order). Documents never
    /// supply this value.
    pub capacity: CapacityDerivation,
    /// Neighbour rows from the `ip_neighbor` command probe.
    pub neighbours: Vec<ApiNeighbour>,
    /// Interface rows from `single` tables feeding `details.interfaces`.
    pub interfaces: Vec<SnmpInterface>,
    /// Claims mapped from fields, in document order then row order.
    pub claims: Vec<ObservedClaim>,
    /// Device details assembled from the detail maps.
    pub details: Option<DeviceDetails>,
    /// Decoded reads that stayed out of the model.
    pub residuals: Vec<ResidualObservation>,
    /// Whether every required probe answered (`present` or `empty`).
    pub supported: bool,
}

impl Observation {
    /// Whether every required probe answered.
    pub fn supported(&self) -> bool {
        self.supported
    }
}

/// Runs one validated document through `reader`.
///
/// `observed_at` stamps detail snapshots; `cancel` is consulted before each
/// probe and each read. The run never panics on reader answers: every
/// failure is one of the typed outcomes. Capacity uses
/// [`CapacityPolicy::default`]; use [`probe_with_capacity`] to supply the
/// `[ap_polling]` factors and a CRM rate.
pub async fn probe(
    document: &Document,
    reader: &mut (dyn Reader + Send),
    observed_at: SystemTime,
    cancel: &(dyn Fn() -> bool + Send + Sync),
) -> Observation {
    probe_with_capacity(
        document,
        reader,
        observed_at,
        cancel,
        &CapacityPolicy::default(),
    )
    .await
}

/// Runs one validated document with explicit capacity inputs.
pub async fn probe_with_capacity(
    document: &Document,
    reader: &mut (dyn Reader + Send),
    observed_at: SystemTime,
    cancel: &(dyn Fn() -> bool + Send + Sync),
    capacity: &CapacityPolicy,
) -> Observation {
    let probes: Vec<&Probe> = document.probes.iter().collect();
    run(document, &probes, reader, observed_at, cancel, capacity).await
}

/// Runs only the document's probes on one surface through `reader`.
///
/// A multi-surface device runs each surface with the reader that speaks its
/// transport and merges the observations; the document stays one document
/// and its maps stay one map. A surface id the document does not declare
/// runs no probes.
pub async fn probe_surface(
    document: &Document,
    surface_id: &str,
    reader: &mut (dyn Reader + Send),
    observed_at: SystemTime,
    cancel: &(dyn Fn() -> bool + Send + Sync),
) -> Observation {
    probe_surface_with_capacity(
        document,
        surface_id,
        reader,
        observed_at,
        cancel,
        &CapacityPolicy::default(),
    )
    .await
}

/// Runs one surface with explicit capacity inputs.
pub async fn probe_surface_with_capacity(
    document: &Document,
    surface_id: &str,
    reader: &mut (dyn Reader + Send),
    observed_at: SystemTime,
    cancel: &(dyn Fn() -> bool + Send + Sync),
    capacity: &CapacityPolicy,
) -> Observation {
    let probes: Vec<&Probe> = document
        .probes
        .iter()
        .filter(|probe| probe.surface == surface_id)
        .collect();
    run(document, &probes, reader, observed_at, cancel, capacity).await
}

/// Runs the given probes of `document` and finishes the observation.
async fn run(
    document: &Document,
    probes: &[&Probe],
    reader: &mut (dyn Reader + Send),
    observed_at: SystemTime,
    cancel: &(dyn Fn() -> bool + Send + Sync),
    capacity: &CapacityPolicy,
) -> Observation {
    let mut run = Run {
        document,
        limits: Limits::resolve(&document.budget),
        reader,
        observed_at,
        cancel,
        capacity,
        requests: 0,
        rows: 0,
        bytes: 0,
        started: Instant::now(),
        stopped: None,
        results: Vec::new(),
        lldp: Vec::new(),
        arp: Vec::new(),
        ip_addresses: Vec::new(),
        stations: Vec::new(),
        interfaces: Vec::new(),
        fdb_qbridge: None,
        fdb_bridge: None,
        fdb_ports: BTreeSet::new(),
        port_join: None,
    };
    for probe in probes {
        run.run_probe(probe).await;
    }
    run.finish()
}

/// Why a run stopped early.
#[derive(Clone, Debug)]
enum Stop {
    Cancelled,
    Failed(String),
}

impl Stop {
    fn outcome(&self) -> (OutcomeKind, Option<String>) {
        match self {
            Self::Cancelled => (OutcomeKind::Cancelled, None),
            Self::Failed(reason) => (OutcomeKind::Error, Some(reason.clone())),
        }
    }
}

/// One probe's decoded data.
enum ProbeData {
    Scalars(BTreeMap<String, FieldValue>),
    Rows(Vec<BTreeMap<String, FieldValue>>),
}

impl ProbeData {
    fn rows(&self) -> usize {
        match self {
            Self::Scalars(_) => 1,
            Self::Rows(rows) => rows.len(),
        }
    }

    fn as_residual_rows(&self) -> Vec<BTreeMap<String, FieldValue>> {
        match self {
            Self::Scalars(fields) => vec![fields.clone()],
            Self::Rows(rows) => rows.clone(),
        }
    }
}

/// One probe's stored result.
struct ProbeResult {
    probe: String,
    required: bool,
    outcome: OutcomeKind,
    reason: Option<String>,
    read: Read,
    data: ProbeData,
}

/// The run's mutable state.
struct Run<'a, 'r> {
    document: &'a Document,
    limits: Limits,
    reader: &'r mut (dyn Reader + Send),
    observed_at: SystemTime,
    cancel: &'a (dyn Fn() -> bool + Send + Sync),
    capacity: &'a CapacityPolicy,
    requests: u32,
    rows: usize,
    bytes: u64,
    started: Instant,
    stopped: Option<Stop>,
    results: Vec<ProbeResult>,
    lldp: Vec<LldpNeighbour>,
    arp: Vec<ArpRow>,
    ip_addresses: Vec<IpAddressRow>,
    stations: Vec<AirStation>,
    interfaces: Vec<SnmpInterface>,
    fdb_qbridge: Option<Vec<SnmpQBridgeRow>>,
    fdb_bridge: Option<Vec<SnmpBridgeRow>>,
    fdb_ports: BTreeSet<u32>,
    port_join: Option<Vec<(u32, u32)>>,
}

impl Run<'_, '_> {
    /// Runs one probe, recording its report and decoded data.
    async fn run_probe(&mut self, probe: &Probe) {
        if let Some(stop) = self.stopped.clone() {
            let (outcome, reason) = stop.outcome();
            self.report(probe, outcome, reason, ProbeData::Rows(Vec::new()));
            return;
        }
        if let Some(stop) = self.check_stop() {
            let (outcome, reason) = stop.outcome();
            self.stopped = Some(stop);
            self.report(probe, outcome, reason, ProbeData::Rows(Vec::new()));
            return;
        }
        let (outcome, reason, data) = match &probe.read {
            Read::Scalars { scalars } => self.read_scalars(scalars).await,
            Read::Table { table } => self.read_table(probe, table).await,
            Read::Command { command } => self.read_command(command).await,
        };
        if matches!(outcome, OutcomeKind::Cancelled) && self.stopped.is_none() {
            self.stopped = Some(Stop::Cancelled);
        }
        self.report(probe, outcome, reason, data);
    }

    /// Records one probe's report and data.
    fn report(
        &mut self,
        probe: &Probe,
        outcome: OutcomeKind,
        reason: Option<String>,
        data: ProbeData,
    ) {
        self.results.push(ProbeResult {
            probe: probe.id.clone(),
            required: probe.on_absent == super::OnAbsent::Required,
            outcome,
            reason,
            read: probe.read.clone(),
            data,
        });
    }

    /// The stop state for the next read: cancellation first, then the
    /// deadline, then the budgets.
    fn check_stop(&mut self) -> Option<Stop> {
        if (self.cancel)() {
            return Some(Stop::Cancelled);
        }
        if self.started.elapsed() >= self.limits.deadline {
            return Some(Stop::Failed("deadline exceeded".to_owned()));
        }
        if self.requests >= self.limits.max_requests {
            return Some(Stop::Failed("request budget exhausted".to_owned()));
        }
        if self.rows > usize::try_from(self.limits.max_rows).unwrap_or(usize::MAX) {
            return Some(Stop::Failed("row budget exceeded".to_owned()));
        }
        if self.bytes > self.limits.max_bytes {
            return Some(Stop::Failed("byte budget exceeded".to_owned()));
        }
        None
    }

    /// One SNMP get, counted against the budget.
    async fn get(&mut self, oid: &[u32]) -> Outcome<Vec<Varbind>> {
        self.requests += 1;
        let outcome = self.reader.get(oid).await;
        self.account(&outcome);
        outcome
    }

    /// One SNMP walk, counted against the budget.
    async fn walk(&mut self, root: &[u32]) -> Outcome<Vec<Varbind>> {
        self.requests += 1;
        let outcome = self.reader.walk(root).await;
        self.account(&outcome);
        outcome
    }

    /// One named API command, counted against the budget.
    async fn command(&mut self, name: CommandName) -> Outcome<Vec<CommandRow>> {
        self.requests += 1;
        let outcome = self.reader.command(name).await;
        if let Outcome::Present(rows) = &outcome {
            self.bytes += rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|(key, value)| (key.len() + value.len()) as u64)
                        .sum::<u64>()
                })
                .sum::<u64>();
        }
        outcome
    }

    /// Accounts a read's response size.
    fn account(&mut self, outcome: &Outcome<Vec<Varbind>>) {
        if let Outcome::Present(varbinds) = outcome {
            self.bytes += varbinds
                .iter()
                .map(|varbind| {
                    let oid = (varbind.oid.len() * 4) as u64;
                    let value = match &varbind.value {
                        WalkValue::Bytes(bytes) => bytes.len() as u64,
                        WalkValue::Oid(text) | WalkValue::IpAddress(text) => text.len() as u64,
                        WalkValue::Integer(_)
                        | WalkValue::Counter64(_)
                        | WalkValue::TimeTicks(_) => 8,
                        WalkValue::Empty => 0,
                    };
                    oid + value
                })
                .sum::<u64>();
        }
    }

    /// Reads the scalar probes.
    async fn read_scalars(
        &mut self,
        scalars: &[Scalar],
    ) -> (OutcomeKind, Option<String>, ProbeData) {
        let mut fields = BTreeMap::new();
        let mut present = false;
        for scalar in scalars {
            if let Some(stop) = self.check_stop() {
                self.stopped = Some(stop.clone());
                let (outcome, reason) = stop.outcome();
                return (outcome, reason, ProbeData::Scalars(fields));
            }
            let Some(oid) = oid_arcs(&scalar.oid) else {
                continue;
            };
            match self.get(&oid).await {
                Outcome::Present(varbinds) => {
                    if let Some(varbind) = varbinds.first()
                        && let Some(value) = decode(scalar.parser, &varbind.value)
                    {
                        fields.insert(scalar.name.clone(), value);
                        present = true;
                    }
                }
                Outcome::Empty | Outcome::Absent => {}
                Outcome::Timeout => {
                    return (OutcomeKind::Timeout, None, ProbeData::Scalars(fields));
                }
                Outcome::Error { reason } => {
                    return (OutcomeKind::Error, Some(reason), ProbeData::Scalars(fields));
                }
                Outcome::Cancelled => {
                    return (OutcomeKind::Cancelled, None, ProbeData::Scalars(fields));
                }
            }
        }
        let outcome = if present {
            OutcomeKind::Present
        } else {
            OutcomeKind::Absent
        };
        (outcome, None, ProbeData::Scalars(fields))
    }

    /// Reads and decodes one table probe.
    async fn read_table(
        &mut self,
        probe: &Probe,
        table: &Table,
    ) -> (OutcomeKind, Option<String>, ProbeData) {
        let columns: Vec<ColumnSpec> = table
            .columns
            .iter()
            .filter_map(|column| {
                let oid = oid_arcs(&column.oid)?;
                Some(ColumnSpec {
                    name: column.name.clone(),
                    oid,
                    parser: column.parser,
                })
            })
            .collect();
        let mut parents: BTreeSet<Vec<u32>> = BTreeSet::new();
        if table.index.kind == IndexKind::LldpRem {
            // The layout fixes the remote-systems table: walk its root so
            // every time-mark entry answers, not only the one the column
            // OIDs happen to name.
            parents.insert(crate::snmp_tables::LLDP_REM_TABLE.to_vec());
        } else {
            for column in &columns {
                let mut parent = column.oid.clone();
                parent.pop();
                parents.insert(parent);
            }
        }
        let mut varbinds: Vec<Varbind> = Vec::new();
        let mut answered = false;
        for parent in &parents {
            if let Some(stop) = self.check_stop() {
                self.stopped = Some(stop.clone());
                let (outcome, reason) = stop.outcome();
                return (outcome, reason, ProbeData::Rows(Vec::new()));
            }
            match self.walk(parent).await {
                Outcome::Present(rows) => {
                    answered = true;
                    varbinds.extend(rows);
                }
                Outcome::Empty => answered = true,
                Outcome::Absent => {}
                Outcome::Timeout => {
                    return (OutcomeKind::Timeout, None, ProbeData::Rows(Vec::new()));
                }
                Outcome::Error { reason } => {
                    return (
                        OutcomeKind::Error,
                        Some(reason),
                        ProbeData::Rows(Vec::new()),
                    );
                }
                Outcome::Cancelled => {
                    return (OutcomeKind::Cancelled, None, ProbeData::Rows(Vec::new()));
                }
            }
        }
        if !answered {
            return (OutcomeKind::Absent, None, ProbeData::Rows(Vec::new()));
        }
        let grouped = group_rows(&columns, &varbinds);
        if grouped.is_empty() {
            // An answered FDB walk that decoded no rows is an empty
            // forwarding database, not an unknown one: commit the empty
            // snapshot so the distinction survives. A timeout, error, or
            // cancellation returned above and commits nothing.
            match table.index.kind {
                IndexKind::Dot1dMac | IndexKind::Dot1dRowId => {
                    self.fdb_bridge.get_or_insert_with(Vec::new);
                }
                IndexKind::Dot1qVlanMac => {
                    self.fdb_qbridge.get_or_insert_with(Vec::new);
                }
                _ => {}
            }
            return (OutcomeKind::Empty, None, ProbeData::Rows(Vec::new()));
        }
        match table.index.kind {
            IndexKind::LldpRem => {
                self.decode_lldp(probe, table, &columns, &varbinds, &grouped)
                    .await
            }
            IndexKind::ArpV4 | IndexKind::ArpTyped => {
                let (rows, fields) = decode_arp(table.index.kind, grouped);
                self.arp.extend(rows);
                self.count_rows(&fields);
                (OutcomeKind::Present, None, ProbeData::Rows(fields))
            }
            IndexKind::IpAddressV4 => {
                let mut rows = Vec::new();
                let fields: Vec<BTreeMap<String, FieldValue>> = grouped
                    .into_iter()
                    .filter_map(|(instance, fields)| {
                        let if_index = fields.get("if_index").and_then(FieldValue::as_u32);
                        let netmask = fields.get("netmask").map(FieldValue::text);
                        let row = ip_address_v4_row(&instance, if_index, netmask.as_deref())?;
                        rows.push(row);
                        Some(fields)
                    })
                    .collect();
                self.count_rows(&fields);
                self.ip_addresses.extend(rows);
                (OutcomeKind::Present, None, ProbeData::Rows(fields))
            }
            IndexKind::Dot1dMac | IndexKind::Dot1dRowId | IndexKind::Dot1qVlanMac => {
                self.decode_fdb(table, &grouped).await
            }
            IndexKind::UbntStation | IndexKind::Station | IndexKind::StationMac => {
                let rows = decode_stations(table.index.kind, &grouped);
                let fields: Vec<BTreeMap<String, FieldValue>> =
                    grouped.into_iter().map(|(_, fields)| fields).collect();
                self.count_rows(&fields);
                self.stations.extend(rows);
                (OutcomeKind::Present, None, ProbeData::Rows(fields))
            }
            IndexKind::Single => {
                let fields = decode_single(grouped);
                self.count_rows(&fields);
                (OutcomeKind::Present, None, ProbeData::Rows(fields))
            }
        }
    }

    /// Adds decoded rows to the row budget.
    fn count_rows(&mut self, rows: &[BTreeMap<String, FieldValue>]) {
        self.rows += rows.len();
    }

    /// Decodes an LLDP remote-systems table and assembles neighbours.
    async fn decode_lldp(
        &mut self,
        probe: &Probe,
        _table: &Table,
        columns: &[ColumnSpec],
        rem: &[Varbind],
        grouped: &[(Vec<u32>, BTreeMap<String, FieldValue>)],
    ) -> (OutcomeKind, Option<String>, ProbeData) {
        let Some(columns) = lldp_columns(columns) else {
            return (
                OutcomeKind::Error,
                Some(format!(
                    "probe `{}`: lldp_rem columns must sit under the LLDP remote-systems table",
                    probe.id
                )),
                ProbeData::Rows(Vec::new()),
            );
        };
        if let Some(stop) = self.check_stop() {
            self.stopped = Some(stop.clone());
            let (outcome, reason) = stop.outcome();
            return (outcome, reason, ProbeData::Rows(Vec::new()));
        }
        let man = match self
            .walk(&crate::snmp_tables::LLDP_REM_MAN_ADDR_TABLE)
            .await
        {
            Outcome::Present(rows) => rows,
            Outcome::Empty | Outcome::Absent => Vec::new(),
            Outcome::Timeout => return (OutcomeKind::Timeout, None, ProbeData::Rows(Vec::new())),
            Outcome::Error { reason } => {
                return (
                    OutcomeKind::Error,
                    Some(reason),
                    ProbeData::Rows(Vec::new()),
                );
            }
            Outcome::Cancelled => {
                return (OutcomeKind::Cancelled, None, ProbeData::Rows(Vec::new()));
            }
        };
        if let Some(stop) = self.check_stop() {
            self.stopped = Some(stop.clone());
            let (outcome, reason) = stop.outcome();
            return (outcome, reason, ProbeData::Rows(Vec::new()));
        }
        let loc = match self.walk(&crate::snmp_tables::LLDP_LOC_PORT_TABLE).await {
            Outcome::Present(rows) => rows,
            Outcome::Empty | Outcome::Absent => Vec::new(),
            Outcome::Timeout => return (OutcomeKind::Timeout, None, ProbeData::Rows(Vec::new())),
            Outcome::Error { reason } => {
                return (
                    OutcomeKind::Error,
                    Some(reason),
                    ProbeData::Rows(Vec::new()),
                );
            }
            Outcome::Cancelled => {
                return (OutcomeKind::Cancelled, None, ProbeData::Rows(Vec::new()));
            }
        };
        let rem = pairs(rem);
        let man = pairs(&man);
        let loc = pairs(&loc);
        let neighbours = assemble_neighbours(&rem, &man, &loc_port_rows(&loc), columns);
        let fields: Vec<BTreeMap<String, FieldValue>> =
            grouped.iter().map(|(_, fields)| fields.clone()).collect();
        self.count_rows(&fields);
        self.lldp.extend(neighbours);
        (OutcomeKind::Present, None, ProbeData::Rows(fields))
    }

    /// Decodes an FDB table and records its rows and port join. Nothing is
    /// committed until every read the table needs has answered, so a
    /// cancellation never leaves partial rows behind.
    async fn decode_fdb(
        &mut self,
        table: &Table,
        grouped: &[(Vec<u32>, BTreeMap<String, FieldValue>)],
    ) -> (OutcomeKind, Option<String>, ProbeData) {
        if let Some(stop) = self.check_stop() {
            self.stopped = Some(stop.clone());
            let (outcome, reason) = stop.outcome();
            return (outcome, reason, ProbeData::Rows(Vec::new()));
        }
        let kind = table.index.kind;
        let mut bridge_rows: Vec<SnmpBridgeRow> = Vec::new();
        let mut qbridge_rows: Vec<SnmpQBridgeRow> = Vec::new();
        let mut ports: BTreeSet<u32> = BTreeSet::new();
        for (instance, fields) in grouped {
            let port = fields.get("port").and_then(FieldValue::as_u32);
            let status = fields.get("status").and_then(FieldValue::as_u32);
            let mac_value = fields.get("mac").and_then(FieldValue::as_mac);
            if let Some(port) = port {
                ports.insert(port);
            }
            match kind {
                IndexKind::Dot1dMac => {
                    let Some(mac) = mac_octets(instance) else {
                        continue;
                    };
                    bridge_rows.push(SnmpBridgeRow { mac, port, status });
                }
                IndexKind::Dot1dRowId => {
                    let Some(mac) = mac_value else {
                        continue;
                    };
                    bridge_rows.push(SnmpBridgeRow { mac, port, status });
                }
                IndexKind::Dot1qVlanMac => {
                    let Some((vlan, mac)) = vlan_and_mac(instance) else {
                        continue;
                    };
                    qbridge_rows.push(SnmpQBridgeRow {
                        vlan,
                        mac,
                        port,
                        status,
                    });
                }
                _ => {}
            }
        }
        bridge_rows.sort_by_key(|row| row.mac);
        qbridge_rows.sort_by(|a, b| {
            a.vlan
                .cmp(&b.vlan)
                .then(a.mac.cmp(&b.mac))
                .then(a.port.cmp(&b.port))
        });
        if table.bridge_port_resolution == Some(super::BridgePortResolution::Dot1dBasePortIfindex)
            && self.port_join.is_none()
        {
            if let Some(stop) = self.check_stop() {
                self.stopped = Some(stop.clone());
                let (outcome, reason) = stop.outcome();
                return (outcome, reason, ProbeData::Rows(Vec::new()));
            }
            let join = match self.walk(&DOT1D_BASE_PORT_IF_INDEX).await {
                Outcome::Present(rows) => base_port_if_index_rows(
                    &rows
                        .iter()
                        .map(|varbind| (varbind.oid.clone(), varbind.value.clone()))
                        .collect::<Vec<_>>(),
                    &DOT1D_BASE_PORT_IF_INDEX,
                ),
                Outcome::Empty | Outcome::Absent => Vec::new(),
                Outcome::Timeout => {
                    return (OutcomeKind::Timeout, None, ProbeData::Rows(Vec::new()));
                }
                Outcome::Error { reason } => {
                    return (
                        OutcomeKind::Error,
                        Some(reason),
                        ProbeData::Rows(Vec::new()),
                    );
                }
                Outcome::Cancelled => {
                    return (OutcomeKind::Cancelled, None, ProbeData::Rows(Vec::new()));
                }
            };
            self.port_join = Some(join);
        }
        // Every read answered: commit the rows and the join.
        if kind == IndexKind::Dot1qVlanMac {
            self.fdb_qbridge
                .get_or_insert_with(Vec::new)
                .extend(qbridge_rows);
        } else {
            self.fdb_bridge
                .get_or_insert_with(Vec::new)
                .extend(bridge_rows);
        }
        self.fdb_ports.extend(ports);
        let fields: Vec<BTreeMap<String, FieldValue>> =
            grouped.iter().map(|(_, fields)| fields.clone()).collect();
        self.count_rows(&fields);
        (OutcomeKind::Present, None, ProbeData::Rows(fields))
    }

    /// Reads and maps one command probe.
    async fn read_command(
        &mut self,
        command: &super::Command,
    ) -> (OutcomeKind, Option<String>, ProbeData) {
        match self.command(command.name).await {
            Outcome::Present(rows) => {
                let mapped: Vec<BTreeMap<String, FieldValue>> = rows
                    .iter()
                    .map(|row| {
                        command
                            .fields
                            .iter()
                            .filter_map(|field| {
                                row.get(&field.from).map(|text| {
                                    (field.name.clone(), FieldValue::Text(text.clone()))
                                })
                            })
                            .collect()
                    })
                    .collect();
                self.count_rows(&mapped);
                let outcome = if mapped.is_empty() {
                    OutcomeKind::Empty
                } else {
                    OutcomeKind::Present
                };
                (outcome, None, ProbeData::Rows(mapped))
            }
            Outcome::Empty => (OutcomeKind::Empty, None, ProbeData::Rows(Vec::new())),
            Outcome::Absent => (OutcomeKind::Absent, None, ProbeData::Rows(Vec::new())),
            Outcome::Timeout => (OutcomeKind::Timeout, None, ProbeData::Rows(Vec::new())),
            Outcome::Error { reason } => (
                OutcomeKind::Error,
                Some(reason),
                ProbeData::Rows(Vec::new()),
            ),
            Outcome::Cancelled => (OutcomeKind::Cancelled, None, ProbeData::Rows(Vec::new())),
        }
    }

    /// Whether probe `probe` answered at all: present, empty, or absent.
    fn answered(&self, probe: &str) -> bool {
        self.results.iter().any(|result| {
            result.probe == probe
                && matches!(
                    result.outcome,
                    OutcomeKind::Present | OutcomeKind::Empty | OutcomeKind::Absent
                )
        })
    }

    /// The neighbour rows the `ip_neighbor` command probe decoded, mapped by
    /// the documented field names. A run without that probe, or one that did
    /// not answer, has no neighbours.
    fn neighbours(&self) -> Vec<ApiNeighbour> {
        let Some(result) = self.results.iter().find(|result| {
            matches!(
                &result.read,
                Read::Command { command } if command.name == CommandName::IpNeighbor
            ) && matches!(result.outcome, OutcomeKind::Present)
        }) else {
            return Vec::new();
        };
        rows(&result.data)
            .iter()
            .map(|fields| ApiNeighbour {
                address: field_text(fields, "address"),
                mac_address: field_text(fields, "mac"),
                identity: field_text(fields, "identity"),
                interface: field_text(fields, "interface"),
            })
            .collect()
    }

    /// Builds the observation: reports, typed rows, claims, details,
    /// residuals, and the supported verdict.
    fn finish(mut self) -> Observation {
        // Which document surfaces spoke, decided here where the engine's
        // own layout rules live — never re-derived by a caller.
        let lldp_answered = self
            .document
            .probes
            .iter()
            .find(|probe| {
                matches!(
                    &probe.read,
                    Read::Table { table } if table.index.kind == IndexKind::LldpRem
                )
            })
            .is_some_and(|probe| self.answered(&probe.id));
        let interfaces_answered = self
            .document
            .maps
            .details
            .interfaces
            .as_deref()
            .is_some_and(|probe| self.answered(probe));

        let mut claims = Vec::new();
        let mut residuals: Vec<ResidualObservation> = Vec::new();
        let mut unmapped: BTreeSet<String> = BTreeSet::new();
        let mut details = DeviceDetails {
            interfaces: Vec::new(),
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
        };
        let mut has_details = false;

        // Claims, in document order then row order.
        for claim in &self.document.maps.claims {
            for reference in &claim.fields {
                let Some((probe, field)) = reference.split_once('.') else {
                    continue;
                };
                let Some(result) = self.results.iter().find(|result| result.probe == probe) else {
                    continue;
                };
                if !matches!(result.outcome, OutcomeKind::Present) {
                    continue;
                }
                let Some(source) = claim_source(&result.read) else {
                    unmapped.insert(probe.to_owned());
                    continue;
                };
                match &result.data {
                    ProbeData::Scalars(fields) => {
                        if let Some(value) = fields.get(field) {
                            claims.push(ObservedClaim {
                                probe: probe.to_owned(),
                                row: None,
                                claim: Claim {
                                    kind: claim.kind.model(),
                                    value: value.text(),
                                    source,
                                },
                            });
                        }
                    }
                    ProbeData::Rows(rows) => {
                        for (index, row) in rows.iter().enumerate() {
                            if let Some(value) = row.get(field) {
                                claims.push(ObservedClaim {
                                    probe: probe.to_owned(),
                                    row: Some(index),
                                    claim: Claim {
                                        kind: claim.kind.model(),
                                        value: value.text(),
                                        source,
                                    },
                                });
                            }
                        }
                    }
                }
            }
        }

        // Interfaces first: the ARP detail resolves its interface names
        // against them.
        if let Some(reference) = &self.document.maps.details.interfaces
            && let Some(result) = slot_result(&self.results, "interfaces", reference)
        {
            match &result.read {
                Read::Table { table } if table.index.kind == IndexKind::Single => {
                    self.interfaces = interface_rows(&result.data);
                    let mut built = DeviceDetails::from_snmp_interfaces(&self.interfaces);
                    attach_ip_addresses(&mut built.interfaces, &self.ip_addresses);
                    details.interfaces = built.interfaces;
                    has_details = true;
                }
                _ => match map_detail(
                    "interfaces",
                    result,
                    &self.results,
                    &mut details,
                    &self.interfaces,
                    &self.stations,
                    self.observed_at,
                ) {
                    Ok(mapped) => has_details |= mapped,
                    Err(()) => {
                        unmapped.insert(result.probe.clone());
                    }
                },
            }
        }

        for (slot, reference) in detail_slots(&self.document.maps.details) {
            if slot == "interfaces" {
                continue;
            }
            let Some(result) = slot_result(&self.results, slot, reference) else {
                continue;
            };
            match map_detail(
                slot,
                result,
                &self.results,
                &mut details,
                &self.interfaces,
                &self.stations,
                self.observed_at,
            ) {
                Ok(mapped) => has_details |= mapped,
                Err(()) => {
                    unmapped.insert(result.probe.clone());
                }
            }
        }

        for probe in unmapped {
            if let Some(result) = self.results.iter().find(|result| result.probe == probe)
                && matches!(result.outcome, OutcomeKind::Present)
            {
                residuals.push(ResidualObservation {
                    adapter: self.document.id.clone(),
                    probe,
                    reason: ResidualReason::NoCoreSource,
                    rows: result.data.as_residual_rows(),
                });
            }
        }

        let neighbours = self.neighbours();
        let derived_capacity =
            derive_capacity(details.radio.as_ref(), &details.stations, self.capacity);

        let port_if_index = match &self.port_join {
            Some(join) => join.clone(),
            None => self.fdb_ports.iter().map(|port| (*port, *port)).collect(),
        };
        let fdb = if self.fdb_qbridge.is_some() || self.fdb_bridge.is_some() {
            Some(SnmpFdb {
                qbridge: self.fdb_qbridge,
                bridge: self.fdb_bridge,
                port_if_index,
            })
        } else {
            None
        };

        let supported = self.results.iter().all(|result| {
            !result.required || matches!(result.outcome, OutcomeKind::Present | OutcomeKind::Empty)
        });
        let probes = self
            .results
            .iter()
            .map(|result| ProbeReport {
                probe: result.probe.clone(),
                outcome: result.outcome,
                reason: result.reason.clone(),
                required: result.required,
                rows: if matches!(result.outcome, OutcomeKind::Present) {
                    result.data.rows()
                } else {
                    0
                },
            })
            .collect();

        Observation {
            adapter: self.document.id.clone(),
            probes,
            lldp_answered,
            interfaces_answered,
            lldp: self.lldp,
            arp: self.arp,
            ip_addresses: self.ip_addresses,
            fdb,
            stations: self.stations,
            capacity: derived_capacity,
            neighbours,
            interfaces: self.interfaces,
            claims,
            details: has_details.then_some(details),
            residuals,
            supported,
        }
    }
}

/// One declared column with its parsed OID and parser.
struct ColumnSpec {
    name: String,
    oid: Vec<u32>,
    parser: Parser,
}

/// Groups walked varbinds by instance tail: each declared column matches by
/// OID prefix, and the remaining arcs are the row's instance. A varbind
/// feeds every column that ties for the longest matching prefix, so a
/// document may map one OID to two names (for example the raw Kbps and the
/// Mbps form of one capacity column).
fn group_rows(
    columns: &[ColumnSpec],
    varbinds: &[Varbind],
) -> Vec<(Vec<u32>, BTreeMap<String, FieldValue>)> {
    let mut rows: BTreeMap<Vec<u32>, BTreeMap<String, FieldValue>> = BTreeMap::new();
    for varbind in varbinds {
        let mut best: Option<usize> = None;
        for column in columns {
            let Some(tail) = varbind.oid.strip_prefix(column.oid.as_slice()) else {
                continue;
            };
            if tail.is_empty() {
                continue;
            }
            if best.is_none_or(|current| tail.len() < current) {
                best = Some(tail.len());
            }
        }
        let Some(best) = best else {
            continue;
        };
        for column in columns {
            let Some(tail) = varbind.oid.strip_prefix(column.oid.as_slice()) else {
                continue;
            };
            if tail.is_empty() || tail.len() != best {
                continue;
            }
            let Some(value) = decode(column.parser, &varbind.value) else {
                continue;
            };
            rows.entry(tail.to_vec())
                .or_default()
                .insert(column.name.clone(), value);
        }
    }
    rows.into_iter().collect()
}

/// Decodes one raw value with a parser.
fn decode(parser: Parser, value: &WalkValue) -> Option<FieldValue> {
    match parser {
        Parser::Utf8 => crate::snmp_tables::utf8_value(value).map(FieldValue::Text),
        Parser::U32 => code_value(value).map(FieldValue::U32),
        Parser::I64 => match value {
            WalkValue::Integer(value) => Some(FieldValue::I64(*value)),
            _ => None,
        },
        Parser::U64 => match value {
            WalkValue::Counter64(value) | WalkValue::TimeTicks(value) => {
                Some(FieldValue::U64(*value))
            }
            _ => None,
        },
        Parser::Oid => match value {
            WalkValue::Oid(text) => Some(FieldValue::Oid(text.clone())),
            _ => None,
        },
        Parser::OctetMac => mac_value(value).map(FieldValue::Mac),
        Parser::DisplayMac => display_mac_value(value).map(FieldValue::Mac),
        Parser::ChannelWidthMhz => channel_width_mhz(value).map(FieldValue::U32),
        Parser::KbpsToMbps => {
            code_value(value).map(|kbps| FieldValue::U64((u64::from(kbps) + 500) / 1000))
        }
        Parser::RateSuffix => rate_suffix(value).map(FieldValue::U64),
        Parser::IpAddress => match value {
            WalkValue::IpAddress(text) => Some(FieldValue::Ip(text.clone())),
            _ => None,
        },
    }
}

/// The LLDP column numbers a document declares, when every column sits
/// under the remote-systems table.
fn lldp_columns(columns: &[ColumnSpec]) -> Option<LldpColumns> {
    let number = |name: &str| {
        columns
            .iter()
            .find(|column| column.name == name)
            .and_then(|column| column.oid.last().copied())
    };
    for column in columns {
        if !column
            .oid
            .starts_with(crate::snmp_tables::LLDP_REM_TABLE.as_slice())
        {
            return None;
        }
    }
    Some(LldpColumns {
        local_port: number("local_port")?,
        chassis_id: number("chassis_id")?,
        port_id: number("port_id").unwrap_or(0),
        sys_name: number("sys_name").unwrap_or(0),
        sys_descr: number("sys_descr").unwrap_or(0),
    })
}

/// Decodes ARP rows from grouped instances, and enriches each field map
/// with the `if_index` and `ip` the instance carries.
fn decode_arp(
    kind: IndexKind,
    grouped: Vec<(Vec<u32>, BTreeMap<String, FieldValue>)>,
) -> (Vec<ArpRow>, Vec<BTreeMap<String, FieldValue>>) {
    let mut rows = Vec::new();
    let mut fields = Vec::new();
    for (instance, mut row) in grouped {
        let mac = row.get("mac").and_then(FieldValue::as_mac);
        let decoded = match kind {
            IndexKind::ArpV4 => arp_v4_row(&instance, mac),
            IndexKind::ArpTyped => arp_typed_row(&instance, mac),
            _ => None,
        };
        let Some(decoded) = decoded else {
            continue;
        };
        row.insert("if_index".to_owned(), FieldValue::U32(decoded.if_index));
        row.insert("ip".to_owned(), FieldValue::Ip(decoded.ip.clone()));
        rows.push(decoded);
        fields.push(row);
    }
    (rows, fields)
}

/// Enriches `single` table rows with the `index` the instance carries.
fn decode_single(
    grouped: Vec<(Vec<u32>, BTreeMap<String, FieldValue>)>,
) -> Vec<BTreeMap<String, FieldValue>> {
    grouped
        .into_iter()
        .filter_map(|(instance, mut fields)| {
            let [index] = instance.as_slice() else {
                return None;
            };
            fields.insert("index".to_owned(), FieldValue::U32(*index));
            Some(fields)
        })
        .collect()
}

/// The walked varbinds as the pair slice the shared assemblers take.
fn pairs(varbinds: &[Varbind]) -> Vec<(Vec<u32>, WalkValue)> {
    varbinds
        .iter()
        .map(|varbind| (varbind.oid.clone(), varbind.value.clone()))
        .collect()
}

/// Decodes station rows from grouped instances. The `ubnt_station` layout
/// carries `<wlan-if>.<m1>…<m6>` and decodes the MAC from the index; the
/// `station` layout carries one association id and takes the MAC from the
/// required `mac` column; the `station_mac` layout carries the MAC as the
/// index (six octets, optionally followed by a slot discriminator) and
/// requires the `mac` column to agree. Both non-interface layouts report
/// `wlan_if` 0: the index is not an interface.
fn decode_stations(
    kind: IndexKind,
    rows: &[(Vec<u32>, BTreeMap<String, FieldValue>)],
) -> Vec<AirStation> {
    let mut stations: Vec<AirStation> = rows
        .iter()
        .filter_map(|(instance, fields)| {
            let (wlan_if, mac) = match kind {
                IndexKind::Station => {
                    let [_aid] = instance.as_slice() else {
                        return None;
                    };
                    (0, fields.get("mac").and_then(FieldValue::as_mac)?)
                }
                IndexKind::StationMac => {
                    if instance.len() < 6 {
                        return None;
                    }
                    let index_mac = mac_octets(&instance[..6])?;
                    let column_mac = fields.get("mac").and_then(FieldValue::as_mac)?;
                    if column_mac != index_mac {
                        return None;
                    }
                    (0, column_mac)
                }
                IndexKind::UbntStation => {
                    let [wlan_if, mac @ ..] = instance.as_slice() else {
                        return None;
                    };
                    (*wlan_if, mac_octets(mac)?)
                }
                _ => return None,
            };
            Some(AirStation {
                mac,
                wlan_if,
                hostname: fields.get("hostname").map(FieldValue::text),
                signal_dbm: fields.get("signal_dbm").and_then(FieldValue::as_i64),
                noise_dbm: fields.get("noise_dbm").and_then(FieldValue::as_i64),
                last_ip: fields
                    .get("last_ip")
                    .or_else(|| fields.get("ip"))
                    .map(FieldValue::text),
                tx_bps: fields.get("tx_bps").and_then(FieldValue::as_u64),
                rx_bps: fields.get("rx_bps").and_then(FieldValue::as_u64),
                rx_bytes: fields.get("rx_bytes").and_then(FieldValue::as_u64),
                tx_bytes: fields.get("tx_bytes").and_then(FieldValue::as_u64),
                uptime: fields.get("uptime").and_then(FieldValue::as_u64),
                distance_ccq: fields.get("distance_ccq").and_then(FieldValue::as_i64),
                quality: fields.get("quality").and_then(FieldValue::as_i64),
                channel: fields.get("channel").and_then(FieldValue::as_u32),
                ul_rssi: fields.get("ul_rssi").and_then(FieldValue::as_i64),
                dl_rssi: fields.get("dl_rssi").and_then(FieldValue::as_i64),
                ul_snr: fields.get("ul_snr").and_then(FieldValue::as_i64),
                dl_snr: fields.get("dl_snr").and_then(FieldValue::as_i64),
                tx_capacity: fields.get("tx_capacity").and_then(FieldValue::as_u64),
                rx_capacity: fields.get("rx_capacity").and_then(FieldValue::as_u64),
                tx_quality: fields.get("tx_quality").and_then(FieldValue::as_u64),
                dl_rate_mbps: fields.get("dl_rate_mbps").and_then(FieldValue::as_u64),
                model: fields.get("model").map(FieldValue::text),
                software_version: fields.get("software_version").map(FieldValue::text),
            })
        })
        .collect();
    stations.sort_by(|a, b| a.mac.cmp(&b.mac).then(a.wlan_if.cmp(&b.wlan_if)));
    stations
}

/// The model source a claim from this read can carry, when one exists.
fn claim_source(read: &Read) -> Option<Source> {
    match read {
        Read::Scalars { .. } => Some(Source::SnmpSystem),
        Read::Table { table } => match table.index.kind {
            IndexKind::LldpRem => Some(Source::SnmpLldp),
            IndexKind::ArpV4 | IndexKind::ArpTyped => Some(Source::SnmpArp),
            IndexKind::UbntStation => Some(Source::AirOsStations),
            IndexKind::Station | IndexKind::StationMac => Some(Source::SnmpVendor),
            IndexKind::Single
            | IndexKind::IpAddressV4
            | IndexKind::Dot1dMac
            | IndexKind::Dot1dRowId
            | IndexKind::Dot1qVlanMac => None,
        },
        Read::Command { command } => Some(command_source(command.name)),
    }
}

/// The model source a named command's rows carry.
fn command_source(name: CommandName) -> Source {
    match name {
        CommandName::SystemIdentity => Source::RouterOsIdentity,
        CommandName::SystemRouterboard => Source::RouterOsSerial,
        CommandName::IpNeighbor => Source::RouterOsNeighbors,
        CommandName::Interfaces => Source::RouterOsInterfaces,
        CommandName::IpAddress => Source::RouterOsAddresses,
        CommandName::IpArp => Source::RouterOsArp,
        CommandName::BridgePort => Source::RouterOsBridgePorts,
        CommandName::BridgeVlan => Source::RouterOsBridgeVlans,
        CommandName::BridgeHost => Source::RouterOsBridgeHosts,
        CommandName::SystemResource => Source::RouterOsSystem,
        CommandName::SystemHealth => Source::RouterOsHealth,
        CommandName::IpFirewallNat => Source::RouterOsFirewallNat,
        CommandName::IpDhcpServerLease => Source::RouterOsDhcpLease,
    }
}

/// Every declared detail slot with its probe reference.
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

/// Maps one detail slot. `Ok(true)` means it was filled; `Ok(false)` means
/// the slot maps to nothing this engine can build; `Err` means the read has
/// no core source and the probe becomes a residual observation.
fn map_detail(
    slot: &str,
    result: &ProbeResult,
    results: &[ProbeResult],
    details: &mut DeviceDetails,
    interfaces: &[SnmpInterface],
    stations: &[AirStation],
    observed_at: SystemTime,
) -> Result<bool, ()> {
    match (slot, &result.read) {
        ("interfaces", Read::Command { command }) if command.name == CommandName::Interfaces => {
            let interfaces = router_os_interfaces(rows(&result.data));
            let addresses = router_os_addresses(results);
            details.interfaces = DeviceDetails::from_router_os(&interfaces, &addresses).interfaces;
            Ok(true)
        }
        ("system", Read::Command { command }) if command.name == CommandName::SystemResource => {
            let Some(fields) = first_row(&result.data) else {
                return Ok(false);
            };
            details.system = Some(SystemDetails::from_router_os(
                field_text(fields, "version"),
                field_text(fields, "board_name"),
                field_text(fields, "platform"),
                field_text(fields, "uptime"),
            ));
            Ok(true)
        }
        ("temperatures", Read::Command { command })
            if command.name == CommandName::SystemHealth =>
        {
            let readings: Vec<TemperatureReading> = rows(&result.data)
                .iter()
                .filter_map(|fields| {
                    Some(TemperatureReading::from_router_os(
                        // RouterOS health rows name the sensor by the reply
                        // key, which the document maps as `celsius`; the
                        // field map's `name` is the sensor label when the
                        // board reports one.
                        field_text(fields, "name").unwrap_or_else(|| "temperature".to_owned()),
                        fields.get("celsius")?.as_f64()?,
                        observed_at,
                    ))
                })
                .collect();
            details.temperatures = readings;
            Ok(true)
        }
        ("system", Read::Scalars { .. }) => {
            let ProbeData::Scalars(fields) = &result.data else {
                return Ok(false);
            };
            // Only `version` is a documented system scalar; a probe that
            // decoded nothing under that name says nothing about the device,
            // so the slot stays unknown rather than carrying an empty block.
            let Some(version) = fields.get("version") else {
                return Ok(false);
            };
            details.system = Some(SystemDetails {
                version: Some(version.text()),
                board_name: None,
                platform: None,
                uptime: None,
                provenance: vec![Source::SnmpVendor],
            });
            Ok(true)
        }
        ("radio", Read::Scalars { .. }) => {
            let ProbeData::Scalars(fields) = &result.data else {
                return Ok(false);
            };
            let radio = RadioDetails {
                model: fields.get("model").map(FieldValue::text),
                frequency_mhz: fields.get("frequency_mhz").and_then(FieldValue::as_u32),
                channel_width_mhz: fields.get("channel_width_mhz").and_then(FieldValue::as_u32),
                sm_count: fields.get("sm_count").and_then(FieldValue::as_u32),
                tx_power_dbm: fields.get("tx_power_dbm").and_then(FieldValue::as_i64),
                lan_speed_mbps: fields.get("lan_speed_mbps").and_then(FieldValue::as_u64),
                phy_mbps: fields
                    .get("phy_mbps")
                    .and_then(FieldValue::as_f64)
                    .filter(|value| value.is_finite()),
                provenance: vec![Source::SnmpVendor],
            };
            let has_value = radio.model.is_some()
                || radio.frequency_mhz.is_some()
                || radio.channel_width_mhz.is_some()
                || radio.sm_count.is_some()
                || radio.tx_power_dbm.is_some()
                || radio.lan_speed_mbps.is_some()
                || radio.phy_mbps.is_some();
            if !has_value {
                return Ok(false);
            }
            details.radio = Some(radio);
            Ok(true)
        }
        ("temperatures", Read::Table { .. }) => {
            details.temperatures = rows(&result.data)
                .iter()
                .filter_map(|fields| {
                    Some(TemperatureReading {
                        sensor_name: fields.get("name")?.text(),
                        celsius: fields.get("celsius")?.as_f64()?,
                        provenance: vec![Source::SnmpVendor],
                        observed_at,
                    })
                })
                .collect();
            Ok(true)
        }
        ("stations", Read::Table { table })
            if matches!(
                table.index.kind,
                IndexKind::UbntStation | IndexKind::Station | IndexKind::StationMac
            ) =>
        {
            let source = if matches!(table.index.kind, IndexKind::Station | IndexKind::StationMac) {
                Source::SnmpVendor
            } else {
                Source::AirOsStations
            };
            details.stations = stations
                .iter()
                .map(|station| WirelessStation::from_air_station(station.clone(), vec![source]))
                .collect();
            Ok(true)
        }
        ("arp", Read::Table { table })
            if matches!(table.index.kind, IndexKind::ArpV4 | IndexKind::ArpTyped) =>
        {
            let names: BTreeMap<u32, String> = interfaces
                .iter()
                .filter_map(|row| Some((row.index, row.name.clone()?)))
                .collect();
            let rows: Vec<ArpEntry> = rows(&result.data)
                .iter()
                .map(|fields| {
                    let ip = fields.get("ip").map(FieldValue::text);
                    let mac = fields
                        .get("mac")
                        .and_then(FieldValue::as_mac)
                        .and_then(|bytes| Mac::from_chassis(&bytes));
                    let interface = fields
                        .get("if_index")
                        .and_then(FieldValue::as_u32)
                        .and_then(|index| names.get(&index).cloned());
                    ArpEntry {
                        address: ip,
                        mac_address: mac,
                        interface,
                        dynamic: None,
                        complete: Some(true),
                        disabled: None,
                    }
                })
                .collect();
            details.arp = Some(TableSnapshot {
                rows,
                provenance: vec![Source::SnmpArp],
                observed_at,
            });
            Ok(true)
        }
        ("arp", Read::Command { command }) if command.name == CommandName::IpArp => {
            let rows: Vec<ArpEntry> = rows(&result.data)
                .iter()
                .filter_map(|fields| {
                    let row = RouterOsArp {
                        address: Some(field_text(fields, "address")?),
                        mac_address: field_text(fields, "mac"),
                        interface: field_text(fields, "interface"),
                        dynamic: field_bool(fields, "dynamic"),
                        complete: field_bool(fields, "complete"),
                        disabled: field_bool(fields, "disabled"),
                    };
                    Some(ArpEntry::from_router_os(&row))
                })
                .collect();
            details.arp = Some(TableSnapshot {
                rows,
                provenance: vec![Source::RouterOsArp],
                observed_at,
            });
            Ok(true)
        }
        ("bridge_ports", Read::Command { command }) if command.name == CommandName::BridgePort => {
            let rows: Vec<BridgePort> = rows(&result.data)
                .iter()
                .filter_map(|fields| {
                    Some(BridgePort::from_router_os(&RouterOsBridgePort {
                        bridge: field_text(fields, "bridge"),
                        interface: Some(field_text(fields, "interface")?),
                        pvid: field_u32(fields, "pvid"),
                        frame_types: field_text(fields, "frame_types"),
                        status: field_text(fields, "status"),
                        inactive: field_bool(fields, "inactive"),
                        disabled: field_bool(fields, "disabled"),
                        dynamic: field_bool(fields, "dynamic"),
                    }))
                })
                .collect();
            details.bridge_ports = Some(TableSnapshot {
                rows,
                provenance: vec![Source::RouterOsBridgePorts],
                observed_at,
            });
            Ok(true)
        }
        ("bridge_vlans", Read::Command { command }) if command.name == CommandName::BridgeVlan => {
            let rows: Vec<BridgeVlan> = rows(&result.data)
                .iter()
                .filter_map(|fields| {
                    Some(BridgeVlan::from_router_os(&RouterOsBridgeVlan {
                        bridge: Some(field_text(fields, "bridge")?),
                        vlan_ids: field_list(fields, "vlan_ids")
                            .iter()
                            .filter_map(|id| id.parse().ok())
                            .collect(),
                        tagged: field_list(fields, "tagged"),
                        untagged: field_list(fields, "untagged"),
                        current_tagged: field_list(fields, "current_tagged"),
                        current_untagged: field_list(fields, "current_untagged"),
                    }))
                })
                .collect();
            details.bridge_vlans = Some(TableSnapshot {
                rows,
                provenance: vec![Source::RouterOsBridgeVlans],
                observed_at,
            });
            Ok(true)
        }
        ("bridge_hosts", Read::Table { .. }) => {
            let rows: Vec<BridgeHost> = rows(&result.data)
                .iter()
                .filter_map(|fields| {
                    Some(BridgeHost {
                        bridge: fields.get("bridge").map(FieldValue::text),
                        on_interface: fields
                            .get("on_interface")
                            .or_else(|| fields.get("sm_mac"))
                            .map(FieldValue::text),
                        mac_address: fields
                            .get("mac")
                            .and_then(FieldValue::as_mac)
                            .and_then(|bytes| Mac::from_chassis(&bytes)),
                        age: None,
                        dynamic: None,
                        local: None,
                        external: None,
                    })
                })
                .collect();
            details.bridge_hosts = Some(TableSnapshot {
                rows,
                provenance: vec![Source::SnmpVendor],
                observed_at,
            });
            Ok(true)
        }
        ("bridge_hosts", Read::Command { command }) if command.name == CommandName::BridgeHost => {
            let rows: Vec<BridgeHost> = rows(&result.data)
                .iter()
                .filter_map(|fields| {
                    Some(BridgeHost::from_router_os(&RouterOsBridgeHost {
                        bridge: field_text(fields, "bridge"),
                        on_interface: field_text(fields, "on_interface"),
                        mac_address: Some(field_text(fields, "mac")?),
                        age: field_text(fields, "age"),
                        dynamic: field_bool(fields, "dynamic"),
                        local: field_bool(fields, "local"),
                        external: field_bool(fields, "external"),
                    }))
                })
                .collect();
            details.bridge_hosts = Some(TableSnapshot {
                rows,
                provenance: vec![Source::RouterOsBridgeHosts],
                observed_at,
            });
            Ok(true)
        }
        ("nat", Read::Command { command }) if command.name == CommandName::IpFirewallNat => {
            let rows: Vec<NatRule> = rows(&result.data)
                .iter()
                .filter_map(|fields| {
                    let row = RouterOsNatRule {
                        chain: field_text(fields, "chain"),
                        action: field_text(fields, "action"),
                        in_interface: field_text(fields, "in_interface"),
                        out_interface: field_text(fields, "out_interface"),
                        src_address: field_text(fields, "src_address"),
                        dst_address: field_text(fields, "dst_address"),
                        to_addresses: field_text(fields, "to_addresses"),
                        disabled: field_bool(fields, "disabled"),
                        comment: field_text(fields, "comment"),
                    };
                    nat_row_states_a_rule(&row).then(|| NatRule::from_router_os(&row))
                })
                .collect();
            details.nat = Some(TableSnapshot {
                rows,
                provenance: vec![Source::RouterOsFirewallNat],
                observed_at,
            });
            Ok(true)
        }
        ("dhcp_leases", Read::Command { command })
            if command.name == CommandName::IpDhcpServerLease =>
        {
            let rows: Vec<DhcpLease> = rows(&result.data)
                .iter()
                .filter_map(|fields| {
                    // Lease text is trimmed here so whitespace-only
                    // fields are absent rather than stored as spaces; the
                    // walk and the store trim again as defense for older
                    // dumps.
                    let text = |name: &str| {
                        field_text(fields, name)
                            .map(|text| text.trim().to_owned())
                            .filter(|text| !text.is_empty())
                    };
                    let lease = DhcpLease {
                        address: text("address"),
                        mac: field_text(fields, "mac").and_then(|text| Mac::parse(&text)),
                        hostname: text("hostname"),
                        status: text("status"),
                        dynamic: field_bool(fields, "dynamic"),
                        disabled: field_bool(fields, "disabled"),
                    };
                    // A row naming no client — no MAC, address, or
                    // hostname — states no lease this model keeps.
                    (lease.address.is_some() || lease.mac.is_some() || lease.hostname.is_some())
                        .then_some(lease)
                })
                .collect();
            details.dhcp_leases = Some(TableSnapshot {
                rows,
                provenance: vec![Source::RouterOsDhcpLease],
                observed_at,
            });
            Ok(true)
        }
        _ => Err(()),
    }
}

/// The probe that feeds `slot`. The document's named probe wins when this
/// run executed it; in a surface-filtered run that did not execute it, the
/// command probe from this run whose command shapes the slot fills it
/// instead. The document keeps one `maps.details` for all its surfaces, so
/// the alias is the closed command vocabulary's own [`detail_slot`].
fn slot_result<'r>(
    results: &'r [ProbeResult],
    slot: &str,
    reference: &str,
) -> Option<&'r ProbeResult> {
    let present = |result: &ProbeResult| matches!(result.outcome, OutcomeKind::Present);
    if let Some(result) = results.iter().find(|result| result.probe == reference) {
        return present(result).then_some(result);
    }
    results.iter().find(|result| {
        present(result)
            && matches!(
                &result.read,
                Read::Command { command } if detail_slot(command.name) == Some(slot)
            )
    })
}

/// Whether a NAT row states any rule content. The `disabled` flag alone
/// is not a rule: a row the router sent with none of the configured
/// fields says nothing.
fn nat_row_states_a_rule(row: &RouterOsNatRule) -> bool {
    [
        row.chain.as_deref(),
        row.action.as_deref(),
        row.in_interface.as_deref(),
        row.out_interface.as_deref(),
        row.src_address.as_deref(),
        row.dst_address.as_deref(),
        row.to_addresses.as_deref(),
        row.comment.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| !value.trim().is_empty())
}

/// One decoded command field as text; an empty value is absent.
fn field_text(fields: &BTreeMap<String, FieldValue>, name: &str) -> Option<String> {
    fields
        .get(name)
        .map(FieldValue::text)
        .filter(|text| !text.is_empty())
}

/// One decoded command field as a RouterOS boolean: `true`/`yes` and
/// `false`/`no` parse, anything else stays unknown.
fn field_bool(fields: &BTreeMap<String, FieldValue>, name: &str) -> Option<bool> {
    match field_text(fields, name).as_deref() {
        Some("true") | Some("yes") => Some(true),
        Some("false") | Some("no") => Some(false),
        _ => None,
    }
}

/// One decoded command field as an unsigned integer.
fn field_u32(fields: &BTreeMap<String, FieldValue>, name: &str) -> Option<u32> {
    field_text(fields, name)?.parse().ok()
}

/// One decoded command field as the comma-separated list RouterOS sends,
/// kept in reply order with empty entries dropped.
fn field_list(fields: &BTreeMap<String, FieldValue>, name: &str) -> Vec<String> {
    field_text(fields, name)
        .map(|list| {
            list.split(',')
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Builds the typed interface rows from an `interfaces` command's decoded
/// fields. A row without its `.id` has no stable handle and is dropped, the
/// same rule the deleted collector applied.
fn router_os_interfaces(rows: &[BTreeMap<String, FieldValue>]) -> Vec<RouterOsInterface> {
    rows.iter()
        .filter_map(|fields| {
            Some(RouterOsInterface {
                id: field_text(fields, "id")?,
                name: field_text(fields, "name"),
                raw_type: field_text(fields, "raw_type"),
                mac_address: field_text(fields, "mac"),
                running: field_bool(fields, "running"),
                disabled: field_bool(fields, "disabled"),
            })
        })
        .collect()
}

/// The address rows an `ip_address` command probe decoded in this run, for
/// interface-row enrichment. A run without that probe enriches nothing.
fn router_os_addresses(results: &[ProbeResult]) -> Vec<RouterOsAddress> {
    let Some(result) = results.iter().find(|result| {
        matches!(
            &result.read,
            Read::Command { command } if command.name == CommandName::IpAddress
        ) && matches!(result.outcome, OutcomeKind::Present)
    }) else {
        return Vec::new();
    };
    rows(&result.data)
        .iter()
        .filter_map(|fields| {
            Some(RouterOsAddress {
                address: Some(field_text(fields, "address")?),
                network: field_text(fields, "network"),
                interface: field_text(fields, "interface"),
            })
        })
        .collect()
}

/// Attaches `ipAddrTable` rows to interface details by decimal ifIndex. A
/// row whose interface is not in the spine is dropped: the interface set is
/// what the device reported, and an address without an interface has no
/// home. A row with a prefix renders `address/prefix`; without one, the
/// bare address.
fn attach_ip_addresses(interfaces: &mut [Interface], rows: &[IpAddressRow]) {
    for row in rows {
        let Some(interface) = interfaces
            .iter_mut()
            .find(|interface| interface.id.parse::<u32>() == Ok(row.if_index))
        else {
            continue;
        };
        let address = match row.prefix_len {
            Some(prefix) => format!("{}/{prefix}", row.address),
            None => row.address.clone(),
        };
        interface.addresses.push(address);
        interface.provenance.push(Source::SnmpIpAddresses);
    }
    // Same-source pushes are adjacent by construction, so adjacent dedup
    // lists the query once even when one interface has several addresses.
    for interface in interfaces {
        interface.provenance.dedup();
    }
}

/// Builds `SnmpInterface` rows from a `single` table's decoded fields.
fn interface_rows(data: &ProbeData) -> Vec<SnmpInterface> {
    let mut rows: Vec<SnmpInterface> = rows(data)
        .iter()
        .enumerate()
        .map(|(position, fields)| {
            let index = fields
                .get("index")
                .and_then(FieldValue::as_u32)
                .unwrap_or(position as u32);
            SnmpInterface {
                index,
                name: fields.get("name").map(FieldValue::text),
                descr: fields.get("description").map(FieldValue::text),
                alias: fields.get("alias").map(FieldValue::text),
                raw_type: fields.get("raw_type").and_then(FieldValue::as_u32),
                admin_status: fields.get("admin_state").and_then(FieldValue::as_u32),
                oper_status: fields.get("oper_state").and_then(FieldValue::as_u32),
                phys_address: fields
                    .get("mac")
                    .and_then(FieldValue::as_mac)
                    .map(Vec::from),
            }
        })
        .collect();
    rows.sort_by_key(|row| row.index);
    rows
}

/// The decoded rows of a probe, or an empty slice for scalars.
fn rows(data: &ProbeData) -> &[BTreeMap<String, FieldValue>] {
    match data {
        ProbeData::Rows(rows) => rows,
        ProbeData::Scalars(_) => &[],
    }
}

/// The first decoded row of a probe.
fn first_row(data: &ProbeData) -> Option<&BTreeMap<String, FieldValue>> {
    rows(data).first()
}

/// Parses a dotted OID into arcs; `None` when any arc is not a number.
fn oid_arcs(text: &str) -> Option<Vec<u32>> {
    text.strip_prefix('.')
        .unwrap_or(text)
        .split('.')
        .map(|arc| arc.parse().ok())
        .collect()
}

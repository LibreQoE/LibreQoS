//! Bounded read-only network walk: a breadth-first discovery seeded at one
//! vault target that expands through each node's neighbour tables — the
//! RouterOS `/ip neighbor` feed over the binary API and the LLDP
//! remote-systems feed over SNMP, grouped per normalized MAC by
//! [`crate::identity::resolve`].
//!
//! The core ([`walk_observing`]) is pure over an injectable [`NeighbourSource`]:
//! no I/O and no clock, so the same answers always produce the same
//! [`WalkReport`]. The core is async — probes are awaitable and a probe
//! distinguishes cancellation from a feed answering nothing — but still
//! sequential and deterministic. [`NetworkSource`] — the live source,
//! whose pacing, cancellation flag, and transport calls are the walker's
//! whole I/O surface — lives in [`crate::walk_source`]; it precedes each real
//! request with the configured delay and treats a failed probe as no
//! answer, which records the node as a leaf. Each probe also gathers the
//! node's own statements (API identity and routerboard serial; SNMP
//! sysName and sysDescr) as self-claims, appended to the node's
//! neighbour-reported claims, plus the node's self-reported interface
//! details: the RouterOS API queries first, falling back to the SNMP
//! interface columns when the API has no details and an SNMP credential
//! exists. The RouterOS address rows and the SNMP `ipAddrTable` rows
//! inside those details are promoted onto the node as its own `address`
//! claims (bare IP, `routeros-addresses` or `snmp-ip-addresses`,
//! RouterOS first), and fill the node's address only when the walk never
//! learned one — a neighbour report always wins, and a RouterOS address
//! wins over an SNMP one. A node with no MAC
//! yet — the seed — adopts the MAC of the interface whose address set
//! owns the probed address, so the seed becomes a host keyed by that MAC
//! and its details persist; a MAC the walk already knows is never
//! rewritten. A neighbour feed
//! counts as answered when the protocol spoke —
//! an LLDP walk that returns no rows (an agent declining the table) is
//! still an answer, so such a node keeps its SNMP self-claims. The
//! visited set is the
//! MAC set: a MAC is
//! probed at most once, a cycle links back and stops, and a neighbour that
//! reports a known address claims that node's MAC instead of duplicating
//! it. Credentials for newly found addresses come from the injected
//! resolver as class-ordered candidate lists per protocol — the seed
//! target's credential, then default profiles, then target refs, then
//! scoped profiles — and the live source keeps the first candidate that
//! authenticates each node. An address with no candidate, or a node with no
//! address at all, is recorded as a leaf. LLDP local ports the
//! reporting device's local-port table left at the raw number are joined
//! against the node's own interface details when those were collected.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::identity::Sources;
use crate::identity::{
    self, Claim, ClaimKind, Device, DeviceDetails, Source, WirelessStation, bare_address,
};
use crate::mikrotik::{ApiNeighbour, Password};
use crate::net_policy::{AddressError, AddressGate, GateVerdict, JudgedAddress};
use crate::snmp::{AirStation, Community, LldpNeighbour, SnmpVersion};

pub use crate::walk_source::NetworkSource;

mod credentials;

pub(crate) use credentials::attempt_order;
pub use credentials::{CredentialCandidate, CredentialOrigin, NodeCredentials};

/// Default walk depth: one hop from the seed.
pub const DEFAULT_MAX_DEPTH: usize = 1;

/// Default pause before each live probe request.
pub const DEFAULT_DELAY: Duration = Duration::from_millis(150);

/// Default cap on recorded nodes, including the seed.
pub const DEFAULT_MAX_NODES: usize = 256;

/// Walk bounds: how deep to expand, how politely to probe, how many nodes
/// to record.
#[derive(Clone, Copy, Debug)]
pub struct WalkBounds {
    /// Maximum hop distance from the seed; farther nodes are linked but
    /// not recorded or probed.
    pub max_depth: usize,
    /// Live-source pause before each probe request.
    pub delay: Duration,
    /// Maximum number of recorded nodes, including the seed.
    pub max_nodes: usize,
}

impl Default for WalkBounds {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            delay: DEFAULT_DELAY,
            max_nodes: DEFAULT_MAX_NODES,
        }
    }
}

/// A RouterOS API username and password pair.
///
/// `Debug` is redacted: neither field is named by any `Debug` output, and
/// no `Display` implementation exists.
pub struct ApiCredentials {
    /// API user name.
    pub username: String,
    /// API password.
    pub password: Password,
}

impl fmt::Debug for ApiCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiCredentials { username: ***, password: *** }")
    }
}

/// SNMP access to one node: the community plus the SNMP version to
/// speak (v1 for v1-only agents, v2c otherwise).
#[derive(Debug)]
pub struct SnmpAccess {
    /// The SNMP v1/v2c community.
    pub community: Community,
    /// The SNMP version to speak. A v2c candidate is the preferred version,
    /// not an exclusion: the live source retries the same community as v1
    /// when the v2c system read yields no usable data, and then uses the
    /// version that answered for every document probe.
    pub version: SnmpVersion,
}

impl SnmpAccess {
    /// The same community at `version`: the effective access after the
    /// source's version negotiation. The candidate keeps its own version
    /// and origin; this copy serves the document probes.
    pub(crate) fn at_version(&self, version: SnmpVersion) -> SnmpAccess {
        SnmpAccess {
            community: self.community.clone(),
            version,
        }
    }
}

/// A credential candidate's precedence class: what the walker orders
/// candidates by, independent of the candidate's non-secret origin label.
///
/// The classes run from the walk's initial default to its last-resort
/// fallbacks: the seed target's own credential first on every node, then
/// shared default profiles, then a target's explicit refs, then the
/// per-router scoped profiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CredentialClass {
    /// The seed target's credential — the walk's initial default.
    Seed,
    /// A discovery profile without a scope: a shared default.
    Default,
    /// A target's explicit credential ref: a per-address fallback.
    Target,
    /// A discovery profile with a scope: a per-router fallback.
    Scoped,
}

/// One probe's answers: `None` per feed means no answer — no credential
/// for that feed, or the probe failed. `self_claims` carries the probed
/// node's own statements; a failed self-query contributes none and never
/// fails the probe. `details` carries the node's self-reported interfaces
/// when the API answered and the interface query succeeded.
#[derive(Debug)]
pub struct Probe {
    /// LLDP remote-systems rows, when the feed answered.
    pub lldp: Option<Vec<LldpNeighbour>>,
    /// RouterOS neighbour rows, when the feed answered.
    pub api: Option<Vec<ApiNeighbour>>,
    /// The node's own claims: API identity and routerboard serial for a
    /// speaking API feed, SNMP sysName and sysDescr for a speaking LLDP
    /// feed.
    pub self_claims: Vec<Claim>,
    /// The node's self-reported interface details, when the API answered
    /// and its interface query succeeded; a failed details query yields
    /// none and never fails the probe.
    pub details: Option<DeviceDetails>,
    /// The associated wireless stations, when the node is an airOS access
    /// point whose station table answered. `None` means the stations are
    /// unknown (no airOS marker, or the station walk failed) — distinct
    /// from an empty list, which means the AP answered and reports no
    /// stations.
    pub stations: Option<Vec<AirStation>>,
}

/// One probe's outcome: the answers, or the source's cancellation marker
/// carrying whatever the probe gathered before the flag rose. Distinct —
/// a cancelled probe never masquerades as "no feed answered".
#[derive(Debug)]
pub enum Probed {
    /// The probe ran to its end (feeds answered or failed on their own).
    Answers(Probe),
    /// The source was cancelled mid-probe; the partial answers are
    /// carried but the walker leaves them unabsorbed.
    Cancelled(Probe),
}

/// The neighbour feeds for one node address.
///
/// The boxed future keeps the trait object-safe: the walk core is generic
/// over `&mut dyn NeighbourSource` (the live source in production, fake
/// sources in tests), so dynamic dispatch is genuinely needed here. The
/// `Send` supertrait makes `&mut dyn NeighbourSource` itself `Send`, so
/// the walk future can run as a spawned Tokio task.
pub trait NeighbourSource: Send {
    /// Probes `address` with the node's credential candidates; a protocol
    /// with no candidate is not probed. The source picks the first
    /// candidate that authenticates and uses it for every feed and detail
    /// query of the node. Cancellation (the source's flag, or the caller
    /// dropping the future) ends the probe; the outcome says which way it
    /// ended.
    fn probe<'a>(
        &'a mut self,
        address: &'a str,
        snmp: &'a [CredentialCandidate<SnmpAccess>],
        api: &'a [CredentialCandidate<ApiCredentials>],
    ) -> Pin<Box<dyn Future<Output = Probed> + Send + 'a>>;
}

/// A probed node's own addresses, read from its details: every interface
/// row the RouterOS `/ip/address` query or the SNMP `ipAddrTable` query
/// contributed, its addresses in the details' device order, bare (the
/// prefix stripped) and deduplicated across the node — one address on two
/// interfaces is one fact. RouterOS addresses come first; an address both
/// sources state keeps the RouterOS one.
fn own_addresses(details: &DeviceDetails) -> Vec<(Source, String)> {
    let mut own: Vec<(Source, String)> = Vec::new();
    for source in [Source::RouterOsAddresses, Source::SnmpIpAddresses] {
        for interface in &details.interfaces {
            if !interface.provenance.contains(&source) {
                continue;
            }
            for text in &interface.addresses {
                let Some(address) = bare_address(text) else {
                    continue;
                };
                if !own.iter().any(|(_, known)| known == address) {
                    own.push((source, address.to_owned()));
                }
            }
        }
    }
    own
}

/// The MAC of the probed node's interface that owns `address`: the
/// RouterOS address rows attach addresses to interfaces, so the interface
/// carrying the probed address names the MAC the node answers on. A row
/// without a name or without a MAC cannot be chosen. When more than one
/// interface owns the address, the smallest interface name wins — bridge
/// interfaces are eligible like any other — so the choice never depends on
/// device row order. `None` when no interface owns the address: an
/// address alone states no MAC, and the walk never synthesizes one.
fn owning_interface_mac<'a>(details: &'a DeviceDetails, address: &str) -> Option<&'a str> {
    details
        .interfaces
        .iter()
        .filter(|row| {
            row.addresses
                .iter()
                .any(|text| bare_address(text) == Some(address))
        })
        .filter_map(|row| Some((row.name.as_deref()?, row.mac.as_ref()?)))
        .min_by_key(|(name, _)| *name)
        .map(|(_, mac)| mac.as_str())
}

/// Joins the LLDP local ports against the probed node's own interface
/// details, for the ports the device's local-port table left at the raw
/// number: the standard numbers a non-bridge agent's ports by ifIndex, so
/// a raw number matching a collected interface index takes that row's
/// name, and a name that matches a collected interface's description
/// takes the row's name. A port that is already an interface name the
/// device carries stays as it is, and anything unresolvable keeps the raw
/// number — the number is never assumed to name anything on its own.
pub(crate) fn resolve_local_ports(lldp: &mut [LldpNeighbour], details: &DeviceDetails) {
    let by_index: BTreeMap<u32, _> = details
        .interfaces
        .iter()
        .filter_map(|row| row.id.parse::<u32>().ok().map(|index| (index, row)))
        .collect();
    for neighbour in lldp {
        let Some(port) = &neighbour.local_port else {
            continue;
        };
        if details
            .interfaces
            .iter()
            .any(|row| row.name.as_deref() == Some(port.as_str()))
        {
            continue; // already an interface name the device itself carries
        }
        let row = port
            .parse::<u32>()
            .ok()
            .and_then(|num| by_index.get(&num).copied())
            .or_else(|| {
                details
                    .interfaces
                    .iter()
                    .find(|row| row.description.as_deref() == Some(port.as_str()))
            });
        if let Some(name) = row.and_then(|row| row.name.clone()) {
            neighbour.local_port = Some(name);
        }
    }
}

/// How the walk knows a node: it is the seed, or a feed reported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeSource {
    /// The walk's starting target.
    Seed,
    /// Reported by neighbour feeds, per the merge's [`Sources`].
    Feed(Sources),
}

impl fmt::Display for NodeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Seed => f.write_str("seed"),
            Self::Feed(sources) => write!(f, "{sources}"),
        }
    }
}

/// One node the walk recorded.
#[derive(Debug)]
pub struct WalkNode {
    /// Normalized MAC, when known; the seed's stays unknown until a
    /// neighbour reports it or the node's own details name the interface
    /// that owns its address. This is the node's key, not the device's
    /// only MAC: the details keep every interface's own MAC.
    pub mac: Option<String>,
    /// Best address: the API IPv4 first, else the LLDP management address.
    pub address: Option<String>,
    /// Best known name: the RouterOS identity first, else the sysName.
    pub name: Option<String>,
    /// Hop distance from the seed; the seed is 0.
    pub depth: usize,
    /// How the walk knows this node.
    pub source: NodeSource,
    /// Provenance-tagged claims about this node: neighbour-reported
    /// claims first (feed-then-row order), then the node's own
    /// self-claims once probed.
    pub claims: Vec<Claim>,
    /// The node's self-reported interface details, set once probed;
    /// `None` until then, and when no API feed answered or its interface
    /// query failed.
    pub details: Option<DeviceDetails>,
    /// Whether this node is a wireless station folded in from an access
    /// point's association table: an attachment leaf that is never probed
    /// itself.
    pub station: bool,
}

impl WalkNode {
    /// The label a progress update names this node by: its first
    /// non-empty name, else its normalized MAC, else the probed
    /// `address`.
    fn label(&self, address: &str) -> String {
        match &self.name {
            Some(name) if !name.is_empty() => name.clone(),
            _ => self.mac.clone().unwrap_or_else(|| address.to_owned()),
        }
    }
}

/// The walk's live position, reported once per node immediately before it
/// is probed: the node's display label, the address being probed, and how
/// many discovered nodes are queued behind it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkProgress {
    /// The first non-empty name (RouterOS identity, sysName, or LLDP
    /// sysName), else the normalized MAC, else the probed address.
    pub label: String,
    /// The address the walk is about to probe.
    pub address: String,
    /// Discovered nodes with a probe address waiting behind this one in
    /// the walk's queue, excluding this node itself. Address-less queued
    /// leaves are never probed, so they are not counted.
    pub pending: usize,
}

/// The observer-side port one link carries: the value with the concrete
/// query that stated it, so the fusion can render the right namespace —
/// an API `interface` value is an interface name, an LLDP local-port
/// value is the LLDP numbering.
pub type ObservedPort = (Source, String);

/// One directed parent-to-child link between MACs: one observation, with
/// the ports the observer's own probe of that neighbour attributed —
/// never another observer's claims.
#[derive(Clone, Debug)]
pub struct WalkLink {
    /// The parent's MAC, or its address when it is the seed whose MAC the
    /// walk never learned.
    pub from: String,
    /// The child's normalized MAC.
    pub to: String,
    /// The observer-side port this probe attributed, when any: API
    /// `interface` claims take precedence over LLDP local-port claims.
    pub local_port: Option<ObservedPort>,
    /// The neighbour's own port id, when this probe carried one.
    pub remote_port: Option<String>,
    /// Which feed(s) reported the child in this probe.
    pub sources: Sources,
    /// Whether this is a wireless association observation (a folded
    /// access-point station) rather than a port observation.
    pub wireless: bool,
}

/// The finished walk: nodes in discovery order, links in recording order.
/// Deterministic: the same answers always yield the same lines.
#[derive(Debug)]
pub struct WalkReport {
    /// Every recorded node, seed first.
    pub nodes: Vec<WalkNode>,
    /// Every discovered edge.
    pub links: Vec<WalkLink>,
    /// Addresses the scan-policy gate refused, by reason: refused probe
    /// candidates and refused lease addresses share the counters, and a
    /// lease row is never a probe target.
    pub refusals: Refusals,
    /// Why the seed itself was refused before any probe, when it was.
    pub seed_refusal: Option<SeedRefusal>,
}

/// Why a walk refused its own seed before probing anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeedRefusal {
    /// The seed address did not parse as an IP literal.
    Unparseable {
        /// The seed address as configured.
        address: String,
        /// What was wrong with it.
        reason: String,
    },
    /// The seed's judged address is ignored by the scan policy.
    Ignored {
        /// The seed address in canonical form.
        address: String,
    },
    /// The seed's judged address is outside the allow list.
    NotAllowed {
        /// The seed address in canonical form.
        address: String,
    },
}

/// The addresses a walk's policy gate refused, by reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refusals {
    /// Addresses matched by an ignore rule.
    pub ignored: u64,
    /// Addresses outside every allow rule.
    pub not_allowed: u64,
    /// Addresses that never parsed as an IP literal.
    pub unparseable: u64,
}

impl WalkReport {
    /// A walk that refused its own seed before any probe ran: no nodes,
    /// no links, no contacts.
    fn seed_refused(refusal: SeedRefusal) -> Self {
        Self {
            nodes: Vec::new(),
            links: Vec::new(),
            refusals: Refusals::default(),
            seed_refusal: Some(refusal),
        }
    }
}

/// Walks from the seed and calls `observer` with a node's current state
/// every time a node is newly recorded or updated: after the seed is
/// created, after each probe's self-report or interface details land, and
/// after each discovered device or folded station is recorded or merged.
/// The server's discovery job uses this to land hosts in the store while the
/// walk is still running, and `link` to record every directional
/// observation.
///
/// `gate` judges every candidate address — the seed included. A
/// blocked or unparseable seed refuses the walk before any probe; a
/// blocked candidate is never probed, recorded, or linked, and is
/// counted in the report's refusals.
#[allow(clippy::too_many_arguments)]
pub async fn walk_observing(
    seed_address: &str,
    seed_name: &str,
    seed_credentials: NodeCredentials,
    bounds: WalkBounds,
    resolve: &(dyn Fn(&str) -> NodeCredentials + Send + Sync),
    source: &mut dyn NeighbourSource,
    gate: AddressGate<'_>,
    mut hooks: WalkHooks<'_>,
) -> WalkReport {
    // The seed is a candidate too: canonicalize and judge it before any
    // node exists or any probe runs.
    let seed = match gate.judge(seed_address) {
        Ok(GateVerdict::Admit(seed)) => seed,
        Ok(verdict @ GateVerdict::Ignored(_)) => {
            return WalkReport::seed_refused(SeedRefusal::Ignored {
                address: verdict.address().to_owned(),
            });
        }
        Ok(verdict @ GateVerdict::NotAllowed(_)) => {
            return WalkReport::seed_refused(SeedRefusal::NotAllowed {
                address: verdict.address().to_owned(),
            });
        }
        Err(error) => {
            return WalkReport::seed_refused(SeedRefusal::Unparseable {
                address: seed_address.to_owned(),
                reason: error.reason,
            });
        }
    };
    let mut state = WalkState::new(&seed.address, seed_name, seed_credentials, bounds, gate);
    (hooks.observer)(&state.nodes[0]).await;
    while let Some((parent, credentials)) = state.queue.pop_front() {
        if (hooks.cancelled)() {
            break; // a cancelled walk stops probing between nodes
        }
        let Some(address) = state.nodes[parent].address.clone() else {
            continue; // no address: nothing to query, the node stays a leaf
        };
        let progress = WalkProgress {
            label: state.nodes[parent].label(&address),
            address: address.clone(),
            pending: state
                .queue
                .iter()
                .filter(|(index, _)| state.nodes[*index].address.is_some())
                .count(),
        };
        (hooks.progress)(&progress).await;
        let mut probe = match source
            .probe(&address, &credentials.snmp, &credentials.api)
            .await
        {
            Probed::Answers(probe) => probe,
            Probed::Cancelled(_) => {
                break; // the probe was cancelled: results stay unabsorbed
            }
        };
        if (hooks.cancelled)() {
            break; // the probe's results stay unabsorbed: incomplete
        }
        if state.self_report(parent, &probe.self_claims) {
            (hooks.observer)(&state.nodes[parent]).await;
        }
        if probe.details.is_some() || probe.stations.is_some() {
            // Partial answers are legitimate surfaces: interfaces without
            // stations (the station walk failed) and stations without
            // interfaces (a details-only failure) both stand.
            let mut details = probe
                .details
                .take()
                .unwrap_or_else(|| DeviceDetails::from_snmp_interfaces(&[]));
            if let Some(stations) = probe.stations.take() {
                details.stations = stations
                    .into_iter()
                    .map(|station| {
                        WirelessStation::from_air_station(station, vec![Source::AirOsStations])
                    })
                    .collect();
            }
            // The device's own address rows are node evidence: promote
            // them before the details land, so one observer call carries
            // both. Lease addresses are judged first: a refused lease
            // keeps its row and never reaches a probe.
            state.gate_lease_addresses(&mut details);
            state.promote_own_addresses(parent, &details);
            state.nodes[parent].details = Some(details);
            state.adopt_owning_interface_mac(parent);
            (hooks.observer)(&state.nodes[parent]).await;
        }
        let stations = state.nodes[parent]
            .details
            .as_ref()
            .map(|details| details.stations.clone())
            .unwrap_or_default();
        state.fold_stations(parent, &stations, &mut hooks).await;
        if probe.lldp.is_none() && probe.api.is_none() {
            continue; // answered neither feed: a leaf
        }
        let devices = identity::resolve(
            &probe.lldp.unwrap_or_default(),
            &probe.api.unwrap_or_default(),
        );
        for device in &devices {
            if let Some(index) = state.absorb(parent, device, resolve, &mut hooks).await {
                (hooks.observer)(&state.nodes[index]).await;
            }
            if (hooks.cancelled)() {
                break; // between discovered devices: stop the walk steps
            }
        }
    }
    state.finish()
}

/// One hook call's continuation, boxed so [`WalkHooks`] stays
/// dyn-compatible. The future owns what it needs (a cloned sender), so
/// the hook closures themselves stay borrowable.
pub type HookFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The call-side hooks a walk reports to: `cancelled` is consulted
/// between probes, walk steps, and pacing waits; `observer` fires at
/// every node the walk records or updates; `link` fires at every
/// directional link observation the walk records; `progress` fires once
/// per probe candidate immediately before it is probed. The observers are
/// awaitable: the walk awaits each call, so a hook that awaits a channel
/// send keeps the same backpressure and ordering the synchronous hooks
/// had through their blocking sends.
pub struct WalkHooks<'a> {
    /// Whether the walk should stop probing and report incomplete.
    pub cancelled: &'a (dyn Fn() -> bool + Send + Sync),
    /// Called with a node's current state at every recording or update.
    pub observer: &'a mut (dyn FnMut(&WalkNode) -> HookFuture + Send),
    /// Called with a link's current evidence at every recording.
    pub link: &'a mut (dyn FnMut(&ObservedLink) -> HookFuture + Send),
    /// Called once with the node about to be probed, after the
    /// cancellation and address checks and immediately before the probe.
    pub progress: &'a mut (dyn FnMut(&WalkProgress) -> HookFuture + Send),
}

/// One directional link observation, streamed as the walk records it:
/// the observer-side endpoint (MAC when the node has one, else its
/// address — `None` when the endpoint is not yet identified), the
/// neighbour's MAC, the ports each end owns, the reporting feed(s), and
/// whether the observation is a wireless association.
#[derive(Clone, Debug)]
pub struct ObservedLink {
    /// The observer-side endpoint, MAC first.
    pub from: Option<String>,
    /// Whether `from` is a MAC (false means an address, or `None`).
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
}

/// The walk's mutable state: nodes in discovery order, the visited MAC
/// set, links as recorded per observation, the probe queue, and the
/// policy gate with its refusal counters.
struct WalkState<'a> {
    bounds: WalkBounds,
    gate: AddressGate<'a>,
    refusals: Refusals,
    nodes: Vec<WalkNode>,
    visited: HashSet<String>,
    links: Vec<RawLink>,
    queue: VecDeque<(usize, NodeCredentials)>,
}

/// One recorded observation, before endpoint resolution: the parent node
/// index, the child MAC, and the ports the parent's own probe attributed
/// to it. `wireless` marks an association observation (a folded station)
/// rather than a port observation.
#[derive(Clone)]
struct RawLink {
    parent: usize,
    to: String,
    local_port: Option<ObservedPort>,
    remote_port: Option<String>,
    sources: Sources,
    wireless: bool,
}

impl<'a> WalkState<'a> {
    /// Seeds the state: node 0 is the seed, queued for probing. The seed
    /// address arrives canonicalized (the caller gated it).
    fn new(
        seed_address: &str,
        seed_name: &str,
        seed_credentials: NodeCredentials,
        bounds: WalkBounds,
        gate: AddressGate<'a>,
    ) -> Self {
        Self {
            bounds,
            gate,
            refusals: Refusals::default(),
            nodes: vec![WalkNode {
                mac: None,
                address: Some(seed_address.to_owned()),
                name: Some(seed_name.to_owned()),
                depth: 0,
                source: NodeSource::Seed,
                claims: Vec::new(),
                details: None,
                station: false,
            }],
            visited: HashSet::new(),
            links: Vec::new(),
            queue: VecDeque::from([(0, seed_credentials)]),
        }
    }

    /// Accounts one gate verdict: the admitted address, or `None` with the
    /// matching refusal counter advanced.
    fn account_verdict(
        &mut self,
        verdict: Result<GateVerdict, AddressError>,
    ) -> Option<JudgedAddress> {
        match verdict {
            Ok(GateVerdict::Admit(admitted)) => Some(admitted),
            Ok(GateVerdict::Ignored(_)) => {
                self.refusals.ignored += 1;
                None
            }
            Ok(GateVerdict::NotAllowed(_)) => {
                self.refusals.not_allowed += 1;
                None
            }
            Err(_) => {
                self.refusals.unparseable += 1;
                None
            }
        }
    }

    /// Folds one resolved neighbour device into the walk: a link per
    /// observer-side port observation — the ports come from this probe's
    /// device alone; a new probed node — carrying the device's claims —
    /// only when the MAC is unseen, no known node already owns the
    /// reported address, and both bounds allow it. Returns the index of
    /// the node the device was recorded on or updated, when either
    /// happened.
    async fn absorb(
        &mut self,
        parent: usize,
        device: &Device,
        resolve: &(dyn Fn(&str) -> NodeCredentials + Send + Sync),
        hooks: &mut WalkHooks<'_>,
    ) -> Option<usize> {
        // The policy gate runs before anything is recorded or contacted:
        // a blocked or unparseable candidate leaves no node, no link,
        // and no credential lookup. A candidate without an address (a
        // MAC-only report) is not judged and is unaffected. Under an
        // unrestricted gate every candidate is admitted verbatim.
        let canonical_address = match device.address() {
            Some(raw) => {
                let verdict = self.gate.judge(&raw);
                // A refused candidate leaves no node, no link, and no
                // credential lookup; the helper counts the refusal.
                match self.account_verdict(verdict) {
                    Some(admitted) => Some(admitted),
                    None => return None,
                }
            }
            None => None,
        };
        // The node address is the gated text, so every later comparison
        // and contact uses exactly the judged address.
        let probe_address = canonical_address
            .as_ref()
            .map(|admitted| admitted.address.clone());
        let mac = device.mac.as_str();
        if self.nodes[parent].mac.as_deref() == Some(mac) {
            return None; // a node never links to itself
        }
        let remote_port = device
            .claims()
            .iter()
            .find(|claim| claim.kind == ClaimKind::PortId)
            .map(|claim| claim.value.clone());
        // API `interface` claims are the observer side and keep their
        // precedence; LLDP local ports only name the observation when the
        // API named no interface. One link per distinct port value keeps
        // parallel observations separate entries.
        let api: Vec<_> = device
            .claims()
            .iter()
            .filter(|claim| claim.kind == ClaimKind::Interface)
            .map(|claim| (claim.source, claim.value.clone()))
            .collect();
        let local_ports: Vec<ObservedPort> = if api.is_empty() {
            device
                .claims()
                .iter()
                .filter(|claim| claim.kind == ClaimKind::LocalPort)
                .map(|claim| (claim.source, claim.value.clone()))
                .collect()
        } else {
            api
        };
        // One recorded link + its emitted observation. A sync closure
        // cannot await the hook, so this is a local async fn taking the
        // values the old closure captured.
        async fn push(
            state: &mut WalkState<'_>,
            hooks: &mut WalkHooks<'_>,
            parent: usize,
            mac: &str,
            remote_port: &Option<String>,
            sources: Sources,
            local_port: Option<ObservedPort>,
        ) {
            let link = RawLink {
                parent,
                to: mac.to_owned(),
                local_port,
                remote_port: remote_port.clone(),
                sources,
                wireless: false,
            };
            state.links.push(link.clone());
            state.emit_link(hooks, &link).await;
        }
        if local_ports.is_empty() {
            push(
                self,
                hooks,
                parent,
                mac,
                &remote_port,
                device.sources(),
                None,
            )
            .await;
        }
        for port in local_ports {
            push(
                self,
                hooks,
                parent,
                mac,
                &remote_port,
                device.sources(),
                Some(port),
            )
            .await;
        }
        if self.visited.contains(mac) {
            // seen: the link back is all that is new — and the reporter's
            // claims about the known node are recorded on it.
            let index = self
                .nodes
                .iter()
                .position(|node| node.mac.as_deref() == Some(mac));
            let mut changed = false;
            if let Some(index) = index {
                let known = &mut self.nodes[index];
                for claim in device.claims() {
                    if !known.claims.contains(claim) {
                        known.claims.push(claim.clone());
                        changed = true;
                    }
                }
            }
            return changed.then_some(index).flatten();
            // seen: the link back is all that is new; cycles end here
        }
        // A reported address that belongs to an existing node is that node:
        // claim its MAC (this is how the seed learns its own) and record
        // the reporter's claims about it, instead of duplicating it.
        if let Some(known) = probe_address.as_deref().and_then(|address| {
            self.nodes
                .iter_mut()
                .find(|node| node.address.as_deref() == Some(address))
        }) {
            let mut changed = known.mac.is_none();
            if known.mac.is_none() {
                known.mac = Some(mac.to_owned());
            }
            for claim in device.claims() {
                if !known.claims.contains(claim) {
                    known.claims.push(claim.clone());
                    changed = true;
                }
            }
            self.visited.insert(mac.to_owned());
            return changed.then(|| {
                self.nodes
                    .iter()
                    .position(|node| node.address.as_deref() == probe_address.as_deref())
                    .unwrap_or(0)
            });
        }
        let depth = self.nodes[parent].depth + 1;
        if depth > self.bounds.max_depth || self.nodes.len() >= self.bounds.max_nodes {
            return None; // bounded: the link is recorded, the node is not walked
        }
        let credentials = probe_address
            .as_deref()
            .map(resolve)
            .unwrap_or_else(NodeCredentials::none);
        self.visited.insert(mac.to_owned());
        self.nodes.push(WalkNode {
            mac: Some(mac.to_owned()),
            address: probe_address,
            name: device.name(),
            depth,
            source: NodeSource::Feed(device.sources()),
            claims: device.claims().to_vec(),
            details: None,
            station: false,
        });
        self.queue.push_back((self.nodes.len() - 1, credentials));
        Some(self.nodes.len() - 1)
    }

    /// Folds an access point's associated stations into the walk: each
    /// station is a leaf node keyed by its exact MAC (an existing node
    /// with that MAC is reused, never silently fused with another MAC),
    /// with the hostname and last-seen address riding as claims, and a
    /// wireless link from the AP. Stations are association leaves: they
    /// are never probed and never queued. The observer fires for every
    /// station recorded or updated.
    async fn fold_stations(
        &mut self,
        parent: usize,
        stations: &[WirelessStation],
        hooks: &mut WalkHooks<'_>,
    ) {
        for station in stations {
            let mac = station.mac.as_str().to_owned();
            if self.nodes[parent].mac.as_deref() == Some(mac.as_str()) {
                continue; // a node never links to itself
            }
            // The last-seen IP is claim and display data — and still
            // policy data: a blocked or unparseable address omits the
            // claim (and counts), leaving the MAC-only association
            // leaf. Stations are never probed.
            let judged_address = station
                .last_ip
                .as_deref()
                .and_then(|raw| {
                    let verdict = self.gate.judge(raw);
                    self.account_verdict(verdict)
                })
                .map(|admitted| admitted.address);
            let claims: Vec<_> = [
                (ClaimKind::Name, station.hostname.clone()),
                (ClaimKind::Address, judged_address.clone()),
            ]
            .into_iter()
            .filter_map(|(kind, value)| {
                value.filter(|value| !value.is_empty()).map(|value| Claim {
                    kind,
                    value,
                    source: Source::AirOsStations,
                })
            })
            .collect();
            let link = RawLink {
                parent,
                to: mac.clone(),
                local_port: None,
                remote_port: None,
                sources: Sources::Lldp,
                wireless: true,
            };
            self.links.push(link.clone());
            self.emit_link(hooks, &link).await;
            if self.visited.contains(&mac) {
                if let Some(index) = self
                    .nodes
                    .iter()
                    .position(|node| node.mac.as_deref() == Some(mac.as_str()))
                {
                    let known = &mut self.nodes[index];
                    let mut changed = false;
                    for claim in &claims {
                        if !known.claims.contains(claim) {
                            known.claims.push(claim.clone());
                            changed = true;
                        }
                    }
                    if changed {
                        (hooks.observer)(&self.nodes[index]).await;
                    }
                }
                continue;
            }
            if self.nodes.len() >= self.bounds.max_nodes {
                continue; // bounded: the link is recorded, the node is not
            }
            self.visited.insert(mac.clone());
            self.nodes.push(WalkNode {
                mac: Some(mac),
                address: judged_address,
                name: station.hostname.clone(),
                depth: self.nodes[parent].depth + 1,
                source: NodeSource::Feed(Sources::Lldp),
                claims,
                details: None,
                station: true,
            });
            let index = self.nodes.len() - 1;
            (hooks.observer)(&self.nodes[index]).await;
        }
    }

    /// Appends a probed node's self-claims after its neighbour-reported
    /// claims, deduplicating identical claims. Returns whether anything
    /// was added.
    fn self_report(&mut self, id: usize, self_claims: &[Claim]) -> bool {
        let node = &mut self.nodes[id];
        let mut changed = false;
        for claim in self_claims {
            if !node.claims.contains(claim) {
                node.claims.push(claim.clone());
                changed = true;
            }
        }
        changed
    }

    /// Judges every DHCP lease address through the scan-policy gate
    /// before the details land on the node. The lease text is
    /// canonicalized first, so the guarantee holds even for an
    /// unrestricted gate that would otherwise admit raw text verbatim: a
    /// non-IP address is dropped as unparseable, and an admitted address
    /// is stored canonical. An ignored or not-allowed address is dropped
    /// from the address fact with its refusal counted. The row keeps its
    /// MAC and hostname either way, and a lease address never becomes a
    /// probe target.
    fn gate_lease_addresses(&mut self, details: &mut DeviceDetails) {
        let Some(table) = &mut details.dhcp_leases else {
            return;
        };
        for row in &mut table.rows {
            let Some(address) = row.address.clone() else {
                continue;
            };
            let Ok(canonical) = crate::net_policy::canonicalize(&address) else {
                row.address = None;
                self.refusals.unparseable += 1;
                continue;
            };
            let verdict = self.gate.judge(canonical.probe_text());
            row.address = self
                .account_verdict(verdict)
                .map(|admitted| admitted.address);
        }
    }

    /// Promotes a probed node's own address rows into node state: each bare
    /// address becomes an `address` claim tagged with the query that stated
    /// it (`routeros-addresses` or `snmp-ip-addresses`), RouterOS addresses
    /// first and then SNMP-only addresses, each group in device order and
    /// deduplicated; an address both sources state keeps the RouterOS
    /// claim. The node's best address is a neighbour report and stands; it
    /// is filled from the first own address only when the walk has none.
    /// Self-statements about a node that was already probed: nothing is
    /// queued, re-probed, or gated here.
    fn promote_own_addresses(&mut self, id: usize, details: &DeviceDetails) {
        let own = own_addresses(details);
        let node = &mut self.nodes[id];
        for (source, value) in &own {
            let claim = Claim {
                kind: ClaimKind::Address,
                value: value.clone(),
                source: *source,
            };
            if !node.claims.contains(&claim) {
                node.claims.push(claim);
            }
        }
        if node.address.is_none() {
            node.address = own.into_iter().next().map(|(_, value)| value);
        }
    }

    /// Gives a node its own MAC when its details name the interface that
    /// owns its address: the seed starts MAC-less, and a probe that
    /// reports its address rows ties it to that interface's MAC — the one
    /// defensible self-reported MAC for the probed address. The adopted
    /// MAC joins the visited set, so a neighbour reporting the same MAC
    /// links to this node instead of forking a duplicate. A MAC the walk
    /// already knows — learned from a neighbour — is never replaced, and a
    /// node whose address no interface owns stays MAC-less. A candidate the
    /// visited set already holds is skipped too: adopting it would give two
    /// nodes one MAC.
    fn adopt_owning_interface_mac(&mut self, id: usize) {
        let node = &self.nodes[id];
        if node.mac.is_some() {
            return;
        }
        let Some(address) = node.address.as_deref() else {
            return;
        };
        let Some(mac) = node
            .details
            .as_ref()
            .and_then(|details| owning_interface_mac(details, address))
            .map(str::to_owned)
        else {
            return;
        };
        if self.visited.contains(&mac) {
            return;
        }
        self.nodes[id].mac = Some(mac.clone());
        self.visited.insert(mac);
    }

    /// Emits one link observation through the hooks: the endpoints are
    /// resolved from the node state as it stands now (the observer side
    /// may still be MAC-less and go out as an address, or unresolved).
    async fn emit_link(&self, hooks: &mut WalkHooks<'_>, link: &RawLink) {
        let parent = &self.nodes[link.parent];
        (hooks.link)(&ObservedLink {
            from: parent.mac.clone().or_else(|| parent.address.clone()),
            from_is_mac: parent.mac.is_some(),
            to: link.to.clone(),
            local_port: link.local_port.as_ref().map(|(_, port)| port.clone()),
            local_port_source: link
                .local_port
                .as_ref()
                .map(|(source, _)| source.to_string()),
            remote_port: link.remote_port.clone(),
            sources: link.sources.to_string(),
            wireless: link.wireless,
        })
        .await;
    }

    /// Resolves link endpoints to their final keys (MAC, else address).
    fn finish(self) -> WalkReport {
        let links = self
            .links
            .into_iter()
            .map(|link| WalkLink {
                from: self.nodes[link.parent]
                    .mac
                    .clone()
                    .or_else(|| self.nodes[link.parent].address.clone())
                    .unwrap_or_else(|| "-".to_owned()),
                to: link.to,
                local_port: link.local_port,
                remote_port: link.remote_port,
                sources: link.sources,
                wireless: link.wireless,
            })
            .collect();
        WalkReport {
            nodes: self.nodes,
            links,
            refusals: self.refusals,
            // A seed refusal never constructs state: it returns before
            // the walk exists, so state-carried reports have none.
            seed_refusal: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Interface;
    use crate::net_policy::{ScanDecision, ScanPolicy};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::SystemTime;

    use crate::snmp::{Community, SnmpVersion};

    /// A source that reports one brand-new device per probe — each with
    /// a fresh address and credentials the resolver grants — so the walk
    /// would keep probing forever unless something stops it.
    struct CountingSource {
        probes: Arc<AtomicUsize>,
    }

    impl NeighbourSource for CountingSource {
        fn probe<'a>(
            &'a mut self,
            _address: &'a str,
            _snmp: &'a [CredentialCandidate<SnmpAccess>],
            _api: &'a [CredentialCandidate<ApiCredentials>],
        ) -> Pin<Box<dyn Future<Output = Probed> + Send + 'a>> {
            Box::pin(async move {
                let count = self.probes.fetch_add(1, Ordering::Relaxed) + 1;
                let probe = Probe {
                    lldp: Some(vec![LldpNeighbour {
                        chassis_id: vec![2, 31, 94, 0, 0, u8::try_from(count % 256).unwrap_or(255)],
                        address: Some(format!("10.0.0.{count}")),
                        remote_port_id: Some(format!("eth{count}")),
                        sys_name: Some(format!("node-{count}")),
                        sys_descr: None,
                        local_port: None,
                    }]),
                    api: None,
                    self_claims: Vec::new(),
                    details: None,
                    stations: None,
                };
                // The scripted source answers as soon as it is polled.
                Probed::Answers(probe)
            })
        }
    }

    #[tokio::test]
    async fn cancelled_walk_stops_probing() {
        let probes = Arc::new(AtomicUsize::new(0));
        let mut source = CountingSource {
            probes: probes.clone(),
        };
        let resolve = |_: &str| {
            NodeCredentials::none().with_target_snmp(SnmpAccess {
                community: Community::new("sentinel-community-1".to_owned()),
                version: SnmpVersion::V2c,
            })
        };
        // The predicate turns true after the third probe: the walk must
        // not start a fourth.
        let cancelled = || probes.load(Ordering::Relaxed) >= 3;
        // Three hops: the cancelled walk has depth to reach a third
        // probe, so the test pins the cancellation boundary itself.
        let bounds = WalkBounds {
            max_depth: 3,
            ..WalkBounds::default()
        };
        let report = walk_observing(
            "10.0.0.255",
            "seed",
            NodeCredentials::none(),
            bounds,
            &resolve,
            &mut source,
            AddressGate::unrestricted(),
            WalkHooks {
                cancelled: &cancelled,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(
            probes.load(Ordering::Relaxed),
            3,
            "a cancelled walk stops probing between nodes"
        );
        assert!(
            !report.nodes.is_empty(),
            "the walk still reports what it collected: incomplete, not empty"
        );
    }

    #[tokio::test]
    async fn link_observations_carry_endpoints_and_ports() {
        // One probe returns one LLDP neighbour row; the walk emits the
        // link with the SEED'S ADDRESS as the observer end (the seed has
        // no MAC until a neighbour reports it) and the neighbour's MAC
        // as the far end, carrying the neighbour's remote port.
        struct OneNeighbour;
        impl NeighbourSource for OneNeighbour {
            fn probe<'a>(
                &'a mut self,
                _address: &'a str,
                _snmp: &'a [CredentialCandidate<SnmpAccess>],
                _api: &'a [CredentialCandidate<ApiCredentials>],
            ) -> Pin<Box<dyn Future<Output = Probed> + Send + 'a>> {
                Box::pin(async move {
                    let probe = Probe {
                        lldp: Some(vec![LldpNeighbour {
                            chassis_id: vec![2, 31, 94, 0, 0, 1],
                            address: Some("10.0.0.1".to_owned()),
                            remote_port_id: Some("eth0".to_owned()),
                            sys_name: Some("node-1".to_owned()),
                            sys_descr: None,
                            local_port: None,
                        }]),
                        api: None,
                        self_claims: Vec::new(),
                        details: None,
                        stations: None,
                    };
                    // The scripted source answers as soon as it is polled.
                    Probed::Answers(probe)
                })
            }
        }
        let mut emitted: Vec<ObservedLink> = Vec::new();
        let report = walk_observing(
            "10.0.0.254",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_: &str| NodeCredentials::none(),
            &mut OneNeighbour,
            AddressGate::unrestricted(),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |link: &ObservedLink| {
                    emitted.push(link.clone());
                    Box::pin(async {})
                },
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(report.nodes.len(), 2, "the seed plus the neighbour");
        assert_eq!(emitted.len(), 1, "one probe, one link observation");
        assert_eq!(
            emitted[0].from.as_deref(),
            Some("10.0.0.254"),
            "the observer end is the seed's address (the seed has no MAC yet)"
        );
        assert!(!emitted[0].from_is_mac);
        assert_eq!(emitted[0].to, "02:1f:5e:00:00:01");
        assert_eq!(emitted[0].remote_port.as_deref(), Some("eth0"));
        assert!(!emitted[0].wireless);
    }

    /// A source reporting two brand-new neighbours per probe, so every
    /// probe records two link observations.
    struct GrowingSource {
        probes: Arc<AtomicUsize>,
    }

    impl NeighbourSource for GrowingSource {
        fn probe<'a>(
            &'a mut self,
            _address: &'a str,
            _snmp: &'a [CredentialCandidate<SnmpAccess>],
            _api: &'a [CredentialCandidate<ApiCredentials>],
        ) -> Pin<Box<dyn Future<Output = Probed> + Send + 'a>> {
            Box::pin(async move {
                let count = self.probes.fetch_add(1, Ordering::Relaxed) + 1;
                let base = count * 8;
                let probe = Probe {
                    lldp: Some(vec![
                        LldpNeighbour {
                            chassis_id: vec![
                                2,
                                31,
                                94,
                                0,
                                0,
                                u8::try_from(base % 256).unwrap_or(255),
                            ],
                            address: Some(format!("10.0.{}.1", base)),
                            remote_port_id: Some("eth0".to_owned()),
                            sys_name: None,
                            sys_descr: None,
                            local_port: None,
                        },
                        LldpNeighbour {
                            chassis_id: vec![
                                2,
                                31,
                                94,
                                0,
                                0,
                                u8::try_from((base + 1) % 256).unwrap_or(255),
                            ],
                            address: Some(format!("10.0.{}.2", base)),
                            remote_port_id: Some("eth1".to_owned()),
                            sys_name: None,
                            sys_descr: None,
                            local_port: None,
                        },
                    ]),
                    api: None,
                    self_claims: Vec::new(),
                    details: None,
                    stations: None,
                };
                // The scripted source answers as soon as it is polled.
                Probed::Answers(probe)
            })
        }
    }

    #[tokio::test]
    async fn cancelled_walk_stops_emitting_links() {
        // The predicate turns true after the first probe: the second
        // probe still runs, but its rows stay unabsorbed, so only the
        // first probe's two links are ever emitted.
        let probes = Arc::new(AtomicUsize::new(0));
        let links_seen = Arc::new(AtomicUsize::new(0));
        let mut source = GrowingSource {
            probes: probes.clone(),
        };
        let link_counter = links_seen.clone();
        let probes_for_cancel = probes.clone();
        let cancelled = move || probes_for_cancel.load(Ordering::Relaxed) >= 2;
        walk_observing(
            "10.0.0.254",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_: &str| NodeCredentials::none(),
            &mut source,
            AddressGate::unrestricted(),
            WalkHooks {
                cancelled: &cancelled,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_: &ObservedLink| {
                    link_counter.fetch_add(1, Ordering::Relaxed);
                    Box::pin(async {})
                },
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(probes.load(Ordering::Relaxed), 2);
        assert_eq!(
            links_seen.load(Ordering::Relaxed),
            2,
            "the first probe's two links were emitted; the cancelled second probe's rows stayed unabsorbed, so no more emissions"
        );
    }

    /// Interface rows in the SNMP-collected shape: a numeric row id (the
    /// ifIndex), the name, and the description.
    fn snmp_interface(index: u32, name: Option<&str>, descr: Option<&str>) -> Interface {
        Interface {
            id: index.to_string(),
            name: name.map(str::to_owned),
            description: descr.map(str::to_owned),
            alias: None,
            kind: crate::identity::InterfaceKind::Ethernet,
            raw_type: None,
            mac: None,
            addresses: Vec::new(),
            admin_state: None,
            oper_state: None,
            vlan_id: None,
            parent_interface: None,
            provenance: vec![Source::SnmpInterfaces],
        }
    }

    /// The details the fallback collects: rows keyed by ifIndex.
    fn snmp_details(rows: &[Interface]) -> DeviceDetails {
        DeviceDetails {
            interfaces: rows.to_vec(),
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

    /// One neighbour whose local port starts as `port`.
    fn neighbour(port: Option<&str>) -> LldpNeighbour {
        LldpNeighbour {
            chassis_id: vec![0x00, 0x1f, 0x5e, 0xaa, 0xbb, 0xcc],
            address: None,
            remote_port_id: None,
            sys_name: None,
            sys_descr: None,
            local_port: port.map(str::to_owned),
        }
    }

    #[test]
    fn raw_numbers_join_by_index_and_descriptions_canonicalize_to_names() {
        let details = snmp_details(&[
            snmp_interface(2, Some("ether2"), Some("ether2")),
            snmp_interface(7, Some("br-mgmt"), Some("Main management bridge")),
        ]);
        let mut rows = [
            neighbour(Some("7")),                      // raw ifIndex match
            neighbour(Some("ether2")),                 // an ifName already: untouched
            neighbour(Some("Main management bridge")), // an ifDescr: canonicalizes
            neighbour(Some("9")),                      // no such index: stays raw
            neighbour(None),
        ];
        resolve_local_ports(&mut rows, &details);

        let ports: Vec<_> = rows.iter().map(|row| row.local_port.clone()).collect();
        assert_eq!(
            ports,
            [
                Some("br-mgmt".to_owned()),
                Some("ether2".to_owned()),
                Some("br-mgmt".to_owned()),
                Some("9".to_owned()),
                None,
            ]
        );
    }

    #[test]
    fn non_numeric_row_ids_never_join_and_names_win_over_descriptions() {
        // RouterOS API details: `.id` rows are not ifIndexes, so a raw
        // number stays raw; a local port naming an interface stays put.
        let details = snmp_details(&[Interface {
            id: "*A".to_owned(),
            name: Some("e2-fsw".to_owned()),
            description: None,
            ..snmp_interface(0, None, None)
        }]);
        let mut rows = [neighbour(Some("2")), neighbour(Some("e2-fsw"))];
        resolve_local_ports(&mut rows, &details);
        assert_eq!(rows[0].local_port.as_deref(), Some("2"));
        assert_eq!(rows[1].local_port.as_deref(), Some("e2-fsw"));
    }

    // --- the scan-policy gate ---

    /// A source whose every probe returns the same scripted neighbour
    /// rows, counting how many times it was asked — a blocked candidate
    /// is never asked.
    struct ScriptedNeighbours {
        rows: Vec<LldpNeighbour>,
        probes: Arc<AtomicUsize>,
    }

    impl NeighbourSource for ScriptedNeighbours {
        fn probe<'a>(
            &'a mut self,
            _address: &'a str,
            _snmp: &'a [CredentialCandidate<SnmpAccess>],
            _api: &'a [CredentialCandidate<ApiCredentials>],
        ) -> Pin<Box<dyn Future<Output = Probed> + Send + 'a>> {
            Box::pin(async move {
                self.probes.fetch_add(1, Ordering::Relaxed);
                Probed::Answers(Probe {
                    lldp: Some(self.rows.clone()),
                    api: None,
                    self_claims: Vec::new(),
                    details: None,
                    stations: None,
                })
            })
        }
    }

    /// One LLDP neighbour row with a distinctive chassis tail.
    fn scripted_neighbour(tail: u8, address: &str) -> LldpNeighbour {
        LldpNeighbour {
            chassis_id: vec![0x02, 0x1f, 0x5e, 0x00, 0x00, tail],
            address: Some(address.to_owned()),
            remote_port_id: Some("eth0".to_owned()),
            sys_name: Some(format!("node-{tail}")),
            sys_descr: None,
            local_port: None,
        }
    }

    fn scan_policy(allow: &[&str], ignore: &[&str]) -> ScanPolicy {
        let allow: Vec<String> = allow.iter().map(|rule| (*rule).to_owned()).collect();
        let ignore: Vec<String> = ignore.iter().map(|rule| (*rule).to_owned()).collect();
        ScanPolicy::new(&allow, &ignore).expect("fixture rules should parse")
    }

    #[tokio::test]
    async fn an_allowed_neighbour_is_probed_and_recorded() {
        let probes = Arc::new(AtomicUsize::new(0));
        let mut source = ScriptedNeighbours {
            rows: vec![scripted_neighbour(9, "10.0.0.9")],
            probes: probes.clone(),
        };
        let policy = scan_policy(&["10.0.0.0/8"], &[]);
        let report = walk_observing(
            "10.0.0.1",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_| NodeCredentials::none(),
            &mut source,
            AddressGate::new(Some(&policy)),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(
            probes.load(Ordering::Relaxed),
            2,
            "the seed and the allowed neighbour are both probed"
        );
        assert_eq!(report.refusals, Refusals::default());
        assert!(report.seed_refusal.is_none());
        assert!(
            report
                .nodes
                .iter()
                .any(|node| node.address.as_deref() == Some("10.0.0.9")),
            "the allowed neighbour is recorded"
        );
    }

    #[tokio::test]
    async fn an_ignored_neighbour_is_never_probed_recorded_or_linked() {
        let probes = Arc::new(AtomicUsize::new(0));
        let mut source = ScriptedNeighbours {
            rows: vec![scripted_neighbour(9, "10.0.0.9")],
            probes: probes.clone(),
        };
        let policy = scan_policy(&["10.0.0.0/8"], &["10.0.0.9/32"]);
        let report = walk_observing(
            "10.0.0.1",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_| NodeCredentials::none(),
            &mut source,
            AddressGate::new(Some(&policy)),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(probes.load(Ordering::Relaxed), 1, "only the seed is probed");
        assert_eq!(report.refusals.ignored, 1);
        assert_eq!(report.refusals.not_allowed, 0);
        assert_eq!(report.refusals.unparseable, 0);
        assert_eq!(
            report.nodes.len(),
            1,
            "the seed alone: the blocked neighbour leaves no node"
        );
        assert!(report.links.is_empty(), "no dangling link");
    }

    #[tokio::test]
    async fn an_out_of_allow_neighbour_is_refused_as_not_allowed() {
        let probes = Arc::new(AtomicUsize::new(0));
        let mut source = ScriptedNeighbours {
            rows: vec![scripted_neighbour(9, "10.0.1.9")],
            probes: probes.clone(),
        };
        let policy = scan_policy(&["10.0.0.0/24"], &[]);
        let report = walk_observing(
            "10.0.0.1",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_| NodeCredentials::none(),
            &mut source,
            AddressGate::new(Some(&policy)),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(probes.load(Ordering::Relaxed), 1);
        assert_eq!(report.refusals.not_allowed, 1);
        assert_eq!(report.refusals.ignored, 0);
    }

    #[tokio::test]
    async fn ignore_wins_over_allow_in_the_walk() {
        let probes = Arc::new(AtomicUsize::new(0));
        let mut source = ScriptedNeighbours {
            rows: vec![scripted_neighbour(9, "10.0.0.9")],
            probes: probes.clone(),
        };
        let policy = scan_policy(&["10.0.0.0/8"], &["10.0.0.9/32"]);
        // Sanity: without the ignore the address is inside the allow.
        let allow_wide = scan_policy(&["10.0.0.0/8"], &[]);
        assert!(matches!(
            allow_wide.decide("10.0.0.9".parse().unwrap()),
            ScanDecision::Allowed { .. }
        ));
        let report = walk_observing(
            "10.0.0.1",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_| NodeCredentials::none(),
            &mut source,
            AddressGate::new(Some(&policy)),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(report.refusals.ignored, 1);
    }

    #[test]
    fn lease_addresses_are_gated_before_the_details_land() {
        let policy = scan_policy(&["10.0.0.0/8"], &["10.9.9.9/32"]);
        let mut state = WalkState::new(
            "10.0.0.1",
            "router",
            NodeCredentials::none(),
            WalkBounds::default(),
            AddressGate::new(Some(&policy)),
        );
        let lease = |address: Option<&str>, hostname: &str| crate::identity::DhcpLease {
            address: address.map(str::to_owned),
            mac: crate::identity::Mac::parse("02:00:00:aa:00:20"),
            hostname: Some(hostname.to_owned()),
            status: Some("bound".to_owned()),
            dynamic: Some(true),
            disabled: Some(false),
        };
        let mut details = routeros_details(&[]);
        details.dhcp_leases = Some(crate::identity::TableSnapshot {
            rows: vec![
                // A v4-mapped literal is canonicalized on admission.
                lease(Some("::ffff:10.0.0.5"), "printer"),
                lease(Some("10.9.9.9"), "ignored-tv"),
                lease(Some("192.168.1.5"), "not-allowed"),
                lease(Some("not-an-address"), "unparseable"),
                lease(None, "address-less"),
            ],
            provenance: vec![Source::RouterOsDhcpLease],
            observed_at: SystemTime::UNIX_EPOCH,
        });

        state.gate_lease_addresses(&mut details);

        let table = details.dhcp_leases.expect("the lease table stays");
        assert_eq!(table.rows[0].address.as_deref(), Some("10.0.0.5"));
        assert_eq!(
            table.rows[0].hostname.as_deref(),
            Some("printer"),
            "an admitted row keeps its hostname"
        );
        assert_eq!(table.rows[1].address, None);
        assert_eq!(
            table.rows[1].hostname.as_deref(),
            Some("ignored-tv"),
            "a refused row keeps its hostname"
        );
        assert_eq!(table.rows[2].address, None);
        assert_eq!(table.rows[3].address, None);
        assert_eq!(
            table.rows[4].address, None,
            "a row with no address is untouched"
        );
        assert_eq!(state.refusals.ignored, 1);
        assert_eq!(state.refusals.not_allowed, 1);
        assert_eq!(state.refusals.unparseable, 1);
    }

    #[test]
    fn an_unrestricted_gate_canonicalizes_and_drops_non_ip_lease_addresses() {
        let mut state = WalkState::new(
            "10.0.0.1",
            "router",
            NodeCredentials::none(),
            WalkBounds::default(),
            AddressGate::new(None),
        );
        let lease = |address: &str, hostname: &str| crate::identity::DhcpLease {
            address: Some(address.to_owned()),
            mac: crate::identity::Mac::parse("02:00:00:aa:00:20"),
            hostname: Some(hostname.to_owned()),
            status: Some("bound".to_owned()),
            dynamic: Some(true),
            disabled: Some(false),
        };
        let mut details = routeros_details(&[]);
        details.dhcp_leases = Some(crate::identity::TableSnapshot {
            rows: vec![
                // With no policy the gate admits verbatim; the walk
                // canonicalizes first so the row stores the IPv4 form.
                lease(" ::ffff:10.0.0.5 ", "printer"),
                lease("not-an-address", "unparseable"),
            ],
            provenance: vec![Source::RouterOsDhcpLease],
            observed_at: SystemTime::UNIX_EPOCH,
        });

        state.gate_lease_addresses(&mut details);

        let table = details.dhcp_leases.expect("the lease table stays");
        assert_eq!(table.rows[0].address.as_deref(), Some("10.0.0.5"));
        assert_eq!(
            table.rows[0].hostname.as_deref(),
            Some("printer"),
            "an admitted row keeps its hostname"
        );
        assert_eq!(table.rows[1].address, None);
        assert_eq!(
            table.rows[1].hostname.as_deref(),
            Some("unparseable"),
            "a non-IP row keeps its hostname and loses only the address"
        );
        assert_eq!(state.refusals.unparseable, 1);
        assert_eq!(state.refusals.ignored, 0);
        assert_eq!(state.refusals.not_allowed, 0);
    }

    #[tokio::test]
    async fn ipv6_rules_apply_end_to_end() {
        // fd00::1 is inside the ULA range; 2001:db8::1 is public.
        let probes = Arc::new(AtomicUsize::new(0));
        let mut source = ScriptedNeighbours {
            rows: vec![
                scripted_neighbour(1, "fd00::1"),
                scripted_neighbour(2, "2001:db8::1"),
            ],
            probes: probes.clone(),
        };
        let policy = scan_policy(&["fc00::/7"], &[]);
        let report = walk_observing(
            "fd00::fe",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_| NodeCredentials::none(),
            &mut source,
            AddressGate::new(Some(&policy)),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(
            report.refusals.not_allowed, 2,
            "the public neighbour is refused on every sighting"
        );
        assert!(
            report
                .nodes
                .iter()
                .any(|node| node.address.as_deref() == Some("fd00::1")),
            "the ULA neighbour is recorded"
        );
        assert!(
            !report
                .nodes
                .iter()
                .any(|node| node.address.as_deref() == Some("2001:db8::1"))
        );
    }

    #[tokio::test]
    async fn a_blocked_seed_refuses_before_any_probe() {
        let probes = Arc::new(AtomicUsize::new(0));
        let mut source = ScriptedNeighbours {
            rows: vec![scripted_neighbour(9, "10.0.0.9")],
            probes: probes.clone(),
        };
        // The seed itself is ignored.
        let policy = scan_policy(&["10.0.0.0/8"], &["10.0.0.1/32"]);
        let report = walk_observing(
            "10.0.0.1",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_| NodeCredentials::none(),
            &mut source,
            AddressGate::new(Some(&policy)),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(probes.load(Ordering::Relaxed), 0, "zero probes");
        assert!(report.nodes.is_empty(), "no node is created");
        assert_eq!(
            report.seed_refusal,
            Some(SeedRefusal::Ignored {
                address: "10.0.0.1".to_owned(),
            })
        );

        // An unparseable seed refuses the same way.
        let report = walk_observing(
            "router-gateway",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_| NodeCredentials::none(),
            &mut source,
            AddressGate::new(Some(&policy)),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(probes.load(Ordering::Relaxed), 0, "still zero probes");
        assert_eq!(
            report.seed_refusal,
            Some(SeedRefusal::Unparseable {
                address: "router-gateway".to_owned(),
                reason: "not an IPv4 or IPv6 address".to_owned(),
            })
        );
    }

    #[tokio::test]
    async fn the_seed_address_is_stored_canonicalized() {
        let probes = Arc::new(AtomicUsize::new(0));
        let mut source = ScriptedNeighbours {
            rows: vec![scripted_neighbour(9, "10.0.0.9")],
            probes: probes.clone(),
        };
        let mut trailing_space = " 10.0.0.1 ".to_owned();
        trailing_space.insert(0, ' ');
        let report = walk_observing(
            " 10.0.0.1 ",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_| NodeCredentials::none(),
            &mut source,
            AddressGate::new(Some(&scan_policy(&["10.0.0.0/8"], &[]))),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert_eq!(
            report.nodes[0].address.as_deref(),
            Some("10.0.0.1"),
            "the seed node carries the canonical probe text"
        );
    }

    #[tokio::test]
    async fn a_mac_only_neighbour_is_unaffected_by_the_policy() {
        let probes = Arc::new(AtomicUsize::new(0));
        let mut source = ScriptedNeighbours {
            rows: vec![LldpNeighbour {
                chassis_id: vec![0x02, 0x1f, 0x5e, 0x00, 0x00, 0x11],
                address: None,
                remote_port_id: Some("eth0".to_owned()),
                sys_name: Some("mac-only".to_owned()),
                sys_descr: None,
                local_port: None,
            }],
            probes: probes.clone(),
        };
        let policy = scan_policy(&["192.168.0.0/16"], &[]);
        let report = walk_observing(
            "192.168.100.1",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_| NodeCredentials::none(),
            &mut source,
            AddressGate::new(Some(&policy)),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        assert!(
            report
                .nodes
                .iter()
                .any(|node| node.mac.as_deref() == Some("02:1f:5e:00:00:11")),
            "the MAC-only node is recorded"
        );
        assert_eq!(report.refusals, Refusals::default());
    }

    #[tokio::test]
    async fn a_blocked_station_address_is_omitted_and_counted() {
        use crate::snmp::AirStation;
        struct StationSource {
            stations: Vec<AirStation>,
        }
        impl NeighbourSource for StationSource {
            fn probe<'a>(
                &'a mut self,
                _address: &'a str,
                _snmp: &'a [CredentialCandidate<SnmpAccess>],
                _api: &'a [CredentialCandidate<ApiCredentials>],
            ) -> Pin<Box<dyn Future<Output = Probed> + Send + 'a>> {
                let stations = self.stations.clone();
                Box::pin(async move {
                    Probed::Answers(Probe {
                        lldp: None,
                        api: None,
                        self_claims: Vec::new(),
                        details: None,
                        stations: Some(stations),
                    })
                })
            }
        }
        let station = |tail: u8, last_ip: &str| AirStation {
            mac: [0x02, 0x00, 0x00, 0xaa, 0x00, tail],
            wlan_if: 1,
            hostname: None,
            signal_dbm: Some(-50),
            noise_dbm: None,
            last_ip: Some(last_ip.to_owned()),
            tx_bps: None,
            rx_bps: None,
            rx_bytes: None,
            tx_bytes: None,
            uptime: None,
            distance_ccq: None,
            quality: None,
            channel: None,
            ul_rssi: None,
            dl_rssi: None,
            ul_snr: None,
            dl_snr: None,
            tx_capacity: None,
            rx_capacity: None,
            tx_quality: None,
            dl_rate_mbps: None,
            model: None,
            software_version: None,
        };
        // One station inside the allow list, one outside it.
        let mut source = StationSource {
            stations: vec![station(1, "10.0.0.20"), station(2, "10.9.9.9")],
        };
        let policy = scan_policy(&["10.0.0.0/24"], &[]);
        let report = walk_observing(
            "10.0.0.1",
            "seed",
            NodeCredentials::none(),
            WalkBounds::default(),
            &|_| NodeCredentials::none(),
            &mut source,
            AddressGate::new(Some(&policy)),
            WalkHooks {
                cancelled: &|| false,
                observer: &mut |_| Box::pin(async {}),
                link: &mut |_| Box::pin(async {}),
                progress: &mut |_| Box::pin(async {}),
            },
        )
        .await;
        // The blocked station's address claim is omitted; the MAC-only
        // association leaf stands, and the refusal is counted.
        let blocked = report
            .nodes
            .iter()
            .find(|node| node.mac.as_deref() == Some("02:00:00:aa:00:02"))
            .expect("the blocked station still folds as a leaf");
        assert!(
            !blocked
                .claims
                .iter()
                .any(|claim| claim.kind == ClaimKind::Address),
            "the blocked address claim is omitted"
        );
        assert!(
            !report
                .nodes
                .iter()
                .any(|node| node.address.as_deref() == Some("10.9.9.9")),
            "a blocked IP is never recorded"
        );
        assert_eq!(report.refusals.not_allowed, 1);
    }

    /// A probed RouterOS device's details with one `/ip/address` row per
    /// entry: (interface name, address text). Each row names its own
    /// interface, so the fold keeps them all.
    fn routeros_details(rows: &[(&str, &str)]) -> DeviceDetails {
        let interfaces: Vec<crate::mikrotik::RouterOsInterface> = rows
            .iter()
            .enumerate()
            .map(|(index, (name, _))| crate::mikrotik::RouterOsInterface {
                id: format!("*{}", index + 1),
                name: Some((*name).to_owned()),
                raw_type: Some("ether".to_owned()),
                mac_address: None,
                running: Some(true),
                disabled: Some(false),
            })
            .collect();
        let addresses: Vec<crate::mikrotik::RouterOsAddress> = rows
            .iter()
            .map(|(interface, address)| crate::mikrotik::RouterOsAddress {
                address: Some((*address).to_owned()),
                network: None,
                interface: Some((*interface).to_owned()),
            })
            .collect();
        DeviceDetails::from_router_os(&interfaces, &addresses)
    }

    /// A probed SNMP device's details with one `ipAddrTable` address per
    /// entry: (interface name, address text). The interface rows carry the
    /// address table's provenance, as the engine's attachment produces it.
    fn snmp_address_details(rows: &[(&str, &str)]) -> DeviceDetails {
        let interfaces: Vec<Interface> = rows
            .iter()
            .enumerate()
            .map(|(index, (name, _))| {
                snmp_interface(
                    u32::try_from(index + 1).expect("the fixture has few rows"),
                    Some(name),
                    None,
                )
            })
            .collect();
        let mut details = snmp_details(&interfaces);
        for (row, (_, address)) in details.interfaces.iter_mut().zip(rows) {
            row.addresses.push((*address).to_owned());
            row.provenance.push(Source::SnmpIpAddresses);
        }
        details
    }

    /// A walk state seeded at `10.0.0.1`, over an unrestricted gate: the
    /// fixture for the node-folding tests.
    fn seeded_state() -> WalkState<'static> {
        WalkState::new(
            "10.0.0.1",
            "router",
            NodeCredentials::none(),
            WalkBounds::default(),
            AddressGate::new(None),
        )
    }

    #[test]
    fn own_address_rows_promote_bare_deduplicated_and_in_device_order() {
        let mut state = seeded_state();
        // The node arrived with a neighbour-reported address: it stands.
        state.nodes[0].address = Some("10.0.0.1".to_owned());
        let details = routeros_details(&[
            ("ether1", "192.168.88.1/24"),
            ("loopback", "10.9.9.9/32"),
            ("ether2", "192.168.88.1/30"),
        ]);

        state.promote_own_addresses(0, &details);

        assert_eq!(
            state.nodes[0]
                .claims
                .iter()
                .map(|claim| (claim.kind, claim.value.as_str(), claim.source))
                .collect::<Vec<_>>(),
            vec![
                (
                    ClaimKind::Address,
                    "192.168.88.1",
                    Source::RouterOsAddresses
                ),
                (ClaimKind::Address, "10.9.9.9", Source::RouterOsAddresses),
            ],
            "the prefix is stripped, one address on two interfaces is one claim, and the device order holds"
        );
        assert_eq!(
            state.nodes[0].address.as_deref(),
            Some("10.0.0.1"),
            "the own addresses never replace a neighbour-reported address"
        );
        // A second promotion of the same evidence adds nothing.
        state.promote_own_addresses(0, &details);
        assert_eq!(state.nodes[0].claims.len(), 2);
    }

    #[test]
    fn the_first_own_address_fills_a_missing_node_address_only() {
        let mut state = seeded_state();
        // A node the walk reached without an address of its own.
        state.nodes[0].address = None;

        state.promote_own_addresses(0, &routeros_details(&[("ether1", "192.168.88.1/24")]));
        assert_eq!(
            state.nodes[0].address.as_deref(),
            Some("192.168.88.1"),
            "the first own address fills an address-less node"
        );

        // Once the node has an address, later own addresses leave it
        // alone.
        state.promote_own_addresses(0, &routeros_details(&[("ether1", "172.16.0.1/24")]));
        assert_eq!(state.nodes[0].address.as_deref(), Some("192.168.88.1"));
    }

    #[test]
    fn snmp_address_rows_promote_after_routeros_ones_and_shared_addresses_keep_routeros() {
        let mut details = snmp_address_details(&[
            ("eth1", "10.0.0.2/24"),
            ("eth2", "172.16.99.6/30"),
            ("eth3", "192.168.88.1/30"), // also stated by the RouterOS row below
            ("eth4", "10.0.0.2/30"),     // one address, two interfaces, one fact
        ]);
        details
            .interfaces
            .extend(routeros_details(&[("ether1", "192.168.88.1/24")]).interfaces);
        let mut state = seeded_state();
        state.nodes[0].address = None;

        state.promote_own_addresses(0, &details);

        assert_eq!(
            state.nodes[0]
                .claims
                .iter()
                .map(|claim| (claim.kind, claim.value.as_str(), claim.source))
                .collect::<Vec<_>>(),
            vec![
                (
                    ClaimKind::Address,
                    "192.168.88.1",
                    Source::RouterOsAddresses
                ),
                (ClaimKind::Address, "10.0.0.2", Source::SnmpIpAddresses),
                (ClaimKind::Address, "172.16.99.6", Source::SnmpIpAddresses),
            ],
            "RouterOS addresses lead in device order, SNMP-only addresses follow in device order, and a shared address keeps the RouterOS claim"
        );
        assert_eq!(
            state.nodes[0].address.as_deref(),
            Some("192.168.88.1"),
            "a RouterOS address fills the node before any SNMP-only one"
        );
        state.promote_own_addresses(0, &details);
        assert_eq!(
            state.nodes[0].claims.len(),
            3,
            "repeated promotion adds nothing"
        );
    }

    #[test]
    fn an_snmp_only_node_fills_its_address_from_the_first_snmp_address() {
        let details = snmp_address_details(&[("eth7", "172.16.99.6/30"), ("eth8", "10.0.0.2/24")]);
        let mut state = seeded_state();
        state.nodes[0].address = None;

        state.promote_own_addresses(0, &details);

        assert_eq!(
            state.nodes[0].address.as_deref(),
            Some("172.16.99.6"),
            "an SNMP-only node's first address in device order fills it"
        );
        assert_eq!(
            state.nodes[0]
                .claims
                .iter()
                .map(|claim| (claim.value.as_str(), claim.source))
                .collect::<Vec<_>>(),
            vec![
                ("172.16.99.6", Source::SnmpIpAddresses),
                ("10.0.0.2", Source::SnmpIpAddresses),
            ],
            "an SNMP-only node's addresses stay in device order"
        );
    }

    // --- the seed's own MAC ---

    #[test]
    fn an_adopted_seed_projects_every_details_table_into_the_stored_host() {
        let observed_at = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
        let mut details = routeros_details(&[("ether1", "10.0.0.1/24")]);
        details.interfaces[0].mac = crate::identity::Mac::parse("02:00:00:aa:00:01");
        details.system = Some(crate::identity::SystemDetails {
            version: Some("7.15".to_owned()),
            board_name: Some("RB5009".to_owned()),
            platform: None,
            uptime: Some("1w".to_owned()),
            provenance: vec![Source::RouterOsSystem],
        });
        details.arp = Some(crate::identity::TableSnapshot {
            rows: vec![crate::identity::ArpEntry {
                address: Some("10.0.0.2".to_owned()),
                mac_address: crate::identity::Mac::parse("02:00:00:aa:00:02"),
                interface: Some("ether1".to_owned()),
                dynamic: Some(true),
                complete: Some(true),
                disabled: None,
            }],
            provenance: vec![Source::RouterOsArp],
            observed_at,
        });
        details.bridge_ports = Some(crate::identity::TableSnapshot {
            rows: vec![crate::identity::BridgePort {
                bridge: Some("br1".to_owned()),
                interface: Some("ether1".to_owned()),
                pvid: Some(1),
                frame_types: None,
                status: Some("in-bridge".to_owned()),
                inactive: Some(false),
                disabled: Some(false),
                dynamic: Some(false),
            }],
            provenance: vec![Source::RouterOsBridgePorts],
            observed_at,
        });
        details.bridge_vlans = Some(crate::identity::TableSnapshot {
            rows: vec![crate::identity::BridgeVlan {
                bridge: Some("br1".to_owned()),
                vlan_ids: vec![1],
                tagged: Vec::new(),
                untagged: vec!["ether1".to_owned()],
                current_tagged: Vec::new(),
                current_untagged: vec!["ether1".to_owned()],
            }],
            provenance: vec![Source::RouterOsBridgeVlans],
            observed_at,
        });
        details.bridge_hosts = Some(crate::identity::TableSnapshot {
            rows: vec![crate::identity::BridgeHost {
                bridge: Some("br1".to_owned()),
                on_interface: Some("ether1".to_owned()),
                mac_address: crate::identity::Mac::parse("02:00:00:aa:00:03"),
                age: Some("1m".to_owned()),
                dynamic: Some(true),
                local: Some(false),
                external: None,
            }],
            provenance: vec![Source::RouterOsBridgeHosts],
            observed_at,
        });

        let mut state = seeded_state();
        state.nodes[0].details = Some(details);
        state.adopt_owning_interface_mac(0);
        assert_eq!(state.nodes[0].mac.as_deref(), Some("02:00:00:aa:00:01"));
        assert!(state.visited.contains("02:00:00:aa:00:01"));

        let stored = crate::stored::StoredHost::from_node(&state.nodes[0], SystemTime::UNIX_EPOCH)
            .expect("the adopted seed projects as a host");
        assert_eq!(stored.mac, "02:00:00:aa:00:01");
        let details = stored.details.expect("every details slot persists");
        assert_eq!(details.interfaces.len(), 1);
        assert_eq!(details.interfaces[0].addresses.as_slice(), ["10.0.0.1/24"]);
        assert!(details.system.is_some());
        assert_eq!(details.arp.expect("arp").rows.len(), 1);
        assert_eq!(details.bridge_ports.expect("bridge ports").rows.len(), 1);
        assert_eq!(details.bridge_vlans.expect("bridge vlans").rows.len(), 1);
        assert_eq!(details.bridge_hosts.expect("bridge hosts").rows.len(), 1);
    }

    #[test]
    fn a_known_mac_is_never_replaced_by_an_owning_interface() {
        let mut state = seeded_state();
        state.nodes[0].mac = Some("02:00:00:00:00:99".to_owned());
        state.visited.insert("02:00:00:00:00:99".to_owned());
        let mut details = routeros_details(&[("ether1", "10.0.0.1/24")]);
        details.interfaces[0].mac = crate::identity::Mac::parse("02:00:00:aa:00:01");
        state.nodes[0].details = Some(details);

        state.adopt_owning_interface_mac(0);

        assert_eq!(
            state.nodes[0].mac.as_deref(),
            Some("02:00:00:00:00:99"),
            "a MAC the walk already knows is never replaced"
        );
        assert!(!state.visited.contains("02:00:00:aa:00:01"));
    }

    #[test]
    fn an_owning_interface_without_a_name_or_mac_is_never_chosen() {
        // The interface that owns the probed address reports no MAC: an
        // address alone states no MAC, so the seed stays MAC-less.
        let mut without_mac = seeded_state();
        without_mac.nodes[0].details = Some(routeros_details(&[("ether1", "10.0.0.1/24")]));
        without_mac.adopt_owning_interface_mac(0);
        assert_eq!(
            without_mac.nodes[0].mac, None,
            "a MAC-less owner is not adopted"
        );
        assert!(
            without_mac.visited.is_empty(),
            "nothing was claimed as visited"
        );

        // The same row with a MAC but no interface name is equally
        // unchosen: the name is what makes the interface the owner.
        let mut without_name = seeded_state();
        let mut details = routeros_details(&[("ether1", "10.0.0.1/24")]);
        details.interfaces[0].name = None;
        details.interfaces[0].mac = crate::identity::Mac::parse("02:00:00:aa:00:02");
        without_name.nodes[0].details = Some(details);
        without_name.adopt_owning_interface_mac(0);
        assert_eq!(
            without_name.nodes[0].mac, None,
            "a nameless owner is not adopted"
        );

        // A MAC-less owner beside a complete sibling: the complete
        // interface owns the address and is adopted, whatever its name
        // order.
        let mut sibling = seeded_state();
        let mut details = routeros_details(&[
            ("aaa-incomplete", "10.0.0.1/24"),
            ("zzz-complete", "10.0.0.1/24"),
        ]);
        details.interfaces[1].mac = crate::identity::Mac::parse("02:00:00:aa:00:03");
        sibling.nodes[0].details = Some(details);
        sibling.adopt_owning_interface_mac(0);
        assert_eq!(
            sibling.nodes[0].mac.as_deref(),
            Some("02:00:00:aa:00:03"),
            "the complete sibling wins over an incomplete owner"
        );
        assert!(sibling.visited.contains("02:00:00:aa:00:03"));
    }

    #[test]
    fn a_candidate_mac_the_visited_set_already_holds_is_never_adopted() {
        // Another node already answers to the MAC the seed's own interface
        // reports: adopting it would give two nodes one identity.
        let mut state = seeded_state();
        state.visited.insert("02:00:00:aa:00:01".to_owned());
        let mut details = routeros_details(&[("ether1", "10.0.0.1/24")]);
        details.interfaces[0].mac = crate::identity::Mac::parse("02:00:00:aa:00:01");
        state.nodes[0].details = Some(details);

        state.adopt_owning_interface_mac(0);

        assert_eq!(
            state.nodes[0].mac, None,
            "a MAC the visited set already holds is not adopted"
        );
        assert!(
            state.visited.contains("02:00:00:aa:00:01"),
            "the existing owner keeps the MAC"
        );
    }
}

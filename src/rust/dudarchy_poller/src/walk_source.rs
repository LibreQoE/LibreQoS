//! The live [`NeighbourSource`]: the paced, cancellable probe that gathers
//! one node's neighbour tables, self-claims, and interface details over
//! the wire.
//!
//! Every transport call and every pacing wait lives here — this module is
//! the walker's single I/O surface. The walk core in [`crate::walk`] stays
//! pure over the [`NeighbourSource`] trait: probes are awaitable, pacing
//! waits on Tokio time (so a dropped or aborted task ends them at once),
//! and the cancellation flag is checked before every feed and after every
//! wait. A probe that reaches its end with the flag risen reports
//! [`Probed::Cancelled`]; a flag rising mid-feed abandons the remaining
//! feeds and reports the no-answer probe — exactly what the synchronous
//! source did.
//!
//! The SNMP surfaces — system, LLDP, interfaces, ARP, FDB — and the API
//! surfaces — identity, serial, neighbours, interfaces, addresses, ARP, the
//! bridge tables, resource, health — are run by the adapter document engine:
//! each credential authenticates with a read, the registry selects one
//! document, and the engine executes that document's probes for that
//! surface through the reader for its transport. The SNMP observation is the
//! details fallback; the API observation wins. This module keeps no OID
//! table, column map, or command field map of its own.
//!
//! A node's credential set is a candidate list per protocol. The probe
//! tries the candidates in resolution order and keeps the first one that
//! authenticates: the system read itself is the attempt, so a credential
//! that wins pays for no extra query, an API login trap is a rejection, and
//! a post-login command refusal is still an authenticated node. An SNMP
//! candidate's version is a preference: a `v2c` candidate whose system read
//! is unanswered or carries no usable system data is retried once as v1
//! with the same community, and the version that answered serves every
//! document query. Each attempt's outcome is logged at `debug` with the node
//! address, the protocol, and the candidate's non-secret origin label; each
//! wire-version system read is logged separately, and the per-walk tallies
//! are what the caller logs once the walk ends. Losers are not retried
//! within the node, and nothing is blacklisted globally. The winner is
//! remembered as the walk's preferred candidate for that protocol and tried
//! first on later nodes, but only when that node's own resolution order
//! carries it — target refs stay ahead of every profile.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use crate::adapters::CommandName;
use crate::adapters::Document;
use crate::adapters::engine::{self, ApiSelection, Selection};
use crate::adapters::live::{ApiReader, SnmpReader};
use crate::adapters::reader::{CommandRow, Outcome, Reader, Varbind};
use crate::adapters::registry::Registry;
use crate::identity::{Claim, DeviceDetails, Source};
use crate::mikrotik::{MIKROTIK_TIMEOUT, MikrotikError, command};
use crate::profiles::Protocol;
use crate::snmp::{LldpNeighbour, SNMP_TIMEOUT, SnmpError, SnmpVersion, SystemInfo, system_info};
use crate::snmp_ber::{SYS_DESCR, SYS_NAME, SYS_OBJECT_ID};
use crate::snmp_tables::WalkValue;
use crate::walk::{
    ApiCredentials, CredentialCandidate, CredentialOrigin, NeighbourSource, Probe, Probed,
    SnmpAccess, attempt_order, resolve_local_ports,
};

/// How often the probe's cancellation race re-checks the flag. A plain
/// atomic needs polling; 20 ms bounds the cancellation latency far below
/// any probe deadline.
pub(crate) const CANCEL_POLL: Duration = Duration::from_millis(20);

/// A probe with no answers: what a cancelled source reports for the feeds
/// it did not reach.
fn no_probe() -> Probe {
    Probe {
        lldp: None,
        api: None,
        self_claims: Vec::new(),
        details: None,
        stations: None,
    }
}

/// The live [`NeighbourSource`]: each protocol's selected document surface
/// over its transport — SNMP over the candidate's preferred version, with
/// one v1 retry when a v2c system read yields no usable data, and the
/// RouterOS API over the binary client — with each candidate's first real
/// request preceded by the configured delay. The first candidate that
/// authenticates serves every query of the node; a node whose candidates all
/// lose is a leaf, per the walk contract. The document's own probes state
/// the node's claims: the SNMP system group and LLDP rows, and the API
/// identity and routerboard serial.
///
/// A source built with [`NetworkSource::paced_by`] also carries a
/// cancellation flag: every pacing wait and every feed checks it, and a
/// cancelled source stops where it is and reports [`Probed::Cancelled`]
/// with what it reached.
pub struct NetworkSource {
    io: PacedIo,
    progress: CredentialProgress,
    registry: Registry,
}

impl NetworkSource {
    /// A source that waits `delay` before each candidate attempt and
    /// selects device documents from `registry`.
    pub fn new(delay: Duration, registry: Registry) -> Self {
        Self {
            io: PacedIo {
                delay,
                cancelled: None,
                snmp_timeout: SNMP_TIMEOUT,
            },
            progress: CredentialProgress::default(),
            registry,
        }
    }

    /// A source that additionally aborts its remaining feeds once
    /// `cancelled` is set — the shape the server's discovery jobs use so
    /// a shutdown does not wait out the pacing of an abandoned walk.
    pub fn paced_by(delay: Duration, cancelled: Arc<AtomicBool>, registry: Registry) -> Self {
        Self {
            io: PacedIo {
                delay,
                cancelled: Some(cancelled),
                snmp_timeout: SNMP_TIMEOUT,
            },
            progress: CredentialProgress::default(),
            registry,
        }
    }

    /// The walk's credential attempt tallies, per protocol — what the
    /// discovery job logs once when the walk ends.
    #[cfg(test)]
    pub(crate) fn credential_outcomes(&self) -> CredentialOutcomes {
        self.progress.outcomes
    }
}

/// The probe's paced, cancellable I/O surface: every query of one probe goes
/// through here, each real request preceded by the configured delay and
/// every wait bounded by the cancellation flag. Split from
/// [`NetworkSource`]'s credential progress so one probe can hold the I/O
/// borrow while it mutates the tallies and the preference.
struct PacedIo {
    delay: Duration,
    cancelled: Option<Arc<AtomicBool>>,
    /// The deadline for one SNMP exchange: the system read and every
    /// document probe. [`SNMP_TIMEOUT`] in production; the negotiation
    /// tests in the child module shorten it through this private field.
    /// A test seam only, not public API.
    snmp_timeout: Duration,
}

impl PacedIo {
    fn cancelled(&self) -> bool {
        self.cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    /// Waits out the configured pacing delay on Tokio time. The probe is
    /// raced against the flag as a whole (see the probe impl), so a rise
    /// during a pacing wait ends the probe within one poll interval.
    async fn pace(&self) {
        tokio::time::sleep(self.delay).await;
    }
}

/// Completes when the cancellation flag rises, polled at a short interval:
/// the flag is a plain atomic, and a 20 ms bound on noticing it is orders
/// of magnitude below any ping or SNMP deadline. A source without a flag
/// never completes it.
async fn wait_cancelled(flag: Option<Arc<AtomicBool>>) {
    let Some(flag) = flag else {
        return std::future::pending().await;
    };
    let mut tick = tokio::time::interval(CANCEL_POLL);
    tick.tick().await; // the first tick fires at once; skip it
    loop {
        tick.tick().await;
        if flag.load(Ordering::Relaxed) {
            return;
        }
    }
}

/// One credential attempt's result, before the walk tallies it.
enum Attempted<T> {
    /// The credential authenticated the node; the payload is what the
    /// attempt's own query returned, when it returned anything.
    Answered(T),
    /// The node refused this credential.
    Rejected,
    /// The attempt says nothing about the credential: a timeout, a socket
    /// failure, or a malformed answer.
    Inconclusive,
    /// The node answered with a protocol negotiation this client does not
    /// speak. Another credential cannot change that.
    Unsupported,
    /// The walk was cancelled mid-attempt.
    Cancelled,
}

/// One system read at one wire version, classified for version negotiation
/// and logging. Unlike [`Attempted`], this separates an answer that carried
/// usable system values from one that authenticated without them.
enum SystemRead {
    /// The agent answered with the system data discovery needs.
    Usable(SystemInfo),
    /// The agent answered, but the reply carried no usable system data.
    Unusable,
    /// The agent answered for another community.
    Rejected,
    /// A timeout, socket failure, or malformed answer said nothing about the
    /// community.
    Inconclusive,
    /// The walk was cancelled.
    Cancelled,
}

/// The result of negotiating one SNMP candidate's wire version: the
/// effective access for every later query, with the winning system values
/// when the read produced usable ones.
enum SnmpNegotiation {
    /// The community authenticated. `system` is `None` when no version
    /// produced usable system data; the credential still counts as answered.
    Answered {
        access: SnmpAccess,
        system: Option<SystemInfo>,
    },
    /// The node refused the community; no version changed that.
    Rejected,
    /// No version established anything about the community.
    Inconclusive,
    /// The walk was cancelled during the negotiation.
    Cancelled,
}

/// One attempt outcome the walk tallies and logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateOutcome {
    /// The credential authenticated the node.
    Answered,
    /// The node refused the credential.
    Rejected,
    /// The attempt said nothing about the credential.
    Inconclusive,
    /// The node does not speak the protocol this client does.
    Unsupported,
}

impl CandidateOutcome {
    /// The distinct outcome word the per-candidate log carries.
    fn word(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Rejected => "rejected",
            Self::Inconclusive => "inconclusive",
            Self::Unsupported => "unsupported",
        }
    }
}

/// One protocol's authentication result for one node.
enum AuthResult<'c, T, P> {
    /// This candidate authenticated the node; it serves every later query
    /// and is remembered as the walk's preference for its protocol.
    Winner(&'c CredentialCandidate<T>, P),
    /// No candidate authenticated the node.
    Nothing,
    /// The walk was cancelled mid-fan-out: nothing was decided, tallied, or
    /// preferred.
    Cancelled,
}

/// Authenticates one node over one protocol: puts the candidates in attempt
/// order (honouring the walk's preference within the profiles), tries each
/// once, and records every completed attempt as it returns — so a
/// cancellation mid-fan-out keeps the attempts that already answered in the
/// tallies and the log. A rejected or inconclusive candidate falls through
/// to the next one; an unsupported negotiation ends the fan-out; the winner
/// becomes the walk's preference and its payload is handed back, so the
/// caller reuses that answer instead of querying again.
async fn authenticate<'c, T, P, A, F>(
    progress: &mut CredentialProgress,
    address: &str,
    protocol: Protocol,
    candidates: &'c [CredentialCandidate<T>],
    mut attempt: A,
) -> AuthResult<'c, T, P>
where
    A: FnMut(&'c CredentialCandidate<T>) -> F,
    F: Future<Output = Attempted<P>>,
{
    let preferred = progress.preferred(protocol).cloned();
    let order = attempt_order(candidates, preferred.as_ref());
    for candidate in order {
        let outcome = match attempt(candidate).await {
            Attempted::Answered(payload) => {
                record_attempt(
                    progress,
                    address,
                    protocol,
                    candidate,
                    CandidateOutcome::Answered,
                );
                progress.prefer(protocol, &candidate.origin);
                return AuthResult::Winner(candidate, payload);
            }
            // A cancelled attempt says nothing about the credential: it is
            // neither logged nor tallied, and the attempts that already
            // completed stay recorded.
            Attempted::Cancelled => return AuthResult::Cancelled,
            Attempted::Rejected => CandidateOutcome::Rejected,
            Attempted::Inconclusive => CandidateOutcome::Inconclusive,
            Attempted::Unsupported => CandidateOutcome::Unsupported,
        };
        record_attempt(progress, address, protocol, candidate, outcome);
        if outcome == CandidateOutcome::Unsupported {
            // The node does not speak the protocol: no credential will help,
            // so this is the node's last candidate attempt.
            return AuthResult::Nothing;
        }
    }
    AuthResult::Nothing
}

/// Logs and tallies one completed attempt.
fn record_attempt<T>(
    progress: &mut CredentialProgress,
    address: &str,
    protocol: Protocol,
    candidate: &CredentialCandidate<T>,
    outcome: CandidateOutcome,
) {
    log_attempt(address, protocol, &candidate.origin, outcome);
    progress.outcomes.tally(protocol, outcome);
}

/// The protocol's log word.
fn protocol_word(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Snmp => "snmp",
        Protocol::Api => "api",
    }
}

/// Logs one candidate attempt: the node address, the protocol, the
/// candidate's non-secret origin label, and the distinct outcome word.
/// Failed credentials are normal operation, so this is `debug`.
fn log_attempt(
    address: &str,
    protocol: Protocol,
    origin: &CredentialOrigin,
    outcome: CandidateOutcome,
) {
    tracing::debug!(
        address = %address,
        protocol = %protocol_word(protocol),
        origin = %origin,
        outcome = %outcome.word(),
        "discovery credential candidate"
    );
}

/// Classifies one SNMP attempt's result: a valid response — the read's
/// payload, an exception value, or an error-status the agent reported —
/// proves the community authenticated it; a reply for another community
/// refuses the credential; silence, a socket failure, or a malformed answer
/// says nothing about it.
fn snmp_verdict<T>(result: Result<T, SnmpError>) -> Attempted<Option<T>> {
    match result {
        Ok(payload) => Attempted::Answered(Some(payload)),
        Err(SnmpError::CommunityMismatch { .. }) => Attempted::Rejected,
        Err(SnmpError::Io { .. } | SnmpError::Timeout { .. } | SnmpError::Decode { .. }) => {
            Attempted::Inconclusive
        }
        // A valid response that carried no usable payload: the agent
        // authenticated, and the feed simply has no data.
        Err(
            SnmpError::ErrorStatus { .. }
            | SnmpError::UnsupportedValue { .. }
            | SnmpError::WalkCapExceeded { .. },
        ) => Attempted::Answered(None),
        Err(SnmpError::Cancelled { .. }) => Attempted::Cancelled,
    }
}

/// Classifies one system read for version negotiation, reusing the SNMP
/// verdict vocabulary and splitting answered-with-data from
/// answered-without.
fn classify_system_read(result: Result<SystemInfo, SnmpError>) -> SystemRead {
    match snmp_verdict(result) {
        Attempted::Answered(Some(system)) => SystemRead::Usable(system),
        Attempted::Answered(None) => SystemRead::Unusable,
        Attempted::Rejected => SystemRead::Rejected,
        // SNMP has no negotiation step, so `snmp_verdict` never reports
        // unsupported; an unsupported outcome would say nothing about the
        // credential anyway.
        Attempted::Unsupported | Attempted::Inconclusive => SystemRead::Inconclusive,
        Attempted::Cancelled => SystemRead::Cancelled,
    }
}

/// The wire-version word the negotiation logs carry.
fn version_word(version: SnmpVersion) -> &'static str {
    match version {
        SnmpVersion::V1 => "v1",
        SnmpVersion::V2c => "v2c",
    }
}

/// Logs one completed wire-version system read: the node address, the
/// candidate's non-secret origin, the requested version, the classified
/// outcome, and whether the reply carried usable system data. The
/// community never reaches a field.
fn log_system_read(
    address: &str,
    origin: &CredentialOrigin,
    version: SnmpVersion,
    read: &SystemRead,
) {
    let (outcome, usable) = match read {
        SystemRead::Usable(_) => ("answered", true),
        SystemRead::Unusable => ("answered", false),
        SystemRead::Rejected => ("rejected", false),
        SystemRead::Inconclusive => ("inconclusive", false),
        SystemRead::Cancelled => ("cancelled", false),
    };
    tracing::debug!(
        address = %address,
        origin = %origin,
        version = %version_word(version),
        outcome = %outcome,
        usable = usable,
        "discovery snmp wire version"
    );
}

/// Classifies one RouterOS API attempt's result. The router's own answer
/// separates the cases: a login trap rejects the credential, a command
/// refused after login still proves the credential authenticated, and a
/// malformed or repeated login challenge is a protocol fault this client
/// cannot follow — not a rejected credential, so it ends the fan-out
/// instead of spending the node's remaining candidates.
fn api_verdict(
    result: Result<Vec<CommandRow>, MikrotikError>,
) -> Attempted<Option<Vec<CommandRow>>> {
    match result {
        Ok(rows) => Attempted::Answered(Some(rows)),
        Err(MikrotikError::LoginFailed { .. }) => Attempted::Rejected,
        Err(MikrotikError::CommandFailed { .. }) => Attempted::Answered(None),
        Err(MikrotikError::Protocol { .. }) => Attempted::Unsupported,
        Err(MikrotikError::Io { .. }) => Attempted::Inconclusive,
        Err(MikrotikError::Cancelled { .. }) => Attempted::Cancelled,
    }
}

/// One protocol's attempt tallies for a walk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CandidateTally {
    pub(crate) answered: u64,
    pub(crate) rejected: u64,
    pub(crate) inconclusive: u64,
    pub(crate) unsupported: u64,
}

/// The walk's credential attempt tallies, per protocol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CredentialOutcomes {
    pub(crate) snmp: CandidateTally,
    pub(crate) api: CandidateTally,
}

impl CredentialOutcomes {
    /// Counts one attempt outcome under its protocol.
    fn tally(&mut self, protocol: Protocol, outcome: CandidateOutcome) {
        let tally = match protocol {
            Protocol::Snmp => &mut self.snmp,
            Protocol::Api => &mut self.api,
        };
        match outcome {
            CandidateOutcome::Answered => tally.answered += 1,
            CandidateOutcome::Rejected => tally.rejected += 1,
            CandidateOutcome::Inconclusive => tally.inconclusive += 1,
            CandidateOutcome::Unsupported => tally.unsupported += 1,
        }
    }
}

/// The walk's credential state: the candidate that last won per protocol,
/// and the tallies the caller logs at the end.
#[derive(Default)]
struct CredentialProgress {
    outcomes: CredentialOutcomes,
    preferred_snmp: Option<CredentialOrigin>,
    preferred_api: Option<CredentialOrigin>,
}

impl CredentialProgress {
    /// The credential that last won `protocol`, if any.
    fn preferred(&self, protocol: Protocol) -> Option<&CredentialOrigin> {
        match protocol {
            Protocol::Snmp => self.preferred_snmp.as_ref(),
            Protocol::Api => self.preferred_api.as_ref(),
        }
    }

    /// Remembers the candidate that just authenticated a node.
    fn prefer(&mut self, protocol: Protocol, origin: &CredentialOrigin) {
        let slot = match protocol {
            Protocol::Snmp => &mut self.preferred_snmp,
            Protocol::Api => &mut self.preferred_api,
        };
        *slot = Some(origin.clone());
    }
}

impl NeighbourSource for NetworkSource {
    fn probe<'a>(
        &'a mut self,
        address: &'a str,
        snmp: &'a [CredentialCandidate<SnmpAccess>],
        api: &'a [CredentialCandidate<ApiCredentials>],
    ) -> Pin<Box<dyn Future<Output = Probed> + Send + 'a>> {
        // Cloned before the probe body borrows the source mutably: the
        // cancellation race owns its flag, so the body keeps `&mut self`.
        let flag = self.io.cancelled.clone();
        Box::pin(async move {
            // Cancellation end to end: the whole probe — every socket
            // wait, pacing sleep, and follow-up query — is raced against
            // the flag, so a rise ends it within one poll interval no
            // matter which await it is parked in. Dropping the losing
            // branch drops the in-flight transport futures and their
            // sockets with them.
            tokio::select! {
                probed = self.probe_live(address, snmp, api) => probed,
                _ = wait_cancelled(flag) => Probed::Cancelled(no_probe()),
            }
        })
    }
}

impl NetworkSource {
    /// The probe body: feeds, self-claims, and details, in the walker's
    /// order, each pacing wait and each follow-up feed gated on the
    /// cancellation flag. Each protocol's candidate list is tried in
    /// order, and the candidate that authenticates serves every later query
    /// of the node. The SNMP and API surfaces each run the selected
    /// document's probes for their own transport; the observations merge
    /// with API details taking precedence and the SNMP details as fallback.
    async fn probe_live(
        &mut self,
        address: &str,
        snmp: &[CredentialCandidate<SnmpAccess>],
        api: &[CredentialCandidate<ApiCredentials>],
    ) -> Probed {
        let io = &self.io;
        if io.cancelled() {
            return Probed::Cancelled(no_probe());
        }
        let snmp_auth = authenticate(
            &mut self.progress,
            address,
            Protocol::Snmp,
            snmp,
            |candidate| {
                io.snmp_attempt(
                    address,
                    &candidate.credential,
                    &candidate.origin,
                    &self.registry,
                )
            },
        )
        .await;
        let (_, snmp) = match snmp_auth {
            AuthResult::Winner(candidate, observation) => {
                (Some(&candidate.credential), Some(observation))
            }
            AuthResult::Nothing => (None, None),
            AuthResult::Cancelled => return Probed::Cancelled(no_probe()),
        };
        if io.cancelled() {
            return Probed::Cancelled(no_probe());
        }
        let api_auth = authenticate(
            &mut self.progress,
            address,
            Protocol::Api,
            api,
            |candidate| io.api_attempt(address, &candidate.credential),
        )
        .await;
        let (api_access, api_rows) = match api_auth {
            AuthResult::Winner(candidate, rows) => (Some(&candidate.credential), rows),
            AuthResult::Nothing => (None, None),
            AuthResult::Cancelled => return Probed::Cancelled(no_probe()),
        };
        if io.cancelled() {
            return Probed::Cancelled(no_probe());
        }
        let SnmpObservation {
            claims: snmp_claims,
            lldp,
            details: snmp_details,
            document: snmp_document,
        } = snmp.unwrap_or(SnmpObservation {
            claims: Vec::new(),
            lldp: None,
            details: None,
            document: None,
        });
        // The API surface is a document capability, not a vendor check: the
        // SNMP-selected document serves when it declares one, and an
        // API-only node uses the registry's single API-capable document.
        let snmp_document = snmp_document.and_then(|id| {
            self.registry
                .documents()
                .iter()
                .find(|document| document.id == id)
        });
        let api_observation = match api_access {
            Some(credentials) => match engine::select_api(self.registry.documents(), snmp_document)
            {
                ApiSelection::Selected(document) => {
                    io.api_observation(address, credentials, document, api_rows)
                        .await
                }
                ApiSelection::Unsupported => None,
                ApiSelection::Ambiguous { adapters } => {
                    tracing::warn!(address = %address, "{}", ambiguity(&adapters));
                    None
                }
            },
            None => None,
        };
        let api_claims: Vec<Claim> = api_observation
            .as_ref()
            .map(|observation| {
                observation
                    .claims
                    .iter()
                    // A neighbour or address row states something about that
                    // row's device, which the feed already records; only the
                    // node's own identity and serial are self-claims. Own
                    // addresses arrive through the interface details.
                    .filter(|observed| {
                        matches!(
                            observed.claim.source,
                            Source::RouterOsIdentity | Source::RouterOsSerial
                        )
                    })
                    .map(|observed| observed.claim.clone())
                    .collect()
            })
            .unwrap_or_default();
        let api_neighbours = api_observation
            .as_ref()
            .map(|observation| observation.neighbours.clone());
        // API details win when they succeeded; otherwise the interface and
        // ARP observations the document engine produced are the fallback.
        let details = api_observation
            .and_then(|mut observation| observation.details.take())
            .or(snmp_details);
        let mut self_claims = api_claims;
        self_claims.extend(snmp_claims);
        let mut lldp = lldp;
        if let (Some(rows), Some(details)) = (lldp.as_mut(), details.as_ref()) {
            resolve_local_ports(rows, details);
        }
        let probe = Probe {
            lldp,
            api: api_neighbours,
            self_claims,
            details,
            stations: None,
        };
        if io.cancelled() {
            return Probed::Cancelled(probe);
        }
        Probed::Answers(probe)
    }
}

/// What one authenticated SNMP node yielded through the document engine.
struct SnmpObservation {
    /// The node's own claims the document mapped from the system group, in
    /// document order. LLDP-row claims stay on the neighbour rows; the API's
    /// claims are added by the caller.
    claims: Vec<Claim>,
    /// The LLDP neighbours, when the document's LLDP feed answered.
    lldp: Option<Vec<LldpNeighbour>>,
    /// The interface and ARP details the document mapped.
    details: Option<DeviceDetails>,
    /// The adapter id of the document the system read selected, when one
    /// did. The API surface choice consumes it.
    document: Option<String>,
}

/// Which document serves the device, from the authenticating system facts.
enum DocumentChoice<'a> {
    /// Exactly one document claimed the device.
    Selected(&'a crate::adapters::Document),
    /// No document claimed it.
    Unsupported,
    /// Several documents claimed it; their ids, in document order.
    Ambiguous(Vec<&'a str>),
}

/// Selects the device's document. The engine owns the matching rule; this
/// wrapper only names the three outcomes for the attempt.
fn choose_document<'a>(registry: &'a Registry, system: Option<&SystemInfo>) -> DocumentChoice<'a> {
    match engine::select(
        registry.documents(),
        system.and_then(|info| info.sys_object_id.as_deref()),
        system.map(|info| info.sys_descr.as_str()),
    ) {
        Selection::Selected(document) => DocumentChoice::Selected(document),
        Selection::Unsupported => DocumentChoice::Unsupported,
        Selection::Ambiguous { adapters } => DocumentChoice::Ambiguous(adapters),
    }
}

/// The warning for an ambiguous selection: every matching adapter id, named.
fn ambiguity(adapters: &[&str]) -> String {
    format!("adapter selection is ambiguous between {adapters:?}; no document was executed")
}

/// The credential answered but no document ran: nothing past the system
/// group was read, no claim was mapped, and the node has no feed.
fn answered_nothing() -> Attempted<SnmpObservation> {
    Attempted::Answered(SnmpObservation {
        claims: Vec::new(),
        lldp: None,
        details: None,
        document: None,
    })
}

/// The document's SNMP surface: the transport the SNMP reader speaks. A
/// document declares one in practice; the first SNMP surface serves either
/// SNMP version the candidate negotiated.
fn snmp_surface(document: &Document) -> Option<&crate::adapters::Surface> {
    document.surfaces.iter().find(|surface| {
        matches!(
            surface.transport,
            crate::adapters::Transport::SnmpV1 | crate::adapters::Transport::SnmpV2c
        )
    })
}

/// The varbinds the authenticating system read already fetched, so the
/// document's system probe costs no second request. The standard three are
/// the only ones the read asks for; a probe that names others simply goes
/// to the wire.
fn prefetched(system: Option<&SystemInfo>) -> Vec<Varbind> {
    let Some(system) = system else {
        return Vec::new();
    };
    let mut varbinds = vec![
        Varbind {
            oid: SYS_DESCR.to_vec(),
            value: WalkValue::Bytes(system.sys_descr.as_bytes().to_vec()),
        },
        Varbind {
            oid: SYS_NAME.to_vec(),
            value: WalkValue::Bytes(system.sys_name.as_bytes().to_vec()),
        },
    ];
    if let Some(object_id) = &system.sys_object_id {
        varbinds.push(Varbind {
            oid: SYS_OBJECT_ID.to_vec(),
            value: WalkValue::Oid(object_id.clone()),
        });
    }
    varbinds
}

/// A reader that answers the system values the authenticating read already
/// fetched and forwards every other read to the live transport. The engine
/// still owns decoding: it sees the same varbinds it would have read.
struct PrefetchedReader<'a> {
    live: SnmpReader<'a>,
    prefetched: Vec<Varbind>,
}

impl Reader for PrefetchedReader<'_> {
    fn get<'a>(
        &'a mut self,
        oid: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        if let Some(varbind) = self.prefetched.iter().find(|varbind| varbind.oid == oid) {
            let varbind = varbind.clone();
            return Box::pin(async move { Outcome::Present(vec![varbind]) });
        }
        self.live.get(oid)
    }

    fn walk<'a>(
        &'a mut self,
        root: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        self.live.walk(root)
    }

    fn command<'a>(
        &'a mut self,
        name: CommandName,
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<CommandRow>>> + Send + 'a>> {
        self.live.command(name)
    }
}

impl PacedIo {
    /// One SNMP candidate attempt: negotiate the community's wire version
    /// with a system read, then — when a document claims the device — run
    /// the document's probes once over the version that answered.
    ///
    /// The system read is the authentication: an agent that answers it
    /// proves the community, even when the answer carries an error-status
    /// or an unusable value. A reply for another community is a refusal,
    /// and a timeout, socket failure, or malformed answer says nothing
    /// about the credential either way. A `v2c` candidate that produces no
    /// usable system data is retried once as v1 with the same community;
    /// the winner's values select the document and seed the engine's own
    /// system probe, so the document costs no second system read. The
    /// document's FDB observation, when it answered, attaches to the
    /// details as bridge-host evidence.
    async fn snmp_attempt(
        &self,
        address: &str,
        access: &SnmpAccess,
        origin: &CredentialOrigin,
        registry: &Registry,
    ) -> Attempted<SnmpObservation> {
        self.pace().await;
        let (effective, system) = match self.negotiate_snmp(address, access, origin).await {
            SnmpNegotiation::Answered { access, system } => (access, system),
            SnmpNegotiation::Rejected => return Attempted::Rejected,
            SnmpNegotiation::Inconclusive => return Attempted::Inconclusive,
            SnmpNegotiation::Cancelled => return Attempted::Cancelled,
        };
        tracing::debug!(
            address = %address,
            origin = %origin,
            version = %version_word(effective.version),
            "discovery snmp version selected"
        );
        let document = match choose_document(registry, system.as_ref()) {
            DocumentChoice::Selected(document) => document,
            // No document claims the device. The credential still answered;
            // there is simply nothing to run and nothing to map.
            DocumentChoice::Unsupported => return answered_nothing(),
            DocumentChoice::Ambiguous(adapters) => {
                tracing::warn!(address = %address, "{}", ambiguity(&adapters));
                return answered_nothing();
            }
        };
        let Some(surface) = snmp_surface(document) else {
            return answered_nothing();
        };
        let cancel = || self.cancelled();
        let mut reader = PrefetchedReader {
            live: SnmpReader::new(address, &effective, self.snmp_timeout, &cancel),
            prefetched: prefetched(system.as_ref()),
        };
        let observed_at = SystemTime::now();
        let mut observation =
            engine::probe_surface(document, &surface.id, &mut reader, observed_at, &cancel).await;

        // The engine reports which of the document's surfaces spoke; this
        // module consumes the report rather than re-deriving the rules.
        let claims: Vec<Claim> = observation
            .claims
            .iter()
            .filter(|observed| observed.claim.source == Source::SnmpSystem)
            .map(|observed| observed.claim.clone())
            .collect();
        let lldp = observation
            .lldp_answered
            .then(|| std::mem::take(&mut observation.lldp));
        let mut details = observation.details.take();
        if details.is_none() && observation.interfaces_answered {
            // A speaking interface feed that carried no rows is an empty
            // inventory, not an unknown one: the node reports zero
            // interfaces rather than "unknown".
            details = Some(DeviceDetails::from_snmp_interfaces(&[]));
        }
        // The bridge forwarding database joins the interface inventory, so
        // it is attached to the details the document already mapped: a
        // document that maps no details has nowhere to put the rows, and an
        // FDB probe that never answered carries no snapshot to attach.
        if let (Some(details), Some(fdb)) = (details.as_mut(), observation.fdb.as_ref()) {
            details.attach_snmp_fdb(fdb, observed_at);
        }
        Attempted::Answered(SnmpObservation {
            claims,
            lldp,
            details,
            document: Some(document.id.clone()),
        })
    }

    /// Reads the system group once at the candidate's preferred version and
    /// — for a `v2c` read that produced no usable system data — once more as
    /// v1 with the same community.
    ///
    /// A usable v2c answer ends the negotiation; a rejection is final; a
    /// cancellation stops before the second version. An authenticated v2c
    /// answer without usable data keeps that authenticated outcome when the
    /// v1 retry is rejected or inconclusive: the community is proven even
    /// though no version produced system data.
    async fn negotiate_snmp(
        &self,
        address: &str,
        access: &SnmpAccess,
        origin: &CredentialOrigin,
    ) -> SnmpNegotiation {
        let first = self.wire_system_read(address, access, origin).await;
        if self.cancelled() {
            return SnmpNegotiation::Cancelled;
        }
        let first = match first {
            SystemRead::Usable(system) => {
                return SnmpNegotiation::Answered {
                    access: access.at_version(access.version),
                    system: Some(system),
                };
            }
            SystemRead::Cancelled => return SnmpNegotiation::Cancelled,
            SystemRead::Rejected => return SnmpNegotiation::Rejected,
            first @ (SystemRead::Unusable | SystemRead::Inconclusive) => first,
        };
        if access.version == SnmpVersion::V1 {
            return match first {
                SystemRead::Unusable => SnmpNegotiation::Answered {
                    access: access.at_version(SnmpVersion::V1),
                    system: None,
                },
                _ => SnmpNegotiation::Inconclusive,
            };
        }
        let fallback = access.at_version(SnmpVersion::V1);
        let second = self.wire_system_read(address, &fallback, origin).await;
        if self.cancelled() {
            return SnmpNegotiation::Cancelled;
        }
        let v2c_authenticated = matches!(first, SystemRead::Unusable);
        match second {
            SystemRead::Usable(system) => SnmpNegotiation::Answered {
                access: fallback,
                system: Some(system),
            },
            SystemRead::Unusable => SnmpNegotiation::Answered {
                access: fallback,
                system: None,
            },
            SystemRead::Cancelled => SnmpNegotiation::Cancelled,
            SystemRead::Rejected if v2c_authenticated => SnmpNegotiation::Answered {
                access: access.at_version(SnmpVersion::V2c),
                system: None,
            },
            SystemRead::Rejected => SnmpNegotiation::Rejected,
            SystemRead::Inconclusive if v2c_authenticated => SnmpNegotiation::Answered {
                access: access.at_version(SnmpVersion::V2c),
                system: None,
            },
            SystemRead::Inconclusive => SnmpNegotiation::Inconclusive,
        }
    }

    /// One system read at one wire version, classified and logged.
    async fn wire_system_read(
        &self,
        address: &str,
        access: &SnmpAccess,
        origin: &CredentialOrigin,
    ) -> SystemRead {
        let read = classify_system_read(
            system_info(
                address,
                &access.community,
                access.version,
                self.snmp_timeout,
            )
            .await,
        );
        log_system_read(address, origin, access.version, &read);
        read
    }

    /// One RouterOS API candidate attempt: the neighbour query over that
    /// login. The rows authenticate the credential and seed the API
    /// surface run, so the neighbour table costs one query.
    async fn api_attempt(
        &self,
        address: &str,
        credentials: &ApiCredentials,
    ) -> Attempted<Option<Vec<CommandRow>>> {
        self.pace().await;
        api_verdict(
            command(
                address,
                &credentials.username,
                &credentials.password,
                CommandName::IpNeighbor,
                MIKROTIK_TIMEOUT,
                &|| self.cancelled(),
            )
            .await,
        )
    }

    /// One API surface run: the document's API probes through the live
    /// reader, with the neighbour rows the authentication query already
    /// fetched reused instead of queried again.
    async fn api_observation(
        &self,
        address: &str,
        credentials: &ApiCredentials,
        document: &Document,
        prefetched: Option<Vec<CommandRow>>,
    ) -> Option<engine::Observation> {
        let surface = engine::api_surface(document)?;
        if self.cancelled() {
            return None;
        }
        let cancel = || self.cancelled();
        let mut reader = PrefetchedApiReader {
            live: ApiReader::new(address, credentials, MIKROTIK_TIMEOUT, self.delay, &cancel),
            prefetched,
        };
        let observed_at = SystemTime::now();
        Some(engine::probe_surface(document, &surface.id, &mut reader, observed_at, &cancel).await)
    }
}

/// A reader that answers the `ip_neighbor` command with the rows the
/// authenticating attempt already fetched and forwards every other command
/// to the live API transport. The engine still owns decoding: it sees the
/// same rows it would have read.
struct PrefetchedApiReader<'a> {
    live: ApiReader<'a>,
    prefetched: Option<Vec<CommandRow>>,
}

impl Reader for PrefetchedApiReader<'_> {
    fn get<'a>(
        &'a mut self,
        oid: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        self.live.get(oid)
    }

    fn walk<'a>(
        &'a mut self,
        root: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        self.live.walk(root)
    }

    fn command<'a>(
        &'a mut self,
        name: CommandName,
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<CommandRow>>> + Send + 'a>> {
        if name == CommandName::IpNeighbor
            && let Some(rows) = self.prefetched.take()
        {
            let outcome = if rows.is_empty() {
                Outcome::Empty
            } else {
                Outcome::Present(rows)
            };
            return Box::pin(async move { outcome });
        }
        self.live.command(name)
    }
}

/// One minimal valid document whose detection matches `needle`, shared by
/// the unit tests and the negotiation test module beside this file.
#[cfg(test)]
fn fixture_document(id: &str, needle: &str) -> Document {
    crate::adapters::parse(
        id,
        &format!(
            r#"{{"schema_version":2,"id":"{id}",
                "detect":{{"sys_descr_contains":["{needle}"]}},
                "surfaces":[{{"id":"snmp","transport":"snmp_v2c","credential_ref":"snmp"}}],
                "probes":[{{"id":"system","surface":"snmp","read":{{"kind":"scalars","scalars":[
                    {{"name":"sys_name","oid":"1.3.6.1.2.1.1.5.0","parser":"utf8"}}]}}}}]}}"#
        ),
    )
    .expect("the test document validates")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::snmp::{Community, SnmpVersion};
    use crate::walk::CredentialClass;
    use std::time::Instant;

    /// A target-class SNMP candidate with a fixture community.
    fn snmp_candidate(community: &str) -> CredentialCandidate<SnmpAccess> {
        CredentialCandidate::new(
            CredentialClass::Target,
            CredentialOrigin::Target,
            SnmpAccess {
                community: Community::new(community.to_owned()),
                version: SnmpVersion::V2c,
            },
        )
    }

    /// The class and origin of candidate labels used by the order tests: a
    /// default-class profile and a scoped one, named.
    fn profile_candidate(
        class: CredentialClass,
        name: &'static str,
        credential: &'static str,
    ) -> CredentialCandidate<&'static str> {
        CredentialCandidate::new(
            class,
            CredentialOrigin::Profile(name.to_owned()),
            credential,
        )
    }

    /// The scripted system facts one attempt sees.
    fn system_facts(descr: &str) -> SystemInfo {
        SystemInfo {
            sys_descr: descr.to_owned(),
            sys_name: "probe".to_owned(),
            sys_object_id: Some("1.3.6.1.4.1.9999".to_owned()),
        }
    }

    #[test]
    fn an_unmatched_device_authenticates_without_running_a_document() {
        let registry = Registry::from_documents(vec![fixture_document("other", "OtherOS")]);
        let system = system_facts("AcmeOS 1.0");
        assert!(matches!(
            choose_document(&registry, Some(&system)),
            DocumentChoice::Unsupported
        ));
        let Attempted::Answered(observation) = answered_nothing() else {
            panic!("the credential answered; no document ran");
        };
        assert!(observation.lldp.is_none(), "no feed ran");
        assert!(observation.details.is_none(), "no feed ran");
        assert!(
            observation.claims.is_empty(),
            "the document maps the claims"
        );
    }

    #[tokio::test]
    async fn an_unmatched_device_is_tallied_as_answered() {
        let registry = Registry::from_documents(vec![fixture_document("other", "OtherOS")]);
        let candidates = [snmp_candidate("community")];
        let mut progress = CredentialProgress::default();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Snmp,
            &candidates,
            |_candidate| async {
                match choose_document(&registry, Some(&system_facts("AcmeOS 1.0"))) {
                    DocumentChoice::Selected(_) => panic!("no document matches"),
                    DocumentChoice::Unsupported => answered_nothing(),
                    DocumentChoice::Ambiguous(_) => panic!("one document matches at most"),
                }
            },
        )
        .await;

        let AuthResult::Winner(_, observation) = auth else {
            panic!("an unmatched device still authenticates");
        };
        assert!(observation.lldp.is_none() && observation.details.is_none());
        assert_eq!(progress.outcomes.snmp.answered, 1);
        assert_eq!(progress.outcomes.snmp.inconclusive, 0);
        assert_eq!(progress.outcomes.snmp.rejected, 0);
    }

    #[test]
    fn an_ambiguous_selection_names_every_adapter_and_runs_nothing() {
        let registry = Registry::from_documents(vec![
            fixture_document("first", "SharedOS"),
            fixture_document("second", "SharedOS"),
        ]);
        let system = system_facts("SharedOS 7");
        let DocumentChoice::Ambiguous(adapters) = choose_document(&registry, Some(&system)) else {
            panic!("both documents claim the device");
        };
        assert_eq!(adapters, ["first", "second"]);
        let warning = ambiguity(&adapters);
        assert!(
            warning.contains("first") && warning.contains("second"),
            "the warning names both adapters: {warning}"
        );
        let Attempted::Answered(observation) = answered_nothing() else {
            panic!("the credential answered; no document ran");
        };
        assert!(observation.lldp.is_none() && observation.details.is_none());
        assert!(observation.claims.is_empty());
    }

    #[tokio::test]
    async fn a_paced_source_skips_its_delays_when_cancelled() {
        let mut source = NetworkSource::paced_by(
            Duration::from_secs(60),
            Arc::new(AtomicBool::new(true)),
            Registry::default(),
        );
        let candidate = snmp_candidate("sentinel-community-1");
        let started = Instant::now();
        let probed = source.probe("127.0.0.1:1", &[candidate], &[]).await;
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "a cancelled source must skip its 60 s pacing waits"
        );
        assert!(
            matches!(probed, Probed::Cancelled(_)),
            "a cancelled source reports cancellation, not silence"
        );
    }

    #[tokio::test]
    async fn an_inconclusive_attempt_continues_and_is_tallied_separately() {
        let candidates = [
            profile_candidate(CredentialClass::Default, "silent", "silent"),
            profile_candidate(CredentialClass::Default, "wrong", "wrong"),
            profile_candidate(CredentialClass::Default, "answering", "answering"),
        ];
        let mut progress = CredentialProgress::default();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Snmp,
            &candidates,
            |candidate| async move {
                match candidate.credential {
                    "silent" => Attempted::Inconclusive,
                    "wrong" => Attempted::Rejected,
                    _ => Attempted::Answered("rows"),
                }
            },
        )
        .await;

        let AuthResult::Winner(winner, _) = auth else {
            panic!("the third candidate should have answered");
        };
        assert_eq!(winner.credential, "answering");
        assert_eq!(progress.outcomes.snmp.inconclusive, 1);
        assert_eq!(progress.outcomes.snmp.rejected, 1);
        assert_eq!(progress.outcomes.snmp.answered, 1);
        assert_eq!(progress.outcomes.api, CandidateTally::default());
    }

    #[tokio::test]
    async fn a_failing_candidate_falls_through_every_class() {
        let candidates = [
            CredentialCandidate::new(
                CredentialClass::Seed,
                CredentialOrigin::Seed,
                "seed-credential",
            ),
            profile_candidate(CredentialClass::Default, "shared", "default-credential"),
            CredentialCandidate::new(
                CredentialClass::Target,
                CredentialOrigin::Target,
                "target-credential",
            ),
            profile_candidate(CredentialClass::Scoped, "rack", "scoped-credential"),
        ];
        let mut progress = CredentialProgress::default();
        let auth = authenticate(
            &mut progress,
            "10.0.0.5",
            Protocol::Snmp,
            &candidates,
            |candidate| async move {
                if candidate.credential == "scoped-credential" {
                    Attempted::Answered(())
                } else {
                    Attempted::Rejected
                }
            },
        )
        .await;

        let AuthResult::Winner(winner, ()) = auth else {
            panic!("the scoped fallback should have answered last");
        };
        assert_eq!(winner.credential, "scoped-credential");
        assert_eq!(winner.class, CredentialClass::Scoped);
        assert_eq!(progress.outcomes.snmp.rejected, 3);
        assert_eq!(progress.outcomes.snmp.answered, 1);
        assert_eq!(
            progress.preferred(Protocol::Snmp),
            Some(&CredentialOrigin::Profile("rack".to_owned()))
        );
    }

    #[tokio::test]
    async fn an_unsupported_negotiation_stops_further_candidates() {
        let candidates = [
            profile_candidate(CredentialClass::Default, "malformed", "malformed"),
            profile_candidate(CredentialClass::Default, "would-answer", "would-answer"),
        ];
        let mut progress = CredentialProgress::default();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Api,
            &candidates,
            |candidate| async move {
                if candidate.credential == "malformed" {
                    Attempted::Unsupported
                } else {
                    Attempted::Answered(())
                }
            },
        )
        .await;

        assert!(
            matches!(auth, AuthResult::Nothing),
            "the second candidate is never tried after an unsupported negotiation"
        );
        assert_eq!(progress.outcomes.api.unsupported, 1);
        assert_eq!(progress.outcomes.api.answered, 0);
    }

    #[tokio::test]
    async fn a_rejected_legacy_candidate_lets_the_next_api_candidate_answer() {
        let candidates = [
            profile_candidate(CredentialClass::Default, "first", "first"),
            profile_candidate(CredentialClass::Default, "second", "second"),
        ];
        let mut progress = CredentialProgress::default();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Api,
            &candidates,
            |candidate| async move {
                if candidate.credential == "first" {
                    api_verdict(Err(MikrotikError::LoginFailed {
                        address: "10.0.0.7".to_owned(),
                        message: "cannot log in".to_owned(),
                    }))
                } else {
                    api_verdict(Ok(Vec::<CommandRow>::new()))
                }
            },
        )
        .await;

        let AuthResult::Winner(winner, rows) = auth else {
            panic!("a rejected candidate must let the next one authenticate");
        };
        assert_eq!(winner.credential, "second");
        assert!(rows.is_some(), "the winner's prefetched rows are reused");
        assert_eq!(progress.outcomes.api.rejected, 1);
        assert_eq!(progress.outcomes.api.answered, 1);
    }

    #[tokio::test]
    async fn a_command_refused_after_login_is_not_a_rejected_candidate() {
        let candidates = [profile_candidate(
            CredentialClass::Default,
            "refused",
            "refused",
        )];
        let mut progress = CredentialProgress::default();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Api,
            &candidates,
            |_| async {
                api_verdict(Err(MikrotikError::CommandFailed {
                    address: "10.0.0.7".to_owned(),
                    message: "no permission".to_owned(),
                }))
            },
        )
        .await;

        let AuthResult::Winner(winner, rows) = auth else {
            panic!("a refused command still proves the credential authenticated");
        };
        assert_eq!(winner.credential, "refused");
        assert!(rows.is_none(), "the refused command yields no rows");
        assert_eq!(progress.outcomes.api.rejected, 0);
        assert_eq!(progress.outcomes.api.answered, 1);
    }

    #[tokio::test]
    async fn a_preferred_profile_is_attempted_first() {
        let candidates = two_profile_candidates();
        let mut progress = CredentialProgress::default();
        progress.prefer(
            Protocol::Snmp,
            &CredentialOrigin::Profile("second".to_owned()),
        );
        let mut attempted = Vec::new();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Snmp,
            &candidates,
            |candidate| {
                attempted.push(candidate.origin.to_string());
                std::future::ready(Attempted::<&str>::Rejected)
            },
        )
        .await;

        assert!(matches!(auth, AuthResult::Nothing));
        assert_eq!(
            attempted,
            ["second", "first"],
            "the preferred profile is tried ahead of the other profiles"
        );
        assert_eq!(progress.outcomes.snmp.rejected, 2);
    }

    #[tokio::test]
    async fn an_api_preference_does_not_reorder_snmp_candidates() {
        let candidates = two_profile_candidates();
        let mut progress = CredentialProgress::default();
        progress.prefer(
            Protocol::Api,
            &CredentialOrigin::Profile("second".to_owned()),
        );
        let mut attempted = Vec::new();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Snmp,
            &candidates,
            |candidate| {
                attempted.push(candidate.origin.to_string());
                std::future::ready(Attempted::<&str>::Rejected)
            },
        )
        .await;

        assert!(matches!(auth, AuthResult::Nothing));
        assert_eq!(
            attempted,
            ["first", "second"],
            "only the same protocol's preference reorders the candidates"
        );
    }

    #[tokio::test]
    async fn a_cancelled_attempt_keeps_the_earlier_attempts_recorded() {
        let candidates = two_profile_candidates();
        let mut progress = CredentialProgress::default();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Snmp,
            &candidates,
            |candidate| async move {
                if candidate.credential == "first-credential" {
                    Attempted::<()>::Rejected
                } else {
                    Attempted::<()>::Cancelled
                }
            },
        )
        .await;

        assert!(matches!(auth, AuthResult::Cancelled));
        assert_eq!(
            progress.outcomes.snmp.rejected, 1,
            "the attempt that completed before the cancellation stays tallied"
        );
        assert_eq!(progress.outcomes.snmp.answered, 0);
        assert_eq!(
            progress.outcomes.snmp.inconclusive, 0,
            "the cancelled attempt itself is never counted"
        );
        assert!(progress.preferred(Protocol::Snmp).is_none());
    }

    #[test]
    fn the_preference_is_remembered_per_protocol() {
        let mut progress = CredentialProgress::default();
        assert!(progress.preferred(Protocol::Snmp).is_none());
        let winner = CredentialOrigin::Profile("office-api".to_owned());
        progress.prefer(Protocol::Api, &winner);
        assert_eq!(progress.preferred(Protocol::Api), Some(&winner));
        assert_eq!(
            progress.preferred(Protocol::Snmp),
            None,
            "a winner on one protocol does not stand in for the other"
        );
    }

    /// Two default-profile candidates with distinguishable credentials.
    fn two_profile_candidates() -> [CredentialCandidate<&'static str>; 2] {
        [
            profile_candidate(CredentialClass::Default, "first", "first-credential"),
            profile_candidate(CredentialClass::Default, "second", "second-credential"),
        ]
    }

    #[tokio::test]
    async fn authentication_keeps_the_second_candidate_and_prefers_it() {
        let candidates = two_profile_candidates();
        let mut progress = CredentialProgress::default();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Snmp,
            &candidates,
            |candidate| async move {
                if candidate.credential == "first-credential" {
                    Attempted::Rejected
                } else {
                    Attempted::Answered("second-rows")
                }
            },
        )
        .await;

        let AuthResult::Winner(winner, payload) = auth else {
            panic!("the second candidate should have authenticated");
        };
        assert_eq!(winner.credential, "second-credential");
        assert_eq!(
            winner.origin,
            CredentialOrigin::Profile("second".to_owned())
        );
        assert_eq!(payload, "second-rows", "the winning read is handed back");
        assert_eq!(
            progress.preferred(Protocol::Snmp),
            Some(&CredentialOrigin::Profile("second".to_owned())),
            "the winner becomes the walk's preference for that protocol"
        );
        assert_eq!(progress.outcomes.snmp.rejected, 1);
        assert_eq!(progress.outcomes.snmp.answered, 1);
        assert_eq!(progress.outcomes.snmp.inconclusive, 0);
        assert_eq!(progress.outcomes.api, CandidateTally::default());
    }

    #[tokio::test]
    async fn authentication_without_an_answer_reports_nothing_and_prefers_no_one() {
        let candidates = two_profile_candidates();
        let mut progress = CredentialProgress::default();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Snmp,
            &candidates,
            |_| async { Attempted::<()>::Rejected },
        )
        .await;

        assert!(matches!(auth, AuthResult::Nothing));
        assert_eq!(progress.outcomes.snmp.rejected, 2);
        assert!(progress.preferred(Protocol::Snmp).is_none());
    }

    #[tokio::test]
    async fn a_cancelled_authentication_decides_nothing() {
        let candidates = two_profile_candidates();
        let mut progress = CredentialProgress::default();
        let auth = authenticate(
            &mut progress,
            "10.0.0.7",
            Protocol::Api,
            &candidates,
            |_| async { Attempted::<()>::Cancelled },
        )
        .await;

        assert!(matches!(auth, AuthResult::Cancelled));
        assert_eq!(
            progress.outcomes,
            CredentialOutcomes::default(),
            "a cancelled attempt is never tallied"
        );
        assert!(
            progress.preferred(Protocol::Api).is_none(),
            "a cancelled attempt sets no preference"
        );
    }

    #[test]
    fn a_login_trap_rejects_the_api_credential_and_a_refused_command_does_not() {
        let login = api_verdict(Err(MikrotikError::LoginFailed {
            address: "a".to_owned(),
            message: "bad password".to_owned(),
        }));
        assert!(matches!(login, Attempted::Rejected));

        let refused = api_verdict(Err(MikrotikError::CommandFailed {
            address: "a".to_owned(),
            message: "no permission".to_owned(),
        }));
        assert!(
            matches!(refused, Attempted::Answered(None)),
            "a command refused after login still proves the credential"
        );
    }

    #[test]
    fn a_protocol_fault_is_unsupported_and_a_dead_socket_is_inconclusive() {
        let fault = api_verdict(Err(MikrotikError::Protocol {
            address: "a".to_owned(),
            reason: "login challenge repeated".to_owned(),
        }));
        assert!(matches!(fault, Attempted::Unsupported));

        let dead = api_verdict(Err(MikrotikError::Io {
            address: "a".to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
        }));
        assert!(matches!(dead, Attempted::Inconclusive));

        let cancelled = api_verdict(Err(MikrotikError::Cancelled {
            address: "a".to_owned(),
        }));
        assert!(matches!(cancelled, Attempted::Cancelled));
    }

    #[test]
    fn snmp_answers_classify_by_what_the_agent_said() {
        use crate::snmp::SnmpError;

        assert!(matches!(
            snmp_verdict(Ok::<Vec<u32>, SnmpError>(Vec::new())),
            Attempted::Answered(Some(_))
        ));
        assert!(
            matches!(
                snmp_verdict::<Vec<u32>>(Err(SnmpError::ErrorStatus {
                    address: "a".to_owned(),
                    status: 2,
                })),
                Attempted::Answered(None)
            ),
            "a valid response that is not the table still authenticates"
        );
        assert!(
            matches!(
                snmp_verdict::<Vec<u32>>(Err(SnmpError::UnsupportedValue {
                    address: "a".to_owned(),
                })),
                Attempted::Answered(None)
            ),
            "an exception value is still a valid response"
        );
        assert!(
            matches!(
                snmp_verdict::<Vec<u32>>(Err(SnmpError::WalkCapExceeded {
                    address: "a".to_owned(),
                    steps: 512,
                })),
                Attempted::Answered(None)
            ),
            "a capped walk means the agent answered; the feed just carries no rows"
        );
        assert!(matches!(
            snmp_verdict::<Vec<u32>>(Err(SnmpError::CommunityMismatch {
                address: "a".to_owned(),
            })),
            Attempted::Rejected
        ));
        assert!(matches!(
            snmp_verdict::<Vec<u32>>(Err(SnmpError::Timeout {
                address: "a".to_owned(),
                timeout: SNMP_TIMEOUT,
            })),
            Attempted::Inconclusive
        ));
        assert!(matches!(
            snmp_verdict::<Vec<u32>>(Err(SnmpError::Decode {
                address: "a".to_owned(),
                reason: "truncated".to_owned(),
            })),
            Attempted::Inconclusive
        ));
        assert!(matches!(
            snmp_verdict::<Vec<u32>>(Err(SnmpError::Cancelled {
                address: "a".to_owned(),
            })),
            Attempted::Cancelled
        ));
    }
}

/// The wire-version negotiation suite lives beside this file so the main
/// module stays readable; as a child module it reuses the documented private
/// `PacedIo::snmp_timeout` test seam and the internal types.
#[cfg(test)]
mod snmp_negotiation_tests;

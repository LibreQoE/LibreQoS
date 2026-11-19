//! Offline tests for the walk core: dedup, cycles, bounds, credential
//! scope filtering, leaf recording, and determinism — all over an injectable
//! neighbour source, no network. The report is observed through its fields:
//! `WalkReport` is the server's input, and no text form sits between them.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use dudarchy_poller::adapters::engine;
use dudarchy_poller::adapters::reader::{CommandRow, Outcome, Reader, Varbind};
use dudarchy_poller::adapters::{self, CommandName, Document};
use dudarchy_poller::stored::StoredHost;
use dudarchy_poller::{
    AddressGate, ApiCredentials, ApiNeighbour, Claim, ClaimKind, Community, CredentialCandidate,
    DeviceDetails, HookFuture, LldpNeighbour, Mac, NeighbourSource, NodeCredentials, ObservedLink,
    Password, Probe, Probed, RouterOsAddress, RouterOsInterface, SnmpAccess, SnmpVersion, Source,
    Vault, WalkBounds, WalkHooks, WalkNode, WalkProgress, WalkReport, walk_observing,
};

/// One recorded probe call: the address, and which feeds had credentials.
type ProbeCall = (String, bool, bool);

/// Resolver grants: address -> (snmp community, api user/password). An
/// address with no grant resolves to no credentials at all.
type Grants = HashMap<String, (Option<String>, Option<(String, String)>)>;

/// What the source answers for one address: the rows both feeds returned.
/// An address with no entry answers nothing.
type Answers = HashMap<String, (Vec<LldpNeighbour>, Vec<ApiNeighbour>)>;

/// The injectable neighbour source: scripted answers, scripted self-claims
/// per probed address, scripted interface details, plus a probe log.
struct FakeSource {
    answers: Answers,
    self_claims: HashMap<String, Vec<Claim>>,
    details: HashMap<String, Option<DeviceDetails>>,
    probes: Vec<ProbeCall>,
}

impl NeighbourSource for FakeSource {
    fn probe<'a>(
        &'a mut self,
        address: &'a str,
        snmp: &'a [CredentialCandidate<SnmpAccess>],
        api: &'a [CredentialCandidate<ApiCredentials>],
    ) -> Pin<Box<dyn Future<Output = Probed> + Send + 'a>> {
        Box::pin(async move {
            self.probes
                .push((address.to_owned(), !snmp.is_empty(), !api.is_empty()));
            let self_claims = self.self_claims.get(address).cloned().unwrap_or_default();
            let probe = match self.answers.get(address) {
                Some((lldp, api_rows)) => Probe {
                    lldp: Some(lldp.clone()),
                    api: Some(api_rows.clone()),
                    self_claims,
                    details: self.details.get(address).cloned().flatten(),
                    stations: None,
                },
                None => Probe {
                    lldp: None,
                    api: None,
                    self_claims,
                    details: None,
                    stations: None,
                },
            };
            // The scripted source answers as soon as it is polled.
            Probed::Answers(probe)
        })
    }
}

/// One grant entry: address, optional snmp community, optional api
/// user/password pair.
type GrantEntry<'a> = (&'a str, Option<&'a str>, Option<(&'a str, &'a str)>);

/// Resolver grants: address -> (snmp community, api user/password). An
/// address with no grant resolves to no credentials at all.
fn grants(entries: &[GrantEntry]) -> Grants {
    entries
        .iter()
        .map(|(address, snmp, api)| {
            (
                (*address).to_owned(),
                (
                    snmp.map(str::to_owned),
                    api.map(|(user, pass)| (user.to_owned(), pass.to_owned())),
                ),
            )
        })
        .collect()
}

/// One walk from the standard seed (10.0.0.1, named `seed-a`, with both
/// explicit credentials) over `source` with `grants` and `bounds`, with
/// no-op hooks: the report is the whole result.
async fn walk_report(source: &mut FakeSource, granted: Grants, bounds: WalkBounds) -> WalkReport {
    let resolve = move |address: &str| match granted.get(address) {
        Some((snmp, api)) => node_credentials(
            snmp.as_deref(),
            api.as_ref()
                .map(|(user, pass)| (user.as_str(), pass.as_str())),
        ),
        None => NodeCredentials::none(),
    };
    let cancelled = || false;
    let mut observer = |_: &WalkNode| Box::pin(async {}) as HookFuture;
    let mut link = |_: &ObservedLink| Box::pin(async {}) as HookFuture;
    let mut progress = |_: &WalkProgress| Box::pin(async {}) as HookFuture;
    walk_observing(
        "10.0.0.1",
        "seed-a",
        node_credentials(Some("seed-snmp"), Some(("seed-user", "seed-pass"))),
        bounds,
        &resolve,
        source,
        dudarchy_poller::AddressGate::unrestricted(),
        WalkHooks {
            cancelled: &cancelled,
            observer: &mut observer,
            link: &mut link,
            progress: &mut progress,
        },
    )
    .await
}

/// One walk from the standard seed address with `seed_name`, no
/// credentials, no-op observer and link hooks, and `bounds`; returns the
/// progress updates as `(label, address, pending)`.
async fn walk_progress(
    source: &mut FakeSource,
    seed_name: &str,
    bounds: WalkBounds,
) -> Vec<(String, String, usize)> {
    let resolve = |_: &str| NodeCredentials::none();
    let cancelled = || false;
    let mut observer = |_: &WalkNode| Box::pin(async {}) as HookFuture;
    let mut link = |_: &ObservedLink| Box::pin(async {}) as HookFuture;
    let mut seen = Vec::new();
    let mut progress = |update: &WalkProgress| {
        seen.push((update.label.clone(), update.address.clone(), update.pending));
        Box::pin(async {}) as HookFuture
    };
    walk_observing(
        "10.0.0.1",
        seed_name,
        NodeCredentials::none(),
        bounds,
        &resolve,
        source,
        dudarchy_poller::AddressGate::unrestricted(),
        WalkHooks {
            cancelled: &cancelled,
            observer: &mut observer,
            link: &mut link,
            progress: &mut progress,
        },
    )
    .await;
    seen
}

/// A node's identity fields, in report order: MAC, address, name, depth, and
/// how the walk knows it.
type NodeFields<'a> = (
    Option<&'a str>,
    Option<&'a str>,
    Option<&'a str>,
    usize,
    String,
);
fn node_fields(report: &WalkReport) -> Vec<NodeFields<'_>> {
    report
        .nodes
        .iter()
        .map(|node| {
            (
                node.mac.as_deref(),
                node.address.as_deref(),
                node.name.as_deref(),
                node.depth,
                node.source.to_string(),
            )
        })
        .collect()
}

/// The links' endpoints in recording order.
fn link_pairs(report: &WalkReport) -> Vec<(&str, &str)> {
    report
        .links
        .iter()
        .map(|link| (link.from.as_str(), link.to.as_str()))
        .collect()
}

/// A node's credentials: one target-origin candidate per protocol when the
/// grant names one.
fn node_credentials(snmp: Option<&str>, api: Option<(&str, &str)>) -> NodeCredentials {
    let mut credentials = NodeCredentials::none();
    if let Some(community) = snmp {
        credentials = credentials.with_target_snmp(SnmpAccess {
            community: Community::new(community.to_owned()),
            version: SnmpVersion::V2c,
        });
    }
    if let Some((user, pass)) = api {
        credentials = credentials.with_target_api(ApiCredentials {
            username: user.to_owned(),
            password: Password::new(pass.to_owned()),
        });
    }
    credentials
}

/// An LLDP row fixture: raw 6-byte chassis MAC, optional address and name.
fn lldp_row(mac: &str, address: Option<&str>, name: Option<&str>) -> LldpNeighbour {
    LldpNeighbour {
        chassis_id: mac_bytes(mac),
        address: address.map(str::to_owned),
        remote_port_id: None,
        sys_name: name.map(str::to_owned),
        sys_descr: None,
        local_port: None,
    }
}

/// An API row fixture: text MAC, optional address and identity.
fn api_row(mac: &str, address: Option<&str>, identity: Option<&str>) -> ApiNeighbour {
    ApiNeighbour {
        address: address.map(str::to_owned),
        mac_address: Some(mac.to_owned()),
        identity: identity.map(str::to_owned),
        interface: None,
    }
}

/// Raw MAC bytes from colon-separated hex pairs, for fixture chassis ids.
fn mac_bytes(mac: &str) -> Vec<u8> {
    mac.split(':')
        .map(|pair| u8::from_str_radix(pair, 16).expect("fixture mac byte"))
        .collect()
}

#[tokio::test]
async fn two_parents_reporting_one_mac_visit_it_once() {
    let mut source = FakeSource {
        answers: HashMap::from([
            (
                "10.0.0.1".to_owned(),
                (
                    vec![lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
                    vec![api_row("00:1f:5e:00:00:03", Some("10.0.0.3"), Some("cee"))],
                ),
            ),
            (
                "10.0.0.2".to_owned(),
                (
                    vec![lldp_row("00:1f:5e:00:00:03", Some("10.0.0.3"), None)],
                    vec![],
                ),
            ),
        ]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let report = walk_report(&mut source, grants(&[]), WalkBounds::default()).await;

    assert_eq!(report.nodes.len(), 3, "C is recorded once");
    assert_eq!(
        link_pairs(&report),
        [
            ("10.0.0.1", "00:1f:5e:00:00:02"),
            ("10.0.0.1", "00:1f:5e:00:00:03"),
            ("00:1f:5e:00:00:02", "00:1f:5e:00:00:03"),
        ]
    );
}

#[tokio::test]
async fn progress_reports_the_seed_then_breadth_first_queue_depth() {
    // The seed names two depth-1 neighbours; the first reports one depth-2
    // neighbour with no name. The progress stream must name the seed
    // first, then follow the queue's BFS pop order, with the discovered
    // nodes behind the current one; the nameless node falls back to its
    // MAC.
    let mut source = FakeSource {
        answers: HashMap::from([
            (
                "10.0.0.1".to_owned(),
                (
                    vec![
                        lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee")),
                        lldp_row("00:1f:5e:00:00:03", Some("10.0.0.3"), Some("cee")),
                    ],
                    vec![],
                ),
            ),
            (
                "10.0.0.2".to_owned(),
                (
                    vec![lldp_row("00:1f:5e:00:00:04", Some("10.0.0.4"), None)],
                    vec![],
                ),
            ),
        ]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let mut seen: Vec<(String, String, usize)> = Vec::new();
    let resolve = |_: &str| NodeCredentials::none();
    let cancelled = || false;
    let mut observer = |_: &WalkNode| Box::pin(async {}) as HookFuture;
    let mut link = |_: &ObservedLink| Box::pin(async {}) as HookFuture;
    let mut progress = |update: &WalkProgress| {
        seen.push((update.label.clone(), update.address.clone(), update.pending));
        Box::pin(async {}) as HookFuture
    };
    let bounds = WalkBounds {
        max_depth: 2,
        ..WalkBounds::default()
    };
    walk_observing(
        "10.0.0.1",
        "seed-a",
        NodeCredentials::none(),
        bounds,
        &resolve,
        &mut source,
        dudarchy_poller::AddressGate::unrestricted(),
        WalkHooks {
            cancelled: &cancelled,
            observer: &mut observer,
            link: &mut link,
            progress: &mut progress,
        },
    )
    .await;

    assert_eq!(
        seen.iter()
            .map(|(label, address, pending)| (label.as_str(), address.as_str(), *pending))
            .collect::<Vec<_>>(),
        [
            ("seed-a", "10.0.0.1", 0),
            ("bee", "10.0.0.2", 1),
            ("cee", "10.0.0.3", 1),
            ("00:1f:5e:00:00:04", "10.0.0.4", 0),
        ],
        "seed first, then BFS order with the queue depth behind each node"
    );
}

#[tokio::test]
async fn progress_pending_excludes_address_less_queued_leaves() {
    // The seed reports one probeable neighbour and one row with a MAC but
    // no management address. The address-less node is queued and skipped,
    // so it contributes no progress entry and no pending count.
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![
                    lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee")),
                    lldp_row("00:1f:5e:00:00:03", None, None),
                ],
                vec![],
            ),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let seen = walk_progress(&mut source, "seed-a", WalkBounds::default()).await;

    assert_eq!(
        seen,
        [
            ("seed-a".to_owned(), "10.0.0.1".to_owned(), 0),
            ("bee".to_owned(), "10.0.0.2".to_owned(), 0),
        ],
        "the MAC-only queued leaf is skipped and does not inflate pending"
    );
}

#[tokio::test]
async fn progress_label_falls_back_to_the_address_when_the_seed_has_no_name() {
    // An unnamed, MAC-less seed: the label falls back to the probed
    // address, the equal-label case the UI collapses to one field.
    let mut source = FakeSource {
        answers: HashMap::new(),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let bounds = WalkBounds {
        max_depth: 0,
        ..WalkBounds::default()
    };
    let seen = walk_progress(&mut source, "", bounds).await;

    assert_eq!(
        seen,
        [("10.0.0.1".to_owned(), "10.0.0.1".to_owned(), 0)],
        "the label falls back to the address and equals it"
    );
}

#[tokio::test]
async fn progress_omits_the_node_dequeued_after_a_real_cancellation() {
    // The seed names two neighbours. The observer turns the walk's real
    // cancellation flag true when the first neighbour has been absorbed
    // and queued: the absorption loop stops, and the while loop pops the
    // queued node and abandons it at the loop-top guard before any
    // progress call.
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![
                    lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee")),
                    lldp_row("00:1f:5e:00:00:03", Some("10.0.0.3"), Some("cee")),
                ],
                vec![],
            ),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let resolve = |_: &str| NodeCredentials::none();
    let flag = Arc::new(AtomicBool::new(false));
    let cancelled_flag = Arc::clone(&flag);
    let cancelled = move || cancelled_flag.load(Ordering::SeqCst);
    let observer_flag = Arc::clone(&flag);
    let mut observer = move |node: &WalkNode| {
        if node.mac.is_some() {
            observer_flag.store(true, Ordering::SeqCst);
        }
        Box::pin(async {}) as HookFuture
    };
    let mut link = |_: &ObservedLink| Box::pin(async {}) as HookFuture;
    let mut seen: Vec<(String, String, usize)> = Vec::new();
    let mut progress = |update: &WalkProgress| {
        seen.push((update.label.clone(), update.address.clone(), update.pending));
        Box::pin(async {}) as HookFuture
    };
    walk_observing(
        "10.0.0.1",
        "seed-a",
        NodeCredentials::none(),
        WalkBounds::default(),
        &resolve,
        &mut source,
        dudarchy_poller::AddressGate::unrestricted(),
        WalkHooks {
            cancelled: &cancelled,
            observer: &mut observer,
            link: &mut link,
            progress: &mut progress,
        },
    )
    .await;

    assert_eq!(
        seen,
        [("seed-a".to_owned(), "10.0.0.1".to_owned(), 0)],
        "the cancelled walk reports no progress for the neighbour it queued but never probed"
    );
    assert_eq!(
        source.probes,
        [("10.0.0.1".to_owned(), false, false)],
        "the seed is the only node probed after the cancellation flag flips"
    );
}

#[tokio::test]
async fn a_cycle_links_back_without_revisiting() {
    let mut source = FakeSource {
        answers: HashMap::from([
            (
                "10.0.0.1".to_owned(),
                (
                    vec![lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
                    vec![],
                ),
            ),
            (
                "10.0.0.2".to_owned(),
                (
                    vec![lldp_row(
                        "00:1f:5e:00:00:01",
                        Some("10.0.0.1"),
                        Some("seed-a"),
                    )],
                    vec![],
                ),
            ),
        ]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let report = walk_report(
        &mut source,
        grants(&[("10.0.0.2", Some("bee-snmp"), Some(("bee-user", "bee-pass")))]),
        WalkBounds::default(),
    )
    .await;

    assert_eq!(report.nodes.len(), 2, "no duplicate seed");
    assert_eq!(
        link_pairs(&report),
        [
            ("00:1f:5e:00:00:01", "00:1f:5e:00:00:02"),
            ("00:1f:5e:00:00:02", "00:1f:5e:00:00:01"),
        ],
        "the seed's link resolves to its learned MAC"
    );
    assert_eq!(
        source.probes,
        [
            ("10.0.0.1".to_owned(), true, true),
            ("10.0.0.2".to_owned(), true, true),
        ],
        "each MAC is probed exactly once"
    );
}

#[tokio::test]
async fn the_depth_cap_stops_expansion() {
    let mut source = FakeSource {
        answers: HashMap::from([
            (
                "10.0.0.1".to_owned(),
                (
                    vec![lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
                    vec![],
                ),
            ),
            (
                "10.0.0.2".to_owned(),
                (
                    vec![lldp_row("00:1f:5e:00:00:03", Some("10.0.0.3"), Some("cee"))],
                    vec![],
                ),
            ),
        ]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let bounds = WalkBounds {
        max_depth: 1,
        ..WalkBounds::default()
    };
    let report = walk_report(&mut source, grants(&[]), bounds).await;

    assert_eq!(report.nodes.len(), 2, "C is beyond the cap");
    assert_eq!(
        link_pairs(&report),
        [
            ("10.0.0.1", "00:1f:5e:00:00:02"),
            ("00:1f:5e:00:00:02", "00:1f:5e:00:00:03"),
        ]
    );
    assert_eq!(source.probes.len(), 2, "C is never probed");
}

#[tokio::test]
async fn addresses_outside_every_grant_walk_as_leaves() {
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![
                    lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("in-scope")),
                    lldp_row("00:1f:5e:00:00:03", Some("10.0.0.3"), Some("out")),
                ],
                vec![],
            ),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let granted = grants(&[("10.0.0.2", Some("scoped-community"), None)]);
    let report = walk_report(&mut source, granted, WalkBounds::default()).await;

    assert_eq!(
        source.probes,
        [
            ("10.0.0.1".to_owned(), true, true),
            ("10.0.0.2".to_owned(), true, false),
            ("10.0.0.3".to_owned(), false, false),
        ],
        "the scoped address gets its credential, the other gets none"
    );
    assert_eq!(
        report.nodes.len(),
        3,
        "the credential-less node is recorded"
    );
    assert_eq!(
        link_pairs(&report),
        [
            ("10.0.0.1", "00:1f:5e:00:00:02"),
            ("10.0.0.1", "00:1f:5e:00:00:03"),
        ],
        "the credential-less node is a leaf"
    );
}

#[tokio::test]
async fn an_address_with_no_candidate_at_all_is_a_leaf() {
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
                vec![],
            ),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let report = walk_report(&mut source, grants(&[]), WalkBounds::default()).await;

    assert_eq!(
        source.probes[1..],
        [("10.0.0.2".to_owned(), false, false)],
        "no candidate: no feed is probed with credentials"
    );
    assert_eq!(report.nodes.len(), 2, "the leaf is still recorded");
}

#[tokio::test]
async fn a_mac_without_an_address_is_recorded_as_a_leaf() {
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (vec![], vec![api_row("00:1f:5e:00:00:09", None, None)]),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let report = walk_report(&mut source, grants(&[]), WalkBounds::default()).await;

    assert_eq!(source.probes.len(), 1, "a MAC-only node is never probed");
    let leaf = report
        .nodes
        .iter()
        .find(|node| node.mac.as_deref() == Some("00:1f:5e:00:00:09"))
        .expect("the MAC-only leaf is recorded");
    assert_eq!(leaf.address, None, "no address was reported");
    assert_eq!(leaf.depth, 1);
    assert_eq!(leaf.source.to_string(), "api");
}

#[tokio::test]
async fn the_node_cap_stops_creation_but_keeps_links() {
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![],
                vec![
                    api_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee")),
                    api_row("00:1f:5e:00:00:03", Some("10.0.0.3"), Some("cee")),
                ],
            ),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let bounds = WalkBounds {
        max_nodes: 2,
        ..WalkBounds::default()
    };
    let report = walk_report(&mut source, grants(&[]), bounds).await;

    assert_eq!(report.nodes.len(), 2, "the cap holds");
    assert_eq!(
        link_pairs(&report),
        [
            ("10.0.0.1", "00:1f:5e:00:00:02"),
            ("10.0.0.1", "00:1f:5e:00:00:03"),
        ]
    );
    assert_eq!(source.probes.len(), 2, "only the seed and B are probed");
    assert!(
        !source
            .probes
            .iter()
            .any(|(address, _, _)| address == "10.0.0.3"),
        "the capped node is never probed"
    );
}

#[tokio::test]
async fn the_report_is_deterministic_and_pinned() {
    let make_source = || FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![lldp_row(
                    "00:1f:5e:00:00:02",
                    Some("10.0.0.20"),
                    Some("FSW-BEE"),
                )],
                vec![api_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
            ),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let mut first = make_source();
    let mut second = make_source();
    let first = walk_report(&mut first, grants(&[]), WalkBounds::default()).await;
    let second = walk_report(&mut second, grants(&[]), WalkBounds::default()).await;

    assert_eq!(node_fields(&first), node_fields(&second));
    assert_eq!(link_pairs(&first), link_pairs(&second));
    assert_eq!(
        node_fields(&first),
        [
            (None, Some("10.0.0.1"), Some("seed-a"), 0, "seed".to_owned()),
            (
                Some("00:1f:5e:00:00:02"),
                Some("10.0.0.2"),
                Some("bee"),
                1,
                "both".to_owned()
            ),
        ]
    );
    assert_eq!(link_pairs(&first), [("10.0.0.1", "00:1f:5e:00:00:02")]);
}

#[tokio::test]
async fn self_claims_attach_after_neighbour_claims_and_deduplicate() {
    let mut source = FakeSource {
        answers: HashMap::from([
            (
                "10.0.0.1".to_owned(),
                (
                    vec![],
                    vec![api_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
                ),
            ),
            ("10.0.0.2".to_owned(), (vec![], vec![])),
        ]),
        self_claims: HashMap::from([
            (
                "10.0.0.1".to_owned(),
                vec![Claim {
                    kind: ClaimKind::Serial,
                    value: "SEED-SERIAL".to_owned(),
                    source: Source::RouterOsSerial,
                }],
            ),
            (
                "10.0.0.2".to_owned(),
                vec![
                    Claim {
                        kind: ClaimKind::Name,
                        value: "bee".to_owned(),
                        source: Source::RouterOsIdentity,
                    },
                    Claim {
                        kind: ClaimKind::Serial,
                        value: "BEE-SERIAL".to_owned(),
                        source: Source::RouterOsSerial,
                    },
                ],
            ),
        ]),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    let report = walk_report(&mut source, grants(&[]), WalkBounds::default()).await;

    let claim = |kind, value: &str, source| Claim {
        kind,
        value: value.to_owned(),
        source,
    };
    assert_eq!(
        report.nodes[0].claims,
        [claim(
            ClaimKind::Serial,
            "SEED-SERIAL",
            Source::RouterOsSerial
        )]
    );
    assert_eq!(
        report.nodes[1].claims,
        [
            claim(ClaimKind::Address, "10.0.0.2", Source::RouterOsNeighbors),
            claim(ClaimKind::Name, "bee", Source::RouterOsNeighbors),
            claim(
                ClaimKind::Vendor,
                "Dyna Technology Co.,Ltd. (ma-l 001F5E)",
                Source::OuiRegistry
            ),
            claim(ClaimKind::Name, "bee", Source::RouterOsIdentity),
            claim(ClaimKind::Serial, "BEE-SERIAL", Source::RouterOsSerial),
        ],
        "neighbour claims first, the derived hint, then self-claims; identical claims collapse"
    );
}

#[tokio::test]
async fn self_claims_ride_along_on_nodes_that_answered() {
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
                vec![],
            ),
        )]),
        self_claims: HashMap::from([(
            "10.0.0.2".to_owned(),
            vec![Claim {
                kind: ClaimKind::Serial,
                value: "BEE-SERIAL".to_owned(),
                source: Source::RouterOsSerial,
            }],
        )]),
        details: HashMap::new(),
        probes: Vec::new(),
    };
    // B's feeds are scripted, so it answers; its serial self-claim rides
    // along on the node.
    let granted = grants(&[("10.0.0.2", Some("bee-snmp"), Some(("bee-user", "bee-pass")))]);
    let report = walk_report(&mut source, granted, WalkBounds::default()).await;
    let bee = report
        .nodes
        .iter()
        .find(|node| node.mac.as_deref() == Some("00:1f:5e:00:00:02"))
        .expect("the neighbour is recorded");
    assert!(
        bee.claims
            .iter()
            .any(|claim| claim.kind == ClaimKind::Serial
                && claim.value == "BEE-SERIAL"
                && claim.source == Source::RouterOsSerial),
        "the self-claim rides on the node: {:?}",
        bee.claims
    );
}

#[tokio::test]
async fn interface_details_ride_on_nodes_with_their_own_address_claims() {
    let details = DeviceDetails::from_router_os(
        &[
            RouterOsInterface {
                id: "*1".to_owned(),
                name: Some("ether1".to_owned()),
                raw_type: Some("ether".to_owned()),
                mac_address: Some("6C:3B:6B:E1:45:F0".to_owned()),
                running: Some(true),
                disabled: Some(false),
            },
            RouterOsInterface {
                id: "*2".to_owned(),
                name: Some("vlan92".to_owned()),
                raw_type: Some("vlan".to_owned()),
                mac_address: None,
                running: Some(true),
                disabled: Some(false),
            },
        ],
        &[RouterOsAddress {
            address: Some("10.0.0.1/24".to_owned()),
            network: Some("10.0.0.0".to_owned()),
            interface: Some("ether1".to_owned()),
        }],
    );
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![],
                vec![api_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
            ),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::from([("10.0.0.1".to_owned(), Some(details))]),
        probes: Vec::new(),
    };
    let report = walk_report(&mut source, grants(&[]), WalkBounds::default()).await;

    let seed = report.nodes[0]
        .details
        .as_ref()
        .expect("the seed's details land");
    assert_eq!(seed.interfaces.len(), 2);
    assert_eq!(seed.interfaces[0].name.as_deref(), Some("ether1"));
    assert_eq!(seed.interfaces[0].kind.to_string(), "ethernet");
    assert_eq!(
        seed.interfaces[0].mac.as_ref().map(|mac| mac.to_string()),
        Some("6c:3b:6b:e1:45:f0".to_owned())
    );
    assert_eq!(seed.interfaces[1].kind.to_string(), "vlan");

    // The probed node's own address row is node evidence: the bare IP, the
    // RouterOS address query as its source.
    assert_eq!(
        report.nodes[0].claims,
        vec![Claim {
            kind: ClaimKind::Address,
            value: "10.0.0.1".to_owned(),
            source: Source::RouterOsAddresses,
        }],
        "a probed node's own address list rides as claims"
    );
    let unprobed = report
        .nodes
        .iter()
        .find(|node| node.mac.as_deref() == Some("00:1f:5e:00:00:02"))
        .expect("the neighbour is recorded");
    assert!(unprobed.details.is_none());
    assert!(
        !unprobed
            .claims
            .iter()
            .any(|claim| claim.source == Source::RouterOsAddresses),
        "a node with no details gains no own addresses"
    );
}

#[tokio::test]
async fn a_seed_adopts_the_owning_interfaces_mac_and_its_details_reach_the_host() {
    // Two interfaces own the probed address; the smallest name wins, so
    // the choice never depends on device row order, and a bridge is
    // eligible like any other interface. A subinterface inheriting its
    // parent's MAC is another row: a host has many MACs, and the adopted
    // one is only the store key.
    let details = DeviceDetails::from_router_os(
        &[
            RouterOsInterface {
                id: "*1".to_owned(),
                name: Some("ether1".to_owned()),
                raw_type: Some("ether".to_owned()),
                mac_address: Some("6C:3B:6B:E1:45:F0".to_owned()),
                running: Some(true),
                disabled: Some(false),
            },
            RouterOsInterface {
                id: "*2".to_owned(),
                name: Some("bridge1".to_owned()),
                raw_type: Some("bridge".to_owned()),
                mac_address: Some("6C:3B:6B:E1:45:F1".to_owned()),
                running: Some(true),
                disabled: Some(false),
            },
            RouterOsInterface {
                id: "*3".to_owned(),
                name: Some("ether1.20".to_owned()),
                raw_type: Some("vlan".to_owned()),
                mac_address: Some("6C:3B:6B:E1:45:F0".to_owned()),
                running: Some(true),
                disabled: Some(false),
            },
        ],
        &[
            RouterOsAddress {
                address: Some("10.0.0.1/24".to_owned()),
                network: Some("10.0.0.0".to_owned()),
                interface: Some("ether1".to_owned()),
            },
            RouterOsAddress {
                address: Some("10.0.0.1/24".to_owned()),
                network: Some("10.0.0.0".to_owned()),
                interface: Some("bridge1".to_owned()),
            },
        ],
    );
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
                vec![],
            ),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::from([("10.0.0.1".to_owned(), Some(details))]),
        probes: Vec::new(),
    };
    let report = walk_report(&mut source, grants(&[]), WalkBounds::default()).await;

    let seed = &report.nodes[0];
    assert_eq!(
        seed.mac.as_deref(),
        Some("6c:3b:6b:e1:45:f1"),
        "the smallest-named owning interface keys the seed"
    );
    assert_eq!(seed.address.as_deref(), Some("10.0.0.1"));
    assert_eq!(
        link_pairs(&report),
        [("6c:3b:6b:e1:45:f1", "00:1f:5e:00:00:02")],
        "the seed links as its adopted MAC"
    );

    let stored =
        StoredHost::from_node(seed, SystemTime::UNIX_EPOCH).expect("the seed projects as a host");
    assert_eq!(stored.mac, "6c:3b:6b:e1:45:f1");
    let details = stored.details.expect("the seed's details persist");
    assert_eq!(details.interfaces.len(), 3);
    assert_eq!(
        details
            .interfaces
            .iter()
            .map(|row| row.mac.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("6c:3b:6b:e1:45:f0"),
            Some("6c:3b:6b:e1:45:f1"),
            Some("6c:3b:6b:e1:45:f0"),
        ],
        "every interface MAC stays, including one inherited from its parent"
    );
    assert_eq!(details.interfaces[1].name.as_deref(), Some("bridge1"));
    assert_eq!(
        details.interfaces[1].addresses.as_slice(),
        ["10.0.0.1/24"],
        "the address rows stay bound to their interface"
    );
}

#[tokio::test]
async fn an_adopted_seed_mac_is_visited_so_a_neighbour_cannot_fork_it() {
    let details = DeviceDetails::from_router_os(
        &[RouterOsInterface {
            id: "*1".to_owned(),
            name: Some("ether1".to_owned()),
            raw_type: Some("ether".to_owned()),
            mac_address: Some("6C:3B:6B:E1:45:F0".to_owned()),
            running: Some(true),
            disabled: Some(false),
        }],
        &[RouterOsAddress {
            address: Some("10.0.0.1/24".to_owned()),
            network: Some("10.0.0.0".to_owned()),
            interface: Some("ether1".to_owned()),
        }],
    );
    // The seed adopts its owning interface's MAC; bee later reports that
    // MAC with no address. The adopted MAC is in the visited set, so the
    // row lands on the seed instead of forking a duplicate node.
    let mut source = FakeSource {
        answers: HashMap::from([
            (
                "10.0.0.1".to_owned(),
                (
                    vec![lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
                    vec![],
                ),
            ),
            (
                "10.0.0.2".to_owned(),
                (
                    vec![],
                    vec![api_row("6C:3B:6B:E1:45:F0", None, Some("seed-self"))],
                ),
            ),
        ]),
        self_claims: HashMap::new(),
        details: HashMap::from([("10.0.0.1".to_owned(), Some(details))]),
        probes: Vec::new(),
    };
    // Depth 2 would admit a forked node: only the visited MAC prevents it.
    let bounds = WalkBounds {
        max_depth: 2,
        ..WalkBounds::default()
    };
    let report = walk_report(
        &mut source,
        grants(&[("10.0.0.2", Some("bee-snmp"), None)]),
        bounds,
    )
    .await;

    assert_eq!(
        report.nodes.len(),
        2,
        "no duplicate node for the adopted MAC"
    );
    assert_eq!(report.nodes[0].mac.as_deref(), Some("6c:3b:6b:e1:45:f0"));
    assert!(
        report.nodes[0].claims.iter().any(|claim| {
            claim.kind == ClaimKind::Name
                && claim.value == "seed-self"
                && claim.source == Source::RouterOsNeighbors
        }),
        "the reporter's claim lands on the seed: {:?}",
        report.nodes[0].claims
    );
    assert_eq!(
        link_pairs(&report),
        [
            ("6c:3b:6b:e1:45:f0", "00:1f:5e:00:00:02"),
            ("00:1f:5e:00:00:02", "6c:3b:6b:e1:45:f0"),
        ]
    );
}

#[tokio::test]
async fn a_seed_with_no_owning_interface_stays_mac_less() {
    // The first interface carries a MAC but owns no address; the second
    // owns a different address. Neither owns the probed address, so the
    // seed adopts nothing: never the first interface's MAC, never a MAC
    // made from the address.
    let details = DeviceDetails::from_router_os(
        &[
            RouterOsInterface {
                id: "*1".to_owned(),
                name: Some("ether1".to_owned()),
                raw_type: Some("ether".to_owned()),
                mac_address: Some("6C:3B:6B:E1:45:F0".to_owned()),
                running: Some(true),
                disabled: Some(false),
            },
            RouterOsInterface {
                id: "*2".to_owned(),
                name: Some("ether2".to_owned()),
                raw_type: Some("ether".to_owned()),
                mac_address: Some("6C:3B:6B:E1:45:F1".to_owned()),
                running: Some(true),
                disabled: Some(false),
            },
        ],
        &[RouterOsAddress {
            address: Some("10.0.0.2/24".to_owned()),
            network: Some("10.0.0.0".to_owned()),
            interface: Some("ether2".to_owned()),
        }],
    );
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![lldp_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
                vec![],
            ),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::from([("10.0.0.1".to_owned(), Some(details))]),
        probes: Vec::new(),
    };
    let report = walk_report(&mut source, grants(&[]), WalkBounds::default()).await;

    let seed = &report.nodes[0];
    assert_eq!(seed.mac, None, "no interface owns the probed address");
    assert!(seed.details.is_some(), "the probe's details still land");
    assert!(
        StoredHost::from_node(seed, SystemTime::UNIX_EPOCH).is_none(),
        "a MAC-less seed has no host"
    );
    assert_eq!(
        link_pairs(&report),
        [("10.0.0.1", "00:1f:5e:00:00:02")],
        "the seed stays an address-keyed vantage"
    );
}

#[tokio::test]
async fn a_failed_details_query_leaves_the_node_detail_free() {
    let mut source = FakeSource {
        answers: HashMap::from([(
            "10.0.0.1".to_owned(),
            (
                vec![],
                vec![api_row("00:1f:5e:00:00:02", Some("10.0.0.2"), Some("bee"))],
            ),
        )]),
        self_claims: HashMap::new(),
        details: HashMap::from([("10.0.0.1".to_owned(), None)]),
        probes: Vec::new(),
    };
    let report = walk_report(&mut source, grants(&[]), WalkBounds::default()).await;
    assert!(
        report.nodes.iter().all(|node| node.details.is_none()),
        "a failed details query leaves every node detail-free"
    );
}

/// One RouterOS command row from key/value pairs.
fn command_row(fields: &[(&str, &str)]) -> CommandRow {
    fields
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>()
}

/// The shipped RouterOS document, parsed and validated.
fn routeros_document() -> Document {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/routeros.json");
    let text = std::fs::read_to_string(&path).expect("the shipped RouterOS document reads");
    adapters::parse("routeros.json", &text).unwrap_or_else(|errors| {
        panic!(
            "routeros.json should validate: {}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    })
}

/// A command reader answering only the API probes the RouterOS document
/// requires, plus the lease table under test.
struct LeaseReader {
    leases: Outcome<Vec<CommandRow>>,
}

impl Reader for LeaseReader {
    fn get<'a>(
        &'a mut self,
        _oid: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        Box::pin(async { Outcome::Absent })
    }

    fn walk<'a>(
        &'a mut self,
        _root: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        Box::pin(async { Outcome::Absent })
    }

    fn command<'a>(
        &'a mut self,
        name: CommandName,
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<CommandRow>>> + Send + 'a>> {
        let leases = self.leases.clone();
        Box::pin(async move {
            match name {
                CommandName::SystemIdentity => {
                    Outcome::Present(vec![command_row(&[("name", "router")])])
                }
                CommandName::IpNeighbor => Outcome::Empty,
                CommandName::Interfaces => Outcome::Present(vec![command_row(&[
                    (".id", "*1"),
                    ("name", "ether1"),
                    ("type", "ether"),
                    ("disabled", "false"),
                ])]),
                CommandName::SystemResource => {
                    Outcome::Present(vec![command_row(&[("version", "6.49.18")])])
                }
                CommandName::IpDhcpServerLease => leases,
                _ => Outcome::Absent,
            }
        })
    }
}

#[tokio::test]
async fn walk_gates_lease_addresses_before_the_details_land() {
    let document = routeros_document();
    let mut reader = LeaseReader {
        leases: Outcome::Present(vec![
            // A v4-mapped literal is canonicalized on admission.
            command_row(&[
                ("address", "::ffff:10.0.0.5"),
                ("mac-address", "02:00:00:AA:00:20"),
                ("host-name", "printer"),
                ("status", "bound"),
                ("disabled", "false"),
            ]),
            // Ignored by the scan policy.
            command_row(&[
                ("address", "10.9.9.9"),
                ("mac-address", "02:00:00:AA:00:21"),
                ("host-name", "ignored-tv"),
                ("status", "bound"),
                ("disabled", "false"),
            ]),
            // Outside the allow list.
            command_row(&[
                ("address", "192.168.1.5"),
                ("mac-address", "02:00:00:AA:00:22"),
                ("host-name", "not-allowed"),
                ("status", "bound"),
                ("disabled", "false"),
            ]),
            // Never an IP literal.
            command_row(&[
                ("address", "not-an-address"),
                ("mac-address", "02:00:00:AA:00:23"),
                ("host-name", "unparseable"),
                ("status", "bound"),
                ("disabled", "false"),
            ]),
        ]),
    };
    let observation = engine::probe_surface(
        &document,
        "api",
        &mut reader,
        SystemTime::UNIX_EPOCH,
        &|| false,
    )
    .await;
    let details = observation
        .details
        .expect("the lease table maps into the details");
    assert!(details.dhcp_leases.is_some(), "the lease rows are present");

    let vault =
        Vault::parse(r#"{"scan_policy": {"allow": ["10.0.0.0/8"], "ignore": ["10.9.9.9/32"]}}"#)
            .expect("the fixture policy parses");
    let mut source = FakeSource {
        answers: HashMap::from([("10.0.0.1".to_owned(), (Vec::new(), Vec::new()))]),
        self_claims: HashMap::new(),
        details: HashMap::from([("10.0.0.1".to_owned(), Some(details))]),
        probes: Vec::new(),
    };
    let resolve = |_: &str| NodeCredentials::none();
    let cancelled = || false;
    let mut observer = |_: &WalkNode| Box::pin(async {}) as HookFuture;
    let mut link = |_: &ObservedLink| Box::pin(async {}) as HookFuture;
    let mut progress = |_: &WalkProgress| Box::pin(async {}) as HookFuture;
    let report = walk_observing(
        "10.0.0.1",
        "router",
        NodeCredentials::none(),
        WalkBounds::default(),
        &resolve,
        &mut source,
        AddressGate::new(vault.scan_policy()),
        WalkHooks {
            cancelled: &cancelled,
            observer: &mut observer,
            link: &mut link,
            progress: &mut progress,
        },
    )
    .await;

    let seed = report
        .nodes
        .iter()
        .find(|node| node.address.as_deref() == Some("10.0.0.1"))
        .expect("the seed is recorded");
    let table = seed
        .details
        .as_ref()
        .and_then(|details| details.dhcp_leases.as_ref())
        .expect("the lease table lands on the node");
    assert_eq!(table.rows.len(), 4, "the rows stay; only addresses go");
    assert_eq!(
        table.rows[0].address.as_deref(),
        Some("10.0.0.5"),
        "the admitted v4-mapped address is stored canonical"
    );
    assert_eq!(table.rows[0].hostname.as_deref(), Some("printer"));
    for (row, (hostname, raw_mac)) in table.rows[1..].iter().zip([
        ("ignored-tv", "02:00:00:AA:00:21"),
        ("not-allowed", "02:00:00:AA:00:22"),
        ("unparseable", "02:00:00:AA:00:23"),
    ]) {
        assert_eq!(
            row.address, None,
            "a refused lease address is cleared from the row"
        );
        let mac = Mac::parse(raw_mac).expect("the fixture MAC parses");
        assert_eq!(
            row.mac.as_ref().map(Mac::as_str),
            Some(mac.as_str()),
            "a refused row keeps its normalized client MAC"
        );
        assert_eq!(
            row.hostname.as_deref(),
            Some(hostname),
            "a refused row keeps its hostname"
        );
    }
    assert_eq!(report.refusals.ignored, 1);
    assert_eq!(report.refusals.not_allowed, 1);
    assert_eq!(report.refusals.unparseable, 1);

    // `FakeSource.probes` is the walk's probe-call log: the walk queues
    // discovered devices, never a lease address, so the only recorded probe
    // is the seed. A lease row can only refuse its address, never add a probe
    // target.
    assert!(!source.probes.is_empty(), "the seed probe is recorded");
    assert!(
        source
            .probes
            .iter()
            .all(|(address, _, _)| address == "10.0.0.1"),
        "a lease address is never probed: {:?}",
        source.probes
    );
}

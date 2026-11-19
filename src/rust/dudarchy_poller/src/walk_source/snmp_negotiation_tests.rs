//! Wire-version negotiation coverage: a fake UDP agent scripts both the
//! v2c read and the v1 retry and records every request, so the fallback's
//! wire versions, document reuse, tallies, and cancellation behaviour are
//! pinned through the documented private `PacedIo::snmp_timeout` test seam.
//!
//! Declared as a child of `walk_source`: that seam and the module internals
//! stay reachable, and the fake agent's BER helpers live here instead of in
//! the main module.

use super::*;

use std::net::UdpSocket;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use crate::identity::ClaimKind;
use crate::snmp::{Community, SnmpVersion, resolve_target};
use crate::vault::Vault;
use crate::walk::CredentialClass;

const GET: u8 = 0xa0;
const GET_NEXT: u8 = 0xa1;

/// One request the fake agent received, decoded far enough to pin the
/// wire version, PDU shape, community, and requested OIDs.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedRequest {
    version: i64,
    pdu: u8,
    community: Vec<u8>,
    oids: Vec<Vec<u32>>,
}

/// A deterministic fake UDP SNMP agent: every datagram is decoded and
/// recorded, and `script` decides the reply (`None` is silence). The
/// script runs before the request is recorded, so a cancellation flag it
/// raises is visible to the client once its reply arrives.
struct FakeAgent {
    address: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl FakeAgent {
    fn spawn(script: impl Fn(&RecordedRequest) -> Option<Vec<u8>> + Send + 'static) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("agent should bind");
        let address = socket
            .local_addr()
            .expect("agent should have an address")
            .to_string();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("agent timeout should apply");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while let Ok((len, from)) = socket.recv_from(&mut buffer) {
                let Some(request) = parse_request(&buffer[..len]) else {
                    continue;
                };
                let reply = script(&request);
                recorded.lock().expect("agent log").push(request);
                if let Some(reply) = reply
                    && socket.send_to(&reply, from).is_err()
                {
                    return;
                }
            }
        });
        Self { address, requests }
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("agent log").clone()
    }

    /// Waits until the agent recorded `count` requests, so a silent
    /// request that the client timed out on is still in the log.
    fn wait_for(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.requests().len() < count {
            assert!(
                Instant::now() < deadline,
                "the agent saw {} of {count} expected requests",
                self.requests().len()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// A minimal BER cursor: enough to decode one request message.
struct Ber<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Ber<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn tlv(&mut self) -> Option<(u8, &'a [u8])> {
        let tag = *self.bytes.get(self.at)?;
        self.at += 1;
        let first = *self.bytes.get(self.at)?;
        self.at += 1;
        let len = if first < 0x80 {
            usize::from(first)
        } else {
            let count = usize::from(first & 0x7f);
            let mut len = 0usize;
            for _ in 0..count {
                len = (len << 8) | usize::from(*self.bytes.get(self.at)?);
                self.at += 1;
            }
            len
        };
        let end = self.at.checked_add(len)?;
        let content = self.bytes.get(self.at..end)?;
        self.at = end;
        Some((tag, content))
    }
}

fn decode_oid(content: &[u8]) -> Option<Vec<u32>> {
    let mut arcs = Vec::new();
    let mut value = 0u32;
    for byte in content {
        value = value.checked_shl(7)? | u32::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            if arcs.is_empty() {
                arcs.push(value / 40);
                arcs.push(value % 40);
            } else {
                arcs.push(value);
            }
            value = 0;
        }
    }
    Some(arcs)
}

fn parse_request(bytes: &[u8]) -> Option<RecordedRequest> {
    let (tag, message) = Ber::new(bytes).tlv()?;
    if tag != 0x30 {
        return None;
    }
    let mut body = Ber::new(message);
    let (tag, version) = body.tlv()?;
    if tag != 0x02 {
        return None;
    }
    let version = version
        .iter()
        .fold(0i64, |value, byte| (value << 8) | i64::from(*byte));
    let (tag, community) = body.tlv()?;
    if tag != 0x04 {
        return None;
    }
    let (pdu, content) = body.tlv()?;
    let mut pdu_body = Ber::new(content);
    pdu_body.tlv()?; // request-id
    pdu_body.tlv()?; // error-status
    pdu_body.tlv()?; // error-index
    let (tag, bindings) = pdu_body.tlv()?;
    if tag != 0x30 {
        return None;
    }
    let mut varbinds = Ber::new(bindings);
    let mut oids = Vec::new();
    while let Some((tag, varbind)) = varbinds.tlv() {
        if tag != 0x30 {
            return None;
        }
        let (tag, oid) = Ber::new(varbind).tlv()?;
        if tag != 0x06 {
            return None;
        }
        oids.push(decode_oid(oid)?);
    }
    Some(RecordedRequest {
        version,
        pdu,
        community: community.to_vec(),
        oids,
    })
}

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    if content.len() < 0x80 {
        out.push(u8::try_from(content.len()).expect("short form length"));
    } else {
        let bytes = content.len().to_be_bytes();
        let first = bytes.iter().position(|byte| *byte != 0).unwrap_or(7);
        out.push(0x80 | u8::try_from(8 - first).expect("length bytes fit"));
        out.extend_from_slice(&bytes[first..]);
    }
    out.extend_from_slice(content);
    out
}

fn integer(value: i64) -> Vec<u8> {
    tlv(0x02, &value.to_be_bytes())
}

fn oid_bytes(oid: &[u32]) -> Vec<u8> {
    let mut packed = vec![40 * oid[0] + oid[1]];
    packed.extend_from_slice(&oid[2..]);
    let mut out = Vec::new();
    for value in packed {
        let mut chunk = vec![u8::try_from(value & 0x7f).expect("subidentifier fits")];
        let mut rest = value >> 7;
        while rest > 0 {
            chunk.push(u8::try_from((rest & 0x7f) | 0x80).expect("subidentifier fits"));
            rest >>= 7;
        }
        chunk.reverse();
        out.extend_from_slice(&chunk);
    }
    out
}

fn varbind(oid: &[u32], value: &[u8]) -> Vec<u8> {
    let mut pair = tlv(0x06, &oid_bytes(oid));
    pair.extend_from_slice(value);
    tlv(0x30, &pair)
}

fn response(version: i64, community: &[u8], error_status: i64, varbinds: &[Vec<u8>]) -> Vec<u8> {
    let mut pdu = integer(1);
    pdu.extend(integer(error_status));
    pdu.extend(integer(0));
    pdu.extend(tlv(0x30, &varbinds.concat()));
    let mut message = integer(version);
    message.extend(tlv(0x04, community));
    message.extend(tlv(0xa2, &pdu));
    tlv(0x30, &message)
}

fn octets(bytes: &[u8]) -> Vec<u8> {
    tlv(0x04, bytes)
}

fn object_id(oid: &[u32]) -> Vec<u8> {
    tlv(0x06, &oid_bytes(oid))
}

fn null() -> Vec<u8> {
    tlv(0x05, &[])
}

fn end_of_mib_view() -> Vec<u8> {
    tlv(0x82, &[])
}

fn no_such_object() -> Vec<u8> {
    tlv(0x80, &[])
}

/// The AirOS system facts the real document selects `.10002` on.
const AIROS_OBJECT_ID: [u32; 8] = [1, 3, 6, 1, 4, 1, 10002, 1];

/// The airos document's wireless-station table root: one column walk per
/// station field below it.
const AIROS_STATION_TABLE: [u32; 10] = [1, 3, 6, 1, 4, 1, 41112, 1, 4, 7];

fn is_system_request(request: &RecordedRequest) -> bool {
    request.pdu == GET
        && request.oids
            == [
                SYS_DESCR.to_vec(),
                SYS_OBJECT_ID.to_vec(),
                SYS_NAME.to_vec(),
            ]
}

fn system_response(
    version: i64,
    community: &[u8],
    descr: &[u8],
    object_id_value: &[u32],
    name: &[u8],
) -> Vec<u8> {
    response(
        version,
        community,
        0,
        &[
            varbind(&SYS_DESCR, &octets(descr)),
            varbind(&SYS_OBJECT_ID, &object_id(object_id_value)),
            varbind(&SYS_NAME, &octets(name)),
        ],
    )
}

/// The reply for a document probe: absent objects on v2c, `noSuchName`
/// on v1, and an empty walk either way — enough for the engine to finish
/// with empty tables.
fn generic_response(request: &RecordedRequest) -> Vec<u8> {
    let oid = request
        .oids
        .first()
        .cloned()
        .unwrap_or_else(|| vec![1, 3, 6, 1]);
    if request.version == 0 {
        response(0, &request.community, 2, &[varbind(&oid, &null())])
    } else if request.pdu == GET_NEXT {
        response(
            1,
            &request.community,
            0,
            &[varbind(&oid, &end_of_mib_view())],
        )
    } else {
        response(
            1,
            &request.community,
            0,
            &[varbind(&oid, &no_such_object())],
        )
    }
}

const TEST_TIMEOUT: Duration = Duration::from_millis(100);

/// The shipped registries, so the real airos detection selects the
/// document in the fallback tests.
fn shipped_registry() -> Registry {
    let mut registry = Registry::default();
    registry
        .reload(&Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters"))
        .expect("the shipped adapters validate");
    registry
}

fn test_source(registry: Registry) -> NetworkSource {
    let mut source = NetworkSource::new(Duration::ZERO, registry);
    source.io.snmp_timeout = TEST_TIMEOUT;
    source
}

fn candidate_at(community: &str, version: SnmpVersion) -> CredentialCandidate<SnmpAccess> {
    CredentialCandidate::new(
        CredentialClass::Target,
        CredentialOrigin::Target,
        SnmpAccess {
            community: Community::new(community.to_owned()),
            version,
        },
    )
}

/// An agent silent on v2c and answering v1: usable system facts for the
/// system read, empty tables for every document probe.
fn v1_only_agent() -> FakeAgent {
    FakeAgent::spawn(|request| {
        if request.version != 0 {
            return None;
        }
        if is_system_request(request) {
            Some(system_response(
                0,
                &request.community,
                b"Linux",
                &AIROS_OBJECT_ID,
                b"airmax-ap",
            ))
        } else {
            Some(generic_response(request))
        }
    })
}

fn a_document_with_a_table() -> crate::adapters::Document {
    crate::adapters::parse(
        "tableos",
        r#"{"schema_version":2,"id":"tableos",
            "detect":{"sys_descr_contains":["TableOS"]},
            "surfaces":[{"id":"snmp","transport":"snmp_v2c","credential_ref":"snmp"}],
            "probes":[
                {"id":"system","surface":"snmp","read":{"kind":"scalars","scalars":[
                    {"name":"sys_name","oid":"1.3.6.1.2.1.1.5.0","parser":"utf8"}]}},
                {"id":"if_table","surface":"snmp","read":{"kind":"table","table":{
                    "columns":[{"name":"description","oid":"1.3.6.1.2.1.2.2.1.2","parser":"utf8"}],
                    "index":{"kind":"single"}}}}]}"#,
    )
    .expect("the table document validates")
}

#[tokio::test]
async fn a_silent_v2c_read_falls_back_to_v1_for_the_whole_document() {
    let agent = v1_only_agent();
    let mut source = test_source(shipped_registry());
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let probed = source.probe(&agent.address, &[candidate], &[]).await;
    let Probed::Answers(probe) = probed else {
        panic!("a v1-only agent should answer after the fallback");
    };
    agent.wait_for(2);
    let requests = agent.requests();
    assert_eq!(requests[0].version, 1, "the first read is v2c");
    assert!(is_system_request(&requests[0]));
    assert_eq!(requests[1].version, 0, "the retry is v1");
    assert!(is_system_request(&requests[1]));
    assert!(
        requests[2..].iter().all(|request| request.version == 0),
        "every document read uses the negotiated v1 access"
    );
    let station_requests = requests
        .iter()
        .filter(|request| {
            request
                .oids
                .iter()
                .any(|oid| oid.starts_with(&AIROS_STATION_TABLE))
        })
        .count();
    assert_eq!(
        station_requests, 1,
        "the airos document runs its station probe exactly once: the empty \
         table costs one walk"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| is_system_request(request))
            .count(),
        2,
        "one system read per wire version; the document reuses the winning values"
    );
    assert!(
        probe
            .self_claims
            .iter()
            .any(|claim| claim.kind == ClaimKind::SysDescr && claim.value == "Linux"),
        "the v1 system group stated the node's description: {:?}",
        probe.self_claims
    );
    assert!(
        probe
            .self_claims
            .iter()
            .any(|claim| claim.kind == ClaimKind::Name && claim.value == "airmax-ap"),
        "the v1 system group stated the node's name"
    );
    assert!(
        probe
            .self_claims
            .iter()
            .all(|claim| claim.source == Source::SnmpSystem),
        "the system claims carry SNMP provenance"
    );
    assert_eq!(
        source.progress.preferred(Protocol::Snmp),
        Some(&CredentialOrigin::Target),
        "the winning candidate keeps its origin; the effective v1 version is not walk-wide"
    );
}

#[tokio::test]
async fn a_usable_v2c_read_never_tries_v1() {
    // No document claims the device: the credential answered, nothing ran.
    let agent = FakeAgent::spawn(|request| {
        if is_system_request(request) {
            Some(system_response(
                1,
                &request.community,
                b"AcmeOS 1.0",
                &[1, 3, 6, 1, 4, 1, 9999],
                b"acme",
            ))
        } else {
            Some(generic_response(request))
        }
    });
    let registry = Registry::from_documents(vec![fixture_document("other", "OtherOS")]);
    let mut source = test_source(registry);
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let probed = source.probe(&agent.address, &[candidate], &[]).await;
    assert!(
        matches!(probed, Probed::Answers(_)),
        "an answered credential is a successful probe even with no document"
    );
    let requests = agent.requests();
    assert!(
        requests.iter().all(|request| request.version == 1),
        "a usable v2c answer never spends a v1 request: {requests:?}"
    );
    assert_eq!(source.credential_outcomes().snmp.answered, 1);
    assert_eq!(source.credential_outcomes().snmp.inconclusive, 0);

    // A later optional document probe fails: the version decision is
    // already settled and v1 is never tried.
    let agent = FakeAgent::spawn(|request| {
        if is_system_request(request) {
            Some(system_response(
                1,
                &request.community,
                b"TableOS 1.0",
                &[1, 3, 6, 1, 4, 1, 9999],
                b"tableos",
            ))
        } else {
            Some(response(1, &request.community, 5, &[]))
        }
    });
    let mut source = test_source(Registry::from_documents(vec![a_document_with_a_table()]));
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let probed = source.probe(&agent.address, &[candidate], &[]).await;
    assert!(matches!(probed, Probed::Answers(_)));
    let requests = agent.requests();
    assert!(
        requests.len() > 1,
        "the document ran a probe after the system read"
    );
    assert!(
        requests.iter().all(|request| request.version == 1),
        "a failing document probe is not a version problem: {requests:?}"
    );
}

#[tokio::test]
async fn an_unusable_v2c_answer_falls_back_and_failed_fallbacks_keep_it_answered() {
    // A valid v2c error-status proves the community but carries no
    // system data; v1 then supplies it.
    let agent = FakeAgent::spawn(|request| {
        if request.version == 1 && is_system_request(request) {
            return Some(response(1, &request.community, 5, &[]));
        }
        if request.version == 0 && is_system_request(request) {
            return Some(system_response(
                0,
                &request.community,
                b"Linux",
                &AIROS_OBJECT_ID,
                b"airmax-ap",
            ));
        }
        Some(generic_response(request))
    });
    let mut source = test_source(shipped_registry());
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let Probed::Answers(probe) = source.probe(&agent.address, &[candidate], &[]).await else {
        panic!("the v1 retry should discover the device");
    };
    agent.wait_for(2);
    let requests = agent.requests();
    assert_eq!(requests[0].version, 1);
    assert_eq!(requests[1].version, 0);
    assert!(
        requests[2..].iter().all(|request| request.version == 0),
        "the document runs on the version that answered"
    );
    assert!(
        probe
            .self_claims
            .iter()
            .any(|claim| claim.kind == ClaimKind::SysDescr && claim.value == "Linux")
    );
    assert_eq!(source.credential_outcomes().snmp.answered, 1);

    // A non-string system value is the same authenticated-but-unusable
    // case.
    let agent = FakeAgent::spawn(|request| {
        if request.version == 1 && is_system_request(request) {
            return Some(response(
                1,
                &request.community,
                0,
                &[
                    varbind(&SYS_DESCR, &integer(1)),
                    varbind(&SYS_OBJECT_ID, &object_id(&AIROS_OBJECT_ID)),
                    varbind(&SYS_NAME, &octets(b"airmax-ap")),
                ],
            ));
        }
        if request.version == 0 && is_system_request(request) {
            return Some(system_response(
                0,
                &request.community,
                b"Linux",
                &AIROS_OBJECT_ID,
                b"airmax-ap",
            ));
        }
        Some(generic_response(request))
    });
    let mut source = test_source(shipped_registry());
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let probed = source.probe(&agent.address, &[candidate], &[]).await;
    assert!(
        matches!(probed, Probed::Answers(ref probe) if !probe.self_claims.is_empty()),
        "the v1 retry supplies the system data"
    );

    // v2c answered with no data and the v1 retry fails: the credential
    // stays answered, and the default document still runs over the
    // version that authenticated it.
    let agent = FakeAgent::spawn(|request| {
        if request.version == 0 {
            return None;
        }
        if is_system_request(request) {
            Some(response(1, &request.community, 5, &[]))
        } else {
            Some(generic_response(request))
        }
    });
    let mut source = test_source(shipped_registry());
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let Probed::Answers(probe) = source.probe(&agent.address, &[candidate], &[]).await else {
        panic!("the v2c answer already proved the credential");
    };
    assert!(probe.self_claims.is_empty());
    agent.wait_for(2);
    let requests = agent.requests();
    assert_eq!(requests[0].version, 1);
    assert_eq!(requests[1].version, 0);
    assert!(
        requests[2..].iter().all(|request| request.version == 1),
        "the document runs over the authenticated v2c access: {requests:?}"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.version == 0)
            .count(),
        1,
        "v1 is not retried again"
    );
    let outcomes = source.credential_outcomes().snmp;
    assert_eq!(outcomes.answered, 1);
    assert_eq!(outcomes.rejected, 0);
    assert_eq!(outcomes.inconclusive, 0);
}

#[tokio::test]
async fn a_rejected_v1_retry_after_an_unusable_v2c_answer_keeps_the_candidate_answered_over_v2c() {
    // Every v1 read is refused, but the v2c error-status already proved the
    // community: the candidate stays answered over v2c, the document runs
    // there, and the later candidate is never tried.
    let agent = FakeAgent::spawn(|request| {
        if request.version == 0 {
            return Some(response(0, b"another-community", 0, &[]));
        }
        if is_system_request(request) {
            return Some(response(1, &request.community, 5, &[]));
        }
        Some(generic_response(request))
    });
    let mut source = test_source(shipped_registry());
    let candidates = [
        candidate_at("sentinel-community-1", SnmpVersion::V2c),
        candidate_at("later-community", SnmpVersion::V2c),
    ];
    let Probed::Answers(probe) = source.probe(&agent.address, &candidates, &[]).await else {
        panic!("the v2c answer already proved the credential");
    };
    assert!(
        probe.self_claims.is_empty(),
        "no version produced usable system data"
    );
    let requests = agent.requests();
    assert_eq!(requests[0].version, 1, "the first read is v2c");
    assert!(is_system_request(&requests[0]));
    assert_eq!(requests[1].version, 0, "the retry is v1");
    assert!(is_system_request(&requests[1]));
    assert!(
        requests[2..].iter().all(|request| request.version == 1),
        "the document runs over the authenticated v2c access: {requests:?}"
    );
    assert!(
        requests.iter().any(|request| !is_system_request(request)),
        "the default document still runs"
    );
    assert!(
        requests
            .iter()
            .all(|request| request.community == b"sentinel-community-1"),
        "the answered candidate ends the fan-out; the later candidate is never tried"
    );
    let outcomes = source.credential_outcomes().snmp;
    assert_eq!(outcomes.answered, 1);
    assert_eq!(outcomes.rejected, 0);
    assert_eq!(outcomes.inconclusive, 0);
}

#[tokio::test]
async fn a_rejected_v1_retry_after_v2c_silence_rejects_the_candidate_without_a_document() {
    // Silence says nothing about the community; the refused v1 reply is the
    // only evidence, so the candidate is rejected and nothing runs.
    let agent = FakeAgent::spawn(|request| {
        if request.version == 1 {
            return None;
        }
        Some(response(0, b"another-community", 0, &[]))
    });
    let mut source = test_source(shipped_registry());
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let probed = source.probe(&agent.address, &[candidate], &[]).await;
    assert!(
        matches!(probed, Probed::Answers(ref probe) if probe.self_claims.is_empty()),
        "a rejected candidate is a leaf, not a protocol error"
    );
    agent.wait_for(2);
    let requests = agent.requests();
    assert_eq!(
        requests.len(),
        2,
        "no document query follows a rejected retry: {requests:?}"
    );
    assert_eq!(requests[0].version, 1);
    assert_eq!(requests[1].version, 0);
    let outcomes = source.credential_outcomes().snmp;
    assert_eq!(outcomes.rejected, 1);
    assert_eq!(outcomes.answered, 0);
    assert_eq!(outcomes.inconclusive, 0);
}

#[tokio::test]
async fn an_unusable_v1_retry_answers_over_v1_and_runs_the_document_over_v1() {
    // Both reads are valid answers that carry no system data: the v2c read
    // authenticated, and the v1 retry is the version the document uses.
    let agent = FakeAgent::spawn(|request| {
        if is_system_request(request) {
            return Some(response(request.version, &request.community, 5, &[]));
        }
        Some(generic_response(request))
    });
    let mut source = test_source(shipped_registry());
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let Probed::Answers(probe) = source.probe(&agent.address, &[candidate], &[]).await else {
        panic!("both versions answered, so the credential authenticated");
    };
    assert!(probe.self_claims.is_empty());
    let requests = agent.requests();
    assert_eq!(requests[0].version, 1);
    assert!(is_system_request(&requests[0]));
    assert_eq!(requests[1].version, 0, "the retry is v1");
    assert!(is_system_request(&requests[1]));
    assert!(
        requests.len() > 2,
        "the selected document runs after the negotiation: {requests:?}"
    );
    assert!(
        requests[2..].iter().all(|request| request.version == 0),
        "the unusable v1 answer wins the negotiation: {requests:?}"
    );
    let outcomes = source.credential_outcomes().snmp;
    assert_eq!(outcomes.answered, 1);
    assert_eq!(outcomes.rejected, 0);
}

#[tokio::test]
async fn a_community_mismatch_is_rejected_without_a_v1_retry() {
    let agent = FakeAgent::spawn(|request| {
        if request.community == b"wrong" {
            return Some(response(request.version, b"another-community", 0, &[]));
        }
        if is_system_request(request) {
            Some(system_response(
                1,
                &request.community,
                b"Linux",
                &AIROS_OBJECT_ID,
                b"airmax-ap",
            ))
        } else {
            Some(generic_response(request))
        }
    });
    let mut source = test_source(shipped_registry());
    let candidates = [
        candidate_at("wrong", SnmpVersion::V2c),
        candidate_at("right", SnmpVersion::V2c),
    ];
    let probed = source.probe(&agent.address, &candidates, &[]).await;
    assert!(matches!(probed, Probed::Answers(_)));
    let requests = agent.requests();
    assert!(
        requests.iter().all(|request| request.version == 1),
        "a rejection never falls back to v1: {requests:?}"
    );
    let outcomes = source.credential_outcomes().snmp;
    assert_eq!(outcomes.rejected, 1);
    assert_eq!(outcomes.answered, 1);
}

#[tokio::test]
async fn a_wrong_community_seen_as_silence_spends_both_versions() {
    let agent = FakeAgent::spawn(|request| {
        if request.community == b"quiet" {
            return None;
        }
        if is_system_request(request) {
            Some(system_response(
                1,
                &request.community,
                b"Linux",
                &AIROS_OBJECT_ID,
                b"airmax-ap",
            ))
        } else {
            Some(generic_response(request))
        }
    });
    let mut source = test_source(shipped_registry());
    let candidates = [
        candidate_at("quiet", SnmpVersion::V2c),
        candidate_at("right", SnmpVersion::V2c),
    ];
    let probed = source.probe(&agent.address, &candidates, &[]).await;
    assert!(matches!(probed, Probed::Answers(_)));
    agent.wait_for(3);
    let versions: Vec<i64> = agent
        .requests()
        .iter()
        .map(|request| request.version)
        .collect();
    assert_eq!(
        versions[..3],
        [1, 0, 1],
        "silence cannot establish rejection, so both versions run"
    );
    let outcomes = source.credential_outcomes().snmp;
    assert_eq!(outcomes.inconclusive, 1);
    assert_eq!(outcomes.answered, 1);
}

#[tokio::test]
async fn both_versions_silent_spend_exactly_two_initial_attempts() {
    let agent = FakeAgent::spawn(|_| None);
    let mut source = test_source(shipped_registry());
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let probed = tokio::time::timeout(
        Duration::from_secs(2),
        source.probe(&agent.address, &[candidate], &[]),
    )
    .await
    .expect("the negotiation must settle, not loop");
    assert!(matches!(probed, Probed::Answers(_)));
    agent.wait_for(2);
    let versions: Vec<i64> = agent
        .requests()
        .iter()
        .map(|request| request.version)
        .collect();
    assert_eq!(versions, [1, 0], "one v2c read, one v1 retry, no more");
    let outcomes = source.credential_outcomes().snmp;
    assert_eq!(outcomes.inconclusive, 1);
    assert_eq!(outcomes.answered, 0);
}

#[tokio::test]
async fn an_explicit_v1_candidate_never_asks_v2c() {
    let agent = FakeAgent::spawn(|request| {
        if request.version != 0 {
            return None;
        }
        if is_system_request(request) {
            Some(system_response(
                0,
                &request.community,
                b"Linux",
                &AIROS_OBJECT_ID,
                b"airmax-ap",
            ))
        } else {
            Some(generic_response(request))
        }
    });
    let mut source = test_source(shipped_registry());
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V1);
    let probed = source.probe(&agent.address, &[candidate], &[]).await;
    assert!(matches!(probed, Probed::Answers(_)));
    let requests = agent.requests();
    assert!(
        requests.iter().all(|request| request.version == 0),
        "an explicit v1 candidate is v1 only: {requests:?}"
    );
    assert_eq!(source.credential_outcomes().snmp.answered, 1);
}

#[tokio::test]
async fn a_blank_and_an_explicit_v2c_target_negotiate_identically() {
    let agent = v1_only_agent();
    let vault = Vault::parse(&format!(
        r#"{{"targets": [
                {{"name": "explicit", "address": "{}", "credential": "c", "snmp_version": "v2c"}},
                {{"name": "blank", "address": "{}", "credential": "c"}}
            ],
            "credentials": [{{"name": "c", "community": "sentinel-community-1"}}]}}"#,
        agent.address, agent.address
    ))
    .expect("fixture should parse");
    let explicit = resolve_target(&vault, "explicit").expect("target should resolve");
    let blank = resolve_target(&vault, "blank").expect("target should resolve");
    assert_eq!(explicit.version, SnmpVersion::V2c);
    assert_eq!(
        blank.version,
        SnmpVersion::V2c,
        "an absent snmp_version is the automatic v2c preference"
    );

    for target in [explicit, blank] {
        let agent = v1_only_agent();
        let mut source = test_source(shipped_registry());
        let candidate = CredentialCandidate::new(
            CredentialClass::Target,
            CredentialOrigin::Target,
            SnmpAccess {
                community: target.community,
                version: target.version,
            },
        );
        let probed = source.probe(&agent.address, &[candidate], &[]).await;
        assert!(matches!(probed, Probed::Answers(_)));
        let requests = agent.requests();
        assert_eq!(requests[0].version, 1);
        assert_eq!(requests[1].version, 0);
        assert_eq!(source.credential_outcomes().snmp.answered, 1);
    }
}

#[tokio::test]
async fn a_default_profile_candidate_negotiates_v1_without_a_target_ref() {
    let agent = v1_only_agent();
    let mut source = test_source(shipped_registry());
    let candidate = CredentialCandidate::new(
        CredentialClass::Default,
        CredentialOrigin::Profile("office-snmp".to_owned()),
        SnmpAccess {
            community: Community::new("sentinel-community-1".to_owned()),
            version: SnmpVersion::V2c,
        },
    );
    let probed = source.probe(&agent.address, &[candidate], &[]).await;
    let Probed::Answers(probe) = probed else {
        panic!("the default profile's community should authenticate");
    };
    agent.wait_for(2);
    let requests = agent.requests();
    assert_eq!(requests[0].version, 1);
    assert_eq!(requests[1].version, 0);
    assert!(!probe.self_claims.is_empty());
    assert_eq!(source.credential_outcomes().snmp.answered, 1);
    assert_eq!(
        source.progress.preferred(Protocol::Snmp),
        Some(&CredentialOrigin::Profile("office-snmp".to_owned())),
        "the profile origin is the walk preference, not the v1 version"
    );
}

#[tokio::test]
async fn cancellation_during_the_v2c_wait_stops_before_v1() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let agent = FakeAgent::spawn(move |_| {
        flag.store(true, Ordering::SeqCst);
        None
    });
    let mut source = NetworkSource::paced_by(Duration::ZERO, cancelled.clone(), shipped_registry());
    source.io.snmp_timeout = Duration::from_secs(5);
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let started = Instant::now();
    let probed = source.probe(&agent.address, &[candidate], &[]).await;
    assert!(matches!(probed, Probed::Cancelled(_)));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "cancellation ends the wait promptly"
    );
    agent.wait_for(1);
    assert_eq!(
        agent.requests().len(),
        1,
        "the v1 retry never starts after a cancellation"
    );
    assert_eq!(
        source.credential_outcomes(),
        CredentialOutcomes::default(),
        "a cancelled negotiation is never tallied"
    );
}

#[tokio::test]
async fn a_cancelled_negotiation_between_versions_starts_no_v1_read() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let agent = FakeAgent::spawn(move |request| {
        if request.version == 1 && is_system_request(request) {
            flag.store(true, Ordering::SeqCst);
            return Some(response(1, &request.community, 5, &[]));
        }
        None
    });
    let io = PacedIo {
        delay: Duration::ZERO,
        cancelled: Some(cancelled),
        snmp_timeout: Duration::from_secs(5),
    };
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let requested = io
        .snmp_attempt(
            &agent.address,
            &candidate.credential,
            &candidate.origin,
            &Registry::default(),
        )
        .await;
    assert!(matches!(requested, Attempted::Cancelled));
    agent.wait_for(1);
    assert_eq!(
        agent.requests().len(),
        1,
        "the between-versions check stops before v1"
    );
}

#[tokio::test]
async fn cancellation_during_the_v1_wait_stops_without_a_tally() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let agent = FakeAgent::spawn(move |request| {
        if request.version == 1 && is_system_request(request) {
            return Some(response(1, &request.community, 5, &[]));
        }
        if request.version == 0 {
            flag.store(true, Ordering::SeqCst);
            return None;
        }
        None
    });
    let mut source = NetworkSource::paced_by(Duration::ZERO, cancelled.clone(), shipped_registry());
    source.io.snmp_timeout = Duration::from_secs(5);
    let candidate = candidate_at("sentinel-community-1", SnmpVersion::V2c);
    let started = Instant::now();
    let probed = source.probe(&agent.address, &[candidate], &[]).await;
    assert!(matches!(probed, Probed::Cancelled(_)));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "cancellation ends the v1 wait promptly"
    );
    agent.wait_for(2);
    assert_eq!(
        agent.requests().len(),
        2,
        "no document query follows a cancelled v1 read"
    );
    assert_eq!(
        source.credential_outcomes(),
        CredentialOutcomes::default(),
        "a cancelled v1 read does not become a rejection"
    );
}

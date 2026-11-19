//! Integration coverage for GETNEXT subtree walking: a scripted fake UDP
//! agent answers each walk step offline. The walk is the protocol proof the
//! document engine's live reader shares, so its retry, cap, v1-absence, and
//! malformed-reply edges stay pinned here.

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dudarchy_poller::{
    ResolvedTarget, SnmpError, SnmpVersion, Vault, WalkOutcome, WalkedVarbind, resolve_target,
    walk_subtree,
};

/// Per-exchange timeout used by every test; generous so slow machines do
/// not flake, while real replies on loopback arrive in microseconds.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Spawns a fake agent answering the nth datagram with `answer(n)`, and
/// returns its address. One thread serves every walk step; the walk opens
/// a fresh socket per step and the agent replies to each sender.
fn fake_agent(answer: impl Fn(usize) -> Vec<u8> + Send + 'static) -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("agent should bind");
    let address = socket.local_addr().expect("agent should have an address");
    socket
        .set_read_timeout(Some(TIMEOUT))
        .expect("agent timeout should apply");
    std::thread::spawn(move || {
        for step in 0.. {
            let mut request = [0u8; 512];
            let Ok((_, from)) = socket.recv_from(&mut request) else {
                return;
            };
            if socket.send_to(&answer(step), from).is_err() {
                return;
            }
        }
    });
    address
}

/// A vault fixture whose target `t` points at `address` with community
/// `public`, resolved for SNMP use.
fn resolved(address: &str) -> ResolvedTarget {
    let vault = Vault::parse(&format!(
        r#"{{"targets": [{{"name": "t", "address": "{address}", "credential": "c"}}],
            "credentials": [{{"name": "c", "community": "public"}}]}}"#
    ))
    .expect("fixture should parse");
    resolve_target(&vault, "t").expect("fixture should resolve")
}

/// One BER TLV; every fixture element is short enough for the short form.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![
        tag,
        u8::try_from(content.len()).expect("fixture fits short form"),
    ];
    out.extend_from_slice(content);
    out
}

fn enc_integer(value: i64) -> Vec<u8> {
    tlv(0x02, &value.to_be_bytes())
}

fn enc_octet_string(bytes: &[u8]) -> Vec<u8> {
    tlv(0x04, bytes)
}

/// BER OID content octets: the first two components packed into one
/// subidentifier, the rest base-128.
fn enc_oid(oid: &[u32]) -> Vec<u8> {
    let mut packed = vec![40 * oid[0] + oid[1]];
    packed.extend_from_slice(&oid[2..]);
    let mut out = Vec::new();
    for &value in &packed {
        let mut chunk = vec![u8::try_from(value & 0x7f).expect("subidentifier fits")];
        let mut rest = value >> 7;
        while rest > 0 {
            chunk.push(u8::try_from(rest & 0x7f).expect("subidentifier fits") | 0x80);
            rest >>= 7;
        }
        chunk.reverse();
        out.extend_from_slice(&chunk);
    }
    out
}

/// One varbind binding `oid` to a pre-encoded value.
fn enc_varbind(oid: &[u32], value: &[u8]) -> Vec<u8> {
    let mut pair = tlv(0x06, &enc_oid(oid));
    pair.extend_from_slice(value);
    tlv(0x30, &pair)
}

/// A v2c response for community `public`, request-id 1, no error, with
/// pre-encoded varbinds.
fn response(varbinds: &[Vec<u8>]) -> Vec<u8> {
    response_at_version(1, 0, varbinds)
}

/// A response for community `public`, request-id 1, at `version`, with
/// `error_status` and pre-encoded varbinds.
fn response_at_version(version: i64, error_status: i64, varbinds: &[Vec<u8>]) -> Vec<u8> {
    let mut bindings = Vec::new();
    for varbind in varbinds {
        bindings.extend_from_slice(varbind);
    }
    let mut pdu = enc_integer(1); // request-id
    pdu.extend(enc_integer(error_status));
    pdu.extend(enc_integer(0)); // error-index
    pdu.extend(tlv(0x30, &bindings));
    let mut message = enc_integer(version);
    message.extend(enc_octet_string(b"public"));
    message.extend(tlv(0xa2, &pdu));
    tlv(0x30, &message)
}

fn end_of_mib_view() -> Vec<u8> {
    tlv(0x82, &[])
}

fn enc_null() -> Vec<u8> {
    tlv(0x05, &[])
}

/// A sample subtree root for the walk tests.
fn sample_table() -> Vec<u32> {
    vec![1, 0, 8802, 1, 1, 2, 1, 4, 1]
}

/// An instance OID of `column` on sample-table row `indices`.
fn sample_row(column: u32, indices: &[u32]) -> Vec<u32> {
    [sample_table().as_slice(), &[1, column], indices].concat()
}

/// A completed subtree walk's bindings; cancels are test failures.
fn complete(walk: WalkOutcome) -> Vec<WalkedVarbind> {
    match walk {
        WalkOutcome::Complete(walked) => walked,
        WalkOutcome::Cancelled(walked) => {
            panic!("the walk should complete, not cancel: {walked:?}")
        }
    }
}

#[tokio::test]
async fn walk_stops_cleanly_at_end_of_mib_view() {
    let agent = fake_agent(|_| {
        response(&[enc_varbind(
            &[1, 3, 6, 1, 2, 1, 1, 5, 0],
            &end_of_mib_view(),
        )])
    });
    let target = resolved(&agent.to_string());
    let walked = complete(
        walk_subtree(
            &target.address,
            &target.community,
            &[1, 3, 6, 1],
            SnmpVersion::V2c,
            TIMEOUT,
            &(|| false),
        )
        .await
        .expect("end of view should end the walk"),
    );
    assert!(walked.is_empty());
}

#[tokio::test]
async fn walk_stops_when_the_reply_leaves_the_subtree() {
    // The first answer is sysName.0: valid, but outside the walked subtree.
    let agent = fake_agent(|_| {
        response(&[enc_varbind(
            &[1, 3, 6, 1, 2, 1, 1, 5, 0],
            &enc_octet_string(b"FSW-Office-Core"),
        )])
    });
    let target = resolved(&agent.to_string());
    let walked = complete(
        walk_subtree(
            &target.address,
            &target.community,
            &sample_table(),
            SnmpVersion::V2c,
            TIMEOUT,
            &(|| false),
        )
        .await
        .expect("leaving the subtree should end the walk"),
    );
    assert!(walked.is_empty());
}

#[tokio::test]
async fn truncated_replies_are_decode_errors() {
    let complete = response(&[enc_varbind(
        &sample_row(5, &[0, 1, 1]),
        &enc_octet_string(&[0x00, 0x1f, 0x5e, 0xaa, 0xbb, 0xcc]),
    )]);
    let agent = fake_agent(move |step| {
        if step == 0 {
            complete[..complete.len() - 3].to_vec()
        } else {
            complete.clone()
        }
    });
    let target = resolved(&agent.to_string());
    let error = walk_subtree(
        &target.address,
        &target.community,
        &sample_table(),
        SnmpVersion::V2c,
        TIMEOUT,
        &(|| false),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, SnmpError::Decode { .. }), "got {error:?}");
}

#[tokio::test]
async fn a_runaway_agent_hits_the_step_cap() {
    // Every answer offers a fresh OID one row further into the subtree.
    let agent = fake_agent(|step| {
        response(&[enc_varbind(
            &sample_row(7, &[0, u32::try_from(step).expect("step fits") + 1]),
            &enc_octet_string(b"ether2"),
        )])
    });
    let target = resolved(&agent.to_string());
    let error = walk_subtree(
        &target.address,
        &target.community,
        &sample_table(),
        SnmpVersion::V2c,
        TIMEOUT,
        &(|| false),
    )
    .await
    .unwrap_err();
    let SnmpError::WalkCapExceeded { steps, .. } = error else {
        panic!("expected WalkCapExceeded, got {error:?}");
    };
    assert_eq!(steps, 10_000);
}

#[tokio::test]
async fn a_v1_walk_ends_cleanly_on_no_such_name() {
    // The v1 script answers with version-0 responses; the walk's second
    // GETNEXT draws error-status `noSuchName` (2) on the un-advanced OID,
    // which ends that subtree with the first row standing. Over v2c the
    // same answer keeps failing: noSuchName is not a v2c dialect value.
    let replies = [
        response_at_version(
            0,
            0,
            &[enc_varbind(
                &sample_row(5, &[0, 0, 1]),
                &enc_octet_string(&[0x00, 0x1f, 0x5e, 0xaa, 0xbb, 0xcc]),
            )],
        ),
        response_at_version(
            0,
            2,
            &[enc_varbind(&sample_row(5, &[0, 0, 1]), &enc_null())],
        ),
    ];
    let agent = fake_agent(move |step| replies[step].clone());
    let target = resolved(&agent.to_string());
    let walked = complete(
        walk_subtree(
            &target.address,
            &target.community,
            &sample_table(),
            SnmpVersion::V1,
            TIMEOUT,
            &(|| false),
        )
        .await
        .expect("noSuchName ends a v1 walk cleanly"),
    );
    assert_eq!(
        walked,
        vec![(
            sample_row(5, &[0, 0, 1]),
            Some(vec![0x00, 0x1f, 0x5e, 0xaa, 0xbb, 0xcc])
        )]
    );

    // A fresh agent for the v2c control: the first noSuchName answer
    // fails the walk.
    let replies = [
        response_at_version(
            1,
            0,
            &[enc_varbind(
                &sample_row(7, &[0, 1, 1]),
                &enc_octet_string(b"ether2"),
            )],
        ),
        response_at_version(
            1,
            2,
            &[enc_varbind(&sample_row(7, &[0, 1, 1]), &enc_null())],
        ),
    ];
    let agent = fake_agent(move |step| replies[step].clone());
    let target = resolved(&agent.to_string());
    let error = walk_subtree(
        &target.address,
        &target.community,
        &sample_table(),
        SnmpVersion::V2c,
        TIMEOUT,
        &(|| false),
    )
    .await
    .unwrap_err();
    let SnmpError::ErrorStatus { status, .. } = error else {
        panic!("expected ErrorStatus over v2c");
    };
    assert_eq!(status, 2);
}

#[tokio::test]
async fn a_v1_walk_decodes_counter64_and_timeticks_values() {
    // walk_subtree keeps only octet strings, but the walk must decode
    // counter tags without failing: a Counter64 and a TimeTicks row pass
    // through (dropped by the string filter) beside a string row.
    let counter64 = tlv(0x46, &[0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0x00, 0x01]);
    let timeticks = tlv(0x43, &[0x00, 0x01, 0xe2, 0x40]);
    let replies = [
        response_at_version(
            0,
            0,
            &[enc_varbind(
                &sample_row(7, &[0, 1, 1]),
                &enc_octet_string(b"ether2"),
            )],
        ),
        response_at_version(0, 0, &[enc_varbind(&sample_row(7, &[0, 1, 2]), &counter64)]),
        response_at_version(0, 0, &[enc_varbind(&sample_row(7, &[0, 1, 3]), &timeticks)]),
        response_at_version(0, 0, &[enc_varbind(&sample_table(), &end_of_mib_view())]),
    ];
    let agent = fake_agent(move |step| replies[step].clone());
    let target = resolved(&agent.to_string());
    let walked = complete(
        walk_subtree(
            &target.address,
            &target.community,
            &sample_table(),
            SnmpVersion::V1,
            TIMEOUT,
            &(|| false),
        )
        .await
        .expect("counter tags decode"),
    );
    // The counter rows decode (Counter64 0xdeadbeefcafe0001, TimeTicks
    // 123456) without failing the v1 walk; walk_subtree's octet-string
    // filter leaves them as `None`, exactly as for v2c non-strings.
    assert_eq!(
        walked,
        vec![
            (sample_row(7, &[0, 1, 1]), Some(b"ether2".to_vec())),
            (sample_row(7, &[0, 1, 2]), None),
            (sample_row(7, &[0, 1, 3]), None),
        ]
    );
}

/// The transport deadline: a silent peer answers nothing, and the walk
/// must surface that as [`SnmpError::Timeout`] within the transaction
/// deadline instead of hanging.
#[tokio::test]
async fn a_silent_peer_hits_its_deadline() {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("peer should bind");
    let target = resolved(&socket.local_addr().expect("peer address").to_string());
    let deadline = Duration::from_millis(300);
    let started = std::time::Instant::now();
    let error = walk_subtree(
        &target.address,
        &target.community,
        &sample_table(),
        SnmpVersion::V2c,
        deadline,
        &(|| false),
    )
    .await
    .expect_err("a silent peer times out");
    assert!(
        matches!(error, SnmpError::Timeout { .. }),
        "expected Timeout, got {error:?}"
    );
    assert!(
        started.elapsed() >= deadline - Duration::from_millis(50),
        "the deadline was honoured, not short-circuited"
    );
    assert!(
        started.elapsed() < TIMEOUT,
        "the deadline ended the wait well before the test timeout"
    );
}

/// Cancellation during a receive wait: aborting the walk's task ends the
/// wait promptly, even against a peer that never answers and a deadline
/// that would run for a minute.
#[tokio::test]
async fn cancellation_ends_a_receive_wait_promptly() {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("peer should bind");
    let target = resolved(&socket.local_addr().expect("peer address").to_string());
    // Drop the walk (cancellation) while it is parked in its receive
    // wait: the sleep wins the race only if the wait ended promptly.
    let started = std::time::Instant::now();
    let root = sample_table();
    let outcome = tokio::select! {
        result = walk_subtree(
            &target.address,
            &target.community,
            &root,
            SnmpVersion::V2c,
            Duration::from_secs(60),
            &(|| false),
        ) => panic!("a silent peer cannot answer: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(100)) => "cancelled",
    };
    assert_eq!(outcome, "cancelled");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "dropping the walk ended the in-flight wait at once"
    );
}

/// A cancelled subtree walk returns what it saw, distinctly: the agent
/// keeps answering, but once the cancellation check fires the walk stops
/// and reports `WalkOutcome::Cancelled` with the rows collected so far.
#[tokio::test]
async fn a_cancelled_subtree_walk_returns_what_it_saw() {
    // An agent that answers every GETNEXT with a fresh in-subtree row.
    let agent = fake_agent(|step| {
        response(&[enc_varbind(
            &sample_row(7, &[0, u32::try_from(step).expect("step fits") + 1]),
            &enc_octet_string(b"ether2"),
        )])
    });
    let target = resolved(&agent.to_string());
    // The agent raises the flag once the walk is under way, so the check
    // between steps fires with rows already collected.
    let cancelled = Arc::new(AtomicBool::new(false));
    let agent_flag = cancelled.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        agent_flag.store(true, Ordering::Relaxed);
    });
    let flag = cancelled.clone();
    let outcome = walk_subtree(
        &target.address,
        &target.community,
        &sample_table(),
        SnmpVersion::V2c,
        TIMEOUT,
        &(move || flag.load(Ordering::Relaxed)),
    )
    .await
    .expect("cancellation is not an error");
    let WalkOutcome::Cancelled(walked) = outcome else {
        panic!("the walk should report its cancellation distinctly");
    };
    assert!(
        !walked.is_empty(),
        "the walk reports the rows it collected before the flag rose"
    );
    assert!(
        walked.len() < 10_000,
        "the walk stopped at the cancellation, not at the step cap"
    );
}

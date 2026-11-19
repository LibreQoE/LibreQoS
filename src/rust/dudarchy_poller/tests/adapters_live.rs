//! The live reader over the real SNMP transport: a scripted fake UDP agent
//! on loopback answers arbitrary-OID gets and walks, and the reader reports
//! each of the engine's six typed outcomes. Cancellation is proven to reach a
//! read that is already in flight.

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use dudarchy_poller::adapters::CommandName;
use dudarchy_poller::adapters::live::SnmpReader;
use dudarchy_poller::adapters::reader::{Outcome, Reader, Varbind};
use dudarchy_poller::{Community, SnmpAccess, SnmpVersion, WalkValue};

/// The deadline for tests that expect an answer.
const TIMEOUT: Duration = Duration::from_secs(5);

/// `sysName.0`.
fn sys_name() -> Vec<u32> {
    vec![1, 3, 6, 1, 2, 1, 1, 5, 0]
}

/// `sysDescr.0`.
fn sys_descr() -> Vec<u32> {
    vec![1, 3, 6, 1, 2, 1, 1, 1, 0]
}

/// `lldpRemTable`.
fn lldp_rem_table() -> Vec<u32> {
    vec![1, 0, 8802, 1, 1, 2, 1, 4, 1]
}

/// One LLDP remote-systems instance: `lldpRemSysName.0.1.1`.
fn lldp_sys_name_instance() -> Vec<u32> {
    vec![1, 0, 8802, 1, 1, 2, 1, 4, 1, 1, 9, 0, 1, 1]
}

/// Spawns a fake agent answering the nth datagram with `answer(n)`, and
/// returns its address. One thread serves every request; the reader opens a
/// fresh socket per exchange and the agent replies to each sender.
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

/// Community `public` at `version`, the access every live test resolves.
fn access(version: SnmpVersion) -> SnmpAccess {
    SnmpAccess {
        community: Community::new("public".to_owned()),
        version,
    }
}

/// A reader for `address` over `access`.
fn live_reader<'a>(
    address: &SocketAddr,
    access: &'a SnmpAccess,
    timeout: Duration,
    cancel: &'a (dyn Fn() -> bool + Send + Sync),
) -> SnmpReader<'a> {
    SnmpReader::new(address.to_string(), access, timeout, cancel)
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

fn no_such_object() -> Vec<u8> {
    tlv(0x80, &[])
}

fn end_of_mib_view() -> Vec<u8> {
    tlv(0x82, &[])
}

#[tokio::test]
async fn a_get_returns_the_answered_varbind_for_any_oid() {
    let agent = fake_agent(|_| response(&[enc_varbind(&sys_name(), &enc_octet_string(b"edge"))]));
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&agent, &v2c, TIMEOUT, &|| false);

    assert_eq!(
        reader.get(&sys_name()).await,
        Outcome::Present(vec![Varbind {
            oid: sys_name(),
            value: WalkValue::Bytes(b"edge".to_vec()),
        }])
    );
}

#[tokio::test]
async fn a_get_the_agent_declines_is_absent() {
    let agent = fake_agent(|_| response(&[enc_varbind(&sys_name(), &no_such_object())]));
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&agent, &v2c, TIMEOUT, &|| false);

    assert_eq!(reader.get(&sys_name()).await, Outcome::Absent);
}

#[tokio::test]
async fn a_walk_returns_rows_and_ends_on_end_of_mib_view() {
    let instance = lldp_sys_name_instance();
    let answered = instance.clone();
    let agent = fake_agent(move |step| match step {
        0 => response(&[enc_varbind(&answered, &enc_octet_string(b"neighbor-1"))]),
        _ => response(&[enc_varbind(&answered, &end_of_mib_view())]),
    });
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&agent, &v2c, TIMEOUT, &|| false);

    assert_eq!(
        reader.walk(&lldp_rem_table()).await,
        Outcome::Present(vec![Varbind {
            oid: instance,
            value: WalkValue::Bytes(b"neighbor-1".to_vec()),
        }])
    );
}

#[tokio::test]
async fn a_walk_with_no_rows_is_empty_and_a_declined_table_is_absent() {
    let agent = fake_agent(|_| response(&[enc_varbind(&lldp_rem_table(), &end_of_mib_view())]));
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&agent, &v2c, TIMEOUT, &|| false);
    assert_eq!(reader.walk(&lldp_rem_table()).await, Outcome::Empty);

    let agent = fake_agent(|_| response(&[enc_varbind(&lldp_rem_table(), &no_such_object())]));
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&agent, &v2c, TIMEOUT, &|| false);
    assert_eq!(reader.walk(&lldp_rem_table()).await, Outcome::Absent);
}

#[tokio::test]
async fn a_missed_deadline_is_a_timeout() {
    let silent = UdpSocket::bind("127.0.0.1:0").expect("silent agent should bind");
    let address = silent.local_addr().expect("silent agent address");
    let deadline = Duration::from_millis(200);
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&address, &v2c, deadline, &|| false);

    assert_eq!(reader.get(&sys_name()).await, Outcome::Timeout);
    assert_eq!(reader.walk(&lldp_rem_table()).await, Outcome::Timeout);
}

#[tokio::test]
async fn an_agent_error_and_a_bad_reply_are_errors_with_their_reason() {
    let agent = fake_agent(|step| match step {
        0 => response_at_version(1, 5, &[]),
        1 => b"not an SNMP message".to_vec(),
        _ => response(&[enc_varbind(&sys_descr(), &enc_octet_string(b"wrong"))]),
    });
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&agent, &v2c, TIMEOUT, &|| false);

    match reader.get(&sys_name()).await {
        Outcome::Error { reason } => assert!(reason.contains("error-status 5"), "{reason}"),
        other => panic!("expected the agent's error, got {other:?}"),
    }
    match reader.get(&sys_name()).await {
        Outcome::Error { reason } => {
            assert!(reason.contains("malformed SNMP response"), "{reason}")
        }
        other => panic!("expected a decode error, got {other:?}"),
    }
    // A reply bound to another object must not be read as this scalar.
    match reader.get(&sys_name()).await {
        Outcome::Error { reason } => assert!(reason.contains("another object's oid"), "{reason}"),
        other => panic!("expected a mismatched-oid error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_rising_flag_cancels_an_in_flight_get() {
    let silent = UdpSocket::bind("127.0.0.1:0").expect("silent agent should bind");
    let address = silent.local_addr().expect("silent agent address");
    let flag = Arc::new(AtomicBool::new(false));
    let rising = Arc::clone(&flag);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        rising.store(true, Ordering::SeqCst);
    });
    let cancel = || flag.load(Ordering::SeqCst);
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&address, &v2c, Duration::from_secs(30), &cancel);

    let started = Instant::now();
    assert_eq!(reader.get(&sys_name()).await, Outcome::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancellation ended the in-flight wait promptly"
    );
}

#[tokio::test]
async fn a_rising_flag_cancels_an_in_flight_walk() {
    let silent = UdpSocket::bind("127.0.0.1:0").expect("silent agent should bind");
    let address = silent.local_addr().expect("silent agent address");
    let flag = Arc::new(AtomicBool::new(false));
    let rising = Arc::clone(&flag);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        rising.store(true, Ordering::SeqCst);
    });
    let cancel = || flag.load(Ordering::SeqCst);
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&address, &v2c, Duration::from_secs(30), &cancel);

    let started = Instant::now();
    assert_eq!(reader.walk(&lldp_rem_table()).await, Outcome::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancellation ended the in-flight walk promptly"
    );
}

#[tokio::test]
async fn a_flag_already_up_stops_the_read_before_it_starts() {
    let silent = UdpSocket::bind("127.0.0.1:0").expect("silent agent should bind");
    let address = silent.local_addr().expect("silent agent address");
    let cancel = || true;
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&address, &v2c, Duration::from_secs(30), &cancel);

    let started = Instant::now();
    assert_eq!(reader.get(&sys_name()).await, Outcome::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a cancelled job issues no request"
    );
}

#[tokio::test]
async fn named_commands_are_not_snmp() {
    let silent = UdpSocket::bind("127.0.0.1:0").expect("silent agent should bind");
    let address = silent.local_addr().expect("silent agent address");
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&address, &v2c, TIMEOUT, &|| false);

    match reader.command(CommandName::SystemIdentity).await {
        Outcome::Error { reason } => assert!(reason.contains("RouterOS API"), "{reason}"),
        other => panic!("expected an unsupported-command error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_v1_get_with_no_such_name_is_absent() {
    // SNMP v1 has no exception values: a missing object answers the request
    // with error-status 2, which is an absence, not a transport failure.
    let agent = fake_agent(|_| response_at_version(0, 2, &[]));
    let v1 = access(SnmpVersion::V1);
    let mut reader = live_reader(&agent, &v1, TIMEOUT, &|| false);

    assert_eq!(reader.get(&sys_name()).await, Outcome::Absent);
}

#[tokio::test]
async fn a_walk_with_a_malformed_reply_is_an_error() {
    let agent = fake_agent(|_| b"not an SNMP message".to_vec());
    let v2c = access(SnmpVersion::V2c);
    let mut reader = live_reader(&agent, &v2c, TIMEOUT, &|| false);

    match reader.walk(&lldp_rem_table()).await {
        Outcome::Error { reason } => {
            assert!(reason.contains("malformed SNMP response"), "{reason}")
        }
        other => panic!("expected a decode error, got {other:?}"),
    }
}

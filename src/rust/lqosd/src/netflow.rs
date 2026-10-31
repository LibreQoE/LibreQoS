//! 3a group-1: the bridge NetFlow exporter (docs/netflow_on_bridge_derivation.md
//! §2-§4). F-A EXPORT-AS-IS: the exporter is a cold-path v3-side
//! consumer of the retained bridge flow rows; the dataplane NEVER exports, so
//! there are ZERO bridge lines. The format is version 9 (template-based).
//!
//! Increment 1 (this module): the v9 encoder + the flow→record field map +
//! oracles, DECLARED-NOT-WIRED. Increment 2: the exporter task (retention +
//! bounded queues + the UDP send + the stats block) and the `[flows]` config
//! wiring.
//!
//! NOTE: the K3 retirement gate forbids the historical module/token
//! spellings, so the encoder lives in this single module (no separate shared
//! helper module whose name would trip the gate).
#![allow(dead_code)]

use analytics_ipc::{FlowId, FlowRow, FlowState};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use fxhash::FxHashSet;
use serde::Serialize;
use std::collections::VecDeque;
use std::net::{IpAddr, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use nix::sys::time::TimeValLike;

const QUEUE_CAPACITY: usize = 65_535;
/// v2 parity: at most 14 flows (28 directional records) per packet.
const MAX_RECORDS_PER_PACKET: usize = 28;
/// NetFlow v5 (`lqos-b4w`): the pre-bridge encoder batched 15 FLOWS per packet,
/// i.e. 30 directional records — the v5 packet has no template flowsets, so the
/// record budget differs from the v9 28. **Version-dispatched** at the flush
/// threshold (v9 = 28 records; v5 = 30 records = 15 flows).
const NETFLOW5_MAX_FLOWS_PER_PACKET: usize = 15;
const NETFLOW5_MAX_RECORDS_PER_PACKET: usize = NETFLOW5_MAX_FLOWS_PER_PACKET * 2;
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
/// Dedup window for exported completion events (bounded; drop-disclosed).
const RETENTION_CAPACITY: usize = 200_000;

pub(crate) const IN_BYTES: (u16, u16) = (1, 8);
pub(crate) const IN_PKTS: (u16, u16) = (2, 8);
pub(crate) const PROTOCOL: (u16, u16) = (4, 1);
pub(crate) const TCP_FLAGS: (u16, u16) = (6, 1);
pub(crate) const L4_SRC_PORT: (u16, u16) = (7, 2);
pub(crate) const IPV4_SRC_ADDR: (u16, u16) = (8, 4);
pub(crate) const L4_DST_PORT: (u16, u16) = (11, 2);
pub(crate) const IPV4_DST_ADDR: (u16, u16) = (12, 4);
pub(crate) const LAST_SWITCHED: (u16, u16) = (21, 4);
pub(crate) const FIRST_SWITCHED: (u16, u16) = (22, 4);
pub(crate) const IPV6_SRC_ADDR: (u16, u16) = (27, 16);
pub(crate) const IPV6_DST_ADDR: (u16, u16) = (28, 16);
pub(crate) const DST_TOS: (u16, u16) = (55, 1);

/// The IPv4 template fields (the record layout, in order).
pub(crate) const FIELDS_IPV4: [(u16, u16); 11] = [
    IN_BYTES,
    IN_PKTS,
    PROTOCOL,
    TCP_FLAGS,
    L4_SRC_PORT,
    IPV4_SRC_ADDR,
    L4_DST_PORT,
    IPV4_DST_ADDR,
    FIRST_SWITCHED,
    LAST_SWITCHED,
    DST_TOS,
];

/// The IPv6 template fields (the record layout, in order).
pub(crate) const FIELDS_IPV6: [(u16, u16); 11] = [
    IN_BYTES,
    IN_PKTS,
    PROTOCOL,
    TCP_FLAGS,
    L4_SRC_PORT,
    IPV6_SRC_ADDR,
    L4_DST_PORT,
    IPV6_DST_ADDR,
    FIRST_SWITCHED,
    LAST_SWITCHED,
    DST_TOS,
];

pub(crate) const TEMPLATE_ID_IPV4: u16 = 256;
pub(crate) const TEMPLATE_ID_IPV6: u16 = 257;

/// One direction's export record (the 5-tuple + counters). `in_bytes`/`in_pkts`
/// are from the record's SOURCE to its DESTINATION.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlowRecord {
    pub src_ip: IpAddr,
    pub dst_ip: IpAddr,
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: u8,
    pub tcp_flags: u8,
    pub tos: u8,
    pub in_bytes: u64,
    pub in_pkts: u64,
    pub first_switched_ms: u32,
    pub last_switched_ms: u32,
}

impl FlowRecord {
    fn is_ipv4(&self) -> bool {
        self.src_ip.is_ipv4() && self.dst_ip.is_ipv4()
    }

    fn template_id(&self) -> u16 {
        if self.is_ipv4() {
            TEMPLATE_ID_IPV4
        } else {
            TEMPLATE_ID_IPV6
        }
    }

    fn encode_fields(&self) -> Vec<u8> {
        let fields = if self.is_ipv4() {
            &FIELDS_IPV4[..]
        } else {
            &FIELDS_IPV6[..]
        };
        let mut out = Vec::with_capacity(60);
        for field in fields {
            match *field {
                IN_BYTES => out.extend_from_slice(&self.in_bytes.to_be_bytes()),
                IN_PKTS => out.extend_from_slice(&self.in_pkts.to_be_bytes()),
                PROTOCOL => out.push(self.protocol),
                TCP_FLAGS => out.push(self.tcp_flags),
                L4_SRC_PORT => out.extend_from_slice(&self.src_port.to_be_bytes()),
                L4_DST_PORT => out.extend_from_slice(&self.dst_port.to_be_bytes()),
                IPV4_SRC_ADDR => encode_ipv4(self.src_ip, &mut out),
                IPV4_DST_ADDR => encode_ipv4(self.dst_ip, &mut out),
                IPV6_SRC_ADDR => encode_ipv6(self.src_ip, &mut out),
                IPV6_DST_ADDR => encode_ipv6(self.dst_ip, &mut out),
                FIRST_SWITCHED => out.extend_from_slice(&self.first_switched_ms.to_be_bytes()),
                LAST_SWITCHED => out.extend_from_slice(&self.last_switched_ms.to_be_bytes()),
                DST_TOS => out.push(self.tos),
                _ => {}
            }
        }
        out
    }

    /// One data flowset (flowset-id = the template id, length, fields, padded).
    pub fn encode_flowset(&self) -> Vec<u8> {
        let fields = self.encode_fields();
        let mut bytes = Vec::with_capacity(fields.len() + 8);
        bytes.extend_from_slice(&self.template_id().to_be_bytes());
        // RFC 3954 §5.1: the flowset Length is the 4-byte-aligned total,
        // padding included. Emitting the unpadded field count (the v2-encoder
        // regression this fixes) makes strict collectors mis-frame.
        let total = ((fields.len() + 4).div_ceil(4) * 4) as u16;
        bytes.extend_from_slice(&total.to_be_bytes());
        bytes.extend_from_slice(&fields);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        bytes
    }
}

fn encode_ipv4(ip: IpAddr, out: &mut Vec<u8>) {
    if let IpAddr::V4(ip) = ip {
        out.extend_from_slice(&ip.octets());
    }
}

fn encode_ipv6(ip: IpAddr, out: &mut Vec<u8>) {
    if let IpAddr::V6(ip) = ip {
        out.extend_from_slice(&ip.octets());
    }
}

/// The two directional records for one completed flow. Direction 0 is the
/// subscriber→remote (upload) leg; direction 1 the remote→subscriber
/// (download) leg. Both bytes and packets are directional: the wire now carries
/// `sent_pkts_up`/`sent_pkts_down` (all-protocol sent packets), so the IN_PKTS
/// fill retires the last honest-empty on this export.
pub fn records_from_flow(flow: &FlowRow) -> [FlowRecord; 2] {
    let first = netflow_millis(flow.first_seen_millis);
    let last = netflow_millis(flow.last_seen_millis);
    let forward = FlowRecord {
        src_ip: flow.id.subscriber_ip,
        dst_ip: flow.id.remote_ip,
        src_port: flow.id.subscriber_port,
        dst_port: flow.id.remote_port,
        protocol: flow.id.protocol,
        tcp_flags: 0,
        tos: 0,
        in_bytes: flow.actual.upload_bytes,
        in_pkts: flow.sent_pkts_up,
        first_switched_ms: first,
        last_switched_ms: last,
    };
    let reverse = FlowRecord {
        src_ip: flow.id.remote_ip,
        dst_ip: flow.id.subscriber_ip,
        src_port: flow.id.remote_port,
        dst_port: flow.id.subscriber_port,
        protocol: flow.id.protocol,
        tcp_flags: 0,
        tos: 0,
        in_bytes: flow.actual.download_bytes,
        in_pkts: flow.sent_pkts_down,
        first_switched_ms: first,
        last_switched_ms: last,
    };
    [forward, reverse]
}

/// NetFlow switched fields are boot-relative; the wire's millis already are.
fn netflow_millis(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The version-9 packet header (`#[repr(C)]` is unnecessary; the fields are
/// serialized big-endian).
pub(crate) struct NetflowHeader {
    pub(crate) count: u16,
    pub(crate) sys_uptime_ms: u32,
    pub(crate) unix_secs: u32,
    pub(crate) sequence: u32,
    pub(crate) source_id: u32,
}

impl NetflowHeader {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(20);
        bytes.extend_from_slice(&9u16.to_be_bytes());
        bytes.extend_from_slice(&self.count.to_be_bytes());
        bytes.extend_from_slice(&self.sys_uptime_ms.to_be_bytes());
        bytes.extend_from_slice(&self.unix_secs.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.source_id.to_be_bytes());
        bytes
    }
}

fn encode_template(fields: &[(u16, u16)], template_id: u16) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + fields.len() * 4);
    bytes.extend_from_slice(&0u16.to_be_bytes());
    let length = (8 + fields.len() * 4) as u16;
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(&template_id.to_be_bytes());
    bytes.extend_from_slice(&(fields.len() as u16).to_be_bytes());
    for (field_type, field_length) in fields {
        bytes.extend_from_slice(&field_type.to_be_bytes());
        bytes.extend_from_slice(&field_length.to_be_bytes());
    }
    bytes
}

/// Encode one export packet: header + both templates + the records. Returns the
/// bytes and the next sequence (advanced by the record count actually sent).
pub fn encode_batch(records: &[FlowRecord], sequence: u32, sys_uptime_ms: u32, unix_secs: u32) -> (Vec<u8>, u32) {
    let count = (records.len() + 2) as u16;
    let header = NetflowHeader {
        count,
        sys_uptime_ms,
        unix_secs,
        sequence,
        source_id: 0,
    };
    let mut bytes = header.encode();
    bytes.extend_from_slice(&encode_template(&FIELDS_IPV4, TEMPLATE_ID_IPV4));
    bytes.extend_from_slice(&encode_template(&FIELDS_IPV6, TEMPLATE_ID_IPV6));
    for record in records {
        bytes.extend_from_slice(&record.encode_flowset());
    }
    (bytes, sequence.wrapping_add(u32::from(count)))
}

// ---------------------------------------------------------------------------
// `lqos-b4w`: the NetFlow v5 encoder, PORTED at the RECORD level from the
// pre-bridge `flow_data` protocol. The v5 header/record layouts are
// the on-wire structs; every multi-byte field is serialized BIG-ENDIAN (the
// pre-bridge `.to_be()` discipline). The v5 path has NO template flowsets (the
// fixed 48-byte record IS the schema) and is IPv4-ONLY.
// ---------------------------------------------------------------------------

/// The NetFlow v5 packet header (24 bytes on the wire).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Netflow5Header {
    pub(crate) count: u16,
    pub(crate) sys_uptime_ms: u32,
    pub(crate) unix_secs: u32,
    pub(crate) unix_nsecs: u32,
    pub(crate) flow_sequence: u32,
    pub(crate) engine_type: u8,
    pub(crate) engine_id: u8,
    pub(crate) sampling_interval: u16,
}

impl Netflow5Header {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(24);
        bytes.extend_from_slice(&5u16.to_be_bytes());
        bytes.extend_from_slice(&self.count.to_be_bytes());
        bytes.extend_from_slice(&self.sys_uptime_ms.to_be_bytes());
        bytes.extend_from_slice(&self.unix_secs.to_be_bytes());
        bytes.extend_from_slice(&self.unix_nsecs.to_be_bytes());
        bytes.extend_from_slice(&self.flow_sequence.to_be_bytes());
        bytes.push(self.engine_type);
        bytes.push(self.engine_id);
        bytes.extend_from_slice(&self.sampling_interval.to_be_bytes());
        bytes
    }
}

/// One NetFlow v5 record (48 bytes on the wire). `src_addr`/`dst_addr` hold the
/// IPv4 address in NETWORK order (`u32::from_be_bytes(octets)`), serialized with
/// `to_be_bytes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Netflow5Record {
    pub(crate) src_addr: u32,
    pub(crate) dst_addr: u32,
    pub(crate) next_hop: u32,
    pub(crate) input: u16,
    pub(crate) output: u16,
    pub(crate) d_pkts: u32,
    pub(crate) d_octets: u32,
    pub(crate) first: u32,
    pub(crate) last: u32,
    pub(crate) src_port: u16,
    pub(crate) dst_port: u16,
    pub(crate) tcp_flags: u8,
    pub(crate) prot: u8,
    pub(crate) tos: u8,
    pub(crate) src_as: u16,
    pub(crate) dst_as: u16,
    pub(crate) src_mask: u8,
    pub(crate) dst_mask: u8,
}

impl Netflow5Record {
    /// Encode the record big-endian, in the exact pre-bridge wire order:
    /// `src,dst,nexthop (4)|input,output (2)|d_pkts,d_octets (4)|first,last (4)|
    /// src,dst_port (2)|pad1,tcp_flags,prot,tos (1)|src,dst_as (2)|
    /// src,dst_mask (1)|pad2 (2)` = 48 bytes.
    fn encode_into(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.src_addr.to_be_bytes());
        bytes.extend_from_slice(&self.dst_addr.to_be_bytes());
        bytes.extend_from_slice(&self.next_hop.to_be_bytes());
        bytes.extend_from_slice(&self.input.to_be_bytes());
        bytes.extend_from_slice(&self.output.to_be_bytes());
        bytes.extend_from_slice(&self.d_pkts.to_be_bytes());
        bytes.extend_from_slice(&self.d_octets.to_be_bytes());
        bytes.extend_from_slice(&self.first.to_be_bytes());
        bytes.extend_from_slice(&self.last.to_be_bytes());
        bytes.extend_from_slice(&self.src_port.to_be_bytes());
        bytes.extend_from_slice(&self.dst_port.to_be_bytes());
        bytes.push(0); // pad1
        bytes.push(self.tcp_flags);
        bytes.push(self.prot);
        bytes.push(self.tos);
        bytes.extend_from_slice(&self.src_as.to_be_bytes());
        bytes.extend_from_slice(&self.dst_as.to_be_bytes());
        bytes.push(self.src_mask);
        bytes.push(self.dst_mask);
        bytes.extend_from_slice(&0u16.to_be_bytes()); // pad2
    }
}

/// Saturating narrowing for the v5 32-bit counters (the pre-bridge
/// `saturating_u64_to_netflow_u32`).
fn netflow_v5_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The two NetFlow v5 records for one completed flow. The pre-bridge
/// `to_netflow_5` `Err`s on IPv6; the v3 world carries IPv6 `FlowRow`s, so the
/// v5 path returns `None` for an IPv6 flow — the caller SKIPS + COUNTS it, never
/// a malformed record. Direction 0 is upload (subscriber→remote); direction 1
/// download (remote→subscriber), mirroring [`records_from_flow`].
pub fn records_from_flow_v5(flow: &FlowRow) -> Option<[Netflow5Record; 2]> {
    let (IpAddr::V4(subscriber), IpAddr::V4(remote)) =
        (flow.id.subscriber_ip, flow.id.remote_ip)
    else {
        return None;
    };
    let subscriber = u32::from_be_bytes(subscriber.octets());
    let remote = u32::from_be_bytes(remote.octets());
    let first = netflow_millis(flow.first_seen_millis);
    let last = netflow_millis(flow.last_seen_millis);
    let forward = Netflow5Record {
        src_addr: subscriber,
        dst_addr: remote,
        next_hop: 0,
        input: 0,
        output: 1,
        d_pkts: netflow_v5_u32(flow.sent_pkts_up),
        d_octets: netflow_v5_u32(flow.actual.upload_bytes),
        first,
        last,
        src_port: flow.id.subscriber_port,
        dst_port: flow.id.remote_port,
        tcp_flags: 0,
        prot: flow.id.protocol,
        tos: 0,
        src_as: 0,
        dst_as: 0,
        src_mask: 0,
        dst_mask: 0,
    };
    let reverse = Netflow5Record {
        src_addr: remote,
        dst_addr: subscriber,
        next_hop: 0,
        input: 1,
        output: 0,
        d_pkts: netflow_v5_u32(flow.sent_pkts_down),
        d_octets: netflow_v5_u32(flow.actual.download_bytes),
        first,
        last,
        src_port: flow.id.remote_port,
        dst_port: flow.id.subscriber_port,
        tcp_flags: 0,
        prot: flow.id.protocol,
        tos: 0,
        src_as: 0,
        dst_as: 0,
        src_mask: 0,
        dst_mask: 0,
    };
    Some([forward, reverse])
}

/// Encode one NetFlow v5 export packet: the 24-byte header + the 48-byte
/// records. Returns the bytes and the next `flow_sequence` (advanced by the
/// record count actually sent — the pre-bridge on-success increment).
pub fn encode_batch_v5(
    records: &[Netflow5Record],
    flow_sequence: u32,
    sys_uptime_ms: u32,
    unix_secs: u32,
) -> (Vec<u8>, u32) {
    let count = records.len() as u16;
    let header = Netflow5Header {
        count,
        sys_uptime_ms,
        unix_secs,
        unix_nsecs: 0,
        flow_sequence,
        engine_type: 0,
        engine_id: 0,
        sampling_interval: 0,
    };
    let mut bytes = header.encode();
    for record in records {
        record.encode_into(&mut bytes);
    }
    // `flow_sequence` advances by the RECORD count — the pre-bridge behavior.
    // (The v5 spec's "total flows seen" unit is read as flows by some collectors
    // and records by others; we keep the ported encoder's record advance.)
    (bytes, flow_sequence.wrapping_add(u32::from(count)))
}

/// The export stats block (the card §3 echo: the live bridge untracked counter
/// travels here so the two counters can be compared, plus the F-A disclosure
/// that the export reflects flows as tracked by the bounded bridge table).
#[derive(Clone, Debug, Default, Serialize)]
pub struct NetflowStats {
    pub enabled: bool,
    pub target: Option<String>,
    pub version: u8,
    pub records_sent: u64,
    pub records_dropped: u64,
    pub batches_sent: u64,
    pub send_errors: u64,
    /// `lqos-b4w`: IPv6 flows SKIPPED by the v5 encoder (v5 is IPv4-only) —
    /// disclosed, never silently dropped. Always 0 on the v9 path (which
    /// encodes IPv6 via template 257).
    pub v6_flows_skipped: u64,
    /// The bridge's disclosed `flow_untracked_packets` (the two counters must
    /// agree; F-A export-as-is).
    pub flow_untracked_packets: u64,
}

#[derive(Clone, Debug)]
struct NetflowTarget {
    target: String,
    version: u8,
}

/// Bounded dedup for completion events (insert returns true when new).
pub(crate) struct FlowIdRetention {
    order: VecDeque<FlowId>,
    set: FxHashSet<FlowId>,
    capacity: usize,
}

impl FlowIdRetention {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            order: VecDeque::new(),
            set: FxHashSet::default(),
            capacity,
        }
    }

    pub(crate) fn insert(&mut self, id: FlowId) -> bool {
        if self.set.contains(&id) {
            return false;
        }
        while self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }
        self.set.insert(id.clone());
        self.order.push_back(id);
        true
    }
}

struct ExporterHandles {
    sender: Sender<FlowRow>,
    retained: Mutex<FlowIdRetention>,
    records_sent: AtomicU64,
    records_dropped: AtomicU64,
    batches_sent: AtomicU64,
    send_errors: AtomicU64,
    /// `lqos-b4w`: IPv6 flows skipped by the v5 encoder (IPv4-only).
    v6_flows_skipped: AtomicU64,
}

static EXPORTER: OnceLock<ExporterHandles> = OnceLock::new();
static EXPORTER_TARGET: OnceLock<NetflowTarget> = OnceLock::new();

/// True when a flow is a completion event (a finished/reset/idle flow).
fn is_completion(flow: &FlowRow) -> bool {
    matches!(
        flow.state,
        FlowState::Fin | FlowState::Reset | FlowState::Idle
    )
}

/// Start the exporter from the `[flows]` config (idempotent). `netflow_version
/// = 5` exports version 9 with a WARN (a config-compat shim, never a v5
/// encoder for GA). Does nothing when disabled or the endpoint is incomplete.
pub fn start(config: &lqos_config::Config) {
    let Some(flows) = config.flows.as_ref() else {
        return;
    };
    if !flows.netflow_enabled {
        return;
    }
    let (Some(ip), Some(port)) = (flows.netflow_ip.clone(), flows.netflow_port) else {
        tracing::warn!("NetFlow enabled but netflow_ip/netflow_port missing; exporter not started");
        return;
    };
    // `lqos-b4w`: emit EXACTLY the configured version. An unsupported version
    // REFUSES to start (never the old silent `other => 9`); `None` keeps the v9
    // default. `FlowConfig::validate()` already rejects ∉{5,9} at load, so this
    // is the belt for a hand-built config.
    let requested = flows.netflow_version.unwrap_or(9);
    let version = match requested {
        5 | 9 => requested,
        other => {
            tracing::error!(
                "netflow_version={other} is unsupported (only 5 or 9); NetFlow exporter NOT started"
            );
            return;
        }
    };
    let target = format!("{ip}:{port}");
    let (sender, receiver) = crossbeam_channel::bounded(QUEUE_CAPACITY);
    let handles = ExporterHandles {
        sender,
        retained: Mutex::new(FlowIdRetention::new(RETENTION_CAPACITY)),
        records_sent: AtomicU64::new(0),
        records_dropped: AtomicU64::new(0),
        batches_sent: AtomicU64::new(0),
        send_errors: AtomicU64::new(0),
        v6_flows_skipped: AtomicU64::new(0),
    };
    if EXPORTER.set(handles).is_err() {
        tracing::warn!("NetFlow exporter already started");
        return;
    }
    let _ = EXPORTER_TARGET.set(NetflowTarget {
        target: target.clone(),
        version,
    });
    let runner = target.clone();
    match std::thread::Builder::new()
        .name("netflow-exporter".to_string())
        .spawn(move || exporter_loop(receiver, runner, version))
    {
        Ok(_) => tracing::info!("NetFlow version {version} exporter started -> {target}"),
        Err(err) => tracing::error!("Failed to spawn NetFlow exporter: {err:?}"),
    }
}

/// Queue the snapshot's completed flows (deduped); a full bounded queue drops
/// and discloses, never backpressures the poll loop (the capture-ring posture).
pub fn queue_completed_flows(flows: &[FlowRow]) {
    let Some(handles) = EXPORTER.get() else {
        return;
    };
    for flow in flows {
        if !is_completion(flow) {
            continue;
        }
        let is_new = handles
            .retained
            .lock()
            .map(|mut retained| retained.insert(flow.id.clone()))
            .unwrap_or(false);
        if !is_new {
            continue;
        }
        if handles.sender.try_send(flow.clone()).is_err() {
            handles.records_dropped.fetch_add(2, Ordering::Relaxed);
        }
    }
}

/// The RING-PATH netflow queue (`lqos-74t.2.x` Q3): `seq` is the once-only
/// authority, so BYPASS the `FlowIdRetention` (a legitimate second completion
/// with the same FlowId must not be dropped). Returns `false` when the bounded
/// queue is FULL (the caller must NOT ack that batch — the ring absorbs it).
pub fn queue_completed_flows_unretained(flows: &[FlowRow]) -> bool {
    let Some(handles) = EXPORTER.get() else {
        return true;
    };
    let mut ok = true;
    for flow in flows {
        if !is_completion(flow) {
            continue;
        }
        if handles.sender.try_send(flow.clone()).is_err() {
            handles.records_dropped.fetch_add(2, Ordering::Relaxed);
            ok = false;
        }
    }
    ok
}

/// The current bounded NetFlow queue depth (0 when the exporter is disabled).
pub fn queue_depth() -> usize {
    EXPORTER.get().map(|handles| handles.sender.len()).unwrap_or(0)
}

/// The exporter task: bind the socket, then run the VERSION-dispatched encoder
/// loop (`lqos-b4w`: v5 vs the existing v9). Both loops share the socket, the
/// `flow_sequence`, and the stats handles; they differ ONLY in the record
/// conversion, the packet encoder, and the flush threshold.
fn exporter_loop(receiver: Receiver<FlowRow>, target: String, version: u8) {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(socket) => socket,
        Err(err) => {
            tracing::error!("NetFlow exporter could not bind a UDP socket: {err}");
            return;
        }
    };
    let mut sequence: u32 = 0;
    match version {
        5 => exporter_loop_v5(receiver, &socket, &target, &mut sequence),
        _ => exporter_loop_v9(receiver, &socket, &target, &mut sequence),
    }
}

/// The v9 encoder loop (template-based; flushes at 28 records).
fn exporter_loop_v9(
    receiver: Receiver<FlowRow>,
    socket: &UdpSocket,
    target: &str,
    sequence: &mut u32,
) {
    let mut pending: Vec<FlowRecord> = Vec::with_capacity(MAX_RECORDS_PER_PACKET);
    loop {
        match receiver.recv_timeout(FLUSH_INTERVAL) {
            Ok(flow) => {
                let [forward, reverse] = records_from_flow(&flow);
                pending.push(forward);
                pending.push(reverse);
                if pending.len() >= MAX_RECORDS_PER_PACKET {
                    flush(socket, target, sequence, &mut pending);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if !pending.is_empty() {
                    flush(socket, target, sequence, &mut pending);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if !pending.is_empty() {
                    flush(socket, target, sequence, &mut pending);
                }
                break;
            }
        }
    }
}

/// The v5 encoder loop (`lqos-b4w`): fixed 48-byte records, IPv4-only. An IPv6
/// flow is SKIPPED + COUNTED (never a malformed record). The flush threshold is
/// the v5 record budget (30 records = 15 flows), NOT the v9 28.
fn exporter_loop_v5(
    receiver: Receiver<FlowRow>,
    socket: &UdpSocket,
    target: &str,
    sequence: &mut u32,
) {
    let mut pending: Vec<Netflow5Record> = Vec::with_capacity(NETFLOW5_MAX_RECORDS_PER_PACKET);
    loop {
        match receiver.recv_timeout(FLUSH_INTERVAL) {
            Ok(flow) => {
                match records_from_flow_v5(&flow) {
                    Some([forward, reverse]) => {
                        pending.push(forward);
                        pending.push(reverse);
                    }
                    None => {
                        if let Some(handles) = EXPORTER.get() {
                            handles.v6_flows_skipped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                if pending.len() >= NETFLOW5_MAX_RECORDS_PER_PACKET {
                    flush_v5(socket, target, sequence, &mut pending);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if !pending.is_empty() {
                    flush_v5(socket, target, sequence, &mut pending);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if !pending.is_empty() {
                    flush_v5(socket, target, sequence, &mut pending);
                }
                break;
            }
        }
    }
}

fn flush(socket: &UdpSocket, target: &str, sequence: &mut u32, pending: &mut Vec<FlowRecord>) {
    let uptime_ms = lqos_utils::unix_time::time_since_boot()
        .map(|duration| duration.num_milliseconds() as u32)
        .unwrap_or(0);
    let unix_secs = lqos_utils::unix_time::unix_now().unwrap_or(0) as u32;
    let records = pending.len() as u64;
    let (bytes, next) = encode_batch(pending, *sequence, uptime_ms, unix_secs);
    let Some(handles) = EXPORTER.get() else {
        pending.clear();
        return;
    };
    match socket.send_to(&bytes, target) {
        Ok(_) => {
            *sequence = next;
            handles.records_sent.fetch_add(records, Ordering::Relaxed);
            handles.batches_sent.fetch_add(1, Ordering::Relaxed);
        }
        Err(err) => {
            handles.send_errors.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("NetFlow send to {target} failed: {err}");
        }
    }
    pending.clear();
}

/// `lqos-b4w`: the v5 flush (header + fixed records; the `flow_sequence`
/// advances by the record count on a successful send).
fn flush_v5(
    socket: &UdpSocket,
    target: &str,
    sequence: &mut u32,
    pending: &mut Vec<Netflow5Record>,
) {
    let uptime_ms = lqos_utils::unix_time::time_since_boot()
        .map(|duration| duration.num_milliseconds() as u32)
        .unwrap_or(0);
    let unix_secs = lqos_utils::unix_time::unix_now().unwrap_or(0) as u32;
    let records = pending.len() as u64;
    let (bytes, next) = encode_batch_v5(pending, *sequence, uptime_ms, unix_secs);
    let Some(handles) = EXPORTER.get() else {
        pending.clear();
        return;
    };
    match socket.send_to(&bytes, target) {
        Ok(_) => {
            *sequence = next;
            handles.records_sent.fetch_add(records, Ordering::Relaxed);
            handles.batches_sent.fetch_add(1, Ordering::Relaxed);
        }
        Err(err) => {
            handles.send_errors.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("NetFlow send to {target} failed: {err}");
        }
    }
    pending.clear();
}

/// The current export stats block.
pub fn stats() -> NetflowStats {
    let (target, version) = EXPORTER_TARGET
        .get()
        .map(|config| (Some(config.target.clone()), config.version))
        .unwrap_or((None, 9));
    let (enabled, records_sent, records_dropped, batches_sent, send_errors, v6_flows_skipped) =
        match EXPORTER.get() {
            Some(handles) => (
                true,
                handles.records_sent.load(Ordering::Relaxed),
                handles.records_dropped.load(Ordering::Relaxed),
                handles.batches_sent.load(Ordering::Relaxed),
                handles.send_errors.load(Ordering::Relaxed),
                handles.v6_flows_skipped.load(Ordering::Relaxed),
            ),
            None => (false, 0, 0, 0, 0, 0),
        };
    let flow_untracked_packets = if crate::bridge_analytics::is_active() {
        crate::bridge_analytics::snapshot().flow_untracked_packets
    } else {
        0
    };
    NetflowStats {
        enabled,
        target,
        version: if enabled { version } else { 9 },
        records_sent,
        records_dropped,
        batches_sent,
        send_errors,
        v6_flows_skipped,
        flow_untracked_packets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::{DirectionalCounters, FlowId, FlowState};

    fn flow(subscriber: &str, remote: &str, down: u64, up: u64) -> FlowRow {
        FlowRow {
            id: FlowId {
                circuit_id: 1,
                flow_hash: 1,
                subscriber_ip: subscriber.parse().expect("subscriber IP"),
                remote_ip: remote.parse().expect("remote IP"),
                subscriber_port: 1234,
                remote_port: 443,
                protocol: 6,
            },
            actual: DirectionalCounters {
                download_bytes: down,
                upload_bytes: up,
            },
            shaped: DirectionalCounters::default(),
            rtt_samples_us: Vec::new(),
            tcp_sent_packets: 0,
            tcp_retransmits: 0,
            first_seen_millis: 1_000,
            last_seen_millis: 2_500,
            state: FlowState::Fin,
            ..Default::default()
        }
    }

    fn record(src: &str, dst: &str, sp: u16, dp: u16, bytes: u64) -> FlowRecord {
        FlowRecord {
            src_ip: src.parse().expect("src IP"),
            dst_ip: dst.parse().expect("dst IP"),
            src_port: sp,
            dst_port: dp,
            protocol: 6,
            tcp_flags: 0,
            tos: 0,
            in_bytes: bytes,
            in_pkts: 0,
            first_switched_ms: 1_000,
            last_switched_ms: 2_500,
        }
    }

 /// The IN_PKTS fill, ruling i): each directional record
    /// carries that direction's all-protocol sent packets — dir 0 (upload)
    /// reads `sent_pkts_up`, dir 1 (download) reads `sent_pkts_down`.
    #[test]
    fn records_fill_in_pkts_from_sent_pkts_per_direction() {
        let mut probe = flow("100.97.0.1", "198.51.100.1", 500, 200);
        probe.sent_pkts_up = 7;
        probe.sent_pkts_down = 11;
        let [forward, reverse] = records_from_flow(&probe);
        assert_eq!(forward.in_pkts, 7, "dir 0 (upload) reads sent_pkts_up");
        assert_eq!(
            reverse.in_pkts, 11,
            "dir 1 (download) reads sent_pkts_down"
        );
    }

    /// A5 disclosure pin: the exporter has no per-flow TCP-flags / IP-TOS wire
    /// source, so BOTH directional records force `tcp_flags` and `tos` to 0 —
    /// the fields stay in the template for v2-collector compatibility but never
    /// carry fabricated values.
    #[test]
    fn records_force_zero_tcp_flags_and_tos_without_a_wire_source() {
        let probe = flow("100.97.0.1", "198.51.100.1", 500, 200);
        let [forward, reverse] = records_from_flow(&probe);
        assert_eq!(
            (forward.tcp_flags, forward.tos),
            (0, 0),
            "upload leg carries no fabricated TCP-flags / TOS"
        );
        assert_eq!(
            (reverse.tcp_flags, reverse.tos),
            (0, 0),
            "download leg carries no fabricated TCP-flags / TOS"
        );
    }

    /// The field map: direction 0 is subscriber→remote (upload bytes),
    /// direction 1 is the reverse (download bytes); first/last ride both.
    #[test]
    fn records_map_directions_and_timestamps() {
        let [forward, reverse] = records_from_flow(&flow("100.97.0.1", "198.51.100.1", 500, 200));
        assert_eq!(
            forward,
            record("100.97.0.1", "198.51.100.1", 1234, 443, 200)
        );
        assert_eq!(
            reverse,
            record("198.51.100.1", "100.97.0.1", 443, 1234, 500)
        );
    }

    /// A v9 batch: version 9, count = records + 2 templates, the sequence
    /// advances by the count, and both templates are present.
    #[test]
    fn batch_encodes_header_templates_and_sequence() {
        let flow = flow("100.97.0.1", "198.51.100.1", 500, 200);
        let records = records_from_flow(&flow);
        let (bytes, next) = encode_batch(&records, 10, 1_000, 1_700_000_000);
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), 9, "version 9");
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 4, "2 records + 2 templates");
        assert_eq!(
            u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            10,
            "sequence"
        );
        assert_eq!(next, 14, "sequence advances by the count");
        let template_id = u16::from_be_bytes([bytes[20], bytes[21]]);
        assert_eq!(template_id, 0, "first flowset is the template flowset");
        // The IPv4 template id 256 follows the flowset id + length.
        assert_eq!(
            u16::from_be_bytes([bytes[24], bytes[25]]),
            TEMPLATE_ID_IPV4
        );
    }

    /// A record carries the flowset id, the padded length, and the byte count
    /// in the template's field order.
    #[test]
    fn flowset_encodes_id_length_and_bytes() {
        let rec = record("100.97.0.1", "198.51.100.1", 1234, 443, 0x0102_0304_0506_0708);
        let bytes = rec.encode_flowset();
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), TEMPLATE_ID_IPV4);
        let fields_len: u16 = FIELDS_IPV4.iter().map(|(_, length)| length).sum();
        let aligned_total = (fields_len + 4).div_ceil(4) * 4;
        let declared = u16::from_be_bytes([bytes[2], bytes[3]]);
        assert_eq!(
            declared, aligned_total,
            "flowset length is the 4-aligned total incl. padding (RFC 3954 §5.1)"
        );
        assert_eq!(
            usize::from(declared),
            bytes.len(),
            "the declared length frames the whole flowset"
        );
        // IN_BYTES is the first field, big-endian.
        assert_eq!(&bytes[4..12], &0x0102_0304_0506_0708u64.to_be_bytes());
        assert_eq!(bytes.len() % 4, 0, "padded to 32 bits");
    }

    /// A strict collector must be able to frame the whole packet by walking
    /// each flowset's declared Length: every length is 4-byte aligned and the
    /// walk lands exactly on the end (the §3 regression's collector-visible
    /// assert).
    #[test]
    fn batch_flowsets_are_four_byte_framed() {
        let records: Vec<FlowRecord> = (0..8)
            .flat_map(|i| {
                records_from_flow(&flow(
                    &format!("100.97.0.{i}"),
                    "198.51.100.1",
                    500,
                    200,
                ))
            })
            .collect();
        let (bytes, _) = encode_batch(&records, 0, 0, 0);
        assert_eq!(bytes.len() % 4, 0, "packet stays 4-byte aligned");
        let mut offset = 20usize;
        let mut data_flowsets = 0;
        while offset < bytes.len() {
            let flowset_id = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
            let length =
                usize::from(u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]));
            assert_eq!(length % 4, 0, "flowset at {offset} length {length} aligned");
            assert!(
                length >= 4 && offset + length <= bytes.len(),
                "flowset at {offset} frames within the packet"
            );
            if flowset_id != 0 {
                data_flowsets += 1;
            }
            offset += length;
        }
        assert_eq!(offset, bytes.len(), "framing consumes the packet exactly");
        assert_eq!(data_flowsets, records.len(), "every record is framed");
    }

    /// IPv6 flows use template 257 and 16-byte addresses.
    #[test]
    fn ipv6_flow_uses_the_ipv6_template() {
        let flow = flow("2001:db8::1", "2001:db8::2", 100, 50);
        let records = records_from_flow(&flow);
        assert!(!records[0].is_ipv4());
        assert_eq!(records[0].template_id(), TEMPLATE_ID_IPV6);
        let bytes = records[0].encode_flowset();
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), TEMPLATE_ID_IPV6);
    }

    /// Only completion states are exported (Active flows are not completion
    /// events for the exporter).
    #[test]
    fn completion_filter_accepts_finished_states() {
        let mut probe = flow("100.97.0.1", "198.51.100.1", 1, 1);
        assert!(is_completion(&probe), "Fin");
        probe.state = FlowState::Active;
        assert!(!is_completion(&probe));
        probe.state = FlowState::Reset;
        assert!(is_completion(&probe));
        probe.state = FlowState::Idle;
        assert!(is_completion(&probe));
    }

    /// The retention dedups a flow id and evicts the oldest past capacity.
    #[test]
    fn retention_dedups_and_evicts_oldest() {
        let id = |hash: u64| FlowId {
            circuit_id: 1,
            flow_hash: hash,
            subscriber_ip: "100.97.0.1".parse().expect("IP"),
            remote_ip: "198.51.100.1".parse().expect("IP"),
            subscriber_port: 1,
            remote_port: 2,
            protocol: 6,
        };
        let mut retention = FlowIdRetention::new(2);
        assert!(retention.insert(id(1)));
        assert!(!retention.insert(id(1)), "duplicate rejected");
        assert!(retention.insert(id(2)));
        assert!(retention.insert(id(3)), "capacity evicts the oldest");
        assert!(retention.insert(id(1)), "evicted id can re-enter");
    }

    /// Without a configured exporter the stats block is disabled with the v9
    /// default (and no fabricated counters).
    #[test]
    fn stats_without_an_exporter_is_disabled() {
        let stats = stats();
        assert!(!stats.enabled);
        assert_eq!(stats.version, 9);
        assert_eq!(stats.records_sent, 0);
        assert_eq!(stats.records_dropped, 0);
    }

    /// `lqos-b4w` v9 BYTE-IDENTICAL pin: the v9 packet encoder is unchanged by
    /// the v5 port — a known 1-flow batch produces the exact packet (length,
    /// header, template flowset, sequence advance). Any v9 drift fails here.
    #[test]
    fn v9_batch_is_byte_identical() {
        let flow = flow("100.97.0.1", "198.51.100.1", 500, 200);
        let records = records_from_flow(&flow);
        let (bytes, next) = encode_batch(&records, 10, 1_000, 1_700_000_000);
        // 20-byte header + 2 templates (2*(8+11*4)=104) + 2 flowsets (2*44=88).
        assert_eq!(bytes.len(), 212, "v9 packet length is stable");
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), 9, "version 9");
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 4, "2 records + 2 templates");
        assert_eq!(
            u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            10,
            "flow_sequence"
        );
        assert_eq!(next, 14, "sequence advances by the count (2 + 2 templates)");
        // The 20-byte header, byte-for-byte.
        let mut expected_header = Vec::new();
        expected_header.extend_from_slice(&9u16.to_be_bytes());
        expected_header.extend_from_slice(&4u16.to_be_bytes());
        expected_header.extend_from_slice(&1_000u32.to_be_bytes());
        expected_header.extend_from_slice(&1_700_000_000u32.to_be_bytes());
        expected_header.extend_from_slice(&10u32.to_be_bytes());
        expected_header.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(&bytes[0..20], expected_header.as_slice(), "v9 header bytes");
        // A stable whole-packet digest (FNV-1a over the bytes).
        let digest = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
        assert_eq!(digest, 14348754584298634557, "v9 packet digest is stable");
    }

    /// `lqos-b4w`: the v5 record map — IPv4 upload leg is direction 0, the
    /// download leg is direction 1; the 32-bit counters are the directional
    /// bytes/packets; the timestamps are the boot-relative millis.
    #[test]
    fn v5_records_map_directions_and_counters() {
        let mut probe = flow("100.97.0.1", "198.51.100.1", 500, 200);
        probe.sent_pkts_up = 7;
        probe.sent_pkts_down = 11;
        let [forward, reverse] = records_from_flow_v5(&probe).expect("IPv4 v5 records");
        assert_eq!(forward.src_addr, u32::from_be_bytes([100, 97, 0, 1]));
        assert_eq!(forward.dst_addr, u32::from_be_bytes([198, 51, 100, 1]));
        assert_eq!((forward.src_port, forward.dst_port), (1234, 443));
        assert_eq!((forward.input, forward.output), (0, 1));
        assert_eq!((forward.d_pkts, forward.d_octets), (7, 200), "upload leg");
        assert_eq!(forward.prot, 6);
        assert_eq!((forward.first, forward.last), (1_000, 2_500));
        assert_eq!(reverse.src_addr, forward.dst_addr);
        assert_eq!(reverse.dst_addr, forward.src_addr);
        assert_eq!((reverse.src_port, reverse.dst_port), (443, 1234));
        assert_eq!((reverse.input, reverse.output), (1, 0));
        assert_eq!((reverse.d_pkts, reverse.d_octets), (11, 500), "download leg");
    }

    /// `lqos-b4w` IPv4-ONLY: an IPv6 flow has NO v5 record — the encoder returns
    /// `None` (the caller SKIPS + COUNTS it), never a malformed record.
    #[test]
    fn v5_records_skip_ipv6() {
        let probe = flow("2001:db8::1", "2001:db8::2", 100, 50);
        assert!(
            records_from_flow_v5(&probe).is_none(),
            "v5 is IPv4-only; a v6 flow is skipped"
        );
    }

    /// `lqos-b4w` v5 HEADER/RECORD pin: version 5, count = records, the fixed
    /// 24+48*n framing, big-endian addresses/counters, and the sequence advance.
    #[test]
    fn v5_batch_encodes_header_and_records() {
        let mut probe = flow("100.97.0.1", "198.51.100.1", 500, 200);
        probe.sent_pkts_up = 7;
        let records = records_from_flow_v5(&probe).expect("IPv4 v5 records");
        let (bytes, next) = encode_batch_v5(&records, 10, 1_000, 1_700_000_000);
        assert_eq!(bytes.len(), 24 + 48 * 2, "v5 packet = header + fixed records");
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), 5, "version 5");
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 2, "count = 2 records");
        assert_eq!(
            u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            1_000,
            "sys_uptime"
        );
        assert_eq!(
            u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            1_700_000_000,
            "unix_secs"
        );
        assert_eq!(
            u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
            10,
            "flow_sequence"
        );
        // Record 0 begins at offset 24.
        assert_eq!(&bytes[24..28], &[100, 97, 0, 1], "src_addr (network order)");
        assert_eq!(&bytes[28..32], &[198, 51, 100, 1], "dst_addr");
        assert_eq!(
            u32::from_be_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            7,
            "d_pkts"
        );
        assert_eq!(
            u32::from_be_bytes([bytes[44], bytes[45], bytes[46], bytes[47]]),
            200,
            "d_octets"
        );
        assert_eq!(bytes[62], 6, "protocol");
        assert_eq!(next, 12, "flow_sequence advances by the record count");
    }
}

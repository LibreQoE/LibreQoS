mod circuit_cake_drops;
mod circuit_cake_marks;
mod circuit_retransmits;
mod circuit_rtt;
mod circuit_throughput;
mod general;
mod site_cake_drops;
mod site_cake_marks;
mod site_retransmits;
mod site_rtt;
mod site_throughput;

use crate::lts2_sys::lts2_client::ingestor::commands::IngestorCommand;
use crate::lts2_sys::shared_types::{
    CircuitCakeDrops, CircuitCakeMarks, CircuitRetransmits, CircuitRtt, CircuitThroughput,
    IngestSession, SiteCakeDrops, SiteCakeMarks, SiteRetransmits, SiteRtt, SiteThroughput,
};
use anyhow::Result;
use lqos_config::load_config;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info, warn};
use uuid::Uuid;

/// `lqos-74t.2.1` per-field attribution helper: the serialized CBOR length of
/// ONE `IngestSession` field (the exact wire contribution, since the session
/// uses the default serde encoding). Cold path only (once per collation
/// window); removal gate 2026-12-31 (DRIVER-owned).
fn cbor_len<T: serde::Serialize>(value: &T) -> usize {
    serde_cbor::to_vec(value).map_or(0, |bytes| bytes.len())
}

/// `lqos-74t.2.1` STEP 3: the **permanent** LTS per-session DECOMPRESSED-CBOR
/// byte budget. The ingestor inflates ONE session per `unique_id` with a 16 MiB
/// limit; we split well below it. Named predecessor (complexity ratchet): the
/// unbounded single-session `build_chunks`. Permanent product policy — NOT a
/// 2026-12-31 diagnostic.
pub(crate) const LTS_SESSION_BUDGET_BYTES: usize = 12 * 1024 * 1024;

/// Conservative per-field map-key + header margin for the packer estimate.
const SESSION_FIELD_OVERHEAD_BYTES: usize = 64;

/// `.2.2` Shape A: the operational board-fragment size (bytes).
///
/// A fragment travels as `IngestSession.blackboard_json: Option<Vec<u8>>`, which
/// `serde_cbor` encodes as a CBOR **array of unsigned integers** — each payload
/// byte costs 1 B (< 0x18) or 2 B (≥ 0x18), so the session field is at most
/// `2 × fragment`. The formula bound is
/// `floor((LTS_SESSION_BUDGET_BYTES − base_len − SESSION_FIELD_OVERHEAD_BYTES) / 2.0)`
/// `≈ 6,291,360 B` (r = 2.0 is the content-independent ceiling); this operational
/// value keeps a ~0.55 MiB/session margin. `emit_blackboard_fragments` still
/// shrinks defensively if a session ever exceeds the budget.
pub(crate) const BLACKBOARD_FRAGMENT_MAX_BYTES: usize = 6_000_000;

/// Cumulative whole-field trims (the STEP-3 accounted-drop counter). Exposed to
/// the crate (and carried in the per-session log) so an operator can see the
/// visible product loss.
static LTS_SESSION_TRIMS: AtomicU64 = AtomicU64::new(0);

/// The cumulative LTS session trim count (STEP-3 accounted drops).
pub(crate) fn lts_session_trim_count() -> u64 {
    LTS_SESSION_TRIMS.load(Ordering::Relaxed)
}

/// One packable `IngestSession` field (WHOLE-FIELD granularity: NO row
/// sub-slicing; the server merge semantics for a partial row list are
/// unverified).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionField {
    ShaperThroughput,
    ShapedDevices,
    NetworkTree,
    CircuitThroughput,
    CircuitRetransmits,
    CircuitRtt,
    CircuitCakeDrops,
    CircuitCakeMarks,
    SiteCakeDrops,
    SiteCakeMarks,
    SiteRetransmits,
    SiteRtt,
    SiteThroughput,
    ShaperUtilization,
    OneWayFlows,
    TwoWayFlows,
    AllowedIps,
    IgnoredIps,
    FlowCount,
    /// The one ATOMIC field (a single opaque CBOR blob).
    BlackboardJson,
}

/// Deterministic pack order (stable sort ties keep this order). `BlackboardJson`
/// LAST so the core fields pack together and the atomic blob is decoupled.
const SESSION_FIELDS: [SessionField; 20] = [
    SessionField::ShaperThroughput,
    SessionField::ShapedDevices,
    SessionField::NetworkTree,
    SessionField::CircuitThroughput,
    SessionField::CircuitRetransmits,
    SessionField::CircuitRtt,
    SessionField::CircuitCakeDrops,
    SessionField::CircuitCakeMarks,
    SessionField::SiteCakeDrops,
    SessionField::SiteCakeMarks,
    SessionField::SiteRetransmits,
    SessionField::SiteRtt,
    SessionField::SiteThroughput,
    SessionField::ShaperUtilization,
    SessionField::OneWayFlows,
    SessionField::TwoWayFlows,
    SessionField::AllowedIps,
    SessionField::IgnoredIps,
    SessionField::FlowCount,
    SessionField::BlackboardJson,
];

/// The `IngestSession` field's wire name (for the trim `warn!`).
fn field_name(field: SessionField) -> &'static str {
    match field {
        SessionField::ShaperThroughput => "shaper_throughput",
        SessionField::ShapedDevices => "shaped_devices",
        SessionField::NetworkTree => "network_tree",
        SessionField::CircuitThroughput => "circuit_throughput",
        SessionField::CircuitRetransmits => "circuit_retransmits",
        SessionField::CircuitRtt => "circuit_rtt",
        SessionField::CircuitCakeDrops => "circuit_cake_drops",
        SessionField::CircuitCakeMarks => "circuit_cake_marks",
        SessionField::SiteCakeDrops => "site_cake_drops",
        SessionField::SiteCakeMarks => "site_cake_marks",
        SessionField::SiteRetransmits => "site_retransmits",
        SessionField::SiteRtt => "site_rtt",
        SessionField::SiteThroughput => "site_throughput",
        SessionField::ShaperUtilization => "shaper_utilization",
        SessionField::OneWayFlows => "one_way_flows",
        SessionField::TwoWayFlows => "two_way_flows",
        SessionField::AllowedIps => "allowed_ips",
        SessionField::IgnoredIps => "ignored_ips",
        SessionField::FlowCount => "flow_count",
        SessionField::BlackboardJson => "blackboard_json",
    }
}

/// The field's serialized CBOR length (its wire contribution).
fn field_bytes(message: &IngestSession, field: SessionField) -> usize {
    match field {
        SessionField::ShaperThroughput => cbor_len(&message.shaper_throughput),
        SessionField::ShapedDevices => cbor_len(&message.shaped_devices),
        SessionField::NetworkTree => cbor_len(&message.network_tree),
        SessionField::CircuitThroughput => cbor_len(&message.circuit_throughput),
        SessionField::CircuitRetransmits => cbor_len(&message.circuit_retransmits),
        SessionField::CircuitRtt => cbor_len(&message.circuit_rtt),
        SessionField::CircuitCakeDrops => cbor_len(&message.circuit_cake_drops),
        SessionField::CircuitCakeMarks => cbor_len(&message.circuit_cake_marks),
        SessionField::SiteCakeDrops => cbor_len(&message.site_cake_drops),
        SessionField::SiteCakeMarks => cbor_len(&message.site_cake_marks),
        SessionField::SiteRetransmits => cbor_len(&message.site_retransmits),
        SessionField::SiteRtt => cbor_len(&message.site_rtt),
        SessionField::SiteThroughput => cbor_len(&message.site_throughput),
        SessionField::ShaperUtilization => cbor_len(&message.shaper_utilization),
        SessionField::OneWayFlows => cbor_len(&message.one_way_flows),
        SessionField::TwoWayFlows => cbor_len(&message.two_way_flows),
        SessionField::AllowedIps => cbor_len(&message.allowed_ips),
        SessionField::IgnoredIps => cbor_len(&message.ignored_ips),
        SessionField::FlowCount => cbor_len(&message.flow_count),
        SessionField::BlackboardJson => cbor_len(&message.blackboard_json),
    }
}

/// Move ONE whole field from `src` into `dst` (each field is assigned exactly
/// once by the packer).
fn take_field(dst: &mut IngestSession, src: &mut IngestSession, field: SessionField) {
    match field {
        SessionField::ShaperThroughput => {
            dst.shaper_throughput = std::mem::take(&mut src.shaper_throughput);
        }
        SessionField::ShapedDevices => {
            dst.shaped_devices = std::mem::take(&mut src.shaped_devices);
        }
        SessionField::NetworkTree => {
            dst.network_tree = std::mem::take(&mut src.network_tree);
        }
        SessionField::CircuitThroughput => {
            dst.circuit_throughput = std::mem::take(&mut src.circuit_throughput);
        }
        SessionField::CircuitRetransmits => {
            dst.circuit_retransmits = std::mem::take(&mut src.circuit_retransmits);
        }
        SessionField::CircuitRtt => {
            dst.circuit_rtt = std::mem::take(&mut src.circuit_rtt);
        }
        SessionField::CircuitCakeDrops => {
            dst.circuit_cake_drops = std::mem::take(&mut src.circuit_cake_drops);
        }
        SessionField::CircuitCakeMarks => {
            dst.circuit_cake_marks = std::mem::take(&mut src.circuit_cake_marks);
        }
        SessionField::SiteCakeDrops => {
            dst.site_cake_drops = std::mem::take(&mut src.site_cake_drops);
        }
        SessionField::SiteCakeMarks => {
            dst.site_cake_marks = std::mem::take(&mut src.site_cake_marks);
        }
        SessionField::SiteRetransmits => {
            dst.site_retransmits = std::mem::take(&mut src.site_retransmits);
        }
        SessionField::SiteRtt => {
            dst.site_rtt = std::mem::take(&mut src.site_rtt);
        }
        SessionField::SiteThroughput => {
            dst.site_throughput = std::mem::take(&mut src.site_throughput);
        }
        SessionField::ShaperUtilization => {
            dst.shaper_utilization = std::mem::take(&mut src.shaper_utilization);
        }
        SessionField::OneWayFlows => {
            dst.one_way_flows = std::mem::take(&mut src.one_way_flows);
        }
        SessionField::TwoWayFlows => {
            dst.two_way_flows = std::mem::take(&mut src.two_way_flows);
        }
        SessionField::AllowedIps => {
            dst.allowed_ips = std::mem::take(&mut src.allowed_ips);
        }
        SessionField::IgnoredIps => {
            dst.ignored_ips = std::mem::take(&mut src.ignored_ips);
        }
        SessionField::FlowCount => {
            dst.flow_count = std::mem::take(&mut src.flow_count);
        }
        SessionField::BlackboardJson => {
            dst.blackboard_json = std::mem::take(&mut src.blackboard_json);
        }
    }
}

fn serialize_or_warn(message: &IngestSession) -> Option<Vec<u8>> {
    match serde_cbor::to_vec(message) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            warn!("Failed to serialize LTS session: {error}");
            None
        }
    }
}

/// Compress (deflate) + chunk a serialized session. Chunk size is 60 KiB.
fn chunk_raw(raw_bytes: &[u8]) -> Vec<Vec<u8>> {
    const CHUNK_SIZE: usize = 60 * 1024;
    miniz_oxide::deflate::compress_to_vec(raw_bytes, 10)
        .chunks(CHUNK_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

/// `lqos-74t.2.1` per-session size + per-field attribution (keep with the
/// STEP-3 guardrail; the size log is the oracle read).
fn log_session(
    message: &IngestSession,
    raw_len: usize,
    chunks: &[Vec<u8>],
    index: usize,
    total: usize,
) {
    let compressed_len: usize = chunks.iter().map(Vec::len).sum();
    let n_chunks = chunks.len();
    const INGEST_DECOMPRESSED_LIMIT_BYTES: usize = 16 * 1024 * 1024;
    let one_way = message.one_way_flows.as_ref().map_or(0, Vec::len);
    let two_way = message.two_way_flows.as_ref().map_or(0, Vec::len);
    info!(target: "lqos_lts_volume",
        "LTS2 ingest session {}/{}: one_way_flows={} two_way_flows={} flows_total={} \
         shaper_util_rows={} circuit_throughput_rows={} site_throughput_rows={} \
         raw_bytes={} compressed_bytes={} chunks={} decompressed_limit_bytes={} over_limit={} \
         cumulative_trims={}",
        index,
        total,
        one_way,
        two_way,
        one_way + two_way,
        message.shaper_utilization.as_ref().map_or(0, Vec::len),
        message.circuit_throughput.len(),
        message.site_throughput.len(),
        raw_len,
        compressed_len,
        n_chunks,
        INGEST_DECOMPRESSED_LIMIT_BYTES,
        raw_len > INGEST_DECOMPRESSED_LIMIT_BYTES,
        lts_session_trim_count(),
    );
    let flows_total = one_way + two_way;
    let one_way_bytes = cbor_len(&message.one_way_flows);
    let two_way_bytes = cbor_len(&message.two_way_flows);
    let flows_avg_bytes = (one_way_bytes + two_way_bytes)
        .checked_div(flows_total)
        .unwrap_or(0);
    info!(target: "lqos_lts_volume",
        "LTS2 ingest session bytes by field {}/{}: raw_bytes={} license_key={} node_id={} node_name={} \
         shaper_throughput={} shaped_devices={} \
         network_tree={} circuit_throughput={} circuit_retransmits={} circuit_rtt={} \
         circuit_cake_drops={} circuit_cake_marks={} site_cake_drops={} site_cake_marks={} \
         site_retransmits={} site_rtt={} site_throughput={} shaper_utilization={} \
         one_way_flow_bytes={} two_way_flow_bytes={} blackboard_json={} allowed_ips={} ignored_ips={} \
         flow_count={} device_count={} flows_total={} flows_avg_bytes={}",
        index,
        total,
        raw_len,
        cbor_len(&message.license_key),
        cbor_len(&message.node_id),
        cbor_len(&message.node_name),
        cbor_len(&message.shaper_throughput),
        cbor_len(&message.shaped_devices),
        cbor_len(&message.network_tree),
        cbor_len(&message.circuit_throughput),
        cbor_len(&message.circuit_retransmits),
        cbor_len(&message.circuit_rtt),
        cbor_len(&message.circuit_cake_drops),
        cbor_len(&message.circuit_cake_marks),
        cbor_len(&message.site_cake_drops),
        cbor_len(&message.site_cake_marks),
        cbor_len(&message.site_retransmits),
        cbor_len(&message.site_rtt),
        cbor_len(&message.site_throughput),
        cbor_len(&message.shaper_utilization),
        one_way_bytes,
        two_way_bytes,
        cbor_len(&message.blackboard_json),
        cbor_len(&message.allowed_ips),
        cbor_len(&message.ignored_ips),
        cbor_len(&message.flow_count),
        message.shaped_devices.len(),
        flows_total,
        flows_avg_bytes,
    );
}

/// `lqos-74t.2.1` STEP 3 whole-field packer. Returns one `IngestSession` per
/// emitted session plus the list of fields that alone exceed the budget (the
/// terminal accounted TRIMs). `base` (identity-only) already carries every map
/// key + its default value, so `base_len + Σ(field + margin)` is a safe UPPER
/// bound on each session's serialized size. A row-`Vec` larger than the budget
/// is trimmed WHOLE — never sub-sliced, so a snapshot list is never partial.
fn pack_sessions(
    message: &mut IngestSession,
    license_uuid: Uuid,
    node_id: &str,
    node_name: &str,
    budget: usize,
) -> (Vec<IngestSession>, Vec<(SessionField, usize)>) {
    let identity = || IngestSession {
        license_key: license_uuid,
        node_id: node_id.to_string(),
        node_name: node_name.to_string(),
        ..Default::default()
    };
    let base_len = cbor_len(&identity());
    let mut sessions: Vec<IngestSession> = Vec::new();
    let mut trimmed: Vec<(SessionField, usize)> = Vec::new();
    let mut cur = identity();
    let mut cur_len = base_len;
    let mut cur_count = 0usize;
    for field in SESSION_FIELDS {
        let bytes = field_bytes(message, field);
        let charged = bytes.saturating_add(SESSION_FIELD_OVERHEAD_BYTES);
        if base_len.saturating_add(charged) > budget {
            // A single whole field alone cannot fit ⇒ terminal accounted trim.
            trimmed.push((field, bytes));
            continue;
        }
        if cur_len.saturating_add(charged) > budget {
            if cur_count > 0 {
                sessions.push(std::mem::replace(&mut cur, identity()));
            }
            cur_len = base_len;
            cur_count = 0;
        }
        take_field(&mut cur, message, field);
        cur_len = cur_len.saturating_add(charged);
        cur_count += 1;
    }
    if cur_count > 0 {
        sessions.push(cur);
    }
    (sessions, trimmed)
}

/// STEP-3 emit: FAST path (full session ≤ budget ⇒ ONE session, byte-identical
/// to the old `build_chunks`) or WHOLE-FIELD packing into multiple sessions.
/// A whole field that alone exceeds the budget is a **terminal accounted trim**
/// (dropped + counter + `warn!`; until STEP 2 lands the blackboard trim is a
/// visible product loss). Returns one chunk list per session; the caller sends
/// each with its own consecutive `serial`.
fn emit_sessions(
    message: &mut IngestSession,
    license_uuid: Uuid,
    node_id: &str,
    node_name: &str,
    budget: usize,
) -> Result<Vec<Vec<Vec<u8>>>> {
    let Some(raw_bytes) = serialize_or_warn(message) else {
        return Ok(vec![]);
    };
    if raw_bytes.len() <= budget {
        let chunks = chunk_raw(&raw_bytes);
        log_session(message, raw_bytes.len(), &chunks, 1, 1);
        return Ok(vec![chunks]);
    }

    let (sessions, trimmed) = pack_sessions(message, license_uuid, node_id, node_name, budget);
    if !trimmed.is_empty() {
        let total: usize = trimmed.iter().map(|(_, bytes)| *bytes).sum();
        let cumulative = LTS_SESSION_TRIMS
            .fetch_add(trimmed.len() as u64, Ordering::Relaxed)
            .saturating_add(trimmed.len() as u64);
        for (field, bytes) in &trimmed {
            warn!(
                "LTS session budget: TRIMMED field {} ({} bytes) — permanent accounted drop (visible product loss until STEP 2); cumulative_trims={}",
                field_name(*field),
                bytes,
                cumulative,
            );
        }
        info!(target: "lqos_lts_volume",
            "LTS session budget: {} session(s); trimmed {} field(s), {} bytes of {} raw (over the {} MiB budget)",
            sessions.len().max(1),
            trimmed.len(),
            total,
            raw_bytes.len(),
            budget / (1024 * 1024),
        );
    }

    let total_sessions = sessions.len().max(1);
    let mut out = Vec::with_capacity(sessions.len());
    for (index, session) in sessions.iter().enumerate() {
        let Some(raw) = serialize_or_warn(session) else {
            continue;
        };
        if raw.len() > budget {
            // Estimate invariant violated: NEVER ship an over-budget session
            // (it would reproduce the ingest failure). Drop + account it.
            let cumulative = LTS_SESSION_TRIMS.fetch_add(1, Ordering::Relaxed) + 1;
            warn!(
                "LTS session budget: session {}/{} raw_bytes={} EXCEEDS the {} MiB budget; DROPPED (estimate invariant violated); cumulative_trims={cumulative}",
                index + 1,
                total_sessions,
                raw.len(),
                budget / (1024 * 1024),
            );
            continue;
        }
        let chunks = chunk_raw(&raw);
        log_session(session, raw.len(), &chunks, index + 1, total_sessions);
        out.push(chunks);
    }
    Ok(out)
}

/// `.2.2` Shape A: emit ONE fragment-only `IngestSession` per ordered blackboard
/// fragment. `blackboard.rs` splits the board at [`BLACKBOARD_FRAGMENT_MAX_BYTES`];
/// each fragment is re-checked here and, if its serialized session somehow
/// exceeds `budget`, halved and retried (shrink guard) — `emit_sessions` DROPS an
/// over-budget session, and losing a fragment would corrupt the server's bytewise
/// concatenation. Each session carries ONLY `blackboard_json` + the identity
/// envelope, so the server merge `extend`s every once-consumed field exactly once.
fn emit_blackboard_fragments(
    fragments: Vec<Vec<u8>>,
    license_uuid: Uuid,
    node_id: &str,
    node_name: &str,
    budget: usize,
) -> Vec<Vec<Vec<u8>>> {
    if fragments.is_empty() {
        return Vec::new();
    }
    let requested = fragments.len();
    let mut pending: VecDeque<Vec<u8>> = fragments.into();
    let mut out = Vec::new();
    let mut emitted = 0usize;
    while let Some(fragment) = pending.pop_front() {
        let fragment_bytes = fragment.len();
        let mut session = IngestSession {
            license_key: license_uuid,
            node_id: node_id.to_string(),
            node_name: node_name.to_string(),
            blackboard_json: Some(fragment),
            ..Default::default()
        };
        let Some(raw) = serialize_or_warn(&session) else {
            continue;
        };
        if raw.len() > budget {
            if session.blackboard_json.as_ref().is_some_and(|f| f.len() > 1) {
                let mut fragment = session.blackboard_json.take().expect("checked above");
                let tail = fragment.split_off(fragment.len() / 2);
                pending.push_front(tail);
                pending.push_front(fragment);
                continue;
            }
            let cumulative = LTS_SESSION_TRIMS.fetch_add(1, Ordering::Relaxed) + 1;
            warn!(
                "LTS session budget: blackboard fragment session raw_bytes={} EXCEEDS the {} MiB budget; DROPPED (shrink exhausted); cumulative_trims={cumulative}",
                raw.len(),
                budget / (1024 * 1024),
            );
            continue;
        }
        let chunks = chunk_raw(&raw);
        emitted += 1;
        info!(target: "lqos_lts_volume",
            "LTS2 blackboard fragment session {emitted}: raw_bytes={} fragment_bytes={} chunks={} (requested {requested} fragment(s))",
            raw.len(),
            fragment_bytes,
            chunks.len(),
        );
        out.push(chunks);
    }
    out
}

/// Provides holders for messages that have been received from the ingestor,
/// and not yet submitted to the LTS2 server. It divides many message types by
/// the type, to maximize batching.
#[derive(Clone)]
pub(crate) struct MessageQueue {
    /// All messages of type `IngestorCommand::General` that have been received,
    /// that haven't been categorized for batching.
    general_queue: Vec<IngestorCommand>,
    circuit_throughput: Vec<CircuitThroughput>,
    circuit_retransmits: Vec<CircuitRetransmits>,
    circuit_rtt: Vec<CircuitRtt>,
    circuit_cake_drops: Vec<CircuitCakeDrops>,
    circuit_cake_marks: Vec<CircuitCakeMarks>,
    site_throughput: Vec<SiteThroughput>,
    site_retransmits: Vec<SiteRetransmits>,
    site_cake_drops: Vec<SiteCakeDrops>,
    site_cake_marks: Vec<SiteCakeMarks>,
    site_rtt: Vec<SiteRtt>,
    /// `.2.2` Shape A: ordered blackboard fragments received this window (each
    /// becomes its own fragment-only session).
    blackboard_fragments: Vec<Vec<u8>>,
}

impl MessageQueue {
    pub(crate) fn new() -> Self {
        Self {
            general_queue: Vec::new(),
            circuit_throughput: Vec::new(),
            circuit_retransmits: Vec::new(),
            circuit_rtt: Vec::new(),
            circuit_cake_drops: Vec::new(),
            circuit_cake_marks: Vec::new(),
            site_throughput: Vec::new(),
            site_retransmits: Vec::new(),
            site_cake_drops: Vec::new(),
            site_cake_marks: Vec::new(),
            site_rtt: Vec::new(),
            blackboard_fragments: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.general_queue.is_empty()
            && self.circuit_throughput.is_empty()
            && self.circuit_retransmits.is_empty()
            && self.circuit_rtt.is_empty()
            && self.circuit_cake_drops.is_empty()
            && self.circuit_cake_marks.is_empty()
            && self.site_throughput.is_empty()
            && self.site_retransmits.is_empty()
            && self.site_cake_drops.is_empty()
            && self.site_cake_marks.is_empty()
            && self.site_rtt.is_empty()
            && self.blackboard_fragments.is_empty()
    }

    /// `lqos-74t.2.1` instrument: the pending per-window flow-command counts
    /// (one-way + two-way) in the general queue, available even when the box
    /// cannot submit (no license) — the per-window emitted-flow measure the
    /// pre-code `#12` requires vs v2's 50k–466k flows / 20 min. Cold path (one
    /// call per collation flush; default 60 s). Removal gate 2026-12-31
    /// (DRIVER-owned).
    pub(crate) fn pending_flow_counts(&self) -> (usize, usize) {
        let mut one_way = 0usize;
        let mut two_way = 0usize;
        for command in &self.general_queue {
            match command {
                IngestorCommand::OneWayFlow { .. } => one_way += 1,
                IngestorCommand::TwoWayFlow { .. } => two_way += 1,
                _ => {}
            }
        }
        (one_way, two_way)
    }

    pub(crate) fn ingest(&mut self, command: IngestorCommand) {
        match command {
            IngestorCommand::IngestBatchComplete { .. } => {
                tracing::error!("IngestBatchComplete Should Never Reach Here");
            }
            IngestorCommand::CircuitThroughputBatch(batch) => {
                self.circuit_throughput.extend(batch);
            }
            IngestorCommand::CircuitRetransmitsBatch(batch) => {
                self.circuit_retransmits.extend(batch);
            }
            IngestorCommand::CircuitRttBatch(batch) => {
                self.circuit_rtt.extend(batch);
            }
            IngestorCommand::CircuitCakeDropsBatch(batch) => {
                self.circuit_cake_drops.extend(batch);
            }
            IngestorCommand::CircuitCakeMarksBatch(batch) => {
                self.circuit_cake_marks.extend(batch);
            }
            IngestorCommand::SiteThroughputBatch(batch) => {
                self.site_throughput.extend(batch);
            }
            IngestorCommand::SiteRetransmitsBatch(batch) => {
                self.site_retransmits.extend(batch);
            }
            IngestorCommand::SiteCakeDropsBatch(batch) => {
                self.site_cake_drops.extend(batch);
            }
            IngestorCommand::SiteCakeMarksBatch(batch) => {
                self.site_cake_marks.extend(batch);
            }
            IngestorCommand::SiteRttBatch(batch) => {
                self.site_rtt.extend(batch);
            }
            IngestorCommand::BlackboardFragments(fragments) => {
                // `.2.2`: the server byte-concatenates every session's
                // `blackboard_json` within a batch, so two boards in one window
                // would concatenate to invalid CBOR (`trailing data`) and BOTH
                // would be dropped. Keep ONLY the newest board — the pre-.2.2
                // path overwrote `message.blackboard_json` (last wins). A stall
                // (bailed flush) can carry a board into the next window.
                if !self.blackboard_fragments.is_empty() {
                    warn!(
                        "LTS blackboard: a new board arrived while {} fragment(s) were pending; replacing the older board to avoid a corrupt concatenation",
                        self.blackboard_fragments.len(),
                    );
                    self.blackboard_fragments.clear();
                }
                self.blackboard_fragments.extend(fragments);
            }
            _ => self.general_queue.push(command),
        }
    }

    /// `lqos-74t.2.1` STEP 3: build and emit the LTS sessions — the FAST path
    /// (one session when the CBOR fits `LTS_SESSION_BUDGET_BYTES`) or WHOLE-FIELD
    /// packing into multiple sessions; see [`emit_sessions`].
    pub(crate) fn build_sessions(&mut self) -> Result<Vec<Vec<Vec<u8>>>> {
        if !crate::lts2_sys::can_submit_long_term_stats() {
            self.clear();
            return Ok(vec![]);
        }

        // Gather the license info for bundling
        let (license_key, node_id, node_name) = {
            let Ok(lock) = load_config() else {
                warn!("Failed to load config");
                return Ok(vec![]);
            };
            (
                lock.long_term_stats
                    .license_key
                    .clone()
                    .unwrap_or("".to_string()),
                lock.node_id.clone(),
                lock.node_name.clone(),
            )
        };
        let Ok(license_uuid) = Uuid::parse_str(&license_key.replace("-", "")) else {
            return Ok(vec![]);
        };

        // Build the message
        let mut message = IngestSession {
            license_key: license_uuid,
            node_id: node_id.clone(),
            node_name: node_name.clone(),
            ..Default::default()
        };
        general::add_general(&mut message, &mut self.general_queue);
        circuit_throughput::add_circuit_throughput(&mut message, &mut self.circuit_throughput);
        circuit_retransmits::add_circuit_retransmits(&mut message, &mut self.circuit_retransmits);
        circuit_rtt::add_circuit_rtt(&mut message, &mut self.circuit_rtt);
        circuit_cake_drops::add_circuit_cake_drops(&mut message, &mut self.circuit_cake_drops);
        circuit_cake_marks::add_circuit_cake_marks(&mut message, &mut self.circuit_cake_marks);
        site_cake_drops::add_site_cake_drops(&mut message, &mut self.site_cake_drops);
        site_cake_marks::add_site_cake_marks(&mut message, &mut self.site_cake_marks);
        site_retransmits::add_site_retransmits(&mut message, &mut self.site_retransmits);
        site_rtt::add_site_rtt(&mut message, &mut self.site_rtt);
        site_throughput::add_site_throughput(&mut message, &mut self.site_throughput);

        let mut sessions = emit_sessions(
            &mut message,
            license_uuid,
            &node_id,
            &node_name,
            LTS_SESSION_BUDGET_BYTES,
        )?;
        // `.2.2` Shape A: the board was NOT applied to `message` (it was routed
        // to `blackboard_fragments` in `ingest`), so the packed field session(s)
        // above carry `blackboard_json = None`. Emit one fragment-only session
        // per ordered fragment; the lts2 server byte-concatenates them within one
        // arena batch (`merge_ingest_sessions`).
        let fragments = std::mem::take(&mut self.blackboard_fragments);
        sessions.extend(emit_blackboard_fragments(
            fragments,
            license_uuid,
            &node_id,
            &node_name,
            LTS_SESSION_BUDGET_BYTES,
        ));
        Ok(sessions)
    }

    pub(crate) fn clear(&mut self) {
        self.general_queue.clear();
        self.circuit_throughput.clear();
        self.circuit_retransmits.clear();
        self.circuit_rtt.clear();
        self.circuit_cake_drops.clear();
        self.circuit_cake_marks.clear();
        self.site_throughput.clear();
        self.site_retransmits.clear();
        self.site_cake_drops.clear();
        self.site_cake_marks.clear();
        self.site_rtt.clear();
        self.blackboard_fragments.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lts2_sys::shared_types::{OneWayFlow, TwoWayFlow};
    use std::net::{IpAddr, Ipv4Addr};

    fn local_ip() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))
    }

    fn remote_ip() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(198, 51, 100, 20))
    }

    fn two_way(port: u16) -> TwoWayFlow {
        TwoWayFlow {
            start_time: 1_700_000_000,
            end_time: 1_700_000_010,
            local_ip: local_ip(),
            remote_ip: remote_ip(),
            protocol: 6,
            dst_port: 443,
            src_port: port,
            bytes_down: 1_000_000,
            bytes_up: 250_000,
            retransmit_times_down: Vec::new(),
            retransmit_times_up: Vec::new(),
            rtt: [12_000.0, 15_000.0],
            circuit_hash: 42,
            packets_down: Some(1_000),
            packets_up: Some(250),
        }
    }

    fn one_way(port: u16) -> OneWayFlow {
        OneWayFlow {
            start_time: 1_700_000_000,
            end_time: 1_700_000_010,
            local_ip: local_ip(),
            remote_ip: remote_ip(),
            protocol: 17,
            dst_port: 443,
            src_port: port,
            bytes: 512,
            circuit_hash: 42,
        }
    }

    /// `lqos-74t.2.1` arithmetic: the per-ENTRY CBOR cost grounds the 16 MiB
    /// flow threshold. The ingestor inflates ONE `IngestSession`; a session
    /// with N flow entries (one-way + two-way) is ~N × `per_entry`. Measured
    /// here so the live instrument's `raw_bytes` can be read against the limit
    /// reproducibly.
    #[test]
    fn ingest_session_size_per_flow_grounds_the_16mib_threshold() {
        let n = 1_000usize;
        let empty = serde_cbor::to_vec(&IngestSession::default())
            .expect("empty session serializes")
            .len();
        let message = IngestSession {
            two_way_flows: Some((0..n).map(|i| two_way(1_000 + i as u16)).collect()),
            one_way_flows: Some((0..n).map(|i| one_way(2_000 + i as u16)).collect()),
            ..Default::default()
        };
        let raw = serde_cbor::to_vec(&message)
            .expect("session serializes")
            .len();
        // 2*n flow ENTRIES (n two-way + n one-way); `per_entry` is per flow row.
        let entries = 2 * n;
        let per_entry = (raw - empty) / entries;
        assert!(raw > empty, "flows must add bytes");
        // Sanity bound: a flow entry is well under 400 B ⇒ 16 MiB holds
        // > ~40k flow entries before the ingest limit is reached.
        assert!(
            per_entry < 400,
            "per-entry CBOR unexpectedly large: {per_entry} B"
        );
        let threshold_entries = (16 * 1024 * 1024) / per_entry;
        assert!(
            threshold_entries > 10_000,
            "16 MiB entry threshold unexpectedly small: {threshold_entries}"
        );
        println!(
            "ingest session size: empty={empty}B raw({entries} entries)={raw}B per_entry={per_entry}B 16MiB_entries~{threshold_entries}"
        );
    }

    fn session_two_way(n: usize) -> IngestSession {
        IngestSession {
            license_key: Uuid::nil(),
            node_id: "node".to_string(),
            node_name: "Node".to_string(),
            two_way_flows: Some(
                (0..n)
                    .map(|i| two_way(1_000 + (i % 50_000) as u16))
                    .collect(),
            ),
            ..Default::default()
        }
    }

    /// STEP 3 FAST path: a session within the budget packs to ONE session with
    /// NO trims (the pre-guardrail behaviour).
    #[test]
    fn step3_under_budget_packs_one_session_without_trims() {
        let mut message = session_two_way(10);
        let original = message.two_way_flows.as_ref().map_or(0, Vec::len);
        let (sessions, trimmed) =
            pack_sessions(&mut message, Uuid::nil(), "node", "Node", 64 * 1024 * 1024);
        assert_eq!(sessions.len(), 1, "under budget must be one session");
        assert!(
            trimmed.is_empty(),
            "no field should be trimmed: {trimmed:?}"
        );
        let retained = sessions[0].two_way_flows.as_ref().map_or(0, Vec::len);
        assert_eq!(retained, original, "whole-field packing is lossless");
    }

    /// STEP 3 SPLIT: when MULTIPLE whole fields together exceed the budget but
    /// each fits alone, the packer emits ≥2 sessions, each VERIFIED under the
    /// budget, and the union of every field's rows is the original (lossless —
    /// whole-field granularity, NO row sub-slicing).
    #[test]
    fn step3_over_budget_splits_losslessly_and_bounds_each_session() {
        let mut message = IngestSession {
            license_key: Uuid::nil(),
            node_id: "node".to_string(),
            node_name: "Node".to_string(),
            one_way_flows: Some((0..300).map(|i| one_way(1_000 + i as u16)).collect()),
            two_way_flows: Some((0..300).map(|i| two_way(1_000 + i as u16)).collect()),
            ..Default::default()
        };
        let original_one = message.one_way_flows.as_ref().map_or(0, Vec::len);
        let original_two = message.two_way_flows.as_ref().map_or(0, Vec::len);
        // A budget that fits EACH whole field alone but NOT both together ⇒ the
        // packer must split (no row sub-slicing).
        let base = IngestSession {
            license_key: Uuid::nil(),
            node_id: "node".to_string(),
            node_name: "Node".to_string(),
            ..Default::default()
        };
        let base_len = cbor_len(&base);
        let one_sz = cbor_len(&message.one_way_flows) + SESSION_FIELD_OVERHEAD_BYTES;
        let two_sz = cbor_len(&message.two_way_flows) + SESSION_FIELD_OVERHEAD_BYTES;
        let budget = base_len + one_sz.max(two_sz) + 64;
        let (sessions, trimmed) = pack_sessions(&mut message, Uuid::nil(), "node", "Node", budget);
        assert!(
            sessions.len() >= 2,
            "two whole fields over the budget must split: {} session(s)",
            sessions.len()
        );
        assert!(
            trimmed.is_empty(),
            "no single field is oversized here: {trimmed:?}"
        );
        let mut retained_one = 0usize;
        let mut retained_two = 0usize;
        for session in &sessions {
            let raw = serde_cbor::to_vec(session).expect("session serializes");
            assert!(
                raw.len() < budget,
                "session raw {} >= budget {budget}",
                raw.len()
            );
            retained_one += session.one_way_flows.as_ref().map_or(0, Vec::len);
            retained_two += session.two_way_flows.as_ref().map_or(0, Vec::len);
        }
        assert_eq!(
            retained_one, original_one,
            "the split preserves every one-way row"
        );
        assert_eq!(
            retained_two, original_two,
            "the split preserves every two-way row"
        );
    }

    /// STEP 3 TRIM: an atomic whole field that alone exceeds the budget is a
    /// terminal accounted drop (returned in `trimmed`, never shipped).
    #[test]
    fn step3_oversized_atomic_field_is_trimmed_not_shipped() {
        let mut message = IngestSession {
            license_key: Uuid::nil(),
            node_id: "node".to_string(),
            node_name: "Node".to_string(),
            blackboard_json: Some(vec![7u8; 200_000]),
            two_way_flows: Some(vec![two_way(1_000)]),
            ..Default::default()
        };
        let budget = 8 * 1024;
        let (sessions, trimmed) = pack_sessions(&mut message, Uuid::nil(), "node", "Node", budget);
        assert!(
            trimmed
                .iter()
                .any(|(field, _)| *field == SessionField::BlackboardJson),
            "the oversized blackboard must be trimmed: {trimmed:?}"
        );
        for session in &sessions {
            assert!(
                session.blackboard_json.is_none(),
                "a trimmed field must never ship"
            );
            let raw = serde_cbor::to_vec(session).expect("session serializes");
            assert!(
                raw.len() < budget,
                "session raw {} >= budget {budget}",
                raw.len()
            );
        }
    }

    /// `.2.2` Shape A oracle: a board split into fragments emits fragment-only
    /// sessions (one per fragment), each within the budget, whose
    /// `blackboard_json` byte-concatenates back to the original board — exactly
    /// the server's `merge_ingest_sessions` semantics. No trim may increment.
    #[test]
    fn blackboard_fragments_emit_in_budget_sessions_that_reassemble() {
        // Deterministic pseudo-random board spanning > 2 fragments.
        let board: Vec<u8> = (0..(BLACKBOARD_FRAGMENT_MAX_BYTES * 2 + 1234))
            .map(|i| ((i * 31 + 7) % 251) as u8)
            .collect();
        let fragments: Vec<Vec<u8>> = board
            .chunks(BLACKBOARD_FRAGMENT_MAX_BYTES)
            .map(<[u8]>::to_vec)
            .collect();
        assert_eq!(fragments.len(), 3);

        let before = LTS_SESSION_TRIMS.load(Ordering::Relaxed);
        let sessions = emit_blackboard_fragments(
            fragments,
            Uuid::nil(),
            "node",
            "Node",
            LTS_SESSION_BUDGET_BYTES,
        );
        assert_eq!(sessions.len(), 3, "one session per ordered fragment");
        assert_eq!(
            LTS_SESSION_TRIMS.load(Ordering::Relaxed),
            before,
            "no fragment session may be trimmed"
        );

        let mut rebuilt = Vec::new();
        for chunks in &sessions {
            let compressed: Vec<u8> = chunks.iter().flatten().copied().collect();
            let raw =
                miniz_oxide::inflate::decompress_to_vec(&compressed).expect("session inflates");
            assert!(
                raw.len() <= LTS_SESSION_BUDGET_BYTES,
                "fragment session raw {} must fit the budget",
                raw.len()
            );
            let session: IngestSession = serde_cbor::from_slice(&raw).expect("session decodes");
            rebuilt.extend(session.blackboard_json.expect("fragment present"));
        }
        assert_eq!(rebuilt, board, "fragments must reassemble in order");
    }

    /// `.2.2` oracle: two boards in one window must NOT concatenate (the server
    /// byte-concatenates sessions, so board1||board2 would decode as invalid
    /// CBOR / trailing data). The newest board replaces the pending one — the
    /// pre-.2.2 last-wins overwrite — and the emit path reassembles ONLY the
    /// last board.
    #[test]
    fn a_second_board_in_one_window_replaces_the_pending_board() {
        let mut queue = MessageQueue::new();
        queue.ingest(IngestorCommand::BlackboardFragments(vec![
            vec![1u8; 4],
            vec![2u8; 4],
        ]));
        assert_eq!(
            queue.blackboard_fragments,
            vec![vec![1u8; 4], vec![2u8; 4]]
        );

        let latest = vec![vec![9u8; 3], vec![8u8; 5]];
        queue.ingest(IngestorCommand::BlackboardFragments(latest.clone()));
        assert_eq!(
            queue.blackboard_fragments, latest,
            "the newest board replaces (never concatenates) the pending one"
        );

        // The emit path must reassemble ONLY the last board.
        let sessions = emit_blackboard_fragments(
            std::mem::take(&mut queue.blackboard_fragments),
            Uuid::nil(),
            "node",
            "Node",
            LTS_SESSION_BUDGET_BYTES,
        );
        let mut rebuilt = Vec::new();
        for chunks in &sessions {
            let compressed: Vec<u8> = chunks.iter().flatten().copied().collect();
            let raw =
                miniz_oxide::inflate::decompress_to_vec(&compressed).expect("session inflates");
            let session: IngestSession = serde_cbor::from_slice(&raw).expect("session decodes");
            rebuilt.extend(session.blackboard_json.expect("fragment present"));
        }
        let expected: Vec<u8> = latest.into_iter().flatten().collect();
        assert_eq!(rebuilt, expected, "only the last board is emitted");
    }
}

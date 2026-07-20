//! Owns the live active-flow table and publishes lock-free snapshots for readers.

use crate::throughput_tracker::tracking_data::MAX_RETRY_TIMES;

use super::{
    RttBuffer, RttData,
    flow_analysis::{FlowAnalysis, get_asn_name_and_country},
};
use crate::throughput_tracker::flow_data::flow_analysis::FlowbeeEffectiveDirection;
use allocative_derive::Allocative;
use arc_swap::ArcSwap;
use fxhash::FxHashMap;
use lqos_bus::FlowbeeProtocol;
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use lqos_utils::hash_to_i64;
use lqos_utils::qoo::QoqScores;
use lqos_utils::rtt::RttBucket;
use lqos_utils::units::DownUpOrder;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::ser::SerializeStruct;
use serde::ser::SerializeTuple;
use serde::{Serialize, Serializer};
use smallvec::SmallVec;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(test)]
use std::sync::{Mutex as StdMutex, MutexGuard, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Allocative)]
pub struct AsnId(pub u32);

static ALL_FLOWS: Lazy<Mutex<FlowTracker>> = Lazy::new(|| Mutex::new(FlowTracker::default()));
static ACTIVE_FLOW_SNAPSHOT: Lazy<ArcSwap<Vec<ActiveFlowSnapshot>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(Vec::new())));
static ACTIVE_FLOW_LIVE_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Default, Allocative)]
struct FlowTracker {
    flow_data: FxHashMap<FlowbeeKey, (FlowbeeLocalData, FlowAnalysis)>,
}

/// Shared display fields derived when active-flow snapshots are published.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ActiveFlowDisplayFields {
    /// Remote endpoint address as display text.
    pub(crate) remote_ip: String,
    /// Local endpoint address as display text.
    pub(crate) local_ip: String,
    /// Source port from the original flow key.
    pub(crate) src_port: u16,
    /// Destination port from the original flow key.
    pub(crate) dst_port: u16,
    /// Protocol from the original flow key.
    pub(crate) ip_protocol: FlowbeeProtocol,
    /// Remote ASN identifier.
    pub(crate) remote_asn: u32,
    /// Remote ASN display name.
    pub(crate) remote_asn_name: String,
    /// Remote ASN country code.
    pub(crate) remote_asn_country: String,
    /// Flow protocol/application analysis label.
    pub(crate) analysis: String,
}

/// Copied view of an active flow for read-heavy UI, bus, and Insight paths.
///
/// This type intentionally stores only owned data from the live flow table and common display
/// fields derived during the scheduled snapshot refresh.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ActiveFlowSnapshot {
    pub(crate) key: FlowbeeKey,
    pub(crate) display: ActiveFlowDisplayFields,
    pub(crate) bytes_sent: DownUpOrder<u64>,
    pub(crate) packets_sent: DownUpOrder<u64>,
    pub(crate) rate_estimate_bps: DownUpOrder<u32>,
    pub(crate) tcp_retransmits: DownUpOrder<u16>,
    pub(crate) end_status: u8,
    pub(crate) tos: u8,
    pub(crate) flags: u8,
    pub(crate) circuit_hash: Option<i64>,
    pub(crate) device_hash: Option<i64>,
    pub(crate) circuit_id: String,
    pub(crate) circuit_name: String,
    pub(crate) device_name: String,
    pub(crate) last_seen: u64,
    pub(crate) start_time: u64,
    pub(crate) rtt_nanos: DownUpOrder<u64>,
    pub(crate) qoo: DownUpOrder<Option<f32>>,
}

impl ActiveFlowSnapshot {
    /// Returns the flow age relative to the caller's timestamp.
    pub(crate) fn age_nanos(&self, now_nanos: u64) -> u64 {
        now_nanos.saturating_sub(self.last_seen)
    }
}

impl ActiveFlowSnapshot {
    fn from_entry(key: &FlowbeeKey, local: &FlowbeeLocalData, analysis: &FlowAnalysis) -> Self {
        let key = *key;
        Self {
            key,
            display: ActiveFlowDisplayFields {
                remote_ip: key.remote_ip.as_ip().to_string(),
                local_ip: key.local_ip.as_ip().to_string(),
                src_port: key.src_port,
                dst_port: key.dst_port,
                ip_protocol: FlowbeeProtocol::from(key.ip_protocol),
                remote_asn: analysis.asn_id.0,
                remote_asn_name: String::new(),
                remote_asn_country: String::new(),
                analysis: analysis.protocol_analysis.to_string(),
            },
            bytes_sent: local.bytes_sent,
            packets_sent: local.packets_sent,
            rate_estimate_bps: local.rate_estimate_bps,
            tcp_retransmits: local.tcp_retransmits,
            end_status: local.end_status,
            tos: local.tos,
            flags: local.get_flags(),
            circuit_hash: local.circuit_hash,
            device_hash: local.device_hash,
            last_seen: local.last_seen,
            start_time: local.start_time,
            rtt_nanos: DownUpOrder::new(
                local.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download),
                local.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Upload),
            ),
            qoo: {
                let scores = local.get_qoq_scores();
                DownUpOrder::new(scores.download_total_f32(), scores.upload_total_f32())
            },
            circuit_id: local.circuit_id_hint.clone().unwrap_or_default(),
            circuit_name: String::new(),
            device_name: String::new(),
        }
    }

    fn enrich_display_fields(&mut self, catalog: &lqos_network_devices::NetworkDevicesCatalog) {
        let geo = get_asn_name_and_country(self.key.remote_ip.as_ip());
        let device = crate::throughput_tracker::resolve_flow_device(
            catalog,
            &self.key.local_ip,
            self.device_hash,
            self.circuit_hash,
        );
        let (circuit_id, circuit_name) =
            crate::throughput_tracker::flow_circuit_metadata_from_device(
                device,
                (!self.circuit_id.is_empty()).then_some(self.circuit_id.as_str()),
            );

        self.display.remote_asn_name = geo.name;
        self.display.remote_asn_country = geo.country;
        self.circuit_hash = device
            .map(|device| device.circuit_hash)
            .or(self.circuit_hash)
            .or_else(|| (!circuit_id.is_empty()).then(|| hash_to_i64(&circuit_id)));
        self.device_hash = device.map(|device| device.device_hash).or(self.device_hash);
        self.circuit_id = circuit_id;
        self.circuit_name = circuit_name;
        self.device_name = device
            .map(|device| device.device_name.clone())
            .unwrap_or_default();
    }
}

fn copy_active_flow_snapshot(
    flow_data: &FxHashMap<FlowbeeKey, (FlowbeeLocalData, FlowAnalysis)>,
) -> Vec<ActiveFlowSnapshot> {
    flow_data
        .iter()
        .map(|(key, (local, analysis))| ActiveFlowSnapshot::from_entry(key, local, analysis))
        .collect()
}

fn enrich_active_flow_snapshot(
    snapshot: &mut [ActiveFlowSnapshot],
    catalog: &lqos_network_devices::NetworkDevicesCatalog,
) {
    for flow in snapshot {
        flow.enrich_display_fields(catalog);
    }
}

fn publish_active_flow_snapshot(snapshot: Vec<ActiveFlowSnapshot>) {
    ACTIVE_FLOW_SNAPSHOT.store(Arc::new(snapshot));
}

/// Refreshes the cached active-flow snapshot from the live flow table.
///
/// The throughput monitor calls this once per update cycle after live flow writes complete.
pub(in crate::throughput_tracker) fn refresh_active_flow_snapshot() {
    let mut snapshot = {
        let all_flows = ALL_FLOWS.lock();
        copy_active_flow_snapshot(&all_flows.flow_data)
    };
    let catalog = lqos_network_devices::network_devices_catalog();
    enrich_active_flow_snapshot(&mut snapshot, &catalog);
    publish_active_flow_snapshot(snapshot);
}

/// Returns the latest cached active-flow snapshot.
///
/// The snapshot is refreshed by the throughput monitor after the live flow table is updated.
pub(crate) fn active_flow_snapshot() -> Arc<Vec<ActiveFlowSnapshot>> {
    ACTIVE_FLOW_SNAPSHOT.load_full()
}

/// Returns the number of active flows in the latest published snapshot.
#[cfg(test)]
pub(crate) fn active_flow_snapshot_count() -> u64 {
    ACTIVE_FLOW_SNAPSHOT.load().len() as u64
}

/// Returns the latest live active-flow count published by the write gateway.
pub(crate) fn live_active_flow_count() -> u64 {
    ACTIVE_FLOW_LIVE_COUNT.load(Ordering::Relaxed)
}

/// Visits active-flow snapshot rows for a circuit after a recent cutoff.
pub(crate) fn for_each_active_flow_for_circuit(
    circuit_hash: i64,
    recent_cutoff_nanos: u64,
    mut visit: impl FnMut(&ActiveFlowSnapshot),
) {
    let snapshot = active_flow_snapshot();
    for flow in snapshot.iter().filter(|flow| {
        flow.last_seen >= recent_cutoff_nanos
            && (flow.circuit_hash == Some(circuit_hash)
                || (flow.circuit_hash.is_none()
                    && !flow.circuit_id.is_empty()
                    && hash_to_i64(&flow.circuit_id) == circuit_hash))
    }) {
        visit(flow);
    }
}

/// Mutates the live flow table.
///
/// This function is the only write gateway for the live active-flow table. The caller's closure
/// must avoid blocking I/O, channel sends, DNS/network lookups, and other unrelated work while it
/// has access to the live map. Cached reader snapshots are refreshed separately once per throughput
/// update cycle.
pub(in crate::throughput_tracker) fn mutate_all_flows<R>(
    f: impl FnOnce(&mut FxHashMap<FlowbeeKey, (FlowbeeLocalData, FlowAnalysis)>) -> R,
) -> R {
    let mut all_flows = ALL_FLOWS.lock();
    let result = f(&mut all_flows.flow_data);
    ACTIVE_FLOW_LIVE_COUNT.store(all_flows.flow_data.len() as u64, Ordering::Relaxed);
    result
}

/// Converts non-zero retry timestamps with a caller-provided boot-time-to-Unix converter.
pub(crate) fn retry_times_to_unix_seconds(
    times: &[u64],
    to_unix_seconds: impl Fn(u64) -> u64,
) -> Vec<u64> {
    times
        .iter()
        .copied()
        .filter(|timestamp| *timestamp > 0)
        .map(to_unix_seconds)
        .collect()
}

#[cfg(test)]
pub(crate) fn active_flow_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| StdMutex::new(())).lock().unwrap()
}

#[cfg(test)]
fn replace_active_flows_for_test_inner(
    flows: impl IntoIterator<Item = (FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
    publish_snapshot: bool,
) {
    mutate_all_flows(|flow_data| {
        flow_data.clear();
        flow_data.extend(flows);
    });
    if publish_snapshot {
        refresh_active_flow_snapshot();
    }
}

#[cfg(test)]
pub(crate) fn replace_active_flows_for_test(
    flows: impl IntoIterator<Item = (FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
) {
    replace_active_flows_for_test_inner(flows, true);
}

#[cfg(test)]
pub(crate) fn replace_active_flows_live_for_test(
    flows: impl IntoIterator<Item = (FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
) {
    replace_active_flows_for_test_inner(flows, false);
}

#[derive(Clone)]
struct RetryTimesWire {
    idx: usize,
    times: [u64; MAX_RETRY_TIMES],
}

impl Serialize for RetryTimesWire {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tup = serializer.serialize_tuple(2)?;
        tup.serialize_element(&self.idx)?;
        tup.serialize_element(&self.times.as_slice())?;
        tup.end()
    }
}

#[derive(Debug, Clone, Allocative)]
pub struct FlowbeeLocalDataTcp {
    /// Raw TCP flags
    pub flags: u8,
    /// Recent RTT data for the flow
    pub rtt: RttBuffer,
    /// Cached current p50 RTT values for snapshot readers.
    pub summary_rtt_nanos: DownUpOrder<u64>,
    /// QoQ scores (0..100) for the flow, derived from RTT/throughput/retransmits.
    pub qoq: QoqScores,
    /// When did the retries happen? In nanoseconds since kernel boot
    #[allocative(skip)]
    pub retry_times_down: SmallVec<[u64; 2]>,
    /// When did the retries happen? In nanoseconds since kernel boot
    #[allocative(skip)]
    pub retry_times_up: SmallVec<[u64; 2]>,
}

/// Condensed representation of the FlowbeeData type. This contains
/// only the information we want to keep locally for analysis purposes,
/// adds RTT data, and uses Rust-friendly typing.
#[derive(Debug, Clone, Allocative)]
pub struct FlowbeeLocalData {
    /// Time (nanos) when the connection was established
    pub start_time: u64,
    /// Time (nanos) when the connection was last seen
    pub last_seen: u64,
    /// Bytes transmitted
    pub bytes_sent: DownUpOrder<u64>,
    /// Packets transmitted
    pub packets_sent: DownUpOrder<u64>,
    /// Rate estimate
    pub rate_estimate_bps: DownUpOrder<u32>,
    /// Optional UI-oriented display rate. This is populated by specific
    /// websocket/API surfaces that need plan-aware presentation guards.
    pub display_rate_bps: Option<DownUpOrder<u32>>,
    /// TCP Retransmission count (also counts duplicates)
    pub tcp_retransmits: DownUpOrder<u16>,
    /// Has the connection ended?
    /// 0 = Alive, 1 = FIN, 2 = RST
    pub end_status: u8,
    /// Raw IP TOS
    pub tos: u8,
    /// TC handle from the `ip_info` match (0 if unshaped).
    pub tc_handle: u32,
    /// CPU mapping from the `ip_info` match (0 if unshaped).
    pub cpu: u32,
    /// Hashed circuit identifier (bit-pattern of `hash_to_i64` stored as `u64`).
    pub circuit_hash: Option<i64>,
    /// Hashed device identifier (bit-pattern of `hash_to_i64` stored as `u64`).
    pub device_hash: Option<i64>,
    /// Last-known circuit ID copied from the throughput table when catalog metadata is available.
    pub circuit_id_hint: Option<String>,
    /// TCP-only data. Boxed for now; TODO: use a slab/slot type setup for coherence in the future.
    pub tcp_info: Option<Box<FlowbeeLocalDataTcp>>,
}

impl Serialize for FlowbeeLocalData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Note: Keep this wire format stable (UI compatibility) while we refactor internal storage.
        let mut state = serializer.serialize_struct("FlowbeeLocalData", 14)?;
        state.serialize_field("start_time", &self.start_time)?;
        state.serialize_field("last_seen", &self.last_seen)?;
        state.serialize_field("bytes_sent", &self.bytes_sent)?;
        state.serialize_field("packets_sent", &self.packets_sent)?;
        state.serialize_field("rate_estimate_bps", &self.rate_estimate_bps)?;
        if let Some(display_rate_bps) = &self.display_rate_bps {
            state.serialize_field("display_rate_bps", display_rate_bps)?;
        }
        state.serialize_field("tcp_retransmits", &self.tcp_retransmits)?;
        state.serialize_field("end_status", &self.end_status)?;
        state.serialize_field("tos", &self.tos)?;

        // TCP-only fields (default to zero/None if this isn't a TCP flow).
        state.serialize_field("flags", &self.get_flags())?;
        state.serialize_field("rtt", &self.get_rtt_array())?;
        state.serialize_field("qoq", &self.get_qoq_scores())?;
        let retry_times_down = self.get_retry_times_down_wire();
        let retry_times_up = self.get_retry_times_up_wire();
        state.serialize_field("retry_times_down", &retry_times_down)?;
        state.serialize_field("retry_times_up", &retry_times_up)?;

        state.end()
    }
}

impl FlowbeeLocalData {
    pub fn from_flow(data: &FlowbeeData, key: &FlowbeeKey) -> Self {
        Self {
            start_time: data.start_time,
            last_seen: data.last_seen,
            bytes_sent: data.bytes_sent,
            packets_sent: data.packets_sent,
            rate_estimate_bps: data.rate_estimate_bps,
            display_rate_bps: None,
            tcp_retransmits: data.tcp_retransmits,
            end_status: data.end_status,
            tos: data.tos,
            tc_handle: data.tc_handle,
            cpu: data.cpu,
            circuit_hash: if data.circuit_hash == 0 {
                None
            } else {
                Some(data.circuit_hash as i64)
            },
            device_hash: if data.device_hash == 0 {
                None
            } else {
                Some(data.device_hash as i64)
            },
            circuit_id_hint: None,
            tcp_info: if key.ip_protocol == 6 {
                Some(Box::new(FlowbeeLocalDataTcp {
                    flags: data.flags,
                    rtt: RttBuffer::default(),
                    summary_rtt_nanos: DownUpOrder::zeroed(),
                    qoq: QoqScores::default(),
                    retry_times_down: SmallVec::new(),
                    retry_times_up: SmallVec::new(),
                }))
            } else {
                None
            },
        }
    }

    pub fn get_summary_rtt_as_nanos(&self, direction: FlowbeeEffectiveDirection) -> u64 {
        let Some(tcp_info) = &self.tcp_info else {
            return 0;
        };
        match direction {
            FlowbeeEffectiveDirection::Download => tcp_info.summary_rtt_nanos.down,
            FlowbeeEffectiveDirection::Upload => tcp_info.summary_rtt_nanos.up,
        }
    }

    pub fn get_summary_rtt_as_micros(&self, direction: FlowbeeEffectiveDirection) -> f64 {
        self.get_summary_rtt_as_nanos(direction) as f64 / 1_000.0
    }

    pub fn get_retry_times_down(&self) -> &[u64] {
        let Some(tcp_info) = &self.tcp_info else {
            return &[];
        };
        tcp_info.retry_times_down.as_slice()
    }

    pub fn get_retry_times_up(&self) -> &[u64] {
        let Some(tcp_info) = &self.tcp_info else {
            return &[];
        };
        tcp_info.retry_times_up.as_slice()
    }

    fn retry_times_to_wire(times: &[u64]) -> Option<RetryTimesWire> {
        if times.is_empty() {
            return None;
        }

        let mut buffer = [0u64; MAX_RETRY_TIMES];
        let count = usize::min(times.len(), MAX_RETRY_TIMES);
        buffer[..count].copy_from_slice(&times[..count]);
        Some(RetryTimesWire {
            idx: count,
            times: buffer,
        })
    }

    fn get_retry_times_down_wire(&self) -> Option<RetryTimesWire> {
        let Some(tcp_info) = &self.tcp_info else {
            return None;
        };
        Self::retry_times_to_wire(tcp_info.retry_times_down.as_slice())
    }

    fn get_retry_times_up_wire(&self) -> Option<RetryTimesWire> {
        let Some(tcp_info) = &self.tcp_info else {
            return None;
        };
        Self::retry_times_to_wire(tcp_info.retry_times_up.as_slice())
    }

    pub fn get_rtt_array(&self) -> [RttData; 2] {
        let Some(tcp_info) = &self.tcp_info else {
            return [RttData::from_nanos(0); 2];
        };
        [
            tcp_info
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
                .unwrap_or(RttData::from_nanos(0)),
            tcp_info
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50)
                .unwrap_or(RttData::from_nanos(0)),
        ]
    }

    pub fn get_qoq_scores(&self) -> QoqScores {
        let Some(tcp_info) = &self.tcp_info else {
            return QoqScores::default();
        };
        tcp_info.qoq
    }

    pub fn get_flags(&self) -> u8 {
        let Some(tcp_info) = &self.tcp_info else {
            return 0;
        };
        tcp_info.flags
    }

    pub fn set_last_seen(&mut self, last_seen: u64) {
        self.last_seen = last_seen;
    }

    pub fn set_bytes_sent(&mut self, bytes_sent: DownUpOrder<u64>) {
        self.bytes_sent = bytes_sent;
    }

    pub fn set_packets_sent(&mut self, packets_sent: DownUpOrder<u64>) {
        self.packets_sent = packets_sent;
    }

    pub fn set_rate_estimate_bps(&mut self, rate_estimate_bps: DownUpOrder<u32>) {
        self.rate_estimate_bps = rate_estimate_bps;
    }

    pub fn set_tcp_retransmits(&mut self, tcp_retransmits: DownUpOrder<u16>) {
        self.tcp_retransmits = tcp_retransmits;
    }

    pub fn set_end_status(&mut self, end_status: u8) {
        self.end_status = end_status;
    }

    pub fn set_tos(&mut self, tos: u8) {
        self.tos = tos;
    }

    pub fn set_flags(&mut self, flags: u8) {
        let Some(tcp_info) = &mut self.tcp_info else {
            return;
        };
        tcp_info.flags = flags;
    }

    /// Updates the live circuit/device metadata copied into active-flow snapshots.
    pub fn set_tracking_metadata(
        &mut self,
        circuit_hash: Option<i64>,
        device_hash: Option<i64>,
        circuit_id: Option<&str>,
    ) {
        if circuit_hash.is_some() {
            self.circuit_hash = circuit_hash;
        }
        if device_hash.is_some() {
            self.device_hash = device_hash;
        }
        self.set_circuit_id_hint(circuit_id);
    }

    /// Updates the last-known circuit ID hint without clearing it on transient misses.
    pub fn set_circuit_id_hint(&mut self, circuit_id: Option<&str>) {
        let Some(circuit_id) = circuit_id.filter(|id| !id.is_empty()) else {
            return;
        };
        if self.circuit_id_hint.as_deref() != Some(circuit_id) {
            self.circuit_id_hint = Some(circuit_id.to_string());
        }
    }

    pub fn set_rtt_buffer(&mut self, rtt: RttBuffer) {
        let Some(tcp_info) = &mut self.tcp_info else {
            return;
        };
        tcp_info.rtt.merge_fresh_from(rtt);
        tcp_info.summary_rtt_nanos = DownUpOrder::new(
            tcp_info
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
                .unwrap_or(RttData::from_nanos(0))
                .as_nanos(),
            tcp_info
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50)
                .unwrap_or(RttData::from_nanos(0))
                .as_nanos(),
        );
    }

    /// Retires the raw RTT histogram while retaining the last-known summary and QoO values.
    pub fn retire_stale_rtt(&mut self, expire_before_nanos: u64) -> bool {
        let Some(tcp_info) = &mut self.tcp_info else {
            return false;
        };
        if tcp_info.rtt.last_seen > expire_before_nanos {
            return false;
        }

        tcp_info.rtt.clear();
        true
    }

    pub fn set_qoq_scores(&mut self, scores: QoqScores) {
        let Some(tcp_info) = &mut self.tcp_info else {
            return;
        };
        tcp_info.qoq = scores;
    }

    pub fn record_tcp_retry_time(
        &mut self,
        direction: FlowbeeEffectiveDirection,
        timestamp_nanos: u64,
    ) {
        let Some(tcp_info) = &mut self.tcp_info else {
            return;
        };

        let target = match direction {
            FlowbeeEffectiveDirection::Download => &mut tcp_info.retry_times_down,
            FlowbeeEffectiveDirection::Upload => &mut tcp_info.retry_times_up,
        };

        // Keep the most recent `MAX_RETRY_TIMES` entries.
        if target.len() >= MAX_RETRY_TIMES {
            // Not a hot path, and MAX is small enough that shifting is OK.
            target.remove(0);
        }
        target.push(timestamp_nanos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::throughput_tracker::flow_data::flow_analysis::FlowProtocol;
    use lqos_utils::qoo::QoqScores;

    fn sample_flow() -> (FlowbeeKey, FlowbeeLocalData, FlowAnalysis) {
        let mut key = FlowbeeKey::default();
        key.ip_protocol = 6;
        key.src_port = 443;
        key.dst_port = 51_515;

        let mut raw = FlowbeeData::default();
        raw.start_time = 10;
        raw.last_seen = 20;
        raw.bytes_sent = DownUpOrder::new(1_000, 2_000);
        raw.packets_sent = DownUpOrder::new(10, 20);
        raw.rate_estimate_bps = DownUpOrder::new(30_000, 40_000);
        raw.tcp_retransmits = DownUpOrder::new(1, 2);
        raw.end_status = 0;
        raw.tos = 4;
        raw.flags = 0x12;
        raw.tc_handle = 123;
        raw.cpu = 2;
        raw.circuit_hash = 44;
        raw.device_hash = 55;

        let mut local = FlowbeeLocalData::from_flow(&raw, &key);
        let mut rtt = RttBuffer::new(
            RttData::from_nanos(12_000_000),
            FlowbeeEffectiveDirection::Download,
            raw.last_seen,
        );
        rtt.push(
            RttData::from_nanos(12_000_000),
            FlowbeeEffectiveDirection::Download,
            raw.last_seen,
        );
        rtt.push(
            RttData::from_nanos(34_000_000),
            FlowbeeEffectiveDirection::Upload,
            raw.last_seen,
        );
        rtt.push(
            RttData::from_nanos(34_000_000),
            FlowbeeEffectiveDirection::Upload,
            raw.last_seen,
        );
        local.set_rtt_buffer(rtt);
        local.set_qoq_scores(QoqScores {
            download_total: 77,
            upload_total: 66,
        });
        let analysis = FlowAnalysis {
            asn_id: AsnId(64_512),
            protocol_analysis: FlowProtocol::Https,
        };
        (key, local, analysis)
    }

    #[test]
    fn active_flow_snapshot_copies_live_flow_fields() {
        let (key, mut local, analysis) = sample_flow();
        local.set_circuit_id_hint(Some("hint-circuit"));
        let mut flows = FxHashMap::default();
        flows.insert(key, (local.clone(), analysis));

        let mut snapshot = copy_active_flow_snapshot(&flows);
        let catalog = lqos_network_devices::NetworkDevicesCatalog::from_snapshots(
            lqos_network_devices::ShapedDevicesCatalog::from_shaped_devices(Arc::new(
                lqos_config::ConfigShapedDevices::default(),
            )),
            Arc::new(Vec::new()),
        );
        enrich_active_flow_snapshot(&mut snapshot, &catalog);

        assert_eq!(snapshot.len(), 1);
        let row = &snapshot[0];
        assert_eq!(row.key, key);
        assert_eq!(row.display.remote_ip, key.remote_ip.as_ip().to_string());
        assert_eq!(row.display.local_ip, key.local_ip.as_ip().to_string());
        assert_eq!(row.display.src_port, key.src_port);
        assert_eq!(row.display.dst_port, key.dst_port);
        assert_eq!(row.display.ip_protocol, FlowbeeProtocol::TCP);
        assert_eq!(row.display.remote_asn, analysis.asn_id.0);
        assert_eq!(row.display.analysis, "HTTPS");
        assert_eq!(row.bytes_sent, local.bytes_sent);
        assert_eq!(row.packets_sent, local.packets_sent);
        assert_eq!(row.rate_estimate_bps, local.rate_estimate_bps);
        assert_eq!(row.tcp_retransmits, local.tcp_retransmits);
        assert_eq!(row.end_status, local.end_status);
        assert_eq!(row.tos, local.tos);
        assert_eq!(row.flags, local.get_flags());
        assert_eq!(row.circuit_hash, Some(44));
        assert_eq!(row.device_hash, Some(55));
        assert_eq!(row.circuit_id, "hint-circuit");
        assert_eq!(row.circuit_name, "");
        assert_eq!(row.device_name, "");
        assert_eq!(row.last_seen, local.last_seen);
        assert_eq!(row.start_time, local.start_time);
        assert_eq!(row.rtt_nanos, DownUpOrder::new(14_000_000, 35_000_000));
        assert_eq!(row.qoo, DownUpOrder::new(Some(77.0), Some(66.0)));
    }

    #[test]
    fn retry_times_to_unix_seconds_filters_zeroes_and_preserves_order() {
        let converted =
            retry_times_to_unix_seconds(&[0, 2_000_000_000, 1_000_000_000, 0], |timestamp| {
                1_700_000_000 + timestamp / 1_000_000_000
            });

        assert_eq!(converted, vec![1_700_000_002, 1_700_000_001]);
    }

    #[test]
    fn retire_stale_rtt_keeps_cached_summary_and_qoo() {
        let (_key, mut local, _analysis) = sample_flow();
        assert_eq!(
            local.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download),
            14_000_000
        );
        assert_eq!(
            local.get_rtt_array()[0].as_nanos(),
            local.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download)
        );
        assert_eq!(local.get_qoq_scores().download_total_f32(), Some(77.0));

        assert!(local.retire_stale_rtt(21));

        assert_eq!(
            local.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download),
            14_000_000
        );
        assert_eq!(
            local.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Upload),
            35_000_000
        );
        assert_eq!(local.get_rtt_array()[0].as_nanos(), 0);
        assert_eq!(local.get_rtt_array()[1].as_nanos(), 0);
        assert_eq!(local.get_qoq_scores().download_total_f32(), Some(77.0));

        let (_key, mut boundary_local, _analysis) = sample_flow();
        assert!(!boundary_local.retire_stale_rtt(19));
        assert_ne!(boundary_local.get_rtt_array()[0].as_nanos(), 0);
        assert!(boundary_local.retire_stale_rtt(20));
        assert_eq!(boundary_local.get_rtt_array()[0].as_nanos(), 0);
    }

    #[test]
    fn tracking_metadata_survives_transient_missing_metadata() {
        let (_key, mut local, _analysis) = sample_flow();

        local.set_tracking_metadata(Some(100), Some(200), Some("circuit-one"));
        local.set_tracking_metadata(Some(101), Some(201), None);
        local.set_tracking_metadata(None, None, None);

        assert_eq!(local.circuit_hash, Some(101));
        assert_eq!(local.device_hash, Some(201));
        assert_eq!(local.circuit_id_hint.as_deref(), Some("circuit-one"));
    }

    #[test]
    fn circuit_snapshot_reader_matches_preserved_circuit_id_when_hash_is_missing() {
        let _guard = active_flow_test_lock();
        let (key, mut local, analysis) = sample_flow();
        local.circuit_hash = None;
        local.device_hash = None;
        local.set_circuit_id_hint(Some("hint-circuit"));

        mutate_all_flows(|flows| {
            flows.clear();
            flows.insert(key, (local, analysis));
        });
        refresh_active_flow_snapshot();

        let mut matched = Vec::new();
        for_each_active_flow_for_circuit(hash_to_i64("hint-circuit"), 0, |flow| {
            matched.push(flow.key);
        });

        assert_eq!(matched, vec![key]);

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
    }

    #[test]
    fn circuit_snapshot_reader_does_not_match_stale_hint_when_hash_is_present() {
        let _guard = active_flow_test_lock();
        let (key, mut local, analysis) = sample_flow();
        local.circuit_hash = Some(hash_to_i64("current-circuit"));
        local.device_hash = None;
        local.set_circuit_id_hint(Some("old-circuit"));

        mutate_all_flows(|flows| {
            flows.clear();
            flows.insert(key, (local, analysis));
        });
        refresh_active_flow_snapshot();

        let mut current_matches = Vec::new();
        for_each_active_flow_for_circuit(hash_to_i64("current-circuit"), 0, |flow| {
            current_matches.push(flow.key);
        });
        let mut old_matches = Vec::new();
        for_each_active_flow_for_circuit(hash_to_i64("old-circuit"), 0, |flow| {
            old_matches.push(flow.key);
        });

        assert_eq!(current_matches, vec![key]);
        assert!(old_matches.is_empty());

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
    }

    #[test]
    fn refresh_active_flow_snapshot_publishes_count_and_stable_arcs() {
        let _guard = active_flow_test_lock();
        let (key, local, analysis) = sample_flow();

        mutate_all_flows(|flows| {
            flows.clear();
            flows.insert(key, (local, analysis));
        });
        refresh_active_flow_snapshot();

        let snapshot = active_flow_snapshot();
        assert_eq!(active_flow_snapshot_count(), 1);
        assert_eq!(live_active_flow_count(), 1);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].key, key);

        mutate_all_flows(|flows| {
            flows.clear();
        });
        refresh_active_flow_snapshot();

        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].key, key);
        assert_eq!(active_flow_snapshot_count(), 0);
        assert_eq!(live_active_flow_count(), 0);
        assert!(active_flow_snapshot().is_empty());
    }
}

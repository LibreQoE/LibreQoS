pub mod flow_data;
pub(crate) mod stats_submission;
mod throughput_entry;
mod tracking_data;

use self::flow_data::{
    ActiveFlowSnapshot, active_flow_snapshot, get_asn_name_and_country, get_asn_name_by_id,
    live_active_flow_count, lookup_asn_id, snapshot_asn_heatmaps,
};
use self::throughput_entry::ThroughputEntry;
use crate::throughput_tracker::flow_data::FlowbeeEffectiveDirection;
use crate::throughput_tracker::tracking_data::ThroughputTracker;
use arc_swap::ArcSwap;
pub(crate) use flow_data::RttBuffer;
use fxhash::{FxHashMap, FxHashSet};
use lqos_bus::{
    AsnHeatmapData, BusResponse, CircuitHeatmapData, ExecutiveSummaryHeader, IpStats,
    SiteHeatmapData, TcHandle, TopFlowType, XdpPpingResult,
};
use lqos_queue_tracker::{ALL_QUEUE_SUMMARY, queue_stats_stale};
#[cfg(test)]
use lqos_utils::qoo::QoqScores;
use lqos_utils::rtt::RttBucket;
#[cfg(test)]
use lqos_utils::rtt::RttData;
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample, down_up_retransmit_sample};
use lqos_utils::{XdpIpAddress, hash_to_i64, unix_time::time_since_boot};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::time::Duration;
use tracing::warn;

const RETIRE_AFTER_SECONDS: u64 = 30;

pub static THROUGHPUT_TRACKER: Lazy<ThroughputTracker> = Lazy::new(ThroughputTracker::new);
pub(crate) static CIRCUIT_RTT_BUFFERS: Lazy<ArcSwap<FxHashMap<i64, RttBuffer>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(FxHashMap::default())));

/// Returns the current per-circuit RTT p50 values from the shared circuit RTT buffers.
pub(crate) fn circuit_current_rtt_p50_nanos(circuit_hash: i64) -> DownUpOrder<Option<u64>> {
    let snapshot = CIRCUIT_RTT_BUFFERS.load();
    let rtt = snapshot.get(&circuit_hash);

    DownUpOrder {
        down: rtt
            .and_then(|rtt| {
                rtt.percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
            })
            .map(|rtt| rtt.as_nanos()),
        up: rtt
            .and_then(|rtt| {
                rtt.percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50)
            })
            .map(|rtt| rtt.as_nanos()),
    }
}

/// Returns the current per-circuit QoO values from the shared circuit QoO heatmaps.
pub(crate) fn circuit_current_qoo(circuit_hash: i64) -> DownUpOrder<Option<f32>> {
    // 3a group-1 increment 2: bridge boxes read the accumulator snapshot.
    if let Some(snapshot) = crate::bridge_analytics::circuit_qoo_snapshot() {
        return snapshot
            .by_circuit
            .iter()
            .find(|(circuit_id, _)| hash_to_i64(circuit_id) == circuit_hash)
            .map(|(_, blocks)| blocks.current_values())
            .unwrap_or_default();
    }
    let qoq_heatmaps = THROUGHPUT_TRACKER.circuit_qoq_heatmaps.lock();
    let Some(heatmap) = qoq_heatmaps.get(&circuit_hash) else {
        return DownUpOrder::default();
    };
    let blocks = heatmap.blocks();
    blocks.current_values()
}

/// Resolves the shaped-device row for an active flow from its hashes, then its local IP.
pub(crate) fn resolve_flow_device<'a>(
    catalog: &'a lqos_network_devices::NetworkDevicesCatalog,
    ip: &XdpIpAddress,
    device_hash: Option<i64>,
    circuit_hash: Option<i64>,
) -> Option<&'a lqos_config::ShapedDevice> {
    catalog
        .device_by_hashes(device_hash, circuit_hash)
        .or_else(|| catalog.device_longest_match_for_ip(ip).map(|(_, dev)| dev))
}

/// Resolves circuit ID/name metadata for an active-flow or throughput row.
pub(crate) fn flow_circuit_metadata_from_device(
    device: Option<&lqos_config::ShapedDevice>,
    circuit_id_hint: Option<&str>,
) -> (String, String) {
    let mut circuit_id = circuit_id_hint.unwrap_or_default().to_string();
    let mut circuit_name = String::new();

    if let Some(device) = device {
        if circuit_id.is_empty() {
            circuit_id = device.circuit_id.clone();
        }
        circuit_name = device.circuit_name.clone();
    }

    (circuit_id, circuit_name)
}

fn resolve_circuit_metadata_for_entry(
    catalog: &lqos_network_devices::NetworkDevicesCatalog,
    ip: &XdpIpAddress,
    entry: &ThroughputEntry,
) -> (String, String) {
    flow_circuit_metadata_from_device(
        resolve_flow_device(catalog, ip, entry.device_hash, entry.circuit_hash),
        entry.circuit_id.as_deref(),
    )
}

pub(crate) fn resolve_circuit_metadata_for_ip(ip: &XdpIpAddress) -> (String, String) {
    let catalog = lqos_network_devices::network_devices_catalog();
    let throughput = THROUGHPUT_TRACKER.raw_data.lock();
    let Some(entry) = throughput.get(ip) else {
        return (String::new(), String::new());
    };
    resolve_circuit_metadata_for_entry(&catalog, ip, entry)
}

#[cfg(test)]
pub(crate) struct RawThroughputTestEntry {
    pub(crate) ip: XdpIpAddress,
    pub(crate) circuit_hash: Option<i64>,
    pub(crate) device_hash: Option<i64>,
    pub(crate) most_recent_cycle: u64,
    pub(crate) bytes_per_second: DownUpOrder<u64>,
    pub(crate) tcp_packets: DownUpOrder<u64>,
    pub(crate) tcp_retransmits: DownUpOrder<u64>,
}

#[cfg(test)]
pub(crate) struct RawThroughputTestGuard {
    old_cycle: u64,
    old_raw_data: Option<std::collections::HashMap<XdpIpAddress, ThroughputEntry>>,
}

#[cfg(test)]
impl Drop for RawThroughputTestGuard {
    fn drop(&mut self) {
        THROUGHPUT_TRACKER
            .cycle
            .store(self.old_cycle, std::sync::atomic::Ordering::Relaxed);
        if let Some(old_raw_data) = self.old_raw_data.take() {
            *THROUGHPUT_TRACKER.raw_data.lock() = old_raw_data;
        }
    }
}

#[cfg(test)]
pub(crate) fn replace_raw_throughput_for_test(
    cycle: u64,
    entries: Vec<RawThroughputTestEntry>,
) -> RawThroughputTestGuard {
    let old_cycle = THROUGHPUT_TRACKER
        .cycle
        .swap(cycle, std::sync::atomic::Ordering::Relaxed);
    let mut raw_data = THROUGHPUT_TRACKER.raw_data.lock();
    let old_raw_data = std::mem::take(&mut *raw_data);
    for entry in entries {
        raw_data.insert(
            entry.ip,
            ThroughputEntry {
                circuit_id: None,
                circuit_hash: entry.circuit_hash,
                device_hash: entry.device_hash,
                network_json_parents: None,
                first_cycle: 0,
                most_recent_cycle: entry.most_recent_cycle,
                bytes: DownUpOrder::zeroed(),
                actual_bytes: DownUpOrder::zeroed(),
                packets: DownUpOrder::zeroed(),
                tcp_packets: entry.tcp_packets,
                udp_packets: DownUpOrder::zeroed(),
                icmp_packets: DownUpOrder::zeroed(),
                prev_bytes: DownUpOrder::zeroed(),
                prev_actual_bytes: DownUpOrder::zeroed(),
                prev_packets: DownUpOrder::zeroed(),
                prev_tcp_packets: DownUpOrder::zeroed(),
                prev_udp_packets: DownUpOrder::zeroed(),
                prev_icmp_packets: DownUpOrder::zeroed(),
                bytes_per_second: entry.bytes_per_second,
                actual_bytes_per_second: DownUpOrder::zeroed(),
                packets_per_second: DownUpOrder::zeroed(),
                tc_handle: TcHandle::from_u32(0),
                rtt_buffer: RttBuffer::default(),
                recent_rtt_data: [RttData::from_nanos(0); 60],
                last_fresh_rtt_data_cycle: 0,
                last_seen: 0,
                tcp_retransmits: entry.tcp_retransmits,
                tcp_retransmit_packets: DownUpOrder::zeroed(),
                qoq: QoqScores::default(),
            },
        );
    }
    RawThroughputTestGuard {
        old_cycle,
        old_raw_data: Some(old_raw_data),
    }
}

pub fn current_throughput() -> BusResponse {
    // K2 slice 1 source-dispatch: on a bridge box the kernel monitor is not
    // the owner — read the bridge's own top-level interval counters.
    // 3b §1.1: the pps series folds the actor's per-IP SENT rows (all
    // transports), and the tcp/udp/icmp splits come from the per-flow packet
 // delta seed ruling c) — CLASSIFIED-only, so the split sum tracks
    // the total up to the Unknown-transport residual.
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        let protocol = snapshot.protocol_packets_per_second;
 // 3b display semantics: customer-facing throughput = the
        // DELIVERED (shaped) rate everywhere; the pre-shaping ingress demand is
        // a separate labeled metric, never this headline.
        return BusResponse::CurrentThroughput {
            bits_per_second: snapshot.bridge_shaped_bytes_per_second.to_bits_from_bytes(),
            packets_per_second: crate::bridge_analytics::total_sent_packets_per_second(
                &snapshot.by_ip,
            ),
            shaped_bits_per_second: snapshot.bridge_shaped_bytes_per_second.to_bits_from_bytes(),
            tcp_packets_per_second: protocol.tcp,
            udp_packets_per_second: protocol.udp,
            icmp_packets_per_second: protocol.icmp,
        };
    }
    let (bits_per_second, packets_per_second, shaped_bits_per_second, tcp_pps, udp_pps, icmp_pps) = {
        (
            THROUGHPUT_TRACKER.actual_bits_per_second(),
            THROUGHPUT_TRACKER.packets_per_second(),
            THROUGHPUT_TRACKER.shaped_actual_bits_per_second(),
            THROUGHPUT_TRACKER.tcp_packets_per_second(),
            THROUGHPUT_TRACKER.udp_packets_per_second(),
            THROUGHPUT_TRACKER.icmp_packets_per_second(),
        )
    };
    BusResponse::CurrentThroughput {
        bits_per_second,
        packets_per_second,
        shaped_bits_per_second,
        tcp_packets_per_second: tcp_pps,
        udp_packets_per_second: udp_pps,
        icmp_packets_per_second: icmp_pps,
    }
}

pub fn host_counters() -> BusResponse {
    // K2 slice-3 3a: on a bridge box the per-IP actual interval counters are
    // in the actor cache; the kernel tracker is never written post-K3.
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
 // 3b demand-source sweep: the host rate map reports the
        // DELIVERED (shaped) series.
        let result = snapshot
            .by_ip
            .iter()
            .map(|(ip, row)| (*ip, row.shaped_bytes_per_second))
            .collect();
        return BusResponse::HostCounters(result);
    }
    let mut result = Vec::new();
    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .for_each(|(k, v)| {
            let ip = k.as_ip();
            result.push((ip, v.bytes_per_second));
        });
    BusResponse::HostCounters(result)
}

#[inline(always)]
pub(crate) fn retire_check(cycle: u64, recent_cycle: u64) -> bool {
    cycle < recent_cycle + RETIRE_AFTER_SECONDS
}

type TopList = (
    XdpIpAddress,
    DownUpOrder<u64>,
    DownUpOrder<u64>,
    f32,
    TcHandle,
    String,
    DownUpOrder<TcpRetransmitSample>,
);

/// K2 slice-3 3a: one bridge per-IP row for the ranked tracker readers. The
/// kernel `TopList` analog off `BridgeStatsSnapshot.by_ip`: actual interval
/// rates, the RTT p50, the retransmit sample, and the catalog circuit join.
/// `tc_handle` has no bridge source (default); the bridge wire is
/// direction-agnostic (the same value is copied into both directions by
/// `translate`), so downstream directional splits are honest-equal.
struct BridgeIpRow {
    ip: IpAddr,
    bytes: DownUpOrder<u64>,
    packets: DownUpOrder<u64>,
    rtt_ms: Option<f32>,
    circuit_hash: Option<i64>,
    runtime_circuit_id: String,
    retransmits: DownUpOrder<TcpRetransmitSample>,
}

impl BridgeIpRow {
    fn to_ip_stats(&self) -> IpStats {
        IpStats {
            ip_address: self.ip.to_string(),
            circuit_id: self.runtime_circuit_id.clone(),
            bits_per_second: self.bytes.to_bits_from_bytes(),
            packets_per_second: self.packets,
            median_tcp_rtt: self.rtt_ms.unwrap_or(0.0),
            tc_handle: TcHandle::default(),
            tcp_retransmit_sample: self.retransmits,
        }
    }
}

/// The bridge-ranked reader rows (empty + stale-safe on a kernel box is not
/// reachable here — callers gate on `is_active`).
fn bridge_ip_rows() -> Vec<BridgeIpRow> {
    let snapshot = crate::bridge_analytics::snapshot();
    let catalog = lqos_network_devices::network_devices_catalog();
    snapshot
        .by_ip
        .iter()
        .filter(|(ip, _)| !ip.is_loopback())
        .map(|(ip, live)| {
            let (circuit_hash, circuit_id) = catalog
                .device_longest_match_for_ip(&XdpIpAddress::from_ip(*ip))
                .map(|(_, device)| (Some(device.circuit_hash), device.circuit_id.clone()))
                .unwrap_or((None, String::new()));
            BridgeIpRow {
                ip: *ip,
 // 3b demand-source sweep: ranked/displayed throughput
                // is the DELIVERED (shaped) series.
                bytes: live.shaped_bytes_per_second,
                packets: live.packets_per_second,
                rtt_ms: live
                    .rtt_current_p50_nanos
                    .map(|nanos| nanos as f32 / 1_000_000.0),
                circuit_hash,
                runtime_circuit_id: circuit_id,
                retransmits: down_up_retransmit_sample(
                    DownUpOrder::new(live.tcp_retransmits, 0),
                    DownUpOrder::new(live.tcp_packets, 0),
                ),
            }
        })
        .collect()
}

/// §`lqos-74t.49.2` REQUIRED-3 (CALL-SITE-ONLY repoint): the best-effort /
/// unmapped host-class reader — a sibling of `bridge_ip_rows`. The SHARED
/// `bridge_ip_rows` keeps its FOUR shaped ranked callers byte-identical on
/// `snapshot.by_ip`; ONLY `all_unknown_ips` reads this map. The class has no flow
/// deltas (its rate fields stay 0); the reader carries the host identity + the
/// catalog-miss filter.
fn bridge_best_effort_ip_rows() -> Vec<BridgeIpRow> {
    let snapshot = crate::bridge_analytics::snapshot();
    let catalog = lqos_network_devices::network_devices_catalog();
    snapshot
        .best_effort_by_ip
        .iter()
        .filter(|(ip, _)| !ip.is_loopback())
        .map(|(ip, live)| {
            let (circuit_hash, circuit_id) = catalog
                .device_longest_match_for_ip(&XdpIpAddress::from_ip(*ip))
                .map(|(_, device)| (Some(device.circuit_hash), device.circuit_id.clone()))
                .unwrap_or((None, String::new()));
            BridgeIpRow {
                ip: *ip,
                // The best-effort class has no interval rate (no flow deltas);
                // `bytes_per_second` is 0. The unknown-IP PAGE shows the
                // cumulative `total_bytes` directly.
                bytes: live.bytes_per_second,
                packets: live.packets_per_second,
                rtt_ms: live
                    .rtt_current_p50_nanos
                    .map(|nanos| nanos as f32 / 1_000_000.0),
                circuit_hash,
                runtime_circuit_id: circuit_id,
                retransmits: down_up_retransmit_sample(
                    DownUpOrder::new(live.tcp_retransmits, 0),
                    DownUpOrder::new(live.tcp_packets, 0),
                ),
            }
        })
        .collect()
}

pub fn top_n(start: u32, end: u32) -> BusResponse {
    // K2 slice 1 source-dispatch: bridge mode ranks the per-IP interval
    // rates from the actor cache (packets are not on the wire; RTT is the
    // per-IP p50; the circuit comes from the catalog join; TC handle is a
    // kernel concept and stays default).
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        let catalog = lqos_network_devices::network_devices_catalog();
        let mut rows: Vec<(&IpAddr, &crate::bridge_analytics::IpLiveRow)> = snapshot
            .by_ip
            .iter()
            .filter(|(ip, _)| !ip.is_loopback())
            .collect();
 // 3b demand-source sweep: rank + report the DELIVERED
        // (shaped) series; `bytes_per_second` is the pre-shaping demand.
        rows.sort_by_key(|(_, row)| std::cmp::Reverse(row.shaped_bytes_per_second.down));
        let result = rows
            .iter()
            .skip(start as usize)
            .take(end.saturating_sub(start) as usize)
            .map(|(ip, row)| {
                let (circuit_id, _circuit_name) = catalog
                    .device_longest_match_for_ip(&XdpIpAddress::from_ip(**ip))
                    .map(|(_, device)| (device.circuit_id.clone(), device.circuit_name.clone()))
                    .unwrap_or_default();
                IpStats {
                    ip_address: ip.to_string(),
                    circuit_id,
                    bits_per_second: row.shaped_bytes_per_second.to_bits_from_bytes(),
                    packets_per_second: row.packets_per_second,
                    median_tcp_rtt: row
                        .rtt_current_p50_nanos
                        .map(|nanos| nanos as f32 / 1_000_000.0)
                        .unwrap_or(0.0),
                    tc_handle: TcHandle::default(),
                    tcp_retransmit_sample: down_up_retransmit_sample(
                        DownUpOrder::new(row.tcp_retransmits, 0),
                        DownUpOrder::new(row.tcp_packets, 0),
                    ),
                }
            })
            .collect();
        return BusResponse::TopDownloaders(result);
    }
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        let catalog = lqos_network_devices::network_devices_catalog();
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .map(|(k, te)| {
                let (circuit_id, _circuit_name) =
                    resolve_circuit_metadata_for_entry(&catalog, k, te);
                (
                    *k,
                    te.actual_bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    circuit_id,
                    down_up_retransmit_sample(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    full_list.sort_by_key(|row| std::cmp::Reverse(row.1.down));
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, circuit_id, tcp_retransmits)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: *tcp_retransmits,
            },
        )
        .collect();
    BusResponse::TopDownloaders(result)
}

pub fn top_n_up(start: u32, end: u32) -> BusResponse {
    // K2 slice-3 3a: mirror the download-side bridge dispatch, ranking the
    // per-IP upload interval rate.
    if crate::bridge_analytics::is_active() {
        let mut rows = bridge_ip_rows();
        rows.sort_by_key(|row| std::cmp::Reverse(row.bytes.up));
        let result = rows
            .iter()
            .skip(start as usize)
            .take(end.saturating_sub(start) as usize)
            .map(BridgeIpRow::to_ip_stats)
            .collect();
        return BusResponse::TopUploaders(result);
    }
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        let catalog = lqos_network_devices::network_devices_catalog();
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .map(|(k, te)| {
                let (circuit_id, _circuit_name) =
                    resolve_circuit_metadata_for_entry(&catalog, k, te);
                (
                    *k,
                    te.actual_bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    circuit_id,
                    down_up_retransmit_sample(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    full_list.sort_by_key(|row| std::cmp::Reverse(row.1.up));
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, circuit_id, tcp_retransmits)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: *tcp_retransmits,
            },
        )
        .collect();
    BusResponse::TopUploaders(result)
}

/// 3a cluster-1 increment 2: per-circuit (name, max-down-Mbps, max-up-Mbps)
/// from the catalog — the plan denominator for the bridge heatmap rows.
fn circuit_meta_by_id() -> FxHashMap<String, (String, f32, f32)> {
    let catalog = lqos_network_devices::network_devices_catalog();
    let mut meta: FxHashMap<String, (String, f32, f32)> = FxHashMap::default();
    for device in catalog.iter_all_devices() {
        let entry = meta
            .entry(device.circuit_id.clone())
            .or_insert_with(|| (device.circuit_name.clone(), 0.0, 0.0));
        entry.1 = entry.1.max(device.download_max_mbps);
        entry.2 = entry.2.max(device.upload_max_mbps);
    }
    meta
}

/// Retrieve per-circuit heatmap data for the executive summary.
pub fn circuit_heatmaps() -> BusResponse {
    let enabled = lqos_config::load_config()
        .map(|cfg| cfg.enable_circuit_heatmaps)
        .unwrap_or(true);
    if !enabled {
        return BusResponse::CircuitHeatmaps(Vec::new());
    }

    // 3a cluster-1 increment 2: on a bridge box the rows come from the
    // analytics accumulator (exact per-minute accounting; utilization unit).
    if let Some(snapshot) = crate::bridge_analytics::circuit_heatmap_snapshot() {
        let meta = circuit_meta_by_id();
        let qoo = crate::bridge_analytics::circuit_qoo_snapshot();
        let mut rows: Vec<CircuitHeatmapData> = snapshot
            .by_circuit
            .iter()
            .map(|(circuit_id, minutes)| {
                let (circuit_name, down_mbps, up_mbps) = meta
                    .get(circuit_id)
                    .cloned()
                    .unwrap_or_else(|| (circuit_id.clone(), 0.0, 0.0));
                CircuitHeatmapData {
                    circuit_hash: hash_to_i64(circuit_id),
                    circuit_id: circuit_id.clone(),
                    circuit_name,
                    blocks: crate::bridge_analytics::circuit_heatmap::blocks_from_minutes(
                        minutes, down_mbps, up_mbps,
                    ),
                    qoq_blocks: qoo
                        .as_ref()
                        .and_then(|qoo| qoo.by_circuit.get(circuit_id).cloned()),
                }
            })
            .collect();
        rows.sort_by(|a, b| a.circuit_id.cmp(&b.circuit_id));
        return BusResponse::CircuitHeatmaps(rows);
    }

    let catalog = lqos_network_devices::network_devices_catalog();
    let mut circuit_meta: FxHashMap<i64, (String, String)> = FxHashMap::default();
    catalog.iter_all_devices().for_each(|device| {
        circuit_meta
            .entry(device.circuit_hash)
            .or_insert_with(|| (device.circuit_id.clone(), device.circuit_name.clone()));
    });

    let heatmaps = THROUGHPUT_TRACKER.circuit_heatmaps.lock();
    let qoq_heatmaps = THROUGHPUT_TRACKER.circuit_qoq_heatmaps.lock();
    let mut rows: Vec<CircuitHeatmapData> = heatmaps
        .iter()
        .map(|(hash, heatmap)| {
            let (circuit_id, circuit_name) = circuit_meta
                .get(hash)
                .cloned()
                .unwrap_or_else(|| (String::new(), String::new()));
            CircuitHeatmapData {
                circuit_hash: *hash,
                circuit_id,
                circuit_name,
                blocks: heatmap.blocks(),
                qoq_blocks: qoq_heatmaps.get(hash).map(|heatmap| heatmap.blocks()),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.circuit_id.cmp(&b.circuit_id));
    BusResponse::CircuitHeatmaps(rows)
}

/// Retrieve per-site heatmap data for the executive summary.
pub fn site_heatmaps() -> BusResponse {
    let enabled = lqos_config::load_config()
        .map(|cfg| cfg.enable_site_heatmaps)
        .unwrap_or(true);
    if !enabled {
        return BusResponse::SiteHeatmaps(Vec::new());
    }

    // 3a cluster-1 increment 2: bridge boxes roll the sites up from the
    // per-circuit accumulator (circuit id == the tree node name, the same
    // join the tree live-view uses; self-inclusive parents chain).
    if let Some(snapshot) = crate::bridge_analytics::circuit_heatmap_snapshot() {
        return site_heatmaps_from_snapshot(&snapshot);
    }

    lqos_network_devices::with_network_json_read(|net_json| {
        let mut rows: Vec<SiteHeatmapData> = net_json
            .get_nodes_when_ready()
            .iter()
            .filter_map(|node| {
                if node.name == "Root" || node.name.parse::<std::net::IpAddr>().is_ok() {
                    return None;
                }
                node.heatmap.as_ref().map(|heatmap| SiteHeatmapData {
                    site_name: node.name.clone(),
                    node_type: node.node_type.clone(),
                    depth: node.parents.len().saturating_sub(1),
                    blocks: heatmap.blocks(),
                    // §`lqos-74t.47.1.4`: the site QoQ now reads the unified
                    // `circuit_qoo` snapshot (the old `node.qoq_heatmap` is retired).
                    qoq_blocks: crate::bridge_analytics::circuit_qoo_snapshot()
                        .and_then(|snapshot| snapshot.by_node.get(&node.name).cloned()),
                })
            })
            .collect();
        rows.sort_by(|a, b| a.site_name.cmp(&b.site_name));
        BusResponse::SiteHeatmaps(rows)
    })
}

/// §lqos-74t.40 pin 1 (whole-window): stamp the A1 pooled percentile onto EVERY
/// populated RTT block. The executive "Median RTT (ms)" reads `median_value`
/// over the whole 15-minute series, so overwriting only the newest block leaves
/// the older per-member `max_optional` blocks to swamp the median. A real
/// no-data (missed) minute — `rtt[index] == None` — stays `None` (never a
/// fabricated value). The pooled direction-agnostic source is folded into both
/// directions, so the combined `rtt` is their mean (matching the newest-block
/// rule). Pure and unit-gated (`pooled_site_rtt_covers_the_whole_window`).
fn apply_pooled_site_rtt(
    blocks: &mut lqos_utils::temporal_heatmap::HeatmapBlocks,
    p50_down: Option<f32>,
    p50_up: Option<f32>,
    p90_down: Option<f32>,
    p90_up: Option<f32>,
) {
    let pooled = match (p50_down, p50_up) {
        (None, up) => up,
        (down, None) => down,
        (Some(down), Some(up)) => Some((down + up) / 2.0),
    };
    for index in 0..blocks.rtt.len() {
        if blocks.rtt[index].is_none() {
            continue;
        }
        blocks.rtt_p50_down[index] = p50_down;
        blocks.rtt_p50_up[index] = p50_up;
        blocks.rtt_p90_down[index] = p90_down;
        blocks.rtt_p90_up[index] = p90_up;
        blocks.rtt[index] = pooled;
    }
}

#[cfg(test)]
mod site_rtt_window_tests {
    use super::apply_pooled_site_rtt;
    use crate::bridge_analytics::circuit_heatmap::{
        HeatmapMinute, RETENTION_MINUTES, blocks_from_minutes,
    };

    /// §lqos-74t.40 pin 1 whole-window (the bounded (a) correction): every
    /// POPULATED block is stamped with the A1 pooled percentile, so the
    /// executive's `median_value` over the series reads the pooled value and not
    /// an old per-member `max_optional`; a real no-data minute stays `None`.
    #[test]
    fn pooled_site_rtt_covers_the_whole_window() {
        let minutes = vec![
            HeatmapMinute {
                start_millis: 0,
                rtt_p50_down_ms: Some(500.0),
                rtt_p50_up_ms: Some(500.0),
                ..HeatmapMinute::default()
            },
            HeatmapMinute {
                start_millis: 60_000,
                rtt_p50_down_ms: Some(500.0),
                rtt_p50_up_ms: Some(500.0),
                ..HeatmapMinute::default()
            },
            // A real missed minute: no RTT sample (sampled_intervals == 0).
            HeatmapMinute {
                start_millis: 120_000,
                ..HeatmapMinute::default()
            },
        ];
        let mut blocks = blocks_from_minutes(&minutes, 100.0, 100.0);
        // The A1 pooled source reads 10 ms (p50) / 20 ms (p90).
        apply_pooled_site_rtt(&mut blocks, Some(10.0), Some(10.0), Some(20.0), Some(20.0));

        let newest = RETENTION_MINUTES - 1;
        // The two populated minutes are the last two blocks: 10 ms, not 500 ms.
        assert_eq!(blocks.rtt[newest - 1], Some(10.0));
        assert_eq!(blocks.rtt[newest - 2], Some(10.0));
        assert_eq!(blocks.rtt_p90_down[newest - 1], Some(20.0));
        // No old `max_optional` survivor anywhere in the populated series.
        assert!(
            blocks.rtt.iter().flatten().all(|value| *value < 100.0),
            "every populated block reads the pooled source, not 500 ms"
        );
        // The newest minute is a real no-data miss: honest None, not fabricated.
        assert_eq!(blocks.rtt[newest], None);
        assert_eq!(blocks.rtt_p50_down[newest], None);
    }
}

/// 3a cluster-1 increment 2: site heatmaps roll the member circuits up through
/// the tree (circuit id == node name, self-inclusive parent chain). The
/// denominator is the summed member plan; `qoq_blocks` is honest-empty until
/// the pinned QoO port lands.
fn site_heatmaps_from_snapshot(
    snapshot: &crate::bridge_analytics::circuit_heatmap::CircuitHeatmapSnapshot,
) -> BusResponse {
    let meta = circuit_meta_by_id();
    // 3b DEFECT 1: the shared circuit→parent-node attribution — the tree keys
    // nodes by NAME, not by circuit id.
    let parent_by_circuit = crate::bridge_analytics::circuit_parent_node_by_id();
    // §`lqos-74t.40` INSTRUMENT-FIRST (`docs/site_rollup_rtt_retrans_qoo_card.md`
    // §5): when `LIBREQOS_TRACE_SITE` names a site (name substring), log its
    // descendant circuit-id set, which members are attributed to the synthetic
    // `orphans` umbrella, and the per-minute `max` RTT winner — distinguishing
    // source-filter vs max-statistic vs mis-attribution. Diagnostic only.
    let trace_site = std::env::var("LIBREQOS_TRACE_SITE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        let node_circuits = crate::bridge_analytics::node_circuits_by_index(
            nodes,
            &parent_by_circuit,
            snapshot.all_circuits(),
        );
        if let Some(trace) = trace_site.as_deref() {
            for (idx, circuits) in &node_circuits {
                let Some(node) = nodes.get(*idx) else {
                    continue;
                };
                if !node.name.contains(trace) {
                    continue;
                }
                let orphan_children: Vec<&str> = circuits
                    .iter()
                    .copied()
                    .filter(|c| {
                        parent_by_circuit
                            .get(*c)
                            .map(|p| p.to_ascii_lowercase().contains("orphan"))
                            .unwrap_or(false)
                    })
                    .collect();
                // The per-minute `max` winner for the down RTT p50 (the exact
                // `max_optional` hazard) across the site's members.
                let mut winner: Option<(f32, &str, u64)> = None;
                for c in circuits.iter() {
                    if let Some(minutes) = snapshot.by_circuit.get(*c) {
                        for minute in minutes {
                            if let Some(value) = minute.rtt_p50_down_ms
                                && winner.map(|(w, _, _)| value > w).unwrap_or(true)
                            {
                                winner = Some((value, c, minute.start_millis));
                            }
                        }
                    }
                }
                warn!(
                    event = "SITE_ROLLUP_TRACE",
                    site = %node.name,
                    circuit_count = circuits.len(),
                    circuits = ?circuits,
                    orphan_children = ?orphan_children,
                    max_rtt_p50_down_ms = winner.map(|(v, _, _)| v).unwrap_or(0.0),
                    max_winner = winner.map(|(_, c, _)| c).unwrap_or(""),
                    "site rollup trace (descendants / orphans / max winner)"
                );
            }
            let unmatched: Vec<&str> = snapshot
                .all_circuits()
                .filter(|c| !parent_by_circuit.contains_key(*c))
                .collect();
            if !unmatched.is_empty() {
                warn!(
                    event = "SITE_ROLLUP_TRACE_UNMATCHED",
                    count = unmatched.len(),
                    circuits = ?unmatched,
                    "circuits with no parent-node attribution (dropped from site rollups)"
                );
            }
        }
        let mut rows: Vec<SiteHeatmapData> = node_circuits
            .into_iter()
            .filter_map(|(idx, circuits)| {
                let node = nodes.get(idx)?;
                if node.name == "Root" || node.name.parse::<IpAddr>().is_ok() {
                    return None;
                }
                // §lqos-74t.40 pin 4: the synthetic `orphans` umbrella is not a
                // real site — never render it as a site row. Its directly
                // attributed circuits are likewise a non-entity (see
                // `node_circuits_by_index`), so they never reach an ancestor.
                if node.id.as_deref() == Some(lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID) {
                    return None;
                }
                let mut down_mbps = 0.0_f32;
                let mut up_mbps = 0.0_f32;
                for circuit_id in &circuits {
                    if let Some((_, down, up)) = meta.get(*circuit_id) {
                        down_mbps += *down;
                        up_mbps += *up;
                    }
                }
                let minutes = snapshot.rollup(circuits.iter().copied());
                // Pooled-distribution QoO rollup (per node, keyed by name).
                let qoq_blocks = crate::bridge_analytics::circuit_qoo_snapshot()
                    .and_then(|qoo| qoo.by_node.get(&node.name).cloned());
                // §lqos-74t.40 pin 1: the site RTT = the A1 POOLED percentile over
                // the node's member samples (`node.rtt_buffer`, folded by
                // `apply_tree_rtt_accumulation`) — NOT the per-member `max`.
                let mut blocks = crate::bridge_analytics::circuit_heatmap::blocks_from_minutes(
                    &minutes, down_mbps, up_mbps,
                );
                // §lqos-74t.40 pin 1: the site RTT series = the A1 pooled
                // percentile over the WHOLE window (the executive "Median RTT (ms)"
                // reads `median_value` over all 15 blocks, so a newest-only rewrite
                // leaves the older per-member `max_optional` blocks to swamp the
                // median — the bounded (a) correction).
                let down_pct = |p: u8| {
                    node.rtt_buffer
                        .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, p)
                        .map(|d| d.as_millis() as f32)
                };
                let up_pct = |p: u8| {
                    node.rtt_buffer
                        .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, p)
                        .map(|d| d.as_millis() as f32)
                };
                apply_pooled_site_rtt(
                    &mut blocks,
                    down_pct(50),
                    up_pct(50),
                    down_pct(90),
                    up_pct(90),
                );
                Some(SiteHeatmapData {
                    site_name: node.name.clone(),
                    node_type: node.node_type.clone(),
                    depth: node.parents.len().saturating_sub(1),
                    blocks,
                    qoq_blocks,
                })
            })
            .collect();
        rows.sort_by(|a, b| a.site_name.cmp(&b.site_name));
        BusResponse::SiteHeatmaps(rows)
    })
}

/// Retrieve per-ASN heatmap data for the executive summary.
pub fn asn_heatmaps() -> BusResponse {
    let enabled = lqos_config::load_config()
        .map(|cfg| cfg.enable_asn_heatmaps)
        .unwrap_or(true);
    if !enabled {
        return BusResponse::AsnHeatmaps(Vec::new());
    }

    let rows: Vec<AsnHeatmapData> = snapshot_asn_heatmaps()
        .into_iter()
        .map(|(asn, blocks)| {
            let name = get_asn_name_by_id(asn);
            let asn_name = if name.eq_ignore_ascii_case("unknown") {
                None
            } else {
                Some(name)
            };
            AsnHeatmapData {
                asn,
                asn_name,
                blocks,
            }
        })
        .collect();
    BusResponse::AsnHeatmaps(rows)
}

/// Retrieve the global roll-up heatmap data for the executive summary.
pub fn global_heatmap() -> BusResponse {
    // 3a cluster-1 increment 2: the bridge global roll-up (Σ observed rows;
    // denominator = the total configured per-circuit capacity).
    if let Some(snapshot) = crate::bridge_analytics::circuit_heatmap_snapshot() {
        let meta = circuit_meta_by_id();
        let mut down_mbps = 0.0_f32;
        let mut up_mbps = 0.0_f32;
        for (_, down, up) in meta.values() {
            down_mbps += *down;
            up_mbps += *up;
        }
        let minutes = snapshot.rollup(snapshot.all_circuits());
        return BusResponse::GlobalHeatmap(
            crate::bridge_analytics::circuit_heatmap::blocks_from_minutes(
                &minutes, down_mbps, up_mbps,
            ),
        );
    }
    let heatmap = THROUGHPUT_TRACKER.global_heatmap.lock();
    BusResponse::GlobalHeatmap(heatmap.blocks())
}

pub fn worst_n(start: u32, end: u32) -> BusResponse {
    // K2 slice-3 3a: rank the bridge's per-IP RTT p50 (the kernel's median
    // latency analog); the RTT exclusions still apply via the catalog hash.
    if crate::bridge_analytics::is_active() {
        let mut rows: Vec<BridgeIpRow> = bridge_ip_rows()
            .into_iter()
            .filter(|row| row.rtt_ms.is_some())
            .filter(|row| {
                row.circuit_hash
                    .map(|hash| !crate::rtt_exclusions::is_excluded_hash(hash))
                    .unwrap_or(true)
            })
            .collect();
        rows.sort_by(|a, b| b.rtt_ms.unwrap_or(0.0).total_cmp(&a.rtt_ms.unwrap_or(0.0)));
        let result = rows
            .iter()
            .skip(start as usize)
            .take(end.saturating_sub(start) as usize)
            .map(BridgeIpRow::to_ip_stats)
            .collect();
        return BusResponse::WorstRtt(result);
    }
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        let catalog = lqos_network_devices::network_devices_catalog();
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(_k, d)| {
                d.circuit_hash
                    .map(|h| !crate::rtt_exclusions::is_excluded_hash(h))
                    .unwrap_or(true)
            })
            .filter(|(_k, te)| te.median_latency().is_some())
            .map(|(k, te)| {
                let (circuit_id, _circuit_name) =
                    resolve_circuit_metadata_for_entry(&catalog, k, te);
                (
                    *k,
                    te.actual_bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    circuit_id,
                    down_up_retransmit_sample(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    full_list.sort_by(|a, b| b.3.total_cmp(&a.3));
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, circuit_id, tcp_retransmits)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: *tcp_retransmits,
            },
        )
        .collect();
    BusResponse::WorstRtt(result)
}

pub fn worst_n_retransmits(start: u32, end: u32) -> BusResponse {
    // K2 slice-3 3a: rank the bridge's per-IP retransmit fraction (the same
    // comparator shape as the kernel; the wire is direction-agnostic, so the
    // sample's two directions carry the same value).
    if crate::bridge_analytics::is_active() {
        let mut rows: Vec<BridgeIpRow> = bridge_ip_rows()
            .into_iter()
            .filter(|row| row.rtt_ms.is_some())
            .collect();
        rows.sort_by(|a, b| {
            let total_a = a
                .retransmits
                .down
                .fraction()
                .map(|f| f.get())
                .unwrap_or(0.0)
                + a.retransmits.up.fraction().map(|f| f.get()).unwrap_or(0.0);
            let total_b = b
                .retransmits
                .down
                .fraction()
                .map(|f| f.get())
                .unwrap_or(0.0)
                + b.retransmits.up.fraction().map(|f| f.get()).unwrap_or(0.0);
            total_b.total_cmp(&total_a)
        });
        let result = rows
            .iter()
            .skip(start as usize)
            .take(end.saturating_sub(start) as usize)
            .map(BridgeIpRow::to_ip_stats)
            .collect();
        return BusResponse::WorstRetransmits(result);
    }
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        let catalog = lqos_network_devices::network_devices_catalog();
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(_k, te)| te.median_latency().is_some())
            .map(|(k, te)| {
                let (circuit_id, _circuit_name) =
                    resolve_circuit_metadata_for_entry(&catalog, k, te);
                (
                    *k,
                    te.actual_bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    circuit_id,
                    down_up_retransmit_sample(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    // Use a total order for floating-point comparison to avoid panics
    // when NaN/Inf are present and ensure comparator transitivity.
    full_list.sort_by(|a, b| {
        let total_a = a.6.down.fraction().map(|f| f.get()).unwrap_or(0.0)
            + a.6.up.fraction().map(|f| f.get()).unwrap_or(0.0);
        let total_b = b.6.down.fraction().map(|f| f.get()).unwrap_or(0.0)
            + b.6.up.fraction().map(|f| f.get()).unwrap_or(0.0);
        total_b.total_cmp(&total_a)
    });
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, circuit_id, tcp_retransmits)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: *tcp_retransmits,
            },
        )
        .collect();
    BusResponse::WorstRetransmits(result)
}

pub fn best_n(start: u32, end: u32) -> BusResponse {
    // K2 slice-3 3a: rank the bridge's per-IP RTT p50 ascending.
    if crate::bridge_analytics::is_active() {
        let mut rows: Vec<BridgeIpRow> = bridge_ip_rows()
            .into_iter()
            .filter(|row| row.rtt_ms.is_some())
            .filter(|row| {
                row.circuit_hash
                    .map(|hash| !crate::rtt_exclusions::is_excluded_hash(hash))
                    .unwrap_or(true)
            })
            .collect();
        rows.sort_by(|a, b| a.rtt_ms.unwrap_or(0.0).total_cmp(&b.rtt_ms.unwrap_or(0.0)));
        let result = rows
            .iter()
            .skip(start as usize)
            .take(end.saturating_sub(start) as usize)
            .map(BridgeIpRow::to_ip_stats)
            .collect();
        return BusResponse::BestRtt(result);
    }
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        let catalog = lqos_network_devices::network_devices_catalog();
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(_k, d)| {
                d.circuit_hash
                    .map(|h| !crate::rtt_exclusions::is_excluded_hash(h))
                    .unwrap_or(true)
            })
            .filter(|(_k, te)| te.median_latency().is_some())
            .map(|(k, te)| {
                let (circuit_id, _circuit_name) =
                    resolve_circuit_metadata_for_entry(&catalog, k, te);
                (
                    *k,
                    te.actual_bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    circuit_id,
                    down_up_retransmit_sample(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    full_list.sort_by(|a, b| b.3.total_cmp(&a.3));
    full_list.reverse();
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, circuit_id, tcp_retransmits)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: *tcp_retransmits,
            },
        )
        .collect();
    BusResponse::BestRtt(result)
}

pub fn xdp_pping_compat() -> BusResponse {
    let raw_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);
    let result = THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .filter(|(_k, d)| retire_check(raw_cycle, d.most_recent_cycle))
        .filter_map(|(_k, data)| {
            if data.tc_handle.as_u32() > 0 {
                let mut valid_samples: Vec<u32> = data
                    .recent_rtt_data
                    .iter()
                    .filter(|d| d.as_millis_times_100() > 0.0)
                    .map(|d| d.as_millis_times_100() as u32)
                    .collect();
                let samples = valid_samples.len() as u32;
                if samples > 0 {
                    valid_samples.sort_by(|a, b| (*a).cmp(b));
                    let median = valid_samples[valid_samples.len() / 2] as f32 / 100.0;
                    let min = if let Some(v) = valid_samples.first() {
                        *v as f32 / 100.0
                    } else {
                        // No valid min; skip this submission as if no samples
                        return None;
                    };
                    let max = if let Some(v) = valid_samples.last() {
                        *v as f32 / 100.0
                    } else {
                        // No valid max; skip this submission as if no samples
                        return None;
                    };
                    let sum = valid_samples.iter().sum::<u32>() as f32 / 100.0;
                    let avg = sum / samples as f32;

                    Some(XdpPpingResult {
                        tc: data.tc_handle.to_string(),
                        median,
                        avg,
                        max,
                        min,
                        samples,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();
    BusResponse::XdpPping(result)
}

#[allow(dead_code)] // K3-B carve-out: LTS2 submission payload helper; used by stats_submission.
pub struct MinMaxMedianRtt {
    pub min: f32,
    pub max: f32,
    pub median: f32,
}

#[allow(dead_code)] // K3-B carve-out: LTS2 submission payload helper; used by stats_submission.
pub fn min_max_median_rtt() -> Option<MinMaxMedianRtt> {
    // 3a LTS repoint: on a bridge box the RTT p50s ride the actor's by-IP rows.
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        let mut samples: Vec<f32> = snapshot
            .by_ip
            .values()
            .filter_map(|row| row.rtt_current_p50_nanos)
            .map(|nanos| nanos as f32 / 1_000_000.0)
            .filter(|millis| *millis > 0.0)
            .collect();
        if samples.is_empty() {
            return None;
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        return Some(MinMaxMedianRtt {
            min: samples[0],
            max: samples[samples.len() - 1],
            median: samples[samples.len() / 2],
        });
    }

    let reader_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);

    // Put all valid RTT samples into a big buffer
    let mut samples: Vec<f32> = Vec::new();

    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .filter(|(_k, d)| retire_check(reader_cycle, d.most_recent_cycle))
        .for_each(|(_k, d)| {
            samples.extend(
                d.recent_rtt_data
                    .iter()
                    .filter(|d| d.as_millis() > 0.0)
                    .map(|d| d.as_millis() as f32)
                    .collect::<Vec<f32>>(),
            );
        });

    if samples.is_empty() {
        return None;
    }

    // Sort the buffer
    samples.sort_by(|a, b| a.total_cmp(b));

    let result = MinMaxMedianRtt {
        min: samples[0],
        max: samples[samples.len() - 1],
        median: samples[samples.len() / 2],
    };

    Some(result)
}

#[derive(Debug, Serialize)]
pub struct TcpRetransmitTotal {
    pub up: i32,
    pub down: i32,
    pub tcp_up: u64,
    pub tcp_down: u64,
}

pub fn min_max_median_tcp_retransmits() -> TcpRetransmitTotal {
    // K2 slice-3 3a: sum the actor's per-IP retransmit/packet counters. The
    // bridge wire is direction-agnostic, so both land in the DOWN side (the
    // `flow_row_to_summary` convention); up stays zero.
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        let mut total = TcpRetransmitTotal {
            up: 0,
            down: 0,
            tcp_down: 0,
            tcp_up: 0,
        };
        for row in snapshot.by_ip.values() {
            total.down += row.tcp_retransmits as i32;
            total.tcp_down += row.tcp_packets;
        }
        return total;
    }
    let reader_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);

    let total_tcp = THROUGHPUT_TRACKER.tcp_packets_per_second();
    let mut total = TcpRetransmitTotal {
        up: 0,
        down: 0,
        tcp_down: total_tcp.down,
        tcp_up: total_tcp.up,
    };

    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .filter(|(_k, d)| retire_check(reader_cycle, d.most_recent_cycle))
        .for_each(|(_k, d)| {
            total.up += d.tcp_retransmits.up as i32;
            total.down += d.tcp_retransmits.down as i32;
        });

    total
}

pub fn rtt_histogram<const N: usize>() -> BusResponse {
    // K2 slice 1 source-dispatch: bridge mode buckets the per-IP RTT p50s
    // (median-per-host; the widget's 10 ms-wide columns).
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        let mut result = vec![0; N];
        for row in snapshot.by_ip.values() {
            let Some(nanos) = row.rtt_current_p50_nanos else {
                continue;
            };
            let median_ms = nanos as f64 / 1_000_000.0;
            let column = (median_ms / 10.0) as usize;
            result[usize::min(column, N - 1)] += 1;
        }
        return BusResponse::RttHistogram(result);
    }
    let mut result = vec![0; N];
    let reader_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);
    for (_k, data) in THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .filter(|(_k, d)| retire_check(reader_cycle, d.most_recent_cycle))
    {
        let valid_samples: Vec<f64> = data
            .recent_rtt_data
            .iter()
            .filter(|d| d.as_millis() > 0.0)
            .map(|d| d.as_millis())
            .collect();
        let samples = valid_samples.len() as u32;
        if samples > 0 {
            let median = valid_samples[valid_samples.len() / 2] as f32 / 10.0;
            let median = f32::min(N as f32 * 10.0, median);
            let column = median as usize;
            result[usize::min(column, N - 1)] += 1;
        }
    }

    BusResponse::RttHistogram(result)
}

pub fn host_counts() -> BusResponse {
    let (total, shaped) = current_host_counts();
    BusResponse::HostCounts((total, shaped))
}

fn current_host_counts() -> (u32, u32) {
    // K2 slice-3 3a: on a bridge box, count the actor's per-IP rows; "shaped"
    // is a row with shaped (dequeued) bytes this interval (there is no bridge
    // tc_handle).
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        let total = snapshot.by_ip.len() as u32;
        let shaped = snapshot
            .by_ip
            .values()
            .filter(|row| {
                row.shaped_bytes_per_second.down > 0 || row.shaped_bytes_per_second.up > 0
            })
            .count() as u32;
        return (total, shaped);
    }
    let mut total = 0;
    let mut shaped = 0;
    let tp_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);
    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
        .for_each(|(_k, d)| {
            total += 1;
            if d.tc_handle.as_u32() != 0 {
                shaped += 1;
            }
        });
    (total, shaped)
}

/// Gather headline metrics for the Executive Summary header cards.
pub fn executive_summary_header() -> BusResponse {
    let catalog = lqos_network_devices::network_devices_catalog();
    let mut circuits: FxHashSet<i64> = FxHashSet::default();
    let mut device_count = 0_u64;
    for device in catalog.iter_all_devices() {
        device_count = device_count.saturating_add(1);
        circuits.insert(device.circuit_hash);
    }
    let circuit_count = circuits.len() as u64;

    let site_count = lqos_network_devices::with_network_json_read(|net_json| {
        let total_nodes = net_json.get_nodes_when_ready().len();
        // Remove the synthetic root node when counting sites.
        total_nodes.saturating_sub(1) as u64
    });

    let (total_hosts, shaped_hosts) = current_host_counts();
    let mapped_ip_count = shaped_hosts as u64;
    let unmapped_ip_count = total_hosts.saturating_sub(shaped_hosts) as u64;

    let queue_counts = ALL_QUEUE_SUMMARY.queue_counts();
    let queue_stats_stale = queue_stats_stale();
    let insight_connected = crate::lts2_sys::current_capabilities().control_service_reachable;

    BusResponse::ExecutiveSummaryHeader(ExecutiveSummaryHeader {
        circuit_count,
        device_count,
        site_count,
        mapped_ip_count,
        unmapped_ip_count,
        htb_queue_count: queue_counts.htb as u64,
        cake_queue_count: queue_counts.cake as u64,
        fq_codel_queue_count: queue_counts.fq_codel as u64,
        queue_stats_stale,
        insight_connected,
    })
}

type FullList = (
    XdpIpAddress,
    DownUpOrder<u64>,
    DownUpOrder<u64>,
    f32,
    TcHandle,
    u64,
);

pub fn all_unknown_ips() -> BusResponse {
    // K2 slice-3 3a: bridge mode lists the actor's per-IP rows that no catalog
    // device resolves (the unconfirmed/unmapped class). The per-IP rows only
    // exist while the actor observed them, so the kernel's 5-minute window is
    // inherent.
    if crate::bridge_analytics::is_active() {
        let catalog = lqos_network_devices::network_devices_catalog();
        let mut result: Vec<IpStats> = bridge_best_effort_ip_rows()
            .into_iter()
            .filter(|row| {
                catalog
                    .device_longest_match_for_ip(&XdpIpAddress::from_ip(row.ip))
                    .is_none()
            })
            .map(|row| row.to_ip_stats())
            .collect();
        result.sort_by_key(|row| std::cmp::Reverse(row.bits_per_second.down));
        return BusResponse::AllUnknownIps(result);
    }
    let boot_time = time_since_boot();
    if boot_time.is_err() {
        warn!("The Linux system clock isn't available to provide time since boot, yet.");
        warn!("This only happens immediately after a reboot.");
        return BusResponse::NotReadyYet;
    }
    let Ok(boot_time) = boot_time else {
        return BusResponse::Fail("Boot time unavailable".to_string());
    };

    // Safely convert TimeSpec to Duration - handle potential negative values
    let time_since_boot = match boot_time.tv_sec() {
        sec if sec < 0 => {
            warn!(
                "Negative boot time detected: {:?}. Using 0 duration.",
                boot_time
            );
            Duration::from_secs(0)
        }
        sec => Duration::from_secs(sec as u64) + Duration::from_nanos(boot_time.tv_nsec() as u64),
    };

    let five_minutes_ago = time_since_boot.saturating_sub(Duration::from_secs(300));
    let five_minutes_ago_nanoseconds = five_minutes_ago.as_nanos();

    let mut full_list: Vec<FullList> = {
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| d.tc_handle.as_u32() == 0)
            .filter(|(_k, d)| d.last_seen as u128 > five_minutes_ago_nanoseconds)
            .map(|(k, te)| {
                (
                    *k,
                    te.bytes,
                    te.packets,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.most_recent_cycle,
                )
            })
            .collect()
    };
    full_list.sort_by_key(|row| std::cmp::Reverse(row.5));
    let result = full_list
        .iter()
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, _last_seen)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: String::new(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: DownUpOrder::new(
                    TcpRetransmitSample::new(0, 0),
                    TcpRetransmitSample::new(0, 0),
                ),
            },
        )
        .collect();
    BusResponse::AllUnknownIps(result)
}

/// `.65`: the per-flow DELIVERED (shaped) bytes/second for an active bridge
/// flow. Absent on the flow's first interval or after a counter reset (no
/// rate), in which case the caller supplies zero.
fn bridge_flow_rate_bytes_per_second(
    snapshot: &crate::bridge_analytics::BridgeStatsSnapshot,
    flow: &analytics_ipc::FlowRow,
) -> DownUpOrder<u64> {
    snapshot
        .flow_bytes_per_second_directional
        .get(&flow.id)
        .copied()
        .unwrap_or_else(DownUpOrder::zeroed)
}

/// K2 slice 1: map a bridge `FlowRow` to the lqtop summary shape. Bytes come
/// from the wire's DELIVERED (shaped) counters; packets are not on the wire
/// (zero). `.65`: `rate_estimate_bps` (bits/s) is derived from the
/// caller-supplied per-flow DELIVERED bytes/second (×8). ASN comes from the
/// v3-side GeoTable join and the circuit from the catalog.
fn flow_row_to_summary(
    flow: &analytics_ipc::FlowRow,
    rate_bytes_per_second: DownUpOrder<u64>,
) -> lqos_bus::FlowbeeSummaryData {
    let remote_ip = flow.id.remote_ip;
    let subscriber_ip = flow.id.subscriber_ip;
    let (circuit_id, circuit_name) = lqos_network_devices::network_devices_catalog()
        .device_longest_match_for_ip(&XdpIpAddress::from_ip(subscriber_ip))
        .map(|(_, device)| (device.circuit_id.clone(), device.circuit_name.clone()))
        .unwrap_or_default();
    let asn = get_asn_name_and_country(remote_ip);
    let mut rtt_samples = flow.rtt_samples_us.clone();
    rtt_samples.sort_unstable();
    let rtt_nanos = rtt_samples
        .get(rtt_samples.len() / 2)
        .map(|us| u64::from(*us) * 1_000)
        .unwrap_or(0);
    let end_status = match flow.state {
        analytics_ipc::FlowState::Active | analytics_ipc::FlowState::Idle => 0,
        analytics_ipc::FlowState::Fin => 1,
        analytics_ipc::FlowState::Reset => 2,
    };
    lqos_bus::FlowbeeSummaryData {
        remote_ip: remote_ip.to_string(),
        local_ip: subscriber_ip.to_string(),
        src_port: flow.id.subscriber_port,
        dst_port: flow.id.remote_port,
        ip_protocol: flow.id.protocol.into(),
        start_time: flow.first_seen_millis,
        last_seen: flow.last_seen_millis,
 // 3b: `bytes_sent` reports the DELIVERED (shaped) per-flow
        // series — naming parity (sent != enqueued); `actual` is the demand.
        bytes_sent: DownUpOrder {
            down: flow.shaped.download_bytes,
            up: flow.shaped.upload_bytes,
        },
        packets_sent: DownUpOrder::zeroed(),
        // `.65`: the per-flow DELIVERED bytes/second → bits/s (saturating to the
        // wire field's u32 width).
        rate_estimate_bps: DownUpOrder {
            down: (rate_bytes_per_second.down.saturating_mul(8)).min(u64::from(u32::MAX)) as u32,
            up: (rate_bytes_per_second.up.saturating_mul(8)).min(u64::from(u32::MAX)) as u32,
        },
        tcp_retransmits: DownUpOrder {
            down: flow.tcp_retransmits.min(u64::from(u16::MAX)) as u16,
            up: 0,
        },
        end_status,
        tos: 0,
        flags: 0,
        rtt_nanos: DownUpOrder {
            down: rtt_nanos,
            up: 0,
        },
        remote_asn: lookup_asn_id(remote_ip).unwrap_or(0),
        remote_asn_name: asn.name,
        remote_asn_country: asn.country,
        analysis: String::new(),
        circuit_id,
        circuit_name,
    }
}

fn flow_summary_from_snapshot(flow: &ActiveFlowSnapshot) -> lqos_bus::FlowbeeSummaryData {
    lqos_bus::FlowbeeSummaryData {
        remote_ip: flow.display.remote_ip.clone(),
        local_ip: flow.display.local_ip.clone(),
        src_port: flow.display.src_port,
        dst_port: flow.display.dst_port,
        ip_protocol: flow.display.ip_protocol.clone(),
        bytes_sent: flow.bytes_sent,
        packets_sent: flow.packets_sent,
        rate_estimate_bps: flow.rate_estimate_bps,
        tcp_retransmits: flow.tcp_retransmits,
        end_status: flow.end_status,
        tos: flow.tos,
        flags: flow.flags,
        remote_asn: flow.display.remote_asn,
        remote_asn_name: flow.display.remote_asn_name.clone(),
        remote_asn_country: flow.display.remote_asn_country.clone(),
        analysis: flow.display.analysis.clone(),
        last_seen: flow.last_seen,
        start_time: flow.start_time,
        rtt_nanos: flow.rtt_nanos,
        circuit_id: flow.circuit_id.clone(),
        circuit_name: flow.circuit_name.clone(),
    }
}

/// Returns all rows from the latest active-flow snapshot.
pub fn dump_active_flows() -> BusResponse {
    // `.65` source-dispatch: bridge mode serves the wire's active flow rows
    // with their DELIVERED (shaped) per-flow rate (bytes/s → bits/s), so the
    // Shaper Top ASN widgets (which aggregate `rate_estimate_bps`) are
    // non-empty and non-zero. The kernel snapshot is retired in bridge mode.
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        let result: Vec<lqos_bus::FlowbeeSummaryData> = snapshot
            .active_flows
            .iter()
            .map(|flow| {
                flow_row_to_summary(flow, bridge_flow_rate_bytes_per_second(&snapshot, flow))
            })
            .collect();
        return BusResponse::AllActiveFlows(result);
    }
    let snapshot = active_flow_snapshot();
    let result: Vec<lqos_bus::FlowbeeSummaryData> =
        snapshot.iter().map(flow_summary_from_snapshot).collect();

    BusResponse::AllActiveFlows(result)
}

/// Count active flows
pub fn count_active_flows() -> BusResponse {
    // K2 slice 1 source-dispatch: bridge mode counts the subscriber IPs
    // with at least one active flow this poll (already stored).
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        return BusResponse::CountActiveFlows(snapshot.active_flow_ips.len() as u64);
    }
    BusResponse::CountActiveFlows(live_active_flow_count())
}

fn compare_top_flow(
    a: &ActiveFlowSnapshot,
    b: &ActiveFlowSnapshot,
    flow_type: &TopFlowType,
) -> Ordering {
    match flow_type {
        TopFlowType::RateEstimate => b.rate_estimate_bps.sum().cmp(&a.rate_estimate_bps.sum()),
        TopFlowType::Bytes => b.bytes_sent.sum().cmp(&a.bytes_sent.sum()),
        TopFlowType::Packets => b.packets_sent.sum().cmp(&a.packets_sent.sum()),
        TopFlowType::Drops => b.tcp_retransmits.sum().cmp(&a.tcp_retransmits.sum()),
        TopFlowType::RoundTripTime => a.rtt_nanos.down.cmp(&b.rtt_nanos.down),
    }
}

/// Top Flows Report
pub fn top_flows(n: u32, flow_type: TopFlowType) -> BusResponse {
    // K2 slice 1 source-dispatch: bridge mode ranks the wire's active flow
    // rows (bytes from actual/shaped; packets/rate are not on the wire).
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        let mut table: Vec<&analytics_ipc::FlowRow> = snapshot.active_flows.iter().collect();
        let limit = n as usize;
        let key = |flow: &analytics_ipc::FlowRow| -> u64 {
            match flow_type {
                // `.65`: rank the rate view by the DELIVERED (shaped) rate — the
                // same quantity the summary now reports.
                TopFlowType::RateEstimate => {
                    let rate = bridge_flow_rate_bytes_per_second(&snapshot, flow);
                    rate.down.saturating_add(rate.up)
                }
                TopFlowType::Bytes => flow
                    .actual
                    .download_bytes
                    .saturating_add(flow.actual.upload_bytes),
                TopFlowType::Packets => flow.tcp_sent_packets,
                TopFlowType::Drops => flow.tcp_retransmits,
                TopFlowType::RoundTripTime => flow
                    .rtt_samples_us
                    .iter()
                    .copied()
                    .max()
                    .map(u64::from)
                    .unwrap_or(0),
            }
        };
        if limit == 0 {
            table.clear();
        } else {
            table.sort_by_key(|flow| std::cmp::Reverse(key(flow)));
            table.truncate(limit);
        }
        let result = table
            .iter()
            .map(|flow| {
                flow_row_to_summary(flow, bridge_flow_rate_bytes_per_second(&snapshot, flow))
            })
            .collect();
        return BusResponse::TopFlows(result);
    }
    let snapshot = active_flow_snapshot();
    let mut table: Vec<&ActiveFlowSnapshot> = snapshot.iter().collect();
    let limit = n as usize;

    if limit == 0 {
        table.clear();
    } else if limit < table.len() {
        table.select_nth_unstable_by(limit, |a, b| compare_top_flow(a, b, &flow_type));
        table.truncate(limit);
    }
    table.sort_by(|a, b| compare_top_flow(a, b, &flow_type));

    let result = table
        .iter()
        .map(|flow| flow_summary_from_snapshot(flow))
        .collect();

    BusResponse::TopFlows(result)
}

/// Flows by IP
pub fn flows_by_ip(ip: &str) -> BusResponse {
    if let Ok(ip) = ip.parse::<IpAddr>() {
        // `.65` source-dispatch: bridge mode filters the wire's active flow
        // rows by subscriber IP, with the DELIVERED per-flow rate.
        if crate::bridge_analytics::is_active() {
            let snapshot = crate::bridge_analytics::snapshot();
            let matching_flows: Vec<lqos_bus::FlowbeeSummaryData> = snapshot
                .active_flows
                .iter()
                .filter(|flow| flow.id.subscriber_ip == ip)
                .map(|flow| {
                    flow_row_to_summary(flow, bridge_flow_rate_bytes_per_second(&snapshot, flow))
                })
                .collect();
            return BusResponse::FlowsByIp(matching_flows);
        }
        let ip = XdpIpAddress::from_ip(ip);
        let snapshot = active_flow_snapshot();
        let matching_flows: Vec<_> = snapshot
            .iter()
            .filter(|flow| flow.key.local_ip == ip)
            .map(flow_summary_from_snapshot)
            .collect();

        return BusResponse::FlowsByIp(matching_flows);
    }
    BusResponse::Ack
}

/// Current endpoints by country
pub fn current_endpoints_by_country() -> BusResponse {
    let snapshot = crate::bridge_analytics::flow_history_snapshot().unwrap_or_default();
    BusResponse::CurrentEndpointsByCountry(snapshot.country_summary())
}

/// Current endpoint lat/lon
pub fn current_lat_lon() -> BusResponse {
    let snapshot = crate::bridge_analytics::flow_history_snapshot().unwrap_or_default();
    BusResponse::CurrentLatLon(snapshot.lat_lon_endpoints())
}

/// Ether Protocol Summary
pub fn ether_protocol_summary() -> BusResponse {
    let snapshot = crate::bridge_analytics::flow_history_snapshot().unwrap_or_default();
    crate::node_manager::local_api::flow_wire::ether_protocol_summary(&snapshot)
}

/// IP Protocol Summary
pub fn ip_protocol_summary() -> BusResponse {
    let snapshot = crate::bridge_analytics::flow_history_snapshot().unwrap_or_default();
    BusResponse::IpProtocols(snapshot.ip_protocol_summary())
}

/// Flow duration summary
pub fn flow_duration() -> BusResponse {
    let snapshot = crate::bridge_analytics::flow_history_snapshot().unwrap_or_default();
    BusResponse::FlowDuration(
        crate::node_manager::local_api::flow_wire::flow_duration_summary(&snapshot),
    )
}

#[allow(dead_code)] // K3-B carve-out: LTS2 submission payload helper; used by stats_submission.
type RawNetJs = std::collections::HashMap<String, RawNetJsBody>;

#[allow(dead_code)] // K3-B carve-out: LTS2 submission payload helper; used by stats_submission.
#[derive(Deserialize, Debug)]
struct RawNetJsBody {
    #[serde(rename = "downloadBandwidthMbps")]
    download_bandwidth_mbps: u32,
    #[serde(rename = "uploadBandwidthMbps")]
    upload_bandwidth_mbps: u32,
    #[serde(default)]
    latitude: Option<f32>,
    #[serde(default)]
    longitude: Option<f32>,
    #[serde(rename = "type")]
    site_type: Option<String>,
    children: Option<RawNetJs>,
}

#[allow(dead_code)] // K3-B carve-out: LTS2 submission payload helper; used by stats_submission.
#[derive(Serialize, Debug)]
struct Lts2NetJs {
    name: String,
    site_hash: i64,
    site_type: Option<String>,
    download_bandwidth_mbps: u32,
    upload_bandwidth_mbps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    latitude: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    longitude: Option<f32>,
    children: Vec<Lts2NetJs>,
}

#[allow(dead_code)] // K3-B carve-out: LTS2 submission payload helper; used by stats_submission.
impl RawNetJsBody {
    fn to_lts2(&self, name: &str) -> Lts2NetJs {
        let mut result = Lts2NetJs {
            name: name.to_string(),
            site_hash: hash_to_i64(name),
            site_type: self.site_type.clone(),
            download_bandwidth_mbps: self.download_bandwidth_mbps,
            upload_bandwidth_mbps: self.upload_bandwidth_mbps,
            latitude: self.latitude,
            longitude: self.longitude,
            children: vec![],
        };

        if let Some(children) = &self.children {
            for (name, body) in children.iter() {
                result.children.push(body.to_lts2(name));
            }
        }

        result
    }
}

#[allow(dead_code)] // K3-B carve-out: LTS2 submission payload helper; used by stats_submission.
#[repr(C)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lts2Circuit {
    pub circuit_id: String,
    pub circuit_name: String,
    pub circuit_hash: i64,
    pub download_min_mbps: u32,
    pub upload_min_mbps: u32,
    pub download_max_mbps: u32,
    pub upload_max_mbps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_min_mbps_exact: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_min_mbps_exact: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_max_mbps_exact: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_max_mbps_exact: Option<f32>,
    pub parent_node: i64,
    pub parent_node_name: Option<String>,
    pub devices: Vec<Lts2Device>,
}

#[allow(dead_code)] // K3-B carve-out: LTS2 submission payload helper; used by stats_submission.
#[repr(C)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lts2Device {
    pub device_id: String,
    pub device_name: String,
    pub device_hash: i64,
    pub mac: String,
    pub ipv4: Vec<([u8; 4], u8)>,
    pub ipv6: Vec<([u8; 16], u8)>,
    pub comment: String,
}

#[cfg(test)]
mod compatibility_tests {
    use super::{
        CIRCUIT_RTT_BUFFERS, Lts2Circuit, RawNetJsBody, circuit_current_qoo,
        circuit_current_rtt_p50_nanos, resolve_circuit_metadata_for_entry, resolve_flow_device,
    };
    use crate::test_support::ActiveFlowSnapshotTestContext;
    use crate::throughput_tracker::flow_data::{
        FlowAnalysis, FlowbeeEffectiveDirection, FlowbeeLocalData, RttData, active_flow_snapshot,
        active_flow_test_lock, mutate_all_flows, refresh_active_flow_snapshot,
        replace_active_flows_live_for_test,
    };
    use crate::throughput_tracker::throughput_entry::ThroughputEntry;
    use fxhash::FxHashMap;
    use lqos_bus::{BusResponse, TcHandle, TopFlowType};
    use lqos_config::{ConfigShapedDevices, ShapedDevice};
    use lqos_network_devices::{NetworkDevicesCatalog, ShapedDevicesCatalog};
    use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
    use lqos_utils::qoo::QoqScores;
    use lqos_utils::qoq_heatmap::TemporalQoqHeatmap;
    use lqos_utils::rtt::RttBuffer;
    use lqos_utils::units::DownUpOrder;
    use lqos_utils::{XdpIpAddress, hash_to_i64};
    use serde::{Deserialize, Serialize};
    use serde_json::to_value;
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    #[allow(dead_code)]
    #[derive(Debug, Deserialize, Serialize)]
    struct OldLts2Device {
        device_id: String,
        device_name: String,
        device_hash: i64,
        mac: String,
        ipv4: Vec<([u8; 4], u8)>,
        ipv6: Vec<([u8; 16], u8)>,
        comment: String,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize, Serialize)]
    struct OldLts2Circuit {
        circuit_id: String,
        circuit_name: String,
        circuit_hash: i64,
        download_min_mbps: u32,
        upload_min_mbps: u32,
        download_max_mbps: u32,
        upload_max_mbps: u32,
        parent_node: i64,
        parent_node_name: Option<String>,
        devices: Vec<OldLts2Device>,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct OldLts2NetJs {
        name: String,
        site_hash: i64,
        site_type: Option<String>,
        download_bandwidth_mbps: u32,
        upload_bandwidth_mbps: u32,
        children: Vec<OldLts2NetJs>,
    }

    #[test]
    fn old_receivers_ignore_exact_rate_fields() {
        let current = Lts2Circuit {
            circuit_id: "cid".to_string(),
            circuit_name: "Circuit".to_string(),
            circuit_hash: 7,
            download_min_mbps: 3,
            upload_min_mbps: 2,
            download_max_mbps: 7,
            upload_max_mbps: 4,
            download_min_mbps_exact: Some(2.5),
            upload_min_mbps_exact: Some(1.5),
            download_max_mbps_exact: Some(6.6),
            upload_max_mbps_exact: Some(3.3),
            parent_node: 9,
            parent_node_name: Some("Parent".to_string()),
            devices: Vec::new(),
        };

        let bytes = serde_cbor::to_vec(&current).expect("current payload serializes");
        let decoded: OldLts2Circuit =
            serde_cbor::from_slice(&bytes).expect("legacy shape ignores extra fields");

        assert_eq!(decoded.download_max_mbps, 7);
        assert_eq!(decoded.upload_max_mbps, 4);
    }

    #[test]
    fn current_lts2_circuit_round_trips_exact_rate_fields() {
        let current = Lts2Circuit {
            circuit_id: "cid".to_string(),
            circuit_name: "Circuit".to_string(),
            circuit_hash: 7,
            download_min_mbps: 3,
            upload_min_mbps: 2,
            download_max_mbps: 7,
            upload_max_mbps: 4,
            download_min_mbps_exact: Some(2.5),
            upload_min_mbps_exact: Some(1.5),
            download_max_mbps_exact: Some(6.6),
            upload_max_mbps_exact: Some(3.3),
            parent_node: 9,
            parent_node_name: Some("Parent".to_string()),
            devices: Vec::new(),
        };

        let bytes = serde_cbor::to_vec(&current).expect("current payload serializes");
        let decoded: Lts2Circuit =
            serde_cbor::from_slice(&bytes).expect("current payload round trips");

        assert_eq!(decoded.download_min_mbps_exact, Some(2.5));
        assert_eq!(decoded.upload_min_mbps_exact, Some(1.5));
        assert_eq!(decoded.download_max_mbps_exact, Some(6.6));
        assert_eq!(decoded.upload_max_mbps_exact, Some(3.3));
    }

    #[test]
    fn current_receivers_default_missing_exact_rate_fields() {
        let old = OldLts2Circuit {
            circuit_id: "cid".to_string(),
            circuit_name: "Circuit".to_string(),
            circuit_hash: 7,
            download_min_mbps: 3,
            upload_min_mbps: 2,
            download_max_mbps: 7,
            upload_max_mbps: 4,
            parent_node: 9,
            parent_node_name: Some("Parent".to_string()),
            devices: Vec::new(),
        };

        let bytes = serde_cbor::to_vec(&old).expect("legacy payload serializes");
        let decoded: Lts2Circuit =
            serde_cbor::from_slice(&bytes).expect("current shape accepts missing exact fields");

        assert_eq!(decoded.download_max_mbps, 7);
        assert_eq!(decoded.upload_max_mbps, 4);
        assert_eq!(decoded.download_min_mbps_exact, None);
        assert_eq!(decoded.upload_min_mbps_exact, None);
        assert_eq!(decoded.download_max_mbps_exact, None);
        assert_eq!(decoded.upload_max_mbps_exact, None);
    }

    #[test]
    fn post_write_snapshot_refresh_publishes_live_writes() {
        let _guard = active_flow_test_lock();
        let mut key = FlowbeeKey::default();
        key.ip_protocol = 6;
        key.src_port = 443;
        key.dst_port = 54_322;

        let mut raw = FlowbeeData::default();
        raw.start_time = 10;
        raw.last_seen = 20;
        raw.bytes_sent = DownUpOrder::new(3_000, 4_000);
        raw.packets_sent = DownUpOrder::new(30, 40);
        raw.rate_estimate_bps = DownUpOrder::new(50_000, 60_000);

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
        assert!(active_flow_snapshot().is_empty());

        mutate_all_flows(|flows| {
            flows.insert(
                key,
                (
                    FlowbeeLocalData::from_flow(&raw, &key),
                    FlowAnalysis::new(&key),
                ),
            );
        });
        assert!(active_flow_snapshot().is_empty());

        refresh_active_flow_snapshot();

        let snapshot = active_flow_snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].key, key);

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
    }

    /// K2 slice 1: the bridge `FlowRow` → lqtop summary field map (bytes,
    /// ports, protocol, RTT p50, end status). ASN/circuit joins depend on the
    /// process-global tables, so they are exercised by the rig confirm.
    #[test]
    fn flow_row_to_summary_maps_the_wire_fields() {
        let flow = analytics_ipc::FlowRow {
            id: analytics_ipc::FlowId {
                circuit_id: 1,
                flow_hash: 1,
                subscriber_ip: "192.0.2.10".parse().expect("test IP should parse"),
                remote_ip: "203.0.113.9".parse().expect("test IP should parse"),
                subscriber_port: 1234,
                remote_port: 443,
                protocol: 6,
            },
 // 3b sweep-fixture guard: demand != delivered.
            actual: analytics_ipc::DirectionalCounters {
                download_bytes: 999,
                upload_bytes: 999,
            },
            shaped: analytics_ipc::DirectionalCounters {
                download_bytes: 100,
                upload_bytes: 20,
            },
            rtt_samples_us: vec![10_000, 20_000, 30_000],
            tcp_sent_packets: 5,
            tcp_retransmits: 1,
            first_seen_millis: 1,
            last_seen_millis: 2,
            state: analytics_ipc::FlowState::Fin,
            ..analytics_ipc::FlowRow::default()
        };
        // `.65`: pass a DELIVERED bytes/second rate; the summary reports bits/s.
        let summary = super::flow_row_to_summary(&flow, DownUpOrder::new(1_000, 250));
        assert_eq!(summary.remote_ip, "203.0.113.9");
        assert_eq!(summary.local_ip, "192.0.2.10");
        assert_eq!(summary.src_port, 1234);
        assert_eq!(summary.dst_port, 443);
        assert_eq!(summary.bytes_sent.down, 100);
        assert_eq!(summary.bytes_sent.up, 20);
        assert_eq!(
            summary.rate_estimate_bps,
            DownUpOrder::new(8_000, 2_000),
            "the per-flow DELIVERED bytes/s is reported as bits/s (×8)"
        );
        assert_eq!(
            summary.rtt_nanos.down, 20_000_000,
            "median of the µs samples in ns"
        );
        assert_eq!(summary.end_status, 1, "FIN");
        assert!(matches!(
            summary.ip_protocol,
            lqos_bus::FlowbeeProtocol::TCP
        ));
    }

    /// `.65` Overview widgets: the per-flow DELIVERED rate reader keys off the
    /// DIRECTIONAL map (`flow_bytes_per_second_directional`) and returns BOTH
    /// slots; an absent flow reads zeroed. This is the widget-side contract the
    /// circuit-page U=0 fix must not disturb (pins the map stays wired).
    #[test]
    fn bridge_flow_rate_reads_the_directional_map() {
        let flow = analytics_ipc::FlowRow {
            id: analytics_ipc::FlowId {
                circuit_id: 1,
                flow_hash: 42,
                subscriber_ip: "192.0.2.10".parse().expect("test IP should parse"),
                remote_ip: "203.0.113.9".parse().expect("test IP should parse"),
                subscriber_port: 1234,
                remote_port: 443,
                protocol: 6,
            },
            ..analytics_ipc::FlowRow::default()
        };
        let mut snapshot = crate::bridge_analytics::BridgeStatsSnapshot::default();
        assert_eq!(
            super::bridge_flow_rate_bytes_per_second(&snapshot, &flow),
            DownUpOrder::zeroed(),
            "absent flow => zeroed"
        );
        snapshot
            .flow_bytes_per_second_directional
            .insert(flow.id.clone(), DownUpOrder::new(1_000, 250));
        assert_eq!(
            super::bridge_flow_rate_bytes_per_second(&snapshot, &flow),
            DownUpOrder::new(1_000, 250),
            "both directions come from the directional map"
        );
    }

    #[test]
    fn active_flow_bus_readers_use_published_snapshot_fields() {
        let _guard = active_flow_test_lock();
        let local_ip_1 = XdpIpAddress::from_ip("192.0.2.10".parse().expect("test IP should parse"));
        let local_ip_2 = XdpIpAddress::from_ip("192.0.2.11".parse().expect("test IP should parse"));
        let remote_ip_1 =
            XdpIpAddress::from_ip("198.51.100.20".parse().expect("test IP should parse"));
        let remote_ip_2 =
            XdpIpAddress::from_ip("198.51.100.21".parse().expect("test IP should parse"));

        let mut key_1 = FlowbeeKey::default();
        key_1.local_ip = local_ip_1;
        key_1.remote_ip = remote_ip_1;
        key_1.ip_protocol = 6;
        key_1.src_port = 443;
        key_1.dst_port = 50_000;

        let mut key_2 = key_1;
        key_2.local_ip = local_ip_2;
        key_2.remote_ip = remote_ip_2;
        key_2.dst_port = 50_001;

        let mut raw_1 = FlowbeeData::default();
        raw_1.start_time = 10_000_000_000;
        raw_1.last_seen = 20_000_000_000;
        raw_1.bytes_sent = DownUpOrder::new(1_000, 2_000);
        raw_1.packets_sent = DownUpOrder::new(10, 20);
        raw_1.rate_estimate_bps = DownUpOrder::new(30_000, 40_000);
        raw_1.tcp_retransmits = DownUpOrder::new(1, 2);
        raw_1.tos = 4;
        raw_1.flags = 0x12;
        raw_1.circuit_hash = 9_001;
        raw_1.device_hash = 9_002;

        let mut raw_2 = raw_1.clone();
        raw_2.bytes_sent = DownUpOrder::new(10_000, 20_000);
        raw_2.packets_sent = DownUpOrder::new(100, 200);
        raw_2.rate_estimate_bps = DownUpOrder::new(300_000, 400_000);
        raw_2.tcp_retransmits = DownUpOrder::new(9, 9);

        mutate_all_flows(|flows| {
            flows.clear();
            let mut local_1 = FlowbeeLocalData::from_flow(&raw_1, &key_1);
            let mut rtt_1 = RttBuffer::new(
                RttData::from_nanos(1_000_000),
                FlowbeeEffectiveDirection::Download,
                raw_1.last_seen,
            );
            rtt_1.push(
                RttData::from_nanos(1_000_000),
                FlowbeeEffectiveDirection::Download,
                raw_1.last_seen,
            );
            local_1.set_rtt_buffer(rtt_1);
            let mut local_2 = FlowbeeLocalData::from_flow(&raw_2, &key_2);
            let mut rtt_2 = RttBuffer::new(
                RttData::from_nanos(100_000_000),
                FlowbeeEffectiveDirection::Download,
                raw_2.last_seen,
            );
            rtt_2.push(
                RttData::from_nanos(100_000_000),
                FlowbeeEffectiveDirection::Download,
                raw_2.last_seen,
            );
            local_2.set_rtt_buffer(rtt_2);
            flows.insert(key_1, (local_1, FlowAnalysis::new(&key_1)));
            flows.insert(key_2, (local_2, FlowAnalysis::new(&key_2)));
        });
        let BusResponse::CountActiveFlows(count) = super::count_active_flows() else {
            panic!("active flow count response should match request");
        };
        assert_eq!(count, 2);
        refresh_active_flow_snapshot();

        let BusResponse::CountActiveFlows(count) = super::count_active_flows() else {
            panic!("active flow count response should match request");
        };
        assert_eq!(count, 2);

        let BusResponse::AllActiveFlows(all_flows) = super::dump_active_flows() else {
            panic!("active flow dump response should match request");
        };
        let first = all_flows
            .iter()
            .find(|flow| flow.local_ip == "192.0.2.10")
            .expect("first flow should be in active-flow dump");
        assert_eq!(first.remote_ip, "198.51.100.20");
        assert_eq!(first.src_port, 443);
        assert_eq!(first.dst_port, 50_000);
        assert_eq!(first.bytes_sent, raw_1.bytes_sent);
        assert_eq!(first.packets_sent, raw_1.packets_sent);
        assert_eq!(first.rate_estimate_bps, raw_1.rate_estimate_bps);
        assert_eq!(first.tcp_retransmits, raw_1.tcp_retransmits);
        assert_eq!(first.tos, raw_1.tos);
        assert_eq!(first.flags, raw_1.flags);
        assert_eq!(first.remote_asn, 0);
        assert_eq!(first.remote_asn_name, "");
        assert_eq!(first.remote_asn_country, "");
        assert_eq!(first.analysis, "HTTPS");
        assert_eq!(first.last_seen, raw_1.last_seen);
        assert_eq!(first.start_time, raw_1.start_time);
        assert_eq!(first.circuit_id, "");
        assert_eq!(first.circuit_name, "");

        let BusResponse::TopFlows(top_flows) = super::top_flows(1, TopFlowType::Bytes) else {
            panic!("top flows response should match request");
        };
        assert_eq!(top_flows.len(), 1);
        assert_eq!(top_flows[0].local_ip, "192.0.2.11");
        assert_eq!(top_flows[0].bytes_sent, raw_2.bytes_sent);

        let BusResponse::TopFlows(top_flows) = super::top_flows(1, TopFlowType::RateEstimate)
        else {
            panic!("top flows response should match request");
        };
        assert_eq!(top_flows.len(), 1);
        assert_eq!(top_flows[0].local_ip, "192.0.2.11");
        assert_eq!(top_flows[0].rate_estimate_bps, raw_2.rate_estimate_bps);

        let BusResponse::TopFlows(top_flows) = super::top_flows(1, TopFlowType::Packets) else {
            panic!("top flows response should match request");
        };
        assert_eq!(top_flows.len(), 1);
        assert_eq!(top_flows[0].local_ip, "192.0.2.11");
        assert_eq!(top_flows[0].packets_sent, raw_2.packets_sent);

        let BusResponse::TopFlows(top_flows) = super::top_flows(1, TopFlowType::Drops) else {
            panic!("top flows response should match request");
        };
        assert_eq!(top_flows.len(), 1);
        assert_eq!(top_flows[0].local_ip, "192.0.2.11");
        assert_eq!(top_flows[0].tcp_retransmits, raw_2.tcp_retransmits);

        let BusResponse::TopFlows(top_flows) = super::top_flows(1, TopFlowType::RoundTripTime)
        else {
            panic!("top flows response should match request");
        };
        assert_eq!(top_flows.len(), 1);
        assert_eq!(top_flows[0].local_ip, "192.0.2.10");
        assert!(top_flows[0].rtt_nanos.down > 0);

        let BusResponse::TopFlows(top_flows) = super::top_flows(0, TopFlowType::Bytes) else {
            panic!("top flows response should match request");
        };
        assert!(top_flows.is_empty());

        let BusResponse::TopFlows(top_flows) = super::top_flows(10, TopFlowType::Bytes) else {
            panic!("top flows response should match request");
        };
        assert_eq!(
            top_flows
                .iter()
                .map(|flow| flow.local_ip.as_str())
                .collect::<Vec<_>>(),
            vec!["192.0.2.11", "192.0.2.10"]
        );

        let BusResponse::FlowsByIp(flows_by_ip) = super::flows_by_ip("192.0.2.10") else {
            panic!("flows-by-ip response should match request");
        };
        assert_eq!(flows_by_ip.len(), 1);
        assert_eq!(flows_by_ip[0].local_ip, "192.0.2.10");
        assert_eq!(flows_by_ip[0].remote_ip, "198.51.100.20");

        replace_active_flows_live_for_test(Vec::new());
        let BusResponse::CountActiveFlows(count) = super::count_active_flows() else {
            panic!("active flow count response should match request");
        };
        assert_eq!(count, 0);
        let BusResponse::FlowsByIp(flows_by_ip) = super::flows_by_ip("192.0.2.10") else {
            panic!("flows-by-ip response should match request");
        };
        assert_eq!(flows_by_ip.len(), 1);

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
    }

    #[test]
    fn active_flow_snapshot_publishes_catalog_metadata_for_bus_readers() {
        let mut ctx = ActiveFlowSnapshotTestContext::with_shaped_devices(
            "active-flow-test",
            vec![ShapedDevice {
                circuit_id: "circuit-meta".to_string(),
                circuit_name: "Circuit Metadata".to_string(),
                device_id: "device-meta".to_string(),
                device_name: "Device Metadata".to_string(),
                parent_node: "Parent".to_string(),
                ipv4: vec![(Ipv4Addr::new(192, 0, 2, 20), 32)],
                ..Default::default()
            }],
        );
        let local_ip = XdpIpAddress::from_ip("192.0.2.20".parse().expect("test IP should parse"));
        let remote_ip =
            XdpIpAddress::from_ip("198.51.100.30".parse().expect("test IP should parse"));

        let mut key = FlowbeeKey::default();
        key.local_ip = local_ip;
        key.remote_ip = remote_ip;
        key.ip_protocol = 6;
        key.src_port = 443;
        key.dst_port = 50_100;
        let mut raw = FlowbeeData::default();
        raw.start_time = 10;
        raw.last_seen = 20;
        raw.bytes_sent = DownUpOrder::new(1_000, 2_000);
        raw.packets_sent = DownUpOrder::new(10, 20);
        raw.rate_estimate_bps = DownUpOrder::new(30_000, 40_000);
        let mut local = FlowbeeLocalData::from_flow(&raw, &key);
        local.set_circuit_id_hint(Some("stale-circuit"));

        mutate_all_flows(|flows| {
            flows.clear();
            flows.insert(key, (local, FlowAnalysis::new(&key)));
        });
        refresh_active_flow_snapshot();

        let BusResponse::FlowsByIp(flows_by_ip) = super::flows_by_ip("192.0.2.20") else {
            panic!("flows-by-ip response should match request");
        };
        assert_eq!(flows_by_ip.len(), 1);
        assert_eq!(flows_by_ip[0].circuit_id, "stale-circuit");
        assert_eq!(flows_by_ip[0].circuit_name, "Circuit Metadata");
        let snapshot = active_flow_snapshot();
        assert_eq!(snapshot[0].circuit_hash, Some(hash_to_i64("circuit-meta")));
        assert_eq!(snapshot[0].device_hash, Some(hash_to_i64("device-meta")));
        assert_eq!(snapshot[0].device_name, "Device Metadata");

        ctx.replace_shaped_devices(
            "active-flow-test-updated",
            vec![ShapedDevice {
                circuit_id: "circuit-meta-updated".to_string(),
                circuit_name: "Circuit Metadata Updated".to_string(),
                device_id: "device-meta-updated".to_string(),
                device_name: "Device Metadata Updated".to_string(),
                parent_node: "Parent".to_string(),
                ipv4: vec![(Ipv4Addr::new(192, 0, 2, 20), 32)],
                ..Default::default()
            }],
        );

        let BusResponse::FlowsByIp(flows_by_ip) = super::flows_by_ip("192.0.2.20") else {
            panic!("flows-by-ip response should match request");
        };
        assert_eq!(flows_by_ip[0].circuit_id, "stale-circuit");

        refresh_active_flow_snapshot();
        let BusResponse::FlowsByIp(flows_by_ip) = super::flows_by_ip("192.0.2.20") else {
            panic!("flows-by-ip response should match request");
        };
        assert_eq!(flows_by_ip[0].circuit_id, "stale-circuit");
        assert_eq!(flows_by_ip[0].circuit_name, "Circuit Metadata Updated");
        assert_eq!(
            active_flow_snapshot()[0].device_name,
            "Device Metadata Updated"
        );

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
    }

    #[test]
    fn active_flow_snapshot_prefers_hash_lookup_over_ip_fallback() {
        let ip_matched_device = ShapedDevice {
            circuit_id: "ip-circuit".to_string(),
            circuit_name: "IP Circuit".to_string(),
            device_id: "ip-device".to_string(),
            device_name: "IP Device".to_string(),
            parent_node: "Parent".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 0, 2, 30), 32)],
            ..Default::default()
        };
        let hash_matched_device = ShapedDevice {
            circuit_id: "hash-circuit".to_string(),
            circuit_name: "Hash Circuit".to_string(),
            device_id: "hash-device".to_string(),
            device_name: "Hash Device".to_string(),
            parent_node: "Parent".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 0, 2, 31), 32)],
            ..Default::default()
        };
        let _ctx = ActiveFlowSnapshotTestContext::with_shaped_devices(
            "active-flow-hash-precedence-test",
            vec![ip_matched_device, hash_matched_device],
        );

        let local_ip = XdpIpAddress::from_ip("192.0.2.30".parse().expect("test IP should parse"));
        let catalog = lqos_network_devices::network_devices_catalog();
        let resolved = resolve_flow_device(
            &catalog,
            &local_ip,
            Some(hash_to_i64("hash-device")),
            Some(hash_to_i64("hash-circuit")),
        )
        .expect("hash-backed device should resolve");
        assert_eq!(resolved.device_id, "hash-device");

        let mut key = FlowbeeKey::default();
        key.local_ip = local_ip;
        key.remote_ip =
            XdpIpAddress::from_ip("198.51.100.40".parse().expect("test IP should parse"));
        key.ip_protocol = 6;
        key.src_port = 443;
        key.dst_port = 50_110;
        let mut raw = FlowbeeData::default();
        raw.start_time = 10;
        raw.last_seen = 20;
        raw.bytes_sent = DownUpOrder::new(1_000, 2_000);
        raw.packets_sent = DownUpOrder::new(10, 20);
        raw.rate_estimate_bps = DownUpOrder::new(30_000, 40_000);
        raw.device_hash = hash_to_i64("hash-device") as u64;
        raw.circuit_hash = hash_to_i64("hash-circuit") as u64;

        mutate_all_flows(|flows| {
            flows.clear();
            flows.insert(
                key,
                (
                    FlowbeeLocalData::from_flow(&raw, &key),
                    FlowAnalysis::new(&key),
                ),
            );
        });
        refresh_active_flow_snapshot();

        let snapshot = active_flow_snapshot();
        assert_eq!(snapshot[0].circuit_id, "hash-circuit");
        assert_eq!(snapshot[0].circuit_name, "Hash Circuit");
        assert_eq!(snapshot[0].device_name, "Hash Device");

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
    }

    #[test]
    fn insight_topology_conversion_preserves_coordinates() {
        let current = RawNetJsBody {
            download_bandwidth_mbps: 900,
            upload_bandwidth_mbps: 800,
            latitude: Some(31.861_029),
            longitude: Some(-106.549_46),
            site_type: Some("Site".to_string()),
            children: None,
        }
        .to_lts2("Site A");

        let encoded = to_value(&current).expect("topology payload serializes");
        assert_eq!(
            encoded.get("latitude").and_then(|value| value.as_f64()),
            Some(31.861_028_671_264_65_f64)
        );
        assert_eq!(
            encoded.get("longitude").and_then(|value| value.as_f64()),
            Some(-106.549_461_364_746_1_f64)
        );
    }

    #[test]
    fn old_receivers_ignore_topology_coordinate_fields() {
        let current = RawNetJsBody {
            download_bandwidth_mbps: 900,
            upload_bandwidth_mbps: 800,
            latitude: Some(31.861_029),
            longitude: Some(-106.549_46),
            site_type: Some("Site".to_string()),
            children: None,
        }
        .to_lts2("Site A");

        let bytes = serde_cbor::to_vec(&current).expect("current topology payload serializes");
        let decoded: OldLts2NetJs =
            serde_cbor::from_slice(&bytes).expect("legacy topology shape ignores extra fields");

        assert_eq!(decoded.name, "Site A");
        assert_eq!(decoded.download_bandwidth_mbps, 900);
        assert_eq!(decoded.upload_bandwidth_mbps, 800);
        assert_eq!(decoded.site_type.as_deref(), Some("Site"));
    }

    #[test]
    fn circuit_metadata_falls_back_to_ip_when_entry_circuit_id_is_blank() {
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(vec![ShapedDevice {
            circuit_id: "circuit-1".to_string(),
            circuit_name: "Circuit Alpha".to_string(),
            device_id: "device-1".to_string(),
            parent_node: "Parent-A".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 168, 1, 10), 32)],
            ..Default::default()
        }]);
        let shaped_catalog = ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped));
        let catalog = NetworkDevicesCatalog::from_snapshots(shaped_catalog, Arc::new(Vec::new()));
        let ip = XdpIpAddress::from_ip("192.168.1.10".parse().expect("test IP should parse"));
        let mut entry = ThroughputEntry {
            circuit_id: None,
            circuit_hash: None,
            device_hash: None,
            network_json_parents: None,
            first_cycle: 0,
            most_recent_cycle: 0,
            bytes: DownUpOrder::zeroed(),
            actual_bytes: DownUpOrder::zeroed(),
            packets: DownUpOrder::zeroed(),
            tcp_packets: DownUpOrder::zeroed(),
            udp_packets: DownUpOrder::zeroed(),
            icmp_packets: DownUpOrder::zeroed(),
            prev_bytes: DownUpOrder::zeroed(),
            prev_actual_bytes: DownUpOrder::zeroed(),
            prev_packets: DownUpOrder::zeroed(),
            prev_tcp_packets: DownUpOrder::zeroed(),
            prev_udp_packets: DownUpOrder::zeroed(),
            prev_icmp_packets: DownUpOrder::zeroed(),
            bytes_per_second: DownUpOrder::zeroed(),
            actual_bytes_per_second: DownUpOrder::zeroed(),
            packets_per_second: DownUpOrder::zeroed(),
            tc_handle: TcHandle::from_u32(0),
            rtt_buffer: Default::default(),
            recent_rtt_data: [RttData::from_nanos(0); 60],
            last_fresh_rtt_data_cycle: 0,
            last_seen: 0,
            tcp_retransmits: DownUpOrder::zeroed(),
            tcp_retransmit_packets: DownUpOrder::zeroed(),
            qoq: QoqScores::default(),
        };

        let (circuit_id, circuit_name) = resolve_circuit_metadata_for_entry(&catalog, &ip, &entry);

        assert_eq!(circuit_id, "circuit-1");
        assert_eq!(circuit_name, "Circuit Alpha");

        entry.circuit_id = Some("hint-circuit".to_string());
        let (circuit_id, circuit_name) = resolve_circuit_metadata_for_entry(&catalog, &ip, &entry);

        assert_eq!(circuit_id, "hint-circuit");
        assert_eq!(circuit_name, "Circuit Alpha");
    }

    #[test]
    fn circuit_current_rtt_p50_nanos_reads_from_shared_circuit_buffer() {
        let old_rtt = CIRCUIT_RTT_BUFFERS.load_full();

        let mut rtt = RttBuffer::default();
        rtt.push(
            RttData::from_nanos(11_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        rtt.push(
            RttData::from_nanos(11_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Upload,
            1,
        );
        rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Upload,
            1,
        );

        let mut rtt_map = FxHashMap::default();
        rtt_map.insert(123_i64, rtt);
        CIRCUIT_RTT_BUFFERS.store(Arc::new(rtt_map));

        let result = circuit_current_rtt_p50_nanos(123);

        assert_eq!(result.down, Some(12_000_000));
        assert_eq!(result.up, Some(35_000_000));

        CIRCUIT_RTT_BUFFERS.store(old_rtt);
    }

    #[test]
    fn circuit_current_qoo_reads_from_shared_circuit_heatmap() {
        let mut heatmap = TemporalQoqHeatmap::new();
        heatmap.add_sample(Some(88.0), Some(77.0));

        let mut qoq_heatmaps = crate::throughput_tracker::THROUGHPUT_TRACKER
            .circuit_qoq_heatmaps
            .lock();
        let old_qoq = qoq_heatmaps.clone();
        qoq_heatmaps.clear();
        qoq_heatmaps.insert(456_i64, heatmap);
        drop(qoq_heatmaps);

        let result = circuit_current_qoo(456);
        assert_eq!(result.down, Some(88.0));
        assert_eq!(result.up, Some(77.0));

        let mut qoq_heatmaps = crate::throughput_tracker::THROUGHPUT_TRACKER
            .circuit_qoq_heatmaps
            .lock();
        *qoq_heatmaps = old_qoq;
    }
}

/// K2 slice-3 3a: field oracles for the bridge-ranked reader rows.
#[cfg(test)]
mod bridge_reader_tests {
    use super::{BridgeIpRow, down_up_retransmit_sample};
    use lqos_bus::TcHandle;
    use lqos_utils::units::DownUpOrder;

    /// The bridge per-IP row maps onto the ranked reader's `IpStats` shape
    /// (actual bytes → bits/s, RTT nanos → ms, the direction-agnostic
    /// retransmit sample, the catalog circuit id, default tc handle).
    #[test]
    fn bridge_ip_row_maps_the_reader_fields() {
        let row = BridgeIpRow {
            ip: "192.0.2.10".parse().expect("test IP"),
            bytes: DownUpOrder::new(1_000, 2_000),
            packets: DownUpOrder::new(10, 20),
            rtt_ms: Some(12.5),
            circuit_hash: Some(7),
            runtime_circuit_id: "c1".to_string(),
            retransmits: down_up_retransmit_sample(
                DownUpOrder::new(3, 0),
                DownUpOrder::new(100, 0),
            ),
        };
        let stats = row.to_ip_stats();
        assert_eq!(stats.ip_address, "192.0.2.10");
        assert_eq!(stats.circuit_id, "c1");
        assert_eq!(stats.bits_per_second, DownUpOrder::new(8_000, 16_000));
        assert_eq!(stats.packets_per_second, DownUpOrder::new(10, 20));
        assert_eq!(stats.median_tcp_rtt, 12.5);
        assert_eq!(stats.tc_handle, TcHandle::default());
    }
}

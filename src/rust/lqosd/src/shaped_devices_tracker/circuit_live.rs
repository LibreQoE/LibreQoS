use crate::throughput_tracker::THROUGHPUT_TRACKER;
use fxhash::{FxHashMap, FxHashSet};
use lqos_utils::rtt::{FlowbeeEffectiveDirection, RttBucket};
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample, down_up_retransmit_sample};
use lqos_utils::unix_time::time_since_boot;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{CIRCUIT_LIVE_LAST_REFRESH_SECS, CIRCUIT_LIVE_REFRESH_LOCK, CIRCUIT_LIVE_SNAPSHOT};
use super::{SHAPED_DEVICE_HASH_CACHE, SHAPED_DEVICES};

/// Per-circuit live metrics aggregated from the device-level throughput tracker.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitLiveRollup {
    pub circuit_id: String,
    pub circuit_name: String,
    pub parent_node: String,
    pub device_names: Vec<String>,
    pub ip_addrs: Vec<String>,
    pub plan_mbps: DownUpOrder<f32>,
    pub enqueue_bytes_per_second: DownUpOrder<u64>,
    #[serde(default)]
    pub xmit_bytes_per_second: DownUpOrder<u64>,
    pub rtt_current_p50_nanos: DownUpOrder<Option<u64>>,
    pub qoo: DownUpOrder<Option<f32>>,
    pub tcp_retransmit_sample: DownUpOrder<TcpRetransmitSample>,
    pub last_seen_nanos: u64,
}

/// Shared once-per-second snapshot of circuit rollups and parent-node indexes.
#[derive(Clone, Debug, Default)]
pub struct CircuitLiveSnapshot {
    pub by_circuit_id: FxHashMap<String, CircuitLiveRollup>,
    pub circuit_ids_by_parent_node: FxHashMap<String, Vec<String>>,
}

#[derive(Default)]
struct CircuitAccumulator {
    circuit_name: String,
    parent_node: String,
    device_names: FxHashSet<String>,
    ip_addrs: FxHashSet<String>,
    plan_mbps: DownUpOrder<f32>,
    enqueue_bytes_per_second: DownUpOrder<u64>,
    xmit_bytes_per_second: DownUpOrder<u64>,
    rtt_current_p50_nanos: DownUpOrder<Option<u64>>,
    qoo: DownUpOrder<Option<f32>>,
    enqueue_tcp_packets: DownUpOrder<u64>,
    tcp_retransmits: DownUpOrder<u64>,
    last_seen_nanos: Option<u64>,
}

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn max_opt_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn min_opt_f32(left: Option<f32>, right: Option<f32>) -> Option<f32> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn sort_string_set(values: FxHashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = values.into_iter().collect();
    out.sort_unstable();
    out
}

fn ip_to_string(ip: IpAddr) -> String {
    ip.to_string()
}

fn kernel_age_from_last_seen(kernel_now: std::time::Duration, last_seen: u64) -> u64 {
    if last_seen == 0 {
        return u64::MAX;
    }
    let since_boot = kernel_now.as_nanos();
    since_boot.saturating_sub(last_seen as u128) as u64
}

/// Rebuilds the shared circuit-live snapshot from current tracker state.
///
/// Side effects: stores the rebuilt snapshot in the global `CIRCUIT_LIVE_SNAPSHOT`
/// cache and updates the refresh timestamp used by `fresh_circuit_live_snapshot`.
pub fn rebuild_circuit_live_snapshot() -> Arc<CircuitLiveSnapshot> {
    let Ok(kernel_now) = time_since_boot() else {
        let empty = Arc::new(CircuitLiveSnapshot::default());
        CIRCUIT_LIVE_SNAPSHOT.store(empty.clone());
        return empty;
    };
    let kernel_now: std::time::Duration = kernel_now.into();

    let shaped_devices = SHAPED_DEVICES.load();
    let cache = SHAPED_DEVICE_HASH_CACHE.load();
    let mut by_circuit_id: FxHashMap<String, CircuitAccumulator> = FxHashMap::default();

    for (ip_key, data) in THROUGHPUT_TRACKER.raw_data.lock().iter() {
        let device = data
            .device_hash
            .and_then(|device_hash| cache.index_by_device_hash(&shaped_devices, device_hash))
            .or_else(|| {
                data.circuit_hash.and_then(|circuit_hash| {
                    cache.index_by_circuit_hash(&shaped_devices, circuit_hash)
                })
            })
            .and_then(|idx| shaped_devices.devices.get(idx));
        let Some(device) = device else {
            continue;
        };
        if device.circuit_id.trim().is_empty() {
            continue;
        }

        let entry = by_circuit_id.entry(device.circuit_id.clone()).or_default();
        if entry.circuit_name.is_empty() {
            entry.circuit_name = device.circuit_name.clone();
        }
        if entry.parent_node.is_empty() {
            entry.parent_node = device.parent_node.clone();
        }
        if !device.device_name.trim().is_empty() {
            entry.device_names.insert(device.device_name.clone());
        }
        entry.ip_addrs.insert(ip_to_string(ip_key.as_ip()));
        entry.plan_mbps.down = entry.plan_mbps.down.max(device.download_max_mbps.round());
        entry.plan_mbps.up = entry.plan_mbps.up.max(device.upload_max_mbps.round());
        entry.enqueue_bytes_per_second.down += data.enqueue_bytes_per_second.down;
        entry.enqueue_bytes_per_second.up += data.enqueue_bytes_per_second.up;
        entry.xmit_bytes_per_second.down += data.xmit_bytes_per_second.down;
        entry.xmit_bytes_per_second.up += data.xmit_bytes_per_second.up;
        entry.enqueue_tcp_packets.down += data.tcp_retransmit_packets.down;
        entry.enqueue_tcp_packets.up += data.tcp_retransmit_packets.up;
        entry.tcp_retransmits.down += data.tcp_retransmits.down;
        entry.tcp_retransmits.up += data.tcp_retransmits.up;
        entry.last_seen_nanos = Some(match entry.last_seen_nanos {
            Some(current) => current.min(kernel_age_from_last_seen(kernel_now, data.last_seen)),
            None => kernel_age_from_last_seen(kernel_now, data.last_seen),
        });
        entry.rtt_current_p50_nanos.down = max_opt_u64(
            entry.rtt_current_p50_nanos.down,
            data.rtt_buffer
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
                .map(|rtt| rtt.as_nanos()),
        );
        entry.rtt_current_p50_nanos.up = max_opt_u64(
            entry.rtt_current_p50_nanos.up,
            data.rtt_buffer
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50)
                .map(|rtt| rtt.as_nanos()),
        );
        entry.qoo.down = min_opt_f32(entry.qoo.down, data.qoq.download_total_f32());
        entry.qoo.up = min_opt_f32(entry.qoo.up, data.qoq.upload_total_f32());
    }

    let mut finalized: FxHashMap<String, CircuitLiveRollup> = FxHashMap::default();
    let mut circuit_ids_by_parent_node: FxHashMap<String, Vec<String>> = FxHashMap::default();
    for (circuit_id, value) in by_circuit_id {
        if !value.parent_node.trim().is_empty() {
            circuit_ids_by_parent_node
                .entry(value.parent_node.clone())
                .or_default()
                .push(circuit_id.clone());
        }
        finalized.insert(
            circuit_id.clone(),
            CircuitLiveRollup {
                circuit_id,
                circuit_name: value.circuit_name,
                parent_node: value.parent_node,
                device_names: sort_string_set(value.device_names),
                ip_addrs: sort_string_set(value.ip_addrs),
                plan_mbps: value.plan_mbps,
                enqueue_bytes_per_second: value.enqueue_bytes_per_second,
                xmit_bytes_per_second: value.xmit_bytes_per_second,
                rtt_current_p50_nanos: value.rtt_current_p50_nanos,
                qoo: value.qoo,
                tcp_retransmit_sample: down_up_retransmit_sample(
                    value.tcp_retransmits,
                    value.enqueue_tcp_packets,
                ),
                last_seen_nanos: value.last_seen_nanos.unwrap_or(u64::MAX),
            },
        );
    }
    for ids in circuit_ids_by_parent_node.values_mut() {
        ids.sort_unstable();
    }

    let snapshot = Arc::new(CircuitLiveSnapshot {
        by_circuit_id: finalized,
        circuit_ids_by_parent_node,
    });
    CIRCUIT_LIVE_SNAPSHOT.store(snapshot.clone());
    CIRCUIT_LIVE_LAST_REFRESH_SECS
        .store(current_epoch_secs(), std::sync::atomic::Ordering::Release);
    snapshot
}

/// Returns the current once-per-second circuit-live snapshot, rebuilding it if stale.
///
/// Side effects: may rebuild and replace the global snapshot cache.
pub fn fresh_circuit_live_snapshot() -> Arc<CircuitLiveSnapshot> {
    let now_secs = current_epoch_secs();
    if CIRCUIT_LIVE_LAST_REFRESH_SECS.load(std::sync::atomic::Ordering::Acquire) == now_secs {
        return CIRCUIT_LIVE_SNAPSHOT.load_full();
    }
    let _guard = CIRCUIT_LIVE_REFRESH_LOCK.lock();
    if CIRCUIT_LIVE_LAST_REFRESH_SECS.load(std::sync::atomic::Ordering::Acquire) == now_secs {
        return CIRCUIT_LIVE_SNAPSHOT.load_full();
    }
    rebuild_circuit_live_snapshot()
}

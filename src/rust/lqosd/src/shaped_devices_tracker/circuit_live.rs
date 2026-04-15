use crate::throughput_tracker::{
    THROUGHPUT_TRACKER, circuit_current_qoo, circuit_current_rtt_p50_nanos,
};
use fxhash::{FxHashMap, FxHashSet};
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample, down_up_retransmit_sample};
use lqos_utils::unix_time::time_since_boot;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{CIRCUIT_LIVE_LAST_REFRESH_SECS, CIRCUIT_LIVE_REFRESH_LOCK, CIRCUIT_LIVE_SNAPSHOT};

/// Per-circuit live metrics aggregated from the device-level throughput tracker.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitLiveRollup {
    pub circuit_id: String,
    pub circuit_name: String,
    pub parent_node: String,
    pub device_names: Vec<String>,
    pub ip_addrs: Vec<String>,
    pub plan_mbps: DownUpOrder<f32>,
    pub bytes_per_second: DownUpOrder<u64>,
    pub rtt_current_p50_nanos: DownUpOrder<Option<u64>>,
    pub qoo: DownUpOrder<Option<f32>>,
    pub tcp_retransmit_sample: DownUpOrder<TcpRetransmitSample>,
    pub last_seen_nanos: u64,
}

/// Shared once-per-second snapshot of circuit rollups and parent-node indexes.
#[derive(Clone, Debug, Default)]
pub struct CircuitLiveSnapshot {
    pub by_circuit_id: FxHashMap<String, CircuitLiveRollup>,
}

#[derive(Default)]
struct CircuitAccumulator {
    circuit_hash: Option<i64>,
    circuit_name: String,
    parent_node: String,
    device_names: FxHashSet<String>,
    ip_addrs: FxHashSet<String>,
    plan_mbps: DownUpOrder<f32>,
    bytes_per_second: DownUpOrder<u64>,
    tcp_packets: DownUpOrder<u64>,
    tcp_retransmits: DownUpOrder<u64>,
    last_seen_nanos: Option<u64>,
}

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

    let catalog = lqos_network_devices::network_devices_catalog();
    let mut by_circuit_id: FxHashMap<String, CircuitAccumulator> = FxHashMap::default();

    for (ip_key, data) in THROUGHPUT_TRACKER.raw_data.lock().iter() {
        let device = catalog
            .device_by_hashes(data.device_hash, data.circuit_hash)
            .or_else(|| {
                catalog
                    .device_longest_match_for_ip(ip_key)
                    .map(|(_, dev)| dev)
            });
        let Some(device) = device else {
            continue;
        };
        if device.circuit_id.trim().is_empty() {
            continue;
        }

        let entry = by_circuit_id.entry(device.circuit_id.clone()).or_default();
        if entry.circuit_hash.is_none() {
            entry.circuit_hash = Some(device.circuit_hash);
        }
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
        entry.bytes_per_second.down += data.bytes_per_second.down;
        entry.bytes_per_second.up += data.bytes_per_second.up;
        entry.tcp_packets.down += data.tcp_retransmit_packets.down;
        entry.tcp_packets.up += data.tcp_retransmit_packets.up;
        entry.tcp_retransmits.down += data.tcp_retransmits.down;
        entry.tcp_retransmits.up += data.tcp_retransmits.up;
        entry.last_seen_nanos = Some(match entry.last_seen_nanos {
            Some(current) => current.min(kernel_age_from_last_seen(kernel_now, data.last_seen)),
            None => kernel_age_from_last_seen(kernel_now, data.last_seen),
        });
    }

    let mut finalized: FxHashMap<String, CircuitLiveRollup> = FxHashMap::default();
    for (circuit_id, value) in by_circuit_id {
        let parent_node = super::effective_parent_for_circuit(&circuit_id)
            .map(|parent| parent.name)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(value.parent_node);
        let rtt_current_p50_nanos = value
            .circuit_hash
            .map(circuit_current_rtt_p50_nanos)
            .unwrap_or_else(DownUpOrder::default);
        let qoo = value
            .circuit_hash
            .map(circuit_current_qoo)
            .unwrap_or_default();
        finalized.insert(
            circuit_id.clone(),
            CircuitLiveRollup {
                circuit_id,
                circuit_name: value.circuit_name,
                parent_node,
                device_names: sort_string_set(value.device_names),
                ip_addrs: sort_string_set(value.ip_addrs),
                plan_mbps: value.plan_mbps,
                bytes_per_second: value.bytes_per_second,
                rtt_current_p50_nanos,
                qoo,
                tcp_retransmit_sample: down_up_retransmit_sample(
                    value.tcp_retransmits,
                    value.tcp_packets,
                ),
                last_seen_nanos: value.last_seen_nanos.unwrap_or(u64::MAX),
            },
        );
    }

    let snapshot = Arc::new(CircuitLiveSnapshot {
        by_circuit_id: finalized,
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

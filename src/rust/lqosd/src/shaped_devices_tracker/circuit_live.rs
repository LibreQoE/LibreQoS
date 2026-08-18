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

fn rollup_to_bus(rollup: &CircuitLiveRollup) -> lqos_bus::CircuitRollup {
    lqos_bus::CircuitRollup {
        circuit_id: rollup.circuit_id.clone(),
        circuit_name: rollup.circuit_name.clone(),
        parent_node: rollup.parent_node.clone(),
        device_names: rollup.device_names.clone(),
        ip_addrs: rollup.ip_addrs.clone(),
        plan_mbps: rollup.plan_mbps,
        bytes_per_second: rollup.bytes_per_second,
        rtt_current_p50_nanos: rollup.rtt_current_p50_nanos,
        qoo: rollup.qoo,
        tcp_retransmit_sample: rollup.tcp_retransmit_sample,
        last_seen_nanos: rollup.last_seen_nanos,
    }
}

fn rollups_from_snapshot(snapshot: &CircuitLiveSnapshot) -> Vec<lqos_bus::CircuitRollup> {
    let mut rollups = snapshot
        .by_circuit_id
        .values()
        .map(rollup_to_bus)
        .collect::<Vec<_>>();
    rollups.sort_by(|left, right| left.circuit_id.cmp(&right.circuit_id));
    rollups
}

fn rollup_from_snapshot(
    snapshot: &CircuitLiveSnapshot,
    circuit_id: &str,
) -> Option<lqos_bus::CircuitRollup> {
    let circuit_id = circuit_id.trim();
    if circuit_id.is_empty() {
        return None;
    }
    snapshot.by_circuit_id.get(circuit_id).map(rollup_to_bus)
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
        entry.plan_mbps.down = entry.plan_mbps.down.max(device.download_max_mbps);
        entry.plan_mbps.up = entry.plan_mbps.up.max(device.upload_max_mbps);
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

/// Returns live circuit rollups aggregated by circuit ID.
///
/// Side effects: may refresh the shared circuit-live snapshot if it is stale.
pub fn all_circuit_rollups() -> Vec<lqos_bus::CircuitRollup> {
    rollups_from_snapshot(&fresh_circuit_live_snapshot())
}

/// Returns the live circuit rollup for one circuit ID.
///
/// Side effects: may refresh the shared circuit-live snapshot if it is stale.
pub fn circuit_rollup_by_id(circuit_id: &str) -> Option<lqos_bus::CircuitRollup> {
    rollup_from_snapshot(&fresh_circuit_live_snapshot(), circuit_id)
}

#[cfg(test)]
mod tests {
    use super::{
        CircuitLiveRollup, CircuitLiveSnapshot, rollup_from_snapshot, rollups_from_snapshot,
    };
    use fxhash::FxHashMap;
    use lqos_utils::units::{DownUpOrder, TcpRetransmitSample};

    fn rollup(circuit_id: &str, bytes_down: u64) -> CircuitLiveRollup {
        CircuitLiveRollup {
            circuit_id: circuit_id.to_string(),
            circuit_name: format!("{circuit_id} name"),
            parent_node: "parent".to_string(),
            device_names: vec![format!("{circuit_id} device")],
            ip_addrs: vec!["192.0.2.10".to_string()],
            plan_mbps: DownUpOrder {
                down: 100.0,
                up: 50.0,
            },
            bytes_per_second: DownUpOrder {
                down: bytes_down,
                up: 123,
            },
            rtt_current_p50_nanos: DownUpOrder {
                down: Some(1000),
                up: Some(2000),
            },
            qoo: DownUpOrder {
                down: Some(95.0),
                up: Some(93.0),
            },
            tcp_retransmit_sample: DownUpOrder {
                down: TcpRetransmitSample::new(2, 100),
                up: TcpRetransmitSample::new(1, 50),
            },
            last_seen_nanos: 10,
        }
    }

    fn snapshot(rollups: Vec<CircuitLiveRollup>) -> CircuitLiveSnapshot {
        let mut by_circuit_id = FxHashMap::default();
        for rollup in rollups {
            by_circuit_id.insert(rollup.circuit_id.clone(), rollup);
        }
        CircuitLiveSnapshot { by_circuit_id }
    }

    #[test]
    fn rollups_from_snapshot_are_sorted_and_preserve_totals() {
        let snapshot = snapshot(vec![rollup("circuit-b", 200), rollup("circuit-a", 100)]);

        let rollups = rollups_from_snapshot(&snapshot);

        assert_eq!(rollups[0].circuit_id, "circuit-a");
        assert_eq!(rollups[0].bytes_per_second.down, 100);
        assert_eq!(rollups[1].circuit_id, "circuit-b");
        assert_eq!(rollups[1].bytes_per_second.down, 200);
    }

    #[test]
    fn rollup_from_snapshot_matches_exact_trimmed_circuit_id() {
        let snapshot = snapshot(vec![rollup("Circuit-1", 300)]);

        let rollup = rollup_from_snapshot(&snapshot, " Circuit-1 ")
            .expect("trimmed circuit ID should match");

        assert_eq!(rollup.circuit_id, "Circuit-1");
        assert_eq!(rollup.bytes_per_second.down, 300);
        assert!(rollup_from_snapshot(&snapshot, "circuit-1").is_none());
        assert!(rollup_from_snapshot(&snapshot, " ").is_none());
    }
}

use crate::throughput_tracker::THROUGHPUT_TRACKER;
use axum::Json;
use serde::Serialize;
use std::time::Duration;

const RECENT_WINDOW_SECONDS: u64 = 5 * 60;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub(crate) enum ShapedDeviceMatchSource {
    DeviceHash,
    CircuitHash,
    IpFallback,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct ThroughputAttributionDebug {
    window_seconds: u64,
    total_entries: usize,
    recent_entries: usize,
    recent_shaped_entries: usize,
    recent_unshaped_entries: usize,
    recent_entries_missing_device_hash: usize,
    recent_entries_missing_circuit_hash: usize,
    recent_entries_missing_both_hashes: usize,
    recent_entries_resolved_by_device_hash: usize,
    recent_entries_resolved_by_circuit_hash: usize,
    recent_entries_resolved_by_ip_fallback: usize,
    recent_ip_fallback_entries_missing_device_hash: usize,
    recent_ip_fallback_entries_missing_circuit_hash: usize,
    recent_ip_fallback_entries_missing_both_hashes: usize,
    recent_ip_fallback_entries_with_both_hashes_present: usize,
    recent_entries_unresolved: usize,
}

/// Returns attribution counters for recent throughput entries.
///
/// This endpoint is read-only. It inspects the in-memory throughput tracker to
/// show how active entries are currently being associated with shaped devices.
pub(crate) async fn throughput_attribution_debug() -> Json<ThroughputAttributionDebug> {
    Json(throughput_attribution_debug_data())
}

pub(crate) fn throughput_attribution_debug_data() -> ThroughputAttributionDebug {
    let Ok(time_since_boot) = lqos_utils::unix_time::time_since_boot() else {
        return ThroughputAttributionDebug {
            window_seconds: RECENT_WINDOW_SECONDS,
            total_entries: 0,
            recent_entries: 0,
            recent_shaped_entries: 0,
            recent_unshaped_entries: 0,
            recent_entries_missing_device_hash: 0,
            recent_entries_missing_circuit_hash: 0,
            recent_entries_missing_both_hashes: 0,
            recent_entries_resolved_by_device_hash: 0,
            recent_entries_resolved_by_circuit_hash: 0,
            recent_entries_resolved_by_ip_fallback: 0,
            recent_ip_fallback_entries_missing_device_hash: 0,
            recent_ip_fallback_entries_missing_circuit_hash: 0,
            recent_ip_fallback_entries_missing_both_hashes: 0,
            recent_ip_fallback_entries_with_both_hashes_present: 0,
            recent_entries_unresolved: 0,
        };
    };
    let now = Duration::from(time_since_boot).as_nanos() as u64;
    let recent_cutoff_nanos = RECENT_WINDOW_SECONDS * 1_000_000_000;

    let catalog = lqos_network_devices::network_devices_catalog();
    let raw = THROUGHPUT_TRACKER.raw_data.lock();

    let mut stats = ThroughputAttributionDebug {
        window_seconds: RECENT_WINDOW_SECONDS,
        total_entries: raw.len(),
        recent_entries: 0,
        recent_shaped_entries: 0,
        recent_unshaped_entries: 0,
        recent_entries_missing_device_hash: 0,
        recent_entries_missing_circuit_hash: 0,
        recent_entries_missing_both_hashes: 0,
        recent_entries_resolved_by_device_hash: 0,
        recent_entries_resolved_by_circuit_hash: 0,
        recent_entries_resolved_by_ip_fallback: 0,
        recent_ip_fallback_entries_missing_device_hash: 0,
        recent_ip_fallback_entries_missing_circuit_hash: 0,
        recent_ip_fallback_entries_missing_both_hashes: 0,
        recent_ip_fallback_entries_with_both_hashes_present: 0,
        recent_entries_unresolved: 0,
    };

    for (ip, entry) in raw.iter() {
        if now.saturating_sub(entry.last_seen) >= recent_cutoff_nanos {
            continue;
        }

        stats.recent_entries += 1;
        if entry.tc_handle.as_u32() == 0 {
            stats.recent_unshaped_entries += 1;
        } else {
            stats.recent_shaped_entries += 1;
        }

        let missing_device_hash = entry.device_hash.is_none();
        let missing_circuit_hash = entry.circuit_hash.is_none();
        if missing_device_hash {
            stats.recent_entries_missing_device_hash += 1;
        }
        if missing_circuit_hash {
            stats.recent_entries_missing_circuit_hash += 1;
        }
        if missing_device_hash && missing_circuit_hash {
            stats.recent_entries_missing_both_hashes += 1;
        }

        match match_source_for_entry(&catalog, ip, entry.device_hash, entry.circuit_hash) {
            Some((_device, ShapedDeviceMatchSource::DeviceHash)) => {
                stats.recent_entries_resolved_by_device_hash += 1;
            }
            Some((_device, ShapedDeviceMatchSource::CircuitHash)) => {
                stats.recent_entries_resolved_by_circuit_hash += 1;
            }
            Some((_device, ShapedDeviceMatchSource::IpFallback)) => {
                stats.recent_entries_resolved_by_ip_fallback += 1;
                if missing_device_hash {
                    stats.recent_ip_fallback_entries_missing_device_hash += 1;
                }
                if missing_circuit_hash {
                    stats.recent_ip_fallback_entries_missing_circuit_hash += 1;
                }
                if missing_device_hash && missing_circuit_hash {
                    stats.recent_ip_fallback_entries_missing_both_hashes += 1;
                }
                if !missing_device_hash && !missing_circuit_hash {
                    stats.recent_ip_fallback_entries_with_both_hashes_present += 1;
                }
            }
            None => {
                stats.recent_entries_unresolved += 1;
            }
        }
    }

    stats
}

fn match_source_for_entry<'a>(
    catalog: &'a lqos_network_devices::NetworkDevicesCatalog,
    ip: &lqos_utils::XdpIpAddress,
    device_hash: Option<i64>,
    circuit_hash: Option<i64>,
) -> Option<(&'a lqos_config::ShapedDevice, ShapedDeviceMatchSource)> {
    if let Some(hash) = device_hash {
        if let Some(device) = catalog.device_by_hashes(Some(hash), None) {
            return Some((device, ShapedDeviceMatchSource::DeviceHash));
        }
    }

    if let Some(hash) = circuit_hash {
        if let Some(device) = catalog.device_by_hashes(None, Some(hash)) {
            return Some((device, ShapedDeviceMatchSource::CircuitHash));
        }
    }

    catalog
        .device_longest_match_for_ip(ip)
        .map(|(_net, device)| (device, ShapedDeviceMatchSource::IpFallback))
}

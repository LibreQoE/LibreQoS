use crate::throughput_tracker::THROUGHPUT_TRACKER;
use itertools::Itertools;
use lqos_config::load_config;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::time_since_boot;
use serde::Serialize;
use std::collections::HashSet;
use std::time::Duration;
use tracing::warn;

#[derive(Debug, Serialize)]
pub struct UnknownIp {
    ip: String,
    last_seen_nanos: u64,
    total_bytes: DownUpOrder<u64>,
    current_bytes: DownUpOrder<u64>,
}

pub fn get_unknown_ips() -> Vec<UnknownIp> {
    const FIVE_MINUTES_IN_NANOS: u64 = 5 * 60 * 1_000_000_000;

    let Ok(config) = load_config() else {
        warn!("Failed to load config");
        return vec![];
    };
    let Ok(allowed_ips) = config.ip_ranges.allowed_network_table() else {
        warn!("Unable to load allowed network table");
        return vec![];
    };
    let Ok(ignored_ips) = config.ip_ranges.ignored_network_table() else {
        warn!("Unable to load ignored network table");
        return vec![];
    };

    let now = time_since_boot()
        .map(|ts| Duration::from(ts).as_nanos() as u64)
        .unwrap_or(0);
    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        // Remove all loopback devices
        .filter(|(k, _v)| !k.as_ip().is_loopback())
        // Remove any items that have a tc_handle of 0
        .filter(|(_k, d)| d.tc_handle.as_u32() == 0)
        // Remove any items that are matched by the shaped devices file
        .filter(|(k, _d)| {
            let ip = k.as_ip();
            // If the IP is in the ignored list, ignore it
            if config.ip_ranges.unknown_ip_honors_ignore.unwrap_or(true)
                && ignored_ips.longest_match(ip).is_some()
            {
                return false;
            }
            // If the IP is not in the allowed list, ignore it
            if config.ip_ranges.unknown_ip_honors_allow.unwrap_or(true)
                && allowed_ips.longest_match(ip).is_none()
            {
                return false;
            }
            true
        })
        // Convert to UnknownIp
        .map(|(k, d)| UnknownIp {
            ip: k.as_ip().to_string(),
            last_seen_nanos: now.saturating_sub(d.last_seen),
            total_bytes: d.bytes,
            current_bytes: d.bytes_per_second,
        })
        // Remove any items that have not been seen in the last 5 minutes
        .filter(|u| u.last_seen_nanos < FIVE_MINUTES_IN_NANOS)
        .sorted_by(|a, b| a.last_seen_nanos.cmp(&b.last_seen_nanos))
        .collect()
}

pub fn unknown_ips_csv_data() -> String {
    let list = get_unknown_ips();

    let mut csv = String::new();
    csv.push_str("IP Address,Total Download (bytes),Total Upload (bytes)\n");
    for unknown in list.into_iter() {
        csv.push_str(&format!(
            "{},{},{}\n",
            unknown.ip, unknown.total_bytes.down, unknown.total_bytes.up
        ));
    }

    csv
}

#[derive(Debug, Serialize)]
pub struct ClearUnknownIpsResponse {
    cleared: usize,
}

pub fn clear_unknown_ips_data() -> ClearUnknownIpsResponse {
    // Load config and shaped devices to mirror the same filtering logic used for display
    let Ok(config) = load_config() else {
        warn!("Failed to load config");
        return ClearUnknownIpsResponse { cleared: 0 };
    };
    let Ok(allowed_ips) = config.ip_ranges.allowed_network_table() else {
        warn!("Could not load allowed IP table");
        return ClearUnknownIpsResponse { cleared: 0 };
    };
    let Ok(ignored_ips) = config.ip_ranges.ignored_network_table() else {
        warn!("Could not load ignored IP table");
        return ClearUnknownIpsResponse { cleared: 0 };
    };

    // Now time for last_seen comparison (match the 5 minute window used in get_unknown_ips)
    // If the system clock isn't ready yet (very early after boot), do nothing to avoid mass deletion.
    let Ok(ts) = time_since_boot() else {
        return ClearUnknownIpsResponse { cleared: 0 };
    };
    let now = Duration::from(ts).as_nanos() as u64;
    const FIVE_MINUTES_IN_NANOS: u64 = 5 * 60 * 1_000_000_000;

    // Determine the set of keys to remove
    let to_remove: Vec<lqos_utils::XdpIpAddress> = {
        let raw = THROUGHPUT_TRACKER.raw_data.lock();
        raw.iter()
            // Remove all loopback devices
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            // Only those with tc_handle == 0 (unknown/unshaped)
            .filter(|(_k, d)| d.tc_handle.as_u32() == 0)
            // Honor ignored list (if enabled)
            .filter(|(k, _d)| {
                let ip = k.as_ip();
                !(config.ip_ranges.unknown_ip_honors_ignore.unwrap_or(true)
                    && ignored_ips.longest_match(ip).is_some())
            })
            // Honor allowed list (if enabled)
            .filter(|(k, _d)| {
                let ip = k.as_ip();
                !(config.ip_ranges.unknown_ip_honors_allow.unwrap_or(true)
                    && allowed_ips.longest_match(ip).is_none())
            })
            // Only those seen within the last 5 minutes (matches display)
            .filter(|(_k, d)| now.saturating_sub(d.last_seen) < FIVE_MINUTES_IN_NANOS)
            .map(|(k, _)| *k)
            .collect()
    };

    let to_remove_set: HashSet<_> = to_remove.iter().cloned().collect();
    let cleared = to_remove_set.len();

    // Hard clear: remove from in-memory tracker and expire from kernel map
    // Minimize lock time: remove only targeted keys; avoid full scan and shrink.
    {
        let mut raw = THROUGHPUT_TRACKER.raw_data.lock();
        for k in to_remove_set.iter() {
            raw.remove(k);
        }
    }

    if cleared > 0 {
        if let Err(e) = lqos_sys::expire_throughput(to_remove) {
            warn!("Failed to expire throughput entries during clear: {:?}", e);
        }
    }

    ClearUnknownIpsResponse { cleared }
}

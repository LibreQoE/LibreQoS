use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
use itertools::Itertools;
use lqos_config::load_config;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::time_since_boot;
use serde::Serialize;
use std::collections::HashSet;
use std::time::Duration;
use tracing::warn;

#[derive(Serialize)]
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
    let allowed_ips = config.ip_ranges.allowed_network_table();
    let ignored_ips = config.ip_ranges.ignored_network_table();

    let now = Duration::from(time_since_boot().unwrap()).as_nanos() as u64;
    let sd_reader = SHAPED_DEVICES.load();
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
            // If the IP is in shaped devices, ignore it
            sd_reader.trie.longest_match(ip).is_none()
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

pub async fn unknown_ips() -> axum::Json<Vec<UnknownIp>> {
    axum::Json(get_unknown_ips())
}

pub async fn unknown_ips_csv() -> String {
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

#[derive(Serialize)]
pub struct ClearUnknownIpsResponse {
    cleared: usize,
}

pub async fn clear_unknown_ips() -> axum::Json<ClearUnknownIpsResponse> {
    // Load config and shaped devices to mirror the same filtering logic used for display
    let Ok(config) = load_config() else {
        warn!("Failed to load config");
        return axum::Json(ClearUnknownIpsResponse { cleared: 0 });
    };
    let allowed_ips = config.ip_ranges.allowed_network_table();
    let ignored_ips = config.ip_ranges.ignored_network_table();

    let sd_reader = SHAPED_DEVICES.load();

    // Now time for last_seen comparison (match the 5 minute window used in get_unknown_ips)
    let now = match time_since_boot() {
        Ok(ts) => Duration::from(ts).as_nanos() as u64,
        Err(_) => 0,
    };
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
            // Only those not in shaped devices
            .filter(|(k, _d)| sd_reader.trie.longest_match(k.as_ip()).is_none())
            // Only those seen within the last 5 minutes (matches display)
            .filter(|(_k, d)| now.saturating_sub(d.last_seen) < FIVE_MINUTES_IN_NANOS)
            .map(|(k, _)| *k)
            .collect()
    };

    let to_remove_set: HashSet<_> = to_remove.iter().cloned().collect();
    let cleared = to_remove_set.len();

    // Hard clear: remove from in-memory tracker and expire from kernel map
    {
        let mut raw = THROUGHPUT_TRACKER.raw_data.lock();
        raw.retain(|k, _| !to_remove_set.contains(k));
        raw.shrink_to_fit();
    }

    if cleared > 0 {
        if let Err(e) = lqos_sys::expire_throughput(to_remove) {
            warn!("Failed to expire throughput entries during clear: {:?}", e);
        }
    }

    axum::Json(ClearUnknownIpsResponse { cleared })
}

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

/// `lqos-74t.52`: the page/CSV row order — bytes **Uploaded** descending, with
/// equal-upload ties broken by IP ascending so the order is reproducible across
/// refreshes. A plain "stable" sort would instead keep the source hash-map
/// iteration order (`best_effort_by_ip: FxHashMap`), which can reorder an
/// all-equal group between polls.
fn cmp_upload_desc_then_ip(a: &UnknownIp, b: &UnknownIp) -> std::cmp::Ordering {
    b.total_bytes.up.cmp(&a.total_bytes.up).then_with(|| a.ip.cmp(&b.ip))
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

    // §`lqos-74t.49.2`: on a v3 DPDK box the kernel XDP `raw_data` source is
    // RETIRED (no writer post-K3). Read the bridge's best-effort/unmapped host
    // class instead (`BridgeStatsSnapshot.best_effort_by_ip`, `.49.1`). That
    // class carries CUMULATIVE bytes (`total_bytes`) + `last_seen_nanos`; it has
    // no flow deltas, so there is no interval rate — `current_bytes` is 0
    // (stated, never a fake rate). The kernel path stays for `!is_active()`.
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        return snapshot
            .best_effort_by_ip
            .iter()
            // Remove all loopback entries.
            .filter(|(ip, _)| !ip.is_loopback())
            // The SAME ignore/allow filters the kernel path uses.
            .filter(|(ip, _)| {
                let ip = **ip;
                if config.ip_ranges.unknown_ip_honors_ignore.unwrap_or(true)
                    && ignored_ips.longest_match(ip).is_some()
                {
                    return false;
                }
                if config.ip_ranges.unknown_ip_honors_allow.unwrap_or(true)
                    && allowed_ips.longest_match(ip).is_none()
                {
                    return false;
                }
                true
            })
            // The 300 s window backstop (identical to `.49.1`'s map retention);
            // also drops `u64::MAX` never-seen rows.
            .filter(|(_ip, row)| row.last_seen_nanos < FIVE_MINUTES_IN_NANOS)
            .map(|(ip, row)| UnknownIp {
                ip: ip.to_string(),
                last_seen_nanos: row.last_seen_nanos,
                // The enqueued/pre-shaping CUMULATIVE totals.
                total_bytes: row.total_bytes,
                // No interval rate for the class — the page shows the totals.
                current_bytes: DownUpOrder::new(0, 0),
            })
            .sorted_by(|a, b| cmp_upload_desc_then_ip(a, b))
            .collect();
    }

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
        .sorted_by(|a, b| cmp_upload_desc_then_ip(a, b))
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
    // §`lqos-74t.49.2`: on a v3 DPDK box the source is the bridge's LIVE
    // best-effort snapshot (an interval view), NOT an accumulating kernel map —
    // there is nothing to clear. Disclosed no-op (`cleared: 0`); the kernel path
    // stays for `!is_active()`.
    if crate::bridge_analytics::is_active() {
        return ClearUnknownIpsResponse { cleared: 0 };
    }
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

    let _ = to_remove;

    ClearUnknownIpsResponse { cleared }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ip: &str, up: u64) -> UnknownIp {
        UnknownIp {
            ip: ip.to_string(),
            last_seen_nanos: 0,
            total_bytes: DownUpOrder::new(0, up),
            current_bytes: DownUpOrder::new(0, 0),
        }
    }

    /// `lqos-74t.52` oracle: bytes Uploaded strictly DESCENDING, with equal-upload
    /// ties ordered by IP ascending (reproducible across refreshes — NOT the
    /// source hash map's iteration order).
    #[test]
    fn unknown_ips_sort_by_upload_desc_then_ip() {
        let mut rows = vec![
            row("10.0.0.3", 100),
            row("10.0.0.1", 500),
            row("10.0.0.2", 100),
            row("10.0.0.4", 999),
        ];
        rows.sort_by(cmp_upload_desc_then_ip);
        let order: Vec<&str> = rows.iter().map(|r| r.ip.as_str()).collect();
        assert_eq!(order, vec!["10.0.0.4", "10.0.0.1", "10.0.0.2", "10.0.0.3"]);
        let uploads: Vec<u64> = rows.iter().map(|r| r.total_bytes.up).collect();
        assert_eq!(uploads, vec![999, 500, 100, 100], "descending by uploaded bytes");
        // Equal-upload ties are IP ascending (deterministic, not hash order).
        assert_eq!(rows[2].total_bytes.up, rows[3].total_bytes.up);
        assert!(rows[2].ip < rows[3].ip, "equal-upload ties are IP ascending");
    }
}

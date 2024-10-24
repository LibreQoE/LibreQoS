use std::time::Duration;
use itertools::Itertools;
use serde::Serialize;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::time_since_boot;
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::THROUGHPUT_TRACKER;

#[derive(Serialize)]
pub struct UnknownIp {
    ip: String,
    last_seen_nanos: u64,
    total_bytes: DownUpOrder<u64>,
    current_bytes: DownUpOrder<u64>,
}

pub fn get_unknown_ips() -> Vec<UnknownIp> {
    const FIVE_MINUTES_IN_NANOS: u64 = 5 * 60 * 1_000_000_000;

    let now = Duration::from(time_since_boot().unwrap()).as_nanos() as u64;
    let sd_reader = SHAPED_DEVICES.read().unwrap();
    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .unwrap()
        .iter()
        .filter(|(k,_v)| !k.as_ip().is_loopback())
        .filter(|(_k,d)| d.tc_handle.as_u32() == 0)
        .filter(|(k,_d)| {
            let ip = k.as_ip();
            !sd_reader.trie.longest_match(ip).is_some()
        })
        .map(|(k,d)| {
            UnknownIp {
                ip: k.as_ip().to_string(),
                last_seen_nanos: now - d.last_seen,
                total_bytes: d.bytes,
                current_bytes: d.bytes_per_second,
            }
        })
        .filter(|u| u.last_seen_nanos <FIVE_MINUTES_IN_NANOS )
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
            unknown.ip,
            unknown.total_bytes.down,
            unknown.total_bytes.up
        ));
    }

    csv
}
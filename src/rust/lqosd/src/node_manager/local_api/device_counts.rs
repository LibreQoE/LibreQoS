use std::net::IpAddr;
use axum::Json;
use serde::Serialize;
use lqos_bus::{BusResponse, IpStats};
use crate::shaped_devices_tracker::SHAPED_DEVICES;

#[derive(Serialize)]
pub struct DeviceCount {
    pub shaped_devices: usize,
    pub unknown_ips: usize,
}

fn unknown_device_count() -> usize {
    if let BusResponse::AllUnknownIps(unknowns) = crate::throughput_tracker::all_unknown_ips() {
        let cfg = SHAPED_DEVICES.read().unwrap();
        let really_unknown: Vec<IpStats> = unknowns
            .iter()
            .filter(|ip| {
                if let Ok(ip) = ip.ip_address.parse::<IpAddr>() {
                    let lookup = match ip {
                        IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                        IpAddr::V6(ip) => ip,
                    };
                    cfg.trie.longest_match(lookup).is_none()
                } else {
                    false
                }
            })
            .cloned()
            .collect();
        return really_unknown.len();
    }
    
    0
}

pub async fn count_users() -> Json<DeviceCount> {
    Json(DeviceCount{
        shaped_devices: SHAPED_DEVICES.read().unwrap().devices.len(),
        unknown_ips: unknown_device_count(),
    })
}

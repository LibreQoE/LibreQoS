mod cache_manager;
mod cache;
pub use cache::{SHAPED_DEVICES, UNKNOWN_DEVICES};
pub use cache_manager::update_tracking;
use std::net::IpAddr;
use lqos_bus::{IpStats, TcHandle};
use rocket::serde::{json::Json, Serialize, Deserialize};
use crate::tracker::cache::ThroughputPerSecond;
use self::cache::{CURRENT_THROUGHPUT, THROUGHPUT_BUFFER, CPU_USAGE, MEMORY_USAGE, TOP_10_DOWNLOADERS, WORST_10_RTT, RTT_HISTOGRAM, HOST_COUNTS};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "rocket::serde")]
pub struct IpStatsWithPlan {
    pub ip_address: String,
    pub bits_per_second: (u64, u64),
    pub packets_per_second: (u64, u64),
    pub median_tcp_rtt: f32,
    pub tc_handle: TcHandle,
    pub circuit_id: String,
    pub plan: (u32, u32),
}

impl From<&IpStats> for IpStatsWithPlan {
    fn from(i: &IpStats) -> Self {
        let mut result = Self {
            ip_address: i.ip_address.clone(),
            bits_per_second: i.bits_per_second,
            packets_per_second: i.packets_per_second,
            median_tcp_rtt: i.median_tcp_rtt,
            tc_handle: i.tc_handle,
            circuit_id: String::new(),
            plan: (0, 0),
        };
        if let Ok(ip) = result.ip_address.parse::<IpAddr>() {
            let lookup = match ip {
                IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                IpAddr::V6(ip) => ip,
            };
            let cfg = SHAPED_DEVICES.read();
            if let Some((_, id)) = cfg.trie.longest_match(lookup) {
                result.ip_address = format!("{} ({})", cfg.devices[*id].circuit_name, result.ip_address);
                result.plan.0 = cfg.devices[*id].download_max_mbps;
                result.plan.1 = cfg.devices[*id].upload_max_mbps;
                result.circuit_id = cfg.devices[*id].circuit_id.clone();
            }
        }
        result
    }
}

#[get("/api/current_throughput")]
pub fn current_throughput() -> Json<ThroughputPerSecond> {
    let result = CURRENT_THROUGHPUT.read().clone();
    Json(result)
}

#[get("/api/throughput_ring")]
pub fn throughput_ring() -> Json<Vec<ThroughputPerSecond>> {
    let result = THROUGHPUT_BUFFER.read().get_result();
    Json(result)
}

#[get("/api/cpu")]
pub fn cpu_usage() -> Json<Vec<f32>> {
    let cpu_usage = CPU_USAGE.read().clone();

    Json(cpu_usage)
}

#[get("/api/ram")]
pub fn ram_usage() -> Json<Vec<u64>> {
    let ram_usage = MEMORY_USAGE.read().clone();
    Json(ram_usage)
}

#[get("/api/top_10_downloaders")]
pub fn top_10_downloaders() -> Json<Vec<IpStatsWithPlan>> {
    let tt : Vec<IpStatsWithPlan> = TOP_10_DOWNLOADERS.read().iter().map(|tt| tt.into()).collect();
    Json(tt)
}

#[get("/api/worst_10_rtt")]
pub fn worst_10_rtt() -> Json<Vec<IpStatsWithPlan>> {
    let tt : Vec<IpStatsWithPlan> = WORST_10_RTT.read().iter().map(|tt| tt.into()).collect();
    Json(tt)
}


#[get("/api/rtt_histogram")]
pub fn rtt_histogram() -> Json<Vec<u32>> {
    Json(RTT_HISTOGRAM.read().clone())
}

#[get("/api/host_counts")]
pub fn host_counts() -> Json<(u32, u32)> {
    let shaped_reader = SHAPED_DEVICES.read();
    let n_devices = shaped_reader.devices.len();
    let host_counts = HOST_COUNTS.read();
    let unknown = host_counts.0 - host_counts.1;
    Json((n_devices as u32, unknown))
}

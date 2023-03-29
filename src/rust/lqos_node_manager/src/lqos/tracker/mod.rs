mod cache;
mod cache_manager;

use self::cache::{
  CPU_USAGE, CURRENT_THROUGHPUT, HOST_COUNTS, NUM_CPUS, RAM_USED,
  RTT_HISTOGRAM, THROUGHPUT_BUFFER, TOP_10_DOWNLOADERS, TOTAL_RAM,
  WORST_10_RTT,
};
pub use cache::{SHAPED_DEVICES, UNKNOWN_DEVICES};
pub use cache_manager::update_tracking;
use axum::Json;
use lazy_static::lazy_static;
use lqos_bus::{IpStats, TcHandle};
use lqos_config::{LibreQoSConfig, ShapedDevice};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

use cache::ThroughputPerSecond;

lazy_static! {
  static ref CONFIG: Mutex<LibreQoSConfig> =
    Mutex::new(lqos_config::LibreQoSConfig::load().unwrap());
}

#[derive(Serialize, Deserialize, Clone, Debug)]
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
        result.ip_address =
          format!("{} ({})", cfg.devices[*id].circuit_name, result.ip_address);
        result.plan.0 = cfg.devices[*id].download_max_mbps;
        result.plan.1 = cfg.devices[*id].upload_max_mbps;
        result.circuit_id = cfg.devices[*id].circuit_id.clone();
      }
    }
    result
  }
}

pub fn current_throughput() -> ThroughputPerSecond {
  let result = *CURRENT_THROUGHPUT.read();
  result
}

pub fn throughput_ring() -> Vec<ThroughputPerSecond> {
  let result = THROUGHPUT_BUFFER.read().get_result();
  result
}

pub fn cpu_usage() -> Vec<u32> {
  let usage: Vec<u32> = CPU_USAGE
    .iter()
    .take(NUM_CPUS.load(std::sync::atomic::Ordering::Relaxed))
    .map(|cpu| cpu.load(std::sync::atomic::Ordering::Relaxed))
    .collect();

    usage
}

pub fn ram_usage() -> Vec<u64> {
  let ram_usage = RAM_USED.load(std::sync::atomic::Ordering::Relaxed);
  let total_ram = TOTAL_RAM.load(std::sync::atomic::Ordering::Relaxed);
  vec![ram_usage, total_ram]
}

pub fn top_10_downloaders() -> Vec<IpStatsWithPlan> {
  let tt = TOP_10_DOWNLOADERS.read().iter().map(|tt| tt.into()).collect();
  tt
}

pub fn worst_10_rtt() -> Vec<IpStatsWithPlan> {
  let tt = WORST_10_RTT.read().iter().map(|tt| tt.into()).collect();
  tt
}

pub fn rtt_histogram() -> Vec<u32> {
  let rtt = RTT_HISTOGRAM.read().clone();
  rtt
}

pub fn shaped_devices_count() -> u32 {
  let shaped_reader = SHAPED_DEVICES.read();
  let devices = shaped_reader.devices.len();
  devices as u32
}

pub fn shaped_devices() -> Vec<ShapedDevice> {
  let shaped_reader = SHAPED_DEVICES.read();
  let devices = shaped_reader.devices.clone();
  devices
}

pub fn unknown_hosts_count() -> u32 {
  let host_counts = HOST_COUNTS.read();
  let unknown = host_counts.0 - host_counts.1;
  unknown
}

pub fn unknown_hosts() -> Vec<IpStats> {
  let result = UNKNOWN_DEVICES.read().clone();
  result
}

pub fn busy_quantile() -> Vec<(u32, u32)> {
  let (down_capacity, up_capacity) = {
    let lock = CONFIG.lock();
    (
      lock.total_download_mbps as f64 * 1_000_000.0,
      lock.total_upload_mbps as f64 * 1_000_000.0,
    )
  };
  let throughput = THROUGHPUT_BUFFER.read().get_result();
  let mut result = vec![(0, 0); 10];
  throughput.iter().for_each(|tp| {
    let (down, up) = tp.bits_per_second;
    let (down, up) = (down * 8, up * 8);
    //println!("{down_capacity}, {up_capacity}, {down}, {up}");
    let (down, up) = (
      if down_capacity > 0.0 { down as f64 / down_capacity } else { 0.0 },
      if up_capacity > 0.0 { up as f64 / up_capacity } else { 0.0 },
    );
    let (down, up) = ((down * 10.0) as usize, (up * 10.0) as usize);
    result[usize::min(9, down)].0 += 1;
    result[usize::min(0, up)].1 += 1;
  });
  result
}

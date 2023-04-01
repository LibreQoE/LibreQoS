mod cache;
mod cache_manager;
use std::net::IpAddr;

use self::cache::{
  CPU_USAGE, NUM_CPUS, RAM_USED, TOTAL_RAM, THROUGHPUT_BUFFER
};
pub use cache::SHAPED_DEVICES;
pub use cache_manager::{update_tracking, update_total_throughput_buffer};
use lqos_bus::{bus_request, BusRequest, BusResponse, IpStats, TcHandle};
use lqos_config::ShapedDevice;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use axum::Json;

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
      circuit_id: i.circuit_id.clone(),
      plan: (0, 0),
    };

    if !result.circuit_id.is_empty() {
      if let Some(circuit) = SHAPED_DEVICES
        .read()
        .unwrap()
        .devices
        .iter()
        .find(|sd| sd.circuit_id == result.circuit_id)
      {
        let name = if circuit.circuit_name.len() > 20 {
          &circuit.circuit_name[0..20]
        } else {
          &circuit.circuit_name
        };
        result.ip_address = format!("{} ({})", name, result.ip_address);
        result.plan = (circuit.download_max_mbps, circuit.download_min_mbps);
      }
    }

    result
  }
}

/// Stores total system throughput per second.
#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct ThroughputPerSecond {
  pub bits_per_second: (u64, u64),
  pub packets_per_second: (u64, u64),
  pub shaped_bits_per_second: (u64, u64),
}

pub async fn current_throughput() -> ThroughputPerSecond {
  THROUGHPUT_BUFFER.read().await.current()
}

pub async fn throughput_ring() -> (usize, Vec<ThroughputPerSecond>) {
  THROUGHPUT_BUFFER.read().await.copy()
}

pub async fn cpu_usage() -> Vec<u32> {
  CPU_USAGE
    .iter()
    .take(NUM_CPUS.load(std::sync::atomic::Ordering::Relaxed))
    .map(|cpu| cpu.load(std::sync::atomic::Ordering::Relaxed))
    .collect()
}

pub async fn ram_usage() -> serde_json::Value {
  let ram_usage = RAM_USED.load(std::sync::atomic::Ordering::Relaxed);
  let total_ram = TOTAL_RAM.load(std::sync::atomic::Ordering::Relaxed);
  json!(vec![ram_usage, total_ram])
}

pub async fn top_10_downloaders() -> Vec<IpStatsWithPlan> {
  if let Ok(messages) = bus_request(vec![BusRequest::GetTopNDownloaders { start: 0, end: 10 }]).await {
    for msg in messages {
      if let BusResponse::TopDownloaders(stats) = msg {
        return stats.iter().map(|tt| tt.into()).collect();
      }
    }
  }
  Vec::new()
}

pub async fn worst_10_rtt() -> Vec<IpStatsWithPlan> {
  if let Ok(messages) = bus_request(vec![BusRequest::GetWorstRtt { start: 0, end: 10 }]).await {
    for msg in messages {
      if let BusResponse::WorstRtt(stats) = msg {
        return stats.iter().map(|tt| tt.into()).collect();
      }
    }
  }
  Vec::new()
}

pub async fn rtt_histogram() -> Vec<u32> {
  if let Ok(messages) = bus_request(vec![BusRequest::RttHistogram]).await {
    for msg in messages {
      if let BusResponse::RttHistogram(stats) = msg {
        return stats
      }
    }
  }
  Vec::new()
}

pub async fn shaped_devices_count() -> usize {
  shaped_devices().await.len()
}

pub async fn shaped_devices() -> Vec<ShapedDevice> {
  let shaped_reader = SHAPED_DEVICES.read().unwrap();
  shaped_reader.devices.clone()
}

pub async fn unknown_hosts_count() -> usize {
  unknown_hosts().await.len()
}

pub async fn unknown_hosts() -> Vec<IpStats> {
  if let Ok(messages) = bus_request(vec![BusRequest::AllUnknownIps]).await {
    for msg in messages {
      if let BusResponse::AllUnknownIps(unknowns) = msg {
        let result: Vec<IpStats> = unknowns
          .iter()
          .filter(|ip| {
            if let Ok(ip) = ip.ip_address.parse::<IpAddr>() {
              let lookup = match ip {
                IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                IpAddr::V6(ip) => ip,
              };
              SHAPED_DEVICES.read().unwrap().trie.longest_match(lookup).is_none()
            } else {
              false
            }
          })
          .cloned()
          .collect();
        return result
      }
    }
  }
  Vec::new()
}

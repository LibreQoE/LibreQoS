mod cache;
mod cache_manager;
use std::net::IpAddr;

use self::cache::{
  CPU_USAGE, NUM_CPUS, RAM_USED, TOTAL_RAM, THROUGHPUT_BUFFER,
};
use crate::{auth_guard::AuthGuard, cache_control::NoCache};
pub use cache::SHAPED_DEVICES;
pub use cache_manager::{update_tracking, update_total_throughput_buffer};
use lqos_bus::{bus_request, BusRequest, BusResponse, IpStats, TcHandle};
use rocket::serde::{Deserialize, Serialize, msgpack::MsgPack};
pub use cache::lookup_dns;

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
  pub tcp_retransmits: (u64, u64),
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
      tcp_retransmits: i.tcp_retransmits,
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
        result.plan = (circuit.download_max_mbps, circuit.upload_max_mbps);
      }
    }

    result
  }
}

/// Stores total system throughput per second.
#[derive(Debug, Clone, Copy, Serialize, Default)]
#[serde(crate = "rocket::serde")]
pub struct ThroughputPerSecond {
  pub bits_per_second: (u64, u64),
  pub packets_per_second: (u64, u64),
  pub shaped_bits_per_second: (u64, u64),
}

#[get("/api/current_throughput")]
pub async fn current_throughput(
  _auth: AuthGuard,
) -> NoCache<MsgPack<ThroughputPerSecond>> {
  let result = THROUGHPUT_BUFFER.current();
  NoCache::new(MsgPack(result))
}

#[get("/api/throughput_ring_buffer")]
pub async fn throughput_ring_buffer(
  _auth: AuthGuard,
) -> NoCache<MsgPack<(usize, Vec<ThroughputPerSecond>)>> {
  let result = THROUGHPUT_BUFFER.copy();
  NoCache::new(MsgPack(result))
}

#[get("/api/cpu")]
pub fn cpu_usage(_auth: AuthGuard) -> NoCache<MsgPack<Vec<u32>>> {
  let usage: Vec<u32> = CPU_USAGE
    .iter()
    .take(NUM_CPUS.load(std::sync::atomic::Ordering::Relaxed))
    .map(|cpu| cpu.load(std::sync::atomic::Ordering::Relaxed))
    .collect();

  NoCache::new(MsgPack(usage))
}

#[get("/api/ram")]
pub fn ram_usage(_auth: AuthGuard) -> NoCache<MsgPack<Vec<u64>>> {
  let ram_usage = RAM_USED.load(std::sync::atomic::Ordering::Relaxed);
  let total_ram = TOTAL_RAM.load(std::sync::atomic::Ordering::Relaxed);
  NoCache::new(MsgPack(vec![ram_usage, total_ram]))
}

#[get("/api/top_10_downloaders")]
pub async fn top_10_downloaders(_auth: AuthGuard) -> NoCache<MsgPack<Vec<IpStatsWithPlan>>> {
  if let Ok(messages) = bus_request(vec![BusRequest::GetTopNDownloaders { start: 0, end: 10 }]).await
  {
    for msg in messages {
      if let BusResponse::TopDownloaders(stats) = msg {
        let result = stats.iter().map(|tt| tt.into()).collect();
        return NoCache::new(MsgPack(result));
      }
    }
  }

  NoCache::new(MsgPack(Vec::new()))
}

#[get("/api/worst_10_rtt")]
pub async fn worst_10_rtt(_auth: AuthGuard) -> NoCache<MsgPack<Vec<IpStatsWithPlan>>> {
  if let Ok(messages) = bus_request(vec![BusRequest::GetWorstRtt { start: 0, end: 10 }]).await
  {
    for msg in messages {
      if let BusResponse::WorstRtt(stats) = msg {
        let result = stats.iter().map(|tt| tt.into()).collect();
        return NoCache::new(MsgPack(result));
      }
    }
  }

  NoCache::new(MsgPack(Vec::new()))
}

#[get("/api/worst_10_tcp")]
pub async fn worst_10_tcp(_auth: AuthGuard) -> NoCache<MsgPack<Vec<IpStatsWithPlan>>> {
  if let Ok(messages) = bus_request(vec![BusRequest::GetWorstRetransmits { start: 0, end: 10 }]).await
  {
    for msg in messages {
      if let BusResponse::WorstRetransmits(stats) = msg {
        let result = stats.iter().map(|tt| tt.into()).collect();
        return NoCache::new(MsgPack(result));
      }
    }
  }

  NoCache::new(MsgPack(Vec::new()))
}

#[get("/api/rtt_histogram")]
pub async fn rtt_histogram(_auth: AuthGuard) -> NoCache<MsgPack<Vec<u32>>> {
  if let Ok(messages) = bus_request(vec![BusRequest::RttHistogram]).await
  {
    for msg in messages {
      if let BusResponse::RttHistogram(stats) = msg {
        let result = stats;
        return NoCache::new(MsgPack(result));
      }
    }
  }

  NoCache::new(MsgPack(Vec::new()))
}

#[get("/api/host_counts")]
pub async fn host_counts(_auth: AuthGuard) -> NoCache<MsgPack<(u32, u32)>> {
  let mut host_counts = (0, 0);
  if let Ok(messages) = bus_request(vec![BusRequest::AllUnknownIps]).await {
    for msg in messages {
      if let BusResponse::AllUnknownIps(unknowns) = msg {
        let really_unknown: Vec<IpStats> = unknowns
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

          host_counts = (really_unknown.len() as u32, 0);
      }
    }
  }

  let n_devices = SHAPED_DEVICES.read().unwrap().devices.len();
  let unknown = host_counts.0 - host_counts.1;
  NoCache::new(MsgPack((n_devices as u32, unknown)))
}

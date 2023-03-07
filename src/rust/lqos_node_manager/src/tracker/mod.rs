mod cache;
mod cache_manager;
use self::cache::{
  CPU_USAGE, HOST_COUNTS, NUM_CPUS, RAM_USED, RTT_HISTOGRAM,
  TOP_10_DOWNLOADERS, TOTAL_RAM, WORST_10_RTT,
};
use crate::auth_guard::AuthGuard;
pub use cache::{SHAPED_DEVICES, UNKNOWN_DEVICES};
pub use cache_manager::update_tracking;
use lqos_bus::{bus_request, BusRequest, BusResponse, IpStats, TcHandle};
use rocket::serde::{json::Json, Deserialize, Serialize};

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
#[serde(crate = "rocket::serde")]
pub struct ThroughputPerSecond {
  pub bits_per_second: (u64, u64),
  pub packets_per_second: (u64, u64),
  pub shaped_bits_per_second: (u64, u64),
}

#[get("/api/current_throughput")]
pub async fn current_throughput(
  _auth: AuthGuard,
) -> Json<ThroughputPerSecond> {
  let mut result = ThroughputPerSecond::default();
  if let Ok(messages) =
    bus_request(vec![BusRequest::GetCurrentThroughput]).await
  {
    for msg in messages {
      if let BusResponse::CurrentThroughput {
        bits_per_second,
        packets_per_second,
        shaped_bits_per_second,
      } = msg
      {
        result.bits_per_second = bits_per_second;
        result.packets_per_second = packets_per_second;
        result.shaped_bits_per_second = shaped_bits_per_second;
      }
    }
  }
  Json(result)
}

#[get("/api/cpu")]
pub fn cpu_usage(_auth: AuthGuard) -> Json<Vec<u32>> {
  let usage: Vec<u32> = CPU_USAGE
    .iter()
    .take(NUM_CPUS.load(std::sync::atomic::Ordering::Relaxed))
    .map(|cpu| cpu.load(std::sync::atomic::Ordering::Relaxed))
    .collect();

  Json(usage)
}

#[get("/api/ram")]
pub fn ram_usage(_auth: AuthGuard) -> Json<Vec<u64>> {
  let ram_usage = RAM_USED.load(std::sync::atomic::Ordering::Relaxed);
  let total_ram = TOTAL_RAM.load(std::sync::atomic::Ordering::Relaxed);
  Json(vec![ram_usage, total_ram])
}

#[get("/api/top_10_downloaders")]
pub fn top_10_downloaders(_auth: AuthGuard) -> Json<Vec<IpStatsWithPlan>> {
  let tt: Vec<IpStatsWithPlan> =
    TOP_10_DOWNLOADERS.read().unwrap().iter().map(|tt| tt.into()).collect();
  Json(tt)
}

#[get("/api/worst_10_rtt")]
pub fn worst_10_rtt(_auth: AuthGuard) -> Json<Vec<IpStatsWithPlan>> {
  let tt: Vec<IpStatsWithPlan> =
    WORST_10_RTT.read().unwrap().iter().map(|tt| tt.into()).collect();
  Json(tt)
}

#[get("/api/rtt_histogram")]
pub fn rtt_histogram(_auth: AuthGuard) -> Json<Vec<u32>> {
  Json(RTT_HISTOGRAM.read().unwrap().clone())
}

#[get("/api/host_counts")]
pub fn host_counts(_auth: AuthGuard) -> Json<(u32, u32)> {
  let shaped_reader = SHAPED_DEVICES.read().unwrap();
  let n_devices = shaped_reader.devices.len();
  let host_counts = HOST_COUNTS.read().unwrap();
  let unknown = host_counts.0 - host_counts.1;
  Json((n_devices as u32, unknown))
}

//static CONFIG: Lazy<Mutex<LibreQoSConfig>> =
//  Lazy::new(|| Mutex::new(lqos_config::LibreQoSConfig::load().unwrap()));

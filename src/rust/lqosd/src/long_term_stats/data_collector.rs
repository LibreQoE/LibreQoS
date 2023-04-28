use crate::{throughput_tracker::THROUGHPUT_TRACKER, shaped_devices_tracker::SHAPED_DEVICES};
use once_cell::sync::Lazy;
use tokio::sync::Mutex;
use std::{
  net::IpAddr,
  sync::atomic::AtomicU64,
};

static SUBMISSION_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) struct StatsSession {
  pub(crate) bits_per_second: (u64, u64),
  pub(crate) packets_per_second: (u64, u64),
  pub(crate) shaped_bits_per_second: (u64, u64),
  pub(crate) hosts: Vec<SessionHost>,
}

pub(crate) struct SessionHost {
  pub(crate) circuit_id: String,
  pub(crate) ip_address: IpAddr,
  pub(crate) bits_per_second: (u64, u64),
  pub(crate) median_rtt: f32,
  pub(crate) tree_parent_indices: Vec<usize>,
  pub(crate) device_id: String,
  pub(crate) parent_node: String,
  pub(crate) circuit_name: String,
  pub(crate) device_name: String,
  pub(crate) mac: String,
}

pub(crate) static SESSION_BUFFER: Lazy<Mutex<Vec<StatsSession>>> =
  Lazy::new(|| Mutex::new(Vec::new()));

pub(crate) async fn gather_throughput_stats() {
  let count =
    SUBMISSION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
  if count < 5 {
    // Ignore the first few sets of data
    return;
  }

  // Gather Global Stats
  let packets_per_second = (
    THROUGHPUT_TRACKER
      .packets_per_second
      .0
      .load(std::sync::atomic::Ordering::Relaxed),
    THROUGHPUT_TRACKER
      .packets_per_second
      .1
      .load(std::sync::atomic::Ordering::Relaxed),
  );
  let bits_per_second = THROUGHPUT_TRACKER.bits_per_second();
  let shaped_bits_per_second = THROUGHPUT_TRACKER.shaped_bits_per_second();

  let mut session = StatsSession {
    bits_per_second,
    shaped_bits_per_second,
    packets_per_second,
    hosts: Vec::with_capacity(THROUGHPUT_TRACKER.raw_data.len()),
  };  

  THROUGHPUT_TRACKER
    .raw_data
    .iter()
    .for_each(|tp| {
      let shaped_devices = SHAPED_DEVICES.read().unwrap();
      let mut circuit_id = String::new();
      let mut device_id = tp.key().as_ip().to_string();
      let mut parent_node = String::new();
      let mut circuit_name = String::new();
      let mut device_name = String::new();
      let mut mac = String::new();
      let ip = tp.key().as_ip();
      let lookup = match ip {
        IpAddr::V4(ip) => ip.to_ipv6_mapped(),
        IpAddr::V6(ip) => ip,
      };
      if let Some((_, index)) = shaped_devices.trie.longest_match(lookup) {
        let shaped_device = &shaped_devices.devices[*index];
        circuit_id = shaped_device.circuit_id.clone();
        device_id = shaped_device.device_id.clone();
        parent_node = shaped_device.parent_node.clone();
        circuit_name = shaped_device.circuit_name.clone();
        device_name = shaped_device.device_name.clone();
        mac = shaped_device.mac.clone();
      }

      let bytes_per_second = tp.bytes_per_second;
      let bits_per_second = (bytes_per_second.0 * 8, bytes_per_second.1 * 8);
      session.hosts.push(SessionHost {
        circuit_id,
        ip_address: tp.key().as_ip(),
        bits_per_second,
        median_rtt: tp.median_latency(),
        tree_parent_indices: tp.network_json_parents.clone().unwrap_or(Vec::new()),
        device_id,
        parent_node,
        circuit_name,
        device_name,
        mac,
      });
    });

  SESSION_BUFFER.lock().await.push(session);
}

mod throughput_entry;
mod tracking_data;
mod heimdall_data;
pub use heimdall_data::get_flow_stats;
use crate::{
  shaped_devices_tracker::NETWORK_JSON,
  throughput_tracker::tracking_data::ThroughputTracker, stats::TIME_TO_POLL_HOSTS,
};
use log::{info, warn};
use lqos_bus::{BusResponse, IpStats, TcHandle, XdpPpingResult};
use lqos_utils::{fdtimer::periodic, unix_time::time_since_boot, XdpIpAddress};
use once_cell::sync::Lazy;
use std::time::Duration;

const RETIRE_AFTER_SECONDS: u64 = 30;

pub static THROUGHPUT_TRACKER: Lazy<ThroughputTracker> =
  Lazy::new(ThroughputTracker::new);

pub fn spawn_throughput_monitor() {
  info!("Starting the bandwidth monitor thread.");
  let interval_ms = 1000; // 1 second
  info!("Bandwidth check period set to {interval_ms} ms.");

  std::thread::spawn(move || {
    periodic(interval_ms, "Throughput Monitor", &mut || {
      let start = std::time::Instant::now();
      {
        let net_json = NETWORK_JSON.read().unwrap();
        net_json.zero_throughput_and_rtt();
      } // Scope to end the lock
      THROUGHPUT_TRACKER.copy_previous_and_reset_rtt();
      THROUGHPUT_TRACKER.apply_new_throughput_counters();
      THROUGHPUT_TRACKER.apply_rtt_data();
      THROUGHPUT_TRACKER.update_totals();
      THROUGHPUT_TRACKER.next_cycle();
      let duration_ms = start.elapsed().as_micros();
      TIME_TO_POLL_HOSTS.store(duration_ms as u64, std::sync::atomic::Ordering::Relaxed);
    });
  });
}

pub fn current_throughput() -> BusResponse {
  let (bits_per_second, packets_per_second, shaped_bits_per_second) = {
    (
      THROUGHPUT_TRACKER.bits_per_second(),
      THROUGHPUT_TRACKER.packets_per_second(),
      THROUGHPUT_TRACKER.shaped_bits_per_second(),
    )
  };
  BusResponse::CurrentThroughput {
    bits_per_second,
    packets_per_second,
    shaped_bits_per_second,
  }
}

pub fn host_counters() -> BusResponse {
  let mut result = Vec::new();
  THROUGHPUT_TRACKER.raw_data.iter().for_each(|v| {
    let ip = v.key().as_ip();
    let (down, up) = v.bytes_per_second;
    result.push((ip, down, up));
  });
  BusResponse::HostCounters(result)
}

#[inline(always)]
fn retire_check(cycle: u64, recent_cycle: u64) -> bool {
  cycle < recent_cycle + RETIRE_AFTER_SECONDS
}

type TopList = (XdpIpAddress, (u64, u64), (u64, u64), f32, TcHandle, String);

pub fn top_n(start: u32, end: u32) -> BusResponse {
  let mut full_list: Vec<TopList> = {
    let tp_cycle = THROUGHPUT_TRACKER.cycle.load(std::sync::atomic::Ordering::Relaxed);
    THROUGHPUT_TRACKER.raw_data
      .iter()
      .filter(|v| !v.key().as_ip().is_loopback())
      .filter(|d| retire_check(tp_cycle, d.most_recent_cycle))
      .map(|te| {
        (
          *te.key(),
          te.bytes_per_second,
          te.packets_per_second,
          te.median_latency().unwrap_or(0.0),
          te.tc_handle,
          te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
        )
      })
      .collect()
  };
  full_list.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
  let result = full_list
    .iter()
    .skip(start as usize)
    .take((end as usize) - (start as usize))
    .map(
      |(
        ip,
        (bytes_dn, bytes_up),
        (packets_dn, packets_up),
        median_rtt,
        tc_handle,
        circuit_id,
      )| IpStats {
        ip_address: ip.as_ip().to_string(),
        circuit_id: circuit_id.clone(),
        bits_per_second: (bytes_dn * 8, bytes_up * 8),
        packets_per_second: (*packets_dn, *packets_up),
        median_tcp_rtt: *median_rtt,
        tc_handle: *tc_handle,
      },
    )
    .collect();
  BusResponse::TopDownloaders(result)
}

pub fn worst_n(start: u32, end: u32) -> BusResponse {
  let mut full_list: Vec<TopList> = {
    let tp_cycle = THROUGHPUT_TRACKER.cycle.load(std::sync::atomic::Ordering::Relaxed);
    THROUGHPUT_TRACKER.raw_data
      .iter()
      .filter(|v| !v.key().as_ip().is_loopback())
      .filter(|d| retire_check(tp_cycle, d.most_recent_cycle))
      .filter(|te| te.median_latency().is_some())
      .map(|te| {
        (
          *te.key(),
          te.bytes_per_second,
          te.packets_per_second,
          te.median_latency().unwrap_or(0.0),
          te.tc_handle,
          te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
        )
      })
      .collect()
  };
  full_list.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
  let result = full_list
    .iter()
    .skip(start as usize)
    .take((end as usize) - (start as usize))
    .map(
      |(
        ip,
        (bytes_dn, bytes_up),
        (packets_dn, packets_up),
        median_rtt,
        tc_handle,
        circuit_id,
      )| IpStats {
        ip_address: ip.as_ip().to_string(),
        circuit_id: circuit_id.clone(),
        bits_per_second: (bytes_dn * 8, bytes_up * 8),
        packets_per_second: (*packets_dn, *packets_up),
        median_tcp_rtt: *median_rtt,
        tc_handle: *tc_handle,
      },
    )
    .collect();
  BusResponse::WorstRtt(result)
}

pub fn best_n(start: u32, end: u32) -> BusResponse {
  let mut full_list: Vec<TopList> = {
    let tp_cycle = THROUGHPUT_TRACKER.cycle.load(std::sync::atomic::Ordering::Relaxed);
    THROUGHPUT_TRACKER.raw_data
      .iter()
      .filter(|v| !v.key().as_ip().is_loopback())
      .filter(|d| retire_check(tp_cycle, d.most_recent_cycle))
      .filter(|te| te.median_latency().is_some())
      .map(|te| {
        (
          *te.key(),
          te.bytes_per_second,
          te.packets_per_second,
          te.median_latency().unwrap_or(0.0),
          te.tc_handle,
          te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
        )
      })
      .collect()
  };
  full_list.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
  full_list.reverse();
  let result = full_list
    .iter()
    .skip(start as usize)
    .take((end as usize) - (start as usize))
    .map(
      |(
        ip,
        (bytes_dn, bytes_up),
        (packets_dn, packets_up),
        median_rtt,
        tc_handle,
        circuit_id,
      )| IpStats {
        ip_address: ip.as_ip().to_string(),
        circuit_id: circuit_id.clone(),
        bits_per_second: (bytes_dn * 8, bytes_up * 8),
        packets_per_second: (*packets_dn, *packets_up),
        median_tcp_rtt: *median_rtt,
        tc_handle: *tc_handle,
      },
    )
    .collect();
  BusResponse::BestRtt(result)
}

pub fn xdp_pping_compat() -> BusResponse {
  let raw_cycle = THROUGHPUT_TRACKER.cycle.load(std::sync::atomic::Ordering::Relaxed);
  let result = THROUGHPUT_TRACKER
    .raw_data
    .iter()
    .filter(|d| retire_check(raw_cycle, d.most_recent_cycle))
    .filter_map(|data| {
      if data.tc_handle.as_u32() > 0 {
        let mut valid_samples: Vec<u32> =
          data.recent_rtt_data.iter().filter(|d| **d > 0).copied().collect();
        let samples = valid_samples.len() as u32;
        if samples > 0 {
          valid_samples.sort_by(|a, b| (*a).cmp(b));
          let median = valid_samples[valid_samples.len() / 2] as f32 / 100.0;
          let max = *(valid_samples.iter().max().unwrap()) as f32 / 100.0;
          let min = *(valid_samples.iter().min().unwrap()) as f32 / 100.0;
          let sum = valid_samples.iter().sum::<u32>() as f32 / 100.0;
          let avg = sum / samples as f32;

          Some(XdpPpingResult {
            tc: data.tc_handle.to_string(),
            median,
            avg,
            max,
            min,
            samples,
          })
        } else {
          None
        }
      } else {
        None
      }
    })
    .collect();
  BusResponse::XdpPping(result)
}

pub fn rtt_histogram() -> BusResponse {
  let mut result = vec![0; 20];
  let reader_cycle = THROUGHPUT_TRACKER.cycle.load(std::sync::atomic::Ordering::Relaxed);
  for data in THROUGHPUT_TRACKER
    .raw_data
    .iter()
    .filter(|d| retire_check(reader_cycle, d.most_recent_cycle))
  {
    let valid_samples: Vec<u32> =
      data.recent_rtt_data.iter().filter(|d| **d > 0).copied().collect();
    let samples = valid_samples.len() as u32;
    if samples > 0 {
      let median = valid_samples[valid_samples.len() / 2] as f32 / 100.0;
      let median = f32::min(200.0, median);
      let column = (median / 10.0) as usize;
      result[usize::min(column, 19)] += 1;
    }
  }

  BusResponse::RttHistogram(result)
}

pub fn host_counts() -> BusResponse {
  let mut total = 0;
  let mut shaped = 0;
  let tp_cycle = THROUGHPUT_TRACKER.cycle.load(std::sync::atomic::Ordering::Relaxed);
  THROUGHPUT_TRACKER.raw_data
    .iter()
    .filter(|d| retire_check(tp_cycle, d.most_recent_cycle))
    .for_each(|d| {
      total += 1;
      if d.tc_handle.as_u32() != 0 {
        shaped += 1;
      }
    });
  BusResponse::HostCounts((total, shaped))
}

type FullList = (XdpIpAddress, (u64, u64), (u64, u64), f32, TcHandle, u64);

pub fn all_unknown_ips() -> BusResponse {
  let boot_time = time_since_boot();
  if boot_time.is_err() {
    warn!("The Linux system clock isn't available to provide time since boot, yet.");
    warn!("This only happens immediately after a reboot.");
    return BusResponse::NotReadyYet;
  }
  let boot_time = boot_time.unwrap();
  let time_since_boot = Duration::from(boot_time);
  let five_minutes_ago =
    time_since_boot.saturating_sub(Duration::from_secs(300));
  let five_minutes_ago_nanoseconds = five_minutes_ago.as_nanos();

  let mut full_list: Vec<FullList> = {
    THROUGHPUT_TRACKER.raw_data
      .iter()
      .filter(|v| !v.key().as_ip().is_loopback())
      .filter(|d| d.tc_handle.as_u32() == 0)
      .filter(|d| d.last_seen as u128 > five_minutes_ago_nanoseconds)
      .map(|te| {
        (
          *te.key(),
          te.bytes,
          te.packets,
          te.median_latency().unwrap_or(0.0),
          te.tc_handle,
          te.most_recent_cycle,
        )
      })
      .collect()
  };
  full_list.sort_by(|a, b| b.5.partial_cmp(&a.5).unwrap());
  let result = full_list
    .iter()
    .map(
      |(
        ip,
        (bytes_dn, bytes_up),
        (packets_dn, packets_up),
        median_rtt,
        tc_handle,
        _last_seen,
      )| IpStats {
        ip_address: ip.as_ip().to_string(),
        circuit_id: String::new(),
        bits_per_second: (bytes_dn * 8, bytes_up * 8),
        packets_per_second: (*packets_dn, *packets_up),
        median_tcp_rtt: *median_rtt,
        tc_handle: *tc_handle,
      },
    )
    .collect();
  BusResponse::AllUnknownIps(result)
}

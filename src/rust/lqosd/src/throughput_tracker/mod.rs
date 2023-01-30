mod throughput_entry;
mod tracking_data;
use crate::throughput_tracker::tracking_data::ThroughputTracker;
use lazy_static::*;
use lqos_bus::{BusResponse, IpStats, TcHandle, XdpPpingResult};
use lqos_sys::XdpIpAddress;
use nix::sys::{timerfd::{TimerFd, ClockId, TimerFlags, Expiration, TimerSetTimeFlags}, time::{TimeSpec, TimeValLike}};
use parking_lot::RwLock;
use std::{time::Duration, sync::atomic::AtomicBool};
use log::{info, warn, error};

const RETIRE_AFTER_SECONDS: u64 = 30;

lazy_static! {
  static ref THROUGHPUT_TRACKER: RwLock<ThroughputTracker> =
    RwLock::new(ThroughputTracker::new());
}

pub fn spawn_throughput_monitor() {
  info!("Starting the bandwidth monitor thread.");
  let interval_ms = 1000; // 1 second
  info!("Bandwidth check period set to {interval_ms} ms.");

  std::thread::spawn(move || {
    let monitor_busy = AtomicBool::new(false);
    if let Ok(timer) = TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::empty()) {
      if timer.set(Expiration::Interval(TimeSpec::milliseconds(interval_ms as i64)), TimerSetTimeFlags::TFD_TIMER_ABSTIME).is_ok() {
          loop {
              if timer.wait().is_ok() {
                  if monitor_busy.load(std::sync::atomic::Ordering::Relaxed) {
                      warn!("Queue tick fired while another queue read is ongoing. Skipping this cycle.");
                  } else {
                      monitor_busy.store(true, std::sync::atomic::Ordering::Relaxed);
                      //info!("Bandwidth tracking timer fired.");
                      let mut throughput = THROUGHPUT_TRACKER.write();
                      throughput.copy_previous_and_reset_rtt();
                      throughput.apply_new_throughput_counters();
                      throughput.apply_rtt_data();
                      throughput.update_totals();
                      throughput.next_cycle();
                      monitor_busy.store(false, std::sync::atomic::Ordering::Relaxed);
                  }
              } else {
                  error!("Error in timer wait (Linux fdtimer). This should never happen.");
              }
          }
      } else {
          error!("Unable to set the Linux fdtimer timer interval. Bandwidth will not be monitored.");
      }
  } else {
      error!("Unable to acquire Linux fdtimer. Bandwidth will not be monitored.");
  }
  });
}

pub fn current_throughput() -> BusResponse {
  let (bits_per_second, packets_per_second, shaped_bits_per_second) = {
    let tp = THROUGHPUT_TRACKER.read();
    (
      tp.bits_per_second(),
      tp.packets_per_second(),
      tp.shaped_bits_per_second(),
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
  let tp = THROUGHPUT_TRACKER.read();
  tp.raw_data.iter().for_each(|(k, v)| {
    let ip = k.as_ip();
    let (down, up) = v.bytes_per_second;
    result.push((ip, down, up));
  });
  BusResponse::HostCounters(result)
}

#[inline(always)]
fn retire_check(cycle: u64, recent_cycle: u64) -> bool {
  cycle < recent_cycle + RETIRE_AFTER_SECONDS
}

pub fn top_n(start: u32, end: u32) -> BusResponse {
  let mut full_list: Vec<(
    XdpIpAddress,
    (u64, u64),
    (u64, u64),
    f32,
    TcHandle,
  )> = {
    let tp = THROUGHPUT_TRACKER.read();
    tp.raw_data
      .iter()
      .filter(|(ip, _)| !ip.as_ip().is_loopback())
      .filter(|(_, d)| retire_check(tp.cycle, d.most_recent_cycle))
      .map(|(ip, te)| {
        (
          *ip,
          te.bytes_per_second,
          te.packets_per_second,
          te.median_latency(),
          te.tc_handle,
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
      )| IpStats {
        ip_address: ip.as_ip().to_string(),
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
  let mut full_list: Vec<(
    XdpIpAddress,
    (u64, u64),
    (u64, u64),
    f32,
    TcHandle,
  )> = {
    let tp = THROUGHPUT_TRACKER.read();
    tp.raw_data
      .iter()
      .filter(|(ip, _)| !ip.as_ip().is_loopback())
      .filter(|(_, d)| retire_check(tp.cycle, d.most_recent_cycle))
      .map(|(ip, te)| {
        (
          *ip,
          te.bytes_per_second,
          te.packets_per_second,
          te.median_latency(),
          te.tc_handle,
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
      )| IpStats {
        ip_address: ip.as_ip().to_string(),
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
  let mut full_list: Vec<(
    XdpIpAddress,
    (u64, u64),
    (u64, u64),
    f32,
    TcHandle,
  )> = {
    let tp = THROUGHPUT_TRACKER.read();
    tp.raw_data
      .iter()
      .filter(|(ip, _)| !ip.as_ip().is_loopback())
      .filter(|(_, d)| retire_check(tp.cycle, d.most_recent_cycle))
      .map(|(ip, te)| {
        (
          *ip,
          te.bytes_per_second,
          te.packets_per_second,
          te.median_latency(),
          te.tc_handle,
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
      )| IpStats {
        ip_address: ip.as_ip().to_string(),
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
  let raw = THROUGHPUT_TRACKER.read();
  let result = raw
    .raw_data
    .iter()
    .filter(|(_, d)| retire_check(raw.cycle, d.most_recent_cycle))
    .filter_map(|(_ip, data)| {
      if data.tc_handle.as_u32() > 0 {
        let mut valid_samples: Vec<u32> = data
          .recent_rtt_data
          .iter()
          .filter(|d| **d > 0)
          .map(|d| *d)
          .collect();
        let samples = valid_samples.len() as u32;
        if samples > 0 {
          valid_samples.sort_by(|a, b| (*a).cmp(&b));
          let median = valid_samples[valid_samples.len() / 2] as f32 / 100.0;
          let max = *(valid_samples.iter().max().unwrap()) as f32 / 100.0;
          let min = *(valid_samples.iter().min().unwrap()) as f32 / 100.0;
          let sum = valid_samples.iter().sum::<u32>() as f32 / 100.0;
          let avg = sum / samples as f32;

          Some(XdpPpingResult {
            tc: format!("{}", data.tc_handle.to_string()),
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
  let reader = THROUGHPUT_TRACKER.read();
  for (_, data) in reader
    .raw_data
    .iter()
    .filter(|(_, d)| retire_check(reader.cycle, d.most_recent_cycle))
  {
    let valid_samples: Vec<u32> =
      data.recent_rtt_data.iter().filter(|d| **d > 0).map(|d| *d).collect();
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
  let tp = THROUGHPUT_TRACKER.read();
  tp.raw_data
    .iter()
    .filter(|(_, d)| retire_check(tp.cycle, d.most_recent_cycle))
    .for_each(|(_, d)| {
      total += 1;
      if d.tc_handle.as_u32() != 0 {
        shaped += 1;
      }
    });
  BusResponse::HostCounts((total, shaped))
}

pub fn all_unknown_ips() -> BusResponse {
  let boot_time =
    nix::time::clock_gettime(nix::time::ClockId::CLOCK_BOOTTIME)
      .expect("Unable to obtain kernel time.");
  let time_since_boot = Duration::from(boot_time);
  let five_minutes_ago = time_since_boot - Duration::from_secs(300);
  let five_minutes_ago_nanoseconds = five_minutes_ago.as_nanos();

  let mut full_list: Vec<(
    XdpIpAddress,
    (u64, u64),
    (u64, u64),
    f32,
    TcHandle,
    u64,
  )> = {
    let tp = THROUGHPUT_TRACKER.read();
    tp.raw_data
      .iter()
      .filter(|(ip, _)| !ip.as_ip().is_loopback())
      .filter(|(_, d)| d.tc_handle.as_u32() == 0)
      .filter(|(_, d)| d.last_seen as u128 > five_minutes_ago_nanoseconds)
      .map(|(ip, te)| {
        (
          *ip,
          te.bytes,
          te.packets,
          te.median_latency(),
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
        bits_per_second: (bytes_dn * 8, bytes_up * 8),
        packets_per_second: (*packets_dn, *packets_up),
        median_tcp_rtt: *median_rtt,
        tc_handle: *tc_handle,
      },
    )
    .collect();
  BusResponse::AllUnknownIps(result)
}

//! The Cache mod stores data that is periodically updated
//! on the server-side, to avoid re-requesting repeatedly
//! when there are multiple clients.
use super::cache::*;
use anyhow::Result;
use lqos_bus::{bus_request, BusRequest, BusResponse, IpStats};
use lqos_config::ConfigShapedDevices;
use lqos_utils::file_watcher::FileWatcher;
use nix::sys::{
  time::{TimeSpec, TimeValLike},
  timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags},
};
use rocket::tokio::task::spawn_blocking;
use std::{net::IpAddr, sync::atomic::AtomicBool};

/// Once per second, update CPU and RAM usage and ask
/// `lqosd` for updated system statistics.
/// Called from the main program as a "fairing", meaning
/// it runs as part of start-up - and keeps running.
/// Designed to never return or fail on error.
pub async fn update_tracking() {
  use sysinfo::CpuExt;
  use sysinfo::System;
  use sysinfo::SystemExt;
  let mut sys = System::new_all();

  spawn_blocking(|| {
    info!("Watching for ShapedDevices.csv changes");
    let _ = watch_for_shaped_devices_changing();
  });
  let interval_ms = 1000;
  info!("Updating throughput ring buffer at {interval_ms} ms cadence.");

  let monitor_busy = AtomicBool::new(false);
  if let Ok(timer) =
    TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::empty())
  {
    if timer
      .set(
        Expiration::Interval(TimeSpec::milliseconds(interval_ms as i64)),
        TimerSetTimeFlags::TFD_TIMER_ABSTIME,
      )
      .is_ok()
    {
      loop {
        if timer.wait().is_ok() {
          if monitor_busy.load(std::sync::atomic::Ordering::Relaxed) {
            warn!("Ring buffer tick fired while another queue read is ongoing. Skipping this cycle.");
          } else {
            monitor_busy.store(true, std::sync::atomic::Ordering::Relaxed);
            //info!("Queue tracking timer fired.");

            sys.refresh_cpu();
            sys.refresh_memory();
            let cpu_usage = sys
              .cpus()
              .iter()
              .map(|cpu| cpu.cpu_usage())
              .collect::<Vec<f32>>();
            *CPU_USAGE.write() = cpu_usage;
            {
              let mut mem_use = MEMORY_USAGE.write();
              mem_use[0] = sys.used_memory();
              mem_use[1] = sys.total_memory();
            }
            let error = get_data_from_server().await; // Ignoring errors to keep running
            if let Err(error) = error {
              error!("Error in usage update loop: {:?}", error);
            }

            monitor_busy.store(false, std::sync::atomic::Ordering::Relaxed);
          }
        } else {
          error!(
            "Error in timer wait (Linux fdtimer). This should never happen."
          );
        }
      }
    } else {
      error!("Unable to set the Linux fdtimer timer interval. Queues will not be monitored.");
    }
  } else {
    error!("Unable to acquire Linux fdtimer. Queues will not be monitored.");
  }
}

fn load_shaped_devices() {
  let shaped_devices = ConfigShapedDevices::load();
  if let Ok(new_file) = shaped_devices {
    info!("ShapedDevices.csv loaded");
    *SHAPED_DEVICES.write() = new_file;
  }
}

/// Fires up a Linux file system watcher than notifies
/// when `ShapedDevices.csv` changes, and triggers a reload.
fn watch_for_shaped_devices_changing() -> Result<()> {
  let watch_path = ConfigShapedDevices::path();
  if watch_path.is_err() {
    error!("Unable to generate path for ShapedDevices.csv");
    return Err(anyhow::Error::msg(
      "Unable to create path for ShapedDevices.csv",
    ));
  }
  let watch_path = watch_path.unwrap();

  let mut watcher = FileWatcher::new("ShapedDevices.csv", watch_path);
  watcher.set_file_exists_callback(load_shaped_devices);
  watcher.set_file_created_callback(load_shaped_devices);
  watcher.set_file_changed_callback(load_shaped_devices);
  let _ = watcher.watch();
  Ok(())
}

/// Requests data from `lqosd` and stores it in local
/// caches.
async fn get_data_from_server() -> Result<()> {
  // Send request to lqosd
  let requests = vec![
    BusRequest::GetCurrentThroughput,
    BusRequest::GetTopNDownloaders { start: 0, end: 10 },
    BusRequest::GetWorstRtt { start: 0, end: 10 },
    BusRequest::RttHistogram,
    BusRequest::AllUnknownIps,
  ];

  for r in bus_request(requests).await?.iter() {
    match r {
      BusResponse::CurrentThroughput {
        bits_per_second,
        packets_per_second,
        shaped_bits_per_second,
      } => {
        {
          let mut lock = CURRENT_THROUGHPUT.write();
          lock.bits_per_second = *bits_per_second;
          lock.packets_per_second = *packets_per_second;
        } // Lock scope
        {
          let mut lock = THROUGHPUT_BUFFER.write();
          lock.store(ThroughputPerSecond {
            packets_per_second: *packets_per_second,
            bits_per_second: *bits_per_second,
            shaped_bits_per_second: *shaped_bits_per_second,
          });
        }
      }
      BusResponse::TopDownloaders(stats) => {
        *TOP_10_DOWNLOADERS.write() = stats.clone();
      }
      BusResponse::WorstRtt(stats) => {
        *WORST_10_RTT.write() = stats.clone();
      }
      BusResponse::RttHistogram(stats) => {
        *RTT_HISTOGRAM.write() = stats.clone();
      }
      BusResponse::AllUnknownIps(unknowns) => {
        *HOST_COUNTS.write() = (unknowns.len() as u32, 0);
        let cfg = SHAPED_DEVICES.read();
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
        *HOST_COUNTS.write() = (really_unknown.len() as u32, 0);
        *UNKNOWN_DEVICES.write() = really_unknown;
      }
      // Default
      _ => {}
    }
  }

  Ok(())
}

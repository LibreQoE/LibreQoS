//! The Cache mod stores data that is periodically updated
//! on the server-side, to avoid re-requesting repeatedly
//! when there are multiple clients.
use super::cache::*;
use anyhow::Result;
use lqos_config::ConfigShapedDevices;
use lqos_utils::file_watcher::FileWatcher;
use nix::sys::{
  time::{TimeSpec, TimeValLike},
  timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags},
};
use rocket::tokio::{task::spawn_blocking, time::Instant};
use std::{sync::atomic::AtomicBool, time::Duration};

/// Once per second, update CPU and RAM usage and ask
/// `lqosd` for updated system statistics.
/// Called from the main program as a "fairing", meaning
/// it runs as part of start-up - and keeps running.
/// Designed to never return or fail on error.
pub async fn update_tracking() {
  use sysinfo::System;
  let mut sys = System::new_all();

  spawn_blocking(|| {
    info!("Watching for ShapedDevices.csv changes");
    let _ = watch_for_shaped_devices_changing();
  });
  let interval_ms = 1000;
  info!("Updating throughput ring buffer at {interval_ms} ms cadence.");

  std::thread::sleep(std::time::Duration::from_secs(10));
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

            sys
              .cpus()
              .iter()
              .enumerate()
              .map(|(i, cpu)| (i, cpu.cpu_usage() as u32)) // Always rounds down
              .for_each(|(i, cpu)| {
                CPU_USAGE[i].store(cpu, std::sync::atomic::Ordering::Relaxed)
              });

            NUM_CPUS
              .store(sys.cpus().len(), std::sync::atomic::Ordering::Relaxed);
            RAM_USED
              .store(sys.used_memory(), std::sync::atomic::Ordering::Relaxed);
            TOTAL_RAM
              .store(sys.total_memory(), std::sync::atomic::Ordering::Relaxed);

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
  info!("ShapedDevices.csv has changed. Attempting to load it.");
  let shaped_devices = ConfigShapedDevices::load();
  if let Ok(new_file) = shaped_devices {
    info!("ShapedDevices.csv loaded");
    *SHAPED_DEVICES.write().unwrap() = new_file;
  } else {
    warn!("ShapedDevices.csv failed to load, see previous error messages. Reverting to empty set.");
    *SHAPED_DEVICES.write().unwrap() = ConfigShapedDevices::default();
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
  loop {
    let result = watcher.watch();
    info!("ShapedDevices watcher returned: {result:?}");
  }
}

/// Fires once per second and updates the global traffic ringbuffer.
pub async fn update_total_throughput_buffer() {
  loop {
    let now = Instant::now();
    THROUGHPUT_BUFFER.tick().await;
    let elapsed = now.elapsed();
    if elapsed < Duration::from_secs(1) {
      rocket::tokio::time::sleep(Duration::from_secs(1) - elapsed).await;
    }
  }
}
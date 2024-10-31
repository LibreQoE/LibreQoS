//! Provides an interface to the Heimdall packet watching
//! system. Heimdall watches traffic flows, and is notified
//! about their contents via the eBPF Perf system.
#![warn(missing_docs)]
mod config;
/// Interface to the performance tracking system
pub mod perf_interface;
pub mod stats;

use std::time::Duration;
use tracing::{debug, error, warn};
use timerfd::{SetTimeFlags, TimerFd, TimerState};
pub use config::{HeimdalConfig, HeimdallMode};
mod timeline;
pub use timeline::{n_second_packet_dump, n_second_pcap, hyperfocus_on_target};
mod pcap;
mod watchlist;
pub use watchlist::{heimdall_expire, heimdall_watch_ip, set_heimdall_mode};
use anyhow::Result;

use crate::timeline::expire_timeline;

/// How long should Heimdall keep watching a flow after being requested
/// to do so? Setting this to a long period increases CPU load after the
/// client has stopped looking. Too short a delay will lead to missed
/// collections if the client hasn't maintained the 1s request cadence.
const EXPIRE_WATCHES_SECS: u64 = 5;

/// How long should Heimdall retain packet timeline data?
const TIMELINE_EXPIRE_SECS: u64 = 10;

/// How long should an analysis session remain in memory?
const SESSION_EXPIRE_SECONDS: u64 = 600;

/// Interface to running Heimdall (start this when lqosd starts)
pub fn start_heimdall() -> Result<()> {
  if set_heimdall_mode(HeimdallMode::WatchOnly).is_err() {
    error!(
      "Unable to set Heimdall Mode. Packet watching will be unavailable."
    );
    anyhow::bail!("Unable to set Heimdall Mode.");
  }

  let interval_ms = 1000; // 1 second
  debug!("Heimdall check period set to {interval_ms} ms.");

  std::thread::Builder::new()
        .name("Heimdall Packet Watcher".to_string())
        .spawn(move || {
    let mut tfd = TimerFd::new().unwrap();
    assert_eq!(tfd.get_state(), TimerState::Disarmed);
    tfd.set_state(TimerState::Periodic{
      current: Duration::from_millis(interval_ms),
      interval: Duration::from_millis(interval_ms) }
                  , SetTimeFlags::Default
    );

    loop {
      heimdall_expire();
      expire_timeline();

      let missed_ticks = tfd.read();
      if missed_ticks > 1 {
        warn!("Heimdall Missed {} ticks", missed_ticks - 1);
      }
    }
  })?;

  Ok(())
}

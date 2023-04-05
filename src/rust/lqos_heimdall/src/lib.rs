//! Provides an interface to the Heimdall packet watching
//! system. Heimdall watches traffic flows, and is notified
//! about their contents via the eBPF Perf system.
#![warn(missing_docs)]
mod config;
/// Interface to the performance tracking system
pub mod perf_interface;
pub mod stats;
pub use config::{HeimdalConfig, HeimdallMode};
mod flows;
pub use flows::{expire_heimdall_flows, get_flow_stats};
mod timeline;
pub use timeline::{n_second_packet_dump, n_second_pcap, hyperfocus_on_target};
mod pcap;
mod watchlist;
use lqos_utils::fdtimer::periodic;
pub use watchlist::{heimdall_expire, heimdall_watch_ip, set_heimdall_mode};

use crate::flows::read_flows;

/// How long should Heimdall keep watching a flow after being requested
/// to do so? Setting this to a long period increases CPU load after the
/// client has stopped looking. Too short a delay will lead to missed
/// collections if the client hasn't maintained the 1s request cadence.
const EXPIRE_WATCHES_SECS: u64 = 5;

/// How long should Heimdall retain flow summary data?
const FLOW_EXPIRE_SECS: u64 = 10;

/// How long should Heimdall retain packet timeline data?
const TIMELINE_EXPIRE_SECS: u64 = 10;

/// How long should an analysis session remain in memory?
const SESSION_EXPIRE_SECONDS: u64 = 600;

/// Interface to running Heimdall (start this when lqosd starts)
/// This is async to match the other spawning systems.
pub async fn start_heimdall() {
  if set_heimdall_mode(HeimdallMode::WatchOnly).is_err() {
    log::error!(
      "Unable to set Heimdall Mode. Packet watching will be unavailable."
    );
    return;
  }

  let interval_ms = 1000; // 1 second
  log::info!("Heimdall check period set to {interval_ms} ms.");

  std::thread::spawn(move || {
    periodic(interval_ms, "Heimdall Packet Watcher", &mut || {
      read_flows();
      expire_heimdall_flows();
      heimdall_expire();
    });
  });
}

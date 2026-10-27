//! Provides an interface to the Heimdall packet watching
//! system. Heimdall watches traffic flows, and is notified
//! about their contents via the eBPF Perf system.
#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]

pub mod events;
pub mod stats;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tracing::{debug, error, warn};
mod timeline;
pub use timeline::{hyperfocus_on_target_bridge, n_second_packet_dump, n_second_pcap};
mod pcap;
use anyhow::Result;

use crate::timeline::expire_timeline;

/// How long should Heimdall retain packet timeline data?
const TIMELINE_EXPIRE_SECS: u64 = 10;

/// How long should an analysis session remain in memory?
const SESSION_EXPIRE_SECONDS: u64 = 600;

/// The transport Heimdall watches through (heimdall_bridge_rederivation.md
/// §1.4a). K3-A collapsed the dual selection: the kernel eBPF watch map +
/// perf ring is gone, so the bridge analytics socket is the only transport.
#[derive(Clone, Debug)]
pub enum HeimdallTransport {
    /// Bridge analytics socket: `SetWatchList` + `DrainCapture`.
    Bridge {
        /// The bridge's analytics unix-socket path.
        socket: PathBuf,
    },
}

static TRANSPORT: OnceLock<HeimdallTransport> = OnceLock::new();

/// The transport selected by [`start_heimdall`], if it has run.
pub fn transport() -> Option<&'static HeimdallTransport> {
    TRANSPORT.get()
}

/// Interface to running Heimdall (start this when lqosd starts).
///
/// §1.4a(3): runs on BOTH kernel and bridge-only boxes; the caller selects
/// the transport from the ph1 kernels/bridge posture.
pub fn start_heimdall(transport: HeimdallTransport) -> Result<()> {
    if TRANSPORT.set(transport).is_err() {
        debug!("Heimdall transport already selected; ignoring duplicate start.");
    }

    let interval_ms = 1000; // 1 second
    debug!("Heimdall check period set to {interval_ms} ms.");

    std::thread::Builder::new()
        .name("Heimdall Packet Watcher".to_string())
        .spawn(move || {
            let Ok(mut tfd) = TimerFd::new() else {
                error!(
                    "Unable to created timer file descriptor. No Heimdall data will be available."
                );
                return;
            };
            assert_eq!(tfd.get_state(), TimerState::Disarmed);
            tfd.set_state(
                TimerState::Periodic {
                    current: Duration::from_millis(interval_ms),
                    interval: Duration::from_millis(interval_ms),
                },
                SetTimeFlags::Default,
            );

            loop {
                expire_timeline();

                let missed_ticks = tfd.read();
                if missed_ticks > 1 {
                    warn!("Heimdall Missed {} ticks", missed_ticks - 1);
                }
            }
        })?;

    Ok(())
}

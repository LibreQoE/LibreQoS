//! LibreQoS StormGuard. Automatic top-level HTB rate adjustment,
//! based on capacity monitoring.
//!
//! Heavily inspired by LynxTheCat's Cake AutoRate project.
//! https://github.com/lynxthecat/cake-autorate
//!
//! Copyright (C) 2025 LibreQoS. GPLv2 licensed.

mod console;
mod config;
mod queue_structure;
mod site_state;
mod datalog;

const READING_ACCUMULATOR_SIZE: usize = 15;
const MOVING_AVERAGE_BUFFER_SIZE: usize = 15;

use std::time::Duration;
use anyhow::Result;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tracing::{debug, info};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    console::set_console_logging()?;
    info!("Watching for queue structure changes...");
    lqos_queue_tracker::spawn_queue_structure_monitor()?;
    let _ = tokio::time::sleep(Duration::from_secs(1)).await;

    info!("Starting LibreQoS StormGuard...");
    let config = config::configure()?;
    let log_sender = datalog::start_datalog(&config)?;
    let mut site_state_tracker = site_state::SiteStateTracker::from_config(&config);

    // Main Cycle
    let mut tfd = TimerFd::new()?;
    assert_eq!(tfd.get_state(), TimerState::Disarmed);
    tfd.set_state(
        TimerState::Periodic {
            current: Duration::new(1, 0),
            interval: Duration::new(1, 0),
        },
        SetTimeFlags::Default,
    );

    loop {
        // Update all the ring buffers
        site_state_tracker.read_new_tick_data().await;

        // Check for state changes
        site_state_tracker.check_state();
        let recommendations = site_state_tracker.recommendations();
        if !recommendations.is_empty() {
            site_state_tracker.apply_recommendations(recommendations, &config, log_sender.clone());
        }

        // Sleep until the next second
        let missed_ticks = tfd.read();
        if missed_ticks > 1 {
            debug!("Missed {} ticks", missed_ticks);
        }
    }
    // Unreachable
}

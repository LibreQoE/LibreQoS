//! LibreQoS StormGuard. Automatic top-level HTB rate adjustment,
//! based on capacity monitoring.
//!
//! Heavily inspired by LynxTheCat's Cake AutoRate project.
//! https://github.com/lynxthecat/cake-autorate
//!
//! Copyright (C) 2025 LibreQoS. GPLv2 licensed.

use lqos_queue_tracker::QUEUE_STRUCTURE_CHANGED_STORMGUARD;
use parking_lot::Mutex;
use std::time::Duration;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tracing::{debug, info};
use lqos_bakery::BakeryCommands;

mod config;
mod queue_structure;
mod site_state;
mod datalog;

const READING_ACCUMULATOR_SIZE: usize = 15;
const MOVING_AVERAGE_BUFFER_SIZE: usize = 15;

pub static STORMGUARD_STATS: Mutex<Vec<(String, u64, u64)>> = Mutex::new(Vec::new());

/// Launches the StormGuard component. Will exit if there's
/// nothing to do.
pub async fn start_stormguard(bakery: crossbeam_channel::Sender<BakeryCommands>) -> anyhow::Result<()> {
    let _ = tokio::time::sleep(Duration::from_secs(1)).await;

    info!("Starting LibreQoS StormGuard...");
    let mut config = config::configure()?;
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
        if QUEUE_STRUCTURE_CHANGED_STORMGUARD.swap(false, std::sync::atomic::Ordering::Relaxed) {
            debug!("Queue structure changed, resetting StormGuard state");
            config.refresh_sites();
            if !config.is_empty() {
                // Reload the site state tracker
                site_state_tracker = site_state::SiteStateTracker::from_config(&config);
            }
        }
        if config.is_empty() {
            debug!("No StormGuard sites configured and available.");
            continue;
        }

        // Update all the ring buffers
        site_state_tracker.read_new_tick_data().await;

        // Check for state changes
        site_state_tracker.check_state();
        let recommendations = site_state_tracker.recommendations();
        if !recommendations.is_empty() {
            site_state_tracker.apply_recommendations(recommendations, &config, log_sender.clone(), bakery.clone());
        }

        // Sleep until the next second
        let missed_ticks = tfd.read();
        if missed_ticks > 1 {
            debug!("Missed {} ticks", missed_ticks);
        }
    }
}
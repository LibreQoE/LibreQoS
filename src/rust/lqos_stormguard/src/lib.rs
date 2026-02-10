//! LibreQoS StormGuard. Automatic top-level HTB rate adjustment,
//! based on capacity monitoring.
//!
//! Heavily inspired by LynxTheCat's Cake AutoRate project.
//! https://github.com/lynxthecat/cake-autorate
//!
//! Copyright (C) 2025 LibreQoS. GPLv2 licensed.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]

use lqos_bakery::BakeryCommands;
use lqos_queue_tracker::QUEUE_STRUCTURE_CHANGED_STORMGUARD;
use parking_lot::Mutex;
use std::time::Duration;
use tracing::{debug, info};
use lqos_bus::StormguardDebugEntry;

mod config;
mod datalog;
mod queue_structure;
mod site_state;

const READING_ACCUMULATOR_SIZE: usize = 15;
const MOVING_AVERAGE_BUFFER_SIZE: usize = 15;

/// Globally accessible stormguard statistics
pub static STORMGUARD_STATS: Mutex<Vec<(String, u64, u64)>> = Mutex::new(Vec::new());

/// Debug snapshots of StormGuard evaluation state
pub static STORMGUARD_DEBUG: Mutex<Vec<StormguardDebugEntry>> = Mutex::new(Vec::new());

/// Launches the StormGuard component. Will exit if there's
/// nothing to do.
pub async fn start_stormguard(
    bakery: crossbeam_channel::Sender<BakeryCommands>,
) -> anyhow::Result<()> {
    let _ = tokio::time::sleep(Duration::from_secs(1)).await;

    info!("Starting LibreQoS StormGuard...");

    // Initialize in "waiting" state - we'll configure when queue structure is available
    let mut config: Option<config::StormguardConfig> = None;
    let mut log_sender: Option<std::sync::mpsc::Sender<datalog::LogCommand>> = None;
    let mut site_state_tracker: Option<site_state::SiteStateTracker> = None;

    // Main Cycle - use tokio interval instead of blocking TimerFd
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        // Check if queue structure has changed or if we need initial configuration
        let queue_structure_changed =
            QUEUE_STRUCTURE_CHANGED_STORMGUARD.swap(false, std::sync::atomic::Ordering::Relaxed);

        if config.is_none() || queue_structure_changed {
            // Try to (re)configure StormGuard
            match config::configure() {
                Ok(new_config) => {
                    if new_config.is_empty() {
                        debug!("No StormGuard sites found in queue structure yet");
                        config = None;
                    } else {
                        info!("StormGuard configuration loaded successfully");
                        // Initialize or reinitialize everything
                        if log_sender.is_none() {
                            log_sender = datalog::start_datalog(&new_config).ok();
                        }
                        site_state_tracker =
                            Some(site_state::SiteStateTracker::from_config(&new_config));
                        config = Some(new_config);
                    }
                }
                Err(e) => {
                    debug!("StormGuard configuration not ready: {}", e);
                    config = None;
                }
            }
        }

        // Only process if we have a valid configuration
        if let (Some(cfg), Some(tracker)) = (&config, &mut site_state_tracker) {
            // Update all the ring buffers
            tracker.read_new_tick_data().await;

            // Check for state changes
            tracker.check_state();
            // Update debug snapshot for UI/diagnostics
            let snapshot = tracker.debug_snapshot(cfg);
            {
                let mut lock = STORMGUARD_DEBUG.lock();
                *lock = snapshot;
            }
            let recommendations = tracker.recommendations();
            if !recommendations.is_empty() {
                if let Some(sender) = &log_sender {
                    tracker.apply_recommendations(
                        recommendations,
                        cfg,
                        sender.clone(),
                        bakery.clone(),
                    );
                }
            }
        }
    }
}

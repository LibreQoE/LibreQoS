//! LibreQoS StormGuard. Automatic top-level HTB rate adjustment,
//! based on capacity monitoring.
//!
//! Heavily inspired by LynxTheCat's Cake AutoRate project.
//! https://github.com/lynxthecat/cake-autorate
//!
//! Copyright (C) 2025 LibreQoS. GPLv2 licensed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tracing::{debug, info};
use lqos_bus::StormguardStatsSnapshot;

mod config;
mod queue_structure;
mod site_state;
mod datalog;

const READING_ACCUMULATOR_SIZE: usize = 15;
const MOVING_AVERAGE_BUFFER_SIZE: usize = 15;

/// Statistics structure for Stormguard monitoring
#[derive(Debug, Default)]
pub struct StormguardStats {
    // Per-cycle counters (reset each tick)
    pub adjustments_up: AtomicU64,
    pub adjustments_down: AtomicU64,
    pub sites_evaluated: AtomicU64,
    
    // Current state counters
    pub sites_in_warmup: AtomicU64,
    pub sites_in_cooldown: AtomicU64,
    pub sites_active: AtomicU64,
    pub total_sites_managed: AtomicU64,
    
    // Performance metrics
    pub last_cycle_duration_ms: AtomicU64,
    pub recommendations_generated: AtomicU64,
}


impl StormguardStats {
    pub fn reset_per_cycle_counters(&self) {
        self.adjustments_up.store(0, Ordering::Relaxed);
        self.adjustments_down.store(0, Ordering::Relaxed);
        self.sites_evaluated.store(0, Ordering::Relaxed);
        self.recommendations_generated.store(0, Ordering::Relaxed);
    }
    
    pub fn snapshot(&self) -> StormguardStatsSnapshot {
        StormguardStatsSnapshot {
            adjustments_up: self.adjustments_up.load(Ordering::Relaxed),
            adjustments_down: self.adjustments_down.load(Ordering::Relaxed),
            sites_evaluated: self.sites_evaluated.load(Ordering::Relaxed),
            sites_in_warmup: self.sites_in_warmup.load(Ordering::Relaxed),
            sites_in_cooldown: self.sites_in_cooldown.load(Ordering::Relaxed),
            sites_active: self.sites_active.load(Ordering::Relaxed),
            total_sites_managed: self.total_sites_managed.load(Ordering::Relaxed),
            last_cycle_duration_ms: self.last_cycle_duration_ms.load(Ordering::Relaxed),
            recommendations_generated: self.recommendations_generated.load(Ordering::Relaxed),
        }
    }
}

// Global instance of statistics
pub static STORMGUARD_STATS: once_cell::sync::Lazy<Arc<StormguardStats>> = once_cell::sync::Lazy::new(|| {
    Arc::new(StormguardStats::default())
});

/// Launches the StormGuard component. Will exit if there's
/// nothing to do.
pub async fn start_stormguard(bakery_sender: crossbeam_channel::Sender<lqos_bakery::BakeryCommands>) -> anyhow::Result<()> {
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
        let cycle_start = std::time::Instant::now();
        
        // Reset per-cycle counters
        STORMGUARD_STATS.reset_per_cycle_counters();
        
        // Update all the ring buffers
        site_state_tracker.read_new_tick_data().await;

        // Check for state changes
        site_state_tracker.check_state(&STORMGUARD_STATS);
        let recommendations = site_state_tracker.recommendations();
        if !recommendations.is_empty() {
            STORMGUARD_STATS.recommendations_generated.store(recommendations.len() as u64, Ordering::Relaxed);
            site_state_tracker.apply_recommendations(recommendations, &config, log_sender.clone(), bakery_sender.clone(), &STORMGUARD_STATS);
        }
        
        // Update cycle duration
        let cycle_duration = cycle_start.elapsed();
        STORMGUARD_STATS.last_cycle_duration_ms.store(cycle_duration.as_millis() as u64, Ordering::Relaxed);

        // Sleep until the next second
        let missed_ticks = tfd.read();
        if missed_ticks > 1 {
            debug!("Missed {} ticks", missed_ticks);
        }
    }
}
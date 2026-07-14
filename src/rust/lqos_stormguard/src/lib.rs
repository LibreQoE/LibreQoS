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
use lqos_bus::StormguardDebugEntry;
use lqos_config::NetworkJsonTransport;
use lqos_probe::ProbeClient;
use lqos_queue_tracker::QUEUE_STRUCTURE_CHANGED_STORMGUARD;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::time::Duration;
use tracing::{debug, info, warn};

mod active_ping;
mod adaptive_actions;
mod config;
mod datalog;
mod queue_structure;
mod site_state;

const READING_ACCUMULATOR_SIZE: usize = 15;
const MOVING_AVERAGE_BUFFER_SIZE: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StormguardRuntimeMode {
    Disabled,
    DryRun,
    Live,
}

fn runtime_mode(enabled: bool, dry_run: bool) -> StormguardRuntimeMode {
    if !enabled {
        StormguardRuntimeMode::Disabled
    } else if dry_run {
        StormguardRuntimeMode::DryRun
    } else {
        StormguardRuntimeMode::Live
    }
}

fn requested_runtime_mode() -> Option<StormguardRuntimeMode> {
    lqos_config::load_config().ok().map(|config| {
        config
            .stormguard
            .as_ref()
            .map(|stormguard| runtime_mode(stormguard.enabled, stormguard.dry_run))
            .unwrap_or(StormguardRuntimeMode::Disabled)
    })
}

fn active_runtime_mode(config: Option<&config::StormguardConfig>) -> StormguardRuntimeMode {
    config
        .map(|config| runtime_mode(true, config.dry_run))
        .unwrap_or(StormguardRuntimeMode::Disabled)
}

fn requires_live_reset(
    active_mode: StormguardRuntimeMode,
    requested_mode: StormguardRuntimeMode,
) -> bool {
    active_mode == StormguardRuntimeMode::Live && requested_mode != StormguardRuntimeMode::Live
}

fn rebuild_occurred_since(observed: u64, current: u64, latest_rebuild: u64) -> bool {
    observed < latest_rebuild && latest_rebuild <= current
}

fn clear_published_state() {
    STORMGUARD_STATS.lock().clear();
    STORMGUARD_DEBUG.lock().clear();
}

/// Globally accessible stormguard statistics
pub static STORMGUARD_STATS: Mutex<Vec<(String, u64, u64)>> = Mutex::new(Vec::new());

/// Debug snapshots of StormGuard evaluation state
pub static STORMGUARD_DEBUG: Mutex<Vec<StormguardDebugEntry>> = Mutex::new(Vec::new());

/// Launches the StormGuard component. Will exit if there's
/// nothing to do.
pub async fn start_stormguard(
    bakery: crossbeam_channel::Sender<BakeryCommands>,
    network_map_provider: fn() -> Vec<(usize, NetworkJsonTransport)>,
    probe_client: ProbeClient,
) -> anyhow::Result<()> {
    let _ = tokio::time::sleep(Duration::from_secs(1)).await;

    info!("Starting LibreQoS StormGuard...");

    // Initialize in "waiting" state - we'll configure when queue structure is available
    let mut config: Option<config::StormguardConfig> = None;
    let mut log_sender: Option<std::sync::mpsc::Sender<datalog::LogCommand>> = None;
    let mut site_state_tracker: Option<site_state::SiteStateTracker> = None;
    let mut active_ping = active_ping::ActivePingManager::new(probe_client);
    let mut inactive_reconciled = false;
    let mut live_reset_pending = false;
    let mut observed_tree_generation = lqos_bakery::stormguard_tree_generation();
    let mut retained_circuit_fallbacks = HashSet::new();

    // Main Cycle - use tokio interval instead of blocking TimerFd
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        let Some(requested_mode) = requested_runtime_mode() else {
            warn!("StormGuard could not read the current runtime mode; retaining existing state.");
            continue;
        };
        let active_mode = active_runtime_mode(config.as_ref());
        let dry_run_plan_changed = QUEUE_STRUCTURE_CHANGED_STORMGUARD
            .swap(false, std::sync::atomic::Ordering::Relaxed)
            && active_mode == StormguardRuntimeMode::DryRun
            && requested_mode == StormguardRuntimeMode::DryRun;
        let current_tree_generation = lqos_bakery::stormguard_tree_generation();
        let bakery_tree_changed = current_tree_generation != observed_tree_generation;
        let bakery_tree_rebuilt = rebuild_occurred_since(
            observed_tree_generation,
            current_tree_generation,
            lqos_bakery::stormguard_tree_rebuild_generation(),
        );
        if bakery_tree_changed {
            if let Err(error) = lqos_queue_tracker::reload_queue_structure() {
                warn!(
                    "StormGuard could not load the queue structure for Bakery generation {current_tree_generation}; retrying: {error}"
                );
                continue;
            }
            if lqos_bakery::stormguard_tree_generation() != current_tree_generation {
                debug!(
                    "Bakery tree changed again while StormGuard refreshed its queue snapshot; retrying."
                );
                continue;
            }
            if bakery_tree_rebuilt
                && let Err(error) =
                    site_state::discard_bakery_adjustments(current_tree_generation, bakery.clone())
                        .await
            {
                warn!("StormGuard rebuild reconciliation will retry: {error}");
                continue;
            }
            inactive_reconciled = false;
        }
        if active_mode != StormguardRuntimeMode::Live {
            observed_tree_generation = current_tree_generation;
        }
        if active_mode == StormguardRuntimeMode::Live && bakery_tree_changed {
            if bakery_tree_rebuilt {
                retained_circuit_fallbacks.clear();
            } else if let Some(tracker) = &site_state_tracker {
                retained_circuit_fallbacks.extend(tracker.active_circuit_fallbacks());
            }
            observed_tree_generation = current_tree_generation;
            config = None;
            site_state_tracker = None;
            log_sender = None;
            live_reset_pending = false;
            clear_published_state();
        }
        if !bakery_tree_changed && requires_live_reset(active_mode, requested_mode) {
            live_reset_pending = true;
        }
        if live_reset_pending {
            let Some(tracker) = &mut site_state_tracker else {
                warn!("StormGuard reset is pending without an active tracker; retrying.");
                continue;
            };
            info!("StormGuard is restoring planned queue rates before reconfiguration.");
            match tracker.reset_adjustments(bakery.clone()).await {
                Ok(()) => {
                    config = None;
                    site_state_tracker = None;
                    log_sender = None;
                    inactive_reconciled = true;
                    live_reset_pending = false;
                    clear_published_state();
                }
                Err(error) => {
                    warn!("StormGuard could not restore planned queue rates; retrying: {error}");
                    continue;
                }
            }
        } else if active_mode != requested_mode && active_mode != StormguardRuntimeMode::Disabled {
            config = None;
            site_state_tracker = None;
            log_sender = None;
            clear_published_state();
        }

        if requested_mode != StormguardRuntimeMode::Live
            && !retained_circuit_fallbacks.is_empty()
        {
            if let Err(error) = site_state::clear_retained_circuit_fallbacks(
                &retained_circuit_fallbacks,
                current_tree_generation,
                bakery.clone(),
            )
            .await
            {
                warn!("StormGuard retained circuit cleanup will retry: {error}");
                continue;
            }
            retained_circuit_fallbacks.clear();
        }

        if requested_mode != StormguardRuntimeMode::Live && !inactive_reconciled {
            match site_state::reconcile_inactive_state(bakery.clone()).await {
                Ok(()) => inactive_reconciled = true,
                Err(error) => {
                    warn!("StormGuard inactive-state cleanup will retry: {error}");
                    continue;
                }
            }
        } else if requested_mode == StormguardRuntimeMode::Live {
            inactive_reconciled = false;
        }

        if requested_mode == StormguardRuntimeMode::Disabled {
            config = None;
            site_state_tracker = None;
            log_sender = None;
            clear_published_state();
            active_ping.reconfigure(None);
            continue;
        }

        if config.is_none() || bakery_tree_changed || dry_run_plan_changed {
            let configuration_generation = lqos_bakery::stormguard_tree_generation();
            // Try to (re)configure StormGuard
            match config::configure() {
                Ok(new_config) => {
                    if lqos_bakery::stormguard_tree_generation() != configuration_generation {
                        debug!(
                            "Bakery tree changed while StormGuard was configuring; retrying with the new generation."
                        );
                        config = None;
                        site_state_tracker = None;
                        continue;
                    }
                    if new_config.is_empty() {
                        debug!("No StormGuard sites found in queue structure yet");
                        config = None;
                        site_state_tracker = None;
                        clear_published_state();
                    } else {
                        info!("StormGuard configuration loaded successfully");
                        // Initialize or reinitialize everything
                        if log_sender.is_none() {
                            log_sender = datalog::start_datalog(&new_config).ok();
                        }
                        let mut tracker = site_state::SiteStateTracker::from_config(
                            &new_config,
                            configuration_generation,
                        );
                        if let Err(error) = tracker
                            .replay_persisted_adjustments(&new_config, bakery.clone())
                            .await
                        {
                            warn!("StormGuard persisted adjustment replay will retry: {error}");
                            config = None;
                            site_state_tracker = None;
                            continue;
                        }
                        tracker.retain_active_circuit_fallbacks(std::mem::take(
                            &mut retained_circuit_fallbacks,
                        ));
                        site_state_tracker = Some(tracker);
                        config = Some(new_config);
                    }
                }
                Err(e) => {
                    debug!("StormGuard configuration not ready: {}", e);
                    config = None;
                    site_state_tracker = None;
                    clear_published_state();
                }
            }
        }

        // Only process if we have a valid configuration
        active_ping.reconfigure(config.as_ref());

        if let (Some(cfg), Some(tracker)) = (&config, &mut site_state_tracker) {
            let (active_ping_sample, active_ping_updated) = active_ping.latest();
            // Update all the ring buffers
            tracker.read_new_tick_data(
                cfg,
                active_ping_sample,
                active_ping_updated,
                network_map_provider(),
            );

            // Check for state changes
            tracker.check_state(cfg);
            // Update debug snapshot for UI/diagnostics
            let snapshot = tracker.debug_snapshot(cfg);
            {
                let mut lock = STORMGUARD_DEBUG.lock();
                *lock = snapshot;
            }
            let recommendations = tracker.recommendations(cfg);
            if !recommendations.is_empty()
                && let Some(sender) = &log_sender
            {
                tracker
                    .apply_recommendations(recommendations, cfg, sender.clone(), bakery.clone())
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StormguardRuntimeMode, rebuild_occurred_since, requires_live_reset, runtime_mode,
    };

    #[test]
    fn runtime_mode_tracks_disabled_dry_run_and_live_states() {
        assert_eq!(runtime_mode(false, false), StormguardRuntimeMode::Disabled);
        assert_eq!(runtime_mode(false, true), StormguardRuntimeMode::Disabled);
        assert_eq!(runtime_mode(true, true), StormguardRuntimeMode::DryRun);
        assert_eq!(runtime_mode(true, false), StormguardRuntimeMode::Live);
    }

    #[test]
    fn live_mode_resets_only_when_leaving_live_operation() {
        assert!(requires_live_reset(
            StormguardRuntimeMode::Live,
            StormguardRuntimeMode::Disabled,
        ));
        assert!(requires_live_reset(
            StormguardRuntimeMode::Live,
            StormguardRuntimeMode::DryRun,
        ));
        assert!(!requires_live_reset(
            StormguardRuntimeMode::Live,
            StormguardRuntimeMode::Live,
        ));
        assert!(!requires_live_reset(
            StormguardRuntimeMode::DryRun,
            StormguardRuntimeMode::Disabled,
        ));
    }

    #[test]
    fn incremental_generation_preserves_bakery_ownership() {
        assert!(!rebuild_occurred_since(4, 5, 4));
        assert!(rebuild_occurred_since(4, 6, 5));
    }
}

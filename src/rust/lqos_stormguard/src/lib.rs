//! LibreQoS StormGuard. Automatic top-level HTB rate adjustment,
//! based on capacity monitoring.
//!
//! Heavily inspired by LynxTheCat's Cake AutoRate project.
//! https://github.com/lynxthecat/cake-autorate
//!
//! Copyright (C) 2025 LibreQoS. GPLv2 licensed.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]

use lqos_bus::{StormguardDebugEntry, StormguardRuntimeSettings, StormguardRuntimeStatus};
use lqos_config::NetworkJsonTransport;
use lqos_probe::ProbeClient;
use parking_lot::Mutex;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

mod active_ping;
mod adaptive_actions;
mod bus;
mod config;
mod datalog;
mod site_state;

pub use bus::StormguardBusSender;

const READING_ACCUMULATOR_SIZE: usize = 15;
const MOVING_AVERAGE_BUFFER_SIZE: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StormguardRuntimeMode {
    Disabled,
    DryRun,
    Live,
}

impl StormguardRuntimeMode {
    fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::DryRun => "dry_run",
            Self::Live => "live",
        }
    }
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

fn clear_published_state() {
    STORMGUARD_STATS.lock().clear();
    STORMGUARD_DEBUG.lock().clear();
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

/// K1b startup decoupling §1.4: the bridge path has no queue-structure change
/// signal; the canonical topology state is the watched-set source, so watch it.
static CANONICAL_CHANGED: AtomicBool = AtomicBool::new(false);

/// One WARN per process when the kernel/bakery arms are unavailable.
static BAKERY_ABSENT_WARNED: AtomicBool = AtomicBool::new(false);

fn warn_bakery_absent(context: &str) {
    if !BAKERY_ABSENT_WARNED.swap(true, Ordering::Relaxed) {
        warn!(
            "StormGuard kernel/bakery arms are unavailable on this box (no kernel path); skipping {context}"
        );
    }
}

/// Spawns the canonical-state FileWatcher (cold path) that marks the watched
/// set for re-derivation when the canonical topology changes.
fn spawn_canonical_watcher(config: &lqos_config::Config) {
    let path = lqos_config::topology_canonical_state_path(config);
    let spawn = std::thread::Builder::new()
        .name("stormguard-canonical-watch".to_string())
        .spawn(move || {
            let mut watcher = lqos_utils::file_watcher::FileWatcher::new(
                lqos_config::TOPOLOGY_CANONICAL_STATE_FILENAME,
                path,
            );
            watcher.set_file_created_callback(|| CANONICAL_CHANGED.store(true, Ordering::Relaxed));
            watcher.set_file_changed_callback(|| CANONICAL_CHANGED.store(true, Ordering::Relaxed));
            loop {
                if let Err(error) = watcher.watch() {
                    info!("StormGuard canonical-state watcher returned {error:?}");
                }
            }
        });
    if let Err(error) = spawn {
        warn!("StormGuard could not start the canonical-state watcher: {error}");
    }
}

static STORMGUARD_RUNTIME_STATUS: LazyLock<Mutex<StormguardRuntimeStatus>> = LazyLock::new(|| {
    Mutex::new(StormguardRuntimeStatus {
        configured_enabled: false,
        mode: "disabled".to_string(),
        phase: "disabled".to_string(),
        bakery_ready: false,
        cleanup_pending: false,
        strategy: None,
        active_ping_target: None,
        active_ping_weight: None,
        settings: None,
        message: None,
        last_error: None,
        updated_at_unix_ms: unix_time_ms(),
    })
});

fn publish_runtime_status(
    configured: Option<&config::StormguardConfig>,
    mode: StormguardRuntimeMode,
    phase: &str,
    cleanup_pending: bool,
    message: Option<String>,
    last_error: Option<String>,
) {
    let (strategy, active_ping_target, active_ping_weight, settings) = configured
        .map(|config| {
            (
                Some(match config.strategy {
                    lqos_config::StormguardStrategy::LegacyScore => "legacy_score".to_string(),
                    lqos_config::StormguardStrategy::DelayProbe => "delay_probe".to_string(),
                    lqos_config::StormguardStrategy::DelayProbeActive => {
                        "delay_probe_active".to_string()
                    }
                }),
                Some(config.active_ping_target.clone()),
                Some(config.active_ping_weight),
                Some(StormguardRuntimeSettings {
                    increase_fast_multiplier: config.increase_fast_multiplier,
                    increase_multiplier: config.increase_multiplier,
                    decrease_multiplier: config.decrease_multiplier,
                    decrease_fast_multiplier: config.decrease_fast_multiplier,
                    delay_threshold_ms: config.delay_threshold_ms,
                    delay_threshold_ratio: config.delay_threshold_ratio,
                    probe_interval_seconds: config.probe_interval_seconds,
                    min_throughput_mbps_for_rtt: config.min_throughput_mbps_for_rtt,
                }),
            )
        })
        .unwrap_or((None, None, None, None));
    *STORMGUARD_RUNTIME_STATUS.lock() = StormguardRuntimeStatus {
        configured_enabled: mode != StormguardRuntimeMode::Disabled,
        mode: mode.label().to_string(),
        phase: phase.to_string(),
        bakery_ready: false,
        cleanup_pending,
        strategy,
        active_ping_target,
        active_ping_weight,
        settings,
        message,
        last_error,
        updated_at_unix_ms: unix_time_ms(),
    };
}

/// Returns the latest StormGuard lifecycle and dependency-health snapshot.
///
/// This function is not pure: it reads process-local runtime state.
pub fn runtime_status() -> StormguardRuntimeStatus {
    STORMGUARD_RUNTIME_STATUS.lock().clone()
}

/// Globally accessible stormguard statistics
pub static STORMGUARD_STATS: Mutex<Vec<(String, u64, u64)>> = Mutex::new(Vec::new());

/// Debug snapshots of StormGuard evaluation state
pub static STORMGUARD_DEBUG: Mutex<Vec<StormguardDebugEntry>> = Mutex::new(Vec::new());

/// K2 slice 2 729b: the bridged passive-RTT provider — per-node p90 (ms)
/// keyed by node name, from the bridge's RTT histogram. `None` on the kernel
/// path (the `rtts` p50-based p90 stands).
pub type BridgePassiveRttProvider =
    fn() -> Option<std::collections::HashMap<String, f32>>;

/// Launches the StormGuard component. Will exit if there's
/// nothing to do.
pub async fn start_stormguard(
    network_map_provider: fn() -> Vec<(usize, NetworkJsonTransport)>,
    bridge_rtt_p90_provider: BridgePassiveRttProvider,
    probe_client: ProbeClient,
    bus_tx: bus::StormguardBusSender,
) -> anyhow::Result<()> {
    let _ = tokio::time::sleep(Duration::from_secs(1)).await;

    info!("Starting LibreQoS StormGuard...");

    // K1b startup decoupling (stormguard_startup_decoupling_card.md §1.1): on a
    // bridge-only box the kernel/bakery arms no-op. The site-clamp path
    // (override layer + republish) runs regardless.
    warn_bakery_absent("kernel/bakery arms");
    if let Ok(config) = lqos_config::load_config() {
        spawn_canonical_watcher(&config);
    }

    // Initialize in "waiting" state - we'll configure when queue structure is available
    let mut config: Option<config::StormguardConfig> = None;
    let mut log_sender: Option<datalog::DatalogHandle> = None;
    let mut logger_error: Option<String> = None;
    let mut last_application_error: Option<String> = None;
    let mut site_state_tracker: Option<site_state::SiteStateTracker> = None;
    let mut active_ping = active_ping::ActivePingManager::new(probe_client);
    let mut inactive_reconciled = false;
    let mut live_reset_pending = false;

    // Main Cycle - use tokio interval instead of blocking TimerFd
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        let Some(requested_mode) = requested_runtime_mode() else {
            warn!("StormGuard could not read the current runtime mode; retaining existing state.");
            publish_runtime_status(
                config.as_ref(),
                active_runtime_mode(config.as_ref()),
                "degraded",
                false,
                None,
                Some("StormGuard could not read the current runtime mode".to_string()),
            );
            continue;
        };
        let active_mode = active_runtime_mode(config.as_ref());
        // K1b §4.6 pin 1: drop lapsed machine-authored clamps from the layer in
        // one RMW so the source-generation hash flips and the topology manager
        // republishes the un-clamped rates. A dead detector's clamp is bounded
        // by the TTL regardless of this loop's health.
        if active_mode == StormguardRuntimeMode::Live
            && let Err(error) = adaptive_actions::sweep_lapsed_site_overrides(unix_time_ms() / 1000)
        {
            warn!("StormGuard could not sweep lapsed clamp overrides; will retry: {error}");
        }
        // K3-B: the kernel queue-structure change signal is retired; the
        // canonical-state watcher is the bridge watched-set change signal.
        let dry_run_plan_changed = false;
        // K1b startup decoupling §1.4: on the bridge path the canonical-state
        // watcher is the watched-set change signal.
        let canonical_changed = CANONICAL_CHANGED.swap(false, Ordering::Relaxed);
        if requires_live_reset(active_mode, requested_mode) {
            live_reset_pending = true;
        }
        if live_reset_pending {
            let Some(tracker) = &mut site_state_tracker else {
                warn!("StormGuard reset is pending without an active tracker; retrying.");
                publish_runtime_status(
                    config.as_ref(),
                    requested_mode,
                    "degraded",
                    true,
                    None,
                    Some("StormGuard reset is pending without an active tracker".to_string()),
                );
                continue;
            };
            info!("StormGuard is restoring planned queue rates before reconfiguration.");
            match tracker.reset_adjustments().await {
                Ok(()) => {
                    config = None;
                    site_state_tracker = None;
                    log_sender = None;
                    logger_error = None;
                    last_application_error = None;
                    inactive_reconciled = true;
                    live_reset_pending = false;
                    clear_published_state();
                }
                Err(error) => {
                    warn!("StormGuard could not restore planned queue rates; retrying: {error}");
                    publish_runtime_status(
                        config.as_ref(),
                        requested_mode,
                        "cleanup_pending",
                        true,
                        Some("Restoring planned queue rates".to_string()),
                        Some(error.to_string()),
                    );
                    continue;
                }
            }
        } else if active_mode != requested_mode && active_mode != StormguardRuntimeMode::Disabled {
            config = None;
            site_state_tracker = None;
            log_sender = None;
            logger_error = None;
            last_application_error = None;
            clear_published_state();
        }

        if requested_mode != StormguardRuntimeMode::Live && !inactive_reconciled {
            match site_state::reconcile_inactive_state().await {
                Ok(()) => inactive_reconciled = true,
                Err(error) => {
                    warn!("StormGuard inactive-state cleanup will retry: {error}");
                    publish_runtime_status(
                        config.as_ref(),
                        requested_mode,
                        "cleanup_pending",
                        true,
                        Some("Inactive-state cleanup will retry".to_string()),
                        Some(error.to_string()),
                    );
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
            logger_error = None;
            last_application_error = None;
            clear_published_state();
            active_ping.reconfigure(None);
            publish_runtime_status(
                None,
                requested_mode,
                "disabled",
                false,
                Some("StormGuard is disabled and retained state is clean".to_string()),
                None,
            );
            continue;
        }

        if config.is_none() || canonical_changed || dry_run_plan_changed {
            // Try to (re)configure StormGuard
            match config::configure() {
                Ok(new_config) => {
                    if new_config.is_empty() {
                        debug!("No StormGuard sites found in queue structure yet");
                        config = None;
                        site_state_tracker = None;
                        clear_published_state();
                    } else {
                        info!("StormGuard configuration loaded successfully");
                        // Initialize or reinitialize everything
                        let configured_log_path =
                            new_config.log_filename.as_deref().map(std::path::Path::new);
                        let active_log_path = log_sender.as_ref().map(|sender| sender.path());
                        if active_log_path != configured_log_path {
                            log_sender = None;
                            logger_error = None;
                        }
                        if log_sender.is_none() && new_config.log_filename.is_some() {
                            match datalog::start_datalog(&new_config) {
                                Ok(sender) => {
                                    log_sender = Some(sender);
                                    logger_error = None;
                                }
                                Err(error) => {
                                    let error = format!(
                                        "StormGuard diagnostic logger could not start: {error}"
                                    );
                                    warn!("{error}");
                                    logger_error = Some(error);
                                }
                            }
                        }
                        let tracker = site_state::SiteStateTracker::from_config(&new_config);
                        site_state_tracker = Some(tracker);
                        last_application_error = None;
                        config = Some(new_config);
                        publish_runtime_status(
                            config.as_ref(),
                            requested_mode,
                            if logger_error.is_some() {
                                "degraded"
                            } else if requested_mode == StormguardRuntimeMode::DryRun {
                                "dry_run"
                            } else {
                                "live"
                            },
                            false,
                            Some("StormGuard is evaluating watched sites".to_string()),
                            logger_error.clone(),
                        );
                    }
                }
                Err(e) => {
                    debug!("StormGuard configuration not ready: {}", e);
                    config = None;
                    site_state_tracker = None;
                    clear_published_state();
                    publish_runtime_status(
                        None,
                        requested_mode,
                        "degraded",
                        false,
                        None,
                        Some(e.to_string()),
                    );
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
                // K2 slice 2 729b: bridge histogram p90 when available; the
                // None case keeps the per-node rtts p50-based p90.
                bridge_rtt_p90_provider().as_ref(),
            );

            // Check for state changes
            tracker.check_state(cfg);
            let recommendations = tracker.recommendations(cfg);
            let application_report = if recommendations.is_empty() {
                site_state::ApplicationReport::default()
            } else {
                tracker
                    .apply_recommendations(recommendations, cfg, bus_tx.clone())
                    .await
            };
            if let Some(error) = application_report.errors.last() {
                last_application_error = Some(error.clone());
            } else if application_report.acknowledged_success {
                last_application_error = None;
            }

            // Capture decisions and application outcomes after this tick's work.
            let snapshot = tracker.debug_snapshot(cfg);
            if let Some(sender) = log_sender.clone() {
                let command = datalog::LogCommand::Snapshot {
                    entries: snapshot.clone(),
                    mode: if cfg.dry_run { "dry_run" } else { "live" }.to_string(),
                    timestamp_unix_ms: unix_time_ms(),
                };
                match sender.try_send(command) {
                    Ok(()) => logger_error = sender.last_error(),
                    Err(crossbeam_channel::TrySendError::Full(_)) => {
                        let error = "StormGuard diagnostic log queue is full".to_string();
                        warn!("{error}");
                        logger_error = Some(error);
                    }
                    Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                        let error = "StormGuard diagnostic logger stopped unexpectedly".to_string();
                        warn!("{error}");
                        logger_error = Some(error);
                        log_sender = None;
                    }
                }
            }
            *STORMGUARD_DEBUG.lock() = snapshot;

            let runtime_error = last_application_error
                .clone()
                .or_else(|| logger_error.clone());
            publish_runtime_status(
                Some(cfg),
                requested_mode,
                if runtime_error.is_some() {
                    "degraded"
                } else if cfg.dry_run {
                    "dry_run"
                } else {
                    "live"
                },
                false,
                Some("StormGuard is evaluating watched sites".to_string()),
                runtime_error,
            );
        } else {
            publish_runtime_status(
                config.as_ref(),
                requested_mode,
                "initializing",
                false,
                Some("Waiting for watched sites and queue state".to_string()),
                logger_error.clone(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{StormguardRuntimeMode, requires_live_reset, runtime_mode};

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

}

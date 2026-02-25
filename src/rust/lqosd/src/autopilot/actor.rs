//! Autopilot actor loop.
//!
//! The actor is responsible for sampling telemetry, maintaining state machines,
//! and applying (or dry-running) any decisions.

use crate::node_manager::ws::messages::{AutopilotActivityEntry, AutopilotStatusData};
use crate::autopilot::AutopilotError;
use crate::system_stats::SystemStats;
use crossbeam_channel::{Receiver, Sender};
use lqos_config::load_config;
use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

static AUTOPILOT_SENDER: OnceLock<Sender<AutopilotCommand>> = OnceLock::new();

/// A message sent to the Autopilot actor.
#[derive(Debug)]
pub(crate) enum AutopilotCommand {
    /// Request a status snapshot.
    GetStatus {
        /// One-shot reply channel. Side effect: sends a snapshot to the requester.
        reply: tokio::sync::oneshot::Sender<AutopilotStatusData>,
    },
    /// Request an activity snapshot.
    GetActivity {
        /// One-shot reply channel. Side effect: sends a snapshot to the requester.
        reply: tokio::sync::oneshot::Sender<Vec<AutopilotActivityEntry>>,
    },
}

/// Starts the Autopilot actor.
///
/// This function has side effects: it spawns the Autopilot background thread and registers a
/// global sender used for UI snapshot requests.
pub(crate) fn start_autopilot_actor(
    system_usage_tx: Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) -> Result<(), AutopilotError> {
    if AUTOPILOT_SENDER.get().is_some() {
        return Ok(());
    }

    let (tx, rx) = crossbeam_channel::bounded::<AutopilotCommand>(64);
    let _ = AUTOPILOT_SENDER.set(tx);

    std::thread::Builder::new()
        .name("Autopilot".to_string())
        .spawn(move || autopilot_actor_loop(rx, system_usage_tx))?;

    Ok(())
}

/// Requests a status snapshot from the Autopilot actor.
///
/// This function is not pure: it sends a message to the Autopilot actor thread.
pub(crate) async fn request_status_snapshot() -> Option<AutopilotStatusData> {
    let sender = AUTOPILOT_SENDER.get()?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    if sender
        .try_send(AutopilotCommand::GetStatus { reply: tx })
        .is_err()
    {
        return None;
    }
    rx.await.ok()
}

/// Requests an activity snapshot from the Autopilot actor.
///
/// This function is not pure: it sends a message to the Autopilot actor thread.
pub(crate) async fn request_activity_snapshot() -> Option<Vec<AutopilotActivityEntry>> {
    let sender = AUTOPILOT_SENDER.get()?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    if sender
        .try_send(AutopilotCommand::GetActivity { reply: tx })
        .is_err()
    {
        return None;
    }
    rx.await.ok()
}

fn autopilot_actor_loop(
    rx: Receiver<AutopilotCommand>,
    system_usage_tx: Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
    debug!("Autopilot actor started");

    let mut status = AutopilotStatusData {
        enabled: false,
        dry_run: true,
        cpu_max_pct: None,
        managed_nodes: 0,
        managed_circuits: 0,
        last_action_summary: None,
        warnings: Vec::new(),
    };
    let mut activity: VecDeque<AutopilotActivityEntry> = VecDeque::new();

    let mut tick_seconds: u64 = 1;
    let mut last_tick = Instant::now();

    loop {
        let next_tick = last_tick + Duration::from_secs(tick_seconds);
        let timeout = next_tick.saturating_duration_since(Instant::now());

        match rx.recv_timeout(timeout) {
            Ok(cmd) => handle_command(cmd, &status, &activity),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                last_tick = Instant::now();
                run_tick(&mut status, &mut activity, &system_usage_tx, &mut tick_seconds);
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                warn!("Autopilot actor command channel disconnected; exiting actor");
                return;
            }
        }
    }
}

fn handle_command(
    cmd: AutopilotCommand,
    status: &AutopilotStatusData,
    activity: &VecDeque<AutopilotActivityEntry>,
) {
    match cmd {
        AutopilotCommand::GetStatus { reply } => {
            let _ = reply.send(status.clone());
        }
        AutopilotCommand::GetActivity { reply } => {
            let data: Vec<AutopilotActivityEntry> = activity.iter().cloned().rev().collect();
            let _ = reply.send(data);
        }
    }
}

fn run_tick(
    status: &mut AutopilotStatusData,
    _activity: &mut VecDeque<AutopilotActivityEntry>,
    system_usage_tx: &Sender<tokio::sync::oneshot::Sender<SystemStats>>,
    tick_seconds: &mut u64,
) {
    let mut warnings = Vec::new();

    let Ok(config) = load_config() else {
        status.enabled = false;
        status.dry_run = true;
        status.cpu_max_pct = None;
        status.managed_nodes = 0;
        status.managed_circuits = 0;
        status.last_action_summary = None;
        status.warnings = vec!["Unable to load configuration; Autopilot inactive.".to_string()];
        return;
    };

    let ap = &config.autopilot;
    *tick_seconds = ap.tick_seconds.max(1);

    if ap.enabled && ap.links.nodes.is_empty() && ap.circuits.circuits.is_empty() {
        warnings.push(
            "Autopilot is enabled but no nodes/circuits are allowlisted. No actions will occur."
                .to_string(),
        );
    }

    let cpu_max_pct = (|| -> Option<u8> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        system_usage_tx.send(tx).ok()?;
        let reply = rx.blocking_recv().ok()?;
        let max = reply.cpu_usage.iter().copied().max()?;
        Some(max.min(100) as u8)
    })();

    if ap.enabled && cpu_max_pct.is_none() {
        warnings.push("Unable to sample CPU usage; CPU-aware behavior may be degraded.".to_string());
    }

    status.enabled = ap.enabled;
    status.dry_run = ap.dry_run;
    status.cpu_max_pct = cpu_max_pct;
    status.managed_nodes = ap.links.nodes.len();
    status.managed_circuits = ap.circuits.circuits.len();
    status.warnings = warnings;
}

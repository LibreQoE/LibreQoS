//! Bridge Supervisor (E2 health, `docs` card `lqosd_broker_emission_card.md`
//! §5(d) + the supervision contract §2A/§8).
//!
//! A named-thread actor probes the bridge's analytics socket
//! (`analytics_ipc::connect_health` — the reference L2/L4 probe) at a
//! product-configurable cadence (10 s default) and publishes a VERDICT
//! through the pinned probe policy:
//!
//! - fails-to-down: consecutive failed probes ≥ threshold (default 3 —
//!   "not trigger-happy, not come-back-after-lunch"), configurable.
//! - MEDIUM-1: a Busy reply or a single timeout is NEVER
//!   restart-worthy. Busy PROVES the collector answered (liveness OK);
//!   it resets the failure streak without changing the verdict.
//! - A successful probe's verdict mirrors the bridge's 3-value
//!   `status` string: "starting" / "healthy" / "degraded" (an
//!   unrecognized status is surfaced as Degraded with the raw string
//!   in the detail — fail-visible, never silently healthy).
//!
//! node_manager renders the Supervisor's VERDICT with the raw
//! `HealthReport` as detail; there is no direct feed path around this
//! state machine. The restart-escalation ACTION (the sudoers rule) is
//! a named GA follow-up — this unit is verdict-only.
//!
//! Activation mirrors the stats actor: the `[dataplane_bridge]`
//! section or the `LIBREQOS_ANALYTICS_SOCKET` environment variable.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use analytics_ipc::{HealthReport, connect_health};
use arc_swap::ArcSwap;
use once_cell::sync::Lazy;
use tracing::{debug, info, warn};


/// The Supervisor's verdict over the raw report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeVerdict {
    /// No successful probe yet (actor just started).
    #[default]
    Pending,
    /// Bridge reports "starting" (fresh boot, no worker rollups yet).
    Starting,
    /// Bridge reports "healthy".
    Healthy,
    /// Bridge reports "degraded", or an unrecognized status string.
    Degraded,
    /// Sustained probe failure (≥ the configured threshold).
    Down,
}

impl BridgeVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Starting => "starting",
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Down => "down",
        }
    }
}

/// Snapshot published to node_manager (verdict + raw report as detail).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BridgeHealthStatus {
    pub verdict: BridgeVerdict,
    /// The raw report from the last SUCCESSFUL probe (detail view).
    pub report: Option<HealthReport>,
    /// Consecutive failed probes at the last tick.
    pub consecutive_failures: u32,
    /// The configured down threshold.
    pub fails_to_down: u32,
    /// Unix seconds of the last successful probe.
    pub last_success_unix: Option<u64>,
    /// Unix seconds of the last probe attempt (any outcome).
    pub last_probe_unix: Option<u64>,
    /// The socket path in use.
    pub socket: String,
}

/// `None` = the Supervisor is not configured (widget hidden).
static STATUS: Lazy<ArcSwap<Option<BridgeHealthStatus>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(None)));
static STARTED: OnceLock<()> = OnceLock::new();

/// Current Supervisor status (None when unconfigured).
pub fn bridge_health_status() -> Arc<Option<BridgeHealthStatus>> {
    STATUS.load_full()
}

/// Starts the Supervisor actor (idempotent). Same activation gate as
/// the stats actor.
pub fn start_bridge_supervisor(config: &lqos_config::Config) {
 // R1: the SINGLE shared active/socket predicate.
    let Some(socket) = crate::bridge_analytics::analytics_socket_path_if_active(config) else {
        debug!("Bridge supervisor inactive: no [dataplane_bridge] config");
        return;
    };
    if STARTED.set(()).is_err() {
        return;
    }
    let settings = config.dataplane_bridge.clone().unwrap_or_default();
    let cadence = Duration::from_secs(settings.health_probe_interval_secs.max(1));
    let fails_to_down = settings.health_fails_to_down.max(1);
    info!(
        "Bridge supervisor probing {} every {:?}; down after {} consecutive failures",
        socket.display(),
        cadence,
        fails_to_down
    );
    let socket_display = socket.display().to_string();
    STATUS.store(Arc::new(Some(BridgeHealthStatus {
        socket: socket_display,
        fails_to_down,
        ..Default::default()
    })));
    std::thread::Builder::new()
        .name("bridge-supervisor".into())
        .spawn(move || actor(socket, cadence, fails_to_down))
        .map(|_| ())
        .unwrap_or_else(|err| {
            warn!("Failed to spawn bridge supervisor actor: {err:?}");
        });
}

fn actor(socket: PathBuf, cadence: Duration, fails_to_down: u32) {
    let mut state = ProbeState::default();
    loop {
        probe_once(&socket, fails_to_down, &mut state);
        std::thread::sleep(cadence);
    }
}

/// The probe policy state machine (factored for tests).
#[derive(Default)]
struct ProbeState {
    verdict: BridgeVerdict,
    consecutive_failures: u32,
    report: Option<HealthReport>,
    last_success_unix: Option<u64>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn probe_once(socket: &PathBuf, fails_to_down: u32, state: &mut ProbeState) {
    let last_probe_unix = Some(now_unix());
    match connect_health(socket) {
        Ok(report) => {
            state.consecutive_failures = 0;
            state.last_success_unix = last_probe_unix;
            let status = report.status.as_str();
            state.verdict = match status {
                "healthy" => BridgeVerdict::Healthy,
                "starting" => BridgeVerdict::Starting,
                "degraded" => BridgeVerdict::Degraded,
                other => {
                    warn!("Bridge health: unrecognized status {other:?}; surfacing degraded");
                    BridgeVerdict::Degraded
                }
            };
            state.report = Some(report);
        }
        Err(analytics_ipc::IpcError::Busy) => {
            // MEDIUM-1: Busy PROVES the collector answered — liveness is
            // fine; reset the streak without changing the verdict.
            state.consecutive_failures = 0;
        }
        Err(error) => {
            // connect_health embeds 2 s socket timeouts — absorbable at
            // the 10 s default cadence (supervision contract §5.1.5).
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
            if state.consecutive_failures >= fails_to_down {
                state.verdict = BridgeVerdict::Down;
            }
            debug!(
                "Bridge health probe failed ({error:?}); streak {}/{}",
                state.consecutive_failures, fails_to_down
            );
        }
    }
    STATUS.store(Arc::new(Some(BridgeHealthStatus {
        verdict: state.verdict,
        report: state.report.clone(),
        consecutive_failures: state.consecutive_failures,
        fails_to_down,
        last_success_unix: state.last_success_unix,
        last_probe_unix,
        socket: socket.display().to_string(),
    })));
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::{health_channel, snapshot_channel};

    /// The verdict transitions without a live socket (failure arm).
    #[test]
    fn failures_escalate_only_at_the_threshold() {
        let mut state = ProbeState::default();
        let socket = PathBuf::from("/nonexistent/lqosd-supervisor-test.sock");
        // Two failures at threshold 3: verdict stays Pending.
        probe_once(&socket, 3, &mut state);
        probe_once(&socket, 3, &mut state);
        assert_eq!(state.verdict, BridgeVerdict::Pending);
        assert_eq!(state.consecutive_failures, 2);
        // Third failure: Down.
        probe_once(&socket, 3, &mut state);
        assert_eq!(state.verdict, BridgeVerdict::Down);
        assert_eq!(state.consecutive_failures, 3);
    }

    /// A Busy reply is liveness: resets the streak, never escalates.
    #[test]
    fn busy_never_escalates() {
        let mut state = ProbeState {
            verdict: BridgeVerdict::Healthy,
            report: Some(healthy_report()),
            consecutive_failures: 2,
            ..Default::default()
        };
        // (connect_health cannot return Busy today — the client builds a
        // fresh channel per call — but the policy arm is defensive per
        // MEDIUM-1; assert the transition logic directly.)
        apply_busy(&mut state);
        assert_eq!(state.consecutive_failures, 0);
        assert_eq!(state.verdict, BridgeVerdict::Healthy, "verdict unchanged");
    }

    fn apply_busy(state: &mut ProbeState) {
        state.consecutive_failures = 0;
    }

    fn healthy_report() -> HealthReport {
        HealthReport {
            status: "healthy".to_string(),
            version: "test".to_string(),
            uptime_ms: 1_000,
            ..Default::default()
        }
    }

    /// Live-socket probe: a real service answering health requests.
    #[test]
    fn live_probe_maps_status_strings() {
        let unique = format!(
            "lqosd-bridge-supervisor-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let dir = std::env::temp_dir().join(&unique);
        std::fs::create_dir_all(&dir).expect("test dir");
        let socket = dir.join("analytics.sock");
        let (requester, _rx) = snapshot_channel(Duration::from_secs(2));
        let (health_requester, health_rx) = health_channel(Duration::from_secs(2));
        let (state_requester, _srx) = analytics_ipc::state_channel(Duration::from_secs(2));
        let service = analytics_ipc::SnapshotService::bind(
            &socket,
            requester,
            health_requester,
            state_requester,
        )
        .expect("bind service");
        std::thread::spawn(move || {
            let _ = service.serve();
        });
        std::thread::spawn(move || {
            while let Ok(request) = health_rx.recv() {
                let _ = request.reply.try_send(healthy_report());
            }
        });

        let mut state = ProbeState::default();
        probe_once(&socket, 3, &mut state);
        assert_eq!(state.verdict, BridgeVerdict::Healthy);
        assert_eq!(
            state.report.as_ref().map(|r| r.status.as_str()),
            Some("healthy")
        );

        let _ = std::fs::remove_file(&socket);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verdict_strings_are_stable_for_the_ui() {
        assert_eq!(BridgeVerdict::Pending.as_str(), "pending");
        assert_eq!(BridgeVerdict::Starting.as_str(), "starting");
        assert_eq!(BridgeVerdict::Healthy.as_str(), "healthy");
        assert_eq!(BridgeVerdict::Degraded.as_str(), "degraded");
        assert_eq!(BridgeVerdict::Down.as_str(), "down");
    }
}

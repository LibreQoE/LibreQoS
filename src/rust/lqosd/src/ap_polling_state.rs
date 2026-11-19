//! In-memory AP-polling status and the per-device failure alert.
//!
//! The poller publishes one [`SweepOutcome`] per managed device after each
//! sweep; this module folds them into the per-device status that the read-only
//! `GET /local-api/ap-polling` surface reports. It also owns the
//! `AP_POLL_FAILED` episode: exactly one journal line and one urgent item after
//! `[ap_polling].failures_before_alert` consecutive failures, cleared by the
//! next success.
//!
//! The status is diagnostic only: nothing here reads or writes shaping state,
//! so a failing AP can never slow or alter packet processing. The observation
//! file (`ap_observations.json`) remains the persisted store; this module only
//! mirrors its latest sweep for the local API and adds the failure bookkeeping
//! the file does not carry.

use crate::urgent;
use lqos_bus::{UrgentSeverity, UrgentSource};
use lqos_utils::unix_time::unix_now;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::net::IpAddr;
use tracing::{info, warn};

/// Urgent/alert code for one device's failure episode. The node id is the
/// dedupe key, so two APs never collapse into one alert.
pub(crate) const AP_POLL_FAILED_CODE: &str = "AP_POLL_FAILED";

/// Lifecycle of the polling task, as far as the API surface can prove it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServiceState {
    /// `[ap_polling].enabled` is false; the task never started.
    Disabled,
    /// The task is running (it may not have completed a sweep yet).
    Running,
    /// The task started and has stopped (shutdown or disabled by a reload).
    Stopped,
}

impl ServiceState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Running => "running",
            Self::Stopped => "stopped",
        }
    }
}

/// One device's poll state. `Observing` means no poll has completed yet and is
/// never used as a substitute for a real result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceState {
    /// Tracked, but no poll has completed yet.
    Observing,
    /// The latest poll produced an observation.
    Ok,
    /// The latest poll produced no observation.
    Failing,
}

impl DeviceState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Observing => "observing",
            Self::Ok => "ok",
            Self::Failing => "failing",
        }
    }
}

/// The latest known status of one AP.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeviceStatus {
    /// Integration-minted node id.
    pub(crate) node_id: String,
    /// Model from the compiled `DeviceRef`, when known.
    pub(crate) model: Option<String>,
    /// Management address from the compiled `DeviceRef`, when known.
    pub(crate) mgmt_ip: Option<IpAddr>,
    /// Capacity from the latest observation (kept from the last good poll
    /// while the device is failing, so the row can still show what was known).
    pub(crate) capacity_mbps: Option<f64>,
    /// `phy`, `per_sm_rate`, `crm`, or `unknown` from the last good poll.
    pub(crate) capacity_source: Option<String>,
    /// Station rows in the last good poll.
    pub(crate) station_count: Option<u64>,
    /// Circuits the compiled topology pins under this node, from the last
    /// supplement report. Display-only.
    pub(crate) crm_pinned_count: Option<u64>,
    /// `station_count - crm_pinned_count` for the latest good poll: positive
    /// means the AP sees more stations than the CRM pins under it.
    pub(crate) drift_delta: Option<i64>,
    /// Unix time of the latest completed poll (success or failure).
    pub(crate) last_poll: Option<u64>,
    /// Unix time of the latest successful observation.
    pub(crate) last_ok: Option<u64>,
    /// Consecutive polls without an observation.
    pub(crate) consecutive_failures: u32,
    /// The adapter that produced the last good observation, when known.
    pub(crate) adapter_id: Option<String>,
    /// True while the `AP_POLL_FAILED` episode is open for this device.
    pub(crate) alerted: bool,
}

impl DeviceStatus {
    pub(crate) fn state(&self) -> DeviceState {
        if self.consecutive_failures > 0 {
            DeviceState::Failing
        } else if self.last_ok.is_some() {
            DeviceState::Ok
        } else {
            DeviceState::Observing
        }
    }
}

/// A read-only copy of the store, taken under the lock for one response.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Snapshot {
    /// Task lifecycle.
    pub(crate) service: ServiceState,
    /// Effective poll interval in seconds.
    pub(crate) interval_s: u64,
    /// Threshold in force for the latest sweep.
    pub(crate) failures_before_alert: u32,
    /// Unix time of the latest completed sweep, `None` before the first one.
    pub(crate) last_sweep_at: Option<u64>,
    /// Devices in the latest sweep, ordered by node id.
    pub(crate) devices: Vec<DeviceStatus>,
}

/// One device's result from a single sweep.
///
/// `observed == false` covers every no-observation outcome (timeout, unknown
/// family, absent management IP, unsupported SNMP version); the poller does
/// not distinguish them today.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SweepOutcome {
    pub(crate) node_id: String,
    pub(crate) model: Option<String>,
    pub(crate) mgmt_ip: Option<IpAddr>,
    pub(crate) adapter_id: Option<String>,
    pub(crate) observed: bool,
    /// Observation fields; `None` when the poll produced no observation.
    pub(crate) capacity_mbps: Option<f64>,
    pub(crate) capacity_source: Option<String>,
    pub(crate) station_count: Option<u64>,
    /// Pins read from the last supplement report for this node, when present.
    pub(crate) crm_pinned_count: Option<u64>,
}

#[derive(Debug)]
struct Store {
    service: ServiceState,
    interval_s: u64,
    failures_before_alert: u32,
    last_sweep_at: Option<u64>,
    devices: BTreeMap<String, DeviceStatus>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

#[cfg(test)]
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Serialises tests that mutate the process-global store.
#[cfg(test)]
pub(crate) fn test_lock() -> parking_lot::MutexGuard<'static, ()> {
    TEST_LOCK.lock()
}

/// Installs the store at daemon startup, before the task is spawned.
///
/// A disabled section is a first-class state: the endpoint must say
/// `ap_polling_disabled`, never an empty-but-healthy list.
pub(crate) fn init(config: &lqos_config::Config) {
    let mut guard = STORE.lock();
    *guard = Some(Store {
        service: if config.ap_polling.enabled {
            // Enabled but not spawned yet; `mark_running` moves it on.
            ServiceState::Stopped
        } else {
            ServiceState::Disabled
        },
        interval_s: config.ap_polling.interval_s,
        failures_before_alert: config.ap_polling.failures_before_alert.max(1),
        last_sweep_at: None,
        devices: BTreeMap::new(),
    });
}

/// Marks the polling task as running (no-op without a store).
pub(crate) fn mark_running() {
    if let Some(store) = STORE.lock().as_mut()
        && store.service != ServiceState::Disabled
    {
        store.service = ServiceState::Running;
    }
}

/// Marks the task stopped after it has left its loop.
pub(crate) fn mark_stopped() {
    if let Some(store) = STORE.lock().as_mut()
        && store.service == ServiceState::Running
    {
        store.service = ServiceState::Stopped;
    }
}

/// Marks the task disabled (`[ap_polling].enabled = false`).
pub(crate) fn mark_disabled() {
    if let Some(store) = STORE.lock().as_mut() {
        store.service = ServiceState::Disabled;
    }
}

/// Copies the current status for one API response.
pub(crate) fn snapshot() -> Option<Snapshot> {
    STORE.lock().as_ref().map(|store| Snapshot {
        service: store.service,
        interval_s: store.interval_s,
        failures_before_alert: store.failures_before_alert,
        last_sweep_at: store.last_sweep_at,
        devices: store.devices.values().cloned().collect(),
    })
}

/// Folds one sweep's outcomes into the store and runs the alert episodes.
///
/// Devices absent from `outcomes` are dropped (they left the compiled
/// topology). The last known capacity/station count is retained while a
/// device fails so the API row does not lose the payload that made the device
/// useful; `last_ok` and `state` carry the staleness.
pub(crate) fn apply_sweep(outcomes: &[SweepOutcome], interval_s: u64, failures_before_alert: u32) {
    let now = unix_now().unwrap_or(0);
    let threshold = failures_before_alert.max(1);
    let mut guard = STORE.lock();
    let Some(store) = guard.as_mut() else {
        return;
    };
    store.interval_s = interval_s;
    store.failures_before_alert = threshold;
    store.last_sweep_at = Some(now);
    if store.service != ServiceState::Disabled {
        store.service = ServiceState::Running;
    }

    let mut next = BTreeMap::new();
    for outcome in outcomes {
        let previous = store.devices.remove(&outcome.node_id);
        let mut status = DeviceStatus {
            node_id: outcome.node_id.clone(),
            model: outcome.model.clone(),
            mgmt_ip: outcome.mgmt_ip,
            capacity_mbps: previous
                .as_ref()
                .and_then(|previous| previous.capacity_mbps),
            capacity_source: previous
                .as_ref()
                .and_then(|previous| previous.capacity_source.clone()),
            station_count: previous
                .as_ref()
                .and_then(|previous| previous.station_count),
            crm_pinned_count: previous
                .as_ref()
                .and_then(|previous| previous.crm_pinned_count),
            drift_delta: previous.as_ref().and_then(|previous| previous.drift_delta),
            last_poll: Some(now),
            last_ok: previous.as_ref().and_then(|previous| previous.last_ok),
            consecutive_failures: previous
                .as_ref()
                .map_or(0, |previous| previous.consecutive_failures),
            adapter_id: previous
                .as_ref()
                .and_then(|previous| previous.adapter_id.clone()),
            alerted: previous.as_ref().is_some_and(|previous| previous.alerted),
        };

        if outcome.crm_pinned_count.is_some() {
            status.crm_pinned_count = outcome.crm_pinned_count;
        }

        if outcome.observed {
            status.last_ok = Some(now);
            status.consecutive_failures = 0;
            status.capacity_mbps = outcome.capacity_mbps;
            status.capacity_source = outcome.capacity_source.clone();
            status.station_count = outcome.station_count;
            status.drift_delta = status
                .station_count
                .zip(status.crm_pinned_count)
                .map(|(observed, pinned)| observed as i64 - pinned as i64);
            if outcome.adapter_id.is_some() {
                status.adapter_id = outcome.adapter_id.clone();
            }
            if status.alerted {
                status.alerted = false;
                clear_alert(&status.node_id, "answered again");
            }
        } else {
            status.consecutive_failures = status.consecutive_failures.saturating_add(1);
            if !status.alerted && status.consecutive_failures >= threshold {
                raise_alert(&status);
                status.alerted = true;
            }
        }

        next.insert(status.node_id.clone(), status);
    }
    // A device that leaves the compiled topology ends its episode with it; an
    // orphaned urgent item would otherwise outlive the device by its TTL.
    for (node_id, previous) in std::mem::take(&mut store.devices) {
        if previous.alerted {
            clear_alert(&node_id, "left the compiled topology");
        }
    }
    store.devices = next;
}

/// Raises one episode: the journal line comes FIRST (74t.195: an in-memory-only
/// item is undiagnosable from the field), then the urgent item the UI shows.
fn raise_alert(status: &DeviceStatus) {
    let message = format!(
        "AP_POLL_FAILED: node {} has not produced an observation for {} consecutive polls \
         (mgmt_ip={}, adapter={}); check reachability, SNMP community/version, and adapter \
         detection. Shaping is unaffected.",
        status.node_id,
        status.consecutive_failures,
        status
            .mgmt_ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        status.adapter_id.as_deref().unwrap_or("unknown"),
    );
    journal_failure(&message);
    urgent::submit(
        UrgentSource::System,
        UrgentSeverity::Warning,
        AP_POLL_FAILED_CODE.to_string(),
        message,
        Some(format!("node_id={}", status.node_id)),
        Some(status.node_id.clone()),
    );
}

/// Closes the episode: the urgent item goes away; the recovery is journaled at
/// info (the failure line above is the default-visible one).
fn clear_alert(node_id: &str, why: &str) {
    let cleared = urgent::clear_by_identity(AP_POLL_FAILED_CODE, node_id);
    if cleared > 0 {
        info!(
            target: "ap_polling",
            "AP_POLL_RECOVERED: node {node_id} {why}; cleared {cleared} alert(s)"
        );
    }
}

fn journal_failure(message: &str) {
    warn!(target: "ap_polling", "{message}");
    #[cfg(test)]
    journal_capture::record(message);
}

/// Test-only capture of the journal line (the `warn!` itself is not observable
/// without a global subscriber; the lesson is that the line is emitted on the
/// same path the alert is raised).
#[cfg(test)]
mod journal_capture {
    use parking_lot::Mutex;

    static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    pub(super) fn record(line: &str) {
        LINES.lock().push(line.to_string());
    }

    pub(super) fn take() -> Vec<String> {
        std::mem::take(&mut *LINES.lock())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::urgent::test_lock as urgent_test_lock;
    use lqos_config::{ApPollingConfig, Config};

    fn config(enabled: bool) -> Config {
        Config {
            ap_polling: ApPollingConfig {
                enabled,
                interval_s: 30,
                failures_before_alert: 3,
                ..Default::default()
            },
            ..Config::default()
        }
    }

    fn reset() {
        *STORE.lock() = None;
        let _ = journal_capture::take();
        urgent::clear_all();
    }

    fn device(node_id: &str, observed: bool) -> SweepOutcome {
        SweepOutcome {
            node_id: node_id.to_string(),
            model: Some("ePMP 4500L".to_string()),
            mgmt_ip: Some("100.124.6.121".parse().expect("test IP")),
            adapter_id: observed.then(|| "cambium_epmp".to_string()),
            observed,
            capacity_mbps: observed.then_some(192.5),
            capacity_source: observed.then(|| "per_sm_rate".to_string()),
            station_count: observed.then_some(3),
            crm_pinned_count: observed.then_some(2),
        }
    }

    fn alerts() -> Vec<lqos_bus::UrgentIssue> {
        urgent::list()
            .into_iter()
            .filter(|issue| issue.code == AP_POLL_FAILED_CODE)
            .collect()
    }

    #[test]
    fn alert_fires_once_at_the_threshold_and_journals() {
        let _urgent = urgent_test_lock();
        let _store = test_lock();
        reset();
        init(&config(true));

        for _ in 0..2 {
            apply_sweep(&[device("node-ap-1", false)], 30, 3);
        }
        assert!(
            journal_capture::take().is_empty(),
            "below threshold is quiet"
        );
        assert!(alerts().is_empty(), "below threshold raises no urgent item");

        apply_sweep(&[device("node-ap-1", false)], 30, 3);
        let lines = journal_capture::take();
        assert_eq!(lines.len(), 1, "the threshold raises one journal line");
        assert!(lines[0].starts_with("AP_POLL_FAILED: node node-ap-1"));
        assert_eq!(alerts().len(), 1, "one urgent item per episode");
        assert_eq!(alerts()[0].dedupe_key.as_deref(), Some("node-ap-1"));

        // Still failing: no re-fire, no second journal line.
        for _ in 0..5 {
            apply_sweep(&[device("node-ap-1", false)], 30, 3);
        }
        assert!(journal_capture::take().is_empty());
        assert_eq!(alerts().len(), 1);
        let snapshot = snapshot().expect("store exists");
        assert_eq!(snapshot.devices[0].consecutive_failures, 8);
        assert!(snapshot.devices[0].alerted);

        reset();
    }

    #[test]
    fn success_clears_the_episode_and_resets_the_counter() {
        let _urgent = urgent_test_lock();
        let _store = test_lock();
        reset();
        init(&config(true));

        for _ in 0..3 {
            apply_sweep(&[device("node-ap-1", false)], 30, 3);
        }
        assert_eq!(alerts().len(), 1);
        assert_eq!(journal_capture::take().len(), 1);

        apply_sweep(&[device("node-ap-1", true)], 30, 3);
        assert!(
            alerts().is_empty(),
            "the next success clears the urgent item"
        );
        let snapshot = snapshot().expect("store exists");
        let row = &snapshot.devices[0];
        assert_eq!(row.consecutive_failures, 0);
        assert!(!row.alerted);
        assert_eq!(row.state(), DeviceState::Ok);
        assert_eq!(row.capacity_mbps, Some(192.5));
        assert_eq!(row.station_count, Some(3));
        assert_eq!(row.crm_pinned_count, Some(2));
        assert_eq!(row.drift_delta, Some(1), "3 observed - 2 pinned");
        assert!(row.last_ok.is_some());

        // A new episode after recovery alerts again.
        for _ in 0..3 {
            apply_sweep(&[device("node-ap-1", false)], 30, 3);
        }
        assert_eq!(alerts().len(), 1);
        assert_eq!(journal_capture::take().len(), 1);

        reset();
    }

    #[test]
    fn a_failing_device_keeps_its_last_good_payload_and_last_ok() {
        let _urgent = urgent_test_lock();
        let _store = test_lock();
        reset();
        init(&config(true));

        apply_sweep(&[device("node-ap-1", true)], 30, 3);
        let observed_at = snapshot().expect("store exists").devices[0]
            .last_ok
            .expect("a good poll records last_ok");

        apply_sweep(&[device("node-ap-1", false)], 30, 3);
        let row = &snapshot().expect("store exists").devices[0];
        assert_eq!(row.state(), DeviceState::Failing);
        assert_eq!(row.last_ok, Some(observed_at));
        assert_eq!(row.capacity_mbps, Some(192.5));
        assert_eq!(row.adapter_id.as_deref(), Some("cambium_epmp"));
        assert_eq!(row.station_count, Some(3));
        assert_eq!(row.crm_pinned_count, Some(2));
        assert_eq!(
            row.drift_delta,
            Some(1),
            "the stale payload keeps its last live drift"
        );

        reset();
    }

    #[test]
    fn zero_threshold_alerts_on_the_first_failure() {
        let _urgent = urgent_test_lock();
        let _store = test_lock();
        reset();
        init(&config(true));

        apply_sweep(&[device("node-ap-1", false)], 30, 0);
        assert_eq!(alerts().len(), 1);
        assert_eq!(snapshot().expect("store exists").failures_before_alert, 1);

        reset();
    }

    #[test]
    fn removed_devices_leave_the_store_and_lifecycle_is_explicit() {
        let _urgent = urgent_test_lock();
        let _store = test_lock();
        reset();
        init(&config(false));
        assert_eq!(
            snapshot().expect("store exists").service,
            ServiceState::Disabled
        );

        init(&config(true));
        mark_running();
        apply_sweep(
            &[device("node-ap-1", true), device("node-ap-2", true)],
            30,
            3,
        );
        assert_eq!(snapshot().expect("store exists").devices.len(), 2);

        // Open an episode on node-ap-2 while node-ap-1 stays healthy.
        for _ in 0..3 {
            apply_sweep(
                &[device("node-ap-1", true), device("node-ap-2", false)],
                30,
                3,
            );
        }
        assert_eq!(alerts().len(), 1, "node-ap-2 is alerting");

        // node-ap-2 leaves the topology: its episode closes with it.
        apply_sweep(&[device("node-ap-1", true)], 30, 3);
        assert!(
            alerts().is_empty(),
            "an alert dies with the device that left the topology"
        );
        let latest = snapshot().expect("store exists");
        assert_eq!(latest.devices.len(), 1);
        assert_eq!(latest.devices[0].node_id, "node-ap-1");
        assert_eq!(latest.service, ServiceState::Running);

        mark_stopped();
        assert_eq!(
            snapshot().expect("store exists").service,
            ServiceState::Stopped
        );
        mark_disabled();
        assert_eq!(
            snapshot().expect("store exists").service,
            ServiceState::Disabled
        );

        reset();
        assert!(snapshot().is_none());
    }

    #[test]
    fn journal_line_names_the_device_and_never_contains_shaping_claims() {
        let _urgent = urgent_test_lock();
        let _store = test_lock();
        reset();
        init(&config(true));

        apply_sweep(&[device("node-ap-1", false)], 30, 1);
        let line = journal_capture::take().pop().expect("a journal line");
        assert!(line.contains("node-ap-1"));
        assert!(line.contains("100.124.6.121"));
        assert!(line.contains("Shaping is unaffected"));

        reset();
    }

    #[test]
    fn the_snapshot_is_ordered_by_node_id() {
        let _urgent = urgent_test_lock();
        let _store = test_lock();
        reset();
        init(&config(true));

        apply_sweep(
            &[
                device("node-z", true),
                device("node-a", false),
                device("node-m", true),
            ],
            30,
            3,
        );
        let ids: Vec<String> = snapshot()
            .expect("store exists")
            .devices
            .into_iter()
            .map(|row| row.node_id)
            .collect();
        assert_eq!(ids, vec!["node-a", "node-m", "node-z"]);

        reset();
    }
}

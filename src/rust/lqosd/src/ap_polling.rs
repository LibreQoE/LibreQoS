//! AP polling seam: a disabled-by-default background task that reads the
//! compiled topology's typed `DeviceRef`s, polls through a pluggable
//! [`Poller`], and publishes a generation-stamped `ap_observations.json`.
//!
//! Step 3 wires the vendored dudarchy engine; test builds use a
//! no-observation stub to exercise the disabled/fail-open paths.
//! The task is never on the import path and is only spawned when
//! `[ap_polling].enabled` is true.

use crate::ap_poller_engine::DudarchyPoller;
use crate::ap_polling_state::{self, SweepOutcome};
use futures_util::stream::{self, StreamExt};
use lqos_config::{
    Config, DeviceRef, TopologyCanonicalStateFile, topology_dataplane_transaction_path,
};
use lqos_topology_compile::ap_supplement::AP_SUPPLEMENT_REPORT_FILENAME;
use lqos_utils::unix_time::unix_now;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::net::IpAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Name of the generation-stamped observation file, written beside
/// `dataplane_transaction.json` in the state directory.
pub const AP_OBSERVATIONS_FILENAME: &str = "ap_observations.json";

/// Current observation-file schema version.
pub const AP_OBSERVATIONS_SCHEMA_VERSION: u32 = 1;

/// Upper bound on in-flight per-device polls in one sweep.
const MAX_CONCURRENT_POLLS: usize = 32;

/// The future returned by [`Poller::poll_device`].
pub type PollFuture<'a> = Pin<Box<dyn Future<Output = Option<DeviceObservation>> + Send + 'a>>;

/// Per-device inputs the poller needs besides the `DeviceRef` and timeout.
///
/// `config` is the sweep's reloaded config (credentials/version/factors);
/// `crm_rate_mbps` is the node's rate from the compiled canonical topology,
/// when one exists. It is `Option` because a node may have no CRM rate.
#[derive(Clone, Copy)]
pub struct PollContext<'a> {
    /// The sweep's configuration.
    pub config: &'a Config,
    /// The compiled topology's rate for this node, when one exists.
    pub crm_rate_mbps: Option<f64>,
}

/// One successful device observation.
///
/// `fields` is a JSON object supplied by the engine and flattened into the
/// per-device record. The writer adds `sequence`, `observed_at`, and
/// `mgmt_ip` around it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeviceObservation {
    /// Unix timestamp of the observation.
    pub observed_at: u64,
    /// Engine-specific observation fields (must serialize as an object).
    #[serde(flatten)]
    pub fields: serde_json::Value,
}

/// The polling seam. Step 3 implements this with the vendored dudarchy engine.
pub trait Poller: Send + Sync {
    /// Poll one device with the configured per-device timeout.
    ///
    /// `None` means the device produced no observation this sweep (unknown
    /// family, unparseable/absent IP, timeout, or an unsupported credential
    /// version); the caller counts it and never treats it as fatal. The
    /// caller also enforces `timeout`, so an implementation that ignores the
    /// parameter still cannot hang a sweep.
    fn poll_device<'a>(
        &'a self,
        device: &'a DeviceRef,
        timeout: Duration,
        context: PollContext<'a>,
    ) -> PollFuture<'a>;
}

/// The test stub: every device yields no observation.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NullPoller;

#[cfg(test)]
impl Poller for NullPoller {
    fn poll_device<'a>(
        &'a self,
        _device: &'a DeviceRef,
        _timeout: Duration,
        _context: PollContext<'a>,
    ) -> PollFuture<'a> {
        Box::pin(async { None })
    }
}

/// One persisted device record.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ApDeviceRecord {
    /// Per-node monotonic sequence. Starts at 0; persistence is a later step.
    pub sequence: u64,
    /// Unix timestamp of the observation.
    pub observed_at: u64,
    /// Management address from the DeviceRef, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mgmt_ip: Option<IpAddr>,
    /// Engine-specific observation fields.
    #[serde(flatten)]
    pub fields: serde_json::Value,
}

/// The complete `ap_observations.json` document.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ApObservationsFile {
    /// Schema version for compatibility checks.
    pub schema_version: u32,
    /// Compiled-topology generation this sweep was taken against.
    pub topology_generation: String,
    /// Unix timestamp when the file was written.
    pub written_at: u64,
    /// Observations keyed by the integration-minted node id.
    pub devices: BTreeMap<String, ApDeviceRecord>,
}

/// Errors from one sweep's output.
#[derive(Debug, Error)]
pub enum ApPollingError {
    /// Reading or writing the observation file failed.
    #[error("Unable to access ap_observations.json: {0}")]
    Io(#[from] std::io::Error),
    /// Serializing the observation file failed.
    #[error("Unable to serialize ap_observations.json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Result of one polling sweep.
#[derive(Clone, Debug, PartialEq)]
pub enum PassOutcome {
    /// The file was written.
    Written {
        /// Path written.
        path: PathBuf,
        /// Number of observed devices in the file.
        device_count: usize,
        /// Devices that produced no observation this sweep (unknown family,
        /// absent/bad IP, timeout, or unsupported version); counted, never
        /// fatal.
        skipped_count: usize,
        /// Per-device status for the read-only API and the failure alert.
        /// The task publishes this after the write; it is not persisted.
        outcomes: Vec<SweepOutcome>,
    },
    /// No compiled-topology generation was available; nothing was written.
    SkippedNoGeneration,
    /// The canonical topology could not be read; nothing was written.
    SkippedNoTopology,
}

/// In-memory per-node sequence state.
///
/// TODO(step 3): persist this with the last-known-good store so a process
/// restart continues at `last_sequence + 1`. Until then a restart resets to 0.
#[derive(Debug, Default)]
struct SequenceStore {
    next: HashMap<String, u64>,
}

impl SequenceStore {
    fn next_sequence(&mut self, node_id: &str) -> u64 {
        let next = self.next.entry(node_id.to_string()).or_insert(0);
        let sequence = *next;
        *next = next.saturating_add(1);
        sequence
    }
}

/// Handle to the running AP polling task.
///
/// The owner keeps this handle for its lifetime; dropping it wakes and
/// cancels the loop. Tests exercise explicit abort/shutdown.
pub struct ApPollingTask {
    stop: watch::Sender<bool>,
    join: Option<JoinHandle<()>>,
}

// Test-only lifecycle surface; the shipped path relies on Drop.
#[cfg(test)]
impl ApPollingTask {
    /// Cancel the task without waiting.
    pub fn abort(&self) {
        if let Some(join) = &self.join {
            join.abort();
        }
    }

    /// Cancel the task and await its termination.
    pub async fn abort_and_wait(mut self) {
        self.abort();
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }

    /// Ask the task to stop and await its clean termination.
    pub async fn shutdown(mut self) -> Result<(), tokio::task::JoinError> {
        let _ = self.stop.send(true);
        match self.join.take() {
            Some(join) => join.await,
            None => Ok(()),
        }
    }
}

impl Drop for ApPollingTask {
    fn drop(&mut self) {
        // A dropped handle never leaves an orphan: wake the loop and cancel.
        let _ = self.stop.send(true);
        if let Some(join) = &self.join {
            join.abort();
        }
    }
}

/// Starts the AP poller only when `[ap_polling].enabled` is true.
///
/// Disabled is the default and returns `None` without spawning anything.
/// The running task re-reads the config each sweep, so a reload that disables
/// the section or changes the interval/timeout takes effect without a restart.
pub fn start_configured_ap_polling(config: Arc<Config>) -> Option<ApPollingTask> {
    if !config.ap_polling.enabled {
        ap_polling_state::mark_disabled();
        return None;
    }
    info!(
        "AP polling task starting (interval {}s, timeout {}ms)",
        config.ap_polling.interval_s, config.ap_polling.snmp_timeout_ms
    );
    Some(start_ap_polling_with_source(
        ConfigSource::Reload,
        Arc::new(DudarchyPoller::new()),
    ))
}

/// Starts the AP poller with an explicit [`Poller`] implementation.
#[cfg(test)]
pub fn start_ap_polling(config: Arc<Config>, poller: Arc<dyn Poller>) -> ApPollingTask {
    start_ap_polling_with_source(ConfigSource::Fixed(config), poller)
}

fn start_ap_polling_with_source(source: ConfigSource, poller: Arc<dyn Poller>) -> ApPollingTask {
    let (stop, stop_rx) = watch::channel(false);
    let join = tokio::spawn(run_loop(source, poller, stop_rx));
    ApPollingTask {
        stop,
        join: Some(join),
    }
}

enum ConfigSource {
    #[cfg(test)]
    Fixed(Arc<Config>),
    Reload,
}

impl ConfigSource {
    fn load(&self) -> Option<Arc<Config>> {
        match self {
            #[cfg(test)]
            Self::Fixed(config) => Some(config.clone()),
            Self::Reload => match lqos_config::load_config() {
                Ok(config) => Some(config),
                Err(err) => {
                    warn!("AP polling stopped: unable to reload config: {err}");
                    None
                }
            },
        }
    }
}

async fn run_loop(source: ConfigSource, poller: Arc<dyn Poller>, mut stop: watch::Receiver<bool>) {
    let mut sequences = SequenceStore::default();
    ap_polling_state::mark_running();
    loop {
        if *stop.borrow() {
            break;
        }
        let Some(config) = source.load() else {
            break;
        };
        if !config.ap_polling.enabled {
            info!("AP polling task stopped: [ap_polling].enabled is false");
            ap_polling_state::mark_disabled();
            break;
        }
        match run_poll_pass(config.as_ref(), poller.clone(), &mut sequences).await {
            Ok(PassOutcome::Written {
                path,
                device_count,
                skipped_count,
                outcomes,
            }) => {
                ap_polling_state::apply_sweep(
                    &outcomes,
                    config.ap_polling.interval_s,
                    config.ap_polling.failures_before_alert,
                );
                debug!(
                    "AP polling wrote {} observation(s) to {}; {} device(s) skipped",
                    device_count,
                    path.display(),
                    skipped_count
                );
            }
            Ok(PassOutcome::SkippedNoGeneration) => {
                debug!("AP polling skipped: compiled-topology generation unavailable")
            }
            Ok(PassOutcome::SkippedNoTopology) => {
                debug!("AP polling skipped: canonical topology unavailable")
            }
            Err(err) => warn!("AP polling sweep failed: {err}"),
        }

        let base = Duration::from_secs(config.ap_polling.interval_s.max(1));
        let delay = jittered_interval(base);
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            _ = tokio::time::sleep(delay) => {}
        }
    }
    ap_polling_state::mark_stopped();
    info!("AP polling task stopped");
}

async fn run_poll_pass(
    config: &Config,
    poller: Arc<dyn Poller>,
    sequences: &mut SequenceStore,
) -> Result<PassOutcome, ApPollingError> {
    let Some(generation) = load_topology_generation(config) else {
        return Ok(PassOutcome::SkippedNoGeneration);
    };
    let canonical = match TopologyCanonicalStateFile::load(config) {
        Ok(canonical) => canonical,
        Err(err) => {
            warn!("AP polling skipped: unable to read canonical topology: {err}");
            return Ok(PassOutcome::SkippedNoTopology);
        }
    };

    let timeout = Duration::from_millis(config.ap_polling.snmp_timeout_ms);
    let crm_rates = canonical_crm_rates(&canonical);
    let results: Vec<(DeviceRef, Option<DeviceObservation>)> =
        stream::iter(canonical.device_refs())
            .map(|device| {
                let poller = poller.clone();
                let crm_rate_mbps = crm_rates.get(&device.node_id).copied();
                async move {
                    let context = PollContext {
                        config,
                        crm_rate_mbps,
                    };
                    let observation = tokio::time::timeout(
                        timeout,
                        poller.poll_device(&device, timeout, context),
                    )
                    .await
                    .unwrap_or_default();
                    (device, observation)
                }
            })
            .buffer_unordered(MAX_CONCURRENT_POLLS)
            .collect()
            .await;

    // Per-device outcomes feed the read-only API and the AP_POLL_FAILED
    // episodes. Nothing here touches shaping: a failed poll is a diagnostic
    // fact, not a scheduling input.
    let pins = supplement_pins(config);
    let mut devices = BTreeMap::new();
    let mut outcomes = Vec::with_capacity(results.len());
    let mut skipped_count = 0;
    for (device, observation) in results {
        let node_id = device.node_id.trim();
        let crm_pinned_count = pins.get(node_id).copied();
        match observation {
            None => {
                skipped_count += 1;
                if !node_id.is_empty() {
                    outcomes.push(sweep_outcome(&device, None, crm_pinned_count));
                }
            }
            Some(observation) => {
                if node_id.is_empty() {
                    continue;
                }
                outcomes.push(sweep_outcome(&device, Some(&observation), crm_pinned_count));
                let sequence = sequences.next_sequence(node_id);
                devices.insert(
                    node_id.to_string(),
                    ApDeviceRecord {
                        sequence,
                        observed_at: observation.observed_at,
                        mgmt_ip: device.mgmt_ip,
                        fields: observation.fields,
                    },
                );
            }
        }
    }

    let file = ApObservationsFile {
        schema_version: AP_OBSERVATIONS_SCHEMA_VERSION,
        topology_generation: generation,
        written_at: unix_now().unwrap_or(0),
        devices,
    };
    let device_count = file.devices.len();
    let path = write_ap_observations(config, &file)?;
    Ok(PassOutcome::Written {
        path,
        device_count,
        skipped_count,
        outcomes,
    })
}

/// The per-device row the API store and the alert consume. Absent observation
/// fields are `None`, never zeroed.
fn sweep_outcome(
    device: &DeviceRef,
    observation: Option<&DeviceObservation>,
    crm_pinned_count: Option<u64>,
) -> SweepOutcome {
    let field = |name: &str| observation.and_then(|observation| observation.fields.get(name));
    SweepOutcome {
        node_id: device.node_id.trim().to_string(),
        model: device.model.clone(),
        mgmt_ip: device.mgmt_ip,
        adapter_id: field("adapter_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        observed: observation.is_some(),
        capacity_mbps: field("capacity_mbps").and_then(serde_json::Value::as_f64),
        capacity_source: field("capacity_source")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        station_count: field("station_count").and_then(serde_json::Value::as_u64),
        crm_pinned_count,
    }
}

/// The compiled topology's per-node pin counts from the last supplement
/// report. A missing or malformed report leaves every count unknown (`None`),
/// never zero: the page must say "unknown", not "nothing is pinned".
fn supplement_pins(config: &Config) -> HashMap<String, u64> {
    let path = config.shaping_state_file_path(AP_SUPPLEMENT_REPORT_FILENAME);
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(report) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return HashMap::new();
    };
    report
        .get("station_inventory")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let node_id = entry.get("node_id")?.as_str()?.trim();
                    if node_id.is_empty() {
                        return None;
                    }
                    let pinned = entry.get("crm_pinned_count")?.as_u64()?;
                    Some((node_id.to_string(), pinned))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The compiled topology's per-node rate, when one exists. The poller passes
/// it to the 3c capacity path as the CRM rate; the supplement still owns the
/// precedence (operator pin > CRM rate > polled capacity).
fn canonical_crm_rates(canonical: &TopologyCanonicalStateFile) -> HashMap<String, f64> {
    canonical
        .nodes
        .iter()
        .filter_map(|node| {
            let rate = node
                .rate_input
                .intrinsic_download_mbps
                .or(node.rate_input.legacy_imported_download_mbps)
                .filter(|rate| *rate > 0)?;
            Some((node.node_id.clone(), rate as f64))
        })
        .collect()
}

/// Returns the observation file path (`<state>/shaping/ap_observations.json`,
/// beside `dataplane_transaction.json`).
pub fn ap_observations_path(config: &Config) -> PathBuf {
    config.shaping_state_file_path(AP_OBSERVATIONS_FILENAME)
}

fn load_topology_generation(config: &Config) -> Option<String> {
    let path = topology_dataplane_transaction_path(config);
    let raw = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let generation = value.get("generation")?.as_str()?.trim();
    if generation.is_empty() {
        None
    } else {
        Some(generation.to_string())
    }
}

fn write_ap_observations(
    config: &Config,
    file: &ApObservationsFile,
) -> Result<PathBuf, ApPollingError> {
    let path = ap_observations_path(config);
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::other("ap_observations.json has no parent state directory")
    })?;
    std::fs::create_dir_all(parent)?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(file)?;
    std::fs::write(&tmp, bytes)?;
    if let Err(err) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(path)
}

fn jittered_interval(base: Duration) -> Duration {
    // +/-10% jitter. `rand::random` is used as a value, so no !Send RNG is
    // held across an await.
    let factor = 0.9 + rand::random::<f64>() * 0.2;
    base.mul_f64(factor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_config::TOPOLOGY_CANONICAL_STATE_FILENAME;
    use serde_json::json;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "lqosd-ap-polling-{label}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn test_config(root: &Path) -> Config {
        Config {
            lqos_directory: root.display().to_string(),
            state_directory: Some(root.display().to_string()),
            ap_polling: lqos_config::ApPollingConfig {
                enabled: true,
                interval_s: 3600,
                snmp_timeout_ms: 50,
                ..Default::default()
            },
            ..Config::default()
        }
    }

    fn write_compiled_topology(config: &Config, generation: &str) {
        let topology_dir = config.resolved_state_directory().join("topology");
        std::fs::create_dir_all(&topology_dir).expect("topology directory creates");
        let canonical = json!({
            "schema_version": 2,
            "source": "test",
            "nodes": [],
            "compatibility_network_json": {
                "AP": {
                    "id": "node-ap-1",
                    "device_ref": {
                        "node_id": "node-ap-1",
                        "mgmt_ip": "192.0.2.10"
                    },
                    "children": {}
                }
            }
        });
        std::fs::write(
            config.topology_state_file_path(TOPOLOGY_CANONICAL_STATE_FILENAME),
            serde_json::to_vec(&canonical).expect("canonical state serializes"),
        )
        .expect("canonical state writes");

        let transaction_path = topology_dataplane_transaction_path(config);
        std::fs::create_dir_all(
            transaction_path
                .parent()
                .expect("transaction path has a parent"),
        )
        .expect("shaping directory creates");
        let transaction = json!({
            "generation": generation,
            "inner_ops": [],
            "leaf_ops": [],
            "mapping_ops": []
        });
        std::fs::write(
            transaction_path,
            serde_json::to_vec(&transaction).expect("transaction serializes"),
        )
        .expect("transaction writes");
    }

    struct FixedPoller;

    impl Poller for FixedPoller {
        fn poll_device<'a>(
            &'a self,
            _device: &'a DeviceRef,
            _timeout: Duration,
            _context: PollContext<'a>,
        ) -> PollFuture<'a> {
            Box::pin(async {
                Some(DeviceObservation {
                    observed_at: 42,
                    fields: json!({ "ap": "ok" }),
                })
            })
        }
    }

    fn read_observations(path: &Path) -> ApObservationsFile {
        let raw = std::fs::read_to_string(path).expect("observation file reads");
        serde_json::from_str(&raw).expect("observation file parses")
    }

    #[tokio::test]
    async fn disabled_config_starts_no_task_and_writes_no_file() {
        let root = temp_root("disabled");
        let mut config = test_config(&root);
        config.ap_polling.enabled = false;

        assert!(start_configured_ap_polling(Arc::new(config.clone())).is_none());
        assert!(!ap_observations_path(&config).exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn enabled_null_poller_writes_schema_and_generation() {
        let root = temp_root("null");
        let config = test_config(&root);
        write_compiled_topology(&config, "gen-1");

        let mut sequences = SequenceStore::default();
        let outcome = run_poll_pass(&config, Arc::new(NullPoller), &mut sequences)
            .await
            .expect("null sweep writes");
        let PassOutcome::Written {
            path,
            device_count,
            skipped_count,
            ..
        } = outcome
        else {
            panic!("expected a written observation file, got {outcome:?}");
        };
        assert_eq!(device_count, 0);
        assert_eq!(skipped_count, 1);

        let file = read_observations(&path);
        assert_eq!(file.schema_version, AP_OBSERVATIONS_SCHEMA_VERSION);
        assert_eq!(file.topology_generation, "gen-1");
        assert!(file.written_at > 0);
        assert!(file.devices.is_empty());
        assert!(!path.with_extension("json.tmp").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn observed_device_carries_node_id_mgmt_ip_and_monotonic_sequence() {
        let root = temp_root("sequence");
        let config = test_config(&root);
        write_compiled_topology(&config, "gen-2");

        let mut sequences = SequenceStore::default();
        run_poll_pass(&config, Arc::new(FixedPoller), &mut sequences)
            .await
            .expect("first sweep writes");
        let first = read_observations(&ap_observations_path(&config));
        let record = first.devices.get("node-ap-1").expect("node is observed");
        assert_eq!(record.sequence, 0);
        assert_eq!(record.observed_at, 42);
        assert_eq!(
            record.mgmt_ip,
            Some("192.0.2.10".parse::<IpAddr>().expect("test IP parses"))
        );
        assert_eq!(
            record.fields.get("ap").and_then(serde_json::Value::as_str),
            Some("ok")
        );

        run_poll_pass(&config, Arc::new(FixedPoller), &mut sequences)
            .await
            .expect("second sweep writes");
        let second = read_observations(&ap_observations_path(&config));
        assert_eq!(
            second
                .devices
                .get("node-ap-1")
                .expect("node is observed")
                .sequence,
            1
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sweep_outcome_reads_observation_fields_and_nulls_missing_ones() {
        let device = DeviceRef {
            node_id: "node-ap-1".to_string(),
            mgmt_ip: Some("192.0.2.10".parse().expect("test IP")),
            model: Some("ePMP 4500L".to_string()),
            ..Default::default()
        };
        let observation = DeviceObservation {
            observed_at: 7,
            fields: json!({
                "station_count": 3,
                "capacity_mbps": 192.5,
                "capacity_source": "per_sm_rate",
                "adapter_id": "cambium_epmp",
            }),
        };
        let observed = sweep_outcome(&device, Some(&observation), Some(2));
        assert!(observed.observed);
        assert_eq!(observed.node_id, "node-ap-1");
        assert_eq!(observed.model.as_deref(), Some("ePMP 4500L"));
        assert_eq!(observed.adapter_id.as_deref(), Some("cambium_epmp"));
        assert_eq!(observed.capacity_mbps, Some(192.5));
        assert_eq!(observed.capacity_source.as_deref(), Some("per_sm_rate"));
        assert_eq!(observed.station_count, Some(3));
        assert_eq!(observed.crm_pinned_count, Some(2));

        let failed = sweep_outcome(&device, None, None);
        assert!(!failed.observed);
        assert_eq!(failed.capacity_mbps, None);
        assert_eq!(failed.capacity_source, None);
        assert_eq!(failed.station_count, None);
        assert_eq!(failed.adapter_id, None);
        assert_eq!(failed.crm_pinned_count, None);
    }

    #[test]
    fn supplement_pins_reads_the_report_and_leaves_unknowns_absent() {
        let root = temp_root("pins");
        let config = test_config(&root);
        let path = config.shaping_state_file_path(AP_SUPPLEMENT_REPORT_FILENAME);
        std::fs::create_dir_all(path.parent().expect("report parent")).expect("state dir");
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "station_inventory": [
                    {"node_id": "node-ap-1", "station_count": 3, "crm_pinned_count": 2, "delta": 1},
                    {"node_id": "node-ap-2", "station_count": 0, "crm_pinned_count": 0, "delta": 0},
                    {"node_id": "  ", "crm_pinned_count": 9},
                    {"node_id": "node-ap-3"}
                ]
            }))
            .expect("report serializes"),
        )
        .expect("report writes");

        let pins = supplement_pins(&config);
        assert_eq!(pins.get("node-ap-1"), Some(&2));
        assert_eq!(pins.get("node-ap-2"), Some(&0));
        assert_eq!(
            pins.len(),
            2,
            "blank node ids and absent counts are skipped"
        );

        let absent = test_config(&temp_root("pins-absent"));
        assert!(
            supplement_pins(&absent).is_empty(),
            "a missing report is unknown, never zero"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn running_task_writes_then_shuts_down_cleanly() {
        let root = temp_root("shutdown");
        let config = test_config(&root);
        write_compiled_topology(&config, "gen-3");

        let task = start_ap_polling(Arc::new(config.clone()), Arc::new(NullPoller));
        let path = ap_observations_path(&config);
        let mut written = false;
        for _ in 0..200 {
            if path.exists() {
                written = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(written, "the first sweep writes the observation file");

        task.shutdown().await.expect("task joins cleanly");
        assert!(read_observations(&path).devices.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn abort_and_wait_terminates_without_panicking() {
        let root = temp_root("abort");
        let config = test_config(&root);
        write_compiled_topology(&config, "gen-4");

        let task = start_ap_polling(Arc::new(config), Arc::new(NullPoller));
        tokio::time::timeout(Duration::from_secs(1), task.abort_and_wait())
            .await
            .expect("abort and wait completes");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn malformed_or_absent_generation_skips_the_write() {
        let root = temp_root("generation");
        let config = test_config(&root);
        let mut sequences = SequenceStore::default();

        let absent = run_poll_pass(&config, Arc::new(NullPoller), &mut sequences)
            .await
            .expect("absent generation is fail-open");
        assert_eq!(absent, PassOutcome::SkippedNoGeneration);
        assert!(!ap_observations_path(&config).exists());

        let transaction_path = topology_dataplane_transaction_path(&config);
        std::fs::create_dir_all(
            transaction_path
                .parent()
                .expect("transaction path has a parent"),
        )
        .expect("shaping directory creates");
        std::fs::write(&transaction_path, b"not json").expect("malformed transaction writes");

        let malformed = run_poll_pass(&config, Arc::new(NullPoller), &mut sequences)
            .await
            .expect("malformed generation is fail-open");
        assert_eq!(malformed, PassOutcome::SkippedNoGeneration);
        assert!(!ap_observations_path(&config).exists());

        std::fs::write(
            &transaction_path,
            serde_json::to_vec(&json!({
                "generation": "",
                "inner_ops": [],
                "leaf_ops": [],
                "mapping_ops": []
            }))
            .expect("empty-generation transaction serializes"),
        )
        .expect("empty-generation transaction writes");

        let empty = run_poll_pass(&config, Arc::new(NullPoller), &mut sequences)
            .await
            .expect("empty generation is fail-open");
        assert_eq!(empty, PassOutcome::SkippedNoGeneration);
        assert!(!ap_observations_path(&config).exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn jitter_stays_within_ten_percent() {
        let base = Duration::from_secs(300);
        for _ in 0..100 {
            let jittered = jittered_interval(base);
            assert!(jittered >= Duration::from_secs(270));
            assert!(jittered <= Duration::from_secs(330));
        }
    }
}

//! `tunnel_circuits.json` ingestion adapter (spec v1.1).
//!
//! Watches `config.lqos_directory/tunnel_circuits.json`, validates the whole
//! file fail-loud, and on acceptance persists a last-good snapshot (read by
//! the shaping-input build) plus the per-site canonical nodes. A rejected
//! revision leaves the last-good world in force and surfaces a reject count +
//! reason in lqosd status.

use lqos_bus::{UrgentSeverity, UrgentSource};
use lqos_config::{
    Config, TUNNEL_CIRCUITS_FILENAME, TUNNEL_CIRCUITS_MAX_BYTES, TopologyCanonicalStateFile,
    TunnelCircuitsError, TunnelCircuitsFile, load_config, tunnel_circuits_input_path,
};
use lqos_utils::file_watcher::FileWatcher;
use lqos_utils::unix_time::unix_now;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{error, info, warn};

use crate::urgent;

/// Lifecycle state of the ingestion adapter.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelCircuitsIngestState {
    /// No provisioning file present; the adapter shapes no tunnel circuits.
    #[default]
    Inert,
    /// The most recent revision was accepted.
    Accepted,
    /// The most recent revision was rejected; the last-good snapshot stands.
    Rejected,
}

/// Operator-visible status for the tunnel-circuits ingestion adapter.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TunnelCircuitsIngestStatus {
    /// Lifecycle state of the most recent ingest.
    pub state: TunnelCircuitsIngestState,
    /// Circuit count in the last accepted snapshot.
    pub circuit_count: usize,
    /// Distinct site count in the last accepted snapshot.
    pub site_count: usize,
    /// Number of rejected revisions observed.
    pub reject_count: u64,
    /// Reason for the most recent rejection.
    pub last_reject_reason: Option<String>,
    /// Unix time of the last acceptance.
    pub last_accepted_unix: Option<u64>,
    /// Unix time of the last status update.
    pub updated_unix: Option<u64>,
}

impl Default for TunnelCircuitsIngestStatus {
    fn default() -> Self {
        Self {
            state: TunnelCircuitsIngestState::Inert,
            circuit_count: 0,
            site_count: 0,
            reject_count: 0,
            last_reject_reason: None,
            last_accepted_unix: None,
            updated_unix: None,
        }
    }
}

/// What one ingest pass did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelCircuitsIngestOutcome {
    /// The provisioning file is absent; the adapter is inert.
    Absent,
    /// The content hash matched the last acceptance; nothing re-published.
    Unchanged,
    /// The revision was accepted and the snapshot/canonical state updated.
    Accepted,
    /// The revision was rejected whole; the last-good snapshot stands.
    Rejected,
}

static STATUS: Lazy<RwLock<TunnelCircuitsIngestStatus>> =
    Lazy::new(|| RwLock::new(TunnelCircuitsIngestStatus::default()));
static LAST_ACCEPTED_HASH: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));
static LAST_REJECT_KEY: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));

/// Returns a snapshot of the current ingestion status.
pub fn get_status() -> TunnelCircuitsIngestStatus {
    STATUS.read().clone()
}

/// Spawns the long-lived watcher thread.
pub fn start_tunnel_circuits_watcher() -> anyhow::Result<()> {
    std::thread::Builder::new()
        .name("Tunnel Circuits Watcher".to_string())
        .spawn(|| {
            let Ok(config) = load_config() else {
                error!("Unable to load LibreQoS config to watch tunnel_circuits.json");
                return;
            };
            let watch_path = tunnel_circuits_input_path(config.as_ref());
            // Cold/first-tick pass: accept a file already present at boot, or
            // log the named inert INFO when it is absent.
            ingest_tunnel_circuits();
            let mut watcher = FileWatcher::new(TUNNEL_CIRCUITS_FILENAME, watch_path);
            watcher.set_file_exists_callback(ingest_tunnel_circuits);
            watcher.set_file_created_callback(ingest_tunnel_circuits);
            watcher.set_file_changed_callback(ingest_tunnel_circuits);
            loop {
                let result = watcher.watch();
                info!("tunnel_circuits.json watcher returned: {result:?}");
            }
        })?;
    Ok(())
}

fn ingest_tunnel_circuits() {
    let Ok(config) = load_config() else {
        warn!("Unable to load LibreQoS config for a tunnel_circuits.json change");
        return;
    };
    match ingest_once(config.as_ref()) {
        Ok(TunnelCircuitsIngestOutcome::Accepted) => {
            info!("tunnel_circuits.json accepted; snapshot + site nodes updated")
        }
        Ok(TunnelCircuitsIngestOutcome::Rejected) => {
            warn!("tunnel_circuits.json rejected; last-good snapshot retained")
        }
        Ok(_) => {}
        Err(err) => warn!("tunnel_circuits ingest failed: {err}"),
    }
}

/// Runs one ingest pass against `config`'s paths.
///
/// Side effects: reads the watched file, writes the snapshot and canonical
/// state on acceptance, and updates the status + urgent channel.
pub fn ingest_once(config: &Config) -> Result<TunnelCircuitsIngestOutcome, TunnelCircuitsError> {
    let path = tunnel_circuits_input_path(config);
    if !path.exists() {
        mark_inert();
        info!("tunnel_circuits.json is absent; the adapter is inert (no tunnel circuits)");
        return Ok(TunnelCircuitsIngestOutcome::Absent);
    }

    let metadata = std::fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        record_reject(
            "symlink".to_string(),
            "watched path is a symlink; a regular file is required".to_string(),
        );
        return Ok(TunnelCircuitsIngestOutcome::Rejected);
    }
    if metadata.len() > TUNNEL_CIRCUITS_MAX_BYTES {
        let reason = format!(
            "file is {} bytes, exceeding the {TUNNEL_CIRCUITS_MAX_BYTES}-byte bound",
            metadata.len()
        );
        record_reject(reason.clone(), reason);
        return Ok(TunnelCircuitsIngestOutcome::Rejected);
    }

    let bytes = std::fs::read(&path)?;
    let content_hash = sha256_hex(&bytes);
    if LAST_ACCEPTED_HASH.read().as_deref() == Some(content_hash.as_str()) {
        return Ok(TunnelCircuitsIngestOutcome::Unchanged);
    }

    let file = match TunnelCircuitsFile::parse_and_validate(&bytes) {
        Ok(file) => file,
        Err(err) => {
            record_reject(content_hash, err.to_string());
            return Ok(TunnelCircuitsIngestOutcome::Rejected);
        }
    };

    file.save_snapshot(config)?;
    merge_canonical_sites(config, &file)?;
    *LAST_ACCEPTED_HASH.write() = Some(content_hash);
    let now = now_unix();
    let mut status = STATUS.write();
    status.state = TunnelCircuitsIngestState::Accepted;
    status.circuit_count = file.circuits.len();
    status.site_count = file.site_names().len();
    status.last_accepted_unix = now;
    status.updated_unix = now;
    Ok(TunnelCircuitsIngestOutcome::Accepted)
}

fn merge_canonical_sites(
    config: &Config,
    file: &TunnelCircuitsFile,
) -> Result<(), TunnelCircuitsError> {
    let mut canonical = TopologyCanonicalStateFile::load(config)
        .map_err(|err| TunnelCircuitsError::Canonical(err.to_string()))?;
    if file.merge_site_nodes(&mut canonical.nodes) {
        canonical
            .save(config)
            .map_err(|err| TunnelCircuitsError::Canonical(err.to_string()))?;
    }
    Ok(())
}

fn mark_inert() {
    let mut status = STATUS.write();
    status.state = TunnelCircuitsIngestState::Inert;
    status.updated_unix = now_unix();
}

fn record_reject(key: String, reason: String) {
    let is_new_revision = LAST_REJECT_KEY.read().as_deref() != Some(key.as_str());
    {
        let mut status = STATUS.write();
        status.state = TunnelCircuitsIngestState::Rejected;
        if is_new_revision {
            status.reject_count += 1;
        }
        status.last_reject_reason = Some(reason.clone());
        status.updated_unix = now_unix();
    }
    if is_new_revision {
        *LAST_REJECT_KEY.write() = Some(key);
        warn!("tunnel_circuits.json rejected: {reason}");
        urgent::submit(
            UrgentSource::System,
            UrgentSeverity::Warning,
            "TUNNEL_CIRCUITS_REJECTED".to_string(),
            format!(
                "The tunnel_circuits.json provisioning file was rejected; the previous \
                 last-good snapshot remains in force. Reason: {reason}"
            ),
            None,
            Some("tunnel-circuits:rejected".to_string()),
        );
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_unix() -> Option<u64> {
    unix_now().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_config::{TunnelCircuitEntry, tunnel_circuits_snapshot_path};

    /// The status/hash statics are process-global; serialize + reset the tests.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset_statics() {
        *STATUS.write() = TunnelCircuitsIngestStatus::default();
        *LAST_ACCEPTED_HASH.write() = None;
        *LAST_REJECT_KEY.write() = None;
    }

    fn temp_config(tag: &str) -> Config {
        let dir = std::env::temp_dir().join(format!(
            "lqosd-tunnel-circuits-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Config {
            lqos_directory: dir.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        }
    }

    fn write_input(config: &Config, file: &TunnelCircuitsFile) {
        std::fs::write(
            tunnel_circuits_input_path(config),
            serde_json::to_vec(file).unwrap(),
        )
        .unwrap();
    }

    fn sample() -> TunnelCircuitsFile {
        TunnelCircuitsFile {
            schema_version: 1,
            allow_empty: false,
            generated_at: None,
            circuits: vec![TunnelCircuitEntry {
                vni: 1010,
                vlan: 64,
                subscriber_id: "SUB-1".to_string(),
                display_name: None,
                parent_site: "Riverside".to_string(),
                site_display_name: Some("Riverside MDU 7".to_string()),
                plan_down_mbps: 500,
                plan_up_mbps: 500,
            }],
        }
    }

    #[test]
    fn cold_first_tick_absent_is_inert() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_statics();
        let config = temp_config("absent");
        let outcome = ingest_once(&config).unwrap();
        assert_eq!(outcome, TunnelCircuitsIngestOutcome::Absent);
        assert_eq!(
            get_status().state,
            TunnelCircuitsIngestState::Inert
        );
    }

    #[test]
    fn accept_writes_snapshot_and_canonical_site() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_statics();
        let config = temp_config("accept");
        write_input(&config, &sample());
        let outcome = ingest_once(&config).unwrap();
        assert_eq!(outcome, TunnelCircuitsIngestOutcome::Accepted);
        let snapshot = TunnelCircuitsFile::load_snapshot(&config)
            .unwrap()
            .expect("snapshot written");
        assert_eq!(snapshot.circuits.len(), 1);
        let canonical = TopologyCanonicalStateFile::load(&config).unwrap();
        assert!(
            canonical
                .nodes
                .iter()
                .any(|node| node.node_id == "tunnel-circuits:site:Riverside"),
            "site node must be merged into canonical state"
        );
        // A byte-identical rewrite is deduped.
        assert_eq!(
            ingest_once(&config).unwrap(),
            TunnelCircuitsIngestOutcome::Unchanged
        );
    }

    #[test]
    fn reject_keeps_last_good_snapshot() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_statics();
        let config = temp_config("reject");
        write_input(&config, &sample());
        assert_eq!(ingest_once(&config).unwrap(), TunnelCircuitsIngestOutcome::Accepted);
        let good = std::fs::read(tunnel_circuits_snapshot_path(&config)).unwrap();

        // Empty circuits without allow_empty must reject whole.
        std::fs::write(
            tunnel_circuits_input_path(&config),
            br#"{"schema_version":1,"circuits":[]}"#,
        )
        .unwrap();
        assert_eq!(ingest_once(&config).unwrap(), TunnelCircuitsIngestOutcome::Rejected);
        let status = get_status();
        assert_eq!(status.state, TunnelCircuitsIngestState::Rejected);
        assert!(status.reject_count >= 1);
        assert!(status.last_reject_reason.is_some());
        assert_eq!(
            std::fs::read(tunnel_circuits_snapshot_path(&config)).unwrap(),
            good,
            "last-good snapshot must be retained on reject"
        );
    }

    #[test]
    fn absent_then_late_is_picked_up() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_statics();
        let config = temp_config("late");
        assert_eq!(ingest_once(&config).unwrap(), TunnelCircuitsIngestOutcome::Absent);
        write_input(&config, &sample());
        assert_eq!(ingest_once(&config).unwrap(), TunnelCircuitsIngestOutcome::Accepted);
    }
}

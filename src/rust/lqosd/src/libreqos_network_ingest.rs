//! `libreqos_network.json` watched full-state ingestion (lqos-74t.150 PR3).
//!
//! Always spawns a mode-neutral watcher next to the tunnel adapter. Every
//! cold pass/accept reloads the config and evaluates the 151 source predicate;
//! when the active source is `libreqos_network`, an accepted file compiles and
//! writes through the shared core. A rejected revision leaves the last-good
//! world in force and surfaces the typed reason in status/urgent.

use lqos_bus::{UrgentSeverity, UrgentSource};
use lqos_config::{Config, TopologySource, load_config, topology_source_is};
use lqos_topology_compile::{
    LIBREQOS_NETWORK_FILENAME, LIBREQOS_NETWORK_MAX_BYTES, TopologyImportFile,
    TopologyIngestError, TopologyWriteOrigin, compile_and_write_libreqos_network_json,
    libreqos_network_input_path,
};
use lqos_utils::file_watcher::FileWatcher;
use lqos_utils::unix_time::unix_now;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{error, info, warn};

use crate::urgent;

/// Lifecycle state of the `libreqos_network.json` adapter.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibreqosNetworkIngestState {
    /// The source is active but no file is present.
    #[default]
    Inert,
    /// The 151 selector is not `libreqos_network`; accepts are inert.
    Gated,
    /// The most recent revision was accepted.
    Accepted,
    /// The most recent revision was rejected; the last-good world stands.
    Rejected,
}

/// Operator-visible status for the libreqos_network watcher.
///
/// This type is owned by 150; 151 PR3 consumes it (do not duplicate).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibreqosNetworkIngestStatus {
    /// Lifecycle state of the most recent pass.
    pub state: LibreqosNetworkIngestState,
    /// True when the 151 selector names `libreqos_network`.
    pub gate_active: bool,
    /// Writer read from the last on-disk `topology_import.json`, refreshed on
    /// every status read so an API write is visible.
    pub last_artifact_writer: Option<String>,
    /// SHA-256 read from the last on-disk `topology_import.json`, refreshed on
    /// every status read so an API write is visible.
    pub last_artifact_sha256: Option<String>,
    /// Writer of the last fully accepted revision.
    pub last_accepted_writer: Option<String>,
    /// SHA-256 of the last fully accepted revision.
    pub last_accepted_sha256: Option<String>,
    /// Unix time of the last full acceptance.
    pub last_accepted_unix: Option<u64>,
    /// Successful full-write count observed by this watcher.
    pub accepted_count: u64,
    /// Rejected revision count (deduped by revision key).
    pub reject_count: u64,
    /// Reason for the most recent rejection.
    pub last_reject_reason: Option<String>,
    /// Unix time of the most recent rejection.
    pub last_reject_unix: Option<u64>,
    /// Canonical node count in the last accepted state.
    pub node_count: usize,
    /// Circuit count in the last accepted state.
    pub circuit_count: usize,
    /// Compile mode used for the last acceptance.
    pub compile_mode: String,
    /// Per-write diagnostic sequence from the last acceptance.
    pub write_sequence: u64,
    /// Unix time of the last status update.
    pub updated_unix: Option<u64>,
}

impl Default for LibreqosNetworkIngestStatus {
    fn default() -> Self {
        Self {
            state: LibreqosNetworkIngestState::Inert,
            gate_active: false,
            last_artifact_writer: None,
            last_artifact_sha256: None,
            last_accepted_writer: None,
            last_accepted_sha256: None,
            last_accepted_unix: None,
            accepted_count: 0,
            reject_count: 0,
            last_reject_reason: None,
            last_reject_unix: None,
            node_count: 0,
            circuit_count: 0,
            compile_mode: String::new(),
            write_sequence: 0,
            updated_unix: None,
        }
    }
}

/// What one ingest pass did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibreqosNetworkIngestOutcome {
    /// The selector is not `libreqos_network`.
    Gated,
    /// The selector is active but the file is absent.
    Absent,
    /// Byte-identical to the last acceptance; no compile/write.
    Unchanged,
    /// The revision was accepted and all outputs were written.
    Accepted,
    /// The revision was rejected whole; the last-good world stands.
    Rejected,
}

static STATUS: Lazy<RwLock<LibreqosNetworkIngestStatus>> =
    Lazy::new(|| RwLock::new(LibreqosNetworkIngestStatus::default()));
static LAST_ACCEPTED_HASH: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));
static LAST_REJECT_KEY: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));
static GATE_INFO_LOGGED: AtomicBool = AtomicBool::new(false);

/// Returns a snapshot of the current watcher status.
///
/// `last_artifact_writer`/`last_artifact_sha256` are refreshed from the
/// on-disk `topology_import.json` on read, so a Python API write is visible
/// even though it cannot update this process's memory. `last_accepted_*`
/// remains watcher-only: it advances only after all four outputs of a
/// watcher accept succeed. An API write does NOT advance it.
pub fn get_status() -> LibreqosNetworkIngestStatus {
    match load_config() {
        Ok(config) => get_status_for_config(config.as_ref()),
        Err(_) => STATUS.read().clone(),
    }
}

/// Returns status with the artifact view refreshed from `config`.
pub fn get_status_for_config(config: &Config) -> LibreqosNetworkIngestStatus {
    let mut status = STATUS.read().clone();
    if let Ok(Some(import)) = TopologyImportFile::load(config) {
        status.last_artifact_writer = import.writer;
        status.last_artifact_sha256 = import.source_sha256;
    }
    status
}

/// Spawns the long-lived, always-on watcher thread.
///
/// The source gate is evaluated on every pass, not at spawn: a later source
/// switch takes effect on the next file event without an lqosd restart.
pub fn start_libreqos_network_watcher() -> anyhow::Result<()> {
    std::thread::Builder::new()
        .name("libreqos_network Watcher".to_string())
        .spawn(|| {
            let Ok(config) = load_config() else {
                error!("Unable to load LibreQoS config to watch libreqos_network.json");
                return;
            };
            let watch_path = libreqos_network_input_path(config.as_ref());
            ingest_libreqos_network();
            let mut watcher = FileWatcher::new(LIBREQOS_NETWORK_FILENAME, watch_path);
            watcher.set_file_exists_callback(ingest_libreqos_network);
            watcher.set_file_created_callback(ingest_libreqos_network);
            watcher.set_file_changed_callback(ingest_libreqos_network);
            loop {
                let result = watcher.watch();
                info!("libreqos_network.json watcher returned: {result:?}");
            }
        })?;
    Ok(())
}

fn ingest_libreqos_network() {
    let Ok(config) = load_config() else {
        warn!("Unable to load LibreQoS config for a libreqos_network.json change");
        return;
    };
    match ingest_once(config.as_ref()) {
        Ok(LibreqosNetworkIngestOutcome::Accepted) => {
            info!("libreqos_network.json accepted; topology outputs updated")
        }
        Ok(LibreqosNetworkIngestOutcome::Rejected) => {
            warn!("libreqos_network.json rejected; last-good topology retained")
        }
        Ok(_) => {}
        Err(error) => warn!("libreqos_network ingest failed: {error}"),
    }
}

/// Runs one ingest pass against `config`.
pub fn ingest_once(
    config: &Config,
) -> Result<LibreqosNetworkIngestOutcome, TopologyIngestError> {
    if !topology_source_is(config, TopologySource::LibreqosNetwork) {
        mark_gated(config);
        return Ok(LibreqosNetworkIngestOutcome::Gated);
    }

    let path = libreqos_network_input_path(config);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            mark_inert();
            return Ok(LibreqosNetworkIngestOutcome::Absent);
        }
        Err(error) => {
            record_reject(
                config,
                format!("meta:{}", path.display()),
                input_error(&path, error.to_string()).to_string(),
            );
            return Ok(LibreqosNetworkIngestOutcome::Rejected);
        }
    };
    if metadata.file_type().is_symlink() {
        let reason = "watched path is a symlink; a regular file is required".to_string();
        record_reject(config, format!("symlink:{}", path.display()), reason);
        return Ok(LibreqosNetworkIngestOutcome::Rejected);
    }
    if !metadata.is_file() {
        let reason = "watched path is not a regular file".to_string();
        record_reject(config, format!("not-file:{}", path.display()), reason);
        return Ok(LibreqosNetworkIngestOutcome::Rejected);
    }
    if metadata.len() > LIBREQOS_NETWORK_MAX_BYTES as u64 {
        let reason = format!(
            "file is {} bytes, exceeding the {LIBREQOS_NETWORK_MAX_BYTES}-byte bound",
            metadata.len()
        );
        record_reject(config, format!("oversize:{}", path.display()), reason);
        return Ok(LibreqosNetworkIngestOutcome::Rejected);
    }

    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            record_reject(
                config,
                format!("read:{}", path.display()),
                input_error(&path, error.to_string()).to_string(),
            );
            return Ok(LibreqosNetworkIngestOutcome::Rejected);
        }
    };
    let content_hash = sha256_hex(&bytes);
    if LAST_ACCEPTED_HASH.read().as_deref() == Some(content_hash.as_str()) {
        return Ok(LibreqosNetworkIngestOutcome::Unchanged);
    }
    let raw = match String::from_utf8(bytes) {
        Ok(raw) => raw,
        Err(error) => {
            record_reject(
                config,
                content_hash,
                input_error(&path, format!("file is not UTF-8: {error}")).to_string(),
            );
            return Ok(LibreqosNetworkIngestOutcome::Rejected);
        }
    };

    let compile_mode = config.shared_topology_compile_mode().unwrap_or("full");
    match compile_and_write_libreqos_network_json(
        config,
        "libreqos_network",
        &raw,
        compile_mode,
        TopologyWriteOrigin::Watcher,
        Some(&content_hash),
    ) {
        Ok(receipt) => {
            *LAST_ACCEPTED_HASH.write() = Some(content_hash);
            *LAST_REJECT_KEY.write() = None;
            let now = now_unix();
            let mut status = STATUS.write();
            status.state = LibreqosNetworkIngestState::Accepted;
            status.gate_active = true;
            status.last_artifact_writer = Some(receipt.origin.as_str().to_string());
            status.last_artifact_sha256 = receipt.source_sha256.clone();
            status.last_accepted_writer = Some(receipt.origin.as_str().to_string());
            status.last_accepted_sha256 = receipt.source_sha256;
            status.last_accepted_unix = now;
            status.accepted_count = status.accepted_count.saturating_add(1);
            status.node_count = receipt.node_count;
            status.circuit_count = receipt.circuit_count;
            status.compile_mode = compile_mode.to_string();
            status.write_sequence = receipt.write_sequence;
            status.updated_unix = now;
            Ok(LibreqosNetworkIngestOutcome::Accepted)
        }
        Err(error) => {
            record_reject(config, content_hash, error.to_string());
            Ok(LibreqosNetworkIngestOutcome::Rejected)
        }
    }
}

fn mark_gated(config: &Config) {
    let mut status = STATUS.write();
    status.state = LibreqosNetworkIngestState::Gated;
    status.gate_active = false;
    status.updated_unix = now_unix();
    drop(status);
    if !GATE_INFO_LOGGED.swap(true, Ordering::SeqCst) {
        info!(
            "libreqos_network watcher gated off: topology source is {}",
            config.topology_source_label().unwrap_or("none")
        );
    }
}

fn mark_inert() {
    let mut status = STATUS.write();
    status.state = LibreqosNetworkIngestState::Inert;
    status.gate_active = true;
    status.updated_unix = now_unix();
}

fn record_reject(config: &Config, key: String, reason: String) {
    let is_new_revision = LAST_REJECT_KEY.read().as_deref() != Some(key.as_str());
    {
        let mut status = STATUS.write();
        status.state = LibreqosNetworkIngestState::Rejected;
        status.gate_active = true;
        if is_new_revision {
            status.reject_count = status.reject_count.saturating_add(1);
        }
        status.last_reject_reason = Some(reason.clone());
        status.last_reject_unix = now_unix();
        status.updated_unix = now_unix();
    }
    refresh_artifact_status(config);
    if is_new_revision {
        *LAST_REJECT_KEY.write() = Some(key);
        warn!("libreqos_network.json rejected: {reason}");
        urgent::submit(
            UrgentSource::System,
            UrgentSeverity::Warning,
            "LIBREQOS_NETWORK_REJECTED".to_string(),
            format!(
                "The libreqos_network.json topology file was rejected; the previous \
                 last-good topology remains in force. Reason: {reason}"
            ),
            None,
            Some("libreqos-network:rejected".to_string()),
        );
    }
}

fn refresh_artifact_status(config: &Config) {
    if let Ok(Some(import)) = TopologyImportFile::load(config) {
        let mut status = STATUS.write();
        status.last_artifact_writer = import.writer;
        status.last_artifact_sha256 = import.source_sha256;
    }
}

fn input_error(path: &Path, reason: impl Into<String>) -> TopologyIngestError {
    TopologyIngestError::Input {
        path: path.to_path_buf(),
        reason: reason.into(),
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
    use lqos_config::{
        TOPOLOGY_CANONICAL_STATE_FILENAME, TOPOLOGY_COMPILED_SHAPING_FILENAME,
        TOPOLOGY_IMPORT_FILENAME,
    };
    use lqos_topology_compile::TopologyCompiledShapingFile;

    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset_statics() {
        *STATUS.write() = LibreqosNetworkIngestStatus::default();
        *LAST_ACCEPTED_HASH.write() = None;
        *LAST_REJECT_KEY.write() = None;
        GATE_INFO_LOGGED.store(false, Ordering::SeqCst);
    }

    fn temp_config(tag: &str) -> Config {
        let dir = std::env::temp_dir().join(format!(
            "lqosd-libreqos-network-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut config = Config {
            lqos_directory: dir.display().to_string(),
            state_directory: Some(dir.join("state").display().to_string()),
            ..Config::default()
        };
        config.topology.source = Some(TopologySource::LibreqosNetwork);
        config.topology.compile_mode = "ap_only".to_string();
        config
    }

    fn v2_payload(ceil_down_mbps: f64) -> String {
        format!(
            r#"{{
                "schema_version": 2,
                "match": {{ "mode": "ip" }},
                "topology": [
                    {{ "id": "s1", "type": "site", "name": "Site 1" }},
                    {{ "id": "ap1", "type": "ap", "parent": "s1", "name": "AP 1" }}
                ],
                "circuits": [
                    {{ "id": "c1", "parent": "ap1", "display_name": "C1",
                       "plan": {{ "commit_down_mbps": 1.0, "commit_up_mbps": 1.0,
                                 "ceil_down_mbps": {ceil_down_mbps}, "ceil_up_mbps": 2.0 }},
                       "devices": [ {{ "id": "d1", "ipv4": ["192.0.2.10/32"] }} ],
                       "match": {{ "prefixes": ["192.0.2.10/32"] }} }}
                ]
            }}"#
        )
    }

    fn write_input(config: &Config, raw: &str) {
        std::fs::write(libreqos_network_input_path(config), raw).expect("write input");
    }

    #[test]
    fn gated_off_is_inert_and_writes_nothing() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_statics();
        let mut config = temp_config("gated");
        config.topology.source = Some(TopologySource::ShapedDevices);
        write_input(&config, &v2_payload(10.0));
        assert_eq!(
            ingest_once(&config).expect("gated pass"),
            LibreqosNetworkIngestOutcome::Gated
        );
        let status = get_status();
        assert_eq!(status.state, LibreqosNetworkIngestState::Gated);
        assert!(!status.gate_active);
        assert!(
            !config
                .topology_state_file_path(TOPOLOGY_IMPORT_FILENAME)
                .exists()
        );
    }

    #[test]
    fn accept_then_same_bytes_is_a_noop() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_statics();
        let config = temp_config("accept");
        write_input(&config, &v2_payload(10.0));
        assert_eq!(
            ingest_once(&config).expect("accept"),
            LibreqosNetworkIngestOutcome::Accepted
        );
        let first = get_status();
        assert_eq!(first.state, LibreqosNetworkIngestState::Accepted);
        assert!(first.gate_active);
        assert_eq!(first.last_accepted_writer.as_deref(), Some("watcher"));
        assert_eq!(first.node_count, 2);
        assert_eq!(first.circuit_count, 1);
        assert!(first.last_accepted_sha256.is_some());
        assert!(first.write_sequence > 0);
        assert!(
            config
                .topology_state_file_path(TOPOLOGY_IMPORT_FILENAME)
                .is_file()
        );
        assert!(
            config
                .shaping_state_file_path(TOPOLOGY_COMPILED_SHAPING_FILENAME)
                .is_file()
        );
        assert!(
            config
                .topology_state_file_path(TOPOLOGY_CANONICAL_STATE_FILENAME)
                .is_file()
        );

        assert_eq!(
            ingest_once(&config).expect("unchanged"),
            LibreqosNetworkIngestOutcome::Unchanged
        );
        let second = get_status();
        assert_eq!(second.last_accepted_unix, first.last_accepted_unix);
        assert_eq!(second.accepted_count, first.accepted_count);
    }

    #[test]
    fn reject_whole_retains_last_good() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_statics();
        let config = temp_config("reject");
        write_input(&config, &v2_payload(10.0));
        assert_eq!(
            ingest_once(&config).expect("accept"),
            LibreqosNetworkIngestOutcome::Accepted
        );
        let accepted = get_status();
        let shaping_path = config.shaping_state_file_path(TOPOLOGY_COMPILED_SHAPING_FILENAME);
        let good = std::fs::read(&shaping_path).expect("shaping bytes");

        write_input(&config, r#"{"schema_version":1,"commit_down_bps":1}"#);
        assert_eq!(
            ingest_once(&config).expect("reject"),
            LibreqosNetworkIngestOutcome::Rejected
        );
        let rejected = get_status();
        assert_eq!(rejected.state, LibreqosNetworkIngestState::Rejected);
        assert_eq!(rejected.reject_count, 1);
        assert!(rejected.last_reject_reason.is_some());
        assert_eq!(rejected.last_accepted_sha256, accepted.last_accepted_sha256);
        assert_eq!(std::fs::read(&shaping_path).expect("good bytes"), good);
    }

    #[test]
    fn symlink_and_oversize_reject_before_apply() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_statics();
        let config = temp_config("bad-input");
        let path = libreqos_network_input_path(&config);
        let target = path.with_extension("target");
        std::fs::write(&target, v2_payload(10.0)).expect("target");
        std::os::unix::fs::symlink(&target, &path).expect("symlink");
        assert_eq!(
            ingest_once(&config).expect("symlink reject"),
            LibreqosNetworkIngestOutcome::Rejected
        );
        assert!(
            get_status()
                .last_reject_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("symlink"))
        );

        reset_statics();
        let config = temp_config("oversize");
        let path = libreqos_network_input_path(&config);
        let file = std::fs::File::create(&path).expect("oversize file");
        file.set_len(LIBREQOS_NETWORK_MAX_BYTES as u64 + 1)
            .expect("set len");
        assert_eq!(
            ingest_once(&config).expect("oversize reject"),
            LibreqosNetworkIngestOutcome::Rejected
        );
        assert!(
            get_status()
                .last_reject_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("bound"))
        );
        assert!(
            !config
                .topology_state_file_path(TOPOLOGY_IMPORT_FILENAME)
                .exists()
        );
    }

    #[test]
    fn non_utf8_input_is_a_rejected_state_not_a_bare_warn() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_statics();
        let config = temp_config("non-utf8");
        std::fs::write(libreqos_network_input_path(&config), [0xff, 0xfe, 0x00])
            .expect("non-utf8 input");
        assert_eq!(
            ingest_once(&config).expect("input reject"),
            LibreqosNetworkIngestOutcome::Rejected
        );
        let status = get_status();
        assert_eq!(status.state, LibreqosNetworkIngestState::Rejected);
        assert_eq!(status.reject_count, 1);
        assert!(
            status
                .last_reject_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("UTF-8"))
        );
        assert!(status.last_reject_unix.is_some());
        assert_eq!(status.write_sequence, 0);
        assert!(status.last_accepted_sha256.is_none());
    }

    #[test]
    fn status_reads_an_api_written_artifact_without_advancing_last_accepted() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_statics();
        let config = temp_config("api-artifact");
        write_input(&config, &v2_payload(10.0));
        assert_eq!(
            ingest_once(&config).expect("watcher accept"),
            LibreqosNetworkIngestOutcome::Accepted
        );
        let accepted = get_status_for_config(&config);

        let mut import = TopologyImportFile::load(&config)
            .expect("import load")
            .expect("import written");
        import.writer = Some("api".to_string());
        import.source_sha256 = Some("api-sha".to_string());
        import.save(&config).expect("simulated API write");

        let status = get_status_for_config(&config);
        assert_eq!(status.last_artifact_writer.as_deref(), Some("api"));
        assert_eq!(status.last_artifact_sha256.as_deref(), Some("api-sha"));
        assert_eq!(
            status.last_accepted_writer, accepted.last_accepted_writer,
            "an API write does not advance last_accepted"
        );
        assert_eq!(
            status.last_accepted_sha256, accepted.last_accepted_sha256,
            "an API write does not advance last_accepted"
        );
    }

    #[test]
    fn full_state_replacement_updates_compiled_outputs() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_statics();
        let config = temp_config("replace");
        write_input(&config, &v2_payload(10.0));
        assert_eq!(
            ingest_once(&config).expect("first accept"),
            LibreqosNetworkIngestOutcome::Accepted
        );
        write_input(&config, &v2_payload(250.5));
        assert_eq!(
            ingest_once(&config).expect("replacement accept"),
            LibreqosNetworkIngestOutcome::Accepted
        );
        let compiled = TopologyCompiledShapingFile::load(&config)
            .expect("compiled load")
            .expect("compiled written");
        let max = compiled
            .shaping_artifacts()
            .shaped_devices
            .devices
            .iter()
            .map(|device| device.download_max_mbps)
            .fold(0.0_f32, f32::max);
        assert_eq!(max, 250.5);
        assert_eq!(get_status().accepted_count, 2);
    }
}

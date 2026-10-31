//! Shared compile + write core for the topology producers.
//!
//! The Python payload doors and the watched-file adapter must persist the same
//! four artifacts from the same imported bundle. This module is that one path:
//! it owns the `TopologyImportFile`/`TopologyCompiledShapingFile` construction,
//! the compile step, the ordered writes, the stale-anchor cleanup, and the
//! per-write receipt. It has no PyO3 dependency.

use crate::{
    CompiledTopologyBundle, ImportedTopologyBundle, LibreQosNetworkError, TopologyCompileError,
    TopologyCompileMode, TopologyCompiledShapingFile, TopologyImportFile, compile_topology,
    parse_libreqos_network,
};
use lqos_config::Config;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

/// Which producer drove one successful write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyWriteOrigin {
    /// A Python/integration payload door.
    Api,
    /// The watched `libreqos_network.json` adapter.
    Watcher,
}

impl TopologyWriteOrigin {
    /// Stable serialized label (also stamped into `topology_import.json`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Watcher => "watcher",
        }
    }
}

/// Terminal state of one successful write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyWriteState {
    /// All four outputs and the stale-anchor cleanup succeeded.
    Applied,
}

/// Receipt for one successful full write.
///
/// `write_sequence` is the per-write diagnostic value (also mirrored by the
/// process-local [`write_sequence`] accessor); it is not a lease or a guard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyWriteReceipt {
    /// Producer that wrote this state.
    pub origin: TopologyWriteOrigin,
    /// SHA-256 of the source bytes, when the producer has file/payload bytes.
    pub source_sha256: Option<String>,
    /// Monotonic successful-write sequence (process-local diagnostics).
    pub write_sequence: u64,
    /// Canonical topology node count in the written state.
    pub node_count: usize,
    /// Circuit anchor count in the written state.
    pub circuit_count: usize,
    /// Outputs written, in order.
    pub files: [&'static str; 4],
    /// Whether the stale duplicate anchor was removed.
    pub stale_anchor_removed: bool,
    /// Terminal write state.
    pub state: TopologyWriteState,
}

/// Typed text for a failed output write.
///
/// This is a newtype rather than a bare `String` because `thiserror` treats a
/// field named `source` as an error source and requires `std::error::Error`.
/// Keeping the variant field name `source` preserves the card's `Write{file,
/// source}` shape without pulling `anyhow` into the public error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyWriteSource(String);

impl TopologyWriteSource {
    /// Returns the underlying error text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TopologyWriteSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TopologyWriteSource {}

impl From<String> for TopologyWriteSource {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Typed failure from the shared compile + write core.
#[derive(Debug, Error)]
pub enum TopologyIngestError {
    /// The `libreqos_network.json` reader rejected the payload.
    #[error("Unable to parse libreqos_network.json: {0}")]
    Parse(#[from] LibreQosNetworkError),
    /// The topology compiler rejected the imported bundle.
    #[error("Unable to compile topology: {0}")]
    Compile(#[from] TopologyCompileError),
    /// The compile-mode spelling is not supported.
    #[error("Unknown topology compile mode '{0}'")]
    Mode(String),
    /// A pre-read/input check failed (symlink, not-regular, stat, read, oversize).
    #[error("Unable to read {path}: {reason}")]
    Input {
        /// Watched path.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },
    /// One output could not be written or the stale anchor could not be removed.
    #[error("Unable to write {file}: {source}")]
    Write {
        /// Stable output name (e.g. `topology_import.json`).
        file: &'static str,
        /// Underlying error text.
        source: TopologyWriteSource,
    },
}

/// Monotonic count of successful full writes, process-local diagnostics only.
static WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Returns the current successful-write sequence.
pub fn write_sequence() -> u64 {
    WRITE_SEQUENCE.load(Ordering::SeqCst)
}

/// Parses the Python-facing compile-mode spelling into the compiler enum.
///
/// Mirrors the historical Python door, including `full2` -> [`TopologyCompileMode::Full2`].
pub fn parse_compile_mode(mode: &str) -> Result<TopologyCompileMode, TopologyIngestError> {
    match mode.trim().to_lowercase().as_str() {
        "flat" => Ok(TopologyCompileMode::Flat),
        "ap_only" => Ok(TopologyCompileMode::ApOnly),
        "ap_site" => Ok(TopologyCompileMode::ApSite),
        "full" => Ok(TopologyCompileMode::Full),
        "full2" => Ok(TopologyCompileMode::Full2),
        _ => Err(TopologyIngestError::Mode(mode.to_string())),
    }
}

/// Compiles one imported bundle and writes the four topology outputs.
///
/// The caller supplies the already-parsed imported facts, the compile mode
/// (the watcher reads it from config; the Python doors keep their explicit
/// argument), and the write origin/source hash for the receipt and import
/// artifact stamp.
pub fn compile_and_write_imported(
    config: &Config,
    imported: ImportedTopologyBundle,
    compile_mode: &str,
    origin: TopologyWriteOrigin,
    source_sha256: Option<&str>,
) -> Result<TopologyWriteReceipt, TopologyIngestError> {
    let mut topology_import = TopologyImportFile::from_imported_bundle(&imported, compile_mode);
    topology_import.writer = Some(origin.as_str().to_string());
    topology_import.source_sha256 = source_sha256.map(str::to_string);

    let mode = parse_compile_mode(compile_mode)?;
    let node_count = imported
        .native_canonical
        .as_ref()
        .map(|canonical| canonical.nodes.len())
        .unwrap_or(0);
    let circuit_count = imported.circuit_anchors.anchors.len();
    let compiled = compile_topology(imported, mode)?;
    let compiled_shaping = TopologyCompiledShapingFile::from_compiled(&compiled, compile_mode);
    let mut receipt = write_compiled_topology_outputs(
        config,
        &topology_import,
        &compiled_shaping,
        compiled,
        origin,
        source_sha256,
    )?;
    receipt.node_count = node_count;
    receipt.circuit_count = circuit_count;
    Ok(receipt)
}

/// Parses a raw `libreqos_network.json` payload, projects it, and writes it.
///
/// This is the public entry the watched-file adapter (PR3) will call with
/// [`TopologyWriteOrigin::Watcher`]; the Python network door calls it with
/// [`TopologyWriteOrigin::Api`].
pub fn compile_and_write_libreqos_network_json(
    config: &Config,
    source: &str,
    raw: &str,
    compile_mode: &str,
    origin: TopologyWriteOrigin,
    source_sha256: Option<&str>,
) -> Result<TopologyWriteReceipt, TopologyIngestError> {
    let file = parse_libreqos_network(raw)?;
    let imported = file
        .into_imported_bundle(source.to_string())
        .map_err(TopologyIngestError::Parse)?;
    compile_and_write_imported(config, imported, compile_mode, origin, source_sha256)
}

/// Persists the four topology outputs in their existing order and removes the
/// stale duplicate `circuit_anchors.json`.
///
/// Each `.save` is individually atomic. Cross-file atomicity is **not**
/// claimed: a mid-sequence failure can leave earlier files new while later
/// files stay last-good. A failure never advances the write sequence; the
/// receipt exists only on full success.
pub fn write_compiled_topology_outputs(
    config: &Config,
    topology_import: &TopologyImportFile,
    compiled_shaping: &TopologyCompiledShapingFile,
    compiled: CompiledTopologyBundle,
    origin: TopologyWriteOrigin,
    source_sha256: Option<&str>,
) -> Result<TopologyWriteReceipt, TopologyIngestError> {
    preflight_output_paths(config)?;
    topology_import
        .save(config)
        .map_err(|error| write_error("topology_import.json", error))?;
    compiled_shaping
        .save(config)
        .map_err(|error| write_error("topology_compiled_shaping.json", error))?;
    compiled
        .parent_candidates
        .save(config)
        .map_err(|error| write_error("topology_parent_candidates.json", error))?;
    // Rung 1.5 §1.3: topology_editor_state.json is never persisted.
    compiled
        .canonical
        .save(config)
        .map_err(|error| write_error("topology_canonical_state.json", error))?;

    let anchors_path = config.legacy_runtime_file_path(lqos_config::CIRCUIT_ANCHORS_FILENAME);
    let mut stale_anchor_removed = false;
    match std::fs::remove_file(&anchors_path) {
        Ok(()) => stale_anchor_removed = true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(TopologyIngestError::Write {
                file: "circuit_anchors.json",
                source: format!(
                    "Unable to remove stale duplicate at {}: {error}",
                    anchors_path.display()
                )
                .into(),
            });
        }
    }

    let write_sequence = WRITE_SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1;
    Ok(TopologyWriteReceipt {
        origin,
        source_sha256: source_sha256.map(str::to_string),
        write_sequence,
        node_count: 0,
        circuit_count: 0,
        files: [
            "topology_import.json",
            "topology_compiled_shaping.json",
            "topology_parent_candidates.json",
            "topology_canonical_state.json",
        ],
        stale_anchor_removed,
        state: TopologyWriteState::Applied,
    })
}

/// Creates the state parents without rejecting a target that is currently a
/// directory: a mid-sequence write may still fail there, which is the
/// documented no-cross-file-atomicity boundary.
fn preflight_output_paths(config: &Config) -> Result<(), TopologyIngestError> {
    let targets = [
        (
            "topology_import.json",
            config.topology_state_file_path(lqos_config::TOPOLOGY_IMPORT_FILENAME),
        ),
        (
            "topology_compiled_shaping.json",
            config.shaping_state_file_path(lqos_config::TOPOLOGY_COMPILED_SHAPING_FILENAME),
        ),
        (
            "topology_parent_candidates.json",
            config.topology_state_file_path(lqos_config::TOPOLOGY_PARENT_CANDIDATES_FILENAME),
        ),
        (
            "topology_canonical_state.json",
            config.topology_state_file_path(lqos_config::TOPOLOGY_CANONICAL_STATE_FILENAME),
        ),
    ];
    for (file, path) in targets {
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            return Err(TopologyIngestError::Write {
                file,
                source: format!("Unable to create {}: {error}", parent.display()).into(),
            });
        }
    }
    Ok(())
}

fn write_error(file: &'static str, error: impl std::fmt::Display) -> TopologyIngestError {
    TopologyIngestError::Write {
        file,
        source: error.to_string().into(),
    }
}

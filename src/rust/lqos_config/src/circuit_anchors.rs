use crate::Config;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Runtime filename carrying circuit-to-topology anchor assignments emitted by integrations.
pub const CIRCUIT_ANCHORS_FILENAME: &str = "circuit_anchors.json";

/// Errors returned while reading or writing circuit anchor snapshots.
#[derive(Debug, Error)]
pub enum CircuitAnchorsError {
    /// Reading or writing the snapshot file failed.
    #[error("Unable to access circuit anchors file: {0}")]
    Io(#[from] std::io::Error),
    /// Serializing or deserializing the snapshot failed.
    #[error("Unable to parse circuit anchors JSON: {0}")]
    Json(#[from] serde_json::Error),
}

fn default_circuit_anchors_schema_version() -> u32 {
    1
}

/// One circuit-to-topology anchor assignment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CircuitAnchor {
    /// Stable circuit identifier.
    pub circuit_id: String,
    /// Human-readable circuit name, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circuit_name: Option<String>,
    /// Stable topology node identifier the circuit should attach beneath.
    pub anchor_node_id: String,
    /// Human-readable topology node name, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_node_name: Option<String>,
}

/// Integration-generated circuit anchor snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CircuitAnchorsFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_circuit_anchors_schema_version")]
    pub schema_version: u32,
    /// Human-readable source such as `uisp/full2` or `python/integration_common`.
    #[serde(default)]
    pub source: String,
    /// Unix timestamp when the file was generated, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_unix: Option<u64>,
    /// Ordered circuit-to-anchor assignments.
    #[serde(default)]
    pub anchors: Vec<CircuitAnchor>,
}

/// Returns the path of the circuit anchor runtime file.
///
/// This function is pure: it has no side effects.
pub fn circuit_anchors_path(config: &Config) -> PathBuf {
    config.topology_state_read_path(CIRCUIT_ANCHORS_FILENAME)
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), CircuitAnchorsError> {
    let raw = serde_json::to_string_pretty(value)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

impl CircuitAnchorsFile {
    /// Loads the circuit anchor snapshot if it exists.
    ///
    /// Side effects: reads `circuit_anchors.json` from `config.lqos_directory`.
    pub fn load(config: &Config) -> Result<Self, CircuitAnchorsError> {
        let path = circuit_anchors_path(config);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Saves the circuit anchor snapshot atomically.
    ///
    /// Side effects: writes `circuit_anchors.json` into `config.lqos_directory`.
    pub fn save(&self, config: &Config) -> Result<(), CircuitAnchorsError> {
        atomic_write_json(
            &config.topology_state_file_path(CIRCUIT_ANCHORS_FILENAME),
            self,
        )
    }
}

//! Dynamic-circuit store FILE SCHEMA (K1 §1.6).
//!
//! Relocated to this shared low-level crate so `lqos_topology` can fold the
//! store into the shaping inputs without inverting the crate graph
//! (`lqos_topology` must not depend on `lqos_network_devices`). The actor in
//! `lqos_network_devices` keeps ownership of the file; this module owns the
//! serialized shape.

use crate::{Config, ShapedDevice};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Runtime filename carrying the RADIUS dynamic-circuit overlay.
pub const DYNAMIC_CIRCUITS_FILENAME: &str = "dynamic_circuits.json";

/// Returns the dynamic-circuit store path (`config.lqos_directory`).
pub fn dynamic_circuits_path(config: &Config) -> PathBuf {
    config.legacy_runtime_file_path(DYNAMIC_CIRCUITS_FILENAME)
}

fn default_dynamic_circuits_schema_version() -> u32 {
    1
}

/// One dynamic circuit: a shaped-device-like row plus its liveness stamp.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DynamicCircuit {
    /// Shaped-device-like fields describing the dynamic circuit/device.
    pub shaped: ShapedDevice,
    /// Unix timestamp (seconds) when this dynamic circuit was last seen active.
    #[serde(default)]
    pub last_seen_unix: u64,
}

/// The serialized contents of `dynamic_circuits.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DynamicCircuitsFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_dynamic_circuits_schema_version")]
    pub schema_version: u32,
    /// The dynamic circuit rows.
    #[serde(default)]
    pub circuits: Vec<DynamicCircuit>,
}

impl DynamicCircuitsFile {
    /// Loads the store file; a missing, empty, or unparseable file is the
    /// honest-empty posture (the fold then contributes nothing).
    ///
    /// Side effects: reads `dynamic_circuits.json` from `config.lqos_directory`.
    pub fn load(config: &Config) -> Self {
        let path = dynamic_circuits_path(config);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        if raw.trim().is_empty() {
            return Self::default();
        }
        serde_json::from_str(&raw).unwrap_or_default()
    }
}

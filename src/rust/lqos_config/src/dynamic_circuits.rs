//! Dynamic-circuit store FILE SCHEMA (K1 §1.6).
//!
//! Relocated to this shared low-level crate so `lqos_topology` can fold the
//! store into the shaping inputs without inverting the crate graph
//! (`lqos_topology` must not depend on `lqos_network_devices`). The actor in
//! `lqos_network_devices` keeps ownership of the files; this module owns the
//! serialized shapes.
//!
//! Two files, two jobs:
//! - `dynamic_circuits.json` carries MEMBERSHIP and RATES only. Its bytes are
//!   hashed into the topology source generation, so a liveness touch here would
//!   rebuild the whole topology for a non-change.
//! - `dynamic_circuits_state.json` carries LIVENESS
//!   (`{circuit_id: last_seen_unix}`). It is deliberately absent from
//!   `compute_topology_source_generation` and is checkpointed at a bounded
//!   interval.

use crate::{Config, ShapedDevice};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Runtime filename carrying the RADIUS dynamic-circuit overlay (membership and
/// rates only).
pub const DYNAMIC_CIRCUITS_FILENAME: &str = "dynamic_circuits.json";

/// Runtime filename carrying dynamic-circuit liveness checkpoints.
///
/// Never hashed into the topology source generation: liveness motion must not
/// rebuild the topology.
pub const DYNAMIC_CIRCUITS_STATE_FILENAME: &str = "dynamic_circuits_state.json";

/// Returns the dynamic-circuit store path (`config.lqos_directory`).
pub fn dynamic_circuits_path(config: &Config) -> PathBuf {
    config.legacy_runtime_file_path(DYNAMIC_CIRCUITS_FILENAME)
}

/// Returns the dynamic-circuit liveness checkpoint path
/// (`config.lqos_directory`).
pub fn dynamic_circuits_state_path(config: &Config) -> PathBuf {
    config.legacy_runtime_file_path(DYNAMIC_CIRCUITS_STATE_FILENAME)
}

fn default_dynamic_circuits_schema_version() -> u32 {
    1
}

/// One dynamic circuit: a shaped-device-like row plus its liveness stamp.
///
/// `shaped.parent_node` may carry the RESERVED root sentinel `"Root"` with no
/// `parent_node_id`: unknown-IP promotion writes it when the range rule's
/// `attach_to` is blank (driver ruling 2026-11-17 — "no `attach_to` means the top
/// of the tree"). It is NOT a `network.json` node name; the topology fold
/// recognizes it and resolves it to the wire root (`root`). Audit surfaces must
/// render it as the root, never as a node-lookup key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DynamicCircuit {
    /// Shaped-device-like fields describing the dynamic circuit/device.
    pub shaped: ShapedDevice,
    /// Unix timestamp (seconds) when this dynamic circuit was last seen active.
    ///
    /// In-memory liveness only. It is never serialized into
    /// `dynamic_circuits.json` — that file must carry membership and rates
    /// only, because its bytes are hashed into the topology source generation.
    /// Liveness is persisted in the unhashed `dynamic_circuits_state.json`
    /// checkpoint instead. Deserialization still reads the legacy field so a
    /// pre-checkpoint store file can import it once during migration.
    #[serde(default, skip_serializing)]
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

//! Shared topology runtime domain logic for attachment health and effective topology.

#![warn(missing_docs)]

mod runtime;

use anyhow::{Context, Result};
use lqos_config::{
    CircuitAnchor, Config, ConfigShapedDevices, TOPOLOGY_ATTACHMENT_AUTO_ID,
    TopLevelPlannerItem, TopLevelPlannerMode, TopLevelPlannerParams, TopologyAllowedParent,
    TopologyAttachmentHealthStateFile, TopologyAttachmentHealthStatus, TopologyAttachmentOption,
    TopologyAttachmentRateSource, TopologyAttachmentRole, TopologyCanonicalIngressKind,
    TopologyCanonicalNode, TopologyCanonicalRateInputSource, TopologyCanonicalStateFile,
    TopologyEditorNode, TopologyEditorStateFile, TopologyEffectiveAttachmentState,
    TopologyEffectiveNodeState, TopologyEffectiveStateFile,
    TopologyRuntimeStatusFile, TopologyShapingCircuitInput, TopologyShapingDeviceInput,
    TopologyShapingInputsFile, TopologyShapingResolutionSource, TunnelCircuitsFile,
    compute_effective_network_generation, detect_shaping_cpus, load_shaping_inputs_at_path,
    plan_top_level_assignments,
    topology_auto_attachment_option,
    topology_effective_network_path, topology_effective_state_path,
    topology_dataplane_transaction_path, topology_operator_view_path,
    topology_runtime_status_path, topology_shaping_inputs_path,
};
use lqos_overrides::{
    CircuitAdjustment, NetworkAdjustment, OverrideStore, TopologyAttachmentMode,
    TopologyOverridesFile,
};
use lqos_topology_compile::{TopologyCompiledShapingFile, TopologyImportFile};
use lqos_utils::process_lock::{ProcessFileLock, ProcessLockConfig};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, hash_map::Entry};
use std::fs::File;
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;

pub use runtime::start_topology_thread;

pub mod circuit_capacity;
pub use circuit_capacity::{
    CIRCUIT_CAPACITY_FLOOR, applied_circuit_capacity, derive_circuit_capacity,
    note_applied_circuit_capacity,
};

include!("common.rs");
pub use lqos_dataplane_types::*;
pub mod dataplane_compiler;
pub use dataplane_compiler::{
    aqm_from_override, aqm_with_default, compile_dataplane_transaction,
    compile_dataplane_transaction_with_ranges, compile_dataplane_transaction_with_ranges_and_sqm,
};
include!("shaping_inputs.rs");
include!("flat_mode.rs");
include!("direct_circuit_hosts.rs");
include!("artifacts.rs");
include!("operator_view.rs");
include!("publish.rs");
include!("effective_state.rs");
include!("network_reparent.rs");
include!("network_bandwidth.rs");
include!("runtime_squash.rs");
include!("validation_export.rs");

#[cfg(test)]
mod tests;

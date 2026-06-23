//! Shared topology runtime domain logic for attachment health and effective topology.

#![warn(missing_docs)]

mod runtime;

use anyhow::{Context, Result};
use lqos_config::{
    CircuitAnchor, CircuitAnchorsFile, Config, ConfigShapedDevices, TOPOLOGY_ATTACHMENT_AUTO_ID,
    TopLevelPlannerItem, TopLevelPlannerMode, TopLevelPlannerParams, TopologyAllowedParent,
    TopologyAttachmentHealthStateFile, TopologyAttachmentHealthStatus, TopologyAttachmentOption,
    TopologyAttachmentRateSource, TopologyAttachmentRole, TopologyCanonicalIngressKind,
    TopologyCanonicalNode, TopologyCanonicalRateInputSource, TopologyCanonicalStateFile,
    TopologyEditorNode, TopologyEditorStateFile, TopologyEffectiveAttachmentState,
    TopologyEffectiveNodeState, TopologyEffectiveStateFile, TopologyQueueVisibilityPolicy,
    TopologyRuntimeStatusFile, TopologyShapingCircuitInput, TopologyShapingDeviceInput,
    TopologyShapingInputsFile, TopologyShapingResolutionSource, circuit_anchors_path,
    compute_effective_network_generation, detect_shaping_cpus, plan_top_level_assignments,
    topology_effective_network_path, topology_effective_state_path, topology_runtime_status_path,
    topology_shaping_inputs_path,
};
use lqos_overrides::{
    CircuitAdjustment, NetworkAdjustment, OverrideStore, TopologyAttachmentMode,
    TopologyOverridesFile,
};
use lqos_topology_compile::{TopologyCompiledShapingFile, TopologyImportFile};
use lqos_utils::process_lock::{ProcessFileLock, ProcessLockConfig};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet, hash_map::Entry};
use std::fs::File;
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;

pub use runtime::start_topology_thread;

include!("common.rs");
include!("shaping_inputs.rs");
include!("flat_mode.rs");
include!("queue_virtualization.rs");
include!("artifacts.rs");
include!("publish.rs");
include!("effective_state.rs");
include!("network_reparent.rs");
include!("network_bandwidth.rs");
include!("runtime_squash.rs");
include!("validation_export.rs");

#[cfg(test)]
mod tests;

//! The `lqos_config` crate stores and handles LibreQoS configuration.
//! Configuration is drawn from:
//! * The `ispConfig.py` file.
//! * The `/etc/lqos.conf` file.
//! * `ShapedDevices.csv` files.
//! * `network.json` files.
//! * `topology_import.json` files for compiler-backed integration ingress.
//! * `topology_compiled_shaping.json` files for compiler-selected integration shaping ingress.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]
pub mod authentication;
mod circuit_anchors;
mod circuit_ethernet_metadata;
mod cpu_topology;
mod dynamic_circuits;
mod etc;
mod ethernet_port_limits;
mod hex_encoding;
mod mikrotik_ipv6_credentials;
mod network_json;
mod planner;
mod program_control;
mod qoo_profiles;
mod runtime_state_migration;
pub use runtime_state_migration::repair_runtime_state;
mod shaped_devices;
mod topology_canonical_state;
mod topology_operator_view;
mod topology_editor_state;
mod topology_parent_candidates;
mod topology_runtime_state;
mod tunnel_circuits;

#[cfg(test)]
mod mode_flag_retirement_gate;

pub use authentication::{AuthenticatedUser, UserRole, WebUser, WebUsers};
pub use circuit_anchors::{
    CIRCUIT_ANCHORS_FILENAME, CircuitAnchor, CircuitAnchorsError, CircuitAnchorsFile,
    circuit_anchors_path,
};
pub use circuit_ethernet_metadata::{
    CIRCUIT_ETHERNET_METADATA_FILENAME, CircuitEthernetMetadata, CircuitEthernetMetadataFile,
    EthernetCapTargetKind, circuit_ethernet_metadata_path,
};
pub use cpu_topology::{
    CpuListParseError, ShapingCpuDetection, ShapingCpuSource, detect_shaping_cpus,
};
pub use dynamic_circuits::{
    DYNAMIC_CIRCUITS_FILENAME, DynamicCircuit, DynamicCircuitsFile, dynamic_circuits_path,
};
pub use etc::{
    BridgeConfig, Config, DEFAULT_ANALYTICS_SOCKET, DEFAULT_CONFIG_PATH, DataplaneBridgeConfig,
    DynamicCircuitRangeRule, DynamicCircuitsConfig, LocalApiKeyConfig,
    MAX_LOCAL_API_KEYS, MikrotikIpv6Config, PowerConfig, PowerProfile, QueueMode,
    RadiusAccountingClient,
    RadiusAccountingConfig, RadiusClientSource, RadiusDynamicCircuitApplicationConfig,
    RadiusFallbackSpeedProfile, RadiusSharedSecretSource, RateProfileValidationError,
    RttThresholds, SingleInterfaceConfig, SslConfig, StormguardConfig, StormguardStrategy,
    MaintenanceWindowConfig, TopologyConfig, TopologySource, TopologySourceConflict,
    TopologySourceOrigin, TopologySourceRuntime, Tunables, clear_cached_config, config_path,
    topology_source_is, validate_topology_selection,
    load_config, normalize_external_hostname, update_config, validate_rate_profile_mbps,
};
pub use ethernet_port_limits::{
    DEFAULT_ETHERNET_PORT_LIMIT_MULTIPLIER, EthernetPortLimitPolicy, EthernetPortObservation,
    EthernetRateDecision, RequestedCircuitRates, apply_ethernet_rate_cap, usable_ethernet_cap_mbps,
};
pub use mikrotik_ipv6_credentials::{
    MikrotikIpv6CredentialError, MikrotikIpv6CredentialsFile, MikrotikIpv6RouterCredential,
    load_mikrotik_ipv6_router_credentials, migrate_legacy_mikrotik_ipv6_credentials,
};
pub use network_json::{NetworkJson, NetworkJsonNode, NetworkJsonTransport};
pub use planner::{
    CircuitIdentityAssignment, CircuitIdentityGroupInput, ClassIdentityPlannerConstraints,
    ClassIdentityPlannerOutput, PlannerCircuitIdentityState, PlannerMinorReservations,
    PlannerSiteIdentityState, SiteIdentityAssignment, SiteIdentityInput, TopLevelPlannerItem,
    TopLevelPlannerMode, TopLevelPlannerOutput, TopLevelPlannerParams,
    build_class_identity_reservations, plan_class_identities,
    plan_class_identities_with_constraints, plan_top_level_assignments,
};
pub use program_control::{RELOAD_REQUEST_ACK, reload_request_path, request_libreqos_reload};
pub use qoo_profiles::{
    DEFAULT_QOO_PROFILE_ID, QooProfileInfo, QooProfilesError, active_qoo_profile,
    list_qoo_profiles, load_qoo_profiles_file,
};
pub use shaped_devices::{ConfigShapedDevices, ShapedDevice};
pub use topology_canonical_state::{
    deduped_display_names, validate_topology_nodes, TOPOLOGY_CANONICAL_STATE_FILENAME, TopologyCanonicalIngressKind,
    TopologyCanonicalNode, TopologyCanonicalRateInput, TopologyCanonicalRateInputSource,
    TopologyCanonicalStateError, TopologyCanonicalStateFile, repair_stale_topology_state,
    topology_canonical_state_path, topology_ingress_identity_from_tokens,
};
pub use topology_operator_view::{
    topology_operator_view_path, TOPOLOGY_OPERATOR_VIEW_FILENAME, OperatorRateSource,
    OperatorViewDevice, OperatorViewNode, TopologyOperatorViewFile,
};
pub use topology_editor_state::{
    TOPOLOGY_ATTACHMENT_AUTO_ID, TOPOLOGY_EDITOR_STATE_FILENAME, TopologyAllowedParent,
    TopologyAttachmentHealthStatus, TopologyAttachmentOption, TopologyAttachmentRateSource,
    TopologyAttachmentRole, TopologyEditorNode, TopologyEditorStateError, TopologyEditorStateFile,
    topology_auto_attachment_option, topology_editor_state_path,
};
pub use topology_parent_candidates::{
    TOPOLOGY_PARENT_CANDIDATES_FILENAME, TopologyParentCandidate, TopologyParentCandidatesError,
    TopologyParentCandidatesFile, TopologyParentCandidatesNode, topology_parent_candidates_path,
};
pub use tunnel_circuits::{
    TUNNEL_CIRCUITS_FILENAME, TUNNEL_CIRCUITS_MAX_BYTES, TUNNEL_CIRCUITS_MAX_ENTRIES,
    TUNNEL_CIRCUITS_SCHEMA_VERSION, TUNNEL_CIRCUITS_SNAPSHOT_FILENAME, TUNNEL_CIRCUIT_ID_PREFIX,
    TUNNEL_PLAN_MBPS_MAX, TUNNEL_SITE_ID_PREFIX, TUNNEL_VLAN_MAX, TUNNEL_VNI_MAX,
    TunnelCircuitEntry, TunnelCircuitsError, TunnelCircuitsFile, tunnel_circuit_id,
    tunnel_circuits_input_path, tunnel_circuits_snapshot_path, tunnel_site_node_id,
};
#[allow(deprecated)]
pub use topology_runtime_state::{
    TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME, TOPOLOGY_COMPILED_SHAPING_FILENAME,
    TOPOLOGY_EFFECTIVE_NETWORK_FILENAME, TOPOLOGY_EFFECTIVE_STATE_FILENAME,
    TOPOLOGY_IMPORT_FILENAME, TOPOLOGY_RUNTIME_STATUS_FILENAME, TOPOLOGY_SHAPING_INPUTS_FILENAME,
    TOPOLOGY_DATA_PLANE_TRANSACTION_FILENAME,
    TopologyAttachmentEndpointStatus, TopologyAttachmentHealthEntry,
    TopologyAttachmentHealthStateFile, TopologyCircuitMatchIdentity, TopologyCircuitMatchIndex,
    TopologyEffectiveAttachmentState,
    TopologyEffectiveNodeState, TopologyEffectiveStateFile, TopologyRuntimeShapingPayloadIdentity,
    TopologyRuntimeStateError, TopologyRuntimeStatusFile, TopologyShapingCircuitInput,
    TopologyShapingDeviceInput, TopologyShapingInputsFile, TopologyShapingResolutionSource,
    TunnelIdentity,
    active_runtime_shaping_inputs_path, active_runtime_shaping_inputs_path_from_status,
    compute_effective_network_file_generation, compute_effective_network_generation,
    compute_topology_source_generation,
    load_active_runtime_shaping_inputs, load_active_runtime_shaping_inputs_from_status,
    load_shaping_inputs_at_path,
    topology_attachment_health_state_path, topology_compiled_shaping_path,
    topology_dataplane_transaction_path, topology_effective_network_path,
    topology_effective_state_path, topology_import_path, topology_runtime_status_path,
    topology_shaping_inputs_path, validated_runtime_shaping_inputs_path,
};

/// T1 named legacy escape (t1_tree_repoint_derivation T1b): when true, the
/// dashboard tree store is populated from the legacy `network.json` instead
/// of the v3 compiler artifact. Pure: loads config, no side effects beyond
/// that read.
pub fn legacy_network_json_tree_enabled() -> bool {
    crate::load_config()
        .map(|cfg| cfg.legacy_network_json_tree)
        .unwrap_or(false)
}

/// Returns whether the current integration import artifact advertises any shaped devices.
///
/// This is a lightweight readiness predicate used by migration and runtime callers that need to
/// know whether integration publication has taken over from legacy manual files.
///
/// This function is pure: it has no side effects.
pub fn topology_import_has_shaped_devices(config: &Config) -> bool {
    let import_path = config.topology_state_file_path(TOPOLOGY_IMPORT_FILENAME);
    let Ok(raw) = std::fs::read_to_string(&import_path) else {
        return false;
    };
    let Ok(decoded) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };

    decoded
        .get("imported")
        .and_then(|imported| imported.get("shaped_devices"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|devices| !devices.is_empty())
}

/// Used as a constant in determining buffer preallocation
pub const SUPPORTED_CUSTOMERS: usize = 100_000;

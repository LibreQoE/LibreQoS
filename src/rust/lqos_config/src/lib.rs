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
mod etc;
mod ethernet_port_limits;
mod mikrotik_ipv6_credentials;
mod network_json;
mod planner;
mod program_control;
mod qoo_profiles;
mod runtime_state_migration;
mod shaped_devices;
mod topology_canonical_state;
mod topology_editor_state;
mod topology_parent_candidates;
mod topology_runtime_state;

pub use authentication::{AuthenticatedUser, UserRole, WebUser, WebUsers};
pub use circuit_anchors::{
    CIRCUIT_ANCHORS_FILENAME, CircuitAnchor, CircuitAnchorsError, CircuitAnchorsFile,
    circuit_anchors_path,
};
pub use circuit_ethernet_metadata::{
    CIRCUIT_ETHERNET_METADATA_FILENAME, CircuitEthernetMetadata, CircuitEthernetMetadataFile,
    circuit_ethernet_metadata_path,
};
pub use cpu_topology::{
    CpuListParseError, ShapingCpuDetection, ShapingCpuSource, detect_shaping_cpus,
};
pub use etc::{
    BridgeConfig, Config, DynamicCircuitRangeRule, DynamicCircuitsConfig, LazyQueueMode, QueueMode,
    MikrotikIpv6Config, RttThresholds, SingleInterfaceConfig, StormguardConfig,
    StormguardStrategy, TopologyConfig,
    TreeguardCircuitsConfig, TreeguardConfig, TreeguardCpuConfig, TreeguardCpuMode,
    TreeguardLinksConfig, TreeguardQooConfig, Tunables, clear_cached_config, disable_xdp_bridge,
    enable_long_term_stats, load_config, treeguard_cpu_mode_migration_notice, update_config,
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
pub use program_control::load_libreqos;
pub use qoo_profiles::{
    DEFAULT_QOO_PROFILE_ID, QooProfileInfo, QooProfilesError, active_qoo_profile,
    list_qoo_profiles, load_qoo_profiles_file,
};
pub use shaped_devices::{ConfigShapedDevices, ShapedDevice};
pub use topology_canonical_state::{
    TOPOLOGY_CANONICAL_STATE_FILENAME, TopologyCanonicalIngressKind, TopologyCanonicalNode,
    TopologyCanonicalRateInput, TopologyCanonicalRateInputSource, TopologyCanonicalStateError,
    TopologyCanonicalStateFile, topology_canonical_state_path,
    topology_ingress_identity_from_tokens,
};
pub use topology_editor_state::{
    TOPOLOGY_ATTACHMENT_AUTO_ID, TOPOLOGY_EDITOR_STATE_FILENAME, TopologyAllowedParent,
    TopologyAttachmentHealthStatus, TopologyAttachmentOption, TopologyAttachmentRateSource,
    TopologyAttachmentRole, TopologyEditorNode, TopologyEditorStateError, TopologyEditorStateFile,
    TopologyQueueVisibilityPolicy, topology_editor_state_path,
};
pub use topology_parent_candidates::{
    TOPOLOGY_PARENT_CANDIDATES_FILENAME, TopologyParentCandidate, TopologyParentCandidatesError,
    TopologyParentCandidatesFile, TopologyParentCandidatesNode, topology_parent_candidates_path,
};
pub use topology_runtime_state::{
    TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME, TOPOLOGY_COMPILED_SHAPING_FILENAME,
    TOPOLOGY_EFFECTIVE_NETWORK_FILENAME, TOPOLOGY_EFFECTIVE_STATE_FILENAME,
    TOPOLOGY_IMPORT_FILENAME, TOPOLOGY_RUNTIME_STATUS_FILENAME, TOPOLOGY_SHAPING_INPUTS_FILENAME,
    TopologyAttachmentEndpointStatus, TopologyAttachmentHealthEntry,
    TopologyAttachmentHealthStateFile, TopologyEffectiveAttachmentState,
    TopologyEffectiveNodeState, TopologyEffectiveStateFile, TopologyRuntimeStateError,
    TopologyRuntimeStatusFile, TopologyShapingCircuitInput, TopologyShapingDeviceInput,
    TopologyShapingInputsFile, TopologyShapingResolutionSource,
    compute_effective_network_generation, compute_topology_source_generation,
    topology_attachment_health_state_path, topology_compiled_shaping_path,
    topology_effective_network_path, topology_effective_state_path, topology_import_path,
    topology_runtime_status_path, topology_shaping_inputs_path,
};

/// Used as a constant in determining buffer preallocation
pub const SUPPORTED_CUSTOMERS: usize = 100_000;

//! The `lqos_config` crate stores and handles LibreQoS configuration.
//! Configuration is drawn from:
//! * The `ispConfig.py` file.
//! * The `/etc/lqos.conf` file.
//! * `ShapedDevices.csv` files.
//! * `network.json` files.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]
pub mod authentication;
mod cpu_topology;
mod etc;
mod network_json;
mod planner;
mod program_control;
mod qoo_profiles;
mod shaped_devices;

pub use authentication::{AuthenticatedUser, UserRole, WebUser, WebUsers};
pub use cpu_topology::{
    CpuListParseError, ShapingCpuDetection, ShapingCpuSource, detect_shaping_cpus,
};
pub use etc::{
    BridgeConfig, Config, LazyQueueMode, RttThresholds, SingleInterfaceConfig, StormguardConfig,
    StormguardStrategy, TreeguardCircuitsConfig, TreeguardConfig, TreeguardCpuConfig,
    TreeguardCpuMode, TreeguardLinksConfig, TreeguardQooConfig, Tunables, clear_cached_config,
    disable_xdp_bridge, enable_long_term_stats, load_config, treeguard_cpu_mode_migration_notice,
    update_config,
};
pub use network_json::{NetworkJson, NetworkJsonNode, NetworkJsonTransport};
pub use planner::{
    CircuitIdentityAssignment, CircuitIdentityGroupInput, ClassIdentityPlannerOutput,
    PlannerCircuitIdentityState, PlannerMinorReservations, PlannerSiteIdentityState,
    SiteIdentityAssignment, SiteIdentityInput, TopLevelPlannerItem, TopLevelPlannerMode,
    TopLevelPlannerOutput, TopLevelPlannerParams, build_class_identity_reservations,
    plan_class_identities, plan_class_identities_with_constraints, plan_top_level_assignments,
};
pub use program_control::load_libreqos;
pub use qoo_profiles::{
    DEFAULT_QOO_PROFILE_ID, QooProfileInfo, QooProfilesError, active_qoo_profile,
    list_qoo_profiles, load_qoo_profiles_file,
};
pub use shaped_devices::{ConfigShapedDevices, ShapedDevice};

/// Used as a constant in determining buffer preallocation
pub const SUPPORTED_CUSTOMERS: usize = 100_000;

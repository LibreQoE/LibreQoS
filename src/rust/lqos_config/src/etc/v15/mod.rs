//! Handles the 1.5.0 configuration file format.

mod top_config;
pub use top_config::Config;
pub use top_config::RttThresholds;
pub use top_config::{SslConfig, normalize_external_hostname};
mod bridge;
mod dataplane;
mod dataplane_bridge;
mod dynamic_circuits;
mod flows;
mod maintenance_window;
pub mod influxdb;
pub use maintenance_window::MaintenanceWindowConfig;
mod integration_common;
mod ip_ranges;
mod local_api;
pub use local_api::{LocalApiKeyConfig, MAX_LOCAL_API_KEYS};
mod long_term_stats;
mod mikrotik_ipv6;
mod netzur_integration;
mod power;
mod powercode_integration;
mod queues;
mod radius_accounting;
mod sonar_integration;
mod splynx_integration;
mod stormguard;
mod topology;
mod tuning;
mod uisp_integration;
mod visp_integration;
mod wispgate;

pub use bridge::*;
pub use dataplane::DataplaneConfig;
pub use dataplane_bridge::{DEFAULT_ANALYTICS_SOCKET, DataplaneBridgeConfig};
pub use dynamic_circuits::*;
pub use integration_common::IntegrationConfig;
pub use long_term_stats::LongTermStats;
pub use mikrotik_ipv6::MikrotikIpv6Config;
pub use power::{PowerConfig, PowerProfile};
pub use queues::QueueMode;
pub use radius_accounting::{
    RadiusAccountingClient, RadiusAccountingConfig, RadiusClientSource,
    RadiusDynamicCircuitApplicationConfig, RadiusFallbackSpeedProfile, RadiusSharedSecretSource,
    RateProfileValidationError, validate_rate_profile_mbps,
};
pub use stormguard::{StormguardConfig, StormguardStrategy};
pub use topology::{
    TopologyConfig, TopologySource, TopologySourceConflict, TopologySourceOrigin,
    TopologySourceRuntime, normalize_topology_compile_mode, validate_topology_selection,
};
pub use tuning::Tunables;

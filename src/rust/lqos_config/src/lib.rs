//! The `lqos_config` crate stores and handles LibreQoS configuration.
//! Configuration is drawn from:
//! * The `ispConfig.py` file.
//! * The `/etc/lqos.conf` file.
//! * `ShapedDevices.csv` files.
//! * `network.json` files.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]
pub mod authentication;
mod etc;
mod network_json;
mod program_control;
mod qoo_profiles;
mod shaped_devices;

pub use authentication::{UserRole, WebUser, WebUsers};
pub use etc::{
    BridgeConfig, Config, LazyQueueMode, SingleInterfaceConfig, StormguardConfig, Tunables,
    disable_xdp_bridge, enable_long_term_stats, load_config, update_config,
};
pub use network_json::{NetworkJson, NetworkJsonNode, NetworkJsonTransport};
pub use program_control::load_libreqos;
pub use qoo_profiles::{
    DEFAULT_QOO_PROFILE_ID, QooProfileInfo, QooProfilesError, active_qoo_profile,
    list_qoo_profiles, load_qoo_profiles_file,
};
pub use shaped_devices::{ConfigShapedDevices, ShapedDevice};

/// Used as a constant in determining buffer preallocation
pub const SUPPORTED_CUSTOMERS: usize = 100_000;

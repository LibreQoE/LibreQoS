//! The `lqos_config` crate stores and handles LibreQoS configuration.
//! Configuration is drawn from:
//! * The `ispConfig.py` file.
//! * The `/etc/lqos.conf` file.
//! * `ShapedDevices.csv` files.
//! * `network.json` files.

#![warn(missing_docs)]
mod authentication;
mod etc;
mod network_json;
mod program_control;
mod shaped_devices;

pub use authentication::{UserRole, WebUsers};
pub use etc::{load_config, Config, enable_long_term_stats, Tunables, BridgeConfig, update_config, disable_xdp_bridge};
pub use network_json::{NetworkJson, NetworkJsonNode, NetworkJsonTransport};
pub use program_control::load_libreqos;
pub use shaped_devices::{ConfigShapedDevices, ShapedDevice};

/// Used as a constant in determining buffer preallocation
pub const SUPPORTED_CUSTOMERS: usize = 16_000_000;

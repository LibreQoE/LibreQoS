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
mod shaped_devices;

pub use authentication::{UserRole, WebUser, WebUsers};
pub use etc::{
    BridgeConfig, Config, Tunables, disable_xdp_bridge, enable_long_term_stats, load_config,
    update_config, LazyQueueMode, SingleInterfaceConfig, StormguardConfig, SandwichMode, SandwichRateLimiter,
};
pub use network_json::{NetworkJson, NetworkJsonNode, NetworkJsonTransport};
pub use program_control::load_libreqos;
pub use shaped_devices::{ConfigShapedDevices, ShapedDevice};

/// Used as a constant in determining buffer preallocation
pub const SUPPORTED_CUSTOMERS: usize = 100_000;

/// The name of the veth interface facing the Internet in sandwich mode
pub const SANDWICH_TO_INTERNET: &str = "veth_lq_inet";
/// The name of the veth interface facing the ISP in sandwich mode
pub const SANDWICH_TO_NETWORK: &str = "veth_lq_isp";
/// The name of the bridge facing the Internet in sandwich mode
pub const BRIDGE_TO_INTERNET: &str = "br_lq_inet";
/// The name of the bridge facing the ISP in sandwich mode
pub const BRIDGE_TO_NETWORK: &str = "br_lq_isp";
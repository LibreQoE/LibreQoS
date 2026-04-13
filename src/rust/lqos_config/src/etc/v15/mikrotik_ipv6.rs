use allocative::Allocative;
use serde::{Deserialize, Serialize};

fn default_mikrotik_ipv6_config_path() -> String {
    "/etc/libreqos/mikrotik_ipv6.toml".to_string()
}

/// Dedicated Mikrotik IPv6 enrichment secrets/config file location.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct MikrotikIpv6Config {
    /// Path to the Mikrotik IPv6 enrichment router credential file.
    #[serde(default = "default_mikrotik_ipv6_config_path")]
    pub config_path: String,
}

impl Default for MikrotikIpv6Config {
    fn default() -> Self {
        Self {
            config_path: default_mikrotik_ipv6_config_path(),
        }
    }
}

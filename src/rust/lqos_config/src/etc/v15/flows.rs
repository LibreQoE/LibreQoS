//! Provides netflow support for tracking network flows.
//!
//! You can enable them by adding a `[flows]` section to your configuration file.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct FlowConfig {
    pub netflow_enabled: bool,
    pub netflow_port: Option<u16>,
    pub netflow_ip: Option<String>,
    pub netflow_version: Option<u8>,
    /// §`lqos-74t.31` W1: CIDRs whose flows/RTT are NOT tracked (the v2
    /// "Do-Not-Track Subnets" set — e.g. speed-test / monitoring hosts). The
    /// reader applies it to the live DPDK flow/RTT rollups + the hosted-Insight
    /// `ip_policies`. Empty/`None` ⇒ track everything.
    #[serde(default)]
    pub do_not_track_subnets: Option<Vec<String>>,
}

impl Default for FlowConfig {
    fn default() -> Self {
        Self {
            netflow_enabled: false,
            netflow_port: None,
            netflow_ip: None,
            netflow_version: None,
            do_not_track_subnets: None,
        }
    }
}

impl FlowConfig {
    /// The configured do-not-track CIDR strings (empty when unset).
    pub fn do_not_track_cidrs(&self) -> &[String] {
        self.do_not_track_subnets.as_deref().unwrap_or(&[])
    }

    /// `lqos-b4w`: validate the `[flows]` section at config load. The NetFlow
    /// exporter can emit v5 or v9; any OTHER `netflow_version` is a LOUD load
    /// error — never the historical silent upgrade to v9. `None` keeps the v9
    /// default (so an omitted version stays valid).
    pub fn validate(&self) -> Result<(), String> {
        if let Some(version) = self.netflow_version
            && version != 5
            && version != 9
        {
            return Err(format!(
                "flows.netflow_version must be 5 or 9 (got {version})"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_the_supported_versions_and_none() {
        for version in [None, Some(5), Some(9)] {
            let config = FlowConfig {
                netflow_version: version,
                ..Default::default()
            };
            assert!(config.validate().is_ok(), "netflow_version={version:?}");
        }
    }

    #[test]
    fn validate_rejects_an_unsupported_version() {
        for version in [0u8, 1, 6, 10, 255] {
            let config = FlowConfig {
                netflow_version: Some(version),
                ..Default::default()
            };
            assert!(
                config.validate().is_err(),
                "netflow_version={version} must be rejected at load"
            );
        }
    }
}

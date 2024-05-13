//! Provides netflow support for tracking network flows.
//!
//! You can enable them by adding a `[flows]` section to your configuration file.

use serde::{Serialize, Deserialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct FlowConfig {
    pub flow_timeout_seconds: u64,
    pub netflow_enabled: bool,
    pub netflow_port: Option<u16>,
    pub netflow_ip: Option<String>,
    pub netflow_version: Option<u8>,
    pub do_not_track_subnets: Option<Vec<String>>,
}

impl Default for FlowConfig {
    fn default() -> Self {
        Self {
            flow_timeout_seconds: 30,
            netflow_enabled: false,
            netflow_port: None,
            netflow_ip: None,
            netflow_version: None,
            do_not_track_subnets: None,
        }
    }
}

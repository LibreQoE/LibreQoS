//! Common integration variables, shared between integrations

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct IntegrationConfig {
    /// Replace names with addresses?
    pub circuit_name_as_address: bool,

    /// Always overwrite network.json?
    pub always_overwrite_network_json: bool,

    /// Queue refresh interval in minutes
    pub queue_refresh_interval_mins: u32,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            circuit_name_as_address: false,
            always_overwrite_network_json: false,
            queue_refresh_interval_mins: 30,
        }
    }
}
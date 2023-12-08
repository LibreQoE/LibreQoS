//! Common integration variables, shared between integrations

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct IntegrationConfig {
    /// Replace names with addresses?
    pub circuit_name_as_address: bool,

    /// Always overwrite network.json?
    pub always_overwrite_network_json: bool,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            circuit_name_as_address: false,
            always_overwrite_network_json: false,
        }
    }
}
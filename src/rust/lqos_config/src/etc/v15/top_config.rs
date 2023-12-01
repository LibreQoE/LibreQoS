//! Top-level configuration file for LibreQoS.

use super::anonymous_stats::UsageStats;
use super::tuning::Tunables;
use serde::{Deserialize, Serialize};
use sha2::digest::Update;
use sha2::Digest;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Version number for the configuration file.
    /// This will be set to "1.5". Versioning will make
    /// it easier to handle schema updates moving forward.
    pub version: String,

    /// Directory in which LibreQoS is installed
    pub lqos_directory: String,

    /// Node ID - uniquely identifies this shaper.
    pub node_id: String,

    /// Node name - human-readable name for this shaper.
    pub node_name: String,    

    /// Anonymous usage statistics
    pub usage_stats: UsageStats,

    /// Tuning instructions
    pub tuning: Tunables,

    /// Bridge configuration
    pub bridge: Option<super::bridge::BridgeConfig>,

    /// Single-interface configuration
    pub single_interface: Option<super::bridge::SingleInterfaceConfig>,

    /// Queue Definition data (originally from ispConfig.py)
    pub queues: super::queues::QueueConfig,

    /// Long-term stats configuration
    pub long_term_stats: super::long_term_stats::LongTermStats,

    /// IP Range definitions
    pub ip_ranges: super::ip_ranges::IpRanges,

    /// Integration Common Variables
    pub integration_common: super::integration_common::IntegrationConfig,
}

impl Config {
    /// Calculate a node ID based on the machine ID. If Machine ID is unavailable,
    /// generate a random UUID.
    pub fn calculate_node_id() -> String {
        if let Ok(machine_id) = std::fs::read_to_string("/etc/machine-id") {
            let hash = sha2::Sha256::new().chain(machine_id).finalize();
            format!("{:x}", hash)
        } else {
            Uuid::new_v4().to_string()
        }
    }

    /// Test is a configuration is valid.
    pub fn validate(&self) -> Result<(), String> {
        if self.bridge.is_some() && self.single_interface.is_some() {
            return Err(
                "Configuration file may not contain both a bridge and a single-interface section."
                    .to_string(),
            );
        }
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: "1.5".to_string(),
            lqos_directory: "/opt/libreqos/src".to_string(),
            node_id: Self::calculate_node_id(),
            node_name: "LibreQoS".to_string(),            
            usage_stats: UsageStats::default(),
            tuning: Tunables::default(),
            bridge: Some(super::bridge::BridgeConfig::default()),
            single_interface: None,
            queues: super::queues::QueueConfig::default(),
            long_term_stats: super::long_term_stats::LongTermStats::default(),
            ip_ranges: super::ip_ranges::IpRanges::default(),
            integration_common: super::integration_common::IntegrationConfig::default(),
        }
    }
}

//! Top-level configuration file for LibreQoS.

use super::anonymous_stats::UsageStats;
use super::tuning::Tunables;
use serde::{Deserialize, Serialize};
use sha2::digest::Update;
use sha2::Digest;
use uuid::Uuid;

/// Top-level configuration file for LibreQoS.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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

    /// Packet capture time
    pub packet_capture_time: usize,

    /// Queue refresh interval
    pub queue_check_period_ms: u64,

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

    /// Network flows configuration
    pub flows: Option<super::flows::FlowConfig>,

    /// Integration Common Variables
    pub integration_common: super::integration_common::IntegrationConfig,

    /// Spylnx Integration
    pub spylnx_integration: super::spylnx_integration::SplynxIntegration,

    /// UISP Integration
    pub uisp_integration: super::uisp_integration::UispIntegration,

    /// Powercode Integration
    pub powercode_integration: super::powercode_integration::PowercodeIntegration,

    /// Sonar Integration
    pub sonar_integration: super::sonar_integration::SonarIntegration,

    /// InfluxDB Configuration
    pub influxdb: super::influxdb::InfluxDbConfig,
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
        if self.version.trim() != "1.5" {
            return Err(format!(
                "Configuration file is at version [{}], but this version of lqos only supports version 1.5.0",
                self.version
            ));
        }
        if self.node_id.is_empty() {
            return Err("Node ID must be set".to_string());
        }
        Ok(())
    }

    /// Loads a config file from a string (used for testing only)
    #[allow(dead_code)]
    pub fn load_from_string(s: &str) -> Result<Self, String> {
        let config: Config = toml::from_str(s).map_err(|e| format!("Error parsing config: {}", e))?;
        config.validate()?;
        Ok(config)
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
            spylnx_integration: super::spylnx_integration::SplynxIntegration::default(),
            uisp_integration: super::uisp_integration::UispIntegration::default(),
            powercode_integration: super::powercode_integration::PowercodeIntegration::default(),
            sonar_integration: super::sonar_integration::SonarIntegration::default(),
            influxdb: super::influxdb::InfluxDbConfig::default(),
            packet_capture_time: 10,
            queue_check_period_ms: 1000,
            flows: None,
        }
    }
}

impl Config {
    /// Calculate the unterface facing the Internet
    pub fn internet_interface(&self) -> String {
        if let Some(bridge) = &self.bridge {
            bridge.to_internet.clone()
        } else if let Some(single_interface) = &self.single_interface {
            single_interface.interface.clone()
        } else {
            panic!("No internet interface configured")
        }
    }

    /// Calculate the interface facing the ISP
    pub fn isp_interface(&self) -> String {
        if let Some(bridge) = &self.bridge {
            bridge.to_network.clone()
        } else if let Some(single_interface) = &self.single_interface {
            single_interface.interface.clone()
        } else {
            panic!("No ISP interface configured")
        }
    }

    /// Are we in single-interface mode?
    pub fn on_a_stick_mode(&self) -> bool {
        self.bridge.is_none()
    }

    /// Get the VLANs for the stick interface
    pub fn stick_vlans(&self) -> (u32, u32) {
        if let Some(stick) = &self.single_interface {
            (stick.network_vlan, stick.internet_vlan)
        } else {
            (0, 0)
        }
    }
}

#[cfg(test)]
mod test {
    use super::Config;

    #[test]
    fn load_example() {
        let config = Config::load_from_string(include_str!("example.toml")).unwrap();
        assert_eq!(config.version, "1.5");
    }
}
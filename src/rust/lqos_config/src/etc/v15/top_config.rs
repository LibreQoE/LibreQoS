//! Top-level configuration file for LibreQoS.

use super::tuning::Tunables;
use crate::etc::v15::stormguard;
use allocative::Allocative;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use sha2::digest::Update;
use toml_edit::DocumentMut;
use uuid::Uuid;

fn default_true() -> bool {
    true
}

/// Top-level configuration file for LibreQoS.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
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

    /// Optional QoO profile id (loaded from `qoo_profiles.json`) used for QoO/QoQ scoring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qoo_profile_id: Option<String>,

    /// Packet capture time
    pub packet_capture_time: usize,

    /// Queue refresh interval
    pub queue_check_period_ms: u64,

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

    /// Splynx Integration configuration. Optional so older configs without this
    /// section still deserialize cleanly.
    #[serde(default)]
    pub splynx_integration: super::splynx_integration::SplynxIntegration,

    /// Netzur Integration configuration. Optional so older configs without this
    /// section still deserialize cleanly.
    pub netzur_integration: Option<super::netzur_integration::NetzurIntegration>,

    /// VISP Integration configuration. Optional so older configs without this
    /// section still deserialize cleanly.
    pub visp_integration: Option<super::visp_integration::VispIntegration>,

    /// UISP Integration
    pub uisp_integration: super::uisp_integration::UispIntegration,

    /// Powercode Integration
    pub powercode_integration: super::powercode_integration::PowercodeIntegration,

    /// Sonar Integration
    pub sonar_integration: super::sonar_integration::SonarIntegration,

    /// InfluxDB Configuration
    pub influxdb: Option<super::influxdb::InfluxDbConfig>,

    /// WispGate Integration
    pub wispgate_integration: Option<super::wispgate::WispGateIntegration>,

    /// Option to disable the webserver for headless/CLI operation
    pub disable_webserver: Option<bool>,

    /// Listen options for the webserver
    pub webserver_listen: Option<String>,

    /// Support for Tornado/Auto-rate.
    pub stormguard: Option<stormguard::StormguardConfig>,

    /// Disable ICMP Ping Monitoring for Devices in the hosts view
    pub disable_icmp_ping: Option<bool>,

    /// Enable per-circuit TemporalHeatmap collection.
    #[serde(default = "default_true")]
    pub enable_circuit_heatmaps: bool,

    /// Enable per-site TemporalHeatmap collection.
    #[serde(default = "default_true")]
    pub enable_site_heatmaps: bool,

    /// Enable per-ASN TemporalHeatmap collection.
    #[serde(default = "default_true")]
    pub enable_asn_heatmaps: bool,
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
        // Validate that default_sqm is not empty to prevent incomplete TC commands
        if self.queues.default_sqm.trim().is_empty() {
            return Err("default_sqm cannot be empty. Please specify a qdisc type (e.g., 'cake diffserv4' or 'fq_codel')".to_string());
        }
        Ok(())
    }

    /// Loads a config file from a string (used for testing only)
    #[allow(dead_code)]
    pub fn load_from_string(s: &str) -> Result<Self, String> {
        let normalized = normalize_splynx_compat_keys(s)?;
        let config: Config =
            toml::from_str(&normalized).map_err(|e| format!("Error parsing config: {}", e))?;
        config.validate()?;
        Ok(config)
    }
}

/// Normalizes historical misspellings of Splynx keys in the TOML configuration.
///
/// This operates purely in-memory so existing installations don't have their `/etc/lqos.conf`
/// rewritten just by upgrading. The canonical schema uses:
/// - `[splynx_integration]`
/// - `enable_splynx = true/false`
///
/// Compatibility shims accepted:
/// - `[spylnx_integration]`
/// - `enable_spylnx = true/false`
fn normalize_splynx_compat_keys(raw: &str) -> Result<String, String> {
    let mut doc = raw
        .parse::<DocumentMut>()
        .map_err(|e| format!("Error parsing config: {}", e))?;

    // Section rename: [spylnx_integration] -> [splynx_integration]
    if doc.get("splynx_integration").is_none() {
        if let Some(item) = doc.remove("spylnx_integration") {
            doc.insert("splynx_integration", item);
        }
    } else if doc.get("spylnx_integration").is_some() {
        // If both exist, prefer the canonical section.
        doc.remove("spylnx_integration");
    }

    // Key rename inside the section: enable_spylnx -> enable_splynx
    if let Some(table) = doc
        .get_mut("splynx_integration")
        .and_then(|item| item.as_table_mut())
    {
        if table.get("enable_splynx").is_none() {
            if let Some(item) = table.remove("enable_spylnx") {
                table.insert("enable_splynx", item);
            }
        } else if table.get("enable_spylnx").is_some() {
            // If both exist, prefer the canonical key.
            table.remove("enable_spylnx");
        }
    }

    Ok(doc.to_string())
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: "1.5".to_string(),
            lqos_directory: "/opt/libreqos/src".to_string(),
            node_id: Self::calculate_node_id(),
            node_name: "LibreQoS".to_string(),
            qoo_profile_id: None,
            tuning: Tunables::default(),
            bridge: Some(super::bridge::BridgeConfig::default()),
            single_interface: None,
            queues: super::queues::QueueConfig::default(),
            long_term_stats: super::long_term_stats::LongTermStats::default(),
            ip_ranges: super::ip_ranges::IpRanges::default(),
            integration_common: super::integration_common::IntegrationConfig::default(),
            splynx_integration: super::splynx_integration::SplynxIntegration::default(),
            netzur_integration: Some(super::netzur_integration::NetzurIntegration::default()),
            visp_integration: Some(super::visp_integration::VispIntegration::default()),
            uisp_integration: super::uisp_integration::UispIntegration::default(),
            powercode_integration: super::powercode_integration::PowercodeIntegration::default(),
            sonar_integration: super::sonar_integration::SonarIntegration::default(),
            wispgate_integration: None,
            influxdb: None,
            packet_capture_time: 10,
            queue_check_period_ms: 1000,
            flows: None,
            disable_webserver: None,
            webserver_listen: None,
            stormguard: None,
            disable_icmp_ping: Some(false),
            enable_circuit_heatmaps: true,
            enable_site_heatmaps: true,
            enable_asn_heatmaps: true,
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
        let config = Config::load_from_string(include_str!("example.toml"))
            .expect("Cannot read example toml file");
        assert_eq!(config.version, "1.5");
    }

    #[test]
    fn load_example_legacy_spylnx() {
        let legacy = include_str!("example.toml")
            .replace("[splynx_integration]", "[spylnx_integration]")
            .replace("enable_splynx", "enable_spylnx");
        let config =
            Config::load_from_string(&legacy).expect("Cannot read legacy spylnx example toml");
        assert_eq!(config.version, "1.5");
    }

    #[test]
    fn load_example_without_splynx_section() {
        let no_splynx = include_str!("example.toml")
            .lines()
            .scan(false, |in_section, line| {
                if line.trim() == "[splynx_integration]" {
                    *in_section = true;
                    return Some(None);
                }
                if *in_section && line.starts_with('[') && line.ends_with(']') {
                    *in_section = false;
                }
                if *in_section {
                    Some(None)
                } else {
                    Some(Some(line))
                }
            })
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");

        let config = Config::load_from_string(&no_splynx)
            .expect("Config without splynx section should still deserialize");
        assert!(!config.splynx_integration.enable_splynx);
    }

    #[test]
    fn serialize_uses_splynx_keys() {
        let config =
            Config::load_from_string(include_str!("example.toml")).expect("Cannot read example");
        let serialized = toml::to_string_pretty(&config).expect("Cannot serialize config");
        assert!(serialized.contains("splynx_integration"));
        assert!(!serialized.contains("spylnx_integration"));
        assert!(serialized.contains("enable_splynx"));
        assert!(!serialized.contains("enable_spylnx"));
    }
}

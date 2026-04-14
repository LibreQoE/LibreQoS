//! Top-level configuration file for LibreQoS.

use super::tuning::Tunables;
use crate::etc::v15::stormguard;
use crate::etc::v15::treeguard;
use allocative::Allocative;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use sha2::digest::Update;
use std::path::{Path, PathBuf};
use toml_edit::DocumentMut;
use uuid::Uuid;

fn default_true() -> bool {
    true
}

fn default_rtt_green_ms() -> u32 {
    0
}

fn default_rtt_yellow_ms() -> u32 {
    100
}

fn default_rtt_red_ms() -> u32 {
    200
}

/// RTT color scale thresholds (milliseconds) used by the web UI.
///
/// These represent the color "anchor points" for RTT:
/// - `green_ms`: values at/below this are green
/// - `yellow_ms`: this point is yellow
/// - `red_ms`: values at/above this are red
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct RttThresholds {
    /// RTT at/below this value (milliseconds) is colored green.
    #[serde(default = "default_rtt_green_ms")]
    pub green_ms: u32,
    /// RTT at this value (milliseconds) is colored yellow.
    #[serde(default = "default_rtt_yellow_ms")]
    pub yellow_ms: u32,
    /// RTT at/above this value (milliseconds) is colored red.
    #[serde(default = "default_rtt_red_ms")]
    pub red_ms: u32,
}

impl Default for RttThresholds {
    fn default() -> Self {
        Self {
            green_ms: default_rtt_green_ms(),
            yellow_ms: default_rtt_yellow_ms(),
            red_ms: default_rtt_red_ms(),
        }
    }
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

    /// Directory in which LibreQoS stores machine-managed runtime state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_directory: Option<String>,

    /// Node ID - uniquely identifies this shaper.
    pub node_id: String,

    /// Node name - human-readable name for this shaper.
    pub node_name: String,

    /// Optional QoO profile id (loaded from `qoo_profiles.json`) used for QoO/QoQ scoring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qoo_profile_id: Option<String>,

    /// Optional RTT thresholds used for RTT color scaling in the UI.
    ///
    /// Defaults to the executive-dashboard ramp: 0ms=green, 100ms=yellow, 200ms=red.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_thresholds: Option<RttThresholds>,

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

    /// Dynamic circuits configuration.
    ///
    /// Optional so older configs without this section still deserialize cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_circuits: Option<super::dynamic_circuits::DynamicCircuitsConfig>,

    /// Network flows configuration
    pub flows: Option<super::flows::FlowConfig>,

    /// Integration Common Variables
    #[serde(default)]
    pub integration_common: super::integration_common::IntegrationConfig,

    /// Shared topology compiler settings.
    #[serde(default)]
    pub topology: super::topology::TopologyConfig,

    /// Dedicated Mikrotik IPv6 enrichment secrets/config path.
    #[serde(default)]
    pub mikrotik_ipv6: super::mikrotik_ipv6::MikrotikIpv6Config,

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
    #[serde(default)]
    pub uisp_integration: super::uisp_integration::UispIntegration,

    /// Powercode Integration
    #[serde(default)]
    pub powercode_integration: super::powercode_integration::PowercodeIntegration,

    /// Sonar Integration
    #[serde(default)]
    pub sonar_integration: super::sonar_integration::SonarIntegration,

    /// InfluxDB Configuration
    pub influxdb: Option<super::influxdb::InfluxDbConfig>,

    /// WispGate Integration
    pub wispgate_integration: Option<super::wispgate::WispGateIntegration>,

    /// Option to disable the webserver for headless/CLI operation
    pub disable_webserver: Option<bool>,

    /// Listen options for the webserver
    pub webserver_listen: Option<String>,

    /// Controls whether an operator-provided cobrand image should be shown in the WebUI.
    #[serde(default)]
    pub display_cobrand: bool,

    /// Support for Tornado/Auto-rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stormguard: Option<stormguard::StormguardConfig>,

    /// TreeGuard intelligent node management.
    #[serde(default)]
    pub treeguard: treeguard::TreeguardConfig,

    /// Disable ICMP Ping Monitoring for Devices in the hosts view
    pub disable_icmp_ping: Option<bool>,

    /// Exclude efficiency cores (E-cores) from CPU assignment / shaping where possible.
    ///
    /// On hybrid CPUs, this tries several detection methods, caches the resolved
    /// P-core/E-core split under the LibreQoS runtime directory, and restricts
    /// shaping/XDP CPU assignment to performance cores when the split is trustworthy.
    #[serde(default = "default_true")]
    pub exclude_efficiency_cores: bool,

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
            crate::hex_encoding::encode_hex_lower(hash)
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
        if self
            .state_directory
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err("state_directory must not be empty when configured".to_string());
        }
        if self.mikrotik_ipv6.config_path.trim().is_empty() {
            return Err("mikrotik_ipv6.config_path must not be empty".to_string());
        }
        if let Some(rtt) = &self.rtt_thresholds {
            if rtt.red_ms == 0 {
                return Err("rtt_thresholds.red_ms must be > 0".to_string());
            }
            if rtt.green_ms > rtt.yellow_ms || rtt.yellow_ms > rtt.red_ms {
                return Err(
                    "rtt_thresholds must satisfy green_ms <= yellow_ms <= red_ms".to_string(),
                );
            }
        }
        if let Some(multiplier) = self.integration_common.ethernet_port_limit_multiplier
            && (!multiplier.is_finite() || multiplier <= 0.0 || multiplier > 1.0)
        {
            return Err(
                "integration_common.ethernet_port_limit_multiplier must satisfy 0 < value <= 1"
                    .to_string(),
            );
        }
        if !self.topology.compile_mode.trim().is_empty()
            && super::topology::normalize_topology_compile_mode(&self.topology.compile_mode)
                .is_none()
        {
            return Err(
                "topology.compile_mode must be one of flat, ap_only, ap_site, or full".to_string(),
            );
        }
        let airmax_flexible_frame_download_ratio =
            self.uisp_integration.airmax_flexible_frame_download_ratio;
        if !airmax_flexible_frame_download_ratio.is_finite()
            || airmax_flexible_frame_download_ratio <= 0.0
            || airmax_flexible_frame_download_ratio >= 1.0
        {
            return Err(
                "uisp_integration.airmax_flexible_frame_download_ratio must satisfy 0 < value < 1"
                    .to_string(),
            );
        }
        // Validate that default_sqm is not empty to prevent incomplete TC commands
        if self.queues.default_sqm.trim().is_empty() {
            return Err("default_sqm cannot be empty. Please specify a qdisc type (e.g., 'cake diffserv4' or 'fq_codel')".to_string());
        }
        if let Some(stormguard) = &self.stormguard {
            stormguard.validate()?;
        }
        self.treeguard.validate()?;
        if let Some(dynamic_circuits) = &self.dynamic_circuits {
            dynamic_circuits.validate()?;
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
            state_directory: Some("/opt/libreqos/state".to_string()),
            node_id: Self::calculate_node_id(),
            node_name: "LibreQoS".to_string(),
            qoo_profile_id: None,
            rtt_thresholds: None,
            tuning: Tunables::default(),
            bridge: Some(super::bridge::BridgeConfig::default()),
            single_interface: None,
            queues: super::queues::QueueConfig::default(),
            long_term_stats: super::long_term_stats::LongTermStats::default(),
            ip_ranges: super::ip_ranges::IpRanges::default(),
            dynamic_circuits: None,
            integration_common: super::integration_common::IntegrationConfig::default(),
            topology: super::topology::TopologyConfig::default(),
            mikrotik_ipv6: super::mikrotik_ipv6::MikrotikIpv6Config::default(),
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
            display_cobrand: false,
            stormguard: None,
            treeguard: treeguard::TreeguardConfig::default(),
            disable_icmp_ping: Some(false),
            exclude_efficiency_cores: true,
            enable_circuit_heatmaps: true,
            enable_site_heatmaps: true,
            enable_asn_heatmaps: true,
        }
    }
}

impl Config {
    /// Returns the resolved state-directory path for machine-managed runtime files.
    pub fn resolved_state_directory(&self) -> PathBuf {
        self.state_directory
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| Self::default_state_directory_for(&self.lqos_directory))
    }

    fn default_state_directory_for(lqos_directory: &str) -> PathBuf {
        let base = Path::new(lqos_directory);
        if base
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "src")
            && let Some(parent) = base.parent()
        {
            return parent.join("state");
        }
        base.join("state")
    }

    /// Returns the configured Mikrotik IPv6 secrets/config path.
    pub fn resolved_mikrotik_ipv6_config_path(&self) -> PathBuf {
        PathBuf::from(self.mikrotik_ipv6.config_path.trim())
    }

    /// Returns the preferred topology-state path for `filename`.
    pub fn topology_state_file_path(&self, filename: &str) -> PathBuf {
        self.resolved_state_directory()
            .join("topology")
            .join(filename)
    }

    /// Returns the preferred shaping-state path for `filename`.
    pub fn shaping_state_file_path(&self, filename: &str) -> PathBuf {
        self.resolved_state_directory()
            .join("shaping")
            .join(filename)
    }

    /// Returns the preferred stats-state path for `filename`.
    pub fn stats_state_file_path(&self, filename: &str) -> PathBuf {
        self.resolved_state_directory().join("stats").join(filename)
    }

    /// Returns the preferred cache-state path for `filename`.
    pub fn cache_state_file_path(&self, filename: &str) -> PathBuf {
        self.resolved_state_directory().join("cache").join(filename)
    }

    /// Returns the preferred debug-state path for `filename`.
    pub fn debug_state_file_path(&self, filename: &str) -> PathBuf {
        self.resolved_state_directory().join("debug").join(filename)
    }

    /// Returns the quarantine directory for stale runtime artifacts.
    pub fn quarantine_state_directory_path(&self) -> PathBuf {
        self.resolved_state_directory().join("quarantine")
    }

    /// Returns the quarantine directory for legacy runtime artifacts encountered during upgrade.
    pub fn legacy_quarantine_directory_path(&self) -> PathBuf {
        self.quarantine_state_directory_path().join("legacy")
    }

    /// Returns the legacy root path for a machine-managed runtime file.
    pub fn legacy_runtime_file_path(&self, filename: &str) -> PathBuf {
        Path::new(&self.lqos_directory).join(filename)
    }

    /// Returns the canonical read path for a topology runtime file.
    pub fn topology_state_read_path(&self, filename: &str) -> PathBuf {
        self.topology_state_file_path(filename)
    }

    /// Returns the canonical read path for a shaping runtime file.
    pub fn shaping_state_read_path(&self, filename: &str) -> PathBuf {
        self.shaping_state_file_path(filename)
    }

    /// Returns the canonical read path for a stats runtime file.
    pub fn stats_state_read_path(&self, filename: &str) -> PathBuf {
        self.stats_state_file_path(filename)
    }

    /// Returns the canonical read path for a cache runtime file.
    pub fn cache_state_read_path(&self, filename: &str) -> PathBuf {
        self.cache_state_file_path(filename)
    }

    /// Returns the explicitly configured shared topology compile mode, when set.
    pub fn shared_topology_compile_mode(&self) -> Option<&'static str> {
        super::topology::normalize_topology_compile_mode(&self.topology.compile_mode)
    }

    /// Returns the topology compile mode that UISP should use during the transition period.
    pub fn resolved_topology_compile_mode_for_uisp(&self) -> &'static str {
        self.shared_topology_compile_mode()
            .filter(|mode| {
                super::topology::integration_supports_topology_compile_mode("uisp", mode)
            })
            .or_else(|| {
                super::topology::normalize_supported_topology_compile_mode(
                    "uisp",
                    &self.uisp_integration.strategy,
                )
            })
            .unwrap_or("full")
    }

    /// Returns the topology compile mode that Splynx should use during the transition period.
    pub fn resolved_topology_compile_mode_for_splynx(&self) -> &'static str {
        self.shared_topology_compile_mode()
            .filter(|mode| {
                super::topology::integration_supports_topology_compile_mode("splynx", mode)
            })
            .or_else(|| {
                super::topology::normalize_supported_topology_compile_mode(
                    "splynx",
                    &self.splynx_integration.strategy,
                )
            })
            .unwrap_or("ap_site")
    }

    /// Returns the topology compile mode that Sonar should use.
    pub fn resolved_topology_compile_mode_for_sonar(&self) -> &'static str {
        self.shared_topology_compile_mode()
            .filter(|mode| {
                super::topology::integration_supports_topology_compile_mode("sonar", mode)
            })
            .unwrap_or("full")
    }

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
    use super::{Config, RttThresholds};

    fn remove_sections(raw: &str, sections: &[&str]) -> String {
        let mut output = Vec::new();
        let mut skip_section = false;

        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let section_name = &trimmed[1..trimmed.len() - 1];
                skip_section = sections.iter().any(|section| {
                    section_name == *section
                        || section_name.strip_prefix(&format!("{section}.")).is_some()
                });
            }

            if !skip_section {
                output.push(line);
            }
        }

        output.join("\n")
    }

    fn remove_keys(raw: &str, keys: &[&str]) -> String {
        raw.lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !keys.iter().any(|key| trimmed.starts_with(&format!("{key} =")))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

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
    fn load_example_without_integration_sections() {
        let stripped = remove_sections(
            include_str!("example.toml"),
            &[
                "integration_common",
                "splynx_integration",
                "spylnx_integration",
                "netzur_integration",
                "visp_integration",
                "uisp_integration",
                "powercode_integration",
                "sonar_integration",
                "wispgate_integration",
            ],
        );

        let config = Config::load_from_string(&stripped)
            .expect("Config without integrations should still deserialize");
        assert!(!config.splynx_integration.enable_splynx);
        assert!(!config.uisp_integration.enable_uisp);
        assert!(!config.powercode_integration.enable_powercode);
        assert!(!config.sonar_integration.enable_sonar);
    }

    #[test]
    fn sonar_mode_resolution_rejects_ap_site_and_ap_only() {
        let cfg = Config {
            topology: crate::etc::v15::TopologyConfig {
                compile_mode: "ap_site".to_string(),
                ..Default::default()
            },
            ..Config::default()
        };
        assert_eq!(cfg.resolved_topology_compile_mode_for_sonar(), "full");

        let cfg = Config {
            topology: crate::etc::v15::TopologyConfig {
                compile_mode: "ap_only".to_string(),
                ..Default::default()
            },
            ..Config::default()
        };
        assert_eq!(cfg.resolved_topology_compile_mode_for_sonar(), "full");

        let cfg = Config {
            topology: crate::etc::v15::TopologyConfig {
                compile_mode: "flat".to_string(),
                ..Default::default()
            },
            ..Config::default()
        };
        assert_eq!(cfg.resolved_topology_compile_mode_for_sonar(), "flat");
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

    #[test]
    fn tuning_cpu_governor_defaults_to_true_when_missing() {
        let raw = include_str!("example.toml")
            .lines()
            .filter(|line| !line.trim().starts_with("set_cpu_governor_performance"))
            .collect::<Vec<_>>()
            .join("\n");
        let config = Config::load_from_string(&raw)
            .expect("Config without tuning governor flag should load");
        assert!(config.tuning.set_cpu_governor_performance);
    }

    #[test]
    fn tuning_cpu_governor_explicit_false_deserializes() {
        let raw = include_str!("example.toml").replace(
            "set_cpu_governor_performance = true",
            "set_cpu_governor_performance = false",
        );
        let config =
            Config::load_from_string(&raw).expect("Config with tuning governor flag should load");
        assert!(!config.tuning.set_cpu_governor_performance);
    }

    #[test]
    fn rtt_thresholds_default_matches_executive_ramp() {
        let d = RttThresholds::default();
        assert_eq!(d.green_ms, 0);
        assert_eq!(d.yellow_ms, 100);
        assert_eq!(d.red_ms, 200);
    }

    #[test]
    fn rtt_thresholds_validation_requires_ordered() {
        let cfg = Config {
            rtt_thresholds: Some(RttThresholds {
                green_ms: 0,
                yellow_ms: 200,
                red_ms: 100,
            }),
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rtt_thresholds_validation_rejects_zero_red() {
        let cfg = Config {
            rtt_thresholds: Some(RttThresholds {
                green_ms: 0,
                yellow_ms: 0,
                red_ms: 0,
            }),
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn treeguard_defaults_match_default_on_rollout() {
        let cfg = Config::default();
        assert!(cfg.treeguard.enabled);
        assert!(!cfg.treeguard.dry_run);
        assert_eq!(
            cfg.treeguard.cpu.mode,
            crate::etc::v15::treeguard::TreeguardCpuMode::CpuAware
        );
        assert!(!cfg.treeguard.links.enabled);
        assert!(cfg.treeguard.links.all_nodes);
        assert!(!cfg.treeguard.links.top_level_auto_virtualize);
        assert!(cfg.treeguard.circuits.enabled);
        assert!(cfg.treeguard.circuits.all_circuits);
        assert_eq!(cfg.topology.queue_auto_virtualize_threshold_mbps, 5_001);
    }

    #[test]
    fn load_example_without_treeguard_section_uses_defaults() {
        let stripped = remove_sections(include_str!("example.toml"), &["treeguard"]);
        let config = Config::load_from_string(&stripped)
            .expect("Config without treeguard should still deserialize");
        assert!(config.treeguard.enabled);
        assert!(!config.treeguard.dry_run);
        assert!(config.treeguard.links.all_nodes);
        assert!(!config.treeguard.links.enabled);
        assert!(config.treeguard.circuits.all_circuits);
    }

    #[test]
    fn load_example_without_stormguard_section_deserializes() {
        let stripped = remove_sections(include_str!("example.toml"), &["stormguard"]);
        let config = Config::load_from_string(&stripped)
            .expect("Config without stormguard should still deserialize");
        assert!(config.stormguard.is_none());
    }

    #[test]
    fn display_cobrand_defaults_false_when_omitted() {
        let stripped = remove_keys(include_str!("example.toml"), &["display_cobrand"]);
        let config = Config::load_from_string(&stripped)
            .expect("Config without display_cobrand should still deserialize");
        assert!(!config.display_cobrand);
    }

    #[test]
    fn treeguard_validation_rejects_invalid_thresholds() {
        let mut cfg = Config::default();
        cfg.treeguard.cpu.cpu_low_pct = 90;
        cfg.treeguard.cpu.cpu_high_pct = 80;
        assert!(cfg.validate().is_err());

        cfg = Config::default();
        cfg.treeguard.circuits.idle_util_pct = 10.0;
        cfg.treeguard.circuits.upgrade_util_pct = 5.0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn stormguard_defaults_are_safe_and_off_by_default() {
        let cfg = crate::etc::v15::stormguard::StormguardConfig::default();
        assert!(!cfg.enabled);
        assert!(!cfg.all_sites);
        assert!(cfg.targets.is_empty());
        assert!(cfg.exclude_sites.is_empty());
        assert!(cfg.dry_run);
        assert_eq!(
            cfg.strategy,
            crate::etc::v15::stormguard::StormguardStrategy::DelayProbe
        );
        assert_eq!(cfg.minimum_download_percentage, 0.5);
        assert_eq!(cfg.minimum_upload_percentage, 0.5);
        assert_eq!(cfg.increase_fast_multiplier, 1.30);
        assert_eq!(cfg.increase_multiplier, 1.15);
        assert_eq!(cfg.decrease_multiplier, 0.95);
        assert_eq!(cfg.decrease_fast_multiplier, 0.88);
        assert_eq!(cfg.increase_fast_cooldown_seconds, 2.0);
        assert_eq!(cfg.increase_cooldown_seconds, 1.0);
        assert_eq!(cfg.decrease_cooldown_seconds, 3.75);
        assert_eq!(cfg.decrease_fast_cooldown_seconds, 7.5);
        assert!(!cfg.circuit_fallback_enabled);
        assert!(cfg.circuit_fallback_persist);
        assert_eq!(cfg.circuit_fallback_sqm, "fq_codel");
        assert_eq!(cfg.delay_threshold_ms, 40.0);
        assert_eq!(cfg.delay_threshold_ratio, 1.10);
        assert_eq!(cfg.baseline_alpha_up, 0.01);
        assert_eq!(cfg.baseline_alpha_down, 0.10);
        assert_eq!(cfg.probe_interval_seconds, 10.0);
        assert_eq!(cfg.min_throughput_mbps_for_rtt, 0.05);
        assert_eq!(cfg.active_ping_target, "1.1.1.1");
        assert_eq!(cfg.active_ping_interval_seconds, 10.0);
        assert_eq!(cfg.active_ping_weight, 0.70);
        assert_eq!(cfg.active_ping_timeout_seconds, 1.0);
    }

    #[test]
    fn stormguard_validation_rejects_invalid_ranges() {
        let mut cfg = Config {
            stormguard: Some(crate::etc::v15::stormguard::StormguardConfig {
                enabled: true,
                targets: vec!["Site A".to_string()],
                ..Default::default()
            }),
            ..Config::default()
        };

        let stormguard = cfg
            .stormguard
            .as_mut()
            .expect("stormguard config should be present");
        stormguard.minimum_download_percentage = 0.0;
        assert!(cfg.validate().is_err());

        let stormguard = cfg
            .stormguard
            .as_mut()
            .expect("stormguard config should be present");
        stormguard.minimum_download_percentage = 0.5;
        stormguard.decrease_multiplier = 1.1;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn legacy_stormguard_config_loads_with_new_defaults() {
        let mut raw = include_str!("example.toml").to_string();
        raw.push_str(
            r#"

[stormguard]
enabled = true
targets = ["Site A"]
dry_run = true
minimum_download_percentage = 0.5
minimum_upload_percentage = 0.5
"#,
        );
        let cfg =
            Config::load_from_string(&raw).expect("legacy stormguard config should deserialize");

        let stormguard = cfg.stormguard.expect("stormguard section missing");
        assert!(!stormguard.all_sites);
        assert!(stormguard.exclude_sites.is_empty());
        assert_eq!(stormguard.increase_fast_multiplier, 1.30);
        assert_eq!(stormguard.decrease_fast_cooldown_seconds, 7.5);
        assert!(!stormguard.circuit_fallback_enabled);
        assert_eq!(stormguard.circuit_fallback_sqm, "fq_codel");
    }
}

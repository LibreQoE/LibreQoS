use std::path::{Path, PathBuf};

use parking_lot::Mutex;

use once_cell::sync::Lazy;

pub static CURRENT_CONFIG: Lazy<Mutex<ConfigBuilder>> =
    Lazy::new(|| Mutex::new(ConfigBuilder::new()));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub enum BridgeMode {
    Linux,
    XDP,
    CompatibilityShim,
    Single,
}

impl BridgeMode {
    pub(crate) const fn uses_netplan_helper(self) -> bool {
        matches!(self, Self::Linux | Self::Single)
    }
}

#[derive(Clone, Debug)]
pub struct ConfigBuilder {
    pub bridge_mode: BridgeMode,
    pub to_internet: String,
    pub to_network: String,
    pub internet_vlan: u32,
    pub network_vlan: u32,
    pub mbps_to_internet: u64,
    pub mbps_to_network: u64,
    pub allow_subnets: Vec<String>,
    pub node_name: String,
    pub config_load_error: Option<String>,
}

impl ConfigBuilder {
    pub fn new() -> Self {
        let config_path = current_config_path();
        if let Ok(cfg) = lqos_config::load_config() {
            let mut to_internet = String::new();
            let mut to_network = String::new();
            let mut internet_vlan = 0;
            let mut network_vlan = 0;
            let mode = if let Some(bridge) = &cfg.bridge {
                if bridge.compatibility_shim_enabled() {
                    to_internet = bridge.to_internet.clone();
                    to_network = bridge.to_network.clone();
                    BridgeMode::CompatibilityShim
                } else if bridge.use_xdp_bridge {
                    to_internet = bridge.to_internet.clone();
                    to_network = bridge.to_network.clone();
                    BridgeMode::XDP
                } else {
                    to_internet = bridge.to_internet.clone();
                    to_network = bridge.to_network.clone();
                    BridgeMode::Linux
                }
            } else if let Some(si) = &cfg.single_interface {
                to_internet = si.interface.clone();
                internet_vlan = si.internet_vlan;
                network_vlan = si.network_vlan;
                BridgeMode::Single
            } else {
                BridgeMode::Linux
            };
            ConfigBuilder {
                bridge_mode: mode,
                to_internet,
                to_network,
                internet_vlan,
                network_vlan,
                mbps_to_internet: cfg.queues.downlink_bandwidth_mbps,
                mbps_to_network: cfg.queues.uplink_bandwidth_mbps,
                allow_subnets: cfg.ip_ranges.allow_subnets.clone(),
                node_name: cfg.node_name.clone(),
                config_load_error: None,
            }
        } else {
            let config_load_error = existing_config_load_error_for_path(&config_path);
            // Default configuration if no config is loaded
            ConfigBuilder {
                bridge_mode: BridgeMode::Linux,
                to_internet: String::new(),
                to_network: String::new(),
                internet_vlan: 0,
                network_vlan: 0,
                mbps_to_internet: 9_400,
                mbps_to_network: 9_400,
                allow_subnets: vec![
                    "172.16.0.0/12".to_string(),
                    "10.0.0.0/8".to_string(),
                    "100.64.0.0/10".to_string(),
                    "192.168.0.0/16".to_string(),
                ],
                node_name: "LibreQoS".to_string(),
                config_load_error,
            }
        }
    }
}

pub fn current_config_path() -> PathBuf {
    std::env::var_os("LQOS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/lqos.conf"))
}

fn existing_config_load_error_for_path(config_path: &Path) -> Option<String> {
    config_path.exists().then(|| {
        format!(
            "Existing LibreQoS configuration at {} could not be loaded. Fix or replace it before setup can continue.",
            config_path.display()
        )
    })
}

pub fn existing_config_load_error() -> Option<String> {
    existing_config_load_error_for_path(&current_config_path())
}

pub fn existing_config_uses_direct_xdp() -> bool {
    lqos_config::load_config()
        .ok()
        .and_then(|config| {
            config
                .bridge
                .as_ref()
                .map(|bridge| bridge.use_xdp_bridge && !bridge.compatibility_shim_enabled())
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{BridgeMode, ConfigBuilder, existing_config_uses_direct_xdp};
    use crate::test_support::ConfigEnvGuard;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn invalid_existing_config_sets_blocking_load_error() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "libreqos-setup-invalid-config-{}-{unique}.toml",
            std::process::id()
        ));
        fs::write(&path, "not valid toml = [\n").expect("write invalid config");
        let _env_guard = ConfigEnvGuard::set_lqos_config(&path);

        let builder = ConfigBuilder::new();

        assert!(builder.config_load_error.is_some());

        fs::remove_file(path).expect("remove temp config");
    }

    #[test]
    fn compatibility_shim_config_loads_as_its_own_setup_mode() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "libreqos-setup-shim-config-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create shim config directory");
        let path = root.join("lqos.conf");
        let _env_guard = ConfigEnvGuard::set_lqos_config(&path);
        let mut config = lqos_config::Config {
            lqos_directory: root.display().to_string(),
            state_directory: Some(root.join("state").display().to_string()),
            bridge: Some(lqos_config::BridgeConfig {
                use_xdp_bridge: true,
                compatibility_shim: true,
                to_internet: "bond0".to_string(),
                to_network: "bond1".to_string(),
                ..lqos_config::BridgeConfig::default()
            }),
            ..lqos_config::Config::default()
        };
        let serialized = toml::to_string_pretty(&config).expect("serialize shim config");
        fs::write(&path, serialized).expect("write shim config");
        lqos_config::clear_cached_config();

        let builder = ConfigBuilder::new();

        assert_eq!(builder.bridge_mode, BridgeMode::CompatibilityShim);
        assert_eq!(builder.to_internet, "bond0");
        assert_eq!(builder.to_network, "bond1");
        assert!(!existing_config_uses_direct_xdp());

        config
            .bridge
            .as_mut()
            .expect("bridge config")
            .compatibility_shim = false;
        let serialized = toml::to_string_pretty(&config).expect("serialize direct XDP config");
        fs::write(&path, serialized).expect("write direct XDP config");
        lqos_config::clear_cached_config();
        assert!(existing_config_uses_direct_xdp());

        fs::remove_dir_all(root).expect("remove temp config directory");
    }

    #[test]
    fn compatibility_shim_updates_config_without_managing_netplan() {
        assert!(!BridgeMode::CompatibilityShim.uses_netplan_helper());
        assert!(!BridgeMode::XDP.uses_netplan_helper());
        assert!(BridgeMode::Linux.uses_netplan_helper());
        assert!(BridgeMode::Single.uses_netplan_helper());
    }
}

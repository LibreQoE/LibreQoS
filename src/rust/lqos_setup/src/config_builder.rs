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
    Single,
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
                if bridge.use_xdp_bridge {
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

pub fn existing_config_uses_xdp() -> bool {
    lqos_config::load_config()
        .ok()
        .and_then(|config| config.bridge.as_ref().map(|bridge| bridge.use_xdp_bridge))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::ConfigBuilder;
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    static CONFIG_ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    #[test]
    fn invalid_existing_config_sets_blocking_load_error() {
        let _guard = CONFIG_ENV_LOCK.lock();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "libreqos-setup-invalid-config-{}-{unique}.toml",
            std::process::id()
        ));
        fs::write(&path, "not valid toml = [\n").expect("write invalid config");
        let old_lqos_config = std::env::var_os("LQOS_CONFIG");
        unsafe {
            std::env::set_var("LQOS_CONFIG", &path);
        }
        lqos_config::clear_cached_config();

        let builder = ConfigBuilder::new();

        assert!(builder.config_load_error.is_some());

        match old_lqos_config {
            Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
            None => unsafe { std::env::remove_var("LQOS_CONFIG") },
        }
        lqos_config::clear_cached_config();
        fs::remove_file(path).expect("remove temp config");
    }
}

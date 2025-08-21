use parking_lot::Mutex;

use once_cell::sync::Lazy;

pub static CURRENT_CONFIG: Lazy<Mutex<ConfigBuilder>> =
    Lazy::new(|| Mutex::new(ConfigBuilder::new()));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl ConfigBuilder {
    pub fn new() -> Self {
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
            }
        } else {
            // Default configuration if no config is loaded
            ConfigBuilder {
                bridge_mode: BridgeMode::Linux,
                to_internet: String::new(),
                to_network: String::new(),
                internet_vlan: 0,
                network_vlan: 0,
                mbps_to_internet: 1_000,
                mbps_to_network: 1_000,
                allow_subnets: vec![
                    "172.16.0.0/12".to_string(),
                    "10.0.0.0/8".to_string(),
                    "100.64.0.0/10".to_string(),
                    "192.168.0.0/16".to_string(),
                ],
                node_name: "LibreQoS".to_string(),
            }
        }
    }
}

use std::path::Path;
use serde::Deserialize;
use anyhow::{Result, Error};

#[derive(Deserialize, Clone, Debug)]
pub struct EtcLqos {
    pub lqos_directory: String,
    pub bridge: Option<BridgeConfig>,
    pub tuning: Option<Tunables>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Tunables {
    pub stop_irq_balance: bool,
    pub netdev_budget_usecs: u32,
    pub netdev_budget_packets: u32,
    pub rx_usecs: u32,
    pub tx_usecs: u32,
    pub disable_rxvlan: bool,
    pub disable_txvlan: bool,
    pub disable_offload: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct BridgeConfig {
    pub use_kernel_bridge: bool,
    pub interface_mapping: Vec<BridgeInterface>,
    pub vlan_mapping: Vec<BridgeVlan>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct BridgeInterface {
    pub name: String,
    pub scan_vlans: bool,
    pub redirect_to: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct BridgeVlan {
    pub parent: String,
    pub tag: u32,
    pub redirect_to: u32,
}

impl EtcLqos {
    pub fn load() -> Result<Self> {
        if !Path::new("/etc/lqos").exists() {
            return Err(Error::msg("You must setup /etc/lqos"));
        }
        let raw = std::fs::read_to_string("/etc/lqos")?;
        let config: Self = toml::from_str(&raw)?;
        //println!("{:?}", config);
        Ok(config)
    }
}
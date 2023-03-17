//! Manages the `/etc/lqos.conf` file.
use log::error;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::{path::Path, fs};
use thiserror::Error;

/// Represents the top-level of the `/etc/lqos.conf` file. Serialization
/// structure.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EtcLqos {
  /// The directory in which LibreQoS is installed.
  pub lqos_directory: String,

  /// How frequently should `lqosd` read the `tc show qdisc` data?
  /// In ms.
  pub queue_check_period_ms: u64,

  /// If present, provides a unique ID for the node. Used for
  /// anonymous stats (to identify nodes without providing an actual
  /// identity), and will be used for long-term data retention to
  /// disambiguate cluster or multi-head-end nodes.
  pub node_id: Option<String>,

  /// If present, defines how the Bifrost XDP bridge operates.
  pub bridge: Option<BridgeConfig>,

  /// If present, defines the values for various `sysctl` and `ethtool`
  /// tweaks.
  pub tuning: Option<Tunables>,

  /// If present, defined anonymous usage stat sending
  pub usage_stats: Option<UsageStats>,
}

/// Represents a set of `sysctl` and `ethtool` tweaks that may be
/// applied (in place of the previous version's offload service)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Tunables {
  /// Should the `irq_balance` system service be stopped?
  pub stop_irq_balance: bool,

  /// Set the netdev budget (usecs)
  pub netdev_budget_usecs: u32,

  /// Set the netdev budget (packets)
  pub netdev_budget_packets: u32,

  /// Set the RX side polling frequency
  pub rx_usecs: u32,

  /// Set the TX side polling frequency
  pub tx_usecs: u32,

  /// Disable RXVLAN offloading? You generally want to do this.
  pub disable_rxvlan: bool,

  /// Disable TXVLAN offloading? You generally want to do this.
  pub disable_txvlan: bool,

  /// A list of `ethtool` offloads to be disabled.
  /// The default list is: [ "gso", "tso", "lro", "sg", "gro" ]
  pub disable_offload: Vec<String>,
}

/// Defines the BiFrost XDP bridge accelerator parameters
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BridgeConfig {
  /// Should the XDP bridge be enabled?
  pub use_xdp_bridge: bool,

  /// A list of interface mappings.
  pub interface_mapping: Vec<BridgeInterface>,

  /// A list of VLAN mappings.
  pub vlan_mapping: Vec<BridgeVlan>,
}

/// An interface within the Bifrost XDP bridge.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BridgeInterface {
  /// The interface name. It *must* match an interface name
  /// findable by Linux.
  pub name: String,

  /// Should Bifrost read VLAN tags and determine redirect
  /// policy from there?
  pub scan_vlans: bool,

  /// The outbound interface - data that arrives in the interface
  /// defined by `name` will be redirected to this interface.
  ///
  /// If you are using an "on a stick" configuration, this will
  /// be the same as `name`.
  pub redirect_to: String,
}

/// If `scan_vlans` is enabled for an interface, then VLANs
/// are examined on the way through the XDP BiFrost bridge.
///
/// If a VLAN is on the `parent` interface, and matches `tag` - it
/// will be moved to VLAN `redirect_to`.
///
/// You often need to make reciprocal pairs of these.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BridgeVlan {
  /// The parent interface name on which the VLAN occurs.
  pub parent: String,

  /// The VLAN tag number to redirect if matched.
  pub tag: u32,

  /// The destination VLAN tag number if matched.
  pub redirect_to: u32,
}

/// Definitions for anonymous usage submission
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UsageStats {
  /// Are we allowed to send stats at all?
  pub send_anonymous: bool,

  /// Where do we send them?
  pub anonymous_server: String,
}

impl EtcLqos {
  /// Loads `/etc/lqos.conf`.
  pub fn load() -> Result<Self, EtcLqosError> {
    if !Path::new("/etc/lqos.conf").exists() {
      error!("/etc/lqos.conf does not exist!");
      return Err(EtcLqosError::ConfigDoesNotExist);
    }
    if let Ok(raw) = std::fs::read_to_string("/etc/lqos.conf") {
      let config_result: Result<Self, toml::de::Error> = toml::from_str(&raw);
      match config_result {
        Ok(mut config) => {
          check_config(&mut config);
          Ok(config)
        }
        Err(e) => {
          error!("Unable to parse TOML from /etc/lqos.conf");
          error!("Full error: {:?}", e);
          Err(EtcLqosError::CannotParseToml)
        }
      }
    } else {
      error!("Unable to read contents of /etc/lqos.conf");
      Err(EtcLqosError::CannotReadFile)
    }
  }

  /// Saves changes made to /etc/lqos.conf
  /// Copies current configuration into /etc/lqos.conf.backup first
  pub fn save(&self) -> Result<(), EtcLqosError> {
    let cfg_path = Path::new("/etc/lqos.conf");
    let backup_path = Path::new("/etc/lqos.conf.backup");
    if let Err(e) = std::fs::copy(cfg_path, backup_path) {
      log::error!("Unable to backup /etc/lqos.conf");
      log::error!("{e:?}");
      return Err(EtcLqosError::BackupFail);
    }
    let new_cfg = toml::to_string_pretty(&self);
    match new_cfg {
      Err(e) => {
        log::error!("Unable to serialize new /etc/lqos.conf");
        log::error!("{e:?}");
        return Err(EtcLqosError::SerializeFail);
      }
      Ok(new_cfg) => {
        if let Err(e) = fs::write(cfg_path, new_cfg) {
          log::error!("Unable to write to /etc/lqos.conf");
          log::error!("{e:?}");
          return Err(EtcLqosError::WriteFail);
        }
      }
    }
    Ok(())
  }
}

fn check_config(cfg: &mut EtcLqos) {
  let mut changed = false;
  if cfg.node_id.is_none() {
    let new_id = Uuid::new_v4();
    cfg.node_id = Some(new_id.to_string());
    changed = true;
  }

  if changed {
    let _ = cfg.save();
  }
}


#[derive(Error, Debug)]
pub enum EtcLqosError {
  #[error(
    "/etc/lqos.conf not found. You must setup this file to use LibreQoS."
  )]
  ConfigDoesNotExist,
  #[error("Unable to read contents of /etc/lqos.conf.")]
  CannotReadFile,
  #[error("Unable to parse TOML in /etc/lqos.conf")]
  CannotParseToml,
  #[error("Unable to backup /etc/lqos.conf to /etc/lqos.conf.backup")]
  BackupFail,
  #[error("Unable to serialize new configuration")]
  SerializeFail,
  #[error("Unable to write to /etc/lqos.conf")]
  WriteFail,
}

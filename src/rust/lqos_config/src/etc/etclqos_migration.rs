//! Manages the `/etc/lqos.conf` file.
use crate::{load_config, update_config};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use thiserror::Error;
use toml_edit::{DocumentMut, value};
use tracing::{error, info};

/// Represents the top-level of the `/etc/lqos.conf` file. Serialization
/// structure.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EtcLqos {
    /// The directory in which LibreQoS is installed.
    pub lqos_directory: String,

    /// How frequently should `lqosd` read the `tc show qdisc` data?
    /// In ms.
    pub queue_check_period_ms: u64,

    /// If present, provides a unique ID for the node. Used for Insight.
    pub node_id: Option<String>,

    /// If present, provide a name for the node.
    pub node_name: Option<String>,

    /// If present, defines how the Bifrost XDP bridge operates.
    pub bridge: Option<BridgeConfig>,

    /// If present, defines the values for various `sysctl` and `ethtool`
    /// tweaks.
    pub tuning: Option<Tunables>,

    /// Defines for how many seconds a libpcap compatible capture should
    /// run. Short times are good, there's a real performance penalty to
    /// capturing high-throughput streams. Defaults to 10 seconds.
    pub packet_capture_time: Option<usize>,

    /// Long-term statistics retention settings.
    pub long_term_stats: Option<LongTermStats>,

    /// Enable per-circuit TemporalHeatmap collection.
    pub enable_circuit_heatmaps: Option<bool>,

    /// Enable per-site TemporalHeatmap collection.
    pub enable_site_heatmaps: Option<bool>,

    /// Enable per-ASN TemporalHeatmap collection.
    pub enable_asn_heatmaps: Option<bool>,
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

/// Long Term Data Retention
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LongTermStats {
    /// Should we store long-term stats at all?
    pub gather_stats: bool,

    /// How frequently should stats be accumulated into a long-term
    /// min/max/avg format per tick?
    pub collation_period_seconds: u32,

    /// The license key for submitting stats to a LibreQoS hosted
    /// statistics server
    pub license_key: Option<String>,

    /// UISP reporting period (in seconds). UISP queries can be slow,
    /// so hitting it every second or 10 seconds is going to cause problems
    /// for some people. A good default may be 5 minutes. Not specifying this
    /// disabled UISP integration.
    pub uisp_reporting_interval_seconds: Option<u64>,
}

impl EtcLqos {
    /// Loads `/etc/lqos.conf`.
    pub fn load() -> Result<Self, EtcLqosError> {
        if !Path::new("/etc/lqos.conf").exists() {
            error!("/etc/lqos.conf does not exist!");
            return Err(EtcLqosError::ConfigDoesNotExist);
        }
        if let Ok(raw) = std::fs::read_to_string("/etc/lqos.conf") {
            Self::load_from_string(&raw)
        } else {
            error!("Unable to read contents of /etc/lqos.conf");
            Err(EtcLqosError::CannotReadFile)
        }
    }

    pub(crate) fn load_from_string(raw: &str) -> Result<Self, EtcLqosError> {
        info!("Trying to load old TOML version from /etc/lqos.conf");
        let document = raw.parse::<DocumentMut>();
        match document {
            Err(e) => {
                error!("Unable to parse TOML from /etc/lqos.conf");
                error!("Full error: {:?}", e);
                Err(EtcLqosError::CannotParseToml)
            }
            Ok(mut config_doc) => {
                let cfg = toml_edit::de::from_document::<EtcLqos>(config_doc.clone());
                match cfg {
                    Ok(mut cfg) => {
                        check_config(&mut config_doc, &mut cfg);
                        Ok(cfg)
                    }
                    Err(e) => {
                        error!("Unable to parse TOML from /etc/lqos.conf");
                        error!("Full error: {:?}", e);
                        Err(EtcLqosError::CannotParseToml)
                    }
                }
            }
        }
    }

    /// Saves changes made to /etc/lqos.conf
    /// Copies current configuration into /etc/lqos.conf.backup first
    pub fn save(&self, document: &mut DocumentMut) -> Result<(), EtcLqosError> {
        let cfg_path = Path::new("/etc/lqos.conf");
        let backup_path = Path::new("/etc/lqos.conf.backup");
        if let Err(e) = std::fs::copy(cfg_path, backup_path) {
            error!("Unable to backup /etc/lqos.conf");
            error!("{e:?}");
            return Err(EtcLqosError::BackupFail);
        }
        let new_cfg = document.to_string();
        if let Err(e) = fs::write(cfg_path, new_cfg) {
            error!("Unable to write to /etc/lqos.conf");
            error!("{e:?}");
            return Err(EtcLqosError::WriteFail);
        }
        Ok(())
    }
}

/// Run this if you've received the OK from the licensing server, and been
/// sent a license key. This appends a [long_term_stats] section to your
/// config file - ONLY if one doesn't already exist.
#[allow(dead_code)]
pub fn enable_long_term_stats(license_key: String) {
    let Ok(config) = load_config() else { return };
    let mut new_config = (*config).clone();
    new_config.long_term_stats.gather_stats = true;
    new_config.long_term_stats.license_key = Some(license_key);
    new_config.long_term_stats.collation_period_seconds = 60;
    if config.uisp_integration.enable_uisp {
        new_config.long_term_stats.uisp_reporting_interval_seconds = Some(300);
    }
    match update_config(&new_config) {
        Ok(_) => info!("Long-term stats enabled"),
        Err(e) => {
            error!("Unable to update configuration: {e:?}");
        }
    }
}

fn check_config(cfg_doc: &mut DocumentMut, cfg: &mut EtcLqos) {
    use sha2::Digest;
    use sha2::digest::Update;

    if cfg.node_id.is_none()
        && let Ok(machine_id) = std::fs::read_to_string("/etc/machine-id")
    {
        let hash = sha2::Sha256::new().chain(machine_id).finalize();
        cfg.node_id = Some(format!("{:x}", hash));
        cfg_doc["node_id"] = value(format!("{:x}", hash));
        println!("Updating");
        if let Err(e) = cfg.save(cfg_doc) {
            error!("Unable to save /etc/lqos.conf");
            error!("{e:?}");
        }
    }
}

#[derive(Error, Debug)]
pub enum EtcLqosError {
    #[error("/etc/lqos.conf not found. You must setup this file to use LibreQoS.")]
    ConfigDoesNotExist,
    #[error("Unable to read contents of /etc/lqos.conf.")]
    CannotReadFile,
    #[error("Unable to parse TOML in /etc/lqos.conf")]
    CannotParseToml,
    #[error("Unable to backup /etc/lqos.conf to /etc/lqos.conf.backup")]
    BackupFail,
    #[error("Unable to write to /etc/lqos.conf")]
    WriteFail,
}

#[cfg(test)]
mod test {
    const EXAMPLE_LQOS_CONF: &str = include_str!("../../../../lqos.example");

    #[test]
    fn round_trip_toml() {
        let doc = EXAMPLE_LQOS_CONF
            .parse::<toml_edit::DocumentMut>()
            .expect("Unable to read example config file");
        let reserialized = doc.to_string();
        assert_eq!(EXAMPLE_LQOS_CONF.trim(), reserialized.trim());
    }

    #[test]
    fn add_node_id() {
        let mut doc = EXAMPLE_LQOS_CONF
            .parse::<toml_edit::DocumentMut>()
            .expect("Unable to read example config file");
        doc["node_id"] = toml_edit::value("test");
        let reserialized = doc.to_string();
        assert!(reserialized.contains("node_id = \"test\""));
    }
}

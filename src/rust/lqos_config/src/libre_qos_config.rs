//! `ispConfig.py` is part of the Python side of LibreQoS. This module
//! reads, writes and maps values from the Python file.

use crate::etc;
use ip_network::IpNetwork;
use log::error;
use serde::{Deserialize, Serialize};
use std::{
  fs::{self, read_to_string, remove_file, OpenOptions},
  io::Write,
  net::IpAddr,
  path::{Path, PathBuf},
};
use thiserror::Error;

/// Represents the contents of an `ispConfig.py` file.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LibreQoSConfig {
  /// Interface facing the Internet
  pub internet_interface: String,

  /// Interface facing the ISP Core Router
  pub isp_interface: String,

  /// Are we in "on a stick" (single interface) mode?
  pub on_a_stick_mode: bool,

  /// If we are, which VLAN represents which direction?
  /// In (internet, ISP) order.
  pub stick_vlans: (u16, u16),

  /// The value of the SQM field from `ispConfig.py`
  pub sqm: String,

  /// Are we in monitor-only mode (not shaping)?
  pub monitor_mode: bool,

  /// Total available download (in Mbps)
  pub total_download_mbps: u32,

  /// Total available upload (in Mbps)
  pub total_upload_mbps: u32,

  /// If a node is generated, how much download (Mbps) should it offer?
  pub generated_download_mbps: u32,

  /// If a node is generated, how much upload (Mbps) should it offer?
  pub generated_upload_mbps: u32,

  /// Should the Python queue builder use the bin packing strategy to
  /// try to optimize CPU assignment?
  pub use_binpacking: bool,

  /// Should the Python program use actual shell commands (and execute)
  /// them?
  pub enable_shell_commands: bool,

  /// Should every issued command be prefixed with `sudo`?
  pub run_as_sudo: bool,

  /// WARNING: generally don't touch this.
  pub override_queue_count: u32,

  /// Is UISP integration enabled?
  pub automatic_import_uisp: bool,

  /// UISP Authentication Token
  pub uisp_auth_token: String,

  /// UISP Base URL (e.g. billing.myisp.com)
  pub uisp_base_url: String,

  /// Root site for UISP tree generation
  pub uisp_root_site: String,

  /// Circuit names use address?
  pub circuit_name_use_address: bool,

  /// UISP Strategy
  pub uisp_strategy: String,

  /// UISP Suspension Strategy
  pub uisp_suspended_strategy: String,

  /// Bandwidth Overhead Factor
  pub bandwidth_overhead_factor: f32,

  /// Subnets allowed to be included in device lists
  pub allowed_subnets: String,

  /// Subnets explicitly ignored from device lists
  pub ignored_subnets: String,

  /// Overwrite network.json even if it exists
  pub overwrite_network_json_always: bool,
}

impl LibreQoSConfig {
  /// Does the ispConfig.py file exist?
  pub fn config_exists() -> bool {
    if let Ok(cfg) = etc::EtcLqos::load() {
      let base_path = Path::new(&cfg.lqos_directory);
      let final_path = base_path.join("ispConfig.py");
      final_path.exists()
    } else {
      false
    }
  }

  /// Loads `ispConfig.py` into a management object.
  pub fn load() -> Result<Self, LibreQoSConfigError> {
    if let Ok(cfg) = etc::EtcLqos::load() {
      let base_path = Path::new(&cfg.lqos_directory);
      let final_path = base_path.join("ispConfig.py");
      Ok(Self::load_from_path(&final_path)?)
    } else {
      error!("Unable to read LibreQoS config from /etc/lqos.conf");
      Err(LibreQoSConfigError::CannotOpenEtcLqos)
    }
  }

  fn load_from_path(path: &PathBuf) -> Result<Self, LibreQoSConfigError> {
    let path = Path::new(path);
    if !path.exists() {
      error!("Unable to find ispConfig.py");
      return Err(LibreQoSConfigError::FileNotFoud);
    }

    // Read the config
    let mut result = Self {
      internet_interface: String::new(),
      isp_interface: String::new(),
      on_a_stick_mode: false,
      stick_vlans: (0, 0),
      sqm: String::new(),
      monitor_mode: false,
      total_download_mbps: 0,
      total_upload_mbps: 0,
      generated_download_mbps: 0,
      generated_upload_mbps: 0,
      use_binpacking: false,
      enable_shell_commands: true,
      run_as_sudo: false,
      override_queue_count: 0,
      automatic_import_uisp: false,
      uisp_auth_token: "".to_string(),
      uisp_base_url: "".to_string(),
      uisp_root_site: "".to_string(),
      circuit_name_use_address: false,
      uisp_strategy: "".to_string(),
      uisp_suspended_strategy: "".to_string(),
      bandwidth_overhead_factor: 1.0,
      allowed_subnets: "".to_string(),
      ignored_subnets: "".to_string(),
      overwrite_network_json_always: false,
    };
    result.parse_isp_config(path)?;
    Ok(result)
  }

  fn parse_isp_config(
    &mut self,
    path: &Path,
  ) -> Result<(), LibreQoSConfigError> {
    let read_result = fs::read_to_string(path);
    match read_result {
      Err(e) => {
        error!("Unable to read contents of ispConfig.py. Check permissions.");
        error!("{:?}", e);
        return Err(LibreQoSConfigError::CannotReadFile);
      }
      Ok(content) => {
        for line in content.split('\n') {
          if line.starts_with("interfaceA") {
            self.isp_interface = split_at_equals(line);
          }
          if line.starts_with("interfaceB") {
            self.internet_interface = split_at_equals(line);
          }
          if line.starts_with("OnAStick") {
            let mode = split_at_equals(line);
            if mode == "True" {
              self.on_a_stick_mode = true;
            }
          }
          if line.starts_with("StickVlanA") {
            let vlan_string = split_at_equals(line);
            if let Ok(vlan) = vlan_string.parse() {
              self.stick_vlans.0 = vlan;
            } else {
              error!(
                "Unable to parse contents of StickVlanA from ispConfig.py"
              );
              error!("{line}");
              return Err(LibreQoSConfigError::ParseError(line.to_string()));
            }
          }
          if line.starts_with("StickVlanB") {
            let vlan_string = split_at_equals(line);
            if let Ok(vlan) = vlan_string.parse() {
              self.stick_vlans.1 = vlan;
            } else {
              error!(
                "Unable to parse contents of StickVlanB from ispConfig.py"
              );
              error!("{line}");
              return Err(LibreQoSConfigError::ParseError(line.to_string()));
            }
          }
          if line.starts_with("sqm") {
            self.sqm = split_at_equals(line);
          }
          if line.starts_with("upstreamBandwidthCapacityDownloadMbps") {
            if let Ok(mbps) = split_at_equals(line).parse() {
              self.total_download_mbps = mbps;
            } else {
              error!("Unable to parse contents of upstreamBandwidthCapacityDownloadMbps from ispConfig.py");
              error!("{line}");
              return Err(LibreQoSConfigError::ParseError(line.to_string()));
            }
          }
          if line.starts_with("upstreamBandwidthCapacityUploadMbps") {
            if let Ok(mbps) = split_at_equals(line).parse() {
              self.total_upload_mbps = mbps;
            } else {
              error!("Unable to parse contents of upstreamBandwidthCapacityUploadMbps from ispConfig.py");
              error!("{line}");
              return Err(LibreQoSConfigError::ParseError(line.to_string()));
            }
          }
          if line.starts_with("monitorOnlyMode ") {
            let mode = split_at_equals(line);
            if mode == "True" {
              self.monitor_mode = true;
            }
          }
          if line.starts_with("generatedPNDownloadMbps") {
            if let Ok(mbps) = split_at_equals(line).parse() {
              self.generated_download_mbps = mbps;
            } else {
              error!("Unable to parse contents of generatedPNDownloadMbps from ispConfig.py");
              error!("{line}");
              return Err(LibreQoSConfigError::ParseError(line.to_string()));
            }
          }
          if line.starts_with("generatedPNUploadMbps") {
            if let Ok(mbps) = split_at_equals(line).parse() {
              self.generated_upload_mbps = mbps;
            } else {
              error!("Unable to parse contents of generatedPNUploadMbps from ispConfig.py");
              error!("{line}");
              return Err(LibreQoSConfigError::ParseError(line.to_string()));
            }
          }
          if line.starts_with("useBinPackingToBalanceCPU") {
            let mode = split_at_equals(line);
            if mode == "True" {
              self.use_binpacking = true;
            }
          }
          if line.starts_with("enableActualShellCommands") {
            let mode = split_at_equals(line);
            if mode == "True" {
              self.enable_shell_commands = true;
            }
          }
          if line.starts_with("runShellCommandsAsSudo") {
            let mode = split_at_equals(line);
            if mode == "True" {
              self.run_as_sudo = true;
            }
          }
          if line.starts_with("queuesAvailableOverride") {
            self.override_queue_count =
              split_at_equals(line).parse().unwrap_or(0);
          }
          if line.starts_with("automaticImportUISP") {
            let mode = split_at_equals(line);
            if mode == "True" {
              self.automatic_import_uisp = true;
            }
          }
          if line.starts_with("uispAuthToken") {
            self.uisp_auth_token = split_at_equals(line);
          }
          if line.starts_with("UISPbaseURL") {
            self.uisp_base_url = split_at_equals(line);
          }
          if line.starts_with("uispSite") {
            self.uisp_root_site = split_at_equals(line);
          }
          if line.starts_with("circuitNameUseAddress") {
            let mode = split_at_equals(line);
            if mode == "True" {
              self.circuit_name_use_address = true;
            }
          }
          if line.starts_with("uispStrategy") {
            self.uisp_strategy = split_at_equals(line);
          }
          if line.starts_with("uispSuspendedStrategy") {
            self.uisp_suspended_strategy = split_at_equals(line);
          }
          if line.starts_with("bandwidthOverheadFactor") {
            self.bandwidth_overhead_factor =
              split_at_equals(line).parse().unwrap_or(1.0);
          }
          if line.starts_with("allowedSubnets") {
            self.allowed_subnets = split_at_equals(line);
          }
          if line.starts_with("ignoreSubnets") {
            self.ignored_subnets = split_at_equals(line);
          }
          if line.starts_with("overwriteNetworkJSONalways") {
            let mode = split_at_equals(line);
            if mode == "True" {
              self.overwrite_network_json_always = true;
            }
          }
        }
      }
    }
    Ok(())
  }

  /// Saves the current values to `ispConfig.py` and store the
  /// previous settings in `ispConfig.py.backup`.
  ///
  pub fn save(&self) -> Result<(), LibreQoSConfigError> {
    // Find the config
    let cfg = etc::EtcLqos::load().map_err(|_| {
      crate::libre_qos_config::LibreQoSConfigError::CannotOpenEtcLqos
    })?;
    let base_path = Path::new(&cfg.lqos_directory);
    let final_path = base_path.join("ispConfig.py");
    let backup_path = base_path.join("ispConfig.py.backup");
    if std::fs::copy(&final_path, &backup_path).is_err() {
      error!(
        "Unable to copy {} to {}.",
        final_path.display(),
        backup_path.display()
      );
      return Err(LibreQoSConfigError::CannotCopy);
    }

    // Load existing file
    let original = read_to_string(&final_path);
    if original.is_err() {
      error!("Unable to read ispConfig.py");
      return Err(LibreQoSConfigError::CannotReadFile);
    }
    let original = original.unwrap();

    // Temporary
    //let final_path = base_path.join("ispConfig.py.test");

    // Update config entries line by line
    let mut config = String::new();
    for line in original.split('\n') {
      let mut line = line.to_string();
      if line.starts_with("interfaceA") {
        line = format!("interfaceA = '{}'", self.isp_interface);
      }
      if line.starts_with("interfaceB") {
        line = format!("interfaceB = '{}'", self.internet_interface);
      }
      if line.starts_with("OnAStick") {
        line = format!(
          "OnAStick = {}",
          if self.on_a_stick_mode { "True" } else { "False" }
        );
      }
      if line.starts_with("StickVlanA") {
        line = format!("StickVlanA = {}", self.stick_vlans.0);
      }
      if line.starts_with("StickVlanB") {
        line = format!("StickVlanB = {}", self.stick_vlans.1);
      }
      if line.starts_with("sqm") {
        line = format!("sqm = '{}'", self.sqm);
      }
      if line.starts_with("upstreamBandwidthCapacityDownloadMbps") {
        line = format!(
          "upstreamBandwidthCapacityDownloadMbps = {}",
          self.total_download_mbps
        );
      }
      if line.starts_with("upstreamBandwidthCapacityUploadMbps") {
        line = format!(
          "upstreamBandwidthCapacityUploadMbps = {}",
          self.total_upload_mbps
        );
      }
      if line.starts_with("monitorOnlyMode") {
        line = format!(
          "monitorOnlyMode = {}",
          if self.monitor_mode { "True" } else { "False" }
        );
      }
      if line.starts_with("generatedPNDownloadMbps") {
        line = format!(
          "generatedPNDownloadMbps = {}",
          self.generated_download_mbps
        );
      }
      if line.starts_with("generatedPNUploadMbps") {
        line =
          format!("generatedPNUploadMbps = {}", self.generated_upload_mbps);
      }
      if line.starts_with("useBinPackingToBalanceCPU") {
        line = format!(
          "useBinPackingToBalanceCPU = {}",
          if self.use_binpacking { "True" } else { "False" }
        );
      }
      if line.starts_with("enableActualShellCommands") {
        line = format!(
          "enableActualShellCommands = {}",
          if self.enable_shell_commands { "True" } else { "False" }
        );
      }
      if line.starts_with("runShellCommandsAsSudo") {
        line = format!(
          "runShellCommandsAsSudo = {}",
          if self.run_as_sudo { "True" } else { "False" }
        );
      }
      if line.starts_with("queuesAvailableOverride") {
        line =
          format!("queuesAvailableOverride = {}", self.override_queue_count);
      }
      config += &format!("{line}\n");
    }

    // Actually save to disk
    if final_path.exists() {
      remove_file(&final_path)
        .map_err(|_| LibreQoSConfigError::CannotRemove)?;
    }
    if let Ok(mut file) =
      OpenOptions::new().write(true).create_new(true).open(&final_path)
    {
      if file.write_all(config.as_bytes()).is_err() {
        error!("Unable to write to ispConfig.py");
        return Err(LibreQoSConfigError::CannotWrite);
      }
    } else {
      error!("Unable to open ispConfig.py for writing.");
      return Err(LibreQoSConfigError::CannotOpenForWrite);
    }
    Ok(())
  }

  /// Convert the Allowed Subnets list into a Trie for fast search
  pub fn allowed_subnets_trie(&self) -> ip_network_table::IpNetworkTable<usize> {
    let ip_list = ip_list_to_ips(&self.allowed_subnets).unwrap();
    //println!("{ip_list:#?}");
    ip_list_to_trie(&ip_list)
  }

  /// Convert the Ignored Subnets list into a Trie for fast search
  pub fn ignored_subnets_trie(&self) -> ip_network_table::IpNetworkTable<usize> {
    let ip_list = ip_list_to_ips(&self.ignored_subnets).unwrap();
    //println!("{ip_list:#?}");
    ip_list_to_trie(&ip_list)
  }
}

fn split_at_equals(line: &str) -> String {
  line.split('=').nth(1).unwrap_or("").trim().replace(['\"', '\''], "")
}

#[derive(Debug, Error)]
pub enum LibreQoSConfigError {
  #[error("Unable to read /etc/lqos.conf. See other errors for details.")]
  CannotOpenEtcLqos,
  #[error("Unable to locate (path to LibreQoS)/ispConfig.py. Check your path and that you have configured it.")]
  FileNotFoud,
  #[error(
    "Unable to read the contents of ispConfig.py. Check file permissions."
  )]
  CannotReadFile,
  #[error("Unable to parse ispConfig.py")]
  ParseError(String),
  #[error("Could not backup configuration")]
  CannotCopy,
  #[error("Could not remove the previous configuration.")]
  CannotRemove,
  #[error("Could not open ispConfig.py for write")]
  CannotOpenForWrite,
  #[error("Unable to write to ispConfig.py")]
  CannotWrite,
  #[error("Unable to read IP")]
  CannotReadIP,
}

fn ip_list_to_ips(
  source: &str,
) -> Result<Vec<(IpAddr, u8)>, LibreQoSConfigError> {
  // Remove any square brackets, spaces
  let source = source.replace(['[', ']', ' '], "");

  // Split at commas
  Ok(
    source
      .split(',')
      .map(|raw| {
        let split: Vec<&str> = raw.split('/').collect();
        let cidr = split[1].parse::<u8>().unwrap();
        let addr = split[0].parse::<IpAddr>().unwrap();
        (addr, cidr)
      })
      .collect(),
  )
}

fn ip_list_to_trie(
  source: &[(IpAddr, u8)],
) -> ip_network_table::IpNetworkTable<usize> {
  let mut table = ip_network_table::IpNetworkTable::new();
  source
    .iter()
    .map(|(ip, subnet)| {
      (
        match ip {
          IpAddr::V4(ip) => ip.to_ipv6_mapped(),
          IpAddr::V6(ip) => *ip,
        },
        match ip {
          IpAddr::V4(..) => *subnet + 96,
          IpAddr::V6(..) => *subnet
        },
      )
    })
    .map(|(ip, cidr)| IpNetwork::new(ip, cidr).unwrap())
    .enumerate()
    .for_each(|(id, net)| {
      table.insert(net, id);
    });
  table
}

//! `ispConfig.py` is part of the Python side of LibreQoS. This module
//! reads, writes and maps values from the Python file.

use crate::etc;
use anyhow::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, read_to_string, remove_file, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// Represents the contents of an `ispConfig.py` file.
#[derive(Serialize, Deserialize, Debug)]
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
}

impl LibreQoSConfig {
    /// Loads `ispConfig.py` into a management object.
    pub fn load() -> Result<Self> {
        let cfg = etc::EtcLqos::load()?;
        let base_path = Path::new(&cfg.lqos_directory);
        let final_path = base_path.join("ispConfig.py");
        Ok(Self::load_from_path(&final_path)?)
    }

    fn load_from_path(path: &PathBuf) -> Result<Self> {
        let path = Path::new(path);
        if !path.exists() {
            return Err(Error::msg("Unable to find ispConfig.py"));
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
        };
        result.parse_isp_config(path)?;
        Ok(result)
    }

    fn parse_isp_config(&mut self, path: &Path) -> Result<()> {
        let content = fs::read_to_string(path)?;
        for line in content.split("\n") {
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
                let vlan: u16 = vlan_string.parse()?;
                self.stick_vlans.0 = vlan;
            }
            if line.starts_with("StickVlanB") {
                let vlan_string = split_at_equals(line);
                let vlan: u16 = vlan_string.parse()?;
                self.stick_vlans.1 = vlan;
            }
            if line.starts_with("sqm") {
                self.sqm = split_at_equals(line);
            }
            if line.starts_with("upstreamBandwidthCapacityDownloadMbps") {
                self.total_download_mbps = split_at_equals(line).parse()?;
            }
            if line.starts_with("upstreamBandwidthCapacityUploadMbps") {
                self.total_upload_mbps = split_at_equals(line).parse()?;
            }
            if line.starts_with("monitorOnlyMode ") {
                let mode = split_at_equals(line);
                if mode == "True" {
                    self.monitor_mode = true;
                }
            }
            if line.starts_with("generatedPNDownloadMbps") {
                self.generated_download_mbps = split_at_equals(line).parse()?;
            }
            if line.starts_with("generatedPNUploadMbps") {
                self.generated_upload_mbps = split_at_equals(line).parse()?;
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
                self.override_queue_count = split_at_equals(line).parse().unwrap_or(0);
            }
        }
        Ok(())
    }

    /// Saves the current values to `ispConfig.py` and store the
    /// previous settings in `ispConfig.py.backup`.
    ///
    pub fn save(&self) -> Result<()> {
        // Find the config
        let cfg = etc::EtcLqos::load()?;
        let base_path = Path::new(&cfg.lqos_directory);
        let final_path = base_path.clone().join("ispConfig.py");
        let backup_path = base_path.join("ispConfig.py.backup");
        std::fs::copy(&final_path, &backup_path)?;

        // Load existing file
        let original = read_to_string(&final_path)?;

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
                    if self.on_a_stick_mode {
                        "True"
                    } else {
                        "False"
                    }
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
                line = format!("generatedPNDownloadMbps = {}", self.generated_download_mbps);
            }
            if line.starts_with("generatedPNUploadMbps") {
                line = format!("generatedPNUploadMbps = {}", self.generated_upload_mbps);
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
                    if self.enable_shell_commands {
                        "True"
                    } else {
                        "False"
                    }
                );
            }
            if line.starts_with("runShellCommandsAsSudo") {
                line = format!(
                    "runShellCommandsAsSudo = {}",
                    if self.run_as_sudo { "True" } else { "False" }
                );
            }
            if line.starts_with("queuesAvailableOverride") {
                line = format!("queuesAvailableOverride = {}", self.override_queue_count);
            }
            config += &format!("{line}\n");
        }

        // Actually save to disk
        if final_path.exists() {
            remove_file(&final_path)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&final_path)?;
        file.write_all(&config.as_bytes())?;
        Ok(())
    }
}

fn split_at_equals(line: &str) -> String {
    line.split('=')
        .nth(1)
        .unwrap_or("")
        .trim()
        .replace("\"", "")
        .replace("'", "")
}

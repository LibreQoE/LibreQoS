use anyhow::{Error, Result};
use serde::{Serialize, Deserialize};
use std::{fs::{self, remove_file, OpenOptions, read_to_string}, path::{Path, PathBuf}, io::Write};
use crate::etc;

#[derive(Serialize, Deserialize, Debug)]
pub struct LibreQoSConfig {
    pub internet_interface: String,
    pub isp_interface: String,
    pub on_a_stick_mode: bool,
    pub stick_vlans: (u16, u16),
    pub sqm: String,
    pub monitor_mode: bool,
    pub total_download_mbps: u32,
    pub total_upload_mbps: u32,
    pub generated_download_mbps: u32,
    pub generated_upload_mbps: u32,
    pub use_binpacking: bool,
    pub enable_shell_commands: bool,
    pub run_as_sudo: bool,
    pub override_queue_count: u32,
}

impl LibreQoSConfig {
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
            stick_vlans: (0,0),
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
                let vlan : u16 = vlan_string.parse()?;
                self.stick_vlans.0 = vlan;
            }
            if line.starts_with("StickVlanB") {
                let vlan_string = split_at_equals(line);
                let vlan : u16 = vlan_string.parse()?;
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

    pub fn save(&self) -> Result<()> {
        // Find the config
        let cfg = etc::EtcLqos::load()?;
        let base_path = Path::new(&cfg.lqos_directory);
        let final_path = base_path.join("ispConfig.py");
        let backup_path = base_path.join("ispConfig.py.backup");
        std::fs::copy(&final_path, &backup_path)?;

        // Load existing file
        let original = read_to_string(final_path)?;

        // Temporary
        let final_path = base_path.join("ispConfig.py.test");

        // Update config entries line by line
        let mut config = String::new();
        for line in original.split('\n') {
            let mut line = line.to_string();
            if line.starts_with("interfaceA") { line = format!("interfaceA = '{}'", self.isp_interface); }
            if line.starts_with("interfaceB") { line = format!("interfaceB = '{}'", self.internet_interface); }
            if line.starts_with("OnAStick") { line = format!("OnAStick = {}", if self.on_a_stick_mode { "True" } else { "False" } ); }
            if line.starts_with("StickVlanA") { line = format!("StickVlanA = {}", self.stick_vlans.0); }
            if line.starts_with("StickVlanB") { line = format!("StickVlanB = {}", self.stick_vlans.1); }
            if line.starts_with("sqm") { line = format!("sqm = '{}'", self.sqm); }
            if line.starts_with("upstreamBandwidthCapacityDownloadMbps") { line = format!("upstreamBandwidthCapacityDownloadMbps = {}", self.total_download_mbps); }
            if line.starts_with("upstreamBandwidthCapacityUploadMbps") { line = format!("upstreamBandwidthCapacityUploadMbps = {}", self.total_upload_mbps); }
            if line.starts_with("monitorOnlyMode") { line = format!("monitorOnlyMode = {}", if self.monitor_mode { "True" } else { "False" } ); }
            if line.starts_with("generatedPNDownloadMbps") { line = format!("generatedPNDownloadMbps = {}", self.generated_download_mbps); }
            if line.starts_with("generatedPNUploadMbps") { line = format!("generatedPNUploadMbps = {}", self.generated_upload_mbps); }
            if line.starts_with("useBinPackingToBalanceCPU") { line = format!("useBinPackingToBalanceCPU = {}", if self.use_binpacking { "True" } else { "False" } ); }
            if line.starts_with("enableActualShellCommands") { line = format!("enableActualShellCommands = {}", if self.enable_shell_commands { "True" } else { "False" } ); }
            if line.starts_with("runShellCommandsAsSudo") { line = format!("runShellCommandsAsSudo = {}", if self.run_as_sudo { "True" } else { "False" } ); }
            if line.starts_with("queuesAvailableOverride") { line = format!("queuesAvailableOverride = {}", self.override_queue_count); }
            config += &format!("{line}\n");
        }

        // Actually save to disk
        if final_path.exists() {
            remove_file(&final_path)?;
        }
        let mut file = OpenOptions::new().write(true).create_new(true).open(&final_path)?;
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

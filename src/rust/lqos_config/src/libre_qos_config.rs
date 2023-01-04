use anyhow::{Error, Result};
use std::{fs, path::{Path, PathBuf}};
use crate::etc;

pub struct LibreQoSConfig {
    pub internet_interface: String,
    pub isp_interface: String,
    pub on_a_stick_mode: bool,
    pub stick_vlans: (u16, u16),
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
        }
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

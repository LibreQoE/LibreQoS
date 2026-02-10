use std::{fs::read_to_string, path::Path};

use serde::{Deserialize, Serialize};
use anyhow::Result;
use lqos_config::ShapedDevice;

use crate::overrides_file::file_lock::FileLock;

mod file_lock;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CircuitAdjustment {
    CircuitAdjustSpeed {
        circuit_id: String,
        min_download_bandwidth: Option<f32>,
        max_download_bandwidth: Option<f32>,
        min_upload_bandwidth: Option<f32>,
        max_upload_bandwidth: Option<f32>,
    },
    DeviceAdjustSpeed {
        device_id: String,
        min_download_bandwidth: Option<f32>,
        max_download_bandwidth: Option<f32>,
        min_upload_bandwidth: Option<f32>,
        max_upload_bandwidth: Option<f32>,
    },
    RemoveCircuit { circuit_id: String },
    RemoveDevice { device_id: String },
    ReparentCircuit { circuit_id: String, parent_node: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NetworkAdjustment {
    AdjustSiteSpeed {
        site_name: String,
        download_bandwidth_mbps: Option<u32>,
        upload_bandwidth_mbps: Option<u32>,
    },
    SetNodeVirtual {
        node_name: String,
        #[serde(rename = "virtual")]
        virtual_node: bool,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UispOverrides {
    #[serde(default)]
    pub bandwidth_overrides: std::collections::HashMap<String, (f32, f32)>,
    #[serde(default)]
    pub route_overrides: Vec<UispRouteOverride>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UispRouteOverride {
    pub from_site: String,
    pub to_site: String,
    pub cost: u32,
}

#[derive(Serialize, Deserialize, Default)]
pub struct OverrideFile {
    /// Devices that will be persisted into ShapedDevices.csv by the scheduler. Useful
    /// for adding persistent "catch all", or API-controlled new devices that are somehow detached
    /// from the scheduler integration.
    #[serde(default, alias = "devices_to_append")]
    persistent_devices: Vec<ShapedDevice>,
    /// Adjustments that the scheduler will apply to circuits/devices when persisting CSV.
    #[serde(default)]
    circuit_adjustments: Vec<CircuitAdjustment>,
    /// Adjustments that affect network.json structure/settings.
    #[serde(default)]
    network_adjustments: Vec<NetworkAdjustment>,
    /// UISP integration consolidated overrides
    #[serde(default)]
    uisp: Option<UispOverrides>,
}

impl OverrideFile {
    pub fn load() -> Result<Self> {
        let lock = FileLock::new()?;
        let config = lqos_config::load_config()?;
        let path = Path::new(&config.lqos_directory).join("lqos_overrides.json");
        if !path.exists() {
            // Create a default empty file
            let new_override_file = OverrideFile::default();
            let as_json = serde_json::to_string(&new_override_file)?;
            std::fs::write(&path, as_json.as_bytes())?;
        }
        let raw = read_to_string(path)?;
        let as_json = serde_json::from_str(&raw)?;
        drop(lock); // Explicitly drop for clarity. RAII does it anyway.
        Ok(as_json)
    }

    pub fn save(&self) -> Result<()> {
        let lock = FileLock::new()?;
        let config = lqos_config::load_config()?;
        let path = Path::new(&config.lqos_directory).join("lqos_overrides.json");
        let as_json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, as_json.as_bytes())?;
        drop(lock); // Explicitly drop for clarity. RAII does it anyway.
        Ok(())
    }

    /// Borrow the list of persistent devices without modifying the file.
    pub fn persistent_devices(&self) -> &[ShapedDevice] {
        &self.persistent_devices
    }

    /// Borrow the list of circuit adjustments without modifying the file.
    pub fn circuit_adjustments(&self) -> &[CircuitAdjustment] {
        &self.circuit_adjustments
    }

    /// Borrow the list of network adjustments without modifying the file.
    pub fn network_adjustments(&self) -> &[NetworkAdjustment] {
        &self.network_adjustments
    }

    /// Borrow UISP overrides if present
    pub fn uisp(&self) -> Option<&UispOverrides> {
        self.uisp.as_ref()
    }

    /// Add or replace a shaped device by `device_id`. Returns true if changed.
    pub fn add_persistent_shaped_device_return_changed(&mut self, device: ShapedDevice) -> bool {
        if let Some(existing) = self
            .persistent_devices
            .iter()
            .find(|d| d.device_id == device.device_id)
        {
            if existing == &device {
                // No change needed
                return false;
            }
        }
        self.persistent_devices
            .retain(|d| d.device_id != device.device_id);
        self.persistent_devices.push(device);
        true
    }

    /// Remove all devices matching `circuit_id`. Returns number removed.
    pub fn remove_persistent_shaped_device_by_circuit_count(&mut self, circuit_id: &str) -> usize {
        let before = self.persistent_devices.len();
        self.persistent_devices
            .retain(|d| d.circuit_id != circuit_id);
        before.saturating_sub(self.persistent_devices.len())
    }

    /// Remove all devices matching `device_id`. Returns number removed.
    pub fn remove_persistent_shaped_device_by_device_count(&mut self, device_id: &str) -> usize {
        let before = self.persistent_devices.len();
        self.persistent_devices
            .retain(|d| d.device_id != device_id);
        before.saturating_sub(self.persistent_devices.len())
    }

    /// Add a circuit adjustment entry.
    pub fn add_circuit_adjustment(&mut self, adj: CircuitAdjustment) {
        self.circuit_adjustments.push(adj);
    }

    /// Remove a circuit adjustment by index. Returns true if removed.
    pub fn remove_circuit_adjustment_by_index(&mut self, index: usize) -> bool {
        if index < self.circuit_adjustments.len() {
            self.circuit_adjustments.remove(index);
            return true;
        }
        false
    }

    /// Add a network adjustment entry.
    pub fn add_network_adjustment(&mut self, adj: NetworkAdjustment) {
        self.network_adjustments.push(adj);
    }

    /// Add or replace a virtual-node flag for a specific network.json node name.
    pub fn set_network_node_virtual(&mut self, node_name: String, virtual_node: bool) {
        self.network_adjustments.retain(|adj| match adj {
            NetworkAdjustment::SetNodeVirtual { node_name: n, .. } => n != &node_name,
            _ => true,
        });
        self.network_adjustments.push(NetworkAdjustment::SetNodeVirtual {
            node_name,
            virtual_node,
        });
    }

    /// Remove any virtual-node overrides for `node_name`. Returns number removed.
    pub fn remove_network_node_virtual_by_name_count(&mut self, node_name: &str) -> usize {
        let before = self.network_adjustments.len();
        self.network_adjustments.retain(|adj| match adj {
            NetworkAdjustment::SetNodeVirtual { node_name: n, .. } => n != node_name,
            _ => true,
        });
        before.saturating_sub(self.network_adjustments.len())
    }

    /// Remove a network adjustment by index. Returns true if removed.
    pub fn remove_network_adjustment_by_index(&mut self, index: usize) -> bool {
        if index < self.network_adjustments.len() {
            self.network_adjustments.remove(index);
            return true;
        }
        false
    }

    fn ensure_uisp_mut(&mut self) -> &mut UispOverrides {
        if self.uisp.is_none() {
            self.uisp = Some(UispOverrides::default());
        }
        self.uisp.as_mut().unwrap()
    }

    pub fn set_uisp_bandwidth_override(&mut self, site_name: String, down: f32, up: f32) {
        let uisp = self.ensure_uisp_mut();
        uisp.bandwidth_overrides.insert(site_name, (down, up));
    }

    pub fn remove_uisp_bandwidth_override(&mut self, site_name: &str) -> bool {
        let uisp = self.ensure_uisp_mut();
        uisp.bandwidth_overrides.remove(site_name).is_some()
    }

    pub fn add_uisp_route_override(&mut self, from_site: String, to_site: String, cost: u32) {
        let uisp = self.ensure_uisp_mut();
        uisp.route_overrides.push(UispRouteOverride { from_site, to_site, cost });
    }

    pub fn remove_uisp_route_by_index(&mut self, index: usize) -> bool {
        let uisp = self.ensure_uisp_mut();
        if index < uisp.route_overrides.len() {
            uisp.route_overrides.remove(index);
            return true;
        }
        false
    }
}

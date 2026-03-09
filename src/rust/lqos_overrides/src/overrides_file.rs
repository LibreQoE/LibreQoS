use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use anyhow::Result;
use lqos_config::ShapedDevice;

use crate::overrides_file::file_lock::FileLock;

mod file_lock;

const OPERATOR_OVERRIDES_FILE: &str = "lqos_overrides.json";
const STORMGUARD_OVERRIDES_FILE: &str = "lqos_overrides.stormguard.json";
const TREEGUARD_OVERRIDES_FILE: &str = "lqos_overrides.treeguard.json";
const LEGACY_AUTOPILOT_OVERRIDES_FILE: &str = "lqos_overrides.autopilot.json";

/// Selects which overrides file to load/save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverrideLayer {
    /// Operator-owned overrides (`lqos_overrides.json`).
    Operator,
    /// StormGuard-owned overrides (`lqos_overrides.stormguard.json`).
    Stormguard,
    /// TreeGuard-owned overrides (`lqos_overrides.treeguard.json`).
    Treeguard,
}

/// Helper for working with layered override files.
pub struct OverrideStore;

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
    /// Circuit IDs excluded from RTT aggregation/summarization in the UI.
    #[serde(default)]
    rtt_excluded_circuits: Vec<String>,
    /// UISP integration consolidated overrides
    #[serde(default)]
    uisp: Option<UispOverrides>,
}

fn overrides_path(config: &lqos_config::Config, layer: OverrideLayer) -> PathBuf {
    let file = match layer {
        OverrideLayer::Operator => OPERATOR_OVERRIDES_FILE,
        OverrideLayer::Stormguard => STORMGUARD_OVERRIDES_FILE,
        OverrideLayer::Treeguard => TREEGUARD_OVERRIDES_FILE,
    };
    Path::new(&config.lqos_directory).join(file)
}

fn treeguard_read_path(config: &lqos_config::Config) -> PathBuf {
    let canonical = Path::new(&config.lqos_directory).join(TREEGUARD_OVERRIDES_FILE);
    if canonical.exists() {
        return canonical;
    }
    let legacy = Path::new(&config.lqos_directory).join(LEGACY_AUTOPILOT_OVERRIDES_FILE);
    if legacy.exists() {
        legacy
    } else {
        canonical
    }
}

fn load_from_path(path: &Path) -> Result<OverrideFile> {
    let raw = read_to_string(path)?;
    let as_json = serde_json::from_str(&raw)?;
    Ok(as_json)
}

fn ensure_exists_default(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    // Create a default empty file
    let new_override_file = OverrideFile::default();
    let as_json = serde_json::to_string(&new_override_file)?;
    std::fs::write(path, as_json.as_bytes())?;
    Ok(())
}

fn save_to_path(path: &Path, overrides: &OverrideFile) -> Result<()> {
    let as_json = serde_json::to_string_pretty(overrides)?;
    std::fs::write(path, as_json.as_bytes())?;
    Ok(())
}

fn merge_persistent_devices_by_id(
    operator_devices: &[ShapedDevice],
    stormguard_devices: &[ShapedDevice],
    treeguard_devices: &[ShapedDevice],
) -> Vec<ShapedDevice> {
    use std::collections::{HashMap, HashSet};

    let mut by_id: HashMap<&str, ShapedDevice> = HashMap::new();
    for dev in operator_devices {
        by_id.insert(dev.device_id.as_str(), dev.clone());
    }
    for dev in stormguard_devices {
        by_id.insert(dev.device_id.as_str(), dev.clone());
    }
    for dev in treeguard_devices {
        by_id.insert(dev.device_id.as_str(), dev.clone());
    }

    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();

    for dev in operator_devices {
        let did = dev.device_id.as_str();
        if seen.contains(did) {
            continue;
        }
        if let Some(merged) = by_id.get(did) {
            out.push(merged.clone());
            seen.insert(did);
        }
    }

    for dev in stormguard_devices {
        let did = dev.device_id.as_str();
        if seen.contains(did) {
            continue;
        }
        if let Some(merged) = by_id.get(did) {
            out.push(merged.clone());
            seen.insert(did);
        }
    }

    for dev in treeguard_devices {
        let did = dev.device_id.as_str();
        if seen.contains(did) {
            continue;
        }
        if let Some(merged) = by_id.get(did) {
            out.push(merged.clone());
            seen.insert(did);
        }
    }

    out
}

fn merge_network_adjustments_owned(
    operator_adjustments: &[NetworkAdjustment],
    stormguard_adjustments: &[NetworkAdjustment],
    treeguard_adjustments: &[NetworkAdjustment],
) -> Vec<NetworkAdjustment> {
    use std::collections::{HashMap, HashSet};

    let mut stormguard_site_speeds: HashMap<&str, (Option<u32>, Option<u32>)> = HashMap::new();
    let mut stormguard_site_order: Vec<&str> = Vec::new();
    let mut stormguard_site_seen: HashSet<&str> = HashSet::new();
    for adj in stormguard_adjustments {
        if let NetworkAdjustment::AdjustSiteSpeed {
            site_name,
            download_bandwidth_mbps,
            upload_bandwidth_mbps,
        } = adj
        {
            let name = site_name.as_str();
            stormguard_site_speeds.insert(name, (*download_bandwidth_mbps, *upload_bandwidth_mbps));
            if !stormguard_site_seen.contains(name) {
                stormguard_site_order.push(name);
                stormguard_site_seen.insert(name);
            }
        }
    }

    let mut treeguard_virtual: HashMap<&str, bool> = HashMap::new();
    let mut treeguard_virtual_order: Vec<&str> = Vec::new();
    let mut treeguard_virtual_seen: HashSet<&str> = HashSet::new();

    for adj in treeguard_adjustments {
        if let NetworkAdjustment::SetNodeVirtual {
            node_name,
            virtual_node,
        } = adj
        {
            let name = node_name.as_str();
            treeguard_virtual.insert(name, *virtual_node);
            if !treeguard_virtual_seen.contains(name) {
                treeguard_virtual_order.push(name);
                treeguard_virtual_seen.insert(name);
            }
        }
    }

    let mut out = Vec::new();
    let mut used_treeguard_virtual: HashSet<&str> = HashSet::new();
    let mut operator_virtual_seen: HashSet<&str> = HashSet::new();
    let mut operator_site_speed_seen: HashSet<&str> = HashSet::new();

    for adj in operator_adjustments {
        match adj {
            NetworkAdjustment::SetNodeVirtual {
                node_name,
                virtual_node,
            } => {
                let name = node_name.as_str();
                if operator_virtual_seen.contains(name) {
                    continue;
                }
                operator_virtual_seen.insert(name);

                if let Some(v) = treeguard_virtual.get(name) {
                    out.push(NetworkAdjustment::SetNodeVirtual {
                        node_name: node_name.clone(),
                        virtual_node: *v,
                    });
                    used_treeguard_virtual.insert(name);
                } else {
                    out.push(NetworkAdjustment::SetNodeVirtual {
                        node_name: node_name.clone(),
                        virtual_node: *virtual_node,
                    });
                }
            }
            NetworkAdjustment::AdjustSiteSpeed {
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
            } => {
                let name = site_name.as_str();
                if operator_site_speed_seen.contains(name) {
                    continue;
                }
                operator_site_speed_seen.insert(name);
                out.push(NetworkAdjustment::AdjustSiteSpeed {
                    site_name: site_name.clone(),
                    download_bandwidth_mbps: *download_bandwidth_mbps,
                    upload_bandwidth_mbps: *upload_bandwidth_mbps,
                });
            }
        }
    }

    for name in stormguard_site_order {
        if operator_site_speed_seen.contains(name) {
            continue;
        }
        let Some((download_bandwidth_mbps, upload_bandwidth_mbps)) = stormguard_site_speeds.get(name)
        else {
            continue;
        };
        out.push(NetworkAdjustment::AdjustSiteSpeed {
            site_name: name.to_string(),
            download_bandwidth_mbps: *download_bandwidth_mbps,
            upload_bandwidth_mbps: *upload_bandwidth_mbps,
        });
    }

    // Append TreeGuard-only virtual-node entries (ignore other network adjustments).
    for name in treeguard_virtual_order {
        if used_treeguard_virtual.contains(name) {
            continue;
        }
        let Some(v) = treeguard_virtual.get(name) else {
            continue;
        };
        out.push(NetworkAdjustment::SetNodeVirtual {
            node_name: name.to_string(),
            virtual_node: *v,
        });
    }

    out
}

fn merge_owned_sections(
    mut operator: OverrideFile,
    stormguard: OverrideFile,
    treeguard: OverrideFile,
) -> OverrideFile {
    let operator_devices = std::mem::take(&mut operator.persistent_devices);
    operator.persistent_devices = merge_persistent_devices_by_id(
        &operator_devices,
        &stormguard.persistent_devices,
        &treeguard.persistent_devices,
    );

    let operator_network = std::mem::take(&mut operator.network_adjustments);
    operator.network_adjustments = merge_network_adjustments_owned(
        &operator_network,
        &stormguard.network_adjustments,
        &treeguard.network_adjustments,
    );

    operator
}

impl OverrideFile {
    pub fn load() -> Result<Self> {
        let lock = FileLock::new()?;
        let config = lqos_config::load_config()?;
        let path = overrides_path(&config, OverrideLayer::Operator);
        ensure_exists_default(&path)?;
        let as_json = load_from_path(&path)?;
        drop(lock); // Explicitly drop for clarity. RAII does it anyway.
        Ok(as_json)
    }

    pub fn save(&self) -> Result<()> {
        let lock = FileLock::new()?;
        let config = lqos_config::load_config()?;
        let path = overrides_path(&config, OverrideLayer::Operator);
        save_to_path(&path, self)?;
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

    /// Borrow the list of circuit IDs excluded from RTT aggregation/summarization.
    pub fn rtt_excluded_circuits(&self) -> &[String] {
        &self.rtt_excluded_circuits
    }

    /// Returns true if the circuit ID is excluded from RTT aggregation/summarization.
    pub fn is_circuit_rtt_excluded(&self, circuit_id: &str) -> bool {
        self.rtt_excluded_circuits.iter().any(|c| c == circuit_id)
    }

    /// Add/remove a circuit ID from the RTT exclusion list. Returns true if changed.
    pub fn set_circuit_rtt_excluded_return_changed(
        &mut self,
        circuit_id: &str,
        excluded: bool,
    ) -> bool {
        let id = circuit_id.trim();
        if id.is_empty() {
            return false;
        }

        let matches = self
            .rtt_excluded_circuits
            .iter()
            .filter(|c| c.trim() == id)
            .count();

        if excluded {
            if matches == 1 {
                return false;
            }
            self.rtt_excluded_circuits.retain(|c| c.trim() != id);
            self.rtt_excluded_circuits.push(id.to_string());
            return true;
        }

        if matches == 0 {
            return false;
        }
        self.rtt_excluded_circuits.retain(|c| c.trim() != id);
        true
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

    /// Add or replace a site bandwidth override for `site_name`.
    pub fn set_site_bandwidth_override(
        &mut self,
        site_name: String,
        download_bandwidth_mbps: Option<u32>,
        upload_bandwidth_mbps: Option<u32>,
    ) {
        self.network_adjustments.retain(|adj| match adj {
            NetworkAdjustment::AdjustSiteSpeed { site_name: current, .. } => current != &site_name,
            _ => true,
        });
        self.network_adjustments.push(NetworkAdjustment::AdjustSiteSpeed {
            site_name,
            download_bandwidth_mbps,
            upload_bandwidth_mbps,
        });
    }

    /// Remove any site bandwidth overrides for `site_name`. Returns number removed.
    pub fn remove_site_bandwidth_override_by_name_count(&mut self, site_name: &str) -> usize {
        let before = self.network_adjustments.len();
        self.network_adjustments.retain(|adj| match adj {
            NetworkAdjustment::AdjustSiteSpeed { site_name: current, .. } => current != site_name,
            _ => true,
        });
        before.saturating_sub(self.network_adjustments.len())
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

impl OverrideStore {
    /// Loads a single overrides layer.
    ///
    /// Side effects: acquires the global overrides lock and may create the operator overrides file.
    pub fn load_layer(layer: OverrideLayer) -> Result<OverrideFile> {
        let lock = FileLock::new()?;
        let config = lqos_config::load_config()?;
        let path = match layer {
            OverrideLayer::Operator => overrides_path(&config, layer),
            OverrideLayer::Stormguard => overrides_path(&config, layer),
            OverrideLayer::Treeguard => treeguard_read_path(&config),
        };
        let overrides = match layer {
            OverrideLayer::Operator => {
                ensure_exists_default(&path)?;
                load_from_path(&path)?
            }
            OverrideLayer::Stormguard | OverrideLayer::Treeguard => {
                if !path.exists() {
                    OverrideFile::default()
                } else {
                    load_from_path(&path)?
                }
            }
        };
        drop(lock);
        Ok(overrides)
    }

    /// Saves a single overrides layer.
    ///
    /// Side effects: acquires the global overrides lock and writes the selected overrides file.
    pub fn save_layer(layer: OverrideLayer, overrides: &OverrideFile) -> Result<()> {
        let lock = FileLock::new()?;
        let config = lqos_config::load_config()?;
        let path = overrides_path(&config, layer);
        save_to_path(&path, overrides)?;
        drop(lock);
        Ok(())
    }

    /// Loads the effective overrides view used during shaping.
    ///
    /// When adaptive layers are disabled, this is equivalent to loading the operator layer only.
    ///
    /// Side effects: acquires the global overrides lock and may create the operator overrides file.
    pub fn load_effective(apply_stormguard: bool, apply_treeguard: bool) -> Result<OverrideFile> {
        let lock = FileLock::new()?;
        let config = lqos_config::load_config()?;

        let operator_path = overrides_path(&config, OverrideLayer::Operator);
        ensure_exists_default(&operator_path)?;
        let operator = load_from_path(&operator_path)?;

        if !apply_stormguard && !apply_treeguard {
            drop(lock);
            return Ok(operator);
        }

        let stormguard_path = overrides_path(&config, OverrideLayer::Stormguard);
        let stormguard = if !apply_stormguard || !stormguard_path.exists() {
            OverrideFile::default()
        } else {
            load_from_path(&stormguard_path)?
        };

        let treeguard_path = treeguard_read_path(&config);
        let treeguard = if !apply_treeguard || !treeguard_path.exists() {
            OverrideFile::default()
        } else {
            load_from_path(&treeguard_path)?
        };

        let merged = merge_owned_sections(operator, stormguard, treeguard);
        drop(lock);
        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shaped_device_with_sqm(device_id: &str, sqm: &str) -> ShapedDevice {
        let mut dev = ShapedDevice::default();
        dev.device_id = device_id.to_string();
        dev.sqm_override = Some(sqm.to_string());
        dev
    }

    #[test]
    fn effective_merge_treeguard_wins_for_persistent_devices() {
        let mut operator = OverrideFile::default();
        operator.add_persistent_shaped_device_return_changed(shaped_device_with_sqm("dev1", "cake"));
        operator.add_persistent_shaped_device_return_changed(shaped_device_with_sqm("dev2", "cake"));

        let mut stormguard = OverrideFile::default();
        stormguard
            .add_persistent_shaped_device_return_changed(shaped_device_with_sqm("dev1", "fq_codel"));
        stormguard
            .add_persistent_shaped_device_return_changed(shaped_device_with_sqm("dev3", "fq_codel"));

        let mut treeguard = OverrideFile::default();
        treeguard
            .add_persistent_shaped_device_return_changed(shaped_device_with_sqm("dev1", "cake"));
        treeguard
            .add_persistent_shaped_device_return_changed(shaped_device_with_sqm("dev4", "cake"));

        let merged = merge_owned_sections(operator, stormguard, treeguard);
        let sqm_by_id: std::collections::HashMap<&str, &str> = merged
            .persistent_devices
            .iter()
            .filter_map(|d| d.sqm_override.as_deref().map(|sqm| (d.device_id.as_str(), sqm)))
            .collect();

        assert_eq!(sqm_by_id.get("dev1"), Some(&"cake"));
        assert_eq!(sqm_by_id.get("dev2"), Some(&"cake"));
        assert_eq!(sqm_by_id.get("dev3"), Some(&"fq_codel"));
        assert_eq!(sqm_by_id.get("dev4"), Some(&"cake"));
    }

    #[test]
    fn effective_merge_treeguard_wins_for_node_virtual_only() {
        let mut operator = OverrideFile::default();
        operator.set_network_node_virtual("NodeA".to_string(), false);
        operator.add_network_adjustment(NetworkAdjustment::AdjustSiteSpeed {
            site_name: "Site1".to_string(),
            download_bandwidth_mbps: Some(100),
            upload_bandwidth_mbps: Some(50),
        });

        let mut stormguard = OverrideFile::default();
        stormguard.set_site_bandwidth_override("Site1".to_string(), Some(80), Some(40));
        stormguard.set_site_bandwidth_override("Site2".to_string(), Some(150), Some(75));

        let mut treeguard = OverrideFile::default();
        treeguard.set_network_node_virtual("NodeA".to_string(), true);
        treeguard.add_network_adjustment(NetworkAdjustment::AdjustSiteSpeed {
            site_name: "Site3".to_string(),
            download_bandwidth_mbps: Some(200),
            upload_bandwidth_mbps: Some(100),
        });

        let merged = merge_owned_sections(operator, stormguard, treeguard);

        let node_a_virtual = merged.network_adjustments().iter().find_map(|adj| match adj {
            NetworkAdjustment::SetNodeVirtual {
                node_name,
                virtual_node,
            } if node_name == "NodeA" => Some(*virtual_node),
            _ => None,
        });
        assert_eq!(node_a_virtual, Some(true));

        // Operator site speed should beat StormGuard; TreeGuard site speed should be ignored.
        let site_speed_names: Vec<&str> = merged.network_adjustments().iter().filter_map(|adj| match adj {
            NetworkAdjustment::AdjustSiteSpeed { site_name, .. } => Some(site_name.as_str()),
            _ => None,
        }).collect();
        assert_eq!(site_speed_names, vec!["Site1", "Site2"]);
    }

    #[test]
    fn override_file_defaults_accept_empty_json() {
        let of: OverrideFile = serde_json::from_str("{}").expect("empty JSON should deserialize");
        assert!(of.rtt_excluded_circuits().is_empty());
    }

    #[test]
    fn rtt_excluded_set_unset_is_idempotent() {
        let mut of = OverrideFile::default();
        assert!(!of.is_circuit_rtt_excluded("C1"));

        assert!(of.set_circuit_rtt_excluded_return_changed("C1", true));
        assert!(of.is_circuit_rtt_excluded("C1"));

        assert!(!of.set_circuit_rtt_excluded_return_changed("C1", true));

        assert!(of.set_circuit_rtt_excluded_return_changed("C1", false));
        assert!(!of.is_circuit_rtt_excluded("C1"));

        assert!(!of.set_circuit_rtt_excluded_return_changed("C1", false));
    }
}

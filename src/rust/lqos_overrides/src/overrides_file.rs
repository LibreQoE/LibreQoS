use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
};

use anyhow::Result;
use lqos_config::ShapedDevice;
use serde::{Deserialize, Serialize};

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

/// A circuit- or device-level override applied while generating shaped-device output.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CircuitAdjustment {
    /// Replaces some or all circuit bandwidth values for a specific circuit.
    CircuitAdjustSpeed {
        /// Circuit identifier to update.
        circuit_id: String,
        /// Replacement minimum download bandwidth in Mbps.
        min_download_bandwidth: Option<f32>,
        /// Replacement maximum download bandwidth in Mbps.
        max_download_bandwidth: Option<f32>,
        /// Replacement minimum upload bandwidth in Mbps.
        min_upload_bandwidth: Option<f32>,
        /// Replacement maximum upload bandwidth in Mbps.
        max_upload_bandwidth: Option<f32>,
    },
    /// Replaces some or all bandwidth values for a specific device.
    DeviceAdjustSpeed {
        /// Device identifier to update.
        device_id: String,
        /// Replacement minimum download bandwidth in Mbps.
        min_download_bandwidth: Option<f32>,
        /// Replacement maximum download bandwidth in Mbps.
        max_download_bandwidth: Option<f32>,
        /// Replacement minimum upload bandwidth in Mbps.
        min_upload_bandwidth: Option<f32>,
        /// Replacement maximum upload bandwidth in Mbps.
        max_upload_bandwidth: Option<f32>,
    },
    /// Replaces the SQM override token for a specific device without changing any other fields.
    DeviceAdjustSqm {
        /// Device identifier to update.
        device_id: String,
        /// Replacement SQM override token. `None` or empty removes the override.
        sqm_override: Option<String>,
    },
    /// Removes a circuit from generated output by circuit ID.
    RemoveCircuit {
        /// Circuit identifier to remove.
        circuit_id: String,
    },
    /// Removes a device from generated output by device ID.
    RemoveDevice {
        /// Device identifier to remove.
        device_id: String,
    },
    /// Assigns a circuit to a different parent node.
    ReparentCircuit {
        /// Circuit identifier to move.
        circuit_id: String,
        /// Target parent node name.
        parent_node: String,
    },
}

/// A network-level override applied while generating `network.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NetworkAdjustment {
    /// Replaces site bandwidth values for a named site.
    AdjustSiteSpeed {
        /// Optional stable node identifier to update.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
        /// Site name to update.
        site_name: String,
        /// Replacement download bandwidth in Mbps.
        download_bandwidth_mbps: Option<f32>,
        /// Replacement upload bandwidth in Mbps.
        upload_bandwidth_mbps: Option<f32>,
    },
    /// Marks a named node as virtual or non-virtual.
    SetNodeVirtual {
        /// Node name to update.
        node_name: String,
        #[serde(rename = "virtual")]
        /// Whether the node should be treated as virtual.
        virtual_node: bool,
    },
}

/// Consolidated UISP-specific overrides stored in an override file.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UispOverrides {
    #[serde(default)]
    /// Per-site UISP bandwidth overrides keyed by site name as `(download_mbps, upload_mbps)`.
    pub bandwidth_overrides: std::collections::HashMap<String, (f32, f32)>,
    #[serde(default)]
    /// Route overrides applied between UISP sites.
    pub route_overrides: Vec<UispRouteOverride>,
}

/// A UISP route cost override between two sites.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UispRouteOverride {
    /// Source site name.
    pub from_site: String,
    /// Destination site name.
    pub to_site: String,
    /// Replacement routing cost between the two sites.
    pub cost: u32,
}

/// The serialized contents of a single LibreQoS overrides file.
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
    if legacy.exists() { legacy } else { canonical }
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
    for dev in stormguard_devices
        .iter()
        .filter(|dev| !persistent_device_carries_sqm(dev))
    {
        by_id.insert(dev.device_id.as_str(), dev.clone());
    }
    for dev in treeguard_devices
        .iter()
        .filter(|dev| !persistent_device_carries_sqm(dev))
    {
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

    for dev in stormguard_devices
        .iter()
        .filter(|dev| !persistent_device_carries_sqm(dev))
    {
        let did = dev.device_id.as_str();
        if seen.contains(did) {
            continue;
        }
        if let Some(merged) = by_id.get(did) {
            out.push(merged.clone());
            seen.insert(did);
        }
    }

    for dev in treeguard_devices
        .iter()
        .filter(|dev| !persistent_device_carries_sqm(dev))
    {
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

fn persistent_device_carries_sqm(device: &ShapedDevice) -> bool {
    device
        .sqm_override
        .as_deref()
        .is_some_and(|sqm| !sqm.trim().is_empty())
}

fn legacy_sqm_adjustments_from_devices(devices: &[ShapedDevice]) -> Vec<CircuitAdjustment> {
    devices
        .iter()
        .filter_map(|device| {
            let sqm_override = device.sqm_override.as_deref()?.trim();
            if sqm_override.is_empty() {
                return None;
            }
            Some(CircuitAdjustment::DeviceAdjustSqm {
                device_id: device.device_id.clone(),
                sqm_override: Some(sqm_override.to_string()),
            })
        })
        .collect()
}

fn circuit_adjustment_merge_key(adj: &CircuitAdjustment) -> (&'static str, &str) {
    match adj {
        CircuitAdjustment::CircuitAdjustSpeed { circuit_id, .. } => ("circuit_speed", circuit_id),
        CircuitAdjustment::DeviceAdjustSpeed { device_id, .. } => ("device_speed", device_id),
        CircuitAdjustment::DeviceAdjustSqm { device_id, .. } => ("device_sqm", device_id),
        CircuitAdjustment::RemoveCircuit { circuit_id } => ("remove_circuit", circuit_id),
        CircuitAdjustment::RemoveDevice { device_id } => ("remove_device", device_id),
        CircuitAdjustment::ReparentCircuit { circuit_id, .. } => ("reparent_circuit", circuit_id),
    }
}

fn merge_circuit_adjustments_owned(
    operator_adjustments: &[CircuitAdjustment],
    stormguard_adjustments: &[CircuitAdjustment],
    treeguard_adjustments: &[CircuitAdjustment],
    stormguard_devices: &[ShapedDevice],
    treeguard_devices: &[ShapedDevice],
) -> Vec<CircuitAdjustment> {
    use std::collections::{HashMap, HashSet};

    let mut by_key: HashMap<(&'static str, &str), CircuitAdjustment> = HashMap::new();
    let stormguard_combined: Vec<CircuitAdjustment> = stormguard_adjustments
        .iter()
        .cloned()
        .chain(legacy_sqm_adjustments_from_devices(stormguard_devices))
        .collect();
    let treeguard_combined: Vec<CircuitAdjustment> = treeguard_adjustments
        .iter()
        .cloned()
        .chain(legacy_sqm_adjustments_from_devices(treeguard_devices))
        .collect();

    for adj in operator_adjustments {
        by_key.insert(circuit_adjustment_merge_key(adj), adj.clone());
    }
    for adj in &stormguard_combined {
        by_key.insert(circuit_adjustment_merge_key(adj), adj.clone());
    }
    for adj in &treeguard_combined {
        by_key.insert(circuit_adjustment_merge_key(adj), adj.clone());
    }

    let mut out = Vec::new();
    let mut seen: HashSet<(&'static str, &str)> = HashSet::new();

    for adj in operator_adjustments {
        let key = circuit_adjustment_merge_key(adj);
        if seen.contains(&key) {
            continue;
        }
        if let Some(merged) = by_key.get(&key) {
            out.push(merged.clone());
            seen.insert(key);
        }
    }
    for adj in &stormguard_combined {
        let key = circuit_adjustment_merge_key(adj);
        if seen.contains(&key) {
            continue;
        }
        if let Some(merged) = by_key.get(&key) {
            out.push(merged.clone());
            seen.insert(key);
        }
    }
    for adj in &treeguard_combined {
        let key = circuit_adjustment_merge_key(adj);
        if seen.contains(&key) {
            continue;
        }
        if let Some(merged) = by_key.get(&key) {
            out.push(merged.clone());
            seen.insert(key);
        }
    }

    out
}

fn merge_network_adjustments_owned(
    operator_adjustments: &[NetworkAdjustment],
    stormguard_adjustments: &[NetworkAdjustment],
    _treeguard_adjustments: &[NetworkAdjustment],
) -> Vec<NetworkAdjustment> {
    use std::collections::{HashMap, HashSet};

    let mut stormguard_site_speeds: HashMap<String, NetworkAdjustment> = HashMap::new();
    let mut stormguard_site_order: Vec<String> = Vec::new();
    let mut stormguard_site_seen: HashSet<String> = HashSet::new();
    for adj in stormguard_adjustments {
        if let NetworkAdjustment::AdjustSiteSpeed {
            node_id,
            site_name,
            download_bandwidth_mbps,
            upload_bandwidth_mbps,
        } = adj
        {
            let key = site_speed_key(node_id.as_deref(), site_name);
            stormguard_site_speeds.insert(
                key.clone(),
                NetworkAdjustment::AdjustSiteSpeed {
                    node_id: node_id.clone(),
                    site_name: site_name.clone(),
                    download_bandwidth_mbps: *download_bandwidth_mbps,
                    upload_bandwidth_mbps: *upload_bandwidth_mbps,
                },
            );
            if !stormguard_site_seen.contains(&key) {
                stormguard_site_order.push(key.clone());
                stormguard_site_seen.insert(key);
            }
        }
    }

    let mut out = Vec::new();
    let mut operator_virtual_seen: HashSet<&str> = HashSet::new();
    let mut operator_site_speed_seen: HashSet<String> = HashSet::new();
    let mut operator_site_name_seen: HashSet<String> = HashSet::new();

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
                out.push(NetworkAdjustment::SetNodeVirtual {
                    node_name: node_name.clone(),
                    virtual_node: *virtual_node,
                });
            }
            NetworkAdjustment::AdjustSiteSpeed {
                node_id,
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
            } => {
                let key = site_speed_key(node_id.as_deref(), site_name);
                if operator_site_speed_seen.contains(&key) {
                    continue;
                }
                operator_site_speed_seen.insert(key);
                operator_site_name_seen.insert(site_name.clone());
                out.push(NetworkAdjustment::AdjustSiteSpeed {
                    node_id: node_id.clone(),
                    site_name: site_name.clone(),
                    download_bandwidth_mbps: *download_bandwidth_mbps,
                    upload_bandwidth_mbps: *upload_bandwidth_mbps,
                });
            }
        }
    }

    for key in stormguard_site_order {
        if operator_site_speed_seen.contains(&key) {
            continue;
        }
        let Some(NetworkAdjustment::AdjustSiteSpeed {
            node_id,
            site_name,
            download_bandwidth_mbps,
            upload_bandwidth_mbps,
        }) = stormguard_site_speeds.get(&key)
        else {
            continue;
        };
        if operator_site_name_seen.contains(site_name) {
            continue;
        }
        out.push(NetworkAdjustment::AdjustSiteSpeed {
            node_id: node_id.clone(),
            site_name: site_name.clone(),
            download_bandwidth_mbps: *download_bandwidth_mbps,
            upload_bandwidth_mbps: *upload_bandwidth_mbps,
        });
    }

    out
}

fn merge_owned_sections(
    mut operator: OverrideFile,
    stormguard: OverrideFile,
    treeguard: OverrideFile,
) -> OverrideFile {
    let operator_circuit = std::mem::take(&mut operator.circuit_adjustments);
    let operator_devices = std::mem::take(&mut operator.persistent_devices);
    operator.persistent_devices = merge_persistent_devices_by_id(
        &operator_devices,
        &stormguard.persistent_devices,
        &treeguard.persistent_devices,
    );
    operator.circuit_adjustments = merge_circuit_adjustments_owned(
        &operator_circuit,
        &stormguard.circuit_adjustments,
        &treeguard.circuit_adjustments,
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
    /// Loads the operator-owned overrides file, creating an empty file if it does not exist.
    pub fn load() -> Result<Self> {
        let lock = FileLock::new()?;
        let config = lqos_config::load_config()?;
        let path = overrides_path(&config, OverrideLayer::Operator);
        ensure_exists_default(&path)?;
        let as_json = load_from_path(&path)?;
        drop(lock); // Explicitly drop for clarity. RAII does it anyway.
        Ok(as_json)
    }

    /// Saves this value to the operator-owned overrides file.
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
            && existing == &device
        {
            // No change needed
            return false;
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
        self.persistent_devices.retain(|d| d.device_id != device_id);
        before.saturating_sub(self.persistent_devices.len())
    }

    /// Add a circuit adjustment entry.
    pub fn add_circuit_adjustment(&mut self, adj: CircuitAdjustment) {
        self.circuit_adjustments.push(adj);
    }

    /// Add or replace an SQM override token for `device_id`. Returns true if changed.
    pub fn set_device_sqm_override_return_changed(
        &mut self,
        device_id: String,
        sqm_override: Option<String>,
    ) -> bool {
        let normalized = sqm_override.and_then(|sqm| {
            let trimmed = sqm.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        if self.circuit_adjustments.iter().any(|adj| {
            matches!(
                adj,
                CircuitAdjustment::DeviceAdjustSqm {
                    device_id: current,
                    sqm_override: existing_sqm,
                } if current == &device_id && existing_sqm == &normalized
            )
        }) {
            return false;
        }

        self.circuit_adjustments.retain(|adj| {
            !matches!(
                adj,
                CircuitAdjustment::DeviceAdjustSqm {
                    device_id: current, ..
                } if current == &device_id
            )
        });
        self.circuit_adjustments
            .push(CircuitAdjustment::DeviceAdjustSqm {
                device_id,
                sqm_override: normalized,
            });
        true
    }

    /// Remove any SQM override adjustments for `device_id`. Returns number removed.
    pub fn remove_device_sqm_override_by_device_count(&mut self, device_id: &str) -> usize {
        let before = self.circuit_adjustments.len();
        self.circuit_adjustments.retain(|adj| {
            !matches!(
                adj,
                CircuitAdjustment::DeviceAdjustSqm {
                    device_id: current, ..
                } if current == device_id
            )
        });
        before.saturating_sub(self.circuit_adjustments.len())
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

    /// Returns the stored site bandwidth override that best matches this node.
    ///
    /// Preference order:
    /// 1. Exact `node_id` match when one is supplied.
    /// 2. Legacy name-only match for the same `site_name`.
    pub fn find_site_bandwidth_override(
        &self,
        node_id: Option<&str>,
        site_name: &str,
    ) -> Option<&NetworkAdjustment> {
        if let Some(node_id) = node_id
            && let Some(found) = self.network_adjustments.iter().find(|adj| {
                matches!(
                    adj,
                    NetworkAdjustment::AdjustSiteSpeed {
                        node_id: Some(current_node_id),
                        ..
                    } if current_node_id == node_id
                )
            })
        {
            return Some(found);
        }

        self.network_adjustments.iter().find(|adj| {
            matches!(
                adj,
                NetworkAdjustment::AdjustSiteSpeed {
                    node_id: None,
                    site_name: current_site_name,
                    ..
                } if current_site_name == site_name
            )
        })
    }

    /// Add or replace a site bandwidth override for `site_name`.
    pub fn set_site_bandwidth_override(
        &mut self,
        node_id: Option<String>,
        site_name: String,
        download_bandwidth_mbps: Option<f32>,
        upload_bandwidth_mbps: Option<f32>,
    ) -> bool {
        let desired = NetworkAdjustment::AdjustSiteSpeed {
            node_id: node_id.clone(),
            site_name: site_name.clone(),
            download_bandwidth_mbps,
            upload_bandwidth_mbps,
        };
        if self.find_site_bandwidth_override(node_id.as_deref(), &site_name) == Some(&desired) {
            return false;
        }

        self.network_adjustments.retain(|adj| match adj {
            NetworkAdjustment::AdjustSiteSpeed {
                node_id: current_node_id,
                site_name: current_site_name,
                ..
            } => !site_speed_override_matches(
                current_node_id.as_deref(),
                current_site_name,
                node_id.as_deref(),
                &site_name,
            ),
            _ => true,
        });
        self.network_adjustments.push(desired);
        true
    }

    /// Remove any site bandwidth overrides for `site_name` or `node_id`. Returns number removed.
    pub fn remove_site_bandwidth_override_count(
        &mut self,
        node_id: Option<&str>,
        site_name: &str,
    ) -> usize {
        let before = self.network_adjustments.len();
        self.network_adjustments.retain(|adj| match adj {
            NetworkAdjustment::AdjustSiteSpeed {
                node_id: current_node_id,
                site_name: current_site_name,
                ..
            } => !site_speed_override_matches(
                current_node_id.as_deref(),
                current_site_name,
                node_id,
                site_name,
            ),
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
        self.network_adjustments
            .push(NetworkAdjustment::SetNodeVirtual {
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

    /// Adds or replaces the UISP bandwidth override for `site_name`.
    pub fn set_uisp_bandwidth_override(&mut self, site_name: String, down: f32, up: f32) {
        let uisp = self.ensure_uisp_mut();
        uisp.bandwidth_overrides.insert(site_name, (down, up));
    }

    /// Removes the UISP bandwidth override for `site_name`. Returns `true` when one existed.
    pub fn remove_uisp_bandwidth_override(&mut self, site_name: &str) -> bool {
        let uisp = self.ensure_uisp_mut();
        uisp.bandwidth_overrides.remove(site_name).is_some()
    }

    /// Appends a UISP route cost override.
    pub fn add_uisp_route_override(&mut self, from_site: String, to_site: String, cost: u32) {
        let uisp = self.ensure_uisp_mut();
        uisp.route_overrides.push(UispRouteOverride {
            from_site,
            to_site,
            cost,
        });
    }

    /// Removes a UISP route override by index. Returns `true` when the index was valid.
    pub fn remove_uisp_route_by_index(&mut self, index: usize) -> bool {
        let uisp = self.ensure_uisp_mut();
        if index < uisp.route_overrides.len() {
            uisp.route_overrides.remove(index);
            return true;
        }
        false
    }
}

fn site_speed_key(node_id: Option<&str>, site_name: &str) -> String {
    match node_id {
        Some(node_id) if !node_id.trim().is_empty() => format!("id:{node_id}"),
        _ => format!("name:{site_name}"),
    }
}

fn site_speed_override_matches(
    current_node_id: Option<&str>,
    current_site_name: &str,
    requested_node_id: Option<&str>,
    requested_site_name: &str,
) -> bool {
    if let Some(requested_node_id) = requested_node_id {
        if current_node_id == Some(requested_node_id) {
            return true;
        }
        return current_node_id.is_none() && current_site_name == requested_site_name;
    }

    current_site_name == requested_site_name
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
        ShapedDevice {
            device_id: device_id.to_string(),
            sqm_override: Some(sqm.to_string()),
            ..ShapedDevice::default()
        }
    }

    fn shaped_device_with_comment(device_id: &str, comment: &str) -> ShapedDevice {
        ShapedDevice {
            device_id: device_id.to_string(),
            comment: comment.to_string(),
            ..ShapedDevice::default()
        }
    }

    #[test]
    fn effective_merge_treeguard_wins_for_persistent_devices() {
        let mut operator = OverrideFile::default();
        operator.add_persistent_shaped_device_return_changed(shaped_device_with_comment(
            "dev1", "operator",
        ));
        operator.add_persistent_shaped_device_return_changed(shaped_device_with_comment(
            "dev2", "operator",
        ));

        let mut stormguard = OverrideFile::default();
        stormguard.add_persistent_shaped_device_return_changed(shaped_device_with_comment(
            "dev1",
            "stormguard",
        ));
        stormguard.add_persistent_shaped_device_return_changed(shaped_device_with_comment(
            "dev3",
            "stormguard",
        ));

        let mut treeguard = OverrideFile::default();
        treeguard.add_persistent_shaped_device_return_changed(shaped_device_with_comment(
            "dev1",
            "treeguard",
        ));
        treeguard.add_persistent_shaped_device_return_changed(shaped_device_with_comment(
            "dev4",
            "treeguard",
        ));

        let merged = merge_owned_sections(operator, stormguard, treeguard);
        let comments_by_id: std::collections::HashMap<&str, &str> = merged
            .persistent_devices
            .iter()
            .map(|d| (d.device_id.as_str(), d.comment.as_str()))
            .collect();

        assert_eq!(comments_by_id.get("dev1"), Some(&"treeguard"));
        assert_eq!(comments_by_id.get("dev2"), Some(&"operator"));
        assert_eq!(comments_by_id.get("dev3"), Some(&"stormguard"));
        assert_eq!(comments_by_id.get("dev4"), Some(&"treeguard"));
    }

    #[test]
    fn effective_merge_moves_adaptive_legacy_sqm_into_circuit_adjustments() {
        let mut operator = OverrideFile::default();
        let mut operator_device = shaped_device_with_sqm("operator_dev", "cake");
        operator_device.download_max_mbps = 100.0;
        operator.add_persistent_shaped_device_return_changed(operator_device);

        let mut stormguard = OverrideFile::default();
        let mut sg_device = shaped_device_with_sqm("dev1", "fq_codel");
        sg_device.download_max_mbps = 10.0;
        stormguard.add_persistent_shaped_device_return_changed(sg_device);

        let mut treeguard = OverrideFile::default();
        let mut tg_device = shaped_device_with_sqm("dev2", "cake");
        tg_device.download_max_mbps = 20.0;
        treeguard.add_persistent_shaped_device_return_changed(tg_device);

        let merged = merge_owned_sections(operator, stormguard, treeguard);

        let merged_ids: Vec<&str> = merged
            .persistent_devices()
            .iter()
            .map(|d| d.device_id.as_str())
            .collect();
        assert_eq!(merged_ids, vec!["operator_dev"]);

        let sqm_by_id: std::collections::HashMap<&str, &str> = merged
            .circuit_adjustments()
            .iter()
            .filter_map(|adj| match adj {
                CircuitAdjustment::DeviceAdjustSqm {
                    device_id,
                    sqm_override,
                } => sqm_override.as_deref().map(|sqm| (device_id.as_str(), sqm)),
                _ => None,
            })
            .collect();
        assert_eq!(sqm_by_id.get("dev1"), Some(&"fq_codel"));
        assert_eq!(sqm_by_id.get("dev2"), Some(&"cake"));
    }

    #[test]
    fn set_device_sqm_override_is_idempotent() {
        let mut of = OverrideFile::default();
        assert!(of.set_device_sqm_override_return_changed(
            "dev1".to_string(),
            Some("fq_codel".to_string())
        ));
        assert!(!of.set_device_sqm_override_return_changed(
            "dev1".to_string(),
            Some("fq_codel".to_string())
        ));
        assert!(
            of.set_device_sqm_override_return_changed("dev1".to_string(), Some("cake".to_string()))
        );
        assert_eq!(of.remove_device_sqm_override_by_device_count("dev1"), 1);
        assert_eq!(of.remove_device_sqm_override_by_device_count("dev1"), 0);
    }

    #[test]
    fn effective_merge_keeps_operator_node_virtual_and_ignores_treeguard_runtime_virtualization() {
        let mut operator = OverrideFile::default();
        operator.set_network_node_virtual("NodeA".to_string(), false);
        operator.add_network_adjustment(NetworkAdjustment::AdjustSiteSpeed {
            node_id: Some("node-site-1".to_string()),
            site_name: "Site1".to_string(),
            download_bandwidth_mbps: Some(100.0),
            upload_bandwidth_mbps: Some(50.0),
        });

        let mut stormguard = OverrideFile::default();
        stormguard.set_site_bandwidth_override(None, "Site1".to_string(), Some(80.0), Some(40.0));
        stormguard.set_site_bandwidth_override(None, "Site2".to_string(), Some(150.0), Some(75.0));

        let mut treeguard = OverrideFile::default();
        treeguard.set_network_node_virtual("NodeA".to_string(), true);
        treeguard.add_network_adjustment(NetworkAdjustment::AdjustSiteSpeed {
            node_id: Some("node-site-3".to_string()),
            site_name: "Site3".to_string(),
            download_bandwidth_mbps: Some(200.0),
            upload_bandwidth_mbps: Some(100.0),
        });

        let merged = merge_owned_sections(operator, stormguard, treeguard);

        let node_a_virtual = merged
            .network_adjustments()
            .iter()
            .find_map(|adj| match adj {
                NetworkAdjustment::SetNodeVirtual {
                    node_name,
                    virtual_node,
                } if node_name == "NodeA" => Some(*virtual_node),
                _ => None,
            });
        assert_eq!(node_a_virtual, Some(false));

        let node_three_virtual = merged
            .network_adjustments()
            .iter()
            .find_map(|adj| match adj {
                NetworkAdjustment::SetNodeVirtual {
                    node_name,
                    virtual_node,
                } if node_name == "Node3" => Some(*virtual_node),
                _ => None,
            });
        assert_eq!(node_three_virtual, None);

        // Operator site speed should beat StormGuard; TreeGuard site speed should be ignored.
        let site_speed_names: Vec<&str> = merged
            .network_adjustments()
            .iter()
            .filter_map(|adj| match adj {
                NetworkAdjustment::AdjustSiteSpeed { site_name, .. } => Some(site_name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(site_speed_names, vec!["Site1", "Site2"]);
    }

    #[test]
    fn find_site_bandwidth_override_prefers_node_id_match() {
        let mut of = OverrideFile::default();
        of.set_site_bandwidth_override(None, "AP27".to_string(), Some(80.0), Some(40.0));
        of.set_site_bandwidth_override(
            Some("node-ap27".to_string()),
            "AP27".to_string(),
            Some(120.5),
            Some(60.25),
        );

        let found = of.find_site_bandwidth_override(Some("node-ap27"), "AP27");
        assert!(matches!(
            found,
            Some(NetworkAdjustment::AdjustSiteSpeed {
                node_id: Some(node_id),
                download_bandwidth_mbps: Some(download),
                upload_bandwidth_mbps: Some(upload),
                ..
            }) if node_id == "node-ap27" && *download == 120.5 && *upload == 60.25
        ));
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

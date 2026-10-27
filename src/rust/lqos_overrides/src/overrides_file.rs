use std::{
    fs::read_to_string,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Result;
use lqos_config::ShapedDevice;
use serde::{Deserialize, Serialize};

use crate::file_lock::FileLock;
use crate::topology_overrides::TopologyOverridesFile;

const OPERATOR_OVERRIDES_FILE: &str = "lqos_overrides.json";
const STORMGUARD_OVERRIDES_FILE: &str = "lqos_overrides.stormguard.json";

/// Selects which overrides file to load/save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverrideLayer {
    /// Operator-owned overrides (`lqos_overrides.json`).
    Operator,
    /// StormGuard-owned overrides (`lqos_overrides.stormguard.json`).
    Stormguard,
}

/// Helper for working with layered override files.
pub struct OverrideStore;

/// A circuit- or device-level override applied while generating shaped-device output.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
    /// K1b ph2: sets a circuit-level SQM/AQM token (the StormGuard circuit
    /// fallback wire). Applied as a POST-pass on the assembled circuit rows so
    /// it outranks any device-level seed. Effective grammar is `none | cake`.
    CircuitAdjustAqm {
        /// Circuit identifier to update.
        circuit_id: String,
        /// Replacement AQM token. `none`/empty clears the fallback.
        aqm_token: String,
        /// K1b ph2 §9.2: unix-seconds expiry for machine-authored
        /// (StormGuard) entries. Operator entries are `None` (immortal);
        /// StormGuard entries are refreshed on every reasserting pass — the
        /// dead-man's switch, exactly the `AdjustSiteSpeed` precedent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at_unix: Option<u64>,
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
        /// K1b §4.6 pin 1: unix-seconds expiry for machine-authored
        /// (StormGuard) entries. Operator entries are `None` (immortal);
        /// StormGuard entries are refreshed on every reasserting pass — the
        /// dead-man's switch bounding a clamp whose detector stopped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at_unix: Option<u64>,
    },
    /// Overrides immediate upstream parent selection for a topology node.
    TopologyParentOverride {
        /// Stable node identifier from `network.json`.
        node_id: String,
        /// Display name of the node being overridden.
        node_name: String,
        /// Override behavior mode.
        mode: TopologyParentOverrideMode,
        /// Ordered parent node identifiers.
        parent_node_ids: Vec<String>,
        /// Ordered parent display names matching `parent_node_ids`.
        parent_node_names: Vec<String>,
    },
}

/// Override behavior for topology parent selection.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopologyParentOverrideMode {
    /// Force one immediate parent.
    Pinned,
    /// Try parents in operator-specified order, then fall back to native selection.
    PreferredOrder,
}

/// The suspend floor in Mbps (the config suspend encoding: the v3
/// compiler rounds 0.01 to 0, the broker floors 0/0 to the trickle —
/// docs/e3_derivation.md.
pub const SUSPEND_FLOOR_MBPS: f32 = 0.01;

/// A circuit-level speed override as exposed to admin callers.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CircuitSpeedOverride {
    /// Replacement minimum download bandwidth in Mbps (`None` = keep base).
    pub min_download_bandwidth: Option<f32>,
    /// Replacement maximum download bandwidth in Mbps (`None` = keep base).
    pub max_download_bandwidth: Option<f32>,
    /// Replacement minimum upload bandwidth in Mbps (`None` = keep base).
    pub min_upload_bandwidth: Option<f32>,
    /// Replacement maximum upload bandwidth in Mbps (`None` = keep base).
    pub max_upload_bandwidth: Option<f32>,
}

impl CircuitSpeedOverride {
    /// True when every direction is floored at
    /// [`SUSPEND_FLOOR_MBPS`] — the suspend encoding written by the
    /// circuit page's suspend toggle.
    pub fn is_suspend_floor(&self) -> bool {
        [
            self.min_download_bandwidth,
            self.max_download_bandwidth,
            self.min_upload_bandwidth,
            self.max_upload_bandwidth,
        ]
        .iter()
        .all(|value| value.is_some_and(|value| value == SUSPEND_FLOOR_MBPS))
    }

    /// The suspend-floor override for every direction.
    pub fn suspend_floor() -> Self {
        Self {
            min_download_bandwidth: Some(SUSPEND_FLOOR_MBPS),
            max_download_bandwidth: Some(SUSPEND_FLOOR_MBPS),
            min_upload_bandwidth: Some(SUSPEND_FLOOR_MBPS),
            max_upload_bandwidth: Some(SUSPEND_FLOOR_MBPS),
        }
    }
}

/// Consolidated UISP-specific overrides stored in an override file.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UispOverrides {
    #[serde(default)]
    /// Per-site UISP bandwidth overrides keyed by site name as `(download_mbps, upload_mbps)`.
    /// Deprecated legacy entries. Current UISP builds ignore these and use
    /// operator `AdjustSiteSpeed` entries instead.
    pub bandwidth_overrides: std::collections::HashMap<String, (f32, f32)>,
    #[serde(default)]
    /// Deprecated legacy UISP route overrides. Current UISP builds ignore these
    /// entries and use Topology Manager parent or attachment preferences instead.
    pub route_overrides: Vec<UispRouteOverride>,
}

/// A UISP route cost override between two sites.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UispRouteOverride {
    /// Source site name for a deprecated legacy UISP route override.
    pub from_site: String,
    /// Destination site name for a deprecated legacy UISP route override.
    pub to_site: String,
    /// Replacement routing cost between the two sites. Current UISP builds ignore
    /// this value.
    pub cost: u32,
}

/// The serialized contents of a single LibreQoS overrides file.
#[derive(Serialize, Deserialize, Default, Clone)]
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
    /// Topology-manager operator overrides. Rung 1.5 §1.2: the former
    /// `topology_overrides.json` folded into this single operator file.
    #[serde(default)]
    topology: TopologyOverridesFile,
}

fn overrides_path(config: &lqos_config::Config, layer: OverrideLayer) -> PathBuf {
    let file = match layer {
        OverrideLayer::Operator => OPERATOR_OVERRIDES_FILE,
        OverrideLayer::Stormguard => STORMGUARD_OVERRIDES_FILE,
    };
    Path::new(&config.lqos_directory).join(file)
}

fn load_from_path(path: &Path) -> Result<OverrideFile> {
    let raw = read_to_string(path)?;
    if raw.trim().is_empty() {
        anyhow::bail!("Overrides file '{}' was empty", path.display());
    }
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
    write_atomic(path, as_json.as_bytes())?;
    Ok(())
}

fn save_to_path(path: &Path, overrides: &OverrideFile) -> Result<()> {
    let as_json = serde_json::to_string_pretty(overrides)?;
    write_atomic(path, as_json.as_bytes())?;
    Ok(())
}

/// Rung 1.5 §1.2: fold a legacy `topology_overrides.json` into the operator
/// file's `topology` section ONCE. On first sight: import, back up, WARN, and
/// never read the old file again. An absent file is a no-op, and no load path
/// re-creates it.
pub(crate) fn migrate_legacy_topology_overrides(config: &lqos_config::Config) -> Result<()> {
    let legacy_path = TopologyOverridesFile::path_for_config(config);
    if !legacy_path.exists() {
        return Ok(());
    }
    let operator_path = overrides_path(config, OverrideLayer::Operator);
    let mut operator = if operator_path.exists() {
        load_from_path(&operator_path)?
    } else {
        OverrideFile::default()
    };
    if operator.topology().is_empty() {
        let legacy = TopologyOverridesFile::load_from_explicit_path(&legacy_path)?;
        operator.set_topology(legacy);
        save_to_path(&operator_path, &operator)?;
    }
    let backup_path = legacy_path.with_file_name("topology_overrides.json.migrated");
    std::fs::rename(&legacy_path, &backup_path)?;
    tracing::warn!(
        "Migrated legacy topology_overrides.json into lqos_overrides.json (backup: {}). \
         The old file is never read again.",
        backup_path.display()
    );
    Ok(())
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "Overrides path '{}' has no parent directory",
            path.display()
        )
    })?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Overrides path '{}' has no file name", path.display()))?;
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!("{}.tmp.{}.{}", file_name.to_string_lossy(), pid, nanos);
    let tmp_path = parent.join(tmp_name);

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);

    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn merge_persistent_devices_by_id(
    operator_devices: &[ShapedDevice],
    stormguard_devices: &[ShapedDevice],
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
        CircuitAdjustment::CircuitAdjustAqm { circuit_id, .. } => ("circuit_aqm", circuit_id),
        CircuitAdjustment::RemoveCircuit { circuit_id } => ("remove_circuit", circuit_id),
        CircuitAdjustment::RemoveDevice { device_id } => ("remove_device", device_id),
        CircuitAdjustment::ReparentCircuit { circuit_id, .. } => ("reparent_circuit", circuit_id),
    }
}

fn merge_circuit_adjustments_owned(
    operator_adjustments: &[CircuitAdjustment],
    stormguard_adjustments: &[CircuitAdjustment],
    stormguard_devices: &[ShapedDevice],
) -> Vec<CircuitAdjustment> {
    use std::collections::{HashMap, HashSet};

    let mut by_key: HashMap<(&'static str, &str), CircuitAdjustment> = HashMap::new();
    let stormguard_combined: Vec<CircuitAdjustment> = stormguard_adjustments
        .iter()
        .cloned()
        .chain(legacy_sqm_adjustments_from_devices(stormguard_devices))
        .collect();

    for adj in operator_adjustments {
        by_key.insert(circuit_adjustment_merge_key(adj), adj.clone());
    }
    for adj in &stormguard_combined {
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

    out
}

fn merge_network_adjustments_owned(
    operator_adjustments: &[NetworkAdjustment],
    stormguard_adjustments: &[NetworkAdjustment],
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
            expires_at_unix,
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
                    expires_at_unix: *expires_at_unix,
                },
            );
            if !stormguard_site_seen.contains(&key) {
                stormguard_site_order.push(key.clone());
                stormguard_site_seen.insert(key);
            }
        }
    }

    let mut out = Vec::new();
    let mut operator_site_speed_seen: HashSet<String> = HashSet::new();
    let mut operator_site_name_seen: HashSet<String> = HashSet::new();
    let mut operator_topology_seen: HashSet<&str> = HashSet::new();

    for adj in operator_adjustments {
        match adj {
            NetworkAdjustment::AdjustSiteSpeed {
                node_id,
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
                expires_at_unix,
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
                    expires_at_unix: *expires_at_unix,
                });
            }
            NetworkAdjustment::TopologyParentOverride {
                node_id,
                node_name,
                mode,
                parent_node_ids,
                parent_node_names,
            } => {
                let node_id_ref = node_id.as_str();
                if operator_topology_seen.contains(node_id_ref) {
                    continue;
                }
                operator_topology_seen.insert(node_id_ref);
                out.push(NetworkAdjustment::TopologyParentOverride {
                    node_id: node_id.clone(),
                    node_name: node_name.clone(),
                    mode: *mode,
                    parent_node_ids: parent_node_ids.clone(),
                    parent_node_names: parent_node_names.clone(),
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
            expires_at_unix,
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
            expires_at_unix: *expires_at_unix,
        });
    }

    out
}

fn merge_owned_sections(mut operator: OverrideFile, stormguard: OverrideFile) -> OverrideFile {
    let operator_circuit = std::mem::take(&mut operator.circuit_adjustments);
    let operator_devices = std::mem::take(&mut operator.persistent_devices);
    operator.persistent_devices =
        merge_persistent_devices_by_id(&operator_devices, &stormguard.persistent_devices);
    operator.circuit_adjustments = merge_circuit_adjustments_owned(
        &operator_circuit,
        &stormguard.circuit_adjustments,
        &stormguard.persistent_devices,
    );

    let operator_network = std::mem::take(&mut operator.network_adjustments);
    operator.network_adjustments =
        merge_network_adjustments_owned(&operator_network, &stormguard.network_adjustments);

    operator
}

impl OverrideFile {
    /// Returns the canonical operator overrides path for `config`.
    ///
    /// This function is pure: it has no side effects.
    pub fn operator_path_for_config(config: &lqos_config::Config) -> PathBuf {
        overrides_path(config, OverrideLayer::Operator)
    }

    /// Loads overrides from an explicit filesystem path.
    ///
    /// Side effects: reads the selected overrides file from disk.
    pub fn load_from_explicit_path(path: &Path) -> Result<Self> {
        load_from_path(path)
    }

    /// Saves overrides to an explicit filesystem path.
    ///
    /// Side effects: writes the selected overrides file to disk.
    pub fn save_to_explicit_path(&self, path: &Path) -> Result<()> {
        save_to_path(path, self)
    }

    /// Loads the operator-owned overrides file, creating an empty file if it does not exist.
    pub fn load() -> Result<Self> {
        let lock = FileLock::new_for_operation("load operator overrides")?;
        let config = lqos_config::load_config()?;
        let result = Self::load_for_config(&config);
        drop(lock); // Explicitly drop for clarity. RAII does it anyway.
        result
    }

    /// Phase D: READ-ONLY variant of [`Self::load`] — never creates or migrates
    /// the operator file. Read callers use this.
    pub fn load_read_only() -> Result<Self> {
        let lock = FileLock::new_for_operation("load operator overrides")?;
        let config = lqos_config::load_config()?;
        let path = overrides_path(&config, OverrideLayer::Operator);
        let result = if path.exists() {
            load_from_path(&path)
        } else {
            Ok(OverrideFile::default())
        };
        drop(lock);
        result
    }

    /// Loads the operator-owned overrides file for an EXPLICIT config, running
    /// the one-time topology-overrides migration first.
    ///
    /// Side effects: may migrate/back up `topology_overrides.json` and create
    /// an empty `lqos_overrides.json`.
    pub fn load_for_config(config: &lqos_config::Config) -> Result<Self> {
        migrate_legacy_topology_overrides(config)?;
        let path = overrides_path(config, OverrideLayer::Operator);
        ensure_exists_default(&path)?;
        load_from_path(&path)
    }

    /// Saves this value to the operator-owned overrides file.
    pub fn save(&self) -> Result<()> {
        let lock = FileLock::new_for_operation("save operator overrides")?;
        let config = lqos_config::load_config()?;
        let result = self.save_for_config(&config);
        drop(lock); // Explicitly drop for clarity. RAII does it anyway.
        result
    }

    /// Saves this value to the operator-owned overrides file for an EXPLICIT config.
    pub fn save_for_config(&self, config: &lqos_config::Config) -> Result<()> {
        let path = overrides_path(config, OverrideLayer::Operator);
        save_to_path(&path, self)
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

    /// Borrow the topology-manager operator overrides section.
    pub fn topology(&self) -> &TopologyOverridesFile {
        &self.topology
    }

    /// Replace the topology-manager operator overrides section.
    pub fn set_topology(&mut self, topology: TopologyOverridesFile) {
        self.topology = topology;
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

    /// K1b ph2: add or replace the circuit-level AQM token for `circuit_id`,
    /// carrying an optional unix-seconds expiry (the §9.2 dead-man's switch).
    /// `None`/empty token removes the entry. Returns true if the stored layer
    /// changed.
    pub fn set_circuit_aqm_return_changed(
        &mut self,
        circuit_id: String,
        aqm_token: Option<String>,
        expires_at_unix: Option<u64>,
    ) -> bool {
        let normalized = aqm_token.and_then(|token| {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        if self.circuit_adjustments.iter().any(|adj| {
            matches!(
                adj,
                CircuitAdjustment::CircuitAdjustAqm {
                    circuit_id: current,
                    aqm_token: existing,
                    expires_at_unix: existing_expires,
                } if current == &circuit_id
                    && Some(existing) == normalized.as_ref()
                    && existing_expires == &expires_at_unix
            )
        }) {
            return false;
        }

        self.circuit_adjustments.retain(|adj| {
            !matches!(
                adj,
                CircuitAdjustment::CircuitAdjustAqm {
                    circuit_id: current, ..
                } if current == &circuit_id
            )
        });
        if let Some(token) = normalized {
            self.circuit_adjustments
                .push(CircuitAdjustment::CircuitAdjustAqm {
                    circuit_id,
                    aqm_token: token,
                    expires_at_unix,
                });
        }
        true
    }

    /// K1b ph2 §9.2: drop circuit-level AQM entries whose machine-authored TTL
    /// has lapsed (`expires_at_unix <= now_unix`). Operator entries (`None`) are
    /// immortal. Returns the dropped circuit ids so the caller can WARN once per
    /// entry.
    pub fn remove_lapsed_circuit_aqm_overrides(&mut self, now_unix: u64) -> Vec<String> {
        let mut removed = Vec::new();
        self.circuit_adjustments.retain(|adj| match adj {
            CircuitAdjustment::CircuitAdjustAqm {
                circuit_id,
                expires_at_unix: Some(expires),
                ..
            } if *expires <= now_unix => {
                removed.push(circuit_id.clone());
                false
            }
            _ => true,
        });
        removed
    }

    /// K1b ph2: remove any circuit-level AQM entry for `circuit_id`.
    /// Returns number removed.
    pub fn remove_circuit_aqm_by_circuit_count(&mut self, circuit_id: &str) -> usize {
        let before = self.circuit_adjustments.len();
        self.circuit_adjustments.retain(|adj| {
            !matches!(
                adj,
                CircuitAdjustment::CircuitAdjustAqm {
                    circuit_id: current, ..
                } if current == circuit_id
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

    /// Upsert the operator circuit-speed override for one circuit
    /// (merge-keyed: a prior `CircuitAdjustSpeed` for the same circuit
    /// id is replaced). Returns true if the stored adjustments changed.
    ///
    /// Validation: every PRESENT rate must be finite and > 0 — raw
    /// zero is never a valid operator rate (0/0 is the suspend
    /// encoding; suspension is the explicit 0.01 floor, never a rate
    /// edit), and NaN/inf would poison every later compile.
    pub fn set_circuit_speed_return_changed(
        &mut self,
        circuit_id: &str,
        speeds: CircuitSpeedOverride,
    ) -> Result<bool, String> {
        for (label, value) in [
            ("min_download_bandwidth", speeds.min_download_bandwidth),
            ("max_download_bandwidth", speeds.max_download_bandwidth),
            ("min_upload_bandwidth", speeds.min_upload_bandwidth),
            ("max_upload_bandwidth", speeds.max_upload_bandwidth),
        ] {
            if let Some(value) = value {
                if !value.is_finite() {
                    return Err(format!("{label} must be finite"));
                }
                if value <= 0.0 {
                    return Err(format!(
                        "{label} must be > 0 (to suspend a circuit use the {SUSPEND_FLOOR_MBPS} Mbps floor)"
                    ));
                }
            }
        }
        let replacement = CircuitAdjustment::CircuitAdjustSpeed {
            circuit_id: circuit_id.to_string(),
            min_download_bandwidth: speeds.min_download_bandwidth,
            max_download_bandwidth: speeds.max_download_bandwidth,
            min_upload_bandwidth: speeds.min_upload_bandwidth,
            max_upload_bandwidth: speeds.max_upload_bandwidth,
        };
        let existing = self
            .circuit_adjustments
            .iter()
            .position(|adj| {
                matches!(adj, CircuitAdjustment::CircuitAdjustSpeed { circuit_id: existing, .. }
                    if existing == circuit_id)
            });
        if let Some(index) = existing {
            if self.circuit_adjustments[index] == replacement {
                return Ok(false);
            }
            self.circuit_adjustments[index] = replacement;
        } else {
            self.circuit_adjustments.push(replacement);
        }
        Ok(true)
    }

    /// Removes any operator circuit-speed override for one circuit.
    /// Returns how many entries were removed (0 or 1 — the merge key
    /// keeps at most one).
    pub fn remove_circuit_speed_by_circuit_count(&mut self, circuit_id: &str) -> usize {
        let before = self.circuit_adjustments.len();
        self.circuit_adjustments
            .retain(|adj| !matches!(adj, CircuitAdjustment::CircuitAdjustSpeed { circuit_id: existing, .. }
                if existing == circuit_id));
        before - self.circuit_adjustments.len()
    }

    /// Returns the operator circuit-speed override for one circuit, if set.
    pub fn circuit_speed_override(&self, circuit_id: &str) -> Option<CircuitSpeedOverride> {
        self.circuit_adjustments.iter().find_map(|adj| match adj {
            CircuitAdjustment::CircuitAdjustSpeed {
                circuit_id: existing,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } if existing == circuit_id => Some(CircuitSpeedOverride {
                min_download_bandwidth: *min_download_bandwidth,
                max_download_bandwidth: *max_download_bandwidth,
                min_upload_bandwidth: *min_upload_bandwidth,
                max_upload_bandwidth: *max_upload_bandwidth,
            }),
            _ => None,
        })
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
        self.set_site_bandwidth_override_with_expiry(
            node_id,
            site_name,
            download_bandwidth_mbps,
            upload_bandwidth_mbps,
            None,
        )
    }

    /// K1b §4.6 pin 1: add or replace a site bandwidth override carrying an
    /// optional unix-seconds expiry. `None` = immortal (operator); `Some` =
    /// machine-authored (StormGuard) with a dead-man's-switch TTL.
    pub fn set_site_bandwidth_override_with_expiry(
        &mut self,
        node_id: Option<String>,
        site_name: String,
        download_bandwidth_mbps: Option<f32>,
        upload_bandwidth_mbps: Option<f32>,
        expires_at_unix: Option<u64>,
    ) -> bool {
        let desired = NetworkAdjustment::AdjustSiteSpeed {
            node_id: node_id.clone(),
            site_name: site_name.clone(),
            download_bandwidth_mbps,
            upload_bandwidth_mbps,
            expires_at_unix,
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

    /// Rung-3 1a: migrate name-only `AdjustSiteSpeed` entries to their stable
    /// node ids in one pass, using `node_ids_by_name` (canonical node name →
    /// node id). Name-only entries whose name resolves are rewritten to carry
    /// the id; entries that already carry an id, or whose name is unresolved,
    /// are left untouched. A name-only entry whose resolved id is already
    /// claimed by an id-carrying entry is dropped as a duplicate (the
    /// id-carrying entry wins). Returns `true` when the layer changed and the
    /// caller must persist it.
    ///
    /// This function is pure: it has no side effects.
    pub fn migrate_site_bandwidth_overrides_to_node_ids(
        &mut self,
        node_ids_by_name: &std::collections::HashMap<String, String>,
    ) -> bool {
        let mut claimed_ids: std::collections::HashSet<String> = self
            .network_adjustments
            .iter()
            .filter_map(|adj| match adj {
                NetworkAdjustment::AdjustSiteSpeed {
                    node_id: Some(node_id),
                    ..
                } => {
                    let node_id = node_id.trim();
                    (!node_id.is_empty()).then(|| node_id.to_string())
                }
                _ => None,
            })
            .collect();

        let mut changed = false;
        let mut migrated = Vec::with_capacity(self.network_adjustments.len());
        for adj in self.network_adjustments.drain(..) {
            let NetworkAdjustment::AdjustSiteSpeed {
                node_id,
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
                expires_at_unix,
            } = adj
            else {
                migrated.push(adj);
                continue;
            };

            let carries_id = node_id
                .as_deref()
                .is_some_and(|node_id| !node_id.trim().is_empty());
            if carries_id {
                migrated.push(NetworkAdjustment::AdjustSiteSpeed {
                    node_id,
                    site_name,
                    download_bandwidth_mbps,
                    upload_bandwidth_mbps,
                    expires_at_unix,
                });
                continue;
            }

            let Some(node_id) = node_ids_by_name.get(&site_name) else {
                migrated.push(NetworkAdjustment::AdjustSiteSpeed {
                    node_id,
                    site_name,
                    download_bandwidth_mbps,
                    upload_bandwidth_mbps,
                    expires_at_unix,
                });
                continue;
            };
            if !claimed_ids.insert(node_id.clone()) {
                // An id-carrying entry already claims this node: the legacy
                // name-only duplicate is dropped, never double-applied.
                changed = true;
                continue;
            }
            migrated.push(NetworkAdjustment::AdjustSiteSpeed {
                node_id: Some(node_id.clone()),
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
                expires_at_unix,
            });
            changed = true;
        }
        self.network_adjustments = migrated;
        changed
    }

    /// K1b §4.6 pin 1: drop site-speed entries whose machine-authored TTL has
    /// lapsed (`expires_at_unix <= now_unix`). Operator entries (`None`) are
    /// immortal. Returns the dropped site names so the caller can WARN once per
    /// entry.
    pub fn remove_lapsed_site_speed_overrides(&mut self, now_unix: u64) -> Vec<String> {
        let mut removed = Vec::new();
        self.network_adjustments.retain(|adj| match adj {
            NetworkAdjustment::AdjustSiteSpeed {
                site_name,
                expires_at_unix: Some(expires),
                ..
            } if *expires <= now_unix => {
                removed.push(site_name.clone());
                false
            }
            _ => true,
        });
        removed
    }

    /// Returns the stored topology parent override for `node_id`, if present.
    ///
    /// This function is pure: it has no side effects.
    pub fn find_topology_parent_override(&self, node_id: &str) -> Option<&NetworkAdjustment> {
        self.network_adjustments.iter().find(|adj| {
            matches!(
                adj,
                NetworkAdjustment::TopologyParentOverride {
                    node_id: current_node_id,
                    ..
                } if current_node_id == node_id
            )
        })
    }

    /// Adds or replaces a topology parent override for `node_id`. Returns true if changed.
    pub fn set_topology_parent_override_return_changed(
        &mut self,
        node_id: String,
        node_name: String,
        mode: TopologyParentOverrideMode,
        parent_nodes: Vec<(String, String)>,
    ) -> bool {
        let normalized_node_id = node_id.trim().to_string();
        let normalized_node_name = node_name.trim().to_string();
        let mut parent_node_ids = Vec::new();
        let mut parent_node_names = Vec::new();
        let mut seen_parent_ids = std::collections::HashSet::new();

        for (parent_id, parent_name) in parent_nodes {
            let trimmed_id = parent_id.trim();
            let trimmed_name = parent_name.trim();
            if trimmed_id.is_empty() || !seen_parent_ids.insert(trimmed_id.to_string()) {
                continue;
            }
            parent_node_ids.push(trimmed_id.to_string());
            parent_node_names.push(trimmed_name.to_string());
        }

        let desired = NetworkAdjustment::TopologyParentOverride {
            node_id: normalized_node_id.clone(),
            node_name: normalized_node_name,
            mode,
            parent_node_ids,
            parent_node_names,
        };
        if self.find_topology_parent_override(&normalized_node_id) == Some(&desired) {
            return false;
        }

        self.network_adjustments.retain(|adj| {
            !matches!(
                adj,
                NetworkAdjustment::TopologyParentOverride {
                    node_id: current_node_id,
                    ..
                } if current_node_id == &normalized_node_id
            )
        });
        self.network_adjustments.push(desired);
        true
    }

    /// Removes any topology parent override for `node_id`. Returns number removed.
    pub fn remove_topology_parent_override_by_node_id_count(&mut self, node_id: &str) -> usize {
        let before = self.network_adjustments.len();
        self.network_adjustments.retain(|adj| {
            !matches!(
                adj,
                NetworkAdjustment::TopologyParentOverride {
                    node_id: current_node_id,
                    ..
                } if current_node_id == node_id
            )
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

    /// Adds or replaces a deprecated legacy UISP bandwidth override for `site_name`.
    ///
    /// Current UISP builds ignore these entries. Use `set_site_bandwidth_override`
    /// to create authoritative operator `AdjustSiteSpeed` overrides instead.
    pub fn set_uisp_bandwidth_override(&mut self, site_name: String, down: f32, up: f32) {
        let uisp = self.ensure_uisp_mut();
        uisp.bandwidth_overrides.insert(site_name, (down, up));
    }

    /// Removes a deprecated legacy UISP bandwidth override for `site_name`.
    /// Returns `true` when one existed.
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
    /// Returns the filesystem path for one overrides layer using a caller-supplied config snapshot.
    ///
    /// This function is pure: it has no side effects.
    pub fn path_for_layer_config(config: &lqos_config::Config, layer: OverrideLayer) -> PathBuf {
        overrides_path(config, layer)
    }

    /// Loads a single overrides layer. Phase D: READ-ONLY — never creates the
    /// operator file (the write arm is the explicit RMW/save path).
    ///
    /// Side effects: acquires the global overrides lock and reads the layer.
    pub fn load_layer(layer: OverrideLayer) -> Result<OverrideFile> {
        let lock = FileLock::new_for_operation(&format!("load {layer:?} overrides"))?;
        let config = lqos_config::load_config()?;
        let path = Self::path_for_layer_config(&config, layer);
        let overrides = if path.exists() {
            load_from_path(&path)?
        } else {
            OverrideFile::default()
        };
        drop(lock);
        Ok(overrides)
    }

    /// Saves a single overrides layer.
    ///
    /// Side effects: acquires the global overrides lock and writes the selected overrides file.
    pub fn save_layer(layer: OverrideLayer, overrides: &OverrideFile) -> Result<()> {
        let lock = FileLock::new_for_operation(&format!("save {layer:?} overrides"))?;
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
    pub fn load_effective(apply_stormguard: bool) -> Result<OverrideFile> {
        let lock = FileLock::new_for_operation("load effective overrides")?;
        let config = lqos_config::load_config()?;
        let merged = Self::load_effective_for_config_inner(&config, apply_stormguard)?;
        drop(lock);
        Ok(merged)
    }

    /// Loads the effective overrides view using a caller-supplied config snapshot.
    ///
    /// This function reads override files from the config's LibreQoS directory without
    /// reloading `/etc/lqos.conf`.
    ///
    /// Side effects: reads override files from disk and may create the operator
    /// overrides file if it does not exist.
    pub fn load_effective_for_config(
        config: &lqos_config::Config,
        apply_stormguard: bool,
    ) -> Result<OverrideFile> {
        Self::load_effective_for_config_inner(config, apply_stormguard)
    }

    /// Locked read-modify-write of the OPERATOR circuit-speed override
    /// for one circuit (E3 admin actions). `None` deletes the entry
    /// (resume); `Some` upserts it (rate edit / suspend floor).
    ///
    /// The whole sequence runs under ONE cross-process lock
    /// acquisition — the plain `load()` + mutate + `save()` sequence
    /// is a lost-update race and must not be used for writes.
    ///
    /// Side effects: reads and possibly rewrites the operator
    /// overrides file (temp+rename); creates the empty operator file
    /// when absent.
    pub fn set_circuit_speed_override(
        circuit_id: &str,
        speeds: Option<CircuitSpeedOverride>,
    ) -> Result<bool> {
        let circuit_id = circuit_id.trim();
        if circuit_id.is_empty() {
            anyhow::bail!("circuit id must not be empty");
        }
        let lock = FileLock::new_for_operation("set circuit speed override")?;
        let config = lqos_config::load_config()?;
        let path = overrides_path(&config, OverrideLayer::Operator);
        ensure_exists_default(&path)?;
        let mut overrides = load_from_path(&path)?;
        let changed = match speeds {
            None => overrides.remove_circuit_speed_by_circuit_count(circuit_id) > 0,
            Some(speeds) => overrides
                .set_circuit_speed_return_changed(circuit_id, speeds)
                .map_err(|message| anyhow::anyhow!("{message}"))?,
        };
        if changed {
            save_to_path(&path, &overrides)?;
        }
        drop(lock);
        Ok(changed)
    }

    /// Reads the OPERATOR circuit-speed override for one circuit.
    ///
    /// Side effects: reads the operator overrides file (creating the
    /// empty file when absent).
    pub fn circuit_speed_override(circuit_id: &str) -> Result<Option<CircuitSpeedOverride>> {
        let circuit_id = circuit_id.trim();
        if circuit_id.is_empty() {
            anyhow::bail!("circuit id must not be empty");
        }
        let lock = FileLock::new_for_operation("read circuit speed override")?;
        let config = lqos_config::load_config()?;
        let path = overrides_path(&config, OverrideLayer::Operator);
        if !path.exists() {
            drop(lock);
            return Ok(None);
        }
        let overrides = load_from_path(&path)?;
        drop(lock);
        Ok(overrides.circuit_speed_override(circuit_id))
    }

    fn load_effective_for_config_inner(
        config: &lqos_config::Config,
        apply_stormguard: bool,
    ) -> Result<OverrideFile> {
        Self::load_effective_for_config_inner_with(config, apply_stormguard, true)
    }

    /// Read-only variant of [`Self::load_effective_for_config`]: loads the
    /// effective override layers WITHOUT creating or migrating the operator
    /// overrides file. A read path must never write (the scheduler readiness
    /// verifier runs this per tick).
    pub fn load_effective_for_config_read_only(
        config: &lqos_config::Config,
        apply_stormguard: bool,
    ) -> Result<OverrideFile> {
        Self::load_effective_for_config_inner_with(config, apply_stormguard, false)
    }

    /// Phase D: read-only variant of [`Self::load_effective`] — loads config
    /// then reads the effective layers WITHOUT creating/migrating the operator
    /// file. The write arm is the explicit RMW (`set_circuit_speed_override`
    /// etc.); read callers use this.
    pub fn load_effective_read_only(apply_stormguard: bool) -> Result<OverrideFile> {
        let config = lqos_config::load_config()?;
        Self::load_effective_for_config_read_only(&config, apply_stormguard)
    }

    fn load_effective_for_config_inner_with(
        config: &lqos_config::Config,
        apply_stormguard: bool,
        create_if_missing: bool,
    ) -> Result<OverrideFile> {
        let operator_path = overrides_path(config, OverrideLayer::Operator);
        let operator = if create_if_missing {
            ensure_exists_default(&operator_path)?;
            load_from_path(&operator_path)?
        } else if operator_path.exists() {
            load_from_path(&operator_path)?
        } else {
            OverrideFile::default()
        };

        if !apply_stormguard {
            return Ok(operator);
        }

        let stormguard_path = overrides_path(config, OverrideLayer::Stormguard);
        let stormguard = if !stormguard_path.exists() {
            OverrideFile::default()
        } else {
            load_from_path(&stormguard_path)?
        };

        // T4 amendment 2 (t4_legacy_display_names_derivation.md §5.2):
        // name-only StormGuard site-speed entries go stale when the
        // tree's naming changes. The staleness WARN lives at the
        // CONSUMPTION point (lqos_stormguard's site rollout —
        // config.rs), not here: this crate is below the tree store in
        // the layering and cannot see live node names. Id-carrying
        // entries are unaffected (ids are stable across naming —
        // pin 4).

        Ok(merge_owned_sections(operator, stormguard))
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
    fn effective_merge_stormguard_wins_for_persistent_devices() {
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

        let merged = merge_owned_sections(operator, stormguard);
        let comments_by_id: std::collections::HashMap<&str, &str> = merged
            .persistent_devices
            .iter()
            .map(|d| (d.device_id.as_str(), d.comment.as_str()))
            .collect();

        assert_eq!(comments_by_id.get("dev1"), Some(&"stormguard"));
        assert_eq!(comments_by_id.get("dev2"), Some(&"operator"));
        assert_eq!(comments_by_id.get("dev3"), Some(&"stormguard"));
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

        let merged = merge_owned_sections(operator, stormguard);

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
    fn effective_merge_keeps_operator_site_speed_and_adds_stormguard() {
        let mut operator = OverrideFile::default();
        operator.add_network_adjustment(NetworkAdjustment::AdjustSiteSpeed {
            node_id: Some("node-site-1".to_string()),
            site_name: "Site1".to_string(),
            download_bandwidth_mbps: Some(100.0),
            upload_bandwidth_mbps: Some(50.0),
            expires_at_unix: None,
        });

        let mut stormguard = OverrideFile::default();
        stormguard.set_site_bandwidth_override(None, "Site1".to_string(), Some(80.0), Some(40.0));
        stormguard.set_site_bandwidth_override(None, "Site2".to_string(), Some(150.0), Some(75.0));

        let merged = merge_owned_sections(operator, stormguard);

        // Operator site speed should beat StormGuard for the same site;
        // StormGuard's non-conflicting site is carried through.
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
    fn merge_preserves_stormguard_site_speed_expiry() {
        // K1b §4.6 pin 1: the merge must carry the TTL, or a clamped site
        // becomes immortal the moment it passes through the effective store.
        let operator = OverrideFile::default();
        let mut stormguard = OverrideFile::default();
        stormguard.set_site_bandwidth_override_with_expiry(
            None,
            "Site1".to_string(),
            Some(80.0),
            Some(40.0),
            Some(1234),
        );
        let merged = merge_owned_sections(operator, stormguard);
        let expiry = merged.network_adjustments().iter().find_map(|adj| match adj {
            NetworkAdjustment::AdjustSiteSpeed {
                expires_at_unix, ..
            } => Some(*expires_at_unix),
            _ => None,
        });
        assert_eq!(expiry, Some(Some(1234)));
    }

    #[test]
    fn remove_lapsed_site_speed_overrides_drops_only_lapsed_machine_entries() {
        // K1b §4.6 pin 1: operator entries (None) are immortal; a machine
        // entry past its TTL is dropped and its name reported.
        let mut of = OverrideFile::default();
        of.set_site_bandwidth_override(None, "Operator".to_string(), Some(1.0), Some(1.0));
        of.set_site_bandwidth_override_with_expiry(
            None,
            "Future".to_string(),
            Some(2.0),
            Some(2.0),
            Some(10_000),
        );
        of.set_site_bandwidth_override_with_expiry(
            None,
            "Lapsed".to_string(),
            Some(3.0),
            Some(3.0),
            Some(1_000),
        );

        let removed = of.remove_lapsed_site_speed_overrides(5_000);
        assert_eq!(removed, vec!["Lapsed".to_string()]);

        let names: Vec<&str> = of
            .network_adjustments()
            .iter()
            .filter_map(|adj| match adj {
                NetworkAdjustment::AdjustSiteSpeed { site_name, .. } => Some(site_name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["Operator", "Future"]);
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
    fn name_only_site_speed_entries_migrate_to_node_ids() {
        // 1a: a pre-migration file stored the clamp by site name; one load pass
        // rewrites it to the stable node id, preserving rate + TTL.
        let mut of = OverrideFile::default();
        of.set_site_bandwidth_override_with_expiry(
            None,
            "Tower-1".to_string(),
            Some(800.0),
            Some(400.0),
            Some(4_000),
        );
        let mut ids_by_name = std::collections::HashMap::new();
        ids_by_name.insert("Tower-1".to_string(), "uisp:site:tower-1".to_string());

        assert!(of.migrate_site_bandwidth_overrides_to_node_ids(&ids_by_name));
        let found = of.find_site_bandwidth_override(Some("uisp:site:tower-1"), "Tower-1");
        assert!(matches!(
            found,
            Some(NetworkAdjustment::AdjustSiteSpeed {
                node_id: Some(node_id),
                download_bandwidth_mbps: Some(download),
                upload_bandwidth_mbps: Some(upload),
                expires_at_unix: Some(4_000),
                ..
            }) if node_id == "uisp:site:tower-1" && *download == 800.0 && *upload == 400.0
        ));

        // Idempotent: a second pass finds nothing to rewrite.
        assert!(!of.migrate_site_bandwidth_overrides_to_node_ids(&ids_by_name));
    }

    #[test]
    fn migration_keeps_unresolved_name_only_entries_untouched() {
        let mut of = OverrideFile::default();
        of.set_site_bandwidth_override(None, "Stale".to_string(), Some(10.0), None);
        let ids_by_name = std::collections::HashMap::new();

        assert!(!of.migrate_site_bandwidth_overrides_to_node_ids(&ids_by_name));
        let found = of.find_site_bandwidth_override(None, "Stale");
        assert!(matches!(
            found,
            Some(NetworkAdjustment::AdjustSiteSpeed {
                node_id: None,
                download_bandwidth_mbps: Some(10.0),
                ..
            })
        ));
    }

    #[test]
    fn migration_drops_name_only_duplicate_when_id_entry_exists() {
        // An id-carrying entry (new write path) plus a legacy name-only twin:
        // the migration must not leave both, which would double-apply.
        let mut of = OverrideFile::default();
        of.add_network_adjustment(NetworkAdjustment::AdjustSiteSpeed {
            node_id: None,
            site_name: "Tower-1".to_string(),
            download_bandwidth_mbps: Some(800.0),
            upload_bandwidth_mbps: None,
            expires_at_unix: Some(4_000),
        });
        of.add_network_adjustment(NetworkAdjustment::AdjustSiteSpeed {
            node_id: Some("uisp:site:tower-1".to_string()),
            site_name: "Tower-1".to_string(),
            download_bandwidth_mbps: Some(900.0),
            upload_bandwidth_mbps: None,
            expires_at_unix: Some(4_000),
        });
        let mut ids_by_name = std::collections::HashMap::new();
        ids_by_name.insert("Tower-1".to_string(), "uisp:site:tower-1".to_string());

        assert!(of.migrate_site_bandwidth_overrides_to_node_ids(&ids_by_name));
        let entries: Vec<&NetworkAdjustment> = of
            .network_adjustments()
            .iter()
            .filter(|adj| matches!(adj, NetworkAdjustment::AdjustSiteSpeed { .. }))
            .collect();
        assert_eq!(entries.len(), 1, "the duplicate must be dropped");
        let found = of.find_site_bandwidth_override(Some("uisp:site:tower-1"), "Tower-1");
        assert!(matches!(
            found,
            Some(NetworkAdjustment::AdjustSiteSpeed {
                download_bandwidth_mbps: Some(900.0),
                ..
            })
        ));
    }

    #[test]
    fn topology_parent_override_set_replace_and_remove_are_idempotent() {
        let mut of = OverrideFile::default();
        assert!(of.set_topology_parent_override_return_changed(
            "uisp:site:site-t2".to_string(),
            "T2".to_string(),
            TopologyParentOverrideMode::PreferredOrder,
            vec![
                ("uisp:site:site-t1".to_string(), "T1".to_string()),
                ("uisp:site:site-t3".to_string(), "T3".to_string()),
            ],
        ));
        assert!(!of.set_topology_parent_override_return_changed(
            "uisp:site:site-t2".to_string(),
            "T2".to_string(),
            TopologyParentOverrideMode::PreferredOrder,
            vec![
                ("uisp:site:site-t1".to_string(), "T1".to_string()),
                ("uisp:site:site-t3".to_string(), "T3".to_string()),
            ],
        ));

        let found = of.find_topology_parent_override("uisp:site:site-t2");
        assert!(matches!(
            found,
            Some(NetworkAdjustment::TopologyParentOverride {
                mode: TopologyParentOverrideMode::PreferredOrder,
                parent_node_ids,
                ..
            }) if parent_node_ids == &vec![
                "uisp:site:site-t1".to_string(),
                "uisp:site:site-t3".to_string(),
            ]
        ));

        assert!(of.set_topology_parent_override_return_changed(
            "uisp:site:site-t2".to_string(),
            "T2".to_string(),
            TopologyParentOverrideMode::Pinned,
            vec![("uisp:site:site-t1".to_string(), "T1".to_string())],
        ));
        let found = of.find_topology_parent_override("uisp:site:site-t2");
        assert!(matches!(
            found,
            Some(NetworkAdjustment::TopologyParentOverride {
                mode: TopologyParentOverrideMode::Pinned,
                parent_node_ids,
                ..
            }) if parent_node_ids == &vec!["uisp:site:site-t1".to_string()]
        ));

        assert_eq!(
            of.remove_topology_parent_override_by_node_id_count("uisp:site:site-t2"),
            1
        );
        assert_eq!(
            of.remove_topology_parent_override_by_node_id_count("uisp:site:site-t2"),
            0
        );
    }

    #[test]
    fn effective_merge_keeps_operator_topology_parent_override() {
        let mut operator = OverrideFile::default();
        operator.set_topology_parent_override_return_changed(
            "uisp:site:site-t2".to_string(),
            "T2".to_string(),
            TopologyParentOverrideMode::Pinned,
            vec![("uisp:site:site-t1".to_string(), "T1".to_string())],
        );

        let stormguard = OverrideFile::default();

        let merged = merge_owned_sections(operator, stormguard);
        assert!(matches!(
            merged.find_topology_parent_override("uisp:site:site-t2"),
            Some(NetworkAdjustment::TopologyParentOverride {
                mode: TopologyParentOverrideMode::Pinned,
                parent_node_ids,
                ..
            }) if parent_node_ids == &vec!["uisp:site:site-t1".to_string()]
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

#[cfg(test)]
mod circuit_speed_tests {
    use super::*;

    #[test]
    fn set_upserts_by_circuit_and_detects_change() {
        let mut file = OverrideFile::default();
        let speeds = CircuitSpeedOverride {
            min_download_bandwidth: Some(10.0),
            max_download_bandwidth: Some(100.0),
            min_upload_bandwidth: Some(5.0),
            max_upload_bandwidth: Some(50.0),
        };
        assert!(file.set_circuit_speed_return_changed("c1", speeds).unwrap());
        // Same content: no change.
        assert!(!file.set_circuit_speed_return_changed("c1", speeds).unwrap());
        // Different content: replaced, never duplicated.
        let edited = CircuitSpeedOverride {
            max_download_bandwidth: Some(200.0),
            ..speeds
        };
        assert!(file.set_circuit_speed_return_changed("c1", edited).unwrap());
        assert_eq!(file.circuit_adjustments().len(), 1);
        let stored = file.circuit_speed_override("c1").expect("present");
        assert_eq!(stored.max_download_bandwidth, Some(200.0));
    }

    #[test]
    fn remove_clears_the_entry_once() {
        let mut file = OverrideFile::default();
        file.set_circuit_speed_return_changed("c1", CircuitSpeedOverride::suspend_floor())
            .unwrap();
        assert_eq!(file.remove_circuit_speed_by_circuit_count("c1"), 1);
        assert_eq!(file.remove_circuit_speed_by_circuit_count("c1"), 0);
        assert!(file.circuit_speed_override("c1").is_none());
    }

    #[test]
    fn zero_nan_and_negative_rates_are_rejected() {
        let mut file = OverrideFile::default();
        let base = CircuitSpeedOverride {
            min_download_bandwidth: Some(10.0),
            max_download_bandwidth: Some(100.0),
            min_upload_bandwidth: Some(5.0),
            max_upload_bandwidth: Some(50.0),
        };
        let zeroed = CircuitSpeedOverride {
            max_upload_bandwidth: Some(0.0),
            ..base
        };
        assert!(file.set_circuit_speed_return_changed("c1", zeroed).is_err());
        let nan = CircuitSpeedOverride {
            min_download_bandwidth: Some(f32::NAN),
            ..base
        };
        assert!(file.set_circuit_speed_return_changed("c1", nan).is_err());
        let negative = CircuitSpeedOverride {
            max_download_bandwidth: Some(-1.0),
            ..base
        };
        assert!(file.set_circuit_speed_return_changed("c1", negative).is_err());
        // Rejections never mutate.
        assert!(file.circuit_adjustments().is_empty());
        // The suspend floor itself is legal (0.01 > 0).
        assert!(file
            .set_circuit_speed_return_changed("c1", CircuitSpeedOverride::suspend_floor())
            .unwrap());
    }

    #[test]
    fn suspend_floor_recognizes_only_the_full_floor() {
        let floor = CircuitSpeedOverride::suspend_floor();
        assert!(floor.is_suspend_floor());
        let partial = CircuitSpeedOverride {
            max_upload_bandwidth: Some(0.02),
            ..floor
        };
        assert!(!partial.is_suspend_floor());
        // A partial override (only some directions set) is not a suspend.
        let partial_override = CircuitSpeedOverride {
            min_download_bandwidth: Some(SUSPEND_FLOOR_MBPS),
            max_download_bandwidth: Some(SUSPEND_FLOOR_MBPS),
            min_upload_bandwidth: None,
            max_upload_bandwidth: None,
        };
        assert!(!partial_override.is_suspend_floor());
    }

    #[test]
    fn other_circuits_adjustments_are_untouched() {
        let mut file = OverrideFile::default();
        file.set_circuit_speed_return_changed("c1", CircuitSpeedOverride::suspend_floor())
            .unwrap();
        file.set_circuit_speed_return_changed("c2", CircuitSpeedOverride::suspend_floor())
            .unwrap();
        assert_eq!(file.remove_circuit_speed_by_circuit_count("c1"), 1);
        assert!(file.circuit_speed_override("c2").is_some());
        assert_eq!(file.circuit_adjustments().len(), 1);
    }

    fn temp_overrides_config(tag: &str) -> lqos_config::Config {
        let dir = std::env::temp_dir().join(format!(
            "lqos-overrides-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        lqos_config::Config {
            lqos_directory: dir.to_string_lossy().to_string(),
            state_directory: None,
            ..lqos_config::Config::default()
        }
    }

    fn topology_override(child: &str) -> crate::topology_overrides::TopologyAttachmentOverride {
        crate::topology_overrides::TopologyAttachmentOverride {
            child_node_id: child.to_string(),
            child_node_name: format!("{child}-name"),
            parent_node_id: "parent-1".to_string(),
            parent_node_name: "Parent".to_string(),
            mode: crate::topology_overrides::TopologyAttachmentMode::Auto,
            attachment_preference_ids: Vec::new(),
            attachment_preference_names: Vec::new(),
        }
    }

    /// Rung 1.5 §1.2 migration gate: a legacy file is imported ONCE, backed up,
    /// and never dual-read or re-created.
    #[test]
    fn legacy_topology_overrides_migrate_once_and_are_never_dual_read() {
        let config = temp_overrides_config("topology-migration");
        let legacy_path =
            crate::topology_overrides::TopologyOverridesFile::path_for_config(&config);
        let legacy = crate::topology_overrides::TopologyOverridesFile {
            schema_version: 3,
            overrides: vec![topology_override("child-1")],
            ..Default::default()
        };
        std::fs::write(&legacy_path, serde_json::to_string_pretty(&legacy).unwrap())
            .expect("legacy file");

        // Phase D: the explicit repair entry runs the migration; the loader is
        // read-only.
        crate::topology_overrides::TopologyOverridesFile::repair(&config)
            .expect("repair should migrate");
        let loaded = crate::topology_overrides::TopologyOverridesFile::load_for_config(&config)
            .expect("load");
        assert_eq!(loaded.overrides.len(), 1);
        assert!(!legacy_path.exists(), "legacy file must be renamed away");
        assert!(
            legacy_path
                .with_file_name("topology_overrides.json.migrated")
                .exists()
        );

        let again = crate::topology_overrides::TopologyOverridesFile::load_for_config(&config)
            .expect("second load");
        assert_eq!(again.overrides.len(), 1);
        assert!(
            !legacy_path.exists(),
            "legacy file must not be re-created after migration"
        );
    }

    #[test]
    fn absent_legacy_topology_file_is_a_noop_and_not_recreated() {
        let config = temp_overrides_config("topology-absent");
        let legacy_path =
            crate::topology_overrides::TopologyOverridesFile::path_for_config(&config);
        let loaded = crate::topology_overrides::TopologyOverridesFile::load_for_config(&config)
            .expect("load");
        assert!(loaded.is_empty());
        assert!(!legacy_path.exists(), "no legacy file may be created");
    }

    /// Rung 1.5 §1.2 #12-MED gate: a topology section save is a section-merge
    /// RMW — sibling sections are preserved byte-for-byte.
    #[test]
    fn topology_section_save_preserves_sibling_sections() {
        let config = temp_overrides_config("topology-siblings");
        let mut combined = OverrideFile::default();
        combined.add_persistent_shaped_device_return_changed(ShapedDevice {
            device_id: "dev-keep".to_string(),
            comment: "keep".to_string(),
            ..ShapedDevice::default()
        });
        combined
            .set_circuit_speed_return_changed(
                "circuit-keep",
                CircuitSpeedOverride {
                    min_download_bandwidth: Some(10.0),
                    max_download_bandwidth: Some(100.0),
                    min_upload_bandwidth: Some(10.0),
                    max_upload_bandwidth: Some(100.0),
                },
            )
            .expect("circuit speed");
        combined.set_topology_parent_override_return_changed(
            "node-keep".to_string(),
            "Node Keep".to_string(),
            TopologyParentOverrideMode::Pinned,
            vec![("p1".to_string(), "P1".to_string())],
        );
        let path = OverrideFile::operator_path_for_config(&config);
        combined.save_to_explicit_path(&path).expect("seed combined");
        let siblings_before = serde_json::to_value(
            OverrideFile::load_from_explicit_path(&path).expect("load seed"),
        )
        .expect("serialize");

        crate::topology_overrides::TopologyOverridesFile {
            schema_version: 3,
            overrides: vec![topology_override("child-new")],
            ..Default::default()
        }
        .save_for_config(&config)
        .expect("section save");

        let after =
            OverrideFile::load_from_explicit_path(&path).expect("load after section save");
        let siblings_after = serde_json::to_value(&after).expect("serialize");
        for key in [
            "persistent_devices",
            "circuit_adjustments",
            "network_adjustments",
        ] {
            assert_eq!(
                siblings_before[key], siblings_after[key],
                "sibling section {key} must be preserved byte-equal"
            );
        }
        assert_eq!(after.topology().overrides.len(), 1);
        assert_eq!(after.topology().overrides[0].child_node_id, "child-new");
    }
}

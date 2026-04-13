use lqos_config::{
    CircuitAnchorsFile, CircuitEthernetMetadata, Config, ConfigShapedDevices, ShapedDevice,
    TopologyCanonicalStateFile, TopologyEditorStateFile, TopologyParentCandidatesFile,
    topology_compiled_shaping_path, topology_import_path, topology_ingress_identity_from_tokens,
};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn default_topology_import_schema_version() -> u32 {
    1
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let raw = serde_json::to_string_pretty(value)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct ImportedTopologySnapshot {
    source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generated_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ingress_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_canonical: Option<TopologyCanonicalStateFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_editor: Option<TopologyEditorStateFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_candidates: Option<TopologyParentCandidatesFile>,
    #[serde(default)]
    compatibility_network_json: serde_json::Value,
    #[serde(default)]
    shaped_devices: Vec<ShapedDevice>,
    #[serde(default)]
    circuit_anchors: CircuitAnchorsFile,
    #[serde(default)]
    ethernet_advisories: Vec<CircuitEthernetMetadata>,
}

impl ImportedTopologySnapshot {
    fn from_bundle(imported: &ImportedTopologyBundle) -> Self {
        Self {
            source: imported.source.clone(),
            generated_unix: imported.generated_unix,
            ingress_identity: imported.ingress_identity(),
            native_canonical: imported.native_canonical.clone(),
            native_editor: imported.native_editor.clone(),
            parent_candidates: imported.parent_candidates.clone(),
            compatibility_network_json: imported.compatibility_network_json.clone(),
            shaped_devices: imported.shaped_devices.devices.clone(),
            circuit_anchors: imported.circuit_anchors.clone(),
            ethernet_advisories: imported.ethernet_advisories.clone(),
        }
    }

    fn into_bundle(self) -> ImportedTopologyBundle {
        let mut shaped_devices = ConfigShapedDevices::default();
        shaped_devices.replace_with_new_data(self.shaped_devices);
        ImportedTopologyBundle {
            source: self.source,
            generated_unix: self.generated_unix,
            ingress_identity: self.ingress_identity,
            native_canonical: self.native_canonical,
            native_editor: self.native_editor,
            parent_candidates: self.parent_candidates,
            compatibility_network_json: self.compatibility_network_json,
            shaped_devices,
            circuit_anchors: self.circuit_anchors,
            ethernet_advisories: self.ethernet_advisories,
        }
    }
}

/// Serialized compiler-selected shaping rows and circuit anchors for one integration mode.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TopologyCompiledShapingFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_topology_import_schema_version")]
    pub schema_version: u32,
    /// Human-readable compiler source such as `uisp/ap_only`.
    #[serde(default)]
    pub source: String,
    /// Selected topology compile mode such as `ap_only`.
    #[serde(default)]
    pub compile_mode: String,
    /// Generation timestamp when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_unix: Option<u64>,
    /// Stable identity of imported topology facts plus selected compile mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_identity: Option<String>,
    #[serde(default)]
    shaped_devices: Vec<ShapedDevice>,
    #[serde(default)]
    circuit_anchors: CircuitAnchorsFile,
}

impl TopologyCompiledShapingFile {
    /// Builds a serialized compiled-shaping artifact from one compiled bundle.
    ///
    /// This function is pure: it has no side effects.
    pub fn from_compiled(
        compiled: &CompiledTopologyBundle,
        compile_mode: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: default_topology_import_schema_version(),
            source: compiled.source.clone(),
            compile_mode: compile_mode.into(),
            generated_unix: compiled.generated_unix,
            ingress_identity: compiled.ingress_identity.clone(),
            shaped_devices: compiled.shaped_devices.devices.clone(),
            circuit_anchors: compiled.circuit_anchors.clone(),
        }
    }

    /// Loads the compiled-shaping artifact when it exists.
    ///
    /// Side effects: reads `topology_compiled_shaping.json` from `config.lqos_directory`.
    pub fn load(config: &Config) -> anyhow::Result<Option<Self>> {
        let path = topology_compiled_shaping_path(config);
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    /// Saves the serialized compiled-shaping artifact atomically.
    ///
    /// Side effects: writes `topology_compiled_shaping.json` into `config.lqos_directory`.
    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        atomic_write_json(
            &config.shaping_state_file_path(lqos_config::TOPOLOGY_COMPILED_SHAPING_FILENAME),
            self,
        )
    }

    /// Returns the shaping-device rows and circuit anchors for integration ingress.
    ///
    /// This function is pure: it has no side effects.
    pub fn shaping_artifacts(self) -> (ConfigShapedDevices, Vec<lqos_config::CircuitAnchor>) {
        let mut shaped_devices = ConfigShapedDevices::default();
        shaped_devices.replace_with_new_data(self.shaped_devices);
        (shaped_devices, self.circuit_anchors.anchors)
    }
}

/// Serialized integration import artifact carrying imported facts before mode selection.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TopologyImportFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_topology_import_schema_version")]
    pub schema_version: u32,
    /// Human-readable importer source such as `uisp/full2`.
    #[serde(default)]
    pub source: String,
    /// Selected topology compile mode such as `ap_only`.
    #[serde(default)]
    pub compile_mode: String,
    /// Generation timestamp when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_unix: Option<u64>,
    /// Stable identity of imported topology facts plus selected compile mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_identity: Option<String>,
    /// Imported topology facts before projection into a specific runtime shape.
    imported: ImportedTopologySnapshot,
}

/// Imported topology facts supplied by one integration adapter before mode selection.
pub struct ImportedTopologyBundle {
    /// Human-readable source such as `uisp/full2` or `python/integration_common`.
    pub source: String,
    /// Generation timestamp when known.
    pub generated_unix: Option<u64>,
    /// Stable identity of the imported topology facts before mode projection.
    pub ingress_identity: Option<String>,
    /// Rich canonical topology state, when the importer can provide it directly.
    pub native_canonical: Option<TopologyCanonicalStateFile>,
    /// Rich editor/runtime topology state, when the importer can provide it directly.
    pub native_editor: Option<TopologyEditorStateFile>,
    /// Legacy immediate-parent candidate snapshot, when available.
    pub parent_candidates: Option<TopologyParentCandidatesFile>,
    /// Compatibility topology tree used by legacy readers.
    pub compatibility_network_json: serde_json::Value,
    /// Shaping-device rows associated with the imported topology.
    pub shaped_devices: ConfigShapedDevices,
    /// Circuit anchors associated with the imported topology.
    pub circuit_anchors: CircuitAnchorsFile,
    /// Ethernet advisories associated with the imported topology.
    pub ethernet_advisories: Vec<CircuitEthernetMetadata>,
}

/// Fully compiled topology outputs for one selected mode.
pub struct CompiledTopologyBundle {
    /// Human-readable compiled source, such as `uisp/ap_site`.
    pub source: String,
    /// Generation timestamp when known.
    pub generated_unix: Option<u64>,
    /// Stable identity of the imported topology facts plus selected compile mode.
    pub ingress_identity: Option<String>,
    /// Canonical topology state consumed by runtime compilation.
    pub canonical: TopologyCanonicalStateFile,
    /// Editor topology state presented to Topology Manager.
    pub editor: TopologyEditorStateFile,
    /// Legacy immediate-parent candidate snapshot kept for compatibility.
    pub parent_candidates: TopologyParentCandidatesFile,
    /// Compatibility topology tree emitted to `network.json`.
    pub compatibility_network_json: serde_json::Value,
    /// Shaping-device rows emitted to `ShapedDevices.csv`.
    pub shaped_devices: ConfigShapedDevices,
    /// Circuit anchors emitted to `circuit_anchors.json`.
    pub circuit_anchors: CircuitAnchorsFile,
    /// Ethernet advisories emitted to runtime metadata.
    pub ethernet_advisories: Vec<CircuitEthernetMetadata>,
}

impl ImportedTopologyBundle {
    /// Returns the stable identity for this imported topology bundle.
    ///
    /// This function is pure: it has no side effects.
    pub fn ingress_identity(&self) -> Option<String> {
        if let Some(identity) = self
            .ingress_identity
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(identity.to_string());
        }

        let mut tokens = vec![format!("source:{}", self.source.trim())];
        if let Some(editor) = &self.native_editor {
            tokens.extend(
                editor
                    .nodes
                    .iter()
                    .map(|node| format!("editor-node:{}", node.node_id.trim())),
            );
        } else if let Some(canonical) = &self.native_canonical {
            tokens.extend(
                canonical
                    .nodes
                    .iter()
                    .map(|node| format!("canonical-node:{}", node.node_id.trim())),
            );
        }
        if let Some(parent_candidates) = &self.parent_candidates {
            tokens.extend(
                parent_candidates
                    .nodes
                    .iter()
                    .map(|node| format!("parent-node:{}", node.node_id.trim())),
            );
        }
        tokens.extend(self.shaped_devices.devices.iter().flat_map(|device| {
            [
                format!("circuit:{}", device.circuit_id.trim()),
                format!("device:{}", device.device_id.trim()),
            ]
        }));
        tokens.extend(self.circuit_anchors.anchors.iter().flat_map(|anchor| {
            [
                format!("anchor-circuit:{}", anchor.circuit_id.trim()),
                format!("anchor-node:{}", anchor.anchor_node_id.trim()),
            ]
        }));
        topology_ingress_identity_from_tokens(tokens)
    }

    /// Loads a legacy import bundle from the current runtime artifacts on disk.
    ///
    /// Side effects: reads `network.json`, `ShapedDevices.csv`, `circuit_anchors.json`,
    /// `topology_canonical_state.json`, `topology_editor_state.json`, and
    /// `topology_parent_candidates.json` from `config.lqos_directory` when they exist.
    pub fn from_legacy_artifacts(
        config: &Config,
        source: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let network_path = Path::new(&config.lqos_directory).join("network.json");
        let compatibility_network_json = if network_path.exists() {
            let raw = std::fs::read_to_string(&network_path)?;
            serde_json::from_str(&raw)?
        } else {
            serde_json::Value::Object(serde_json::Map::new())
        };
        let native_canonical = {
            let loaded = TopologyCanonicalStateFile::load(config)?;
            (!loaded.nodes.is_empty()).then_some(loaded)
        };
        let native_editor = {
            let loaded = TopologyEditorStateFile::load(config)?;
            (!loaded.nodes.is_empty()).then_some(loaded)
        };
        let parent_candidates = {
            let loaded = TopologyParentCandidatesFile::load(config)?;
            (!loaded.nodes.is_empty()).then_some(loaded)
        };
        Ok(Self {
            source: source.into(),
            generated_unix: native_canonical
                .as_ref()
                .and_then(|state| state.generated_unix)
                .or(native_editor
                    .as_ref()
                    .and_then(|state| state.generated_unix)),
            ingress_identity: native_canonical
                .as_ref()
                .and_then(|state| state.ingress_identity.clone())
                .or(native_editor
                    .as_ref()
                    .and_then(|state| state.ingress_identity.clone()))
                .or(parent_candidates
                    .as_ref()
                    .and_then(|state| state.ingress_identity.clone())),
            native_canonical,
            native_editor,
            parent_candidates,
            compatibility_network_json,
            shaped_devices: ConfigShapedDevices::load_for_config(config)?,
            circuit_anchors: CircuitAnchorsFile::load(config)?,
            ethernet_advisories: Vec::new(),
        })
    }
}

impl TopologyImportFile {
    /// Builds a serialized import artifact from one imported bundle and selected compile mode.
    ///
    /// This function is pure: it has no side effects.
    pub fn from_imported_bundle(
        imported: &ImportedTopologyBundle,
        compile_mode: impl Into<String>,
    ) -> Self {
        let compile_mode = compile_mode.into();
        let imported_identity = imported.ingress_identity();
        Self {
            schema_version: default_topology_import_schema_version(),
            source: imported.source.clone(),
            compile_mode: compile_mode.clone(),
            generated_unix: imported.generated_unix,
            ingress_identity: imported_identity.map(|identity| {
                topology_ingress_identity_from_tokens([
                    format!("import:{}", identity.trim()),
                    format!("mode:{}", compile_mode.trim()),
                ])
                .unwrap_or(identity)
            }),
            imported: ImportedTopologySnapshot::from_bundle(imported),
        }
    }

    /// Loads the serialized topology import artifact when it exists.
    ///
    /// Side effects: reads `topology_import.json` from `config.lqos_directory`.
    pub fn load(config: &Config) -> anyhow::Result<Option<Self>> {
        let path = topology_import_path(config);
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    /// Reconstructs the in-memory imported topology bundle from the serialized snapshot.
    ///
    /// This function is pure: it has no side effects.
    pub fn into_imported_bundle(self) -> ImportedTopologyBundle {
        self.imported.into_bundle()
    }

    /// Saves the serialized topology import artifact atomically.
    ///
    /// Side effects: writes `topology_import.json` into `config.lqos_directory`.
    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        atomic_write_json(
            &config.topology_state_file_path(lqos_config::TOPOLOGY_IMPORT_FILENAME),
            self,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn imported_bundle_with_identity(identity: &str) -> ImportedTopologyBundle {
        ImportedTopologyBundle {
            source: "test/import".to_string(),
            generated_unix: Some(123),
            ingress_identity: Some(identity.to_string()),
            native_canonical: None,
            native_editor: None,
            parent_candidates: None,
            compatibility_network_json: json!({}),
            shaped_devices: ConfigShapedDevices::default(),
            circuit_anchors: CircuitAnchorsFile::default(),
            ethernet_advisories: Vec::new(),
        }
    }

    #[test]
    fn topology_import_identity_matches_compiled_identity_contract() {
        let imported = imported_bundle_with_identity("import-base");
        let topology_import = TopologyImportFile::from_imported_bundle(&imported, "full");
        let expected = topology_ingress_identity_from_tokens([
            "import:import-base".to_string(),
            "mode:full".to_string(),
        ])
        .expect("expected ingress identity");
        assert_eq!(
            topology_import.ingress_identity.as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn compiled_shaping_file_round_trips_compiled_artifacts() {
        let file = TopologyCompiledShapingFile {
            schema_version: 1,
            source: "test/full".to_string(),
            compile_mode: "full".to_string(),
            generated_unix: Some(123),
            ingress_identity: Some("identity".to_string()),
            shaped_devices: vec![ShapedDevice {
                circuit_id: "circuit-1".to_string(),
                circuit_name: "Circuit 1".to_string(),
                device_id: "device-1".to_string(),
                device_name: "Device 1".to_string(),
                parent_node: "Tower 1".to_string(),
                parent_node_id: Some("tower-1".to_string()),
                anchor_node_id: None,
                mac: String::new(),
                ipv4: Vec::new(),
                ipv6: Vec::new(),
                download_min_mbps: 10.0,
                upload_min_mbps: 10.0,
                download_max_mbps: 100.0,
                upload_max_mbps: 100.0,
                comment: String::new(),
                sqm_override: None,
                circuit_hash: 0,
                device_hash: 0,
                parent_hash: 0,
            }],
            circuit_anchors: CircuitAnchorsFile::default(),
        };
        let (shaped_devices, anchors) = file.shaping_artifacts();
        assert_eq!(shaped_devices.devices.len(), 1);
        assert!(anchors.is_empty());
    }
}

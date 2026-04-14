//! Shared topology runtime domain logic for attachment health and effective topology.

#![warn(missing_docs)]

mod runtime;

use anyhow::{Context, Result};
use lqos_config::{
    CircuitAnchor, CircuitAnchorsFile, Config, ConfigShapedDevices, TOPOLOGY_ATTACHMENT_AUTO_ID,
    TopLevelPlannerItem, TopLevelPlannerMode, TopLevelPlannerParams, TopologyAllowedParent,
    TopologyAttachmentHealthStateFile, TopologyAttachmentHealthStatus, TopologyAttachmentOption,
    TopologyAttachmentRateSource, TopologyAttachmentRole, TopologyCanonicalIngressKind,
    TopologyCanonicalNode, TopologyCanonicalStateFile, TopologyEditorNode, TopologyEditorStateFile,
    TopologyEffectiveAttachmentState, TopologyEffectiveNodeState, TopologyEffectiveStateFile,
    TopologyQueueVisibilityPolicy, TopologyRuntimeStatusFile, TopologyShapingCircuitInput,
    TopologyShapingDeviceInput, TopologyShapingInputsFile, TopologyShapingResolutionSource,
    circuit_anchors_path, compute_effective_network_generation, detect_shaping_cpus,
    plan_top_level_assignments, topology_effective_network_path, topology_effective_state_path,
    topology_runtime_status_path, topology_shaping_inputs_path,
};
use lqos_overrides::{
    CircuitAdjustment, OverrideStore, TopologyAttachmentMode, TopologyOverridesFile,
};
use lqos_topology_compile::{TopologyCompiledShapingFile, TopologyImportFile};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_FILENAME: &str = "topology_effective_publish.lock";
type EffectiveQueueAliasMap = HashMap<String, (String, String)>;

pub use runtime::start_topology;

/// One unique probe pair emitted from topology state plus operator intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentProbeSpec {
    /// Stable pair identifier.
    pub pair_id: String,
    /// Stable attachment identifier.
    pub attachment_id: String,
    /// Display name of the attachment.
    pub attachment_name: String,
    /// Stable node identifier of the child being shaped.
    pub node_id: String,
    /// Display name of the child being shaped.
    pub node_name: String,
    /// Stable parent node identifier for this attachment group.
    pub parent_node_id: String,
    /// Display name of the parent node for this attachment group.
    pub parent_node_name: String,
    /// Local endpoint IP.
    pub local_ip: String,
    /// Remote endpoint IP.
    pub remote_ip: String,
    /// Whether probes are enabled for this pair.
    pub enabled: bool,
}

fn now_unix() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn atomic_write_json_value(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(value)?;
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

fn read_json_value(path: &Path) -> Option<Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn effective_state_payload_equals(
    left: &TopologyEffectiveStateFile,
    right: &TopologyEffectiveStateFile,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.generated_unix = None;
    right.generated_unix = None;
    left == right
}

fn optional_non_empty(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn optional_non_empty_owned(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| optional_non_empty(&value))
}

fn collect_exported_effective_nodes(value: &Value, by_id: &mut HashMap<String, String>) {
    let Some(nodes) = value.as_object() else {
        return;
    };
    for (key, node) in nodes {
        let Some(map) = node.as_object() else {
            continue;
        };
        let is_virtual = map.get("virtual").and_then(Value::as_bool).unwrap_or(false);
        let node_id = map
            .get("id")
            .and_then(Value::as_str)
            .and_then(optional_non_empty);
        let node_name = map
            .get("name")
            .and_then(Value::as_str)
            .and_then(optional_non_empty)
            .or_else(|| optional_non_empty(key));
        if !is_virtual && let (Some(node_id), Some(node_name)) = (node_id, node_name) {
            by_id.insert(node_id, node_name);
        }
        if let Some(children) = map.get("children") {
            collect_exported_effective_nodes(children, by_id);
        }
    }
}

fn collect_exported_effective_aliases(
    value: &Value,
    aliases: &mut HashMap<String, (String, String)>,
) {
    let Some(nodes) = value.as_object() else {
        return;
    };
    for (key, node) in nodes {
        let Some(map) = node.as_object() else {
            continue;
        };
        let is_virtual = map.get("virtual").and_then(Value::as_bool).unwrap_or(false);
        let node_id = map
            .get("id")
            .and_then(Value::as_str)
            .and_then(optional_non_empty);
        let node_name = map
            .get("name")
            .and_then(Value::as_str)
            .and_then(optional_non_empty)
            .or_else(|| optional_non_empty(key));
        let active_attachment_name = map
            .get("active_attachment_name")
            .and_then(Value::as_str)
            .and_then(optional_non_empty);
        if !is_virtual
            && let (Some(alias), Some(node_id), Some(node_name)) =
                (active_attachment_name, node_id, node_name)
        {
            aliases.entry(alias).or_insert((node_id, node_name));
        }
        if let Some(children) = map.get("children") {
            collect_exported_effective_aliases(children, aliases);
        }
    }
}

fn build_effective_queue_aliases(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    exported_effective_nodes: &HashMap<String, String>,
) -> (EffectiveQueueAliasMap, EffectiveQueueAliasMap) {
    let ui_by_node = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut aliases_by_id = HashMap::new();
    let mut aliases_by_name = HashMap::new();

    for effective_node in &effective.nodes {
        if exported_effective_nodes.contains_key(&effective_node.node_id) {
            continue;
        }
        if effective_node.effective_attachment_id.is_some() {
            continue;
        }
        let Some(parent_name) = exported_effective_nodes
            .get(effective_node.logical_parent_node_id.as_str())
            .cloned()
        else {
            continue;
        };
        let resolved = (
            effective_node.logical_parent_node_id.clone(),
            parent_name.clone(),
        );
        aliases_by_id
            .entry(effective_node.node_id.clone())
            .or_insert_with(|| resolved.clone());
        if let Some(ui_node) = ui_by_node.get(effective_node.node_id.as_str()).copied() {
            aliases_by_name
                .entry(ui_node.node_name.clone())
                .or_insert_with(|| resolved.clone());
        }
    }

    (aliases_by_id, aliases_by_name)
}

fn resolve_legacy_parent_from_effective_tree(
    parent_node: &str,
    parent_node_id: Option<&str>,
    exported_effective_nodes: &HashMap<String, String>,
    exported_effective_aliases: &HashMap<String, (String, String)>,
    queue_aliases_by_id: &HashMap<String, (String, String)>,
    queue_aliases_by_name: &HashMap<String, (String, String)>,
) -> Option<(String, String)> {
    let trimmed_id = parent_node_id.and_then(optional_non_empty);
    let trimmed_name = optional_non_empty(parent_node);

    if let Some(parent_id) = trimmed_id.as_deref()
        && let Some(parent_name) = exported_effective_nodes.get(parent_id).cloned()
    {
        return Some((parent_id.to_string(), parent_name));
    }
    if let Some(parent_id) = trimmed_id.as_deref()
        && let Some(resolved) = queue_aliases_by_id.get(parent_id).cloned()
    {
        return Some(resolved);
    }
    if let Some(parent_name) = trimmed_name.as_deref()
        && let Some(parent_id) = exported_effective_nodes
            .iter()
            .find_map(|(node_id, node_name)| (node_name == parent_name).then(|| node_id.clone()))
    {
        return Some((parent_id, parent_name.to_string()));
    }
    if let Some(parent_name) = trimmed_name.as_deref()
        && let Some(resolved) = queue_aliases_by_name.get(parent_name).cloned()
    {
        return Some(resolved);
    }
    trimmed_name.and_then(|alias| exported_effective_aliases.get(&alias).cloned())
}

fn selected_attachment_name_for_node(
    ui_node: &TopologyEditorNode,
    effective_node: &TopologyEffectiveNodeState,
) -> Option<String> {
    effective_node
        .effective_attachment_id
        .as_deref()
        .and_then(|attachment_id| {
            ui_node
                .allowed_parents
                .iter()
                .find_map(|parent| option_name(parent, attachment_id))
        })
        .or_else(|| optional_non_empty_owned(ui_node.effective_attachment_name.clone()))
        .or_else(|| optional_non_empty_owned(ui_node.current_attachment_name.clone()))
}

fn build_attachment_owner_map(
    ui_state: &TopologyEditorStateFile,
) -> HashMap<String, (String, String)> {
    let mut owners = HashMap::new();
    for node in &ui_state.nodes {
        for parent in &node.allowed_parents {
            for option in &parent.attachment_options {
                let Some(attachment_id) = optional_non_empty(&option.attachment_id) else {
                    continue;
                };
                if attachment_id == TOPOLOGY_ATTACHMENT_AUTO_ID {
                    continue;
                }
                owners
                    .entry(attachment_id)
                    .or_insert_with(|| (node.node_id.clone(), node.node_name.clone()));
            }
        }
    }
    owners
}

fn resolve_effective_parent_from_anchor(
    anchor_id: &str,
    ui_by_node: &HashMap<&str, &TopologyEditorNode>,
    effective_by_node: &HashMap<&str, &TopologyEffectiveNodeState>,
    exported_effective_nodes: &HashMap<String, String>,
    attachment_owner_by_attachment_id: &HashMap<String, (String, String)>,
    queue_aliases_by_id: &HashMap<String, (String, String)>,
) -> Option<(String, String, Option<String>, Option<String>)> {
    let anchor_id = anchor_id.trim();
    if anchor_id.is_empty() {
        return None;
    }

    if let Some(parent_name) = exported_effective_nodes.get(anchor_id).cloned() {
        let attachment_id = effective_by_node
            .get(anchor_id)
            .and_then(|node| optional_non_empty_owned(node.effective_attachment_id.clone()));
        let attachment_name = ui_by_node.get(anchor_id).and_then(|ui_node| {
            effective_by_node.get(anchor_id).and_then(|effective_node| {
                selected_attachment_name_for_node(ui_node, effective_node)
            })
        });
        return Some((
            anchor_id.to_string(),
            parent_name,
            attachment_id,
            attachment_name,
        ));
    }
    let Some((owner_node_id, owner_node_name)) =
        attachment_owner_by_attachment_id.get(anchor_id).cloned()
    else {
        if let Some((parent_id, parent_name)) = queue_aliases_by_id.get(anchor_id).cloned() {
            return Some((parent_id, parent_name, None, None));
        }
        return None;
    };
    let owner_ui = ui_by_node.get(owner_node_id.as_str()).copied()?;
    let owner_effective = effective_by_node.get(owner_node_id.as_str()).copied()?;

    if let Some(selected_attachment_id) = owner_effective.effective_attachment_id.as_deref()
        && let Some(parent_name) = exported_effective_nodes
            .get(selected_attachment_id)
            .cloned()
    {
        return Some((
            selected_attachment_id.to_string(),
            parent_name,
            optional_non_empty(selected_attachment_id),
            selected_attachment_name_for_node(owner_ui, owner_effective),
        ));
    }

    if let Some(parent_name) = exported_effective_nodes
        .get(owner_node_id.as_str())
        .cloned()
    {
        return Some((
            owner_node_id,
            parent_name,
            optional_non_empty_owned(owner_effective.effective_attachment_id.clone()),
            selected_attachment_name_for_node(owner_ui, owner_effective),
        ));
    }
    if let Some((parent_id, parent_name)) = queue_aliases_by_id.get(owner_node_id.as_str()).cloned()
    {
        return Some((parent_id, parent_name, None, None));
    }

    Some((
        owner_node_id,
        owner_node_name,
        optional_non_empty_owned(owner_effective.effective_attachment_id.clone()),
        selected_attachment_name_for_node(owner_ui, owner_effective),
    ))
}

fn ipv4_with_prefix_to_string(entry: &(std::net::Ipv4Addr, u32)) -> String {
    if entry.1 >= 32 {
        entry.0.to_string()
    } else {
        format!("{}/{}", entry.0, entry.1)
    }
}

fn ipv6_with_prefix_to_string(entry: &(std::net::Ipv6Addr, u32)) -> String {
    if entry.1 >= 128 {
        entry.0.to_string()
    } else {
        format!("{}/{}", entry.0, entry.1)
    }
}

fn load_circuit_anchors(
    config: &Config,
    shaped_devices_mtime: Option<std::time::SystemTime>,
) -> Vec<CircuitAnchor> {
    let anchors_path = circuit_anchors_path(config);
    let anchors_metadata = std::fs::metadata(&anchors_path).ok();
    if let Some(shaped_devices_mtime) = shaped_devices_mtime
        && let Some(anchors_mtime) = anchors_metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok())
        && anchors_mtime < shaped_devices_mtime
    {
        return Vec::new();
    }

    CircuitAnchorsFile::load(config)
        .map(|file| file.anchors)
        .unwrap_or_default()
}

fn load_integration_shaping_artifacts(
    config: &Config,
) -> Result<Option<(ConfigShapedDevices, Vec<CircuitAnchor>)>> {
    let topology_import = TopologyImportFile::load(config)
        .with_context(|| "Unable to load topology_import.json while validating shaping ingress")?;
    let Some(compiled_shaping) = TopologyCompiledShapingFile::load(config).with_context(
        || "Unable to load topology_compiled_shaping.json while building shaping_inputs.json",
    )?
    else {
        return Ok(None);
    };
    let import_identity = topology_import
        .as_ref()
        .and_then(|file| file.ingress_identity.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let compiled_identity = compiled_shaping
        .ingress_identity
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let (Some(import_identity), Some(compiled_identity)) = (import_identity, compiled_identity)
        && import_identity != compiled_identity
    {
        anyhow::bail!(
            "Integration compiled shaping ingress identity '{}' did not match topology import identity '{}'",
            compiled_identity,
            import_identity
        );
    }
    Ok(Some(compiled_shaping.shaping_artifacts()))
}

fn build_shaping_inputs(
    config: &Config,
    artifacts: &EffectiveTopologyArtifacts,
) -> Result<Option<TopologyShapingInputsFile>> {
    let integration_ingress = topology_import_ingress_enabled(config);
    let (mut shaped_devices, circuit_anchor_rows) = if integration_ingress {
        let Some((shaped_devices, circuit_anchors)) = load_integration_shaping_artifacts(config)?
        else {
            return Ok(None);
        };
        (shaped_devices, circuit_anchors)
    } else {
        let shaped_devices_path = ConfigShapedDevices::path_for_config(config);
        if !shaped_devices_path.exists() {
            return Ok(None);
        }

        let shaped_devices_mtime = std::fs::metadata(&shaped_devices_path)
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        (
            ConfigShapedDevices::load_for_config(config).with_context(
                || "Unable to load ShapedDevices.csv while building shaping_inputs.json",
            )?,
            load_circuit_anchors(config, shaped_devices_mtime),
        )
    };
    if integration_ingress {
        let effective_overrides = load_runtime_shaping_overrides(config).with_context(
            || "Unable to load effective overrides while building shaping_inputs.json",
        )?;
        let runtime_devices = apply_runtime_shaped_device_overrides(
            shaped_devices.devices.clone(),
            &effective_overrides,
        );
        shaped_devices.replace_with_new_data(runtime_devices);
    }
    let flat_bucket_assignments = if runtime_flat_mode(config) {
        Some(build_flat_bucket_assignments(
            config,
            &shaped_devices.devices,
        ))
    } else {
        None
    };
    let circuit_anchors = circuit_anchor_rows
        .into_iter()
        .map(|anchor| (anchor.circuit_id.clone(), anchor))
        .collect::<HashMap<_, _>>();
    let effective_by_node = artifacts
        .effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let ui_by_node = artifacts
        .ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut exported_effective_nodes = HashMap::<String, String>::new();
    let mut exported_effective_aliases = HashMap::<String, (String, String)>::new();
    let attachment_owner_by_attachment_id = build_attachment_owner_map(&artifacts.ui_state);
    if let Some(effective_network) = artifacts.effective_network.as_ref() {
        collect_exported_effective_nodes(effective_network, &mut exported_effective_nodes);
        collect_exported_effective_aliases(effective_network, &mut exported_effective_aliases);
    }
    let (queue_aliases_by_id, queue_aliases_by_name) = build_effective_queue_aliases(
        &artifacts.ui_state,
        &artifacts.effective,
        &exported_effective_nodes,
    );

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut circuits = Vec::<TopologyShapingCircuitInput>::new();
    let mut circuits_by_id = HashMap::<String, usize>::new();

    for device in &shaped_devices.devices {
        let anchor_from_file = circuit_anchors.get(&device.circuit_id);
        let anchor_node_id = anchor_from_file
            .map(|anchor| anchor.anchor_node_id.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| optional_non_empty_owned(device.anchor_node_id.clone()));
        let mut anchor_node_name = anchor_from_file.and_then(|anchor| {
            anchor
                .anchor_node_name
                .as_ref()
                .and_then(|value| optional_non_empty(value))
        });
        let (
            effective_parent_node_id,
            effective_parent_node_name,
            effective_attachment_id,
            effective_attachment_name,
            resolution_source,
        ) = if let Some((bucket_id, bucket_name)) = flat_bucket_assignments
            .as_ref()
            .and_then(|assignments| assignments.get(&device.circuit_id))
        {
            (
                bucket_id.clone(),
                bucket_name.clone(),
                None,
                None,
                TopologyShapingResolutionSource::FlatBucket,
            )
        } else if let Some(anchor_id) = anchor_node_id.as_deref() {
            match resolve_effective_parent_from_anchor(
                anchor_id,
                &ui_by_node,
                &effective_by_node,
                &exported_effective_nodes,
                &attachment_owner_by_attachment_id,
                &queue_aliases_by_id,
            ) {
                Some((
                    resolved_parent_id,
                    resolved_parent_name,
                    resolved_attachment_id,
                    resolved_attachment_name,
                )) => {
                    if let Some(ui_node) = ui_by_node.get(anchor_id) {
                        anchor_node_name = Some(ui_node.node_name.clone());
                    }
                    (
                        resolved_parent_id,
                        resolved_parent_name,
                        resolved_attachment_id,
                        resolved_attachment_name,
                        TopologyShapingResolutionSource::TopologyAnchor,
                    )
                }
                None => match (
                    ui_by_node.get(anchor_id),
                    effective_by_node.get(anchor_id),
                    anchor_from_file,
                ) {
                    (Some(ui_node), Some(_effective_node), _) => {
                        anchor_node_name = Some(ui_node.node_name.clone());
                        warnings.push(format!(
                            "Circuit '{}' anchor '{}' ('{}') did not resolve to an exported effective queue node. Falling back to generated parent-node shaping.",
                            device.circuit_id,
                            anchor_id,
                            ui_node.node_name
                        ));
                        (
                            String::new(),
                            String::new(),
                            None,
                            None,
                            TopologyShapingResolutionSource::RuntimeFallback,
                        )
                    }
                    (None, None, Some(anchor)) => {
                        warnings.push(format!(
                            "Circuit '{}' anchor '{}' ('{}') was not found in the effective topology. Falling back to generated parent-node shaping.",
                            device.circuit_id,
                            anchor_id,
                            anchor
                                .anchor_node_name
                                .as_deref()
                                .unwrap_or_default()
                        ));
                        (
                            String::new(),
                            String::new(),
                            None,
                            None,
                            TopologyShapingResolutionSource::RuntimeFallback,
                        )
                    }
                    _ => {
                        warnings.push(format!(
                            "Circuit '{}' anchor '{}' was not found in the effective topology. Falling back to generated parent-node shaping.",
                            device.circuit_id, anchor_id
                        ));
                        (
                            String::new(),
                            String::new(),
                            None,
                            None,
                            TopologyShapingResolutionSource::RuntimeFallback,
                        )
                    }
                },
            }
        } else if let Some((resolved_parent_id, resolved_parent_name)) =
            resolve_legacy_parent_from_effective_tree(
                &device.parent_node,
                device.parent_node_id.as_deref(),
                &exported_effective_nodes,
                &exported_effective_aliases,
                &queue_aliases_by_id,
                &queue_aliases_by_name,
            )
        {
            (
                resolved_parent_id,
                resolved_parent_name,
                None,
                None,
                TopologyShapingResolutionSource::LegacyParent,
            )
        } else {
            if optional_non_empty(&device.parent_node).is_some()
                || optional_non_empty_owned(device.parent_node_id.clone()).is_some()
            {
                warnings.push(format!(
                    "Circuit '{}' parent reference '{}' ({}) was not found in the exported effective topology. Falling back to generated parent-node shaping.",
                    device.circuit_id,
                    device.parent_node.trim(),
                    device.parent_node_id.clone().unwrap_or_default().trim()
                ));
            }
            (
                String::new(),
                String::new(),
                None,
                None,
                TopologyShapingResolutionSource::RuntimeFallback,
            )
        };

        let logical_parent_node_name = optional_non_empty(&device.parent_node);
        let logical_parent_node_id = optional_non_empty_owned(device.parent_node_id.clone());
        let circuit_index = if let Some(index) = circuits_by_id.get(&device.circuit_id).copied() {
            let circuit = &mut circuits[index];
            if circuit.anchor_node_id != anchor_node_id {
                errors.push(format!(
                    "Circuit '{}' had multiple AnchorNodeID values while building shaping inputs.",
                    device.circuit_id
                ));
            }
            if circuit.effective_parent_node_id != effective_parent_node_id
                || circuit.effective_parent_node_name != effective_parent_node_name
            {
                errors.push(format!(
                    "Circuit '{}' resolved to multiple effective parents while building shaping_inputs.json.",
                    device.circuit_id
                ));
            }
            index
        } else {
            let index = circuits.len();
            circuits_by_id.insert(device.circuit_id.clone(), index);
            circuits.push(TopologyShapingCircuitInput {
                circuit_id: device.circuit_id.clone(),
                circuit_name: device.circuit_name.clone(),
                anchor_node_id: anchor_node_id.clone(),
                anchor_node_name,
                logical_parent_node_name,
                logical_parent_node_id,
                effective_parent_node_name,
                effective_parent_node_id,
                effective_attachment_id,
                effective_attachment_name,
                resolution_source,
                download_min_mbps: device.download_min_mbps,
                upload_min_mbps: device.upload_min_mbps,
                download_max_mbps: device.download_max_mbps,
                upload_max_mbps: device.upload_max_mbps,
                comment: device.comment.clone(),
                sqm_override: device.sqm_override.clone(),
                devices: Vec::new(),
            });
            index
        };

        circuits[circuit_index]
            .devices
            .push(TopologyShapingDeviceInput {
                device_id: device.device_id.clone(),
                device_name: device.device_name.clone(),
                mac: device.mac.clone(),
                ipv4: device.ipv4.iter().map(ipv4_with_prefix_to_string).collect(),
                ipv6: device.ipv6.iter().map(ipv6_with_prefix_to_string).collect(),
                comment: device.comment.clone(),
            });
    }

    circuits.sort_unstable_by(|left, right| left.circuit_id.cmp(&right.circuit_id));
    for circuit in &mut circuits {
        circuit
            .devices
            .sort_unstable_by(|left, right| left.device_id.cmp(&right.device_id));
    }

    let unresolved_runtime_fallbacks = circuits
        .iter()
        .filter(|circuit| {
            circuit.effective_parent_node_id.trim().is_empty()
                && circuit.resolution_source == TopologyShapingResolutionSource::RuntimeFallback
        })
        .count();
    if unresolved_runtime_fallbacks > 0 {
        if config.shared_topology_compile_mode() == Some("flat") {
            warnings.push(format!(
                "Flat topology mode assigned {unresolved_runtime_fallbacks} circuit(s) to generated parent nodes during queue construction."
            ));
        } else {
            for circuit in &circuits {
                if circuit.effective_parent_node_id.trim().is_empty()
                    && circuit.resolution_source == TopologyShapingResolutionSource::RuntimeFallback
                {
                    warnings.push(format!(
                        "Circuit '{}' is unresolved in runtime topology and will be shaped under generated parent nodes.",
                        circuit.circuit_id
                    ));
                }
            }
        }
    }

    if !errors.is_empty() {
        let mut message = String::from(
            "Unable to build shaping_inputs.json due to runtime topology contract errors:",
        );
        for error in errors {
            message.push_str("\n- ");
            message.push_str(&error);
        }
        return Err(anyhow::anyhow!(message));
    }

    Ok(Some(TopologyShapingInputsFile {
        schema_version: 1,
        shaping_generation: String::new(),
        generated_unix: now_unix(),
        canonical_generated_unix: artifacts.effective.canonical_generated_unix,
        effective_generated_unix: artifacts.effective.generated_unix,
        warnings,
        circuits,
    }))
}

fn topology_import_ingress_enabled(config: &Config) -> bool {
    config.uisp_integration.enable_uisp
        || config.splynx_integration.enable_splynx
        || config
            .netzur_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_netzur)
        || config
            .visp_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_visp)
        || config.powercode_integration.enable_powercode
        || config.sonar_integration.enable_sonar
        || config
            .wispgate_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_wispgate)
}

fn runtime_flat_mode(config: &Config) -> bool {
    config.shared_topology_compile_mode() == Some("flat")
}

fn count_interface_tx_queues(interface_name: &str) -> Option<usize> {
    let path = Path::new("/sys/class/net")
        .join(interface_name)
        .join("queues");
    let entries = std::fs::read_dir(path).ok()?;
    let mut count = 0usize;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with("tx-") {
            count += 1;
        }
    }
    Some(count)
}

fn runtime_flat_bucket_count(config: &Config) -> usize {
    let mut queues_available = config
        .queues
        .override_available_queues
        .map(|value| value as usize);
    if queues_available.is_none() {
        queues_available = if config.queues.dry_run {
            Some(16)
        } else {
            let internet_queues = count_interface_tx_queues(&config.internet_interface());
            let isp_queues = count_interface_tx_queues(&config.isp_interface());
            internet_queues
                .zip(isp_queues)
                .map(|(left, right)| left.min(right))
        };
    }

    let shaping_cpu_count = detect_shaping_cpus(config).shaping.len();
    let mut queue_count = queues_available.unwrap_or(shaping_cpu_count.max(1));
    if shaping_cpu_count > 0 {
        queue_count = queue_count.min(shaping_cpu_count);
    }
    if config.on_a_stick_mode() {
        queue_count = (queue_count / 2).max(1);
    }
    queue_count.max(1)
}

fn runtime_flat_bucket_name(index: usize) -> String {
    format!("Generated_PN_{}", index + 1)
}

fn runtime_flat_bucket_id(index: usize) -> String {
    format!("libreqos:generated:flat:bucket:{index}")
}

fn runtime_flat_bucket_network(config: &Config) -> Value {
    let mut root = Map::new();
    for index in 0..runtime_flat_bucket_count(config) {
        let mut node = Map::new();
        node.insert("children".to_string(), Value::Object(Map::new()));
        node.insert(
            "downloadBandwidthMbps".to_string(),
            Value::Number(config.queues.generated_pn_download_mbps.into()),
        );
        node.insert(
            "uploadBandwidthMbps".to_string(),
            Value::Number(config.queues.generated_pn_upload_mbps.into()),
        );
        node.insert(
            "id".to_string(),
            Value::String(runtime_flat_bucket_id(index)),
        );
        node.insert(
            "name".to_string(),
            Value::String(runtime_flat_bucket_name(index)),
        );
        node.insert("type".to_string(), Value::String("Site".to_string()));
        root.insert(runtime_flat_bucket_name(index), Value::Object(node));
    }
    Value::Object(root)
}

fn build_flat_bucket_assignments(
    config: &Config,
    devices: &[lqos_config::ShapedDevice],
) -> HashMap<String, (String, String)> {
    let bucket_count = runtime_flat_bucket_count(config);
    if bucket_count == 0 {
        return HashMap::new();
    }

    let bucket_names = (0..bucket_count)
        .map(runtime_flat_bucket_name)
        .collect::<Vec<_>>();
    let bucket_ids_by_name = bucket_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), runtime_flat_bucket_id(index)))
        .collect::<HashMap<_, _>>();

    let mut item_weights = BTreeMap::<String, f64>::new();
    for device in devices {
        let weight = f64::from(device.download_max_mbps.max(0.0) + device.upload_max_mbps.max(0.0));
        let sanitized = if weight.is_finite() && weight > 0.0 {
            weight
        } else {
            1.0
        };
        item_weights
            .entry(device.circuit_id.clone())
            .and_modify(|current| {
                if sanitized > *current {
                    *current = sanitized;
                }
            })
            .or_insert(sanitized);
    }

    let items = item_weights
        .into_iter()
        .map(|(id, weight)| TopLevelPlannerItem { id, weight })
        .collect::<Vec<_>>();
    let planner_mode = if config.queues.use_binpacking {
        TopLevelPlannerMode::Greedy
    } else {
        TopLevelPlannerMode::RoundRobin
    };
    let planner = plan_top_level_assignments(
        &items,
        &bucket_names,
        &BTreeMap::new(),
        &BTreeMap::new(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(0.0),
        &TopLevelPlannerParams {
            mode: planner_mode,
            hysteresis_threshold: 0.0,
            cooldown_seconds: 0.0,
            move_budget_per_run: usize::MAX,
        },
    );

    planner
        .assignment
        .into_iter()
        .filter_map(|(circuit_id, bucket_name)| {
            bucket_ids_by_name
                .get(&bucket_name)
                .map(|bucket_id| (circuit_id, (bucket_id.clone(), bucket_name)))
        })
        .collect()
}

fn load_runtime_shaping_overrides(config: &Config) -> Result<lqos_overrides::OverrideFile> {
    let apply_stormguard = config
        .stormguard
        .as_ref()
        .is_some_and(|stormguard| stormguard.enabled && !stormguard.dry_run);
    let apply_treeguard = config.treeguard.enabled;
    OverrideStore::load_effective_for_config(config, apply_stormguard, apply_treeguard)
        .with_context(|| "Unable to load effective override layers")
}

fn apply_runtime_shaped_device_overrides(
    base_devices: Vec<lqos_config::ShapedDevice>,
    overrides: &lqos_overrides::OverrideFile,
) -> Vec<lqos_config::ShapedDevice> {
    let mut devices = base_devices;
    for override_device in overrides.persistent_devices() {
        if let Some(existing_index) = devices
            .iter()
            .position(|device| device.device_id == override_device.device_id)
        {
            devices[existing_index] = override_device.clone();
        } else {
            devices.push(override_device.clone());
        }
    }

    for adjustment in overrides.circuit_adjustments() {
        match adjustment {
            CircuitAdjustment::CircuitAdjustSpeed {
                circuit_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.circuit_id == *circuit_id)
                {
                    if let Some(value) = min_download_bandwidth {
                        device.download_min_mbps = *value;
                    }
                    if let Some(value) = max_download_bandwidth {
                        device.download_max_mbps = *value;
                    }
                    if let Some(value) = min_upload_bandwidth {
                        device.upload_min_mbps = *value;
                    }
                    if let Some(value) = max_upload_bandwidth {
                        device.upload_max_mbps = *value;
                    }
                }
            }
            CircuitAdjustment::DeviceAdjustSpeed {
                device_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.device_id == *device_id)
                {
                    if let Some(value) = min_download_bandwidth {
                        device.download_min_mbps = *value;
                    }
                    if let Some(value) = max_download_bandwidth {
                        device.download_max_mbps = *value;
                    }
                    if let Some(value) = min_upload_bandwidth {
                        device.upload_min_mbps = *value;
                    }
                    if let Some(value) = max_upload_bandwidth {
                        device.upload_max_mbps = *value;
                    }
                }
            }
            CircuitAdjustment::DeviceAdjustSqm {
                device_id,
                sqm_override,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.device_id == *device_id)
                {
                    device.sqm_override = sqm_override
                        .as_ref()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty());
                }
            }
            CircuitAdjustment::RemoveCircuit { circuit_id } => {
                devices.retain(|device| device.circuit_id != *circuit_id);
            }
            CircuitAdjustment::RemoveDevice { device_id } => {
                devices.retain(|device| device.device_id != *device_id);
            }
            CircuitAdjustment::ReparentCircuit {
                circuit_id,
                parent_node,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.circuit_id == *circuit_id)
                {
                    device.parent_node = parent_node.clone();
                    device.parent_node_id = None;
                }
            }
        }
    }

    devices
}

/// Loads canonical topology state, falling back to importing legacy `network.json`.
pub fn load_canonical_topology_state(config: &Config) -> TopologyCanonicalStateFile {
    TopologyCanonicalStateFile::load_with_legacy_fallback(config).unwrap_or_default()
}

/// Validated effective-topology artifacts ready for publication.
#[derive(Clone, Debug)]
pub struct EffectiveTopologyArtifacts {
    /// Runtime-effective topology state derived from canonical topology and overrides.
    pub effective: TopologyEffectiveStateFile,
    /// UI-facing merged topology state derived from canonical topology, overrides, and health.
    pub ui_state: TopologyEditorStateFile,
    /// Runtime-effective tree used by shaping/export when canonical compatibility network exists.
    pub effective_network: Option<Value>,
}

struct EffectivePublishLock {
    path: PathBuf,
}

impl Drop for EffectivePublishLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn acquire_effective_publish_lock(config: &Config) -> Result<EffectivePublishLock> {
    let path = Path::new(&config.lqos_directory).join(TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_FILENAME);
    for _ in 0..50 {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(EffectivePublishLock { path }),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = std::fs::metadata(&path)
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|elapsed| elapsed.as_secs() > 30);
                if stale {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "Unable to acquire topology effective publish lock at {:?}",
                        path
                    )
                });
            }
        }
    }
    Err(anyhow::anyhow!(
        "Timed out waiting for topology effective publish lock at {:?}",
        path
    ))
}

/// Builds validated effective-topology artifacts from canonical topology plus operator intent.
///
/// If `canonical_network` is provided, the returned effective network export has already passed
/// structural validation and is safe to publish.
pub fn build_effective_topology_artifacts_from_canonical(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    let prepared = prepared_runtime_topology_editor_state(&canonical.to_editor_state(), overrides);
    build_effective_topology_artifacts_from_prepared(
        config, canonical, &prepared, overrides, health,
    )
}

/// Builds validated effective-topology artifacts from legacy editor state plus compatibility
/// `network.json`.
///
/// This helper preserves existing test call sites while routing through the canonical topology
/// model used in production.
pub fn build_effective_topology_artifacts(
    config: &Config,
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
    canonical_network: Option<&Value>,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    let canonical_state = TopologyCanonicalStateFile::from_editor_and_network(
        canonical,
        canonical_network.unwrap_or(&Value::Object(Map::new())),
        lqos_config::TopologyCanonicalIngressKind::NativeIntegration,
    );
    build_effective_topology_artifacts_from_canonical(config, &canonical_state, overrides, health)
}

fn build_effective_topology_artifacts_from_prepared(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    prepared: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    let effective = compute_effective_state_from_prepared(config, prepared, overrides, health);
    let ui_state =
        merged_topology_state_from_prepared(config, prepared, overrides, health, &effective);
    let canonical_network =
        if canonical.ingress_kind == TopologyCanonicalIngressKind::NativeIntegration {
            canonical.insight_topology_network_json()
        } else {
            canonical.compatibility_network_json().clone()
        };
    let effective_network = if runtime_flat_mode(config) {
        Some(runtime_flat_bucket_network(config))
    } else {
        canonical_network.as_object().map(|_| {
            apply_effective_topology_to_canonical_state(config, canonical, &ui_state, &effective)
        })
    };

    if let Some(effective_network) = effective_network.as_ref() {
        validate_effective_topology_network_from_canonical(
            config,
            canonical,
            &ui_state,
            &effective,
            effective_network,
        )?;
    }

    Ok(EffectiveTopologyArtifacts {
        effective,
        ui_state,
        effective_network,
    })
}

/// Publishes validated effective-topology artifacts under a single writer lock.
///
/// Side effects: writes `topology_effective_state.json`, `network.effective.json` when present,
/// and `shaping_inputs.json` when `ShapedDevices.csv` is available.
/// If no effective network export is present, any stale `network.effective.json` is removed so
/// runtime consumers fall back to canonical integration output.
pub fn publish_effective_topology_artifacts(
    config: &Config,
    artifacts: &EffectiveTopologyArtifacts,
    source_generation: &str,
) -> Result<()> {
    let _lock = acquire_effective_publish_lock(config)?;

    let effective_state_path = topology_effective_state_path(config);
    let current_effective_state = TopologyEffectiveStateFile::load(config).ok();
    if !current_effective_state
        .as_ref()
        .is_some_and(|current| effective_state_payload_equals(current, &artifacts.effective))
    {
        let effective_state_value = serde_json::to_value(&artifacts.effective)?;
        atomic_write_json_value(&effective_state_path, &effective_state_value).with_context(
            || {
                format!(
                    "Unable to publish effective topology state at {:?}",
                    effective_state_path
                )
            },
        )?;
    }

    let effective_network_path = topology_effective_network_path(config);
    let effective_generation = artifacts
        .effective_network
        .as_ref()
        .map(compute_effective_network_generation)
        .transpose()?;
    if let Some(effective_network) = artifacts.effective_network.as_ref() {
        let current_effective_network = read_json_value(&effective_network_path);
        if current_effective_network.as_ref() != Some(effective_network) {
            atomic_write_json_value(&effective_network_path, effective_network).with_context(
                || {
                    format!(
                        "Unable to publish effective topology network at {:?}",
                        effective_network_path
                    )
                },
            )?;
        }
    } else if effective_network_path.exists() {
        std::fs::remove_file(&effective_network_path).with_context(|| {
            format!(
                "Unable to remove stale effective topology network at {:?}",
                effective_network_path
            )
        })?;
    }

    let shaping_inputs_path = topology_shaping_inputs_path(config);
    let shaping_inputs = match build_shaping_inputs(config, artifacts) {
        Ok(value) => value,
        Err(err) => {
            if shaping_inputs_path.exists() {
                std::fs::remove_file(&shaping_inputs_path).with_context(|| {
                    format!(
                        "Unable to remove stale runtime shaping inputs at {:?}",
                        shaping_inputs_path
                    )
                })?;
            }
            return Err(err);
        }
    };
    match shaping_inputs {
        Some(mut shaping_inputs) => {
            shaping_inputs.shaping_generation = shaping_inputs.compute_shaping_generation()?;
            let current_shaping_inputs = TopologyShapingInputsFile::load(config).ok();
            if !current_shaping_inputs
                .as_ref()
                .is_some_and(|current| current.semantic_equals(&shaping_inputs))
            {
                let shaping_inputs_value = serde_json::to_value(&shaping_inputs)?;
                atomic_write_json_value(&shaping_inputs_path, &shaping_inputs_value).with_context(
                    || {
                        format!(
                            "Unable to publish runtime shaping inputs at {:?}",
                            shaping_inputs_path
                        )
                    },
                )?;
            }
            publish_topology_runtime_status(
                config,
                source_generation,
                Some(&shaping_inputs.shaping_generation),
                effective_generation.as_deref(),
                true,
                None,
            )?;
        }
        None => {
            if shaping_inputs_path.exists() {
                std::fs::remove_file(&shaping_inputs_path).with_context(|| {
                    format!(
                        "Unable to remove stale runtime shaping inputs at {:?}",
                        shaping_inputs_path
                    )
                })?;
            }
            publish_topology_runtime_status(
                config,
                source_generation,
                None,
                effective_generation.as_deref(),
                true,
                None,
            )?;
        }
    }

    Ok(())
}

fn topology_runtime_status_snapshot(
    config: &Config,
    source_generation: &str,
    shaping_generation: Option<&str>,
    effective_generation: Option<&str>,
    ready: bool,
    error: Option<String>,
) -> TopologyRuntimeStatusFile {
    TopologyRuntimeStatusFile {
        schema_version: 1,
        source_generation: source_generation.to_string(),
        shaping_generation: shaping_generation.unwrap_or_default().to_string(),
        effective_generation: effective_generation.unwrap_or_default().to_string(),
        ready,
        generated_unix: now_unix(),
        effective_state_path: topology_effective_state_path(config)
            .to_string_lossy()
            .to_string(),
        effective_network_path: topology_effective_network_path(config)
            .to_string_lossy()
            .to_string(),
        shaping_inputs_path: topology_shaping_inputs_path(config)
            .to_string_lossy()
            .to_string(),
        error,
    }
}

/// Publishes topology runtime readiness for one source generation.
///
/// Side effects: writes `topology_runtime_status.json` in `config.lqos_directory`.
pub fn publish_topology_runtime_status(
    config: &Config,
    source_generation: &str,
    shaping_generation: Option<&str>,
    effective_generation: Option<&str>,
    ready: bool,
    error: Option<String>,
) -> Result<()> {
    let status = topology_runtime_status_snapshot(
        config,
        source_generation,
        shaping_generation,
        effective_generation,
        ready,
        error,
    );
    status.save(config).with_context(|| {
        format!(
            "Unable to publish topology runtime status at {:?}",
            topology_runtime_status_path(config)
        )
    })?;
    Ok(())
}

/// Publishes a failed topology runtime status for one source generation.
///
/// Side effects: writes `topology_runtime_status.json` in `config.lqos_directory`.
pub fn publish_topology_runtime_error_status(
    config: &Config,
    source_generation: &str,
    error: &str,
) -> Result<()> {
    publish_topology_runtime_status(
        config,
        source_generation,
        None,
        None,
        false,
        Some(error.to_string()),
    )
}

fn parse_probe_ip(raw: &str) -> Option<IpAddr> {
    raw.trim()
        .split('/')
        .next()
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<IpAddr>().ok())
}

/// Returns the runtime stale cutoff in seconds for topology attachment health.
pub fn health_state_stale_after_seconds(config: &Config) -> u64 {
    config
        .integration_common
        .topology_attachment_health
        .probe_interval_seconds
        .saturating_mul(
            u64::from(
                config
                    .integration_common
                    .topology_attachment_health
                    .fail_after_missed,
            )
            .saturating_mul(3),
        )
}

/// Returns true when `health` is recent enough to be trusted for runtime suppression.
pub fn is_health_state_fresh(config: &Config, health: &TopologyAttachmentHealthStateFile) -> bool {
    let Some(generated_unix) = health.generated_unix else {
        return false;
    };
    let Some(now) = now_unix() else {
        return false;
    };
    now.saturating_sub(generated_unix) <= health_state_stale_after_seconds(config)
}

fn auto_attachment_option() -> TopologyAttachmentOption {
    TopologyAttachmentOption {
        attachment_id: TOPOLOGY_ATTACHMENT_AUTO_ID.to_string(),
        attachment_name: "Auto".to_string(),
        attachment_kind: "auto".to_string(),
        attachment_role: TopologyAttachmentRole::Unknown,
        pair_id: None,
        peer_attachment_id: None,
        peer_attachment_name: None,
        capacity_mbps: None,
        download_bandwidth_mbps: None,
        upload_bandwidth_mbps: None,
        transport_cap_mbps: None,
        transport_cap_reason: None,
        rate_source: TopologyAttachmentRateSource::Unknown,
        can_override_rate: false,
        rate_override_disabled_reason: None,
        has_rate_override: false,
        local_probe_ip: None,
        remote_probe_ip: None,
        probe_enabled: false,
        probeable: false,
        health_status: TopologyAttachmentHealthStatus::Disabled,
        health_reason: None,
        suppressed_until_unix: None,
        effective_selected: false,
    }
}

fn overlay_manual_groups(
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
) -> TopologyEditorStateFile {
    let mut state = canonical.clone();
    for node in &mut state.nodes {
        for parent in &mut node.allowed_parents {
            let Some(group) =
                overrides.find_manual_attachment_group(&node.node_id, &parent.parent_node_id)
            else {
                continue;
            };
            let mut options = vec![auto_attachment_option()];
            for attachment in &group.attachments {
                let local_probe_ip = parse_probe_ip(&attachment.local_probe_ip);
                let remote_probe_ip = parse_probe_ip(&attachment.remote_probe_ip);
                let probeable = local_probe_ip
                    .zip(remote_probe_ip)
                    .is_some_and(|(local, remote)| local != remote);
                options.push(TopologyAttachmentOption {
                    attachment_id: attachment.attachment_id.clone(),
                    attachment_name: attachment.attachment_name.clone(),
                    attachment_kind: "manual".to_string(),
                    attachment_role: TopologyAttachmentRole::Manual,
                    pair_id: Some(attachment.attachment_id.clone()),
                    peer_attachment_id: None,
                    peer_attachment_name: None,
                    capacity_mbps: Some(attachment.capacity_mbps),
                    download_bandwidth_mbps: Some(attachment.capacity_mbps),
                    upload_bandwidth_mbps: Some(attachment.capacity_mbps),
                    transport_cap_mbps: None,
                    transport_cap_reason: None,
                    rate_source: TopologyAttachmentRateSource::Manual,
                    can_override_rate: true,
                    rate_override_disabled_reason: None,
                    has_rate_override: false,
                    local_probe_ip: Some(attachment.local_probe_ip.clone()),
                    remote_probe_ip: Some(attachment.remote_probe_ip.clone()),
                    probe_enabled: attachment.probe_enabled,
                    probeable,
                    health_status: if attachment.probe_enabled {
                        if probeable {
                            TopologyAttachmentHealthStatus::Healthy
                        } else {
                            TopologyAttachmentHealthStatus::ProbeUnavailable
                        }
                    } else {
                        TopologyAttachmentHealthStatus::Disabled
                    },
                    health_reason: None,
                    suppressed_until_unix: None,
                    effective_selected: false,
                });
            }
            parent.attachment_options = options;
        }
    }
    state
}

fn attachment_capacity_mbps(option: &TopologyAttachmentOption) -> Option<u64> {
    match (
        option.download_bandwidth_mbps,
        option.upload_bandwidth_mbps,
        option.capacity_mbps,
    ) {
        (Some(download), Some(upload), _) => Some(download.min(upload)),
        (Some(download), None, _) => Some(download),
        (None, Some(upload), _) => Some(upload),
        (None, None, capacity) => capacity,
    }
}

fn apply_attachment_rate_overrides(
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
) -> TopologyEditorStateFile {
    let mut state = canonical.clone();
    for node in &mut state.nodes {
        for parent in &mut node.allowed_parents {
            for option in &mut parent.attachment_options {
                if option.attachment_id == TOPOLOGY_ATTACHMENT_AUTO_ID {
                    continue;
                }
                let Some(rate_override) = overrides.find_attachment_rate_override(
                    &node.node_id,
                    &parent.parent_node_id,
                    &option.attachment_id,
                ) else {
                    continue;
                };
                if !option.can_override_rate {
                    continue;
                }

                option.download_bandwidth_mbps = Some(rate_override.download_bandwidth_mbps);
                option.upload_bandwidth_mbps = Some(rate_override.upload_bandwidth_mbps);
                option.capacity_mbps = attachment_capacity_mbps(option);
                option.has_rate_override = true;
            }
        }
    }
    state
}

fn probe_enabled_for_option(
    option: &TopologyAttachmentOption,
    overrides: &TopologyOverridesFile,
) -> bool {
    let Some(pair_id) = option.pair_id.as_ref() else {
        return false;
    };
    overrides
        .find_probe_policy(pair_id)
        .map(|policy| policy.enabled)
        .unwrap_or(option.probe_enabled)
}

fn probe_unavailable_reason(option: &TopologyAttachmentOption) -> String {
    let local = option
        .local_probe_ip
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    let remote = option
        .remote_probe_ip
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();

    if local.is_empty() && remote.is_empty() {
        return "Probe unavailable: missing local and remote management IPs".to_string();
    }
    if local.is_empty() {
        return "Probe unavailable: missing local management IP".to_string();
    }
    if remote.is_empty() {
        return "Probe unavailable: missing remote management IP".to_string();
    }
    if parse_probe_ip(local)
        .zip(parse_probe_ip(remote))
        .is_some_and(|(local, remote)| local == remote)
    {
        return "Probe unavailable: local and remote probe IPs are identical".to_string();
    }
    if parse_probe_ip(local).is_none() && parse_probe_ip(remote).is_none() {
        return "Probe unavailable: local and remote probe IPs are invalid".to_string();
    }
    if parse_probe_ip(local).is_none() {
        return "Probe unavailable: local management IP is invalid".to_string();
    }
    if parse_probe_ip(remote).is_none() {
        return "Probe unavailable: remote management IP is invalid".to_string();
    }
    "Probe unavailable".to_string()
}

fn apply_health_to_option(
    option: &TopologyAttachmentOption,
    overrides: &TopologyOverridesFile,
    health_by_pair: &HashMap<&str, &lqos_config::TopologyAttachmentHealthEntry>,
) -> TopologyAttachmentOption {
    if option.attachment_id == TOPOLOGY_ATTACHMENT_AUTO_ID {
        return option.clone();
    }

    let enabled = probe_enabled_for_option(option, overrides);
    let probeable = option
        .local_probe_ip
        .as_ref()
        .zip(option.remote_probe_ip.as_ref())
        .and_then(|(local, remote)| parse_probe_ip(local).zip(parse_probe_ip(remote)))
        .is_some_and(|(local, remote)| local != remote);

    let (health_status, health_reason, suppressed_until_unix) = if !enabled {
        (
            TopologyAttachmentHealthStatus::Disabled,
            Some("Health probe disabled".to_string()),
            None,
        )
    } else if !probeable {
        (
            TopologyAttachmentHealthStatus::ProbeUnavailable,
            Some(probe_unavailable_reason(option)),
            None,
        )
    } else if let Some(pair_id) = option.pair_id.as_deref() {
        if let Some(entry) = health_by_pair.get(pair_id) {
            (
                entry.status,
                entry.reason.clone(),
                entry.suppressed_until_unix,
            )
        } else {
            (TopologyAttachmentHealthStatus::Healthy, None, None)
        }
    } else {
        (TopologyAttachmentHealthStatus::Healthy, None, None)
    };

    let mut out = option.clone();
    out.probe_enabled = enabled;
    out.probeable = probeable;
    out.health_status = health_status;
    out.health_reason = health_reason;
    out.suppressed_until_unix = suppressed_until_unix;
    out
}

fn enrich_allowed_parent(
    parent: &TopologyAllowedParent,
    overrides: &TopologyOverridesFile,
    health_by_pair: &HashMap<&str, &lqos_config::TopologyAttachmentHealthEntry>,
    effective_attachment_id: Option<&str>,
    effective_parent_id: Option<&str>,
) -> TopologyAllowedParent {
    let mut all_attachments_suppressed = true;
    let mut has_probe_unavailable = false;
    let mut saw_explicit = false;
    let attachment_options = parent
        .attachment_options
        .iter()
        .map(|option| {
            let mut option = apply_health_to_option(option, overrides, health_by_pair);
            if option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID {
                saw_explicit = true;
                if option.health_status != TopologyAttachmentHealthStatus::Suppressed {
                    all_attachments_suppressed = false;
                }
                if option.health_status == TopologyAttachmentHealthStatus::ProbeUnavailable {
                    has_probe_unavailable = true;
                }
            }
            option.effective_selected = effective_parent_id == Some(parent.parent_node_id.as_str())
                && effective_attachment_id == Some(option.attachment_id.as_str());
            option
        })
        .collect::<Vec<_>>();

    TopologyAllowedParent {
        parent_node_id: parent.parent_node_id.clone(),
        parent_node_name: parent.parent_node_name.clone(),
        attachment_options,
        all_attachments_suppressed: saw_explicit && all_attachments_suppressed,
        has_probe_unavailable_attachments: has_probe_unavailable,
    }
}

fn valid_attachment_ids(parent: &TopologyAllowedParent) -> HashSet<&str> {
    parent
        .attachment_options
        .iter()
        .filter(|option| option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID)
        .map(|option| option.attachment_id.as_str())
        .collect()
}

fn option_name(parent: &TopologyAllowedParent, attachment_id: &str) -> Option<String> {
    parent
        .attachment_options
        .iter()
        .find(|option| option.attachment_id == attachment_id)
        .map(|option| option.attachment_name.clone())
}

fn parent_has_attachment(parent: &TopologyAllowedParent, attachment_id: &str) -> bool {
    parent
        .attachment_options
        .iter()
        .any(|option| option.attachment_id == attachment_id)
}

fn attachment_selectable_for_auto(option: &TopologyAttachmentOption) -> bool {
    option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID
        && option.health_status != TopologyAttachmentHealthStatus::Suppressed
}

const fn attachment_rate_source_preference(source: TopologyAttachmentRateSource) -> u8 {
    match source {
        TopologyAttachmentRateSource::DynamicIntegration => 3,
        TopologyAttachmentRateSource::Manual => 2,
        TopologyAttachmentRateSource::Static => 1,
        TopologyAttachmentRateSource::Unknown => 0,
    }
}

const fn attachment_health_preference(status: TopologyAttachmentHealthStatus) -> u8 {
    match status {
        TopologyAttachmentHealthStatus::Healthy => 3,
        TopologyAttachmentHealthStatus::Disabled => 2,
        TopologyAttachmentHealthStatus::ProbeUnavailable => 1,
        TopologyAttachmentHealthStatus::Suppressed => 0,
    }
}

fn ranked_auto_attachment_id(
    parent: &TopologyAllowedParent,
    current_attachment_id: Option<&str>,
) -> Option<String> {
    parent
        .attachment_options
        .iter()
        .filter(|option| attachment_selectable_for_auto(option))
        .max_by_key(|option| {
            (
                attachment_rate_source_preference(option.rate_source),
                attachment_capacity_mbps(option).unwrap_or(0),
                attachment_health_preference(option.health_status),
                option.probeable,
                current_attachment_id == Some(option.attachment_id.as_str()),
            )
        })
        .map(|option| option.attachment_id.clone())
}

fn first_selectable_attachment_id(parent: &TopologyAllowedParent) -> Option<String> {
    parent
        .attachment_options
        .iter()
        .find(|option| attachment_selectable_for_auto(option))
        .map(|option| option.attachment_id.clone())
}

fn first_explicit_attachment_id(parent: &TopologyAllowedParent) -> Option<String> {
    parent
        .attachment_options
        .iter()
        .find(|option| option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID)
        .map(|option| option.attachment_id.clone())
}

fn current_parent_for_node<'a>(
    node: &'a TopologyEditorNode,
    parent_id: &str,
) -> Option<&'a TopologyAllowedParent> {
    node.allowed_parents
        .iter()
        .find(|parent| parent.parent_node_id == parent_id)
}

fn runtime_may_infer_parent_from_candidates(source: &str) -> bool {
    !(source.starts_with("uisp/") || source.starts_with("python/"))
}

fn merge_attachment_option(
    existing: &mut TopologyAttachmentOption,
    incoming: &TopologyAttachmentOption,
) {
    if existing.attachment_name.is_empty() && !incoming.attachment_name.is_empty() {
        existing.attachment_name = incoming.attachment_name.clone();
    }
    if existing.attachment_kind.is_empty() && !incoming.attachment_kind.is_empty() {
        existing.attachment_kind = incoming.attachment_kind.clone();
    }
    if existing.attachment_role == TopologyAttachmentRole::Unknown
        && incoming.attachment_role != TopologyAttachmentRole::Unknown
    {
        existing.attachment_role = incoming.attachment_role;
    }
    if existing.pair_id.is_none() {
        existing.pair_id = incoming.pair_id.clone();
    }
    if existing.peer_attachment_id.is_none() {
        existing.peer_attachment_id = incoming.peer_attachment_id.clone();
    }
    if existing.peer_attachment_name.is_none() {
        existing.peer_attachment_name = incoming.peer_attachment_name.clone();
    }
    if existing.capacity_mbps.is_none() {
        existing.capacity_mbps = incoming.capacity_mbps;
    }
    if existing.download_bandwidth_mbps.is_none() {
        existing.download_bandwidth_mbps = incoming.download_bandwidth_mbps;
    }
    if existing.upload_bandwidth_mbps.is_none() {
        existing.upload_bandwidth_mbps = incoming.upload_bandwidth_mbps;
    }
    if existing.transport_cap_mbps.is_none() {
        existing.transport_cap_mbps = incoming.transport_cap_mbps;
    }
    if existing.transport_cap_reason.is_none() {
        existing.transport_cap_reason = incoming.transport_cap_reason.clone();
    }
    if existing.rate_source == TopologyAttachmentRateSource::Unknown
        && incoming.rate_source != TopologyAttachmentRateSource::Unknown
    {
        existing.rate_source = incoming.rate_source;
    }
    existing.can_override_rate |= incoming.can_override_rate;
    if existing.rate_override_disabled_reason.is_none() {
        existing.rate_override_disabled_reason = incoming.rate_override_disabled_reason.clone();
    }
    existing.has_rate_override |= incoming.has_rate_override;
    if existing.local_probe_ip.is_none() {
        existing.local_probe_ip = incoming.local_probe_ip.clone();
    }
    if existing.remote_probe_ip.is_none() {
        existing.remote_probe_ip = incoming.remote_probe_ip.clone();
    }
    existing.probe_enabled |= incoming.probe_enabled;
    existing.probeable |= incoming.probeable;
    if existing.health_status == TopologyAttachmentHealthStatus::Healthy
        && incoming.health_status != TopologyAttachmentHealthStatus::Healthy
    {
        existing.health_status = incoming.health_status;
    }
    if existing.health_reason.is_none() {
        existing.health_reason = incoming.health_reason.clone();
    }
    if existing.suppressed_until_unix.is_none() {
        existing.suppressed_until_unix = incoming.suppressed_until_unix;
    }
    existing.effective_selected |= incoming.effective_selected;
}

fn merge_allowed_parent(existing: &mut TopologyAllowedParent, incoming: &TopologyAllowedParent) {
    if existing.parent_node_name.is_empty() && !incoming.parent_node_name.is_empty() {
        existing.parent_node_name = incoming.parent_node_name.clone();
    }
    for option in &incoming.attachment_options {
        if let Some(existing_option) = existing
            .attachment_options
            .iter_mut()
            .find(|current| current.attachment_id == option.attachment_id)
        {
            merge_attachment_option(existing_option, option);
        } else {
            existing.attachment_options.push(option.clone());
        }
    }
    existing.all_attachments_suppressed &= incoming.all_attachments_suppressed;
    existing.has_probe_unavailable_attachments |= incoming.has_probe_unavailable_attachments;
}

fn normalize_topology_editor_state(canonical: &TopologyEditorStateFile) -> TopologyEditorStateFile {
    let mut nodes = Vec::<TopologyEditorNode>::new();
    let mut index_by_id = HashMap::<String, usize>::new();

    for node in &canonical.nodes {
        if let Some(index) = index_by_id.get(&node.node_id).copied() {
            let existing = &mut nodes[index];
            if existing.node_name.is_empty() && !node.node_name.is_empty() {
                existing.node_name = node.node_name.clone();
            }
            if existing.current_parent_node_id.is_none() {
                existing.current_parent_node_id = node.current_parent_node_id.clone();
            }
            if existing.current_parent_node_name.is_none() {
                existing.current_parent_node_name = node.current_parent_node_name.clone();
            }
            if existing.current_attachment_id.is_none() {
                existing.current_attachment_id = node.current_attachment_id.clone();
            }
            if existing.current_attachment_name.is_none() {
                existing.current_attachment_name = node.current_attachment_name.clone();
            }
            existing.can_move |= node.can_move;
            for parent in &node.allowed_parents {
                if let Some(existing_parent) = existing
                    .allowed_parents
                    .iter_mut()
                    .find(|current| current.parent_node_id == parent.parent_node_id)
                {
                    merge_allowed_parent(existing_parent, parent);
                } else {
                    existing.allowed_parents.push(parent.clone());
                }
            }
            if existing.preferred_attachment_id.is_none() {
                existing.preferred_attachment_id = node.preferred_attachment_id.clone();
            }
            if existing.preferred_attachment_name.is_none() {
                existing.preferred_attachment_name = node.preferred_attachment_name.clone();
            }
            if existing.effective_attachment_id.is_none() {
                existing.effective_attachment_id = node.effective_attachment_id.clone();
            }
            if existing.effective_attachment_name.is_none() {
                existing.effective_attachment_name = node.effective_attachment_name.clone();
            }
            continue;
        }

        index_by_id.insert(node.node_id.clone(), nodes.len());
        nodes.push(node.clone());
    }

    for node in &mut nodes {
        if let Some(current_parent_id) = node.current_parent_node_id.as_deref()
            && node
                .allowed_parents
                .iter()
                .all(|parent| parent.parent_node_id != current_parent_id)
            && let Some(fallback_parent) = node.allowed_parents.first()
        {
            node.current_parent_node_id = Some(fallback_parent.parent_node_id.clone());
            node.current_parent_node_name = Some(fallback_parent.parent_node_name.clone());
        }
    }

    TopologyEditorStateFile {
        schema_version: canonical.schema_version,
        source: canonical.source.clone(),
        generated_unix: canonical.generated_unix,
        ingress_identity: canonical.ingress_identity.clone(),
        nodes,
    }
}

fn prepared_runtime_topology_editor_state(
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
) -> TopologyEditorStateFile {
    let normalized = normalize_topology_editor_state(canonical);
    let manual = overlay_manual_groups(&normalized, overrides);
    apply_attachment_rate_overrides(&manual, overrides)
}

fn health_entries_by_pair<'a>(
    config: &Config,
    health: &'a TopologyAttachmentHealthStateFile,
) -> HashMap<&'a str, &'a lqos_config::TopologyAttachmentHealthEntry> {
    if is_health_state_fresh(config, health) {
        health
            .attachments
            .iter()
            .map(|entry| (entry.attachment_pair_id.as_str(), entry))
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    }
}

/// Prepares the runtime editor-state view used for probe planning and effective compilation.
///
/// Side effects: none. This applies normalization, manual attachment overlays, and attachment
/// rate overrides to the canonical topology editor state.
pub fn prepare_runtime_topology_editor_state_from_canonical(
    canonical: &TopologyCanonicalStateFile,
    overrides: &TopologyOverridesFile,
) -> TopologyEditorStateFile {
    prepared_runtime_topology_editor_state(&canonical.to_editor_state(), overrides)
}

/// Computes the effective attachment selection for all nodes using canonical state,
/// operator intent, and transient runtime health.
pub fn compute_effective_state(
    config: &Config,
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> TopologyEffectiveStateFile {
    let prepared = prepared_runtime_topology_editor_state(canonical, overrides);
    compute_effective_state_from_prepared(config, &prepared, overrides, health)
}

fn compute_effective_state_from_prepared(
    config: &Config,
    prepared: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> TopologyEffectiveStateFile {
    let health_by_pair = health_entries_by_pair(config, health);
    let may_infer_parent = runtime_may_infer_parent_from_candidates(&prepared.source);

    let mut nodes = Vec::with_capacity(prepared.nodes.len());
    for node in &prepared.nodes {
        let selected_parent_id = overrides
            .find_override(&node.node_id)
            .and_then(|saved| {
                current_parent_for_node(node, &saved.parent_node_id)
                    .map(|parent| parent.parent_node_id.clone())
            })
            .or_else(|| node.current_parent_node_id.clone())
            .or_else(|| {
                may_infer_parent
                    .then(|| {
                        node.allowed_parents
                            .first()
                            .map(|parent| parent.parent_node_id.clone())
                    })
                    .flatten()
            });

        let Some(selected_parent_id) = selected_parent_id else {
            nodes.push(TopologyEffectiveNodeState {
                node_id: node.node_id.clone(),
                logical_parent_node_id: String::new(),
                preferred_attachment_id: None,
                effective_attachment_id: None,
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: Vec::new(),
            });
            continue;
        };

        let Some(selected_parent) =
            current_parent_for_node(node, &selected_parent_id).or_else(|| {
                may_infer_parent
                    .then(|| node.allowed_parents.first())
                    .flatten()
            })
        else {
            let fixed_attachment_id = node
                .current_attachment_id
                .clone()
                .filter(|attachment_id| !attachment_id.is_empty());
            nodes.push(TopologyEffectiveNodeState {
                node_id: node.node_id.clone(),
                logical_parent_node_id: selected_parent_id,
                preferred_attachment_id: fixed_attachment_id.clone(),
                effective_attachment_id: fixed_attachment_id,
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: Vec::new(),
            });
            continue;
        };
        let selected_parent_id = selected_parent.parent_node_id.clone();
        let enriched_parent = enrich_allowed_parent(
            selected_parent,
            overrides,
            &health_by_pair,
            None,
            Some(&selected_parent_id),
        );

        let explicit_options = enriched_parent
            .attachment_options
            .iter()
            .filter(|option| option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID)
            .cloned()
            .collect::<Vec<_>>();

        let override_entry = overrides.find_override(&node.node_id);
        let preferred_attachment_id = match override_entry {
            Some(saved)
                if saved.parent_node_id == selected_parent_id
                    && saved.mode == TopologyAttachmentMode::PreferredOrder =>
            {
                let valid_ids = valid_attachment_ids(&enriched_parent);
                saved
                    .attachment_preference_ids
                    .iter()
                    .find(|attachment_id| valid_ids.contains(attachment_id.as_str()))
                    .cloned()
                    .or_else(|| {
                        node.current_attachment_id.clone().filter(|attachment_id| {
                            parent_has_attachment(&enriched_parent, attachment_id)
                        })
                    })
            }
            _ => ranked_auto_attachment_id(&enriched_parent, node.current_attachment_id.as_deref())
                .or_else(|| {
                    node.current_attachment_id.clone().filter(|attachment_id| {
                        parent_has_attachment(&enriched_parent, attachment_id)
                    })
                }),
        };

        let selectable_ids = explicit_options
            .iter()
            .filter(|option| attachment_selectable_for_auto(option))
            .map(|option| option.attachment_id.clone())
            .collect::<HashSet<_>>();

        let mut fallback_reason = None;
        let effective_attachment_id = if explicit_options.is_empty() {
            None
        } else if !selectable_ids.is_empty() {
            match override_entry {
                Some(saved)
                    if saved.parent_node_id == selected_parent_id
                        && saved.mode == TopologyAttachmentMode::PreferredOrder =>
                {
                    saved
                        .attachment_preference_ids
                        .iter()
                        .find(|attachment_id| selectable_ids.contains(*attachment_id))
                        .cloned()
                        .or_else(|| {
                            node.current_attachment_id
                                .clone()
                                .filter(|attachment_id| selectable_ids.contains(attachment_id))
                        })
                        .or_else(|| {
                            ranked_auto_attachment_id(
                                &enriched_parent,
                                node.current_attachment_id.as_deref(),
                            )
                        })
                        .or_else(|| first_selectable_attachment_id(&enriched_parent))
                }
                _ => ranked_auto_attachment_id(
                    &enriched_parent,
                    node.current_attachment_id.as_deref(),
                )
                .or_else(|| first_selectable_attachment_id(&enriched_parent)),
            }
        } else {
            fallback_reason = Some(if enriched_parent.all_attachments_suppressed {
                "All attachments suppressed; using deterministic fallback".to_string()
            } else {
                "No healthy attachment available; using deterministic fallback".to_string()
            });
            node.current_attachment_id
                .clone()
                .filter(|attachment_id| parent_has_attachment(&enriched_parent, attachment_id))
                .or_else(|| first_explicit_attachment_id(&enriched_parent))
        };

        let attachments = explicit_options
            .iter()
            .map(|option| TopologyEffectiveAttachmentState {
                attachment_id: option.attachment_id.clone(),
                health_status: option.health_status,
                health_reason: option.health_reason.clone(),
                suppressed_until_unix: option.suppressed_until_unix,
                probe_enabled: option.probe_enabled,
                probeable: option.probeable,
                effective_selected: effective_attachment_id
                    .as_deref()
                    .is_some_and(|id| id == option.attachment_id),
            })
            .collect::<Vec<_>>();

        nodes.push(TopologyEffectiveNodeState {
            node_id: node.node_id.clone(),
            logical_parent_node_id: selected_parent_id.clone(),
            preferred_attachment_id,
            effective_attachment_id,
            fallback_reason,
            all_attachments_suppressed: enriched_parent.all_attachments_suppressed,
            attachments,
        });
    }

    TopologyEffectiveStateFile {
        schema_version: 1,
        generated_unix: now_unix(),
        canonical_generated_unix: prepared.generated_unix,
        health_generated_unix: health.generated_unix,
        nodes,
    }
}

/// Builds a UI-facing topology editor state with manual groups, probe enablement,
/// health annotations, and effective attachment selection applied.
pub fn merged_topology_state(
    config: &Config,
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
    effective: &TopologyEffectiveStateFile,
) -> TopologyEditorStateFile {
    let prepared = prepared_runtime_topology_editor_state(canonical, overrides);
    merged_topology_state_from_prepared(config, &prepared, overrides, health, effective)
}

fn merged_topology_state_from_prepared(
    config: &Config,
    prepared: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
    effective: &TopologyEffectiveStateFile,
) -> TopologyEditorStateFile {
    let mut state = prepared.clone();
    let health_by_pair = health_entries_by_pair(config, health);
    let effective_by_node = effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();

    for node in &mut state.nodes {
        let effective_node = effective_by_node.get(node.node_id.as_str()).copied();
        let effective_parent_id = effective_node.map(|entry| entry.logical_parent_node_id.as_str());
        let effective_attachment_id =
            effective_node.and_then(|entry| entry.effective_attachment_id.as_deref());
        let preferred_attachment_id =
            effective_node.and_then(|entry| entry.preferred_attachment_id.as_deref());

        node.allowed_parents = node
            .allowed_parents
            .iter()
            .map(|parent| {
                enrich_allowed_parent(
                    parent,
                    overrides,
                    &health_by_pair,
                    effective_attachment_id,
                    effective_parent_id,
                )
            })
            .collect();
        node.preferred_attachment_id = preferred_attachment_id.map(ToString::to_string);
        node.preferred_attachment_name = preferred_attachment_id.and_then(|attachment_id| {
            node.allowed_parents
                .iter()
                .find_map(|parent| option_name(parent, attachment_id))
        });
        node.effective_attachment_id = effective_attachment_id.map(ToString::to_string);
        node.effective_attachment_name = effective_attachment_id.and_then(|attachment_id| {
            node.allowed_parents
                .iter()
                .find_map(|parent| option_name(parent, attachment_id))
        });
    }

    state
}

/// Emits the unique set of enabled/known probe specs from the UI-facing topology state.
pub fn probe_specs_from_state(
    state: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
) -> Vec<AttachmentProbeSpec> {
    let mut seen = HashSet::new();
    let mut specs = Vec::new();
    for node in &state.nodes {
        for parent in &node.allowed_parents {
            for option in &parent.attachment_options {
                if option.attachment_id == TOPOLOGY_ATTACHMENT_AUTO_ID {
                    continue;
                }
                let Some(pair_id) = option.pair_id.clone() else {
                    continue;
                };
                let Some(local_ip) = option.local_probe_ip.clone() else {
                    continue;
                };
                let Some(remote_ip) = option.remote_probe_ip.clone() else {
                    continue;
                };
                if !seen.insert(pair_id.clone()) {
                    continue;
                }
                specs.push(AttachmentProbeSpec {
                    pair_id: pair_id.clone(),
                    attachment_id: option.attachment_id.clone(),
                    attachment_name: option.attachment_name.clone(),
                    node_id: node.node_id.clone(),
                    node_name: node.node_name.clone(),
                    parent_node_id: parent.parent_node_id.clone(),
                    parent_node_name: parent.parent_node_name.clone(),
                    local_ip,
                    remote_ip,
                    enabled: probe_enabled_for_option(option, overrides),
                });
            }
        }
    }
    specs.sort_unstable_by(|left, right| left.pair_id.cmp(&right.pair_id));
    specs
}

fn remove_node_by_id(map: &mut Map<String, Value>, target_id: &str) -> Option<(String, Value)> {
    let keys = map.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let Some(value) = map.get_mut(&key) else {
            continue;
        };
        let Some(node) = value.as_object_mut() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == target_id)
        {
            let removed = map.remove(&key)?;
            return Some((key, removed));
        }
        if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut)
            && let Some(found) = remove_node_by_id(children, target_id)
        {
            return Some(found);
        }
    }
    None
}

fn logical_child_branch_counts(ui_state: &TopologyEditorStateFile) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for node in &ui_state.nodes {
        let Some(parent_id) = node.current_parent_node_id.as_deref() else {
            continue;
        };
        *counts.entry(parent_id.to_string()).or_insert(0) += 1;
    }
    counts
}

fn read_node_rate_mbps(node: &Map<String, Value>, key: &str) -> Option<u64> {
    node.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_f64().map(|rate| rate as u64))
    })
}

fn node_capacity_mbps(node: &Map<String, Value>) -> u64 {
    let download = read_node_rate_mbps(node, "downloadBandwidthMbps").unwrap_or_default();
    let upload = read_node_rate_mbps(node, "uploadBandwidthMbps").unwrap_or_default();
    download.max(upload)
}

fn resolved_queue_visibility_policy(
    config: &Config,
    ui_node: &TopologyEditorNode,
    tree_node: Option<&Value>,
    child_branch_counts: &HashMap<String, usize>,
) -> TopologyQueueVisibilityPolicy {
    match ui_node.queue_visibility_policy {
        TopologyQueueVisibilityPolicy::QueueVisible => TopologyQueueVisibilityPolicy::QueueVisible,
        TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren => {
            TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren
        }
        TopologyQueueVisibilityPolicy::QueueAuto => {
            if ui_node.current_parent_node_id.is_none() {
                return TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren;
            }
            let Some(tree_node) = tree_node.and_then(Value::as_object) else {
                return TopologyQueueVisibilityPolicy::QueueVisible;
            };
            let is_site = tree_node
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("site"));
            if !is_site {
                return TopologyQueueVisibilityPolicy::QueueVisible;
            }
            if child_branch_counts
                .get(ui_node.node_id.as_str())
                .copied()
                .unwrap_or_default()
                == 0
            {
                return TopologyQueueVisibilityPolicy::QueueVisible;
            }
            let threshold = config.topology.queue_auto_virtualize_threshold_mbps;
            if threshold == 0 {
                return TopologyQueueVisibilityPolicy::QueueVisible;
            }
            if node_capacity_mbps(tree_node) >= threshold {
                TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren
            } else {
                TopologyQueueVisibilityPolicy::QueueVisible
            }
        }
    }
}

fn apply_effective_topology_reparenting_only(
    canonical_network: &Value,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> Value {
    let Some(root) = canonical_network.as_object() else {
        return canonical_network.clone();
    };
    let mut out = root.clone();
    let ui_by_node = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();

    for effective_node in &effective.nodes {
        let Some(ui_node) = ui_by_node.get(effective_node.node_id.as_str()).copied() else {
            continue;
        };
        let Some(selected_parent) = ui_node
            .allowed_parents
            .iter()
            .find(|parent| parent.parent_node_id == effective_node.logical_parent_node_id)
        else {
            continue;
        };
        let already_parented = find_parent_id_of_node(&out, &ui_node.node_id, None)
            .flatten()
            .as_deref()
            == Some(selected_parent.parent_node_id.as_str());
        let Some(effective_attachment_id) = effective_node.effective_attachment_id.as_deref()
        else {
            if ui_node.current_parent_node_id.as_deref()
                == Some(effective_node.logical_parent_node_id.as_str())
                && already_parented
            {
                continue;
            }
            let Some((node_key, node_value)) = remove_node_by_id(&mut out, &ui_node.node_id) else {
                continue;
            };
            let _ = insert_node_under_parent_id(
                &mut out,
                &selected_parent.parent_node_id,
                &node_key,
                node_value,
            );
            continue;
        };
        let Some(target_attachment) = selected_parent
            .attachment_options
            .iter()
            .find(|option| option.attachment_id == effective_attachment_id)
        else {
            continue;
        };
        let current_anchor_attachment = find_node_by_id(&out, &ui_node.node_id)
            .map(|node_value| attachment_anchor_for_reparent(node_value, target_attachment))
            .unwrap_or_else(|| target_attachment.clone());
        let already_anchored = already_parented
            && current_anchor_attachment.attachment_id == selected_parent.parent_node_id
            || find_parent_id_of_node(&out, &ui_node.node_id, None)
                .flatten()
                .as_deref()
                == Some(current_anchor_attachment.attachment_id.as_str());
        if ui_node.current_parent_node_id.as_deref()
            == Some(effective_node.logical_parent_node_id.as_str())
            && ui_node.current_attachment_id.as_deref()
                == effective_node.effective_attachment_id.as_deref()
            && already_anchored
        {
            ensure_attachment_node_exists(
                &mut out,
                &selected_parent.parent_node_id,
                &current_anchor_attachment,
            );
            continue;
        }

        let Some((node_key, node_value)) = remove_node_by_id(&mut out, &ui_node.node_id) else {
            continue;
        };
        let anchor_attachment = attachment_anchor_for_reparent(&node_value, target_attachment);
        ensure_attachment_node_exists(
            &mut out,
            &selected_parent.parent_node_id,
            &anchor_attachment,
        );
        let _ = insert_node_under_parent_id(
            &mut out,
            &anchor_attachment.attachment_id,
            &node_key,
            node_value,
        );
    }

    Value::Object(out)
}

fn queue_policy_reference_tree(
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> Value {
    let canonical_network =
        if canonical.ingress_kind == TopologyCanonicalIngressKind::NativeIntegration {
            canonical.insight_topology_network_json()
        } else {
            canonical.compatibility_network_json().clone()
        };
    let mut logical_tree =
        apply_effective_topology_reparenting_only(&canonical_network, ui_state, effective);
    if let Some(root) = logical_tree.as_object_mut() {
        recompile_effective_network_bandwidths(root, canonical, ui_state, effective);
    }
    logical_tree
}

fn queue_hidden_node_ids_in_promotion_order(ui_state: &TopologyEditorStateFile) -> Vec<String> {
    let by_id = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut depth_cache = HashMap::<String, usize>::new();

    fn node_depth<'a>(
        node_id: &'a str,
        by_id: &HashMap<&'a str, &'a TopologyEditorNode>,
        cache: &mut HashMap<String, usize>,
        seen: &mut HashSet<String>,
    ) -> usize {
        if let Some(depth) = cache.get(node_id).copied() {
            return depth;
        }
        if !seen.insert(node_id.to_string()) {
            return 0;
        }
        let depth = by_id
            .get(node_id)
            .and_then(|node| node.current_parent_node_id.as_deref())
            .map(|parent_id| 1 + node_depth(parent_id, by_id, cache, seen))
            .unwrap_or(0);
        seen.remove(node_id);
        cache.insert(node_id.to_string(), depth);
        depth
    }

    let mut node_ids = ui_state
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>();
    node_ids.sort_unstable_by(|left, right| {
        let left_depth = node_depth(left, &by_id, &mut depth_cache, &mut HashSet::new());
        let right_depth = node_depth(right, &by_id, &mut depth_cache, &mut HashSet::new());
        left_depth.cmp(&right_depth).then_with(|| left.cmp(right))
    });
    node_ids
}

fn mark_node_virtual_by_id(map: &mut Map<String, Value>, target_id: &str) -> bool {
    for value in map.values_mut() {
        let Some(node) = value.as_object_mut() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == target_id)
        {
            node.insert("virtual".to_string(), Value::Bool(true));
            return true;
        }
        if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut)
            && mark_node_virtual_by_id(children, target_id)
        {
            return true;
        }
    }
    false
}

fn apply_queue_hidden_node_virtualization(
    config: &Config,
    ui_state: &TopologyEditorStateFile,
    root: &mut Map<String, Value>,
) {
    let ui_by_id = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let child_branch_counts = logical_child_branch_counts(ui_state);
    let hidden_node_ids = queue_hidden_node_ids_in_promotion_order(ui_state);
    for hidden_node_id in hidden_node_ids {
        let Some(ui_node) = ui_by_id.get(hidden_node_id.as_str()).copied() else {
            continue;
        };
        let resolved_policy = resolved_queue_visibility_policy(
            config,
            ui_node,
            find_node_by_id(root, &hidden_node_id),
            &child_branch_counts,
        );
        if resolved_policy != TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren {
            continue;
        }
        let _ = mark_node_virtual_by_id(root, &hidden_node_id);
    }
}

fn find_node_by_id<'a>(map: &'a Map<String, Value>, target_id: &str) -> Option<&'a Value> {
    for value in map.values() {
        let Some(node) = value.as_object() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == target_id)
        {
            return Some(value);
        }
        if let Some(children) = node.get("children").and_then(Value::as_object)
            && let Some(found) = find_node_by_id(children, target_id)
        {
            return Some(found);
        }
    }
    None
}

fn find_parent_id_of_node(
    map: &Map<String, Value>,
    target_id: &str,
    current_parent_id: Option<&str>,
) -> Option<Option<String>> {
    for value in map.values() {
        let Some(node) = value.as_object() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == target_id)
        {
            return Some(current_parent_id.map(ToOwned::to_owned));
        }
        if let Some(children) = node.get("children").and_then(Value::as_object)
            && let Some(found) =
                find_parent_id_of_node(children, target_id, node.get("id").and_then(Value::as_str))
        {
            return Some(found);
        }
    }
    None
}

fn value_subtree_contains_id(value: &Value, target_id: &str) -> bool {
    let Some(node) = value.as_object() else {
        return false;
    };
    if node
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| id == target_id)
    {
        return true;
    }
    node.get("children")
        .and_then(Value::as_object)
        .is_some_and(|children| {
            children
                .values()
                .any(|child| value_subtree_contains_id(child, target_id))
        })
}

fn insert_node_under_parent_id(
    map: &mut Map<String, Value>,
    parent_id: &str,
    node_key: &str,
    node_value: Value,
) -> bool {
    for (key, value) in map.iter_mut() {
        let Some(node) = value.as_object_mut() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == parent_id)
        {
            let children = node
                .entry("children".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(children) = children.as_object_mut() else {
                return false;
            };
            let mut node_value = node_value;
            if let Some(node_object) = node_value.as_object_mut() {
                node_object.insert("parent_site".to_string(), Value::String(key.clone()));
                node_object
                    .entry("name".to_string())
                    .or_insert_with(|| Value::String(node_key.to_string()));
            }
            children.insert(node_key.to_string(), node_value);
            return true;
        }
        if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut)
            && insert_node_under_parent_id(children, parent_id, node_key, node_value.clone())
        {
            return true;
        }
    }
    false
}

fn ensure_attachment_node_exists(
    root: &mut Map<String, Value>,
    parent_id: &str,
    attachment: &TopologyAttachmentOption,
) {
    if update_node_bandwidths_by_id(root, &attachment.attachment_id, attachment) {
        return;
    }
    let download = attachment
        .download_bandwidth_mbps
        .or(attachment.capacity_mbps)
        .unwrap_or(0);
    let upload = attachment
        .upload_bandwidth_mbps
        .or(attachment.capacity_mbps)
        .unwrap_or(0);
    let mut node = Map::new();
    node.insert("children".to_string(), Value::Object(Map::new()));
    node.insert(
        "downloadBandwidthMbps".to_string(),
        Value::Number(download.into()),
    );
    node.insert(
        "uploadBandwidthMbps".to_string(),
        Value::Number(upload.into()),
    );
    node.insert(
        "id".to_string(),
        Value::String(attachment.attachment_id.clone()),
    );
    node.insert(
        "name".to_string(),
        Value::String(attachment.attachment_name.clone()),
    );
    node.insert("type".to_string(), Value::String("AP".to_string()));
    let _ = insert_node_under_parent_id(
        root,
        parent_id,
        &attachment.attachment_name,
        Value::Object(node),
    );
}

fn attachment_anchor_for_reparent(
    moved_subtree: &Value,
    attachment: &TopologyAttachmentOption,
) -> TopologyAttachmentOption {
    let Some(peer_attachment_id) = attachment.peer_attachment_id.as_ref() else {
        return attachment.clone();
    };
    if !value_subtree_contains_id(moved_subtree, &attachment.attachment_id) {
        return attachment.clone();
    }

    let mut anchor = attachment.clone();
    anchor.attachment_id = peer_attachment_id.clone();
    anchor.attachment_name = attachment
        .peer_attachment_name
        .clone()
        .unwrap_or_else(|| peer_attachment_id.clone());
    anchor.peer_attachment_id = Some(attachment.attachment_id.clone());
    anchor.peer_attachment_name = Some(attachment.attachment_name.clone());
    anchor
}

fn update_node_bandwidths_by_id(
    root: &mut Map<String, Value>,
    node_id: &str,
    attachment: &TopologyAttachmentOption,
) -> bool {
    for value in root.values_mut() {
        let Some(node) = value.as_object_mut() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == node_id)
        {
            let download = attachment
                .download_bandwidth_mbps
                .or(attachment.capacity_mbps)
                .unwrap_or(0);
            let upload = attachment
                .upload_bandwidth_mbps
                .or(attachment.capacity_mbps)
                .unwrap_or(0);
            node.insert(
                "downloadBandwidthMbps".to_string(),
                Value::Number(download.into()),
            );
            node.insert(
                "uploadBandwidthMbps".to_string(),
                Value::Number(upload.into()),
            );
            node.insert(
                "name".to_string(),
                Value::String(attachment.attachment_name.clone()),
            );
            return true;
        }
        if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut)
            && update_node_bandwidths_by_id(children, node_id, attachment)
        {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, Debug, Default)]
struct CompiledRatePair {
    download: Option<u64>,
    upload: Option<u64>,
}

fn compiled_rate_pair(download: Option<u64>, upload: Option<u64>) -> CompiledRatePair {
    CompiledRatePair { download, upload }
}

fn rate_pair_from_attachment(attachment: &TopologyAttachmentOption) -> CompiledRatePair {
    compiled_rate_pair(
        attachment
            .download_bandwidth_mbps
            .or(attachment.capacity_mbps),
        attachment
            .upload_bandwidth_mbps
            .or(attachment.capacity_mbps),
    )
}

fn rate_pair_from_value(node: &Map<String, Value>) -> CompiledRatePair {
    compiled_rate_pair(
        node.get("downloadBandwidthMbps")
            .and_then(|value| match value {
                Value::Number(number) => number
                    .as_u64()
                    .or_else(|| number.as_f64().map(|value| value.round() as u64)),
                _ => None,
            }),
        node.get("uploadBandwidthMbps")
            .and_then(|value| match value {
                Value::Number(number) => number
                    .as_u64()
                    .or_else(|| number.as_f64().map(|value| value.round() as u64)),
                _ => None,
            }),
    )
}

fn rate_pair_from_canonical_node(node: &TopologyCanonicalNode) -> CompiledRatePair {
    compiled_rate_pair(
        node.rate_input
            .intrinsic_download_mbps
            .or(node.rate_input.legacy_imported_download_mbps),
        node.rate_input
            .intrinsic_upload_mbps
            .or(node.rate_input.legacy_imported_upload_mbps),
    )
}

fn intersect_rate_pairs(base: CompiledRatePair, limit: CompiledRatePair) -> CompiledRatePair {
    let download = match (base.download, limit.download) {
        (Some(base), Some(limit)) => Some(base.min(limit)),
        (Some(base), None) => Some(base),
        (None, Some(limit)) => Some(limit),
        (None, None) => None,
    };
    let upload = match (base.upload, limit.upload) {
        (Some(base), Some(limit)) => Some(base.min(limit)),
        (Some(base), None) => Some(base),
        (None, Some(limit)) => Some(limit),
        (None, None) => None,
    };
    compiled_rate_pair(download, upload)
}

fn write_rate_pair(node: &mut Map<String, Value>, rates: CompiledRatePair) {
    if let Some(download) = rates.download {
        node.insert(
            "downloadBandwidthMbps".to_string(),
            Value::Number(download.into()),
        );
    }
    if let Some(upload) = rates.upload {
        node.insert(
            "uploadBandwidthMbps".to_string(),
            Value::Number(upload.into()),
        );
    }
}

fn selected_attachment_rate_caps(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> HashMap<String, CompiledRatePair> {
    let ui_nodes = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut caps = HashMap::new();
    for node in &effective.nodes {
        let Some(selected_attachment_id) = node.effective_attachment_id.as_deref() else {
            continue;
        };
        let Some(ui_node) = ui_nodes.get(node.node_id.as_str()).copied() else {
            continue;
        };
        let Some(parent) = ui_node
            .allowed_parents
            .iter()
            .find(|entry| entry.parent_node_id == node.logical_parent_node_id)
        else {
            continue;
        };
        let Some(attachment) = parent
            .attachment_options
            .iter()
            .find(|attachment| attachment.attachment_id == selected_attachment_id)
        else {
            continue;
        };
        caps.insert(node.node_id.clone(), rate_pair_from_attachment(attachment));
    }
    caps
}

fn recompile_effective_bandwidths_for_value(
    value: &mut Value,
    canonical_nodes: &HashMap<&str, &TopologyCanonicalNode>,
    selected_attachment_caps: &HashMap<String, CompiledRatePair>,
    inherited_parent_rates: Option<CompiledRatePair>,
) {
    let Some(node) = value.as_object_mut() else {
        return;
    };
    let existing_rates = rate_pair_from_value(node);
    let node_id = node.get("id").and_then(Value::as_str);
    let mut compiled = node_id
        .and_then(|node_id| canonical_nodes.get(node_id).copied())
        .map(rate_pair_from_canonical_node)
        .unwrap_or(existing_rates);
    if let Some(node_id) = node_id
        && let Some(attachment_rates) = selected_attachment_caps.get(node_id)
    {
        compiled = intersect_rate_pairs(compiled, *attachment_rates);
    }
    if let Some(parent_rates) = inherited_parent_rates {
        compiled = intersect_rate_pairs(compiled, parent_rates);
    }
    if compiled.download.is_none() {
        compiled.download = existing_rates
            .download
            .or(inherited_parent_rates.and_then(|pair| pair.download));
    }
    if compiled.upload.is_none() {
        compiled.upload = existing_rates
            .upload
            .or(inherited_parent_rates.and_then(|pair| pair.upload));
    }
    write_rate_pair(node, compiled);
    let next_parent_rates = Some(rate_pair_from_value(node));
    if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut) {
        for child in children.values_mut() {
            recompile_effective_bandwidths_for_value(
                child,
                canonical_nodes,
                selected_attachment_caps,
                next_parent_rates,
            );
        }
    }
}

fn recompile_effective_network_bandwidths(
    root: &mut Map<String, Value>,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) {
    let canonical_nodes = canonical
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let selected_attachment_caps = selected_attachment_rate_caps(ui_state, effective);
    for node in root.values_mut() {
        recompile_effective_bandwidths_for_value(
            node,
            &canonical_nodes,
            &selected_attachment_caps,
            None,
        );
    }
}

fn node_type_is(value: &Value, expected: &str) -> bool {
    value
        .as_object()
        .and_then(|node| node.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|node_type| node_type == expected)
}

fn node_bandwidth_mbps(node: &Map<String, Value>, field: &str) -> Option<u64> {
    node.get(field).and_then(Value::as_u64).or_else(|| {
        node.get(field)
            .and_then(Value::as_f64)
            .map(|value| value as u64)
    })
}

fn min_chain_bandwidth(
    endpoint: &Map<String, Value>,
    relay_a: &Map<String, Value>,
    relay_b: &Map<String, Value>,
    field: &str,
) -> Option<u64> {
    [
        node_bandwidth_mbps(endpoint, field),
        node_bandwidth_mbps(relay_a, field),
        node_bandwidth_mbps(relay_b, field),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn min_attachment_bandwidth(
    endpoint: &Map<String, Value>,
    attachment: &Map<String, Value>,
    field: &str,
) -> Option<u64> {
    [
        node_bandwidth_mbps(endpoint, field),
        node_bandwidth_mbps(attachment, field),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn should_runtime_squash_chain(
    chain_names: [&str; 4],
    do_not_squash_sites: &HashSet<String>,
) -> bool {
    !chain_names
        .into_iter()
        .any(|name| do_not_squash_sites.contains(name))
}

fn attachment_role_allows_runtime_squash(role: TopologyAttachmentRole) -> bool {
    matches!(
        role,
        TopologyAttachmentRole::PtpBackhaul | TopologyAttachmentRole::WiredUplink
    )
}

fn find_attachment_option_for_node<'a>(
    node: &'a TopologyEditorNode,
    parent_node_id: Option<&str>,
    attachment_id: Option<&str>,
) -> Option<&'a TopologyAttachmentOption> {
    let attachment_id = attachment_id?;
    node.allowed_parents
        .iter()
        .filter(|parent| {
            parent_node_id.is_none_or(|expected_parent| parent.parent_node_id == expected_parent)
        })
        .flat_map(|parent| parent.attachment_options.iter())
        .find(|option| option.attachment_id == attachment_id)
}

fn find_attachment_role_for_node(
    node: &TopologyEditorNode,
    parent_node_id: Option<&str>,
    attachment_id: Option<&str>,
) -> Option<TopologyAttachmentRole> {
    find_attachment_option_for_node(node, parent_node_id, attachment_id)
        .map(|option| option.attachment_role)
}

fn selected_attachment_roles(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> HashMap<String, TopologyAttachmentRole> {
    let mut roles_by_node_id = HashMap::new();
    let ui_by_node = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();

    for node in &ui_state.nodes {
        if let Some(role) = find_attachment_role_for_node(
            node,
            node.current_parent_node_id.as_deref(),
            node.current_attachment_id.as_deref(),
        ) {
            roles_by_node_id.insert(node.node_id.clone(), role);
        }
    }

    for effective_node in &effective.nodes {
        let Some(ui_node) = ui_by_node.get(effective_node.node_id.as_str()).copied() else {
            continue;
        };
        let Some(role) = find_attachment_role_for_node(
            ui_node,
            Some(effective_node.logical_parent_node_id.as_str()),
            effective_node.effective_attachment_id.as_deref(),
        ) else {
            continue;
        };
        roles_by_node_id.insert(effective_node.node_id.clone(), role);
    }

    roles_by_node_id
}

fn selected_attachment_pair_ids(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> HashSet<String> {
    let mut active_pair_ids = HashSet::new();
    let effective_node_ids = effective
        .nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<HashSet<_>>();
    let ui_by_node = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();

    for node in &ui_state.nodes {
        if effective_node_ids.contains(node.node_id.as_str()) {
            continue;
        }
        let Some(option) = find_attachment_option_for_node(
            node,
            node.current_parent_node_id.as_deref(),
            node.current_attachment_id.as_deref(),
        ) else {
            continue;
        };
        let Some(pair_id) = option.pair_id.as_ref() else {
            continue;
        };
        active_pair_ids.insert(pair_id.clone());
    }

    for effective_node in &effective.nodes {
        let Some(ui_node) = ui_by_node.get(effective_node.node_id.as_str()).copied() else {
            continue;
        };
        let Some(option) = find_attachment_option_for_node(
            ui_node,
            Some(effective_node.logical_parent_node_id.as_str()),
            effective_node.effective_attachment_id.as_deref(),
        ) else {
            continue;
        };
        let Some(pair_id) = option.pair_id.as_ref() else {
            continue;
        };
        active_pair_ids.insert(pair_id.clone());
    }

    active_pair_ids
}

fn attachment_pair_memberships(
    ui_state: &TopologyEditorStateFile,
) -> HashMap<String, (TopologyAttachmentRole, String)> {
    let mut pair_by_attachment_id = HashMap::new();
    for node in &ui_state.nodes {
        for parent in &node.allowed_parents {
            for option in &parent.attachment_options {
                let Some(pair_id) = option.pair_id.as_ref() else {
                    continue;
                };
                if !attachment_role_allows_runtime_squash(option.attachment_role) {
                    continue;
                }
                pair_by_attachment_id.insert(
                    option.attachment_id.clone(),
                    (option.attachment_role, pair_id.clone()),
                );
                if let Some(peer_attachment_id) = option.peer_attachment_id.as_ref() {
                    pair_by_attachment_id.insert(
                        peer_attachment_id.clone(),
                        (option.attachment_role, pair_id.clone()),
                    );
                }
            }
        }
    }
    pair_by_attachment_id
}

fn endpoint_attachment_role(
    endpoint_node: &Map<String, Value>,
    roles_by_node_id: &HashMap<String, TopologyAttachmentRole>,
) -> TopologyAttachmentRole {
    endpoint_node
        .get("id")
        .and_then(Value::as_str)
        .and_then(|node_id| roles_by_node_id.get(node_id).copied())
        .unwrap_or_default()
}

fn is_inactive_backhaul_stub_subtree(
    node: &Map<String, Value>,
    pair_by_attachment_id: &HashMap<String, (TopologyAttachmentRole, String)>,
    active_pair_ids: &HashSet<String>,
) -> bool {
    let Some(node_id) = node.get("id").and_then(Value::as_str) else {
        return false;
    };
    let Some((role, pair_id)) = pair_by_attachment_id.get(node_id) else {
        return false;
    };
    if !attachment_role_allows_runtime_squash(*role) || active_pair_ids.contains(pair_id) {
        return false;
    }
    let Some(children) = node.get("children").and_then(Value::as_object) else {
        return true;
    };
    children.values().all(|child| {
        let Some(child_node) = child.as_object() else {
            return false;
        };
        node_type_is(child, "AP")
            && is_inactive_backhaul_stub_subtree(child_node, pair_by_attachment_id, active_pair_ids)
    })
}

fn prune_inactive_backhaul_stubs_in_children(
    children: &mut Map<String, Value>,
    pair_by_attachment_id: &HashMap<String, (TopologyAttachmentRole, String)>,
    active_pair_ids: &HashSet<String>,
) {
    let child_keys = children.keys().cloned().collect::<Vec<_>>();
    for child_key in child_keys {
        let Some(node) = children.get_mut(&child_key).and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(grandchildren) = node.get_mut("children").and_then(Value::as_object_mut) else {
            continue;
        };
        prune_inactive_backhaul_stubs_in_children(
            grandchildren,
            pair_by_attachment_id,
            active_pair_ids,
        );
    }

    let child_keys = children.keys().cloned().collect::<Vec<_>>();
    for child_key in child_keys {
        let should_remove = children
            .get(&child_key)
            .and_then(Value::as_object)
            .is_some_and(|node| {
                is_inactive_backhaul_stub_subtree(node, pair_by_attachment_id, active_pair_ids)
            });
        if should_remove {
            children.remove(&child_key);
        }
    }
}

fn squash_backhaul_pairs_in_children(
    parent_name: Option<&str>,
    children: &mut Map<String, Value>,
    do_not_squash_sites: &HashSet<String>,
    roles_by_node_id: &HashMap<String, TopologyAttachmentRole>,
) {
    let child_keys = children.keys().cloned().collect::<Vec<_>>();
    for child_key in child_keys {
        let Some(node) = children.get_mut(&child_key).and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(grandchildren) = node.get_mut("children").and_then(Value::as_object_mut) else {
            continue;
        };
        squash_backhaul_pairs_in_children(
            Some(&child_key),
            grandchildren,
            do_not_squash_sites,
            roles_by_node_id,
        );
    }

    loop {
        let mut changed = false;
        let child_keys = children.keys().cloned().collect::<Vec<_>>();
        for child_key in child_keys {
            let Some(child_value) = children.get(&child_key) else {
                continue;
            };
            if !node_type_is(child_value, "AP") {
                continue;
            }
            let Some(child_node) = child_value.as_object() else {
                continue;
            };
            let Some(child_children) = child_node.get("children").and_then(Value::as_object) else {
                continue;
            };
            if child_children.len() != 1 {
                continue;
            }
            let Some((grandchild_key, grandchild_value)) = child_children.iter().next() else {
                continue;
            };
            let grandchild_key = grandchild_key.clone();
            if !node_type_is(grandchild_value, "AP") {
                continue;
            }
            let Some(grandchild_node) = grandchild_value.as_object() else {
                continue;
            };
            let Some(grandchild_children) =
                grandchild_node.get("children").and_then(Value::as_object)
            else {
                continue;
            };
            if grandchild_children.len() != 1 {
                continue;
            }
            let Some((endpoint_key, endpoint_value)) = grandchild_children.iter().next() else {
                continue;
            };
            let endpoint_key = endpoint_key.clone();
            if node_type_is(endpoint_value, "AP") {
                continue;
            }
            let Some(endpoint_node) = endpoint_value.as_object() else {
                continue;
            };
            if !attachment_role_allows_runtime_squash(endpoint_attachment_role(
                endpoint_node,
                roles_by_node_id,
            )) {
                continue;
            }
            if !should_runtime_squash_chain(
                [
                    parent_name.unwrap_or_default(),
                    &child_key,
                    &grandchild_key,
                    &endpoint_key,
                ],
                do_not_squash_sites,
            ) {
                continue;
            }

            let Some(mut child_value) = children.remove(&child_key) else {
                continue;
            };
            let Some(child_node) = child_value.as_object_mut() else {
                continue;
            };
            let Some(child_children) = child_node
                .get_mut("children")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            let Some(mut grandchild_value) = child_children.remove(&grandchild_key) else {
                continue;
            };
            let Some(grandchild_node) = grandchild_value.as_object_mut() else {
                continue;
            };
            let Some(grandchild_children) = grandchild_node
                .get_mut("children")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            let Some(mut endpoint_value) = grandchild_children.remove(&endpoint_key) else {
                continue;
            };
            let Some(endpoint_node) = endpoint_value.as_object_mut() else {
                continue;
            };

            if let Some(download) = min_chain_bandwidth(
                endpoint_node,
                child_node,
                grandchild_node,
                "downloadBandwidthMbps",
            ) {
                endpoint_node.insert(
                    "downloadBandwidthMbps".to_string(),
                    Value::Number(download.into()),
                );
            }
            if let Some(upload) = min_chain_bandwidth(
                endpoint_node,
                child_node,
                grandchild_node,
                "uploadBandwidthMbps",
            ) {
                endpoint_node.insert(
                    "uploadBandwidthMbps".to_string(),
                    Value::Number(upload.into()),
                );
            }
            if let Some(parent_name) = parent_name {
                endpoint_node.insert(
                    "parent_site".to_string(),
                    Value::String(parent_name.to_string()),
                );
            }
            endpoint_node
                .entry("name".to_string())
                .or_insert_with(|| Value::String(endpoint_key.clone()));
            endpoint_node.insert(
                "active_attachment_name".to_string(),
                Value::String(grandchild_key.clone()),
            );

            children.insert(endpoint_key.clone(), endpoint_value);
            changed = true;
            break;
        }

        if !changed {
            break;
        }
    }
}

fn squash_single_attachment_hops_in_children(
    parent_name: Option<&str>,
    children: &mut Map<String, Value>,
    do_not_squash_sites: &HashSet<String>,
    roles_by_node_id: &HashMap<String, TopologyAttachmentRole>,
) {
    let child_keys = children.keys().cloned().collect::<Vec<_>>();
    for child_key in child_keys {
        let Some(node) = children.get_mut(&child_key).and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(grandchildren) = node.get_mut("children").and_then(Value::as_object_mut) else {
            continue;
        };
        squash_single_attachment_hops_in_children(
            Some(&child_key),
            grandchildren,
            do_not_squash_sites,
            roles_by_node_id,
        );
    }

    loop {
        let mut changed = false;
        let child_keys = children.keys().cloned().collect::<Vec<_>>();
        for child_key in child_keys {
            let Some(child_value) = children.get(&child_key) else {
                continue;
            };
            if !node_type_is(child_value, "AP") {
                continue;
            }
            let Some(child_node) = child_value.as_object() else {
                continue;
            };
            let Some(child_children) = child_node.get("children").and_then(Value::as_object) else {
                continue;
            };
            if child_children.len() != 1 {
                continue;
            }
            let Some((endpoint_key, endpoint_value)) = child_children.iter().next() else {
                continue;
            };
            let endpoint_key = endpoint_key.clone();
            if node_type_is(endpoint_value, "AP") {
                continue;
            }
            let Some(endpoint_node) = endpoint_value.as_object() else {
                continue;
            };
            if !attachment_role_allows_runtime_squash(endpoint_attachment_role(
                endpoint_node,
                roles_by_node_id,
            )) {
                continue;
            }
            if !should_runtime_squash_chain(
                [
                    parent_name.unwrap_or_default(),
                    &child_key,
                    &endpoint_key,
                    "",
                ],
                do_not_squash_sites,
            ) {
                continue;
            }

            let Some(mut child_value) = children.remove(&child_key) else {
                continue;
            };
            let Some(child_node) = child_value.as_object_mut() else {
                continue;
            };
            let Some(child_children) = child_node
                .get_mut("children")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            let Some(mut endpoint_value) = child_children.remove(&endpoint_key) else {
                continue;
            };
            let Some(endpoint_node) = endpoint_value.as_object_mut() else {
                continue;
            };

            if let Some(download) =
                min_attachment_bandwidth(endpoint_node, child_node, "downloadBandwidthMbps")
            {
                endpoint_node.insert(
                    "downloadBandwidthMbps".to_string(),
                    Value::Number(download.into()),
                );
            }
            if let Some(upload) =
                min_attachment_bandwidth(endpoint_node, child_node, "uploadBandwidthMbps")
            {
                endpoint_node.insert(
                    "uploadBandwidthMbps".to_string(),
                    Value::Number(upload.into()),
                );
            }
            if let Some(parent_name) = parent_name {
                endpoint_node.insert(
                    "parent_site".to_string(),
                    Value::String(parent_name.to_string()),
                );
            }
            endpoint_node
                .entry("name".to_string())
                .or_insert_with(|| Value::String(endpoint_key.clone()));
            endpoint_node.insert(
                "active_attachment_name".to_string(),
                Value::String(child_key.clone()),
            );

            children.insert(endpoint_key.clone(), endpoint_value);
            changed = true;
            break;
        }

        if !changed {
            break;
        }
    }
}

fn apply_runtime_squashing(
    config: &Config,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    root: &mut Map<String, Value>,
) {
    if !ui_state.source.starts_with("uisp/") {
        return;
    }
    if !config.uisp_integration.enable_uisp {
        return;
    }

    let do_not_squash_sites = config
        .uisp_integration
        .do_not_squash_sites
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();
    let active_pair_ids = selected_attachment_pair_ids(ui_state, effective);
    let pair_by_attachment_id = attachment_pair_memberships(ui_state);
    prune_inactive_backhaul_stubs_in_children(root, &pair_by_attachment_id, &active_pair_ids);
    let roles_by_node_id = selected_attachment_roles(ui_state, effective);
    squash_backhaul_pairs_in_children(None, root, &do_not_squash_sites, &roles_by_node_id);
    squash_single_attachment_hops_in_children(None, root, &do_not_squash_sites, &roles_by_node_id);
}

fn count_node_ids(value: &Value, counts: &mut HashMap<String, usize>) {
    let Some(node) = value.as_object() else {
        return;
    };
    if let Some(id) = node.get("id").and_then(Value::as_str) {
        *counts.entry(id.to_string()).or_insert(0) += 1;
    }
    if let Some(children) = node.get("children").and_then(Value::as_object) {
        for child in children.values() {
            count_node_ids(child, counts);
        }
    }
}

fn effective_site_parent_map(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> HashMap<String, String> {
    let effective_by_node = effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut parents = HashMap::new();

    for node in &ui_state.nodes {
        if !node.node_id.contains(":site:") {
            continue;
        }
        let selected_parent = effective_by_node
            .get(node.node_id.as_str())
            .map(|entry| entry.logical_parent_node_id.as_str())
            .filter(|parent_id| !parent_id.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| node.current_parent_node_id.clone());
        let Some(parent_id) = selected_parent else {
            continue;
        };
        if !parent_id.contains(":site:") {
            continue;
        }
        parents.insert(node.node_id.clone(), parent_id);
    }

    parents
}

fn validate_effective_site_parent_cycles(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    errors: &mut Vec<String>,
) {
    let parents = effective_site_parent_map(ui_state, effective);
    for site_id in parents.keys() {
        let mut seen = HashSet::new();
        let mut cursor = site_id.as_str();
        while let Some(parent_id) = parents.get(cursor) {
            if !seen.insert(cursor.to_string()) {
                let node_name = ui_state
                    .find_node(site_id)
                    .map(|node| node.node_name.clone())
                    .unwrap_or_else(|| site_id.clone());
                errors.push(format!(
                    "Effective topology would create a parent cycle involving '{}'.",
                    node_name
                ));
                break;
            }
            cursor = parent_id.as_str();
        }
    }
}

fn validate_effective_node_identity_consistency(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    errors: &mut Vec<String>,
) {
    let mut ui_counts = HashMap::<&str, usize>::new();
    for node in &ui_state.nodes {
        *ui_counts.entry(node.node_id.as_str()).or_default() += 1;
    }
    for (node_id, count) in ui_counts {
        if count > 1 {
            errors.push(format!(
                "Canonical topology editor state contains duplicate node id '{}'.",
                node_id
            ));
        }
    }

    let mut effective_counts = HashMap::<&str, usize>::new();
    for node in &effective.nodes {
        *effective_counts.entry(node.node_id.as_str()).or_default() += 1;
    }
    for (node_id, count) in effective_counts {
        if count > 1 {
            errors.push(format!(
                "Effective topology state contains duplicate node id '{}'.",
                node_id
            ));
        }
    }

    for canonical_node in &ui_state.nodes {
        if !effective
            .nodes
            .iter()
            .any(|effective_node| effective_node.node_id == canonical_node.node_id)
        {
            errors.push(format!(
                "Effective topology state is missing node '{}'.",
                canonical_node.node_name
            ));
        }
    }

    for node in &effective.nodes {
        let Some(ui_node) = ui_state.find_node(&node.node_id) else {
            errors.push(format!(
                "Effective topology state references unknown node id '{}'.",
                node.node_id
            ));
            continue;
        };

        if node.logical_parent_node_id.is_empty() {
            if node.effective_attachment_id.is_some() {
                errors.push(format!(
                    "Effective topology selected attachment for '{}' without a logical parent.",
                    ui_node.node_name
                ));
            }
            continue;
        }

        let Some(selected_parent) = ui_node
            .allowed_parents
            .iter()
            .find(|parent| parent.parent_node_id == node.logical_parent_node_id)
        else {
            let fixed_attachment_id = ui_node
                .current_attachment_id
                .as_deref()
                .filter(|attachment_id| !attachment_id.is_empty());
            let legacy_fixed_parent = ui_node.allowed_parents.is_empty()
                && ui_node.current_parent_node_id.as_deref()
                    == Some(node.logical_parent_node_id.as_str())
                && node.preferred_attachment_id.as_deref() == fixed_attachment_id
                && node.effective_attachment_id.as_deref() == fixed_attachment_id;
            if legacy_fixed_parent {
                continue;
            }
            errors.push(format!(
                "Effective topology selected invalid parent '{}' for '{}'.",
                node.logical_parent_node_id, ui_node.node_name
            ));
            continue;
        };

        if let Some(preferred_attachment_id) = node.preferred_attachment_id.as_deref()
            && !selected_parent
                .attachment_options
                .iter()
                .any(|option| option.attachment_id == preferred_attachment_id)
        {
            errors.push(format!(
                "Effective topology selected invalid preferred attachment '{}' for '{}'.",
                preferred_attachment_id, ui_node.node_name
            ));
        }

        if let Some(effective_attachment_id) = node.effective_attachment_id.as_deref()
            && !selected_parent
                .attachment_options
                .iter()
                .any(|option| option.attachment_id == effective_attachment_id)
        {
            errors.push(format!(
                "Effective topology selected invalid attachment '{}' for '{}'.",
                effective_attachment_id, ui_node.node_name
            ));
        }
    }
}

/// Validates that the candidate effective tree is structurally safe to publish.
///
/// This checks that the effective topology remains ID-consistent, the effective
/// site-parent graph is acyclic, and every canonical site node remains present
/// exactly once in the exported tree.
fn validate_effective_topology_network_from_canonical(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    effective_network: &Value,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    validate_effective_node_identity_consistency(ui_state, effective, &mut errors);
    validate_effective_site_parent_cycles(ui_state, effective, &mut errors);
    let queue_policy_tree = queue_policy_reference_tree(canonical, ui_state, effective);
    let queue_policy_root = queue_policy_tree.as_object();

    let mut counts = HashMap::new();
    let Some(root) = effective_network.as_object() else {
        return Err(vec![
            "Effective topology export is not a JSON object tree.".to_string(),
        ]);
    };
    for child in root.values() {
        count_node_ids(child, &mut counts);
    }
    let child_branch_counts = logical_child_branch_counts(ui_state);

    for node in &ui_state.nodes {
        if !node.node_id.contains(":site:") {
            continue;
        }
        if resolved_queue_visibility_policy(
            config,
            node,
            queue_policy_root.and_then(|root| find_node_by_id(root, &node.node_id)),
            &child_branch_counts,
        ) == TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren
        {
            continue;
        }
        match counts.get(&node.node_id).copied().unwrap_or_default() {
            1 => {}
            0 => errors.push(format!(
                "Effective topology export dropped site '{}'.",
                node.node_name
            )),
            count => errors.push(format!(
                "Effective topology export duplicated site '{}' {} times.",
                node.node_name, count
            )),
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validates that an effective topology export remains structurally safe to publish.
///
/// This legacy helper accepts the candidate canonical network tree directly and reconstructs
/// canonical topology metadata from it so existing call sites and tests can keep using the same
/// interface.
pub fn validate_effective_topology_network(
    config: &Config,
    canonical_network: &Value,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    effective_network: &Value,
) -> Result<(), Vec<String>> {
    let canonical_state = TopologyCanonicalStateFile::from_editor_and_network(
        ui_state,
        canonical_network,
        TopologyCanonicalIngressKind::NativeIntegration,
    );
    validate_effective_topology_network_from_canonical(
        config,
        &canonical_state,
        ui_state,
        effective,
        effective_network,
    )
}

/// Applies the effective attachment selection to a canonical network tree and returns
/// the runtime-effective tree used by shaping/export.
fn apply_effective_topology_to_network_json_from_canonical(
    config: &Config,
    canonical_network: &Value,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> Value {
    let mut out = apply_effective_topology_reparenting_only(canonical_network, ui_state, effective);
    if let Some(root) = out.as_object_mut() {
        recompile_effective_network_bandwidths(root, canonical, ui_state, effective);
        apply_queue_hidden_node_virtualization(config, ui_state, root);
        apply_runtime_squashing(config, ui_state, effective, root);
    }
    out
}

/// Applies the effective attachment selection to a canonical network tree and returns
/// the runtime-effective tree used by shaping/export.
///
/// This legacy helper accepts the candidate canonical network tree directly and reconstructs
/// canonical topology metadata from it so existing call sites and tests can keep using the same
/// interface.
pub fn apply_effective_topology_to_network_json(
    config: &Config,
    canonical_network: &Value,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> Value {
    let canonical_state = TopologyCanonicalStateFile::from_editor_and_network(
        ui_state,
        canonical_network,
        TopologyCanonicalIngressKind::NativeIntegration,
    );
    apply_effective_topology_to_network_json_from_canonical(
        config,
        canonical_network,
        &canonical_state,
        ui_state,
        effective,
    )
}

fn apply_effective_topology_to_canonical_state(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> Value {
    let canonical_network =
        if canonical.ingress_kind == TopologyCanonicalIngressKind::NativeIntegration {
            canonical.insight_topology_network_json()
        } else {
            canonical.compatibility_network_json().clone()
        };
    apply_effective_topology_to_network_json_from_canonical(
        config,
        &canonical_network,
        canonical,
        ui_state,
        effective,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        EffectiveTopologyArtifacts, apply_effective_topology_to_canonical_state,
        apply_effective_topology_to_network_json, auto_attachment_option,
        build_effective_topology_artifacts, build_effective_topology_artifacts_from_canonical,
        build_shaping_inputs, compute_effective_state, publish_effective_topology_artifacts,
        publish_topology_runtime_error_status, validate_effective_topology_network,
    };
    use lqos_config::{
        CircuitAnchor, CircuitAnchorsFile, Config, TopologyAllowedParent,
        TopologyAttachmentHealthStateFile, TopologyAttachmentHealthStatus,
        TopologyAttachmentOption, TopologyAttachmentRateSource, TopologyAttachmentRole,
        TopologyCanonicalIngressKind, TopologyCanonicalNode, TopologyCanonicalStateFile,
        TopologyEditorNode, TopologyEditorStateFile, TopologyEffectiveAttachmentState,
        TopologyEffectiveNodeState, TopologyEffectiveStateFile, TopologyQueueVisibilityPolicy,
        TopologyRuntimeStatusFile, topology_effective_network_path, topology_effective_state_path,
        topology_runtime_status_path, topology_shaping_inputs_path,
    };
    use lqos_overrides::{TopologyAttachmentMode, TopologyOverridesFile};
    use serde_json::{Value, json};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough for tests")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
        fs::create_dir_all(&path).expect("temp directory should be creatable");
        path
    }

    fn write_runtime_json_fixture(path: PathBuf, value: &Value, label: &str) {
        let parent = path.parent().expect("runtime fixture path should have parent");
        fs::create_dir_all(parent).expect("runtime fixture parent should be creatable");
        fs::write(
            &path,
            serde_json::to_string_pretty(value).expect("runtime fixture should serialize"),
        )
        .unwrap_or_else(|_| panic!("{label} should write"));
    }

    fn runtime_tree_max_depth(value: &Value) -> usize {
        fn recurse(node: &Value, depth: usize) -> usize {
            let Some(map) = node.as_object() else {
                return depth;
            };
            let Some(children) = map.get("children").and_then(Value::as_object) else {
                return depth;
            };
            children
                .values()
                .map(|child| recurse(child, depth + 1))
                .max()
                .unwrap_or(depth)
        }

        value
            .as_object()
            .map(|root| {
                root.values()
                    .map(|child| recurse(child, 1))
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
    }

    fn sample_attachment_option(
        attachment_id: &str,
        attachment_name: &str,
    ) -> TopologyAttachmentOption {
        TopologyAttachmentOption {
            attachment_id: attachment_id.to_string(),
            attachment_name: attachment_name.to_string(),
            attachment_kind: "device".to_string(),
            attachment_role: TopologyAttachmentRole::PtpBackhaul,
            pair_id: None,
            peer_attachment_id: None,
            peer_attachment_name: None,
            capacity_mbps: Some(500),
            download_bandwidth_mbps: Some(500),
            upload_bandwidth_mbps: Some(500),
            transport_cap_mbps: None,
            transport_cap_reason: None,
            rate_source: TopologyAttachmentRateSource::Static,
            can_override_rate: false,
            rate_override_disabled_reason: None,
            has_rate_override: false,
            local_probe_ip: None,
            remote_probe_ip: None,
            probe_enabled: false,
            probeable: false,
            health_status: TopologyAttachmentHealthStatus::Healthy,
            health_reason: None,
            suppressed_until_unix: None,
            effective_selected: false,
        }
    }

    fn sample_runtime_artifacts() -> EffectiveTopologyArtifacts {
        EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "tower-1".to_string(),
                    logical_parent_node_id: "site-a".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: Vec::new(),
                }],
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "tower-1".to_string(),
                    node_name: "Tower 1".to_string(),
                    ..TopologyEditorNode::default()
                }],
            },
            effective_network: Some(json!({
                "Tower 1": {
                    "id": "tower-1",
                    "name": "Tower 1",
                    "children": {}
                }
            })),
        }
    }

    #[test]
    fn topology_runtime_status_transitions_from_error_to_ready() {
        let lqos_directory = unique_temp_dir("lqos-topology-runtime-status-transition");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        let generation = "generation-1";

        publish_topology_runtime_error_status(&config, generation, "topology build failed")
            .expect("failed status should publish");
        let failed = TopologyRuntimeStatusFile::load(&config).expect("failed status should load");
        assert_eq!(failed.source_generation, generation);
        assert!(!failed.ready);
        assert_eq!(failed.error.as_deref(), Some("topology build failed"));

        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Tower 1\",\"tower-1\",\"tower-1\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

        publish_effective_topology_artifacts(&config, &sample_runtime_artifacts(), generation)
            .expect("ready status should publish");
        let ready = TopologyRuntimeStatusFile::load(&config).expect("ready status should load");
        assert_eq!(ready.source_generation, generation);
        assert!(ready.ready);
        assert_eq!(ready.error, None);
        assert!(!ready.shaping_generation.is_empty());
        assert_eq!(
            ready.effective_state_path,
            topology_effective_state_path(&config)
                .to_string_lossy()
                .to_string()
        );
        assert_eq!(
            ready.effective_network_path,
            topology_effective_network_path(&config)
                .to_string_lossy()
                .to_string()
        );
        assert_eq!(
            ready.shaping_inputs_path,
            topology_shaping_inputs_path(&config)
                .to_string_lossy()
                .to_string()
        );
        assert!(topology_effective_state_path(&config).exists());
        assert!(topology_effective_network_path(&config).exists());
        assert!(topology_shaping_inputs_path(&config).exists());
        assert!(topology_runtime_status_path(&config).exists());
    }

    #[test]
    fn shaping_inputs_prefer_circuit_anchors_over_csv_anchor_fields() {
        let lqos_directory = unique_temp_dir("lqos-topology-circuit-anchors");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Legacy Parent\",\"legacy-parent\",\"legacy-anchor\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");
        CircuitAnchorsFile {
            schema_version: 1,
            source: "test".to_string(),
            generated_unix: Some(1),
            anchors: vec![CircuitAnchor {
                circuit_id: "circuit-1".to_string(),
                circuit_name: Some("Circuit 1".to_string()),
                anchor_node_id: "tower-1".to_string(),
                anchor_node_name: Some("Tower 1".to_string()),
            }],
        }
        .save(&config)
        .expect("circuit_anchors.json should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "tower-1".to_string(),
                    logical_parent_node_id: "site-a".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: Vec::new(),
                }],
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "tower-1".to_string(),
                    node_name: "Tower 1".to_string(),
                    ..TopologyEditorNode::default()
                }],
            },
            effective_network: Some(json!({
                "Tower 1": {
                    "id": "tower-1",
                    "children": {}
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build")
            .expect("shaping inputs should exist");
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit");

        assert_eq!(circuit.anchor_node_id.as_deref(), Some("tower-1"));
        assert_eq!(circuit.anchor_node_name.as_deref(), Some("Tower 1"));
        assert_eq!(circuit.effective_parent_node_id, "tower-1");
        assert_eq!(circuit.effective_parent_node_name, "Tower 1");
    }

    #[test]
    fn shaping_inputs_apply_effective_overrides_for_integration_ingress() {
        let lqos_directory = unique_temp_dir("lqos-topology-runtime-overrides");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        config.splynx_integration.enable_splynx = true;
        write_runtime_json_fixture(
            config.topology_state_file_path("topology_import.json"),
            &json!({
                "schema_version": 1,
                "source": "splynx/full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "compile_mode": "full",
                "imported": {
                    "source": "splynx/full",
                    "generated_unix": 1,
                    "ingress_identity": "ingress-1",
                    "compatibility_network_json": {
                        "Tower 1": {
                            "id": "tower-1",
                            "children": {}
                        }
                    },
                    "shaped_devices": [
                        {
                            "circuit_id": "circuit-1",
                            "circuit_name": "Circuit 1",
                            "device_id": "device-1",
                            "device_name": "Device 1",
                            "parent_node": "Tower 1",
                            "parent_node_id": "tower-1",
                            "anchor_node_id": null,
                            "mac": "",
                            "ipv4": [],
                            "ipv6": [],
                            "download_min_mbps": 10.0,
                            "upload_min_mbps": 10.0,
                            "download_max_mbps": 100.0,
                            "upload_max_mbps": 100.0,
                            "comment": "",
                            "sqm_override": null
                        }
                    ],
                    "circuit_anchors": {
                        "schema_version": 1,
                        "source": "splynx/full",
                        "generated_unix": 1,
                        "anchors": []
                    },
                    "ethernet_advisories": []
                }
            }),
            "topology import",
        );
        write_runtime_json_fixture(
            config.shaping_state_file_path("topology_compiled_shaping.json"),
            &json!({
                "schema_version": 1,
                "source": "splynx/full",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "shaped_devices": [
                    {
                        "circuit_id": "circuit-1",
                        "circuit_name": "Circuit 1",
                        "device_id": "device-1",
                        "device_name": "Device 1",
                        "parent_node": "Tower 1",
                        "parent_node_id": "tower-1",
                        "anchor_node_id": null,
                        "mac": "",
                        "ipv4": [],
                        "ipv6": [],
                        "download_min_mbps": 10.0,
                        "upload_min_mbps": 10.0,
                        "download_max_mbps": 100.0,
                        "upload_max_mbps": 100.0,
                        "comment": "",
                        "sqm_override": null
                    }
                ],
                "circuit_anchors": {
                    "schema_version": 1,
                    "source": "splynx/full",
                    "generated_unix": 1,
                    "anchors": []
                }
            }),
            "compiled shaping",
        );
        fs::write(
            lqos_directory.join("lqos_overrides.json"),
            serde_json::to_string_pretty(&json!({
                "persistent_devices": [
                    {
                        "circuit_id": "circuit-2",
                        "circuit_name": "Circuit 2",
                        "device_id": "device-2",
                        "device_name": "Device 2",
                        "parent_node": "Tower 1",
                        "parent_node_id": "tower-1",
                        "anchor_node_id": null,
                        "mac": "",
                        "ipv4": [],
                        "ipv6": [],
                        "download_min_mbps": 5.0,
                        "upload_min_mbps": 5.0,
                        "download_max_mbps": 50.0,
                        "upload_max_mbps": 50.0,
                        "comment": "",
                        "sqm_override": null
                    }
                ],
                "circuit_adjustments": [
                    {
                        "type": "device_adjust_speed",
                        "device_id": "device-1",
                        "max_download_bandwidth": 80.0,
                        "max_upload_bandwidth": 60.0
                    }
                ],
                "network_adjustments": []
            }))
            .expect("override json should serialize"),
        )
        .expect("override file should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: Vec::new(),
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: Vec::new(),
            },
            effective_network: Some(json!({
                "Tower 1": {
                    "id": "tower-1",
                    "children": {}
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build")
            .expect("shaping inputs should exist");
        let circuit_one = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit-1");
        let circuit_two = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-2")
            .expect("expected circuit-2");

        assert_eq!(circuit_one.download_max_mbps, 80.0);
        assert_eq!(circuit_one.upload_max_mbps, 60.0);
        assert_eq!(circuit_two.effective_parent_node_id, "tower-1");
        assert_eq!(circuit_two.effective_parent_node_name, "Tower 1");
    }

    #[test]
    fn shaping_inputs_use_topology_import_without_shaped_devices_csv_for_integration_ingress() {
        let lqos_directory = unique_temp_dir("lqos-topology-import-shaped-devices");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;
        write_runtime_json_fixture(
            config.topology_state_file_path("topology_import.json"),
            &json!({
                "schema_version": 1,
                "source": "uisp/full2",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "imported": {
                    "source": "uisp/full2",
                    "generated_unix": 1,
                    "ingress_identity": "imported-1",
                    "compatibility_network_json": {},
                    "shaped_devices": [
                        {
                            "circuit_id": "circuit-1",
                            "circuit_name": "Circuit 1",
                            "device_id": "device-1",
                            "device_name": "Device 1",
                            "parent_node": "Tower 1",
                            "parent_node_id": "tower-1",
                            "anchor_node_id": null,
                            "mac": "",
                            "ipv4": [],
                            "ipv6": [],
                            "download_min_mbps": 10.0,
                            "upload_min_mbps": 10.0,
                            "download_max_mbps": 100.0,
                            "upload_max_mbps": 100.0,
                            "comment": "",
                            "sqm_override": null
                        }
                    ],
                    "circuit_anchors": {
                        "schema_version": 1,
                        "source": "uisp/full",
                        "generated_unix": 1,
                        "anchors": []
                    },
                    "ethernet_advisories": []
                }
            }),
            "topology import",
        );
        write_runtime_json_fixture(
            config.shaping_state_file_path("topology_compiled_shaping.json"),
            &json!({
                "schema_version": 1,
                "source": "uisp/full",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "shaped_devices": [
                    {
                        "circuit_id": "circuit-1",
                        "circuit_name": "Circuit 1",
                        "device_id": "device-1",
                        "device_name": "Device 1",
                        "parent_node": "Compiled Tower 1",
                        "parent_node_id": "compiled-tower-1",
                        "anchor_node_id": null,
                        "mac": "",
                        "ipv4": [],
                        "ipv6": [],
                        "download_min_mbps": 10.0,
                        "upload_min_mbps": 10.0,
                        "download_max_mbps": 100.0,
                        "upload_max_mbps": 100.0,
                        "comment": "",
                        "sqm_override": null
                    }
                ],
                "circuit_anchors": {
                    "schema_version": 1,
                    "source": "uisp/full",
                    "generated_unix": 1,
                    "anchors": []
                }
            }),
            "compiled shaping",
        );

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: Vec::new(),
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: Vec::new(),
            },
            effective_network: Some(json!({
                "Compiled Tower 1": {
                    "id": "compiled-tower-1",
                    "children": {}
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build from topology_compiled_shaping.json")
            .expect("shaping inputs should exist");
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit");

        assert_eq!(circuit.effective_parent_node_id, "compiled-tower-1");
        assert_eq!(circuit.effective_parent_node_name, "Compiled Tower 1");
    }

    #[test]
    fn shaping_inputs_remap_non_selected_attachment_anchor_to_effective_attachment() {
        let lqos_directory = unique_temp_dir("lqos-topology-attachment-anchor-remap");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Legacy Parent\",\"legacy-parent\",\"legacy-anchor\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");
        CircuitAnchorsFile {
            schema_version: 1,
            source: "test".to_string(),
            generated_unix: Some(1),
            anchors: vec![CircuitAnchor {
                circuit_id: "circuit-1".to_string(),
                circuit_name: Some("Circuit 1".to_string()),
                anchor_node_id: "attachment-old".to_string(),
                anchor_node_name: Some("Old Attachment".to_string()),
            }],
        }
        .save(&config)
        .expect("circuit_anchors.json should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: vec![
                    TopologyEffectiveNodeState {
                        node_id: "attachment-old".to_string(),
                        logical_parent_node_id: "site-parent".to_string(),
                        preferred_attachment_id: None,
                        effective_attachment_id: None,
                        fallback_reason: None,
                        all_attachments_suppressed: false,
                        attachments: Vec::new(),
                    },
                    TopologyEffectiveNodeState {
                        node_id: "site-child".to_string(),
                        logical_parent_node_id: "site-parent".to_string(),
                        preferred_attachment_id: Some("attachment-new".to_string()),
                        effective_attachment_id: Some("attachment-new".to_string()),
                        fallback_reason: None,
                        all_attachments_suppressed: false,
                        attachments: Vec::new(),
                    },
                ],
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![
                    TopologyEditorNode {
                        node_id: "attachment-old".to_string(),
                        node_name: "Old Attachment".to_string(),
                        ..TopologyEditorNode::default()
                    },
                    TopologyEditorNode {
                        node_id: "attachment-new".to_string(),
                        node_name: "New Attachment".to_string(),
                        ..TopologyEditorNode::default()
                    },
                    TopologyEditorNode {
                        node_id: "site-child".to_string(),
                        node_name: "Child Site".to_string(),
                        allowed_parents: vec![TopologyAllowedParent {
                            parent_node_id: "site-parent".to_string(),
                            parent_node_name: "Parent Site".to_string(),
                            attachment_options: vec![
                                sample_attachment_option("attachment-old", "Old Attachment"),
                                sample_attachment_option("attachment-new", "New Attachment"),
                            ],
                            all_attachments_suppressed: false,
                            has_probe_unavailable_attachments: false,
                        }],
                        effective_attachment_name: Some("New Attachment".to_string()),
                        ..TopologyEditorNode::default()
                    },
                ],
            },
            effective_network: Some(json!({
                "Parent Site": {
                    "id": "site-parent",
                    "name": "Parent Site",
                    "children": {
                        "New Attachment": {
                            "id": "attachment-new",
                            "name": "New Attachment",
                            "children": {
                                "Child Site": {
                                    "id": "site-child",
                                    "name": "Child Site",
                                    "children": {}
                                }
                            }
                        }
                    }
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build")
            .expect("shaping inputs should exist");
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit");

        assert_eq!(circuit.anchor_node_id.as_deref(), Some("attachment-old"));
        assert_eq!(circuit.effective_parent_node_id, "attachment-new");
        assert_eq!(circuit.effective_parent_node_name, "New Attachment");
        assert_eq!(
            circuit.effective_attachment_id.as_deref(),
            Some("attachment-new")
        );
        assert_eq!(
            circuit.effective_attachment_name.as_deref(),
            Some("New Attachment")
        );
    }

    #[test]
    fn shaping_inputs_resolve_legacy_parent_against_exported_effective_tree() {
        let lqos_directory = unique_temp_dir("lqos-topology-legacy-parent-resolution");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Tower 1\",\"tower-1\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: Vec::new(),
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: Vec::new(),
            },
            effective_network: Some(json!({
                "Tower 1": {
                    "id": "tower-1",
                    "children": {}
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build")
            .expect("shaping inputs should exist");
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit");

        assert_eq!(circuit.effective_parent_node_id, "tower-1");
        assert_eq!(circuit.effective_parent_node_name, "Tower 1");
        assert_eq!(
            circuit.resolution_source,
            lqos_config::TopologyShapingResolutionSource::LegacyParent
        );
    }

    #[test]
    fn shaping_inputs_skip_virtual_effective_nodes_when_resolving_physical_parent() {
        let lqos_directory = unique_temp_dir("lqos-topology-legacy-parent-virtual");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Aggregation\",\"site-agg\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "site-agg".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: Vec::new(),
                }],
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "site-agg".to_string(),
                    node_name: "Aggregation".to_string(),
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Core".to_string()),
                    queue_visibility_policy:
                        TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren,
                    ..TopologyEditorNode::default()
                }],
            },
            effective_network: Some(json!({
                "Core": {
                    "id": "site-root",
                    "name": "Core",
                    "children": {
                        "Aggregation": {
                            "id": "site-agg",
                            "name": "Aggregation",
                            "virtual": true,
                            "children": {}
                        }
                    }
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build")
            .expect("shaping inputs should exist");
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit");

        assert_eq!(circuit.effective_parent_node_id, "site-root");
        assert_eq!(circuit.effective_parent_node_name, "Core");
        assert_eq!(
            circuit.resolution_source,
            lqos_config::TopologyShapingResolutionSource::LegacyParent
        );
    }

    #[test]
    fn shaping_inputs_fallback_to_generated_parents_when_anchor_does_not_resolve() {
        let lqos_directory = unique_temp_dir("lqos-topology-missing-anchor");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Legacy Parent\",\"legacy-parent\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");
        CircuitAnchorsFile {
            schema_version: 1,
            source: "test".to_string(),
            generated_unix: Some(1),
            anchors: vec![CircuitAnchor {
                circuit_id: "circuit-1".to_string(),
                circuit_name: Some("Circuit 1".to_string()),
                anchor_node_id: "missing-anchor".to_string(),
                anchor_node_name: Some("Missing Anchor".to_string()),
            }],
        }
        .save(&config)
        .expect("circuit_anchors.json should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: Vec::new(),
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: Vec::new(),
            },
            effective_network: Some(json!({})),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("missing anchor should no longer fail shaping input generation")
            .expect("shaping inputs should be present");
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("circuit should be present");
        assert_eq!(circuit.effective_parent_node_id, "");
        assert_eq!(circuit.effective_parent_node_name, "");
        assert_eq!(
            circuit.resolution_source,
            lqos_config::TopologyShapingResolutionSource::RuntimeFallback
        );
        assert!(
            shaping_inputs
                .warnings
                .iter()
                .any(|warning| warning.contains("missing-anchor"))
        );
        assert!(
            shaping_inputs
                .warnings
                .iter()
                .any(|warning| warning.contains("generated parent nodes"))
        );
    }

    #[test]
    fn flat_mode_assigns_explicit_generated_parent_buckets() {
        let lqos_directory = unique_temp_dir("lqos-topology-flat-summary");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        config.topology.compile_mode = "flat".to_string();
        config.queues.override_available_queues = Some(2);
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"\",\"\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
                "\"circuit-2\",\"Circuit 2\",\"device-2\",\"Device 2\",\"\",\"\",\"\",\"aa:bb:cc:dd:ee:00\",\"192.0.2.11/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: Vec::new(),
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: Vec::new(),
            },
            effective_network: Some(json!({})),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("flat mode shaping inputs should build")
            .expect("shaping inputs should be present");
        assert!(shaping_inputs.warnings.is_empty());
        assert_eq!(shaping_inputs.circuits.len(), 2);
        assert!(shaping_inputs.circuits.iter().all(|circuit| {
            circuit
                .effective_parent_node_name
                .starts_with("Generated_PN_")
        }));
        assert!(shaping_inputs.circuits.iter().all(|circuit| {
            circuit.resolution_source == lqos_config::TopologyShapingResolutionSource::FlatBucket
        }));
    }

    #[test]
    fn flat_mode_publishes_generated_parent_nodes_into_effective_network() {
        let mut config = Config::default();
        config.topology.compile_mode = "flat".to_string();
        config.queues.override_available_queues = Some(3);

        let canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({}));
        let artifacts = build_effective_topology_artifacts_from_canonical(
            &config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        )
        .expect("flat mode effective artifacts should build");
        let effective_network = artifacts
            .effective_network
            .expect("flat mode should publish an effective network");
        let root = effective_network
            .as_object()
            .expect("effective network should be an object");
        assert_eq!(root.len(), 3);
        for index in 0..3 {
            let name = format!("Generated_PN_{}", index + 1);
            let expected_id = format!("libreqos:generated:flat:bucket:{index}");
            let node = root
                .get(&name)
                .and_then(Value::as_object)
                .expect("generated parent node should exist");
            assert_eq!(
                node.get("id").and_then(Value::as_str),
                Some(expected_id.as_str())
            );
            assert_eq!(
                node.get("name").and_then(Value::as_str),
                Some(name.as_str())
            );
        }
    }

    #[test]
    fn runtime_squashing_collapses_backhaul_pairs_after_attachment_selection() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Relay A": {
                        "children": {
                            "Relay B": {
                                "children": {
                                    "Child Site": {
                                        "children": {
                                            "Leaf AP": {
                                                "children": {},
                                                "downloadBandwidthMbps": 200,
                                                "id": "leaf-ap",
                                                "name": "Leaf AP",
                                                "parent_site": "Child Site",
                                                "type": "AP",
                                                "uploadBandwidthMbps": 150
                                            }
                                        },
                                        "downloadBandwidthMbps": 800,
                                        "id": "child-site",
                                        "name": "Child Site",
                                        "parent_site": "Relay B",
                                        "type": "Site",
                                        "uploadBandwidthMbps": 700
                                    }
                                },
                                "downloadBandwidthMbps": 600,
                                "id": "relay-b",
                                "name": "Relay B",
                                "parent_site": "Relay A",
                                "type": "AP",
                                "uploadBandwidthMbps": 500
                            }
                        },
                        "downloadBandwidthMbps": 900,
                        "id": "relay-a",
                        "name": "Relay A",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 400
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("parent-site".to_string()),
                current_parent_node_name: Some("Parent Site".to_string()),
                current_attachment_id: Some("relay-b".to_string()),
                current_attachment_name: Some("Relay B".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![sample_attachment_option("relay-b", "Relay B")],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "child-site".to_string(),
                logical_parent_node_id: "parent-site".to_string(),
                preferred_attachment_id: Some("relay-b".to_string()),
                effective_attachment_id: Some("relay-b".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![TopologyEffectiveAttachmentState {
                    attachment_id: "relay-b".to_string(),
                    health_status: TopologyAttachmentHealthStatus::Healthy,
                    health_reason: None,
                    suppressed_until_unix: None,
                    probe_enabled: false,
                    probeable: false,
                    effective_selected: true,
                }],
            }],
        };

        let squashed =
            apply_effective_topology_to_network_json(&config, &canonical, &ui_state, &effective);
        let parent_children = squashed["Parent Site"]["children"]
            .as_object()
            .expect("parent should keep children");
        assert!(parent_children.get("Relay A").is_none());
        let child_site = parent_children
            .get("Child Site")
            .and_then(|value| value.as_object())
            .expect("child site should be squashed under parent");
        assert_eq!(child_site["parent_site"].as_str(), Some("Parent Site"));
        assert_eq!(
            child_site["active_attachment_name"].as_str(),
            Some("Relay B")
        );
        assert_eq!(child_site["downloadBandwidthMbps"].as_u64(), Some(500));
        assert_eq!(child_site["uploadBandwidthMbps"].as_u64(), Some(400));
    }

    #[test]
    fn runtime_squashing_collapses_single_attachment_hops_into_site_metadata() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Backhaul Attachment": {
                        "children": {
                            "Child Site": {
                                "children": {},
                                "downloadBandwidthMbps": 940,
                                "id": "child-site",
                                "name": "Child Site",
                                "parent_site": "Backhaul Attachment",
                                "type": "Site",
                                "uploadBandwidthMbps": 940
                            }
                        },
                        "downloadBandwidthMbps": 400,
                        "id": "backhaul-attachment",
                        "name": "Backhaul Attachment",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 400
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });

        let mut single_hop_attachment =
            sample_attachment_option("backhaul-attachment", "Backhaul Attachment");
        single_hop_attachment.capacity_mbps = Some(400);
        single_hop_attachment.download_bandwidth_mbps = Some(400);
        single_hop_attachment.upload_bandwidth_mbps = Some(400);

        let squashed = apply_effective_topology_to_network_json(
            &config,
            &canonical,
            &TopologyEditorStateFile {
                schema_version: 1,
                source: "uisp/full2".to_string(),
                generated_unix: None,
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "child-site".to_string(),
                    node_name: "Child Site".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("parent-site".to_string()),
                    current_parent_node_name: Some("Parent Site".to_string()),
                    current_attachment_id: Some("backhaul-attachment".to_string()),
                    current_attachment_name: Some("Backhaul Attachment".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "parent-site".to_string(),
                        parent_node_name: "Parent Site".to_string(),
                        attachment_options: vec![single_hop_attachment],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                }],
            },
            &TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: None,
                canonical_generated_unix: None,
                health_generated_unix: None,
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "child-site".to_string(),
                    logical_parent_node_id: "parent-site".to_string(),
                    preferred_attachment_id: Some("backhaul-attachment".to_string()),
                    effective_attachment_id: Some("backhaul-attachment".to_string()),
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![TopologyEffectiveAttachmentState {
                        attachment_id: "backhaul-attachment".to_string(),
                        health_status: TopologyAttachmentHealthStatus::Healthy,
                        health_reason: None,
                        suppressed_until_unix: None,
                        probe_enabled: false,
                        probeable: false,
                        effective_selected: true,
                    }],
                }],
            },
        );
        let parent_children = squashed["Parent Site"]["children"]
            .as_object()
            .expect("parent should keep children");
        assert!(parent_children.get("Backhaul Attachment").is_none());
        let child_site = parent_children
            .get("Child Site")
            .and_then(|value| value.as_object())
            .expect("child site should be squashed under parent");
        assert_eq!(child_site["parent_site"].as_str(), Some("Parent Site"));
        assert_eq!(
            child_site["active_attachment_name"].as_str(),
            Some("Backhaul Attachment")
        );
        assert_eq!(child_site["downloadBandwidthMbps"].as_u64(), Some(400));
        assert_eq!(child_site["uploadBandwidthMbps"].as_u64(), Some(400));
    }

    #[test]
    fn runtime_squashing_reduces_export_tree_depth_for_queue_consumers() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Relay A": {
                        "children": {
                            "Relay B": {
                                "children": {
                                    "Child Site": {
                                        "children": {},
                                        "downloadBandwidthMbps": 800,
                                        "id": "child-site",
                                        "name": "Child Site",
                                        "parent_site": "Relay B",
                                        "type": "Site",
                                        "uploadBandwidthMbps": 700
                                    }
                                },
                                "downloadBandwidthMbps": 600,
                                "id": "relay-b",
                                "name": "Relay B",
                                "parent_site": "Relay A",
                                "type": "AP",
                                "uploadBandwidthMbps": 500
                            }
                        },
                        "downloadBandwidthMbps": 900,
                        "id": "relay-a",
                        "name": "Relay A",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 400
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("parent-site".to_string()),
                current_parent_node_name: Some("Parent Site".to_string()),
                current_attachment_id: Some("relay-b".to_string()),
                current_attachment_name: Some("Relay B".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![sample_attachment_option("relay-b", "Relay B")],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "child-site".to_string(),
                logical_parent_node_id: "parent-site".to_string(),
                preferred_attachment_id: Some("relay-b".to_string()),
                effective_attachment_id: Some("relay-b".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![TopologyEffectiveAttachmentState {
                    attachment_id: "relay-b".to_string(),
                    health_status: TopologyAttachmentHealthStatus::Healthy,
                    health_reason: None,
                    suppressed_until_unix: None,
                    probe_enabled: false,
                    probeable: false,
                    effective_selected: true,
                }],
            }],
        };

        let canonical_depth = runtime_tree_max_depth(&canonical);
        let squashed =
            apply_effective_topology_to_network_json(&config, &canonical, &ui_state, &effective);
        let squashed_depth = runtime_tree_max_depth(&squashed);

        assert_eq!(canonical_depth, 4);
        assert_eq!(squashed_depth, 2);
        assert!(squashed_depth < canonical_depth);
    }

    #[test]
    fn native_integration_effective_export_uses_logical_canonical_tree_before_squashing() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;

        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-west".to_string(),
                    node_name: "WestRedd".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "child-site".to_string(),
                    node_name: "Tuscany Ridge".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-west".to_string()),
                    current_parent_node_name: Some("WestRedd".to_string()),
                    current_attachment_id: Some("relay-b".to_string()),
                    current_attachment_name: Some("AVIAT_TuscanyRidge".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-west".to_string(),
                        parent_node_name: "WestRedd".to_string(),
                        attachment_options: vec![sample_attachment_option(
                            "relay-b",
                            "AVIAT_TuscanyRidge",
                        )],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let mut canonical = TopologyCanonicalStateFile::from_editor_and_network(
            &editor_state,
            &json!({
                "AVIAT_WestRedd": {
                    "children": {
                        "AVIAT_TuscanyRidge": {
                            "children": {
                                "Tuscany Ridge": {
                                    "children": {},
                                    "downloadBandwidthMbps": 900,
                                    "id": "child-site",
                                    "name": "Tuscany Ridge",
                                    "type": "Site",
                                    "uploadBandwidthMbps": 900
                                }
                            },
                            "downloadBandwidthMbps": 900,
                            "id": "relay-b",
                            "name": "AVIAT_TuscanyRidge",
                            "type": "AP",
                            "uploadBandwidthMbps": 900
                        }
                    },
                    "downloadBandwidthMbps": 1000,
                    "id": "relay-a",
                    "name": "AVIAT_WestRedd",
                    "type": "AP",
                    "uploadBandwidthMbps": 1000
                }
            }),
            TopologyCanonicalIngressKind::NativeIntegration,
        );
        canonical.nodes.push(TopologyCanonicalNode {
            node_id: "site-west".to_string(),
            node_name: "WestRedd".to_string(),
            latitude: None,
            longitude: None,
            node_kind: "Site".to_string(),
            is_virtual: false,
            current_parent_node_id: None,
            current_parent_node_name: None,
            current_attachment_id: None,
            current_attachment_name: None,
            can_move: false,
            allowed_parents: Vec::new(),
            queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
            rate_input: Default::default(),
        });

        let squashed = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &editor_state,
            &TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: None,
                canonical_generated_unix: None,
                health_generated_unix: None,
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "child-site".to_string(),
                    logical_parent_node_id: "site-west".to_string(),
                    preferred_attachment_id: Some("relay-b".to_string()),
                    effective_attachment_id: Some("relay-b".to_string()),
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![TopologyEffectiveAttachmentState {
                        attachment_id: "relay-b".to_string(),
                        health_status: TopologyAttachmentHealthStatus::Healthy,
                        health_reason: None,
                        suppressed_until_unix: None,
                        probe_enabled: false,
                        probeable: false,
                        effective_selected: true,
                    }],
                }],
            },
        );

        let root_children = squashed
            .as_object()
            .expect("native effective export should be an object");
        let west_children = root_children["WestRedd"]["children"]
            .as_object()
            .expect("WestRedd should stay as a logical root");
        assert!(root_children.get("AVIAT_WestRedd").is_none());
        assert!(west_children.get("AVIAT_WestRedd").is_none());
        assert!(west_children.get("Tuscany Ridge").is_some());
    }

    #[test]
    fn hidden_native_root_remains_virtual_in_effective_tree() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;

        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-west".to_string(),
                    node_name: "WestRedd".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy:
                        TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "child-site".to_string(),
                    node_name: "Tuscany Ridge".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-west".to_string()),
                    current_parent_node_name: Some("WestRedd".to_string()),
                    current_attachment_id: Some("relay-b".to_string()),
                    current_attachment_name: Some("AVIAT_TuscanyRidge".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-west".to_string(),
                        parent_node_name: "WestRedd".to_string(),
                        attachment_options: vec![sample_attachment_option(
                            "relay-b",
                            "AVIAT_TuscanyRidge",
                        )],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let mut canonical = TopologyCanonicalStateFile::from_editor_and_network(
            &editor_state,
            &json!({
                "WestRedd": {
                    "children": {
                        "AVIAT_WestRedd": {
                            "children": {
                                "AVIAT_TuscanyRidge": {
                                    "children": {
                                        "Tuscany Ridge": {
                                            "children": {},
                                            "downloadBandwidthMbps": 900,
                                            "id": "child-site",
                                            "name": "Tuscany Ridge",
                                            "type": "Site",
                                            "uploadBandwidthMbps": 900
                                        }
                                    },
                                    "downloadBandwidthMbps": 900,
                                    "id": "relay-b",
                                    "name": "AVIAT_TuscanyRidge",
                                    "type": "AP",
                                    "uploadBandwidthMbps": 900
                                }
                            },
                            "downloadBandwidthMbps": 1000,
                            "id": "relay-a",
                            "name": "AVIAT_WestRedd",
                            "type": "AP",
                            "uploadBandwidthMbps": 1000
                        }
                    },
                    "downloadBandwidthMbps": 5000,
                    "id": "site-west",
                    "name": "WestRedd",
                    "type": "Site",
                    "uploadBandwidthMbps": 5000
                }
            }),
            TopologyCanonicalIngressKind::NativeIntegration,
        );
        canonical.nodes.push(TopologyCanonicalNode {
            node_id: "site-west".to_string(),
            node_name: "WestRedd".to_string(),
            latitude: None,
            longitude: None,
            node_kind: "Site".to_string(),
            is_virtual: false,
            current_parent_node_id: None,
            current_parent_node_name: None,
            current_attachment_id: None,
            current_attachment_name: None,
            can_move: false,
            allowed_parents: Vec::new(),
            queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren,
            rate_input: Default::default(),
        });

        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "site-west".to_string(),
                    logical_parent_node_id: String::new(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "child-site".to_string(),
                    logical_parent_node_id: "site-west".to_string(),
                    preferred_attachment_id: Some("relay-b".to_string()),
                    effective_attachment_id: Some("relay-b".to_string()),
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![TopologyEffectiveAttachmentState {
                        attachment_id: "relay-b".to_string(),
                        health_status: TopologyAttachmentHealthStatus::Healthy,
                        health_reason: None,
                        suppressed_until_unix: None,
                        probe_enabled: false,
                        probeable: false,
                        effective_selected: true,
                    }],
                },
            ],
        };

        let effective_network = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &editor_state,
            &effective,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let west = root
            .get("WestRedd")
            .and_then(Value::as_object)
            .expect("WestRedd should remain visible as a logical virtual node");
        assert_eq!(west.get("virtual").and_then(Value::as_bool), Some(true));
        let west_children = west["children"]
            .as_object()
            .expect("WestRedd should retain its logical children");
        assert!(west_children.get("AVIAT_WestRedd").is_none());
        assert!(west_children.get("Tuscany Ridge").is_some());
    }

    #[test]
    fn queue_auto_marks_large_site_virtual_without_treeguard() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        config.topology.queue_auto_virtualize_threshold_mbps = 5_000;

        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-root".to_string(),
                    node_name: "Core".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "site-agg".to_string(),
                    node_name: "Aggregation".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Core".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueAuto,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "site-child".to_string(),
                    node_name: "Edge POP".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-agg".to_string()),
                    current_parent_node_name: Some("Aggregation".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let canonical = json!({
            "Core": {
                "children": {
                    "Aggregation": {
                        "children": {
                            "Edge POP": {
                                "children": {},
                                "downloadBandwidthMbps": 2000,
                                "id": "site-child",
                                "name": "Edge POP",
                                "type": "Site",
                                "uploadBandwidthMbps": 2000
                            }
                        },
                        "downloadBandwidthMbps": 7000,
                        "id": "site-agg",
                        "name": "Aggregation",
                        "type": "Site",
                        "uploadBandwidthMbps": 7000
                    }
                },
                "downloadBandwidthMbps": 20000,
                "id": "site-root",
                "name": "Core",
                "type": "Site",
                "uploadBandwidthMbps": 20000
            }
        });

        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "site-root".to_string(),
                    logical_parent_node_id: String::new(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "site-agg".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "site-child".to_string(),
                    logical_parent_node_id: "site-agg".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
            ],
        };

        let effective_network = apply_effective_topology_to_network_json(
            &config,
            &canonical,
            &editor_state,
            &effective,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let core = root["Core"]
            .as_object()
            .expect("Core should remain exported");
        let core_children = core["children"]
            .as_object()
            .expect("Core should remain exported");
        let aggregation = core_children
            .get("Aggregation")
            .and_then(Value::as_object)
            .expect("Aggregation should remain visible as a virtual node");
        assert_eq!(
            aggregation.get("virtual").and_then(Value::as_bool),
            Some(true)
        );
        let aggregation_children = aggregation["children"]
            .as_object()
            .expect("Aggregation should retain its logical children");
        assert!(aggregation_children.get("Edge POP").is_some());
    }

    #[test]
    fn queue_auto_uses_recompiled_effective_rate_before_virtualizing() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        config.topology.queue_auto_virtualize_threshold_mbps = 5_000;

        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-root".to_string(),
                    node_name: "Root".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "site-agg".to_string(),
                    node_name: "Aggregation".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Root".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueAuto,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "site-child".to_string(),
                    node_name: "Edge".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-agg".to_string()),
                    current_parent_node_name: Some("Aggregation".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let canonical = json!({
            "Root": {
                "children": {
                    "Aggregation": {
                        "children": {
                            "Edge": {
                                "children": {},
                                "downloadBandwidthMbps": 1000,
                                "id": "site-child",
                                "name": "Edge",
                                "type": "Site",
                                "uploadBandwidthMbps": 1000
                            }
                        },
                        "downloadBandwidthMbps": 100000,
                        "id": "site-agg",
                        "name": "Aggregation",
                        "type": "Site",
                        "uploadBandwidthMbps": 100000
                    }
                },
                "downloadBandwidthMbps": 2350,
                "id": "site-root",
                "name": "Root",
                "type": "Site",
                "uploadBandwidthMbps": 2350
            }
        });

        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "site-root".to_string(),
                    logical_parent_node_id: String::new(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "site-agg".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "site-child".to_string(),
                    logical_parent_node_id: "site-agg".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
            ],
        };

        let effective_network = apply_effective_topology_to_network_json(
            &config,
            &canonical,
            &editor_state,
            &effective,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let aggregation = root["Root"]["children"]["Aggregation"]
            .as_object()
            .expect("Aggregation should remain exported");
        assert_eq!(
            aggregation
                .get("downloadBandwidthMbps")
                .and_then(Value::as_u64),
            Some(2350)
        );
        assert_eq!(
            aggregation
                .get("uploadBandwidthMbps")
                .and_then(Value::as_u64),
            Some(2350)
        );
        assert_eq!(aggregation.get("virtual").and_then(Value::as_bool), None);
    }

    #[test]
    fn runtime_squashing_respects_do_not_squash_sites() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        config.uisp_integration.do_not_squash_sites = Some(vec!["Child Site".to_string()]);
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Relay A": {
                        "children": {
                            "Relay B": {
                                "children": {
                                    "Child Site": {
                                        "children": {},
                                        "downloadBandwidthMbps": 800,
                                        "id": "child-site",
                                        "name": "Child Site",
                                        "parent_site": "Relay B",
                                        "type": "Site",
                                        "uploadBandwidthMbps": 700
                                    }
                                },
                                "downloadBandwidthMbps": 600,
                                "id": "relay-b",
                                "name": "Relay B",
                                "parent_site": "Relay A",
                                "type": "AP",
                                "uploadBandwidthMbps": 500
                            }
                        },
                        "downloadBandwidthMbps": 900,
                        "id": "relay-a",
                        "name": "Relay A",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 400
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: Vec::new(),
        };
        let effective = TopologyEffectiveStateFile::default();

        let squashed =
            apply_effective_topology_to_network_json(&config, &canonical, &ui_state, &effective);
        assert!(squashed["Parent Site"]["children"]["Relay A"].is_object());
        assert!(squashed["Parent Site"]["children"]["Child Site"].is_null());
    }

    #[test]
    fn runtime_squashing_keeps_ptmp_uplink_aps_visible() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Access AP": {
                        "children": {
                            "Child CPE": {
                                "children": {
                                    "Child Site": {
                                        "children": {},
                                        "downloadBandwidthMbps": 110,
                                        "id": "child-site",
                                        "name": "Child Site",
                                        "parent_site": "Child CPE",
                                        "type": "Site",
                                        "uploadBandwidthMbps": 30
                                    }
                                },
                                "downloadBandwidthMbps": 209,
                                "id": "child-cpe",
                                "name": "Child CPE",
                                "parent_site": "Access AP",
                                "type": "AP",
                                "uploadBandwidthMbps": 40
                            }
                        },
                        "downloadBandwidthMbps": 313,
                        "id": "parent-ap",
                        "name": "Access AP",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 64
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });
        let mut ptmp_attachment = sample_attachment_option("child-cpe", "Child CPE");
        ptmp_attachment.attachment_role = TopologyAttachmentRole::PtmpUplink;
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("parent-site".to_string()),
                current_parent_node_name: Some("Parent Site".to_string()),
                current_attachment_id: Some("child-cpe".to_string()),
                current_attachment_name: Some("Child CPE".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![ptmp_attachment],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "child-site".to_string(),
                logical_parent_node_id: "parent-site".to_string(),
                preferred_attachment_id: Some("child-cpe".to_string()),
                effective_attachment_id: Some("child-cpe".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![TopologyEffectiveAttachmentState {
                    attachment_id: "child-cpe".to_string(),
                    health_status: TopologyAttachmentHealthStatus::Healthy,
                    health_reason: None,
                    suppressed_until_unix: None,
                    probe_enabled: false,
                    probeable: false,
                    effective_selected: true,
                }],
            }],
        };

        let squashed =
            apply_effective_topology_to_network_json(&config, &canonical, &ui_state, &effective);
        let parent_children = squashed["Parent Site"]["children"]
            .as_object()
            .expect("parent should keep children");
        assert!(
            parent_children
                .get("Access AP")
                .and_then(|value| value.as_object())
                .is_some()
        );
        assert!(parent_children.get("Child Site").is_none());
    }

    #[test]
    fn effective_export_keeps_logical_children_without_explicit_attachment() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Leaf AP": {
                        "children": {},
                        "downloadBandwidthMbps": 150,
                        "id": "leaf-ap",
                        "name": "Leaf AP",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 75
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "leaf-ap".to_string(),
                node_name: "Leaf AP".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("parent-site".to_string()),
                current_parent_node_name: Some("Parent Site".to_string()),
                current_attachment_id: Some("parent-site".to_string()),
                current_attachment_name: Some("Parent Site".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![auto_attachment_option()],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "leaf-ap".to_string(),
                logical_parent_node_id: "parent-site".to_string(),
                preferred_attachment_id: None,
                effective_attachment_id: None,
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![],
            }],
        };

        let exported =
            apply_effective_topology_to_network_json(&config, &canonical, &ui_state, &effective);
        let parent_children = exported["Parent Site"]["children"]
            .as_object()
            .expect("parent should keep children");
        assert!(
            parent_children
                .get("Leaf AP")
                .and_then(Value::as_object)
                .is_some()
        );
    }

    #[test]
    fn runtime_prunes_inactive_backhaul_attachment_stubs() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Active Parent Attachment": {
                        "children": {
                            "Active Child Attachment": {
                                "children": {
                                    "Child Site": {
                                        "children": {},
                                        "downloadBandwidthMbps": 900,
                                        "id": "child-site",
                                        "name": "Child Site",
                                        "parent_site": "Active Child Attachment",
                                        "type": "Site",
                                        "uploadBandwidthMbps": 900
                                    }
                                },
                                "downloadBandwidthMbps": 400,
                                "id": "active-child-attachment",
                                "name": "Active Child Attachment",
                                "parent_site": "Active Parent Attachment",
                                "type": "AP",
                                "uploadBandwidthMbps": 400
                            }
                        },
                        "downloadBandwidthMbps": 400,
                        "id": "active-parent-attachment",
                        "name": "Active Parent Attachment",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 400
                    },
                    "Inactive Parent Attachment": {
                        "children": {
                            "Inactive Child Attachment": {
                                "children": {},
                                "downloadBandwidthMbps": 2350,
                                "id": "inactive-child-attachment",
                                "name": "Inactive Child Attachment",
                                "parent_site": "Inactive Parent Attachment",
                                "type": "AP",
                                "uploadBandwidthMbps": 2350
                            }
                        },
                        "downloadBandwidthMbps": 2350,
                        "id": "inactive-parent-attachment",
                        "name": "Inactive Parent Attachment",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 2350
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });

        let mut active_attachment =
            sample_attachment_option("active-child-attachment", "Active Child Attachment");
        active_attachment.pair_id =
            Some("active-child-attachment|active-parent-attachment".to_string());
        active_attachment.peer_attachment_id = Some("active-parent-attachment".to_string());
        active_attachment.peer_attachment_name = Some("Active Parent Attachment".to_string());
        active_attachment.capacity_mbps = Some(400);
        active_attachment.download_bandwidth_mbps = Some(400);
        active_attachment.upload_bandwidth_mbps = Some(400);

        let mut inactive_attachment =
            sample_attachment_option("inactive-child-attachment", "Inactive Child Attachment");
        inactive_attachment.pair_id =
            Some("inactive-child-attachment|inactive-parent-attachment".to_string());
        inactive_attachment.peer_attachment_id = Some("inactive-parent-attachment".to_string());
        inactive_attachment.peer_attachment_name = Some("Inactive Parent Attachment".to_string());
        inactive_attachment.capacity_mbps = Some(2350);
        inactive_attachment.download_bandwidth_mbps = Some(2350);
        inactive_attachment.upload_bandwidth_mbps = Some(2350);

        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("parent-site".to_string()),
                current_parent_node_name: Some("Parent Site".to_string()),
                current_attachment_id: Some("active-child-attachment".to_string()),
                current_attachment_name: Some("Active Child Attachment".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![active_attachment, inactive_attachment],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "child-site".to_string(),
                logical_parent_node_id: "parent-site".to_string(),
                preferred_attachment_id: Some("active-child-attachment".to_string()),
                effective_attachment_id: Some("active-child-attachment".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![TopologyEffectiveAttachmentState {
                    attachment_id: "active-child-attachment".to_string(),
                    health_status: TopologyAttachmentHealthStatus::Healthy,
                    health_reason: None,
                    suppressed_until_unix: None,
                    probe_enabled: false,
                    probeable: false,
                    effective_selected: true,
                }],
            }],
        };

        let squashed =
            apply_effective_topology_to_network_json(&config, &canonical, &ui_state, &effective);
        let parent_children = squashed["Parent Site"]["children"]
            .as_object()
            .expect("parent should keep children");
        assert!(parent_children.get("Inactive Parent Attachment").is_none());
        assert!(parent_children.get("Child Site").is_some());
    }

    #[test]
    fn cross_site_move_anchors_under_peer_attachment_not_child_owned_attachment() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Site Alpha": {
                "children": {
                    "Alpha-Beta-60": {
                        "children": {},
                        "downloadBandwidthMbps": 940,
                        "id": "alpha-beta-60",
                        "name": "Alpha-Beta-60",
                        "parent_site": "Site Alpha",
                        "type": "AP",
                        "uploadBandwidthMbps": 940
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "site-alpha",
                "name": "Site Alpha",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            },
            "Site Beta": {
                "children": {
                    "Beta - Alpha 60": {
                        "children": {},
                        "downloadBandwidthMbps": 940,
                        "id": "beta-alpha-60",
                        "name": "Beta - Alpha 60",
                        "parent_site": "Site Beta",
                        "type": "AP",
                        "uploadBandwidthMbps": 940
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "site-beta",
                "name": "Site Beta",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });

        let mut move_attachment = sample_attachment_option("beta-alpha-60", "Beta - Alpha 60");
        move_attachment.peer_attachment_id = Some("alpha-beta-60".to_string());
        move_attachment.peer_attachment_name = Some("Alpha-Beta-60".to_string());
        move_attachment.download_bandwidth_mbps = Some(940);
        move_attachment.upload_bandwidth_mbps = Some(940);
        move_attachment.capacity_mbps = Some(940);

        let moved = apply_effective_topology_to_network_json(
            &config,
            &canonical,
            &TopologyEditorStateFile {
                schema_version: 1,
                source: "uisp/full2".to_string(),
                generated_unix: None,
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "site-beta".to_string(),
                    node_name: "Site Beta".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-alpha".to_string()),
                    current_parent_node_name: Some("Site Alpha".to_string()),
                    current_attachment_id: Some("beta-alpha-60".to_string()),
                    current_attachment_name: Some("Beta - Alpha 60".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-alpha".to_string(),
                        parent_node_name: "Site Alpha".to_string(),
                        attachment_options: vec![move_attachment],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                }],
            },
            &TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: None,
                canonical_generated_unix: None,
                health_generated_unix: None,
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "site-beta".to_string(),
                    logical_parent_node_id: "site-alpha".to_string(),
                    preferred_attachment_id: Some("beta-alpha-60".to_string()),
                    effective_attachment_id: Some("beta-alpha-60".to_string()),
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![TopologyEffectiveAttachmentState {
                        attachment_id: "beta-alpha-60".to_string(),
                        health_status: TopologyAttachmentHealthStatus::Healthy,
                        health_reason: None,
                        suppressed_until_unix: None,
                        probe_enabled: false,
                        probeable: false,
                        effective_selected: true,
                    }],
                }],
            },
        );

        assert!(moved.get("Site Beta").is_none());
        let matt_children = moved["Site Alpha"]["children"]
            .as_object()
            .expect("Site Alpha should keep children");
        let beta_site = matt_children
            .get("Site Beta")
            .and_then(Value::as_object)
            .expect("Site Beta should remain visible under Site Alpha after squashing");
        assert_eq!(beta_site["id"].as_str(), Some("site-beta"));
        assert_eq!(beta_site["parent_site"].as_str(), Some("Site Alpha"));
        assert_eq!(
            beta_site["active_attachment_name"].as_str(),
            Some("Alpha-Beta-60")
        );
        let beta_children = beta_site["children"]
            .as_object()
            .expect("Site Beta subtree should keep its children");
        assert!(beta_children.get("Beta - Alpha 60").is_some());
    }

    #[test]
    fn duplicate_device_candidates_do_not_block_valid_site_override_publish() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;

        let canonical_network = json!({
            "Site Alpha": {
                "children": {
                    "Alpha-Beta-60": {
                        "children": {},
                        "downloadBandwidthMbps": 940,
                        "id": "alpha-beta-60",
                        "name": "Alpha-Beta-60",
                        "parent_site": "Site Alpha",
                        "type": "AP",
                        "uploadBandwidthMbps": 940
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "site-alpha",
                "name": "Site Alpha",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            },
            "Site Beta": {
                "children": {
                    "Beta - Alpha 60": {
                        "children": {},
                        "downloadBandwidthMbps": 940,
                        "id": "beta-alpha-60",
                        "name": "Beta - Alpha 60",
                        "parent_site": "Site Beta",
                        "type": "AP",
                        "uploadBandwidthMbps": 940
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "site-beta",
                "name": "Site Beta",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });

        let mut beta_alpha_option = sample_attachment_option("beta-alpha-60", "Beta - Alpha 60");
        beta_alpha_option.peer_attachment_id = Some("alpha-beta-60".to_string());
        beta_alpha_option.peer_attachment_name = Some("Alpha-Beta-60".to_string());
        beta_alpha_option.download_bandwidth_mbps = Some(940);
        beta_alpha_option.upload_bandwidth_mbps = Some(940);
        beta_alpha_option.capacity_mbps = Some(940);

        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-beta".to_string(),
                    node_name: "Site Beta".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-gamma".to_string()),
                    current_parent_node_name: Some("Site Gamma".to_string()),
                    current_attachment_id: Some("beta-gamma-60".to_string()),
                    current_attachment_name: Some("Beta - Gamma 60".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-alpha".to_string(),
                        parent_node_name: "Site Alpha".to_string(),
                        attachment_options: vec![beta_alpha_option.clone()],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "beta-alpha-60".to_string(),
                    node_name: "Beta - Alpha 60".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-beta".to_string()),
                    current_parent_node_name: Some("Site Beta".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-beta".to_string(),
                        parent_node_name: "Site Beta".to_string(),
                        attachment_options: vec![],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "beta-alpha-60".to_string(),
                    node_name: "Beta - Alpha 60".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-beta".to_string()),
                    current_parent_node_name: Some("Site Beta".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![
                        TopologyAllowedParent {
                            parent_node_id: "site-alpha".to_string(),
                            parent_node_name: "Site Alpha".to_string(),
                            attachment_options: vec![beta_alpha_option.clone()],
                            all_attachments_suppressed: false,
                            has_probe_unavailable_attachments: false,
                        },
                        TopologyAllowedParent {
                            parent_node_id: "site-beta".to_string(),
                            parent_node_name: "Site Beta".to_string(),
                            attachment_options: vec![],
                            all_attachments_suppressed: false,
                            has_probe_unavailable_attachments: false,
                        },
                    ],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let mut overrides = TopologyOverridesFile::default();
        overrides.set_override_return_changed(
            "site-beta".to_string(),
            "Site Beta".to_string(),
            "site-alpha".to_string(),
            "Site Alpha".to_string(),
            TopologyAttachmentMode::Auto,
            Vec::new(),
        );

        let artifacts = build_effective_topology_artifacts(
            &config,
            &canonical,
            &overrides,
            &TopologyAttachmentHealthStateFile::default(),
            Some(&canonical_network),
        )
        .expect("duplicate device candidates should normalize before validation");

        assert_eq!(
            artifacts
                .effective
                .nodes
                .iter()
                .filter(|node| node.node_id == "beta-alpha-60")
                .count(),
            1
        );
        let moved = artifacts
            .effective_network
            .expect("effective network should be published");
        assert!(moved.is_object());
    }

    #[test]
    fn effective_topology_validation_rejects_missing_site() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "uisp:site:site-beta".to_string(),
                node_name: "Site Beta".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("uisp:site:site-alpha".to_string()),
                current_parent_node_name: Some("Site Alpha".to_string()),
                current_attachment_id: Some("beta-alpha-60".to_string()),
                current_attachment_name: Some("Beta - Alpha 60".to_string()),
                can_move: true,
                allowed_parents: vec![],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "uisp:site:site-beta".to_string(),
                logical_parent_node_id: "uisp:site:site-alpha".to_string(),
                preferred_attachment_id: Some("beta-alpha-60".to_string()),
                effective_attachment_id: Some("beta-alpha-60".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![],
            }],
        };
        let exported = json!({
            "Site Alpha": {
                "children": {},
                "id": "uisp:site:site-alpha",
                "name": "Site Alpha",
                "type": "Site"
            }
        });

        let config = Config::default();
        let canonical_network = json!({
            "Site Alpha": {
                "children": {
                    "Site Beta": {
                        "children": {},
                        "id": "uisp:site:site-beta",
                        "name": "Site Beta",
                        "type": "Site"
                    }
                },
                "id": "uisp:site:site-alpha",
                "name": "Site Alpha",
                "type": "Site"
            }
        });

        let errors = validate_effective_topology_network(
            &config,
            &canonical_network,
            &ui_state,
            &effective,
            &exported,
        )
        .expect_err("missing site should fail validation");
        assert!(errors.iter().any(|error| error.contains("Site Beta")));
    }

    #[test]
    fn effective_topology_validation_rejects_site_parent_cycles() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "uisp:site:site-a".to_string(),
                    node_name: "Site A".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("uisp:site:site-b".to_string()),
                    current_parent_node_name: Some("Site B".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "uisp:site:site-b".to_string(),
                    node_name: "Site B".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("uisp:site:site-a".to_string()),
                    current_parent_node_name: Some("Site A".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "uisp:site:site-a".to_string(),
                    logical_parent_node_id: "uisp:site:site-b".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "uisp:site:site-b".to_string(),
                    logical_parent_node_id: "uisp:site:site-a".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
            ],
        };
        let exported = json!({
            "Site A": {
                "children": {},
                "id": "uisp:site:site-a",
                "name": "Site A",
                "type": "Site"
            },
            "Site B": {
                "children": {},
                "id": "uisp:site:site-b",
                "name": "Site B",
                "type": "Site"
            }
        });

        let config = Config::default();
        let errors = validate_effective_topology_network(
            &config, &exported, &ui_state, &effective, &exported,
        )
        .expect_err("site-parent cycle should fail validation");
        assert!(errors.iter().any(|error| error.contains("parent cycle")));
    }

    #[test]
    fn effective_topology_validation_rejects_invalid_attachment_for_selected_parent() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "uisp:site:site-beta".to_string(),
                node_name: "Site Beta".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("uisp:site:site-alpha".to_string()),
                current_parent_node_name: Some("Site Alpha".to_string()),
                current_attachment_id: Some("alpha-beta-60".to_string()),
                current_attachment_name: Some("Alpha-Beta-60".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "uisp:site:site-alpha".to_string(),
                    parent_node_name: "Site Alpha".to_string(),
                    attachment_options: vec![sample_attachment_option(
                        "alpha-beta-60",
                        "Alpha-Beta-60",
                    )],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "uisp:site:site-beta".to_string(),
                logical_parent_node_id: "uisp:site:site-alpha".to_string(),
                preferred_attachment_id: Some("alpha-beta-60".to_string()),
                effective_attachment_id: Some("beta-alpha-60".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![],
            }],
        };
        let exported = json!({
            "Site Alpha": {
                "children": {
                    "Site Beta": {
                        "children": {},
                        "id": "uisp:site:site-beta",
                        "name": "Site Beta",
                        "type": "Site"
                    }
                },
                "id": "uisp:site:site-alpha",
                "name": "Site Alpha",
                "type": "Site"
            }
        });

        let config = Config::default();
        let errors = validate_effective_topology_network(
            &config, &exported, &ui_state, &effective, &exported,
        )
        .expect_err("invalid attachment should fail validation");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("invalid attachment"))
        );
    }

    #[test]
    fn effective_topology_validation_accepts_fixed_parent_nodes_without_allowed_parents() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/ap_site".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-root".to_string(),
                    node_name: "Site Root".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "ap-child".to_string(),
                    node_name: "AP Child".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Site Root".to_string()),
                    current_attachment_id: Some("legacy-attachment".to_string()),
                    current_attachment_name: Some("Legacy Attachment".to_string()),
                    can_move: false,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "site-root".to_string(),
                    logical_parent_node_id: String::new(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "ap-child".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
                    preferred_attachment_id: Some("legacy-attachment".to_string()),
                    effective_attachment_id: Some("legacy-attachment".to_string()),
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
            ],
        };
        let exported = json!({
            "Site Root": {
                "children": {
                    "AP Child": {
                        "children": {},
                        "id": "ap-child",
                        "name": "AP Child",
                        "parent_site": "Site Root",
                        "type": "AP"
                    }
                },
                "id": "site-root",
                "name": "Site Root",
                "type": "Site"
            }
        });

        let config = Config::default();
        validate_effective_topology_network(&config, &exported, &ui_state, &effective, &exported)
            .expect("fixed-parent legacy nodes should validate");
    }

    #[test]
    fn compute_effective_state_keeps_fixed_parent_nodes_without_allowed_parents() {
        let config = Config::default();
        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/ap_site".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-root".to_string(),
                    node_name: "Site Root".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "ap-child".to_string(),
                    node_name: "AP Child".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Site Root".to_string()),
                    current_attachment_id: Some("legacy-attachment".to_string()),
                    current_attachment_name: Some("Legacy Attachment".to_string()),
                    can_move: false,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let effective = compute_effective_state(
            &config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        );

        assert_eq!(effective.nodes.len(), 2);
        let child = effective
            .nodes
            .iter()
            .find(|node| node.node_id == "ap-child")
            .expect("child node should remain in effective state");
        assert_eq!(child.logical_parent_node_id, "site-root");
        assert_eq!(
            child.preferred_attachment_id.as_deref(),
            Some("legacy-attachment")
        );
        assert_eq!(
            child.effective_attachment_id.as_deref(),
            Some("legacy-attachment")
        );
        assert!(child.attachments.is_empty());
        assert!(!child.all_attachments_suppressed);
    }

    #[test]
    fn compute_effective_state_does_not_infer_parent_for_native_integration_nodes() {
        let config = Config::default();
        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "python/full".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "splynx:site:child".to_string(),
                node_name: "Child Site".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: None,
                current_parent_node_name: None,
                current_attachment_id: None,
                current_attachment_name: None,
                can_move: false,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "splynx:site:parent".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };

        let effective = compute_effective_state(
            &config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        );

        assert_eq!(effective.nodes.len(), 1);
        let child = &effective.nodes[0];
        assert_eq!(child.node_id, "splynx:site:child");
        assert!(child.logical_parent_node_id.is_empty());
        assert!(child.preferred_attachment_id.is_none());
        assert!(child.effective_attachment_id.is_none());
        assert!(child.attachments.is_empty());
    }

    #[test]
    fn compute_effective_state_auto_prefers_dynamic_attachment_when_probes_disabled() {
        let config = Config::default();
        let mut dynamic_attachment =
            sample_attachment_option("dynamic-link", "WavePro-MREToRochester");
        dynamic_attachment.rate_source = TopologyAttachmentRateSource::DynamicIntegration;
        dynamic_attachment.capacity_mbps = Some(2700);
        dynamic_attachment.download_bandwidth_mbps = Some(2700);
        dynamic_attachment.upload_bandwidth_mbps = Some(2700);
        dynamic_attachment.local_probe_ip = Some("100.126.0.226".to_string());
        dynamic_attachment.probe_enabled = false;

        let mut static_attachment = sample_attachment_option("static-link", "4600C_MRE_To_ROCH");
        static_attachment.rate_source = TopologyAttachmentRateSource::Static;
        static_attachment.capacity_mbps = Some(8000);
        static_attachment.download_bandwidth_mbps = Some(8000);
        static_attachment.upload_bandwidth_mbps = Some(8000);
        static_attachment.local_probe_ip = Some("100.126.0.235".to_string());
        static_attachment.remote_probe_ip = Some("100.126.0.234".to_string());
        static_attachment.probe_enabled = false;

        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "site-mre".to_string(),
                node_name: "MRE".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("site-rochester".to_string()),
                current_parent_node_name: Some("7232 Rochester".to_string()),
                current_attachment_id: Some("static-link".to_string()),
                current_attachment_name: Some("4600C_MRE_To_ROCH".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "site-rochester".to_string(),
                    parent_node_name: "7232 Rochester".to_string(),
                    attachment_options: vec![
                        auto_attachment_option(),
                        dynamic_attachment,
                        static_attachment,
                    ],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueAuto,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };

        let effective = compute_effective_state(
            &config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        );

        let node = effective
            .nodes
            .iter()
            .find(|node| node.node_id == "site-mre")
            .expect("MRE node should remain in effective state");
        assert_eq!(
            node.preferred_attachment_id.as_deref(),
            Some("dynamic-link")
        );
        assert_eq!(
            node.effective_attachment_id.as_deref(),
            Some("dynamic-link")
        );
        assert!(node.fallback_reason.is_none());
    }

    #[test]
    fn effective_state_fallback_does_not_keep_old_parent_attachment_after_reparent() {
        use lqos_overrides::TopologyOverridesFile;

        let config = Config::default();
        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "uisp:site:site-beta".to_string(),
                node_name: "Site Beta".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("uisp:site:site-gamma".to_string()),
                current_parent_node_name: Some("Site Gamma".to_string()),
                current_attachment_id: Some("uisp:device:device-beta-gamma".to_string()),
                current_attachment_name: Some("Beta - Gamma MLO6".to_string()),
                can_move: true,
                allowed_parents: vec![
                    TopologyAllowedParent {
                        parent_node_id: "uisp:site:site-alpha".to_string(),
                        parent_node_name: "Site Alpha".to_string(),
                        attachment_options: vec![
                            TopologyAttachmentOption {
                                attachment_id: "auto".to_string(),
                                attachment_name: "Auto".to_string(),
                                attachment_kind: "auto".to_string(),
                                attachment_role: TopologyAttachmentRole::Unknown,
                                pair_id: None,
                                peer_attachment_id: None,
                                peer_attachment_name: None,
                                capacity_mbps: None,
                                download_bandwidth_mbps: None,
                                upload_bandwidth_mbps: None,
                                transport_cap_mbps: None,
                                transport_cap_reason: None,
                                rate_source: TopologyAttachmentRateSource::Unknown,
                                can_override_rate: false,
                                rate_override_disabled_reason: None,
                                has_rate_override: false,
                                local_probe_ip: None,
                                remote_probe_ip: None,
                                probe_enabled: false,
                                probeable: false,
                                health_status: TopologyAttachmentHealthStatus::Disabled,
                                health_reason: None,
                                suppressed_until_unix: None,
                                effective_selected: false,
                            },
                            TopologyAttachmentOption {
                                attachment_id: "uisp:device:device-beta-alpha".to_string(),
                                attachment_name: "Beta - Alpha 60".to_string(),
                                attachment_kind: "device".to_string(),
                                attachment_role: TopologyAttachmentRole::PtpBackhaul,
                                pair_id: None,
                                peer_attachment_id: Some(
                                    "uisp:device:device-alpha-beta".to_string(),
                                ),
                                peer_attachment_name: Some("Alpha-Beta-60".to_string()),
                                capacity_mbps: Some(940),
                                download_bandwidth_mbps: Some(940),
                                upload_bandwidth_mbps: Some(940),
                                transport_cap_mbps: None,
                                transport_cap_reason: None,
                                rate_source: TopologyAttachmentRateSource::DynamicIntegration,
                                can_override_rate: false,
                                rate_override_disabled_reason: None,
                                has_rate_override: false,
                                local_probe_ip: Some("10.1.11.126".to_string()),
                                remote_probe_ip: Some("10.1.11.125".to_string()),
                                probe_enabled: false,
                                probeable: false,
                                health_status: TopologyAttachmentHealthStatus::Disabled,
                                health_reason: None,
                                suppressed_until_unix: None,
                                effective_selected: false,
                            },
                        ],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    },
                    TopologyAllowedParent {
                        parent_node_id: "uisp:site:site-gamma".to_string(),
                        parent_node_name: "Site Gamma".to_string(),
                        attachment_options: vec![
                            TopologyAttachmentOption {
                                attachment_id: "auto".to_string(),
                                attachment_name: "Auto".to_string(),
                                attachment_kind: "auto".to_string(),
                                attachment_role: TopologyAttachmentRole::Unknown,
                                pair_id: None,
                                peer_attachment_id: None,
                                peer_attachment_name: None,
                                capacity_mbps: None,
                                download_bandwidth_mbps: None,
                                upload_bandwidth_mbps: None,
                                transport_cap_mbps: None,
                                transport_cap_reason: None,
                                rate_source: TopologyAttachmentRateSource::Unknown,
                                can_override_rate: false,
                                rate_override_disabled_reason: None,
                                has_rate_override: false,
                                local_probe_ip: None,
                                remote_probe_ip: None,
                                probe_enabled: false,
                                probeable: false,
                                health_status: TopologyAttachmentHealthStatus::Disabled,
                                health_reason: None,
                                suppressed_until_unix: None,
                                effective_selected: false,
                            },
                            TopologyAttachmentOption {
                                attachment_id: "uisp:device:device-beta-gamma".to_string(),
                                attachment_name: "Beta - Gamma MLO6".to_string(),
                                attachment_kind: "device".to_string(),
                                attachment_role: TopologyAttachmentRole::PtpBackhaul,
                                pair_id: None,
                                peer_attachment_id: Some(
                                    "uisp:device:device-gamma-beta".to_string(),
                                ),
                                peer_attachment_name: Some("Gamma - Beta MLO6".to_string()),
                                capacity_mbps: Some(230),
                                download_bandwidth_mbps: Some(230),
                                upload_bandwidth_mbps: Some(230),
                                transport_cap_mbps: None,
                                transport_cap_reason: None,
                                rate_source: TopologyAttachmentRateSource::DynamicIntegration,
                                can_override_rate: false,
                                rate_override_disabled_reason: None,
                                has_rate_override: false,
                                local_probe_ip: Some("10.1.33.23".to_string()),
                                remote_probe_ip: Some("10.1.33.21".to_string()),
                                probe_enabled: false,
                                probeable: false,
                                health_status: TopologyAttachmentHealthStatus::Disabled,
                                health_reason: None,
                                suppressed_until_unix: None,
                                effective_selected: false,
                            },
                        ],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    },
                ],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let mut overrides = TopologyOverridesFile::default();
        overrides.set_override_return_changed(
            "uisp:site:site-beta".to_string(),
            "Site Beta".to_string(),
            "uisp:site:site-alpha".to_string(),
            "Site Alpha".to_string(),
            TopologyAttachmentMode::Auto,
            Vec::new(),
        );

        let effective = compute_effective_state(
            &config,
            &canonical,
            &overrides,
            &TopologyAttachmentHealthStateFile::default(),
        );
        let node = effective
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:site:site-beta")
            .expect("expected Site Beta effective state");

        assert_eq!(node.logical_parent_node_id, "uisp:site:site-alpha");
        assert_eq!(
            node.effective_attachment_id.as_deref(),
            Some("uisp:device:device-beta-alpha")
        );
        assert_ne!(
            node.effective_attachment_id.as_deref(),
            Some("uisp:device:device-beta-gamma")
        );
    }
}

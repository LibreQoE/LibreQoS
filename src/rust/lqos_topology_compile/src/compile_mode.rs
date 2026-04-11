use crate::bundle::{CompiledTopologyBundle, ImportedTopologyBundle};
use crate::validation::validate_compiled_bundle;
use anyhow::Result;
use lqos_config::{
    CircuitAnchor, CircuitAnchorsFile, ConfigShapedDevices, ShapedDevice,
    TopologyCanonicalIngressKind, TopologyCanonicalNode, TopologyCanonicalStateFile,
    TopologyEditorNode, TopologyEditorStateFile, TopologyParentCandidatesFile,
    topology_ingress_identity_from_tokens,
};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Selects which topology projection to compile from the imported bundle.
pub enum TopologyCompileMode {
    /// Preserve the richest imported topology representation.
    Full2,
    /// Preserve the legacy full topology representation.
    Full,
    /// Project to a site -> AP tree.
    ApSite,
    /// Project to AP-only nodes.
    ApOnly,
    /// Collapse all topology structure.
    Flat,
}

#[derive(Debug, Error)]
/// Errors returned while compiling one topology mode.
pub enum TopologyCompileError {
    /// The imported topology bundle was missing required facts.
    #[error("Unable to compile topology mode: {0}")]
    Compile(String),
}

#[derive(Clone, Debug)]
struct ExportedNode {
    id: String,
    name: String,
    kind: String,
    parent_id: Option<String>,
    download_mbps: Option<u64>,
    upload_mbps: Option<u64>,
}

#[derive(Clone)]
struct CircuitProjection {
    node_id: String,
    node_name: String,
}

const GENERATED_UNATTACHED_SITE_ID: &str = "libreqos:generated:site:unattached";
const GENERATED_UNATTACHED_SITE_NAME: &str = "LibreQoS Unattached [Site]";
const GENERATED_UNATTACHED_AP_ID: &str = "libreqos:generated:ap:unattached";
const GENERATED_UNATTACHED_AP_NAME: &str = "LibreQoS Unattached [AP]";

fn empty_parent_candidates(source: &str) -> TopologyParentCandidatesFile {
    TopologyParentCandidatesFile {
        source: source.to_string(),
        ingress_identity: None,
        nodes: Vec::new(),
    }
}

fn mode_name(mode: TopologyCompileMode) -> &'static str {
    match mode {
        TopologyCompileMode::Full2 => "full2",
        TopologyCompileMode::Full => "full",
        TopologyCompileMode::ApSite => "ap_site",
        TopologyCompileMode::ApOnly => "ap_only",
        TopologyCompileMode::Flat => "flat",
    }
}

fn compiled_source(import_source: &str, mode: TopologyCompileMode) -> String {
    let prefix = import_source
        .split('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("compiled");
    let mode_name = match mode {
        TopologyCompileMode::Full2 => "full2",
        TopologyCompileMode::Full => "full",
        TopologyCompileMode::ApSite => "ap_site",
        TopologyCompileMode::ApOnly => "ap_only",
        TopologyCompileMode::Flat => "flat",
    };
    format!("{prefix}/{mode_name}")
}

fn compiled_ingress_identity(
    imported: &ImportedTopologyBundle,
    mode: TopologyCompileMode,
) -> Option<String> {
    let base = imported.ingress_identity()?;
    topology_ingress_identity_from_tokens([
        format!("import:{base}"),
        format!("mode:{}", mode_name(mode)),
    ])
}

fn u64_field(node: &Map<String, Value>, key: &str) -> Option<u64> {
    match node.get(key) {
        Some(Value::Number(number)) => number
            .as_u64()
            .or_else(|| number.as_f64().map(|value| value.round() as u64)),
        _ => None,
    }
}

fn collect_exported_nodes(
    map: &Map<String, Value>,
    parent: Option<(&str, &str)>,
    by_id: &mut HashMap<String, ExportedNode>,
    by_name: &mut HashMap<String, String>,
) {
    for (key, value) in map {
        let Some(node) = value.as_object() else {
            continue;
        };
        let Some(node_id) = node
            .get("id")
            .and_then(Value::as_str)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let node_name = node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_string();
        let node_kind = node
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        by_name
            .entry(node_name.clone())
            .or_insert_with(|| node_id.to_string());
        by_id.insert(
            node_id.to_string(),
            ExportedNode {
                id: node_id.to_string(),
                name: node_name.clone(),
                kind: node_kind,
                parent_id: parent.map(|entry| entry.0.to_string()),
                download_mbps: u64_field(node, "downloadBandwidthMbps"),
                upload_mbps: u64_field(node, "uploadBandwidthMbps"),
            },
        );
        if let Some(children) = node.get("children").and_then(Value::as_object) {
            collect_exported_nodes(
                children,
                Some((node_id, node_name.as_str())),
                by_id,
                by_name,
            );
        }
    }
}

fn exported_index(
    network_json: &Value,
) -> (HashMap<String, ExportedNode>, HashMap<String, String>) {
    let mut by_id = HashMap::new();
    let mut by_name = HashMap::new();
    if let Some(map) = network_json.as_object() {
        collect_exported_nodes(map, None, &mut by_id, &mut by_name);
    }
    (by_id, by_name)
}

fn canonical_node_rates(node: &TopologyCanonicalNode) -> (Option<u64>, Option<u64>) {
    (
        node.rate_input
            .intrinsic_download_mbps
            .or(node.rate_input.legacy_imported_download_mbps),
        node.rate_input
            .intrinsic_upload_mbps
            .or(node.rate_input.legacy_imported_upload_mbps),
    )
}

fn native_projection_index(
    imported: &ImportedTopologyBundle,
) -> (HashMap<String, ExportedNode>, HashMap<String, String>) {
    let Some(canonical) = imported.native_canonical.as_ref() else {
        return exported_index(&imported.compatibility_network_json);
    };

    let mut by_id = HashMap::new();
    let mut by_name = HashMap::new();
    for node in &canonical.nodes {
        let (download_mbps, upload_mbps) = canonical_node_rates(node);
        let exported = ExportedNode {
            id: node.node_id.clone(),
            name: node.node_name.clone(),
            kind: node.node_kind.clone(),
            parent_id: node.current_parent_node_id.clone(),
            download_mbps,
            upload_mbps,
        };
        by_name
            .entry(exported.name.clone())
            .or_insert_with(|| exported.id.clone());
        by_id.insert(exported.id.clone(), exported);
    }

    if by_id.is_empty() {
        exported_index(&imported.compatibility_network_json)
    } else {
        (by_id, by_name)
    }
}

fn nearest_kind(
    nodes_by_id: &HashMap<String, ExportedNode>,
    start_id: &str,
    expected_kind: &str,
) -> Option<String> {
    let mut current = Some(start_id);
    let mut seen = HashSet::new();
    while let Some(candidate) = current {
        if !seen.insert(candidate.to_string()) {
            return None;
        }
        let node = nodes_by_id.get(candidate)?;
        if node.kind.eq_ignore_ascii_case(expected_kind) {
            return Some(node.id.clone());
        }
        current = node.parent_id.as_deref();
    }
    None
}

fn nearest_site_parent(
    nodes_by_id: &HashMap<String, ExportedNode>,
    start_id: &str,
) -> Option<String> {
    let mut current = nodes_by_id.get(start_id)?.parent_id.clone();
    let mut seen = HashSet::new();
    while let Some(candidate) = current {
        if !seen.insert(candidate.clone()) {
            return None;
        }
        let node = nodes_by_id.get(&candidate)?;
        if node.kind.eq_ignore_ascii_case("site") {
            return Some(node.id.clone());
        }
        current = node.parent_id.clone();
    }
    None
}

fn resolve_reference(
    parent_node_id: Option<&str>,
    parent_node_name: &str,
    nodes_by_id: &HashMap<String, ExportedNode>,
    ids_by_name: &HashMap<String, String>,
) -> Option<String> {
    if let Some(parent_id) = parent_node_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && nodes_by_id.contains_key(parent_id)
    {
        return Some(parent_id.to_string());
    }
    let trimmed_name = parent_node_name.trim();
    if trimmed_name.is_empty() {
        None
    } else {
        ids_by_name.get(trimmed_name).cloned()
    }
}

fn circuit_anchor_map(anchors: &CircuitAnchorsFile) -> HashMap<String, CircuitAnchor> {
    anchors
        .anchors
        .iter()
        .map(|anchor| (anchor.circuit_id.clone(), anchor.clone()))
        .collect()
}

fn circuit_projection_map<F>(
    imported: &ImportedTopologyBundle,
    transform: F,
) -> HashMap<String, CircuitProjection>
where
    F: Fn(&HashMap<String, ExportedNode>, &str) -> Option<String>,
{
    let (nodes_by_id, ids_by_name) = native_projection_index(imported);
    let anchors = circuit_anchor_map(&imported.circuit_anchors);
    let mut circuits = HashMap::<String, CircuitProjection>::new();
    for device in &imported.shaped_devices.devices {
        let anchor = anchors.get(&device.circuit_id);
        let candidate = anchor
            .and_then(|anchor| {
                resolve_reference(
                    Some(anchor.anchor_node_id.as_str()),
                    anchor.anchor_node_name.as_deref().unwrap_or_default(),
                    &nodes_by_id,
                    &ids_by_name,
                )
            })
            .or_else(|| {
                resolve_reference(
                    device.parent_node_id.as_deref(),
                    &device.parent_node,
                    &nodes_by_id,
                    &ids_by_name,
                )
            });
        let Some(target_id) = candidate.and_then(|node_id| transform(&nodes_by_id, &node_id))
        else {
            continue;
        };
        let Some(target) = nodes_by_id.get(&target_id) else {
            continue;
        };
        circuits
            .entry(device.circuit_id.clone())
            .or_insert_with(|| CircuitProjection {
                node_id: target.id.clone(),
                node_name: target.name.clone(),
            });
    }
    circuits
}

fn unresolved_circuits(
    imported: &ImportedTopologyBundle,
    projection: &HashMap<String, CircuitProjection>,
) -> Vec<(String, String)> {
    let mut unresolved = Vec::new();
    let mut seen = HashSet::new();
    for device in &imported.shaped_devices.devices {
        if projection.contains_key(&device.circuit_id) || !seen.insert(device.circuit_id.clone()) {
            continue;
        }
        unresolved.push((device.circuit_id.clone(), device.circuit_name.clone()));
    }
    unresolved
}

fn generated_ap_node() -> ExportedNode {
    ExportedNode {
        id: GENERATED_UNATTACHED_AP_ID.to_string(),
        name: GENERATED_UNATTACHED_AP_NAME.to_string(),
        kind: "AP".to_string(),
        parent_id: None,
        download_mbps: None,
        upload_mbps: None,
    }
}

fn generated_site_node() -> ExportedNode {
    ExportedNode {
        id: GENERATED_UNATTACHED_SITE_ID.to_string(),
        name: GENERATED_UNATTACHED_SITE_NAME.to_string(),
        kind: "Site".to_string(),
        parent_id: None,
        download_mbps: None,
        upload_mbps: None,
    }
}

fn exported_unattached_fallback_node(
    nodes_by_id: &HashMap<String, ExportedNode>,
) -> Option<ExportedNode> {
    let mut preferred_site = None;
    let mut preferred_ap = None;
    for node in nodes_by_id.values() {
        let id = node.id.to_ascii_lowercase();
        let name = node.name.to_ascii_lowercase();
        let is_unattached = id.contains("unattach")
            || name.contains("unattach")
            || id.contains("orphan")
            || name.contains("orphan");
        if !is_unattached {
            continue;
        }
        if node.kind.eq_ignore_ascii_case("site") {
            preferred_site = Some(node.clone());
            break;
        }
        if preferred_ap.is_none() && node.kind.eq_ignore_ascii_case("ap") {
            preferred_ap = Some(node.clone());
        }
    }
    preferred_site.or(preferred_ap)
}

fn attach_unresolved_to_projection(
    projection: &mut HashMap<String, CircuitProjection>,
    unresolved: Vec<(String, String)>,
    fallback_node: &ExportedNode,
) {
    for (circuit_id, _circuit_name) in unresolved {
        projection.insert(
            circuit_id,
            CircuitProjection {
                node_id: fallback_node.id.clone(),
                node_name: fallback_node.name.clone(),
            },
        );
    }
}

fn set_number(map: &mut Map<String, Value>, key: &str, value: Option<u64>, fallback: u64) {
    map.insert(
        key.to_string(),
        Value::Number(Number::from(value.unwrap_or(fallback))),
    );
}

fn ap_object(node: &ExportedNode, parent_site: Option<&str>) -> Value {
    let mut out = Map::new();
    out.insert("children".to_string(), Value::Object(Map::new()));
    out.insert("id".to_string(), node.id.clone().into());
    out.insert("name".to_string(), node.name.clone().into());
    out.insert("type".to_string(), "AP".into());
    set_number(&mut out, "downloadBandwidthMbps", node.download_mbps, 1);
    set_number(&mut out, "uploadBandwidthMbps", node.upload_mbps, 1);
    if let Some(parent_site) = parent_site.filter(|value| !value.is_empty()) {
        out.insert("parent_site".to_string(), parent_site.to_string().into());
    }
    Value::Object(out)
}

fn site_object(
    node_id: String,
    node_name: String,
    download_mbps: Option<u64>,
    upload_mbps: Option<u64>,
    children: Map<String, Value>,
) -> Value {
    let mut out = Map::new();
    out.insert("children".to_string(), Value::Object(children));
    out.insert("id".to_string(), node_id.into());
    out.insert("name".to_string(), node_name.into());
    out.insert("type".to_string(), "Site".into());
    set_number(&mut out, "downloadBandwidthMbps", download_mbps, 1);
    set_number(&mut out, "uploadBandwidthMbps", upload_mbps, 1);
    Value::Object(out)
}

fn make_shaped_devices(devices: Vec<ShapedDevice>) -> ConfigShapedDevices {
    let mut shaped_devices = ConfigShapedDevices::default();
    shaped_devices.replace_with_new_data(devices);
    shaped_devices
}

struct SanitizedTopologyIndex {
    node_names_by_id: HashMap<String, String>,
    parent_by_node_id: HashMap<String, Option<String>>,
}

fn sanitized_topology_index(imported: &ImportedTopologyBundle) -> SanitizedTopologyIndex {
    if let Some(native) = imported.native_canonical.as_ref() {
        return SanitizedTopologyIndex {
            node_names_by_id: native
                .nodes
                .iter()
                .map(|node| (node.node_id.clone(), node.node_name.clone()))
                .collect(),
            parent_by_node_id: native
                .nodes
                .iter()
                .map(|node| (node.node_id.clone(), node.current_parent_node_id.clone()))
                .collect(),
        };
    }
    if let Some(native) = imported.native_editor.as_ref() {
        return SanitizedTopologyIndex {
            node_names_by_id: native
                .nodes
                .iter()
                .map(|node| (node.node_id.clone(), node.node_name.clone()))
                .collect(),
            parent_by_node_id: native
                .nodes
                .iter()
                .map(|node| (node.node_id.clone(), node.current_parent_node_id.clone()))
                .collect(),
        };
    }

    let (nodes_by_id, _) = exported_index(&imported.compatibility_network_json);
    SanitizedTopologyIndex {
        node_names_by_id: nodes_by_id
            .iter()
            .map(|(node_id, node)| (node_id.clone(), node.name.clone()))
            .collect(),
        parent_by_node_id: nodes_by_id
            .iter()
            .map(|(node_id, node)| (node_id.clone(), node.parent_id.clone()))
            .collect(),
    }
}

fn sanitize_editor_nodes(
    nodes: Vec<TopologyEditorNode>,
    topology_index: &SanitizedTopologyIndex,
) -> Vec<TopologyEditorNode> {
    nodes
        .into_iter()
        .filter_map(|mut node| {
            if node.node_id.starts_with("libreqos:generated:graph:") {
                return None;
            }
            let exported_name = topology_index.node_names_by_id.get(&node.node_id)?;
            node.allowed_parents.retain(|parent| {
                topology_index
                    .node_names_by_id
                    .contains_key(parent.parent_node_id.as_str())
            });
            let legal_current_parent =
                node.current_parent_node_id
                    .as_deref()
                    .and_then(|parent_id| {
                        if (node.allowed_parents.is_empty()
                            && topology_index.node_names_by_id.contains_key(parent_id))
                            || node
                                .allowed_parents
                                .iter()
                                .any(|parent| parent.parent_node_id == parent_id)
                        {
                            Some(parent_id.to_string())
                        } else {
                            None
                        }
                    });
            let exported_parent = topology_index
                .parent_by_node_id
                .get(&node.node_id)
                .and_then(|parent| parent.as_deref())
                .and_then(|parent_id| {
                    if (node.allowed_parents.is_empty()
                        && topology_index.node_names_by_id.contains_key(parent_id))
                        || node
                            .allowed_parents
                            .iter()
                            .any(|parent| parent.parent_node_id == parent_id)
                    {
                        Some(parent_id.to_string())
                    } else {
                        None
                    }
                });
            let selected_parent = legal_current_parent.or(exported_parent);
            node.current_parent_node_name = selected_parent.as_deref().and_then(|parent_id| {
                node.allowed_parents
                    .iter()
                    .find(|parent| parent.parent_node_id == parent_id)
                    .map(|parent| parent.parent_node_name.clone())
                    .or_else(|| topology_index.node_names_by_id.get(parent_id).cloned())
            });
            node.current_parent_node_id = selected_parent;

            if node
                .current_attachment_id
                .as_deref()
                .is_some_and(|attachment_id| {
                    !topology_index.node_names_by_id.contains_key(attachment_id)
                })
            {
                node.current_attachment_id = None;
                node.current_attachment_name = None;
            }

            node.node_name = exported_name.clone();
            Some(node)
        })
        .collect()
}

fn sanitized_editor_state(
    source: &str,
    generated_unix: Option<u64>,
    imported: &ImportedTopologyBundle,
) -> TopologyEditorStateFile {
    let editor = imported.native_editor.clone().unwrap_or_else(|| {
        imported
            .native_canonical
            .clone()
            .unwrap_or_else(|| {
                TopologyCanonicalStateFile::from_legacy_network_json(
                    &imported.compatibility_network_json,
                )
            })
            .to_editor_state()
    });
    let editor = merge_legacy_parent_candidates(editor, imported.parent_candidates.as_ref());
    let topology_index = sanitized_topology_index(imported);

    TopologyEditorStateFile {
        source: source.to_string(),
        generated_unix,
        nodes: sanitize_editor_nodes(editor.nodes, &topology_index),
        ..editor
    }
}

fn merge_legacy_parent_candidates(
    mut editor: TopologyEditorStateFile,
    parent_candidates: Option<&TopologyParentCandidatesFile>,
) -> TopologyEditorStateFile {
    let Some(parent_candidates) = parent_candidates else {
        return editor;
    };
    let legacy_editor = TopologyEditorStateFile::from_legacy_parent_candidates(parent_candidates);
    let legacy_by_id = legacy_editor
        .nodes
        .into_iter()
        .map(|node| (node.node_id.clone(), node))
        .collect::<HashMap<_, _>>();

    for node in &mut editor.nodes {
        let Some(legacy) = legacy_by_id.get(&node.node_id) else {
            continue;
        };
        if node.current_parent_node_id.is_none() {
            node.current_parent_node_id = legacy.current_parent_node_id.clone();
            node.current_parent_node_name = legacy.current_parent_node_name.clone();
        }
        if !node.can_move && node.allowed_parents.is_empty() {
            node.can_move = legacy.can_move;
            node.allowed_parents = legacy.allowed_parents.clone();
        }
    }

    editor
}

fn projected_outputs(
    imported: ImportedTopologyBundle,
    projection: HashMap<String, CircuitProjection>,
    network_json: Value,
    source: String,
    mode: TopologyCompileMode,
) -> CompiledTopologyBundle {
    let ingress_identity = compiled_ingress_identity(&imported, mode);
    let mut shaped_devices = Vec::with_capacity(imported.shaped_devices.devices.len());
    let circuit_names = imported
        .shaped_devices
        .devices
        .iter()
        .map(|device| (device.circuit_id.clone(), device.circuit_name.clone()))
        .collect::<HashMap<_, _>>();
    for mut device in imported.shaped_devices.devices {
        if let Some(parent) = projection.get(&device.circuit_id) {
            device.parent_node = parent.node_name.clone();
            device.parent_node_id = Some(parent.node_id.clone());
        } else {
            device.parent_node.clear();
            device.parent_node_id = None;
        }
        device.anchor_node_id = None;
        shaped_devices.push(device);
    }

    let mut anchors = projection
        .into_iter()
        .map(|(circuit_id, parent)| CircuitAnchor {
            circuit_name: circuit_names.get(&circuit_id).cloned(),
            circuit_id,
            anchor_node_id: parent.node_id,
            anchor_node_name: Some(parent.node_name),
        })
        .collect::<Vec<_>>();
    anchors.sort_unstable_by(|left, right| left.circuit_id.cmp(&right.circuit_id));
    anchors.dedup_by(|left, right| left.circuit_id == right.circuit_id);

    let editor =
        TopologyCanonicalStateFile::from_legacy_network_json(&network_json).to_editor_state();
    let mut canonical = TopologyCanonicalStateFile::from_legacy_network_json(&network_json);
    canonical.source = source.clone();
    canonical.generated_unix = imported.generated_unix;
    canonical.ingress_identity = ingress_identity.clone();
    canonical.ingress_kind = TopologyCanonicalIngressKind::NativeIntegration;
    let mut editor = editor;
    editor.source = source.clone();
    editor.generated_unix = imported.generated_unix;
    editor.ingress_identity = ingress_identity.clone();

    CompiledTopologyBundle {
        source: source.clone(),
        generated_unix: imported.generated_unix,
        ingress_identity: ingress_identity.clone(),
        canonical,
        editor,
        parent_candidates: TopologyParentCandidatesFile {
            source: source.clone(),
            ingress_identity,
            nodes: Vec::new(),
        },
        compatibility_network_json: network_json,
        shaped_devices: make_shaped_devices(shaped_devices),
        circuit_anchors: CircuitAnchorsFile {
            schema_version: 1,
            source,
            generated_unix: imported.generated_unix,
            anchors,
        },
        ethernet_advisories: imported.ethernet_advisories,
    }
}

fn compile_full_like(
    imported: ImportedTopologyBundle,
    mode: TopologyCompileMode,
) -> Result<CompiledTopologyBundle> {
    let source = compiled_source(&imported.source, mode);
    let ingress_identity = compiled_ingress_identity(&imported, mode);
    let (exported_nodes_by_id, _) = exported_index(&imported.compatibility_network_json);
    let unresolved_fallback = exported_unattached_fallback_node(&exported_nodes_by_id);
    let mut editor = sanitized_editor_state(&source, imported.generated_unix, &imported);
    editor.ingress_identity = ingress_identity.clone();
    let mut canonical = TopologyCanonicalStateFile::from_editor_and_network(
        &editor,
        &imported.compatibility_network_json,
        TopologyCanonicalIngressKind::NativeIntegration,
    );
    canonical.source = source.clone();
    canonical.generated_unix = imported.generated_unix;
    canonical.ingress_identity = ingress_identity.clone();
    let mut parent_candidates = imported
        .parent_candidates
        .unwrap_or_else(|| empty_parent_candidates(&source));
    parent_candidates.source = source.clone();
    parent_candidates.ingress_identity = ingress_identity.clone();
    let mut shaped_devices = imported.shaped_devices.devices;
    if let Some(fallback) = unresolved_fallback {
        for device in &mut shaped_devices {
            let missing_parent = device.parent_node.trim().is_empty()
                && device.parent_node_id.is_none()
                && device.anchor_node_id.is_none();
            if missing_parent {
                device.parent_node = fallback.name.clone();
                device.parent_node_id = Some(fallback.id.clone());
            }
        }
    }
    Ok(CompiledTopologyBundle {
        source: source.clone(),
        generated_unix: imported.generated_unix,
        ingress_identity,
        canonical,
        editor,
        parent_candidates,
        compatibility_network_json: imported.compatibility_network_json,
        shaped_devices: make_shaped_devices(shaped_devices),
        circuit_anchors: CircuitAnchorsFile {
            schema_version: imported.circuit_anchors.schema_version,
            source,
            generated_unix: imported.generated_unix,
            anchors: imported.circuit_anchors.anchors,
        },
        ethernet_advisories: imported.ethernet_advisories,
    })
}

fn compile_flat(imported: ImportedTopologyBundle) -> CompiledTopologyBundle {
    let source = compiled_source(&imported.source, TopologyCompileMode::Flat);
    projected_outputs(
        imported,
        HashMap::new(),
        Value::Object(Map::new()),
        source,
        TopologyCompileMode::Flat,
    )
}

fn compile_ap_only(imported: ImportedTopologyBundle) -> Result<CompiledTopologyBundle> {
    let source = compiled_source(&imported.source, TopologyCompileMode::ApOnly);
    let mut projection = circuit_projection_map(&imported, |nodes_by_id, node_id| {
        nearest_kind(nodes_by_id, node_id, "AP")
    });
    let unresolved = unresolved_circuits(&imported, &projection);
    let (nodes_by_id, _) = native_projection_index(&imported);
    let mut root = BTreeMap::<String, Value>::new();
    let mut seen = HashSet::new();
    for parent in projection.values() {
        if !seen.insert(parent.node_id.clone()) {
            continue;
        }
        let Some(node) = nodes_by_id.get(&parent.node_id) else {
            continue;
        };
        root.insert(node.name.clone(), ap_object(node, None));
    }
    if !unresolved.is_empty() {
        let fallback_ap = generated_ap_node();
        attach_unresolved_to_projection(&mut projection, unresolved, &fallback_ap);
        root.insert(fallback_ap.name.clone(), ap_object(&fallback_ap, None));
    }
    Ok(projected_outputs(
        imported,
        projection,
        Value::Object(root.into_iter().collect()),
        source,
        TopologyCompileMode::ApOnly,
    ))
}

fn compile_ap_site(imported: ImportedTopologyBundle) -> Result<CompiledTopologyBundle> {
    let source = compiled_source(&imported.source, TopologyCompileMode::ApSite);
    let mut projection = circuit_projection_map(&imported, |nodes_by_id, node_id| {
        nearest_kind(nodes_by_id, node_id, "AP")
    });
    let unresolved = unresolved_circuits(&imported, &projection);
    let (nodes_by_id, _) = native_projection_index(&imported);
    let mut groups = BTreeMap::<String, (ExportedNode, BTreeMap<String, Value>)>::new();
    let mut synthetic_counter = 0usize;
    let mut seen = HashSet::new();
    for parent in projection.values() {
        if !seen.insert(parent.node_id.clone()) {
            continue;
        }
        let Some(ap) = nodes_by_id.get(&parent.node_id).cloned() else {
            continue;
        };
        let site_entry = nearest_site_parent(&nodes_by_id, &ap.id)
            .and_then(|site_id| nodes_by_id.get(&site_id).cloned())
            .unwrap_or_else(|| {
                synthetic_counter += 1;
                ExportedNode {
                    id: format!("libreqos:generated:ap_site:{synthetic_counter}"),
                    name: ap.name.clone(),
                    kind: "Site".to_string(),
                    parent_id: None,
                    download_mbps: ap.download_mbps,
                    upload_mbps: ap.upload_mbps,
                }
            });
        let entry = groups
            .entry(site_entry.id.clone())
            .or_insert_with(|| (site_entry.clone(), BTreeMap::new()));
        entry.1.insert(
            ap.name.clone(),
            ap_object(&ap, Some(site_entry.name.as_str())),
        );
    }
    if !unresolved.is_empty() {
        let fallback_site = generated_site_node();
        let fallback_ap = ExportedNode {
            parent_id: Some(fallback_site.id.clone()),
            ..generated_ap_node()
        };
        attach_unresolved_to_projection(&mut projection, unresolved, &fallback_ap);
        let entry = groups
            .entry(fallback_site.id.clone())
            .or_insert_with(|| (fallback_site.clone(), BTreeMap::new()));
        entry.1.insert(
            fallback_ap.name.clone(),
            ap_object(&fallback_ap, Some(fallback_site.name.as_str())),
        );
    }
    let mut root = BTreeMap::<String, Value>::new();
    for (_site_id, (site, children)) in groups {
        root.insert(
            site.name.clone(),
            site_object(
                site.id,
                site.name,
                site.download_mbps,
                site.upload_mbps,
                children.into_iter().collect(),
            ),
        );
    }
    Ok(projected_outputs(
        imported,
        projection,
        Value::Object(root.into_iter().collect()),
        source,
        TopologyCompileMode::ApSite,
    ))
}

/// Compiles one selectable topology mode from imported topology facts.
pub fn compile_topology(
    imported: ImportedTopologyBundle,
    mode: TopologyCompileMode,
) -> Result<CompiledTopologyBundle, TopologyCompileError> {
    let bundle = match mode {
        TopologyCompileMode::Full2 => compile_full_like(imported, TopologyCompileMode::Full2),
        TopologyCompileMode::Full => compile_full_like(imported, TopologyCompileMode::Full),
        TopologyCompileMode::ApOnly => compile_ap_only(imported),
        TopologyCompileMode::ApSite => compile_ap_site(imported),
        TopologyCompileMode::Flat => Ok(compile_flat(imported)),
    }
    .map_err(|err| TopologyCompileError::Compile(err.to_string()))?;
    validate_compiled_bundle(&bundle)
        .map_err(|err| TopologyCompileError::Compile(err.to_string()))?;
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::{
        GENERATED_UNATTACHED_AP_ID, GENERATED_UNATTACHED_AP_NAME, GENERATED_UNATTACHED_SITE_NAME,
        TopologyCompileMode, compile_topology,
    };
    use crate::bundle::ImportedTopologyBundle;
    use lqos_config::{
        CircuitAnchor, CircuitAnchorsFile, ConfigShapedDevices, ShapedDevice,
        TopologyAllowedParent, TopologyAttachmentHealthStatus, TopologyAttachmentOption,
        TopologyAttachmentRateSource, TopologyAttachmentRole, TopologyCanonicalIngressKind,
        TopologyCanonicalStateFile, TopologyEditorNode, TopologyEditorStateFile,
        TopologyParentCandidate, TopologyParentCandidatesFile, TopologyParentCandidatesNode,
    };
    use serde_json::json;

    fn shaped_devices(rows: Vec<ShapedDevice>) -> ConfigShapedDevices {
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(rows);
        shaped
    }

    fn sample_device(circuit_id: &str, parent_name: &str, parent_id: Option<&str>) -> ShapedDevice {
        ShapedDevice {
            circuit_id: circuit_id.to_string(),
            circuit_name: format!("Circuit {circuit_id}"),
            device_id: format!("device-{circuit_id}"),
            device_name: format!("Device {circuit_id}"),
            parent_node: parent_name.to_string(),
            parent_node_id: parent_id.map(ToOwned::to_owned),
            anchor_node_id: None,
            mac: String::new(),
            ipv4: Vec::new(),
            ipv6: Vec::new(),
            download_min_mbps: 10.0,
            upload_min_mbps: 10.0,
            download_max_mbps: 20.0,
            upload_max_mbps: 20.0,
            comment: String::new(),
            sqm_override: None,
            circuit_hash: 0,
            device_hash: 0,
            parent_hash: 0,
        }
    }

    fn imported_bundle() -> ImportedTopologyBundle {
        let network = json!({
            "Site Alpha": {
                "children": {
                    "AP North": {
                        "children": {},
                        "id": "ap-north",
                        "name": "AP North",
                        "type": "AP",
                        "downloadBandwidthMbps": 100,
                        "uploadBandwidthMbps": 50
                    },
                    "AP South": {
                        "children": {},
                        "id": "ap-south",
                        "name": "AP South",
                        "type": "AP",
                        "downloadBandwidthMbps": 90,
                        "uploadBandwidthMbps": 45
                    }
                },
                "id": "site-alpha",
                "name": "Site Alpha",
                "type": "Site",
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000
            }
        });
        ImportedTopologyBundle {
            source: "uisp/full2".to_string(),
            generated_unix: Some(1),
            ingress_identity: None,
            native_canonical: Some(TopologyCanonicalStateFile {
                schema_version: 1,
                source: "uisp/full2".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                ingress_kind: TopologyCanonicalIngressKind::NativeIntegration,
                nodes: Vec::new(),
                compatibility_network_json: network.clone(),
            }),
            native_editor: Some(TopologyEditorStateFile {
                schema_version: 1,
                source: "uisp/full2".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: Vec::new(),
            }),
            parent_candidates: None,
            compatibility_network_json: network,
            shaped_devices: shaped_devices(vec![
                sample_device("circuit-1", "AP North", Some("ap-north")),
                sample_device("circuit-2", "AP South", Some("ap-south")),
                sample_device("circuit-3", "Orphans", None),
            ]),
            circuit_anchors: CircuitAnchorsFile {
                schema_version: 1,
                source: "uisp/full2".to_string(),
                generated_unix: Some(1),
                anchors: vec![
                    CircuitAnchor {
                        circuit_id: "circuit-1".to_string(),
                        circuit_name: Some("Circuit circuit-1".to_string()),
                        anchor_node_id: "ap-north".to_string(),
                        anchor_node_name: Some("AP North".to_string()),
                    },
                    CircuitAnchor {
                        circuit_id: "circuit-2".to_string(),
                        circuit_name: Some("Circuit circuit-2".to_string()),
                        anchor_node_id: "ap-south".to_string(),
                        anchor_node_name: Some("AP South".to_string()),
                    },
                ],
            },
            ethernet_advisories: Vec::new(),
        }
    }

    fn imported_bundle_with_hidden_logical_parent() -> ImportedTopologyBundle {
        let mut bundle = imported_bundle();
        bundle.native_editor = Some(TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: Some(1),
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "ap-north".to_string(),
                node_name: "Wrong Name".to_string(),
                current_parent_node_id: Some("missing-site".to_string()),
                current_parent_node_name: Some("Missing Site".to_string()),
                current_attachment_id: Some("missing-attachment".to_string()),
                current_attachment_name: Some("Missing Attachment".to_string()),
                can_move: false,
                allowed_parents: Vec::new(),
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        });
        bundle
    }

    #[test]
    fn ap_only_drops_unresolved_orphan_parents() {
        let compiled = compile_topology(imported_bundle(), TopologyCompileMode::ApOnly)
            .expect("ap_only compilation should succeed");
        assert_eq!(compiled.circuit_anchors.anchors.len(), 3);
        assert!(compiled.compatibility_network_json[GENERATED_UNATTACHED_AP_NAME].is_object());
        let device = compiled
            .shaped_devices
            .devices
            .iter()
            .find(|device| device.circuit_id == "circuit-3")
            .expect("unresolved circuit should still be emitted");
        assert_eq!(
            device.parent_node_id.as_deref(),
            Some(GENERATED_UNATTACHED_AP_ID)
        );
        assert_eq!(device.parent_node, GENERATED_UNATTACHED_AP_NAME);
    }

    #[test]
    fn ap_site_groups_aps_under_nearest_site() {
        let compiled = compile_topology(imported_bundle(), TopologyCompileMode::ApSite)
            .expect("ap_site compilation should succeed");
        let root = compiled
            .compatibility_network_json
            .as_object()
            .expect("compiled ap_site network should be an object");
        assert!(root.get("Site Alpha").is_some());
        let site = root["Site Alpha"]
            .as_object()
            .expect("site should be an object");
        let children = site["children"]
            .as_object()
            .expect("site should have children");
        assert!(children.get("AP North").is_some());
        assert!(children.get("AP South").is_some());
        let fallback_site = root[GENERATED_UNATTACHED_SITE_NAME]
            .as_object()
            .expect("fallback site should be present for unresolved circuits");
        let fallback_children = fallback_site["children"]
            .as_object()
            .expect("fallback site should have children");
        assert!(
            fallback_children
                .get(GENERATED_UNATTACHED_AP_NAME)
                .is_some()
        );
        let device = compiled
            .shaped_devices
            .devices
            .iter()
            .find(|device| device.circuit_id == "circuit-3")
            .expect("unresolved circuit should still be emitted");
        assert_eq!(
            device.parent_node_id.as_deref(),
            Some(GENERATED_UNATTACHED_AP_ID)
        );
    }

    #[test]
    fn ap_site_projects_from_native_canonical_when_compatibility_tree_is_empty() {
        let mut imported = imported_bundle();
        imported.native_canonical = Some(TopologyCanonicalStateFile::from_legacy_network_json(
            &imported.compatibility_network_json,
        ));
        imported.compatibility_network_json = json!({});
        let compiled = compile_topology(imported, TopologyCompileMode::ApSite)
            .expect("ap_site compilation should succeed from native canonical data");
        let root = compiled
            .compatibility_network_json
            .as_object()
            .expect("compiled ap_site network should be an object");
        assert!(root.get("Site Alpha").is_some());
        let site = root["Site Alpha"]
            .as_object()
            .expect("site should be an object");
        let children = site["children"]
            .as_object()
            .expect("site should have children");
        assert!(children.get("AP North").is_some());
        assert!(children.get("AP South").is_some());
    }

    #[test]
    fn full_mode_sanitizes_hidden_logical_parent_references() {
        let compiled = compile_topology(
            imported_bundle_with_hidden_logical_parent(),
            TopologyCompileMode::Full,
        )
        .expect("full compilation should succeed");
        let node = compiled
            .editor
            .nodes
            .iter()
            .find(|node| node.node_id == "ap-north")
            .expect("compiled editor should retain exported node");
        assert_eq!(node.node_name, "AP North");
        assert_eq!(node.current_parent_node_id.as_deref(), Some("site-alpha"));
        assert_eq!(node.current_parent_node_name.as_deref(), Some("Site Alpha"));
        assert!(node.current_attachment_id.is_none());
        assert!(node.current_attachment_name.is_none());
    }

    #[test]
    fn full_mode_preserves_legal_native_logical_parent_over_export_hop() {
        let imported = ImportedTopologyBundle {
            source: "uisp/full2".to_string(),
            generated_unix: Some(1),
            ingress_identity: None,
            native_canonical: None,
            native_editor: Some(TopologyEditorStateFile {
                schema_version: 1,
                source: "uisp/full2".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "site-beta".to_string(),
                    node_name: "Site Beta".to_string(),
                    current_parent_node_id: Some("site-alpha".to_string()),
                    current_parent_node_name: Some("Site Alpha".to_string()),
                    current_attachment_id: Some("ap-alpha".to_string()),
                    current_attachment_name: Some("AP Alpha".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-alpha".to_string(),
                        parent_node_name: "Site Alpha".to_string(),
                        attachment_options: vec![TopologyAttachmentOption {
                            attachment_id: "ap-alpha".to_string(),
                            attachment_name: "AP Alpha".to_string(),
                            attachment_kind: "wireless".to_string(),
                            attachment_role: TopologyAttachmentRole::PtpBackhaul,
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
                            health_status: TopologyAttachmentHealthStatus::Healthy,
                            health_reason: None,
                            suppressed_until_unix: None,
                            effective_selected: false,
                        }],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                }],
            }),
            parent_candidates: None,
            compatibility_network_json: json!({
                "Site Alpha": {
                    "children": {
                        "AP Alpha": {
                            "children": {
                                "Site Beta": {
                                    "children": {},
                                    "id": "site-beta",
                                    "name": "Site Beta",
                                    "type": "Site"
                                }
                            },
                            "id": "ap-alpha",
                            "name": "AP Alpha",
                            "type": "AP"
                        }
                    },
                    "id": "site-alpha",
                    "name": "Site Alpha",
                    "type": "Site"
                }
            }),
            shaped_devices: shaped_devices(Vec::new()),
            circuit_anchors: CircuitAnchorsFile::default(),
            ethernet_advisories: Vec::new(),
        };

        let compiled = compile_topology(imported, TopologyCompileMode::Full)
            .expect("full compilation should preserve imported logical parent");
        let node = compiled
            .editor
            .nodes
            .iter()
            .find(|node| node.node_id == "site-beta")
            .expect("site beta should remain in editor state");

        assert_eq!(node.current_parent_node_id.as_deref(), Some("site-alpha"));
        assert_eq!(node.current_parent_node_name.as_deref(), Some("Site Alpha"));
        assert_eq!(node.current_attachment_id.as_deref(), Some("ap-alpha"));
        assert_eq!(node.current_attachment_name.as_deref(), Some("AP Alpha"));
    }

    #[test]
    fn full_mode_keeps_native_parent_when_compatibility_tree_flattens_backhaul_hops() {
        let imported = ImportedTopologyBundle {
            source: "uisp/full2".to_string(),
            generated_unix: Some(1),
            ingress_identity: None,
            native_canonical: None,
            native_editor: Some(TopologyEditorStateFile {
                schema_version: 1,
                source: "uisp/full2".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![
                    TopologyEditorNode {
                        node_id: "site-west".to_string(),
                        node_name: "WestRedd".to_string(),
                        current_parent_node_id: None,
                        current_parent_node_name: None,
                        current_attachment_id: None,
                        current_attachment_name: None,
                        can_move: false,
                        allowed_parents: Vec::new(),
                        preferred_attachment_id: None,
                        preferred_attachment_name: None,
                        effective_attachment_id: None,
                        effective_attachment_name: None,
                    },
                    TopologyEditorNode {
                        node_id: "aviat-west".to_string(),
                        node_name: "AVIAT_WestRedd".to_string(),
                        current_parent_node_id: Some("site-west".to_string()),
                        current_parent_node_name: Some("WestRedd".to_string()),
                        current_attachment_id: Some("site-west".to_string()),
                        current_attachment_name: Some("WestRedd".to_string()),
                        can_move: true,
                        allowed_parents: vec![TopologyAllowedParent {
                            parent_node_id: "site-west".to_string(),
                            parent_node_name: "WestRedd".to_string(),
                            attachment_options: vec![TopologyAttachmentOption {
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
                            }],
                            all_attachments_suppressed: false,
                            has_probe_unavailable_attachments: false,
                        }],
                        preferred_attachment_id: None,
                        preferred_attachment_name: None,
                        effective_attachment_id: None,
                        effective_attachment_name: None,
                    },
                ],
            }),
            parent_candidates: None,
            compatibility_network_json: json!({
                "AVIAT_WestRedd": {
                    "children": {},
                    "id": "aviat-west",
                    "name": "AVIAT_WestRedd",
                    "parent_site": "WestRedd",
                    "type": "AP"
                }
            }),
            shaped_devices: shaped_devices(Vec::new()),
            circuit_anchors: CircuitAnchorsFile::default(),
            ethernet_advisories: Vec::new(),
        };

        let compiled = compile_topology(imported, TopologyCompileMode::Full)
            .expect("full compilation should preserve native site parent");
        let node = compiled
            .editor
            .nodes
            .iter()
            .find(|node| node.node_id == "aviat-west")
            .expect("backhaul AP should remain in editor state");

        assert_eq!(node.current_parent_node_id.as_deref(), Some("site-west"));
        assert_eq!(node.current_parent_node_name.as_deref(), Some("WestRedd"));
    }

    #[test]
    fn full_mode_assigns_blank_parent_circuits_to_exported_unattached_site() {
        let mut imported = imported_bundle();
        imported.compatibility_network_json = json!({
            "Site Alpha": {
                "children": {
                    "AP North": {
                        "children": {},
                        "id": "ap-north",
                        "name": "AP North",
                        "type": "AP"
                    },
                    "AP South": {
                        "children": {},
                        "id": "ap-south",
                        "name": "AP South",
                        "type": "AP"
                    }
                },
                "id": "site-alpha",
                "name": "Site Alpha",
                "type": "Site"
            },
            "LibreQoS Unattached [Site]": {
                "children": {},
                "id": "libreqos:generated:splynx:site:unattached",
                "name": "LibreQoS Unattached [Site]",
                "type": "Site"
            }
        });
        imported.native_canonical = Some(TopologyCanonicalStateFile::from_legacy_network_json(
            &imported.compatibility_network_json,
        ));
        if let Some(device) = imported
            .shaped_devices
            .devices
            .iter_mut()
            .find(|device| device.circuit_id == "circuit-3")
        {
            device.parent_node.clear();
            device.parent_node_id = None;
            device.anchor_node_id = None;
        }

        let compiled = compile_topology(imported, TopologyCompileMode::Full)
            .expect("full compilation should succeed");
        let device = compiled
            .shaped_devices
            .devices
            .iter()
            .find(|device| device.circuit_id == "circuit-3")
            .expect("unresolved circuit should still be emitted");

        assert_eq!(device.parent_node, "LibreQoS Unattached [Site]");
        assert_eq!(
            device.parent_node_id.as_deref(),
            Some("libreqos:generated:splynx:site:unattached")
        );
    }

    #[test]
    fn full_mode_merges_legacy_parent_candidates_into_editor_state() {
        let mut imported = imported_bundle();
        imported.native_editor = None;
        imported.native_canonical = None;
        imported.parent_candidates = Some(TopologyParentCandidatesFile {
            source: "python/integration_common".to_string(),
            ingress_identity: None,
            nodes: vec![TopologyParentCandidatesNode {
                node_id: "ap-north".to_string(),
                node_name: "AP North".to_string(),
                current_parent_node_id: Some("site-alpha".to_string()),
                current_parent_node_name: Some("Site Alpha".to_string()),
                candidate_parents: vec![TopologyParentCandidate {
                    node_id: "site-alpha".to_string(),
                    node_name: "Site Alpha".to_string(),
                }],
            }],
        });

        let compiled =
            compile_topology(imported, TopologyCompileMode::Full).expect("full should compile");
        let node = compiled
            .editor
            .nodes
            .iter()
            .find(|node| node.node_id == "ap-north")
            .expect("ap north should remain in editor state");

        assert!(node.can_move);
        assert_eq!(node.allowed_parents.len(), 1);
        assert_eq!(node.current_parent_node_id.as_deref(), Some("site-alpha"));
        assert_eq!(node.current_parent_node_name.as_deref(), Some("Site Alpha"));
    }

    #[test]
    fn full_mode_excludes_generated_graph_sites_from_editor_state() {
        let network = json!({
            "Site Alpha": {
                "children": {
                    "(Generated Site) Customer Alpha": {
                        "children": {},
                        "id": "libreqos:generated:graph:site:customer-alpha",
                        "name": "(Generated Site) Customer Alpha",
                        "type": "Site"
                    },
                    "AP North": {
                        "children": {},
                        "id": "ap-north",
                        "name": "AP North",
                        "type": "AP"
                    }
                },
                "id": "site-alpha",
                "name": "Site Alpha",
                "type": "Site"
            }
        });
        let imported = ImportedTopologyBundle {
            source: "python/splynx".to_string(),
            generated_unix: Some(1),
            ingress_identity: None,
            native_canonical: None,
            native_editor: None,
            parent_candidates: None,
            compatibility_network_json: network,
            shaped_devices: shaped_devices(Vec::new()),
            circuit_anchors: CircuitAnchorsFile::default(),
            ethernet_advisories: Vec::new(),
        };

        let compiled =
            compile_topology(imported, TopologyCompileMode::Full).expect("full should compile");

        assert!(
            compiled
                .editor
                .nodes
                .iter()
                .all(|node| !node.node_id.starts_with("libreqos:generated:graph:"))
        );
        assert!(compiled.editor.find_node("site-alpha").is_some());
        assert!(compiled.editor.find_node("ap-north").is_some());
    }
}

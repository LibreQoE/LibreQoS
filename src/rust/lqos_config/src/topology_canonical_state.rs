use crate::{
    Config, TOPOLOGY_ATTACHMENT_AUTO_ID, TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME,
    TOPOLOGY_COMPILED_SHAPING_FILENAME, TOPOLOGY_EDITOR_STATE_FILENAME,
    TOPOLOGY_EFFECTIVE_NETWORK_FILENAME, TOPOLOGY_EFFECTIVE_STATE_FILENAME,
    TOPOLOGY_IMPORT_FILENAME, TOPOLOGY_RUNTIME_STATUS_FILENAME, TOPOLOGY_SHAPING_INPUTS_FILENAME,
    TopologyAllowedParent, TopologyEditorNode, TopologyEditorStateError, TopologyEditorStateFile,
    TopologyParentCandidatesError, TopologyParentCandidatesFile,
};
use lqos_utils::hash_to_i64;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::warn;

/// Runtime filename carrying canonical topology state used by `lqos_topology`.
pub const TOPOLOGY_CANONICAL_STATE_FILENAME: &str = "topology_canonical_state.json";

/// Source category for canonical topology state ingestion.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopologyCanonicalIngressKind {
    /// Canonical topology was emitted directly by an integration.
    #[default]
    NativeIntegration,
    /// Canonical topology was imported from a legacy `network.json` file.
    LegacyNetworkJson,
}

/// Classification of where canonical node rate inputs came from.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopologyCanonicalRateInputSource {
    /// The canonical node rate input came from the richest available attachment metadata.
    AttachmentMax,
    /// The canonical node rate input came from an imported legacy `network.json`.
    ImportedNetworkJson,
    /// The canonical node rate input came from the compatibility export tree.
    CompatibilityExport,
    /// The source could not be classified.
    #[default]
    Unknown,
}

/// Canonical per-node rate inputs used when compiling effective inherited rates.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopologyCanonicalRateInput {
    /// Best available canonical intrinsic download cap in Mbps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intrinsic_download_mbps: Option<u64>,
    /// Best available canonical intrinsic upload cap in Mbps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intrinsic_upload_mbps: Option<u64>,
    /// Download cap imported from legacy `network.json`, when canonical IR lacked richer data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_imported_download_mbps: Option<u64>,
    /// Upload cap imported from legacy `network.json`, when canonical IR lacked richer data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_imported_upload_mbps: Option<u64>,
    /// Where the chosen canonical rate inputs came from.
    #[serde(default)]
    pub source: TopologyCanonicalRateInputSource,
}

/// Canonical topology metadata for one node.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopologyCanonicalNode {
    /// Stable node identifier.
    pub node_id: String,
    /// Display name for the node.
    pub node_name: String,
    /// Node kind or type label.
    #[serde(default)]
    pub node_kind: String,
    /// Whether the canonical node is logical-only / virtual.
    #[serde(default)]
    pub is_virtual: bool,
    /// Current canonical logical parent node ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_parent_node_id: Option<String>,
    /// Current canonical logical parent node name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_parent_node_name: Option<String>,
    /// Current canonical concrete attachment ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_attachment_id: Option<String>,
    /// Current canonical concrete attachment label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_attachment_name: Option<String>,
    /// Whether operators may move this node.
    #[serde(default)]
    pub can_move: bool,
    /// Allowed logical parents and their attachment choices.
    #[serde(default)]
    pub allowed_parents: Vec<TopologyAllowedParent>,
    /// Canonical rate inputs used for effective rate compilation.
    #[serde(default)]
    pub rate_input: TopologyCanonicalRateInput,
}

/// Canonical topology state consumed by runtime topology compilation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TopologyCanonicalStateFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_topology_canonical_schema_version")]
    pub schema_version: u32,
    /// Human-readable source such as `uisp/full2` or `legacy/network.json`.
    #[serde(default)]
    pub source: String,
    /// Unix timestamp when the state was generated, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_unix: Option<u64>,
    /// Stable identity of the imported topology facts plus selected compile mode, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_identity: Option<String>,
    /// How this canonical state entered LibreQoS.
    #[serde(default)]
    pub ingress_kind: TopologyCanonicalIngressKind,
    /// Canonical node metadata.
    #[serde(default)]
    pub nodes: Vec<TopologyCanonicalNode>,
    /// Compatibility canonical tree export preserved for legacy readers and exporters.
    #[serde(default = "empty_json_object")]
    pub compatibility_network_json: Value,
}

impl Default for TopologyCanonicalStateFile {
    fn default() -> Self {
        Self {
            schema_version: default_topology_canonical_schema_version(),
            source: String::new(),
            generated_unix: None,
            ingress_identity: None,
            ingress_kind: TopologyCanonicalIngressKind::NativeIntegration,
            nodes: Vec::new(),
            compatibility_network_json: empty_json_object(),
        }
    }
}

/// Errors returned while reading or writing canonical topology state.
#[derive(Debug, Error)]
pub enum TopologyCanonicalStateError {
    /// Reading or writing the snapshot file failed.
    #[error("Unable to access topology canonical state file: {0}")]
    Io(#[from] std::io::Error),
    /// Serializing or deserializing the snapshot failed.
    #[error("Unable to parse topology canonical state JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
struct NetworkNodeSnapshot {
    export_name: String,
    node_type: String,
    is_virtual: bool,
    download_mbps: Option<u64>,
    upload_mbps: Option<u64>,
}

fn default_topology_canonical_schema_version() -> u32 {
    1
}

fn empty_json_object() -> Value {
    Value::Object(Map::new())
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

pub(crate) fn topology_ingress_fingerprint_from_tokens<I>(tokens: I) -> Option<String>
where
    I: IntoIterator<Item = String>,
{
    let mut identifiers = tokens
        .into_iter()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if identifiers.is_empty() {
        return None;
    }
    identifiers.sort_unstable();
    identifiers.dedup();

    let mut hasher = Sha256::new();
    hasher.update(b"libreqos-topology-ingress-v1");
    for identifier in identifiers {
        hasher.update([0]);
        hasher.update(identifier.as_bytes());
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// Returns a stable identity hash for imported topology facts and compile mode selection.
pub fn topology_ingress_identity_from_tokens<I>(tokens: I) -> Option<String>
where
    I: IntoIterator<Item = String>,
{
    let mut identifiers = tokens
        .into_iter()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if identifiers.is_empty() {
        return None;
    }
    identifiers.sort_unstable();
    identifiers.dedup();

    let mut hasher = Sha256::new();
    hasher.update(b"libreqos-topology-ingress-identity-v1");
    for identifier in identifiers {
        hasher.update([0]);
        hasher.update(identifier.as_bytes());
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn collect_network_json_ingress_tokens(map: &Map<String, Value>, out: &mut Vec<String>) {
    for (key, value) in map {
        let Some(node) = value.as_object() else {
            continue;
        };
        let node_name = node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_string();
        out.push(legacy_node_id(node, &node_name));
        if let Some(children) = node.get("children").and_then(Value::as_object) {
            collect_network_json_ingress_tokens(children, out);
        }
    }
}

pub(crate) fn topology_ingress_fingerprint_for_network_json(
    network_json: &Value,
) -> Option<String> {
    let map = network_json.as_object()?;
    let mut tokens = Vec::new();
    collect_network_json_ingress_tokens(map, &mut tokens);
    topology_ingress_fingerprint_from_tokens(tokens)
}

pub(crate) fn current_topology_ingress_identity(
    config: &Config,
) -> Result<Option<String>, TopologyCanonicalStateError> {
    let integration_ingress = topology_import_ingress_enabled(config);
    if integration_ingress {
        let topology_import_path = Path::new(&config.lqos_directory).join(TOPOLOGY_IMPORT_FILENAME);
        if topology_import_path.exists() {
            let raw = std::fs::read_to_string(topology_import_path)?;
            let imported = serde_json::from_str::<Value>(&raw)?;
            if let Some(identity) = imported
                .get("ingress_identity")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Ok(Some(identity.to_string()));
            }
        }
    }

    let parent_candidates =
        TopologyParentCandidatesFile::load(config).map_err(|err| match err {
            TopologyParentCandidatesError::Io(inner) => TopologyCanonicalStateError::Io(inner),
            TopologyParentCandidatesError::Json(inner) => TopologyCanonicalStateError::Json(inner),
        })?;
    if let Some(identity) = parent_candidates
        .ingress_identity
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(identity.to_string()));
    }

    if integration_ingress {
        return Ok(None);
    }

    let legacy_network_path = Path::new(&config.lqos_directory).join("network.json");
    if !legacy_network_path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(legacy_network_path)?;
    let network = serde_json::from_str::<Value>(&raw)?;
    Ok(topology_ingress_fingerprint_for_network_json(&network))
}

fn quarantine_target_path(path: &Path, stamp: u64, attempt: usize) -> PathBuf {
    let quarantine_directory =
        topology_stale_directory_path(path.parent().unwrap_or_else(|| Path::new(".")));
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if attempt == 0 {
        quarantine_directory.join(format!("{filename}.stale-{stamp}"))
    } else {
        quarantine_directory.join(format!("{filename}.stale-{stamp}-{attempt}"))
    }
}

fn topology_stale_directory_path(base: &Path) -> PathBuf {
    base.join(".topology_stale")
}

pub(crate) fn quarantine_stale_topology_state(
    config: &Config,
    reason: &str,
) -> Result<(), TopologyCanonicalStateError> {
    let base = Path::new(&config.lqos_directory);
    let quarantine_directory = topology_stale_directory_path(base);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let mut moved = Vec::new();
    let filenames = [
        TOPOLOGY_CANONICAL_STATE_FILENAME,
        TOPOLOGY_EDITOR_STATE_FILENAME,
        TOPOLOGY_COMPILED_SHAPING_FILENAME,
        TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME,
        TOPOLOGY_EFFECTIVE_NETWORK_FILENAME,
        TOPOLOGY_EFFECTIVE_STATE_FILENAME,
        TOPOLOGY_RUNTIME_STATUS_FILENAME,
        TOPOLOGY_SHAPING_INPUTS_FILENAME,
    ];

    std::fs::create_dir_all(&quarantine_directory)?;

    for filename in filenames {
        let path = base.join(filename);
        if !path.exists() {
            continue;
        }

        let mut attempt = 0usize;
        loop {
            let target = quarantine_target_path(&path, stamp, attempt);
            if target.exists() {
                attempt += 1;
                continue;
            }
            std::fs::rename(&path, &target)?;
            moved.push((
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(filename)
                    .to_string(),
                target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string(),
            ));
            break;
        }
    }

    if !moved.is_empty() {
        let summary = moved
            .iter()
            .map(|(from, to)| format!("{from} -> {to}"))
            .collect::<Vec<_>>()
            .join(", ");
        warn!(
            "Quarantined stale topology state because it does not match current ingress: {reason}. {summary}"
        );
    }

    Ok(())
}

#[derive(Clone)]
struct InsightLogicalNodeEntry {
    node_id: String,
    export_name: String,
    node_kind: String,
    is_virtual: bool,
    download_mbps: Option<u64>,
    upload_mbps: Option<u64>,
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

fn normalized_insight_node_kind(kind: &str) -> String {
    if kind.eq_ignore_ascii_case("ap") {
        "AP".to_string()
    } else if kind.is_empty() || kind.eq_ignore_ascii_case("site") {
        "Site".to_string()
    } else {
        kind.to_string()
    }
}

fn short_node_suffix(node_id: &str) -> &str {
    let short = node_id.rsplit(':').next().unwrap_or(node_id);
    &short[..short.len().min(8)]
}

fn unique_export_name(name: &str, node_id: &str, used_names: &mut HashSet<String>) -> String {
    if used_names.insert(name.to_string()) {
        return name.to_string();
    }

    let short = format!("{name} [{}]", short_node_suffix(node_id));
    if used_names.insert(short.clone()) {
        return short;
    }

    let fallback = format!("{name} [{node_id}]");
    used_names.insert(fallback.clone());
    fallback
}

fn build_insight_logical_entry_json(
    entry_id: &str,
    entries: &HashMap<String, InsightLogicalNodeEntry>,
    children_by_parent: &HashMap<String, Vec<String>>,
    visiting: &mut HashSet<String>,
) -> (Value, u64, u64) {
    let Some(entry) = entries.get(entry_id) else {
        return (Value::Object(Map::new()), 1, 1);
    };

    if !visiting.insert(entry_id.to_string()) {
        let download = entry.download_mbps.unwrap_or(1);
        let upload = entry.upload_mbps.unwrap_or(1);
        let mut map = Map::new();
        map.insert("name".into(), entry.export_name.clone().into());
        map.insert("id".into(), entry.node_id.clone().into());
        map.insert(
            "type".into(),
            normalized_insight_node_kind(&entry.node_kind).into(),
        );
        map.insert("downloadBandwidthMbps".into(), download.into());
        map.insert("uploadBandwidthMbps".into(), upload.into());
        map.insert("children".into(), Value::Object(Map::new()));
        return (Value::Object(map), download, upload);
    }

    let mut child_ids = children_by_parent
        .get(entry_id)
        .cloned()
        .unwrap_or_default();
    child_ids.sort_unstable_by(|left_id, right_id| {
        let left = entries
            .get(left_id)
            .expect("child entry should exist when sorting Insight logical tree");
        let right = entries
            .get(right_id)
            .expect("child entry should exist when sorting Insight logical tree");
        left.export_name
            .cmp(&right.export_name)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });

    let mut children = Map::new();
    let mut used_names = HashSet::new();
    let mut max_child_download = 0u64;
    let mut max_child_upload = 0u64;
    for child_id in child_ids {
        let child = entries
            .get(&child_id)
            .expect("child entry should exist when building Insight logical tree");
        let child_key = unique_export_name(&child.export_name, &child.node_id, &mut used_names);
        let (child_value, child_download, child_upload) =
            build_insight_logical_entry_json(&child_id, entries, children_by_parent, visiting);
        max_child_download = max_child_download.max(child_download);
        max_child_upload = max_child_upload.max(child_upload);
        children.insert(child_key, child_value);
    }

    visiting.remove(entry_id);

    let download = entry
        .download_mbps
        .or_else(|| (max_child_download > 0).then_some(max_child_download))
        .unwrap_or(1);
    let upload = entry
        .upload_mbps
        .or_else(|| (max_child_upload > 0).then_some(max_child_upload))
        .unwrap_or(1);

    let mut map = Map::new();
    map.insert("name".into(), entry.export_name.clone().into());
    map.insert("id".into(), entry.node_id.clone().into());
    map.insert(
        "type".into(),
        normalized_insight_node_kind(&entry.node_kind).into(),
    );
    map.insert("downloadBandwidthMbps".into(), download.into());
    map.insert("uploadBandwidthMbps".into(), upload.into());
    if entry.is_virtual {
        map.insert("virtual".into(), true.into());
    }
    map.insert("children".into(), Value::Object(children));

    (Value::Object(map), download, upload)
}

fn atomic_write_json<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), TopologyCanonicalStateError> {
    let raw = serde_json::to_string_pretty(value)?;
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

fn canonical_rate_input_from_network_snapshot(
    node: &TopologyEditorNode,
    snapshot: Option<&NetworkNodeSnapshot>,
) -> TopologyCanonicalRateInput {
    let mut explicit_download = None;
    let mut explicit_upload = None;
    for attachment in node
        .allowed_parents
        .iter()
        .flat_map(|parent| parent.attachment_options.iter())
        .filter(|attachment| attachment.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID)
    {
        if let Some(download) = attachment
            .download_bandwidth_mbps
            .or(attachment.capacity_mbps)
        {
            explicit_download =
                Some(explicit_download.map_or(download, |current: u64| current.max(download)));
        }
        if let Some(upload) = attachment
            .upload_bandwidth_mbps
            .or(attachment.capacity_mbps)
        {
            explicit_upload =
                Some(explicit_upload.map_or(upload, |current: u64| current.max(upload)));
        }
    }

    if explicit_download.is_some() || explicit_upload.is_some() {
        return TopologyCanonicalRateInput {
            intrinsic_download_mbps: explicit_download,
            intrinsic_upload_mbps: explicit_upload,
            legacy_imported_download_mbps: snapshot.and_then(|node| node.download_mbps),
            legacy_imported_upload_mbps: snapshot.and_then(|node| node.upload_mbps),
            source: TopologyCanonicalRateInputSource::AttachmentMax,
        };
    }

    TopologyCanonicalRateInput {
        intrinsic_download_mbps: snapshot.and_then(|node| node.download_mbps),
        intrinsic_upload_mbps: snapshot.and_then(|node| node.upload_mbps),
        legacy_imported_download_mbps: snapshot.and_then(|node| node.download_mbps),
        legacy_imported_upload_mbps: snapshot.and_then(|node| node.upload_mbps),
        source: if snapshot.is_some() {
            TopologyCanonicalRateInputSource::CompatibilityExport
        } else {
            TopologyCanonicalRateInputSource::Unknown
        },
    }
}

fn read_u64_field(node: &Map<String, Value>, key: &str) -> Option<u64> {
    node.get(key).and_then(|value| match value {
        Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                Some(value)
            } else if let Some(value) = number.as_f64() {
                (value >= 0.0).then_some(value.round() as u64)
            } else {
                None
            }
        }
        _ => None,
    })
}

fn build_network_node_index(
    map: &Map<String, Value>,
    by_id: &mut HashMap<String, NetworkNodeSnapshot>,
    by_name: &mut HashMap<String, NetworkNodeSnapshot>,
) {
    for (key, value) in map {
        let Some(node) = value.as_object() else {
            continue;
        };
        let name = node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_string();
        let snapshot = NetworkNodeSnapshot {
            export_name: key.to_string(),
            node_type: node
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            is_virtual: node
                .get("virtual")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            download_mbps: read_u64_field(node, "downloadBandwidthMbps"),
            upload_mbps: read_u64_field(node, "uploadBandwidthMbps"),
        };
        by_name.insert(name.clone(), snapshot.clone());
        if let Some(node_id) = node.get("id").and_then(Value::as_str) {
            by_id.insert(node_id.to_string(), snapshot.clone());
        }
        if let Some(children) = node.get("children").and_then(Value::as_object) {
            build_network_node_index(children, by_id, by_name);
        }
    }
}

fn network_node_snapshot_for_editor_node<'a>(
    node: &TopologyEditorNode,
    by_id: &'a HashMap<String, NetworkNodeSnapshot>,
    by_name: &'a HashMap<String, NetworkNodeSnapshot>,
) -> Option<&'a NetworkNodeSnapshot> {
    by_id
        .get(&node.node_id)
        .or_else(|| by_name.get(&node.node_name))
}

pub(crate) fn legacy_id_for_name(name: &str) -> String {
    format!(
        "libreqos:legacy-network-json:node:{:016x}",
        hash_to_i64(name) as u64
    )
}

fn legacy_node_id(node: &Map<String, Value>, fallback_name: &str) -> String {
    node.get("id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| legacy_id_for_name(fallback_name))
}

fn import_legacy_network_children(
    map: &Map<String, Value>,
    logical_parent: Option<(String, String)>,
    attachment_context: Option<(String, String)>,
    out: &mut Vec<TopologyCanonicalNode>,
) {
    for (key, value) in map {
        let Some(node) = value.as_object() else {
            continue;
        };
        let node_name = node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_string();
        let node_id = legacy_node_id(node, &node_name);
        let node_kind = node
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let is_virtual = node
            .get("virtual")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let download = read_u64_field(node, "downloadBandwidthMbps");
        let upload = read_u64_field(node, "uploadBandwidthMbps");

        let (
            current_parent_node_id,
            current_parent_node_name,
            current_attachment_id,
            current_attachment_name,
        ) = if node_kind.eq_ignore_ascii_case("site") {
            let logical = logical_parent.clone();
            let attachment = attachment_context.clone().or_else(|| logical.clone());
            (
                logical.as_ref().map(|entry| entry.0.clone()),
                logical.as_ref().map(|entry| entry.1.clone()),
                attachment.as_ref().map(|entry| entry.0.clone()),
                attachment.as_ref().map(|entry| entry.1.clone()),
            )
        } else {
            let parent = logical_parent.clone();
            let attachment = attachment_context.clone().or_else(|| parent.clone());
            (
                parent.as_ref().map(|entry| entry.0.clone()),
                parent.as_ref().map(|entry| entry.1.clone()),
                attachment.as_ref().map(|entry| entry.0.clone()),
                attachment.as_ref().map(|entry| entry.1.clone()),
            )
        };

        out.push(TopologyCanonicalNode {
            node_id: node_id.clone(),
            node_name: node_name.clone(),
            node_kind: node_kind.clone(),
            is_virtual,
            current_parent_node_id,
            current_parent_node_name,
            current_attachment_id,
            current_attachment_name,
            can_move: false,
            allowed_parents: Vec::new(),
            rate_input: TopologyCanonicalRateInput {
                intrinsic_download_mbps: download,
                intrinsic_upload_mbps: upload,
                legacy_imported_download_mbps: download,
                legacy_imported_upload_mbps: upload,
                source: TopologyCanonicalRateInputSource::ImportedNetworkJson,
            },
        });

        let next_logical_parent = if node_kind.eq_ignore_ascii_case("site") {
            Some((node_id.clone(), node_name.clone()))
        } else {
            logical_parent.clone()
        };
        let next_attachment_context = Some((node_id.clone(), node_name.clone()));
        if let Some(children) = node.get("children").and_then(Value::as_object) {
            import_legacy_network_children(
                children,
                next_logical_parent,
                next_attachment_context,
                out,
            );
        }
    }
}

/// Returns the path of the topology canonical state runtime file.
///
/// This function is pure: it has no side effects.
pub fn topology_canonical_state_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_CANONICAL_STATE_FILENAME)
}

impl TopologyCanonicalStateFile {
    /// Loads the canonical topology state if it exists.
    ///
    /// Side effects: reads `topology_canonical_state.json` from `config.lqos_directory`.
    pub fn load(config: &Config) -> Result<Self, TopologyCanonicalStateError> {
        let path = topology_canonical_state_path(config);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Loads canonical topology state, falling back to importing legacy `network.json`
    /// only when integration-import ingress is not enabled.
    ///
    /// Side effects: reads `topology_canonical_state.json` and, in DIY/manual mode when missing,
    /// may read `network.json` from `config.lqos_directory`.
    pub fn load_with_legacy_fallback(config: &Config) -> Result<Self, TopologyCanonicalStateError> {
        let state = Self::load(config)?;
        if !state.nodes.is_empty() {
            let is_current = match state.matches_current_ingress(config) {
                Ok(is_current) => is_current,
                Err(err) => {
                    warn!(
                        "Unable to validate topology canonical state against current ingress; preserving existing state: {err}"
                    );
                    true
                }
            };
            if is_current {
                return Ok(state);
            }
            quarantine_stale_topology_state(
                config,
                &format!(
                    "canonical topology source '{}' does not match current topology ingress identity",
                    state.source
                ),
            )?;
        }

        if topology_import_ingress_enabled(config) {
            return Ok(state);
        }

        let legacy_network_path = Path::new(&config.lqos_directory).join("network.json");
        if !legacy_network_path.exists() {
            return Ok(state);
        }
        let raw = std::fs::read_to_string(legacy_network_path)?;
        let network = serde_json::from_str::<Value>(&raw)?;
        Ok(Self::from_legacy_network_json(&network))
    }

    /// Saves the canonical topology state atomically.
    ///
    /// Side effects: writes `topology_canonical_state.json` into `config.lqos_directory`.
    pub fn save(&self, config: &Config) -> Result<(), TopologyCanonicalStateError> {
        atomic_write_json(&topology_canonical_state_path(config), self)
    }

    /// Finds canonical metadata for `node_id`.
    ///
    /// This function is pure: it has no side effects.
    pub fn find_node(&self, node_id: &str) -> Option<&TopologyCanonicalNode> {
        self.nodes.iter().find(|node| node.node_id == node_id)
    }

    /// Returns a stable fingerprint of the topology ingress this canonical state represents.
    ///
    /// This function is pure: it has no side effects.
    pub fn topology_ingress_fingerprint(&self) -> Option<String> {
        topology_ingress_fingerprint_from_tokens(self.nodes.iter().map(|node| {
            let node_id = node.node_id.trim();
            if node_id.is_empty() {
                legacy_id_for_name(&node.node_name)
            } else {
                node_id.to_string()
            }
        }))
    }

    /// Returns true if this canonical state still matches the current topology ingress identity.
    ///
    /// Side effects: reads topology ingress inputs from `config.lqos_directory`.
    pub fn matches_current_ingress(
        &self,
        config: &Config,
    ) -> Result<bool, TopologyCanonicalStateError> {
        if let Some(saved_identity) = self
            .ingress_identity
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let Some(current_identity) = current_topology_ingress_identity(config)? else {
                return Ok(true);
            };
            return Ok(saved_identity == current_identity);
        }

        let Some(current_fingerprint) = current_topology_ingress_identity(config)? else {
            return Ok(true);
        };
        let Some(saved_fingerprint) = self.topology_ingress_fingerprint() else {
            return Ok(false);
        };
        Ok(saved_fingerprint == current_fingerprint)
    }

    /// Converts canonical topology state into the UI-facing topology editor projection.
    ///
    /// This function is pure: it has no side effects.
    pub fn to_editor_state(&self) -> TopologyEditorStateFile {
        TopologyEditorStateFile {
            schema_version: self.schema_version,
            source: self.source.clone(),
            generated_unix: self.generated_unix,
            ingress_identity: self.ingress_identity.clone(),
            nodes: self
                .nodes
                .iter()
                .map(|node| TopologyEditorNode {
                    node_id: node.node_id.clone(),
                    node_name: node.node_name.clone(),
                    current_parent_node_id: node.current_parent_node_id.clone(),
                    current_parent_node_name: node.current_parent_node_name.clone(),
                    current_attachment_id: node.current_attachment_id.clone(),
                    current_attachment_name: node.current_attachment_name.clone(),
                    can_move: node.can_move,
                    allowed_parents: node.allowed_parents.clone(),
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                })
                .collect(),
        }
    }

    /// Returns the compatibility canonical `network.json` tree.
    ///
    /// This function is pure: it has no side effects.
    pub fn compatibility_network_json(&self) -> &Value {
        &self.compatibility_network_json
    }

    /// Builds an Insight-only logical topology tree from canonical parent relationships.
    ///
    /// This function is pure: it has no side effects.
    pub fn insight_topology_network_json(&self) -> Value {
        if self.nodes.is_empty() {
            return self.compatibility_network_json.clone();
        }

        let mut by_id = HashMap::new();
        let mut by_name = HashMap::new();
        if let Some(network_map) = self.compatibility_network_json.as_object() {
            build_network_node_index(network_map, &mut by_id, &mut by_name);
        }

        let mut entries = HashMap::<String, InsightLogicalNodeEntry>::new();
        for node in &self.nodes {
            let (download_mbps, upload_mbps) = canonical_node_rates(node);
            let export_name = by_id
                .get(&node.node_id)
                .or_else(|| by_name.get(&node.node_name))
                .map(|snapshot| snapshot.export_name.clone())
                .unwrap_or_else(|| node.node_name.clone());
            entries.insert(
                node.node_id.clone(),
                InsightLogicalNodeEntry {
                    node_id: node.node_id.clone(),
                    export_name,
                    node_kind: node.node_kind.clone(),
                    is_virtual: node.is_virtual,
                    download_mbps,
                    upload_mbps,
                },
            );
        }

        let mut children_by_parent = HashMap::<String, Vec<String>>::new();
        let mut seen_children = HashSet::<String>::new();
        for node in &self.nodes {
            let Some(parent_id) = node.current_parent_node_id.as_ref() else {
                continue;
            };
            if !entries.contains_key(parent_id) || parent_id == &node.node_id {
                continue;
            }
            children_by_parent
                .entry(parent_id.clone())
                .or_default()
                .push(node.node_id.clone());
            seen_children.insert(node.node_id.clone());
        }

        let mut root_ids = entries
            .keys()
            .filter(|entry_id| !seen_children.contains(*entry_id))
            .cloned()
            .collect::<Vec<_>>();
        root_ids.sort_unstable_by(|left_id, right_id| {
            let left = entries
                .get(left_id)
                .expect("root entry should exist when sorting Insight logical tree");
            let right = entries
                .get(right_id)
                .expect("root entry should exist when sorting Insight logical tree");
            left.export_name
                .cmp(&right.export_name)
                .then_with(|| left.node_id.cmp(&right.node_id))
        });

        let mut out = Map::new();
        let mut used_names = HashSet::new();
        let mut visiting = HashSet::new();
        for root_id in root_ids {
            let root = entries
                .get(&root_id)
                .expect("root entry should exist when building Insight logical tree");
            let export_name = unique_export_name(&root.export_name, &root.node_id, &mut used_names);
            let (value, _, _) = build_insight_logical_entry_json(
                &root_id,
                &entries,
                &children_by_parent,
                &mut visiting,
            );
            out.insert(export_name, value);
        }

        Value::Object(out)
    }

    /// Builds canonical state from integration-emitted editor state plus compatibility `network.json`.
    ///
    /// This function is pure: it has no side effects.
    pub fn from_editor_and_network(
        editor_state: &TopologyEditorStateFile,
        compatibility_network_json: &Value,
        ingress_kind: TopologyCanonicalIngressKind,
    ) -> Self {
        let mut by_id = HashMap::new();
        let mut by_name = HashMap::new();
        if let Some(network_map) = compatibility_network_json.as_object() {
            build_network_node_index(network_map, &mut by_id, &mut by_name);
        }

        let mut nodes = editor_state
            .nodes
            .iter()
            .map(|node| {
                let snapshot = network_node_snapshot_for_editor_node(node, &by_id, &by_name);
                TopologyCanonicalNode {
                    node_id: node.node_id.clone(),
                    node_name: node.node_name.clone(),
                    node_kind: snapshot
                        .map(|snapshot| snapshot.node_type.clone())
                        .unwrap_or_default(),
                    is_virtual: snapshot.is_some_and(|snapshot| snapshot.is_virtual),
                    current_parent_node_id: node.current_parent_node_id.clone(),
                    current_parent_node_name: node.current_parent_node_name.clone(),
                    current_attachment_id: node.current_attachment_id.clone(),
                    current_attachment_name: node.current_attachment_name.clone(),
                    can_move: node.can_move,
                    allowed_parents: node.allowed_parents.clone(),
                    rate_input: canonical_rate_input_from_network_snapshot(node, snapshot),
                }
            })
            .collect::<Vec<_>>();
        nodes.sort_unstable_by(|left, right| left.node_id.cmp(&right.node_id));

        Self {
            schema_version: editor_state.schema_version,
            source: editor_state.source.clone(),
            generated_unix: editor_state.generated_unix,
            ingress_identity: editor_state.ingress_identity.clone(),
            ingress_kind,
            nodes,
            compatibility_network_json: compatibility_network_json.clone(),
        }
    }

    /// Builds canonical state by importing a legacy `network.json` file.
    ///
    /// This function is pure: it has no side effects.
    pub fn from_legacy_network_json(network_json: &Value) -> Self {
        let mut nodes = Vec::new();
        if let Some(map) = network_json.as_object() {
            import_legacy_network_children(map, None, None, &mut nodes);
        }
        nodes.sort_unstable_by(|left, right| left.node_id.cmp(&right.node_id));
        Self {
            schema_version: default_topology_canonical_schema_version(),
            source: "legacy/network.json".to_string(),
            generated_unix: None,
            ingress_identity: topology_ingress_fingerprint_for_network_json(network_json),
            ingress_kind: TopologyCanonicalIngressKind::LegacyNetworkJson,
            nodes,
            compatibility_network_json: network_json.clone(),
        }
    }
}

impl From<TopologyCanonicalStateError> for TopologyEditorStateError {
    fn from(value: TopologyCanonicalStateError) -> Self {
        match value {
            TopologyCanonicalStateError::Io(err) => TopologyEditorStateError::Io(err),
            TopologyCanonicalStateError::Json(err) => TopologyEditorStateError::Json(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TOPOLOGY_CANONICAL_STATE_FILENAME, TopologyCanonicalIngressKind,
        TopologyCanonicalRateInputSource, TopologyCanonicalStateFile,
        current_topology_ingress_identity, legacy_id_for_name,
        topology_ingress_identity_from_tokens,
    };
    use crate::{
        Config, TOPOLOGY_EDITOR_STATE_FILENAME, TOPOLOGY_EFFECTIVE_NETWORK_FILENAME,
        TOPOLOGY_IMPORT_FILENAME, TOPOLOGY_PARENT_CANDIDATES_FILENAME, TopologyAllowedParent,
        TopologyAttachmentHealthStatus, TopologyAttachmentOption, TopologyAttachmentRateSource,
        TopologyAttachmentRole, TopologyCanonicalNode, TopologyEditorNode, TopologyEditorStateFile,
        TopologyParentCandidatesFile,
    };
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

    fn sample_attachment_option(
        attachment_id: &str,
        attachment_name: &str,
        download_bandwidth_mbps: u64,
        upload_bandwidth_mbps: u64,
    ) -> TopologyAttachmentOption {
        TopologyAttachmentOption {
            attachment_id: attachment_id.to_string(),
            attachment_name: attachment_name.to_string(),
            attachment_kind: "device".to_string(),
            attachment_role: TopologyAttachmentRole::PtpBackhaul,
            pair_id: None,
            peer_attachment_id: None,
            peer_attachment_name: None,
            capacity_mbps: Some(download_bandwidth_mbps.min(upload_bandwidth_mbps)),
            download_bandwidth_mbps: Some(download_bandwidth_mbps),
            upload_bandwidth_mbps: Some(upload_bandwidth_mbps),
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

    #[test]
    fn from_editor_and_network_prefers_attachment_rate_inputs() {
        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: Some(123),
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
                current_parent_node_id: Some("parent-site".to_string()),
                current_parent_node_name: Some("Parent Site".to_string()),
                current_attachment_id: Some("child-attachment".to_string()),
                current_attachment_name: Some("Child Attachment".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![sample_attachment_option(
                        "child-attachment",
                        "Child Attachment",
                        350,
                        275,
                    )],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };

        let compatibility_network = json!({
            "Child Site": {
                "children": {},
                "downloadBandwidthMbps": 900,
                "id": "child-site",
                "name": "Child Site",
                "parent_site": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 800
            }
        });

        let canonical = TopologyCanonicalStateFile::from_editor_and_network(
            &editor_state,
            &compatibility_network,
            TopologyCanonicalIngressKind::NativeIntegration,
        );
        let node = canonical
            .find_node("child-site")
            .expect("expected canonical node");
        assert_eq!(node.rate_input.intrinsic_download_mbps, Some(350));
        assert_eq!(node.rate_input.intrinsic_upload_mbps, Some(275));
        assert_eq!(node.rate_input.legacy_imported_download_mbps, Some(900));
        assert_eq!(node.rate_input.legacy_imported_upload_mbps, Some(800));
        assert_eq!(
            node.rate_input.source,
            TopologyCanonicalRateInputSource::AttachmentMax
        );
    }

    #[test]
    fn legacy_network_json_import_is_read_only_with_imported_rate_inputs() {
        let canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({
            "Parent Site": {
                "children": {
                    "Child Site": {
                        "children": {},
                        "downloadBandwidthMbps": 466,
                        "id": "child-site",
                        "name": "Child Site",
                        "parent_site": "Parent Site",
                        "type": "Site",
                        "uploadBandwidthMbps": 179
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        }));

        assert_eq!(
            canonical.ingress_kind,
            TopologyCanonicalIngressKind::LegacyNetworkJson
        );
        let child = canonical
            .find_node("child-site")
            .expect("expected child site");
        assert!(!child.can_move);
        assert!(child.allowed_parents.is_empty());
        assert_eq!(child.current_parent_node_id.as_deref(), Some("parent-site"));
        assert_eq!(
            child.rate_input.source,
            TopologyCanonicalRateInputSource::ImportedNetworkJson
        );
        assert_eq!(child.rate_input.intrinsic_download_mbps, Some(466));
        assert_eq!(child.rate_input.intrinsic_upload_mbps, Some(179));
        assert_eq!(child.rate_input.legacy_imported_download_mbps, Some(466));
        assert_eq!(child.rate_input.legacy_imported_upload_mbps, Some(179));
    }

    #[test]
    fn legacy_network_json_import_hashes_missing_node_ids_from_name() {
        let canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({
            "Parent Site": {
                "children": {
                    "Child Site": {
                        "children": {},
                        "downloadBandwidthMbps": 466,
                        "name": "Child Site",
                        "parent_site": "Parent Site",
                        "type": "Site",
                        "uploadBandwidthMbps": 179
                    }
                },
                "downloadBandwidthMbps": 1000,
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        }));

        let parent = canonical
            .nodes
            .iter()
            .find(|node| node.node_name == "Parent Site")
            .expect("expected parent site");
        let child = canonical
            .nodes
            .iter()
            .find(|node| node.node_name == "Child Site")
            .expect("expected child site");

        assert_eq!(parent.node_id, legacy_id_for_name("Parent Site"));
        assert_eq!(child.node_id, legacy_id_for_name("Child Site"));
        assert_eq!(
            child.current_parent_node_id.as_deref(),
            Some(parent.node_id.as_str())
        );
    }

    #[test]
    fn stale_canonical_state_is_quarantined_when_network_ingress_changes() {
        let lqos_directory = unique_temp_dir("lqos-config-canonical-quarantine");
        fs::write(
            lqos_directory.join("network.json"),
            serde_json::to_string_pretty(&json!({
                "Current AP": {
                    "children": {},
                    "id": "uisp:device:current-ap",
                    "type": "AP",
                    "downloadBandwidthMbps": 100,
                    "uploadBandwidthMbps": 100
                }
            }))
            .expect("network json should serialize"),
        )
        .expect("network json should write");
        fs::write(
            lqos_directory.join(TOPOLOGY_CANONICAL_STATE_FILENAME),
            serde_json::to_string_pretty(&TopologyCanonicalStateFile::from_legacy_network_json(
                &json!({
                    "Old AP": {
                        "children": {},
                        "id": "uisp:device:old-ap",
                        "type": "AP",
                        "downloadBandwidthMbps": 50,
                        "uploadBandwidthMbps": 50
                    }
                }),
            ))
            .expect("canonical state should serialize"),
        )
        .expect("canonical state should write");
        fs::write(
            lqos_directory.join(TOPOLOGY_EDITOR_STATE_FILENAME),
            "{\"schema_version\":1,\"source\":\"legacy:test\",\"nodes\":[{\"node_id\":\"uisp:device:old-ap\",\"node_name\":\"Old AP\"}]}",
        )
        .expect("editor state should write");
        fs::write(
            lqos_directory.join(TOPOLOGY_EFFECTIVE_NETWORK_FILENAME),
            "{\"Old AP\":{\"children\":{}}}",
        )
        .expect("effective network should write");

        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };

        let loaded = TopologyCanonicalStateFile::load_with_legacy_fallback(&config)
            .expect("load with fallback should succeed");

        assert!(loaded.find_node("uisp:device:current-ap").is_some());
        assert!(loaded.find_node("uisp:device:old-ap").is_none());
        assert!(
            !lqos_directory
                .join(TOPOLOGY_CANONICAL_STATE_FILENAME)
                .exists()
        );
        assert!(!lqos_directory.join(TOPOLOGY_EDITOR_STATE_FILENAME).exists());
        assert!(
            !lqos_directory
                .join(TOPOLOGY_EFFECTIVE_NETWORK_FILENAME)
                .exists()
        );

        let quarantine_directory = lqos_directory.join(".topology_stale");
        let quarantined = fs::read_dir(&quarantine_directory)
            .expect("directory should read")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(
            quarantined
                .iter()
                .any(|name| name.starts_with("topology_canonical_state.json.stale-"))
        );
        assert!(
            quarantined
                .iter()
                .any(|name| name.starts_with("topology_editor_state.json.stale-"))
        );
    }

    #[test]
    fn current_ingress_identity_prevents_quarantining_matching_native_state() {
        let lqos_directory = unique_temp_dir("lqos-config-canonical-ingress-identity");
        let ingress_identity = topology_ingress_identity_from_tokens([
            "import:uisp/full2".to_string(),
            "mode:ap_site".to_string(),
            "node:uisp:device:current-ap".to_string(),
        ])
        .expect("identity should hash");
        fs::write(
            lqos_directory.join("network.json"),
            serde_json::to_string_pretty(&json!({
                "Current AP": {
                    "children": {
                        "Orphans": {
                            "children": {},
                            "id": "libreqos:generated:uisp:site:orphans",
                            "type": "Site",
                            "downloadBandwidthMbps": 100,
                            "uploadBandwidthMbps": 100
                        }
                    },
                    "id": "uisp:device:current-ap",
                    "type": "AP",
                    "downloadBandwidthMbps": 100,
                    "uploadBandwidthMbps": 100
                }
            }))
            .expect("network json should serialize"),
        )
        .expect("network json should write");
        fs::write(
            lqos_directory.join(TOPOLOGY_PARENT_CANDIDATES_FILENAME),
            serde_json::to_string_pretty(&TopologyParentCandidatesFile {
                source: "uisp/full2".to_string(),
                ingress_identity: Some(ingress_identity.clone()),
                nodes: Vec::new(),
            })
            .expect("parent candidates should serialize"),
        )
        .expect("parent candidates should write");

        let canonical = TopologyCanonicalStateFile {
            schema_version: 1,
            source: "uisp/ap_site".to_string(),
            generated_unix: Some(1),
            ingress_identity: Some(ingress_identity.clone()),
            ingress_kind: TopologyCanonicalIngressKind::NativeIntegration,
            nodes: vec![TopologyCanonicalNode {
                node_id: "uisp:device:current-ap".to_string(),
                node_name: "Current AP".to_string(),
                node_kind: "AP".to_string(),
                is_virtual: false,
                current_parent_node_id: None,
                current_parent_node_name: None,
                current_attachment_id: None,
                current_attachment_name: None,
                can_move: false,
                allowed_parents: Vec::new(),
                rate_input: super::TopologyCanonicalRateInput {
                    intrinsic_download_mbps: Some(100),
                    intrinsic_upload_mbps: Some(100),
                    legacy_imported_download_mbps: None,
                    legacy_imported_upload_mbps: None,
                    source: TopologyCanonicalRateInputSource::AttachmentMax,
                },
            }],
            compatibility_network_json: json!({}),
        };
        fs::write(
            lqos_directory.join(TOPOLOGY_CANONICAL_STATE_FILENAME),
            serde_json::to_string_pretty(&canonical).expect("canonical should serialize"),
        )
        .expect("canonical should write");
        fs::write(
            lqos_directory.join(TOPOLOGY_EDITOR_STATE_FILENAME),
            serde_json::to_string_pretty(&TopologyEditorStateFile {
                schema_version: 1,
                source: "uisp/ap_site".to_string(),
                generated_unix: Some(1),
                ingress_identity: Some(ingress_identity),
                nodes: vec![TopologyEditorNode {
                    node_id: "uisp:device:current-ap".to_string(),
                    node_name: "Current AP".to_string(),
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
                }],
            })
            .expect("editor should serialize"),
        )
        .expect("editor should write");

        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };

        let loaded = TopologyCanonicalStateFile::load_with_legacy_fallback(&config)
            .expect("load with fallback should succeed");

        assert!(loaded.find_node("uisp:device:current-ap").is_some());
        assert!(
            lqos_directory
                .join(TOPOLOGY_CANONICAL_STATE_FILENAME)
                .exists()
        );
        assert!(lqos_directory.join(TOPOLOGY_EDITOR_STATE_FILENAME).exists());
        let quarantine_directory = lqos_directory.join(".topology_stale");
        let quarantined = if quarantine_directory.exists() {
            fs::read_dir(&quarantine_directory)
                .expect("directory should read")
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect::<Vec<_>>()
        } else {
            Vec::<String>::new()
        };
        assert!(
            !quarantined
                .iter()
                .any(|name| name.starts_with("topology_canonical_state.json.stale-"))
        );
        assert!(
            !quarantined
                .iter()
                .any(|name| name.starts_with("topology_editor_state.json.stale-"))
        );
    }

    #[test]
    fn current_topology_ingress_identity_prefers_topology_import_artifact() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-import-identity");
        fs::write(
            lqos_directory.join(TOPOLOGY_IMPORT_FILENAME),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "source": "uisp/full2",
                "compile_mode": "ap_site",
                "generated_unix": 1,
                "ingress_identity": "import-identity",
                "imported": {
                    "source": "uisp/full2",
                    "generated_unix": 1,
                    "compatibility_network_json": {},
                    "shaped_devices": [],
                    "circuit_anchors": {
                        "schema_version": 1,
                        "source": "uisp/ap_site",
                        "generated_unix": 1,
                        "anchors": []
                    },
                    "ethernet_advisories": []
                }
            }))
            .expect("topology import should serialize"),
        )
        .expect("topology import should write");
        fs::write(
            lqos_directory.join(TOPOLOGY_PARENT_CANDIDATES_FILENAME),
            serde_json::to_string_pretty(&TopologyParentCandidatesFile {
                source: "uisp/ap_site".to_string(),
                ingress_identity: Some("parent-candidates-identity".to_string()),
                nodes: Vec::new(),
            })
            .expect("parent candidates should serialize"),
        )
        .expect("parent candidates should write");
        fs::write(
            lqos_directory.join("network.json"),
            serde_json::to_string_pretty(&json!({
                "Legacy Tower": {
                    "children": {},
                    "id": "legacy-tower",
                    "type": "Site",
                    "downloadBandwidthMbps": 100,
                    "uploadBandwidthMbps": 100
                }
            }))
            .expect("network json should serialize"),
        )
        .expect("network json should write");

        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;

        let identity = current_topology_ingress_identity(&config)
            .expect("ingress identity should load")
            .expect("ingress identity should be present");
        assert_eq!(identity, "import-identity");
    }

    #[test]
    fn current_topology_ingress_identity_does_not_fallback_to_network_json_for_integrations() {
        let lqos_directory = unique_temp_dir("lqos-config-topology-import-no-network-fallback");
        fs::write(
            lqos_directory.join("network.json"),
            serde_json::to_string_pretty(&json!({
                "Legacy Tower": {
                    "children": {},
                    "id": "legacy-tower",
                    "type": "Site",
                    "downloadBandwidthMbps": 100,
                    "uploadBandwidthMbps": 100
                }
            }))
            .expect("network json should serialize"),
        )
        .expect("network json should write");

        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;

        let identity =
            current_topology_ingress_identity(&config).expect("ingress identity should load");
        assert!(identity.is_none());
    }

    #[test]
    fn load_with_legacy_fallback_skips_network_json_for_integration_ingress() {
        let lqos_directory = unique_temp_dir("lqos-config-no-legacy-fallback-integration");
        fs::write(
            lqos_directory.join("network.json"),
            serde_json::to_string_pretty(&json!({
                "Legacy Tower": {
                    "children": {},
                    "id": "legacy-tower",
                    "type": "Site",
                    "downloadBandwidthMbps": 100,
                    "uploadBandwidthMbps": 100
                }
            }))
            .expect("network json should serialize"),
        )
        .expect("network json should write");

        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;

        let loaded = TopologyCanonicalStateFile::load_with_legacy_fallback(&config)
            .expect("load with fallback should succeed");
        assert!(loaded.nodes.is_empty());
    }

    #[test]
    fn insight_topology_network_json_uses_logical_parent_hierarchy() {
        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: Some(123),
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-router".to_string(),
                    node_name: "WestRedd-SiteRouter".to_string(),
                    current_parent_node_id: Some("root-west".to_string()),
                    current_parent_node_name: Some("WestRedd".to_string()),
                    current_attachment_id: Some("site-router".to_string()),
                    current_attachment_name: Some("WestRedd-SiteRouter".to_string()),
                    can_move: false,
                    allowed_parents: Vec::new(),
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "core-west".to_string(),
                    node_name: "Core-WestRedd".to_string(),
                    current_parent_node_id: Some("site-router".to_string()),
                    current_parent_node_name: Some("WestRedd-SiteRouter".to_string()),
                    current_attachment_id: Some("core-west".to_string()),
                    current_attachment_name: Some("Core-WestRedd".to_string()),
                    can_move: false,
                    allowed_parents: Vec::new(),
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "tuscany".to_string(),
                    node_name: "Tuscany Ridge".to_string(),
                    current_parent_node_id: Some("root-west".to_string()),
                    current_parent_node_name: Some("WestRedd".to_string()),
                    current_attachment_id: Some("aviat-west".to_string()),
                    current_attachment_name: Some("AVIAT_WestRedd".to_string()),
                    can_move: false,
                    allowed_parents: Vec::new(),
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "monte".to_string(),
                    node_name: "Monte Del Sol".to_string(),
                    current_parent_node_id: Some("tuscany".to_string()),
                    current_parent_node_name: Some("Tuscany Ridge".to_string()),
                    current_attachment_id: Some("monte-ap".to_string()),
                    current_attachment_name: Some("AF60LR-MonteDelSOl".to_string()),
                    can_move: false,
                    allowed_parents: Vec::new(),
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let compatibility_network = json!({
            "WestRedd-SiteRouter": {
                "children": {
                    "Core-WestRedd": {
                        "children": {},
                        "downloadBandwidthMbps": 5000,
                        "id": "core-west",
                        "name": "Core-WestRedd",
                        "type": "AP",
                        "uploadBandwidthMbps": 5000
                    }
                },
                "downloadBandwidthMbps": 5000,
                "id": "site-router",
                "name": "WestRedd-SiteRouter",
                "type": "AP",
                "uploadBandwidthMbps": 5000
            },
            "AVIAT_WestRedd": {
                "children": {
                    "AVIAT_TuscanyRidge": {
                        "children": {
                            "Tuscany Ridge": {
                                "children": {
                                    "AF60LR-MonteDelSOl": {
                                        "children": {
                                            "Monte Del Sol": {
                                                "children": {},
                                                "downloadBandwidthMbps": 900,
                                                "id": "monte",
                                                "name": "Monte Del Sol",
                                                "type": "Site",
                                                "uploadBandwidthMbps": 900
                                            }
                                        },
                                        "downloadBandwidthMbps": 1200,
                                        "id": "monte-ap",
                                        "name": "AF60LR-MonteDelSOl",
                                        "type": "AP",
                                        "uploadBandwidthMbps": 1200
                                    }
                                },
                                "downloadBandwidthMbps": 1200,
                                "id": "tuscany",
                                "name": "Tuscany Ridge",
                                "type": "Site",
                                "uploadBandwidthMbps": 1200
                            }
                        },
                        "downloadBandwidthMbps": 1200,
                        "id": "aviat-tuscany",
                        "name": "AVIAT_TuscanyRidge",
                        "type": "AP",
                        "uploadBandwidthMbps": 1200
                    }
                },
                "downloadBandwidthMbps": 5000,
                "id": "aviat-west",
                "name": "AVIAT_WestRedd",
                "type": "AP",
                "uploadBandwidthMbps": 5000
            }
        });

        let canonical = TopologyCanonicalStateFile::from_editor_and_network(
            &editor_state,
            &compatibility_network,
            TopologyCanonicalIngressKind::NativeIntegration,
        );

        let insight_tree = canonical.insight_topology_network_json();
        let root_children = insight_tree
            .as_object()
            .expect("expected top-level logical roots");
        assert!(root_children.contains_key("WestRedd-SiteRouter"));
        assert!(root_children.contains_key("Tuscany Ridge"));

        let site_router_children = root_children
            .get("WestRedd-SiteRouter")
            .and_then(Value::as_object)
            .and_then(|node| node.get("children"))
            .and_then(Value::as_object)
            .expect("expected site-router children");
        assert!(site_router_children.contains_key("Core-WestRedd"));

        let tuscany_children = root_children
            .get("Tuscany Ridge")
            .and_then(Value::as_object)
            .and_then(|node| node.get("children"))
            .and_then(Value::as_object)
            .expect("expected Tuscany Ridge children");
        assert!(tuscany_children.contains_key("Monte Del Sol"));
    }
}

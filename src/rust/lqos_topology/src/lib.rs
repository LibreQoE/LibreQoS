//! Shared topology runtime domain logic for attachment health and effective topology.

#![warn(missing_docs)]

use anyhow::{Context, Result};
use lqos_config::{
    Config, TOPOLOGY_ATTACHMENT_AUTO_ID, TopologyAllowedParent, TopologyAttachmentHealthStateFile,
    TopologyAttachmentHealthStatus, TopologyAttachmentOption, TopologyAttachmentRateSource,
    TopologyAttachmentRole, TopologyCanonicalNode, TopologyCanonicalStateFile, TopologyEditorNode,
    TopologyEditorStateFile, TopologyEffectiveAttachmentState, TopologyEffectiveNodeState,
    TopologyEffectiveStateFile, topology_effective_network_path, topology_effective_state_path,
};
use lqos_overrides::{TopologyAttachmentMode, TopologyOverridesFile};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_FILENAME: &str = "topology_effective_publish.lock";

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
    let effective_network = canonical.compatibility_network_json().as_object().map(|_| {
        apply_effective_topology_to_canonical_state(config, canonical, &ui_state, &effective)
    });

    if let Some(effective_network) = effective_network.as_ref() {
        validate_effective_topology_network(&ui_state, &effective, effective_network)?;
    }

    Ok(EffectiveTopologyArtifacts {
        effective,
        ui_state,
        effective_network,
    })
}

/// Publishes validated effective-topology artifacts under a single writer lock.
///
/// Side effects: writes `topology_effective_state.json` and, when present, `network.effective.json`.
/// If no effective network export is present, any stale `network.effective.json` is removed so
/// runtime consumers fall back to canonical integration output.
pub fn publish_effective_topology_artifacts(
    config: &Config,
    artifacts: &EffectiveTopologyArtifacts,
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

    Ok(())
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

fn first_healthy_attachment_id(parent: &TopologyAllowedParent) -> Option<String> {
    parent
        .attachment_options
        .iter()
        .find(|option| {
            option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID
                && option.health_status == TopologyAttachmentHealthStatus::Healthy
        })
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
                node.allowed_parents
                    .first()
                    .map(|parent| parent.parent_node_id.clone())
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

        let Some(selected_parent) = current_parent_for_node(node, &selected_parent_id)
            .or_else(|| node.allowed_parents.first())
        else {
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
            _ => node
                .current_attachment_id
                .clone()
                .filter(|attachment_id| parent_has_attachment(&enriched_parent, attachment_id)),
        };

        let healthy_ids = explicit_options
            .iter()
            .filter(|option| option.health_status == TopologyAttachmentHealthStatus::Healthy)
            .map(|option| option.attachment_id.clone())
            .collect::<HashSet<_>>();

        let mut fallback_reason = None;
        let effective_attachment_id = if explicit_options.is_empty() {
            None
        } else if !healthy_ids.is_empty() {
            match override_entry {
                Some(saved)
                    if saved.parent_node_id == selected_parent_id
                        && saved.mode == TopologyAttachmentMode::PreferredOrder =>
                {
                    saved
                        .attachment_preference_ids
                        .iter()
                        .find(|attachment_id| healthy_ids.contains(*attachment_id))
                        .cloned()
                        .or_else(|| {
                            node.current_attachment_id
                                .clone()
                                .filter(|attachment_id| healthy_ids.contains(attachment_id))
                        })
                        .or_else(|| first_healthy_attachment_id(&enriched_parent))
                }
                _ => node
                    .current_attachment_id
                    .clone()
                    .filter(|attachment_id| healthy_ids.contains(attachment_id))
                    .or_else(|| first_healthy_attachment_id(&enriched_parent)),
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
pub fn validate_effective_topology_network(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    effective_network: &Value,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    validate_effective_node_identity_consistency(ui_state, effective, &mut errors);
    validate_effective_site_parent_cycles(ui_state, effective, &mut errors);

    let mut counts = HashMap::new();
    let Some(root) = effective_network.as_object() else {
        return Err(vec![
            "Effective topology export is not a JSON object tree.".to_string(),
        ]);
    };
    for child in root.values() {
        count_node_ids(child, &mut counts);
    }

    for node in &ui_state.nodes {
        if !node.node_id.contains(":site:") {
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

/// Applies the effective attachment selection to a canonical network tree and returns
/// the runtime-effective tree used by shaping/export.
pub fn apply_effective_topology_to_network_json(
    config: &Config,
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
        let Some(effective_attachment_id) = effective_node.effective_attachment_id.as_deref()
        else {
            continue;
        };
        let Some(target_attachment) = selected_parent
            .attachment_options
            .iter()
            .find(|option| option.attachment_id == effective_attachment_id)
        else {
            continue;
        };
        if ui_node.current_parent_node_id.as_deref()
            == Some(effective_node.logical_parent_node_id.as_str())
            && ui_node.current_attachment_id.as_deref()
                == effective_node.effective_attachment_id.as_deref()
        {
            ensure_attachment_node_exists(
                &mut out,
                &selected_parent.parent_node_id,
                target_attachment,
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

    apply_runtime_squashing(config, ui_state, effective, &mut out);
    Value::Object(out)
}

fn apply_effective_topology_to_canonical_state(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> Value {
    let mut effective_network = apply_effective_topology_to_network_json(
        config,
        canonical.compatibility_network_json(),
        ui_state,
        effective,
    );
    if let Some(root) = effective_network.as_object_mut() {
        recompile_effective_network_bandwidths(root, canonical, ui_state, effective);
    }
    effective_network
}

#[cfg(test)]
mod tests {
    use super::{
        apply_effective_topology_to_network_json, build_effective_topology_artifacts,
        compute_effective_state, validate_effective_topology_network,
    };
    use lqos_config::{
        Config, TopologyAllowedParent, TopologyAttachmentHealthStateFile,
        TopologyAttachmentHealthStatus, TopologyAttachmentOption, TopologyAttachmentRateSource,
        TopologyAttachmentRole, TopologyEditorNode, TopologyEditorStateFile,
        TopologyEffectiveAttachmentState, TopologyEffectiveNodeState, TopologyEffectiveStateFile,
    };
    use lqos_overrides::{TopologyAttachmentMode, TopologyOverridesFile};
    use serde_json::{Value, json};

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
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
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
                nodes: vec![TopologyEditorNode {
                    node_id: "child-site".to_string(),
                    node_name: "Child Site".to_string(),
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
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
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
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
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
            "Matt Koehn": {
                "children": {
                    "Matt-Hoodoo-60": {
                        "children": {},
                        "downloadBandwidthMbps": 940,
                        "id": "matt-hoodoo-60",
                        "name": "Matt-Hoodoo-60",
                        "parent_site": "Matt Koehn",
                        "type": "AP",
                        "uploadBandwidthMbps": 940
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "matt-site",
                "name": "Matt Koehn",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            },
            "Hoodoo Hill": {
                "children": {
                    "Hoodoo - Matt 60": {
                        "children": {},
                        "downloadBandwidthMbps": 940,
                        "id": "hoodoo-matt-60",
                        "name": "Hoodoo - Matt 60",
                        "parent_site": "Hoodoo Hill",
                        "type": "AP",
                        "uploadBandwidthMbps": 940
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "hoodoo-site",
                "name": "Hoodoo Hill",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });

        let mut move_attachment = sample_attachment_option("hoodoo-matt-60", "Hoodoo - Matt 60");
        move_attachment.peer_attachment_id = Some("matt-hoodoo-60".to_string());
        move_attachment.peer_attachment_name = Some("Matt-Hoodoo-60".to_string());
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
                nodes: vec![TopologyEditorNode {
                    node_id: "hoodoo-site".to_string(),
                    node_name: "Hoodoo Hill".to_string(),
                    current_parent_node_id: Some("matt-site".to_string()),
                    current_parent_node_name: Some("Matt Koehn".to_string()),
                    current_attachment_id: Some("hoodoo-matt-60".to_string()),
                    current_attachment_name: Some("Hoodoo - Matt 60".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "matt-site".to_string(),
                        parent_node_name: "Matt Koehn".to_string(),
                        attachment_options: vec![move_attachment],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
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
                    node_id: "hoodoo-site".to_string(),
                    logical_parent_node_id: "matt-site".to_string(),
                    preferred_attachment_id: Some("hoodoo-matt-60".to_string()),
                    effective_attachment_id: Some("hoodoo-matt-60".to_string()),
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![TopologyEffectiveAttachmentState {
                        attachment_id: "hoodoo-matt-60".to_string(),
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

        assert!(moved.get("Hoodoo Hill").is_none());
        let matt_children = moved["Matt Koehn"]["children"]
            .as_object()
            .expect("Matt Koehn should keep children");
        let hoodoo = matt_children
            .get("Hoodoo Hill")
            .and_then(Value::as_object)
            .expect("Hoodoo Hill should remain visible under Matt Koehn after squashing");
        assert_eq!(hoodoo["id"].as_str(), Some("hoodoo-site"));
        assert_eq!(hoodoo["parent_site"].as_str(), Some("Matt Koehn"));
        assert_eq!(
            hoodoo["active_attachment_name"].as_str(),
            Some("Matt-Hoodoo-60")
        );
        let hoodoo_children = hoodoo["children"]
            .as_object()
            .expect("Hoodoo Hill subtree should keep its children");
        assert!(hoodoo_children.get("Hoodoo - Matt 60").is_some());
    }

    #[test]
    fn duplicate_device_candidates_do_not_block_valid_site_override_publish() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;

        let canonical_network = json!({
            "Matt Koehn": {
                "children": {
                    "Matt-Hoodoo-60": {
                        "children": {},
                        "downloadBandwidthMbps": 940,
                        "id": "matt-hoodoo-60",
                        "name": "Matt-Hoodoo-60",
                        "parent_site": "Matt Koehn",
                        "type": "AP",
                        "uploadBandwidthMbps": 940
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "matt-site",
                "name": "Matt Koehn",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            },
            "Hoodoo Hill": {
                "children": {
                    "Hoodoo - Matt 60": {
                        "children": {},
                        "downloadBandwidthMbps": 940,
                        "id": "hoodoo-matt-60",
                        "name": "Hoodoo - Matt 60",
                        "parent_site": "Hoodoo Hill",
                        "type": "AP",
                        "uploadBandwidthMbps": 940
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "hoodoo-site",
                "name": "Hoodoo Hill",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });

        let mut hoodoo_matt_option = sample_attachment_option("hoodoo-matt-60", "Hoodoo - Matt 60");
        hoodoo_matt_option.peer_attachment_id = Some("matt-hoodoo-60".to_string());
        hoodoo_matt_option.peer_attachment_name = Some("Matt-Hoodoo-60".to_string());
        hoodoo_matt_option.download_bandwidth_mbps = Some(940);
        hoodoo_matt_option.upload_bandwidth_mbps = Some(940);
        hoodoo_matt_option.capacity_mbps = Some(940);

        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "hoodoo-site".to_string(),
                    node_name: "Hoodoo Hill".to_string(),
                    current_parent_node_id: Some("willows-site".to_string()),
                    current_parent_node_name: Some("Willows Tower (GT)".to_string()),
                    current_attachment_id: Some("hoodoo-willows-60".to_string()),
                    current_attachment_name: Some("Hoodoo - Willows 60".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "matt-site".to_string(),
                        parent_node_name: "Matt Koehn".to_string(),
                        attachment_options: vec![hoodoo_matt_option.clone()],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "hoodoo-matt-60".to_string(),
                    node_name: "Hoodoo - Matt 60".to_string(),
                    current_parent_node_id: Some("hoodoo-site".to_string()),
                    current_parent_node_name: Some("Hoodoo Hill".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "hoodoo-site".to_string(),
                        parent_node_name: "Hoodoo Hill".to_string(),
                        attachment_options: vec![],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "hoodoo-matt-60".to_string(),
                    node_name: "Hoodoo - Matt 60".to_string(),
                    current_parent_node_id: Some("hoodoo-site".to_string()),
                    current_parent_node_name: Some("Hoodoo Hill".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![
                        TopologyAllowedParent {
                            parent_node_id: "matt-site".to_string(),
                            parent_node_name: "Matt Koehn".to_string(),
                            attachment_options: vec![hoodoo_matt_option.clone()],
                            all_attachments_suppressed: false,
                            has_probe_unavailable_attachments: false,
                        },
                        TopologyAllowedParent {
                            parent_node_id: "hoodoo-site".to_string(),
                            parent_node_name: "Hoodoo Hill".to_string(),
                            attachment_options: vec![],
                            all_attachments_suppressed: false,
                            has_probe_unavailable_attachments: false,
                        },
                    ],
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let mut overrides = TopologyOverridesFile::default();
        overrides.set_override_return_changed(
            "hoodoo-site".to_string(),
            "Hoodoo Hill".to_string(),
            "matt-site".to_string(),
            "Matt Koehn".to_string(),
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
                .filter(|node| node.node_id == "hoodoo-matt-60")
                .count(),
            1
        );
        let moved = artifacts
            .effective_network
            .expect("effective network should be published");
        let matt_children = moved["Matt Koehn"]["children"]
            .as_object()
            .expect("Matt should keep children");
        assert!(matt_children.get("Hoodoo Hill").is_some());
    }

    #[test]
    fn effective_topology_validation_rejects_missing_site() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            nodes: vec![TopologyEditorNode {
                node_id: "uisp:site:hoodoo-site".to_string(),
                node_name: "Hoodoo Hill".to_string(),
                current_parent_node_id: Some("uisp:site:matt-site".to_string()),
                current_parent_node_name: Some("Matt Koehn".to_string()),
                current_attachment_id: Some("hoodoo-matt-60".to_string()),
                current_attachment_name: Some("Hoodoo - Matt 60".to_string()),
                can_move: true,
                allowed_parents: vec![],
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
                node_id: "uisp:site:hoodoo-site".to_string(),
                logical_parent_node_id: "uisp:site:matt-site".to_string(),
                preferred_attachment_id: Some("hoodoo-matt-60".to_string()),
                effective_attachment_id: Some("hoodoo-matt-60".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![],
            }],
        };
        let exported = json!({
            "Matt Koehn": {
                "children": {},
                "id": "uisp:site:matt-site",
                "name": "Matt Koehn",
                "type": "Site"
            }
        });

        let errors = validate_effective_topology_network(&ui_state, &effective, &exported)
            .expect_err("missing site should fail validation");
        assert!(errors.iter().any(|error| error.contains("Hoodoo Hill")));
    }

    #[test]
    fn effective_topology_validation_rejects_site_parent_cycles() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "uisp:site:site-a".to_string(),
                    node_name: "Site A".to_string(),
                    current_parent_node_id: Some("uisp:site:site-b".to_string()),
                    current_parent_node_name: Some("Site B".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![],
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "uisp:site:site-b".to_string(),
                    node_name: "Site B".to_string(),
                    current_parent_node_id: Some("uisp:site:site-a".to_string()),
                    current_parent_node_name: Some("Site A".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![],
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

        let errors = validate_effective_topology_network(&ui_state, &effective, &exported)
            .expect_err("site-parent cycle should fail validation");
        assert!(errors.iter().any(|error| error.contains("parent cycle")));
    }

    #[test]
    fn effective_topology_validation_rejects_invalid_attachment_for_selected_parent() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            nodes: vec![TopologyEditorNode {
                node_id: "uisp:site:hoodoo-site".to_string(),
                node_name: "Hoodoo Hill".to_string(),
                current_parent_node_id: Some("uisp:site:matt-site".to_string()),
                current_parent_node_name: Some("Matt Koehn".to_string()),
                current_attachment_id: Some("matt-hoodoo-60".to_string()),
                current_attachment_name: Some("Matt-Hoodoo-60".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "uisp:site:matt-site".to_string(),
                    parent_node_name: "Matt Koehn".to_string(),
                    attachment_options: vec![sample_attachment_option(
                        "matt-hoodoo-60",
                        "Matt-Hoodoo-60",
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
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "uisp:site:hoodoo-site".to_string(),
                logical_parent_node_id: "uisp:site:matt-site".to_string(),
                preferred_attachment_id: Some("matt-hoodoo-60".to_string()),
                effective_attachment_id: Some("hoodoo-matt-60".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![],
            }],
        };
        let exported = json!({
            "Matt Koehn": {
                "children": {
                    "Hoodoo Hill": {
                        "children": {},
                        "id": "uisp:site:hoodoo-site",
                        "name": "Hoodoo Hill",
                        "type": "Site"
                    }
                },
                "id": "uisp:site:matt-site",
                "name": "Matt Koehn",
                "type": "Site"
            }
        });

        let errors = validate_effective_topology_network(&ui_state, &effective, &exported)
            .expect_err("invalid attachment should fail validation");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("invalid attachment"))
        );
    }

    #[test]
    fn effective_state_fallback_does_not_keep_old_parent_attachment_after_reparent() {
        use lqos_overrides::TopologyOverridesFile;

        let config = Config::default();
        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            nodes: vec![TopologyEditorNode {
                node_id: "uisp:site:hoodoo-site".to_string(),
                node_name: "Hoodoo Hill".to_string(),
                current_parent_node_id: Some("uisp:site:willows-site".to_string()),
                current_parent_node_name: Some("Willows Tower (GT)".to_string()),
                current_attachment_id: Some("uisp:device:hoodoo-willows".to_string()),
                current_attachment_name: Some("Hoodoo - Willows MLO6".to_string()),
                can_move: true,
                allowed_parents: vec![
                    TopologyAllowedParent {
                        parent_node_id: "uisp:site:matt-site".to_string(),
                        parent_node_name: "Matt Koehn".to_string(),
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
                                attachment_id: "uisp:device:hoodoo-matt".to_string(),
                                attachment_name: "Hoodoo - Matt 60".to_string(),
                                attachment_kind: "device".to_string(),
                                attachment_role: TopologyAttachmentRole::PtpBackhaul,
                                pair_id: None,
                                peer_attachment_id: Some("uisp:device:matt-hoodoo".to_string()),
                                peer_attachment_name: Some("Matt-Hoodoo-60".to_string()),
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
                        parent_node_id: "uisp:site:willows-site".to_string(),
                        parent_node_name: "Willows Tower (GT)".to_string(),
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
                                attachment_id: "uisp:device:hoodoo-willows".to_string(),
                                attachment_name: "Hoodoo - Willows MLO6".to_string(),
                                attachment_kind: "device".to_string(),
                                attachment_role: TopologyAttachmentRole::PtpBackhaul,
                                pair_id: None,
                                peer_attachment_id: Some("uisp:device:willows-hoodoo".to_string()),
                                peer_attachment_name: Some("Willows - Hoodoo MLO6".to_string()),
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
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let mut overrides = TopologyOverridesFile::default();
        overrides.set_override_return_changed(
            "uisp:site:hoodoo-site".to_string(),
            "Hoodoo Hill".to_string(),
            "uisp:site:matt-site".to_string(),
            "Matt Koehn".to_string(),
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
            .find(|node| node.node_id == "uisp:site:hoodoo-site")
            .expect("expected Hoodoo Hill effective state");

        assert_eq!(node.logical_parent_node_id, "uisp:site:matt-site");
        assert_eq!(
            node.effective_attachment_id.as_deref(),
            Some("uisp:device:hoodoo-matt")
        );
        assert_ne!(
            node.effective_attachment_id.as_deref(),
            Some("uisp:device:hoodoo-willows")
        );
    }
}

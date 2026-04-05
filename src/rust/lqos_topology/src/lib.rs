//! Shared topology runtime domain logic for attachment health and effective topology.

#![warn(missing_docs)]

use lqos_config::{
    Config, TOPOLOGY_ATTACHMENT_AUTO_ID, TopologyAllowedParent, TopologyAttachmentHealthStateFile,
    TopologyAttachmentHealthStatus, TopologyAttachmentOption, TopologyAttachmentRateSource,
    TopologyAttachmentRole, TopologyEditorNode, TopologyEditorStateFile,
    TopologyEffectiveAttachmentState, TopologyEffectiveNodeState, TopologyEffectiveStateFile,
};
use lqos_overrides::{TopologyAttachmentMode, TopologyOverridesFile};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Computes the effective attachment selection for all nodes using canonical state,
/// operator intent, and transient runtime health.
pub fn compute_effective_state(
    config: &Config,
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> TopologyEffectiveStateFile {
    let manual = overlay_manual_groups(canonical, overrides);
    let base = apply_attachment_rate_overrides(&manual, overrides);
    let health_by_pair = if is_health_state_fresh(config, health) {
        health
            .attachments
            .iter()
            .map(|entry| (entry.attachment_pair_id.as_str(), entry))
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    };

    let mut nodes = Vec::with_capacity(base.nodes.len());
    for node in &base.nodes {
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

        let Some(selected_parent) = current_parent_for_node(node, &selected_parent_id) else {
            continue;
        };
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
                    .or_else(|| node.current_attachment_id.clone())
            }
            _ => node.current_attachment_id.clone(),
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
        canonical_generated_unix: canonical.generated_unix,
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
    let manual = overlay_manual_groups(canonical, overrides);
    let mut state = apply_attachment_rate_overrides(&manual, overrides);
    let health_by_pair = if is_health_state_fresh(config, health) {
        health
            .attachments
            .iter()
            .map(|entry| (entry.attachment_pair_id.as_str(), entry))
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    };
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

        ensure_attachment_node_exists(&mut out, &selected_parent.parent_node_id, target_attachment);
        let Some((node_key, node_value)) = remove_node_by_id(&mut out, &ui_node.node_id) else {
            continue;
        };
        let _ = insert_node_under_parent_id(
            &mut out,
            &target_attachment.attachment_id,
            &node_key,
            node_value,
        );
    }

    apply_runtime_squashing(config, ui_state, effective, &mut out);
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::apply_effective_topology_to_network_json;
    use lqos_config::{
        Config, TopologyAllowedParent, TopologyAttachmentHealthStatus, TopologyAttachmentOption,
        TopologyAttachmentRateSource, TopologyAttachmentRole, TopologyEditorNode,
        TopologyEditorStateFile, TopologyEffectiveAttachmentState, TopologyEffectiveNodeState,
        TopologyEffectiveStateFile,
    };
    use serde_json::json;

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
}

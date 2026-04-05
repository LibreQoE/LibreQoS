//! Shared topology runtime domain logic for attachment health and effective topology.

#![warn(missing_docs)]

use lqos_config::{
    Config, TOPOLOGY_ATTACHMENT_AUTO_ID, TopologyAllowedParent, TopologyAttachmentHealthStateFile,
    TopologyAttachmentHealthStatus, TopologyAttachmentOption, TopologyEditorNode,
    TopologyEditorStateFile, TopologyEffectiveAttachmentState, TopologyEffectiveNodeState,
    TopologyEffectiveStateFile,
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
    /// Stable node identifier of the child being shaped.
    pub node_id: String,
    /// Stable parent node identifier for this attachment group.
    pub parent_node_id: String,
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
pub fn is_health_state_fresh(
    config: &Config,
    health: &TopologyAttachmentHealthStateFile,
) -> bool {
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
        pair_id: None,
        peer_attachment_id: None,
        peer_attachment_name: None,
        capacity_mbps: None,
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
                let probeable = attachment.local_probe_ip.parse::<IpAddr>().is_ok()
                    && attachment.remote_probe_ip.parse::<IpAddr>().is_ok()
                    && attachment.local_probe_ip != attachment.remote_probe_ip;
                options.push(TopologyAttachmentOption {
                    attachment_id: attachment.attachment_id.clone(),
                    attachment_name: attachment.attachment_name.clone(),
                    attachment_kind: "manual".to_string(),
                    pair_id: Some(attachment.attachment_id.clone()),
                    peer_attachment_id: None,
                    peer_attachment_name: None,
                    capacity_mbps: Some(attachment.capacity_mbps),
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
    if local == remote {
        return "Probe unavailable: local and remote probe IPs are identical".to_string();
    }
    if local.parse::<IpAddr>().is_err() && remote.parse::<IpAddr>().is_err() {
        return "Probe unavailable: local and remote probe IPs are invalid".to_string();
    }
    if local.parse::<IpAddr>().is_err() {
        return "Probe unavailable: local management IP is invalid".to_string();
    }
    if remote.parse::<IpAddr>().is_err() {
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
        .map(|(local, remote)| {
            local.parse::<IpAddr>().is_ok() && remote.parse::<IpAddr>().is_ok() && local != remote
        })
        .unwrap_or(false);

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
    let base = overlay_manual_groups(canonical, overrides);
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
            .or_else(|| node.allowed_parents.first().map(|parent| parent.parent_node_id.clone()));

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
                saved.attachment_preference_ids
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
                    saved.attachment_preference_ids
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
    let mut state = overlay_manual_groups(canonical, overrides);
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
                    node_id: node.node_id.clone(),
                    parent_node_id: parent.parent_node_id.clone(),
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

fn find_node_name_by_id(map: &Map<String, Value>, target_id: &str) -> Option<String> {
    for (key, value) in map {
        let Value::Object(node) = value else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == target_id)
        {
            return Some(key.clone());
        }
        if let Some(children) = node.get("children").and_then(Value::as_object)
            && let Some(found) = find_node_name_by_id(children, target_id)
        {
            return Some(found);
        }
    }
    None
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
    if find_node_name_by_id(root, &attachment.attachment_id).is_some() {
        return;
    }
    let capacity = attachment.capacity_mbps.unwrap_or(0);
    let mut node = Map::new();
    node.insert("children".to_string(), Value::Object(Map::new()));
    node.insert(
        "downloadBandwidthMbps".to_string(),
        Value::Number(capacity.into()),
    );
    node.insert(
        "uploadBandwidthMbps".to_string(),
        Value::Number(capacity.into()),
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

/// Applies the effective attachment selection to a canonical network tree and returns
/// the runtime-effective tree used by shaping/export.
pub fn apply_effective_topology_to_network_json(
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
        let Some(effective_attachment_id) = effective_node.effective_attachment_id.as_deref() else {
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

    Value::Object(out)
}

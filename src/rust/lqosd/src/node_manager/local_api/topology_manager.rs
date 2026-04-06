use crate::node_manager::auth::LoginResult;
use axum::http::StatusCode;
use lqos_config::{
    Config, TopologyAllowedParent, TopologyAttachmentHealthStateFile, TopologyCanonicalStateFile,
    TopologyEditorStateFile, load_config,
};
use lqos_overrides::{ManualAttachment, TopologyAttachmentMode, TopologyOverridesFile};
use lqos_topology::{
    build_effective_topology_artifacts_from_canonical, compute_effective_state,
    merged_topology_state, publish_effective_topology_artifacts,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::IpAddr;
use tracing::warn;

/// Update payload for persisting one topology-manager branch move.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyManagerUpdate {
    /// Stable node identifier of the child branch being moved.
    pub child_node_id: String,
    /// Stable node identifier of the newly selected parent branch.
    pub parent_node_id: String,
    /// Attachment resolution mode.
    pub mode: TopologyAttachmentMode,
    /// Ranked attachment identifiers to prefer beneath the selected parent.
    #[serde(default)]
    pub attachment_preference_ids: Vec<String>,
}

/// Clear payload for removing one saved topology-manager override.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyManagerClear {
    /// Stable node identifier of the child branch whose override should be removed.
    pub child_node_id: String,
}

/// Update payload for one attachment-pair probe policy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyManagerProbePolicyUpdate {
    /// Stable attachment pair identifier.
    pub attachment_pair_id: String,
    /// Whether probing is enabled for this pair.
    pub enabled: bool,
}

/// Update payload for one attachment-scoped rate override.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyManagerAttachmentRateOverrideUpdate {
    /// Stable child node identifier.
    pub child_node_id: String,
    /// Stable parent node identifier.
    pub parent_node_id: String,
    /// Stable attachment identifier.
    pub attachment_id: String,
    /// Override download bandwidth in Mbps.
    pub download_bandwidth_mbps: u64,
    /// Override upload bandwidth in Mbps.
    pub upload_bandwidth_mbps: u64,
}

/// Clear payload for one attachment-scoped rate override.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyManagerAttachmentRateOverrideClear {
    /// Stable child node identifier.
    pub child_node_id: String,
    /// Stable parent node identifier.
    pub parent_node_id: String,
    /// Stable attachment identifier.
    pub attachment_id: String,
}

/// One manual attachment definition submitted from the Topology Manager UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyManagerManualAttachmentInput {
    /// Stable attachment identifier.
    pub attachment_id: String,
    /// Human-readable attachment label.
    pub attachment_name: String,
    /// Capacity in Mbps.
    pub capacity_mbps: u64,
    /// Local management IP for probing.
    pub local_probe_ip: String,
    /// Remote management IP for probing.
    pub remote_probe_ip: String,
    /// Whether probing is enabled for this attachment pair.
    #[serde(default)]
    pub probe_enabled: bool,
}

/// Update payload for one manual attachment group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyManagerManualAttachmentGroupUpdate {
    /// Stable child node identifier.
    pub child_node_id: String,
    /// Stable parent node identifier.
    pub parent_node_id: String,
    /// Ordered explicit attachments. List order is preference order.
    #[serde(default)]
    pub attachments: Vec<TopologyManagerManualAttachmentInput>,
}

/// Clear payload for one manual attachment group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyManagerManualAttachmentGroupClear {
    /// Stable child node identifier.
    pub child_node_id: String,
    /// Stable parent node identifier.
    pub parent_node_id: String,
}

/// View-model data for one movable node inside the topology manager.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyManagerNodeData {
    /// Stable node identifier matching `network.json` metadata.
    pub node_id: String,
    /// Display name for the node.
    pub node_name: String,
    /// Currently resolved immediate parent node ID from the latest integration snapshot.
    pub current_parent_node_id: Option<String>,
    /// Currently resolved immediate parent name from the latest integration snapshot.
    pub current_parent_node_name: Option<String>,
    /// Currently resolved concrete attachment identifier from the latest integration snapshot.
    pub current_attachment_id: Option<String>,
    /// Currently resolved concrete attachment label from the latest integration snapshot.
    pub current_attachment_name: Option<String>,
    /// Currently effective logical parent node ID from the runtime-effective topology.
    pub effective_parent_node_id: Option<String>,
    /// Currently effective logical parent name from the runtime-effective topology.
    pub effective_parent_node_name: Option<String>,
    /// Whether operators may move this node.
    pub can_move: bool,
    /// Valid parent targets and attachment choices for this node.
    pub allowed_parents: Vec<TopologyAllowedParent>,
    /// Whether a saved operator override currently exists.
    pub has_override: bool,
    /// Saved override parent node ID.
    pub override_parent_node_id: Option<String>,
    /// Saved override parent node name.
    pub override_parent_node_name: Option<String>,
    /// Saved attachment mode.
    pub override_mode: Option<TopologyAttachmentMode>,
    /// Saved attachment preference IDs.
    pub override_attachment_preference_ids: Vec<String>,
    /// Saved attachment preference names.
    pub override_attachment_preference_names: Vec<String>,
    /// Preferred attachment identifier after combining operator intent and runtime data.
    pub preferred_attachment_id: Option<String>,
    /// Preferred attachment label after combining operator intent and runtime data.
    pub preferred_attachment_name: Option<String>,
    /// Effective attachment identifier currently selected for runtime shaping.
    pub effective_attachment_id: Option<String>,
    /// Effective attachment label currently selected for runtime shaping.
    pub effective_attachment_name: Option<String>,
    /// Whether the saved override is currently reflected in the runtime-effective topology.
    pub override_live: bool,
    /// Non-fatal warnings about stale overrides.
    pub warnings: Vec<String>,
}

/// Full page data for the topology manager.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyManagerStateData {
    /// Whether the current session is allowed to persist overrides.
    pub writable: bool,
    /// Source integration string from the runtime snapshot.
    pub source: String,
    /// Schema version of the runtime snapshot.
    pub schema_version: u32,
    /// Node-level runtime editor data.
    pub nodes: Vec<TopologyManagerNodeData>,
    /// Snapshot-wide warnings, such as stale saved overrides for missing nodes.
    pub global_warnings: Vec<String>,
}

/// Loads the current topology manager page state.
pub fn get_topology_manager_state(
    login: LoginResult,
) -> Result<TopologyManagerStateData, StatusCode> {
    build_topology_manager_state(login)
}

fn publish_candidate_overrides(
    config: &Config,
    health: &TopologyAttachmentHealthStateFile,
    previous_overrides: &TopologyOverridesFile,
    candidate_overrides: &TopologyOverridesFile,
) -> Result<(), StatusCode> {
    let canonical = TopologyCanonicalStateFile::load_with_legacy_fallback(config)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let artifacts = build_effective_topology_artifacts_from_canonical(
        config,
        &canonical,
        candidate_overrides,
        health,
    )
    .map_err(|errors| {
        warn!(
            "Rejecting topology manager update because the candidate effective topology is invalid: {}",
            errors.join(" | ")
        );
        StatusCode::BAD_REQUEST
    })?;

    candidate_overrides
        .save()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if publish_effective_topology_artifacts(config, &artifacts).is_err() {
        let _ = previous_overrides.save();
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(())
}

/// Saves or replaces one topology-manager branch move.
pub fn set_topology_manager_override(
    login: LoginResult,
    update: TopologyManagerUpdate,
) -> Result<TopologyManagerStateData, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let canonical = TopologyEditorStateFile::load_with_legacy_fallback(config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let child = canonical
        .find_node(update.child_node_id.trim())
        .ok_or(StatusCode::BAD_REQUEST)?;
    if !child.can_move {
        return Err(StatusCode::BAD_REQUEST);
    }

    let parent = child
        .allowed_parents
        .iter()
        .find(|entry| entry.parent_node_id == update.parent_node_id.trim())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let attachment_preferences = validate_attachment_preferences(&update, parent)?;
    let health = TopologyAttachmentHealthStateFile::load(config.as_ref()).unwrap_or_default();

    let previous_overrides =
        TopologyOverridesFile::load().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut candidate_overrides = previous_overrides.clone();
    let changed = candidate_overrides.set_override_return_changed(
        child.node_id.clone(),
        child.node_name.clone(),
        parent.parent_node_id.clone(),
        parent.parent_node_name.clone(),
        update.mode,
        attachment_preferences,
    );
    if changed {
        publish_candidate_overrides(
            config.as_ref(),
            &health,
            &previous_overrides,
            &candidate_overrides,
        )?;
    }

    Ok(build_topology_manager_state_from_inputs(
        login,
        config.as_ref(),
        &canonical,
        &candidate_overrides,
        &health,
    ))
}

/// Removes one saved topology-manager branch move.
pub fn clear_topology_manager_override(
    login: LoginResult,
    clear: TopologyManagerClear,
) -> Result<TopologyManagerStateData, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if clear.child_node_id.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let canonical = TopologyEditorStateFile::load_with_legacy_fallback(config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let health = TopologyAttachmentHealthStateFile::load(config.as_ref()).unwrap_or_default();
    let previous_overrides =
        TopologyOverridesFile::load().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut candidate_overrides = previous_overrides.clone();
    let removed =
        candidate_overrides.remove_override_by_child_node_id_count(clear.child_node_id.trim());
    if removed > 0 {
        publish_candidate_overrides(
            config.as_ref(),
            &health,
            &previous_overrides,
            &candidate_overrides,
        )?;
    }

    Ok(build_topology_manager_state_from_inputs(
        login,
        config.as_ref(),
        &canonical,
        &candidate_overrides,
        &health,
    ))
}

/// Saves or replaces one attachment-pair probe policy.
pub fn set_topology_manager_probe_policy(
    login: LoginResult,
    update: TopologyManagerProbePolicyUpdate,
) -> Result<TopologyManagerStateData, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let pair_id = update.attachment_pair_id.trim();
    if pair_id.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let canonical = TopologyEditorStateFile::load_with_legacy_fallback(config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let previous_overrides =
        TopologyOverridesFile::load().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let health = TopologyAttachmentHealthStateFile::load(config.as_ref()).unwrap_or_default();
    let effective =
        compute_effective_state(config.as_ref(), &canonical, &previous_overrides, &health);
    let merged = merged_topology_state(
        config.as_ref(),
        &canonical,
        &previous_overrides,
        &health,
        &effective,
    );

    let known_pair_ids = merged
        .nodes
        .iter()
        .flat_map(|node| node.allowed_parents.iter())
        .flat_map(|parent| parent.attachment_options.iter())
        .filter_map(|option| option.pair_id.as_deref())
        .collect::<HashSet<_>>();
    if !known_pair_ids.contains(pair_id) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut candidate_overrides = previous_overrides.clone();
    let changed =
        candidate_overrides.set_probe_policy_return_changed(pair_id.to_string(), update.enabled);
    if changed {
        publish_candidate_overrides(
            config.as_ref(),
            &health,
            &previous_overrides,
            &candidate_overrides,
        )?;
    }
    Ok(build_topology_manager_state_from_inputs(
        login,
        config.as_ref(),
        &canonical,
        &candidate_overrides,
        &health,
    ))
}

/// Saves or replaces one attachment-scoped rate override.
pub fn set_topology_manager_attachment_rate_override(
    login: LoginResult,
    update: TopologyManagerAttachmentRateOverrideUpdate,
) -> Result<TopologyManagerStateData, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if update.child_node_id.trim().is_empty()
        || update.parent_node_id.trim().is_empty()
        || update.attachment_id.trim().is_empty()
        || update.download_bandwidth_mbps == 0
        || update.upload_bandwidth_mbps == 0
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let canonical = TopologyEditorStateFile::load_with_legacy_fallback(config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let previous_overrides =
        TopologyOverridesFile::load().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let health = TopologyAttachmentHealthStateFile::load(config.as_ref()).unwrap_or_default();
    let effective =
        compute_effective_state(config.as_ref(), &canonical, &previous_overrides, &health);
    let merged = merged_topology_state(
        config.as_ref(),
        &canonical,
        &previous_overrides,
        &health,
        &effective,
    );

    let child = merged
        .find_node(update.child_node_id.trim())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let parent = child
        .allowed_parents
        .iter()
        .find(|entry| entry.parent_node_id == update.parent_node_id.trim())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let option = parent
        .attachment_options
        .iter()
        .find(|entry| entry.attachment_id == update.attachment_id.trim())
        .ok_or(StatusCode::BAD_REQUEST)?;
    if !option.can_override_rate || option.attachment_id == lqos_config::TOPOLOGY_ATTACHMENT_AUTO_ID
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut candidate_overrides = previous_overrides.clone();
    let changed = candidate_overrides.set_attachment_rate_override_return_changed(
        child.node_id.clone(),
        parent.parent_node_id.clone(),
        option.attachment_id.clone(),
        update.download_bandwidth_mbps,
        update.upload_bandwidth_mbps,
    );
    if changed {
        publish_candidate_overrides(
            config.as_ref(),
            &health,
            &previous_overrides,
            &candidate_overrides,
        )?;
    }

    Ok(build_topology_manager_state_from_inputs(
        login,
        config.as_ref(),
        &canonical,
        &candidate_overrides,
        &health,
    ))
}

/// Removes one attachment-scoped rate override.
pub fn clear_topology_manager_attachment_rate_override(
    login: LoginResult,
    clear: TopologyManagerAttachmentRateOverrideClear,
) -> Result<TopologyManagerStateData, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if clear.child_node_id.trim().is_empty()
        || clear.parent_node_id.trim().is_empty()
        || clear.attachment_id.trim().is_empty()
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let canonical = TopologyEditorStateFile::load_with_legacy_fallback(config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let health = TopologyAttachmentHealthStateFile::load(config.as_ref()).unwrap_or_default();
    let previous_overrides =
        TopologyOverridesFile::load().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut candidate_overrides = previous_overrides.clone();
    let removed = candidate_overrides.remove_attachment_rate_override_count(
        clear.child_node_id.trim(),
        clear.parent_node_id.trim(),
        clear.attachment_id.trim(),
    );
    if removed > 0 {
        publish_candidate_overrides(
            config.as_ref(),
            &health,
            &previous_overrides,
            &candidate_overrides,
        )?;
    }

    Ok(build_topology_manager_state_from_inputs(
        login,
        config.as_ref(),
        &canonical,
        &candidate_overrides,
        &health,
    ))
}

/// Saves or replaces one manual attachment group.
pub fn set_topology_manager_manual_attachment_group(
    login: LoginResult,
    update: TopologyManagerManualAttachmentGroupUpdate,
) -> Result<TopologyManagerStateData, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let canonical = TopologyEditorStateFile::load_with_legacy_fallback(config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let child = canonical
        .find_node(update.child_node_id.trim())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let parent = child
        .allowed_parents
        .iter()
        .find(|entry| entry.parent_node_id == update.parent_node_id.trim())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let attachments = validate_manual_attachments(&update)?;
    let health = TopologyAttachmentHealthStateFile::load(config.as_ref()).unwrap_or_default();
    let previous_overrides =
        TopologyOverridesFile::load().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let previous_attachment_ids = previous_overrides
        .find_manual_attachment_group(update.child_node_id.trim(), update.parent_node_id.trim())
        .map(|group| {
            group
                .attachments
                .iter()
                .map(|attachment| attachment.attachment_id.clone())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    if manual_attachment_ids_conflict(&previous_overrides, &update, &attachments) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut candidate_overrides = previous_overrides.clone();
    let changed = candidate_overrides.set_manual_attachment_group_return_changed(
        child.node_id.clone(),
        child.node_name.clone(),
        parent.parent_node_id.clone(),
        parent.parent_node_name.clone(),
        attachments.clone(),
    );
    let mut policy_changed = false;
    for attachment in &attachments {
        policy_changed |= candidate_overrides.set_probe_policy_return_changed(
            attachment.attachment_id.clone(),
            attachment.probe_enabled,
        );
    }
    let current_attachment_ids = attachments
        .iter()
        .map(|attachment| attachment.attachment_id.as_str())
        .collect::<HashSet<_>>();
    let mut rate_override_removed = 0usize;
    for previous_attachment_id in previous_attachment_ids {
        if current_attachment_ids.contains(previous_attachment_id.as_str()) {
            continue;
        }
        rate_override_removed = rate_override_removed.saturating_add(
            candidate_overrides.remove_attachment_rate_override_count(
                update.child_node_id.trim(),
                update.parent_node_id.trim(),
                &previous_attachment_id,
            ),
        );
    }
    if changed || policy_changed || rate_override_removed > 0 {
        publish_candidate_overrides(
            config.as_ref(),
            &health,
            &previous_overrides,
            &candidate_overrides,
        )?;
    }

    Ok(build_topology_manager_state_from_inputs(
        login,
        config.as_ref(),
        &canonical,
        &candidate_overrides,
        &health,
    ))
}

/// Removes one saved manual attachment group.
pub fn clear_topology_manager_manual_attachment_group(
    login: LoginResult,
    clear: TopologyManagerManualAttachmentGroupClear,
) -> Result<TopologyManagerStateData, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if clear.child_node_id.trim().is_empty() || clear.parent_node_id.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let canonical = TopologyEditorStateFile::load_with_legacy_fallback(config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let health = TopologyAttachmentHealthStateFile::load(config.as_ref()).unwrap_or_default();
    let previous_overrides =
        TopologyOverridesFile::load().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let attachment_ids = previous_overrides
        .find_manual_attachment_group(clear.child_node_id.trim(), clear.parent_node_id.trim())
        .map(|group| {
            group
                .attachments
                .iter()
                .map(|attachment| attachment.attachment_id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut candidate_overrides = previous_overrides.clone();
    let removed = candidate_overrides.remove_manual_attachment_group_count(
        clear.child_node_id.trim(),
        clear.parent_node_id.trim(),
    );
    let mut policy_removed = 0usize;
    let mut rate_override_removed = 0usize;
    for attachment_id in attachment_ids {
        policy_removed = policy_removed
            .saturating_add(candidate_overrides.remove_probe_policy_count(&attachment_id));
        rate_override_removed = rate_override_removed.saturating_add(
            candidate_overrides.remove_attachment_rate_override_count(
                clear.child_node_id.trim(),
                clear.parent_node_id.trim(),
                &attachment_id,
            ),
        );
    }
    if removed > 0 || policy_removed > 0 || rate_override_removed > 0 {
        publish_candidate_overrides(
            config.as_ref(),
            &health,
            &previous_overrides,
            &candidate_overrides,
        )?;
    }

    Ok(build_topology_manager_state_from_inputs(
        login,
        config.as_ref(),
        &canonical,
        &candidate_overrides,
        &health,
    ))
}

fn build_topology_manager_state(
    login: LoginResult,
) -> Result<TopologyManagerStateData, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let canonical = TopologyEditorStateFile::load_with_legacy_fallback(config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let overrides = TopologyOverridesFile::load().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let health = TopologyAttachmentHealthStateFile::load(config.as_ref()).unwrap_or_default();
    Ok(build_topology_manager_state_from_inputs(
        login,
        config.as_ref(),
        &canonical,
        &overrides,
        &health,
    ))
}

fn build_topology_manager_state_from_inputs(
    login: LoginResult,
    config: &Config,
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> TopologyManagerStateData {
    let effective = compute_effective_state(config, canonical, overrides, health);
    let state = merged_topology_state(config, canonical, overrides, health, &effective);
    let effective_by_node_id = effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<std::collections::HashMap<_, _>>();
    let node_name_by_id = state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node.node_name.as_str()))
        .collect::<std::collections::HashMap<_, _>>();

    let mut nodes = Vec::with_capacity(state.nodes.len());
    for node in &state.nodes {
        let saved_override = overrides.find_override(&node.node_id);
        let mut warnings = Vec::new();
        let effective_node = effective_by_node_id.get(node.node_id.as_str()).copied();
        let effective_parent_node_id = effective_node
            .map(|entry| entry.logical_parent_node_id.trim())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| node.current_parent_node_id.clone());
        let effective_parent_node_name = effective_parent_node_id.as_deref().map(|parent_id| {
            node_name_by_id
                .get(parent_id)
                .map(|name| (*name).to_string())
                .or_else(|| {
                    node.allowed_parents
                        .iter()
                        .find(|entry| entry.parent_node_id == parent_id)
                        .map(|entry| entry.parent_node_name.clone())
                })
                .or_else(|| {
                    if node.current_parent_node_id.as_deref() == Some(parent_id) {
                        node.current_parent_node_name.clone()
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| parent_id.to_string())
        });

        let (
            has_override,
            override_parent_node_id,
            override_parent_node_name,
            override_mode,
            override_attachment_preference_ids,
            override_attachment_preference_names,
        ) = if let Some(saved_override) = saved_override {
            let maybe_parent = node
                .allowed_parents
                .iter()
                .find(|entry| entry.parent_node_id == saved_override.parent_node_id);
            if maybe_parent.is_none() {
                warnings.push(format!(
                    "Saved parent override is no longer valid under the current integration snapshot: {}",
                    saved_override.parent_node_name
                ));
            } else if let Some(parent) = maybe_parent {
                let valid_attachment_ids = parent
                    .attachment_options
                    .iter()
                    .map(|option| option.attachment_id.as_str())
                    .collect::<std::collections::HashSet<_>>();
                let stale_attachments = saved_override
                    .attachment_preference_ids
                    .iter()
                    .filter(|attachment_id| !valid_attachment_ids.contains(attachment_id.as_str()))
                    .cloned()
                    .collect::<Vec<_>>();
                if !stale_attachments.is_empty() {
                    warnings.push(format!(
                        "Saved attachment preferences are no longer available: {}",
                        stale_attachments.join(", ")
                    ));
                }
            }

            (
                true,
                Some(saved_override.parent_node_id.clone()),
                Some(saved_override.parent_node_name.clone()),
                Some(saved_override.mode),
                saved_override.attachment_preference_ids.clone(),
                saved_override.attachment_preference_names.clone(),
            )
        } else {
            (false, None, None, None, Vec::new(), Vec::new())
        };

        for rate_override in overrides
            .attachment_rate_overrides
            .iter()
            .filter(|entry| entry.child_node_id == node.node_id)
        {
            let Some(parent) = node
                .allowed_parents
                .iter()
                .find(|entry| entry.parent_node_id == rate_override.parent_node_id)
            else {
                warnings.push(format!(
                    "Saved attachment rate override is no longer valid because parent '{}' is unavailable.",
                    rate_override.parent_node_id
                ));
                continue;
            };
            let Some(option) = parent
                .attachment_options
                .iter()
                .find(|entry| entry.attachment_id == rate_override.attachment_id)
            else {
                warnings.push(format!(
                    "Saved attachment rate override is no longer valid because attachment '{}' is unavailable.",
                    rate_override.attachment_id
                ));
                continue;
            };
            if !option.can_override_rate {
                warnings.push(format!(
                    "Saved attachment rate override for '{}' is currently ignored: {}",
                    option.attachment_name,
                    option
                        .rate_override_disabled_reason
                        .clone()
                        .unwrap_or_else(|| "rate overrides are unavailable".to_string())
                ));
            }
        }

        let override_live = has_override
            && override_parent_node_id.as_deref() == effective_parent_node_id.as_deref();
        nodes.push(TopologyManagerNodeData {
            node_id: node.node_id.clone(),
            node_name: node.node_name.clone(),
            current_parent_node_id: node.current_parent_node_id.clone(),
            current_parent_node_name: node.current_parent_node_name.clone(),
            current_attachment_id: node.current_attachment_id.clone(),
            current_attachment_name: node.current_attachment_name.clone(),
            can_move: node.can_move,
            allowed_parents: node.allowed_parents.clone(),
            has_override,
            override_parent_node_id,
            override_parent_node_name,
            override_mode,
            override_attachment_preference_ids,
            override_attachment_preference_names,
            preferred_attachment_id: node.preferred_attachment_id.clone(),
            preferred_attachment_name: node.preferred_attachment_name.clone(),
            effective_attachment_id: node.effective_attachment_id.clone(),
            effective_attachment_name: node.effective_attachment_name.clone(),
            effective_parent_node_id: effective_parent_node_id.clone(),
            effective_parent_node_name: effective_parent_node_name.clone(),
            override_live,
            warnings,
        });
    }
    nodes.sort_unstable_by(|left, right| left.node_name.cmp(&right.node_name));

    let runtime_ids = state
        .nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let global_warnings = overrides
        .overrides
        .iter()
        .filter(|entry| !runtime_ids.contains(entry.child_node_id.as_str()))
        .map(|entry| {
            format!(
                "Saved topology override for '{}' is not present in the current topology snapshot.",
                entry.child_node_name
            )
        })
        .chain(
            overrides
                .attachment_rate_overrides
                .iter()
                .filter(|entry| !runtime_ids.contains(entry.child_node_id.as_str()))
                .map(|entry| {
                    format!(
                        "Saved attachment rate override for '{}' is not present in the current topology snapshot.",
                        entry.child_node_id
                    )
                }),
        )
        .collect::<Vec<_>>();

    TopologyManagerStateData {
        writable: login == LoginResult::Admin,
        source: state.source,
        schema_version: state.schema_version,
        nodes,
        global_warnings,
    }
}

fn validate_manual_attachments(
    update: &TopologyManagerManualAttachmentGroupUpdate,
) -> Result<Vec<ManualAttachment>, StatusCode> {
    if update.attachments.len() < 2 || update.attachments.len() > 8 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut seen_ids = HashSet::<String>::new();
    let mut out = Vec::with_capacity(update.attachments.len());
    for attachment in &update.attachments {
        let attachment_id = attachment.attachment_id.trim();
        let attachment_name = attachment.attachment_name.trim();
        let local_probe_ip = attachment.local_probe_ip.trim();
        let remote_probe_ip = attachment.remote_probe_ip.trim();
        if attachment_id.is_empty()
            || attachment_name.is_empty()
            || local_probe_ip.is_empty()
            || remote_probe_ip.is_empty()
            || attachment.capacity_mbps == 0
            || local_probe_ip == remote_probe_ip
            || !seen_ids.insert(attachment_id.to_string())
            || local_probe_ip.parse::<IpAddr>().is_err()
            || remote_probe_ip.parse::<IpAddr>().is_err()
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        out.push(ManualAttachment {
            attachment_id: attachment_id.to_string(),
            attachment_name: attachment_name.to_string(),
            capacity_mbps: attachment.capacity_mbps,
            local_probe_ip: local_probe_ip.to_string(),
            remote_probe_ip: remote_probe_ip.to_string(),
            probe_enabled: attachment.probe_enabled,
        });
    }
    Ok(out)
}

fn validate_attachment_preferences(
    update: &TopologyManagerUpdate,
    parent: &TopologyAllowedParent,
) -> Result<Vec<(String, String)>, StatusCode> {
    let allowed_attachment_names = parent
        .attachment_options
        .iter()
        .map(|option| {
            (
                option.attachment_id.as_str(),
                option.attachment_name.as_str(),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();

    match update.mode {
        TopologyAttachmentMode::Auto => Ok(Vec::new()),
        TopologyAttachmentMode::PreferredOrder => {
            let mut seen = std::collections::HashSet::new();
            let normalized = update
                .attachment_preference_ids
                .iter()
                .map(|id| id.trim())
                .filter(|id| !id.is_empty())
                .filter(|id| seen.insert((*id).to_string()))
                .collect::<Vec<_>>();
            if normalized.is_empty() {
                return Err(StatusCode::BAD_REQUEST);
            }

            let mut out = Vec::with_capacity(normalized.len());
            for attachment_id in normalized {
                let Some(name) = allowed_attachment_names.get(attachment_id) else {
                    return Err(StatusCode::BAD_REQUEST);
                };
                out.push((attachment_id.to_string(), (*name).to_string()));
            }
            Ok(out)
        }
    }
}

fn manual_attachment_ids_conflict(
    overrides: &TopologyOverridesFile,
    update: &TopologyManagerManualAttachmentGroupUpdate,
    attachments: &[ManualAttachment],
) -> bool {
    let group_key = (update.child_node_id.trim(), update.parent_node_id.trim());
    let submitted_ids = attachments
        .iter()
        .map(|attachment| attachment.attachment_id.as_str())
        .collect::<HashSet<_>>();
    overrides
        .manual_attachment_groups
        .iter()
        .filter(|group| (group.child_node_id.as_str(), group.parent_node_id.as_str()) != group_key)
        .flat_map(|group| group.attachments.iter())
        .any(|attachment| submitted_ids.contains(attachment.attachment_id.as_str()))
}

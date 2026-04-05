use crate::node_manager::auth::LoginResult;
use axum::http::StatusCode;
use lqos_config::{TopologyParentCandidate, TopologyParentCandidatesFile, load_config};
use lqos_overrides::{NetworkAdjustment, OverrideLayer, OverrideStore, TopologyParentOverrideMode};
use serde::{Deserialize, Serialize};

const GENERATED_NODE_ID_PREFIX: &str = "libreqos:generated:";

/// Query payload for inspecting the operator topology override state of a tree node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeTopologyOverrideQuery {
    /// Stable node identifier from `network.json`, when available.
    pub node_id: Option<String>,
    /// Display name of the selected node.
    pub node_name: String,
}

/// Inspector/view-model data for tree topology overrides.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeTopologyOverrideData {
    /// Whether the current session is allowed to persist overrides.
    pub writable: bool,
    /// Whether editing is allowed for this specific node and session.
    pub can_edit: bool,
    /// Human-readable reason editing is disabled, when applicable.
    pub disabled_reason: Option<String>,
    /// Whether an operator-owned topology override currently exists.
    pub has_override: bool,
    /// Stored override mode, when present.
    pub override_mode: Option<TopologyParentOverrideMode>,
    /// Stored override parent IDs.
    pub override_parent_node_ids: Vec<String>,
    /// Stored override parent names.
    pub override_parent_node_names: Vec<String>,
    /// Currently resolved immediate parent ID from the latest integration snapshot.
    pub current_parent_node_id: Option<String>,
    /// Currently resolved immediate parent name from the latest integration snapshot.
    pub current_parent_node_name: Option<String>,
    /// Immediate upstream parent candidates detected by the active integration.
    pub candidate_parents: Vec<TopologyParentCandidate>,
    /// Non-fatal warnings about stale or partially invalid overrides.
    pub warnings: Vec<String>,
}

/// Load the current topology override inspector data.
pub fn get_node_topology_override_data(
    login: LoginResult,
    query: NodeTopologyOverrideQuery,
) -> Result<NodeTopologyOverrideData, StatusCode> {
    build_node_topology_override_data(login, query)
}

fn build_node_topology_override_data(
    login: LoginResult,
    query: NodeTopologyOverrideQuery,
) -> Result<NodeTopologyOverrideData, StatusCode> {
    let overrides = OverrideStore::load_layer(OverrideLayer::Operator)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let candidate_meta = query
        .node_id
        .as_deref()
        .map(candidate_metadata_for_node)
        .transpose()?;

    let matched_override = query
        .node_id
        .as_deref()
        .and_then(|node_id| overrides.find_topology_parent_override(node_id));
    let mut warnings = Vec::new();
    let (has_override, override_mode, override_parent_node_ids, override_parent_node_names) =
        match matched_override {
            Some(NetworkAdjustment::TopologyParentOverride {
                mode,
                parent_node_ids,
                parent_node_names,
                ..
            }) => {
                if *mode != TopologyParentOverrideMode::Pinned {
                    warnings.push(
                        "Legacy preferred-upstream topology overrides are no longer editable from this UI; the first saved parent is treated as the pinned parent."
                            .to_string(),
                    );
                }
                (
                    true,
                    parent_node_ids
                        .first()
                        .map(|_| TopologyParentOverrideMode::Pinned),
                    parent_node_ids.first().into_iter().cloned().collect(),
                    parent_node_names.first().into_iter().cloned().collect(),
                )
            }
            _ => (false, None, Vec::new(), Vec::new()),
        };

    if let Some(candidate_meta) = candidate_meta.as_ref() {
        let candidate_ids: std::collections::HashSet<&str> = candidate_meta
            .candidate_parents
            .iter()
            .map(|candidate| candidate.node_id.as_str())
            .collect();
        let missing_ids: Vec<&str> = override_parent_node_ids
            .iter()
            .map(String::as_str)
            .filter(|parent_id| !candidate_ids.contains(parent_id))
            .collect();
        if !missing_ids.is_empty() {
            warnings.push(format!(
                "Saved parent override references parent IDs that are no longer detected: {}",
                missing_ids.join(", ")
            ));
        }
    }

    let disabled_reason = edit_disabled_reason(login, &query)?;
    Ok(NodeTopologyOverrideData {
        writable: login == LoginResult::Admin,
        can_edit: disabled_reason.is_none(),
        disabled_reason,
        has_override,
        override_mode,
        override_parent_node_ids,
        override_parent_node_names,
        current_parent_node_id: candidate_meta
            .as_ref()
            .and_then(|meta| meta.current_parent_node_id.clone()),
        current_parent_node_name: candidate_meta
            .as_ref()
            .and_then(|meta| meta.current_parent_node_name.clone()),
        candidate_parents: candidate_meta
            .as_ref()
            .map(|meta| meta.candidate_parents.clone())
            .unwrap_or_default(),
        warnings,
    })
}

fn candidate_metadata_for_node(
    node_id: &str,
) -> Result<lqos_config::TopologyParentCandidatesNode, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let file = TopologyParentCandidatesFile::load(config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    file.find_node(node_id)
        .cloned()
        .ok_or(StatusCode::BAD_REQUEST)
}

fn topology_override_feature_enabled() -> Result<bool, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(config.uisp_integration.enable_uisp
        && matches!(
            config.uisp_integration.strategy.to_lowercase().as_str(),
            "full" | "full2"
        ))
}

fn edit_disabled_reason(
    login: LoginResult,
    query: &NodeTopologyOverrideQuery,
) -> Result<Option<String>, StatusCode> {
    if login != LoginResult::Admin {
        return Ok(Some(
            "Only administrators can edit topology overrides.".to_string(),
        ));
    }
    if !topology_override_feature_enabled()? {
        return Ok(Some(
            "Topology overrides are currently available only for the UISP full strategy."
                .to_string(),
        ));
    }
    let trimmed_name = query.node_name.trim();
    if trimmed_name.is_empty() {
        return Ok(Some(
            "This node cannot be edited because it does not expose a stable name.".into(),
        ));
    }
    let Some(node_id) = query.node_id.as_deref() else {
        if trimmed_name == "Root" {
            return Ok(Some(
                "The Root node has no upstream parent, so a pinned-parent override does not apply."
                    .to_string(),
            ));
        }
        return Ok(Some(
            "This node cannot be edited from the tree because it does not expose a stable node ID."
                .to_string(),
        ));
    };
    if node_id.starts_with(GENERATED_NODE_ID_PREFIX) {
        return Ok(Some(
            "Generated nodes cannot be edited from the tree.".to_string(),
        ));
    }

    let candidate_meta = candidate_metadata_for_node(node_id);
    match candidate_meta {
        Ok(meta) => {
            if meta.candidate_parents.is_empty() {
                Ok(Some(
                    "No detected upstream candidates are currently available for this node."
                        .to_string(),
                ))
            } else {
                Ok(None)
            }
        }
        Err(StatusCode::BAD_REQUEST) => Ok(Some(
            "This node is not currently exposed by the active topology override candidate source."
                .to_string(),
        )),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn query_round_trips_basic_fields() {
        let query = super::NodeTopologyOverrideQuery {
            node_id: Some("uisp:site:site-t2".to_string()),
            node_name: "T2".to_string(),
        };
        assert_eq!(query.node_id.as_deref(), Some("uisp:site:site-t2"));
        assert_eq!(query.node_name, "T2");
    }
}

use crate::node_manager::auth::LoginResult;
use axum::http::StatusCode;
use lqos_config::load_config;
use lqos_overrides::{NetworkAdjustment, OverrideLayer, OverrideStore};
use serde::{Deserialize, Serialize};
use std::path::Path;

const GENERATED_NODE_ID_PREFIX: &str = "libreqos:generated:";
const GENERATED_NODE_NAME_PREFIX: &str = "(Generated Site) ";
const LEGACY_UISP_FILE: &str = "integrationUISPbandwidths.csv";
const LEGACY_SPLYNX_FILE: &str = "integrationSplynxBandwidths.csv";

/// Query payload for inspecting the operator rate override state of a tree node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeRateOverrideQuery {
    /// Stable node identifier from `network.json`, when available.
    pub node_id: Option<String>,
    /// Display name of the selected node.
    pub node_name: String,
}

/// Update payload for persisting an operator-owned tree node rate override.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeRateOverrideUpdate {
    /// Stable node identifier from `network.json`.
    pub node_id: String,
    /// Display name of the selected node.
    pub node_name: String,
    /// Override download bandwidth in Mbps.
    pub download_bandwidth_mbps: Option<f32>,
    /// Override upload bandwidth in Mbps.
    pub upload_bandwidth_mbps: Option<f32>,
}

/// Inspector/view-model data for the tree node rate override panel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeRateOverrideData {
    /// Whether the current session is allowed to persist overrides.
    pub writable: bool,
    /// Whether editing is allowed for this specific node and session.
    pub can_edit: bool,
    /// Human-readable reason editing is disabled, when applicable.
    pub disabled_reason: Option<String>,
    /// Whether an operator-owned override currently exists.
    pub has_override: bool,
    /// Stable node identifier stored on the matching override, when present.
    pub override_node_id: Option<String>,
    /// Stored operator override download bandwidth in Mbps.
    pub override_download_bandwidth_mbps: Option<f32>,
    /// Stored operator override upload bandwidth in Mbps.
    pub override_upload_bandwidth_mbps: Option<f32>,
    /// Warnings about active legacy integration bandwidth override files.
    pub legacy_warnings: Vec<String>,
}

/// Load the current tree node rate override inspector data.
pub fn get_node_rate_override_data(
    login: LoginResult,
    query: NodeRateOverrideQuery,
) -> Result<NodeRateOverrideData, StatusCode> {
    build_node_rate_override_data(login, query)
}

/// Save or replace the operator-owned rate override for a tree node.
pub fn set_node_rate_override_data(
    login: LoginResult,
    update: NodeRateOverrideUpdate,
) -> Result<NodeRateOverrideData, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    validate_update_payload(&update)?;

    let query = NodeRateOverrideQuery {
        node_id: Some(update.node_id.clone()),
        node_name: update.node_name.clone(),
    };
    if let Some(reason) = edit_disabled_reason(login, &query) {
        tracing::warn!(
            node_name = %update.node_name,
            node_id = %update.node_id,
            "Rejected tree node override save: {reason}"
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut overrides = OverrideStore::load_layer(OverrideLayer::Operator)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let changed = overrides.set_site_bandwidth_override(
        Some(update.node_id),
        update.node_name,
        update.download_bandwidth_mbps,
        update.upload_bandwidth_mbps,
    );
    if changed {
        OverrideStore::save_layer(OverrideLayer::Operator, &overrides)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    build_node_rate_override_data(login, query)
}

/// Remove the operator-owned rate override for a tree node.
pub fn clear_node_rate_override_data(
    login: LoginResult,
    query: NodeRateOverrideQuery,
) -> Result<NodeRateOverrideData, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if let Some(reason) = edit_disabled_reason(login, &query) {
        tracing::warn!(
            node_name = %query.node_name,
            node_id = %query.node_id.clone().unwrap_or_default(),
            "Rejected tree node override clear: {reason}"
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    let Some(node_id) = query.node_id.as_deref() else {
        return Err(StatusCode::BAD_REQUEST);
    };

    let mut overrides = OverrideStore::load_layer(OverrideLayer::Operator)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let removed = overrides.remove_site_bandwidth_override_count(Some(node_id), &query.node_name);
    if removed > 0 {
        OverrideStore::save_layer(OverrideLayer::Operator, &overrides)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    build_node_rate_override_data(login, query)
}

fn build_node_rate_override_data(
    login: LoginResult,
    query: NodeRateOverrideQuery,
) -> Result<NodeRateOverrideData, StatusCode> {
    let overrides = OverrideStore::load_layer(OverrideLayer::Operator)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let matched_override =
        overrides.find_site_bandwidth_override(query.node_id.as_deref(), &query.node_name);
    let (has_override, override_node_id, override_down, override_up) = match matched_override {
        Some(NetworkAdjustment::AdjustSiteSpeed {
            node_id,
            download_bandwidth_mbps,
            upload_bandwidth_mbps,
            ..
        }) => (
            true,
            node_id.clone(),
            *download_bandwidth_mbps,
            *upload_bandwidth_mbps,
        ),
        _ => (false, None, None, None),
    };

    let disabled_reason = edit_disabled_reason(login, &query);
    Ok(NodeRateOverrideData {
        writable: login == LoginResult::Admin,
        can_edit: disabled_reason.is_none(),
        disabled_reason,
        has_override,
        override_node_id,
        override_download_bandwidth_mbps: override_down,
        override_upload_bandwidth_mbps: override_up,
        legacy_warnings: legacy_warning_messages()?,
    })
}

fn edit_disabled_reason(login: LoginResult, query: &NodeRateOverrideQuery) -> Option<String> {
    if login != LoginResult::Admin {
        return Some("Only administrators can edit node rate overrides.".to_string());
    }
    let trimmed_name = query.node_name.trim();
    if trimmed_name.is_empty() {
        return Some("This node cannot be edited because it does not expose a stable name.".into());
    }
    let Some(node_id) = query.node_id.as_deref() else {
        return Some(
            "This node cannot be edited from the tree because it does not expose a stable node ID."
                .to_string(),
        );
    };
    if node_id.starts_with(GENERATED_NODE_ID_PREFIX)
        || trimmed_name.starts_with(GENERATED_NODE_NAME_PREFIX)
    {
        return Some("Generated nodes cannot be edited from the tree.".to_string());
    }
    None
}

fn validate_update_payload(update: &NodeRateOverrideUpdate) -> Result<(), StatusCode> {
    if update.node_id.trim().is_empty() || update.node_name.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if update.download_bandwidth_mbps.is_none() && update.upload_bandwidth_mbps.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }

    for value in [update.download_bandwidth_mbps, update.upload_bandwidth_mbps]
        .into_iter()
        .flatten()
    {
        if !value.is_finite() || value < 0.0 {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    Ok(())
}

fn legacy_warning_messages() -> Result<Vec<String>, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let base = Path::new(&config.lqos_directory);
    let mut warnings = Vec::new();

    if config.uisp_integration.enable_uisp && base.join(LEGACY_UISP_FILE).exists() {
        warnings.push(format!(
            "Legacy UISP bandwidth overrides are present in `{LEGACY_UISP_FILE}`. Tree edits write operator overrides to `lqos_overrides.json` and do not modify that file."
        ));
    }
    if config.splynx_integration.enable_splynx && base.join(LEGACY_SPLYNX_FILE).exists() {
        warnings.push(format!(
            "Legacy Splynx bandwidth overrides are present in `{LEGACY_SPLYNX_FILE}`. Tree edits write operator overrides to `lqos_overrides.json` and do not modify that file."
        ));
    }

    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::{
        GENERATED_NODE_ID_PREFIX, GENERATED_NODE_NAME_PREFIX, NodeRateOverrideQuery,
        NodeRateOverrideUpdate, edit_disabled_reason, validate_update_payload,
    };
    use crate::node_manager::auth::LoginResult;
    use axum::http::StatusCode;

    #[test]
    fn read_only_sessions_cannot_edit_even_for_real_nodes() {
        let query = NodeRateOverrideQuery {
            node_id: Some("node-ap27".to_string()),
            node_name: "AP27".to_string(),
        };
        assert_eq!(
            edit_disabled_reason(LoginResult::ReadOnly, &query),
            Some("Only administrators can edit node rate overrides.".to_string())
        );
    }

    #[test]
    fn generated_nodes_are_blocked_for_admins() {
        let query = NodeRateOverrideQuery {
            node_id: Some(format!("{GENERATED_NODE_ID_PREFIX}site:ap27")),
            node_name: format!("{GENERATED_NODE_NAME_PREFIX}AP27"),
        };
        assert_eq!(
            edit_disabled_reason(LoginResult::Admin, &query),
            Some("Generated nodes cannot be edited from the tree.".to_string())
        );
    }

    #[test]
    fn missing_node_id_is_rejected_for_writes() {
        let query = NodeRateOverrideQuery {
            node_id: None,
            node_name: "AP27".to_string(),
        };
        assert_eq!(
            edit_disabled_reason(LoginResult::Admin, &query),
            Some(
                "This node cannot be edited from the tree because it does not expose a stable node ID."
                    .to_string()
            )
        );
    }

    #[test]
    fn update_payload_requires_non_negative_finite_rates() {
        let update = NodeRateOverrideUpdate {
            node_id: "node-ap27".to_string(),
            node_name: "AP27".to_string(),
            download_bandwidth_mbps: Some(-1.0),
            upload_bandwidth_mbps: Some(10.0),
        };
        assert_eq!(validate_update_payload(&update), Err(StatusCode::BAD_REQUEST));
    }
}

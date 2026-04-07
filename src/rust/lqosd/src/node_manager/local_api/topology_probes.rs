use crate::node_manager::auth::LoginResult;
use axum::http::StatusCode;
use lqos_config::{TopologyAttachmentHealthEntry, TopologyAttachmentHealthStateFile, load_config};
use serde::{Deserialize, Serialize};

/// Full page data for the topology probe debug page.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopologyProbesStateData {
    /// Unix timestamp when the health-state snapshot was generated.
    pub generated_unix: Option<u64>,
    /// Current probe-debug entries, one per attachment pair.
    pub entries: Vec<TopologyAttachmentHealthEntry>,
}

fn status_rank(status: lqos_config::TopologyAttachmentHealthStatus) -> u8 {
    match status {
        lqos_config::TopologyAttachmentHealthStatus::Suppressed => 0,
        lqos_config::TopologyAttachmentHealthStatus::ProbeUnavailable => 1,
        lqos_config::TopologyAttachmentHealthStatus::Disabled => 2,
        lqos_config::TopologyAttachmentHealthStatus::Healthy => 3,
    }
}

/// Loads the current topology probe debug state.
pub fn get_topology_probes_state(
    _login: LoginResult,
) -> Result<TopologyProbesStateData, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut state = TopologyAttachmentHealthStateFile::load(config.as_ref()).unwrap_or_default();
    state.attachments.sort_by(|left, right| {
        status_rank(left.status)
            .cmp(&status_rank(right.status))
            .then_with(|| {
                left.child_node_name
                    .as_deref()
                    .unwrap_or_default()
                    .cmp(right.child_node_name.as_deref().unwrap_or_default())
            })
            .then_with(|| {
                left.parent_node_name
                    .as_deref()
                    .unwrap_or_default()
                    .cmp(right.parent_node_name.as_deref().unwrap_or_default())
            })
            .then_with(|| {
                left.attachment_name
                    .as_deref()
                    .unwrap_or_default()
                    .cmp(right.attachment_name.as_deref().unwrap_or_default())
            })
            .then_with(|| left.attachment_pair_id.cmp(&right.attachment_pair_id))
    });
    Ok(TopologyProbesStateData {
        generated_unix: state.generated_unix,
        entries: state.attachments,
    })
}

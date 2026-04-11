use crate::state;
use lqos_config::NetworkJsonNode;

/// Canonical parent-node metadata resolved from `network.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedParentNode {
    /// Canonical node name from `network.json`.
    pub name: String,
    /// Optional stable node identifier from `network.json` metadata.
    pub id: Option<String>,
}

/// Resolve a shaped-device parent reference into canonical `network.json`
/// parent metadata, preferring a stable node ID when one is available.
pub fn resolve_parent_node_reference(
    parent_node: &str,
    parent_node_id: Option<&str>,
) -> Option<ResolvedParentNode> {
    let trimmed_id = parent_node_id.map(str::trim).filter(|id| !id.is_empty());
    let trimmed = parent_node.trim();
    if trimmed.is_empty() && trimmed_id.is_none() {
        return None;
    }

    state::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();

        if let Some(parent_node_id) = trimmed_id
            && let Some(node) = find_node_by_id(nodes, parent_node_id)
        {
            return Some(ResolvedParentNode {
                name: node.name.clone(),
                id: node.id.clone(),
            });
        }

        if let Some(node) = nodes.iter().find(|node| node.name == trimmed) {
            return Some(ResolvedParentNode {
                name: node.name.clone(),
                id: node.id.clone(),
            });
        }

        nodes.iter().find_map(|node| {
            node.active_attachment_name
                .as_deref()
                .filter(|alias| alias.trim() == trimmed)
                .map(|_| ResolvedParentNode {
                    name: node.name.clone(),
                    id: node.id.clone(),
                })
        })
    })
}

/// Resolve a shaped-device parent node or active attachment alias into canonical `network.json`
/// parent metadata.
pub fn resolve_parent_node(parent_node: &str) -> Option<ResolvedParentNode> {
    resolve_parent_node_reference(parent_node, None)
}

fn find_node_by_id<'a>(nodes: &'a [NetworkJsonNode], id: &str) -> Option<&'a NetworkJsonNode> {
    nodes.iter().find(|node| node.id.as_deref() == Some(id))
}

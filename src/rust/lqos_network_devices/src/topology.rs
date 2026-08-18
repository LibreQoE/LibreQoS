use crate::state;
use lqos_config::NetworkJsonNode;
use std::collections::HashMap;

/// Canonical parent-node metadata resolved from `network.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedParentNode {
    /// Canonical node name from `network.json`.
    pub name: String,
    /// Optional stable node identifier from `network.json` metadata.
    pub id: Option<String>,
}

/// Borrowed parent-node indexes for resolving a batch against one node snapshot.
#[derive(Clone, Debug, Default)]
pub struct ParentNodeLookup<'a> {
    by_id: HashMap<&'a str, &'a NetworkJsonNode>,
    by_name: HashMap<&'a str, &'a NetworkJsonNode>,
    by_alias: HashMap<&'a str, &'a NetworkJsonNode>,
}

impl<'a> ParentNodeLookup<'a> {
    /// Builds ID, name, and active-attachment alias indexes from a borrowed node slice.
    pub fn from_nodes(nodes: &'a [NetworkJsonNode]) -> Self {
        let mut lookup = Self::default();
        for node in nodes {
            if let Some(id) = node.id.as_deref() {
                lookup.by_id.entry(id).or_insert(node);
            }
            lookup.by_name.entry(node.name.as_str()).or_insert(node);
            if let Some(alias) = node.active_attachment_name.as_deref() {
                lookup.by_alias.entry(alias.trim()).or_insert(node);
            }
        }
        lookup
    }

    /// Resolves a parent reference with ID, canonical-name, then alias precedence.
    pub fn resolve(
        &self,
        parent_node: &str,
        parent_node_id: Option<&str>,
    ) -> Option<ResolvedParentNode> {
        resolve_parent_node_with(
            parent_node,
            parent_node_id,
            |id| self.by_id.get(id).copied(),
            |name| self.by_name.get(name).copied(),
            |alias| self.by_alias.get(alias).copied(),
        )
    }
}

/// Resolve a shaped-device parent reference into canonical `network.json`
/// parent metadata, preferring a stable node ID when one is available.
pub fn resolve_parent_node_reference(
    parent_node: &str,
    parent_node_id: Option<&str>,
) -> Option<ResolvedParentNode> {
    state::with_network_json_read(|net_json| {
        resolve_parent_node_reference_in_nodes(
            net_json.get_nodes_when_ready(),
            parent_node,
            parent_node_id,
        )
    })
}

/// Resolve a parent reference against an already-borrowed `network.json` node slice.
///
/// This preserves the canonical ID, name, and active-attachment alias precedence without
/// acquiring the shared `network.json` lock. Callers that already hold a network snapshot can
/// therefore resolve a batch of references under one read lock.
pub fn resolve_parent_node_reference_in_nodes(
    nodes: &[NetworkJsonNode],
    parent_node: &str,
    parent_node_id: Option<&str>,
) -> Option<ResolvedParentNode> {
    resolve_parent_node_with(
        parent_node,
        parent_node_id,
        |id| find_node_by_id(nodes, id),
        |name| nodes.iter().find(|node| node.name == name),
        |alias| {
            nodes.iter().find(|node| {
                node.active_attachment_name
                    .as_deref()
                    .is_some_and(|candidate| candidate.trim() == alias)
            })
        },
    )
}

/// Resolve a shaped-device parent node or active attachment alias into canonical `network.json`
/// parent metadata.
pub fn resolve_parent_node(parent_node: &str) -> Option<ResolvedParentNode> {
    resolve_parent_node_reference(parent_node, None)
}

fn find_node_by_id<'a>(nodes: &'a [NetworkJsonNode], id: &str) -> Option<&'a NetworkJsonNode> {
    nodes.iter().find(|node| node.id.as_deref() == Some(id))
}

fn resolved_parent_node(node: &NetworkJsonNode) -> ResolvedParentNode {
    ResolvedParentNode {
        name: node.name.clone(),
        id: node.id.clone(),
    }
}

fn resolve_parent_node_with<'a>(
    parent_node: &str,
    parent_node_id: Option<&str>,
    resolve_id: impl FnOnce(&str) -> Option<&'a NetworkJsonNode>,
    resolve_name: impl FnOnce(&str) -> Option<&'a NetworkJsonNode>,
    resolve_alias: impl FnOnce(&str) -> Option<&'a NetworkJsonNode>,
) -> Option<ResolvedParentNode> {
    let trimmed_id = parent_node_id.map(str::trim).filter(|id| !id.is_empty());
    let trimmed_name = parent_node.trim();
    if trimmed_name.is_empty() && trimmed_id.is_none() {
        return None;
    }
    trimmed_id
        .and_then(resolve_id)
        .or_else(|| resolve_name(trimmed_name))
        .or_else(|| resolve_alias(trimmed_name))
        .map(resolved_parent_node)
}

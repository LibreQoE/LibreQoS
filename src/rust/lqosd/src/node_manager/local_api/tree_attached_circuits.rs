use crate::shaped_devices_tracker::NETWORK_JSON;
use crate::shaped_devices_tracker::circuit_live::fresh_circuit_live_snapshot;
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample};
use serde::{Deserialize, Deserializer, Serialize};

const DEFAULT_ATTACHED_CIRCUITS_PAGE_SIZE: usize = 100;
const MAX_ATTACHED_CIRCUITS_PAGE_SIZE: usize = 250;

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum MaybeString {
        String(String),
        Null,
    }

    Ok(match Option::<MaybeString>::deserialize(deserializer)? {
        Some(MaybeString::String(value)) => Some(value),
        Some(MaybeString::Null) | None => None,
    })
}

/// Sort options for the tree page's attached-circuits table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TreeAttachedCircuitsSort {
    CircuitName,
    LastSeen,
    ThroughputDown,
    ThroughputUp,
}

/// Query for the selected node's attached-circuits table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TreeAttachedCircuitsQuery {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub node_id: Option<String>,
    pub node_path: Option<Vec<String>>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub search: Option<String>,
    pub sort: Option<TreeAttachedCircuitsSort>,
    pub descending: Option<bool>,
}

/// One aggregated circuit row for the tree page's attached-circuits table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TreeAttachedCircuitRow {
    pub circuit_id: String,
    pub circuit_name: String,
    pub parent_node: String,
    pub device_names: Vec<String>,
    pub ip_addrs: Vec<String>,
    pub plan_mbps: DownUpOrder<f32>,
    pub bytes_per_second: DownUpOrder<u64>,
    pub rtt_current_p50_nanos: DownUpOrder<Option<u64>>,
    pub qoo: DownUpOrder<Option<f32>>,
    pub tcp_retransmit_sample: DownUpOrder<TcpRetransmitSample>,
    pub last_seen_nanos: u64,
}

/// One server-paged response for the selected node's attached circuits.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TreeAttachedCircuitsPage {
    pub query: TreeAttachedCircuitsQuery,
    pub total_rows: usize,
    pub rows: Vec<TreeAttachedCircuitRow>,
}

fn normalized_page_size(query: &TreeAttachedCircuitsQuery) -> usize {
    query
        .page_size
        .unwrap_or(DEFAULT_ATTACHED_CIRCUITS_PAGE_SIZE)
        .clamp(1, MAX_ATTACHED_CIRCUITS_PAGE_SIZE)
}

fn resolve_node_name(query: &TreeAttachedCircuitsQuery) -> Option<String> {
    let reader = NETWORK_JSON.read();
    let nodes = reader.get_nodes_when_ready();

    if let Some(node_id) = query.node_id.as_deref()
        && let Some(node) = nodes
            .iter()
            .find(|node| node.id.as_deref() == Some(node_id))
    {
        return Some(node.name.clone());
    }

    let node_path = query.node_path.as_ref()?;
    if node_path.is_empty() {
        return None;
    }
    nodes.iter().find_map(|node| {
        let path = if node.name == "Root" {
            vec!["Root".to_string()]
        } else {
            let parent_indexes = if node.parents.is_empty() {
                vec![node.immediate_parent.unwrap_or(0)]
            } else {
                node.parents.clone()
            };
            let mut names = Vec::new();
            for idx in parent_indexes {
                let parent = nodes.get(idx)?;
                names.push(parent.name.clone());
            }
            if names.last() != Some(&node.name) {
                names.push(node.name.clone());
            }
            names
        };
        if &path == node_path {
            Some(node.name.clone())
        } else {
            None
        }
    })
}

/// Returns one filtered, sorted page of circuits attached to the selected tree node.
pub fn tree_attached_circuits(query: TreeAttachedCircuitsQuery) -> TreeAttachedCircuitsPage {
    let page = query.page.unwrap_or(0);
    let page_size = normalized_page_size(&query);
    let search = query.search.as_deref().unwrap_or("").trim().to_lowercase();
    let sort = query
        .sort
        .clone()
        .unwrap_or(TreeAttachedCircuitsSort::CircuitName);
    let descending = query.descending.unwrap_or(false);
    let node_name = resolve_node_name(&query);

    let snapshot = fresh_circuit_live_snapshot();
    let mut rows: Vec<TreeAttachedCircuitRow> = match node_name.as_deref() {
        Some(node_name) => snapshot
            .circuit_ids_by_parent_node
            .get(node_name)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| snapshot.by_circuit_id.get(id))
            .map(|row| TreeAttachedCircuitRow {
                circuit_id: row.circuit_id.clone(),
                circuit_name: row.circuit_name.clone(),
                parent_node: row.parent_node.clone(),
                device_names: row.device_names.clone(),
                ip_addrs: row.ip_addrs.clone(),
                plan_mbps: row.plan_mbps,
                bytes_per_second: row.bytes_per_second,
                rtt_current_p50_nanos: row.rtt_current_p50_nanos,
                qoo: row.qoo,
                tcp_retransmit_sample: row.tcp_retransmit_sample,
                last_seen_nanos: row.last_seen_nanos,
            })
            .collect(),
        None => Vec::new(),
    };

    if !search.is_empty() {
        rows.retain(|row| {
            row.circuit_name.to_lowercase().contains(&search)
                || row.circuit_id.to_lowercase().contains(&search)
                || row.parent_node.to_lowercase().contains(&search)
                || row
                    .device_names
                    .iter()
                    .any(|name| name.to_lowercase().contains(&search))
                || row
                    .ip_addrs
                    .iter()
                    .any(|ip| ip.to_lowercase().contains(&search))
        });
    }

    rows.sort_by(|left, right| match sort {
        TreeAttachedCircuitsSort::CircuitName => left
            .circuit_name
            .cmp(&right.circuit_name)
            .then_with(|| left.circuit_id.cmp(&right.circuit_id)),
        TreeAttachedCircuitsSort::LastSeen => left
            .last_seen_nanos
            .cmp(&right.last_seen_nanos)
            .then_with(|| left.circuit_name.cmp(&right.circuit_name)),
        TreeAttachedCircuitsSort::ThroughputDown => left
            .bytes_per_second
            .down
            .cmp(&right.bytes_per_second.down)
            .then_with(|| left.circuit_name.cmp(&right.circuit_name)),
        TreeAttachedCircuitsSort::ThroughputUp => left
            .bytes_per_second
            .up
            .cmp(&right.bytes_per_second.up)
            .then_with(|| left.circuit_name.cmp(&right.circuit_name)),
    });
    if descending {
        rows.reverse();
    }

    let total_rows = rows.len();
    let start = page.saturating_mul(page_size);
    let rows = if start >= total_rows {
        Vec::new()
    } else {
        let end = (start + page_size).min(total_rows);
        rows[start..end].to_vec()
    };

    TreeAttachedCircuitsPage {
        query: TreeAttachedCircuitsQuery {
            node_id: query.node_id,
            node_path: query.node_path,
            page: Some(page),
            page_size: Some(page_size),
            search: if search.is_empty() {
                None
            } else {
                query.search
            },
            sort: Some(sort),
            descending: Some(descending),
        },
        total_rows,
        rows,
    }
}

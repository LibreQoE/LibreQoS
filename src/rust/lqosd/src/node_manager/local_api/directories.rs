use crate::node_manager::local_api::network_tree;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_CIRCUIT_DIRECTORY_PAGE_SIZE: usize = 100;
const MAX_CIRCUIT_DIRECTORY_PAGE_SIZE: usize = 250;

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

/// Query parameters for the paged circuit directory used by configuration UIs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitDirectoryQuery {
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub search: Option<String>,
}

/// Compact circuit metadata row for selectors and lookups.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitDirectoryRow {
    pub circuit_id: String,
    pub circuit_name: String,
    pub parent_node: String,
    pub sqm_override: Option<String>,
}

/// A server-paged slice of the circuit directory.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitDirectoryPage {
    pub query: CircuitDirectoryQuery,
    pub total_rows: usize,
    pub rows: Vec<CircuitDirectoryRow>,
}

/// Lightweight node directory entry for UI link resolution and selectors.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NodeDirectoryEntry {
    pub tree_index: usize,
    pub node_id: Option<String>,
    pub node_name: String,
    pub node_type: Option<String>,
}

/// Small TreeGuard summary used by dashboard/status panels.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TreeGuardMetadataSummary {
    pub total_nodes: usize,
    pub total_circuits: usize,
    pub virtualized_nodes: usize,
    pub fq_codel_circuits: usize,
}

fn normalized_page_size(query: &CircuitDirectoryQuery) -> usize {
    query
        .page_size
        .unwrap_or(DEFAULT_CIRCUIT_DIRECTORY_PAGE_SIZE)
        .clamp(1, MAX_CIRCUIT_DIRECTORY_PAGE_SIZE)
}

fn has_fq_codel_override(raw: &str) -> bool {
    raw.split('/')
        .map(|part| part.trim().to_ascii_lowercase())
        .any(|part| part == "fq_codel")
}

/// Returns one filtered, sorted page of circuit directory rows.
pub fn circuit_directory_page(query: CircuitDirectoryQuery) -> CircuitDirectoryPage {
    let page = query.page.unwrap_or(0);
    let page_size = normalized_page_size(&query);
    let search = query.search.as_deref().unwrap_or("").trim().to_lowercase();
    let devices = lqos_network_devices::shaped_devices_snapshot();

    let mut circuits: BTreeMap<String, CircuitDirectoryRow> = BTreeMap::new();
    for device in &devices.devices {
        let circuit_id = device.circuit_id.trim().to_string();
        if circuit_id.is_empty() {
            continue;
        }
        let row = circuits
            .entry(circuit_id.clone())
            .or_insert_with(|| CircuitDirectoryRow {
                circuit_id: circuit_id.clone(),
                circuit_name: device.circuit_name.trim().to_string(),
                parent_node: device.parent_node.trim().to_string(),
                sqm_override: {
                    let override_value = device.sqm_override.as_deref().unwrap_or("").trim();
                    if override_value.is_empty() {
                        None
                    } else {
                        Some(override_value.to_string())
                    }
                },
            });
        if row.circuit_name.is_empty() && !device.circuit_name.trim().is_empty() {
            row.circuit_name = device.circuit_name.trim().to_string();
        }
        if row.parent_node.is_empty() && !device.parent_node.trim().is_empty() {
            row.parent_node = device.parent_node.trim().to_string();
        }
        let sqm_override = device.sqm_override.as_deref().unwrap_or("").trim();
        if row.sqm_override.is_none() && !sqm_override.is_empty() {
            row.sqm_override = Some(sqm_override.to_string());
        }
    }

    let mut filtered: Vec<CircuitDirectoryRow> = circuits
        .into_values()
        .filter(|row| {
            if search.is_empty() {
                return true;
            }
            row.circuit_id.to_lowercase().contains(&search)
                || row.circuit_name.to_lowercase().contains(&search)
                || row.parent_node.to_lowercase().contains(&search)
                || row
                    .sqm_override
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&search)
        })
        .collect();
    filtered.sort_by(|left, right| {
        left.circuit_name
            .cmp(&right.circuit_name)
            .then_with(|| left.circuit_id.cmp(&right.circuit_id))
    });

    let total_rows = filtered.len();
    let start = page.saturating_mul(page_size);
    let rows = if start >= total_rows {
        Vec::new()
    } else {
        let end = (start + page_size).min(total_rows);
        filtered[start..end].to_vec()
    };

    CircuitDirectoryPage {
        query: CircuitDirectoryQuery {
            page: Some(page),
            page_size: Some(page_size),
            search: if search.is_empty() {
                None
            } else {
                query.search
            },
        },
        total_rows,
        rows,
    }
}

/// Returns a compact flattened node directory for UI link resolution.
pub fn node_directory_data() -> Vec<NodeDirectoryEntry> {
    let mut nodes = network_tree::network_tree_data()
        .into_iter()
        .filter_map(|(tree_index, node)| {
            let node_name = node.name.trim().to_string();
            if node_name.is_empty() || node_name == "Root" {
                return None;
            }
            Some(NodeDirectoryEntry {
                tree_index,
                node_id: node.id.clone(),
                node_name,
                node_type: node.node_type.clone(),
            })
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.node_name
            .cmp(&right.node_name)
            .then_with(|| left.tree_index.cmp(&right.tree_index))
    });
    nodes
}

/// Returns a compact TreeGuard metadata summary without loading full device/node payloads into the browser.
pub fn treeguard_metadata_summary() -> TreeGuardMetadataSummary {
    let network = network_tree::network_tree_data();
    let total_nodes = network
        .iter()
        .filter(|(_, node)| {
            let name = node.name.trim();
            !name.is_empty() && name != "Root"
        })
        .count();
    let virtualized_nodes = network
        .iter()
        .filter(|(_, node)| {
            let name = node.name.trim();
            !name.is_empty() && name != "Root" && node.runtime_virtualized
        })
        .count();

    let devices = lqos_network_devices::shaped_devices_snapshot();
    let mut circuit_ids = BTreeSet::new();
    let mut fq_codel_circuit_ids = BTreeSet::new();
    for device in &devices.devices {
        let circuit_id = device.circuit_id.trim().to_string();
        if circuit_id.is_empty() {
            continue;
        }
        circuit_ids.insert(circuit_id.clone());
        if has_fq_codel_override(device.sqm_override.as_deref().unwrap_or("").trim()) {
            fq_codel_circuit_ids.insert(circuit_id);
        }
    }

    TreeGuardMetadataSummary {
        total_nodes,
        total_circuits: circuit_ids.len(),
        virtualized_nodes,
        fq_codel_circuits: fq_codel_circuit_ids.len(),
    }
}

use crate::node_manager::local_api::ethernet_caps::{EthernetCapBadge, ethernet_cap_badge_map};
use crate::shaped_devices_tracker::{
    NETWORK_JSON, SHAPED_DEVICES, effective_parent_for_circuit, resolve_parent_node_reference,
};
use crate::shaped_devices_tracker::circuit_live::fresh_circuit_live_snapshot;
use fxhash::{FxHashMap, FxHashSet};
use lqos_config::{ConfigShapedDevices, ShapedDevice};
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample};
use serde::{Deserialize, Deserializer, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

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
    /// Optional Ethernet speed-cap advisory badge when this circuit was auto-capped.
    pub ethernet_cap_badge: Option<EthernetCapBadge>,
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

#[derive(Clone, Debug, Default)]
struct SubtreeMatcher {
    node_names: FxHashSet<String>,
    node_ids: FxHashSet<String>,
}

#[derive(Clone, Debug, Default)]
struct CircuitRowAccumulator {
    circuit_id: String,
    circuit_name: String,
    parent_node: String,
    device_names: FxHashSet<String>,
    ip_addrs: FxHashSet<String>,
    plan_mbps: DownUpOrder<f32>,
}

fn normalized_page_size(query: &TreeAttachedCircuitsQuery) -> usize {
    query
        .page_size
        .unwrap_or(DEFAULT_ATTACHED_CIRCUITS_PAGE_SIZE)
        .clamp(1, MAX_ATTACHED_CIRCUITS_PAGE_SIZE)
}

fn resolve_selected_node_index(query: &TreeAttachedCircuitsQuery) -> Option<usize> {
    let reader = NETWORK_JSON.read();
    let nodes = reader.get_nodes_when_ready();

    if let Some(node_id) = query.node_id.as_deref()
        && let Some(node) = nodes
            .iter()
            .position(|node| node.id.as_deref() == Some(node_id))
    {
        return Some(node);
    }

    let node_path = query.node_path.as_ref()?;
    if node_path.is_empty() {
        return None;
    }
    nodes.iter().enumerate().find_map(|(idx, node)| {
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
            Some(idx)
        } else {
            None
        }
    })
}

fn build_subtree_matcher(query: &TreeAttachedCircuitsQuery) -> Option<SubtreeMatcher> {
    let selected_idx = resolve_selected_node_index(query)?;
    let reader = NETWORK_JSON.read();
    let nodes = reader.get_nodes_when_ready();
    let selected = nodes.get(selected_idx)?;

    let mut node_names = FxHashSet::default();
    let mut node_ids = FxHashSet::default();
    node_names.insert(selected.name.clone());
    if let Some(node_id) = selected.id.clone() {
        node_ids.insert(node_id);
    }

    for node in nodes {
        let mut current = Some(node);
        let mut in_subtree = false;
        while let Some(candidate) = current {
            if candidate.name == selected.name && candidate.id == selected.id {
                in_subtree = true;
                break;
            }
            current = candidate
                .immediate_parent
                .and_then(|parent_idx| nodes.get(parent_idx));
        }
        if !in_subtree {
            continue;
        }
        node_names.insert(node.name.clone());
        if let Some(node_id) = node.id.clone() {
            node_ids.insert(node_id);
        }
    }

    Some(SubtreeMatcher {
        node_names,
        node_ids,
    })
}

fn format_ipv4(addr: Ipv4Addr, prefix: u32) -> String {
    if prefix >= 32 {
        addr.to_string()
    } else {
        format!("{addr}/{prefix}")
    }
}

fn format_ipv6(addr: Ipv6Addr, prefix: u32) -> String {
    if prefix >= 128 {
        addr.to_string()
    } else {
        format!("{addr}/{prefix}")
    }
}

fn circuit_parent_matches_subtree(
    matcher: &SubtreeMatcher,
    parent_name: &str,
    parent_id: Option<&str>,
) -> bool {
    let trimmed_name = parent_name.trim();
    if !trimmed_name.is_empty() && matcher.node_names.contains(trimmed_name) {
        return true;
    }

    let trimmed_id = parent_id.map(str::trim).filter(|id| !id.is_empty());
    if let Some(parent_id) = trimmed_id
        && matcher.node_ids.contains(parent_id)
    {
        return true;
    }

    false
}

fn effective_or_canonical_parent(device: &ShapedDevice) -> Option<(String, Option<String>)> {
    if let Some(runtime_parent) = effective_parent_for_circuit(&device.circuit_id) {
        return Some((runtime_parent.name, runtime_parent.id));
    }

    resolve_parent_node_reference(&device.parent_node, device.parent_node_id.as_deref())
        .map(|resolved| (resolved.name, resolved.id))
        .or_else(|| {
            let trimmed = device.parent_node.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some((trimmed.to_string(), device.parent_node_id.clone()))
            }
        })
}

fn aggregate_attached_circuit_rows(
    matcher: &SubtreeMatcher,
    shaped_devices: &ConfigShapedDevices,
) -> Vec<CircuitRowAccumulator> {
    let mut rows: FxHashMap<String, CircuitRowAccumulator> = FxHashMap::default();

    for device in &shaped_devices.devices {
        if device.circuit_id.trim().is_empty() {
            continue;
        }
        let Some((parent_name, parent_id)) = effective_or_canonical_parent(device) else {
            continue;
        };
        if !circuit_parent_matches_subtree(matcher, &parent_name, parent_id.as_deref()) {
            continue;
        }

        let entry = rows
            .entry(device.circuit_id.clone())
            .or_insert_with(|| CircuitRowAccumulator {
                circuit_id: device.circuit_id.clone(),
                circuit_name: device.circuit_name.clone(),
                parent_node: parent_name.clone(),
                ..CircuitRowAccumulator::default()
            });
        if entry.circuit_name.is_empty() {
            entry.circuit_name = device.circuit_name.clone();
        }
        if entry.parent_node.is_empty() {
            entry.parent_node = parent_name;
        }
        if !device.device_name.trim().is_empty() {
            entry.device_names.insert(device.device_name.clone());
        }
        for (addr, prefix) in &device.ipv4 {
            entry.ip_addrs.insert(format_ipv4(*addr, *prefix));
        }
        for (addr, prefix) in &device.ipv6 {
            entry.ip_addrs.insert(format_ipv6(*addr, *prefix));
        }
        entry.plan_mbps.down = entry.plan_mbps.down.max(device.download_max_mbps.round());
        entry.plan_mbps.up = entry.plan_mbps.up.max(device.upload_max_mbps.round());
    }

    let mut rows: Vec<CircuitRowAccumulator> = rows.into_values().collect();
    rows.sort_by(|left, right| {
        left.circuit_name
            .cmp(&right.circuit_name)
            .then_with(|| left.circuit_id.cmp(&right.circuit_id))
    });
    rows
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
    let matcher = build_subtree_matcher(&query);

    let snapshot = fresh_circuit_live_snapshot();
    let ethernet_badges = ethernet_cap_badge_map();
    let shaped_devices = SHAPED_DEVICES.load();
    let mut rows: Vec<TreeAttachedCircuitRow> = matcher
        .as_ref()
        .map(|matcher| {
            aggregate_attached_circuit_rows(matcher, &shaped_devices)
                .into_iter()
                .map(|row| {
                    let live = snapshot.by_circuit_id.get(&row.circuit_id);
                    TreeAttachedCircuitRow {
                        circuit_id: row.circuit_id.clone(),
                        circuit_name: row.circuit_name.clone(),
                        ethernet_cap_badge: ethernet_badges
                            .get(&row.circuit_id.to_ascii_lowercase())
                            .cloned(),
                        parent_node: live
                            .map(|live| live.parent_node.clone())
                            .filter(|parent| !parent.trim().is_empty())
                            .unwrap_or(row.parent_node),
                        device_names: live
                            .map(|live| live.device_names.clone())
                            .unwrap_or_else(|| {
                                let mut names: Vec<String> =
                                    row.device_names.into_iter().collect();
                                names.sort_unstable();
                                names
                            }),
                        ip_addrs: live
                            .map(|live| live.ip_addrs.clone())
                            .unwrap_or_else(|| {
                                let mut addrs: Vec<String> = row.ip_addrs.into_iter().collect();
                                addrs.sort_unstable();
                                addrs
                            }),
                        plan_mbps: live.map(|live| live.plan_mbps).unwrap_or(row.plan_mbps),
                        bytes_per_second: live
                            .map(|live| live.bytes_per_second)
                            .unwrap_or_default(),
                        rtt_current_p50_nanos: live
                            .map(|live| live.rtt_current_p50_nanos)
                            .unwrap_or_default(),
                        qoo: live.map(|live| live.qoo).unwrap_or_default(),
                        tcp_retransmit_sample: live
                            .map(|live| live.tcp_retransmit_sample)
                            .unwrap_or_default(),
                        last_seen_nanos: live
                            .map(|live| live.last_seen_nanos)
                            .unwrap_or(u64::MAX),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn sample_device(
        circuit_id: &str,
        circuit_name: &str,
        device_id: &str,
        device_name: &str,
        parent_node: &str,
        parent_node_id: Option<&str>,
        ipv4: &[&str],
    ) -> ShapedDevice {
        ShapedDevice {
            circuit_id: circuit_id.to_string(),
            circuit_name: circuit_name.to_string(),
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
            parent_node: parent_node.to_string(),
            parent_node_id: parent_node_id.map(str::to_string),
            ipv4: ipv4
                .iter()
                .map(|value| {
                    (
                        Ipv4Addr::from_str(value).expect("test IPv4 addresses should parse"),
                        32,
                    )
                })
                .collect(),
            download_max_mbps: 100.0,
            upload_max_mbps: 50.0,
            ..ShapedDevice::default()
        }
    }

    #[test]
    fn aggregate_attached_rows_include_idle_circuits_in_selected_subtree() {
        let matcher = SubtreeMatcher {
            node_names: ["Parent A".to_string(), "Child A".to_string()]
                .into_iter()
                .collect(),
            node_ids: ["node-a".to_string(), "node-child".to_string()]
                .into_iter()
                .collect(),
        };
        let shaped_devices = ConfigShapedDevices {
            devices: vec![
                sample_device(
                    "circuit-1",
                    "Circuit One",
                    "device-1",
                    "Device One",
                    "Parent A",
                    Some("node-a"),
                    &["100.64.0.1"],
                ),
                sample_device(
                    "circuit-1",
                    "Circuit One",
                    "device-2",
                    "Device Two",
                    "Parent A",
                    Some("node-a"),
                    &["100.64.0.2"],
                ),
                sample_device(
                    "circuit-2",
                    "Circuit Two",
                    "device-3",
                    "Device Three",
                    "Elsewhere",
                    Some("node-b"),
                    &["100.64.0.3"],
                ),
            ],
            ..ConfigShapedDevices::default()
        };

        let rows = aggregate_attached_circuit_rows(&matcher, &shaped_devices);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].circuit_id, "circuit-1");
        assert_eq!(rows[0].circuit_name, "Circuit One");
        assert_eq!(rows[0].parent_node, "Parent A");
        assert_eq!(rows[0].device_names.len(), 2);
        assert_eq!(rows[0].ip_addrs.len(), 2);
    }
}

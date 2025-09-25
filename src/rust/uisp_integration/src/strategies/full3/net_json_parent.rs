use crate::strategies::full3::GraphType;
use crate::strategies::full3::graph_mapping::GraphMapping;
use lqos_config::Config;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Debug)]
pub struct NetJsonParent<'a> {
    pub parent_name: String,
    pub mapping: &'a GraphMapping,
    pub download: u64,
    pub upload: u64,
}

pub fn walk_parents(
    parents: &HashMap<String, NetJsonParent>,
    name: &String,
    node_info: &NetJsonParent,
    config: &Arc<Config>,
    graph: &GraphType,
    visited: &mut HashSet<String>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    // Entries are name, type, uisp_device or site, downloadBandwidthMbps, uploadBandwidthMbps, children
    map.insert("name".into(), name.clone().into());
    map.insert("downloadBandwidthMbps".into(), node_info.download.into());
    map.insert("uploadBandwidthMbps".into(), node_info.upload.into());
    match node_info.mapping {
        GraphMapping::Root { id, .. } | GraphMapping::Site { id, .. } => {
            map.insert("type".into(), "Site".into());
            map.insert("uisp_site".into(), id.clone().into());
            map.insert("parent_site".into(), name.clone().into());
        }
        GraphMapping::GeneratedSite { .. } => {
            map.insert("type".into(), "Site".into());
        }
        GraphMapping::AccessPoint {
            name: _,
            id,
            site_name,
            ..
        } => {
            map.insert("type".into(), "AP".into());
            map.insert("parent_site".into(), site_name.clone().into());
            map.insert("uisp_device".into(), id.clone().into());
        }
    }

    let mut children = serde_json::Map::new();
    for (name, node_info) in parents
        .iter()
        .filter(|(_node_name, node_info)| node_info.parent_name == *name)
    {
        if visited.contains(name) {
            continue;
        }
        visited.insert(name.to_string());
        let child = walk_parents(parents, name, &node_info, config, graph, visited);
        children.insert(name.into(), child.into());
    }

    map.insert("children".into(), children.into());

    map
}

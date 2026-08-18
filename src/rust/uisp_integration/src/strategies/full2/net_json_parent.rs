use crate::strategies::full2::graph_mapping::GraphMapping;
use std::collections::{HashMap, HashSet};

#[derive(Debug)]
pub struct NetJsonParent<'a> {
    pub node_id: String,
    pub node_name: String,
    pub export_name: String,
    pub parent_id: Option<String>,
    pub parent_name: String,
    pub mapping: &'a GraphMapping,
    pub download: u64,
    pub upload: u64,
}

fn export_name_candidates(base_name: &str, mapping: &GraphMapping, node_id: &str) -> Vec<String> {
    let kind_label = match mapping {
        GraphMapping::AccessPoint { .. } => "AP",
        GraphMapping::Root { .. }
        | GraphMapping::Site { .. }
        | GraphMapping::GeneratedSite { .. } => "Site",
    };
    let short_id = node_id.rsplit(':').next().unwrap_or(node_id);
    let short_id = &short_id[..short_id.len().min(8)];

    match mapping {
        GraphMapping::AccessPoint { .. } => vec![
            format!("{base_name} [AP]"),
            format!("{base_name} [AP {short_id}]"),
        ],
        GraphMapping::Root { .. }
        | GraphMapping::Site { .. }
        | GraphMapping::GeneratedSite { .. } => {
            vec![
                base_name.to_string(),
                format!("{base_name} [Site]"),
                format!("{base_name} [Site {short_id}]"),
                format!("{base_name} [{kind_label} {short_id}]"),
            ]
        }
    }
}

pub fn assign_export_names<'a, I>(nodes: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, &'a GraphMapping)>,
{
    let mut entries: Vec<(String, &'a GraphMapping)> = nodes.into_iter().collect();
    let mut name_counts = HashMap::<String, usize>::new();
    for (_node_id, mapping) in &entries {
        *name_counts.entry(mapping.name()).or_insert(0) += 1;
    }

    entries.sort_unstable_by(|(left_id, left_mapping), (right_id, right_mapping)| {
        left_mapping
            .name()
            .cmp(&right_mapping.name())
            .then_with(|| match (left_mapping, right_mapping) {
                (GraphMapping::AccessPoint { .. }, GraphMapping::AccessPoint { .. }) => {
                    left_id.cmp(right_id)
                }
                (GraphMapping::AccessPoint { .. }, _) => std::cmp::Ordering::Greater,
                (_, GraphMapping::AccessPoint { .. }) => std::cmp::Ordering::Less,
                _ => left_id.cmp(right_id),
            })
    });

    let mut export_names = HashMap::with_capacity(entries.len());
    let mut used_names = HashSet::with_capacity(entries.len());

    for (node_id, mapping) in entries {
        let base_name = mapping.name();
        let export_name = if name_counts.get(&base_name).copied().unwrap_or(0) <= 1 {
            base_name
        } else {
            export_name_candidates(&base_name, mapping, &node_id)
                .into_iter()
                .find(|candidate| !used_names.contains(candidate))
                .unwrap_or_else(|| format!("{base_name} [{}]", node_id))
        };
        used_names.insert(export_name.clone());
        export_names.insert(node_id, export_name);
    }

    export_names
}

pub fn walk_parents(
    parents: &HashMap<String, NetJsonParent>,
    node_id: &str,
    visited: &mut HashSet<String>,
) -> serde_json::Map<String, serde_json::Value> {
    let node_info = parents
        .get(node_id)
        .expect("walk_parents requires the current node to exist in the parents map");
    let mut map = serde_json::Map::new();
    // Entries are name, type, uisp_device or site, downloadBandwidthMbps, uploadBandwidthMbps, children
    map.insert("name".into(), node_info.export_name.clone().into());
    map.insert("id".into(), node_info.mapping.network_json_id().into());
    map.insert("downloadBandwidthMbps".into(), node_info.download.into());
    map.insert("uploadBandwidthMbps".into(), node_info.upload.into());
    match node_info.mapping {
        GraphMapping::Root {
            id,
            latitude,
            longitude,
            ..
        }
        | GraphMapping::Site {
            id,
            latitude,
            longitude,
            ..
        } => {
            map.insert("type".into(), "Site".into());
            map.insert("uisp_site".into(), id.clone().into());
            map.insert("parent_site".into(), node_info.parent_name.clone().into());
            if let Some(latitude) = latitude
                && let Some(number) = serde_json::Number::from_f64(*latitude as f64)
            {
                map.insert("latitude".into(), serde_json::Value::Number(number));
            }
            if let Some(longitude) = longitude
                && let Some(number) = serde_json::Number::from_f64(*longitude as f64)
            {
                map.insert("longitude".into(), serde_json::Value::Number(number));
            }
        }
        GraphMapping::GeneratedSite { .. } => {
            map.insert("type".into(), "Site".into());
        }
        GraphMapping::AccessPoint {
            name: _,
            id,
            site_name: _,
            ..
        } => {
            map.insert("type".into(), "AP".into());
            map.insert("parent_site".into(), node_info.parent_name.clone().into());
            map.insert("uisp_device".into(), id.clone().into());
        }
    }

    let mut children = serde_json::Map::new();
    let mut child_ids: Vec<&str> = parents
        .iter()
        .filter(|(_child_id, node_info)| node_info.parent_id.as_deref() == Some(node_id))
        .map(|(child_id, _node_info)| child_id.as_str())
        .collect();
    child_ids.sort_unstable_by(|left_id, right_id| {
        let left = parents
            .get(*left_id)
            .expect("child id should exist when sorting walk_parents output");
        let right = parents
            .get(*right_id)
            .expect("child id should exist when sorting walk_parents output");
        left.export_name
            .cmp(&right.export_name)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    for child_id in child_ids {
        let child_info = parents
            .get(child_id)
            .expect("child id should exist when building walk_parents output");
        if visited.contains(child_id) {
            continue;
        }
        visited.insert(child_id.to_string());
        let child = walk_parents(parents, child_id, visited);
        children.insert(child_info.export_name.clone(), child.into());
    }

    map.insert("children".into(), children.into());

    map
}

#[cfg(test)]
mod tests {
    use super::{NetJsonParent, assign_export_names, walk_parents};
    use crate::strategies::full2::graph_mapping::GraphMapping;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn emits_graph_parent_site_for_sites_and_access_points() {
        let root_mapping = GraphMapping::Root {
            name: "WestRedd-SiteRouter".to_string(),
            id: "site-root".to_string(),
            latitude: None,
            longitude: None,
        };
        let child_site_mapping = GraphMapping::Site {
            name: "MDC El Paso".to_string(),
            id: "site-mdc".to_string(),
            latitude: None,
            longitude: None,
        };
        let child_ap_mapping = GraphMapping::AccessPoint {
            name: "Core-WestRedd".to_string(),
            id: "device-core-westredd".to_string(),
            site_name: "MDC El Paso".to_string(),
            download_mbps: 5000,
            upload_mbps: 5000,
        };
        let export_names = assign_export_names([
            (root_mapping.network_json_id(), &root_mapping),
            (child_site_mapping.network_json_id(), &child_site_mapping),
            (child_ap_mapping.network_json_id(), &child_ap_mapping),
        ]);
        let root_id = root_mapping.network_json_id();
        let site_id = child_site_mapping.network_json_id();
        let ap_id = child_ap_mapping.network_json_id();

        let mut parents = HashMap::new();
        parents.insert(
            site_id.clone(),
            NetJsonParent {
                node_id: site_id.clone(),
                node_name: "MDC El Paso".to_string(),
                export_name: export_names.get(&site_id).unwrap().clone(),
                parent_id: Some(root_id.clone()),
                parent_name: "WestRedd-SiteRouter".to_string(),
                mapping: &child_site_mapping,
                download: 5000,
                upload: 5000,
            },
        );
        parents.insert(
            ap_id.clone(),
            NetJsonParent {
                node_id: ap_id.clone(),
                node_name: "Core-WestRedd".to_string(),
                export_name: export_names.get(&ap_id).unwrap().clone(),
                parent_id: Some(site_id.clone()),
                parent_name: "MDC El Paso".to_string(),
                mapping: &child_ap_mapping,
                download: 5000,
                upload: 5000,
            },
        );

        let root_node = NetJsonParent {
            node_id: root_id.clone(),
            node_name: "WestRedd-SiteRouter".to_string(),
            export_name: export_names.get(&root_id).unwrap().clone(),
            parent_id: None,
            parent_name: "WestRedd".to_string(),
            mapping: &root_mapping,
            download: 5000,
            upload: 5000,
        };
        parents.insert(root_id.clone(), root_node);

        let tree = walk_parents(&parents, &root_id, &mut HashSet::new());
        assert_eq!(
            tree.get("parent_site").and_then(|value| value.as_str()),
            Some("WestRedd")
        );

        let site = tree
            .get("children")
            .and_then(|value| value.as_object())
            .and_then(|children| children.get("MDC El Paso"))
            .and_then(|value| value.as_object())
            .expect("site child should be present");
        assert_eq!(
            site.get("parent_site").and_then(|value| value.as_str()),
            Some("WestRedd-SiteRouter")
        );

        let ap = site
            .get("children")
            .and_then(|value| value.as_object())
            .and_then(|children| children.get("Core-WestRedd"))
            .and_then(|value| value.as_object())
            .expect("ap child should be present");
        assert_eq!(
            ap.get("parent_site").and_then(|value| value.as_str()),
            Some("MDC El Paso")
        );
    }

    #[test]
    fn disambiguates_duplicate_site_and_ap_names_without_losing_the_site_branch() {
        let root_mapping = GraphMapping::Root {
            name: "MRE".to_string(),
            id: "site-mre".to_string(),
            latitude: None,
            longitude: None,
        };
        let trailer_site_mapping = GraphMapping::Site {
            name: "TrailerCity".to_string(),
            id: "site-trailercity".to_string(),
            latitude: None,
            longitude: None,
        };
        let trailer_ap_mapping = GraphMapping::AccessPoint {
            name: "TrailerCity".to_string(),
            id: "device-trailercity".to_string(),
            site_name: "TrailerCity".to_string(),
            download_mbps: 1200,
            upload_mbps: 1200,
        };
        let child_ap_mapping = GraphMapping::AccessPoint {
            name: "JR_AP_TC_A".to_string(),
            id: "device-jr-ap-tc-a".to_string(),
            site_name: "TrailerCity".to_string(),
            download_mbps: 5000,
            upload_mbps: 5000,
        };

        let export_names = assign_export_names([
            (root_mapping.network_json_id(), &root_mapping),
            (
                trailer_site_mapping.network_json_id(),
                &trailer_site_mapping,
            ),
            (trailer_ap_mapping.network_json_id(), &trailer_ap_mapping),
            (child_ap_mapping.network_json_id(), &child_ap_mapping),
        ]);
        let root_id = root_mapping.network_json_id();
        let site_id = trailer_site_mapping.network_json_id();
        let trailer_ap_id = trailer_ap_mapping.network_json_id();
        let child_ap_id = child_ap_mapping.network_json_id();

        assert_eq!(
            export_names.get(&site_id).map(String::as_str),
            Some("TrailerCity")
        );
        assert_eq!(
            export_names.get(&trailer_ap_id).map(String::as_str),
            Some("TrailerCity [AP]")
        );

        let mut parents = HashMap::new();
        parents.insert(
            root_id.clone(),
            NetJsonParent {
                node_id: root_id.clone(),
                node_name: "MRE".to_string(),
                export_name: export_names.get(&root_id).unwrap().clone(),
                parent_id: None,
                parent_name: "Root".to_string(),
                mapping: &root_mapping,
                download: 5000,
                upload: 5000,
            },
        );
        parents.insert(
            site_id.clone(),
            NetJsonParent {
                node_id: site_id.clone(),
                node_name: "TrailerCity".to_string(),
                export_name: export_names.get(&site_id).unwrap().clone(),
                parent_id: Some(root_id.clone()),
                parent_name: "MRE".to_string(),
                mapping: &trailer_site_mapping,
                download: 1200,
                upload: 1200,
            },
        );
        parents.insert(
            trailer_ap_id.clone(),
            NetJsonParent {
                node_id: trailer_ap_id.clone(),
                node_name: "TrailerCity".to_string(),
                export_name: export_names.get(&trailer_ap_id).unwrap().clone(),
                parent_id: Some(site_id.clone()),
                parent_name: "TrailerCity".to_string(),
                mapping: &trailer_ap_mapping,
                download: 1200,
                upload: 1200,
            },
        );
        parents.insert(
            child_ap_id.clone(),
            NetJsonParent {
                node_id: child_ap_id.clone(),
                node_name: "JR_AP_TC_A".to_string(),
                export_name: export_names.get(&child_ap_id).unwrap().clone(),
                parent_id: Some(site_id.clone()),
                parent_name: "TrailerCity".to_string(),
                mapping: &child_ap_mapping,
                download: 5000,
                upload: 5000,
            },
        );

        let tree = walk_parents(&parents, &root_id, &mut HashSet::new());
        let site = tree
            .get("children")
            .and_then(|value| value.as_object())
            .and_then(|children| children.get("TrailerCity"))
            .and_then(|value| value.as_object())
            .expect("site branch should survive duplicate-name export");
        let children = site
            .get("children")
            .and_then(|value| value.as_object())
            .expect("site children should exist");
        assert!(children.contains_key("TrailerCity [AP]"));
        assert!(children.contains_key("JR_AP_TC_A"));
    }
}

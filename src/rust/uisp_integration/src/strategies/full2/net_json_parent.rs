use crate::strategies::full2::graph_mapping::GraphMapping;
use std::collections::{HashMap, HashSet};

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
    visited: &mut HashSet<String>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    // Entries are name, type, uisp_device or site, downloadBandwidthMbps, uploadBandwidthMbps, children
    map.insert("name".into(), name.clone().into());
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
    for (name, node_info) in parents
        .iter()
        .filter(|(_node_name, node_info)| node_info.parent_name == *name)
    {
        if visited.contains(name) {
            continue;
        }
        visited.insert(name.to_string());
        let child = walk_parents(parents, name, node_info, visited);
        children.insert(name.into(), child.into());
    }

    map.insert("children".into(), children.into());

    map
}

#[cfg(test)]
mod tests {
    use super::{NetJsonParent, walk_parents};
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

        let mut parents = HashMap::new();
        parents.insert(
            "MDC El Paso".to_string(),
            NetJsonParent {
                parent_name: "WestRedd-SiteRouter".to_string(),
                mapping: &child_site_mapping,
                download: 5000,
                upload: 5000,
            },
        );
        parents.insert(
            "Core-WestRedd".to_string(),
            NetJsonParent {
                parent_name: "MDC El Paso".to_string(),
                mapping: &child_ap_mapping,
                download: 5000,
                upload: 5000,
            },
        );

        let root_name = "WestRedd-SiteRouter".to_string();
        let root_node = NetJsonParent {
            parent_name: "WestRedd".to_string(),
            mapping: &root_mapping,
            download: 5000,
            upload: 5000,
        };

        let tree = walk_parents(&parents, &root_name, &root_node, &mut HashSet::new());
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
}

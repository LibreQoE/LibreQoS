use crate::errors::UispIntegrationError;
use crate::uisp_types::{UispSite, UispSiteType};
use lqos_config::Config;
use std::fs::write;
use std::path::Path;
use tracing::{error, info};

/// Writes the network.json file for UISP
///
/// # Arguments
/// * `config` - The configuration
/// * `sites` - The list of sites
/// * `root_idx` - The index of the root site
///
/// # Returns
/// * An `Ok` if the operation was successful
/// * An `Err` if the operation failed
pub fn write_network_file(
    config: &Config,
    sites: &[UispSite],
    root_idx: usize,
) -> Result<(), UispIntegrationError> {
    let network_path = Path::new(&config.lqos_directory).join("network.json");

    // Write the network JSON file
    let root = traverse_sites(sites, root_idx, 0)?;
    if let Some(children) = root.get("children") {
        let json = serde_json::to_string_pretty(&children).unwrap();
        write(network_path, json).map_err(|e| {
            error!("Unable to write network.json");
            error!("{e:?}");
            UispIntegrationError::WriteNetJson
        })?;
        info!("Written network.json");
    }

    Ok(())
}

fn traverse_sites(
    sites: &[UispSite],
    idx: usize,
    depth: u32,
) -> Result<serde_json::Map<String, serde_json::Value>, UispIntegrationError> {
    let mut entry = serde_json::Map::new();
    entry.insert(
        "downloadBandwidthMbps".to_string(),
        serde_json::Value::Number(sites[idx].max_down_mbps.into()),
    );
    entry.insert(
        "uploadBandwidthMbps".to_string(),
        serde_json::Value::Number(sites[idx].max_up_mbps.into()),
    );
    entry.insert(
        "type".to_string(),
        serde_json::Value::String(sites[idx].site_type.as_network_json_string()),
    );
    entry.insert(
        "id".to_string(),
        serde_json::Value::String(generic_node_id(&sites[idx])),
    );
    if let (Some(latitude), Some(longitude)) = (sites[idx].latitude, sites[idx].longitude) {
        if let Some(number) = serde_json::Number::from_f64(latitude as f64) {
            entry.insert("latitude".to_string(), serde_json::Value::Number(number));
        }
        if let Some(number) = serde_json::Number::from_f64(longitude as f64) {
            entry.insert("longitude".to_string(), serde_json::Value::Number(number));
        }
    }

    if depth < 10 {
        let mut children = serde_json::Map::new();
        for (child_id, child) in sites.iter().enumerate() {
            if let Some(parent) = child.selected_parent
                && parent == idx
                && should_traverse(&sites[child_id].site_type)
            {
                children.insert(
                    child.name.clone(),
                    serde_json::Value::Object(traverse_sites(sites, child_id, depth + 1)?),
                );
            }
        }
        if !children.is_empty() {
            entry.insert("children".to_string(), serde_json::Value::Object(children));
        }
    }

    Ok(entry)
}

fn should_traverse(t: &UispSiteType) -> bool {
    !matches!(t, UispSiteType::Client)
}

fn generic_node_id(site: &UispSite) -> String {
    if site.site_type == UispSiteType::ClientWithChildren {
        format!(
            "libreqos:generated:uisp:site:{}",
            slugify_generated_name(&site.name)
        )
    } else {
        format!("uisp:site:{}", site.id)
    }
}

fn slugify_generated_name(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_was_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

#[cfg(test)]
mod test {
    use super::{generic_node_id, slugify_generated_name};
    use crate::uisp_types::{UispSite, UispSiteType};

    #[test]
    fn slugifies_generated_site_names() {
        assert_eq!(slugify_generated_name("Orphans"), "orphans");
        assert_eq!(
            slugify_generated_name("Generated Site / Tower 1"),
            "generated-site-tower-1"
        );
    }

    #[test]
    fn generated_site_ids_use_slugged_names() {
        let site = UispSite {
            id: "ignored-for-generated".to_string(),
            name: "Generated Site / Tower 1".to_string(),
            site_type: UispSiteType::ClientWithChildren,
            ..Default::default()
        };

        assert_eq!(
            generic_node_id(&site),
            "libreqos:generated:uisp:site:generated-site-tower-1"
        );
    }
}

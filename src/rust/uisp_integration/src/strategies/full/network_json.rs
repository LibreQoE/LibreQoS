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
    if network_path.exists() && !config.integration_common.always_overwrite_network_json {
        tracing::warn!("Network.json exists, and always overwrite network json is not true - not writing network.json");
        return Ok(());
    }

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

    if depth < 10 {
        let mut children = serde_json::Map::new();
        for (child_id, child) in sites.iter().enumerate() {
            if let Some(parent) = child.selected_parent {
                if parent == idx && should_traverse(&sites[child_id].site_type) {
                    children.insert(
                        child.name.clone(),
                        serde_json::Value::Object(traverse_sites(sites, child_id, depth + 1)?),
                    );
                }
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

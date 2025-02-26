mod data_link;
mod device; // UISP data definition for a device, including interfaces
/// UISP Data Structures
///
/// Strong-typed implementation of the UISP API system. Used by long-term
/// stats to attach device information, possibly in the future used to
/// accelerate the UISP integration.
mod rest; // REST HTTP services
mod site;

use std::sync::Arc;
// UISP data definition for a site, pulled from the JSON
use lqos_config::Config;
// UISP data link definitions
use self::rest::nms_request_get_vec;
use anyhow::Result;
pub use data_link::*;
pub use device::Device;
pub use site::{Description, Site, SiteId};

/// Loads a complete list of all sites from UISP
pub async fn load_all_sites(config: Arc<Config>) -> Result<Vec<Site>> {
    let mut raw_sites = nms_request_get_vec(
        "sites",
        &config.uisp_integration.token,
        &config.uisp_integration.url,
    )
    .await?;

    // Do not load sites from the excluded sites list.
    raw_sites.retain(|site: &Site| {
        if let Some(name) = &site.name() {
            !config.uisp_integration.exclude_sites.contains(name)
        } else {
            true
        }
    });

    Ok(raw_sites)
}

/// Load all devices from UISP that are authorized, and include their full interface definitions
pub async fn load_all_devices_with_interfaces(config: Arc<Config>) -> Result<Vec<Device>> {
    Ok(nms_request_get_vec(
        "devices?withInterfaces=true&authorized=true",
        &config.uisp_integration.token,
        &config.uisp_integration.url,
    )
    .await?)
}

/// Loads all data links from UISP (including links in client sites)
pub async fn load_all_data_links(config: Arc<Config>) -> Result<Vec<DataLink>> {
    Ok(nms_request_get_vec(
        "data-links",
        &config.uisp_integration.token,
        &config.uisp_integration.url,
    )
    .await?)
}

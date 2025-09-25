use crate::errors::UispIntegrationError;
use lqos_config::Config;
use std::sync::Arc;
use tokio::join;
use tracing::{error, info};
use uisp::{DataLink, Device, Site};

/// Load required data from UISP, using the API.
/// Requires a valid configuration with working token data.
pub async fn load_uisp_data(
    config: Arc<Config>,
) -> Result<(Vec<Site>, Vec<Device>, Vec<DataLink>), UispIntegrationError> {
    info!("Loading Devices, Sites and Data-Links from UISP");
    let (devices, sites, data_links) = join!(
        uisp::load_all_devices_with_interfaces(config.clone()),
        uisp::load_all_sites(config.clone()),
        uisp::load_all_data_links(config.clone()),
    );

    // Error Handling
    if devices.is_err() {
        error!("Error downloading devices list from UISP");
        error!("{:?}", devices);
        return Err(UispIntegrationError::UispConnectError);
    }
    let mut devices = devices.unwrap();

    if sites.is_err() {
        error!("Error downloading sites list from UISP");
        error!("{:?}", sites);
        return Err(UispIntegrationError::UispConnectError);
    }
    let sites = sites.unwrap();

    if data_links.is_err() {
        error!("Error downloading data_links list from UISP");
        error!("{:?}", data_links);
        return Err(UispIntegrationError::UispConnectError);
    }
    let data_links = data_links.unwrap();

    // Build a quick lookup of excluded site IDs to avoid repeated scans
    let excluded_site_ids: std::collections::HashSet<&str> = sites
        .iter()
        .filter(|site| {
            config
                .uisp_integration
                .exclude_sites
                .contains(&site.name_or_blank())
        })
        .map(|site| site.id.as_str())
        .collect();

    // Remove any devices that are in excluded sites
    devices.retain(|dev| {
        if let Some(site_id) = dev.get_site_id() {
            !excluded_site_ids.contains(site_id.as_str())
        } else {
            true
        }
    });

    info!(
        "Loaded backing data: {} sites, {} devices, {} links",
        sites.len(),
        devices.len(),
        data_links.len()
    );
    Ok((sites, devices, data_links))
}

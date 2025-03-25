use std::sync::Arc;
use lqos_config::Config;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::common::UispData;
use crate::uisp_types::UispSiteType;

/// Creates a network with APs detected from clients,
/// and then a single site above them (shared if the site
/// matches).
pub async fn build_ap_site_network(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    let uisp_data = UispData::fetch_uisp_data(config.clone(), ip_ranges).await?;

    // Find the clients
    let ap_mappings = uisp_data.map_clients_to_aps();

    // Locate any APs that are located within client sites
    for (ap_name, _) in ap_mappings.iter() {
        if let Some(device) = uisp_data.find_device_by_name(ap_name) {
            if let Some(device_site_id) = device.get_site_id() {
                if let Some(device_site) = uisp_data.sites.iter().find(|s| s.id == device_site_id) {
                    if device_site.site_type == UispSiteType::Client {
                        println!("Found AP {} in client site {}", ap_name, device_site.name);
                    }
                }
            }
        }
    }

    Ok(())
}
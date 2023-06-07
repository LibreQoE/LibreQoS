/// UISP Data Structures
/// 
/// Strong-typed implementation of the UISP API system. Used by long-term
/// stats to attach device information, possibly in the future used to
/// accelerate the UISP integration.

mod rest; // REST HTTP services
mod site; // UISP data definition for a site, pulled from the JSON
mod device; // UISP data definition for a device, including interfaces
mod data_link; // UISP data link definitions
use lqos_config::LibreQoSConfig;
pub use site::Site;
pub use device::Device;
pub use data_link::DataLink;
use self::rest::nms_request_get_vec;
use anyhow::Result;

/// Loads a complete list of all sites from UISP
pub async fn load_all_sites(config: LibreQoSConfig) -> Result<Vec<Site>> {
    Ok(nms_request_get_vec("sites", &config.uisp_auth_token, &config.uisp_base_url).await?)
}

/// Load all devices from UISP that are authorized, and include their full interface definitions
pub async fn load_all_devices_with_interfaces(config: LibreQoSConfig) -> Result<Vec<Device>> {
    Ok(nms_request_get_vec(
        "devices?withInterfaces=true&authorized=true",
        &config.uisp_auth_token,
        &config.uisp_base_url,
    )
    .await?)
}

/// Loads all data links from UISP (including links in client sites)
pub async fn load_all_data_links(config: LibreQoSConfig) -> Result<Vec<DataLink>> {
    Ok(nms_request_get_vec("data-links", &config.uisp_auth_token, &config.uisp_base_url).await?)
}
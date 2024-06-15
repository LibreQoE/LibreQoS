use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::uisp_types::UispDevice;
use lqos_config::Config;
use serde::Serialize;
use std::fs;
use std::path::Path;
use tracing::{error, info};

/// Represents a shaped device in the ShapedDevices.csv file.
#[derive(Serialize, Debug)]
struct ShapedDevice {
    pub circuit_id: String,
    pub circuit_name: String,
    pub device_id: String,
    pub device_name: String,
    pub parent_node: String,
    pub mac: String,
    pub ipv4: String,
    pub ipv6: String,
    pub download_min: u64,
    pub upload_min: u64,
    pub download_max: u64,
    pub upload_max: u64,
    pub comment: String,
}

/// Builds a flat network for UISP
/// 
/// # Arguments
/// * `config` - The configuration
/// * `ip_ranges` - The IP ranges to use for the network
pub async fn build_flat_network(
    config: Config,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    // Load the devices from UISP
    let devices = uisp::load_all_devices_with_interfaces(config.clone())
        .await
        .map_err(|e| {
            error!("Unable to load device list from UISP");
            error!("{e:?}");
            UispIntegrationError::UispConnectError
        })?;
    let sites = uisp::load_all_sites(config.clone()).await.map_err(|e| {
        error!("Unable to load device list from UISP");
        error!("{e:?}");
        UispIntegrationError::UispConnectError
    })?;

    // Create a {} network.json
    let net_json_path = Path::new(&config.lqos_directory).join("network.json");
    fs::write(net_json_path, "{}\n").map_err(|e| {
        error!("Unable to access network.json");
        error!("{e:?}");
        UispIntegrationError::WriteNetJson
    })?;

    // Simple Shaped Devices File
    let mut shaped_devices = Vec::new();
    let ipv4_to_v6 = Vec::new();
    for site in sites.iter() {
        if let Some(site_id) = &site.identification {
            if let Some(site_type) = &site_id.site_type {
                if site_type == "endpoint" {
                    let (download_max, upload_max) = site.qos(
                        config.queues.generated_pn_download_mbps,
                        config.queues.generated_pn_upload_mbps,
                    );
                    let download_min = (download_max as f32
                        * config.uisp_integration.commit_bandwidth_multiplier)
                        as u64;
                    let upload_min = (upload_max as f32
                        * config.uisp_integration.commit_bandwidth_multiplier)
                        as u64;
                    for device in devices.iter() {
                        let dev = UispDevice::from_uisp(device, &config, &ip_ranges, &ipv4_to_v6);
                        if dev.site_id == site.id {
                            // We're an endpoint in the right sight. We're getting there
                            let sd = ShapedDevice {
                                circuit_id: site.id.clone(),
                                circuit_name: site.name_or_blank(),
                                device_id: device.get_id(),
                                device_name: device.get_name().unwrap_or("".to_string()),
                                parent_node: "".to_string(),
                                mac: device.identification.mac.clone().unwrap_or("".to_string()),
                                ipv4: dev.ipv4_list(),
                                ipv6: dev.ipv6_list(),
                                download_min: u64::max(2, download_min),
                                download_max: u64::max(3, download_max as u64),
                                upload_min: u64::max(2, upload_min),
                                upload_max: u64::max(3, upload_max as u64),
                                comment: "".to_string(),
                            };
                            shaped_devices.push(sd);
                        }
                    }
                }
            }
        }
    }

    // Write it to disk
    let file_path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_path(file_path)
        .unwrap();

    for d in shaped_devices.iter() {
        writer.serialize(d).unwrap();
    }
    writer.flush().map_err(|e| {
        error!("Unable to flush CSV file");
        error!("{e:?}");
        UispIntegrationError::CsvError
    })?;
    info!("Wrote {} lines to ShapedDevices.csv", shaped_devices.len());

    Ok(())
}

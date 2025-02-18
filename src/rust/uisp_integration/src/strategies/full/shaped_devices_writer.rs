use crate::errors::UispIntegrationError;
use crate::uisp_types::{UispDevice, UispSite, UispSiteType};
use lqos_config::Config;
use serde::Serialize;
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

/// Writes the ShapedDevices.csv file for UISP
/// 
/// # Arguments
/// * `config` - The configuration
/// * `sites` - The list of sites
/// * `root_idx` - The index of the root site
/// * `devices` - The list of devices
pub fn write_shaped_devices(
    config: &Config,
    sites: &[UispSite],
    root_idx: usize,
    devices: &[UispDevice],
) -> Result<(), UispIntegrationError> {
    let file_path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let mut shaped_devices = Vec::new();

    // Traverse
    traverse(
        sites,
        root_idx,
        0,
        devices,
        &mut shaped_devices,
        config,
        root_idx,
    );

    // Write the CSV
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

fn traverse(
    sites: &[UispSite],
    idx: usize,
    depth: u32,
    devices: &[UispDevice],
    shaped_devices: &mut Vec<ShapedDevice>,
    config: &Config,
    root_idx: usize,
) {
    if !sites[idx].device_indices.is_empty() {
        // We have devices!
        if sites[idx].site_type == UispSiteType::Client {
            // Add as normal clients
            for device in sites[idx].device_indices.iter() {
                let device = &devices[*device];
                if device.has_address() {
                    let download_max = (sites[idx].max_down_mbps as f32
                        * config.uisp_integration.bandwidth_overhead_factor)
                        as u64;
                    let upload_max = (sites[idx].max_up_mbps as f32
                        * config.uisp_integration.bandwidth_overhead_factor)
                        as u64;
                    let download_min = 1
                        as u64;
                    let upload_min = 1
                        as u64;
                    let sd = ShapedDevice {
                        circuit_id: sites[idx].id.clone(),
                        circuit_name: sites[idx].name.clone(),
                        device_id: device.id.clone(),
                        device_name: device.name.clone(),
                        parent_node: sites[sites[idx].selected_parent.unwrap()].name.clone(),
                        mac: device.mac.clone(),
                        ipv4: device.ipv4_list(),
                        ipv6: device.ipv6_list(),
                        download_min: u64::max(1, download_min),
                        download_max: u64::max(2, download_max),
                        upload_min: u64::max(1, upload_min),
                        upload_max: u64::max(2, upload_max),
                        comment: "".to_string(),
                    };
                    shaped_devices.push(sd);
                }
            }
        } else {
            // It's an infrastructure node
            for device in sites[idx].device_indices.iter() {
                let device = &devices[*device];
                let parent_node = if idx != root_idx {
                    sites[idx].name.clone()
                } else {
                    format!("{}_Infrastructure", sites[idx].name.clone())
                };
                if device.has_address() {
                    let download_max = (sites[idx].max_down_mbps as f32
                        * config.uisp_integration.bandwidth_overhead_factor)
                        as u64;
                    let upload_max = (sites[idx].max_up_mbps as f32
                        * config.uisp_integration.bandwidth_overhead_factor)
                        as u64;
                    let download_min = (download_max as f32
                        * config.uisp_integration.commit_bandwidth_multiplier)
                        as u64;
                    let upload_min = (upload_max as f32
                        * config.uisp_integration.commit_bandwidth_multiplier)
                        as u64;
                    let sd = ShapedDevice {
                        circuit_id: format!("{}-inf", sites[idx].id),
                        circuit_name: format!("{} Infrastructure", sites[idx].name),
                        device_id: device.id.clone(),
                        device_name: device.name.clone(),
                        parent_node,
                        mac: device.mac.clone(),
                        ipv4: device.ipv4_list(),
                        ipv6: device.ipv6_list(),
                        download_min: u64::max(1, download_min),
                        download_max: u64::max(2, download_max),
                        upload_min: u64::max(1, upload_min),
                        upload_max: u64::max(2, upload_max),
                        comment: "Infrastructure Entry".to_string(),
                    };
                    shaped_devices.push(sd);
                }
            }
        }
    }

    if depth < 10 {
        for (child_idx, child) in sites.iter().enumerate() {
            if let Some(parent_idx) = child.selected_parent {
                if parent_idx == idx {
                    traverse(
                        sites,
                        child_idx,
                        depth + 1,
                        devices,
                        shaped_devices,
                        config,
                        root_idx,
                    );
                }
            }
        }
    }
}

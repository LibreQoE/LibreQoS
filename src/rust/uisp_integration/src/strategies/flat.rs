use crate::blackboard_blob;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::common::dedup_site_names;
use crate::uisp_types::UispDevice;
use lqos_config::Config;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

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
    pub download_min: f32,
    pub upload_min: f32,
    pub download_max: f32,
    pub upload_max: f32,
    pub comment: String,
}

/// Builds a flat network for UISP
///
/// # Arguments
/// * `config` - The configuration
/// * `ip_ranges` - The IP ranges to use for the network
pub async fn build_flat_network(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    // Load the devices from UISP
    let (devices, json_devices) = uisp::load_all_devices_with_interfaces(config.clone())
        .await
        .map_err(|e| {
            error!("Unable to load device list from UISP");
            error!("{e:?}");
            UispIntegrationError::UispConnectError
        })?;
    let mut sites = uisp::load_all_sites(config.clone()).await.map_err(|e| {
        error!("Unable to load device list from UISP");
        error!("{e:?}");
        UispIntegrationError::UispConnectError
    })?;
    let data_links = uisp::load_all_data_links(config.clone())
        .await
        .map_err(|e| {
            error!("Unable to load device list from UISP");
            error!("{e:?}");
            UispIntegrationError::UispConnectError
        })?;

    // Normalize duplicate site names before building any structures
    dedup_site_names(&mut sites);

    if let Err(e) = blackboard_blob("uisp_sites", &sites).await {
        warn!("Unable to write sites to blackboard: {e:?}");
    }
    if let Err(e) = blackboard_blob("uisp_devices", &json_devices).await {
        warn!("Unable to write devices to blackboard: {e:?}");
    }
    if let Err(e) = blackboard_blob("uisp_data_links", &data_links).await {
        warn!("Unable to write data links to blackboard: {e:?}");
    }

    // Create a {} network.json
    let net_json_path = Path::new(&config.lqos_directory).join("network.json");
    fs::write(net_json_path, "{}\n").map_err(|e| {
        error!("Unable to access network.json");
        error!("{e:?}");
        UispIntegrationError::WriteNetJson
    })?;

    // Simple Shaped Devices File
    let mut shaped_devices = Vec::new();
    let mut seen_pairs = HashSet::new();
    let ipv4_to_v6 = Vec::new();
    for site in sites.iter() {
        if let Some(site_id) = &site.identification {
            if let Some(site_type) = &site_id.site_type {
                if site_type == "endpoint" {
                    // Prefer UISP QoS + burst for client sites
                    let mut download_min: f32 = 0.1;
                    let mut upload_min: f32 = 0.1;
                    let mut download_max: f32 = 0.1;
                    let mut upload_max: f32 = 0.1;
                    let suspended_slow = site
                        .identification
                        .as_ref()
                        .map(|id| id.suspended)
                        .unwrap_or(false)
                        && config.uisp_integration.suspended_strategy == "slow";
                    if suspended_slow {
                        download_min = 0.1;
                        upload_min = 0.1;
                        download_max = 0.1;
                        upload_max = 0.1;
                    } else if let Some(qos) = &site.qos {
                        let base_down = qos
                            .downloadSpeed
                            .map(|v| (v as f32) / 1_000_000.0)
                            .unwrap_or(0.0);
                        let base_up = qos
                            .uploadSpeed
                            .map(|v| (v as f32) / 1_000_000.0)
                            .unwrap_or(0.0);
                        let burst_down = qos
                            .downloadBurstSize
                            .map(|v| (v as f32) * 8.0 / 1000.0 / 1024.0)
                            .unwrap_or(0.0);
                        let burst_up = qos
                            .uploadBurstSize
                            .map(|v| (v as f32) * 8.0 / 1000.0 / 1024.0)
                            .unwrap_or(0.0);
                        if base_down > 0.0 || base_up > 0.0 {
                            download_min = f32::max(
                                0.1,
                                base_down * config.uisp_integration.commit_bandwidth_multiplier,
                            );
                            upload_min = f32::max(
                                0.1,
                                base_up * config.uisp_integration.commit_bandwidth_multiplier,
                            );
                            download_max = f32::max(
                                0.1,
                                (base_down + burst_down)
                                    * config.uisp_integration.bandwidth_overhead_factor,
                            );
                            upload_max = f32::max(
                                0.1,
                                (base_up + burst_up)
                                    * config.uisp_integration.bandwidth_overhead_factor,
                            );
                        } else {
                            // Fallback to legacy capacity-based min/max
                            let (dl_cap, ul_cap) = site.qos(
                                config.queues.generated_pn_download_mbps,
                                config.queues.generated_pn_upload_mbps,
                            );
                            let dl_max_f =
                                (dl_cap as f32) * config.uisp_integration.bandwidth_overhead_factor;
                            let ul_max_f =
                                (ul_cap as f32) * config.uisp_integration.bandwidth_overhead_factor;
                            let dl_min_f =
                                dl_max_f * config.uisp_integration.commit_bandwidth_multiplier;
                            let ul_min_f =
                                ul_max_f * config.uisp_integration.commit_bandwidth_multiplier;
                            download_min = f32::max(0.1, dl_min_f);
                            upload_min = f32::max(0.1, ul_min_f);
                            download_max = f32::max(0.1, dl_max_f);
                            upload_max = f32::max(0.1, ul_max_f);
                        }
                    } else {
                        // Fallback if qos entirely missing
                        let (dl_cap, ul_cap) = site.qos(
                            config.queues.generated_pn_download_mbps,
                            config.queues.generated_pn_upload_mbps,
                        );
                        let dl_max_f =
                            (dl_cap as f32) * config.uisp_integration.bandwidth_overhead_factor;
                        let ul_max_f =
                            (ul_cap as f32) * config.uisp_integration.bandwidth_overhead_factor;
                        let dl_min_f =
                            dl_max_f * config.uisp_integration.commit_bandwidth_multiplier;
                        let ul_min_f =
                            ul_max_f * config.uisp_integration.commit_bandwidth_multiplier;
                        download_min = f32::max(0.1, dl_min_f);
                        upload_min = f32::max(0.1, ul_min_f);
                        download_max = f32::max(0.1, dl_max_f);
                        upload_max = f32::max(0.1, ul_max_f);
                    }
                    // Ensure max >= min
                    if download_max < download_min {
                        download_max = download_min;
                    }
                    if upload_max < upload_min {
                        upload_max = upload_min;
                    }
                    for device in devices.iter() {
                        let dev = UispDevice::from_uisp(device, &config, &ip_ranges, &ipv4_to_v6);
                        if dev.site_id == site.id {
                            // We're an endpoint in the right sight. We're getting there
                            let key = (site.id.clone(), device.get_id());
                            if !seen_pairs.insert(key) {
                                continue;
                            }

                            let sd = ShapedDevice {
                                circuit_id: site.id.clone(),
                                circuit_name: site.name_or_blank(),
                                device_id: device.get_id(),
                                device_name: device.get_name().unwrap_or("".to_string()),
                                parent_node: "".to_string(),
                                mac: device.identification.mac.clone().unwrap_or("".to_string()),
                                ipv4: dev.ipv4_list(),
                                ipv6: dev.ipv6_list(),
                                download_min,
                                download_max,
                                upload_min,
                                upload_max,
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

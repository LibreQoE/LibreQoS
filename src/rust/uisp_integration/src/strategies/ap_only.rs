use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::common::UispData;
use crate::strategies::full::shaped_devices_writer::ShapedDevice;
use lqos_config::Config;
use std::collections::HashSet;
use std::fs::write;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Creates a network with only APs detected
/// from clients.
pub async fn build_ap_only_network(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    let uisp_data = UispData::fetch_uisp_data(config.clone(), ip_ranges).await?;

    // Find the clients
    let mappings = uisp_data.map_clients_to_aps();

    // Write network.json
    let network_path = Path::new(&config.lqos_directory).join("network.json");
    if network_path.exists() && !config.integration_common.always_overwrite_network_json {
        warn!(
            "Network.json exists, and always overwrite network json is not true - not writing network.json"
        );
    } else {
        let mut root = serde_json::Map::new();
        for ap in mappings.keys() {
            if let Some(ap_device) = uisp_data.devices.iter().find(|d| d.name == *ap) {
                let mut ap_object = serde_json::Map::new();
                // Empy children
                ap_object.insert("children".to_string(), serde_json::Map::new().into());

                // Limits
                ap_object.insert(
                    "downloadBandwidthMbps".to_string(),
                    serde_json::Value::Number(ap_device.download.into()),
                );
                ap_object.insert(
                    "uploadBandwidthMbps".to_string(),
                    serde_json::Value::Number(ap_device.upload.into()),
                );

                // Metadata
                ap_object.insert("type".to_string(), "AP".to_string().into());
                ap_object.insert("uisp_device".to_string(), ap_device.id.clone().into());

                // Save the entry
                root.insert(ap.to_string(), ap_object.into());
            }
        }
        let json = serde_json::to_string_pretty(&root).unwrap();
        write(network_path, json).map_err(|e| {
            error!("Unable to write network.json");
            error!("{e:?}");
            UispIntegrationError::WriteNetJson
        })?;
        info!("Written network.json");
    }

    // Write ShapedDevices.csv
    let file_path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let mut shaped_devices = Vec::new();
    let mut seen_pairs = HashSet::new();
    for (parent, client_ids) in mappings.iter() {
        for client_id in client_ids {
            let site = uisp_data.sites.iter().find(|s| *client_id == s.id).unwrap();
            let devices = uisp_data
                .devices
                .iter()
                .filter(|d| d.site_id == *client_id)
                .collect::<Vec<_>>();
            for device in devices.iter().filter(|d| d.has_address()) {
                // Compute subscriber rates: prefer UISP QoS + burst
                let (mut download_min, mut download_max, mut upload_min, mut upload_max) =
                    if let Some((dl_min, dl_max, ul_min, ul_max)) = site.burst_rates(&config) {
                        (
                            f32::max(0.1, dl_min),
                            f32::max(0.1, dl_max),
                            f32::max(0.1, ul_min),
                            f32::max(0.1, ul_max),
                        )
                    } else if site.suspended && config.uisp_integration.suspended_strategy == "slow"
                    {
                        (0.1, 0.1, 0.1, 0.1)
                    } else {
                        (
                            f32::max(
                                0.1,
                                site.max_down_mbps as f32
                                    * config.uisp_integration.commit_bandwidth_multiplier,
                            ),
                            f32::max(
                                0.1,
                                site.max_down_mbps as f32
                                    * config.uisp_integration.bandwidth_overhead_factor,
                            ),
                            f32::max(
                                0.1,
                                site.max_up_mbps as f32
                                    * config.uisp_integration.commit_bandwidth_multiplier,
                            ),
                            f32::max(
                                0.1,
                                site.max_up_mbps as f32
                                    * config.uisp_integration.bandwidth_overhead_factor,
                            ),
                        )
                    };
                if download_max < download_min {
                    download_max = download_min;
                }
                if upload_max < upload_min {
                    upload_max = upload_min;
                }

                let key = (site.id.clone(), device.id.clone());
                if !seen_pairs.insert(key) {
                    continue;
                }

                let sd = ShapedDevice {
                    circuit_id: site.id.clone(),
                    circuit_name: site.name.clone(),
                    device_id: device.id.clone(),
                    device_name: device.name.clone(),
                    parent_node: parent.clone(),
                    mac: device.mac.clone(),
                    ipv4: device.ipv4_list(),
                    ipv6: device.ipv6_list(),
                    download_min,
                    upload_min,
                    download_max,
                    upload_max,
                    comment: "".to_string(),
                };
                shaped_devices.push(sd);
            }
        }
    }
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

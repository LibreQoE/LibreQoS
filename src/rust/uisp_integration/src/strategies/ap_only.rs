use std::collections::HashMap;
use std::fs::write;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};
use lqos_config::Config;
use crate::blackboard_blob;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::full::shaped_devices_writer::ShapedDevice;
use crate::uisp_types::UispSiteType;

/// Creates a network with only APs detected
/// from clients.
pub async fn build_ap_only_network(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    // Obtain the UISP data and transform it into easier to work with types
    let (sites_raw, devices_raw, data_links_raw) = crate::strategies::full::uisp_fetch::load_uisp_data(config.clone()).await?;

    if let Err(e) = blackboard_blob("uisp_sites", &sites_raw).await {
        warn!("Unable to write sites to blackboard: {e:?}");
    }
    if let Err(e) = blackboard_blob("uisp_devices", &devices_raw).await {
        warn!("Unable to write devices to blackboard: {e:?}");
    }
    if let Err(e) = blackboard_blob("uisp_data_links", &data_links_raw).await {
        warn!("Unable to write data links to blackboard: {e:?}");
    }

    // If Mikrotik is enabled, we need to fetch the Mikrotik data
    let ipv4_to_v6 = crate::strategies::full::mikrotik::mikrotik_data(&config)
        .await
        .unwrap_or_else(|_| Vec::new());

    // Parse the UISP data into a more usable format
    let (sites, _data_links, devices) = crate::strategies::full::parse::parse_uisp_datasets(
        &sites_raw,
        &data_links_raw,
        &devices_raw,
        &config,
        &ip_ranges,
        ipv4_to_v6,
    );

    // Find the clients
    let mut mappings = HashMap::new();
    for client in sites.iter().filter(|s| s.site_type == UispSiteType::Client) {
        let mut found = false;
        let mut parent = None;
        for device in devices_raw.iter().filter(|d| d.get_site_id().unwrap_or_default() == client.id) {
            //println!("Client {} has a device {:?}", client.name, device.get_name());
            // Look for Parent AP attributes
            if let Some(attr) = &device.attributes {
                if let Some(ap) = &attr.apDevice {
                    if let Some(ap_id) = &ap.id {
                        //println!("AP ID: {}", ap_id);
                        if let Some(apdev) = devices_raw.iter().find(|d| d.identification.id == *ap_id) {
                            //println!("AP Device: {:?}", apdev.get_name());
                            parent = Some(("AP", apdev.get_name().unwrap_or_default()));
                            found = true;
                        }
                    }
                }
            }

            // Look for data links with this device
            if !found {
                for link in data_links_raw.iter() {
                    // Check the FROM side
                    if let Some(from_device) = &link.from.device {
                        if from_device.identification.id == device.identification.id {
                            if let Some(to_device) = &link.to.device {
                                if let Some(apdev) = devices_raw.iter().find(|d| d.identification.id == to_device.identification.id) {
                                    parent = Some(("AP", apdev.get_name().unwrap_or_default()));
                                    found = true;
                                }
                            }
                        }
                    }
                    // Check the TO side
                    if let Some(to_device) = &link.to.device {
                        if to_device.identification.id == device.identification.id {
                            if let Some(from_device) = &link.from.device {
                                if let Some(apdev) = devices_raw.iter().find(|d| d.identification.id == from_device.identification.id) {
                                    parent = Some(("AP", apdev.get_name().unwrap_or_default()));
                                    found = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        // If we still haven't found anything, let's try data links to the client site as a whole
        if !found {
            for link in data_links_raw.iter() {
                if let Some(from_site) = &link.from.site {
                    if from_site.identification.id == client.id {
                        if let Some(to_device) = &link.to.device {
                            if let Some(apdev) = devices_raw.iter().find(|d| d.identification.id == to_device.identification.id) {
                                parent = Some(("AP", apdev.get_name().unwrap_or_default()));
                                found = true;
                            }
                        }
                    }
                }
                if let Some(to_site) = &link.to.site {
                    if to_site.identification.id == client.id {
                        if let Some(from_device) = &link.from.device {
                            if let Some(apdev) = devices_raw.iter().find(|d| d.identification.id == from_device.identification.id) {
                                parent = Some(("AP", apdev.get_name().unwrap_or_default()));
                                found = true;
                            }
                        }
                    }
                }
            }
        }

        if !found {
            //println!("Client {} has no obvious parent AP", client.name);
            let entry = mappings.entry("Orphans".to_string()).or_insert_with(Vec::new);
            entry.push(client.id.clone());
        } else {
            //info!("Client {} is connected to {:?}", client.name, parent);
            if let Some((_, parent)) = &parent {
                let entry = mappings.entry(parent.to_string()).or_insert_with(Vec::new);
                entry.push(client.id.clone());
            }
        }
    }

    // We now have enough to build the network
    //println!("{:#?}", mappings);

    // Write network.json
    let network_path = Path::new(&config.lqos_directory).join("network.json");
    if network_path.exists() && !config.integration_common.always_overwrite_network_json {
        warn!(
            "Network.json exists, and always overwrite network json is not true - not writing network.json"
        );
        return Ok(());
    }
    let mut root = serde_json::Map::new();
    for ap in mappings.keys() {
        if let Some(ap_device) = devices.iter().find(|d| d.name == *ap) {
            let mut ap_object = serde_json::Map::new();
            // Empy children
            ap_object.insert("children".to_string(), serde_json::Map::new().into());

            // Limits
            ap_object.insert("downloadBandwidthMbps".to_string(), serde_json::Value::Number(ap_device.download.into()));
            ap_object.insert("uploadBandwidthMbps".to_string(), serde_json::Value::Number(ap_device.upload.into()));

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

    // Write ShapedDevices.csv
    let file_path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let mut shaped_devices = Vec::new();
    for (parent, client_ids) in mappings.iter() {
        for client_id in client_ids {
            let site = sites.iter().find(|s| *client_id == s.id).unwrap();
            let devices = devices.iter().filter(|d| d.site_id == *client_id).collect::<Vec<_>>();
            for device in devices.iter().filter(|d| d.has_address()) {
                let sd = ShapedDevice {
                    circuit_id: site.id.clone(),
                    circuit_name: site.name.clone(),
                    device_id: device.id.clone(),
                    device_name: device.name.clone(),
                    parent_node: parent.clone(),
                    mac: device.mac.clone(),
                    ipv4: device.ipv4_list(),
                    ipv6: device.ipv6_list(),
                    download_min: (site.max_down_mbps as f32
                        * config.uisp_integration.commit_bandwidth_multiplier)
                        as u64,
                    upload_min: (site.max_up_mbps as f32
                        * config.uisp_integration.commit_bandwidth_multiplier)
                        as u64,
                    download_max: (site.max_down_mbps as f32
                        * config.uisp_integration.bandwidth_overhead_factor)
                        as u64,
                    upload_max: (site.max_up_mbps as f32
                        * config.uisp_integration.bandwidth_overhead_factor)
                        as u64,
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
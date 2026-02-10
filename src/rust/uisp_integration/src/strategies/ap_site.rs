use crate::blackboard_blob;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::common::UispData;
use crate::strategies::full::shaped_devices_writer::ShapedDevice;
use lqos_config::Config;
use std::collections::{HashMap, HashSet};
use std::fs::write;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Clone)]
pub enum GraphMapping {
    Root,
    SiteByName(String),
    //GeneratedSiteByName(String),
    AccessPointByName(String),
    ClientById(String),
}

/// Creates a network with APs detected from clients,
/// and then a single site above them (shared if the site
/// matches).
pub async fn build_ap_site_network(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    let uisp_data = UispData::fetch_uisp_data(config.clone(), ip_ranges).await?;

    // Find trouble-spots!
    let _trouble = find_troublesome_sites(&uisp_data).await.map_err(|e| {
        error!("Error finding troublesome sites");
        error!("{e:?}");
        UispIntegrationError::UnknownSiteType
    })?;

    // Find the clients
    let ap_mappings = uisp_data.map_clients_to_aps();

    // Make AP Layer entries
    let access_points = get_ap_layer(&ap_mappings);

    // Site mappings
    let sites = map_sites_above_aps(&uisp_data, ap_mappings, access_points);

    // Insert the root
    let root = Layer {
        id: GraphMapping::Root,
        children: sites.values().cloned().collect(),
    };
    //println!("{:#?}", root);

    let mut shaped_devices = Vec::new();
    let net_json = root.walk_children(None, &uisp_data, &mut shaped_devices, &config);

    let network_path = Path::new(&config.lqos_directory).join("network.json");
    if network_path.exists() && !config.integration_common.always_overwrite_network_json {
        warn!(
            "Network.json exists, and always overwrite network json is not true - not writing network.json"
        );
    } else {
        let json = serde_json::to_string_pretty(&net_json).unwrap();
        write(network_path, json).map_err(|e| {
            error!("Unable to write network.json");
            error!("{e:?}");
            UispIntegrationError::WriteNetJson
        })?;
        info!("Written network.json");
    }

    let _ = write_shaped_devices(&config, &mut shaped_devices);
    info!("Wrote {} lines to ShapedDevices.csv", shaped_devices.len());

    Ok(())
}

pub(crate) fn write_shaped_devices(
    config: &Arc<Config>,
    shaped_devices: &mut Vec<ShapedDevice>,
) -> Result<(), UispIntegrationError> {
    let file_path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let mut seen_pairs = HashSet::new();
    shaped_devices.retain(|sd| seen_pairs.insert((sd.circuit_id.clone(), sd.device_id.clone())));
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

pub(crate) fn map_sites_above_aps(
    uisp_data: &UispData,
    ap_mappings: HashMap<String, Vec<String>>,
    access_points: HashMap<String, Layer>,
) -> HashMap<String, Layer> {
    let mut sites = HashMap::new();
    for (ap_name, client_ids) in ap_mappings.iter() {
        if let Some(device) = uisp_data.find_device_by_name(ap_name) {
            if let Some(device_site_id) = device.get_site_id() {
                if let Some(device_site) = uisp_data.sites.iter().find(|s| s.id == device_site_id) {
                    let site_entry =
                        sites
                            .entry(device_site.name.clone())
                            .or_insert_with(|| Layer {
                                id: GraphMapping::SiteByName(device_site.name.clone()),
                                children: Vec::new(),
                            });
                    let ap_map = access_points.get(ap_name).unwrap().clone();
                    site_entry.children.push(ap_map);
                }
            }
        } else {
            let mut detached = Layer {
                id: GraphMapping::SiteByName(ap_name.clone()),
                children: vec![],
            };
            for client_id in client_ids.iter() {
                detached.children.push(Layer {
                    id: GraphMapping::ClientById(client_id.clone()),
                    children: vec![],
                });
            }
            sites.insert(ap_name.clone(), detached);
        }
    }
    sites
}

pub(crate) fn get_ap_layer(ap_mappings: &HashMap<String, Vec<String>>) -> HashMap<String, Layer> {
    let mut access_points = HashMap::new();
    for (ap_name, client_ids) in ap_mappings.iter() {
        let mut ap_layer = Layer {
            id: GraphMapping::AccessPointByName(ap_name.clone()),
            children: Vec::new(),
        };
        for client_id in client_ids.iter() {
            ap_layer.children.push(Layer {
                id: GraphMapping::ClientById(client_id.clone()),
                children: Vec::new(),
            });
        }
        access_points.insert(ap_name.clone(), ap_layer);
    }
    access_points
}

#[derive(Debug, Clone)]
pub(crate) struct Layer {
    pub(crate) id: GraphMapping,
    pub(crate) children: Vec<Layer>,
}

impl Layer {
    pub(crate) fn walk_children(
        &self,
        parent: Option<&str>,
        uisp_data: &UispData,
        shaped_devices: &mut Vec<ShapedDevice>,
        config: &Config,
    ) -> serde_json::Map<String, serde_json::Value> {
        let mut children = serde_json::Map::new();
        let parent_name = match &self.id {
            GraphMapping::SiteByName(name) | GraphMapping::AccessPointByName(name) => {
                name.to_owned()
            }
            _ => "".to_owned(),
        };
        for child in self.children.iter() {
            match &child.id {
                GraphMapping::SiteByName(name) | GraphMapping::AccessPointByName(name) => {
                    children.insert(
                        name.clone(),
                        child
                            .walk_children(Some(&parent_name), uisp_data, shaped_devices, config)
                            .into(),
                    );
                }
                GraphMapping::ClientById(_client_id) => {
                    let _ =
                        child.walk_children(Some(&parent_name), uisp_data, shaped_devices, config);
                }
                _ => {}
            }
        }
        let mut root = serde_json::Map::new();
        if let Some(parent) = parent {
            match &self.id {
                GraphMapping::SiteByName(name) => {
                    root.insert("type".to_string(), "Site".into());
                    root.insert("name".to_string(), name.clone().into());
                    if let Some(site) = uisp_data.sites.iter().find(|s| s.name == *name) {
                        root.insert(
                            "downloadBandwidthMbps".to_owned(),
                            site.max_down_mbps.into(),
                        );
                        root.insert("uploadBandwidthMbps".to_owned(), site.max_up_mbps.into());
                        root.insert("uisp_site".to_string(), site.id.clone().into());
                        root.insert("parent_site".to_string(), name.to_string().into());
                    }
                }
                GraphMapping::AccessPointByName(name) => {
                    root.insert("type".to_string(), "AP".into());
                    root.insert("name".to_string(), name.clone().into());
                    root.insert("parent_site".to_string(), parent.to_string().into());
                    if let Some(device) = uisp_data.devices.iter().find(|d| d.name == *name) {
                        root.insert("downloadBandwidthMbps".to_owned(), device.download.into());
                        root.insert("uploadBandwidthMbps".to_owned(), device.upload.into());
                        root.insert("uisp_device".to_string(), device.id.clone().into());
                    }
                }
                GraphMapping::ClientById(client_id) => {
                    if let Some(site) = uisp_data.sites.iter().find(|c| c.id == *client_id) {
                        let devices = uisp_data
                            .devices
                            .iter()
                            .filter(|d| d.site_id == *client_id)
                            .collect::<Vec<_>>();
                        for device in devices.iter().filter(|d| d.has_address()) {
                            // Compute subscriber rates: prefer UISP QoS + burst
                            let (
                                mut download_min,
                                mut download_max,
                                mut upload_min,
                                mut upload_max,
                            ) = if let Some(site) =
                                uisp_data.sites.iter().find(|s| s.id == *client_id)
                            {
                                if let Some((dl_min, dl_max, ul_min, ul_max)) =
                                    site.burst_rates(&config)
                                {
                                    (
                                        f32::max(0.1, dl_min),
                                        f32::max(0.1, dl_max),
                                        f32::max(0.1, ul_min),
                                        f32::max(0.1, ul_max),
                                    )
                                } else if site.suspended
                                    && config.uisp_integration.suspended_strategy == "slow"
                                {
                                    (0.1, 0.1, 0.1, 0.1)
                                } else {
                                    (
                                        f32::max(
                                            0.1,
                                            site.max_down_mbps as f32
                                                * config
                                                    .uisp_integration
                                                    .commit_bandwidth_multiplier,
                                        ),
                                        f32::max(
                                            0.1,
                                            site.max_down_mbps as f32
                                                * config.uisp_integration.bandwidth_overhead_factor,
                                        ),
                                        f32::max(
                                            0.1,
                                            site.max_up_mbps as f32
                                                * config
                                                    .uisp_integration
                                                    .commit_bandwidth_multiplier,
                                        ),
                                        f32::max(
                                            0.1,
                                            site.max_up_mbps as f32
                                                * config.uisp_integration.bandwidth_overhead_factor,
                                        ),
                                    )
                                }
                            } else {
                                (0.1, 0.1, 0.1, 0.1)
                            };
                            if download_max < download_min {
                                download_max = download_min;
                            }
                            if upload_max < upload_min {
                                upload_max = upload_min;
                            }

                            let sd = ShapedDevice {
                                circuit_id: site.id.clone(),
                                circuit_name: site.name.clone(),
                                device_id: device.id.clone(),
                                device_name: device.name.clone(),
                                parent_node: parent.to_owned(),
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
                    } else {
                        warn!("Client not found: {}", client_id);
                    }
                }
                _ => {}
            }
        }
        if parent.is_some() {
            root.insert(
                "children".to_string(),
                serde_json::to_value(children).unwrap(),
            );
        } else {
            for child in children {
                root.insert(child.0, child.1);
            }
        }
        root
    }
}

pub struct TroublesomeClients {
    #[allow(dead_code)] // Used in serialization
    pub multi_entry_points: HashSet<String>,
    #[allow(dead_code)] // Used in serialization
    pub client_of_clients: HashSet<String>,
}

pub(crate) async fn find_troublesome_sites(data: &UispData) -> anyhow::Result<TroublesomeClients> {
    let multi_entry_points = find_clients_with_multiple_entry_points(data)?;
    let client_of_clients = find_clients_linked_from_other_clients(data)?;

    let _ = blackboard_blob("uisp-trouble-multi-entry", vec![multi_entry_points.clone()]).await;
    let _ = blackboard_blob(
        "uisp-trouble-client-of-client",
        vec![client_of_clients.clone()],
    )
    .await;

    Ok(TroublesomeClients {
        multi_entry_points,
        client_of_clients,
    })
}

fn find_clients_with_multiple_entry_points(data: &UispData) -> anyhow::Result<HashSet<String>> {
    let mut result = HashSet::new();
    for client in data.find_client_sites() {
        let mut links_to_client = HashSet::new();
        for link in data.data_links_raw.iter() {
            if let (Some(from_site), Some(to_site)) = (&link.from.site, &link.to.site) {
                if from_site.identification.id == client.id
                    && to_site.identification.id != client.id
                {
                    links_to_client.insert(to_site.identification.id.clone());
                } else if from_site.identification.id != client.id
                    && to_site.identification.id == client.id
                {
                    links_to_client.insert(from_site.identification.id.clone());
                }
            }
        }
        if links_to_client.len() > 1 {
            warn!(
                "Client {} has multiple entry points: {:?}",
                client.name, links_to_client
            );
            result.insert(client.id.clone());
        }
    }

    Ok(result)
}

fn find_clients_linked_from_other_clients(data: &UispData) -> anyhow::Result<HashSet<String>> {
    let all_clients = data.find_client_sites();
    let mut result = HashSet::new();
    for client in &all_clients {
        for link in data.data_links_raw.iter() {
            if let (Some(from_site), Some(to_site)) = (&link.from.site, &link.to.site) {
                if from_site.identification.id == client.id
                    && to_site.identification.id != client.id
                    && all_clients
                        .iter()
                        .any(|c| c.id == to_site.identification.id)
                {
                    warn!(
                        "Client {} is linked from another client: {}",
                        client.name, to_site.identification.id
                    );
                    result.insert(client.id.clone());
                }
                if from_site.identification.id != client.id
                    && to_site.identification.id == client.id
                    && all_clients
                        .iter()
                        .any(|c| c.id == from_site.identification.id)
                {
                    warn!(
                        "Client {} is linked to another client: {}",
                        client.name, from_site.identification.id
                    );
                    result.insert(client.id.clone());
                }
            }
        }
    }
    Ok(result)
}

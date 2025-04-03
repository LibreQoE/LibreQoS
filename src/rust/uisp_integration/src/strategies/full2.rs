use std::collections::{HashMap, HashSet};
use std::fs::write;
use std::path::Path;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::common::UispData;
use crate::uisp_types::UispDevice;
use lqos_config::Config;
use std::sync::Arc;
use petgraph::Undirected;
use tracing::{error, info, warn};
use petgraph::visit::{EdgeRef, NodeRef};
use serde::Serialize;
use crate::blackboard_blob;
use crate::strategies::full::routes_override::RouteOverride;
use crate::strategies::full::shaped_devices_writer::ShapedDevice;

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Clone, Serialize)]
pub enum GraphMapping {
    Root { name: String, id: String },
    Site { name: String, id: String },
    GeneratedSite { name: String },
    AccessPoint { name: String, id: String },
}

impl GraphMapping {
    pub fn name(&self) -> String {
        match self {
            GraphMapping::Root { name, .. } => name.clone(),
            GraphMapping::Site { name, .. } => name.clone(),
            GraphMapping::GeneratedSite { name } => name.clone(),
            GraphMapping::AccessPoint { name, .. } => name.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum LinkMapping {
    Ethernet,
    DevicePair(String, String),
}

type GraphType = petgraph::Graph<GraphMapping, LinkMapping, Undirected>;

pub async fn build_full_network_v2(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    // Fetch the data
    let uisp_data = UispData::fetch_uisp_data(config.clone(), ip_ranges).await?;

    // Report on obvious UISP errors that should be fixed
    let _trouble = crate::strategies::ap_site::find_troublesome_sites(&uisp_data).map_err(|e| {
        error!("Error finding troublesome sites");
        error!("{e:?}");
        UispIntegrationError::UnknownSiteType
    })?;

    // Load overrides
    let bandwidth_overrides = crate::strategies::full::bandwidth_overrides::get_site_bandwidth_overrides(&config)?;
    let routing_overrides = crate::strategies::full::routes_override::get_route_overrides(&config)?;

    // Create a new graph
    let mut graph = GraphType::new_undirected();

    // Find the root
    let root_site_name = config.uisp_integration.site.clone();

    // Add all sites to the graph
    let mut site_map = HashMap::new();
    let mut root_idx = None;
    for site in uisp_data.sites_raw.iter().filter(|s| !s.is_client_site()) {
        let site_name = site.name_or_blank();
        let id = site.id.clone();
        if site_name == root_site_name {
            // Add the root site
            let root_entry = GraphMapping::Root {
                name: site_name,
                id,
            };
            let root_ref = graph.add_node(root_entry);
            root_idx = Some(root_ref);
            site_map.insert(site.id.clone(), root_ref);
            continue;
        }
        let site_entry = GraphMapping::Site {
            name: site_name,
            id,
        };
        let site_ref = graph.add_node(site_entry);
        site_map.insert(site.id.clone(), site_ref);
    }
    let root_idx = root_idx.expect("Root site not found");

    // Iterate all UISP devices and if their parent site is in the graph, add them
    let mut device_map = HashMap::new();
    for device in uisp_data.devices_raw.iter() {
        let Some(site_id) = &device.identification.site else {
            continue;
        };
        let site_id = &site_id.id;
        let Some(site_ref) = site_map.get(site_id) else {
            continue;
        };
        let device_entry = GraphMapping::AccessPoint {
            name: device.get_name().unwrap_or_default(),
            id: device.identification.id.clone(),
        };
        let device_ref = graph.add_node(device_entry);
        device_map.insert(device.identification.id.clone(), device_ref);
        let _ = graph.add_edge(device_ref, *site_ref, LinkMapping::Ethernet);
    }

    // Now we iterate all the data links looking for DEVICE linkage
    for link in uisp_data.data_links_raw.iter() {
        let Some(from_device) = &link.from.device else {
            continue;
        };
        let Some(to_device) = &link.to.device else {
            continue;
        };
        let Some(a_ref) = device_map.get(&from_device.identification.id) else {
            continue;
        };
        let Some(b_ref) = device_map.get(&to_device.identification.id) else {
            continue;
        };
        if a_ref == b_ref {
            // If the devices are the same, we don't need to add an edge
            continue;
        }
        if let Some(dev_a) = uisp_data.devices_raw.iter().find(|d| d.get_id() == from_device.identification.id) {
            if let Some(dev_b) = uisp_data.devices_raw.iter().find(|d| d.get_id() == to_device.identification.id) {
                if dev_a.get_site_id().unwrap_or_default() == dev_b.get_site_id().unwrap_or_default() {
                    // If the devices are in the same site, we don't need to add an edge
                    continue;
                }
            }
        }
        if graph.contains_edge(*a_ref, *b_ref) {
            // If the edge already exists, we don't need to add it
            continue;
        }
        graph.add_edge(*a_ref, *b_ref, LinkMapping::DevicePair(from_device.identification.id.clone(), to_device.identification.id.clone()));
    }

    // Now we iterate only sites, looking for connectivity to the root
    let orphans = graph.add_node(GraphMapping::GeneratedSite{ name: "Orphans".to_string()});
    graph.add_edge(root_idx, orphans, LinkMapping::Ethernet);

    for (_, site_ref) in site_map.iter() {
        if *site_ref == root_idx {
            continue;
        }
        let a_star_run = petgraph::algo::astar(
            &graph,
            *site_ref,
            |n| {
                n.id() == root_idx.id()
            },
            |_| 0,
            |_| 0,
        );

        if a_star_run.is_none() {
            warn!(
                "No path is detected from {:?} to {}",
                graph[*site_ref], root_site_name
            );
            graph.add_edge(*site_ref, orphans, LinkMapping::Ethernet);
        }
    }

    // Client mapping
    let client_mappings = uisp_data.map_clients_to_aps();

    // Find the APs that have clients
    let mut aps_with_clients = HashSet::new();
    for (ap_name, _client_ids) in client_mappings.iter() {
        let Some(ap_device) = uisp_data.devices_raw.iter().find(|d| d.get_name().unwrap_or_default() == *ap_name) else {
            // Orphaning is already handled
            continue;
        };
        aps_with_clients.insert(ap_device.identification.id.clone());
    }

    // Count linkages to APs
    let mut ap_link_count = HashMap::<String, usize>::new();
    for edge_ref in graph.edge_references() {
        if let GraphMapping::AccessPoint{ref id, ..} = graph[edge_ref.source()] {
            let entry = ap_link_count.entry(id.clone()).or_insert(0);
            *entry += 1;
        };
        if let GraphMapping::AccessPoint{ref id, ..} = graph[edge_ref.target()] {
            let entry = ap_link_count.entry(id.clone()).or_insert(0);
            *entry += 1;
        };
    }

    // Cull the APs that have no clients or only one link
    let mut to_remove = Vec::new();
    for (ap_id, ap_ref) in device_map.iter() {
        if aps_with_clients.contains(ap_id) {
            continue;
        }
        if let Some(link_count) = ap_link_count.get(ap_id) {
            if *link_count > 1 {
                continue;
            }
        }
        to_remove.push(*ap_ref);
    }
    info!("Removing {} APs with no clients or only one link", to_remove.len());
    for ap_ref in to_remove.iter() {
        graph.remove_node(*ap_ref);
    }

    // Visualizer
    save_dot_file(&graph)?;
    let _ = blackboard_blob("UISP-Graph", &graph).await;

    // Figure out the network.json layers
    let mut parents = HashMap::new();
    for node in graph.node_indices() {
        if node == root_idx {
            continue;
        }
        match &graph[node] {
            GraphMapping::GeneratedSite{name} | GraphMapping::Site{name, ..} |
            GraphMapping::AccessPoint{name, ..} => {
                let route = petgraph::algo::astar(
                    &graph,
                    root_idx,
                    |n| n == node,
                    |e| (10_000u64).saturating_sub(link_capacity_mbps(&e.weight(), &uisp_data.devices, &routing_overrides)),
                    |_| 0,
                ).unwrap_or((0, vec![]));

                if route.1.is_empty() {
                    //println!("No path detected from {:?} to {}", graph[node], root_site_name);
                    parents.insert(name, ("Orphans".to_owned(), (config.queues.generated_pn_download_mbps, config.queues.generated_pn_upload_mbps)));
                } else {
                    let mut capacity = (config.queues.generated_pn_download_mbps,config.queues.generated_pn_upload_mbps);
                        if !config.uisp_integration.ignore_calculated_capacity {
                            match &graph[node] {
                                GraphMapping::AccessPoint { name, id } => {
                                    if let Some(device) = uisp_data.devices.iter().find(|d| d.id == *id) {
                                        println!("Device: {} / {}", device.download, device.upload);
                                        capacity = (device.download, device.upload);
                                        if let Some(bw_override) = bandwidth_overrides.get(&device.name) {
                                            capacity = (bw_override.0 as u64, bw_override.1 as u64);
                                        }
                                    }
                                }
                            _ => {}
                        }
                    }

                    let parent_node = route.1[route.1.len() - 2];
                    // We need the weight from node to parent_node in the graph edges
                    if let Some(edge) = graph.find_edge(parent_node, node) {
                        let parent = graph[route.1[route.1.len() - 2]].name();
                        parents.insert(name, (parent, capacity));
                    } else {
                        panic!("DID NOT FIND THE EDGE");
                    }
                }

                //let parent_index = route.1.iter().last().unwrap();
                //let parent = graph[*parent_index].clone();
                //println!("Parent of {:?} is {:?}", graph[node], parent);
            }
            _ => {}
        }
    }
    //println!("Parents: {:#?}", parents);

    // Write the network.json file
    let mut network_json = serde_json::Map::new();
    for (name, (_parent, (download, upload))) in parents.iter().filter(|(_, (parent, _))| *parent == root_site_name) {
        network_json.insert((*name).into(), walk_parents(&parents, name, &config, *download, *upload).into());
    }
    let network_path = Path::new(&config.lqos_directory).join("network.json");
    if network_path.exists() && !config.integration_common.always_overwrite_network_json {
        warn!(
            "Network.json exists, and always overwrite network json is not true - not writing network.json"
        );
        return Ok(());
    }
    let json = serde_json::to_string_pretty(&network_json).unwrap();
    write(network_path, json).map_err(|e| {
        error!("Unable to write network.json");
        error!("{e:?}");
        UispIntegrationError::WriteNetJson
    })?;
    info!("Written network.json");

    // Shaped Devices
    let mut shaped_devices = Vec::new();

    for (ap_id, client_sites) in client_mappings.iter() {
        for site_id in client_sites.iter() {
            let Some(ap_device) = uisp_data.devices.iter().find(|d| d.name == *ap_id) else {
                continue;
            };
            let Some(site) = uisp_data.sites.iter().find(|s| s.id == *site_id) else {
                continue;
            };
            for device in uisp_data.devices.iter().filter(|d| d.site_id == *site_id) {
                if !device.has_address() {
                    continue;
                }
                let shaped_device = ShapedDevice {
                    circuit_id: site.id.to_owned(),
                    circuit_name: site.name.to_owned(),
                    device_id: device.id.to_owned(),
                    device_name: device.name.to_owned(),
                    parent_node: ap_device.name.to_owned(),
                    mac: device.mac.to_owned(),
                    ipv4: device.ipv4_list(),
                    ipv6: device.ipv6_list(),
                    download_min: (config.uisp_integration.commit_bandwidth_multiplier * site.max_down_mbps as f32) as u64,
                    upload_min: (config.uisp_integration.commit_bandwidth_multiplier * site.max_up_mbps as f32) as u64,
                    download_max: site.max_down_mbps,
                    upload_max: site.max_up_mbps,
                    comment: "".to_string(),
                };
                shaped_devices.push(shaped_device);
            }
        }
    }
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

fn save_dot_file(graph: &GraphType) -> Result<(), UispIntegrationError> {
    // Save the dot file
    let dot_data = format!("{:?}", petgraph::dot::Dot::with_config(graph, &[petgraph::dot::Config::EdgeNoLabel]));
    let _ = std::fs::write("graph.dot", dot_data.as_bytes());
    let _ = std::process::Command::new("dot")
        .arg("-Tpng")
        .arg("graph.dot")
        .arg("-o")
        .arg("graph.png")
        .output();
    Ok(())
}

fn walk_parents(
    parents: &HashMap<&String, (String, (u64, u64))>,
    name: &String,
    config: &Arc<Config>,
    download: u64,
    upload: u64,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();

    // Entries are name, type, uisp_device or site, downloadBandwidthMbps, uploadBandwidthMbps, children
    map.insert("name".into(), name.clone().into());
    map.insert("downloadBandwidthMbps".into(), download.into());
    map.insert("uploadBandwidthMbps".into(), upload.into());

    let mut children = serde_json::Map::new();
    for (name, (_parent, (download, upload))) in parents.iter().filter(|(_, (parent, _))|
        *parent == *name) {
        let child = walk_parents(parents, name, config, *download, *upload);
        children.insert((*name).into(), child.into());
    }

    map.insert("children".into(), children.into());

    map
}

fn network_json_capacity(
    config: &Arc<Config>,
    link: &LinkMapping,
    devices: &[UispDevice],
) -> (u64, u64) {
    // Handle ignoring capacity
    if config.uisp_integration.ignore_calculated_capacity {
        return (config.queues.generated_pn_download_mbps, config.queues.generated_pn_upload_mbps);
    }

    // Handle the link type
    match link {
        LinkMapping::Ethernet => {
            return (config.queues.generated_pn_download_mbps, config.queues.generated_pn_upload_mbps);
        }
        LinkMapping::DevicePair(device_a, device_b) => {
            // Find the devices
            let device_a = devices.iter().find(|d| d.id == *device_a);
            let device_b = devices.iter().find(|d| d.id == *device_b);

            if let Some(device_a) = device_a {
                if let Some(device_b) = device_b {
                    return (
                        device_a.download,
                        device_b.upload,
                    );
                }
            }
        }
    }

    (config.queues.generated_pn_download_mbps, config.queues.generated_pn_upload_mbps)
}


fn link_capacity_mbps(link_mapping: &LinkMapping, devices: &[UispDevice], route_overrides: &[RouteOverride]) -> u64 {
    match link_mapping {
        LinkMapping::Ethernet => 10_000,
        LinkMapping::DevicePair(device_a, device_b) => {
            let capacity ;

            if let Some(device_a) = devices.iter().find(|d| d.id == *device_a) {
                if let Some(override_a) = route_overrides.iter().find(|o| o.from_site == device_a.name || o.to_site == device_a.name) {
                    capacity = override_a.cost as u64;
                } else {
                    capacity = device_a.download;
                }
            } else if let Some(device_b) = devices.iter().find(|d| d.id == *device_b) {
                if let Some(override_b) = route_overrides.iter().find(|o| o.from_site == device_b.name || o.to_site == device_b.name) {
                    capacity = override_b.cost as u64;
                } else {
                    capacity = device_b.download;
                }
            } else {
                capacity = 10_000;
            }

            capacity
        }
    }
}

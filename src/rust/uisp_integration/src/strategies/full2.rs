mod dot;
mod graph_mapping;
mod link_mapping;
mod net_json_parent;

use crate::blackboard_blob;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::common::UispData;
use crate::strategies::full::routes_override::RouteOverride;
use crate::strategies::full::shaped_devices_writer::ShapedDevice;
use crate::strategies::full2::dot::save_dot_file;
use crate::strategies::full2::graph_mapping::GraphMapping;
use crate::strategies::full2::link_mapping::LinkMapping;
use crate::strategies::full2::net_json_parent::{NetJsonParent, walk_parents};
use crate::uisp_types::UispDevice;
use lqos_config::Config;
use petgraph::graph::NodeIndex;
use petgraph::visit::{EdgeRef, NodeRef};
use petgraph::Directed;
use std::collections::{HashMap, HashSet};
use std::fs::write;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

type GraphType = petgraph::Graph<GraphMapping, LinkMapping, Directed>;

pub async fn build_full_network_v2(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    // Fetch the data
    let uisp_data = UispData::fetch_uisp_data(config.clone(), ip_ranges).await?;

    if let Err(e) = blackboard_blob("uisp_sites", &uisp_data.sites_raw).await {
        warn!("Unable to write sites to blackboard: {e:?}");
    }
    if let Err(e) = blackboard_blob("uisp_devices", &uisp_data.devices_raw).await {
        warn!("Unable to write devices to blackboard: {e:?}");
    }
    if let Err(e) = blackboard_blob("uisp_data_links", &uisp_data.data_links_raw).await {
        warn!("Unable to write data links to blackboard: {e:?}");
    }

    // Report on obvious UISP errors that should be fixed
    let _trouble = crate::strategies::ap_site::find_troublesome_sites(&uisp_data)
        .await
        .map_err(|e| {
            error!("Error finding troublesome sites");
            error!("{e:?}");
            UispIntegrationError::UnknownSiteType
        })?;

    // Load overrides
    let bandwidth_overrides =
        crate::strategies::full::bandwidth_overrides::get_site_bandwidth_overrides(&config)?;
    let routing_overrides = crate::strategies::full::routes_override::get_route_overrides(&config)?;

    // Create a new graph
    let mut graph = GraphType::new();

    // Find the root
    let root_site_name = config.uisp_integration.site.clone();

    // Add all sites to the graph
    let mut site_map = HashMap::new();
    let mut root_idx = None;
    add_all_sites_to_graph(
        &uisp_data,
        &mut graph,
        &root_site_name,
        &mut site_map,
        &mut root_idx,
    );
    let root_idx = root_idx.expect("Root site not found");

    // Iterate all UISP devices and if their parent site is in the graph, add them
    let mut device_map = HashMap::new();
    add_devices_to_graph(&uisp_data, &mut graph, &mut site_map, &mut device_map, &config, &bandwidth_overrides);

    // Now we iterate all the data links looking for DEVICE linkage
    add_device_links_to_graph(&uisp_data, &mut graph, &mut device_map, &config);

    // Now we iterate only sites, looking for connectivity to the root
    let orphans = graph.add_node(GraphMapping::GeneratedSite {
        name: "Orphans".to_string(),
    });
    graph.add_edge(root_idx, orphans, LinkMapping::ethernet(config.queues.generated_pn_download_mbps));
    graph.add_edge(orphans, root_idx, LinkMapping::ethernet(config.queues.generated_pn_upload_mbps));
    for (_, site_ref) in site_map.iter() {
        if *site_ref == root_idx {
            continue;
        }
        let a_star_run =
            petgraph::algo::astar(&graph, *site_ref, |n| n.id() == root_idx.id(), |_| 0, |_| 0);

        if a_star_run.is_none() {
            warn!(
                "No path is detected from {:?} to {}",
                graph[*site_ref], root_site_name
            );
            graph.add_edge(*site_ref, orphans, LinkMapping::ethernet(config.queues.generated_pn_download_mbps));
            graph.add_edge(orphans, *site_ref, LinkMapping::ethernet(config.queues.generated_pn_upload_mbps));
        }
    }

    // Client mapping
    let client_mappings = uisp_data.map_clients_to_aps();

    // Find the APs that have clients
    let mut aps_with_clients = HashSet::new();
    for (ap_name, _client_ids) in client_mappings.iter() {
        let Some(ap_device) = uisp_data
            .devices_raw
            .iter()
            .find(|d| d.get_name().unwrap_or_default() == *ap_name)
        else {
            // Orphaning is already handled
            continue;
        };
        aps_with_clients.insert(ap_device.identification.id.clone());
    }

    // Count linkages to APs
    let mut ap_link_count = HashMap::<String, usize>::new();
    for edge_ref in graph.edge_references() {
        if let GraphMapping::AccessPoint { ref id, .. } = graph[edge_ref.source()] {
            let entry = ap_link_count.entry(id.clone()).or_insert(0);
            *entry += 1;
        };
        if let GraphMapping::AccessPoint { ref id, .. } = graph[edge_ref.target()] {
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
    info!(
        "Removing {} APs with no clients or only one link",
        to_remove.len()
    );
    for ap_ref in to_remove.iter() {
        graph.remove_node(*ap_ref);
    }

    // Visualizer
    save_dot_file(&graph)?;
    let _ = blackboard_blob("uisp-graph", vec![graph.clone()]).await;

    // Figure out the network.json layers
    let mut parents = HashMap::<String, NetJsonParent>::new();
    for node in graph.node_indices() {
        if node == root_idx {
            continue;
        }
        match &graph[node] {
            GraphMapping::GeneratedSite { name }
            | GraphMapping::Site { name, .. }
            | GraphMapping::AccessPoint { name, .. } => {
                // Generate two routes - one per direction
                let route_from_root_to_node = petgraph::algo::astar(
                    &graph,
                    root_idx,
                    |n| n == node,
                    |e| {
                        (10_000u64).saturating_sub(link_capacity_mbps_for_routing(
                            &e.weight(),
                            &uisp_data.devices,
                            &routing_overrides,
                        ))
                    },
                    |_| 0,
                )
                .unwrap_or((0, vec![]));

                let route_from_node_to_root = petgraph::algo::astar(
                    &graph,
                    node,
                    |n| n == root_idx,
                    |e| {
                        (10_000u64).saturating_sub(link_capacity_mbps_for_routing(
                            &e.weight(),
                            &uisp_data.devices,
                            &routing_overrides,
                        ))
                    },
                    |_| 0,
                )
                    .unwrap_or((0, vec![]));

                // println!("From node to root:");
                // println!("{:?}", route_from_node_to_root);
                // println!("{:?}", edges_from_node_path(&graph, &route_from_node_to_root.1));
                //
                // println!("From root to node:");
                // println!("{:?}", route_from_root_to_node);
                // println!("{:?}", edges_from_node_path(&graph, &route_from_root_to_node.1));

                if route_from_root_to_node.1.is_empty() {
                    //println!("No path detected from {:?} to {}", graph[node], root_site_name);
                    parents.insert(
                        name.to_owned(),
                        NetJsonParent {
                            parent_name: "Orphans".to_string(),
                            mapping: &graph[node],
                            download: config.queues.generated_pn_download_mbps,
                            upload: config.queues.generated_pn_upload_mbps,
                        },
                    );
                } else {
                    // Obtain capacities from route traversal
                    let mut download_capacity = min_capacity_along_route(&graph, &route_from_root_to_node.1);
                    let mut upload_capacity = min_capacity_along_route(&graph, &route_from_node_to_root.1);

                    // Apply AP capacities
                    if let GraphMapping::AccessPoint { download_mbps, upload_mbps, .. } = &graph[node] {
                        //println!("AP device capacity: {download_mbps}/{upload_mbps} (prev {download_capacity}/{upload_capacity})");
                        download_capacity = u64::min(download_capacity, *download_mbps);
                        upload_capacity = u64::min(upload_capacity, *upload_mbps);
                        //println!("Now: {download_capacity}/{upload_capacity}");
                    }

                    // 0 isn't possible
                    if download_capacity < 1 {
                        download_capacity = config.queues.generated_pn_download_mbps;
                    }
                    if upload_capacity < 1 {
                        upload_capacity = config.queues.generated_pn_upload_mbps;
                    }

                    // Ignore option
                    if config.uisp_integration.ignore_calculated_capacity {
                        download_capacity = config.queues.generated_pn_download_mbps;
                        upload_capacity = config.queues.generated_pn_upload_mbps;
                    }

                    // Overrides
                    if !bandwidth_overrides.is_empty() {
                        if let Some(bw_override) = bandwidth_overrides.get(name) {
                            info!("Applying bandwidth override for {}", name);
                            info!("Capacity was: {} / {}", download_capacity, upload_capacity);
                            download_capacity = bw_override.0 as u64;
                            upload_capacity = bw_override.1 as u64;
                            info!("Capacity is now: {} / {}", download_capacity, upload_capacity);
                        }
                    }

                    let parent_node = route_from_root_to_node.1[route_from_root_to_node.1.len() - 2];
                    // We need the weight from node to parent_node in the graph edges
                    if let Some(_edge) = graph.find_edge(parent_node, node) {
                        let parent = graph[route_from_root_to_node.1[route_from_root_to_node.1.len() - 2]].name();
                        parents.insert(
                            name.to_owned(),
                            NetJsonParent {
                                parent_name: parent,
                                mapping: &graph[node],
                                download: download_capacity,
                                upload: upload_capacity,
                            },
                        );
                    } else {
                        panic!("DID NOT FIND THE EDGE");
                    }
                }
            }
            _ => {}
        }
    }

    // Write the network.json file
    let mut network_json = serde_json::Map::new();
    for (name, node_info) in parents
        .iter()
        .filter(|(_name, parent)| parent.parent_name == root_site_name)
    {
        network_json.insert(
            name.into(),
            walk_parents(&parents, name, &node_info, &config, &graph).into(),
        );
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

                let download =
                    (site.max_down_mbps as f32) * config.uisp_integration.bandwidth_overhead_factor;
                let upload =
                    (site.max_up_mbps as f32) * config.uisp_integration.bandwidth_overhead_factor;
                let download_min =
                    (download * config.uisp_integration.commit_bandwidth_multiplier) as u64;
                let upload_min =
                    (upload * config.uisp_integration.commit_bandwidth_multiplier) as u64;
                let download_max = download as u64;
                let upload_max = upload as u64;

                let parent_node = {
                    if parents.get(&ap_device.name).is_some() {
                        ap_device.name.clone()
                    } else {
                        "Orphans".to_string()
                    }
                };

                let shaped_device = ShapedDevice {
                    circuit_id: site.id.to_owned(),
                    circuit_name: site.name.to_owned(),
                    device_id: device.id.to_owned(),
                    device_name: device.name.to_owned(),
                    parent_node,
                    mac: device.mac.to_owned(),
                    ipv4: device.ipv4_list(),
                    ipv6: device.ipv6_list(),
                    download_min,
                    upload_min,
                    download_max,
                    upload_max,
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

fn add_device_links_to_graph(
    uisp_data: &UispData,
    graph: &mut GraphType,
    device_map: &mut HashMap<String, NodeIndex>,
    config: &Arc<Config>,
) {
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
        if let Some(dev_a) = uisp_data
            .devices_raw
            .iter()
            .find(|d| d.get_id() == from_device.identification.id)
        {
            if let Some(dev_b) = uisp_data
                .devices_raw
                .iter()
                .find(|d| d.get_id() == to_device.identification.id)
            {
                if dev_a.get_site_id().unwrap_or_default()
                    == dev_b.get_site_id().unwrap_or_default()
                {
                    // If the devices are in the same site, we don't need to add an edge
                    continue;
                }
            }
        }
        if graph.contains_edge(*a_ref, *b_ref) {
            // If the edge already exists, we don't need to add it
            continue;
        }
        if graph.contains_edge(*b_ref, *a_ref) {
            // If the edge already exists, we don't need to add it
            continue;
        }

        let (down, up) = get_capacity_from_datalink_device(
            &from_device.identification.id,
            &uisp_data.devices,
            &config,
        );
        graph.add_edge(
            *a_ref,
            *b_ref,
            LinkMapping::DevicePair {
                device_a: from_device.identification.id.clone(),
                device_b: to_device.identification.id.clone(),
                speed_mbps: down,
            },
        );
        graph.add_edge(
            *b_ref,
            *a_ref,
            LinkMapping::DevicePair {
                device_b: from_device.identification.id.clone(),
                device_a: to_device.identification.id.clone(),
                speed_mbps: up,
            },
        );
    }
}

fn get_capacity_from_datalink_device(
    device_id: &str,
    devices: &[UispDevice],
    config: &Arc<Config>,
) -> (u64, u64) {
    if let Some(device) = devices.iter().find(|d| d.id == device_id) {
        return (device.download, device.upload);
    }

    (config.queues.generated_pn_download_mbps, config.queues.generated_pn_upload_mbps)
}

fn add_devices_to_graph(
    uisp_data: &UispData,
    graph: &mut GraphType,
    site_map: &mut HashMap<String, NodeIndex>,
    device_map: &mut HashMap<String, NodeIndex>,
    config: &Arc<Config>,
    bandwidth_overrides: &BandwidthOverrides,
) {
    for device in uisp_data.devices_raw.iter() {
        let Some(site_id) = &device.identification.site else {
            continue;
        };
        let site_id = &site_id.id;
        let Some(device_details) = uisp_data.devices.iter().find(|d| d.id == device.identification.id) else {
            continue;
        };
        let Some(site_ref) = site_map.get(site_id) else {
            continue;
        };
        let mut download_mbps = device_details.download;
        let mut upload_mbps = device_details.upload;

        if download_mbps < 1 {
            download_mbps = config.queues.generated_pn_download_mbps;
        }
        if upload_mbps < 1 {
            upload_mbps = config.queues.generated_pn_upload_mbps;
        }

        if config.uisp_integration.ignore_calculated_capacity {
            download_mbps = config.queues.generated_pn_download_mbps;
            upload_mbps = config.queues.generated_pn_upload_mbps;
        }

        if let Some(bw_override) = bandwidth_overrides.get(&device.get_name().unwrap_or_default()) {
            download_mbps = bw_override.0 as u64;
            upload_mbps = bw_override.1 as u64;
        }

        let device_entry = GraphMapping::AccessPoint {
            name: device.get_name().unwrap_or_default(),
            id: device.identification.id.clone(),
            site_name: graph[*site_ref].name(),
            download_mbps,
            upload_mbps,
        };
        let device_ref = graph.add_node(device_entry);
        device_map.insert(device.identification.id.clone(), device_ref);
        let _ = graph.add_edge(device_ref, *site_ref, LinkMapping::ethernet(config.queues.generated_pn_download_mbps));
        let _ = graph.add_edge(*site_ref, device_ref, LinkMapping::ethernet(config.queues.generated_pn_upload_mbps));
    }
}

pub fn add_all_sites_to_graph(
    uisp_data: &UispData,
    graph: &mut GraphType,
    root_site_name: &String,
    site_map: &mut HashMap<String, NodeIndex>,
    root_idx: &mut Option<NodeIndex>,
) {
    for site in uisp_data.sites_raw.iter().filter(|s| !s.is_client_site()) {
        let site_name = site.name_or_blank();
        let id = site.id.clone();
        if site_name == *root_site_name {
            // Add the root site
            let root_entry = GraphMapping::Root {
                name: site_name,
                id,
            };
            let root_ref = graph.add_node(root_entry);
            *root_idx = Some(root_ref);
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
}

fn link_capacity_mbps_for_routing(
    link_mapping: &LinkMapping,
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> u64 {
    match link_mapping {
        LinkMapping::Ethernet { speed_mbps } => *speed_mbps,
        LinkMapping::DevicePair { speed_mbps, device_a, device_b } => {
            // Check for overrides: make sure we have both devices (we need names, are storing IDs)
            let Some(device_a) = devices.iter().find(|d| d.id == *device_a) else {
                return *speed_mbps;
            };
            let Some(device_b) = devices.iter().find(|d| d.id == *device_b) else {
                return *speed_mbps;
            };

            // Check for the direct direction
            if let Some(route_override) = route_overrides
                .iter()
                .find(|route_override|
                    (route_override.from_site == device_a.name || route_override.to_site == device_a.name)
                        && (route_override.from_site == device_b.name || route_override.to_site == device_b.name)
                ) {
                return route_override.cost as u64;
            }

            // Check for the reverse direction
            if let Some(route_override) = route_overrides
                .iter()
                .find(|route_override|
                    (route_override.from_site == device_b.name || route_override.to_site == device_b.name)
                        && (route_override.from_site == device_a.name || route_override.to_site == device_a.name)
                ) {
                return route_override.cost as u64;
            }

            *speed_mbps
        }
    }
}

use petgraph::prelude::*;
use crate::strategies::full::bandwidth_overrides::BandwidthOverrides;

fn edges_from_node_path(
    graph: &GraphType,
    path: &[NodeIndex],
) -> Vec<EdgeIndex>
{
    let mut edge_ids = Vec::new();
    for window in path.windows(2) {
        let source = window[0];
        let target = window[1];
        // This assumes a directed graph; if undirected, you might need to check both directions
        if let Some(edge) = graph.find_edge(source, target) {
            edge_ids.push(edge);
        } else {
            // Handle case where no edge is found (could be error or special case)
            panic!("No edge found between {:?} and {:?}", source, target);
        }
    }
    edge_ids
}

fn min_capacity_along_route(
    graph: &GraphType,
    path: &[NodeIndex],
) -> u64 {
    edges_from_node_path(graph, path)
        .iter()
        .map(|edge| graph[*edge].capacity_mbps())
        .min()
        .unwrap_or(0)
}
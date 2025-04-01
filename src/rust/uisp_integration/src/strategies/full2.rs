use std::collections::HashMap;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::ap_site::GraphMapping;
use crate::strategies::common::UispData;
use crate::uisp_types::{UispDevice, UispSiteType};
use lqos_config::Config;
use petgraph::data::Build;
use std::sync::Arc;
use tracing::{error, info, warn};
use uisp::{DataLinkDevice, DataLinkSite, Device, Site};

#[derive(Debug, Clone)]
pub enum LinkMapping {
    Ethernet,
    DevicePair(String, String),
}

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

    // Find the clients
    let ap_mappings = uisp_data.map_clients_to_aps();

    // Make AP Layer entries
    let access_points = crate::strategies::ap_site::get_ap_layer(&ap_mappings);

    // Site mappings
    let sites =
        crate::strategies::ap_site::map_sites_above_aps(&uisp_data, ap_mappings, access_points);

    // Insert the root
    let mut root = crate::strategies::ap_site::Layer {
        id: GraphMapping::Root,
        children: sites.values().cloned().collect(),
    };

    // Now transform this into a Petgraph graph
    let mut graph = petgraph::Graph::<GraphMapping, LinkMapping>::new();
    root.walk_site_ap_to_pet_graph(&mut graph, None);

    // Then add other sites
    for site in uisp_data.sites_raw.iter() {
        // Skip clients
        if site.is_client_site() {
            continue;
        }
        // Find the node index of an entry in the graph if there is one
        let mut node_index = graph
            .node_indices()
            .find(|i| graph[*i] == GraphMapping::SiteByName(site.name_or_blank()));

        // If it already exists, then we're interested in finding links between it and other sites
        if let Some(_node_index) = node_index {
            // Do nothing
        } else {
            // If it doesn't exist, then we need to add it
            graph.add_node(GraphMapping::SiteByName(site.name_or_blank()));
        }
    }

    // Now iterate all the links!
    for link in uisp_data.data_links_raw.iter() {
        if let Some(from_site) = &link.from.site {
            if let Some(to_site) = &link.to.site {
                add_link_if_new(
                    from_site,
                    to_site,
                    &link.from.device,
                    &link.to.device,
                    &mut graph,
                    &uisp_data.sites_raw,
                );
            }
        }
    }

    // Now we make a heroic effort to find sites that are linked through UISP's semi-broken
    // API!
    for graph_node in graph.node_indices() {
        let name = graph[graph_node].name();
        if let Some(site_info) = uisp_data.find_site_by_name(&name) {
            let site_id = &site_info.id;
            for device in uisp_data.devices_raw.iter() {
                if let Some(site) = &device.identification.site {
                    if site.id == *site_id {
                        if let Some(attr) = &device.attributes {
                            if let Some(ap) = &attr.apDevice {
                                if let Some(ap_device_id) = &ap.id {
                                    if let Some(ap_device) = uisp_data.devices_raw.iter().find(|d| d.get_id() == *ap_device_id) {
                                        if ap_device.get_site_id().unwrap_or_default() != *site_id {
                                            // After all this nesting, we have found a link
                                            if let Some(other_site) = uisp_data.sites_raw.iter().find(|s| s.id == ap_device.get_site_id().unwrap_or_default()) {
                                                if !other_site.is_client_site() {
                                                    // Add the link
                                                    //println!("Adding link from {} to {}", name, other_site.name_or_blank());
                                                    add_link_if_new_from_ap_check(
                                                        &site_info,
                                                        other_site,
                                                        device,
                                                        ap_device,
                                                        &mut graph,
                                                    )
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Find the root
    let root_site_name = config.uisp_integration.site.clone();
    let Some(root_idx) = graph.node_indices().find(|n| {
        if let GraphMapping::SiteByName(name) = &graph[*n] {
            *name == root_site_name
        } else {
            false
        }
    }) else {
        error!("Unable to locate the root note, {}", root_site_name);
        return Err(UispIntegrationError::NoRootSite);
    };

    // Working up to adding the shaper to the detected (named) root
    let mut orphans = None;
    for node in graph.node_indices() {
        if node == root_idx {
            continue;
        }
        let a_star_run = petgraph::algo::astar(
            &graph,
            root_idx,
            |n| n == node,
            |e| (10_000u64).saturating_sub(link_capacity_mbps(&e.weight(), &uisp_data.devices)),
            |_| 0,
        );

        if a_star_run.is_none() {
            warn!(
                "No path is detected from {:?} to {}",
                graph[node], root_site_name
            );
            if orphans.is_none() {
                orphans =
                    Some(graph.add_node(GraphMapping::GeneratedSiteByName("Orphans".to_string())));
                graph.add_edge(root_idx, orphans.unwrap(), LinkMapping::Ethernet);
            }
            if let Some(orphans) = orphans {
                graph.add_edge(node, orphans, LinkMapping::Ethernet);
            }
        } else {
            let all_paths =
                petgraph::algo::all_simple_paths::<Vec<_>, _>(&graph, node, root_idx, 0, None)
                    .collect::<Vec<_>>();

            //println!("Path detected from {:?} to {}", graph[node], root_site_name);
            if all_paths.len() > 1 {
                println!(
                    "Multiple paths detected from {:?} to {}",
                    graph[node], root_site_name
                );
            }
        }
    }

    // Save the dot file
    let dot_data = format!("{:?}", petgraph::dot::Dot::with_config(&graph, &[petgraph::dot::Config::EdgeNoLabel]));
    let _ = std::fs::write("graph.dot", dot_data.as_bytes());
    let _ = std::process::Command::new("dot")
        .arg("-Tpng")
        .arg("graph.dot")
        .arg("-o")
        .arg("graph.png")
        .output();

    // Build the output data
    let mut shaped_devices = Vec::new();
    let _ = root.walk_children(None, &uisp_data, &mut shaped_devices, &config);

    // Write SD
    let _ = crate::strategies::ap_site::write_shaped_devices(&config, &mut shaped_devices);
    info!("Wrote {} lines to ShapedDevices.csv", shaped_devices.len());

    // Figure out the network.json layers
    let mut parents = HashMap::new();
    for node in graph.node_indices() {
        if node == root_idx {
            continue;
        }
        match &graph[node] {
            GraphMapping::GeneratedSiteByName(name) | GraphMapping::SiteByName(name) |
            GraphMapping::AccessPointByName(name) => {
                let route = petgraph::algo::astar(
                    &graph,
                    root_idx,
                    |n| n == node,
                    |e| (10_000u64).saturating_sub(link_capacity_mbps(&e.weight(), &uisp_data.devices)),
                    |_| 0,
                ).unwrap_or((0, vec![]));

                if route.1.is_empty() {
                    //println!("No path detected from {:?} to {}", graph[node], root_site_name);
                    parents.insert(name, ("Orphans".to_owned(), (config.queues.generated_pn_download_mbps, config.queues.generated_pn_upload_mbps)));
                } else {
                    let parent_node = route.1[route.1.len() - 2];
                    // We need the weight from node to parent_node in the graph edges
                    if let Some(edge) = graph.find_edge(parent_node, node) {
                        // From EdgeIndex to LinkMapping
                        let edge = graph[edge].clone();
                        //println!("FOUND THE EDGE: {:?}", edge);

                        let parent = graph[route.1[route.1.len() - 2]].name();
                        parents.insert(name, (parent, network_json_capacity(&config, &edge, &uisp_data.devices)));
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
    for (name, (parent, (download, upload))) in parents.iter().filter(|(_, (parent, _))| *parent == root_site_name) {
        network_json.insert(name.clone().into(), walk_parents().into());
    }

    Ok(())
}

fn walk_parents() -> serde_json::Map<String, serde_json::Value> {

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

fn link_capacity_mbps(link_mapping: &LinkMapping, devices: &[UispDevice]) -> u64 {
    match link_mapping {
        LinkMapping::Ethernet => 10_000,
        LinkMapping::DevicePair(device_a, device_b) => {
            let mut capacity = 0;
            if let Some(device_a) = devices.iter().find(|d| d.id == *device_a) {
                capacity = device_a.download;
            } else if let Some(device_b) = devices.iter().find(|d| d.id == *device_b) {
                capacity = device_b.download;
            } else {
                capacity = 10_000;
            }

            capacity
        }
    }
}

fn add_link_if_new_from_ap_check(
    site_a: &Site,
    site_b: &Site,
    device_a: &Device,
    device_b: &Device,
    graph: &mut petgraph::Graph<GraphMapping, LinkMapping>,
) {
    if site_a.id == site_b.id {
        // If the sites are the same, we don't need to add an edge
        return;
    }
    let Some(node_a) = graph
        .node_indices()
        .find(|n| graph[*n] == GraphMapping::SiteByName(site_a.name_or_blank()))
    else {
        return;
    };
    let Some(node_b) = graph
        .node_indices()
        .find(|n| graph[*n] == GraphMapping::SiteByName(site_b.name_or_blank()))
    else {
        return;
    };

    if graph.contains_edge(node_a, node_b) {
        // If the edge already exists, we don't need to add it
        return;
    }
    if graph.contains_edge(node_b, node_a) {
        // If the edge already exists in the opposite direction, we don't need to add it
        return;
    }

    // Try to figure out the type of link
    let mut link_type = LinkMapping::Ethernet;

    link_type = LinkMapping::DevicePair(
        device_a.identification.id.clone(),
        device_b.identification.id.clone(),
    );

    // Add the edge
    println!("(AP Scan) Adding edge from {:?} to {:?}", graph[node_a], graph[node_b]);
    graph.add_edge(node_a, node_b, link_type);
}

fn add_link_if_new(
    from_site: &DataLinkSite,
    to_site: &DataLinkSite,
    from_device: &Option<DataLinkDevice>,
    to_device: &Option<DataLinkDevice>,
    graph: &mut petgraph::Graph<GraphMapping, LinkMapping>,
    sites_raw: &[Site],
) {
    if from_site.identification.id == to_site.identification.id {
        // If the sites are the same, we don't need to add an edge
        return;
    }
    let Some(site_a) = sites_raw
        .iter()
        .find(|s| s.id == from_site.identification.id)
    else {
        return;
    };
    if site_a.is_client_site() {
        return;
    }
    let Some(node_a) = graph
        .node_indices()
        .find(|n| graph[*n] == GraphMapping::SiteByName(site_a.name_or_blank()))
    else {
        return;
    };
    let Some(site_b) = sites_raw.iter().find(|s| s.id == to_site.identification.id) else {
        return;
    };
    if site_b.is_client_site() {
        return;
    }
    let Some(node_b) = graph
        .node_indices()
        .find(|n| graph[*n] == GraphMapping::SiteByName(site_b.name_or_blank()))
    else {
        return;
    };

    if graph.contains_edge(node_a, node_b) {
        // If the edge already exists, we don't need to add it
        return;
    }
    if graph.contains_edge(node_b, node_a) {
        // If the edge already exists in the opposite direction, we don't need to add it
        return;
    }

    // Try to figure out the type of link
    let mut link_type = LinkMapping::Ethernet;

    if let Some(device_a) = &from_device {
        if let Some(device_b) = &to_device {
            link_type = LinkMapping::DevicePair(
                device_a.identification.id.clone(),
                device_b.identification.id.clone(),
            );
        }
    }

    // Add the edge
    //println!("Adding edge from {:?} to {:?}", graph[node_a], graph[node_b]);
    graph.add_edge(node_a, node_b, link_type);
}

impl crate::strategies::ap_site::Layer {
    fn walk_site_ap_to_pet_graph(
        &self,
        graph: &mut petgraph::Graph<GraphMapping, LinkMapping>,
        parent: Option<petgraph::graph::NodeIndex>,
    ) {
        // Add the current node
        if let GraphMapping::ClientById(..) = self.id {
            // If this is a client, we don't want to add it to the graph
            return;
        }
        let node_index = if let GraphMapping::Root = self.id {
            None
        } else {
            let node_index = graph.add_node(self.id.clone());
            if let Some(parent_index) = parent {
                graph.add_edge(parent_index, node_index, LinkMapping::Ethernet);
            }
            Some(node_index)
        };

        // Recursively walk to children
        for child in &self.children {
            child.walk_site_ap_to_pet_graph(graph, node_index);
        }
    }
}

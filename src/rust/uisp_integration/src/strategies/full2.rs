mod dot;
mod directionality;
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
use crate::strategies::full2::directionality::{build_device_capacity_map, build_device_link_meta_map, directed_caps_mbps};
use crate::strategies::full2::graph_mapping::GraphMapping;
use crate::strategies::full2::link_mapping::LinkMapping;
use crate::strategies::full2::net_json_parent::{NetJsonParent, walk_parents};
use crate::uisp_types::UispDevice;
use lqos_config::Config;
use petgraph::Directed;
use petgraph::graph::NodeIndex;
use petgraph::visit::{EdgeRef, NodeRef};
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
    add_devices_to_graph(
        &uisp_data,
        &mut graph,
        &mut site_map,
        &mut device_map,
        &config,
        &bandwidth_overrides,
    );

    // Now we iterate all the data links looking for DEVICE linkage
    add_device_links_to_graph(&uisp_data, &mut graph, &mut device_map, &config);

    // Now we iterate only sites, looking for connectivity to the root
    let orphans = graph.add_node(GraphMapping::GeneratedSite {
        name: "Orphans".to_string(),
    });
    graph.add_edge(
        root_idx,
        orphans,
        LinkMapping::ethernet(config.queues.generated_pn_download_mbps),
    );
    graph.add_edge(
        orphans,
        root_idx,
        LinkMapping::ethernet(config.queues.generated_pn_upload_mbps),
    );
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
            graph.add_edge(
                *site_ref,
                orphans,
                LinkMapping::ethernet(config.queues.generated_pn_download_mbps),
            );
            graph.add_edge(
                orphans,
                *site_ref,
                LinkMapping::ethernet(config.queues.generated_pn_upload_mbps),
            );
        }
    }

    // Point-to-point squashing will happen after we know which APs have clients

    // Client mapping
    let client_mappings = uisp_data.map_clients_to_aps();

    // Find the APs that have clients
    let mut aps_with_clients = HashSet::new();
    for (ap_id, _client_ids) in client_mappings.iter() {
        let Some(ap_device) = uisp_data
            .devices_raw
            .iter()
            .find(|d| d.identification.id == *ap_id)
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

    // Now look for point-to-point squash candidates (after client mapping)
    if config.uisp_integration.enable_squashing.unwrap_or(false) {
        find_point_to_point_squash_candidates(&mut graph, &aps_with_clients, &config);
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
                    let mut download_capacity =
                        min_capacity_along_route(&graph, &route_from_root_to_node.1);
                    let mut upload_capacity =
                        min_capacity_along_route(&graph, &route_from_node_to_root.1);

                    // Apply AP capacities
                    if let GraphMapping::AccessPoint {
                        download_mbps,
                        upload_mbps,
                        ..
                    } = &graph[node]
                    {
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
                            info!(
                                "Capacity is now: {} / {}",
                                download_capacity, upload_capacity
                            );
                        }
                    }

                    let parent_node =
                        route_from_root_to_node.1[route_from_root_to_node.1.len() - 2];
                    // We need the weight from node to parent_node in the graph edges
                    if let Some(_edge) = graph.find_edge(parent_node, node) {
                        let parent = graph
                            [route_from_root_to_node.1[route_from_root_to_node.1.len() - 2]]
                            .name();
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

    // Process promote_to_root list
    let promote_to_root_set: std::collections::HashSet<String> = config
        .integration_common
        .promote_to_root
        .as_ref()
        .map(|list| list.iter().cloned().collect())
        .unwrap_or_default();

    if !promote_to_root_set.is_empty() {
        info!(
            "Applying promote_to_root rules for {} nodes: {:?}",
            promote_to_root_set.len(),
            promote_to_root_set
        );
    }

    // Write the network.json file
    let mut network_json = serde_json::Map::new();
    let mut visited = HashSet::new();
    for (name, node_info) in parents.iter().filter(|(_name, parent)| {
        // Include if it's a direct child of root OR if it's in promote_to_root list
        parent.parent_name == root_site_name || promote_to_root_set.contains(_name.as_str())
    }) {
        network_json.insert(
            name.into(),
            walk_parents(&parents, name, &node_info, &config, &graph, &mut visited).into(),
        );
    }
    let network_path = Path::new(&config.lqos_directory).join("network.json");
    if network_path.exists() && !config.integration_common.always_overwrite_network_json {
        warn!(
            "Network.json exists, and always overwrite network json is not true - not writing network.json"
        );
    } else {
        let json = serde_json::to_string_pretty(&network_json).unwrap();
        write(network_path, json).map_err(|e| {
            error!("Unable to write network.json");
            error!("{e:?}");
            UispIntegrationError::WriteNetJson
        })?;
        info!("Written network.json");
    }

    // Shaped Devices
    let mut shaped_devices = Vec::new();
    let mut seen_pairs = HashSet::new();

    for (ap_id, client_sites) in client_mappings.iter() {
        for site_id in client_sites.iter() {
            let Some(ap_device) = uisp_data.devices.iter().find(|d| d.id == *ap_id) else {
                continue;
            };
            let Some(site) = uisp_data.sites.iter().find(|s| s.id == *site_id) else {
                continue;
            };
            info!(
                "Processing site: {} (ID: {}) with AP: {} (ID: {})",
                site.name, site.id, ap_device.name, ap_device.id
            );
            for device in uisp_data.devices.iter().filter(|d| d.site_id == *site_id) {
                if !device.has_address() {
                    continue;
                }

                // Compute subscriber rates: prefer UISP QoS + burst; fallback to capacity-based
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
                        let download_f32 = (site.max_down_mbps as f32)
                            * config.uisp_integration.bandwidth_overhead_factor;
                        let upload_f32 = (site.max_up_mbps as f32)
                            * config.uisp_integration.bandwidth_overhead_factor;
                        let download_min_f32 =
                            download_f32 * config.uisp_integration.commit_bandwidth_multiplier;
                        let upload_min_f32 =
                            upload_f32 * config.uisp_integration.commit_bandwidth_multiplier;
                        (
                            f32::max(0.1, download_min_f32),
                            f32::max(0.1, download_f32),
                            f32::max(0.1, upload_min_f32),
                            f32::max(0.1, upload_f32),
                        )
                    };
                // Ensure max >= min
                if download_max < download_min {
                    download_max = download_min;
                }
                if upload_max < upload_min {
                    upload_max = upload_min;
                }

                let parent_node = {
                    if parents.get(&ap_device.name).is_some() {
                        ap_device.name.clone()
                    } else {
                        warn!(
                            "AP device '{}' not found in parents HashMap, assigning to Orphans",
                            ap_device.name
                        );
                        "Orphans".to_string()
                    }
                };

                let key = (site.id.clone(), device.id.clone());
                if !seen_pairs.insert(key) {
                    continue;
                }

                let shaped_device = ShapedDevice {
                    circuit_id: site.id.to_owned(),
                    circuit_name: site.name.to_owned(),
                    device_id: device.id.to_owned(),
                    device_name: device.name.to_owned(),
                    parent_node: parent_node.clone(),
                    mac: device.mac.to_owned(),
                    ipv4: device.ipv4_list(),
                    ipv6: device.ipv6_list(),
                    download_min,
                    upload_min,
                    download_max,
                    upload_max,
                    comment: "".to_string(),
                };
                info!(
                    "Created shaped device for '{}' in site '{}' with parent '{}'",
                    device.name, site.name, parent_node
                );
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
    let meta_by_id = build_device_link_meta_map(&uisp_data.devices_raw);
    let caps_by_id = build_device_capacity_map(&uisp_data.devices);

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

        let id_a = from_device.identification.id.as_str();
        let id_b = to_device.identification.id.as_str();
        let (cap_ab, cap_ba) =
            if let Some((cap_ab, cap_ba)) = directed_caps_mbps(&meta_by_id, &caps_by_id, config, id_a, id_b) {
                (cap_ab, cap_ba)
            } else {
                warn!(
                    link_id = %link.id,
                    from_id = %id_a,
                    to_id = %id_b,
                    from_name = %from_device.identification.name,
                    to_name = %to_device.identification.name,
                    "Unable to determine AP/station direction for UISP data-link; falling back to from/to mapping (capacity may be reversed)"
                );
                get_capacity_from_datalink_device(id_a, &uisp_data.devices, config)
            };
        graph.add_edge(
            *a_ref,
            *b_ref,
            LinkMapping::DevicePair {
                device_a: id_a.to_string(),
                device_b: id_b.to_string(),
                speed_mbps: cap_ab,
            },
        );
        graph.add_edge(
            *b_ref,
            *a_ref,
            LinkMapping::DevicePair {
                device_a: id_b.to_string(),
                device_b: id_a.to_string(),
                speed_mbps: cap_ba,
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

    (
        config.queues.generated_pn_download_mbps,
        config.queues.generated_pn_upload_mbps,
    )
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
        let Some(device_details) = uisp_data
            .devices
            .iter()
            .find(|d| d.id == device.identification.id)
        else {
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
        let _ = graph.add_edge(
            device_ref,
            *site_ref,
            LinkMapping::ethernet(config.queues.generated_pn_download_mbps),
        );
        let _ = graph.add_edge(
            *site_ref,
            device_ref,
            LinkMapping::ethernet(config.queues.generated_pn_upload_mbps),
        );
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
        LinkMapping::DevicePair {
            speed_mbps,
            device_a,
            device_b,
        } => {
            // Check for overrides: make sure we have both devices (we need names, are storing IDs)
            let Some(device_a) = devices.iter().find(|d| d.id == *device_a) else {
                return *speed_mbps;
            };
            let Some(device_b) = devices.iter().find(|d| d.id == *device_b) else {
                return *speed_mbps;
            };

            // Check for the direct direction
            if let Some(route_override) = route_overrides.iter().find(|route_override| {
                (route_override.from_site == device_a.name
                    || route_override.to_site == device_a.name)
                    && (route_override.from_site == device_b.name
                        || route_override.to_site == device_b.name)
            }) {
                return route_override.cost as u64;
            }

            // Check for the reverse direction
            if let Some(route_override) = route_overrides.iter().find(|route_override| {
                (route_override.from_site == device_b.name
                    || route_override.to_site == device_b.name)
                    && (route_override.from_site == device_a.name
                        || route_override.to_site == device_a.name)
            }) {
                return route_override.cost as u64;
            }

            *speed_mbps
        }
    }
}

use crate::strategies::full::bandwidth_overrides::BandwidthOverrides;
use petgraph::prelude::*;

fn edges_from_node_path(graph: &GraphType, path: &[NodeIndex]) -> Vec<EdgeIndex> {
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

fn min_capacity_along_route(graph: &GraphType, path: &[NodeIndex]) -> u64 {
    edges_from_node_path(graph, path)
        .iter()
        .map(|edge| graph[*edge].capacity_mbps())
        .min()
        .unwrap_or(0)
}

#[derive(Debug)]
struct SquashCandidate {
    endpoint_a: NodeIndex,
    endpoint_a_name: String,
    relay_a: NodeIndex,
    relay_a_name: String,
    relay_b: NodeIndex,
    relay_b_name: String,
    endpoint_b: NodeIndex,
    endpoint_b_name: String,
}

fn find_point_to_point_squash_candidates(
    graph: &mut GraphType,
    aps_with_clients: &HashSet<String>,
    config: &Arc<Config>,
) {
    let mut candidates = Vec::new();

    // Find all nodes with exactly total degree 4 in bidirectional graph (2 unique neighbors)
    // This accounts for bidirectional edges: A->B and B->A both exist
    let relay_nodes: Vec<NodeIndex> = graph
        .node_indices()
        .filter(|&node| {
            let incoming = graph.neighbors_directed(node, petgraph::Incoming).count();
            let outgoing = graph.neighbors_directed(node, petgraph::Outgoing).count();
            let total_degree = incoming + outgoing;

            // In bidirectional graph, relay nodes have degree 4 (2 in, 2 out)
            // but only 2 unique neighbors
            if total_degree == 4 {
                let mut unique_neighbors = std::collections::HashSet::new();
                for neighbor in graph.neighbors_directed(node, petgraph::Incoming) {
                    unique_neighbors.insert(neighbor);
                }
                for neighbor in graph.neighbors_directed(node, petgraph::Outgoing) {
                    unique_neighbors.insert(neighbor);
                }
                unique_neighbors.len() == 2
            } else {
                false
            }
        })
        .collect();

    // For each potential relay node, check if it's part of a 2-relay chain
    for &relay_node in &relay_nodes {
        // Skip if this relay node is an AP with clients
        if let GraphMapping::AccessPoint { id, .. } = &graph[relay_node] {
            if aps_with_clients.contains(id) {
                continue;
            }
        }
        // Get the unique neighbors for this relay node
        let mut unique_neighbors = std::collections::HashSet::new();
        for neighbor in graph.neighbors_directed(relay_node, petgraph::Incoming) {
            unique_neighbors.insert(neighbor);
        }
        for neighbor in graph.neighbors_directed(relay_node, petgraph::Outgoing) {
            unique_neighbors.insert(neighbor);
        }

        if unique_neighbors.len() != 2 {
            continue;
        }

        let neighbors: Vec<NodeIndex> = unique_neighbors.into_iter().collect();
        let node_a = neighbors[0];
        let node_b = neighbors[1];

        // Check if each neighbor is a relay or endpoint in bidirectional context
        let is_node_a_relay = {
            let mut a_neighbors = std::collections::HashSet::new();
            for neighbor in graph.neighbors_directed(node_a, petgraph::Incoming) {
                a_neighbors.insert(neighbor);
            }
            for neighbor in graph.neighbors_directed(node_a, petgraph::Outgoing) {
                a_neighbors.insert(neighbor);
            }
            let a_incoming = graph.neighbors_directed(node_a, petgraph::Incoming).count();
            let a_outgoing = graph.neighbors_directed(node_a, petgraph::Outgoing).count();
            (a_incoming + a_outgoing) == 4 && a_neighbors.len() == 2
        };

        let is_node_b_relay = {
            let mut b_neighbors = std::collections::HashSet::new();
            for neighbor in graph.neighbors_directed(node_b, petgraph::Incoming) {
                b_neighbors.insert(neighbor);
            }
            for neighbor in graph.neighbors_directed(node_b, petgraph::Outgoing) {
                b_neighbors.insert(neighbor);
            }
            let b_incoming = graph.neighbors_directed(node_b, petgraph::Incoming).count();
            let b_outgoing = graph.neighbors_directed(node_b, petgraph::Outgoing).count();
            (b_incoming + b_outgoing) == 4 && b_neighbors.len() == 2
        };

        // We want exactly one of the neighbors to be a relay and one to be an endpoint
        // Case 1: node_a is endpoint, node_b is relay
        if !is_node_a_relay && is_node_b_relay {
            // First check that node_a is a meaningful endpoint
            let mut node_a_neighbors = std::collections::HashSet::new();
            for neighbor in graph.neighbors_directed(node_a, petgraph::Incoming) {
                node_a_neighbors.insert(neighbor);
            }
            for neighbor in graph.neighbors_directed(node_a, petgraph::Outgoing) {
                node_a_neighbors.insert(neighbor);
            }
            let is_node_a_meaningful = node_a_neighbors.len() >= 3
                && !matches!(graph[node_a], GraphMapping::AccessPoint { .. });

            if !is_node_a_meaningful {
                continue;
            }

            // Also verify that node_b is a pure relay (exactly 2 unique neighbors, no more)
            let mut node_b_neighbors = std::collections::HashSet::new();
            for neighbor in graph.neighbors_directed(node_b, petgraph::Incoming) {
                node_b_neighbors.insert(neighbor);
            }
            for neighbor in graph.neighbors_directed(node_b, petgraph::Outgoing) {
                node_b_neighbors.insert(neighbor);
            }
            if node_b_neighbors.len() != 2 {
                continue;
            }

            // Check if node_b (relay) is an AP with clients - if so, skip
            if let GraphMapping::AccessPoint { id, .. } = &graph[node_b] {
                if aps_with_clients.contains(id) {
                    continue;
                }
            }

            // Find the other neighbor of node_b (the endpoint on the far side)
            let mut b_neighbors = std::collections::HashSet::new();
            for neighbor in graph.neighbors_directed(node_b, petgraph::Incoming) {
                b_neighbors.insert(neighbor);
            }
            for neighbor in graph.neighbors_directed(node_b, petgraph::Outgoing) {
                b_neighbors.insert(neighbor);
            }
            b_neighbors.remove(&relay_node); // Remove the current relay node

            if b_neighbors.len() == 1 {
                let endpoint_b = *b_neighbors.iter().next().unwrap();
                // Verify endpoint_b is not a relay
                let mut endpoint_b_neighbors = std::collections::HashSet::new();
                for neighbor in graph.neighbors_directed(endpoint_b, petgraph::Incoming) {
                    endpoint_b_neighbors.insert(neighbor);
                }
                for neighbor in graph.neighbors_directed(endpoint_b, petgraph::Outgoing) {
                    endpoint_b_neighbors.insert(neighbor);
                }
                let endpoint_b_degree = graph
                    .neighbors_directed(endpoint_b, petgraph::Incoming)
                    .count()
                    + graph
                        .neighbors_directed(endpoint_b, petgraph::Outgoing)
                        .count();

                // Endpoint must not be a relay AND must have sufficient connections to be meaningful
                // Also prefer Sites over AccessPoints as endpoints
                let is_meaningful_endpoint = !(endpoint_b_degree == 4 && endpoint_b_neighbors.len() == 2)
                        && endpoint_b_neighbors.len() >= 3 // Must connect to at least 3 other nodes
                        && !matches!(graph[endpoint_b], GraphMapping::AccessPoint { .. }); // Avoid APs as endpoints
                if is_meaningful_endpoint {
                    // Check do_not_squash_sites - skip if any node in the chain is in the exclusion list
                    let do_not_squash = &config
                        .uisp_integration
                        .do_not_squash_sites
                        .clone()
                        .unwrap_or_default();
                    let node_names = [
                        &graph[node_a].name(),
                        &graph[relay_node].name(),
                        &graph[node_b].name(),
                        &graph[endpoint_b].name(),
                    ];

                    if node_names.iter().any(|name| do_not_squash.contains(*name)) {
                        continue;
                    }

                    candidates.push(SquashCandidate {
                        endpoint_a: node_a,
                        endpoint_a_name: graph[node_a].name(),
                        relay_a: relay_node,
                        relay_a_name: graph[relay_node].name(),
                        relay_b: node_b,
                        relay_b_name: graph[node_b].name(),
                        endpoint_b,
                        endpoint_b_name: graph[endpoint_b].name(),
                    });
                }
            }
        }

        // Case 2: node_a is relay, node_b is endpoint
        if is_node_a_relay && !is_node_b_relay {
            // First check that node_b is a meaningful endpoint
            let mut node_b_neighbors = std::collections::HashSet::new();
            for neighbor in graph.neighbors_directed(node_b, petgraph::Incoming) {
                node_b_neighbors.insert(neighbor);
            }
            for neighbor in graph.neighbors_directed(node_b, petgraph::Outgoing) {
                node_b_neighbors.insert(neighbor);
            }
            let is_node_b_meaningful = node_b_neighbors.len() >= 3
                && !matches!(graph[node_b], GraphMapping::AccessPoint { .. });

            if !is_node_b_meaningful {
                continue;
            }

            // Also verify that node_a is a pure relay (exactly 2 unique neighbors, no more)
            let mut node_a_neighbors = std::collections::HashSet::new();
            for neighbor in graph.neighbors_directed(node_a, petgraph::Incoming) {
                node_a_neighbors.insert(neighbor);
            }
            for neighbor in graph.neighbors_directed(node_a, petgraph::Outgoing) {
                node_a_neighbors.insert(neighbor);
            }
            if node_a_neighbors.len() != 2 {
                continue;
            }

            // Check if node_a (relay) is an AP with clients - if so, skip
            if let GraphMapping::AccessPoint { id, .. } = &graph[node_a] {
                if aps_with_clients.contains(id) {
                    continue;
                }
            }

            // Find the other neighbor of node_a (the endpoint on the far side)
            let mut a_neighbors = std::collections::HashSet::new();
            for neighbor in graph.neighbors_directed(node_a, petgraph::Incoming) {
                a_neighbors.insert(neighbor);
            }
            for neighbor in graph.neighbors_directed(node_a, petgraph::Outgoing) {
                a_neighbors.insert(neighbor);
            }
            a_neighbors.remove(&relay_node); // Remove the current relay node

            if a_neighbors.len() == 1 {
                let endpoint_a = *a_neighbors.iter().next().unwrap();
                // Verify endpoint_a is not a relay
                let mut endpoint_a_neighbors = std::collections::HashSet::new();
                for neighbor in graph.neighbors_directed(endpoint_a, petgraph::Incoming) {
                    endpoint_a_neighbors.insert(neighbor);
                }
                for neighbor in graph.neighbors_directed(endpoint_a, petgraph::Outgoing) {
                    endpoint_a_neighbors.insert(neighbor);
                }
                let endpoint_a_degree = graph
                    .neighbors_directed(endpoint_a, petgraph::Incoming)
                    .count()
                    + graph
                        .neighbors_directed(endpoint_a, petgraph::Outgoing)
                        .count();

                // Endpoint must not be a relay AND must have sufficient connections to be meaningful
                // Also prefer Sites over AccessPoints as endpoints
                let is_meaningful_endpoint = !(endpoint_a_degree == 4 && endpoint_a_neighbors.len() == 2)
                        && endpoint_a_neighbors.len() >= 3 // Must connect to at least 3 other nodes
                        && !matches!(graph[endpoint_a], GraphMapping::AccessPoint { .. }); // Avoid APs as endpoints
                if is_meaningful_endpoint {
                    // Check do_not_squash_sites - skip if any node in the chain is in the exclusion list
                    let do_not_squash = &config
                        .uisp_integration
                        .do_not_squash_sites
                        .clone()
                        .unwrap_or_default();
                    let node_names = [
                        &graph[endpoint_a].name(),
                        &graph[node_a].name(),
                        &graph[relay_node].name(),
                        &graph[node_b].name(),
                    ];

                    if node_names.iter().any(|name| do_not_squash.contains(*name)) {
                        continue;
                    }

                    candidates.push(SquashCandidate {
                        endpoint_a,
                        endpoint_a_name: graph[endpoint_a].name(),
                        relay_a: node_a,
                        relay_a_name: graph[node_a].name(),
                        relay_b: relay_node,
                        relay_b_name: graph[relay_node].name(),
                        endpoint_b: node_b,
                        endpoint_b_name: graph[node_b].name(),
                    });
                }
            }
        }
    }

    // Remove duplicates (same chain detected from both relay nodes)
    candidates.dedup_by(|a, b| {
        (a.endpoint_a == b.endpoint_a && a.endpoint_b == b.endpoint_b)
            || (a.endpoint_a == b.endpoint_b && a.endpoint_b == b.endpoint_a)
    });

    info!(
        "Found {} point-to-point squash candidates:",
        candidates.len()
    );
    for candidate in &candidates {
        info!(
            "  {} -> {} -> {} -> {}",
            candidate.endpoint_a_name,
            candidate.relay_a_name,
            candidate.relay_b_name,
            candidate.endpoint_b_name
        );
    }

    // Perform the actual squashing
    perform_squashing(graph, &candidates);
}

fn perform_squashing(graph: &mut GraphType, candidates: &[SquashCandidate]) {
    let mut nodes_to_remove = std::collections::HashSet::new();

    info!("Performing squashing on {} candidates...", candidates.len());

    for candidate in candidates {
        info!(
            "Squashing: {} -> {} -> {} -> {}",
            candidate.endpoint_a_name,
            candidate.relay_a_name,
            candidate.relay_b_name,
            candidate.endpoint_b_name
        );

        // Calculate the minimum capacity along the original chain in both directions
        let chain_nodes = [
            candidate.endpoint_a,
            candidate.relay_a,
            candidate.relay_b,
            candidate.endpoint_b,
        ];
        let (forward_capacity, reverse_capacity) = calculate_chain_capacity(graph, &chain_nodes);

        // Check if endpoints still exist (previous squashing might have removed them)
        if !graph.node_weight(candidate.endpoint_a).is_some()
            || !graph.node_weight(candidate.endpoint_b).is_some()
        {
            info!("  Skipping - endpoints no longer exist");
            continue;
        }

        // Check if there's already a direct edge between endpoints
        if graph
            .find_edge(candidate.endpoint_a, candidate.endpoint_b)
            .is_some()
        {
            info!("  Skipping - direct edge already exists");
            continue;
        }

        // Create direct bidirectional edges between endpoints with proper capacities
        graph.add_edge(
            candidate.endpoint_a,
            candidate.endpoint_b,
            LinkMapping::ethernet(forward_capacity),
        );
        graph.add_edge(
            candidate.endpoint_b,
            candidate.endpoint_a,
            LinkMapping::ethernet(reverse_capacity),
        );

        // Mark relay nodes for removal
        nodes_to_remove.insert(candidate.relay_a);
        nodes_to_remove.insert(candidate.relay_b);

        info!(
            "  Created direct link with {}/{}Mbps capacity (forward/reverse)",
            forward_capacity, reverse_capacity
        );
    }

    // Remove all relay nodes that are no longer needed
    let removed_count = nodes_to_remove.len();
    for node_to_remove in nodes_to_remove {
        if graph.node_weight(node_to_remove).is_some() {
            let node_name = graph[node_to_remove].name();
            graph.remove_node(node_to_remove);
            info!("Removed relay node: {}", node_name);
        }
    }

    info!(
        "Squashing complete: removed {} relay nodes, created {} direct links",
        removed_count,
        candidates.len()
    );
}

fn calculate_chain_capacity(graph: &GraphType, chain_nodes: &[NodeIndex]) -> (u64, u64) {
    let mut min_forward_capacity = u64::MAX;
    let mut min_reverse_capacity = u64::MAX;

    // Check capacity of each edge in both directions along the chain
    for window in chain_nodes.windows(2) {
        let from_node = window[0];
        let to_node = window[1];

        // Forward direction
        if let Some(edge_idx) = graph.find_edge(from_node, to_node) {
            let edge_capacity = graph[edge_idx].capacity_mbps();
            min_forward_capacity = min_forward_capacity.min(edge_capacity);
        }

        // Reverse direction
        if let Some(edge_idx) = graph.find_edge(to_node, from_node) {
            let edge_capacity = graph[edge_idx].capacity_mbps();
            min_reverse_capacity = min_reverse_capacity.min(edge_capacity);
        }
    }

    // Default to 100Mbps if we couldn't determine capacity
    let forward_capacity = if min_forward_capacity == u64::MAX {
        100
    } else {
        min_forward_capacity
    };
    let reverse_capacity = if min_reverse_capacity == u64::MAX {
        100
    } else {
        min_reverse_capacity
    };

    (forward_capacity, reverse_capacity)
}

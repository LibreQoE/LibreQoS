mod directionality;
mod dot;
mod graph_mapping;
mod link_mapping;
mod net_json_parent;

use crate::blackboard_blob;
use crate::errors::UispIntegrationError;
use crate::ethernet_advisory::{apply_ethernet_rate_cap, write_ethernet_advisories};
use crate::ip_ranges::IpRanges;
use crate::strategies::common::UispData;
use crate::strategies::full::bandwidth_overrides::{BandwidthOverride, find_bandwidth_override};
use crate::strategies::full::routes_override::RouteOverride;
use crate::strategies::full::shaped_devices_writer::ShapedDevice;
use crate::strategies::full2::directionality::{
    build_device_capacity_map, build_device_link_meta_map, directed_caps_mbps,
};
use crate::strategies::full2::dot::save_dot_file;
use crate::strategies::full2::graph_mapping::GraphMapping;
use crate::strategies::full2::link_mapping::LinkMapping;
use crate::strategies::full2::net_json_parent::{NetJsonParent, walk_parents};
use crate::uisp_types::UispDevice;
use lqos_config::{
    CircuitEthernetMetadata, Config, EthernetPortLimitPolicy, RequestedCircuitRates,
    TopologyParentCandidate, TopologyParentCandidatesFile, TopologyParentCandidatesNode,
};
use lqos_overrides::TopologyParentOverrideMode;
use petgraph::Directed;
use petgraph::graph::NodeIndex;
use petgraph::visit::{EdgeRef, NodeRef};
use std::collections::{HashMap, HashSet};
use std::fs::write;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

type GraphType = petgraph::Graph<GraphMapping, LinkMapping, Directed>;

#[derive(Clone, Debug)]
struct TopologyParentOverrideSelection {
    mode: TopologyParentOverrideMode,
    parent_node_ids: Vec<String>,
}

pub async fn build_full_network_v2(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    let ethernet_policy = EthernetPortLimitPolicy::from(&config.integration_common);
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
    let topology_parent_overrides = load_topology_parent_overrides();

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
        if let Some(link_count) = ap_link_count.get(ap_id)
            && *link_count > 1
        {
            continue;
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
    let mut topology_parent_candidates = Vec::<TopologyParentCandidatesNode>::new();
    for node in graph.node_indices() {
        if node == root_idx {
            continue;
        }
        match &graph[node] {
            GraphMapping::GeneratedSite { name }
            | GraphMapping::Site { name, .. }
            | GraphMapping::AccessPoint { name, .. } => {
                let mut route_from_root_to_node = astar_route(
                    &graph,
                    root_idx,
                    node,
                    &uisp_data.devices,
                    &routing_overrides,
                );
                let mut route_from_node_to_root = astar_route(
                    &graph,
                    node,
                    root_idx,
                    &uisp_data.devices,
                    &routing_overrides,
                );

                // println!("From node to root:");
                // println!("{:?}", route_from_node_to_root);
                // println!("{:?}", edges_from_node_path(&graph, &route_from_node_to_root.1));
                //
                // println!("From root to node:");
                // println!("{:?}", route_from_root_to_node);
                // println!("{:?}", edges_from_node_path(&graph, &route_from_root_to_node.1));

                if route_from_root_to_node.is_empty() {
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
                    let native_parent = immediate_parent_from_route(&route_from_root_to_node);
                    let candidate_parents = topology_parent_candidates_for_node(
                        &graph,
                        root_idx,
                        node,
                        &uisp_data.devices,
                        &routing_overrides,
                    );
                    let resolved_parent = resolve_parent_candidate(
                        &graph,
                        node,
                        native_parent,
                        &candidate_parents,
                        &topology_parent_overrides,
                    );

                    if let Some(parent_node) = resolved_parent
                        && native_parent != Some(parent_node)
                        && let Some((resolved_from_root, resolved_to_root)) =
                            build_constrained_route(
                                &graph,
                                root_idx,
                                node,
                                parent_node,
                                &uisp_data.devices,
                                &routing_overrides,
                            )
                    {
                        route_from_root_to_node = resolved_from_root;
                        route_from_node_to_root = resolved_to_root;
                    }

                    // Obtain capacities from route traversal
                    let mut download_capacity =
                        min_capacity_along_route(&graph, &route_from_root_to_node);
                    let mut upload_capacity =
                        min_capacity_along_route(&graph, &route_from_node_to_root);

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
                    if !bandwidth_overrides.is_empty()
                        && let Some(bw_override) = find_bandwidth_override(
                            &bandwidth_overrides,
                            Some(&graph[node].network_json_id()),
                            name,
                        )
                    {
                        debug!("Applying bandwidth override for {}", name);
                        debug!("Capacity was: {} / {}", download_capacity, upload_capacity);
                        if let Some(down) = bw_override.download_bandwidth_mbps {
                            download_capacity = down as u64;
                        }
                        if let Some(up) = bw_override.upload_bandwidth_mbps {
                            upload_capacity = up as u64;
                        }
                        debug!(
                            "Capacity is now: {} / {}",
                            download_capacity, upload_capacity
                        );
                    }

                    let Some(parent_node) = immediate_parent_from_route(&route_from_root_to_node)
                    else {
                        continue;
                    };
                    // We need the weight from node to parent_node in the graph edges
                    if let Some(_edge) = graph.find_edge(parent_node, node) {
                        let parent = graph[parent_node].name();
                        let current_parent = if is_real_topology_node(&graph, parent_node) {
                            Some(TopologyParentCandidate {
                                node_id: graph[parent_node].network_json_id(),
                                node_name: parent.clone(),
                            })
                        } else {
                            None
                        };
                        let candidate_parent_rows: Vec<TopologyParentCandidate> = candidate_parents
                            .iter()
                            .copied()
                            .filter(|candidate| is_real_topology_node(&graph, *candidate))
                            .map(|candidate| TopologyParentCandidate {
                                node_id: graph[candidate].network_json_id(),
                                node_name: graph[candidate].name(),
                            })
                            .collect();

                        if is_real_topology_node(&graph, node) {
                            topology_parent_candidates.push(TopologyParentCandidatesNode {
                                node_id: graph[node].network_json_id(),
                                node_name: name.to_owned(),
                                current_parent_node_id: current_parent
                                    .as_ref()
                                    .map(|candidate| candidate.node_id.clone()),
                                current_parent_node_name: current_parent
                                    .as_ref()
                                    .map(|candidate| candidate.node_name.clone()),
                                candidate_parents: candidate_parent_rows,
                            });
                        }
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
            walk_parents(&parents, name, node_info, &mut visited).into(),
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

    topology_parent_candidates.sort_unstable_by(|left, right| left.node_id.cmp(&right.node_id));
    let topology_candidates_file = TopologyParentCandidatesFile {
        source: "uisp/full".to_string(),
        nodes: topology_parent_candidates,
    };
    if let Err(err) = topology_candidates_file.save(&config) {
        warn!("Unable to write topology parent candidates snapshot: {err:?}");
    }

    // Shaped Devices
    let mut shaped_devices = Vec::new();
    let mut seen_pairs = HashSet::new();
    let mut processed_site_pairs = 0usize;
    let mut shaped_device_count = 0usize;
    let mut ethernet_advisories: Vec<CircuitEthernetMetadata> = Vec::new();

    for (ap_id, client_sites) in client_mappings.iter() {
        for site_id in client_sites.iter() {
            let Some(ap_device) = uisp_data.devices.iter().find(|d| d.id == *ap_id) else {
                continue;
            };
            let Some(site) = uisp_data.sites.iter().find(|s| s.id == *site_id) else {
                continue;
            };
            processed_site_pairs = processed_site_pairs.saturating_add(1);
            debug!(
                "Processing site: {} (ID: {}) with AP: {} (ID: {})",
                site.name, site.id, ap_device.name, ap_device.id
            );
            let site_devices: Vec<&UispDevice> = uisp_data
                .devices
                .iter()
                .filter(|d| d.site_id == *site_id && d.has_address())
                .collect();

            let requested =
                if let Some((dl_min, dl_max, ul_min, ul_max)) = site.burst_rates(&config) {
                    (
                        f32::max(0.1, dl_min),
                        f32::max(0.1, dl_max),
                        f32::max(0.1, ul_min),
                        f32::max(0.1, ul_max),
                    )
                } else if site.suspended && config.uisp_integration.suspended_strategy == "slow" {
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
            let ethernet_decision = apply_ethernet_rate_cap(
                ethernet_policy,
                &site.id,
                &site.name,
                site_devices.iter().copied(),
                RequestedCircuitRates {
                    download_min: requested.0,
                    upload_min: requested.2,
                    download_max: requested.1,
                    upload_max: requested.3,
                },
            );
            if let Some(advisory) = ethernet_decision.advisory.clone() {
                ethernet_advisories.push(advisory);
            }
            for device in uisp_data.devices.iter().filter(|d| d.site_id == *site_id) {
                if !device.has_address() {
                    continue;
                }

                let parent_node = {
                    if parents.contains_key(&ap_device.name) {
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
                    download_min: ethernet_decision.download_min,
                    upload_min: ethernet_decision.upload_min,
                    download_max: ethernet_decision.download_max,
                    upload_max: ethernet_decision.upload_max,
                    comment: "".to_string(),
                };
                debug!(
                    "Created shaped device for '{}' in site '{}' with parent '{}'",
                    device.name, site.name, parent_node
                );
                shaped_device_count = shaped_device_count.saturating_add(1);
                shaped_devices.push(shaped_device);
            }
        }
    }
    info!(
        "UISP shaped devices: processed {} site/AP pair(s) and produced {} shaped device row(s).",
        processed_site_pairs, shaped_device_count
    );
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
    write_ethernet_advisories(&config, &ethernet_advisories)?;
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
            && let Some(dev_b) = uisp_data
                .devices_raw
                .iter()
                .find(|d| d.get_id() == to_device.identification.id)
            && dev_a.get_site_id().unwrap_or_default() == dev_b.get_site_id().unwrap_or_default()
        {
            // If the devices are in the same site, we don't need to add an edge
            continue;
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
        let (cap_ab, cap_ba) = if let Some((cap_ab, cap_ba)) =
            directed_caps_mbps(&meta_by_id, &caps_by_id, config, id_a, id_b)
        {
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
    bandwidth_overrides: &[BandwidthOverride],
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

        if let Some(bw_override) = find_bandwidth_override(
            bandwidth_overrides,
            Some(&format!("uisp:device:{}", device.identification.id)),
            &device.get_name().unwrap_or_default(),
        ) {
            if let Some(down) = bw_override.download_bandwidth_mbps {
                download_mbps = down as u64;
            }
            if let Some(up) = bw_override.upload_bandwidth_mbps {
                upload_mbps = up as u64;
            }
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
                latitude: site
                    .description
                    .as_ref()
                    .and_then(|description| description.location.as_ref())
                    .map(|location| location.latitude as f32),
                longitude: site
                    .description
                    .as_ref()
                    .and_then(|description| description.location.as_ref())
                    .map(|location| location.longitude as f32),
            };
            let root_ref = graph.add_node(root_entry);
            *root_idx = Some(root_ref);
            site_map.insert(site.id.clone(), root_ref);
            continue;
        }
        let site_entry = GraphMapping::Site {
            name: site_name,
            id,
            latitude: site
                .description
                .as_ref()
                .and_then(|description| description.location.as_ref())
                .map(|location| location.latitude as f32),
            longitude: site
                .description
                .as_ref()
                .and_then(|description| description.location.as_ref())
                .map(|location| location.longitude as f32),
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

use petgraph::prelude::*;

fn astar_route(
    graph: &GraphType,
    start: NodeIndex,
    end: NodeIndex,
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> Vec<NodeIndex> {
    petgraph::algo::astar(
        graph,
        start,
        |n| n == end,
        |e| {
            (10_000u64).saturating_sub(link_capacity_mbps_for_routing(
                e.weight(),
                devices,
                route_overrides,
            ))
        },
        |_| 0,
    )
    .map(|(_, path)| path)
    .unwrap_or_default()
}

fn load_topology_parent_overrides() -> HashMap<String, TopologyParentOverrideSelection> {
    let Ok(overrides) = lqos_overrides::OverrideFile::load() else {
        warn!("Unable to load operator topology parent overrides from lqos_overrides.json");
        return HashMap::new();
    };

    overrides
        .network_adjustments()
        .iter()
        .filter_map(|adj| match adj {
            lqos_overrides::NetworkAdjustment::TopologyParentOverride {
                node_id,
                mode,
                parent_node_ids,
                ..
            } => Some((
                node_id.clone(),
                TopologyParentOverrideSelection {
                    mode: *mode,
                    parent_node_ids: parent_node_ids.clone(),
                },
            )),
            _ => None,
        })
        .collect()
}

fn is_real_topology_node(graph: &GraphType, node: NodeIndex) -> bool {
    matches!(
        graph[node],
        GraphMapping::Root { .. } | GraphMapping::Site { .. } | GraphMapping::AccessPoint { .. }
    )
}

fn topology_parent_candidates_for_node(
    graph: &GraphType,
    root_idx: NodeIndex,
    node: NodeIndex,
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> Vec<NodeIndex> {
    let mut candidates: Vec<NodeIndex> = graph
        .neighbors_directed(node, petgraph::Incoming)
        .filter(|candidate| is_real_topology_node(graph, *candidate))
        .filter(|candidate| {
            !astar_route(graph, root_idx, *candidate, devices, route_overrides).is_empty()
        })
        .collect();
    candidates.sort_unstable_by_key(|candidate| stable_node_key(graph, *candidate));
    candidates.dedup();
    candidates
}

fn immediate_parent_from_route(path: &[NodeIndex]) -> Option<NodeIndex> {
    if path.len() < 2 {
        None
    } else {
        path.get(path.len().saturating_sub(2)).copied()
    }
}

fn resolve_parent_candidate(
    graph: &GraphType,
    node: NodeIndex,
    native_parent: Option<NodeIndex>,
    candidates: &[NodeIndex],
    overrides: &HashMap<String, TopologyParentOverrideSelection>,
) -> Option<NodeIndex> {
    let node_id = graph[node].network_json_id();
    let Some(override_entry) = overrides.get(&node_id) else {
        return native_parent.or_else(|| candidates.first().copied());
    };

    match override_entry.mode {
        TopologyParentOverrideMode::Pinned | TopologyParentOverrideMode::PreferredOrder => {
            if override_entry.mode == TopologyParentOverrideMode::PreferredOrder {
                warn!(
                    node = %graph[node].name(),
                    node_id = %node_id,
                    "Legacy preferred-upstream topology override detected; using the first saved parent as a pinned parent"
                );
            }
            let chosen = override_entry
                .parent_node_ids
                .first()
                .and_then(|desired_id| {
                    candidates
                        .iter()
                        .copied()
                        .find(|candidate| graph[*candidate].network_json_id() == *desired_id)
                });
            if chosen.is_none() {
                warn!(
                    node = %graph[node].name(),
                    node_id = %node_id,
                    "Pinned topology parent override is stale; falling back to native UISP selection"
                );
            }
            chosen
                .or(native_parent)
                .or_else(|| candidates.first().copied())
        }
    }
}

fn build_constrained_route(
    graph: &GraphType,
    root_idx: NodeIndex,
    node: NodeIndex,
    parent: NodeIndex,
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> Option<(Vec<NodeIndex>, Vec<NodeIndex>)> {
    if graph.find_edge(parent, node).is_none() || graph.find_edge(node, parent).is_none() {
        return None;
    }

    let mut from_root = astar_route(graph, root_idx, parent, devices, route_overrides);
    let mut to_root = astar_route(graph, parent, root_idx, devices, route_overrides);
    if from_root.is_empty() || to_root.is_empty() {
        return None;
    }

    from_root.push(node);
    let mut from_node = vec![node];
    from_node.append(&mut to_root);
    Some((from_root, from_node))
}

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
    endpoint_a_id: String,
    endpoint_a_name: String,
    relay_a: NodeIndex,
    relay_a_id: String,
    relay_a_name: String,
    relay_b: NodeIndex,
    relay_b_id: String,
    relay_b_name: String,
    endpoint_b: NodeIndex,
    endpoint_b_id: String,
    endpoint_b_name: String,
}

impl SquashCandidate {
    fn new(
        graph: &GraphType,
        endpoint_a: NodeIndex,
        relay_a: NodeIndex,
        relay_b: NodeIndex,
        endpoint_b: NodeIndex,
    ) -> Self {
        let candidate = Self {
            endpoint_a,
            endpoint_a_id: graph[endpoint_a].network_json_id(),
            endpoint_a_name: graph[endpoint_a].name(),
            relay_a,
            relay_a_id: graph[relay_a].network_json_id(),
            relay_a_name: graph[relay_a].name(),
            relay_b,
            relay_b_id: graph[relay_b].network_json_id(),
            relay_b_name: graph[relay_b].name(),
            endpoint_b,
            endpoint_b_id: graph[endpoint_b].network_json_id(),
            endpoint_b_name: graph[endpoint_b].name(),
        };

        candidate.canonicalized()
    }

    fn canonicalized(self) -> Self {
        if self.reverse_key() < self.forward_key() {
            self.reversed()
        } else {
            self
        }
    }

    fn canonical_key(&self) -> (String, String, String, String) {
        let forward = self.forward_key();
        let reverse = self.reverse_key();
        if reverse < forward { reverse } else { forward }
    }

    fn forward_key(&self) -> (String, String, String, String) {
        (
            self.endpoint_a_id.clone(),
            self.relay_a_id.clone(),
            self.relay_b_id.clone(),
            self.endpoint_b_id.clone(),
        )
    }

    fn reverse_key(&self) -> (String, String, String, String) {
        (
            self.endpoint_b_id.clone(),
            self.relay_b_id.clone(),
            self.relay_a_id.clone(),
            self.endpoint_a_id.clone(),
        )
    }

    fn reversed(self) -> Self {
        Self {
            endpoint_a: self.endpoint_b,
            endpoint_a_id: self.endpoint_b_id,
            endpoint_a_name: self.endpoint_b_name,
            relay_a: self.relay_b,
            relay_a_id: self.relay_b_id,
            relay_a_name: self.relay_b_name,
            relay_b: self.relay_a,
            relay_b_id: self.relay_a_id,
            relay_b_name: self.relay_a_name,
            endpoint_b: self.endpoint_a,
            endpoint_b_id: self.endpoint_a_id,
            endpoint_b_name: self.endpoint_a_name,
        }
    }
}

fn stable_node_key(graph: &GraphType, node: NodeIndex) -> (String, String) {
    (graph[node].network_json_id(), graph[node].name())
}

fn sorted_unique_neighbors(graph: &GraphType, node: NodeIndex) -> Vec<NodeIndex> {
    let mut neighbors: Vec<NodeIndex> = graph
        .neighbors_directed(node, petgraph::Incoming)
        .chain(graph.neighbors_directed(node, petgraph::Outgoing))
        .collect();
    neighbors.sort_unstable_by_key(|neighbor| stable_node_key(graph, *neighbor));
    neighbors.dedup();
    neighbors
}

#[cfg(test)]
mod topology_override_tests {
    use super::{
        GraphType, TopologyParentOverrideSelection, resolve_parent_candidate,
        topology_parent_candidates_for_node,
    };
    use crate::strategies::full2::graph_mapping::GraphMapping;
    use crate::strategies::full2::link_mapping::LinkMapping;
    use lqos_overrides::TopologyParentOverrideMode;
    use std::collections::HashMap;

    fn site(name: &str, id: &str) -> GraphMapping {
        GraphMapping::Site {
            name: name.to_string(),
            id: id.to_string(),
            latitude: None,
            longitude: None,
        }
    }

    fn build_test_graph() -> (
        GraphType,
        petgraph::graph::NodeIndex,
        petgraph::graph::NodeIndex,
        petgraph::graph::NodeIndex,
        petgraph::graph::NodeIndex,
    ) {
        let mut graph = GraphType::new();
        let root = graph.add_node(GraphMapping::Root {
            name: "Upstream".to_string(),
            id: "root".to_string(),
            latitude: None,
            longitude: None,
        });
        let t1 = graph.add_node(site("T1", "t1"));
        let t2 = graph.add_node(site("T2", "t2"));
        let t3 = graph.add_node(site("T3", "t3"));

        for (from, to) in [
            (root, t1),
            (t1, root),
            (root, t3),
            (t3, root),
            (t1, t2),
            (t2, t1),
            (t3, t2),
            (t2, t3),
            (root, t2),
            (t2, root),
        ] {
            graph.add_edge(from, to, LinkMapping::ethernet(1_000));
        }

        (graph, root, t1, t2, t3)
    }

    #[test]
    fn legacy_preferred_override_uses_first_saved_parent() {
        let (graph, root, t1, t2, t3) = build_test_graph();
        let candidates = topology_parent_candidates_for_node(&graph, root, t2, &[], &[]);
        let native_parent = Some(t1);
        let mut overrides = HashMap::new();
        overrides.insert(
            graph[t2].network_json_id(),
            TopologyParentOverrideSelection {
                mode: TopologyParentOverrideMode::PreferredOrder,
                parent_node_ids: vec![graph[t3].network_json_id(), graph[t1].network_json_id()],
            },
        );

        let resolved = resolve_parent_candidate(&graph, t2, native_parent, &candidates, &overrides);
        assert_eq!(resolved, Some(t3));
    }

    #[test]
    fn stale_override_falls_back_to_native_parent() {
        let (graph, root, t1, t2, _t3) = build_test_graph();
        let candidates = topology_parent_candidates_for_node(&graph, root, t2, &[], &[]);
        let native_parent = Some(t1);
        let mut overrides = HashMap::new();
        overrides.insert(
            graph[t2].network_json_id(),
            TopologyParentOverrideSelection {
                mode: TopologyParentOverrideMode::Pinned,
                parent_node_ids: vec!["uisp:site:missing".to_string()],
            },
        );

        let resolved = resolve_parent_candidate(&graph, t2, native_parent, &candidates, &overrides);
        assert_eq!(resolved, Some(t1));
    }
}

fn total_degree(graph: &GraphType, node: NodeIndex) -> usize {
    graph.neighbors_directed(node, petgraph::Incoming).count()
        + graph.neighbors_directed(node, petgraph::Outgoing).count()
}

fn is_relay_node(graph: &GraphType, node: NodeIndex) -> bool {
    total_degree(graph, node) == 4 && sorted_unique_neighbors(graph, node).len() == 2
}

fn is_meaningful_endpoint(graph: &GraphType, node: NodeIndex) -> bool {
    let unique_neighbors = sorted_unique_neighbors(graph, node);
    !(total_degree(graph, node) == 4 && unique_neighbors.len() == 2)
        && unique_neighbors.len() >= 3
        && !matches!(graph[node], GraphMapping::AccessPoint { .. })
}

fn should_skip_squash(
    graph: &GraphType,
    config: &Arc<Config>,
    chain_nodes: [NodeIndex; 4],
) -> bool {
    let do_not_squash = config
        .uisp_integration
        .do_not_squash_sites
        .clone()
        .unwrap_or_default();

    chain_nodes
        .into_iter()
        .map(|node| graph[node].name())
        .any(|name| do_not_squash.contains(&name))
}

fn find_point_to_point_squash_candidates(
    graph: &mut GraphType,
    aps_with_clients: &HashSet<String>,
    config: &Arc<Config>,
) {
    let candidates = collect_point_to_point_squash_candidates(graph, aps_with_clients, config);

    info!(
        "Found {} point-to-point squash candidates:",
        candidates.len()
    );
    for candidate in &candidates {
        debug!(
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

fn collect_point_to_point_squash_candidates(
    graph: &GraphType,
    aps_with_clients: &HashSet<String>,
    config: &Arc<Config>,
) -> Vec<SquashCandidate> {
    let mut candidates = Vec::new();

    // Find all nodes with exactly total degree 4 in bidirectional graph (2 unique neighbors)
    // This accounts for bidirectional edges: A->B and B->A both exist
    let mut relay_nodes: Vec<NodeIndex> = graph
        .node_indices()
        .filter(|&node| is_relay_node(graph, node))
        .collect();
    relay_nodes.sort_unstable_by_key(|node| stable_node_key(graph, *node));

    // For each potential relay node, check if it's part of a 2-relay chain
    for &relay_node in &relay_nodes {
        // Skip if this relay node is an AP with clients
        if let GraphMapping::AccessPoint { id, .. } = &graph[relay_node]
            && aps_with_clients.contains(id)
        {
            continue;
        }
        let neighbors = sorted_unique_neighbors(graph, relay_node);
        if neighbors.len() != 2 {
            continue;
        }
        let node_a = neighbors[0];
        let node_b = neighbors[1];

        // Check if each neighbor is a relay or endpoint in bidirectional context
        let is_node_a_relay = is_relay_node(graph, node_a);
        let is_node_b_relay = is_relay_node(graph, node_b);

        // We want exactly one of the neighbors to be a relay and one to be an endpoint
        // Case 1: node_a is endpoint, node_b is relay
        if !is_node_a_relay && is_node_b_relay {
            if !is_meaningful_endpoint(graph, node_a) {
                continue;
            }

            if sorted_unique_neighbors(graph, node_b).len() != 2 {
                continue;
            }

            // Check if node_b (relay) is an AP with clients - if so, skip
            if let GraphMapping::AccessPoint { id, .. } = &graph[node_b]
                && aps_with_clients.contains(id)
            {
                continue;
            }

            // Find the other neighbor of node_b (the endpoint on the far side)
            let candidate_neighbors: Vec<NodeIndex> = sorted_unique_neighbors(graph, node_b)
                .into_iter()
                .filter(|neighbor| *neighbor != relay_node)
                .collect();

            if candidate_neighbors.len() == 1 {
                let endpoint_b = candidate_neighbors[0];
                if is_meaningful_endpoint(graph, endpoint_b)
                    && !should_skip_squash(graph, config, [node_a, relay_node, node_b, endpoint_b])
                {
                    candidates.push(SquashCandidate::new(
                        graph, node_a, relay_node, node_b, endpoint_b,
                    ));
                }
            }
        }

        // Case 2: node_a is relay, node_b is endpoint
        if is_node_a_relay && !is_node_b_relay {
            if !is_meaningful_endpoint(graph, node_b) {
                continue;
            }

            if sorted_unique_neighbors(graph, node_a).len() != 2 {
                continue;
            }

            // Check if node_a (relay) is an AP with clients - if so, skip
            if let GraphMapping::AccessPoint { id, .. } = &graph[node_a]
                && aps_with_clients.contains(id)
            {
                continue;
            }

            // Find the other neighbor of node_a (the endpoint on the far side)
            let candidate_neighbors: Vec<NodeIndex> = sorted_unique_neighbors(graph, node_a)
                .into_iter()
                .filter(|neighbor| *neighbor != relay_node)
                .collect();

            if candidate_neighbors.len() == 1 {
                let endpoint_a = candidate_neighbors[0];
                if is_meaningful_endpoint(graph, endpoint_a)
                    && !should_skip_squash(graph, config, [endpoint_a, node_a, relay_node, node_b])
                {
                    candidates.push(SquashCandidate::new(
                        graph, endpoint_a, node_a, relay_node, node_b,
                    ));
                }
            }
        }
    }

    candidates.sort_unstable_by_key(SquashCandidate::canonical_key);
    candidates.dedup_by(|a, b| a.canonical_key() == b.canonical_key());
    candidates
}

fn perform_squashing(graph: &mut GraphType, candidates: &[SquashCandidate]) {
    let mut nodes_to_remove = std::collections::HashSet::new();

    info!("Performing squashing on {} candidates...", candidates.len());

    for candidate in candidates {
        debug!(
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
        if graph.node_weight(candidate.endpoint_a).is_none()
            || graph.node_weight(candidate.endpoint_b).is_none()
        {
            debug!("  Skipping - endpoints no longer exist");
            continue;
        }

        // Check if there's already a direct edge between endpoints
        if graph
            .find_edge(candidate.endpoint_a, candidate.endpoint_b)
            .is_some()
        {
            debug!("  Skipping - direct edge already exists");
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

        debug!(
            "  Created direct link with {}/{}Mbps capacity (forward/reverse)",
            forward_capacity, reverse_capacity
        );
    }

    // Remove all relay nodes that are no longer needed
    let removed_count = nodes_to_remove.len();
    let mut nodes_to_remove: Vec<NodeIndex> = nodes_to_remove.into_iter().collect();
    nodes_to_remove.sort_unstable_by_key(|node| std::cmp::Reverse(node.index()));
    for node_to_remove in nodes_to_remove {
        if graph.node_weight(node_to_remove).is_some() {
            let node_name = graph[node_to_remove].name();
            graph.remove_node(node_to_remove);
            debug!("Removed relay node: {}", node_name);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn add_site(graph: &mut GraphType, name: &str, id: &str) -> NodeIndex {
        graph.add_node(GraphMapping::Site {
            name: name.to_string(),
            id: id.to_string(),
            latitude: None,
            longitude: None,
        })
    }

    fn add_ap(graph: &mut GraphType, name: &str, id: &str, site_name: &str) -> NodeIndex {
        graph.add_node(GraphMapping::AccessPoint {
            name: name.to_string(),
            id: id.to_string(),
            site_name: site_name.to_string(),
            download_mbps: 1000,
            upload_mbps: 1000,
        })
    }

    fn add_bidirectional_edge(graph: &mut GraphType, a: NodeIndex, b: NodeIndex, speed_mbps: u64) {
        graph.add_edge(a, b, LinkMapping::ethernet(speed_mbps));
        graph.add_edge(b, a, LinkMapping::ethernet(speed_mbps));
    }

    fn build_point_to_point_graph(reverse_insertion: bool) -> GraphType {
        let mut graph = GraphType::new();

        let mut add_node = |label: &str| match label {
            "left_endpoint" => add_site(&mut graph, "Left Endpoint", "site-left"),
            "right_endpoint" => add_site(&mut graph, "Right Endpoint", "site-right"),
            "left_extra_a" => add_site(&mut graph, "Left Extra A", "site-left-extra-a"),
            "left_extra_b" => add_site(&mut graph, "Left Extra B", "site-left-extra-b"),
            "right_extra_a" => add_site(&mut graph, "Right Extra A", "site-right-extra-a"),
            "right_extra_b" => add_site(&mut graph, "Right Extra B", "site-right-extra-b"),
            "relay_a" => add_ap(&mut graph, "Relay A", "device-relay-a", "Left Endpoint"),
            "relay_b" => add_ap(&mut graph, "Relay B", "device-relay-b", "Right Endpoint"),
            _ => unreachable!("unknown node label"),
        };

        let ordered_labels = if reverse_insertion {
            vec![
                "right_extra_b",
                "right_extra_a",
                "relay_b",
                "right_endpoint",
                "left_extra_b",
                "left_extra_a",
                "relay_a",
                "left_endpoint",
            ]
        } else {
            vec![
                "left_endpoint",
                "relay_a",
                "left_extra_a",
                "left_extra_b",
                "right_endpoint",
                "relay_b",
                "right_extra_a",
                "right_extra_b",
            ]
        };

        let mut nodes = HashMap::new();
        for label in ordered_labels {
            nodes.insert(label, add_node(label));
        }

        let mut edges = vec![
            ("left_endpoint", "relay_a", 500),
            ("relay_a", "relay_b", 400),
            ("relay_b", "right_endpoint", 300),
            ("left_endpoint", "left_extra_a", 100),
            ("left_endpoint", "left_extra_b", 100),
            ("right_endpoint", "right_extra_a", 100),
            ("right_endpoint", "right_extra_b", 100),
        ];
        if reverse_insertion {
            edges.reverse();
        }

        for (from, to, speed) in edges {
            add_bidirectional_edge(&mut graph, nodes[from], nodes[to], speed);
        }

        graph
    }

    fn graph_node_ids(graph: &GraphType) -> Vec<String> {
        let mut ids: Vec<String> = graph
            .node_indices()
            .map(|node| graph[node].network_json_id())
            .collect();
        ids.sort();
        ids
    }

    fn find_node_by_id(graph: &GraphType, id: &str) -> NodeIndex {
        graph
            .node_indices()
            .find(|node| graph[*node].network_json_id() == id)
            .expect("expected node to exist")
    }

    #[test]
    fn squash_candidate_collection_is_deterministic() {
        let config = Arc::new(Config::default());
        let aps_with_clients = HashSet::new();

        let graph_a = build_point_to_point_graph(false);
        let graph_b = build_point_to_point_graph(true);

        let candidates_a =
            collect_point_to_point_squash_candidates(&graph_a, &aps_with_clients, &config);
        let candidates_b =
            collect_point_to_point_squash_candidates(&graph_b, &aps_with_clients, &config);

        assert_eq!(candidates_a.len(), 1);
        assert_eq!(candidates_b.len(), 1);
        assert_eq!(
            candidates_a[0].canonical_key(),
            candidates_b[0].canonical_key()
        );
        assert_eq!(candidates_a[0].endpoint_a_id, "uisp:site:site-left");
        assert_eq!(candidates_a[0].relay_a_id, "uisp:device:device-relay-a");
        assert_eq!(candidates_a[0].relay_b_id, "uisp:device:device-relay-b");
        assert_eq!(candidates_a[0].endpoint_b_id, "uisp:site:site-right");
    }

    #[test]
    fn squashing_result_is_stable_across_insertion_order() {
        let config = Arc::new(Config::default());
        let aps_with_clients = HashSet::new();

        let mut graph_a = build_point_to_point_graph(false);
        let mut graph_b = build_point_to_point_graph(true);

        find_point_to_point_squash_candidates(&mut graph_a, &aps_with_clients, &config);
        find_point_to_point_squash_candidates(&mut graph_b, &aps_with_clients, &config);

        assert_eq!(graph_node_ids(&graph_a), graph_node_ids(&graph_b));
        assert!(!graph_node_ids(&graph_a).contains(&"uisp:device:device-relay-a".to_string()));
        assert!(!graph_node_ids(&graph_a).contains(&"uisp:device:device-relay-b".to_string()));

        let left_a = find_node_by_id(&graph_a, "uisp:site:site-left");
        let right_a = find_node_by_id(&graph_a, "uisp:site:site-right");
        let left_b = find_node_by_id(&graph_b, "uisp:site:site-left");
        let right_b = find_node_by_id(&graph_b, "uisp:site:site-right");

        assert!(graph_a.find_edge(left_a, right_a).is_some());
        assert!(graph_a.find_edge(right_a, left_a).is_some());
        assert!(graph_b.find_edge(left_b, right_b).is_some());
        assert!(graph_b.find_edge(right_b, left_b).is_some());
    }
}

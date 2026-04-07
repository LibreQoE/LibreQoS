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
use crate::strategies::full2::net_json_parent::{NetJsonParent, assign_export_names, walk_parents};
use crate::uisp_types::{UispAttachmentRateSource, UispDevice};
use lqos_config::{
    CircuitEthernetMetadata, Config, EthernetPortLimitPolicy, RequestedCircuitRates,
    TOPOLOGY_ATTACHMENT_AUTO_ID, TopologyAllowedParent, TopologyAttachmentOption,
    TopologyAttachmentRateSource, TopologyAttachmentRole, TopologyCanonicalIngressKind,
    TopologyCanonicalStateFile, TopologyEditorNode, TopologyEditorStateFile,
    TopologyParentCandidate, TopologyParentCandidatesFile, TopologyParentCandidatesNode,
};
#[cfg(test)]
use lqos_overrides::{TopologyAttachmentMode, TopologyParentOverrideMode};
use petgraph::Directed;
use petgraph::graph::NodeIndex;
use petgraph::visit::{EdgeRef, NodeRef};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};

type GraphType = petgraph::Graph<GraphMapping, LinkMapping, Directed>;

#[cfg(test)]
#[derive(Clone, Debug)]
struct TopologyParentOverrideSelection {
    mode: TopologyParentOverrideMode,
    parent_node_ids: Vec<String>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct TopologyAttachmentOverrideSelection {
    parent_node_id: String,
    mode: TopologyAttachmentMode,
    attachment_preference_ids: Vec<String>,
}

#[derive(Clone, Debug)]
struct TopologyAllowedParentGroup {
    logical_parent: NodeIndex,
    candidate_nodes: Vec<NodeIndex>,
}

fn atomic_write_string(path: &Path, raw: &str) -> std::io::Result<()> {
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

fn debug_graph_output_enabled() -> bool {
    env::var_os("LIBREQOS_UISP_DEBUG_GRAPH").is_some()
}

pub async fn build_full_network_v2(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    let build_started = Instant::now();
    let ethernet_policy = EthernetPortLimitPolicy::from(&config.integration_common);
    // Fetch the data
    let uisp_data = UispData::fetch_uisp_data(config.clone(), ip_ranges).await?;

    // Report on obvious UISP errors that should be fixed
    let _trouble = crate::strategies::ap_site::find_troublesome_sites(&uisp_data)
        .await
        .map_err(|e| {
            error!("Error finding troublesome sites");
            error!("{e:?}");
            UispIntegrationError::UnknownSiteType
        })?;

    let bandwidth_overrides =
        crate::strategies::full::bandwidth_overrides::get_site_bandwidth_overrides(&config)?;
    let routing_overrides = crate::strategies::full::routes_override::get_route_overrides(&config)?;

    // Create a new graph
    let mut graph = GraphType::new();

    // Find the root
    let root_site_name = config.uisp_integration.site.clone();

    // Add all sites to the graph
    let graph_build_started = Instant::now();
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
    let site_graph_elapsed_ms = graph_build_started.elapsed().as_millis();

    // Iterate all UISP devices and if their parent site is in the graph, add them
    let mut device_map = HashMap::new();
    let device_graph_started = Instant::now();
    add_devices_to_graph(
        &uisp_data,
        &mut graph,
        &mut site_map,
        &mut device_map,
        &config,
        &bandwidth_overrides,
    );
    let device_graph_elapsed_ms = device_graph_started.elapsed().as_millis();

    // Now we iterate all the data links looking for DEVICE linkage
    let device_link_graph_started = Instant::now();
    add_device_links_to_graph(&uisp_data, &mut graph, &mut device_map, &config);
    let device_link_graph_elapsed_ms = device_link_graph_started.elapsed().as_millis();

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
    let client_mapping_started = Instant::now();
    let client_mappings = uisp_data.map_clients_to_aps();
    let client_mapping_elapsed_ms = client_mapping_started.elapsed().as_millis();

    // Find the APs that have clients
    let mut aps_with_clients = HashSet::new();
    for (ap_id, _client_ids) in client_mappings.iter() {
        let Some(ap_device) = uisp_data.find_device_by_id(ap_id) else {
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

    if debug_graph_output_enabled() {
        save_dot_file(&graph)?;
        let _ = blackboard_blob("uisp-graph", vec![graph.clone()]).await;
    }

    info!(
        site_nodes = site_map.len(),
        access_points = device_map.len(),
        graph_nodes = graph.node_count(),
        graph_edges = graph.edge_count(),
        site_graph_elapsed_ms,
        device_graph_elapsed_ms,
        device_link_graph_elapsed_ms,
        client_mapping_elapsed_ms,
        "Built UISP topology graph"
    );

    let mut route_cache = RouteCache::new(&uisp_data.devices, &routing_overrides);
    let export_names = assign_export_names(
        graph
            .node_indices()
            .map(|node| (graph[node].network_json_id(), &graph[node])),
    );
    let root_node_id = graph[root_idx].network_json_id();
    let orphans_node_id = graph[orphans].network_json_id();
    let orphans_node_name = export_names
        .get(&orphans_node_id)
        .cloned()
        .unwrap_or_else(|| graph[orphans].name());

    // Figure out the network.json layers
    let export_started = Instant::now();
    let mut parents = HashMap::<String, NetJsonParent>::new();
    let mut topology_parent_candidates = Vec::<TopologyParentCandidatesNode>::new();
    let mut topology_editor_nodes = Vec::<TopologyEditorNode>::new();
    for node in graph.node_indices() {
        if node == root_idx {
            continue;
        }
        match &graph[node] {
            GraphMapping::GeneratedSite { name }
            | GraphMapping::Site { name, .. }
            | GraphMapping::AccessPoint { name, .. } => {
                let node_id = graph[node].network_json_id();
                let export_name = export_names
                    .get(&node_id)
                    .cloned()
                    .unwrap_or_else(|| name.to_owned());
                let route_from_root_to_node = route_cache.route(&graph, root_idx, node);
                let route_from_node_to_root = route_cache.route(&graph, node, root_idx);

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
                        node_id.clone(),
                        NetJsonParent {
                            node_id: node_id.clone(),
                            node_name: name.to_owned(),
                            export_name: export_name.clone(),
                            parent_id: Some(orphans_node_id.clone()),
                            parent_name: orphans_node_name.clone(),
                            mapping: &graph[node],
                            download: config.queues.generated_pn_download_mbps,
                            upload: config.queues.generated_pn_upload_mbps,
                        },
                    );
                } else {
                    let native_parent = immediate_parent_from_route(&route_from_root_to_node);
                    let candidate_parents = topology_parent_candidates_for_node_cached(
                        &graph,
                        root_idx,
                        node,
                        &mut route_cache,
                    );
                    let grouped_allowed_parents = topology_allowed_parent_groups_for_node_cached(
                        &graph,
                        root_idx,
                        node,
                        &candidate_parents,
                        &mut route_cache,
                    );
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
                        let parent_id = graph[parent_node].network_json_id();
                        let parent = graph[parent_node].name();
                        let parent_export_name = export_names
                            .get(&parent_id)
                            .cloned()
                            .unwrap_or_else(|| parent.clone());
                        let current_parent = if is_real_topology_node(&graph, parent_node) {
                            Some(TopologyParentCandidate {
                                node_id: parent_id.clone(),
                                node_name: parent.clone(),
                            })
                        } else {
                            None
                        };
                        let logical_current_parent = native_parent
                            .map(|candidate| {
                                logical_parent_for_candidate_cached(
                                    &graph,
                                    root_idx,
                                    node,
                                    candidate,
                                    &mut route_cache,
                                )
                            })
                            .filter(|candidate| is_real_topology_node(&graph, *candidate))
                            .map(|candidate| TopologyParentCandidate {
                                node_id: graph[candidate].network_json_id(),
                                node_name: graph[candidate].name(),
                            });
                        let candidate_parent_rows: Vec<TopologyParentCandidate> = candidate_parents
                            .iter()
                            .copied()
                            .filter(|candidate| is_real_topology_node(&graph, *candidate))
                            .map(|candidate| TopologyParentCandidate {
                                node_id: graph[candidate].network_json_id(),
                                node_name: graph[candidate].name(),
                            })
                            .collect();
                        let allowed_parents = topology_allowed_parents_from_groups_cached(
                            &graph,
                            &grouped_allowed_parents,
                            &aps_with_clients,
                            &mut route_cache,
                        );

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
                            topology_editor_nodes.push(TopologyEditorNode {
                                node_id: graph[node].network_json_id(),
                                node_name: name.to_owned(),
                                current_parent_node_id: logical_current_parent
                                    .as_ref()
                                    .map(|candidate| candidate.node_id.clone()),
                                current_parent_node_name: logical_current_parent
                                    .as_ref()
                                    .map(|candidate| candidate.node_name.clone()),
                                current_attachment_id: current_parent
                                    .as_ref()
                                    .map(|candidate| candidate.node_id.clone()),
                                current_attachment_name: current_parent
                                    .as_ref()
                                    .map(|candidate| candidate.node_name.clone()),
                                can_move: !allowed_parents.is_empty(),
                                allowed_parents,
                                preferred_attachment_id: None,
                                preferred_attachment_name: None,
                                effective_attachment_id: None,
                                effective_attachment_name: None,
                            });
                        }
                        parents.insert(
                            node_id.clone(),
                            NetJsonParent {
                                node_id: node_id.clone(),
                                node_name: name.to_owned(),
                                export_name: export_name.clone(),
                                parent_id: Some(parent_id),
                                parent_name: parent_export_name,
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
    let mut root_child_ids: Vec<&String> = parents
        .iter()
        .filter(|(_node_id, parent)| {
            parent.parent_id.as_deref() == Some(root_node_id.as_str())
                || promote_to_root_set.contains(parent.node_name.as_str())
        })
        .map(|(node_id, _node_info)| node_id)
        .collect();
    root_child_ids.sort_unstable_by(|left_id, right_id| {
        let left = parents
            .get(*left_id)
            .expect("top-level node id should exist when sorting");
        let right = parents
            .get(*right_id)
            .expect("top-level node id should exist when sorting");
        left.export_name
            .cmp(&right.export_name)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    for node_id in root_child_ids {
        let node_info = parents
            .get(node_id)
            .expect("top-level node id should exist when building network.json");
        visited.insert(node_id.clone());
        network_json.insert(
            node_info.export_name.clone(),
            walk_parents(&parents, node_id, &mut visited).into(),
        );
    }
    let network_path = Path::new(&config.lqos_directory).join("network.json");
    if network_path.exists() && !config.integration_common.always_overwrite_network_json {
        warn!(
            "Network.json exists, and always overwrite network json is not true - not writing network.json"
        );
    } else {
        let json = serde_json::to_string_pretty(&network_json).unwrap();
        atomic_write_string(&network_path, &json).map_err(|e| {
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
    topology_editor_nodes.sort_unstable_by(|left, right| left.node_id.cmp(&right.node_id));
    let topology_editor_state = TopologyEditorStateFile {
        schema_version: 1,
        source: "uisp/full2".to_string(),
        generated_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs()),
        nodes: topology_editor_nodes,
    };
    if let Err(err) = topology_editor_state.save(&config) {
        warn!("Unable to write topology editor state snapshot: {err:?}");
    }
    let canonical_state = TopologyCanonicalStateFile::from_editor_and_network(
        &topology_editor_state,
        &Value::Object(network_json.clone()),
        TopologyCanonicalIngressKind::NativeIntegration,
    );
    if let Err(err) = canonical_state.save(&config) {
        warn!("Unable to write topology canonical state snapshot: {err:?}");
    }
    let export_elapsed_ms = export_started.elapsed().as_millis();

    // Shaped Devices
    let shaped_devices_started = Instant::now();
    let mut shaped_devices = Vec::new();
    let mut seen_pairs = HashSet::new();
    let mut processed_site_pairs = 0usize;
    let mut shaped_device_count = 0usize;
    let mut ethernet_advisories: Vec<CircuitEthernetMetadata> = Vec::new();

    for (ap_id, client_sites) in client_mappings.iter() {
        for site_id in client_sites.iter() {
            let Some(ap_device) = uisp_data.find_parsed_device_by_id(ap_id) else {
                continue;
            };
            let Some(site) = uisp_data.find_site_by_id(site_id) else {
                continue;
            };
            processed_site_pairs = processed_site_pairs.saturating_add(1);
            debug!(
                "Processing site: {} (ID: {}) with AP: {} (ID: {})",
                site.name, site.id, ap_device.name, ap_device.id
            );
            let site_devices: Vec<&UispDevice> = uisp_data
                .find_parsed_devices_in_site(site_id)
                .into_iter()
                .filter(|d| d.has_address())
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
            for device in uisp_data.find_parsed_devices_in_site(site_id) {
                if !device.has_address() {
                    continue;
                }

                let parent_node = {
                    let ap_node_id = format!("uisp:device:{}", ap_device.id);
                    if let Some(parent) = parents.get(&ap_node_id) {
                        parent.export_name.clone()
                    } else {
                        warn!(
                            "AP device '{}' ({}) not found in parents HashMap, assigning to {}",
                            ap_device.name, ap_device.id, orphans_node_name
                        );
                        orphans_node_name.clone()
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
    info!(
        shaped_devices = shaped_devices.len(),
        shaped_device_elapsed_ms = shaped_devices_started.elapsed().as_millis(),
        export_elapsed_ms,
        total_elapsed_ms = build_started.elapsed().as_millis(),
        "Completed UISP full2 export"
    );

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
        if let Some(dev_a) = uisp_data.find_device_by_id(&from_device.identification.id)
            && let Some(dev_b) = uisp_data.find_device_by_id(&to_device.identification.id)
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
            get_capacity_from_datalink_device(id_a, uisp_data, config)
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
    uisp_data: &UispData,
    config: &Arc<Config>,
) -> (u64, u64) {
    if let Some(device) = uisp_data.find_parsed_device_by_id(device_id) {
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
        let Some(device_details) = uisp_data.find_parsed_device_by_id(&device.identification.id)
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

struct RouteCache<'a> {
    devices: &'a [UispDevice],
    route_overrides: &'a [RouteOverride],
    cached_paths: HashMap<(NodeIndex, NodeIndex), Vec<NodeIndex>>,
}

impl<'a> RouteCache<'a> {
    fn new(devices: &'a [UispDevice], route_overrides: &'a [RouteOverride]) -> Self {
        Self {
            devices,
            route_overrides,
            cached_paths: HashMap::new(),
        }
    }

    fn route(&mut self, graph: &GraphType, start: NodeIndex, end: NodeIndex) -> Vec<NodeIndex> {
        if let Some(path) = self.cached_paths.get(&(start, end)) {
            return path.clone();
        }
        let path = astar_route(graph, start, end, self.devices, self.route_overrides);
        self.cached_paths.insert((start, end), path.clone());
        path
    }
}

fn is_real_topology_node(graph: &GraphType, node: NodeIndex) -> bool {
    matches!(
        graph[node],
        GraphMapping::Root { .. } | GraphMapping::Site { .. } | GraphMapping::AccessPoint { .. }
    )
}

fn is_site_anchor_node(graph: &GraphType, node: NodeIndex) -> bool {
    matches!(
        graph[node],
        GraphMapping::Root { .. } | GraphMapping::Site { .. } | GraphMapping::GeneratedSite { .. }
    )
}

fn topology_site_neighbor_for_access_point(
    graph: &GraphType,
    candidate: NodeIndex,
) -> Option<NodeIndex> {
    if !matches!(graph[candidate], GraphMapping::AccessPoint { .. }) {
        return None;
    }

    graph
        .neighbors_directed(candidate, petgraph::Incoming)
        .chain(graph.neighbors_directed(candidate, petgraph::Outgoing))
        .find(|neighbor| is_site_anchor_node(graph, *neighbor))
}

fn logical_parent_from_path(graph: &GraphType, path: &[NodeIndex]) -> Option<NodeIndex> {
    path.iter()
        .rev()
        .skip(1)
        .copied()
        .find(|node| {
            matches!(
                graph[*node],
                GraphMapping::Root { .. } | GraphMapping::Site { .. }
            )
        })
        .or_else(|| immediate_parent_from_route(path))
}

#[cfg(test)]
#[allow(dead_code)]
fn remote_logical_parent_for_local_attachment(
    graph: &GraphType,
    root_idx: NodeIndex,
    node_being_edited: NodeIndex,
    candidate: NodeIndex,
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> Option<NodeIndex> {
    let mut route_cache = RouteCache::new(devices, route_overrides);
    remote_logical_parent_for_local_attachment_cached(
        graph,
        root_idx,
        node_being_edited,
        candidate,
        &mut route_cache,
    )
}

fn remote_logical_parent_for_local_attachment_cached(
    graph: &GraphType,
    root_idx: NodeIndex,
    node_being_edited: NodeIndex,
    candidate: NodeIndex,
    route_cache: &mut RouteCache<'_>,
) -> Option<NodeIndex> {
    let local_site = topology_site_neighbor_for_access_point(graph, candidate)?;
    if local_site != node_being_edited {
        return None;
    }

    graph
        .neighbors_directed(candidate, petgraph::Incoming)
        .chain(graph.neighbors_directed(candidate, petgraph::Outgoing))
        .filter(|peer| matches!(graph[*peer], GraphMapping::AccessPoint { .. }))
        .filter(|peer| *peer != candidate)
        .filter_map(|peer| {
            let peer_site = topology_site_neighbor_for_access_point(graph, peer)?;
            if peer_site == local_site {
                return None;
            }

            let path = route_cache.route(graph, root_idx, peer);
            if path.is_empty() || path.contains(&node_being_edited) {
                return None;
            }

            let logical_parent = logical_parent_from_path(graph, &path)?;
            if !is_real_topology_node(graph, logical_parent) || logical_parent == node_being_edited
            {
                return None;
            }

            Some((
                path.len(),
                stable_node_key(graph, logical_parent),
                stable_node_key(graph, peer),
                logical_parent,
            ))
        })
        .min_by_key(|candidate| (candidate.0, candidate.1.clone(), candidate.2.clone()))
        .map(|candidate| candidate.3)
}

#[cfg(test)]
#[allow(dead_code)]
fn logical_parent_for_candidate(
    graph: &GraphType,
    root_idx: NodeIndex,
    node_being_edited: NodeIndex,
    candidate: NodeIndex,
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> NodeIndex {
    let mut route_cache = RouteCache::new(devices, route_overrides);
    logical_parent_for_candidate_cached(
        graph,
        root_idx,
        node_being_edited,
        candidate,
        &mut route_cache,
    )
}

fn logical_parent_for_candidate_cached(
    graph: &GraphType,
    root_idx: NodeIndex,
    node_being_edited: NodeIndex,
    candidate: NodeIndex,
    route_cache: &mut RouteCache<'_>,
) -> NodeIndex {
    if let Some(remote_parent) = remote_logical_parent_for_local_attachment_cached(
        graph,
        root_idx,
        node_being_edited,
        candidate,
        route_cache,
    ) {
        return remote_parent;
    }

    match &graph[candidate] {
        GraphMapping::Root { .. } | GraphMapping::Site { .. } => candidate,
        GraphMapping::GeneratedSite { .. } => candidate,
        GraphMapping::AccessPoint { .. } => {
            let path = route_cache.route(graph, root_idx, candidate);
            logical_parent_from_path(graph, &path).unwrap_or(candidate)
        }
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn topology_allowed_parent_groups_for_node(
    graph: &GraphType,
    root_idx: NodeIndex,
    node_being_edited: NodeIndex,
    candidate_parents: &[NodeIndex],
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> Vec<TopologyAllowedParentGroup> {
    let mut route_cache = RouteCache::new(devices, route_overrides);
    topology_allowed_parent_groups_for_node_cached(
        graph,
        root_idx,
        node_being_edited,
        candidate_parents,
        &mut route_cache,
    )
}

fn topology_allowed_parent_groups_for_node_cached(
    graph: &GraphType,
    root_idx: NodeIndex,
    node_being_edited: NodeIndex,
    candidate_parents: &[NodeIndex],
    route_cache: &mut RouteCache<'_>,
) -> Vec<TopologyAllowedParentGroup> {
    let mut groups = Vec::<TopologyAllowedParentGroup>::new();

    for candidate in candidate_parents
        .iter()
        .copied()
        .filter(|candidate| is_real_topology_node(graph, *candidate))
    {
        let logical_parent = logical_parent_for_candidate_cached(
            graph,
            root_idx,
            node_being_edited,
            candidate,
            route_cache,
        );
        if let Some(existing) = groups
            .iter_mut()
            .find(|group| group.logical_parent == logical_parent)
        {
            let candidate_id = graph[candidate].network_json_id();
            if !existing.candidate_nodes.iter().any(|existing_candidate| {
                graph[*existing_candidate].network_json_id() == candidate_id
            }) {
                existing.candidate_nodes.push(candidate);
            }
            continue;
        }

        groups.push(TopologyAllowedParentGroup {
            logical_parent,
            candidate_nodes: vec![candidate],
        });
    }

    groups.sort_unstable_by_key(|group| stable_node_key(graph, group.logical_parent));
    groups
}

fn attachment_kind_for_candidate(graph: &GraphType, candidate: NodeIndex) -> &'static str {
    match &graph[candidate] {
        GraphMapping::AccessPoint { .. } => "device",
        GraphMapping::Root { .. }
        | GraphMapping::Site { .. }
        | GraphMapping::GeneratedSite { .. } => "site",
    }
}

fn access_point_id(graph: &GraphType, node: NodeIndex) -> Option<&str> {
    match &graph[node] {
        GraphMapping::AccessPoint { id, .. } => Some(id.as_str()),
        _ => None,
    }
}

fn attachment_role_for_candidate(
    graph: &GraphType,
    candidate: NodeIndex,
    peer_candidate: Option<NodeIndex>,
    aps_with_clients: &HashSet<String>,
) -> TopologyAttachmentRole {
    if graph[candidate].network_json_id() == TOPOLOGY_ATTACHMENT_AUTO_ID {
        return TopologyAttachmentRole::Unknown;
    }
    if attachment_kind_for_candidate(graph, candidate) == "site" {
        return TopologyAttachmentRole::WiredUplink;
    }

    if peer_candidate
        .and_then(|peer| access_point_id(graph, peer))
        .is_some_and(|peer_id| aps_with_clients.contains(peer_id))
    {
        return TopologyAttachmentRole::PtmpUplink;
    }

    if peer_candidate.is_some() {
        return TopologyAttachmentRole::PtpBackhaul;
    }

    TopologyAttachmentRole::WiredUplink
}

fn first_probe_ip_for_device(devices: &[UispDevice], device_id: &str) -> Option<String> {
    let device = devices.iter().find(|device| device.id == device_id)?;
    device
        .probe_ipv4
        .iter()
        .min()
        .cloned()
        .or_else(|| device.probe_ipv6.iter().min().cloned())
}

fn attachment_pair_id(left: &str, right: &str) -> String {
    if left <= right {
        format!("{left}|{right}")
    } else {
        format!("{right}|{left}")
    }
}

struct AttachmentRateMetadata {
    rate_source: TopologyAttachmentRateSource,
    can_override_rate: bool,
    rate_override_disabled_reason: Option<String>,
    download_bandwidth_mbps: Option<u64>,
    upload_bandwidth_mbps: Option<u64>,
    transport_cap_mbps: Option<u64>,
    transport_cap_reason: Option<String>,
}

fn rate_override_metadata_for_candidate(
    graph: &GraphType,
    candidate: NodeIndex,
    devices: &[UispDevice],
) -> AttachmentRateMetadata {
    match &graph[candidate] {
        GraphMapping::AccessPoint {
            id,
            download_mbps,
            upload_mbps,
            ..
        } => {
            let matched_device = devices.iter().find(|device| device.id == *id);
            let rate_source = matched_device
                .map(|device| match device.attachment_rate_source {
                    UispAttachmentRateSource::DynamicIntegration
                        if *download_mbps == device.download && *upload_mbps == device.upload =>
                    {
                        TopologyAttachmentRateSource::DynamicIntegration
                    }
                    _ => TopologyAttachmentRateSource::Static,
                })
                .unwrap_or(TopologyAttachmentRateSource::Unknown);
            let (can_override_rate, disabled_reason) = if rate_source
                == TopologyAttachmentRateSource::DynamicIntegration
            {
                (
                    false,
                    Some("Rates are driven by dynamic UISP radio capacity telemetry.".to_string()),
                )
            } else {
                (true, None)
            };
            AttachmentRateMetadata {
                rate_source,
                can_override_rate,
                rate_override_disabled_reason: disabled_reason,
                download_bandwidth_mbps: Some(*download_mbps),
                upload_bandwidth_mbps: Some(*upload_mbps),
                transport_cap_mbps: matched_device.and_then(|device| device.transport_cap_mbps),
                transport_cap_reason: matched_device
                    .and_then(|device| device.transport_cap_reason.clone()),
            }
        }
        _ => AttachmentRateMetadata {
            rate_source: TopologyAttachmentRateSource::Unknown,
            can_override_rate: false,
            rate_override_disabled_reason: Some(
                "This attachment does not expose attachment-scoped rate controls.".to_string(),
            ),
            download_bandwidth_mbps: None,
            upload_bandwidth_mbps: None,
            transport_cap_mbps: None,
            transport_cap_reason: None,
        },
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn topology_attachment_option_for_candidate(
    graph: &GraphType,
    logical_parent: NodeIndex,
    candidate: NodeIndex,
    devices: &[UispDevice],
    aps_with_clients: &HashSet<String>,
    route_overrides: &[RouteOverride],
) -> TopologyAttachmentOption {
    let mut route_cache = RouteCache::new(devices, route_overrides);
    topology_attachment_option_for_candidate_cached(
        graph,
        logical_parent,
        candidate,
        devices,
        aps_with_clients,
        &mut route_cache,
    )
}

fn topology_attachment_option_for_candidate_cached(
    graph: &GraphType,
    logical_parent: NodeIndex,
    candidate: NodeIndex,
    devices: &[UispDevice],
    aps_with_clients: &HashSet<String>,
    route_cache: &mut RouteCache<'_>,
) -> TopologyAttachmentOption {
    let attachment_id = graph[candidate].network_json_id();
    let attachment_name = graph[candidate].name();
    let attachment_kind = attachment_kind_for_candidate(graph, candidate).to_string();
    let attachment_rate_metadata = rate_override_metadata_for_candidate(graph, candidate, devices);
    let rate_source = attachment_rate_metadata.rate_source;
    let can_override_rate = attachment_rate_metadata.can_override_rate;
    let rate_override_disabled_reason = attachment_rate_metadata.rate_override_disabled_reason;
    let download_bandwidth_mbps = attachment_rate_metadata.download_bandwidth_mbps;
    let upload_bandwidth_mbps = attachment_rate_metadata.upload_bandwidth_mbps;
    let transport_cap_mbps = attachment_rate_metadata.transport_cap_mbps;
    let transport_cap_reason = attachment_rate_metadata.transport_cap_reason;
    let capacity_mbps = match (download_bandwidth_mbps, upload_bandwidth_mbps) {
        (Some(download), Some(upload)) => Some(download.min(upload)),
        (Some(download), None) => Some(download),
        (None, Some(upload)) => Some(upload),
        (None, None) => None,
    };

    let peer_candidate = peer_attachment_candidate_for_candidate_cached(
        graph,
        logical_parent,
        candidate,
        route_cache,
    );

    let peer_attachment_id = peer_candidate.map(|node| graph[node].network_json_id());
    let peer_attachment_name = peer_candidate.map(|node| graph[node].name());
    let attachment_role =
        attachment_role_for_candidate(graph, candidate, peer_candidate, aps_with_clients);
    let local_probe_ip = match &graph[candidate] {
        GraphMapping::AccessPoint { id, .. } => first_probe_ip_for_device(devices, id),
        _ => None,
    };
    let remote_probe_ip = match peer_candidate {
        Some(node) => match &graph[node] {
            GraphMapping::AccessPoint { id, .. } => first_probe_ip_for_device(devices, id),
            _ => None,
        },
        None => None,
    };
    let pair_id = peer_attachment_id
        .as_ref()
        .map(|peer_id| attachment_pair_id(&attachment_id, peer_id));

    TopologyAttachmentOption {
        attachment_id,
        attachment_name,
        attachment_kind,
        attachment_role,
        pair_id,
        peer_attachment_id,
        peer_attachment_name,
        capacity_mbps,
        download_bandwidth_mbps,
        upload_bandwidth_mbps,
        transport_cap_mbps,
        transport_cap_reason,
        rate_source,
        can_override_rate,
        rate_override_disabled_reason,
        has_rate_override: false,
        local_probe_ip,
        remote_probe_ip,
        probe_enabled: false,
        probeable: false,
        health_status: lqos_config::TopologyAttachmentHealthStatus::Disabled,
        health_reason: None,
        suppressed_until_unix: None,
        effective_selected: false,
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn peer_attachment_candidate_for_candidate(
    graph: &GraphType,
    logical_parent: NodeIndex,
    candidate: NodeIndex,
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> Option<NodeIndex> {
    let mut route_cache = RouteCache::new(devices, route_overrides);
    peer_attachment_candidate_for_candidate_cached(
        graph,
        logical_parent,
        candidate,
        &mut route_cache,
    )
}

fn peer_attachment_candidate_for_candidate_cached(
    graph: &GraphType,
    logical_parent: NodeIndex,
    candidate: NodeIndex,
    route_cache: &mut RouteCache<'_>,
) -> Option<NodeIndex> {
    let route_from_parent = route_cache.route(graph, logical_parent, candidate);
    if route_from_parent.len() < 2 {
        return None;
    }

    route_from_parent
        .get(route_from_parent.len().saturating_sub(2))
        .copied()
        .filter(|node| *node != logical_parent && *node != candidate)
}

#[cfg(test)]
#[allow(dead_code)]
fn topology_allowed_parents_from_groups(
    graph: &GraphType,
    groups: &[TopologyAllowedParentGroup],
    devices: &[UispDevice],
    aps_with_clients: &HashSet<String>,
    route_overrides: &[RouteOverride],
) -> Vec<TopologyAllowedParent> {
    let mut route_cache = RouteCache::new(devices, route_overrides);
    topology_allowed_parents_from_groups_cached(graph, groups, aps_with_clients, &mut route_cache)
}

fn topology_allowed_parents_from_groups_cached(
    graph: &GraphType,
    groups: &[TopologyAllowedParentGroup],
    aps_with_clients: &HashSet<String>,
    route_cache: &mut RouteCache<'_>,
) -> Vec<TopologyAllowedParent> {
    let mut allowed_parents = Vec::with_capacity(groups.len());
    for group in groups {
        let parent = {
            let mut attachment_options = vec![TopologyAttachmentOption {
                attachment_id: TOPOLOGY_ATTACHMENT_AUTO_ID.to_string(),
                attachment_name: "Auto".to_string(),
                attachment_kind: "auto".to_string(),
                attachment_role: TopologyAttachmentRole::Unknown,
                pair_id: None,
                peer_attachment_id: None,
                peer_attachment_name: None,
                capacity_mbps: None,
                download_bandwidth_mbps: None,
                upload_bandwidth_mbps: None,
                transport_cap_mbps: None,
                transport_cap_reason: None,
                rate_source: TopologyAttachmentRateSource::Unknown,
                can_override_rate: false,
                rate_override_disabled_reason: None,
                has_rate_override: false,
                local_probe_ip: None,
                remote_probe_ip: None,
                probe_enabled: false,
                probeable: false,
                health_status: lqos_config::TopologyAttachmentHealthStatus::Disabled,
                health_reason: None,
                suppressed_until_unix: None,
                effective_selected: false,
            }];
            let mut seen_attachment_ids = HashSet::from([TOPOLOGY_ATTACHMENT_AUTO_ID.to_string()]);

            attachment_options.extend(
                group
                    .candidate_nodes
                    .iter()
                    .copied()
                    .filter(|candidate| {
                        matches!(graph[*candidate], GraphMapping::AccessPoint { .. })
                    })
                    .filter(|candidate| {
                        seen_attachment_ids.insert(graph[*candidate].network_json_id())
                    })
                    .map(|candidate| {
                        topology_attachment_option_for_candidate_cached(
                            graph,
                            group.logical_parent,
                            candidate,
                            route_cache.devices,
                            aps_with_clients,
                            route_cache,
                        )
                    }),
            );

            TopologyAllowedParent {
                parent_node_id: graph[group.logical_parent].network_json_id(),
                parent_node_name: graph[group.logical_parent].name(),
                attachment_options,
                all_attachments_suppressed: false,
                has_probe_unavailable_attachments: false,
            }
        };
        allowed_parents.push(parent);
    }
    allowed_parents
}

#[cfg(test)]
#[allow(dead_code)]
fn topology_parent_candidates_for_node(
    graph: &GraphType,
    root_idx: NodeIndex,
    node: NodeIndex,
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> Vec<NodeIndex> {
    let mut route_cache = RouteCache::new(devices, route_overrides);
    topology_parent_candidates_for_node_cached(graph, root_idx, node, &mut route_cache)
}

fn topology_parent_candidates_for_node_cached(
    graph: &GraphType,
    root_idx: NodeIndex,
    node: NodeIndex,
    route_cache: &mut RouteCache<'_>,
) -> Vec<NodeIndex> {
    let mut candidates: Vec<NodeIndex> = graph
        .neighbors_directed(node, petgraph::Incoming)
        .filter(|candidate| is_real_topology_node(graph, *candidate))
        .filter(|candidate| {
            is_upstream_parent_candidate_cached(graph, root_idx, node, *candidate, route_cache)
        })
        .collect();
    candidates.sort_unstable_by_key(|candidate| stable_node_key(graph, *candidate));
    let mut seen_ids = HashSet::new();
    candidates.retain(|candidate| seen_ids.insert(graph[*candidate].network_json_id()));
    candidates
}

#[cfg(test)]
#[allow(dead_code)]
fn is_upstream_parent_candidate(
    graph: &GraphType,
    root_idx: NodeIndex,
    node: NodeIndex,
    candidate: NodeIndex,
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> bool {
    let mut route_cache = RouteCache::new(devices, route_overrides);
    is_upstream_parent_candidate_cached(graph, root_idx, node, candidate, &mut route_cache)
}

fn is_upstream_parent_candidate_cached(
    graph: &GraphType,
    root_idx: NodeIndex,
    node: NodeIndex,
    candidate: NodeIndex,
    route_cache: &mut RouteCache<'_>,
) -> bool {
    if node == candidate {
        return false;
    }

    let path_to_candidate = route_cache.route(graph, root_idx, candidate);
    if path_to_candidate.is_empty() {
        return false;
    }

    // Direct child radios normally look like descendants because the shortest path to the
    // local radio can traverse the site being edited. When that radio has a remote peer that
    // reaches the root without traversing the local site, treat it as a legal inter-site
    // parent candidate instead of rejecting it as a child loop.
    if !path_to_candidate.contains(&node) {
        return true;
    }

    remote_logical_parent_for_local_attachment_cached(graph, root_idx, node, candidate, route_cache)
        .is_some()
}

fn immediate_parent_from_route(path: &[NodeIndex]) -> Option<NodeIndex> {
    if path.len() < 2 {
        None
    } else {
        path.get(path.len().saturating_sub(2)).copied()
    }
}

#[cfg(test)]
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

#[cfg(test)]
fn resolve_attachment_parent_candidate(
    graph: &GraphType,
    node: NodeIndex,
    native_parent: Option<NodeIndex>,
    groups: &[TopologyAllowedParentGroup],
    overrides: &HashMap<String, TopologyAttachmentOverrideSelection>,
) -> Option<NodeIndex> {
    let node_id = graph[node].network_json_id();
    let override_entry = overrides.get(&node_id)?;

    let Some(group) = groups.iter().find(|group| {
        graph[group.logical_parent].network_json_id() == override_entry.parent_node_id
    }) else {
        warn!(
            node = %graph[node].name(),
            node_id = %node_id,
            parent_node_id = %override_entry.parent_node_id,
            "Topology manager override parent is stale; falling back to legacy/native selection"
        );
        return None;
    };

    let native_in_group =
        native_parent.filter(|candidate| group.candidate_nodes.contains(candidate));
    let first_candidate = group.candidate_nodes.first().copied();

    match override_entry.mode {
        TopologyAttachmentMode::Auto => native_in_group.or(first_candidate),
        TopologyAttachmentMode::PreferredOrder => override_entry
            .attachment_preference_ids
            .iter()
            .find_map(|attachment_id| {
                group
                    .candidate_nodes
                    .iter()
                    .copied()
                    .find(|candidate| graph[*candidate].network_json_id() == *attachment_id)
            })
            .or(native_in_group)
            .or(first_candidate),
    }
}

#[cfg(test)]
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

#[cfg(test)]
fn export_parent_anchor_for_override(
    graph: &GraphType,
    node: NodeIndex,
    logical_parent: NodeIndex,
    candidate: NodeIndex,
    devices: &[UispDevice],
    route_overrides: &[RouteOverride],
) -> NodeIndex {
    let GraphMapping::AccessPoint { site_name, .. } = &graph[candidate] else {
        return candidate;
    };
    if *site_name != graph[node].name() {
        return candidate;
    }

    peer_attachment_candidate_for_candidate(
        graph,
        logical_parent,
        candidate,
        devices,
        route_overrides,
    )
    .unwrap_or(candidate)
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
        GraphType, TopologyAllowedParentGroup, TopologyAttachmentOverrideSelection,
        TopologyParentOverrideSelection, UispDevice, build_constrained_route,
        export_parent_anchor_for_override, first_probe_ip_for_device, immediate_parent_from_route,
        is_upstream_parent_candidate, logical_parent_for_candidate,
        resolve_attachment_parent_candidate, resolve_parent_candidate,
        topology_allowed_parent_groups_for_node, topology_allowed_parents_from_groups,
        topology_parent_candidates_for_node,
    };
    use crate::strategies::full2::graph_mapping::GraphMapping;
    use crate::strategies::full2::link_mapping::LinkMapping;
    use crate::uisp_types::UispAttachmentRateSource;
    use lqos_config::TopologyAttachmentRole;
    use lqos_overrides::{TopologyAttachmentMode, TopologyParentOverrideMode};
    use std::collections::{HashMap, HashSet};

    fn site(name: &str, id: &str) -> GraphMapping {
        GraphMapping::Site {
            name: name.to_string(),
            id: id.to_string(),
            latitude: None,
            longitude: None,
        }
    }

    fn ap(name: &str, id: &str, site_name: &str) -> GraphMapping {
        GraphMapping::AccessPoint {
            name: name.to_string(),
            id: id.to_string(),
            site_name: site_name.to_string(),
            download_mbps: 1000,
            upload_mbps: 1000,
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

    #[test]
    fn child_access_points_are_not_offered_as_topology_parent_candidates() {
        let mut graph = GraphType::new();
        let root = graph.add_node(GraphMapping::Root {
            name: "Upstream".to_string(),
            id: "root".to_string(),
            latitude: None,
            longitude: None,
        });
        let target_site = graph.add_node(site("TrailerCity", "site-trailercity"));
        let parent_site = graph.add_node(site("MRE", "site-mre"));
        let child_ap = graph.add_node(GraphMapping::AccessPoint {
            name: "JR_AP_TC_A".to_string(),
            id: "device-jr-ap-tc-a".to_string(),
            site_name: "TrailerCity".to_string(),
            download_mbps: 1000,
            upload_mbps: 1000,
        });

        for (from, to) in [
            (root, parent_site),
            (parent_site, root),
            (parent_site, target_site),
            (target_site, parent_site),
            (target_site, child_ap),
            (child_ap, target_site),
        ] {
            graph.add_edge(from, to, LinkMapping::ethernet(1_000));
        }

        let candidates = topology_parent_candidates_for_node(&graph, root, target_site, &[], &[]);
        assert_eq!(candidates, vec![parent_site]);
        assert!(is_upstream_parent_candidate(
            &graph,
            root,
            target_site,
            parent_site,
            &[],
            &[],
        ));
        assert!(!is_upstream_parent_candidate(
            &graph,
            root,
            target_site,
            child_ap,
            &[],
            &[],
        ));
    }

    #[test]
    fn legacy_bad_child_override_falls_back_to_native_parent() {
        let mut graph = GraphType::new();
        let root = graph.add_node(GraphMapping::Root {
            name: "Upstream".to_string(),
            id: "root".to_string(),
            latitude: None,
            longitude: None,
        });
        let parent_site = graph.add_node(site("MRE", "site-mre"));
        let target_site = graph.add_node(site("TrailerCity", "site-trailercity"));
        let child_ap = graph.add_node(GraphMapping::AccessPoint {
            name: "JR_AP_TC_A".to_string(),
            id: "device-jr-ap-tc-a".to_string(),
            site_name: "TrailerCity".to_string(),
            download_mbps: 1000,
            upload_mbps: 1000,
        });

        for (from, to) in [
            (root, parent_site),
            (parent_site, root),
            (parent_site, target_site),
            (target_site, parent_site),
            (target_site, child_ap),
            (child_ap, target_site),
        ] {
            graph.add_edge(from, to, LinkMapping::ethernet(1_000));
        }

        let candidates = topology_parent_candidates_for_node(&graph, root, target_site, &[], &[]);
        let native_parent = Some(parent_site);
        let mut overrides = HashMap::new();
        overrides.insert(
            graph[target_site].network_json_id(),
            TopologyParentOverrideSelection {
                mode: TopologyParentOverrideMode::Pinned,
                parent_node_ids: vec![graph[child_ap].network_json_id()],
            },
        );

        let resolved =
            resolve_parent_candidate(&graph, target_site, native_parent, &candidates, &overrides);
        assert_eq!(resolved, Some(parent_site));
    }

    fn build_parallel_attachment_graph() -> (
        GraphType,
        petgraph::graph::NodeIndex,
        petgraph::graph::NodeIndex,
        petgraph::graph::NodeIndex,
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
        let parent_site = graph.add_node(site("Parent Site", "site-parent"));
        let child_site = graph.add_node(site("Child Site", "site-child"));
        let parent_wave = graph.add_node(ap(
            "Backhaul Wave Parent",
            "device-parent-wave",
            "Parent Site",
        ));
        let child_wave =
            graph.add_node(ap("Backhaul Wave Child", "device-child-wave", "Child Site"));
        let parent_4600 = graph.add_node(ap(
            "Backhaul AirFiber Parent",
            "device-parent-airfiber",
            "Parent Site",
        ));
        let child_4600 = graph.add_node(ap(
            "Backhaul AirFiber Child",
            "device-child-airfiber",
            "Child Site",
        ));

        for (from, to) in [
            (root, parent_site),
            (parent_site, root),
            (parent_site, parent_wave),
            (parent_wave, parent_site),
            (parent_site, parent_4600),
            (parent_4600, parent_site),
            (parent_wave, child_wave),
            (child_wave, parent_wave),
            (parent_4600, child_4600),
            (child_4600, parent_4600),
            (child_site, child_wave),
            (child_wave, child_site),
            (child_site, child_4600),
            (child_4600, child_site),
        ] {
            graph.add_edge(from, to, LinkMapping::ethernet(1_000));
        }

        (
            graph,
            root,
            parent_site,
            child_site,
            parent_wave,
            child_wave,
            parent_4600,
            child_4600,
        )
    }

    #[test]
    fn parallel_device_candidates_group_under_one_logical_parent() {
        let (
            graph,
            root,
            parent_site,
            child_site,
            _parent_wave,
            child_wave,
            _parent_4600,
            child_4600,
        ) = build_parallel_attachment_graph();

        let candidates = topology_parent_candidates_for_node(&graph, root, child_site, &[], &[]);
        assert_eq!(candidates, vec![child_4600, child_wave]);

        let groups = topology_allowed_parent_groups_for_node(
            &graph,
            root,
            child_site,
            &candidates,
            &[],
            &[],
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].logical_parent, parent_site);
        assert_eq!(groups[0].candidate_nodes, vec![child_4600, child_wave]);

        let allowed =
            topology_allowed_parents_from_groups(&graph, &groups, &[], &HashSet::new(), &[]);
        assert_eq!(allowed.len(), 1);
        assert_eq!(
            allowed[0].parent_node_id,
            graph[parent_site].network_json_id()
        );
        assert_eq!(allowed[0].parent_node_name, "Parent Site");
        assert_eq!(
            allowed[0]
                .attachment_options
                .iter()
                .map(|option| option.attachment_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Auto", "Backhaul AirFiber Child", "Backhaul Wave Child"]
        );
        assert!(
            allowed[0]
                .attachment_options
                .iter()
                .skip(1)
                .all(|option| option.attachment_role == TopologyAttachmentRole::PtpBackhaul)
        );
    }

    #[test]
    fn ptmp_child_attachment_is_classified_as_ptmp_uplink() {
        let mut graph = GraphType::new();
        let root = graph.add_node(GraphMapping::Root {
            name: "Upstream".to_string(),
            id: "root".to_string(),
            latitude: None,
            longitude: None,
        });
        let parent_site = graph.add_node(site("Parent Site", "site-parent"));
        let child_site = graph.add_node(site("Child Site", "site-child"));
        let parent_ap = graph.add_node(ap("Access AP", "device-parent-ap", "Parent Site"));
        let child_cpe = graph.add_node(ap("Child CPE", "device-child-cpe", "Child Site"));

        for (from, to) in [
            (root, parent_site),
            (parent_site, root),
            (parent_site, parent_ap),
            (parent_ap, parent_site),
            (parent_ap, child_cpe),
            (child_cpe, parent_ap),
            (child_site, child_cpe),
            (child_cpe, child_site),
        ] {
            graph.add_edge(from, to, LinkMapping::ethernet(1_000));
        }

        let groups = vec![TopologyAllowedParentGroup {
            logical_parent: parent_site,
            candidate_nodes: vec![child_cpe],
        }];
        let mut aps_with_clients = HashSet::new();
        aps_with_clients.insert("device-parent-ap".to_string());

        let allowed =
            topology_allowed_parents_from_groups(&graph, &groups, &[], &aps_with_clients, &[]);
        let attachment = allowed[0]
            .attachment_options
            .iter()
            .find(|option| option.attachment_id == graph[child_cpe].network_json_id())
            .expect("expected explicit child-side attachment");
        assert_eq!(
            attachment.attachment_role,
            TopologyAttachmentRole::PtmpUplink
        );
        assert_eq!(
            attachment.peer_attachment_name.as_deref(),
            Some("Access AP")
        );
    }

    #[test]
    fn topology_manager_attachment_override_picks_requested_parallel_path() {
        let (
            graph,
            root,
            parent_site,
            child_site,
            _parent_wave,
            child_wave,
            _parent_4600,
            child_4600,
        ) = build_parallel_attachment_graph();
        let candidates = topology_parent_candidates_for_node(&graph, root, child_site, &[], &[]);
        let groups = topology_allowed_parent_groups_for_node(
            &graph,
            root,
            child_site,
            &candidates,
            &[],
            &[],
        );
        let native_parent = Some(child_wave);
        let mut overrides = HashMap::new();
        overrides.insert(
            graph[child_site].network_json_id(),
            TopologyAttachmentOverrideSelection {
                parent_node_id: graph[parent_site].network_json_id(),
                mode: TopologyAttachmentMode::PreferredOrder,
                attachment_preference_ids: vec![graph[child_4600].network_json_id()],
            },
        );

        let resolved = resolve_attachment_parent_candidate(
            &graph,
            child_site,
            native_parent,
            &groups,
            &overrides,
        );
        assert_eq!(resolved, Some(child_4600));
    }

    #[test]
    fn logical_parent_for_parallel_attachment_candidate_is_upstream_site() {
        let (
            graph,
            root,
            parent_site,
            child_site,
            _parent_wave,
            child_wave,
            _parent_4600,
            _child_4600,
        ) = build_parallel_attachment_graph();

        let logical_parent =
            logical_parent_for_candidate(&graph, root, child_site, child_wave, &[], &[]);
        assert_eq!(logical_parent, parent_site);
    }

    #[test]
    fn sibling_site_linked_by_local_attachment_is_legal_parent() {
        let mut graph = GraphType::new();
        let root = graph.add_node(GraphMapping::Root {
            name: "Site Gamma".to_string(),
            id: "site-root".to_string(),
            latitude: None,
            longitude: None,
        });
        let beta_site = graph.add_node(site("Site Beta", "site-beta"));
        let alpha_site = graph.add_node(site("Site Alpha", "site-alpha"));
        let gamma_to_beta = graph.add_node(ap(
            "Gamma - Beta MLO6",
            "device-gamma-beta",
            "Site Gamma",
        ));
        let beta_to_gamma = graph.add_node(ap(
            "Beta - Gamma MLO6",
            "device-beta-gamma",
            "Site Beta",
        ));
        let gamma_to_alpha = graph.add_node(ap(
            "Gamma-Alpha",
            "device-gamma-alpha",
            "Site Gamma",
        ));
        let alpha_to_gamma = graph.add_node(ap(
            "Alpha-Gamma",
            "device-alpha-gamma",
            "Site Alpha",
        ));
        let beta_to_alpha = graph.add_node(ap(
            "Beta - Alpha MLO5",
            "device-beta-alpha",
            "Site Beta",
        ));
        let alpha_to_beta =
            graph.add_node(ap("Alpha - Beta MLO5", "device-alpha-beta", "Site Alpha"));

        for (from, to) in [
            (root, gamma_to_beta),
            (gamma_to_beta, root),
            (gamma_to_beta, beta_to_gamma),
            (beta_to_gamma, gamma_to_beta),
            (beta_site, beta_to_gamma),
            (beta_to_gamma, beta_site),
            (root, gamma_to_alpha),
            (gamma_to_alpha, root),
            (gamma_to_alpha, alpha_to_gamma),
            (alpha_to_gamma, gamma_to_alpha),
            (alpha_site, alpha_to_gamma),
            (alpha_to_gamma, alpha_site),
            (beta_site, beta_to_alpha),
            (beta_to_alpha, beta_site),
            (beta_to_alpha, alpha_to_beta),
            (alpha_to_beta, beta_to_alpha),
            (alpha_site, alpha_to_beta),
            (alpha_to_beta, alpha_site),
        ] {
            graph.add_edge(from, to, LinkMapping::ethernet(1_000));
        }

        let candidates = topology_parent_candidates_for_node(&graph, root, beta_site, &[], &[]);
        assert_eq!(candidates, vec![beta_to_alpha, beta_to_gamma]);
        assert!(is_upstream_parent_candidate(
            &graph,
            root,
            beta_site,
            beta_to_alpha,
            &[],
            &[],
        ));

        let groups = topology_allowed_parent_groups_for_node(
            &graph,
            root,
            beta_site,
            &candidates,
            &[],
            &[],
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].logical_parent, alpha_site);
        assert_eq!(groups[0].candidate_nodes, vec![beta_to_alpha]);
        assert_eq!(groups[1].logical_parent, root);
        assert_eq!(groups[1].candidate_nodes, vec![beta_to_gamma]);

        let allowed =
            topology_allowed_parents_from_groups(&graph, &groups, &[], &HashSet::new(), &[]);
        assert_eq!(
            allowed
                .iter()
                .map(|parent| parent.parent_node_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Site Alpha", "Site Gamma"]
        );
        let alpha_parent = allowed
            .iter()
            .find(|parent| parent.parent_node_name == "Site Alpha")
            .expect("expected Site Alpha as a legal parent");
        assert_eq!(
            alpha_parent
                .attachment_options
                .iter()
                .map(|option| option.attachment_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Auto", "Beta - Alpha MLO5"]
        );
    }

    #[test]
    fn export_override_anchors_under_peer_attachment_for_child_owned_candidate() {
        let mut graph = GraphType::new();
        let root = graph.add_node(GraphMapping::Root {
            name: "Site Gamma".to_string(),
            id: "site-root".to_string(),
            latitude: None,
            longitude: None,
        });
        let beta_site = graph.add_node(site("Site Beta", "site-beta"));
        let alpha_site = graph.add_node(site("Site Alpha", "site-alpha"));
        let gamma_to_alpha = graph.add_node(ap(
            "Gamma-Alpha",
            "device-gamma-alpha",
            "Site Gamma",
        ));
        let alpha_to_gamma = graph.add_node(ap(
            "Alpha-Gamma",
            "device-alpha-gamma",
            "Site Alpha",
        ));
        let beta_to_alpha =
            graph.add_node(ap("Beta - Alpha 60", "device-beta-alpha", "Site Beta"));
        let alpha_to_beta =
            graph.add_node(ap("Alpha-Beta-60", "device-alpha-beta", "Site Alpha"));

        for (from, to) in [
            (root, gamma_to_alpha),
            (gamma_to_alpha, root),
            (gamma_to_alpha, alpha_to_gamma),
            (alpha_to_gamma, gamma_to_alpha),
            (alpha_site, alpha_to_gamma),
            (alpha_to_gamma, alpha_site),
            (beta_site, beta_to_alpha),
            (beta_to_alpha, beta_site),
            (beta_to_alpha, alpha_to_beta),
            (alpha_to_beta, beta_to_alpha),
            (alpha_site, alpha_to_beta),
            (alpha_to_beta, alpha_site),
        ] {
            graph.add_edge(from, to, LinkMapping::ethernet(1_000));
        }

        let (route_from_root, _) =
            build_constrained_route(&graph, root, beta_site, beta_to_alpha, &[], &[])
                .expect("expected constrained route for override export");
        assert_eq!(
            immediate_parent_from_route(&route_from_root),
            Some(beta_to_alpha)
        );

        let logical_parent =
            logical_parent_for_candidate(&graph, root, beta_site, beta_to_alpha, &[], &[]);
        assert_eq!(logical_parent, alpha_site);
        assert_eq!(
            export_parent_anchor_for_override(
                &graph,
                beta_site,
                logical_parent,
                beta_to_alpha,
                &[],
                &[],
            ),
            alpha_to_beta
        );
    }

    #[test]
    fn probe_ip_selection_uses_unfiltered_management_ips_without_cidr() {
        let devices = vec![UispDevice {
            id: "device-mre-wave".to_string(),
            name: "WavePro-MREToRochester".to_string(),
            mac: String::new(),
            role: Some("station".to_string()),
            wireless_mode: Some("sta-ptp".to_string()),
            site_id: "site-mre".to_string(),
            raw_download: 1000,
            raw_upload: 1000,
            download: 1000,
            upload: 1000,
            ipv4: HashSet::new(),
            ipv6: HashSet::new(),
            probe_ipv4: HashSet::from(["100.126.0.226".to_string()]),
            probe_ipv6: HashSet::new(),
            negotiated_ethernet_mbps: None,
            negotiated_ethernet_interface: None,
            transport_cap_mbps: None,
            transport_cap_reason: None,
            attachment_rate_source: UispAttachmentRateSource::Static,
        }];

        assert_eq!(
            first_probe_ip_for_device(&devices, "device-mre-wave").as_deref(),
            Some("100.126.0.226")
        );
    }
}

#[cfg(test)]
fn total_degree(graph: &GraphType, node: NodeIndex) -> usize {
    graph.neighbors_directed(node, petgraph::Incoming).count()
        + graph.neighbors_directed(node, petgraph::Outgoing).count()
}

#[cfg(test)]
fn is_relay_node(graph: &GraphType, node: NodeIndex) -> bool {
    total_degree(graph, node) == 4 && sorted_unique_neighbors(graph, node).len() == 2
}

#[cfg(test)]
fn is_meaningful_endpoint(graph: &GraphType, node: NodeIndex) -> bool {
    let unique_neighbors = sorted_unique_neighbors(graph, node);
    !(total_degree(graph, node) == 4 && unique_neighbors.len() == 2)
        && unique_neighbors.len() >= 3
        && !matches!(graph[node], GraphMapping::AccessPoint { .. })
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

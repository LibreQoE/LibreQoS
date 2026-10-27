mod directionality;
mod graph_mapping;
mod link_mapping;
mod net_json_parent;
mod orphan_latch;

use crate::blackboard_blob;
use crate::errors::UispIntegrationError;
use crate::ethernet_advisory::apply_ethernet_rate_cap;
use crate::ip_ranges::IpRanges;
use crate::strategies::common::UispData;
use crate::strategies::full2::directionality::{
    build_device_capacity_map, build_device_link_meta_map, directed_caps_mbps,
};
use crate::strategies::full2::graph_mapping::GraphMapping;
use crate::strategies::full2::link_mapping::LinkMapping;
use crate::strategies::full2::net_json_parent::{NetJsonParent, assign_export_names, walk_parents};
use crate::strategies::legacy_bandwidth_overrides::{BandwidthOverride, find_bandwidth_override};
use crate::strategies::legacy_routes_override::RouteOverride;
use crate::uisp_types::{UispAttachmentRateSource, UispDevice, UispSite};
use lqos_config::{
    CircuitAnchor, CircuitAnchorsFile, CircuitEthernetMetadata, Config, ConfigShapedDevices,
    EthernetCapTargetKind, EthernetPortLimitPolicy, RequestedCircuitRates,
    ShapedDevice as ConfigShapedDevice, TOPOLOGY_ATTACHMENT_AUTO_ID, TopologyAllowedParent,
    TopologyAttachmentOption, TopologyAttachmentRateSource, TopologyAttachmentRole,
    TopologyCanonicalIngressKind, TopologyCanonicalStateFile, TopologyEditorNode,
    TopologyEditorStateFile, TopologyParentCandidate, TopologyParentCandidatesFile,
    TopologyParentCandidatesNode, topology_auto_attachment_option,
};
#[cfg(test)]
use lqos_overrides::{TopologyAttachmentMode, TopologyParentOverrideMode};
use lqos_topology_compile::ImportedTopologyBundle;
use petgraph::Directed;
use petgraph::graph::NodeIndex;
use petgraph::visit::{EdgeRef, NodeRef};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

type GraphType = petgraph::Graph<GraphMapping, LinkMapping, Directed>;
const GENERATED_INTERNET_ROOT_NAME: &str = "INSERTED_INTERNET";
const GENERATED_INTERNET_ROOT_ID: &str = "ROOT-001";

/// Builds advisory metadata for a topology node whose infrastructure transport cap reduced it.
fn topology_node_ethernet_advisory(device: &UispDevice) -> Option<CircuitEthernetMetadata> {
    let applied_ethernet_cap = device.transport_cap_mbps?;
    let auto_capped = device.download < device.raw_download || device.upload < device.raw_upload;
    if !auto_capped {
        return None;
    }

    let target_id = format!("uisp:device:{}", device.id);
    Some(CircuitEthernetMetadata {
        device_ids: vec![device.id.clone()],
        source: "uisp/infrastructure_transport".to_string(),
        negotiated_ethernet_mbps: device
            .transport_cap_line_rate_mbps
            .or(device.negotiated_ethernet_mbps)
            .unwrap_or(applied_ethernet_cap),
        requested_download_mbps: device.raw_download as f32,
        requested_upload_mbps: device.raw_upload as f32,
        applied_download_mbps: device.download as f32,
        applied_upload_mbps: device.upload as f32,
        auto_capped,
        limiting_device_id: Some(device.id.clone()),
        limiting_device_name: Some(device.name.clone()),
        limiting_interface_name: device
            .transport_cap_interface
            .clone()
            .or_else(|| device.negotiated_ethernet_interface.clone()),
        ..CircuitEthernetMetadata::for_target(
            EthernetCapTargetKind::Node,
            target_id,
            device.name.clone(),
        )
    })
}

/// 3b rate fix(ii)): the base `(download, upload)` Mbps for a
/// plan-less site. The CLIENT devices' UISP service plan wins; the site/PN
/// `generated_pn_*` sentinel is only the last resort (the compiler leaf clamp
/// caps it to the parent's effective ceil). PURE.
fn planless_client_base_mbps(site: &UispSite, site_devices: &[&UispDevice]) -> (f32, f32) {
    let (device_down, device_up) =
        site_devices.iter().fold((0_u64, 0_u64), |(down, up), device| {
            (down.max(device.raw_download), up.max(device.raw_upload))
        });
    if device_down > 0 || device_up > 0 {
        (device_down as f32, device_up as f32)
    } else {
        (site.max_down_mbps as f32, site.max_up_mbps as f32)
    }
}

#[derive(Clone, Debug)]
struct LegacyShapedDevice {
    circuit_id: String,
    circuit_name: String,
    device_id: String,
    device_name: String,
    parent_node: String,
    parent_node_id: String,
    anchor_node_id: String,
    mac: String,
    ipv4: String,
    ipv6: String,
    download_min: f32,
    upload_min: f32,
    download_max: f32,
    upload_max: f32,
    comment: String,
}

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

fn now_unix() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn config_shaped_device(legacy: LegacyShapedDevice) -> ConfigShapedDevice {
    let headers = csv::StringRecord::from(vec![
        "circuit_id",
        "circuit_name",
        "device_id",
        "device_name",
        "parent_node",
        "parent_node_id",
        "anchor_node_id",
        "mac",
        "ipv4",
        "ipv6",
        "download_min",
        "upload_min",
        "download_max",
        "upload_max",
        "comment",
    ]);
    let record = csv::StringRecord::from(vec![
        legacy.circuit_id,
        legacy.circuit_name,
        legacy.device_id,
        legacy.device_name,
        legacy.parent_node,
        legacy.parent_node_id,
        legacy.anchor_node_id,
        legacy.mac,
        legacy.ipv4,
        legacy.ipv6,
        legacy.download_min.to_string(),
        legacy.upload_min.to_string(),
        legacy.download_max.to_string(),
        legacy.upload_max.to_string(),
        legacy.comment,
    ]);
    ConfigShapedDevice::from_csv(&record, Some(&headers))
        .expect("UISP shaped-device rows must remain valid during compile conversion")
}

fn config_shaped_devices(rows: Vec<LegacyShapedDevice>) -> ConfigShapedDevices {
    let mut shaped_devices = ConfigShapedDevices::default();
    shaped_devices.replace_with_new_data(rows.into_iter().map(config_shaped_device).collect());
    shaped_devices
}

fn debug_graph_output_enabled() -> bool {
    env::var_os("LIBREQOS_UISP_DEBUG_GRAPH").is_some()
}

/// The node a circuit's `parent_node` (name-joined) must land on. A link/station
/// radio is attachment-only (not in `parents`); a circuit whose serving device
/// is a link resolves across it to the serving node (else it would be a dangling
/// `parent_node` — `divergent_parent_nodes`, zero live data).
fn resolved_anchor_node_id(
    device_id: &str,
    link_serving_node: &HashMap<String, String>,
) -> String {
    let node_id = format!("uisp:device:{device_id}");
    link_serving_node
        .get(&node_id)
        .cloned()
        .unwrap_or(node_id)
}

fn anchor_for_site_ap(
    site_id: &str,
    site_name: &str,
    anchor_node_id: String,
    parents: &HashMap<String, NetJsonParent<'_>>,
) -> Option<CircuitAnchor> {
    let anchor = parents.get(&anchor_node_id)?;
    Some(CircuitAnchor {
        circuit_id: site_id.to_owned(),
        circuit_name: Some(site_name.to_owned()),
        anchor_node_id,
        anchor_node_name: Some(anchor.node_name.clone()),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum ResolvedRootSite {
    Existing(String),
    GeneratedInternet,
}

impl ResolvedRootSite {
    fn name(&self) -> String {
        match self {
            ResolvedRootSite::Existing(name) => name.clone(),
            ResolvedRootSite::GeneratedInternet => GENERATED_INTERNET_ROOT_NAME.to_string(),
        }
    }
}

fn generated_internet_root() -> GraphMapping {
    GraphMapping::Root {
        name: GENERATED_INTERNET_ROOT_NAME.to_string(),
        id: GENERATED_INTERNET_ROOT_ID.to_string(),
        latitude: None,
        longitude: None,
    }
}

fn resolve_root_site_from_names(
    configured_site: &str,
    site_names: &[String],
    internet_site_candidates: &[String],
    require_configured_root_site: bool,
) -> Result<ResolvedRootSite, UispIntegrationError> {
    let configured_site = configured_site.trim();
    if !configured_site.is_empty() {
        if site_names.iter().any(|site_name| site_name == configured_site) {
            info!("Using root UISP site from /etc/lqos.conf: {configured_site}");
            return Ok(ResolvedRootSite::Existing(configured_site.to_string()));
        }

        let mut sorted_site_names = site_names.to_vec();
        sorted_site_names.sort_unstable();
        let sample_sites = sorted_site_names
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if require_configured_root_site {
            tracing::error!(
                "Configured UISP root site '{configured_site}' was not found. Available non-client site sample: {sample_sites}"
            );
            return Err(UispIntegrationError::NoRootSite(configured_site.to_string()));
        }

        warn!(
            "Configured UISP root site '{configured_site}' was not found. Non-full topology mode will auto-detect a root instead."
        );
    } else {
        warn!("Root UISP site is blank; attempting to auto-detect an import root.");
    }

    let mut candidates = internet_site_candidates
        .iter()
        .filter(|candidate| site_names.iter().any(|site_name| site_name == *candidate))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();

    match candidates.len() {
        0 => {
            tracing::error!("Unable to auto-detect a UISP root site from Internet data-links.");
            Err(UispIntegrationError::NoRootSite(
                "no Internet-connected UISP sites were found".to_string(),
            ))
        }
        1 => {
            info!(
                "Auto-detected root UISP site from Internet data-links: {}",
                candidates[0]
            );
            Ok(ResolvedRootSite::Existing(candidates.remove(0)))
        }
        _ => {
            warn!(
                "Multiple Internet-connected UISP sites detected. Creating a generated Internet root."
            );
            Ok(ResolvedRootSite::GeneratedInternet)
        }
    }
}

fn resolve_root_site(
    config: &Config,
    uisp_data: &UispData,
    require_configured_root_site: bool,
) -> Result<ResolvedRootSite, UispIntegrationError> {
    let mut site_names = uisp_data
        .sites_raw
        .iter()
        .filter(|site| !site.is_client_site())
        .map(|site| site.name_or_blank())
        .collect::<Vec<_>>();
    site_names.sort_unstable();
    site_names.dedup();

    let mut internet_site_ids = uisp_data
        .data_links_raw
        .iter()
        .filter(|link| !link.can_delete)
        .filter_map(|link| {
            link.from
                .site
                .as_ref()
                .map(|site| site.identification.id.clone())
        })
        .collect::<Vec<_>>();
    internet_site_ids.sort_unstable();
    internet_site_ids.dedup();

    let mut internet_site_candidates = uisp_data
        .sites_raw
        .iter()
        .filter(|site| internet_site_ids.iter().any(|site_id| site_id == &site.id))
        .map(|site| site.name_or_blank())
        .collect::<Vec<_>>();
    internet_site_candidates.sort_unstable();
    internet_site_candidates.dedup();

    resolve_root_site_from_names(
        &config.uisp_integration.site,
        &site_names,
        &internet_site_candidates,
        require_configured_root_site,
    )
}

async fn build_imported_full2_bundle_from_data(
    config: Arc<Config>,
    uisp_data: UispData,
    require_configured_root_site: bool,
    build_started: Instant,
    latch: &mut orphan_latch::OrphanLatch,
    poll_unix: u64,
) -> Result<ImportedTopologyBundle, UispIntegrationError> {
    let ethernet_policy = EthernetPortLimitPolicy::from(&config.integration_common);

    let bandwidth_overrides =
        crate::strategies::legacy_bandwidth_overrides::get_site_bandwidth_overrides(&config)?;
    let routing_overrides =
        crate::strategies::legacy_routes_override::get_route_overrides(&config)?;

    // Create a new graph
    let mut graph = GraphType::new();

    // Find the root
    let resolved_root_site =
        resolve_root_site(&config, &uisp_data, require_configured_root_site)?;
    let root_site_name = resolved_root_site.name();

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
    let root_idx = match (root_idx, resolved_root_site) {
        (Some(root_idx), _) => root_idx,
        (None, ResolvedRootSite::GeneratedInternet) => graph.add_node(generated_internet_root()),
        (None, ResolvedRootSite::Existing(root_site_name)) => {
            tracing::error!("Root UISP site '{root_site_name}' was not added to the graph.");
            return Err(UispIntegrationError::NoRootSite(root_site_name));
        }
    };
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
    // §`lqos-74t.11` G2: key the no-path latch by the ROOT identity — a changed
    // root is a full reload / import-identity change and clears the latch (NOT
    // the ~3 s dataplane generation).
    latch.set_root(&graph[root_idx].network_json_id());
    // §`lqos-74t.11` G2: stamp the poll's wall-clock second (the persisted
    // `last_run_unix`; the time-based hold-off key).
    latch.set_run_unix(poll_unix);
    // Resolve a latched `parent_id` (a `network_json_id`) back to its graph node.
    let id_to_ref: HashMap<String, NodeIndex> = graph
        .node_indices()
        .map(|n| (graph[n].network_json_id(), n))
        .collect();
    // The set of nodes root-reachable in the PRE-attach graph (before any
    // orphans/deferral edges are added). A latched parent is only a valid
    // deferral target when it is reachable from its OWN edges — checking the
    // live graph would count the orphans edges added earlier in this loop and
    // make the decision iteration-order dependent.
    let root_reachable: HashSet<NodeIndex> = {
        let mut reachable = HashSet::new();
        let mut stack = vec![root_idx];
        while let Some(n) = stack.pop() {
            if reachable.insert(n) {
                for m in graph.neighbors_directed(n, petgraph::Outgoing) {
                    stack.push(m);
                }
            }
        }
        reachable
    };
    for (site_id, site_ref) in site_map.iter() {
        if *site_ref == root_idx {
            continue;
        }
        let node_id = graph[*site_ref].network_json_id();
        let a_star_run =
            petgraph::algo::astar(&graph, *site_ref, |n| n.id() == root_idx.id(), |_| 0, |_| 0);

        if a_star_run.is_none() {
            // §`lqos-74t.11` BOUNDED DIAGNOSTIC (behavior-neutral; not gated):
            // name WHICH input edge is missing for the no-path site, so a
            // transient UISP input gap is distinguishable from an
            // order-dependent orphan attach. Reports the site's raw child DEVICE
            // rows (id, present-in-`device_map`, parsed) + data links incident to
            // those devices (device-level, the only kind `add_device_links_to_graph`
            // wires) + links incident to the SITE itself (site-level, currently
            // UNWIRED) + the raw counts across polls.
            let child_device_ids: Vec<&str> = uisp_data
                .devices_raw
                .iter()
                .filter(|d| {
                    d.identification
                        .site
                        .as_ref()
                        .is_some_and(|s| &s.id == site_id)
                })
                .map(|d| d.identification.id.as_str())
                .collect();
            let child_device_states: Vec<(&str, bool, bool)> = child_device_ids
                .iter()
                .map(|cid| {
                    (
                        *cid,
                        device_map.contains_key(*cid),
                        uisp_data.find_parsed_device_by_id(cid).is_some(),
                    )
                })
                .collect();
            let mut device_incident_links = 0usize;
            let mut site_incident_links = 0usize;
            for link in uisp_data.data_links_raw.iter() {
                let from_dev = link
                    .from
                    .device
                    .as_ref()
                    .map(|d| d.identification.id.as_str());
                let to_dev = link
                    .to
                    .device
                    .as_ref()
                    .map(|d| d.identification.id.as_str());
                if from_dev.is_some_and(|f| child_device_ids.contains(&f))
                    || to_dev.is_some_and(|t| child_device_ids.contains(&t))
                {
                    device_incident_links += 1;
                }
                let from_site = link
                    .from
                    .site
                    .as_ref()
                    .map(|s| s.identification.id.as_str());
                let to_site = link
                    .to
                    .site
                    .as_ref()
                    .map(|s| s.identification.id.as_str());
                if from_site == Some(site_id.as_str()) || to_site == Some(site_id.as_str()) {
                    site_incident_links += 1;
                }
            }
            warn!(
                event = "ORPHANS_CHILD_DETAIL",
                site_id = %site_id,
                child_devices = child_device_ids.len(),
                child_device_states = ?child_device_states,
                device_incident_data_links = device_incident_links,
                site_incident_data_links = site_incident_links,
                raw_devices = uisp_data.devices_raw.len(),
                raw_data_links = uisp_data.data_links_raw.len(),
                "no-path site detail: child devices (id, in_device_map, parsed) + incident \
                 data links (device-level vs site-level) + raw counts — localizes a transient \
                 UISP input gap / an unwired site-level link vs an order-dependent attach"
            );
            // §`lqos-74t.11` G2: a site is attached to `orphans` only after K
            // consecutive no-path polls. While `count < K`, DEFER to the latched
            // parent IF it is still present and root-reachable; else orphan.
            let deferred = latch
                .note_no_path(&node_id, poll_unix)
                .and_then(|parent_id| id_to_ref.get(&parent_id).copied())
                .filter(|pref| root_reachable.contains(pref));
            let decision = if deferred.is_some() {
                "defer to the latched parent"
            } else {
                "attach to the orphans umbrella"
            };
            // DIAGNOSTIC PROBE (not gated): the no-path site whose
            // root-connectivity flaps (the `lqos-74t.3` real root). It now names
            // the deferral decision so a census sees orphan-vs-defer.
            warn!(
                event = "ORPHANS_CHILD",
                site_id = %node_id,
                site_name = %graph[*site_ref].name(),
                root = %root_site_name,
                deferred = deferred.is_some(),
                "no path to root (the churn toggle child) — lqos-74t.11 G2: {decision}"
            );
            if let Some(pref) = deferred {
                warn!(
                    event = "ORPHANS_CHILD_DEFERRED",
                    site_id = %node_id,
                    latched_parent = %graph[pref].network_json_id(),
                    "lqos-74t.11 G2: no root path for one poll (< K=2) — deferring the \
                     orphans attach to the latched parent"
                );
                graph.add_edge(
                    *site_ref,
                    pref,
                    LinkMapping::ethernet(config.queues.generated_pn_download_mbps),
                );
                graph.add_edge(
                    pref,
                    *site_ref,
                    LinkMapping::ethernet(config.queues.generated_pn_upload_mbps),
                );
            } else {
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
        } else {
            // A site WITH a root path resets its no-path counter.
            latch.note_path(&node_id);
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

    // Cull the APs that have no clients or only one link — EXCEPT a role-keyed
 // link/station radio (the implementer `#12` R5). The cull is structural /
    // client-count; the link decision is ROLE-keyed and must never depend on a
    // node this cull removes. A clientless single-link PtMP station client would
    // otherwise be removed before the attachment decision, so it could never be
    // its site's `current_attachment_id` (the live `.12` "Wave Long-Range absent,
    // attachment null" symptom). Keep it in the graph as the attachment device;
    // the node-emission skip below keeps it out of the tree.
    let mut to_remove = Vec::new();
    for (ap_id, ap_ref) in device_map.iter() {
        if uisp_data
            .find_parsed_device_by_id(ap_id)
            .is_some_and(device_is_link)
        {
            continue;
        }
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
    // `docs/topology_ptp_link_site_attachment_work_order.md` §5: a LINK/PTP
    // device that is a site's attachment is NOT a tree node (the site attaches
    // to the serving AP; the link is only the attachment). Keyed on the device
    // ROLE (`device_is_link`), never structure — a chained AP is structurally
 // identical (the implementer `#12` PIN-1).
    // `docs/topology_ptp_link_site_attachment_work_order.md` (DRIVER): EVERY
    // PTP/backhaul LINK device (by ROLE) is an ATTACHMENT — NOT a node of any
    // kind (no `ap`, no `site`) — regardless of POSITION (root-level or nested)
    // or whether it carries clients. It exists only as its dependent node's
    // `current_attachment_id`.
    let link_only_attachments: HashSet<NodeIndex> = graph
        .node_indices()
        .filter(|&node| {
            access_point_device(&graph, node, &uisp_data.devices).is_some_and(device_is_link)
        })
        .collect();
    // Shaping join (§3): a link/station radio is attachment-only, so it is NOT
    // in `parents`. A circuit whose anchor resolves to a link must instead join
    // the SERVING node across it — otherwise the anchor name matches no store
    // node (`divergent_parent_nodes` ⇒ zero live data). Map link node id to the
    // serving node id once, here, and reuse it for the anchor lookups below.
    let mut link_serving_node: HashMap<String, String> = HashMap::new();
    for &link in &link_only_attachments {
        if let Some(serving) = nearest_serving_node(&graph, root_idx, link, &mut route_cache) {
            link_serving_node.insert(
                graph[link].network_json_id(),
                graph[serving].network_json_id(),
            );
        }
    }
    for node in graph.node_indices() {
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
                        // `docs/topology_ptp_link_site_attachment_work_order.md`:
                        // a SITE's TREE parent is the LOGICAL parent — a site
                        // whose attachment is a LINK/PTP device resolves to the
                        // serving AP (the link stays the ATTACHMENT); direct
                        // site→site children keep their site parent.
                        let mut tree_parent = if matches!(graph[node], GraphMapping::Site { .. }) {
                            native_parent
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
                                .unwrap_or(parent_node)
                        } else {
                            parent_node
                        };
                        // PIN-2: never emit a parent_id that the link SKIP drops
                        // — walk UP across EVERY transparent link to the serving
                        // node (a single hop is not enough for a PTP/PTP chain).
                        while link_only_attachments.contains(&tree_parent) {
                            let up = route_cache.route(&graph, root_idx, tree_parent);
                            match immediate_parent_from_route(&up) {
                                Some(parent) if parent != tree_parent => tree_parent = parent,
                                _ => break,
                            }
                        }
                        let mut parent_id = graph[tree_parent].network_json_id();
                        let mut parent = graph[tree_parent].name();
                        let mut parent_export_name = export_names
                            .get(&parent_id)
                            .cloned()
                            .unwrap_or_else(|| parent.clone());
                        // L1 (`lqos-74t.63`): hold the FIRST-selected candidate
                        // among the site's currently-VALID candidate parents,
                        // suppressing a flap between two VALID redundant backhaul
                        // links. A UISP backhaul-parent oscillation re-selects on a
                        // STABLE id ⇒ a genuine span/owner-set change ⇒ the bridge's
                        // R3 cold rebuild (which drops residual backlog) every ~10
                        // min. Only a real SITE is pinned (a GeneratedSite is pinned
                        // to the root above; an AP keeps its logical resolution). The
                        // emitted rate stays the derived route's — the candidates are
                        // redundant equivalents and an operator capacity override
                        // still wins.
                        if matches!(graph[node], GraphMapping::Site { .. }) {
                            let valid_logical_parents: BTreeSet<String> = grouped_allowed_parents
                                .iter()
                                .map(|group| graph[group.logical_parent].network_json_id())
                                .chain(std::iter::once(parent_id.clone()))
                                .collect();
                            let pinned_id =
                                latch.pin_parent(&node_id, &parent_id, &valid_logical_parents);
                            if pinned_id != parent_id
                                && let Some(&pinned_ref) = id_to_ref.get(&pinned_id)
                                && is_real_topology_node(&graph, pinned_ref)
                            {
                                warn!(
                                    event = "TOPOLOGY_PARENT_PINNED",
                                    site_id = %node_id,
                                    derived_parent = %parent_id,
                                    pinned_parent = %pinned_id,
                                    "lqos-74t.63 L1: holding the first-selected candidate — \
                                     suppressing a re-parent between two VALID redundant \
                                     backhaul candidates"
                                );
                                tree_parent = pinned_ref;
                                parent_id = graph[pinned_ref].network_json_id();
                                parent = graph[pinned_ref].name();
                                parent_export_name = export_names
                                    .get(&parent_id)
                                    .cloned()
                                    .unwrap_or_else(|| parent.clone());
                            }
                        }
                        let current_parent = if is_real_topology_node(&graph, parent_node) {
                            Some(TopologyParentCandidate {
                                node_id: graph[parent_node].network_json_id(),
                                node_name: graph[parent_node].name(),
                            })
                        } else {
                            None
                        };
                        // The operator view's parent == the TREE parent for a
                        // SITE; non-sites keep the existing logical resolution.
                        let logical_current_parent = if matches!(graph[node], GraphMapping::Site { .. })
                        {
                            Some(TopologyParentCandidate {
                                node_id: parent_id.clone(),
                                node_name: parent.clone(),
                            })
                        } else if matches!(graph[node], GraphMapping::GeneratedSite { .. }) {
                            // ROOT A (`docs/broker_span_changing_move_root_card.md`):
                            // a SYNTHETIC umbrella (the `orphans` GeneratedSite) must
                            // have a DETERMINISTIC, STABLE parent — the ROOT — and must
                            // NEVER re-parent (jackrabbit's orphans flipped
                            // Medano Heights Apt 2 <-> Apt 3, a produced move with no
                            // operator intent). Pin it; ignore the route/logical
                            // resolution (which the UISP topology can flip).
                            Some(TopologyParentCandidate {
                                node_id: graph[root_idx].network_json_id(),
                                node_name: graph[root_idx].name(),
                            })
                        } else {
                            native_parent
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
                                })
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
                        let allowed_parents = topology_allowed_parents_from_groups_cached(
                            &graph,
                            &grouped_allowed_parents,
                            &aps_with_clients,
                            &mut route_cache,
                        );

                        // A site's LINK/PTP attachment is ATTACHMENT-ONLY: it is
                        // NOT a canonical/operator node (neither `ap` nor
                        // `site`) — it exists solely as the site's
                        // `current_attachment_id`. Emitting it here (while it is
                        // skipped from `parents`) made the canonical DEFAULT its
                        // kind to `site` (the live regression).
                        if is_real_topology_node(&graph, node)
                            && !link_only_attachments.contains(&node)
                        {
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
                                latitude: graph[node].latitude(),
                                longitude: graph[node].longitude(),
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
                        // A site's LINK/PTP ATTACHMENT is not a tree node: the
                        // site is re-parented to the serving AP, so the link has
                        // no children and must not appear as a level. (It stays
                        // in the EDITOR state as the site's attachment.)
                        if !link_only_attachments.contains(&node) {
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
                        }
                    } else {
                        panic!("DID NOT FIND THE EDGE");
                    }
                }
            }
            GraphMapping::Root { name, .. } => {
                topology_editor_nodes.push(TopologyEditorNode {
                    node_id: graph[node].network_json_id(),
                    node_name: name.to_owned(),
                    latitude: graph[node].latitude(),
                    longitude: graph[node].longitude(),
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                });
            }
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

 // the synthetic `Orphans` parent must be EMITTED as a real
    // root child. When a path-less site is attached to it, the canonical-state
    // write otherwise rejects the WHOLE import (`dangling parent
    // libreqos:generated:uisp:site:orphans`) and no topology ever publishes.
    // Force its entry to the root so the reference always resolves.
    parents.insert(
        orphans_node_id.clone(),
        NetJsonParent {
            node_id: orphans_node_id.clone(),
            node_name: orphans_node_name.clone(),
            export_name: orphans_node_name.clone(),
            parent_id: Some(root_node_id.clone()),
            parent_name: root_site_name.clone(),
            mapping: &graph[orphans],
            download: config.queues.generated_pn_download_mbps,
            upload: config.queues.generated_pn_upload_mbps,
        },
    );
    // §`lqos-74t.11` G2: record each present SITE's current (non-orphan) parent so
    // a future one-poll no-path can defer to it. Orphans-placed sites are skipped
    // (a genuine orphan must never become a deferral target); a site that regains
    // its path refreshes its remembered parent here.
    for parent in parents.values() {
        if matches!(
            parent.mapping,
            GraphMapping::Site { .. } | GraphMapping::GeneratedSite { .. }
        ) && let Some(parent_id) = parent.parent_id.as_deref()
            && parent_id != orphans_node_id.as_str()
        {
            latch.remember_parent(&parent.node_id, parent_id);
        }
    }
    // The canonical snapshot is built from `topology_editor_nodes` (via
    // `from_editor_and_network`), NOT from `parents`/network.json alone. A
    // path-less site references the synthetic Orphans umbrella as its parent,
    // so the umbrella must ALSO be an editor node — otherwise the canonical
    // validator rejects the whole import with
    // `node <site> has dangling parent libreqos:generated:uisp:site:orphans`.
 // The fix covered `parents`; this covers the editor/canonical
    // representation the same fact has a second home in.
    if !topology_editor_nodes
        .iter()
        .any(|node| node.node_id == orphans_node_id)
    {
        topology_editor_nodes.push(TopologyEditorNode {
            node_id: orphans_node_id.clone(),
            node_name: orphans_node_name.clone(),
            latitude: graph[orphans].latitude(),
            longitude: graph[orphans].longitude(),
            current_parent_node_id: Some(root_node_id.clone()),
            current_parent_node_name: Some(root_site_name.clone()),
            current_attachment_id: None,
            current_attachment_name: None,
            can_move: false,
            allowed_parents: Vec::new(),
            preferred_attachment_id: None,
            preferred_attachment_name: None,
            effective_attachment_id: None,
            effective_attachment_name: None,
        });
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
    topology_parent_candidates.sort_unstable_by(|left, right| left.node_id.cmp(&right.node_id));
    let topology_candidates_file = TopologyParentCandidatesFile {
        source: "uisp/full".to_string(),
        ingress_identity: None,
        nodes: topology_parent_candidates,
    };
    topology_editor_nodes.sort_unstable_by(|left, right| left.node_id.cmp(&right.node_id));
    let topology_editor_state = TopologyEditorStateFile {
        schema_version: 1,
        source: "uisp/full2".to_string(),
        generated_unix: now_unix(),
        ingress_identity: None,
        nodes: topology_editor_nodes,
    };
    let exported_uisp_device_ids: HashSet<&str> = topology_editor_state
        .nodes
        .iter()
        .filter_map(|node| node.node_id.strip_prefix("uisp:device:"))
        .collect();
    let canonical_state = TopologyCanonicalStateFile::from_editor_and_network(
        &topology_editor_state,
        &Value::Object(network_json.clone()),
        TopologyCanonicalIngressKind::NativeIntegration,
    );
    let export_elapsed_ms = export_started.elapsed().as_millis();

    // Shaped Devices
    let shaped_devices_started = Instant::now();
    let mut shaped_devices = Vec::new();
    let mut circuit_anchors = Vec::<CircuitAnchor>::new();
    let mut seen_pairs = HashSet::new();
    let mut seen_circuits = HashSet::new();
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
 // 3b rate fix(ii)): a plan-less site takes its
                    // rate from the CLIENT devices' UISP service plan — NEVER
                    // the whole-network `generated_pn_*` sentinel (the 100G
                    // "unlimited PN", appropriate only for the parent SITE/PN
                    // node). The compiler leaf clamp is the backstop when even
                    // the device plan is absent.
                    let (base_down, base_up) =
                        planless_client_base_mbps(site, &site_devices);
                    let download_f32 =
                        base_down * config.uisp_integration.bandwidth_overhead_factor;
                    let upload_f32 =
                        base_up * config.uisp_integration.bandwidth_overhead_factor;
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
            if !site_devices.is_empty() && seen_circuits.insert(site.id.clone()) {
                let anchor_node_id = resolved_anchor_node_id(&ap_device.id, &link_serving_node);
                if let Some(anchor) =
                    anchor_for_site_ap(&site.id, &site.name, anchor_node_id, &parents)
                {
                    circuit_anchors.push(anchor);
                } else {
                    debug!(
                        "Skipping circuit anchor for '{}' because AP '{}' ({}) is not present in topology; circuit will resolve from ingress parent data.",
                        site.name, ap_device.name, ap_device.id
                    );
                }
            }
            for device in uisp_data.find_parsed_devices_in_site(site_id) {
                if !device.has_address() {
                    continue;
                }

                let parent_node = {
                    let ap_node_id = resolved_anchor_node_id(&ap_device.id, &link_serving_node);
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
                let parent_node_id = {
                    let ap_node_id = resolved_anchor_node_id(&ap_device.id, &link_serving_node);
                    parents
                        .get(&ap_node_id)
                        .map(|parent| parent.node_id.clone())
                        .unwrap_or_default()
                };

                let key = (site.id.clone(), device.id.clone());
                if !seen_pairs.insert(key) {
                    continue;
                }

                let shaped_device = LegacyShapedDevice {
                    circuit_id: site.id.to_owned(),
                    circuit_name: site.name.to_owned(),
                    device_id: device.id.to_owned(),
                    device_name: device.name.to_owned(),
                    parent_node: parent_node.clone(),
                    parent_node_id,
                    anchor_node_id: String::new(),
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
    info!(
        shaped_devices = shaped_devices.len(),
        shaped_device_elapsed_ms = shaped_devices_started.elapsed().as_millis(),
        export_elapsed_ms,
        total_elapsed_ms = build_started.elapsed().as_millis(),
        "Completed UISP full2 import bundle"
    );

    ethernet_advisories.extend(
        uisp_data
            .devices
            .iter()
            .filter(|device| exported_uisp_device_ids.contains(device.id.as_str()))
            .filter_map(topology_node_ethernet_advisory),
    );

    Ok(ImportedTopologyBundle {
        source: "uisp/full2".to_string(),
        generated_unix: topology_editor_state.generated_unix,
        ingress_identity: None,
        native_canonical: Some(canonical_state),
        parent_candidates: Some(topology_candidates_file),
        compatibility_network_json: Value::Object(network_json),
        shaped_devices: config_shaped_devices(shaped_devices),
        circuit_anchors: CircuitAnchorsFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: now_unix(),
            anchors: circuit_anchors,
        },
        ethernet_advisories,
        circuit_matches: Default::default(),
    })
}

pub async fn build_imported_full2_bundle(
    config: Arc<Config>,
    ip_ranges: IpRanges,
    require_configured_root_site: bool,
) -> Result<ImportedTopologyBundle, UispIntegrationError> {
    let build_started = Instant::now();
    let uisp_data = UispData::fetch_uisp_data(config.clone(), ip_ranges).await?;
    // §`lqos-74t.11` G2: the no-path latch lives in ONE poll-scoped instance,
    // loaded here and persisted after a successful build. The integration runs
    // as a FRESH subprocess per poll, so the state MUST be file-backed.
    let mut latch = orphan_latch::OrphanLatch::load(&config);
    // One wall-clock second per poll (the time-based hold-off key). The public
    // wrapper is the ONLY non-test caller; tests inject a fixed value.
    let now_unix = now_unix().unwrap_or(0);
    let result = build_imported_full2_bundle_from_data(
        config,
        uisp_data,
        require_configured_root_site,
        build_started,
        &mut latch,
        now_unix,
    )
    .await;
    if result.is_ok() {
        latch.prune();
        latch.save();
    }
    result
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
        if device_map.contains_key(&device.identification.id) {
            warn!(
                device_id = %device.identification.id,
                device_name = %device.get_name().unwrap_or_default(),
                "Skipping duplicate UISP device row while building topology graph"
            );
            continue;
        }
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

/// TRUE when a UISP device is a LINK/PTP device (the station side of a
/// point-to-point backhaul) rather than a serving Access Point. Keyed on the
/// device ROLE / wireless mode (the implementer `#12` PIN-1) — never structure, since a
/// chained AP is structurally identical to a link.
fn device_is_link(device: &UispDevice) -> bool {
    let role = device.role.as_deref().unwrap_or("").to_ascii_lowercase();
    let wireless = device
        .wireless_mode
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    // `docs/topology_ptmp_ptp_attachment_card.md` §2 (the DRIVER's general
    // rule): a radio that is a LINK / STATION is ATTACHMENT-ONLY. Two shapes,
    // ONE predicate:
    //   (a) a PtMP station client / CPE — `role ∈ {station, sta, cpe}` or a
    //       `sta*`/`station*` wireless mode (`sta-ptmp`, `sta-ptp`) —
    //       unified with the existing `is_wireless_station_cpe` classifier;
    //   (b) a PTP / backhaul radio, EITHER end (`ap-ptp` OR `sta-ptp`) —
    //       `role`/`wireless_mode` carries `ptp` (or a `backhaul` role).
    // A sector AP (`role == ap`, wireless `ap-ptmp`) is NOT a link and STAYS a
    // node; `ap-ptmp`/`sta-ptmp` do NOT contain `ptp`, so (b) cannot eat them.
    // Never key on structure, `aps_with_clients`, or a derived kind.
    if device.is_wireless_station_cpe()
        || role.contains("ptp")
        || role.contains("backhaul")
        || wireless.contains("ptp")
    {
        return true;
    }
    // §2bis (the DRIVER's steer): an OPAQUE radio (`role=wireless`, blackBox, no
    // decisive mode) is classified by the CONNECTED SITE TYPE (`classifies_as_link`,
    // set in `UispData::from_parts`): own site endpoint => CPE/attachment; own
    // site infra + connects an endpoint => AP/node; own site infra + infra<->infra
    // => backhaul/attachment. This closes the `4600C_*`/`AVIAT_*` opaque bucket.
    if role == "wireless" {
        return device.classifies_as_link;
    }
    false
}

/// The UISP device behind a graph `AccessPoint` node (via the device list).
fn access_point_device<'a>(
    graph: &GraphType,
    node: NodeIndex,
    devices: &'a [UispDevice],
) -> Option<&'a UispDevice> {
    let GraphMapping::AccessPoint { id, .. } = &graph[node] else {
        return None;
    };
    devices.iter().find(|device| device.id == *id)
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

fn remote_logical_parent_for_local_attachment_cached(
    graph: &GraphType,
    root_idx: NodeIndex,
    node_being_edited: NodeIndex,
    candidate: NodeIndex,
    route_cache: &mut RouteCache<'_>,
) -> Option<(NodeIndex, NodeIndex)> {
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
                // `(nearest_site, serving_ap)`: the caller picks by the
                // attachment's ROLE — a LINK/PTP device resolves to the serving
                // AP (`peer`); a serving/chained AP keeps the nearest site.
                (logical_parent, peer),
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

/// TRUE when the graph node is a role-keyed link/station radio (an
/// ATTACHMENT-ONLY device, never a node). Never structural.
fn is_transparent_link(graph: &GraphType, node: NodeIndex, devices: &[UispDevice]) -> bool {
    access_point_device(graph, node, devices).is_some_and(device_is_link)
}

/// The nearest REAL topology node at or above `start` on the root route that
/// is NOT a link/station radio (walking ACROSS transparent links). A link's
/// "serving" node is the sector AP / switch / site beyond it — so a link's
/// dependent node never parents to the link (which is not a node at all).
fn nearest_serving_node(
    graph: &GraphType,
    root_idx: NodeIndex,
    start: NodeIndex,
    route_cache: &mut RouteCache<'_>,
) -> Option<NodeIndex> {
    let path = route_cache.route(graph, root_idx, start);
    path.iter()
        .rev()
        .copied()
        .find(|node| {
            is_real_topology_node(graph, *node)
                && !is_transparent_link(graph, *node, route_cache.devices)
        })
}

fn logical_parent_for_candidate_cached(
    graph: &GraphType,
    root_idx: NodeIndex,
    node_being_edited: NodeIndex,
    candidate: NodeIndex,
    route_cache: &mut RouteCache<'_>,
) -> NodeIndex {
    match &graph[candidate] {
        GraphMapping::Root { .. } | GraphMapping::Site { .. } => candidate,
        GraphMapping::GeneratedSite { .. } => candidate,
        GraphMapping::AccessPoint { .. } => {
            // `docs/topology_ptmp_ptp_attachment_card.md` §3.4 (DRIVER): a
            // link/station radio is TRANSPARENT — a node whose attachment is a
            // link parents to the SERVING node ACROSS it (the sector AP, a
            // switch, or a site), NEVER to the link (it is not a node). Keyed on
            // the device ROLE, never device-under-device (that would drop a
            // legitimate CHAINED AP — a chained AP is `ap-ptmp`, not a link).
            if is_transparent_link(graph, candidate, route_cache.devices) {
                // Prefer the peer across the link (the far-end serving node);
                // then fall back to the nearest real node above the link itself.
                if let Some((_, peer)) = remote_logical_parent_for_local_attachment_cached(
                    graph,
                    root_idx,
                    node_being_edited,
                    candidate,
                    route_cache,
                ) && let Some(serving) =
                    nearest_serving_node(graph, root_idx, peer, route_cache)
                {
                    return serving;
                }
                if let Some(serving) = nearest_serving_node(graph, root_idx, candidate, route_cache)
                {
                    return serving;
                }
                return candidate;
            }
            // A non-link device (a serving/chained AP, a switch, a router) is
            // the logical parent itself (the FULL MODEL the DRIVER ruled).
            candidate
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
            let mut attachment_options = vec![topology_auto_attachment_option()];
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
        build_imported_full2_bundle_from_data, device_is_link,
        export_parent_anchor_for_override, first_probe_ip_for_device, immediate_parent_from_route,
        is_upstream_parent_candidate, logical_parent_for_candidate,
        orphan_latch::ORPHAN_LATCH_HOLD_OFF_SECONDS, orphan_latch::OrphanLatch,
        planless_client_base_mbps,
        resolve_attachment_parent_candidate, resolve_parent_candidate,
        topology_allowed_parent_groups_for_node, topology_allowed_parents_from_groups,
        topology_parent_candidates_for_node,
    };
    use crate::strategies::common::UispData;
    use crate::strategies::full2::graph_mapping::GraphMapping;
    use crate::strategies::full2::link_mapping::LinkMapping;
    use crate::uisp_types::{UispAttachmentRateSource, UispSite, UispSiteType};
    use lqos_config::TopologyAttachmentRole;
    use lqos_overrides::{TopologyAttachmentMode, TopologyParentOverrideMode};
    use serde_json::{Map, Value, json};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use uisp::{DataLink, Device, Site};

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

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "libreqos-uisp-full2-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("temp dir should be created");
        path
    }

    fn raw_site(id: &str, name: &str, site_type: &str) -> Site {
        serde_json::from_value(json!({
            "id": id,
            "identification": {
                "id": id,
                "name": name,
                "type": site_type,
                "suspended": false
            }
        }))
        .expect("raw site fixture should deserialize")
    }

    fn raw_device(id: &str, name: &str, site_id: &str) -> Device {
        serde_json::from_value(json!({
            "identification": {
                "id": id,
                "hostname": name,
                "site": {
                    "id": site_id
                }
            }
        }))
        .expect("raw device fixture should deserialize")
    }

    struct LinkEndpoint<'a> {
        site_id: &'a str,
        site_name: &'a str,
        device_id: &'a str,
        device_name: &'a str,
    }

    fn data_link_endpoint(endpoint: &LinkEndpoint<'_>) -> serde_json::Value {
        json!({
            "site": {
                "identification": {
                    "id": endpoint.site_id,
                    "name": endpoint.site_name
                }
            },
            "device": {
                "identification": {
                    "id": endpoint.device_id,
                    "name": endpoint.device_name
                }
            }
        })
    }

    fn device_data_link(id: &str, from: LinkEndpoint<'_>, to: LinkEndpoint<'_>) -> DataLink {
        serde_json::from_value(json!({
            "id": id,
            "from": data_link_endpoint(&from),
            "to": data_link_endpoint(&to),
            "canDelete": true
        }))
        .expect("data link fixture should deserialize")
    }

    fn parsed_site(id: &str, name: &str) -> UispSite {
        UispSite {
            id: id.to_string(),
            name: name.to_string(),
            site_type: UispSiteType::Site,
            max_down_mbps: 10_000,
            max_up_mbps: 10_000,
            ..Default::default()
        }
    }

    fn parsed_device(id: &str, name: &str, site_id: &str) -> UispDevice {
        UispDevice {
            id: id.to_string(),
            name: name.to_string(),
            mac: String::new(),
            role: None,
            wireless_mode: None,
            site_id: site_id.to_string(),
            raw_download: 10_000,
            raw_upload: 10_000,
            download: 10_000,
            upload: 10_000,
            ipv4: HashSet::new(),
            ipv6: HashSet::new(),
            probe_ipv4: HashSet::new(),
            probe_ipv6: HashSet::new(),
            negotiated_ethernet_mbps: None,
            negotiated_ethernet_interface: None,
            transport_cap_line_rate_mbps: None,
            transport_cap_interface: None,
            transport_cap_mbps: None,
            transport_cap_reason: None,
            attachment_rate_source: UispAttachmentRateSource::Static,
            classifies_as_link: false,
        }
    }

 /// 3b rate fix(ii), the plan-less-client oracle): a site carrying
    /// the whole-network PN sentinel but a client device plan uses the DEVICE
    /// plan, never the generated-PN 100G.
    #[test]
    fn planless_client_uses_the_device_plan_not_the_pn_sentinel() {
        let mut site = parsed_site("site-1", "Client");
        site.max_down_mbps = 100_000;
        site.max_up_mbps = 100_000;
        let mut device = parsed_device("dev-1", "CPE", "site-1");
        device.raw_download = 322;
        device.raw_upload = 322;
        let devices = vec![&device];
        assert_eq!(
            planless_client_base_mbps(&site, &devices),
            (322.0, 322.0),
            "the client device plan wins over the PN sentinel"
        );

        // Truly plan-less (no QoS, no device plan): the last-resort site
        // value stays; the compiler leaf clamp caps it to the parent.
        let mut zero = parsed_device("dev-2", "CPE2", "site-1");
        zero.raw_download = 0;
        zero.raw_upload = 0;
        let zero_devices = vec![&zero];
        assert_eq!(
            planless_client_base_mbps(&site, &zero_devices),
            (100_000.0, 100_000.0)
        );
    }

    fn find_network_node_with_parent<'a>(
        value: &'a Value,
        target_id: &str,
        parent_id: Option<&str>,
    ) -> Option<(&'a Map<String, Value>, Option<String>)> {
        let map = value.as_object()?;
        if map.get("id").and_then(Value::as_str) == Some(target_id) {
            return Some((map, parent_id.map(ToOwned::to_owned)));
        }

        let node_id = map.get("id").and_then(Value::as_str);
        map.values()
            .find_map(|child| find_network_node_with_parent(child, target_id, node_id))
    }

    fn find_network_node_by_id<'a>(
        value: &'a Value,
        target_id: &str,
    ) -> Option<&'a Map<String, Value>> {
        find_network_node_with_parent(value, target_id, None).map(|(node, _parent_id)| node)
    }

    fn find_network_parent_id(value: &Value, target_id: &str) -> Option<Option<String>> {
        find_network_node_with_parent(value, target_id, None).map(|(_node, parent_id)| parent_id)
    }

    /// Walk the nested `{id, children}` compatibility tree and return the id of
    /// the node's CONTAINER. The older `find_network_parent_id` loses the parent
    /// through the id-less `children` wrapper; this helper carries the container
    /// id correctly (needed for the §`lqos-74t.11` G2 deferral assertions).
    fn network_parent_id_of(value: &Value, target_id: &str) -> Option<Option<String>> {
        fn walk(node: &Value, target: &str, parent: Option<&str>) -> Option<Option<String>> {
            let map = node.as_object()?;
            if map.get("id").and_then(Value::as_str) == Some(target) {
                return Some(parent.map(ToOwned::to_owned));
            }
            let this_id = map.get("id").and_then(Value::as_str);
            let children = map.get("children")?.as_object()?;
            children
                .values()
                .find_map(|child| walk(child, target, this_id))
        }
        value
            .as_object()?
            .values()
            .find_map(|child| walk(child, target_id, None))
    }

    #[tokio::test]
    async fn full2_import_emits_queue_auto_for_device_backed_branches() {
        let temp_dir = unique_temp_dir("uisp-full2-queue-auto");
        let mut config = lqos_config::Config {
            lqos_directory: temp_dir.to_string_lossy().to_string(),
            state_directory: Some(temp_dir.join("state").to_string_lossy().to_string()),
            ..Default::default()
        };
        config.uisp_integration.enable_uisp = true;
        config.uisp_integration.site = "Root Site".to_string();
        config.queues.generated_pn_download_mbps = 10_000;
        config.queues.generated_pn_upload_mbps = 10_000;
        let config_for_topology = config.clone();

        let root_site = raw_site("site-root", "Root Site", "site");
        let child_site = raw_site("site-child", "Child Site", "site");
        let root_device = raw_device("device-root-switch", "Root Switch", "site-root");
        let child_device = raw_device("device-child-switch", "Child Switch", "site-child");
        let root_parsed_device = parsed_device("device-root-switch", "Root Switch", "site-root");
        let child_parsed_device =
            parsed_device("device-child-switch", "Child Switch", "site-child");
        let uisp_data = UispData::from_parts(
            vec![root_site, child_site],
            vec![root_device, child_device],
            vec![device_data_link(
                "link-root-child",
                LinkEndpoint {
                    site_id: "site-root",
                    site_name: "Root Site",
                    device_id: "device-root-switch",
                    device_name: "Root Switch",
                },
                LinkEndpoint {
                    site_id: "site-child",
                    site_name: "Child Site",
                    device_id: "device-child-switch",
                    device_name: "Child Switch",
                },
            )],
            vec![
                parsed_site("site-root", "Root Site"),
                parsed_site("site-child", "Child Site"),
            ],
            vec![root_parsed_device, child_parsed_device],
        );

        let bundle = build_imported_full2_bundle_from_data(
            Arc::new(config),
            uisp_data,
            true,
            std::time::Instant::now(),
            &mut OrphanLatch::default(),
            0,
        )
        .await
        .expect("in-memory UISP import should build");
        // Rung 1.5 §1.3: the editor view is the canonical projection.
        let editor = bundle
            .native_canonical
            .as_ref()
            .expect("full2 import should include native canonical state")
            .to_editor_state();
        let root_switch = editor
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:device:device-root-switch")
            .expect("root switch should be exported as a topology editor node");

        let mut compiled = lqos_topology_compile::compile_topology(
            bundle,
            lqos_topology_compile::TopologyCompileMode::Full,
        )
        .expect("full topology should compile from full2 import");
        let compiled_child_switch = compiled
            .editor
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:device:device-child-switch")
            .expect("compiled editor should include the child switch");
        // FULL MODEL: each node parents to its serving DEVICE (the immediate
        // device), not the flattened site.
        assert_eq!(
            compiled_child_switch.current_parent_node_id.as_deref(),
            Some("uisp:device:device-root-switch")
        );
        assert_eq!(
            compiled_child_switch.current_parent_node_name.as_deref(),
            Some("Root Switch")
        );
        assert_eq!(
            compiled_child_switch.current_attachment_id.as_deref(),
            Some("uisp:device:device-root-switch")
        );
        assert_eq!(
            compiled_child_switch.current_attachment_name.as_deref(),
            Some("Root Switch")
        );
        let compiled_child_site = compiled
            .editor
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:site:site-child")
            .expect("compiled editor should include the child site");
        // FULL MODEL: the child site parents to its serving DEVICE.
        assert_eq!(
            compiled_child_site.current_parent_node_id.as_deref(),
            Some("uisp:device:device-child-switch")
        );
        assert_eq!(
            compiled_child_site.current_parent_node_name.as_deref(),
            Some("Child Switch")
        );
        assert_eq!(
            compiled_child_site.current_attachment_id.as_deref(),
            Some("uisp:device:device-child-switch")
        );
        assert_eq!(
            compiled_child_site.current_attachment_name.as_deref(),
            Some("Child Switch")
        );
        for node in &mut compiled.canonical.nodes {
            match node.node_id.as_str() {
                "uisp:device:device-child-switch" => {
                    node.current_attachment_name = Some("stale root switch label".to_string());
                }
                "uisp:site:site-child" => {
                    node.current_attachment_name = Some("stale child switch label".to_string());
                }
                _ => {}
            }
        }

        let artifacts =
            lqos_topology::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
                &config_for_topology,
                &compiled.canonical,
                &lqos_overrides::TopologyOverridesFile::default(),
                &lqos_config::TopologyAttachmentHealthStateFile::default(),
            )
            .expect("effective topology should compile from full2 canonical output");
        let effective_network = artifacts
            .effective_network
            .as_ref()
            .expect("effective topology should include a network tree");
        assert!(
            find_network_node_by_id(effective_network, "uisp:device:device-root-switch").is_none()
        );
        let effective_child_switch = artifacts
            .effective
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:device:device-child-switch")
            .expect("effective state should retain child switch attachment metadata");
        assert_eq!(
            effective_child_switch.logical_parent_node_id,
            "uisp:device:device-root-switch"
        );
        assert_eq!(
            effective_child_switch.effective_attachment_id.as_deref(),
            Some("uisp:device:device-root-switch")
        );
        let ui_child_switch = artifacts
            .ui_state
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:device:device-child-switch")
            .expect("UI state should retain child switch attachment metadata");
        assert_eq!(
            ui_child_switch.effective_attachment_name.as_deref(),
            Some("Root Switch")
        );
        let effective_child_site = artifacts
            .effective
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:site:site-child")
            .expect("effective state should retain child site attachment metadata");
        assert_eq!(
            effective_child_site.logical_parent_node_id,
            "uisp:device:device-child-switch"
        );
        assert_eq!(
            effective_child_site.effective_attachment_id.as_deref(),
            Some("uisp:device:device-child-switch")
        );
        let ui_child_site = artifacts
            .ui_state
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:site:site-child")
            .expect("UI state should retain child site attachment metadata");
        assert_eq!(
            ui_child_site.effective_attachment_name.as_deref(),
            Some("Child Switch")
        );
        assert_eq!(
            find_network_parent_id(effective_network, "uisp:site:site-child"),
            Some(None)
        );

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    /// A path-less site is parented to the synthetic Orphans umbrella. The
    /// umbrella must exist in BOTH representations the same fact lives in:
 /// `parents`/network.json (the fix) AND the editor/canonical
    /// nodes (`from_editor_and_network`) — otherwise the canonical validator
    /// rejects the whole import (`node <site> has dangling parent
    /// libreqos:generated:uisp:site:orphans`) and no topology publishes on
    /// evolved live data.
    #[tokio::test]
    async fn full2_import_emits_the_orphans_umbrella_editor_node() {
        let temp_dir = unique_temp_dir("uisp-full2-orphans-umbrella");
        let mut config = lqos_config::Config {
            lqos_directory: temp_dir.to_string_lossy().to_string(),
            state_directory: Some(temp_dir.join("state").to_string_lossy().to_string()),
            ..Default::default()
        };
        config.uisp_integration.enable_uisp = true;
        config.uisp_integration.site = "Root Site".to_string();
        config.queues.generated_pn_download_mbps = 10_000;
        config.queues.generated_pn_upload_mbps = 10_000;

        let root_site = raw_site("site-root", "Root Site", "site");
        let orphan_site = raw_site("site-orphan", "Orphan Site", "site");
        let root_device = raw_device("device-root-switch", "Root Switch", "site-root");
        let orphan_device = raw_device("device-orphan-switch", "Orphan Switch", "site-orphan");
        let root_parsed_device = parsed_device("device-root-switch", "Root Switch", "site-root");
        let orphan_parsed_device =
            parsed_device("device-orphan-switch", "Orphan Switch", "site-orphan");
        // No data link: the orphan site has no path to the root, so the import
        // attaches it to the generated Orphans umbrella.
        let uisp_data = UispData::from_parts(
            vec![root_site, orphan_site],
            vec![root_device, orphan_device],
            vec![],
            vec![
                parsed_site("site-root", "Root Site"),
                parsed_site("site-orphan", "Orphan Site"),
            ],
            vec![root_parsed_device, orphan_parsed_device],
        );

        let bundle = build_imported_full2_bundle_from_data(
            Arc::new(config),
            uisp_data,
            true,
            std::time::Instant::now(),
            &mut OrphanLatch::default(),
            0,
        )
        .await
        .expect("an orphan-bearing import must not dangling-parent-reject");

        let editor = bundle
            .native_canonical
            .as_ref()
            .expect("full2 import should include native canonical state")
            .to_editor_state();
        assert!(
            editor
                .nodes
                .iter()
                .any(|node| node.node_id == "libreqos:generated:uisp:site:orphans"),
            "the Orphans umbrella must be emitted as an editor/canonical node"
        );
        let umbrella = editor
            .nodes
            .iter()
            .find(|node| node.node_id == "libreqos:generated:uisp:site:orphans")
            .expect("umbrella present");
        assert_eq!(
            umbrella.current_parent_node_id.as_deref(),
            Some("uisp:site:site-root"),
            "the umbrella must hang off the root node so the chain terminates"
        );

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    // ── §`lqos-74t.11` G2: the deferral WIRING through the real builder ────────
    const G2_ORPHAN: &str = "uisp:site:site-orphan";
    const G2_PARENT: &str = "uisp:site:site-parent";
    const G2_ORPHANS: &str = "libreqos:generated:uisp:site:orphans";
    /// The injected wall-clock second for every G2 deferral fixture.
    const G2_NOW: u64 = 1_000_000;

    /// A 3-site world: root → parent (linked when `link_parent`) and an UNLINKED
    /// orphan (linked to the parent only when `link_orphan`). The orphan has no
    /// root path unless linked.
    fn deferral_uisp_data(link_parent: bool, link_orphan: bool) -> UispData {
        let mut links = Vec::new();
        if link_parent {
            links.push(device_data_link(
                "link-root-parent",
                LinkEndpoint {
                    site_id: "site-root",
                    site_name: "Root Site",
                    device_id: "device-root-switch",
                    device_name: "Root Switch",
                },
                LinkEndpoint {
                    site_id: "site-parent",
                    site_name: "Parent Site",
                    device_id: "device-parent-switch",
                    device_name: "Parent Switch",
                },
            ));
        }
        if link_orphan {
            links.push(device_data_link(
                "link-parent-orphan",
                LinkEndpoint {
                    site_id: "site-parent",
                    site_name: "Parent Site",
                    device_id: "device-parent-switch",
                    device_name: "Parent Switch",
                },
                LinkEndpoint {
                    site_id: "site-orphan",
                    site_name: "Orphan Site",
                    device_id: "device-orphan-switch",
                    device_name: "Orphan Switch",
                },
            ));
        }
        UispData::from_parts(
            vec![
                raw_site("site-root", "Root Site", "site"),
                raw_site("site-parent", "Parent Site", "site"),
                raw_site("site-orphan", "Orphan Site", "site"),
            ],
            vec![
                raw_device("device-root-switch", "Root Switch", "site-root"),
                raw_device("device-parent-switch", "Parent Switch", "site-parent"),
                raw_device("device-orphan-switch", "Orphan Switch", "site-orphan"),
            ],
            links,
            vec![
                parsed_site("site-root", "Root Site"),
                parsed_site("site-parent", "Parent Site"),
                parsed_site("site-orphan", "Orphan Site"),
            ],
            vec![
                parsed_device("device-root-switch", "Root Switch", "site-root"),
                parsed_device("device-parent-switch", "Parent Switch", "site-parent"),
                parsed_device("device-orphan-switch", "Orphan Switch", "site-orphan"),
            ],
        )
    }

    async fn build_deferral_bundle(
        latch: &mut OrphanLatch,
        link_parent: bool,
        link_orphan: bool,
    ) -> lqos_topology_compile::ImportedTopologyBundle {
        let temp_dir = unique_temp_dir("uisp-full2-g2-deferral");
        let mut config = lqos_config::Config {
            lqos_directory: temp_dir.to_string_lossy().to_string(),
            state_directory: Some(temp_dir.join("state").to_string_lossy().to_string()),
            ..Default::default()
        };
        config.uisp_integration.enable_uisp = true;
        config.uisp_integration.site = "Root Site".to_string();
        config.queues.generated_pn_download_mbps = 10_000;
        config.queues.generated_pn_upload_mbps = 10_000;
        let bundle = build_imported_full2_bundle_from_data(
            Arc::new(config),
            deferral_uisp_data(link_parent, link_orphan),
            true,
            std::time::Instant::now(),
            latch,
            G2_NOW,
        )
        .await
        .expect("G2 deferral fixture should build");
        let _ = std::fs::remove_dir_all(temp_dir);
        bundle
    }

    /// Prime a no-path episode that started `age_secs` before `G2_NOW` (the
    /// builder's injected clock), remembering `parent` as the deferral target.
    fn prime_no_path(latch: &mut OrphanLatch, parent: &str, age_secs: u64) {
        latch.remember_parent(G2_ORPHAN, parent);
        // Starts the hold-off; the builder evaluates the age at `G2_NOW`.
        latch.note_no_path(G2_ORPHAN, G2_NOW - age_secs);
    }

    /// (a) A remembered parent + a no-path WITHIN `T_hold` ⇒ the emitted parent
    /// is the LATCHED parent, not the orphans umbrella.
    #[tokio::test]
    async fn g2_defers_a_no_path_within_the_hold_off_to_the_latched_parent() {
        let mut latch = OrphanLatch::default();
        prime_no_path(&mut latch, G2_PARENT, 1);
        let bundle = build_deferral_bundle(&mut latch, true, false).await;
        assert_eq!(
            network_parent_id_of(&bundle.compatibility_network_json, G2_ORPHAN),
            Some(Some(G2_PARENT.to_string())),
            "G2: a no-path within T_hold must be parented to the latched parent"
        );
    }

    /// (b1) A remembered parent ABSENT from the graph ⇒ orphans.
    #[tokio::test]
    async fn g2_orphans_when_the_latched_parent_is_absent() {
        let mut latch = OrphanLatch::default();
        prime_no_path(&mut latch, "uisp:site:no-such-site", 1);
        let bundle = build_deferral_bundle(&mut latch, true, false).await;
        assert_eq!(
            network_parent_id_of(&bundle.compatibility_network_json, G2_ORPHAN),
            Some(Some(G2_ORPHANS.to_string())),
            "an absent latched parent must fall back to orphans"
        );
    }

    /// (b2) A remembered parent PRESENT but not root-reachable ⇒ orphans.
    #[tokio::test]
    async fn g2_orphans_when_the_latched_parent_is_not_root_reachable() {
        let mut latch = OrphanLatch::default();
        prime_no_path(&mut latch, G2_PARENT, 1);
        // The parent site itself is unlinked from the root.
        let bundle = build_deferral_bundle(&mut latch, false, false).await;
        assert_eq!(
            network_parent_id_of(&bundle.compatibility_network_json, G2_ORPHAN),
            Some(Some(G2_ORPHANS.to_string())),
            "a latched parent that is not root-reachable must not be deferred to"
        );
    }

    /// (c) A no-path that has persisted STRICTLY LONGER than `T_hold` ⇒ orphans.
    #[tokio::test]
    async fn g2_orphans_when_the_no_path_exceeds_the_hold_off() {
        let mut latch = OrphanLatch::default();
        prime_no_path(
            &mut latch,
            G2_PARENT,
            ORPHAN_LATCH_HOLD_OFF_SECONDS + 1,
        );
        let bundle = build_deferral_bundle(&mut latch, true, false).await;
        assert_eq!(
            network_parent_id_of(&bundle.compatibility_network_json, G2_ORPHAN),
            Some(Some(G2_ORPHANS.to_string())),
            "a no-path strictly longer than T_hold must attach to orphans"
        );
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
            _parent_site,
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
        // FULL MODEL (DRIVER): the serving DEVICE is the parent ⇒ one group PER
        // serving device (the parallel links attach to different devices).
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].logical_parent, child_4600);
        assert_eq!(groups[0].candidate_nodes, vec![child_4600]);
        assert_eq!(groups[1].logical_parent, child_wave);
        assert_eq!(groups[1].candidate_nodes, vec![child_wave]);

        let allowed =
            topology_allowed_parents_from_groups(&graph, &groups, &[], &HashSet::new(), &[]);
        assert_eq!(allowed.len(), 2);
        assert_eq!(allowed[0].parent_node_name, "Backhaul AirFiber Child");
        assert_eq!(allowed[1].parent_node_name, "Backhaul Wave Child");
        assert!(allowed
            .iter()
            .all(|parent| !parent.attachment_options.is_empty()));
    }

    /// `lqos-74t.63` G1-L1 BOUNDED-1: a DOWN backhaul link genuinely LEAVES the
    /// candidate set — so the pin RELEASES (it can never hold a down parent). A
    /// device kept in the graph but no longer root-reachable is dropped by
    /// `topology_parent_candidates_for_node` (the `is_upstream_parent_candidate`
    /// root-path + child-loop guard), not just by the set the latch is handed.
    #[test]
    fn lqos63_l1_a_down_backhaul_releases_the_pinned_candidate() {
        use std::collections::BTreeSet;
        let (
            mut graph,
            root,
            _parent_site,
            child_site,
            parent_wave,
            child_wave,
            _parent_4600,
            child_4600,
        ) = build_parallel_attachment_graph();
        let node_id = graph[child_site].network_json_id();
        let wave_id = graph[child_wave].network_json_id();
        let airfiber_id = graph[child_4600].network_json_id();

        let valid_logical_parents = |graph: &GraphType| -> BTreeSet<String> {
            let candidates =
                topology_parent_candidates_for_node(graph, root, child_site, &[], &[]);
            let groups = topology_allowed_parent_groups_for_node(
                graph,
                root,
                child_site,
                &candidates,
                &[],
                &[],
            );
            groups
                .iter()
                .map(|group| graph[group.logical_parent].network_json_id())
                .collect()
        };

        // Both parallel backhauls are valid candidates ⇒ the first selection pins.
        let both = valid_logical_parents(&graph);
        assert!(both.contains(&wave_id) && both.contains(&airfiber_id));
        let mut latch = OrphanLatch::default();
        assert_eq!(latch.pin_parent(&node_id, &wave_id, &both), wave_id);

        // The WavePro backhaul LINK goes down: BOTH directed link edges are dropped
        // (the far radio stays in the graph but the link no longer wires). The
        // candidate must leave the set ⇒ the pin RELEASES and re-pins.
        let fwd = graph
            .find_edge(parent_wave, child_wave)
            .expect("the wave upstream edge");
        graph.remove_edge(fwd);
        let rev = graph
            .find_edge(child_wave, parent_wave)
            .expect("the wave downstream edge");
        graph.remove_edge(rev);
        let after = valid_logical_parents(&graph);
        assert!(
            !after.contains(&wave_id),
            "a down link drops the candidate from the set (so the pin cannot hold a down parent)"
        );
        assert!(after.contains(&airfiber_id));
        assert_eq!(latch.pin_parent(&node_id, &airfiber_id, &after), airfiber_id);
    }

    /// `docs/topology_ptp_link_site_attachment_work_order.md` §5: a LINK/PTP
    /// device (device ROLE, `sta-ptp`) that is a site's attachment resolves the
    /// site's logical parent to the SERVING AP across the link (the link is the
    /// attachment, not a tree level).
    #[test]
    fn ptp_link_by_role_parents_the_site_to_the_serving_ap() {
        let mut graph = GraphType::new();
        let root = graph.add_node(site("TrailerCity", "651b2e8a"));
        let ap_idx = graph.add_node(ap("JR_AP_TC_C", "85532523", "TrailerCity"));
        let link_idx = graph.add_node(ap("Wave Long-Range", "6ae6696b", "TrailerCity"));
        let tower = graph.add_node(site("TrailerCity Tower Trailer", "aa9d8941"));
        for (from, to) in [
            (root, ap_idx),
            (ap_idx, root),
            (ap_idx, link_idx),
            (link_idx, ap_idx),
            (tower, link_idx),
            (link_idx, tower),
        ] {
            graph.add_edge(from, to, LinkMapping::ethernet(1_000));
        }
        let mut devices = vec![
            parsed_device("85532523", "JR_AP_TC_C", "651b2e8a"),
            parsed_device("6ae6696b", "Wave Long-Range", "651b2e8a"),
        ];
        // The ROLE is the discriminator (PIN-1), never the structure.
        devices[1].role = Some("station".to_string());
        devices[1].wireless_mode = Some("sta-ptp".to_string());

        assert!(device_is_link(&devices[1]));
        assert!(!device_is_link(&devices[0]));
        let logical_parent =
            logical_parent_for_candidate(&graph, root, tower, link_idx, &devices, &[]);
        assert_eq!(
            logical_parent, ap_idx,
            "a role-keyed link-delivered site attaches to the SERVING AP"
        );
    }

 /// the implementer `#12` PIN-1 (the NON-link fixture): a CHAINED AP (an AP under
    /// another AP) is structurally identical to a link but is NOT one by ROLE ⇒
    /// the site keeps the nearest-site parent and the intermediate AP is NOT
    /// dropped/re-parented to the grandparent.
    #[test]
    fn chained_ap_site_is_not_re_parented_to_the_grandparent() {
        let mut graph = GraphType::new();
        let root = graph.add_node(site("Root Site", "site-root"));
        let ap1 = graph.add_node(ap("Upstream AP", "device-ap1", "Root Site"));
        let ap2 = graph.add_node(ap("Chained AP", "device-ap2", "Root Site"));
        let child = graph.add_node(site("Child Site", "site-child"));
        for (from, to) in [
            (root, ap1),
            (ap1, root),
            (ap1, ap2),
            (ap2, ap1),
            (child, ap2),
            (ap2, child),
        ] {
            graph.add_edge(from, to, LinkMapping::ethernet(1_000));
        }
        // Both are serving PTMP APs — `role=ap`, `ap-ptmp` mode (serve clients;
        // NOT a PTP/backhaul link — a `ptmp` mode carries no `ptp`).
        let mut devices = vec![
            parsed_device("device-ap1", "Upstream AP", "site-root"),
            parsed_device("device-ap2", "Chained AP", "site-root"),
        ];
        for d in &mut devices {
            d.role = Some("ap".to_string());
            d.wireless_mode = Some("ap-ptmp".to_string());
        }
        assert!(
            !devices.iter().any(device_is_link),
            "an `ap-ptmp` serving AP is NOT a PTP/backhaul link"
        );

        let logical_parent = logical_parent_for_candidate(&graph, root, child, ap2, &devices, &[]);
        assert_eq!(
            logical_parent, ap2,
            "a chained-AP site parents to the intermediate AP (NOT the grandparent; no drop)"
        );
    }

    /// Builds the canonical for the canonical TrailerCity/PTMP + WestRedd/PTP
    /// shape: `Root Site --(ap)--> device-ap --(data link)--> device-link (the
    /// radio) --(site)--> Child Site`, with the given link radio ROLE/mode. The
    /// serving AP is a PtMP sector AP (`ap-ptmp`) and STAYS a node.
    async fn build_link_attachment_canonical(
        link_role: &str,
        link_mode: Option<&str>,
        link_name: &str,
    ) -> lqos_config::TopologyCanonicalStateFile {
        let temp_dir = unique_temp_dir("uisp-full2-link-attachment");
        let mut config = lqos_config::Config {
            lqos_directory: temp_dir.to_string_lossy().to_string(),
            state_directory: Some(temp_dir.join("state").to_string_lossy().to_string()),
            ..Default::default()
        };
        config.uisp_integration.enable_uisp = true;
        config.uisp_integration.site = "Root Site".to_string();
        config.queues.generated_pn_download_mbps = 10_000;
        config.queues.generated_pn_upload_mbps = 10_000;

        let root_site = raw_site("site-root", "Root Site", "site");
        let child_site = raw_site("site-child", "Child Site", "site");
        let ap_device = raw_device("device-ap", "Serving AP", "site-root");
        let link_device = raw_device("device-link", link_name, "site-child");
        let mut ap_parsed = parsed_device("device-ap", "Serving AP", "site-root");
        ap_parsed.role = Some("ap".to_string());
        ap_parsed.wireless_mode = Some("ap-ptmp".to_string());
        let mut link_parsed = parsed_device("device-link", link_name, "site-child");
        link_parsed.role = Some(link_role.to_string());
        link_parsed.wireless_mode = link_mode.map(str::to_string);
        assert!(device_is_link(&link_parsed));
        assert!(!device_is_link(&ap_parsed));
        let uisp_data = UispData::from_parts(
            vec![root_site, child_site],
            vec![ap_device, link_device],
            vec![device_data_link(
                "link-ap-child",
                LinkEndpoint {
                    site_id: "site-root",
                    site_name: "Root Site",
                    device_id: "device-ap",
                    device_name: "Serving AP",
                },
                LinkEndpoint {
                    site_id: "site-child",
                    site_name: "Child Site",
                    device_id: "device-link",
                    device_name: link_name,
                },
            )],
            vec![
                parsed_site("site-root", "Root Site"),
                parsed_site("site-child", "Child Site"),
            ],
            vec![ap_parsed, link_parsed],
        );
        let bundle = build_imported_full2_bundle_from_data(
            Arc::new(config),
            uisp_data,
            true,
            std::time::Instant::now(),
            &mut OrphanLatch::default(),
            0,
        )
        .await
        .expect("in-memory UISP import should build");
        bundle
            .native_canonical
            .expect("full2 import should include native canonical state")
    }

    /// `docs/topology_ptmp_ptp_attachment_card.md` §7: for BOTH shapes, (a) the
    /// child site's parent == the serving AP; (b) the link is NOT in the canonical
    /// node list (nor the derived editor list); (c) the link IS the site's
    /// `current_attachment_id`; (d) the client-serving `ap-ptmp` AP STAYS a node;
    /// (e) no node is emitted with the link's kind.
    fn assert_link_attachment_shape(canonical: &lqos_config::TopologyCanonicalStateFile) {
        assert!(
            !canonical
                .nodes
                .iter()
                .any(|node| node.node_id == "uisp:device:device-link"),
            "a link/station radio is NOT a node — attachment-only (no `ap`, no `site`)"
        );
        assert!(
            canonical
                .nodes
                .iter()
                .any(|node| node.node_id == "uisp:device:device-ap"),
            "a client-serving `ap-ptmp` AP STAYS a node (no over-match)"
        );
        let editor = canonical.to_editor_state();
        assert!(
            !editor
                .nodes
                .iter()
                .any(|node| node.node_id == "uisp:device:device-link"),
            "the link is not an EDITOR node either"
        );
        let child = canonical
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:site:site-child")
            .expect("the child site is a node");
        assert_eq!(
            child.current_parent_node_id.as_deref(),
            Some("uisp:device:device-ap"),
            "the site parents ACROSS to the serving AP"
        );
        assert_eq!(
            child.current_attachment_id.as_deref(),
            Some("uisp:device:device-link"),
            "the link/station radio IS the site's attachment"
        );
 // the implementer `#12` R2: the radio's RATE must survive the radio leaving the
        // node list — it rides its ATTACHMENT OPTION (so the site's shaped rate
        // is unchanged while the tree looks right).
        let child_editor = editor
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:site:site-child")
            .expect("the child site is an editor node");
        let ap_group = child_editor
            .allowed_parents
            .iter()
            .find(|parent| parent.parent_node_id == "uisp:device:device-ap")
            .expect("the serving AP is the grouped logical parent");
        let link_option = ap_group
            .attachment_options
            .iter()
            .find(|option| option.attachment_id == "uisp:device:device-link")
            .expect("the link is an attachment option, not a parent candidate");
        assert!(
            link_option.download_bandwidth_mbps.is_some()
                || link_option.capacity_mbps.is_some(),
            "the radio's rate rides the attachment option (R2): {link_option:?}"
        );
    }

    #[tokio::test]
    async fn ptp_link_is_attachment_only_and_the_site_parents_across() {
        let canonical =
            build_link_attachment_canonical("ap", Some("ap-ptp"), "WavePro-WestReddToRochester")
                .await;
        assert_link_attachment_shape(&canonical);
    }

    /// The DRIVER's canonical PtMP case: a site served through a PtMP STATION
    /// CLIENT (`sta-ptmp`, 'Wave Long-Range') parents to the sector AP; the
    /// station client is the attachment only.
    #[tokio::test]
    async fn ptmp_station_client_is_attachment_only_and_the_site_parents_across() {
        let canonical =
            build_link_attachment_canonical("station", Some("sta-ptmp"), "Wave Long-Range").await;
        assert_link_attachment_shape(&canonical);
    }

 /// `docs/topology_ptmp_ptp_attachment_card.md` §2 + §7 (the implementer acceptance):
    /// ONE union predicate covering BOTH shapes — a PtMP station client / CPE
    /// (`sta-ptmp`, the DRIVER's 'Wave Long-Range') AND a PTP/backhaul radio,
    /// EITHER end (`ap-ptp`/`sta-ptp`). A sector AP (`ap-ptmp`) and a plain
    /// wired device stay NODES. A link that carries clients is still a link —
    /// the predicate never consults `aps_with_clients`.
    #[test]
    fn device_is_link_is_the_pmtp_station_and_ptp_backhaul_class() {
        let mut d = parsed_device("d", "D", "s");

        // (a) PtMP station client / CPE — the canonical 'Wave Long-Range'.
        d.role = Some("station".into());
        d.wireless_mode = Some("sta-ptmp".into());
        assert!(device_is_link(&d), "a PtMP station client is a LINK");
        d.role = Some("cpe".into());
        d.wireless_mode = None;
        assert!(device_is_link(&d), "a `cpe` role is a LINK");
        d.role = Some("sta".into());
        d.wireless_mode = None;
        assert!(device_is_link(&d), "a `sta` role is a LINK");
        d.role = None;
        d.wireless_mode = Some("station".into());
        assert!(device_is_link(&d), "a `station*` mode is a LINK");

        // (b) PTP / backhaul — EITHER end.
        d.role = Some("ap".into());
        d.wireless_mode = Some("ap-ptp".into());
        assert!(device_is_link(&d), "AP-side PTP backhaul (WavePro/GigaBeam/…)");
        d.role = Some("station".into());
        d.wireless_mode = Some("sta-ptp".into());
        assert!(device_is_link(&d), "station-side PTP");
        d.role = Some("ptp".into());
        d.wireless_mode = None;
        assert!(device_is_link(&d), "an explicit ptp role");
        d.role = Some("backhaul".into());
        d.wireless_mode = None;
        assert!(device_is_link(&d), "an explicit backhaul role");

        // NOT links — node devices.
        d.role = Some("ap".into());
        d.wireless_mode = Some("ap-ptmp".into());
        assert!(!device_is_link(&d), "a PTMP sector AP STAYS a node");
        d.role = Some("ap".into());
        d.wireless_mode = None;
        assert!(!device_is_link(&d), "a plain AP stays");
        d.role = Some("switch".into());
        d.wireless_mode = None;
        assert!(!device_is_link(&d), "a switch stays");
        d.role = Some("router".into());
        d.wireless_mode = None;
        assert!(!device_is_link(&d), "a router stays");
        d.role = None;
        d.wireless_mode = None;
        assert!(!device_is_link(&d), "no role/mode stays");
    }

    /// §2bis (the DRIVER's steer): an OPAQUE radio (`role=wireless`, blackBox,
    /// no `wirelessMode`) is classified by the CONNECTED SITE TYPE —
    /// `own site endpoint => CPE`; `own site infra + connects an endpoint => AP`;
    /// `own site infra + infra<->infra => backhaul`. Closes `4600C_*`/`AVIAT_*`.
    #[test]
    fn opaque_radio_is_classified_by_the_connected_site_type() {
        use crate::strategies::common::UispData;
        use crate::uisp_types::UispSiteType;
        let mut infra_a = parsed_site("site-a", "Infra A");
        infra_a.site_type = UispSiteType::Site;
        let mut infra_b = parsed_site("site-b", "Infra B");
        infra_b.site_type = UispSiteType::Site;
        let mut sub = parsed_site("sub-1", "Subscriber 1");
        sub.site_type = UispSiteType::Client;

        let raw_ap = raw_device("dev-ap", "JR_AP_RR_B", "site-a");
        let raw_cpe = raw_device("dev-cpe", "Radio-103ThoroughbredCt", "sub-1");
        let raw_bh = raw_device("dev-bh", "4600C_MRE_To_ROCH", "site-a");
        let raw_router = raw_device("dev-router", "Rochester Router", "site-b");

        let mut dev_ap = parsed_device("dev-ap", "JR_AP_RR_B", "site-a");
        dev_ap.role = Some("wireless".into());
        let mut dev_cpe = parsed_device("dev-cpe", "Radio-103ThoroughbredCt", "sub-1");
        dev_cpe.role = Some("wireless".into());
        let mut dev_bh = parsed_device("dev-bh", "4600C_MRE_To_ROCH", "site-a");
        dev_bh.role = Some("wireless".into());
        let mut dev_router = parsed_device("dev-router", "Rochester Router", "site-b");
        dev_router.role = Some("router".into());

        let link_ap_cpe = device_data_link(
            "l1",
            LinkEndpoint {
                site_id: "site-a",
                site_name: "Infra A",
                device_id: "dev-ap",
                device_name: "JR_AP_RR_B",
            },
            LinkEndpoint {
                site_id: "sub-1",
                site_name: "Subscriber 1",
                device_id: "dev-cpe",
                device_name: "Radio-103ThoroughbredCt",
            },
        );
        let link_bh_router = device_data_link(
            "l2",
            LinkEndpoint {
                site_id: "site-a",
                site_name: "Infra A",
                device_id: "dev-bh",
                device_name: "4600C_MRE_To_ROCH",
            },
            LinkEndpoint {
                site_id: "site-b",
                site_name: "Infra B",
                device_id: "dev-router",
                device_name: "Rochester Router",
            },
        );

        let data = UispData::from_parts(
            vec![
                raw_site("site-a", "Infra A", "site"),
                raw_site("site-b", "Infra B", "site"),
                raw_site("sub-1", "Subscriber 1", "endpoint"),
            ],
            vec![raw_ap, raw_cpe, raw_bh, raw_router],
            vec![link_ap_cpe, link_bh_router],
            vec![infra_a, infra_b, sub],
            vec![dev_ap, dev_cpe, dev_bh, dev_router],
        );
        let by_id = |id: &str| data.devices.iter().find(|d| d.id == id).unwrap();
        assert!(
            !device_is_link(by_id("dev-ap")),
            "an infra AP connecting an endpoint is a NODE"
        );
        assert!(
            device_is_link(by_id("dev-cpe")),
            "a radio at a subscriber endpoint is a CPE/attachment"
        );
        assert!(
            device_is_link(by_id("dev-bh")),
            "an infra<->infra radio is a backhaul/attachment"
        );
        assert!(
            !device_is_link(by_id("dev-router")),
            "a non-opaque infrastructure router stays a node"
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
            _parent_site,
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
                // FULL MODEL: the override names the SERVING DEVICE of the
                // preferred path (the group that owns child_4600).
                parent_node_id: graph[child_4600].network_json_id(),
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
    fn logical_parent_for_parallel_attachment_candidate_is_the_serving_device() {
        let (
            graph,
            root,
            _parent_site,
            child_site,
            _parent_wave,
            child_wave,
            _parent_4600,
            _child_4600,
        ) = build_parallel_attachment_graph();

        // FULL MODEL: the serving DEVICE (the attachment) is the logical parent.
        let logical_parent =
            logical_parent_for_candidate(&graph, root, child_site, child_wave, &[], &[]);
        assert_eq!(logical_parent, child_wave);
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
        let gamma_to_beta =
            graph.add_node(ap("Gamma - Beta MLO6", "device-gamma-beta", "Site Gamma"));
        let beta_to_gamma =
            graph.add_node(ap("Beta - Gamma MLO6", "device-beta-gamma", "Site Beta"));
        let gamma_to_alpha = graph.add_node(ap("Gamma-Alpha", "device-gamma-alpha", "Site Gamma"));
        let alpha_to_gamma = graph.add_node(ap("Alpha-Gamma", "device-alpha-gamma", "Site Alpha"));
        let beta_to_alpha =
            graph.add_node(ap("Beta - Alpha MLO5", "device-beta-alpha", "Site Beta"));
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

        let groups =
            topology_allowed_parent_groups_for_node(&graph, root, beta_site, &candidates, &[], &[]);
        // FULL MODEL (DRIVER): the serving DEVICE is the parent.
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].logical_parent, beta_to_alpha);
        assert_eq!(groups[0].candidate_nodes, vec![beta_to_alpha]);
        assert_eq!(groups[1].logical_parent, beta_to_gamma);
        assert_eq!(groups[1].candidate_nodes, vec![beta_to_gamma]);

        let allowed =
            topology_allowed_parents_from_groups(&graph, &groups, &[], &HashSet::new(), &[]);
        assert_eq!(
            allowed
                .iter()
                .map(|parent| parent.parent_node_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Beta - Alpha MLO5", "Beta - Gamma MLO6"]
        );
        let alpha_parent = allowed
            .iter()
            .find(|parent| parent.parent_node_name == "Beta - Alpha MLO5")
            .expect("expected the serving device as a legal parent");
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
        let gamma_to_alpha = graph.add_node(ap("Gamma-Alpha", "device-gamma-alpha", "Site Gamma"));
        let alpha_to_gamma = graph.add_node(ap("Alpha-Gamma", "device-alpha-gamma", "Site Alpha"));
        let beta_to_alpha = graph.add_node(ap("Beta - Alpha 60", "device-beta-alpha", "Site Beta"));
        let alpha_to_beta = graph.add_node(ap("Alpha-Beta-60", "device-alpha-beta", "Site Alpha"));

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

        // FULL MODEL: the serving DEVICE is the logical parent.
        let logical_parent =
            logical_parent_for_candidate(&graph, root, beta_site, beta_to_alpha, &[], &[]);
        assert_eq!(logical_parent, beta_to_alpha);
        assert_eq!(
            export_parent_anchor_for_override(
                &graph,
                beta_site,
                logical_parent,
                beta_to_alpha,
                &[],
                &[],
            ),
            beta_to_alpha
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
            transport_cap_line_rate_mbps: None,
            transport_cap_interface: None,
            transport_cap_mbps: None,
            transport_cap_reason: None,
            attachment_rate_source: UispAttachmentRateSource::Static,
            classifies_as_link: false,
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
    use std::collections::HashSet;

    fn test_uisp_device(id: &str, name: &str) -> UispDevice {
        UispDevice {
            id: id.to_string(),
            name: name.to_string(),
            mac: String::new(),
            role: None,
            wireless_mode: None,
            site_id: String::new(),
            raw_download: 0,
            raw_upload: 0,
            download: 0,
            upload: 0,
            ipv4: HashSet::new(),
            ipv6: HashSet::new(),
            probe_ipv4: HashSet::new(),
            probe_ipv6: HashSet::new(),
            negotiated_ethernet_mbps: None,
            negotiated_ethernet_interface: None,
            transport_cap_line_rate_mbps: None,
            transport_cap_interface: None,
            transport_cap_mbps: None,
            transport_cap_reason: None,
            attachment_rate_source: UispAttachmentRateSource::Static,
            classifies_as_link: false,
        }
    }

    #[test]
    fn topology_node_ethernet_advisory_preserves_the_port_cap_details() {
        let mut device = test_uisp_device("ap-1", "AP One");
        device.raw_download = 500;
        device.raw_upload = 200;
        device.download = 94;
        device.upload = 94;
        device.negotiated_ethernet_mbps = Some(100);
        device.negotiated_ethernet_interface = Some("eth0".to_string());
        device.transport_cap_line_rate_mbps = Some(100);
        device.transport_cap_interface = Some("eth0".to_string());
        device.transport_cap_mbps = Some(94);

        let advisory = topology_node_ethernet_advisory(&device)
            .expect("reduced infrastructure node should produce an Ethernet advisory");

        assert_eq!(advisory.target_kind, EthernetCapTargetKind::Node);
        assert_eq!(advisory.target_id, "uisp:device:ap-1");
        assert_eq!(advisory.negotiated_ethernet_mbps, 100);
        assert_eq!(advisory.requested_download_mbps, 500.0);
        assert_eq!(advisory.applied_upload_mbps, 94.0);
        assert_eq!(advisory.limiting_interface_name.as_deref(), Some("eth0"));
    }

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
    fn root_resolution_accepts_configured_site() {
        let sites = vec!["Main".to_string(), "Backup".to_string()];
        let internet_candidates = vec!["Backup".to_string()];

        let root =
            resolve_root_site_from_names("Main", &sites, &internet_candidates, true).unwrap();

        assert_eq!(root, ResolvedRootSite::Existing("Main".to_string()));
    }

    #[test]
    fn root_resolution_rejects_bad_configured_site_for_full_mode() {
        let sites = vec!["Main".to_string(), "Backup".to_string()];
        let internet_candidates = vec!["Backup".to_string()];

        let error =
            resolve_root_site_from_names("Old Root", &sites, &internet_candidates, true)
                .unwrap_err();

        assert_eq!(error, UispIntegrationError::NoRootSite("Old Root".to_string()));
    }

    #[test]
    fn root_resolution_auto_detects_for_non_full_mode() {
        let sites = vec!["Main".to_string(), "Backup".to_string()];
        let internet_candidates = vec!["Backup".to_string()];

        let root =
            resolve_root_site_from_names("Old Root", &sites, &internet_candidates, false).unwrap();

        assert_eq!(root, ResolvedRootSite::Existing("Backup".to_string()));
    }

    #[test]
    fn root_resolution_uses_generated_root_for_multiple_internet_sites() {
        let sites = vec!["Main".to_string(), "Backup".to_string()];
        let internet_candidates = vec!["Main".to_string(), "Backup".to_string()];

        let root = resolve_root_site_from_names("", &sites, &internet_candidates, false).unwrap();

        assert_eq!(root, ResolvedRootSite::GeneratedInternet);
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

    #[test]
    fn full2_skips_orphaned_circuit_anchor_when_ap_is_not_in_topology() {
        let device = test_uisp_device("device-missing", "Missing AP");
        let parents = HashMap::<String, NetJsonParent<'_>>::new();

        let anchor = anchor_for_site_ap(
            "circuit-1",
            "Circuit 1",
            format!("uisp:device:{}", device.id),
            &parents,
        );

        assert!(anchor.is_none());
    }

    #[test]
    fn full2_emits_circuit_anchor_when_ap_exists_in_topology() {
        let mut graph = GraphType::new();
        let ap = add_ap(&mut graph, "Present AP", "device-present", "Present Site");
        let mapping = &graph[ap];
        let mut parents = HashMap::<String, NetJsonParent<'_>>::new();
        let node_id = mapping.network_json_id();
        parents.insert(
            node_id.clone(),
            NetJsonParent {
                node_id: node_id.clone(),
                node_name: mapping.name(),
                export_name: mapping.name(),
                parent_id: None,
                parent_name: "Parent".to_string(),
                mapping,
                download: 1000,
                upload: 1000,
            },
        );
        let device = test_uisp_device("device-present", "Present AP");

        let anchor = anchor_for_site_ap(
            "circuit-1",
            "Circuit 1",
            format!("uisp:device:{}", device.id),
            &parents,
        )
        .expect("expected anchor to be emitted");

        assert_eq!(anchor.anchor_node_id, "uisp:device:device-present");
        assert_eq!(anchor.anchor_node_name.as_deref(), Some("Present AP"));
    }
}

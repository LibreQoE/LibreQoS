use crate::node_manager::local_api::network_tree_lite::NetworkTreeLiteNode;
use crate::treeguard::actor::is_runtime_virtualized_node;
use anyhow::Result;
use arc_swap::ArcSwap;
use fxhash::{FxHashMap, FxHashSet};
use lqos_bus::{BusResponse, Circuit};
#[cfg(test)]
use lqos_config::load_active_runtime_shaping_inputs;
use lqos_config::{
    NetworkJsonNode, NetworkJsonTransport, TopologyRuntimeShapingPayloadIdentity,
    TopologyRuntimeStatusFile, TopologyShapingInputsFile,
    load_active_runtime_shaping_inputs_from_status, load_config, topology_runtime_status_path,
};
use lqos_queue_tracker::EFFECTIVE_NODE_RATES;
use lqos_utils::file_watcher::FileWatcher;
use lqos_utils::hash_to_i64;
use lqos_utils::rtt::{FlowbeeEffectiveDirection, RttBucket};
use lqos_utils::units::{DownUpOrder, down_up_retransmit_sample};
use lqos_utils::unix_time::time_since_boot;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;
use tracing::{debug, error, info, warn};

#[cfg(test)]
use anyhow::Context;
#[cfg(test)]
use lqos_config::{ConfigShapedDevices, ShapedDevice};
#[cfg(test)]
use std::net::{Ipv4Addr, Ipv6Addr};

pub mod circuit_live;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
pub use circuit_live::CircuitLiveSnapshot;

pub static CIRCUIT_LIVE_SNAPSHOT: Lazy<ArcSwap<CircuitLiveSnapshot>> =
    Lazy::new(|| ArcSwap::new(Arc::new(CircuitLiveSnapshot::default())));
pub static CIRCUIT_LIVE_LAST_REFRESH_SECS: AtomicU64 = AtomicU64::new(0);
pub static CIRCUIT_LIVE_REFRESH_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
pub static EFFECTIVE_CIRCUIT_PARENTS: Lazy<ArcSwap<FxHashMap<String, RuntimeCircuitParent>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(FxHashMap::default())));
static LAST_TOPOLOGY_STATUS_IDENTITY: Lazy<Mutex<Option<TopologyRuntimeShapingPayloadIdentity>>> =
    Lazy::new(|| Mutex::new(None));
#[cfg(test)]
static CIRCUIT_SNAPSHOT_TEST_HOOK: Lazy<
    Mutex<Option<(std::thread::ThreadId, std::sync::mpsc::Sender<()>)>>,
> = Lazy::new(|| Mutex::new(None));

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCircuitParent {
    pub name: String,
    pub id: Option<String>,
}

pub(crate) fn invalidate_circuit_live_snapshot() {
    CIRCUIT_LIVE_LAST_REFRESH_SECS.store(0, std::sync::atomic::Ordering::Release);
}

pub(crate) fn invalidate_executive_cache_snapshot() {
    crate::node_manager::invalidate_executive_cache_snapshot();
}

fn non_empty_circuit_id_key(circuit_id: &str) -> Option<String> {
    let key = lqos_utils::normalize_circuit_id_key(circuit_id);
    if key.is_empty() {
        None
    } else {
        Some(key)
    }
}

fn optional_trimmed_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn build_effective_circuit_parent_map(
    shaping_inputs: &TopologyShapingInputsFile,
) -> FxHashMap<String, RuntimeCircuitParent> {
    let mut by_circuit_id = FxHashMap::default();
    by_circuit_id.reserve(shaping_inputs.circuits.len());
    for circuit in &shaping_inputs.circuits {
        let Some(circuit_key) = non_empty_circuit_id_key(&circuit.circuit_id) else {
            continue;
        };
        let Some(parent_name) = optional_trimmed_string(&circuit.effective_parent_node_name) else {
            continue;
        };
        by_circuit_id
            .entry(circuit_key)
            .or_insert_with(|| RuntimeCircuitParent {
                name: parent_name,
                id: optional_trimmed_string(&circuit.effective_parent_node_id),
            });
    }
    by_circuit_id
}

fn publish_shaping_inputs(shaping_inputs: TopologyShapingInputsFile) {
    let effective_parents = build_effective_circuit_parent_map(&shaping_inputs);
    EFFECTIVE_CIRCUIT_PARENTS.store(Arc::new(effective_parents));
    invalidate_circuit_live_snapshot();
    invalidate_executive_cache_snapshot();
}

fn topology_status_identity(
    status: &TopologyRuntimeStatusFile,
) -> TopologyRuntimeShapingPayloadIdentity {
    status.shaping_payload_identity()
}

fn topology_status_identity_changed(identity: &TopologyRuntimeShapingPayloadIdentity) -> bool {
    LAST_TOPOLOGY_STATUS_IDENTITY.lock().as_ref() != Some(identity)
}

fn remember_topology_status_identity(identity: TopologyRuntimeShapingPayloadIdentity) {
    *LAST_TOPOLOGY_STATUS_IDENTITY.lock() = Some(identity);
}

#[cfg(test)]
fn parse_ipv4_entry(value: &str) -> Option<(Ipv4Addr, u32)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (ip, cidr) = if let Some((ip, cidr)) = trimmed.split_once('/') {
        (ip.trim(), cidr.trim().parse().ok()?)
    } else {
        (trimmed, 32)
    };
    Some((ip.parse().ok()?, cidr))
}

#[cfg(test)]
fn parse_ipv6_entry(value: &str) -> Option<(Ipv6Addr, u32)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (ip, cidr) = if let Some((ip, cidr)) = trimmed.split_once('/') {
        (ip.trim(), cidr.trim().parse().ok()?)
    } else {
        (trimmed, 128)
    };
    Some((ip.parse().ok()?, cidr))
}

#[cfg(test)]
fn parse_ipv4_list(values: &[String]) -> Vec<(Ipv4Addr, u32)> {
    values
        .iter()
        .filter_map(|value| parse_ipv4_entry(value))
        .collect()
}

#[cfg(test)]
fn parse_ipv6_list(values: &[String]) -> Vec<(Ipv6Addr, u32)> {
    values
        .iter()
        .filter_map(|value| parse_ipv6_entry(value))
        .collect()
}

#[cfg(test)]
fn shaped_devices_from_runtime_inputs(
    shaping_inputs: &TopologyShapingInputsFile,
) -> ConfigShapedDevices {
    let mut devices = Vec::new();
    for circuit in &shaping_inputs.circuits {
        let parent_node = optional_trimmed_string(&circuit.effective_parent_node_name)
            .or_else(|| circuit.logical_parent_node_name.clone())
            .unwrap_or_default();
        let parent_node_id = optional_trimmed_string(&circuit.effective_parent_node_id)
            .or_else(|| circuit.logical_parent_node_id.clone());
        for device in &circuit.devices {
            devices.push(ShapedDevice {
                circuit_id: circuit.circuit_id.clone(),
                circuit_name: circuit.circuit_name.clone(),
                device_id: device.device_id.clone(),
                device_name: device.device_name.clone(),
                parent_node: parent_node.clone(),
                parent_node_id: parent_node_id.clone(),
                anchor_node_id: circuit.anchor_node_id.clone(),
                mac: device.mac.clone(),
                ipv4: parse_ipv4_list(&device.ipv4),
                ipv6: parse_ipv6_list(&device.ipv6),
                download_min_mbps: circuit.download_min_mbps,
                upload_min_mbps: circuit.upload_min_mbps,
                download_max_mbps: circuit.download_max_mbps,
                upload_max_mbps: circuit.upload_max_mbps,
                comment: if device.comment.trim().is_empty() {
                    circuit.comment.clone()
                } else {
                    device.comment.clone()
                },
                sqm_override: circuit.sqm_override.clone(),
                ..ShapedDevice::default()
            });
        }
    }

    let mut shaped = ConfigShapedDevices::default();
    shaped.replace_with_new_data(devices);
    shaped
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(test)]
enum ShapedDevicesLoadSource {
    RuntimeShapingInputs,
    TopologyImport,
    ShapedDevicesCsv,
}

#[cfg(test)]
fn integration_ingress_enabled(config: &lqos_config::Config) -> bool {
    config.uisp_integration.enable_uisp
        || config.splynx_integration.enable_splynx
        || config
            .netzur_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_netzur)
        || config
            .visp_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_visp)
        || config.powercode_integration.enable_powercode
        || config.sonar_integration.enable_sonar
        || config
            .wispgate_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_wispgate)
}

#[cfg(test)]
fn load_ready_runtime_shaping_inputs(
    config: &lqos_config::Config,
) -> Result<Option<TopologyShapingInputsFile>> {
    load_active_runtime_shaping_inputs(config)
        .context("Unable to load active runtime shaping inputs")
}

#[cfg(test)]
fn load_shaped_devices_from_preferred_source(
    config: &lqos_config::Config,
) -> Result<(ConfigShapedDevices, ShapedDevicesLoadSource)> {
    if let Some(shaping_inputs) = load_ready_runtime_shaping_inputs(config)? {
        return Ok((
            shaped_devices_from_runtime_inputs(&shaping_inputs),
            ShapedDevicesLoadSource::RuntimeShapingInputs,
        ));
    }

    if integration_ingress_enabled(config) {
        match lqos_topology_compile::TopologyImportFile::load(config) {
            Ok(Some(topology_import)) => {
                let shaped_devices = topology_import.into_imported_bundle().shaped_devices;
                if !shaped_devices.devices.is_empty() {
                    return Ok((shaped_devices, ShapedDevicesLoadSource::TopologyImport));
                }
                debug!(
                    "topology_import.json contained 0 shaped devices; falling back to ShapedDevices.csv"
                );
            }
            Ok(None) => {
                debug!("topology_import.json missing; falling back to ShapedDevices.csv");
            }
            Err(err) => {
                debug!(
                    "Unable to load topology_import.json ({err}); falling back to ShapedDevices.csv"
                );
            }
        }
    }

    let shaped_devices =
        ConfigShapedDevices::load_for_config(config).context("Unable to load ShapedDevices.csv")?;
    Ok((shaped_devices, ShapedDevicesLoadSource::ShapedDevicesCsv))
}
fn load_topology_runtime_status_payload() {
    let Ok(config) = load_config() else {
        warn!("Unable to load LibreQoS config while loading topology runtime status");
        return;
    };
    let status = match TopologyRuntimeStatusFile::load(config.as_ref()) {
        Ok(status) => status,
        Err(err) => {
            warn!(
                "Unable to load topology_runtime_status.json: {err}; keeping last-known-good effective parent cache"
            );
            return;
        }
    };
    let identity = topology_status_identity(&status);
    if !topology_status_identity_changed(&identity) {
        debug!("Topology runtime status changed without shaping payload identity change");
        return;
    }
    match load_active_runtime_shaping_inputs_from_status(config.as_ref(), &status) {
        Ok(Some(shaping_inputs)) => {
            debug!("Loaded shaping inputs from active runtime status");
            publish_shaping_inputs(shaping_inputs);
            remember_topology_status_identity(identity);
        }
        Ok(None) => {
            if lqos_config::integration_ingress_enabled(config.as_ref()) {
                // Preserve the last known-good parent map so a transient publication race or
                // temporary read failure does not blank topology overlays between runtime writes.
                debug!(
                    "No active runtime shaping inputs published; keeping last-known-good effective parent cache"
                );
                remember_topology_status_identity(identity);
            } else {
                debug!("Integration ingress disabled; clearing effective parent cache");
                publish_shaping_inputs(TopologyShapingInputsFile::default());
                remember_topology_status_identity(identity);
            }
        }
        Err(err) => {
            warn!(
                "Unable to load shaping_inputs.json: {err}; keeping last-known-good effective parent cache"
            );
        }
    }
}

pub fn topology_runtime_status_watcher() -> Result<()> {
    std::thread::Builder::new()
        .name("Topology Runtime Status Watcher".to_string())
        .spawn(|| {
            debug!("Watching for topology_runtime_status.json changes");
            if let Err(e) = watch_for_topology_runtime_status_changing() {
                error!("Error watching topology_runtime_status.json: {:?}", e);
            }
        })?;
    Ok(())
}

fn watch_for_topology_runtime_status_changing() -> Result<()> {
    let Ok(config) = load_config() else {
        error!("Unable to load LibreQoS config to watch topology_runtime_status.json");
        return Err(anyhow::Error::msg(
            "Unable to load LibreQoS config for topology_runtime_status.json",
        ));
    };
    let watch_path = topology_runtime_status_path(config.as_ref());

    let mut watcher = FileWatcher::new("topology_runtime_status.json", watch_path);
    watcher.set_file_exists_callback(load_topology_runtime_status_payload);
    watcher.set_file_created_callback(load_topology_runtime_status_payload);
    watcher.set_file_changed_callback(load_topology_runtime_status_payload);
    loop {
        let result = watcher.watch();
        info!("topology_runtime_status.json watcher returned: {result:?}");
    }
}

pub fn effective_parent_for_circuit(circuit_id: &str) -> Option<RuntimeCircuitParent> {
    let circuit_key = non_empty_circuit_id_key(circuit_id)?;
    EFFECTIVE_CIRCUIT_PARENTS.load().get(&circuit_key).cloned()
}

#[derive(Clone, Copy, Debug, Default)]
struct NetworkTreeSummary {
    subtree_site_count: u32,
    subtree_circuit_count: u32,
    subtree_device_count: u32,
}

/// Clones a network node into its transport form and overlays effective inherited limits when
/// the active queue structure contains a matching node entry.
pub fn node_to_transport(node: &NetworkJsonNode) -> NetworkJsonTransport {
    node_to_transport_with_summary(node, NetworkTreeSummary::default())
}

fn node_to_transport_with_summary(
    node: &NetworkJsonNode,
    summary: NetworkTreeSummary,
) -> NetworkJsonTransport {
    let mut transport = node.clone_to_transit();
    transport.runtime_virtualized = is_runtime_virtualized_node(&node.name);
    transport.configured_max_throughput = node.max_throughput;
    transport.effective_max_throughput = EFFECTIVE_NODE_RATES.load().get(&node.name).copied();
    transport.subtree_site_count = summary.subtree_site_count;
    transport.subtree_circuit_count = summary.subtree_circuit_count;
    transport.subtree_device_count = summary.subtree_device_count;
    transport
}

fn build_network_tree_summaries(
    nodes: &[NetworkJsonNode],
    shaped_devices: &lqos_network_devices::NetworkDevicesCatalog,
) -> Vec<NetworkTreeSummary> {
    let mut summaries = vec![NetworkTreeSummary::default(); nodes.len()];
    let mut direct_circuits = vec![FxHashSet::default(); nodes.len()];
    let mut node_index_by_name = FxHashMap::default();
    node_index_by_name.reserve(nodes.len());

    for (idx, node) in nodes.iter().enumerate() {
        node_index_by_name.entry(node.name.as_str()).or_insert(idx);
    }

    for device in shaped_devices.iter_all_devices() {
        let Some(node_idx) = node_index_by_name.get(device.parent_node.as_str()).copied() else {
            continue;
        };
        summaries[node_idx].subtree_device_count =
            summaries[node_idx].subtree_device_count.saturating_add(1);
        direct_circuits[node_idx].insert(device.circuit_hash);
    }

    for (idx, circuits) in direct_circuits.iter().enumerate() {
        summaries[idx].subtree_circuit_count = circuits.len() as u32;
    }

    for idx in (1..nodes.len()).rev() {
        let Some(parent_idx) = nodes[idx].immediate_parent else {
            continue;
        };
        summaries[parent_idx].subtree_site_count = summaries[parent_idx]
            .subtree_site_count
            .saturating_add(1)
            .saturating_add(summaries[idx].subtree_site_count);
        summaries[parent_idx].subtree_circuit_count = summaries[parent_idx]
            .subtree_circuit_count
            .saturating_add(summaries[idx].subtree_circuit_count);
        summaries[parent_idx].subtree_device_count = summaries[parent_idx]
            .subtree_device_count
            .saturating_add(summaries[idx].subtree_device_count);
    }

    summaries
}

pub fn get_one_network_map_layer(parent_idx: usize) -> BusResponse {
    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes_ref = net_json.get_nodes_when_ready();
        let shaped_devices = lqos_network_devices::network_devices_catalog();
        let summaries = build_network_tree_summaries(nodes_ref, &shaped_devices);
        if let Some(parent) = nodes_ref.get(parent_idx) {
            let mut nodes = vec![(
                parent_idx,
                node_to_transport_with_summary(
                    parent,
                    summaries.get(parent_idx).copied().unwrap_or_default(),
                ),
            )];
            nodes.extend(
                nodes_ref
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| node.immediate_parent == Some(parent_idx))
                    .map(|(i, node)| {
                        (
                            i,
                            node_to_transport_with_summary(
                                node,
                                summaries.get(i).copied().unwrap_or_default(),
                            ),
                        )
                    }),
            );
            BusResponse::NetworkMap(nodes)
        } else {
            BusResponse::Fail("No such node".to_string())
        }
    })
}

pub fn full_network_map_snapshot() -> Vec<(usize, NetworkJsonTransport)> {
    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        let shaped_devices = lqos_network_devices::network_devices_catalog();
        let summaries = build_network_tree_summaries(nodes, &shaped_devices);
        nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                (
                    i,
                    node_to_transport_with_summary(
                        n,
                        summaries.get(i).copied().unwrap_or_default(),
                    ),
                )
            })
            .collect()
    })
}

fn node_to_transport_lite(node: &NetworkJsonNode) -> NetworkTreeLiteNode {
    let download =
        node.rtt_buffer
            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50);
    let upload =
        node.rtt_buffer
            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50);

    let rtts = match (download, upload) {
        (None, None) => Vec::new(),
        (Some(d), None) => vec![d.as_millis() as f32; 2],
        (None, Some(u)) => vec![u.as_millis() as f32; 2],
        (Some(d), Some(u)) => vec![d.as_millis() as f32, u.as_millis() as f32],
    };

    let qoo = node
        .qoq_heatmap
        .as_ref()
        .map(|heatmap| {
            let blocks = heatmap.blocks();
            let latest = |values: &[Option<f32>]| values.iter().rev().find_map(|v| *v);
            (latest(&blocks.download_total), latest(&blocks.upload_total))
        })
        .unwrap_or((None, None));

    NetworkTreeLiteNode {
        name: node.name.clone(),
        id: node.id.clone(),
        is_virtual: node.virtual_node,
        runtime_virtualized: is_runtime_virtualized_node(&node.name),
        max_throughput: node.max_throughput,
        current_throughput: (
            node.current_throughput.get_down(),
            node.current_throughput.get_up(),
        ),
        current_tcp_packets: (
            node.current_tcp_packets.get_down(),
            node.current_tcp_packets.get_up(),
        ),
        current_tcp_retransmit_packets: (
            node.current_tcp_retransmit_packets.get_down(),
            node.current_tcp_retransmit_packets.get_up(),
        ),
        current_retransmits: (
            node.current_tcp_retransmits.get_down(),
            node.current_tcp_retransmits.get_up(),
        ),
        rtts,
        qoo,
        parents: node.parents.clone(),
        immediate_parent: node.immediate_parent,
        node_type: node.node_type.clone(),
        latitude: node.latitude,
        longitude: node.longitude,
    }
}

/// Returns a lightweight live snapshot of the network tree for pages that do not need the full
/// `NetworkJsonTransport` payload.
pub fn full_network_map_lite_snapshot() -> Vec<(usize, NetworkTreeLiteNode)> {
    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (i, node_to_transport_lite(n)))
            .collect()
    })
}

pub fn get_full_network_map() -> BusResponse {
    BusResponse::NetworkMap(full_network_map_snapshot())
}

pub fn get_top_n_root_queues(n_queues: usize) -> BusResponse {
    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes_ref = net_json.get_nodes_when_ready();
        let shaped_devices = lqos_network_devices::network_devices_catalog();
        let summaries = build_network_tree_summaries(nodes_ref, &shaped_devices);
        if let Some(parent) = nodes_ref.first() {
            let mut nodes = vec![(
                0,
                node_to_transport_with_summary(
                    parent,
                    summaries.first().copied().unwrap_or_default(),
                ),
            )];
            nodes.extend(
                nodes_ref
                    .iter()
                    .enumerate()
                    .filter(|(idx, node)| *idx != 0 && node.immediate_parent == Some(0))
                    .map(|(idx, node)| {
                        (
                            idx,
                            node_to_transport_with_summary(
                                node,
                                summaries.get(idx).copied().unwrap_or_default(),
                            ),
                        )
                    }),
            );
            // Remove the top-level entry for root
            nodes.remove(0);
            // Sort by total bandwidth (up + down) descending
            nodes.sort_by(|a, b| {
                let total_a = a.1.current_throughput.0 + a.1.current_throughput.1;
                let total_b = b.1.current_throughput.0 + b.1.current_throughput.1;
                total_b.cmp(&total_a)
            });
            // Summarize everything after n_queues
            if nodes.len() > n_queues {
                let mut other_bw = (0, 0);
                let mut other_packets = (0, 0);
                let mut other_tcp_packets = (0, 0);
                let mut other_tcp_retransmit_packets = (0, 0);
                let mut other_udp_packets = (0, 0);
                let mut other_icmp_packets = (0, 0);
                let mut other_xmit = (0, 0);
                let mut other_marks = (0, 0);
                let mut other_drops = (0, 0);
                nodes.drain(n_queues..).for_each(|n| {
                    other_bw.0 += n.1.current_throughput.0;
                    other_bw.1 += n.1.current_throughput.1;
                    other_packets.0 += n.1.current_packets.0;
                    other_packets.1 += n.1.current_packets.1;
                    other_tcp_packets.0 += n.1.current_tcp_packets.0;
                    other_tcp_packets.1 += n.1.current_tcp_packets.1;
                    other_tcp_retransmit_packets.0 += n.1.current_tcp_retransmit_packets.0;
                    other_tcp_retransmit_packets.1 += n.1.current_tcp_retransmit_packets.1;
                    other_udp_packets.0 += n.1.current_udp_packets.0;
                    other_udp_packets.1 += n.1.current_udp_packets.1;
                    other_icmp_packets.0 += n.1.current_icmp_packets.0;
                    other_icmp_packets.1 += n.1.current_icmp_packets.1;
                    other_xmit.0 += n.1.current_retransmits.0;
                    other_xmit.1 += n.1.current_retransmits.1;
                    other_marks.0 += n.1.current_marks.0;
                    other_marks.1 += n.1.current_marks.1;
                    other_drops.0 += n.1.current_drops.0;
                    other_drops.1 += n.1.current_drops.1;
                });

                nodes.push((
                    0,
                    NetworkJsonTransport {
                        name: "Others".into(),
                        id: None,
                        is_virtual: false,
                        runtime_virtualized: false,
                        max_throughput: (0.0, 0.0),
                        configured_max_throughput: (0.0, 0.0),
                        effective_max_throughput: None,
                        current_throughput: other_bw,
                        current_packets: other_packets,
                        current_tcp_packets: other_tcp_packets,
                        current_tcp_retransmit_packets: other_tcp_retransmit_packets,
                        current_udp_packets: other_udp_packets,
                        current_icmp_packets: other_icmp_packets,
                        current_retransmits: other_xmit,
                        current_marks: other_marks,
                        current_drops: other_drops,
                        rtts: Vec::new(),
                        qoo: (None, None),
                        parents: Vec::new(),
                        immediate_parent: None,
                        node_type: None,
                        latitude: None,
                        longitude: None,
                        active_attachment_name: None,
                        subtree_site_count: 0,
                        subtree_circuit_count: 0,
                        subtree_device_count: 0,
                    },
                ));
            }
            BusResponse::NetworkMap(nodes)
        } else {
            BusResponse::Fail("No such node".to_string())
        }
    })
}

pub fn map_node_names(nodes: &[usize]) -> BusResponse {
    lqos_network_devices::with_network_json_read(|net_json| {
        let mut result = Vec::new();
        nodes.iter().for_each(|id| {
            if let Some(node) = net_json.get_nodes_when_ready().get(*id) {
                result.push((*id, node.name.clone()));
            }
        });
        BusResponse::NodeNames(result)
    })
}

pub fn get_funnel(circuit_id: &str) -> BusResponse {
    lqos_network_devices::with_network_json_read(|net_json| {
        if let Some(index) = net_json.get_index_for_name(circuit_id) {
            // Reverse the scanning order and skip the last entry (the parent)
            let mut result = Vec::new();
            for idx in net_json.get_nodes_when_ready()[index]
                .parents
                .iter()
                .rev()
                .skip(1)
            {
                result.push((
                    *idx,
                    node_to_transport(&net_json.get_nodes_when_ready()[*idx]),
                ));
            }
            return BusResponse::NetworkMap(result);
        }

        BusResponse::Fail("Unknown Node".into())
    })
}

struct PendingCircuitParent {
    circuit: Circuit,
    configured_parent: Option<String>,
}

fn snapshot_circuits(desired_circuit_id: Option<&str>) -> Vec<PendingCircuitParent> {
    let Ok(kernel_now) = time_since_boot() else {
        return Vec::new();
    };
    let since_boot_nanos = Duration::from(kernel_now).as_nanos();
    let desired_hash = desired_circuit_id.map(hash_to_i64);
    let catalog = lqos_network_devices::network_devices_catalog();

    let pending = {
        let raw_data = THROUGHPUT_TRACKER.raw_data.lock();
        raw_data
            .iter()
            .filter_map(|(ip, entry)| {
                let device = catalog
                    .device_by_hashes(entry.device_hash, entry.circuit_hash)
                    .or_else(|| {
                        catalog
                            .device_longest_match_for_ip(ip)
                            .map(|(_, device)| device)
                    });
                if let Some(desired_circuit_id) = desired_circuit_id {
                    let desired_hash = desired_hash.expect("desired hash accompanies desired id");
                    let matches_desired = entry.circuit_hash == Some(desired_hash)
                        || entry.circuit_id.as_deref() == Some(desired_circuit_id)
                        || device.is_some_and(|device| device.circuit_hash == desired_hash)
                        || device.is_some_and(|device| device.circuit_id == desired_circuit_id);
                    if !matches_desired {
                        return None;
                    }
                }

                let mut circuit_id = entry.circuit_id.clone();
                let mut circuit_name = None;
                let mut device_id = None;
                let mut device_name = None;
                let mut parent_node = None;
                let mut configured_parent = None;
                let mut plan = DownUpOrder { down: 0.0, up: 0.0 };
                if let Some(device) = device {
                    if circuit_id.as_deref().unwrap_or_default().is_empty() {
                        circuit_id = Some(device.circuit_id.clone());
                    }
                    circuit_name = Some(device.circuit_name.clone());
                    device_id = Some(device.device_id.clone());
                    device_name = Some(device.device_name.clone());
                    if let Some(effective_parent) = effective_parent_for_circuit(&device.circuit_id)
                    {
                        parent_node = Some(effective_parent.name);
                    } else {
                        configured_parent = Some(device.parent_node.clone());
                    }
                    plan.down = device.download_max_mbps.round();
                    plan.up = device.upload_max_mbps.round();
                }
                if circuit_id.is_none() {
                    circuit_id = desired_circuit_id.map(str::to_string);
                }

                let last_seen_nanos = if entry.last_seen > 0 {
                    since_boot_nanos.saturating_sub(entry.last_seen as u128) as u64
                } else {
                    u64::MAX
                };
                let percentile = |bucket, direction, percentile| {
                    entry
                        .rtt_buffer
                        .percentile(bucket, direction, percentile)
                        .map(|rtt| rtt.as_nanos())
                };

                Some(PendingCircuitParent {
                    circuit: Circuit {
                        ip: ip.as_ip(),
                        bytes_per_second: entry.bytes_per_second,
                        actual_bytes_per_second: entry.actual_bytes_per_second,
                        median_latency: entry.median_latency(),
                        rtt_current_p50_nanos: DownUpOrder {
                            down: percentile(
                                RttBucket::Current,
                                FlowbeeEffectiveDirection::Download,
                                50,
                            ),
                            up: percentile(
                                RttBucket::Current,
                                FlowbeeEffectiveDirection::Upload,
                                50,
                            ),
                        },
                        rtt_current_p95_nanos: DownUpOrder {
                            down: percentile(
                                RttBucket::Current,
                                FlowbeeEffectiveDirection::Download,
                                95,
                            ),
                            up: percentile(
                                RttBucket::Current,
                                FlowbeeEffectiveDirection::Upload,
                                95,
                            ),
                        },
                        rtt_total_p50_nanos: DownUpOrder {
                            down: percentile(
                                RttBucket::Total,
                                FlowbeeEffectiveDirection::Download,
                                50,
                            ),
                            up: percentile(RttBucket::Total, FlowbeeEffectiveDirection::Upload, 50),
                        },
                        rtt_total_p95_nanos: DownUpOrder {
                            down: percentile(
                                RttBucket::Total,
                                FlowbeeEffectiveDirection::Download,
                                95,
                            ),
                            up: percentile(RttBucket::Total, FlowbeeEffectiveDirection::Upload, 95),
                        },
                        qoo: DownUpOrder {
                            down: entry.qoq.download_total_f32(),
                            up: entry.qoq.upload_total_f32(),
                        },
                        tcp_retransmit_sample: down_up_retransmit_sample(
                            entry.tcp_retransmits,
                            entry.tcp_retransmit_packets,
                        ),
                        circuit_id,
                        device_id,
                        circuit_name,
                        device_name,
                        parent_node,
                        plan,
                        last_seen_nanos,
                    },
                    configured_parent,
                })
            })
            .collect()
    };
    #[cfg(test)]
    {
        let mut hook = CIRCUIT_SNAPSHOT_TEST_HOOK.lock();
        if hook
            .as_ref()
            .is_some_and(|(thread_id, _)| *thread_id == std::thread::current().id())
            && let Some((_, sender)) = hook.take()
        {
            let _ = sender.send(());
        }
    }
    pending
}

fn resolve_pending_circuit_parents_with(
    pending: Vec<PendingCircuitParent>,
    mut resolve_parent: impl FnMut(&str) -> Option<String>,
) -> Vec<Circuit> {
    let mut resolved_parents: FxHashMap<String, String> = FxHashMap::default();
    pending
        .into_iter()
        .map(|mut pending| {
            if let Some(configured_parent) = pending.configured_parent {
                let resolved_parent =
                    if let Some(resolved) = resolved_parents.get(&configured_parent) {
                        resolved.clone()
                    } else {
                        let resolved = resolve_parent(&configured_parent)
                            .unwrap_or_else(|| configured_parent.clone());
                        resolved_parents.insert(configured_parent, resolved.clone());
                        resolved
                    };
                pending.circuit.parent_node = Some(resolved_parent);
            }
            pending.circuit
        })
        .collect()
}

fn resolve_pending_circuit_parents(pending: Vec<PendingCircuitParent>) -> Vec<Circuit> {
    if pending
        .iter()
        .all(|pending| pending.configured_parent.is_none())
    {
        return pending.into_iter().map(|pending| pending.circuit).collect();
    }

    let configured_parents = pending
        .iter()
        .filter_map(|pending| pending.configured_parent.as_deref())
        .collect::<FxHashSet<_>>();
    let resolved_parents = lqos_network_devices::with_network_json_read(|network_json| {
        let lookup =
            lqos_network_devices::ParentNodeLookup::from_nodes(network_json.get_nodes_when_ready());
        configured_parents
            .into_iter()
            .filter_map(|parent| {
                lookup
                    .resolve(parent, None)
                    .map(|resolved| (parent.to_string(), resolved.name))
            })
            .collect::<FxHashMap<_, _>>()
    });
    resolve_pending_circuit_parents_with(pending, |parent| resolved_parents.get(parent).cloned())
}

pub fn get_all_circuits() -> BusResponse {
    BusResponse::CircuitData(resolve_pending_circuit_parents(snapshot_circuits(None)))
}

pub fn get_circuit_by_id(desired_circuit_id: String) -> BusResponse {
    BusResponse::CircuitData(resolve_pending_circuit_parents(snapshot_circuits(Some(
        &desired_circuit_id,
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::runtime_config_test_lock;
    use crate::throughput_tracker::{
        RawThroughputTestEntry, RttBuffer, replace_raw_throughput_for_test,
    };
    use lqos_config::{
        Config, ConfigShapedDevices, ShapedDevice, TOPOLOGY_RUNTIME_STATUS_FILENAME,
        TopologyShapingCircuitInput, TopologyShapingDeviceInput, TopologyShapingInputsFile,
        compute_effective_network_file_generation, compute_shaping_inputs_file_generation,
    };
    use lqos_utils::XdpIpAddress;
    use std::cell::Cell;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct CircuitLookupStateGuard {
        old_shaped_devices: Option<Arc<ConfigShapedDevices>>,
        old_network_nodes: Option<Vec<NetworkJsonNode>>,
        old_effective_parents: Option<Arc<FxHashMap<String, RuntimeCircuitParent>>>,
    }

    struct CircuitSnapshotTestHookGuard;

    impl Drop for CircuitSnapshotTestHookGuard {
        fn drop(&mut self) {
            CIRCUIT_SNAPSHOT_TEST_HOOK.lock().take();
        }
    }

    impl Drop for CircuitLookupStateGuard {
        fn drop(&mut self) {
            if let Some(nodes) = self.old_network_nodes.take() {
                lqos_network_devices::with_network_json_write(|network_json| {
                    network_json.nodes = nodes;
                });
            }
            if let Some(shaped_devices) = self.old_shaped_devices.take() {
                lqos_network_devices::swap_shaped_devices_snapshot(
                    "circuit-lookup-test-restore",
                    shaped_devices,
                );
            }
            if let Some(effective_parents) = self.old_effective_parents.take() {
                EFFECTIVE_CIRCUIT_PARENTS.store(effective_parents);
            }
        }
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough for tests")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
        fs::create_dir_all(&path).expect("temp directory should be creatable");
        path
    }

    fn write_runtime_status(
        path: &std::path::Path,
        ready: bool,
        shaping_inputs_path: &std::path::Path,
        source_generation: &str,
    ) {
        let effective_network_path = path
            .parent()
            .expect("runtime status path should have a parent")
            .join("network.effective.json");
        fs::write(&effective_network_path, "{}\n").expect("effective network should write");
        let shaping_generation = compute_shaping_inputs_file_generation(shaping_inputs_path)
            .expect("shaping generation should compute");
        let effective_generation =
            compute_effective_network_file_generation(&effective_network_path)
                .expect("effective generation should compute");

        fs::write(
            path,
            serde_json::json!({
                "schema_version": 1,
                "ready": ready,
                "shaping_inputs_path": shaping_inputs_path,
                "effective_state_path": "",
                "effective_network_path": effective_network_path,
                "source_generation": source_generation,
                "shaping_generation": shaping_generation,
                "effective_generation": effective_generation,
            })
            .to_string(),
        )
        .expect("status should write");
    }

    fn pending_circuit(
        parent_node: Option<&str>,
        configured_parent: Option<&str>,
    ) -> PendingCircuitParent {
        PendingCircuitParent {
            circuit: Circuit {
                ip: "192.0.2.1".parse().expect("test IP should parse"),
                bytes_per_second: DownUpOrder::default(),
                actual_bytes_per_second: DownUpOrder::default(),
                median_latency: None,
                rtt_current_p50_nanos: DownUpOrder::default(),
                rtt_current_p95_nanos: DownUpOrder::default(),
                rtt_total_p50_nanos: DownUpOrder::default(),
                rtt_total_p95_nanos: DownUpOrder::default(),
                qoo: DownUpOrder::default(),
                tcp_retransmit_sample: DownUpOrder::default(),
                circuit_id: Some("test-circuit".to_string()),
                device_id: Some("test-device".to_string()),
                parent_node: parent_node.map(str::to_string),
                circuit_name: Some("Test Circuit".to_string()),
                device_name: Some("Test Device".to_string()),
                plan: DownUpOrder::default(),
                last_seen_nanos: 0,
            },
            configured_parent: configured_parent.map(str::to_string),
        }
    }

    #[test]
    fn pending_circuit_parent_resolution_is_cached_and_fallback_safe() {
        let pending = vec![
            pending_circuit(None, Some("Tower Alias")),
            pending_circuit(None, Some("Tower Alias")),
            pending_circuit(None, Some("Unknown Parent")),
            pending_circuit(Some("Effective Parent"), None),
        ];
        let resolution_calls = Cell::new(0usize);

        let circuits = resolve_pending_circuit_parents_with(pending, |parent| {
            resolution_calls.set(resolution_calls.get() + 1);
            (parent == "Tower Alias").then(|| "Canonical Tower".to_string())
        });

        assert_eq!(resolution_calls.get(), 2);
        assert_eq!(circuits[0].parent_node.as_deref(), Some("Canonical Tower"));
        assert_eq!(circuits[1].parent_node.as_deref(), Some("Canonical Tower"));
        assert_eq!(circuits[2].parent_node.as_deref(), Some("Unknown Parent"));
        assert_eq!(circuits[3].parent_node.as_deref(), Some("Effective Parent"));
    }

    #[test]
    fn circuit_bus_readers_release_raw_data_before_resolving_parents() {
        let _runtime_guard = runtime_config_test_lock()
            .lock()
            .expect("runtime config test lock should not be poisoned");
        let effective_circuit_id = "deadlock-effective-circuit";
        let effective_device_id = "deadlock-effective-device";
        let alias_circuit_id = "deadlock-alias-circuit";
        let alias_device_id = "deadlock-alias-device";
        let effective_circuit_hash = hash_to_i64(effective_circuit_id);
        let effective_device_hash = hash_to_i64(effective_device_id);
        let alias_circuit_hash = hash_to_i64(alias_circuit_id);
        let alias_device_hash = hash_to_i64(alias_device_id);
        let mut shaped_devices = ConfigShapedDevices::default();
        shaped_devices.replace_with_new_data(vec![
            ShapedDevice {
                circuit_id: effective_circuit_id.to_string(),
                circuit_name: "Effective Parent Circuit".to_string(),
                device_id: effective_device_id.to_string(),
                device_name: "Effective Parent Device".to_string(),
                parent_node: "Tower Alias".to_string(),
                circuit_hash: effective_circuit_hash,
                device_hash: effective_device_hash,
                download_max_mbps: 100.0,
                upload_max_mbps: 20.0,
                ..ShapedDevice::default()
            },
            ShapedDevice {
                circuit_id: alias_circuit_id.to_string(),
                circuit_name: "Alias Parent Circuit".to_string(),
                device_id: alias_device_id.to_string(),
                device_name: "Alias Parent Device".to_string(),
                parent_node: "Tower Alias".to_string(),
                circuit_hash: alias_circuit_hash,
                device_hash: alias_device_hash,
                download_max_mbps: 200.0,
                upload_max_mbps: 40.0,
                ..ShapedDevice::default()
            },
        ]);
        let old_shaped_devices = lqos_network_devices::swap_shaped_devices_snapshot(
            "circuit-lookup-test",
            Arc::new(shaped_devices),
        );
        let old_network_nodes =
            lqos_network_devices::with_network_json_read(|network_json| network_json.nodes.clone());
        let mut effective_parents = FxHashMap::default();
        effective_parents.insert(
            effective_circuit_id.to_string(),
            RuntimeCircuitParent {
                name: "Effective Tower".to_string(),
                id: Some("effective-tower-id".to_string()),
            },
        );
        let old_effective_parents = EFFECTIVE_CIRCUIT_PARENTS.swap(Arc::new(effective_parents));
        let _state_guard = CircuitLookupStateGuard {
            old_shaped_devices: Some(old_shaped_devices),
            old_network_nodes: Some(old_network_nodes),
            old_effective_parents: Some(old_effective_parents),
        };
        lqos_network_devices::with_network_json_write(|network_json| {
            network_json.nodes = vec![NetworkJsonNode {
                name: "Canonical Tower".to_string(),
                id: Some("canonical-tower-id".to_string()),
                virtual_node: false,
                max_throughput: (0.0, 0.0),
                current_throughput: DownUpOrder::default(),
                current_packets: DownUpOrder::default(),
                current_tcp_packets: DownUpOrder::default(),
                current_udp_packets: DownUpOrder::default(),
                current_icmp_packets: DownUpOrder::default(),
                current_tcp_retransmits: DownUpOrder::default(),
                current_tcp_retransmit_packets: DownUpOrder::default(),
                current_marks: DownUpOrder::default(),
                current_drops: DownUpOrder::default(),
                rtt_buffer: RttBuffer::default(),
                parents: Vec::new(),
                immediate_parent: None,
                node_type: None,
                latitude: None,
                longitude: None,
                active_attachment_name: Some("Tower Alias".to_string()),
                heatmap: None,
                qoq_heatmap: None,
            }];
        });
        let _raw_guard = replace_raw_throughput_for_test(
            1,
            vec![
                RawThroughputTestEntry {
                    ip: XdpIpAddress::from_ip("192.0.2.10".parse().expect("test IP should parse")),
                    circuit_hash: Some(effective_circuit_hash),
                    device_hash: Some(effective_device_hash),
                    most_recent_cycle: 1,
                    bytes_per_second: DownUpOrder::new(1_000, 200),
                    tcp_packets: DownUpOrder::default(),
                    tcp_retransmits: DownUpOrder::default(),
                },
                RawThroughputTestEntry {
                    ip: XdpIpAddress::from_ip("192.0.2.11".parse().expect("test IP should parse")),
                    circuit_hash: Some(alias_circuit_hash),
                    device_hash: Some(alias_device_hash),
                    most_recent_cycle: 1,
                    bytes_per_second: DownUpOrder::new(2_000, 400),
                    tcp_packets: DownUpOrder::default(),
                    tcp_retransmits: DownUpOrder::default(),
                },
            ],
        );

        let (network_locked_tx, network_locked_rx) = mpsc::channel();
        let (release_network_tx, release_network_rx) = mpsc::channel();
        let network_writer = thread::spawn(move || {
            lqos_network_devices::with_network_json_write(|_| {
                network_locked_tx
                    .send(())
                    .expect("test should signal network lock acquisition");
                let _ = release_network_rx.recv_timeout(Duration::from_secs(2));
            });
        });
        network_locked_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("network writer should acquire the lock");

        let (start_reader_tx, start_reader_rx) = mpsc::channel();
        let (snapshot_complete_tx, snapshot_complete_rx) = mpsc::channel();
        let circuit_reader = thread::spawn(move || {
            start_reader_rx
                .recv()
                .expect("test should start the circuit reader");
            get_all_circuits()
        });
        *CIRCUIT_SNAPSHOT_TEST_HOOK.lock() =
            Some((circuit_reader.thread().id(), snapshot_complete_tx));
        let _hook_guard = CircuitSnapshotTestHookGuard;
        start_reader_tx
            .send(())
            .expect("test should start the circuit reader");
        snapshot_complete_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("circuit reader should finish its raw-data snapshot");
        let raw_data_was_released = THROUGHPUT_TRACKER.raw_data.try_lock().is_some();

        release_network_tx
            .send(())
            .expect("test should release the network writer");
        network_writer
            .join()
            .expect("network writer should finish cleanly");
        let BusResponse::CircuitData(all_circuits) = circuit_reader
            .join()
            .expect("circuit reader should finish cleanly")
        else {
            panic!("GetAllCircuits should return circuit data");
        };
        assert!(
            raw_data_was_released,
            "raw_data must be unlocked before waiting for network.json"
        );
        assert_eq!(all_circuits.len(), 2);
        let effective_circuit = all_circuits
            .iter()
            .find(|circuit| circuit.circuit_id.as_deref() == Some(effective_circuit_id))
            .expect("effective-parent circuit should be present");
        assert_eq!(
            effective_circuit.parent_node.as_deref(),
            Some("Effective Tower")
        );
        let alias_circuit = all_circuits
            .iter()
            .find(|circuit| circuit.circuit_id.as_deref() == Some(alias_circuit_id))
            .expect("alias-parent circuit should be present");
        assert_eq!(
            alias_circuit.parent_node.as_deref(),
            Some("Canonical Tower")
        );

        let BusResponse::CircuitData(selected_circuits) =
            get_circuit_by_id(alias_circuit_id.to_string())
        else {
            panic!("GetCircuitById should return circuit data");
        };
        assert_eq!(selected_circuits.len(), 1);
        assert_eq!(
            selected_circuits[0].circuit_id.as_deref(),
            Some(alias_circuit_id)
        );
        assert_eq!(
            selected_circuits[0].parent_node.as_deref(),
            Some("Canonical Tower")
        );
    }

    #[test]
    fn effective_circuit_parent_map_uses_effective_parent_fields() {
        let shaping_inputs = TopologyShapingInputsFile {
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "Circuit-100".to_string(),
                effective_parent_node_name: "Live Parent".to_string(),
                effective_parent_node_id: "node-100".to_string(),
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        };

        let map = build_effective_circuit_parent_map(&shaping_inputs);
        let parent = map
            .get("circuit-100")
            .expect("expected normalized circuit id entry");
        assert_eq!(parent.name, "Live Parent");
        assert_eq!(parent.id.as_deref(), Some("node-100"));
    }

    #[test]
    fn effective_circuit_parent_map_skips_empty_parent_names() {
        let shaping_inputs = TopologyShapingInputsFile {
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "Circuit-200".to_string(),
                effective_parent_node_name: "   ".to_string(),
                effective_parent_node_id: "node-200".to_string(),
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        };

        let map = build_effective_circuit_parent_map(&shaping_inputs);
        assert!(map.is_empty());
    }

    #[test]
    fn load_active_runtime_shaping_inputs_prefers_runtime_status_path_over_state_fallback() {
        let lqos_directory = unique_temp_dir("lqosd-shaped-devices-runtime-status");
        let state_directory = lqos_directory.join("state");
        fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");
        fs::create_dir_all(state_directory.join("shaping")).expect("shaping dir should exist");

        let active_shaping_path = lqos_directory.join("shaping_inputs.json");
        let stale_state_path = state_directory.join("shaping").join("shaping_inputs.json");
        let status_path = state_directory
            .join("topology")
            .join(TOPOLOGY_RUNTIME_STATUS_FILENAME);

        let active_inputs = TopologyShapingInputsFile {
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "active-circuit".to_string(),
                effective_parent_node_name: "Parent-A".to_string(),
                devices: vec![TopologyShapingDeviceInput {
                    device_id: "device-1".to_string(),
                    device_name: "Device Alpha".to_string(),
                    ipv4: vec!["192.168.10.5/32".to_string()],
                    ..TopologyShapingDeviceInput::default()
                }],
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        };
        let stale_inputs = TopologyShapingInputsFile {
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "stale-circuit".to_string(),
                effective_parent_node_name: "Stale Parent".to_string(),
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        };

        fs::write(
            &active_shaping_path,
            serde_json::to_string_pretty(&active_inputs).expect("active shaping should encode"),
        )
        .expect("active shaping should write");
        fs::write(
            &stale_state_path,
            serde_json::to_string_pretty(&stale_inputs).expect("stale shaping should encode"),
        )
        .expect("stale shaping should write");

        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: Some(state_directory.to_string_lossy().to_string()),
            ..Config::default()
        };
        let mut config = config;
        config.uisp_integration.enable_uisp = true;
        let source_generation = lqos_config::compute_topology_source_generation(&config)
            .expect("generation should compute");
        write_runtime_status(&status_path, true, &active_shaping_path, &source_generation);

        let loaded = load_active_runtime_shaping_inputs(&config)
            .expect("runtime shaping should load")
            .expect("runtime shaping inputs should be active");
        assert_eq!(loaded.circuits.len(), 1);
        assert_eq!(loaded.circuits[0].circuit_id, "active-circuit");
    }

    #[test]
    fn load_shaped_devices_from_preferred_source_uses_runtime_inputs_before_csv() {
        let lqos_directory = unique_temp_dir("lqosd-shaped-devices-source-order");
        let state_directory = lqos_directory.join("state");
        fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");
        fs::create_dir_all(state_directory.join("shaping")).expect("shaping dir should exist");

        let active_shaping_path = lqos_directory.join("shaping_inputs.json");
        let status_path = state_directory
            .join("topology")
            .join(TOPOLOGY_RUNTIME_STATUS_FILENAME);
        let csv_path = lqos_directory.join("ShapedDevices.csv");

        let active_inputs = TopologyShapingInputsFile {
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "runtime-circuit".to_string(),
                circuit_name: "Runtime Circuit".to_string(),
                effective_parent_node_name: "Runtime Parent".to_string(),
                effective_parent_node_id: "runtime-parent-id".to_string(),
                devices: vec![TopologyShapingDeviceInput {
                    device_id: "runtime-device".to_string(),
                    device_name: "Runtime Device".to_string(),
                    ipv4: vec!["192.168.44.9/32".to_string()],
                    ipv6: vec!["2001:db8::44/128".to_string()],
                    ..TopologyShapingDeviceInput::default()
                }],
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        };

        fs::write(
            &active_shaping_path,
            serde_json::to_string_pretty(&active_inputs).expect("active shaping should encode"),
        )
        .expect("active shaping should write");
        fs::write(
            &csv_path,
            "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\ncsv-circuit,CSV Circuit,csv-device,CSV Device,CSV Parent,aa:bb:cc:dd:ee:ff,192.168.55.10/32,,0,0,100,100,\n",
        )
        .expect("csv should write");

        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: Some(state_directory.to_string_lossy().to_string()),
            ..Config::default()
        };
        let source_generation = lqos_config::compute_topology_source_generation(&config)
            .expect("generation should compute");
        write_runtime_status(&status_path, true, &active_shaping_path, &source_generation);

        let (loaded, source) = load_shaped_devices_from_preferred_source(&config)
            .expect("preferred source should load");

        assert_eq!(source, ShapedDevicesLoadSource::RuntimeShapingInputs);
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].circuit_id, "runtime-circuit");
        assert_eq!(loaded.devices[0].parent_node, "Runtime Parent");
        assert_eq!(
            loaded.devices[0].parent_node_id.as_deref(),
            Some("runtime-parent-id")
        );
        assert_eq!(
            loaded.devices[0].ipv4,
            vec![(Ipv4Addr::new(192, 168, 44, 9), 32)]
        );
        assert_eq!(
            loaded.devices[0].ipv6,
            vec![("2001:db8::44".parse::<Ipv6Addr>().expect("valid ipv6"), 128)]
        );
    }
}

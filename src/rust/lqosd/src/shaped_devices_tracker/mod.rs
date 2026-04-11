use crate::node_manager::local_api::network_tree_lite::NetworkTreeLiteNode;
use crate::treeguard::actor::is_runtime_virtualized_node;
use anyhow::Result;
use arc_swap::ArcSwap;
use fxhash::{FxHashMap, FxHashSet};
use lqos_bus::{BusResponse, Circuit};
use lqos_config::{
    ConfigShapedDevices, NetworkJsonNode, NetworkJsonTransport, ShapedDevice,
    TopologyRuntimeStatusFile, TopologyShapingInputsFile, load_config, topology_import_path,
    topology_runtime_status_path,
};
use lqos_queue_tracker::EFFECTIVE_NODE_RATES;
use lqos_topology_compile::TopologyImportFile;
use lqos_utils::XdpIpAddress;
use lqos_utils::file_watcher::FileWatcher;
use lqos_utils::hash_to_i64;
use lqos_utils::rtt::{FlowbeeEffectiveDirection, RttBucket};
use lqos_utils::units::{DownUpOrder, down_up_retransmit_sample};
use lqos_utils::unix_time::time_since_boot;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;
use tracing::{debug, error, info, warn};

// Removed rate_for_plan() function - no longer needed with f32 plan structures
const SHAPED_DEVICES_RELOAD_RETRY_DELAY_MS: u64 = 500;
const SHAPED_DEVICES_RELOAD_ATTEMPTS: usize = 2;

pub mod circuit_live;
mod netjson;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
pub use circuit_live::CircuitLiveSnapshot;
pub use netjson::*;

pub static SHAPED_DEVICES: Lazy<ArcSwap<ConfigShapedDevices>> =
    Lazy::new(|| ArcSwap::new(Arc::new(ConfigShapedDevices::default())));

#[derive(Debug, Default)]
pub struct ShapedDeviceHashCache {
    by_device_hash: FxHashMap<i64, usize>,
    by_circuit_hash: FxHashMap<i64, usize>,
}

impl ShapedDeviceHashCache {
    fn from_devices(devices: &[ShapedDevice]) -> Self {
        let mut by_device_hash = FxHashMap::default();
        by_device_hash.reserve(devices.len());
        let mut by_circuit_hash = FxHashMap::default();
        by_circuit_hash.reserve(devices.len());
        for (idx, dev) in devices.iter().enumerate() {
            by_device_hash.insert(dev.device_hash, idx);
            by_circuit_hash.entry(dev.circuit_hash).or_insert(idx);
        }
        Self {
            by_device_hash,
            by_circuit_hash,
        }
    }

    pub fn index_by_device_hash(
        &self,
        shaped: &ConfigShapedDevices,
        device_hash: i64,
    ) -> Option<usize> {
        if let Some(idx) = self.by_device_hash.get(&device_hash).copied()
            && shaped
                .devices
                .get(idx)
                .is_some_and(|d| d.device_hash == device_hash)
        {
            return Some(idx);
        }
        shaped
            .devices
            .iter()
            .position(|d| d.device_hash == device_hash)
    }

    pub fn index_by_circuit_hash(
        &self,
        shaped: &ConfigShapedDevices,
        circuit_hash: i64,
    ) -> Option<usize> {
        if let Some(idx) = self.by_circuit_hash.get(&circuit_hash).copied()
            && shaped
                .devices
                .get(idx)
                .is_some_and(|d| d.circuit_hash == circuit_hash)
        {
            return Some(idx);
        }
        shaped
            .devices
            .iter()
            .position(|d| d.circuit_hash == circuit_hash)
    }
}

pub static SHAPED_DEVICE_HASH_CACHE: Lazy<ArcSwap<ShapedDeviceHashCache>> =
    Lazy::new(|| ArcSwap::new(Arc::new(ShapedDeviceHashCache::default())));
pub static CIRCUIT_LIVE_SNAPSHOT: Lazy<ArcSwap<CircuitLiveSnapshot>> =
    Lazy::new(|| ArcSwap::new(Arc::new(CircuitLiveSnapshot::default())));
pub static CIRCUIT_LIVE_LAST_REFRESH_SECS: AtomicU64 = AtomicU64::new(0);
pub static CIRCUIT_LIVE_REFRESH_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
pub static EFFECTIVE_CIRCUIT_PARENTS: Lazy<ArcSwap<FxHashMap<String, RuntimeCircuitParent>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(FxHashMap::default())));

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

pub(crate) fn shaped_device_from_hashes_or_ip<'a>(
    shaped: &'a ConfigShapedDevices,
    cache: &ShapedDeviceHashCache,
    ip: &XdpIpAddress,
    device_hash: Option<i64>,
    circuit_hash: Option<i64>,
) -> Option<&'a ShapedDevice> {
    device_hash
        .and_then(|hash| cache.index_by_device_hash(shaped, hash))
        .or_else(|| circuit_hash.and_then(|hash| cache.index_by_circuit_hash(shaped, hash)))
        .and_then(|idx| shaped.devices.get(idx))
        .or_else(|| shaped.get_device_from_ip(ip))
}

fn normalize_circuit_id_key(circuit_id: &str) -> Option<String> {
    let trimmed = circuit_id.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_ascii_lowercase())
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
        let Some(circuit_key) = normalize_circuit_id_key(&circuit.circuit_id) else {
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

fn active_runtime_shaping_inputs_path(config: &lqos_config::Config) -> Result<Option<PathBuf>> {
    let status = TopologyRuntimeStatusFile::load(config)?;
    if !status.ready {
        return Ok(None);
    }
    let path = status.shaping_inputs_path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(path)))
}

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

fn parse_ipv4_list(values: &[String]) -> Vec<(Ipv4Addr, u32)> {
    values
        .iter()
        .filter_map(|value| parse_ipv4_entry(value))
        .collect()
}

fn parse_ipv6_list(values: &[String]) -> Vec<(Ipv6Addr, u32)> {
    values
        .iter()
        .filter_map(|value| parse_ipv6_entry(value))
        .collect()
}

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

fn load_shaping_inputs() {
    let Ok(config) = load_config() else {
        warn!("Unable to load LibreQoS config while loading shaping_inputs.json");
        return;
    };
    match load_active_runtime_shaping_inputs(config.as_ref()) {
        Ok(Some(shaping_inputs)) => {
            debug!("Loaded shaping inputs from active runtime status");
            publish_shaping_inputs(shaping_inputs);
        }
        Ok(None) => {
            debug!("No active runtime shaping inputs published; clearing effective parent cache");
            publish_shaping_inputs(TopologyShapingInputsFile::default());
        }
        Err(err) => {
            warn!("Unable to load shaping_inputs.json: {err}");
        }
    }
}

pub fn shaping_inputs_watcher() -> Result<()> {
    std::thread::Builder::new()
        .name("Shaping Inputs Watcher".to_string())
        .spawn(|| {
            debug!("Watching for shaping_inputs.json changes");
            if let Err(e) = watch_for_shaping_inputs_changing() {
                error!("Error watching for shaping_inputs.json changes: {:?}", e);
            }
        })?;
    Ok(())
}

fn watch_for_shaping_inputs_changing() -> Result<()> {
    let Ok(config) = load_config() else {
        error!("Unable to load LibreQoS config to watch shaping_inputs.json");
        return Err(anyhow::Error::msg(
            "Unable to load LibreQoS config for shaping_inputs.json",
        ));
    };
    let watch_path = topology_runtime_status_path(config.as_ref());

    let mut watcher = FileWatcher::new("topology_runtime_status.json", watch_path);
    watcher.set_file_exists_callback(load_shaping_inputs);
    watcher.set_file_created_callback(load_shaping_inputs);
    watcher.set_file_changed_callback(load_shaping_inputs);
    loop {
        let result = watcher.watch();
        info!("topology_runtime_status.json watcher returned: {result:?}");
    }
}

pub fn effective_parent_for_circuit(circuit_id: &str) -> Option<RuntimeCircuitParent> {
    let circuit_key = normalize_circuit_id_key(circuit_id)?;
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
    shaped_devices: &ConfigShapedDevices,
) -> Vec<NetworkTreeSummary> {
    let mut summaries = vec![NetworkTreeSummary::default(); nodes.len()];
    let mut direct_circuits = vec![FxHashSet::default(); nodes.len()];
    let mut node_index_by_name = FxHashMap::default();
    node_index_by_name.reserve(nodes.len());

    for (idx, node) in nodes.iter().enumerate() {
        node_index_by_name.entry(node.name.as_str()).or_insert(idx);
    }

    for device in &shaped_devices.devices {
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

fn publish_shaped_devices(new_file: ConfigShapedDevices) {
    debug!("ShapedDevices.csv loaded");
    let cache = ShapedDeviceHashCache::from_devices(&new_file.devices);
    SHAPED_DEVICES.store(Arc::new(new_file));
    SHAPED_DEVICE_HASH_CACHE.store(Arc::new(cache));
    invalidate_circuit_live_snapshot();
    invalidate_executive_cache_snapshot();
    let nj = NETWORK_JSON.read();
    crate::throughput_tracker::THROUGHPUT_TRACKER.refresh_circuit_ids(&nj);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShapedDevicesLoadSource {
    RuntimeShapingInputs,
    TopologyImport,
    ShapedDevicesCsv,
}

impl ShapedDevicesLoadSource {
    const fn label(self) -> &'static str {
        match self {
            Self::RuntimeShapingInputs => "runtime shaping inputs",
            Self::TopologyImport => "topology_import.json",
            Self::ShapedDevicesCsv => "ShapedDevices.csv",
        }
    }
}

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

fn load_active_runtime_shaping_inputs(
    config: &lqos_config::Config,
) -> Result<Option<TopologyShapingInputsFile>> {
    let Some(active_path) = active_runtime_shaping_inputs_path(config)? else {
        return Ok(None);
    };
    if !active_path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&active_path)?;
    let shaping_inputs = serde_json::from_str(&raw)?;
    Ok(Some(shaping_inputs))
}

fn load_shaped_devices_from_preferred_source(
    config: &lqos_config::Config,
) -> Result<(ConfigShapedDevices, ShapedDevicesLoadSource)> {
    if let Some(shaping_inputs) = load_active_runtime_shaping_inputs(config)? {
        return Ok((
            shaped_devices_from_runtime_inputs(&shaping_inputs),
            ShapedDevicesLoadSource::RuntimeShapingInputs,
        ));
    }

    if integration_ingress_enabled(config)
        && let Some(topology_import) = TopologyImportFile::load(config)?
    {
        return Ok((
            topology_import.into_imported_bundle().shaped_devices,
            ShapedDevicesLoadSource::TopologyImport,
        ));
    }
    Ok((
        ConfigShapedDevices::load_for_config(config)?,
        ShapedDevicesLoadSource::ShapedDevicesCsv,
    ))
}

fn load_shaped_devices() {
    debug!("Shaped-device ingress has changed. Attempting to load it.");
    let Ok(config) = load_config() else {
        warn!("Unable to load LibreQoS config while loading shaped devices");
        return;
    };
    for attempt in 1..=SHAPED_DEVICES_RELOAD_ATTEMPTS {
        match load_shaped_devices_from_preferred_source(config.as_ref()) {
            Ok((new_file, source)) => {
                debug!("Loaded shaped devices from {}", source.label());
                publish_shaped_devices(new_file);
                return;
            }
            Err(err) => {
                if attempt < SHAPED_DEVICES_RELOAD_ATTEMPTS {
                    warn!(
                        "Shaped-device ingress reload attempt {attempt}/{} failed: {err}. Retrying after {} ms.",
                        SHAPED_DEVICES_RELOAD_ATTEMPTS, SHAPED_DEVICES_RELOAD_RETRY_DELAY_MS
                    );
                    std::thread::sleep(Duration::from_millis(SHAPED_DEVICES_RELOAD_RETRY_DELAY_MS));
                } else {
                    let current = SHAPED_DEVICES.load();
                    warn!(
                        "Shaped-device ingress reload failed after {} attempts: {err}. Keeping last-known-good data with {} devices.",
                        SHAPED_DEVICES_RELOAD_ATTEMPTS,
                        current.devices.len()
                    );
                }
            }
        }
    }
}

pub fn shaped_devices_watcher() -> Result<()> {
    std::thread::Builder::new()
        .name("ShapedDevices Watcher".to_string())
        .spawn(|| {
            debug!("Watching for shaped-device ingress changes");
            if let Err(e) = watch_for_shaped_devices_changing() {
                error!("Error watching for shaped-device ingress: {:?}", e);
            }
        })?;
    Ok(())
}

fn watch_shaped_devices_hint_path(watch_name: &str, watch_path: PathBuf) -> Result<()> {
    let mut watcher = FileWatcher::new(watch_name, watch_path);
    watcher.set_file_exists_callback(load_shaped_devices);
    watcher.set_file_created_callback(load_shaped_devices);
    watcher.set_file_changed_callback(load_shaped_devices);
    loop {
        let result = watcher.watch();
        info!("{watch_name} watcher returned: {result:?}");
    }
}

/// Watches runtime status and source-ingress hints, then reevaluates the preferred shaped-device
/// source. The reload path always prefers active runtime shaping inputs over source ingress files.
fn watch_for_shaped_devices_changing() -> Result<()> {
    let config = load_config()?;
    let runtime_status_path = topology_runtime_status_path(config.as_ref());
    let csv_path = ConfigShapedDevices::path_for_config(config.as_ref());
    let topology_import = topology_import_path(config.as_ref());

    std::thread::Builder::new()
        .name("ShapedDevices Runtime Watcher".to_string())
        .spawn(move || {
            if let Err(err) =
                watch_shaped_devices_hint_path("topology_runtime_status.json", runtime_status_path)
            {
                error!("Error watching topology_runtime_status.json: {err:?}");
            }
        })?;

    std::thread::Builder::new()
        .name("ShapedDevices CSV Hint Watcher".to_string())
        .spawn(move || {
            if let Err(err) = watch_shaped_devices_hint_path("ShapedDevices.csv", csv_path) {
                error!("Error watching ShapedDevices.csv: {err:?}");
            }
        })?;

    watch_shaped_devices_hint_path("topology_import.json", topology_import)
}

pub fn get_one_network_map_layer(parent_idx: usize) -> BusResponse {
    let net_json = NETWORK_JSON.read();
    let nodes_ref = net_json.get_nodes_when_ready();
    let shaped_devices = SHAPED_DEVICES.load();
    let summaries = build_network_tree_summaries(nodes_ref, shaped_devices.as_ref());
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
}

pub fn full_network_map_snapshot() -> Vec<(usize, NetworkJsonTransport)> {
    let nj = NETWORK_JSON.read();
    let nodes = nj.get_nodes_when_ready();
    let shaped_devices = SHAPED_DEVICES.load();
    let summaries = build_network_tree_summaries(nodes, shaped_devices.as_ref());
    nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            (
                i,
                node_to_transport_with_summary(n, summaries.get(i).copied().unwrap_or_default()),
            )
        })
        .collect()
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
    let nj = NETWORK_JSON.read();
    let nodes = nj.get_nodes_when_ready();
    nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (i, node_to_transport_lite(n)))
        .collect()
}

pub fn get_full_network_map() -> BusResponse {
    BusResponse::NetworkMap(full_network_map_snapshot())
}

pub fn get_top_n_root_queues(n_queues: usize) -> BusResponse {
    let net_json = NETWORK_JSON.read();
    let nodes_ref = net_json.get_nodes_when_ready();
    let shaped_devices = SHAPED_DEVICES.load();
    let summaries = build_network_tree_summaries(nodes_ref, shaped_devices.as_ref());
    if let Some(parent) = nodes_ref.first() {
        let mut nodes = vec![(
            0,
            node_to_transport_with_summary(parent, summaries.first().copied().unwrap_or_default()),
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
}

pub fn map_node_names(nodes: &[usize]) -> BusResponse {
    let mut result = Vec::new();
    let reader = NETWORK_JSON.read();
    nodes.iter().for_each(|id| {
        if let Some(node) = reader.get_nodes_when_ready().get(*id) {
            result.push((*id, node.name.clone()));
        }
    });
    BusResponse::NodeNames(result)
}

/// Canonical parent-node metadata resolved from `network.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedParentNode {
    /// Canonical node name from `network.json`.
    pub name: String,
    /// Optional stable node identifier from `network.json` metadata.
    pub id: Option<String>,
}

/// Resolve a shaped-device parent reference into canonical `network.json`
/// parent metadata, preferring a stable node ID when one is available.
pub fn resolve_parent_node_reference(
    parent_node: &str,
    parent_node_id: Option<&str>,
) -> Option<ResolvedParentNode> {
    let trimmed_id = parent_node_id.map(str::trim).filter(|id| !id.is_empty());
    let trimmed = parent_node.trim();
    if trimmed.is_empty() && trimmed_id.is_none() {
        return None;
    }

    let reader = NETWORK_JSON.read();
    let nodes = reader.get_nodes_when_ready();

    if let Some(parent_node_id) = trimmed_id
        && let Some(node) = nodes
            .iter()
            .find(|node| node.id.as_deref() == Some(parent_node_id))
    {
        return Some(ResolvedParentNode {
            name: node.name.clone(),
            id: node.id.clone(),
        });
    }

    if let Some(node) = nodes.iter().find(|node| node.name == trimmed) {
        return Some(ResolvedParentNode {
            name: node.name.clone(),
            id: node.id.clone(),
        });
    }

    nodes.iter().find_map(|node| {
        node.active_attachment_name
            .as_deref()
            .filter(|alias| alias.trim() == trimmed)
            .map(|_| ResolvedParentNode {
                name: node.name.clone(),
                id: node.id.clone(),
            })
    })
}

/// Resolve a shaped-device parent node or active attachment alias into canonical `network.json`
/// parent metadata.
pub fn resolve_parent_node(parent_node: &str) -> Option<ResolvedParentNode> {
    resolve_parent_node_reference(parent_node, None)
}

pub fn resolve_parent_node_alias(parent_node: &str) -> Option<String> {
    resolve_parent_node(parent_node).map(|resolved| resolved.name)
}

pub fn get_funnel(circuit_id: &str) -> BusResponse {
    let reader = NETWORK_JSON.read();
    if let Some(index) = reader.get_index_for_name(circuit_id) {
        // Reverse the scanning order and skip the last entry (the parent)
        let mut result = Vec::new();
        for idx in reader.get_nodes_when_ready()[index]
            .parents
            .iter()
            .rev()
            .skip(1)
        {
            result.push((
                *idx,
                node_to_transport(&reader.get_nodes_when_ready()[*idx]),
            ));
        }
        return BusResponse::NetworkMap(result);
    }

    BusResponse::Fail("Unknown Node".into())
}

pub fn get_all_circuits() -> BusResponse {
    if let Ok(kernel_now) = time_since_boot() {
        let devices = SHAPED_DEVICES.load();
        let cache = SHAPED_DEVICE_HASH_CACHE.load();
        let data = THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .map(|(k, v)| {
                let last_seen_nanos = if v.last_seen > 0 {
                    let last_seen_nanos = v.last_seen as u128;
                    let since_boot = Duration::from(kernel_now).as_nanos();
                    //println!("since_boot: {:?}, last_seen: {:?}", since_boot, last_seen_nanos);
                    since_boot.saturating_sub(last_seen_nanos) as u64
                } else {
                    u64::MAX
                };

                // Map to circuit et al
                let mut circuit_id = v.circuit_id.clone();
                let mut circuit_name = None;
                let mut device_id = None;
                let mut device_name = None;
                let mut parent_node = None;
                // Plan is expressed in Mbps as f32
                let mut plan: DownUpOrder<f32> = DownUpOrder { down: 0.0, up: 0.0 };
                let device = shaped_device_from_hashes_or_ip(
                    &devices,
                    &cache,
                    k,
                    v.device_hash,
                    v.circuit_hash,
                );
                if let Some(device) = device {
                    if circuit_id.as_deref().unwrap_or_default().is_empty() {
                        circuit_id = Some(device.circuit_id.clone());
                    }
                    circuit_name = Some(device.circuit_name.clone());
                    device_id = Some(device.device_id.clone());
                    device_name = Some(device.device_name.clone());
                    parent_node = Some(
                        effective_parent_for_circuit(&device.circuit_id)
                            .map(|parent| parent.name)
                            .or_else(|| resolve_parent_node_alias(&device.parent_node))
                            .unwrap_or_else(|| device.parent_node.clone()),
                    );
                    plan.down = device.download_max_mbps.round();
                    plan.up = device.upload_max_mbps.round();
                }

                Circuit {
                    ip: k.as_ip(),
                    bytes_per_second: v.bytes_per_second,
                    actual_bytes_per_second: v.actual_bytes_per_second,
                    median_latency: v.median_latency(),
                    rtt_current_p50_nanos: DownUpOrder {
                        down: v
                            .rtt_buffer
                            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
                            .map(|rtt| rtt.as_nanos()),
                        up: v
                            .rtt_buffer
                            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50)
                            .map(|rtt| rtt.as_nanos()),
                    },
                    rtt_current_p95_nanos: DownUpOrder {
                        down: v
                            .rtt_buffer
                            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 95)
                            .map(|rtt| rtt.as_nanos()),
                        up: v
                            .rtt_buffer
                            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 95)
                            .map(|rtt| rtt.as_nanos()),
                    },
                    rtt_total_p50_nanos: DownUpOrder {
                        down: v
                            .rtt_buffer
                            .percentile(RttBucket::Total, FlowbeeEffectiveDirection::Download, 50)
                            .map(|rtt| rtt.as_nanos()),
                        up: v
                            .rtt_buffer
                            .percentile(RttBucket::Total, FlowbeeEffectiveDirection::Upload, 50)
                            .map(|rtt| rtt.as_nanos()),
                    },
                    rtt_total_p95_nanos: DownUpOrder {
                        down: v
                            .rtt_buffer
                            .percentile(RttBucket::Total, FlowbeeEffectiveDirection::Download, 95)
                            .map(|rtt| rtt.as_nanos()),
                        up: v
                            .rtt_buffer
                            .percentile(RttBucket::Total, FlowbeeEffectiveDirection::Upload, 95)
                            .map(|rtt| rtt.as_nanos()),
                    },
                    qoo: DownUpOrder {
                        down: v.qoq.download_total_f32(),
                        up: v.qoq.upload_total_f32(),
                    },
                    tcp_retransmit_sample: down_up_retransmit_sample(
                        v.tcp_retransmits,
                        v.tcp_retransmit_packets,
                    ),
                    circuit_id,
                    device_id,
                    circuit_name,
                    device_name,
                    parent_node,
                    plan,
                    last_seen_nanos,
                }
            })
            .collect();
        BusResponse::CircuitData(data)
    } else {
        BusResponse::CircuitData(Vec::new())
    }
}

pub fn get_circuit_by_id(desired_circuit_id: String) -> BusResponse {
    if let Ok(kernel_now) = time_since_boot() {
        let desired_hash = hash_to_i64(&desired_circuit_id);
        let devices = SHAPED_DEVICES.load();
        let cache = SHAPED_DEVICE_HASH_CACHE.load();
        let data = THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter_map(|(k, v)| {
                let device = shaped_device_from_hashes_or_ip(
                    &devices,
                    &cache,
                    k,
                    v.device_hash,
                    v.circuit_hash,
                );
                let matches_desired = v.circuit_hash == Some(desired_hash)
                    || v.circuit_id.as_deref() == Some(desired_circuit_id.as_str())
                    || device.is_some_and(|device| device.circuit_id == desired_circuit_id);
                if !matches_desired {
                    return None;
                }
                let last_seen_nanos = if v.last_seen > 0 {
                    let last_seen_nanos = v.last_seen as u128;
                    let since_boot = Duration::from(kernel_now).as_nanos();
                    //println!("since_boot: {:?}, last_seen: {:?}", since_boot, last_seen_nanos);
                    since_boot.saturating_sub(last_seen_nanos) as u64
                } else {
                    u64::MAX
                };

                // Map to circuit et al
                let mut circuit_id = v.circuit_id.clone();
                let mut circuit_name = None;
                let mut device_id = None;
                let mut device_name = None;
                let mut parent_node = None;
                // Plan is expressed in Mbps as f32
                let mut plan: DownUpOrder<f32> = DownUpOrder { down: 0.0, up: 0.0 };
                if let Some(device) = device {
                    if circuit_id.as_deref().unwrap_or_default().is_empty() {
                        circuit_id = Some(device.circuit_id.clone());
                    }
                    circuit_name = Some(device.circuit_name.clone());
                    device_id = Some(device.device_id.clone());
                    device_name = Some(device.device_name.clone());
                    parent_node = Some(
                        effective_parent_for_circuit(&device.circuit_id)
                            .map(|parent| parent.name)
                            .or_else(|| resolve_parent_node_alias(&device.parent_node))
                            .unwrap_or_else(|| device.parent_node.clone()),
                    );
                    plan.down = device.download_max_mbps.round();
                    plan.up = device.upload_max_mbps.round();
                }

                let circuit_id = Some(circuit_id.unwrap_or_else(|| desired_circuit_id.clone()));
                Some(Circuit {
                    ip: k.as_ip(),
                    bytes_per_second: v.bytes_per_second,
                    actual_bytes_per_second: v.actual_bytes_per_second,
                    median_latency: v.median_latency(),
                    rtt_current_p50_nanos: DownUpOrder {
                        down: v
                            .rtt_buffer
                            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
                            .map(|rtt| rtt.as_nanos()),
                        up: v
                            .rtt_buffer
                            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50)
                            .map(|rtt| rtt.as_nanos()),
                    },
                    rtt_current_p95_nanos: DownUpOrder {
                        down: v
                            .rtt_buffer
                            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 95)
                            .map(|rtt| rtt.as_nanos()),
                        up: v
                            .rtt_buffer
                            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 95)
                            .map(|rtt| rtt.as_nanos()),
                    },
                    rtt_total_p50_nanos: DownUpOrder {
                        down: v
                            .rtt_buffer
                            .percentile(RttBucket::Total, FlowbeeEffectiveDirection::Download, 50)
                            .map(|rtt| rtt.as_nanos()),
                        up: v
                            .rtt_buffer
                            .percentile(RttBucket::Total, FlowbeeEffectiveDirection::Upload, 50)
                            .map(|rtt| rtt.as_nanos()),
                    },
                    rtt_total_p95_nanos: DownUpOrder {
                        down: v
                            .rtt_buffer
                            .percentile(RttBucket::Total, FlowbeeEffectiveDirection::Download, 95)
                            .map(|rtt| rtt.as_nanos()),
                        up: v
                            .rtt_buffer
                            .percentile(RttBucket::Total, FlowbeeEffectiveDirection::Upload, 95)
                            .map(|rtt| rtt.as_nanos()),
                    },
                    qoo: DownUpOrder {
                        down: v.qoq.download_total_f32(),
                        up: v.qoq.upload_total_f32(),
                    },
                    tcp_retransmit_sample: down_up_retransmit_sample(
                        v.tcp_retransmits,
                        v.tcp_retransmit_packets,
                    ),
                    circuit_id,
                    device_id,
                    circuit_name,
                    device_name,
                    parent_node,
                    plan,
                    last_seen_nanos,
                })
            })
            .collect();
        BusResponse::CircuitData(data)
    } else {
        BusResponse::CircuitData(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_config::{
        CircuitAnchorsFile, Config, TOPOLOGY_RUNTIME_STATUS_FILENAME, TopologyShapingCircuitInput,
        TopologyShapingDeviceInput, TopologyShapingInputsFile,
    };
    use lqos_topology_compile::{ImportedTopologyBundle, TopologyImportFile};
    use serde_json::json;
    use std::fs;
    use std::net::Ipv4Addr;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough for tests")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
        fs::create_dir_all(&path).expect("temp directory should be creatable");
        path
    }

    fn write_shaped_devices_csv(path: &std::path::Path, circuit_id: &str, ip: &str) {
        fs::write(
            path,
            format!(
                concat!(
                    "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,",
                    "Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                    "\"{}\",\"Circuit {}\",\"device-{}\",",
                    "\"Device {}\",\"Tower 1\",\"aa:bb:cc:dd:ee:ff\",\"{}\",\"\",",
                    "\"10\",\"10\",\"100\",\"100\",\"\"\n",
                ),
                circuit_id, circuit_id, circuit_id, circuit_id, ip,
            ),
        )
        .expect("ShapedDevices.csv should write");
    }

    fn write_runtime_status(
        path: &std::path::Path,
        ready: bool,
        shaping_inputs_path: &std::path::Path,
    ) {
        fs::write(
            path,
            serde_json::json!({
                "schema_version": 1,
                "ready": ready,
                "shaping_inputs_path": shaping_inputs_path,
                "effective_state_path": "",
                "effective_network_path": "",
                "source_generation": "gen-1",
                "shaping_generation": "shape-1",
            })
            .to_string(),
        )
        .expect("status should write");
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
    fn shaped_device_from_hashes_or_ip_falls_back_to_ip() {
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(vec![ShapedDevice {
            circuit_id: "circuit-1".to_string(),
            circuit_name: "Circuit Alpha".to_string(),
            device_id: "device-1".to_string(),
            parent_node: "Parent-A".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 168, 1, 10), 32)],
            ..Default::default()
        }]);
        let cache = ShapedDeviceHashCache::default();
        let ip = XdpIpAddress::from_ip("192.168.1.10".parse().expect("test IP should parse"));

        let device =
            shaped_device_from_hashes_or_ip(&shaped, &cache, &ip, None, None).expect("match");

        assert_eq!(device.circuit_id, "circuit-1");
        assert_eq!(device.circuit_name, "Circuit Alpha");
    }

    #[test]
    fn shaped_devices_from_runtime_inputs_uses_effective_parent_and_ip_lookup() {
        let shaping_inputs = TopologyShapingInputsFile {
            circuits: vec![TopologyShapingCircuitInput {
                circuit_id: "circuit-1".to_string(),
                circuit_name: "Circuit Alpha".to_string(),
                anchor_node_id: Some("anchor-1".to_string()),
                effective_parent_node_name: "Parent-A".to_string(),
                effective_parent_node_id: "parent-id-1".to_string(),
                download_min_mbps: 10.0,
                upload_min_mbps: 5.0,
                download_max_mbps: 50.0,
                upload_max_mbps: 10.0,
                devices: vec![TopologyShapingDeviceInput {
                    device_id: "device-1".to_string(),
                    device_name: "Device Alpha".to_string(),
                    mac: "aa:bb:cc:dd:ee:ff".to_string(),
                    ipv4: vec!["192.168.1.10/32".to_string()],
                    comment: "device-comment".to_string(),
                    ..TopologyShapingDeviceInput::default()
                }],
                comment: "circuit-comment".to_string(),
                ..TopologyShapingCircuitInput::default()
            }],
            ..TopologyShapingInputsFile::default()
        };

        let shaped = shaped_devices_from_runtime_inputs(&shaping_inputs);
        let ip = XdpIpAddress::from_ip("192.168.1.10".parse().expect("test IP should parse"));
        let device = shaped
            .get_device_from_ip(&ip)
            .expect("lookup should resolve");

        assert_eq!(device.parent_node, "Parent-A");
        assert_eq!(device.parent_node_id.as_deref(), Some("parent-id-1"));
        assert_eq!(device.anchor_node_id.as_deref(), Some("anchor-1"));
        assert_eq!(device.comment, "device-comment");
        assert_eq!(device.circuit_id, "circuit-1");
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
        write_runtime_status(&status_path, true, &active_shaping_path);

        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: Some(state_directory.to_string_lossy().to_string()),
            ..Config::default()
        };

        let loaded = load_active_runtime_shaping_inputs(&config)
            .expect("runtime shaping should load")
            .expect("runtime shaping inputs should be active");
        assert_eq!(loaded.circuits.len(), 1);
        assert_eq!(loaded.circuits[0].circuit_id, "active-circuit");
    }

    #[test]
    fn load_shaped_devices_prefers_ready_runtime_even_when_empty() {
        let lqos_directory = unique_temp_dir("lqosd-shaped-devices-runtime-empty");
        let state_directory = lqos_directory.join("state");
        fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");
        fs::create_dir_all(state_directory.join("shaping")).expect("shaping dir should exist");

        let runtime_path = lqos_directory.join("runtime_shaping_inputs.json");
        let status_path = state_directory
            .join("topology")
            .join(TOPOLOGY_RUNTIME_STATUS_FILENAME);
        write_shaped_devices_csv(
            &lqos_directory.join("ShapedDevices.csv"),
            "csv-circuit",
            "192.0.2.10/32",
        );
        fs::write(
            &runtime_path,
            serde_json::to_string_pretty(&TopologyShapingInputsFile::default())
                .expect("empty shaping inputs should encode"),
        )
        .expect("runtime shaping inputs should write");
        write_runtime_status(&status_path, true, &runtime_path);

        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: Some(state_directory.to_string_lossy().to_string()),
            ..Config::default()
        };

        let (loaded, source) = load_shaped_devices_from_preferred_source(&config)
            .expect("preferred shaped-device source should load");
        assert_eq!(source, ShapedDevicesLoadSource::RuntimeShapingInputs);
        assert!(loaded.devices.is_empty());
    }

    #[test]
    fn load_shaped_devices_uses_csv_when_runtime_is_not_ready() {
        let lqos_directory = unique_temp_dir("lqosd-shaped-devices-csv-fallback");
        let state_directory = lqos_directory.join("state");
        fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");

        let runtime_path = lqos_directory.join("runtime_shaping_inputs.json");
        let status_path = state_directory
            .join("topology")
            .join(TOPOLOGY_RUNTIME_STATUS_FILENAME);
        write_shaped_devices_csv(
            &lqos_directory.join("ShapedDevices.csv"),
            "csv-circuit",
            "192.0.2.10/32",
        );
        fs::write(
            &runtime_path,
            serde_json::to_string_pretty(&TopologyShapingInputsFile {
                circuits: vec![TopologyShapingCircuitInput {
                    circuit_id: "runtime-circuit".to_string(),
                    devices: vec![TopologyShapingDeviceInput {
                        device_id: "device-1".to_string(),
                        device_name: "Device 1".to_string(),
                        ipv4: vec!["198.51.100.10/32".to_string()],
                        ..TopologyShapingDeviceInput::default()
                    }],
                    ..TopologyShapingCircuitInput::default()
                }],
                ..TopologyShapingInputsFile::default()
            })
            .expect("runtime shaping inputs should encode"),
        )
        .expect("runtime shaping inputs should write");
        write_runtime_status(&status_path, false, &runtime_path);

        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: Some(state_directory.to_string_lossy().to_string()),
            ..Config::default()
        };

        let (loaded, source) =
            load_shaped_devices_from_preferred_source(&config).expect("CSV fallback should load");
        assert_eq!(source, ShapedDevicesLoadSource::ShapedDevicesCsv);
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].circuit_id, "csv-circuit");
    }

    #[test]
    fn load_shaped_devices_uses_topology_import_when_runtime_is_not_ready() {
        let lqos_directory = unique_temp_dir("lqosd-shaped-devices-topology-import-fallback");
        let state_directory = lqos_directory.join("state");
        fs::create_dir_all(state_directory.join("topology")).expect("topology dir should exist");

        let runtime_path = lqos_directory.join("runtime_shaping_inputs.json");
        let status_path = state_directory
            .join("topology")
            .join(TOPOLOGY_RUNTIME_STATUS_FILENAME);
        fs::write(
            &runtime_path,
            serde_json::to_string_pretty(&TopologyShapingInputsFile::default())
                .expect("runtime shaping inputs should encode"),
        )
        .expect("runtime shaping inputs should write");
        write_runtime_status(&status_path, false, &runtime_path);

        let mut import_devices = ConfigShapedDevices::default();
        import_devices.replace_with_new_data(vec![ShapedDevice {
            circuit_id: "import-circuit".to_string(),
            circuit_name: "Import Circuit".to_string(),
            device_id: "device-import".to_string(),
            parent_node: "Tower Import".to_string(),
            ipv4: vec![(Ipv4Addr::new(203, 0, 113, 10), 32)],
            ..Default::default()
        }]);
        let imported = ImportedTopologyBundle {
            source: "test/import".to_string(),
            generated_unix: Some(123),
            ingress_identity: Some("import-base".to_string()),
            native_canonical: None,
            native_editor: None,
            parent_candidates: None,
            compatibility_network_json: json!({}),
            shaped_devices: import_devices,
            circuit_anchors: CircuitAnchorsFile::default(),
            ethernet_advisories: Vec::new(),
        };
        TopologyImportFile::from_imported_bundle(&imported, "full")
            .save(&Config {
                lqos_directory: lqos_directory.to_string_lossy().to_string(),
                state_directory: Some(state_directory.to_string_lossy().to_string()),
                ..Config::default()
            })
            .expect("topology import should save");

        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: Some(state_directory.to_string_lossy().to_string()),
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;

        let (loaded, source) = load_shaped_devices_from_preferred_source(&config)
            .expect("topology import fallback should load");
        assert_eq!(source, ShapedDevicesLoadSource::TopologyImport);
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].circuit_id, "import-circuit");
    }
}

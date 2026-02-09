use anyhow::Result;
use arc_swap::ArcSwap;
use fxhash::FxHashMap;
use lqos_bus::{BusResponse, Circuit};
use lqos_config::{ConfigShapedDevices, NetworkJsonTransport, ShapedDevice};
use lqos_utils::file_watcher::FileWatcher;
use lqos_utils::hash_to_i64;
use lqos_utils::rtt::{FlowbeeEffectiveDirection, RttBucket};
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::time_since_boot;
use once_cell::sync::Lazy;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

// Removed rate_for_plan() function - no longer needed with f32 plan structures

mod netjson;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
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
        if let Some(idx) = self.by_device_hash.get(&device_hash).copied() {
            if shaped
                .devices
                .get(idx)
                .is_some_and(|d| d.device_hash == device_hash)
            {
                return Some(idx);
            }
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
        if let Some(idx) = self.by_circuit_hash.get(&circuit_hash).copied() {
            if shaped
                .devices
                .get(idx)
                .is_some_and(|d| d.circuit_hash == circuit_hash)
            {
                return Some(idx);
            }
        }
        shaped
            .devices
            .iter()
            .position(|d| d.circuit_hash == circuit_hash)
    }
}

pub static SHAPED_DEVICE_HASH_CACHE: Lazy<ArcSwap<ShapedDeviceHashCache>> =
    Lazy::new(|| ArcSwap::new(Arc::new(ShapedDeviceHashCache::default())));

fn load_shaped_devices() {
    debug!("ShapedDevices.csv has changed. Attempting to load it.");
    let shaped_devices = ConfigShapedDevices::load();
    if let Ok(new_file) = shaped_devices {
        debug!("ShapedDevices.csv loaded");
        let cache = ShapedDeviceHashCache::from_devices(&new_file.devices);
        SHAPED_DEVICES.store(Arc::new(new_file));
        SHAPED_DEVICE_HASH_CACHE.store(Arc::new(cache));
        let nj = NETWORK_JSON.read();
        crate::throughput_tracker::THROUGHPUT_TRACKER.refresh_circuit_ids(&nj);
    } else {
        warn!(
            "ShapedDevices.csv failed to load, see previous error messages. Reverting to empty set."
        );
        SHAPED_DEVICES.store(Arc::new(ConfigShapedDevices::default()));
        SHAPED_DEVICE_HASH_CACHE.store(Arc::new(ShapedDeviceHashCache::default()));
    }
}

pub fn shaped_devices_watcher() -> Result<()> {
    std::thread::Builder::new()
        .name("ShapedDevices Watcher".to_string())
        .spawn(|| {
            debug!("Watching for ShapedDevices.csv changes");
            if let Err(e) = watch_for_shaped_devices_changing() {
                error!("Error watching for ShapedDevices.csv: {:?}", e);
            }
        })?;
    Ok(())
}

/// Fires up a Linux file system watcher than notifies
/// when `ShapedDevices.csv` changes, and triggers a reload.
fn watch_for_shaped_devices_changing() -> Result<()> {
    let watch_path = ConfigShapedDevices::path();
    if watch_path.is_err() {
        error!("Unable to generate path for ShapedDevices.csv");
        return Err(anyhow::Error::msg(
            "Unable to create path for ShapedDevices.csv",
        ));
    }
    let watch_path = watch_path?;

    let mut watcher = FileWatcher::new("ShapedDevices.csv", watch_path);
    watcher.set_file_exists_callback(load_shaped_devices);
    watcher.set_file_created_callback(load_shaped_devices);
    watcher.set_file_changed_callback(load_shaped_devices);
    loop {
        let result = watcher.watch();
        info!("ShapedDevices watcher returned: {result:?}");
    }
}

pub fn get_one_network_map_layer(parent_idx: usize) -> BusResponse {
    let net_json = NETWORK_JSON.read();
    if let Some(parent) = net_json.get_cloned_entry_by_index(parent_idx) {
        let mut nodes = vec![(parent_idx, parent)];
        nodes.extend_from_slice(&net_json.get_cloned_children(parent_idx));
        BusResponse::NetworkMap(nodes)
    } else {
        BusResponse::Fail("No such node".to_string())
    }
}

pub fn get_full_network_map() -> BusResponse {
    let nj = NETWORK_JSON.read();
    let data = {
        nj.get_nodes_when_ready()
            .iter()
            .enumerate()
            .map(|(i, n)| (i, n.clone_to_transit()))
            .collect::<Vec<(usize, NetworkJsonTransport)>>()
    };

    BusResponse::NetworkMap(data)
}

pub fn get_top_n_root_queues(n_queues: usize) -> BusResponse {
    let net_json = NETWORK_JSON.read();
    if let Some(parent) = net_json.get_cloned_entry_by_index(0) {
        let mut nodes = vec![(0, parent)];
        nodes.extend_from_slice(&net_json.get_cloned_children(0));
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
                    is_virtual: false,
                    max_throughput: (0, 0),
                    current_throughput: other_bw,
                    current_packets: other_packets,
                    current_tcp_packets: other_tcp_packets,
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
            result.push((*idx, reader.get_nodes_when_ready()[*idx].clone_to_transit()));
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
                let mut circuit_id = None;
                let mut circuit_name = None;
                let mut device_id = None;
                let mut device_name = None;
                let mut parent_node = None;
                // Plan is expressed in Mbps as f32
                let mut plan: DownUpOrder<f32> = DownUpOrder { down: 0.0, up: 0.0 };
                let device = v
                    .device_hash
                    .and_then(|device_hash| cache.index_by_device_hash(&devices, device_hash))
                    .or_else(|| {
                        v.circuit_hash.and_then(|circuit_hash| {
                            cache.index_by_circuit_hash(&devices, circuit_hash)
                        })
                    })
                    .and_then(|idx| devices.devices.get(idx));
                if let Some(device) = device {
                    circuit_id = Some(device.circuit_id.clone());
                    circuit_name = Some(device.circuit_name.clone());
                    device_id = Some(device.device_id.clone());
                    device_name = Some(device.device_name.clone());
                    parent_node = Some(device.parent_node.clone());
                    plan.down = device.download_max_mbps.round();
                    plan.up = device.upload_max_mbps.round();
                }

                Circuit {
                    ip: k.as_ip(),
                    bytes_per_second: v.bytes_per_second,
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
                    tcp_retransmits: v.tcp_retransmits,
                    tcp_packets: v.tcp_packets.checked_sub_or_zero(v.prev_tcp_packets),
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
                if v.circuit_hash != Some(desired_hash) {
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
                let mut circuit_id = None;
                let mut circuit_name = None;
                let mut device_id = None;
                let mut device_name = None;
                let mut parent_node = None;
                // Plan is expressed in Mbps as f32
                let mut plan: DownUpOrder<f32> = DownUpOrder { down: 0.0, up: 0.0 };
                let device = v
                    .device_hash
                    .and_then(|device_hash| cache.index_by_device_hash(&devices, device_hash))
                    .or_else(|| {
                        v.circuit_hash.and_then(|circuit_hash| {
                            cache.index_by_circuit_hash(&devices, circuit_hash)
                        })
                    })
                    .and_then(|idx| devices.devices.get(idx));
                if let Some(device) = device {
                    circuit_id = Some(device.circuit_id.clone());
                    circuit_name = Some(device.circuit_name.clone());
                    device_id = Some(device.device_id.clone());
                    device_name = Some(device.device_name.clone());
                    parent_node = Some(device.parent_node.clone());
                    plan.down = device.download_max_mbps.round();
                    plan.up = device.upload_max_mbps.round();
                }

                let circuit_id = Some(circuit_id.unwrap_or_else(|| desired_circuit_id.clone()));
                Some(Circuit {
                    ip: k.as_ip(),
                    bytes_per_second: v.bytes_per_second,
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
                    tcp_retransmits: v.tcp_retransmits,
                    tcp_packets: v.tcp_packets.checked_sub_or_zero(v.prev_tcp_packets),
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

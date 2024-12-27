use std::net::IpAddr;
use anyhow::Result;
use tracing::{debug, error, info, warn};
use lqos_bus::{BusResponse, Circuit};
use lqos_config::{ConfigShapedDevices, NetworkJsonTransport};
use lqos_utils::file_watcher::FileWatcher;
use once_cell::sync::Lazy;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;
use arc_swap::ArcSwap;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::time_since_boot;

mod netjson;
pub use netjson::*;
use crate::throughput_tracker::THROUGHPUT_TRACKER;

pub static SHAPED_DEVICES: Lazy<ArcSwap<ConfigShapedDevices>> =
    Lazy::new(|| ArcSwap::new(Arc::new(ConfigShapedDevices::default())));
pub static STATS_NEEDS_NEW_SHAPED_DEVICES: AtomicBool = AtomicBool::new(true);

fn load_shaped_devices() {
    debug!("ShapedDevices.csv has changed. Attempting to load it.");
    let shaped_devices = ConfigShapedDevices::load();
    if let Ok(new_file) = shaped_devices {
        debug!("ShapedDevices.csv loaded");
        SHAPED_DEVICES.store(Arc::new(new_file));
        let nj = NETWORK_JSON.read().unwrap();
        crate::throughput_tracker::THROUGHPUT_TRACKER.refresh_circuit_ids(&nj);
        STATS_NEEDS_NEW_SHAPED_DEVICES.store(true, std::sync::atomic::Ordering::Relaxed);
    } else {
        warn!("ShapedDevices.csv failed to load, see previous error messages. Reverting to empty set.");
        SHAPED_DEVICES.store(Arc::new(ConfigShapedDevices::default()));
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
    let watch_path = watch_path.unwrap();

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
    let net_json = NETWORK_JSON.read().unwrap();
    if let Some(parent) = net_json.get_cloned_entry_by_index(parent_idx) {
        let mut nodes = vec![(parent_idx, parent)];
        nodes.extend_from_slice(&net_json.get_cloned_children(parent_idx));
        BusResponse::NetworkMap(nodes)
    } else {
        BusResponse::Fail("No such node".to_string())
    }
}

pub fn get_full_network_map() -> BusResponse {
    let data = {
            NETWORK_JSON.read().unwrap()
                .get_nodes_when_ready()
                .iter()
                .enumerate()
                .map(|(i, n)| (i, n.clone_to_transit()))
                .collect::<Vec<(usize, NetworkJsonTransport)>>()
    };
    
    BusResponse::NetworkMap(data)
}

pub fn get_top_n_root_queues(n_queues: usize) -> BusResponse {
    let net_json = NETWORK_JSON.read().unwrap();
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
    let reader = NETWORK_JSON.read().unwrap();
    nodes.iter().for_each(|id| {
        if let Some(node) = reader.get_nodes_when_ready().get(*id) {
            result.push((*id, node.name.clone()));
        }
    });
    BusResponse::NodeNames(result)
}

pub fn get_funnel(circuit_id: &str) -> BusResponse {
    let reader = NETWORK_JSON.read().unwrap();
    if let Some(index) = reader.get_index_for_name(circuit_id) {
        // Reverse the scanning order and skip the last entry (the parent)
        let mut result = Vec::new();
        for idx in reader.get_nodes_when_ready()[index].parents.iter().rev().skip(1) {
            result.push((*idx, reader.get_nodes_when_ready()[*idx].clone_to_transit()));
        }
        return BusResponse::NetworkMap(result);
    }

    BusResponse::Fail("Unknown Node".into())
}

pub fn get_all_circuits() -> BusResponse {
    if let Ok(kernel_now) = time_since_boot() {
        let devices = SHAPED_DEVICES.load();
            let data = THROUGHPUT_TRACKER.
            raw_data.lock().unwrap()
            .iter()
            .map(|(k,v)| {
                let ip = k.as_ip();
                let last_seen_nanos = if v.last_seen > 0 {
                    let last_seen_nanos = v.last_seen as u128;
                    let since_boot = Duration::from(kernel_now).as_nanos();
                    //println!("since_boot: {:?}, last_seen: {:?}", since_boot, last_seen_nanos);
                    (since_boot - last_seen_nanos) as u64
                } else {
                    u64::MAX
                };

                // Map to circuit et al
                let mut circuit_id = None;
                let mut circuit_name = None;
                let mut device_id = None;
                let mut device_name = None;
                let mut parent_node = None;
                let mut plan = DownUpOrder::new(0, 0);
                let lookup = match ip {
                    IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                    IpAddr::V6(ip) => ip,
                };
                if let Some(c) = devices.trie.longest_match(lookup) {
                    circuit_id = Some(devices.devices[*c.1].circuit_id.clone());
                    circuit_name = Some(devices.devices[*c.1].circuit_name.clone());
                    device_id = Some(devices.devices[*c.1].device_id.clone());
                    device_name = Some(devices.devices[*c.1].device_name.clone());
                    parent_node = Some(devices.devices[*c.1].parent_node.clone());
                    plan.down = devices.devices[*c.1].download_max_mbps;
                    plan.up = devices.devices[*c.1].upload_max_mbps;
                }

                Circuit {
                    ip: k.as_ip(),
                    bytes_per_second: v.bytes_per_second,
                    median_latency: v.median_latency(),
                    tcp_retransmits: v.tcp_retransmits,
                    circuit_id,
                    device_id,
                    circuit_name,
                    device_name,
                    parent_node,
                    plan,
                    last_seen_nanos,
                }
            }).collect();
        BusResponse::CircuitData(data)
    } else {
        BusResponse::CircuitData(Vec::new())
    }
}

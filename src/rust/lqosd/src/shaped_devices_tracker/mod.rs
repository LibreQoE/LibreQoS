use anyhow::Result;
use log::{error, info, warn};
use lqos_bus::BusResponse;
use lqos_config::{ConfigShapedDevices, NetworkJsonTransport};
use lqos_utils::file_watcher::FileWatcher;
use once_cell::sync::Lazy;
use std::sync::{RwLock, atomic::AtomicBool};
use tokio::task::spawn_blocking;
mod netjson;
pub use netjson::*;

pub static SHAPED_DEVICES: Lazy<RwLock<ConfigShapedDevices>> =
    Lazy::new(|| RwLock::new(ConfigShapedDevices::default()));
pub static STATS_NEEDS_NEW_SHAPED_DEVICES: AtomicBool = AtomicBool::new(false);

fn load_shaped_devices() {
    info!("ShapedDevices.csv has changed. Attempting to load it.");
    let shaped_devices = ConfigShapedDevices::load();
    if let Ok(new_file) = shaped_devices {
        info!("ShapedDevices.csv loaded");
        *SHAPED_DEVICES.write().unwrap() = new_file;
        crate::throughput_tracker::THROUGHPUT_TRACKER.refresh_circuit_ids();
        STATS_NEEDS_NEW_SHAPED_DEVICES.store(true, std::sync::atomic::Ordering::Relaxed);
    } else {
        warn!("ShapedDevices.csv failed to load, see previous error messages. Reverting to empty set.");
        *SHAPED_DEVICES.write().unwrap() = ConfigShapedDevices::default();
    }
}

pub async fn shaped_devices_watcher() {
    spawn_blocking(|| {
        info!("Watching for ShapedDevices.csv changes");
        let _ = watch_for_shaped_devices_changing();
    });
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
            nodes.drain(n_queues..).for_each(|n| {
                other_bw.0 += n.1.current_throughput.0;
                other_bw.1 += n.1.current_throughput.1;
            });

            nodes.push((
                0,
                NetworkJsonTransport {
                    name: "Others".into(),
                    max_throughput: (0, 0),
                    current_throughput: other_bw,
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
        if let Some(node) = reader.nodes.get(*id) {
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
        for idx in reader.nodes[index].parents.iter().rev().skip(1) {
            result.push((*idx, reader.nodes[*idx].clone_to_transit()));
        }
        return BusResponse::NetworkMap(result);
    }

    BusResponse::Fail("Unknown Node".into())
}
